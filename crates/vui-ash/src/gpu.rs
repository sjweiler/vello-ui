#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::too_many_lines,
    reason = "Vulkan vertices and Lyon use f32 coordinates"
)]

use std::{collections::HashMap, ops::Range, sync::Arc};

use bytemuck::{Pod, Zeroable};
use lyon_tessellation::{
    FillOptions, FillTessellator, FillVertex, StrokeOptions, StrokeTessellator, StrokeVertex,
    VertexBuffers, geometry_builder::BuffersBuilder, math::point, path::Path,
};
use skrifa::{
    FontRef, GlyphId, MetadataProvider,
    instance::{LocationRef, NormalizedCoord, Size},
    outline::{DrawSettings, OutlinePen},
};
use vui_core::{
    BOOTSTRAP_ADVANCE_EM, Color, PaintList, PaintOp, PathCommand, Point, RasterImage, Rect, UiFrame,
};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct Vertex {
    pub position: [f32; 2],
    pub color: [f32; 4],
    pub uv: [f32; 2],
    pub textured: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct BootstrapInstance {
    pub translation: [f32; 2],
    pub color: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CommandKind {
    Draw,
    Bootstrap,
    ClipPush,
    ClipPop,
}

#[derive(Clone, Debug)]
pub(crate) struct DrawCommand {
    pub indices: Range<u32>,
    pub instances: Range<u32>,
    pub kind: CommandKind,
    pub stencil_reference: u32,
}

/// Tessellated paint data and a per-frame image atlas ready for Vulkan upload.
#[derive(Clone)]
pub struct GpuScene {
    pub(crate) vertices: Arc<[Vertex]>,
    pub(crate) indices: Arc<[u32]>,
    pub(crate) bootstrap_instances: Arc<[BootstrapInstance]>,
    pub(crate) commands: Arc<[DrawCommand]>,
    pub(crate) atlas: Arc<[u8]>,
    pub(crate) atlas_width: u32,
    pub(crate) atlas_height: u32,
    pub(crate) geometry_revision: u64,
    pub(crate) atlas_revision: u64,
    pub(crate) atlas_base_revision: u64,
    pub(crate) atlas_dirty: Arc<[AtlasDirtyRegion]>,
    pub(crate) atlas_rebuilt: bool,
    atlas_occupancy_bytes: usize,
    atlas_evictions: u64,
    atlas_rebuilds: u64,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuSceneStats {
    pub vertices: usize,
    pub indices: usize,
    pub bootstrap_instances: usize,
    pub draw_commands: usize,
    pub atlas_bytes: usize,
    pub atlas_dirty_bytes: usize,
    pub atlas_occupancy_bytes: usize,
    pub atlas_evictions: u64,
    pub atlas_rebuilds: u64,
}

impl GpuScene {
    #[must_use]
    pub fn stats(&self) -> GpuSceneStats {
        GpuSceneStats {
            vertices: self.vertices.len(),
            indices: self.indices.len(),
            bootstrap_instances: self.bootstrap_instances.len(),
            draw_commands: self.commands.len(),
            atlas_bytes: self.atlas.len(),
            atlas_dirty_bytes: self
                .atlas_dirty
                .iter()
                .map(|dirty| dirty.pixels.len())
                .sum(),
            atlas_occupancy_bytes: self.atlas_occupancy_bytes,
            atlas_evictions: self.atlas_evictions,
            atlas_rebuilds: self.atlas_rebuilds,
        }
    }
}

#[derive(Default)]
pub struct GpuSceneBuilder {
    cached_scene: Option<CachedScene>,
    atlas: PersistentAtlas,
    glyph_cache: HashMap<GlyphKey, VertexBuffers<[f32; 2], u32>>,
    next_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct GlyphKey {
    font_id: u64,
    font_index: u32,
    glyph_id: u32,
    size_bits: u32,
    normalized_coords: Vec<i16>,
}

struct CachedScene {
    paint: PaintList,
    scale_bits: u64,
    scene: GpuScene,
}

#[derive(Clone, Copy)]
struct AtlasEntry {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[derive(Clone)]
pub(crate) struct AtlasDirtyRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<[u8]>,
}

impl GpuSceneBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self {
            cached_scene: None,
            atlas: PersistentAtlas::new(),
            glyph_cache: HashMap::new(),
            next_revision: 0,
        }
    }

    /// Tessellates a VUI frame into triangles and a single image atlas.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid dimensions, excessive clip nesting,
    /// tessellation failures, or images too large for the frame atlas.
    pub fn build(&mut self, frame: &UiFrame, scale: f64) -> Result<GpuScene, String> {
        let (width, height) = physical_size(frame, scale)?;
        let scale_bits = scale.to_bits();
        if let Some(cached) = &self.cached_scene
            && cached.scale_bits == scale_bits
            && cached.scene.width == width
            && cached.scene.height == height
            && cached.paint == frame.paint
        {
            return Ok(cached.scene.clone());
        }
        let scale = scale as f32;
        let atlas = self.atlas.update(frame)?;
        let atlas_pixels = atlas.pixels;
        let atlas_width = atlas.width;
        let atlas_height = atlas.height;
        let atlas_entries = atlas.entries;
        let mut output = SceneOutput {
            vertices: Vec::new(),
            indices: Vec::new(),
            commands: Vec::new(),
            bootstrap_instances: Vec::new(),
            bootstrap_cells: HashMap::new(),
            scale,
            width: width as f32,
            height: height as f32,
        };
        let mut clips: Vec<Path> = Vec::new();

        for operation in frame.paint.operations() {
            match operation {
                PaintOp::PushClip { rect, radius } => {
                    if clips.len() == u8::MAX as usize {
                        return Err("Vulkan stencil clip nesting exceeds 255".to_owned());
                    }
                    let path = rounded_rect(*rect, *radius);
                    output.fill(
                        &path,
                        Color::WHITE,
                        CommandKind::ClipPush,
                        clips.len() as u32,
                    )?;
                    clips.push(path);
                }
                PaintOp::PopClip => {
                    if let Some(path) = clips.pop() {
                        output.fill(
                            &path,
                            Color::WHITE,
                            CommandKind::ClipPop,
                            clips.len() as u32 + 1,
                        )?;
                    }
                }
                PaintOp::FillRect {
                    rect,
                    color,
                    radius,
                } => {
                    output.fill(
                        &rounded_rect(*rect, *radius),
                        *color,
                        CommandKind::Draw,
                        clips.len() as u32,
                    )?;
                }
                PaintOp::StrokeRect {
                    rect,
                    color,
                    width,
                    radius,
                } => {
                    output.stroke(
                        &rounded_rect(*rect, *radius),
                        *color,
                        *width as f32,
                        clips.len() as u32,
                    )?;
                }
                PaintOp::FillPath { commands, color } => {
                    output.fill(
                        &path(commands),
                        *color,
                        CommandKind::Draw,
                        clips.len() as u32,
                    )?;
                }
                PaintOp::StrokePath {
                    commands,
                    color,
                    width,
                } => {
                    output.stroke(&path(commands), *color, *width as f32, clips.len() as u32)?;
                }
                PaintOp::Text {
                    origin,
                    text,
                    color,
                    size,
                } => {
                    output.bootstrap_text(*origin, text, *color, *size, clips.len() as u32)?;
                }
                PaintOp::GlyphRun {
                    origin,
                    font,
                    font_size,
                    normalized_coords,
                    glyphs,
                    color,
                } => {
                    let Ok(font_ref) = FontRef::from_index(font.data.as_ref(), font.index) else {
                        continue;
                    };
                    let coords: Vec<_> = normalized_coords
                        .iter()
                        .copied()
                        .map(NormalizedCoord::from_bits)
                        .collect();
                    let outlines = font_ref.outline_glyphs();
                    for glyph in glyphs {
                        let key = GlyphKey {
                            font_id: font.data.id(),
                            font_index: font.index,
                            glyph_id: glyph.id,
                            size_bits: font_size.to_bits(),
                            normalized_coords: normalized_coords.clone(),
                        };
                        if !self.glyph_cache.contains_key(&key) {
                            let Some(outline) = outlines.get(GlyphId::new(glyph.id)) else {
                                continue;
                            };
                            let mut pen = LyonPen::new(0.0, 0.0);
                            if outline
                                .draw(
                                    DrawSettings::unhinted(
                                        Size::new(*font_size),
                                        LocationRef::new(&coords),
                                    ),
                                    &mut pen,
                                )
                                .is_err()
                            {
                                continue;
                            }
                            if self.glyph_cache.len() >= 4096 {
                                self.glyph_cache.clear();
                            }
                            self.glyph_cache
                                .insert(key.clone(), fill_geometry(&pen.finish())?);
                        }
                        output.append_translated(
                            &self.glyph_cache[&key],
                            [origin.x as f32 + glyph.x, origin.y as f32 + glyph.y],
                            *color,
                            clips.len() as u32,
                        );
                    }
                }
                PaintOp::Image { image, rect } => {
                    if let Some(entry) = atlas_entries.get(&image.id()) {
                        output.image(*rect, *entry, atlas_width, atlas_height, clips.len() as u32);
                    }
                }
            }
        }

        let scene = GpuScene {
            vertices: output.vertices.into(),
            indices: output.indices.into(),
            bootstrap_instances: output.bootstrap_instances.into(),
            commands: output.commands.into(),
            atlas: atlas_pixels,
            atlas_width,
            atlas_height,
            geometry_revision: self.take_revision(),
            atlas_revision: atlas.revision,
            atlas_base_revision: atlas.base_revision,
            atlas_dirty: atlas.dirty,
            atlas_rebuilt: atlas.rebuilt,
            atlas_occupancy_bytes: atlas.occupancy_bytes,
            atlas_evictions: atlas.evictions,
            atlas_rebuilds: atlas.rebuilds,
            width,
            height,
        };
        self.cached_scene = Some(CachedScene {
            paint: frame.paint.clone(),
            scale_bits,
            scene: scene.clone(),
        });
        Ok(scene)
    }

    fn take_revision(&mut self) -> u64 {
        self.next_revision = self.next_revision.wrapping_add(1).max(1);
        self.next_revision
    }
}

struct SceneOutput {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    commands: Vec<DrawCommand>,
    bootstrap_instances: Vec<BootstrapInstance>,
    bootstrap_cells: HashMap<u32, Range<u32>>,
    scale: f32,
    width: f32,
    height: f32,
}

impl SceneOutput {
    fn fill(
        &mut self,
        path: &Path,
        color: Color,
        kind: CommandKind,
        stencil: u32,
    ) -> Result<(), String> {
        let geometry = fill_geometry(path)?;
        self.append(geometry, color, kind, stencil);
        Ok(())
    }

    fn stroke(
        &mut self,
        path: &Path,
        color: Color,
        width: f32,
        stencil: u32,
    ) -> Result<(), String> {
        let mut geometry: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
        StrokeTessellator::new()
            .tessellate_path(
                path,
                &StrokeOptions::default()
                    .with_line_width(width)
                    .with_tolerance(0.15),
                &mut BuffersBuilder::new(&mut geometry, |vertex: StrokeVertex<'_, '_>| {
                    vertex.position().to_array()
                }),
            )
            .map_err(|error| format!("stroke tessellation failed: {error:?}"))?;
        self.append(geometry, color, CommandKind::Draw, stencil);
        Ok(())
    }

    fn append(
        &mut self,
        geometry: VertexBuffers<[f32; 2], u32>,
        color: Color,
        kind: CommandKind,
        stencil: u32,
    ) {
        if geometry.indices.is_empty() {
            return;
        }
        let vertex_base = self.vertices.len() as u32;
        let index_start = self.indices.len() as u32;
        let color = premultiplied(color);
        let scale = self.scale;
        let width = self.width;
        let height = self.height;
        self.vertices
            .extend(geometry.vertices.into_iter().map(|position| Vertex {
                position: project(position[0] * scale, position[1] * scale, width, height),
                color,
                uv: [0.0, 0.0],
                textured: 0.0,
            }));
        self.indices.extend(
            geometry
                .indices
                .into_iter()
                .map(|index| index + vertex_base),
        );
        self.push_command(index_start, kind, stencil);
    }

    fn append_translated(
        &mut self,
        geometry: &VertexBuffers<[f32; 2], u32>,
        translation: [f32; 2],
        color: Color,
        stencil: u32,
    ) {
        if geometry.indices.is_empty() {
            return;
        }
        let vertex_base = self.vertices.len() as u32;
        let index_start = self.indices.len() as u32;
        let color = premultiplied(color);
        self.vertices
            .extend(geometry.vertices.iter().map(|position| {
                let x = (position[0] + translation[0]) * self.scale;
                let y = (position[1] + translation[1]) * self.scale;
                Vertex {
                    position: project(x, y, self.width, self.height),
                    color,
                    uv: [0.0, 0.0],
                    textured: 0.0,
                }
            }));
        self.indices
            .extend(geometry.indices.iter().map(|index| index + vertex_base));
        self.push_command(index_start, CommandKind::Draw, stencil);
    }

    fn image(
        &mut self,
        rect: Rect,
        atlas: AtlasEntry,
        atlas_width: u32,
        atlas_height: u32,
        stencil: u32,
    ) {
        let base = self.vertices.len() as u32;
        let x0 = rect.x as f32 * self.scale;
        let y0 = rect.y as f32 * self.scale;
        let x1 = (rect.x + rect.width) as f32 * self.scale;
        let y1 = (rect.y + rect.height) as f32 * self.scale;
        let u0 = atlas.x as f32 / atlas_width as f32;
        let v0 = atlas.y as f32 / atlas_height as f32;
        let u1 = (atlas.x + atlas.width) as f32 / atlas_width as f32;
        let v1 = (atlas.y + atlas.height) as f32 / atlas_height as f32;
        let white = [1.0; 4];
        self.vertices.extend([
            Vertex {
                position: project(x0, y0, self.width, self.height),
                color: white,
                uv: [u0, v0],
                textured: 1.0,
            },
            Vertex {
                position: project(x1, y0, self.width, self.height),
                color: white,
                uv: [u1, v0],
                textured: 1.0,
            },
            Vertex {
                position: project(x1, y1, self.width, self.height),
                color: white,
                uv: [u1, v1],
                textured: 1.0,
            },
            Vertex {
                position: project(x0, y1, self.width, self.height),
                color: white,
                uv: [u0, v1],
                textured: 1.0,
            },
        ]);
        let start = self.indices.len() as u32;
        self.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        self.push_command(start, CommandKind::Draw, stencil);
    }

    fn push_command(&mut self, start: u32, kind: CommandKind, stencil: u32) {
        let end = self.indices.len() as u32;
        if let Some(previous) = self.commands.last_mut()
            && previous.kind == kind
            && previous.stencil_reference == stencil
            && previous.indices.end == start
        {
            previous.indices.end = end;
            return;
        }
        self.commands.push(DrawCommand {
            indices: start..end,
            instances: 0..1,
            kind,
            stencil_reference: stencil,
        });
    }

    fn bootstrap_text(
        &mut self,
        origin: Point,
        text: &str,
        color: Color,
        size: f32,
        stencil: u32,
    ) -> Result<(), String> {
        let cell_indices = self.bootstrap_cell(size)?;
        let instance_start = self.bootstrap_instances.len() as u32;
        let advance = f64::from(size) * BOOTSTRAP_ADVANCE_EM;
        let pixel = f64::from(size) / 7.0;
        let color = premultiplied(color);
        let mut x = origin.x;
        let mut y = origin.y;
        for character in text.chars() {
            if character == '\n' {
                x = origin.x;
                y += f64::from(size) * 1.35;
                continue;
            }
            for (row, bits) in glyph(character.to_ascii_uppercase())
                .into_iter()
                .enumerate()
            {
                for column in 0..5 {
                    if bits & (1 << (4 - column)) != 0 {
                        self.bootstrap_instances.push(BootstrapInstance {
                            translation: project(
                                (x + f64::from(column) * pixel) as f32 * self.scale,
                                (y - f64::from(size) + row as f64 * pixel) as f32 * self.scale,
                                self.width,
                                self.height,
                            ),
                            color,
                        });
                    }
                }
            }
            x += advance;
        }
        let instance_end = self.bootstrap_instances.len() as u32;
        if instance_end > instance_start {
            if let Some(previous) = self.commands.last_mut()
                && previous.kind == CommandKind::Bootstrap
                && previous.stencil_reference == stencil
                && previous.indices == cell_indices
                && previous.instances.end == instance_start
            {
                previous.instances.end = instance_end;
            } else {
                self.commands.push(DrawCommand {
                    indices: cell_indices,
                    instances: instance_start..instance_end,
                    kind: CommandKind::Bootstrap,
                    stencil_reference: stencil,
                });
            }
        }
        Ok(())
    }

    fn bootstrap_cell(&mut self, size: f32) -> Result<Range<u32>, String> {
        let key = size.to_bits();
        if let Some(indices) = self.bootstrap_cells.get(&key) {
            return Ok(indices.clone());
        }
        let pixel = f64::from(size) / 7.0;
        let geometry = fill_geometry(&rounded_rect(
            Rect::new(0.0, 0.0, pixel, pixel),
            pixel * 0.12,
        ))?;
        let vertex_base = self.vertices.len() as u32;
        let index_start = self.indices.len() as u32;
        self.vertices
            .extend(geometry.vertices.into_iter().map(|position| Vertex {
                position: [
                    position[0] * self.scale * 2.0 / self.width,
                    position[1] * self.scale * -2.0 / self.height,
                ],
                color: [0.0; 4],
                uv: [0.0; 2],
                textured: 0.0,
            }));
        self.indices.extend(
            geometry
                .indices
                .into_iter()
                .map(|index| index + vertex_base),
        );
        let indices = index_start..self.indices.len() as u32;
        self.bootstrap_cells.insert(key, indices.clone());
        Ok(indices)
    }
}

fn fill_geometry(path: &Path) -> Result<VertexBuffers<[f32; 2], u32>, String> {
    let mut geometry = VertexBuffers::new();
    FillTessellator::new()
        .tessellate_path(
            path,
            &FillOptions::default().with_tolerance(0.15),
            &mut BuffersBuilder::new(&mut geometry, |vertex: FillVertex<'_>| {
                vertex.position().to_array()
            }),
        )
        .map_err(|error| format!("fill tessellation failed: {error:?}"))?;
    Ok(geometry)
}

fn premultiplied(color: Color) -> [f32; 4] {
    let alpha = f32::from(color.a) / 255.0;
    [
        f32::from(color.r) / 255.0 * alpha,
        f32::from(color.g) / 255.0 * alpha,
        f32::from(color.b) / 255.0 * alpha,
        alpha,
    ]
}

fn project(x: f32, y: f32, width: f32, height: f32) -> [f32; 2] {
    [x / width * 2.0 - 1.0, 1.0 - y / height * 2.0]
}

fn path(commands: &[PathCommand]) -> Path {
    let mut builder = Path::builder().with_svg();
    for command in commands {
        match *command {
            PathCommand::MoveTo(p) => {
                builder.move_to(point(p.x as f32, p.y as f32));
            }
            PathCommand::LineTo(p) => {
                builder.line_to(point(p.x as f32, p.y as f32));
            }
            PathCommand::QuadTo(c, p) => {
                builder.quadratic_bezier_to(
                    point(c.x as f32, c.y as f32),
                    point(p.x as f32, p.y as f32),
                );
            }
            PathCommand::CurveTo(a, b, p) => {
                builder.cubic_bezier_to(
                    point(a.x as f32, a.y as f32),
                    point(b.x as f32, b.y as f32),
                    point(p.x as f32, p.y as f32),
                );
            }
            PathCommand::Close => builder.close(),
        }
    }
    builder.build()
}

fn rounded_rect(rect: Rect, radius: f64) -> Path {
    let corner = radius.max(0.0).min(rect.width * 0.5).min(rect.height * 0.5) as f32;
    let left = rect.x as f32;
    let top = rect.y as f32;
    let width = rect.width as f32;
    let height = rect.height as f32;
    let mut builder = Path::builder().with_svg();
    builder.move_to(point(left + corner, top));
    builder.line_to(point(left + width - corner, top));
    builder.quadratic_bezier_to(point(left + width, top), point(left + width, top + corner));
    builder.line_to(point(left + width, top + height - corner));
    builder.quadratic_bezier_to(
        point(left + width, top + height),
        point(left + width - corner, top + height),
    );
    builder.line_to(point(left + corner, top + height));
    builder.quadratic_bezier_to(
        point(left, top + height),
        point(left, top + height - corner),
    );
    builder.line_to(point(left, top + corner));
    builder.quadratic_bezier_to(point(left, top), point(left + corner, top));
    builder.close();
    builder.build()
}

struct LyonPen {
    builder: lyon_tessellation::path::path::Builder,
    x: f32,
    y: f32,
}
impl LyonPen {
    fn new(x: f32, y: f32) -> Self {
        Self {
            builder: Path::builder(),
            x,
            y,
        }
    }
    fn finish(self) -> Path {
        self.builder.build()
    }
}
impl OutlinePen for LyonPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.builder.begin(point(self.x + x, self.y - y));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.builder.line_to(point(self.x + x, self.y - y));
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.builder.quadratic_bezier_to(
            point(self.x + cx, self.y - cy),
            point(self.x + x, self.y - y),
        );
    }
    fn curve_to(&mut self, ax: f32, ay: f32, bx: f32, by: f32, x: f32, y: f32) {
        self.builder.cubic_bezier_to(
            point(self.x + ax, self.y - ay),
            point(self.x + bx, self.y - by),
            point(self.x + x, self.y - y),
        );
    }
    fn close(&mut self) {
        self.builder.close();
    }
}

