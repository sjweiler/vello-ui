use crate::Size;
use linebender_resource_handle::FontData;

/// Horizontal advance of the deterministic bootstrap font, in em units.
pub const BOOTSTRAP_ADVANCE_EM: f64 = 6.0 / 7.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CaretPosition {
    pub index: usize,
    pub x: f64,
    pub y: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShapedGlyph {
    pub id: u32,
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShapedGlyphRun {
    pub font: FontData,
    pub font_size: f32,
    pub normalized_coords: Vec<i16>,
    pub glyphs: Vec<ShapedGlyph>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextLayout {
    pub size: Size,
    pub runs: Vec<ShapedGlyphRun>,
    pub fallback_text: Option<String>,
    pub carets: Vec<CaretPosition>,
}

pub trait TextSystem {
    fn layout(&mut self, text: &str, font_size: f32, max_width: Option<f32>) -> TextLayout;
}

#[derive(Default)]
pub(crate) struct BootstrapTextSystem;

impl TextSystem for BootstrapTextSystem {
    fn layout(&mut self, text: &str, font_size: f32, max_width: Option<f32>) -> TextLayout {
        let line_height = f64::from(font_size) * 1.35;
        let carets = bootstrap_carets(text, font_size, max_width);
        let width = carets.iter().map(|caret| caret.x).fold(0.0, f64::max);
        let height = carets
            .iter()
            .map(|caret| caret.y + caret.height)
            .fold(line_height, f64::max);
        let fallback_text = bootstrap_wrapped_text(text, &carets);
        TextLayout {
            size: Size::new(width, height),
            runs: Vec::new(),
            fallback_text: Some(fallback_text),
            carets,
        }
    }
}

fn bootstrap_wrapped_text(text: &str, carets: &[CaretPosition]) -> String {
    let mut wrapped = String::with_capacity(text.len());
    let mut previous_y = 0.0;
    for ((_, character), caret) in text.char_indices().zip(carets.iter().skip(1)) {
        if character != '\n' && caret.y > previous_y {
            wrapped.push('\n');
        }
        wrapped.push(character);
        previous_y = caret.y;
    }
    wrapped
}

fn bootstrap_carets(text: &str, font_size: f32, max_width: Option<f32>) -> Vec<CaretPosition> {
    let advance = f64::from(font_size) * BOOTSTRAP_ADVANCE_EM;
    let height = f64::from(font_size) * 1.35;
    let max_width = max_width.map(f64::from);
    let mut x = 0.0;
    let mut y = 0.0;
    let mut carets = vec![CaretPosition {
        index: 0,
        x,
        y,
        height,
    }];
    for (index, character) in text.char_indices() {
        if character == '\n' {
            x = 0.0;
            y += height;
        } else {
            if max_width.is_some_and(|width| x > 0.0 && x + advance > width) {
                x = 0.0;
                y += height;
            }
            x += advance;
        }
        carets.push(CaretPosition {
            index: index + character.len_utf8(),
            x,
            y,
            height,
        });
    }
    carets.dedup_by_key(|caret| caret.index);
    carets
}
