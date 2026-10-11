//! Deterministic offscreen rendering and visual regression testing for VUI.

use std::{
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
    sync::mpsc,
    time::Duration,
};
use vello::{
    AaConfig, RenderParams, Renderer, RendererOptions, peniko::Color as VelloColor,
    util::RenderContext, wgpu,
};
use vui_core::{Color, PaintList, Size, Theme, Widget};
use vui_testing::TestHarness;
use vui_vello::SceneBuilder;

const BYTES_PER_PIXEL: u32 = 4;
const COPY_ROW_ALIGNMENT: u32 = 256;

/// Error returned by snapshot rendering, storage, or comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotError {
    message: String,
}

impl SnapshotError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for SnapshotError {}

/// An owned, tightly packed RGBA8 image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotImage {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl SnapshotImage {
    /// Creates a packed RGBA8 image.
    ///
    /// # Errors
    ///
    /// Returns an error when the dimensions overflow or `pixels` has the wrong length.
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, SnapshotError> {
        let expected = image_len(width, height)?;
        if pixels.len() != expected {
            return Err(SnapshotError::new(format!(
                "RGBA image is {actual} bytes; expected {expected}",
                actual = pixels.len()
            )));
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Loads an image and converts it to packed RGBA8.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be opened or decoded.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, SnapshotError> {
        let path = path.as_ref();
        let decoded = image::ImageReader::open(path)
            .map_err(|error| SnapshotError::new(format!("open {}: {error}", path.display())))?
            .decode()
            .map_err(|error| SnapshotError::new(format!("decode {}: {error}", path.display())))?
            .to_rgba8();
        Self::new(decoded.width(), decoded.height(), decoded.into_raw())
    }

    /// Encodes this image as PNG.
    ///
    /// # Errors
    ///
    /// Returns an error when directories cannot be created or the PNG cannot be written.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), SnapshotError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                SnapshotError::new(format!("create {}: {error}", parent.display()))
            })?;
        }
        image::save_buffer(
            path,
            &self.pixels,
            self.width,
            self.height,
            image::ColorType::Rgba8,
        )
        .map_err(|error| SnapshotError::new(format!("write {}: {error}", path.display())))
    }
}

/// Fixed inputs used to produce a repeatable frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapshotConfig {
    pub logical_size: Size,
    pub scale_factor: f64,
    pub background: Color,
    pub theme: Theme,
    pub reduced_motion: bool,
}

impl SnapshotConfig {
    #[must_use]
    pub fn new(logical_size: Size, theme: Theme) -> Self {
        Self {
            logical_size,
            scale_factor: 1.0,
            background: theme.window,
            theme,
            reduced_motion: true,
        }
    }

    #[must_use]
    pub fn scale_factor(mut self, scale_factor: f64) -> Self {
        self.scale_factor = scale_factor;
        self
    }

    #[must_use]
    pub const fn background(mut self, background: Color) -> Self {
        self.background = background;
        self
    }

    #[must_use]
    pub const fn reduced_motion(mut self, reduced_motion: bool) -> Self {
        self.reduced_motion = reduced_motion;
        self
    }
}

/// Reusable headless Vello renderer.
pub struct SnapshotRenderer {
    context: RenderContext,
    device_id: usize,
    renderer: Renderer,
    scenes: SceneBuilder,
}

impl SnapshotRenderer {
    /// Creates a headless renderer using the selected wgpu adapter.
    ///
    /// # Errors
    ///
    /// Returns an error when no compatible adapter exists or Vello initialization fails.
    pub fn new() -> Result<Self, SnapshotError> {
        let mut context = RenderContext::new();
        let device_id = pollster::block_on(context.device(None))
            .ok_or_else(|| SnapshotError::new("no compatible Vello/wgpu device available"))?;
        let renderer = Renderer::new(
            &context.devices[device_id].device,
            RendererOptions::default(),
        )
        .map_err(|error| SnapshotError::new(format!("create Vello renderer: {error}")))?;
        Ok(Self {
            context,
            device_id,
            renderer,
            scenes: SceneBuilder::new(),
        })
    }

    /// Lays out and captures one widget using a deterministic test harness.
    ///
    /// # Errors
    ///
    /// Returns an error when configuration, rendering, or GPU readback fails.
    pub fn capture_widget(
        &mut self,
        root: impl Widget + 'static,
        config: SnapshotConfig,
    ) -> Result<SnapshotImage, SnapshotError> {
        let mut harness = TestHarness::new(root, config.logical_size);
        self.capture_harness(&mut harness, config)
    }