struct AtlasSnapshot {
    pixels: Arc<[u8]>,
    width: u32,
    height: u32,
    entries: HashMap<u64, AtlasEntry>,
    revision: u64,
    base_revision: u64,
    dirty: Arc<[AtlasDirtyRegion]>,
    rebuilt: bool,
    occupancy_bytes: usize,
    evictions: u64,
    rebuilds: u64,
}

struct StoredImage {
    image: RasterImage,
    entry: AtlasEntry,
    last_used: u64,
}

#[derive(Clone, Copy)]
struct FreeRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

struct PersistentAtlas {
    pixels: Vec<u8>,
    published_pixels: Arc<[u8]>,
    side: u32,
    max_side: u32,
    entries: HashMap<u64, StoredImage>,
    free: Vec<FreeRect>,
    epoch: u64,
    revision: u64,
    evictions: u64,
    rebuilds: u64,
    needs_full_upload: bool,
}

impl Default for PersistentAtlas {
    fn default() -> Self {
        Self::new()
    }
}

impl PersistentAtlas {
    fn new() -> Self {
        let budget_mb = std::env::var("VUI_VULKAN_ATLAS_MB")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(64)
            .clamp(1, 256);
        let pixels = budget_mb * 1024 * 1024 / 4;
        let mut max_side = 1_u32;
        while u64::from(max_side.saturating_mul(2)).pow(2) <= pixels && max_side < 8192 {
            max_side *= 2;
        }
        Self {
            pixels: vec![255; 4],
            published_pixels: vec![255; 4].into(),
            side: 1,
            max_side,
            entries: HashMap::new(),
            free: vec![FreeRect {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            }],
            epoch: 0,
            revision: 1,
            evictions: 0,
            rebuilds: 1,
            needs_full_upload: true,
        }
    }

