use crate::{
    BoxConstraints, ButtonVariant, ControlState, Event, EventCtx, Insets, Label, LayoutCtx,
    OverlayPlacement, Padding, PaintCtx, PaintOp, PathCommand, Point, Rect, SemanticNode,
    SemanticRole, SemanticsCtx, Size, Surface, Widget, WidgetId,
};

#[derive(Clone, Debug, PartialEq)]
pub struct IconData {
    pub view_box: Size,
    pub commands: Vec<PathCommand>,
    pub filled: bool,
}

impl IconData {
    #[must_use]
    pub fn new(view_box: Size, commands: impl IntoIterator<Item = PathCommand>) -> Self {
        Self {
            view_box: Size::new(
                if view_box.width.is_finite() {
                    view_box.width.max(f64::EPSILON)
                } else {
                    24.0
                },
                if view_box.height.is_finite() {
                    view_box.height.max(f64::EPSILON)
                } else {
                    24.0
                },
            ),
            commands: commands.into_iter().collect(),
            filled: false,
        }
    }

    #[must_use]
    pub const fn filled(mut self, filled: bool) -> Self {
        self.filled = filled;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconName {
    Add,
    Close,
    Command,
    Menu,
    Search,
    Settings,
}

impl IconName {
    #[must_use]
    pub fn data(self) -> IconData {
        use PathCommand::{LineTo, MoveTo};
        let p = Point::new;
        let commands = match self {
            Self::Add => vec![
                MoveTo(p(12.0, 4.0)),
                LineTo(p(12.0, 20.0)),
                MoveTo(p(4.0, 12.0)),
                LineTo(p(20.0, 12.0)),
            ],
            Self::Close => vec![
                MoveTo(p(5.0, 5.0)),
                LineTo(p(19.0, 19.0)),
                MoveTo(p(19.0, 5.0)),
                LineTo(p(5.0, 19.0)),
            ],
            Self::Command => vec![
                MoveTo(p(9.0, 7.0)),
                LineTo(p(9.0, 17.0)),
                MoveTo(p(15.0, 7.0)),
                LineTo(p(15.0, 17.0)),
                MoveTo(p(7.0, 9.0)),
                LineTo(p(17.0, 9.0)),
                MoveTo(p(7.0, 15.0)),
                LineTo(p(17.0, 15.0)),
            ],
            Self::Menu => vec![
                MoveTo(p(4.0, 7.0)),
                LineTo(p(20.0, 7.0)),
                MoveTo(p(4.0, 12.0)),
                LineTo(p(20.0, 12.0)),
                MoveTo(p(4.0, 17.0)),
                LineTo(p(20.0, 17.0)),
            ],
            Self::Search => vec![
                MoveTo(p(19.0, 19.0)),
                LineTo(p(15.5, 15.5)),
                MoveTo(p(10.5, 4.5)),
                PathCommand::CurveTo(p(18.0, 4.5), p(18.0, 16.5), p(10.5, 16.5)),
                PathCommand::CurveTo(p(3.0, 16.5), p(3.0, 4.5), p(10.5, 4.5)),
            ],
            Self::Settings => vec![
                MoveTo(p(12.0, 3.0)),
                LineTo(p(12.0, 6.0)),
                MoveTo(p(12.0, 18.0)),
                LineTo(p(12.0, 21.0)),
                MoveTo(p(3.0, 12.0)),
                LineTo(p(6.0, 12.0)),
                MoveTo(p(18.0, 12.0)),
                LineTo(p(21.0, 12.0)),
                MoveTo(p(5.6, 5.6)),
                LineTo(p(7.7, 7.7)),
                MoveTo(p(16.3, 16.3)),
                LineTo(p(18.4, 18.4)),
                MoveTo(p(18.4, 5.6)),
                LineTo(p(16.3, 7.7)),
                MoveTo(p(7.7, 16.3)),
                LineTo(p(5.6, 18.4)),
            ],
        };
        IconData::new(Size::new(24.0, 24.0), commands)
    }
}

fn transformed(data: &IconData, bounds: Rect) -> Vec<PathCommand> {
    let scale = (bounds.width / data.view_box.width).min(bounds.height / data.view_box.height);
    let dx = bounds.x + (bounds.width - data.view_box.width * scale) * 0.5;
    let dy = bounds.y + (bounds.height - data.view_box.height * scale) * 0.5;
    let point = |value: Point| Point::new(dx + value.x * scale, dy + value.y * scale);
    data.commands
        .iter()
        .map(|command| match *command {
            PathCommand::MoveTo(value) => PathCommand::MoveTo(point(value)),
            PathCommand::LineTo(value) => PathCommand::LineTo(point(value)),
            PathCommand::QuadTo(control, value) => {
                PathCommand::QuadTo(point(control), point(value))
            }
            PathCommand::CurveTo(first, second, value) => {
                PathCommand::CurveTo(point(first), point(second), point(value))
            }
            PathCommand::Close => PathCommand::Close,
        })
        .collect()
}

fn paint_icon(ctx: &mut PaintCtx<'_>, data: &IconData, bounds: Rect, color: crate::Color) {
    let commands = transformed(data, bounds);
    if data.filled {
        ctx.push(PaintOp::FillPath { commands, color });
    } else {
        ctx.push(PaintOp::StrokePath {
            commands,
            color,
            width: 1.8,
        });
    }
}

pub struct Icon {
    id: WidgetId,
    data: IconData,
    size: f64,
    color: Option<crate::Color>,
    label: Option<String>,
    bounds: Rect,
}

impl Icon {
    #[must_use]
    pub fn new(data: IconData) -> Self {
        Self {
            id: WidgetId::next(),
            data,
            size: 20.0,
            color: None,
            label: None,
            bounds: Rect::default(),
        }
    }
    #[must_use]
    pub fn named(name: IconName) -> Self {
        Self::new(name.data())
    }
    #[must_use]
    pub fn size(mut self, size: f64) -> Self {
        self.size = size.max(0.0);
        self
    }
    #[must_use]
    pub const fn color(mut self, color: crate::Color) -> Self {
        self.color = Some(color);
        self
    }
    #[must_use]
    pub fn accessible_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }
}

impl Widget for Icon {
    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let size = constraints.constrain(Size::new(self.size, self.size));
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
    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        paint_icon(
            ctx,
            &self.data,
            self.bounds,
            self.color.unwrap_or(ctx.theme().colors.text),
        );
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
                value: None,
                ime_cursor_area: None,
            });
        }
    }
}