    /// Captures the current state of an existing deterministic test harness.
    ///
    /// # Errors
    ///
    /// Returns an error when configuration, rendering, or GPU readback fails.
    pub fn capture_harness(
        &mut self,
        harness: &mut TestHarness,
        config: SnapshotConfig,
    ) -> Result<SnapshotImage, SnapshotError> {
        harness.resize(config.logical_size);
        harness.set_theme(config.theme);
        harness.set_reduced_motion(config.reduced_motion);
        let frame = harness.frame();
        self.capture_paint(&frame.paint, config)
    }

    /// Renders a paint list into a packed RGBA8 snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when dimensions are invalid, Vello fails, or readback fails.
    pub fn capture_paint(
        &mut self,
        paint: &PaintList,
        config: SnapshotConfig,
    ) -> Result<SnapshotImage, SnapshotError> {
        let (width, height) = physical_size(&config)?;
        let scene = self.scenes.build(paint, config.scale_factor);
        let device = &self.context.devices[self.device_id].device;
        let queue = &self.context.devices[self.device_id].queue;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("vui snapshot target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.renderer
            .render_to_texture(
                device,
                queue,
                scene,
                &view,
                &RenderParams {
                    base_color: VelloColor::from_rgba8(
                        config.background.r,
                        config.background.g,
                        config.background.b,
                        config.background.a,
                    ),
                    width,
                    height,
                    antialiasing_method: AaConfig::Area,
                },
            )
            .map_err(|error| SnapshotError::new(format!("render Vello snapshot: {error}")))?;

        let pixels = read_texture(device, queue, &texture, width, height)?;
        SnapshotImage::new(width, height, pixels)
    }
}

fn read_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, SnapshotError> {
    let unpadded_row = width
        .checked_mul(BYTES_PER_PIXEL)
        .ok_or_else(|| SnapshotError::new("snapshot row size overflow"))?;
    let padded_row = unpadded_row.div_ceil(COPY_ROW_ALIGNMENT) * COPY_ROW_ALIGNMENT;
    let buffer_size = u64::from(padded_row)
        .checked_mul(u64::from(height))
        .ok_or_else(|| SnapshotError::new("snapshot buffer size overflow"))?;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("vui snapshot readback"),
        size: buffer_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("vui snapshot copy"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let submission = queue.submit([encoder.finish()]);
    let (sender, receiver) = mpsc::sync_channel(1);
    readback.map_async(wgpu::MapMode::Read, .., move |result| {
        let _ = sender.send(result);
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(Duration::from_secs(30)),
        })
        .map_err(|error| SnapshotError::new(format!("wait for snapshot readback: {error}")))?;
    receiver
        .recv_timeout(Duration::from_secs(30))
        .map_err(|error| SnapshotError::new(format!("receive snapshot readback: {error}")))?
        .map_err(|error| SnapshotError::new(format!("map snapshot readback: {error}")))?;

    let mapped = readback
        .get_mapped_range(..)
        .map_err(|error| SnapshotError::new(format!("read mapped snapshot: {error}")))?;
    let row_len = usize::try_from(unpadded_row)
        .map_err(|_| SnapshotError::new("snapshot row is too large"))?;
    let padded_len = usize::try_from(padded_row)
        .map_err(|_| SnapshotError::new("padded snapshot row is too large"))?;
    let mut pixels = Vec::with_capacity(image_len(width, height)?);
    for row in mapped.chunks_exact(padded_len) {
        pixels.extend_from_slice(&row[..row_len]);
    }
    drop(mapped);
    readback.unmap();
    Ok(pixels)
}

/// Maximum accepted image difference.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapshotTolerance {
    pub channel_delta: u8,
    pub max_changed_pixels: usize,
    pub max_changed_ratio: f64,
}

impl SnapshotTolerance {
    pub const EXACT: Self = Self {
        channel_delta: 0,
        max_changed_pixels: 0,
        max_changed_ratio: 0.0,
    };
}

/// Difference metrics for two equally sized images.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SnapshotReport {
    pub total_pixels: usize,
    pub changed_pixels: usize,
    pub changed_ratio: f64,
    pub maximum_channel_delta: u8,
}

impl SnapshotReport {
    #[must_use]
    pub fn accepted(self, tolerance: SnapshotTolerance) -> bool {
        self.changed_pixels <= tolerance.max_changed_pixels
            && self.changed_ratio <= tolerance.max_changed_ratio
    }
}

