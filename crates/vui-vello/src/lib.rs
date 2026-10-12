//! Translation of VUI paint operations into Vello scenes.

use std::{
    collections::{HashMap, HashSet},
    sync::mpsc,
    time::Duration,
};
use vello::{
    AaConfig, RenderParams, Renderer, RendererOptions, Scene,
    peniko::{
        Blob, Color as VelloColor, Fill, ImageAlphaType, ImageData, ImageFormat,
        kurbo::{Affine, BezPath, RoundedRect, Stroke},
    },
    util::RenderContext,
    wgpu,
};
use vui_core::{
    BOOTSTRAP_ADVANCE_EM, Color, PaintList, PaintOp, PathCommand, Point, RasterImage, Rect, UiFrame,
};

/// Error produced while preparing or rendering an embedded Vello texture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextureRenderError(String);

impl std::fmt::Display for TextureRenderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for TextureRenderError {}

struct TextureTarget {
    texture: wgpu::Texture,
    width: u32,
    height: u32,
}

const BYTES_PER_PIXEL: u32 = 4;
const COPY_ROW_ALIGNMENT: u32 = 256;

/// A rendered compositor texture that remains owned by [`TextureRenderer`].
pub struct RenderedTexture<'a> {
    pub texture: &'a wgpu::Texture,
    pub width: u32,
    pub height: u32,
}

/// Renders VUI frames into a texture on a host-owned wgpu device and queue.
pub struct TextureRenderer {
    renderer: Renderer,
    scenes: SceneBuilder,
    target: Option<TextureTarget>,
}

impl TextureRenderer {
    /// Creates an embedded renderer on the compositor's existing device.
    ///
    /// # Errors
    ///
    /// Returns an error when Vello cannot initialize its compute pipelines.
    pub fn new(device: &wgpu::Device) -> Result<Self, TextureRenderError> {
        let renderer = Renderer::new(device, RendererOptions::default())
            .map_err(|error| TextureRenderError(format!("create Vello renderer: {error}")))?;
        Ok(Self {
            renderer,
            scenes: SceneBuilder::new(),
            target: None,
        })
    }

