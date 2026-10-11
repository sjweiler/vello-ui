//! Translation of VUI paint operations into Vello scenes.

use std::collections::{HashMap, HashSet};
use vello::{
    Scene,
    peniko::{
        Blob, Color as VelloColor, Fill, ImageAlphaType, ImageData, ImageFormat,
        kurbo::{Affine, BezPath, RoundedRect, Stroke},
    },
};
use vui_core::{
    BOOTSTRAP_ADVANCE_EM, Color, PaintList, PaintOp, PathCommand, Point, RasterImage, Rect,
};

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
}