    fn update(&mut self, frame: &UiFrame) -> Result<AtlasSnapshot, String> {
        let base_revision = self.revision;
        self.epoch = self.epoch.wrapping_add(1).max(1);
        let mut active = Vec::new();
        for operation in frame.paint.operations() {
            if let PaintOp::Image { image, .. } = operation
                && !active
                    .iter()
                    .any(|candidate: &RasterImage| candidate.id() == image.id())
            {
                active.push(image.clone());
            }
        }
        for image in &active {
            if let Some(stored) = self.entries.get_mut(&image.id()) {
                stored.last_used = self.epoch;
            }
        }
        let mut dirty = Vec::new();
        let mut rebuilt = self.needs_full_upload;
        if !active.is_empty() && self.side == 1 {
            let initial_side = 1024.min(self.max_side);
            if !self.rebuild(&active, initial_side) {
                return Err(format!(
                    "active images exceed the {} MiB Vulkan atlas budget",
                    u64::from(self.max_side) * u64::from(self.max_side) * 4 / 1024 / 1024
                ));
            }
            rebuilt = true;
        }
        for image in &active {
            if self.entries.contains_key(&image.id()) {
                continue;
            }
            if image.width() > self.max_side || image.height() > self.max_side {
                return Err(format!(
                    "image {}x{} exceeds the {}x{} Vulkan atlas budget",
                    image.width(),
                    image.height(),
                    self.max_side,
                    self.max_side
                ));
            }
            let active_ids: Vec<u64> = active.iter().map(RasterImage::id).collect();
            let mut slot = self.allocate(image.width(), image.height());
            while slot.is_none() {
                let victim = self
                    .entries
                    .iter()
                    .filter(|(id, _)| !active_ids.contains(id))
                    .min_by_key(|(_, stored)| stored.last_used)
                    .map(|(id, _)| *id);
                let Some(victim) = victim else {
                    break;
                };
                self.evict(victim);
                slot = self.allocate(image.width(), image.height());
            }
            if slot.is_none() {
                if self.rebuild(&active, self.side) {
                    rebuilt = true;
                } else {
                    let mut next_side = (self.side.saturating_mul(2))
                        .max(1024.min(self.max_side))
                        .min(self.max_side);
                    loop {
                        if self.rebuild(&active, next_side) {
                            rebuilt = true;
                            break;
                        }
                        if next_side == self.max_side {
                            break;
                        }
                        next_side = (next_side * 2).min(self.max_side);
                    }
                }
                if !self.entries.contains_key(&image.id()) {
                    return Err(format!(
                        "active images exceed the {} MiB Vulkan atlas budget",
                        u64::from(self.max_side) * u64::from(self.max_side) * 4 / 1024 / 1024
                    ));
                }
                break;
            }
            let entry = slot.expect("checked atlas slot");
            let pixels = premultiply(image);
            self.write(entry, &pixels);
            dirty.push(AtlasDirtyRegion {
                x: entry.x,
                y: entry.y,
                width: entry.width,
                height: entry.height,
                pixels: pixels.into(),
            });
            self.entries.insert(
                image.id(),
                StoredImage {
                    image: image.clone(),
                    entry,
                    last_used: self.epoch,
                },
            );
        }
        if rebuilt {
            dirty = self
                .entries
                .values()
                .map(|stored| AtlasDirtyRegion {
                    x: stored.entry.x,
                    y: stored.entry.y,
                    width: stored.entry.width,
                    height: stored.entry.height,
                    pixels: premultiply(&stored.image).into(),
                })
                .collect();
        }
        if rebuilt || !dirty.is_empty() {
            self.revision = self.revision.wrapping_add(1).max(1);
            self.published_pixels = self.pixels.clone().into();
        }
        self.needs_full_upload = false;
        let active_entries = active
            .iter()
            .filter_map(|image| {
                self.entries
                    .get(&image.id())
                    .map(|stored| (image.id(), stored.entry))
            })
            .collect();
        Ok(AtlasSnapshot {
            pixels: Arc::clone(&self.published_pixels),
            width: self.side,
            height: self.side,
            entries: active_entries,
            revision: self.revision,
            base_revision,
            dirty: dirty.into(),
            rebuilt,
            occupancy_bytes: self
                .entries
                .values()
                .map(|stored| stored.image.pixels().len())
                .sum(),
            evictions: self.evictions,
            rebuilds: self.rebuilds,
        })
    }