type IconButtonAction = dyn FnMut(WidgetId, &mut EventCtx);

pub struct IconButton {
    id: WidgetId,
    icon: IconData,
    label: String,
    tooltip: Option<String>,
    bounds: Rect,
    interaction: u8,
    disabled: bool,
    on_click: Box<IconButtonAction>,
}

impl IconButton {
    const HOVERED: u8 = 1;
    const PRESSED: u8 = 1 << 1;
    const FOCUSED: u8 = 1 << 2;
    const TOOLTIP_VISIBLE: u8 = 1 << 3;

    #[must_use]
    pub fn new(
        name: IconName,
        label: impl Into<String>,
        mut on_click: impl FnMut() + 'static,
    ) -> Self {
        Self {
            id: WidgetId::next(),
            icon: name.data(),
            label: label.into(),
            tooltip: None,
            bounds: Rect::default(),
            interaction: 0,
            disabled: false,
            on_click: Box::new(move |_, _| on_click()),
        }
    }
    #[must_use]
    pub fn action(
        name: IconName,
        label: impl Into<String>,
        on_click: impl FnMut(WidgetId, &mut EventCtx) + 'static,
    ) -> Self {
        Self {
            id: WidgetId::next(),
            icon: name.data(),
            label: label.into(),
            tooltip: None,
            bounds: Rect::default(),
            interaction: 0,
            disabled: false,
            on_click: Box::new(on_click),
        }
    }
    #[must_use]
    pub fn data(
        data: IconData,
        label: impl Into<String>,
        on_click: impl FnMut() + 'static,
    ) -> Self {
        let mut button = Self::new(IconName::Command, label, on_click);
        button.icon = data;
        button
    }
    #[must_use]
    pub fn tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }
    #[must_use]
    pub const fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    fn activate(&mut self, ctx: &mut EventCtx) {
        if !self.disabled {
            (self.on_click)(self.id, ctx);
            ctx.set_handled();
            ctx.request_paint();
        }
    }

    fn show_tooltip(&mut self, ctx: &mut EventCtx) {
        if self.has(Self::TOOLTIP_VISIBLE) {
            return;
        }
        if let Some(text) = &self.tooltip {
            let bubble = Surface::new(Padding::new(
                Insets::all(6.0),
                Label::new(text.clone()).text_size(13.0),
            ))
            .elevation(1)
            .radius(5.0);
            ctx.show_overlay(self.id, OverlayPlacement::Above, bubble);
            self.set(Self::TOOLTIP_VISIBLE, true);
        }
    }

    const fn has(&self, flag: u8) -> bool {
        self.interaction & flag != 0
    }

    fn set(&mut self, flag: u8, enabled: bool) {
        if enabled {
            self.interaction |= flag;
        } else {
            self.interaction &= !flag;
        }
    }
}

