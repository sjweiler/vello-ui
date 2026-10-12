use crate::{
    BoxConstraints, Color, Event, EventCtx, Insets, LayoutCtx, PaintCtx, PaintOp, Point, Rect,
    SemanticsCtx, Size, Theme, UiCommand, Widget, WidgetId,
};

fn forward_origin(bounds: &mut Rect, child: &mut dyn Widget, origin: Point) {
    let dx = origin.x - bounds.x;
    let dy = origin.y - bounds.y;
    bounds.x = origin.x;
    bounds.y = origin.y;
    let child_bounds = child.bounds();
    child.set_origin(Point::new(child_bounds.x + dx, child_bounds.y + dy));
}

pub struct ThemeScope {
    theme: Theme,
    child: Box<dyn Widget>,
    bounds: Rect,
}

impl ThemeScope {
    #[must_use]
    pub fn new(theme: Theme, child: impl Widget + 'static) -> Self {
        Self {
            theme,
            child: Box::new(child),
            bounds: Rect::default(),
        }
    }
}

impl Widget for ThemeScope {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let size = ctx.with_theme(&self.theme, |ctx| self.child.layout(ctx, constraints));
        self.child
            .set_origin(Point::new(self.bounds.x, self.bounds.y));
        self.bounds.width = size.width;
        self.bounds.height = size.height;
        size
    }
    fn set_origin(&mut self, origin: Point) {
        forward_origin(&mut self.bounds, self.child.as_mut(), origin);
    }
    fn bounds(&self) -> Rect {
        self.bounds
    }
    fn event(&mut self, ctx: &mut EventCtx, event: &Event) {
        self.child.event(ctx, event);
    }
    fn event_target(&mut self, ctx: &mut EventCtx, event: &Event, target: WidgetId) -> bool {
        self.child.event_target(ctx, event, target)
    }
    fn apply_command(&mut self, command: UiCommand) -> Result<(), UiCommand> {
        self.child.apply_command(command)
    }
    fn reconcile(&mut self, next: &mut dyn Widget) {
        if let Some(next) = next.as_any_mut().downcast_mut::<Self>() {
            self.child.reconcile(next.child.as_mut());
        }
    }
    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.with_theme(&self.theme, |ctx| self.child.paint(ctx));
    }
    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        self.child.semantics(ctx);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SurfaceTone {
    #[default]
    Default,
    Raised,
}

pub struct Surface {
    child: Box<dyn Widget>,
    insets: Insets,
    tone: SurfaceTone,
    background: Option<Color>,
    border: Option<Color>,
    border_width: f64,
    radius: Option<f64>,
    elevation: u8,
    bounds: Rect,
}

impl Surface {
    #[must_use]
    pub fn new(child: impl Widget + 'static) -> Self {
        Self {
            child: Box::new(child),
            insets: Insets::default(),
            tone: SurfaceTone::Default,
            background: None,
            border: None,
            border_width: 0.0,
            radius: None,
            elevation: 0,
            bounds: Rect::default(),
        }
    }
    #[must_use]
    pub const fn padding(mut self, insets: Insets) -> Self {
        self.insets = insets;
        self
    }
    #[must_use]
    pub const fn tone(mut self, tone: SurfaceTone) -> Self {
        self.tone = tone;
        self
    }
    #[must_use]
    pub const fn background(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }
    #[must_use]
    pub fn border(mut self, color: Color, width: f64) -> Self {
        self.border = Some(color);
        self.border_width = width.max(0.0);
        self
    }
    #[must_use]
    pub fn radius(mut self, radius: f64) -> Self {
        self.radius = Some(radius.max(0.0));
        self
    }
    #[must_use]
    pub const fn elevation(mut self, level: u8) -> Self {
        self.elevation = level;
        self
    }
}