    fn allocate(&mut self, width: u32, height: u32) -> Option<AtlasEntry> {
        let index = self
            .free
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.width >= width && slot.height >= height)
            .min_by_key(|(_, slot)| u64::from(slot.width) * u64::from(slot.height))
            .map(|(index, _)| index)?;
        let slot = self.free.swap_remove(index);
        if slot.width > width {
            self.free.push(FreeRect {
                x: slot.x + width,
                y: slot.y,
                width: slot.width - width,
                height,
            });
        }
        if slot.height > height {
            self.free.push(FreeRect {
                x: slot.x,
                y: slot.y + height,
                width: slot.width,
                height: slot.height - height,
            });
        }
        Some(AtlasEntry {
            x: slot.x,
            y: slot.y,
            width,
            height,
        })
    }

    fn evict(&mut self, id: u64) {
        if let Some(stored) = self.entries.remove(&id) {
            self.free.push(FreeRect {
                x: stored.entry.x,
                y: stored.entry.y,
                width: stored.entry.width,
                height: stored.entry.height,
            });
            self.coalesce();
            self.evictions += 1;
        }
    }

    fn coalesce(&mut self) {
        loop {
            let mut merged = false;
            'outer: for left in 0..self.free.len() {
                for right in left + 1..self.free.len() {
                    let a = self.free[left];
                    let b = self.free[right];
                    let combined = if a.y == b.y && a.height == b.height && a.x + a.width == b.x {
                        Some(FreeRect {
                            width: a.width + b.width,
                            ..a
                        })
                    } else if a.y == b.y && a.height == b.height && b.x + b.width == a.x {
                        Some(FreeRect {
                            width: a.width + b.width,
                            x: b.x,
                            ..a
                        })
                    } else if a.x == b.x && a.width == b.width && a.y + a.height == b.y {
                        Some(FreeRect {
                            height: a.height + b.height,
                            ..a
                        })
                    } else if a.x == b.x && a.width == b.width && b.y + b.height == a.y {
                        Some(FreeRect {
                            height: a.height + b.height,
                            y: b.y,
                            ..a
                        })
                    } else {
                        None
                    };
                    if let Some(combined) = combined {
                        self.free[left] = combined;
                        self.free.swap_remove(right);
                        merged = true;
                        break 'outer;
                    }
                }
            }
            if !merged {
                break;
            }
        }
    }

    fn rebuild(&mut self, active: &[RasterImage], side: u32) -> bool {
        let mut free = vec![FreeRect {
            x: 0,
            y: 0,
            width: side,
            height: side,
        }];
        let mut entries = HashMap::new();
        for image in active {
            let Some(entry) = allocate_from(&mut free, image.width(), image.height()) else {
                return false;
            };
            entries.insert(
                image.id(),
                StoredImage {
                    image: image.clone(),
                    entry,
                    last_used: self.epoch,
                },
            );
        }
        self.side = side;
        self.pixels = vec![0; side as usize * side as usize * 4];
        self.free = free;
        self.entries = entries;
        let writes: Vec<_> = self
            .entries
            .values()
            .map(|stored| (stored.entry, premultiply(&stored.image)))
            .collect();
        for (entry, pixels) in writes {
            self.write(entry, &pixels);
        }
        self.rebuilds += 1;
        true
    }

    fn write(&mut self, entry: AtlasEntry, pixels: &[u8]) {
        for row in 0..entry.height {
            let width = entry.width as usize * 4;
            let src = row as usize * width;
            let dst = ((entry.y + row) * self.side * 4 + entry.x * 4) as usize;
            self.pixels[dst..dst + width].copy_from_slice(&pixels[src..src + width]);
        }
    }
}

