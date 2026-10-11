use crate::{Point, RasterImage, Rect, ShapedGlyph};
use linebender_resource_handle::FontData;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathCommand {
    MoveTo(Point),
    LineTo(Point),
    QuadTo(Point, Point),
    CurveTo(Point, Point, Point),
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    pub const WHITE: Self = Self::rgb(255, 255, 255);

    #[must_use]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    #[must_use]
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PaintOp {
    PushClip {
        rect: Rect,
        radius: f64,
    },
    PopClip,
    FillRect {
        rect: Rect,
        color: Color,
        radius: f64,
    },
    StrokeRect {
        rect: Rect,
        color: Color,
        width: f64,
        radius: f64,
    },
    FillPath {
        commands: Vec<PathCommand>,
        color: Color,
    },
    StrokePath {
        commands: Vec<PathCommand>,
        color: Color,
        width: f64,
    },
    Text {
        origin: Point,
        text: String,
        color: Color,
        size: f32,
    },
    GlyphRun {
        origin: Point,
        font: FontData,
        font_size: f32,
        normalized_coords: Vec<i16>,
        glyphs: Vec<ShapedGlyph>,
        color: Color,
    },
    Image {
        image: RasterImage,
        rect: Rect,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaintList {
    operations: Vec<PaintOp>,
}

impl PaintList {
    pub fn push(&mut self, operation: PaintOp) {
        self.operations.push(operation);
    }

    #[must_use]
    pub fn operations(&self) -> &[PaintOp] {
        &self.operations
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }
}