/// Compares equally sized images after ignoring channel deltas at or below `channel_delta`.
///
/// # Errors
///
/// Returns an error when dimensions differ or their pixel count overflows.
pub fn compare(
    expected: &SnapshotImage,
    actual: &SnapshotImage,
    channel_delta: u8,
) -> Result<SnapshotReport, SnapshotError> {
    ensure_same_size(expected, actual)?;
    let mut changed_pixels = 0;
    let mut maximum_channel_delta = 0;
    for (expected_pixel, actual_pixel) in expected
        .pixels
        .chunks_exact(4)
        .zip(actual.pixels.chunks_exact(4))
    {
        let pixel_maximum = expected_pixel
            .iter()
            .zip(actual_pixel)
            .map(|(expected, actual)| expected.abs_diff(*actual))
            .max()
            .unwrap_or_default();
        maximum_channel_delta = maximum_channel_delta.max(pixel_maximum);
        if pixel_maximum > channel_delta {
            changed_pixels += 1;
        }
    }
    let total_pixels = image_len(expected.width, expected.height)? / 4;
    #[allow(clippy::cast_precision_loss)]
    let changed_ratio = if total_pixels == 0 {
        0.0
    } else {
        changed_pixels as f64 / total_pixels as f64
    };
    Ok(SnapshotReport {
        total_pixels,
        changed_pixels,
        changed_ratio,
        maximum_channel_delta,
    })
}

/// Produces an opaque image containing absolute RGB channel differences.
///
/// # Errors
///
/// Returns an error when the image dimensions differ.
pub fn difference_image(
    expected: &SnapshotImage,
    actual: &SnapshotImage,
) -> Result<SnapshotImage, SnapshotError> {
    ensure_same_size(expected, actual)?;
    let pixels = expected
        .pixels
        .iter()
        .zip(&actual.pixels)
        .enumerate()
        .map(|(index, (expected, actual))| {
            if index % 4 == 3 {
                255
            } else {
                expected.abs_diff(*actual)
            }
        })
        .collect();
    SnapshotImage::new(expected.width, expected.height, pixels)
}

/// Filesystem policy for baseline and failure artifacts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotPaths {
    pub baselines: PathBuf,
    pub artifacts: PathBuf,
}