    /// Renders one UI frame into a transparent, sampleable RGBA8 texture.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid dimensions or a Vello rendering failure.
    pub fn render<'a>(
        &'a mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &UiFrame,
        scale: f64,
    ) -> Result<RenderedTexture<'a>, TextureRenderError> {
        let (width, height) = physical_size(frame.viewport.width, frame.viewport.height, scale)?;
        let recreate = self
            .target
            .as_ref()
            .is_none_or(|target| target.width != width || target.height != height);
        if recreate {
            self.target = Some(TextureTarget {
                texture: device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("vui embedded render target"),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::STORAGE_BINDING
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                }),
                width,
                height,
            });
        }
        let Some(target) = self.target.as_ref() else {
            return Err(TextureRenderError(
                "embedded render target was not created".to_owned(),
            ));
        };
        render_frame_to_texture(
            &mut self.renderer,
            &mut self.scenes,
            device,
            queue,
            frame,
            scale,
            &target.texture,
            target.width,
            target.height,
        )?;
        Ok(RenderedTexture {
            texture: &target.texture,
            width,
            height,
        })
    }

    /// Renders a UI frame directly into a caller-owned RGBA8 storage texture.
    ///
    /// This is used by compositors that import a shared DMA-BUF into wgpu: Vello
    /// writes the allocation in place, so the compositor can sample it without
    /// a GPU-to-CPU readback or a pixel upload.
    ///
    /// # Errors
    ///
    /// Returns an error when the supplied dimensions do not match the frame's
    /// physical viewport or Vello cannot render the frame.
    #[allow(clippy::too_many_arguments)]
    pub fn render_into(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &UiFrame,
        scale: f64,
        texture: &wgpu::Texture,
        width: u32,
        height: u32,
    ) -> Result<(), TextureRenderError> {
        let expected = physical_size(frame.viewport.width, frame.viewport.height, scale)?;
        if expected != (width, height) {
            return Err(TextureRenderError(format!(
                "VUI target is {width}x{height}, expected {}x{}",
                expected.0, expected.1
            )));
        }
        render_frame_to_texture(
            &mut self.renderer,
            &mut self.scenes,
            device,
            queue,
            frame,
            scale,
            texture,
            width,
            height,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn render_frame_to_texture(
    renderer: &mut Renderer,
    scenes: &mut SceneBuilder,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    frame: &UiFrame,
    scale: f64,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Result<(), TextureRenderError> {
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let scene = scenes.build(&frame.paint, scale);
    renderer
        .render_to_texture(
            device,
            queue,
            scene,
            &view,
            &RenderParams {
                base_color: VelloColor::TRANSPARENT,
                width,
                height,
                antialiasing_method: AaConfig::Area,
            },
        )
        .map_err(|error| TextureRenderError(format!("render VUI texture: {error}")))?;
    Ok(())
}

/// An owned, tightly packed premultiplied RGBA8 frame.
pub struct RenderedPixels {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Renders VUI through a headless Vulkan/wgpu device and reads changed frames
/// back for hosts whose compositor renderer does not expose a wgpu device.
///
/// This is intended as a compatibility bridge. Hosts with a wgpu renderer
/// should use [`TextureRenderer`] to keep composition entirely on the GPU.
pub struct ReadbackRenderer {
    context: RenderContext,
    device_id: usize,
    renderer: TextureRenderer,
}

impl ReadbackRenderer {
    /// Creates a headless Vello renderer using the selected wgpu adapter.
    ///
    /// # Errors
    ///
    /// Returns an error when no compatible adapter exists or Vello cannot
    /// initialize on it.
    pub fn new() -> Result<Self, TextureRenderError> {
        let mut context = RenderContext::new();
        let device_id = pollster::block_on(context.device(None)).ok_or_else(|| {
            TextureRenderError("no compatible Vello/wgpu device available".to_owned())
        })?;
        let renderer = TextureRenderer::new(&context.devices[device_id].device)?;
        Ok(Self {
            context,
            device_id,
            renderer,
        })
    }

    /// Renders and returns one tightly packed premultiplied RGBA8 frame.
    ///
    /// # Errors
    ///
    /// Returns an error for rendering, submission, mapping, or size failures.
    pub fn render(
        &mut self,
        frame: &UiFrame,
        scale: f64,
    ) -> Result<RenderedPixels, TextureRenderError> {
        let device = &self.context.devices[self.device_id].device;
        let queue = &self.context.devices[self.device_id].queue;
        let rendered = self.renderer.render(device, queue, frame, scale)?;
        let pixels = read_texture(
            device,
            queue,
            rendered.texture,
            rendered.width,
            rendered.height,
        )?;
        Ok(RenderedPixels {
            pixels,
            width: rendered.width,
            height: rendered.height,
        })
    }
}

fn read_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, TextureRenderError> {
    let unpadded_row = width
        .checked_mul(BYTES_PER_PIXEL)
        .ok_or_else(|| TextureRenderError("VUI readback row size overflow".to_owned()))?;
    let padded_row = unpadded_row.div_ceil(COPY_ROW_ALIGNMENT) * COPY_ROW_ALIGNMENT;
    let buffer_size = u64::from(padded_row)
        .checked_mul(u64::from(height))
        .ok_or_else(|| TextureRenderError("VUI readback buffer size overflow".to_owned()))?;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("vui embedded readback"),
        size: buffer_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("vui embedded readback copy"),
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
            timeout: Some(Duration::from_secs(5)),
        })
        .map_err(|error| TextureRenderError(format!("wait for VUI readback: {error}")))?;
    receiver
        .recv_timeout(Duration::from_secs(5))
        .map_err(|error| TextureRenderError(format!("receive VUI readback: {error}")))?
        .map_err(|error| TextureRenderError(format!("map VUI readback: {error}")))?;

    let mapped = readback
        .get_mapped_range(..)
        .map_err(|error| TextureRenderError(format!("read mapped VUI frame: {error}")))?;
    let row_len = usize::try_from(unpadded_row)
        .map_err(|_| TextureRenderError("VUI readback row is too large".to_owned()))?;
    let padded_len = usize::try_from(padded_row)
        .map_err(|_| TextureRenderError("padded VUI row is too large".to_owned()))?;
    let capacity = row_len
        .checked_mul(
            usize::try_from(height)
                .map_err(|_| TextureRenderError("VUI readback height is too large".to_owned()))?,
        )
        .ok_or_else(|| TextureRenderError("VUI readback image size overflow".to_owned()))?;
    let mut pixels = Vec::with_capacity(capacity);
    for row in mapped.chunks_exact(padded_len) {
        pixels.extend_from_slice(&row[..row_len]);
    }
    drop(mapped);
    readback.unmap();
    Ok(pixels)
}

fn physical_size(
    logical_width: f64,
    logical_height: f64,
    scale: f64,
) -> Result<(u32, u32), TextureRenderError> {
    if !logical_width.is_finite()
        || !logical_height.is_finite()
        || !scale.is_finite()
        || logical_width <= 0.0
        || logical_height <= 0.0
        || scale <= 0.0
    {
        return Err(TextureRenderError(
            "embedded render size and scale must be finite and positive".to_owned(),
        ));
    }
    let width = (logical_width * scale).ceil();
    let height = (logical_height * scale).ceil();
    if width > f64::from(u32::MAX) || height > f64::from(u32::MAX) {
        return Err(TextureRenderError(
            "embedded render dimensions exceed wgpu limits".to_owned(),
        ));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok((width as u32, height as u32))
}

#[derive(Default)]
pub struct SceneBuilder {
    scene: Scene,
    images: HashMap<u64, ImageData>,
}

impl SceneBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn build(&mut self, paint: &PaintList, scale: f64) -> &Scene {
        self.scene.reset();
        let transform = Affine::scale(scale);
        let mut used_images = HashSet::new();
        for operation in paint.operations() {
            match operation {
                PaintOp::PushClip { rect, radius } => {
                    let shape = rounded_rect(*rect, *radius);
                    self.scene.push_clip_layer(Fill::NonZero, transform, &shape);
                }
                PaintOp::PopClip => self.scene.pop_layer(),
                PaintOp::FillRect {
                    rect,
                    color,
                    radius,
                } => {
                    let shape = rounded_rect(*rect, *radius);
                    self.scene.fill(
                        Fill::NonZero,
                        transform,
                        to_vello_color(*color),
                        None,
                        &shape,
                    );
                }
                PaintOp::StrokeRect {
                    rect,
                    color,
                    width,
                    radius,
                } => {
                    let shape = rounded_rect(*rect, *radius);
                    self.scene.stroke(
                        &Stroke::new(*width),
                        transform,
                        to_vello_color(*color),
                        None,
                        &shape,
                    );
                }
                PaintOp::FillPath { commands, color } => {
                    fill_path(&mut self.scene, transform, commands, *color);
                }
                PaintOp::StrokePath {
                    commands,
                    color,
                    width,
                } => {
                    stroke_path(&mut self.scene, transform, commands, *color, *width);
                }
                PaintOp::Text {
                    origin,
                    text,
                    color,
                    size,
                } => draw_bootstrap_text(&mut self.scene, transform, *origin, text, *color, *size),
                PaintOp::GlyphRun {
                    origin,
                    font,
                    font_size,
                    normalized_coords,
                    glyphs,
                    color,
                } => {
                    self.scene
                        .draw_glyphs(font)
                        .font_size(*font_size)
                        .brush(to_vello_color(*color))
                        .normalized_coords(normalized_coords)
                        .transform(transform * Affine::translate((origin.x, origin.y)))
                        .draw(
                            Fill::NonZero,
                            glyphs.iter().map(|glyph| vello::Glyph {
                                id: glyph.id,
                                x: glyph.x,
                                y: glyph.y,
                            }),
                        );
                }
                PaintOp::Image { image, rect } => {
                    used_images.insert(image.id());
                    self.draw_raster(image, *rect, transform);
                }
            }
        }
        self.images.retain(|id, _| used_images.contains(id));
        &self.scene
    }

    fn draw_raster(&mut self, image: &RasterImage, rect: Rect, transform: Affine) {
        let data = self.images.entry(image.id()).or_insert_with(|| ImageData {
            data: Blob::from(image.pixels().to_vec()),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::Alpha,
            width: image.width(),
            height: image.height(),
        });
        let image_transform = Affine::translate((rect.x, rect.y))
            * Affine::scale_non_uniform(
                rect.width / f64::from(image.width()),
                rect.height / f64::from(image.height()),
            );
        self.scene.draw_image(&*data, transform * image_transform);
    }
}

fn rounded_rect(rect: Rect, radius: f64) -> RoundedRect {
    RoundedRect::new(
        rect.x,
        rect.y,
        rect.x + rect.width,
        rect.y + rect.height,
        radius,
    )
}

fn fill_path(scene: &mut Scene, transform: Affine, commands: &[PathCommand], color: Color) {
    let path = to_bez_path(commands);
    scene.fill(Fill::NonZero, transform, to_vello_color(color), None, &path);
}

fn stroke_path(
    scene: &mut Scene,
    transform: Affine,
    commands: &[PathCommand],
    color: Color,
    width: f64,
) {
    let path = to_bez_path(commands);
    scene.stroke(
        &Stroke::new(width),
        transform,
        to_vello_color(color),
        None,
        &path,
    );
}

fn to_bez_path(commands: &[PathCommand]) -> BezPath {
    let mut path = BezPath::new();
    for command in commands {
        match *command {
            PathCommand::MoveTo(point) => path.move_to((point.x, point.y)),
            PathCommand::LineTo(point) => path.line_to((point.x, point.y)),
            PathCommand::QuadTo(control, point) => {
                path.quad_to((control.x, control.y), (point.x, point.y));
            }
            PathCommand::CurveTo(first, second, point) => {
                path.curve_to((first.x, first.y), (second.x, second.y), (point.x, point.y));
            }
            PathCommand::Close => path.close_path(),
        }
    }
    path
}

fn to_vello_color(color: Color) -> VelloColor {
    VelloColor::from_rgba8(color.r, color.g, color.b, color.a)
}

// A deliberately tiny bootstrap font keeps the first vertical slice
// deterministic and asset-free. It is replaced by shaped glyph runs in the
// text milestone; importantly, that change stays behind the same PaintOp.
fn draw_bootstrap_text(
    scene: &mut Scene,
    transform: Affine,
    origin: Point,
    text: &str,
    color: Color,
    size: f32,
) {
    let pixel = f64::from(size) / 7.0;
    let advance = f64::from(size) * BOOTSTRAP_ADVANCE_EM;
    let mut character_x = origin.x;
    let mut line_y = origin.y;
    for character in text.chars() {
        if character == '\n' {
            character_x = origin.x;
            line_y += f64::from(size) * 1.35;
            continue;
        }
        let bitmap = glyph(character.to_ascii_uppercase());
        let mut row_y = line_y - f64::from(size);
        for bits in bitmap {
            for column in 0..5 {
                if bits & (1 << (4 - column)) == 0 {
                    continue;
                }
                let x = character_x + f64::from(column) * pixel;
                let rect = RoundedRect::new(x, row_y, x + pixel, row_y + pixel, pixel * 0.12);
                scene.fill(Fill::NonZero, transform, to_vello_color(color), None, &rect);
            }
            row_y += pixel;
        }
        character_x += advance;
    }
}

fn glyph(character: char) -> [u8; 7] {
    match character {
        'A' => [0x0e, 0x11, 0x11, 0x1f, 0x11, 0x11, 0x11],
        'B' => [0x1e, 0x11, 0x11, 0x1e, 0x11, 0x11, 0x1e],
        'C' => [0x0f, 0x10, 0x10, 0x10, 0x10, 0x10, 0x0f],
        'D' => [0x1e, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1e],
        'E' => [0x1f, 0x10, 0x10, 0x1e, 0x10, 0x10, 0x1f],
        'F' => [0x1f, 0x10, 0x10, 0x1e, 0x10, 0x10, 0x10],
        'G' => [0x0f, 0x10, 0x10, 0x13, 0x11, 0x11, 0x0f],
        'H' => [0x11, 0x11, 0x11, 0x1f, 0x11, 0x11, 0x11],
        'I' => [0x1f, 0x04, 0x04, 0x04, 0x04, 0x04, 0x1f],
        'J' => [0x01, 0x01, 0x01, 0x01, 0x11, 0x11, 0x0e],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1f],
        'M' => [0x11, 0x1b, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11],
        'O' => [0x0e, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0e],
        'P' => [0x1e, 0x11, 0x11, 0x1e, 0x10, 0x10, 0x10],
        'Q' => [0x0e, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0d],
        'R' => [0x1e, 0x11, 0x11, 0x1e, 0x14, 0x12, 0x11],
        'S' => [0x0f, 0x10, 0x10, 0x0e, 0x01, 0x01, 0x1e],
        'T' => [0x1f, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0e],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0a, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0a],
        'X' => [0x11, 0x11, 0x0a, 0x04, 0x0a, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x0a, 0x04, 0x04, 0x04, 0x04],
        'Z' => [0x1f, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1f],
        '0' => [0x0e, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0e],
        '1' => [0x04, 0x0c, 0x04, 0x04, 0x04, 0x04, 0x0e],
        '2' => [0x0e, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1f],
        '3' => [0x1e, 0x01, 0x01, 0x0e, 0x01, 0x01, 0x1e],
        '4' => [0x02, 0x06, 0x0a, 0x12, 0x1f, 0x02, 0x02],
        '5' => [0x1f, 0x10, 0x10, 0x1e, 0x01, 0x01, 0x1e],
        '6' => [0x0e, 0x10, 0x10, 0x1e, 0x11, 0x11, 0x0e],
        '7' => [0x1f, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0e, 0x11, 0x11, 0x0e, 0x11, 0x11, 0x0e],
        '9' => [0x0e, 0x11, 0x11, 0x0f, 0x01, 0x01, 0x0e],
        '!' => [0x04, 0x04, 0x04, 0x04, 0x04, 0x00, 0x04],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04],
        '-' => [0x00, 0x00, 0x00, 0x1f, 0x00, 0x00, 0x00],
        ' ' => [0; 7],
        _ => [0x0e, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vui_core::{PaintList, PaintOp, RasterImage, Rect};

    #[test]
    fn translates_basic_scene() {
        let mut paint = PaintList::default();
        paint.push(PaintOp::FillRect {
            rect: Rect::new(0.0, 0.0, 20.0, 20.0),
            color: Color::WHITE,
            radius: 4.0,
        });
        paint.push(PaintOp::StrokePath {
            commands: vec![
                PathCommand::MoveTo(Point::new(2.0, 2.0)),
                PathCommand::LineTo(Point::new(18.0, 18.0)),
            ],
            color: Color::WHITE,
            width: 2.0,
        });
        paint.push(PaintOp::Image {
            image: RasterImage::new(1, 1, vec![255, 0, 0, 255]).expect("valid pixel"),
            rect: Rect::new(4.0, 4.0, 8.0, 8.0),
        });
        let mut builder = SceneBuilder::new();
        let _scene = builder.build(&paint, 1.0);
    }

    #[test]
    fn embedded_physical_size_scales_and_rejects_invalid_inputs() {
        assert_eq!(physical_size(320.0, 200.0, 1.5), Ok((480, 300)));
        assert!(physical_size(0.0, 200.0, 1.0).is_err());
        assert!(physical_size(320.0, 200.0, f64::NAN).is_err());
    }

    #[test]
    fn readback_renderer_returns_tightly_packed_rgba() {
        let Ok(mut renderer) = ReadbackRenderer::new() else {
            return;
        };
        let mut paint = PaintList::default();
        paint.push(PaintOp::FillRect {
            rect: Rect::new(0.0, 0.0, 8.0, 6.0),
            color: Color::WHITE,
            radius: 0.0,
        });
        let frame = UiFrame {
            viewport: vui_core::Size::new(8.0, 6.0),
            damage: vec![Rect::new(0.0, 0.0, 8.0, 6.0)],
            paint,
            semantics: Vec::new(),
        };

        let image = renderer.render(&frame, 1.0).expect("headless readback");
        assert_eq!((image.width, image.height), (8, 6));
        assert_eq!(image.pixels.len(), 8 * 6 * 4);
        assert!(image.pixels.chunks_exact(4).all(|pixel| pixel[3] == 255));
    }
}
