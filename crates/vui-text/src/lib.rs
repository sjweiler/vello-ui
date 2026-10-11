//! Text shaping and font fallback for VUI.

use fontique::{Blob, Collection, CollectionOptions, GenericFamily, SourceCache};
use parley::editing::Cursor;
use parley::{
    Alignment, AlignmentOptions, FontContext, Layout, LayoutContext, PositionedLayoutItem,
    StyleProperty, layout::Affinity,
};
use vui_core::{CaretPosition, ShapedGlyph, ShapedGlyphRun, Size, TextLayout, TextSystem};

use std::{error::Error, fmt, sync::Arc};

struct StaticFont(&'static [u8]);

impl AsRef<[u8]> for StaticFont {
    fn as_ref(&self) -> &[u8] {
        self.0
    }
}

/// Error returned when an embedded byte slice does not contain a usable font.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmbeddedFontError {
    index: usize,
}

impl EmbeddedFontError {
    #[must_use]
    pub const fn index(self) -> usize {
        self.index
    }
}

impl fmt::Display for EmbeddedFontError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "embedded font at index {} contains no usable faces",
            self.index
        )
    }
}

impl Error for EmbeddedFontError {}

pub struct ParleyTextSystem {
    fonts: FontContext,
    layouts: LayoutContext<()>,
}

impl ParleyTextSystem {
    #[must_use]
    pub fn new() -> Self {
        Self {
            fonts: FontContext::new(),
            layouts: LayoutContext::new(),
        }
    }

    /// Creates a memory-only font context without consulting host system fonts.
    ///
    /// Registered families are queried in input order for the default sans-serif
    /// and system-ui stacks, allowing later fonts to provide script fallback.
    ///
    /// # Errors
    ///
    /// Returns an error identifying the first byte slice that contains no usable font face.
    pub fn with_embedded_fonts(
        fonts: impl IntoIterator<Item = &'static [u8]>,
    ) -> Result<Self, EmbeddedFontError> {
        let mut collection = Collection::new(CollectionOptions {
            shared: false,
            system_fonts: false,
        });
        let mut families = Vec::new();
        for (index, bytes) in fonts.into_iter().enumerate() {
            let registered =
                collection.register_fonts(Blob::new(Arc::new(StaticFont(bytes))), None);
            if registered.is_empty() {
                return Err(EmbeddedFontError { index });
            }
            families.extend(registered.into_iter().map(|(family, _)| family));
        }
        collection.set_generic_families(GenericFamily::SansSerif, families.iter().copied());
        collection.set_generic_families(GenericFamily::SystemUi, families.iter().copied());
        Ok(Self {
            fonts: FontContext {
                collection,
                source_cache: SourceCache::default(),
            },
            layouts: LayoutContext::new(),
        })
    }
}

impl Default for ParleyTextSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl TextSystem for ParleyTextSystem {
    fn layout(&mut self, text: &str, font_size: f32, max_width: Option<f32>) -> TextLayout {
        let mut builder = self
            .layouts
            .ranged_builder(&mut self.fonts, text, 1.0, true);
        builder.push_default(StyleProperty::FontSize(font_size));
        let mut layout: Layout<()> = builder.build(text);
        layout.break_all_lines(max_width);
        layout.align(Alignment::Start, AlignmentOptions::default());

        let mut runs = Vec::new();
        for line in layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                    continue;
                };
                let run = glyph_run.run();
                runs.push(ShapedGlyphRun {
                    font: run.font().clone(),
                    font_size: run.font_size(),
                    normalized_coords: run.normalized_coords().to_vec(),
                    glyphs: glyph_run
                        .positioned_glyphs()
                        .map(|glyph| ShapedGlyph {
                            id: glyph.id,
                            x: glyph.x,
                            y: glyph.y,
                        })
                        .collect(),
                });
            }
        }

        let mut carets = text
            .char_indices()
            .map(|(index, _)| index)
            .chain(std::iter::once(text.len()))
            .map(|index| {
                let cursor = Cursor::from_byte_index(&layout, index, Affinity::Downstream);
                let geometry = cursor.geometry(&layout, 0.0);
                CaretPosition {
                    index: cursor.index(),
                    x: geometry.x0,
                    y: geometry.y0,
                    height: geometry.height(),
                }
            })
            .collect::<Vec<_>>();
        carets.sort_by_key(|caret| caret.index);
        carets.dedup_by_key(|caret| caret.index);

        TextLayout {
            size: Size::new(f64::from(layout.width()), f64::from(layout.height())),
            fallback_text: (runs.is_empty() && !text.is_empty()).then(|| text.to_owned()),
            runs,
            carets,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_system_text() {
        let mut system = ParleyTextSystem::new();
        let layout = system.layout("Hello VUI", 16.0, None);
        assert!(layout.size.width > 0.0);
        assert!(layout.size.height > 0.0);
        assert!(!layout.runs.is_empty() || layout.fallback_text.is_some());
    }

    #[test]
    fn wraps_text_and_emits_visual_line_carets() {
        let mut system = ParleyTextSystem::new();
        let layout = system.layout("alpha beta gamma", 16.0, Some(48.0));
        assert!(layout.size.height > 20.0);
        assert!(layout.carets.iter().any(|caret| caret.y > 0.0));
    }

    #[test]
    fn rejects_invalid_embedded_font_data() {
        let error = ParleyTextSystem::with_embedded_fonts([b"not a font".as_slice()])
            .err()
            .expect("invalid font should fail");
        assert_eq!(error.index(), 0);
    }
}