impl SnapshotPaths {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            baselines: root.join("snapshots"),
            artifacts: root.join("snapshot-artifacts"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotMode {
    Verify,
    Update,
}

impl SnapshotMode {
    #[must_use]
    pub fn from_environment() -> Self {
        if std::env::var_os("VUI_UPDATE_SNAPSHOTS").is_some() {
            Self::Update
        } else {
            Self::Verify
        }
    }
}

/// Verifies against a baseline, or updates it when `VUI_UPDATE_SNAPSHOTS` is set.
///
/// # Errors
///
/// Returns an error for invalid names, missing baselines, unacceptable differences, or I/O failure.
pub fn verify_snapshot(
    name: &str,
    actual: &SnapshotImage,
    paths: &SnapshotPaths,
    tolerance: SnapshotTolerance,
) -> Result<SnapshotReport, SnapshotError> {
    verify_snapshot_with_mode(
        name,
        actual,
        paths,
        tolerance,
        SnapshotMode::from_environment(),
    )
}

/// Verifies or updates a baseline using an explicit mode.
///
/// # Errors
///
/// Returns an error for invalid names, missing baselines, unacceptable differences, or I/O failure.
pub fn verify_snapshot_with_mode(
    name: &str,
    actual: &SnapshotImage,
    paths: &SnapshotPaths,
    tolerance: SnapshotTolerance,
    mode: SnapshotMode,
) -> Result<SnapshotReport, SnapshotError> {
    validate_name(name)?;
    let baseline = paths.baselines.join(format!("{name}.png"));
    if mode == SnapshotMode::Update {
        actual.save(&baseline)?;
        return Ok(SnapshotReport {
            total_pixels: image_len(actual.width, actual.height)? / 4,
            ..SnapshotReport::default()
        });
    }
    if !baseline.exists() {
        return Err(SnapshotError::new(format!(
            "missing baseline {}; run with VUI_UPDATE_SNAPSHOTS=1",
            baseline.display()
        )));
    }
    let expected = SnapshotImage::load(&baseline)?;
    let report = compare(&expected, actual, tolerance.channel_delta)?;
    if report.accepted(tolerance) {
        return Ok(report);
    }
    fs::create_dir_all(&paths.artifacts).map_err(|error| {
        SnapshotError::new(format!("create {}: {error}", paths.artifacts.display()))
    })?;
    let expected_artifact = paths.artifacts.join(format!("{name}.expected.png"));
    let actual_artifact = paths.artifacts.join(format!("{name}.actual.png"));
    let difference_artifact = paths.artifacts.join(format!("{name}.difference.png"));
    expected.save(&expected_artifact)?;
    actual.save(&actual_artifact)?;
    difference_image(&expected, actual)?.save(&difference_artifact)?;
    Err(SnapshotError::new(format!(
        "snapshot {name} changed: {} of {} pixels ({:.3}%), maximum channel delta {}; artifacts in {}",
        report.changed_pixels,
        report.total_pixels,
        report.changed_ratio * 100.0,
        report.maximum_channel_delta,
        paths.artifacts.display()
    )))
}

fn image_len(width: u32, height: u32) -> Result<usize, SnapshotError> {
    usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| SnapshotError::new("snapshot image dimensions overflow"))
}

fn physical_size(config: &SnapshotConfig) -> Result<(u32, u32), SnapshotError> {
    if !config.scale_factor.is_finite() || config.scale_factor <= 0.0 {
        return Err(SnapshotError::new(
            "snapshot scale factor must be finite and positive",
        ));
    }
    let width = config.logical_size.width * config.scale_factor;
    let height = config.logical_size.height * config.scale_factor;
    if !width.is_finite()
        || !height.is_finite()
        || width < 1.0
        || height < 1.0
        || width > f64::from(u32::MAX)
        || height > f64::from(u32::MAX)
    {
        return Err(SnapshotError::new(
            "snapshot physical dimensions must fit positive u32 values",
        ));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok((width.round() as u32, height.round() as u32))
}

fn ensure_same_size(expected: &SnapshotImage, actual: &SnapshotImage) -> Result<(), SnapshotError> {
    if expected.width == actual.width && expected.height == actual.height {
        Ok(())
    } else {
        Err(SnapshotError::new(format!(
            "snapshot dimensions differ: expected {}x{}, actual {}x{}",
            expected.width, expected.height, actual.width, actual.height
        )))
    }
}

fn validate_name(name: &str) -> Result<(), SnapshotError> {
    if !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        Ok(())
    } else {
        Err(SnapshotError::new(
            "snapshot names may contain only ASCII letters, digits, '-' and '_'",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    fn image(pixel: [u8; 4]) -> SnapshotImage {
        SnapshotImage::new(2, 1, [pixel, pixel].concat()).expect("valid image")
    }

    fn temporary_root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "vui-snapshot-test-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn comparison_applies_channel_and_pixel_tolerances() {
        let expected = image([10, 20, 30, 255]);
        let actual =
            SnapshotImage::new(2, 1, vec![12, 18, 30, 255, 40, 20, 30, 255]).expect("valid image");
        let report = compare(&expected, &actual, 2).expect("matching dimensions");
        assert_eq!(report.changed_pixels, 1);
        assert_eq!(report.maximum_channel_delta, 30);
        assert!(report.accepted(SnapshotTolerance {
            channel_delta: 2,
            max_changed_pixels: 1,
            max_changed_ratio: 0.5,
        }));
    }

    #[test]
    fn difference_image_highlights_rgb_and_keeps_opaque_alpha() {
        let expected = image([10, 20, 30, 100]);
        let actual = image([15, 12, 30, 200]);
        let difference = difference_image(&expected, &actual).expect("matching dimensions");
        assert_eq!(&difference.pixels()[..4], &[5, 8, 0, 255]);
    }

    #[test]
    fn update_then_verify_round_trips_a_png_baseline() {
        let root = temporary_root();
        let paths = SnapshotPaths::new(&root);
        let actual = image([10, 20, 30, 255]);
        verify_snapshot_with_mode(
            "round_trip",
            &actual,
            &paths,
            SnapshotTolerance::EXACT,
            SnapshotMode::Update,
        )
        .expect("write baseline");
        let report = verify_snapshot_with_mode(
            "round_trip",
            &actual,
            &paths,
            SnapshotTolerance::EXACT,
            SnapshotMode::Verify,
        )
        .expect("verify baseline");
        assert_eq!(report.changed_pixels, 0);
        fs::remove_dir_all(root).expect("remove temporary snapshots");
    }

    #[test]
    fn mismatch_writes_expected_actual_and_difference_artifacts() {
        let root = temporary_root();
        let paths = SnapshotPaths::new(&root);
        verify_snapshot_with_mode(
            "failure",
            &image([0, 0, 0, 255]),
            &paths,
            SnapshotTolerance::EXACT,
            SnapshotMode::Update,
        )
        .expect("write baseline");
        let error = verify_snapshot_with_mode(
            "failure",
            &image([255, 0, 0, 255]),
            &paths,
            SnapshotTolerance::EXACT,
            SnapshotMode::Verify,
        )
        .expect_err("snapshot should differ");
        assert!(error.to_string().contains("2 of 2 pixels"));
        assert!(paths.artifacts.join("failure.expected.png").exists());
        assert!(paths.artifacts.join("failure.actual.png").exists());
        assert!(paths.artifacts.join("failure.difference.png").exists());
        fs::remove_dir_all(root).expect("remove temporary snapshots");
    }

    #[test]
    fn unsafe_snapshot_names_are_rejected() {
        let error = verify_snapshot_with_mode(
            "../escape",
            &image([0, 0, 0, 255]),
            &SnapshotPaths::new(temporary_root()),
            SnapshotTolerance::EXACT,
            SnapshotMode::Update,
        )
        .expect_err("path traversal must fail");
        assert!(error.to_string().contains("snapshot names"));
    }
}
