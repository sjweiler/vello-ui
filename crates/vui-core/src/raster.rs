use crate::{
    BoxConstraints, Event, EventCtx, LayoutCtx, PaintCtx, PaintOp, Point, Rect, SemanticNode,
    SemanticRole, SemanticsCtx, Size, Widget, WidgetId,
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

static NEXT_IMAGE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RasterImage {
    id: u64,
    width: u32,
    height: u32,
    pixels: Arc<[u8]>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImageFit {
    #[default]
    Contain,
    Cover,
    Fill,
}

pub struct ImageView {
    id: WidgetId,
    image: RasterImage,
    fit: ImageFit,
    label: Option<String>,
    bounds: Rect,
}

impl ImageView {
    #[must_use]
    pub fn new(image: RasterImage) -> Self {
        Self {
            id: WidgetId::next(),
            image,
            fit: ImageFit::Contain,
            label: None,
            bounds: Rect::default(),
        }
    }

    #[must_use]
    pub const fn fit(mut self, fit: ImageFit) -> Self {
        self.fit = fit;
        self
    }

    #[must_use]
    pub fn accessible_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    fn image_rect(&self) -> Rect {
        if self.fit == ImageFit::Fill {
            return self.bounds;
        }
        let natural = Size::new(f64::from(self.image.width), f64::from(self.image.height));
        let contain = (self.bounds.width / natural.width).min(self.bounds.height / natural.height);
        let cover = (self.bounds.width / natural.width).max(self.bounds.height / natural.height);
        let scale = if self.fit == ImageFit::Cover {
            cover
        } else {
            contain
        };
        let width = natural.width * scale;
        let height = natural.height * scale;
        Rect::new(
            self.bounds.x + (self.bounds.width - width) * 0.5,
            self.bounds.y + (self.bounds.height - height) * 0.5,
            width,
            height,
        )
    }
}

impl Widget for ImageView {
    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let natural = Size::new(f64::from(self.image.width), f64::from(self.image.height));
        let size = constraints.constrain(natural);
        self.bounds.width = size.width;
        self.bounds.height = size.height;
        size
    }
    fn set_origin(&mut self, origin: Point) {
        self.bounds.x = origin.x;
        self.bounds.y = origin.y;
    }
    fn bounds(&self) -> Rect {
        self.bounds
    }
    fn event(&mut self, _ctx: &mut EventCtx, _event: &Event) {}
    fn widget_id(&self) -> Option<WidgetId> {
        Some(self.id)
    }
    fn reconcile(&mut self, next: &mut dyn Widget) {
        if let Some(next) = next.as_any_mut().downcast_mut::<Self>() {
            next.id = self.id;
        }
    }
    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        if self.fit == ImageFit::Cover {
            ctx.push(PaintOp::PushClip {
                rect: self.bounds,
                radius: 0.0,
            });
        }
        ctx.push(PaintOp::Image {
            image: self.image.clone(),
            rect: self.image_rect(),
        });
        if self.fit == ImageFit::Cover {
            ctx.push(PaintOp::PopClip);
        }
    }
    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        if let Some(label) = &self.label {
            ctx.push(SemanticNode {
                id: self.id,
                role: SemanticRole::Image,
                bounds: self.bounds,
                label: label.clone(),
                disabled: false,
                focused: false,
                checked: None,
                value: Some(format!("{} × {}", self.image.width, self.image.height)),
                ime_cursor_area: None,
            });
        }
    }
}

impl RasterImage {
    /// Creates an RGBA8 image.
    ///
    /// # Errors
    ///
    /// Returns an error when either dimension is zero, the byte count
    /// overflows, or the buffer is not exactly four bytes per pixel.
    pub fn new(width: u32, height: u32, pixels: impl Into<Arc<[u8]>>) -> Result<Self, String> {
        let expected = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| "image dimensions overflow".to_owned())?;
        let pixels = pixels.into();
        if width == 0 || height == 0 {
            return Err("image dimensions must be non-zero".to_owned());
        }
        if pixels.len() != expected {
            return Err(format!(
                "expected {expected} RGBA bytes, got {}",
                pixels.len()
            ));
        }
        Ok(Self {
            id: NEXT_IMAGE_ID.fetch_add(1, Ordering::Relaxed),
            width,
            height,
            pixels,
        })
    }

    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
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
    pub fn pixels(&self) -> &Arc<[u8]> {
        &self.pixels
    }
}