fn allocate_from(free: &mut Vec<FreeRect>, width: u32, height: u32) -> Option<AtlasEntry> {
    let index = free
        .iter()
        .position(|slot| slot.width >= width && slot.height >= height)?;
    let slot = free.swap_remove(index);
    if slot.width > width {
        free.push(FreeRect {
            x: slot.x + width,
            y: slot.y,
            width: slot.width - width,
            height,
        });
    }
    if slot.height > height {
        free.push(FreeRect {
            x: slot.x,
            y: slot.y + height,
            width: slot.width,
            height: slot.height - height,
        });
    }
    Some(AtlasEntry {
        x: slot.x,
        y: slot.y,
        width,
        height,
    })
}

fn premultiply(image: &RasterImage) -> Vec<u8> {
    let mut pixels = Vec::with_capacity(image.pixels().len());
    for source in image.pixels().chunks_exact(4) {
        let alpha = u16::from(source[3]);
        pixels.extend_from_slice(&[
            ((u16::from(source[0]) * alpha + 127) / 255) as u8,
            ((u16::from(source[1]) * alpha + 127) / 255) as u8,
            ((u16::from(source[2]) * alpha + 127) / 255) as u8,
            source[3],
        ]);
    }
    pixels
}

fn physical_size(frame: &UiFrame, scale: f64) -> Result<(u32, u32), String> {
    let w = (frame.viewport.width * scale).ceil();
    let h = (frame.viewport.height * scale).ceil();
    if !w.is_finite()
        || !h.is_finite()
        || w <= 0.0
        || h <= 0.0
        || w > f64::from(u32::MAX)
        || h > f64::from(u32::MAX)
    {
        return Err("invalid GPU frame dimensions".to_owned());
    }
    Ok((w as u32, h as u32))
}