impl Widget for Surface {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let child_size = self.child.layout(
            ctx,
            constraints.shrink(self.insets.horizontal(), self.insets.vertical()),
        );
        self.child.set_origin(Point::new(
            self.bounds.x + self.insets.left,
            self.bounds.y + self.insets.top,
        ));
        let size = constraints.constrain(Size::new(
            child_size.width + self.insets.horizontal(),
            child_size.height + self.insets.vertical(),
        ));
        self.bounds.width = size.width;
        self.bounds.height = size.height;
        size
    }
    fn set_origin(&mut self, origin: Point) {
        forward_origin(&mut self.bounds, self.child.as_mut(), origin);
    }
    fn bounds(&self) -> Rect {
        self.bounds
    }
    fn event(&mut self, ctx: &mut EventCtx, event: &Event) {
        self.child.event(ctx, event);
    }
    fn event_target(&mut self, ctx: &mut EventCtx, event: &Event, target: WidgetId) -> bool {
        self.child.event_target(ctx, event, target)
    }
    fn apply_command(&mut self, command: UiCommand) -> Result<(), UiCommand> {
        self.child.apply_command(command)
    }
    fn reconcile(&mut self, next: &mut dyn Widget) {
        if let Some(next) = next.as_any_mut().downcast_mut::<Self>() {
            self.child.reconcile(next.child.as_mut());
        }
    }
    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let radius = self.radius.unwrap_or(ctx.theme().radii.lg);
        if self.elevation > 0 {
            let offset = if self.elevation > 1 {
                ctx.theme().elevation.high_offset
            } else {
                ctx.theme().elevation.low_offset
            };
            ctx.push(PaintOp::FillRect {
                rect: Rect::new(
                    self.bounds.x,
                    self.bounds.y + offset,
                    self.bounds.width,
                    self.bounds.height,
                ),
                color: ctx.theme().elevation.shadow,
                radius,
            });
        }
        let background = self.background.unwrap_or(match self.tone {
            SurfaceTone::Default => ctx.theme().colors.surface,
            SurfaceTone::Raised => ctx.theme().colors.surface_raised,
        });
        ctx.push(PaintOp::FillRect {
            rect: self.bounds,
            color: background,
            radius,
        });
        if self.border_width > 0.0 {
            ctx.push(PaintOp::StrokeRect {
                rect: self.bounds,
                color: self.border.unwrap_or(ctx.theme().colors.border),
                width: self.border_width,
                radius,
            });
        }
        self.child.paint(ctx);
    }
    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        self.child.semantics(ctx);
    }
}

pub struct Border {
    child: Box<dyn Widget>,
    color: Option<Color>,
    width: f64,
    radius: f64,
    bounds: Rect,
}

impl Border {
    #[must_use]
    pub fn new(child: impl Widget + 'static) -> Self {
        Self {
            child: Box::new(child),
            color: None,
            width: 1.0,
            radius: 0.0,
            bounds: Rect::default(),
        }
    }
    #[must_use]
    pub const fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }
    #[must_use]
    pub fn width(mut self, width: f64) -> Self {
        self.width = width.max(0.0);
        self
    }
    #[must_use]
    pub fn radius(mut self, radius: f64) -> Self {
        self.radius = radius.max(0.0);
        self
    }
}

impl Widget for Border {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let size = self.child.layout(ctx, constraints);
        self.child
            .set_origin(Point::new(self.bounds.x, self.bounds.y));
        self.bounds.width = size.width;
        self.bounds.height = size.height;
        size
    }
    fn set_origin(&mut self, origin: Point) {
        forward_origin(&mut self.bounds, self.child.as_mut(), origin);
    }
    fn bounds(&self) -> Rect {
        self.bounds
    }
    fn event(&mut self, ctx: &mut EventCtx, event: &Event) {
        self.child.event(ctx, event);
    }
    fn event_target(&mut self, ctx: &mut EventCtx, event: &Event, target: WidgetId) -> bool {
        self.child.event_target(ctx, event, target)
    }
    fn apply_command(&mut self, command: UiCommand) -> Result<(), UiCommand> {
        self.child.apply_command(command)
    }
    fn reconcile(&mut self, next: &mut dyn Widget) {
        if let Some(next) = next.as_any_mut().downcast_mut::<Self>() {
            self.child.reconcile(next.child.as_mut());
        }
    }
    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        self.child.paint(ctx);
        ctx.push(PaintOp::StrokeRect {
            rect: self.bounds,
            color: self.color.unwrap_or(ctx.theme().colors.border),
            width: self.width,
            radius: self.radius,
        });
    }
    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        self.child.semantics(ctx);
    }
}