impl Widget for IconButton {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let style = ctx.theme().button_style(ButtonVariant::Icon);
        let size = constraints.constrain(Size::new(style.min_width, style.height));
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
    fn event(&mut self, ctx: &mut EventCtx, event: &Event) {
        if self.disabled && !matches!(event, Event::FocusChanged(_)) {
            return;
        }
        match event {
            Event::PointerMoved(position) => {
                let hovered = self.bounds.contains(*position);
                if hovered && !self.has(Self::HOVERED) {
                    self.set(Self::HOVERED, true);
                    self.show_tooltip(ctx);
                    ctx.request_paint();
                }
                if !hovered && self.has(Self::HOVERED) {
                    self.set(Self::HOVERED, false);
                    self.set(Self::TOOLTIP_VISIBLE, false);
                    ctx.dismiss_overlay();
                    ctx.request_paint();
                }
            }
            Event::PointerLeft => {
                if self.has(Self::HOVERED) {
                    self.set(Self::HOVERED, false);
                    self.set(Self::TOOLTIP_VISIBLE, false);
                    ctx.dismiss_overlay();
                    ctx.request_paint();
                }
            }
            Event::PointerDown {
                position,
                button: crate::PointerButton::Primary,
                ..
            } if self.bounds.contains(*position) => {
                self.set(Self::PRESSED, true);
                ctx.capture_pointer(self.id);
                ctx.request_focus(self.id);
                ctx.request_paint();
                ctx.set_handled();
            }
            Event::PointerUp {
                position,
                button: crate::PointerButton::Primary,
            } => {
                let activate = self.has(Self::PRESSED) && self.bounds.contains(*position);
                self.set(Self::PRESSED, false);
                ctx.release_pointer();
                if activate {
                    self.activate(ctx);
                }
            }
            Event::KeyDown(crate::Key::Enter | crate::Key::Space) if self.has(Self::FOCUSED) => {
                self.activate(ctx);
            }
            Event::AccessibilityAction {
                target,
                action: crate::SemanticAction::Click,
            } if *target == self.id => self.activate(ctx),
            Event::AccessibilityAction {
                target,
                action: crate::SemanticAction::Focus,
            } if *target == self.id => {
                ctx.request_focus(self.id);
                ctx.set_handled();
            }
            Event::FocusChanged(focused) => {
                self.set(Self::FOCUSED, *focused == Some(self.id));
                ctx.request_paint();
            }
            _ => {}
        }
    }
    fn widget_id(&self) -> Option<WidgetId> {
        Some(self.id)
    }
    fn reconcile(&mut self, next: &mut dyn Widget) {
        if let Some(next) = next.as_any_mut().downcast_mut::<Self>() {
            next.id = self.id;
            next.interaction = self.interaction;
        }
    }
    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let style = ctx.theme().button_style(ButtonVariant::Icon);
        let state = style.resolve(if self.disabled {
            ControlState::Disabled
        } else if self.has(Self::PRESSED) {
            ControlState::Pressed
        } else if self.has(Self::HOVERED) {
            ControlState::Hovered
        } else {
            ControlState::Normal
        });
        ctx.push(PaintOp::FillRect {
            rect: self.bounds,
            color: state.background,
            radius: style.radius,
        });
        if self.has(Self::FOCUSED) {
            ctx.push(PaintOp::StrokeRect {
                rect: self.bounds,
                color: ctx.theme().colors.focus,
                width: ctx.theme().borders.strong,
                radius: style.radius,
            });
        }
        let icon_size = self.bounds.height.min(self.bounds.width) * 0.55;
        let icon_bounds = Rect::new(
            self.bounds.x + (self.bounds.width - icon_size) * 0.5,
            self.bounds.y + (self.bounds.height - icon_size) * 0.5,
            icon_size,
            icon_size,
        );
        paint_icon(ctx, &self.icon, icon_bounds, state.foreground);
    }
    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        ctx.push(SemanticNode {
            id: self.id,
            role: SemanticRole::Button,
            bounds: self.bounds,
            label: self.label.clone(),
            disabled: self.disabled,
            focused: self.has(Self::FOCUSED),
            checked: None,
            value: None,
            ime_cursor_area: None,
        });
    }
}