fn glyph(c: char) -> [u8; 7] {
    match c {
        'A' => [14, 17, 17, 31, 17, 17, 17],
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'C' => [15, 16, 16, 16, 16, 16, 15],
        'D' => [30, 17, 17, 17, 17, 17, 30],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'G' => [15, 16, 16, 19, 17, 17, 15],
        'H' => [17, 17, 17, 31, 17, 17, 17],
        'I' => [31, 4, 4, 4, 4, 4, 31],
        'J' => [1, 1, 1, 1, 17, 17, 14],
        'K' => [17, 18, 20, 24, 20, 18, 17],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'N' => [17, 25, 21, 19, 17, 17, 17],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'Q' => [14, 17, 17, 17, 21, 18, 13],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'W' => [17, 17, 17, 21, 21, 21, 10],
        'X' => [17, 17, 10, 4, 10, 17, 17],
        'Y' => [17, 17, 10, 4, 4, 4, 4],
        'Z' => [31, 1, 2, 4, 8, 16, 31],
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [30, 1, 1, 14, 1, 1, 30],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 16, 30, 1, 1, 30],
        '6' => [14, 16, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 1, 14],
        '!' => [4, 4, 4, 4, 4, 0, 4],
        '.' => [0, 0, 0, 0, 0, 0, 4],
        '-' => [0, 0, 0, 31, 0, 0, 0],
        ' ' => [0; 7],
        _ => [14, 17, 1, 2, 4, 0, 4],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vui_core::{PaintList, Size};

    fn image_frame(images: &[RasterImage]) -> UiFrame {
        let mut paint = PaintList::default();
        for (index, image) in images.iter().enumerate() {
            paint.push(PaintOp::Image {
                image: image.clone(),
                rect: Rect::new(index as f64, 0.0, 1.0, 1.0),
            });
        }
        UiFrame {
            viewport: Size::new(64.0, 64.0),
            damage: vec![],
            paint,
            semantics: vec![],
        }
    }

    #[test]
    fn tessellates_clips_and_shapes() {
        let mut paint = PaintList::default();
        paint.push(PaintOp::PushClip {
            rect: Rect::new(0.0, 0.0, 20.0, 20.0),
            radius: 3.0,
        });
        paint.push(PaintOp::FillRect {
            rect: Rect::new(1.0, 1.0, 10.0, 10.0),
            color: Color::WHITE,
            radius: 2.0,
        });
        paint.push(PaintOp::PopClip);
        let frame = UiFrame {
            viewport: Size::new(20.0, 20.0),
            damage: vec![],
            paint,
            semantics: vec![],
        };
        let mut builder = GpuSceneBuilder::new();
        let scene = builder.build(&frame, 2.0).unwrap();
        assert!(!scene.vertices.is_empty());
        assert_eq!(scene.commands.len(), 3);
        assert_eq!((scene.width, scene.height), (40, 40));

        let cached = builder.build(&frame, 2.0).unwrap();
        assert_eq!(scene.geometry_revision, cached.geometry_revision);
        assert_eq!(scene.atlas_revision, cached.atlas_revision);
        assert!(Arc::ptr_eq(&scene.vertices, &cached.vertices));

        let mut changed = frame.clone();
        changed.paint.push(PaintOp::FillRect {
            rect: Rect::new(2.0, 2.0, 3.0, 3.0),
            color: Color::BLACK,
            radius: 0.0,
        });
        let changed = builder.build(&changed, 2.0).unwrap();
        assert_ne!(scene.geometry_revision, changed.geometry_revision);
        assert_eq!(scene.atlas_revision, changed.atlas_revision);
        assert!(Arc::ptr_eq(&scene.atlas, &changed.atlas));
    }

    #[test]
    fn tessellates_paths_text_and_premultiplied_images() {
        let image = RasterImage::new(1, 1, vec![200, 100, 50, 128]).unwrap();
        let mut paint = PaintList::default();
        paint.push(PaintOp::FillPath {
            commands: vec![
                PathCommand::MoveTo(Point::new(1.0, 1.0)),
                PathCommand::LineTo(Point::new(12.0, 1.0)),
                PathCommand::LineTo(Point::new(6.0, 12.0)),
                PathCommand::Close,
            ],
            color: Color::WHITE,
        });
        paint.push(PaintOp::StrokePath {
            commands: vec![
                PathCommand::MoveTo(Point::new(1.0, 15.0)),
                PathCommand::LineTo(Point::new(18.0, 15.0)),
            ],
            color: Color::BLACK,
            width: 2.0,
        });
        paint.push(PaintOp::Text {
            origin: Point::new(1.0, 28.0),
            text: "GPU".to_owned(),
            color: Color::WHITE,
            size: 7.0,
        });
        paint.push(PaintOp::Image {
            image,
            rect: Rect::new(20.0, 1.0, 8.0, 8.0),
        });
        let frame = UiFrame {
            viewport: Size::new(32.0, 32.0),
            damage: vec![],
            paint,
            semantics: vec![],
        };
        let scene = GpuSceneBuilder::new().build(&frame, 1.0).unwrap();
        assert_eq!(
            scene.commands.len(),
            3,
            "bootstrap instances split otherwise adjacent vector draws"
        );
        assert!(!scene.bootstrap_instances.is_empty());
        assert!(scene.vertices.iter().any(|vertex| vertex.textured > 0.5));
        assert_eq!(&scene.atlas[..4], &[100, 50, 25, 128]);
    }

    #[test]
    fn atlas_keeps_slots_dirty_uploads_and_evicts_inactive_images() {
        let first = RasterImage::new(1, 1, vec![255, 0, 0, 255]).unwrap();
        let second = RasterImage::new(1, 1, vec![0, 255, 0, 255]).unwrap();
        let mut builder = GpuSceneBuilder::new();
        let one = builder
            .build(&image_frame(std::slice::from_ref(&first)), 1.0)
            .unwrap();
        let first_uv = one
            .vertices
            .iter()
            .find(|vertex| vertex.textured > 0.5)
            .unwrap()
            .uv;
        let two = builder
            .build(&image_frame(&[first.clone(), second.clone()]), 1.0)
            .unwrap();
        let retained_uv = two
            .vertices
            .iter()
            .find(|vertex| vertex.textured > 0.5)
            .unwrap()
            .uv;
        assert_eq!(
            first_uv.map(f32::to_bits),
            retained_uv.map(f32::to_bits),
            "retained image slot must remain stable"
        );
        assert!(!two.atlas_rebuilt);
        assert_eq!(two.stats().atlas_dirty_bytes, 4);

        let mut atlas = PersistentAtlas {
            pixels: vec![0; 16],
            published_pixels: vec![0; 16].into(),
            side: 2,
            max_side: 2,
            entries: HashMap::new(),
            free: vec![FreeRect {
                x: 0,
                y: 0,
                width: 2,
                height: 2,
            }],
            epoch: 0,
            revision: 0,
            evictions: 0,
            rebuilds: 0,
            needs_full_upload: false,
        };
        let large_a = RasterImage::new(2, 2, vec![255; 16]).unwrap();
        let large_b = RasterImage::new(2, 2, vec![127; 16]).unwrap();
        atlas.update(&image_frame(&[large_a])).unwrap();
        let replaced = atlas.update(&image_frame(&[large_b])).unwrap();
        assert_eq!(replaced.evictions, 1);
        assert_eq!(
            replaced
                .dirty
                .iter()
                .map(|region| region.pixels.len())
                .sum::<usize>(),
            16
        );
    }

    #[test]
    fn atlas_survives_growth_and_image_churn() {
        let mut atlas = PersistentAtlas {
            pixels: vec![0; 64],
            published_pixels: vec![0; 64].into(),
            side: 4,
            max_side: 8,
            entries: HashMap::new(),
            free: vec![FreeRect {
                x: 0,
                y: 0,
                width: 4,
                height: 4,
            }],
            epoch: 0,
            revision: 0,
            evictions: 0,
            rebuilds: 0,
            needs_full_upload: false,
        };
        let images: Vec<_> = (0_u8..24)
            .map(|value| RasterImage::new(4, 4, vec![value; 64]).unwrap())
            .collect();
        for step in 0..96 {
            let active: Vec<_> = (0..4)
                .map(|offset| images[(step + offset) % images.len()].clone())
                .collect();
            let snapshot = atlas.update(&image_frame(&active)).unwrap();
            let entries: Vec<_> = snapshot.entries.values().collect();
            assert_eq!(entries.len(), 4);
            for (index, left) in entries.iter().enumerate() {
                assert!(left.x + left.width <= snapshot.width);
                assert!(left.y + left.height <= snapshot.height);
                for right in &entries[index + 1..] {
                    let separate = left.x + left.width <= right.x
                        || right.x + right.width <= left.x
                        || left.y + left.height <= right.y
                        || right.y + right.height <= left.y;
                    assert!(separate, "active atlas entries overlap");
                }
            }
        }
        assert!(atlas.evictions > 0);
        assert!(atlas.rebuilds > 0);
        assert!(atlas.side <= atlas.max_side);
    }
}
