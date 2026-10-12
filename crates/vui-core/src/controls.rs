use crate::{
    BoxConstraints, Color, Event, EventCtx, Key, LayoutCtx, Menu, MenuItem, OverlayPlacement,
    PaintCtx, PaintOp, Point, Rect, SemanticAction, SemanticNode, SemanticRole, SemanticsCtx, Size,
    TextLayout, UiCommand, Widget, WidgetId, widgets::paint_text_layout,
};
use std::{
    cell::{Cell, RefCell},
    f64::consts::TAU,
    rc::Rc,
};

#[derive(Clone, Copy, Default)]
struct ControlInteraction {
    hovered: bool,
    pressed: bool,
    focused: bool,
}

type ValueCallback = dyn FnMut(f64);

/// A bounded numeric control with pointer dragging and keyboard adjustment.
pub struct Slider {
    id: WidgetId,
    label: String,
    value: f64,
    min: f64,
    max: f64,
    step: f64,
    disabled: bool,
    bounds: Rect,
    track: Rect,
    interaction: ControlInteraction,
    label_layout: TextLayout,
    value_layout: TextLayout,
    on_change: Box<ValueCallback>,
}

impl Slider {
    #[must_use]
    pub fn new(
        label: impl Into<String>,
        value: f64,
        range: std::ops::RangeInclusive<f64>,
        on_change: impl FnMut(f64) + 'static,
    ) -> Self {
        let min = *range.start();
        let max = (*range.end()).max(min);
        Self {
            id: WidgetId::next(),
            label: label.into(),
            value: value.clamp(min, max),
            min,
            max,
            step: 0.0,
            disabled: false,
            bounds: Rect::default(),
            track: Rect::default(),
            interaction: ControlInteraction::default(),
            label_layout: TextLayout::default(),
            value_layout: TextLayout::default(),
            on_change: Box::new(on_change),
        }
    }

    #[must_use]
    pub fn step(mut self, step: f64) -> Self {
        self.step = step.max(0.0);
        self
    }

    #[must_use]
    pub const fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    #[must_use]
    pub const fn value(&self) -> f64 {
        self.value
    }

    fn fraction(&self) -> f64 {
        let extent = self.max - self.min;
        if extent <= f64::EPSILON {
            0.0
        } else {
            ((self.value - self.min) / extent).clamp(0.0, 1.0)
        }
    }

    fn set_value(&mut self, value: f64, ctx: &mut EventCtx) {
        let mut value = value.clamp(self.min, self.max);
        if self.step > 0.0 {
            value = (self.min + ((value - self.min) / self.step).round() * self.step)
                .clamp(self.min, self.max);
        }
        if (value - self.value).abs() <= f64::EPSILON {
            return;
        }
        self.value = value;
        (self.on_change)(value);
        ctx.set_handled();
        ctx.request_layout();
    }

    fn set_from_pointer(&mut self, x: f64, ctx: &mut EventCtx) {
        let fraction = if self.track.width <= f64::EPSILON {
            0.0
        } else {
            ((x - self.track.x) / self.track.width).clamp(0.0, 1.0)
        };
        self.set_value(self.min + fraction * (self.max - self.min), ctx);
    }

    fn keyboard_step(&self) -> f64 {
        if self.step > 0.0 {
            self.step
        } else {
            (self.max - self.min) / 100.0
        }
    }
}

impl Widget for Slider {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.label_layout = ctx.layout_text(&self.label, 14.0, None);
        self.value_layout = ctx.layout_text(&format!("{:.0}", self.value), 13.0, None);
        let width = if constraints.max.width.is_finite() {
            constraints.max.width.min(360.0)
        } else {
            240.0
        };
        let size = constraints.constrain(Size::new(width.max(120.0), 48.0));
        self.bounds.width = size.width;
        self.bounds.height = size.height;
        self.track = Rect::new(
            self.bounds.x + 8.0,
            self.bounds.y + size.height - 15.0,
            (size.width - 16.0).max(0.0),
            6.0,
        );
        size
    }

    fn set_origin(&mut self, origin: Point) {
        let dx = origin.x - self.bounds.x;
        let dy = origin.y - self.bounds.y;
        self.bounds.x = origin.x;
        self.bounds.y = origin.y;
        self.track.x += dx;
        self.track.y += dy;
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
                if hovered != self.interaction.hovered {
                    self.interaction.hovered = hovered;
                    ctx.request_paint();
                }
                if self.interaction.pressed {
                    self.set_from_pointer(position.x, ctx);
                }
            }
            Event::PointerLeft if !self.interaction.pressed => {
                self.interaction.hovered = false;
                ctx.request_paint();
            }
            Event::PointerDown {
                position, button, ..
            } if *button == crate::PointerButton::Primary && self.bounds.contains(*position) => {
                self.interaction.pressed = true;
                ctx.capture_pointer(self.id);
                ctx.request_focus(self.id);
                self.set_from_pointer(position.x, ctx);
                ctx.set_handled();
            }
            Event::PointerUp { position, button }
                if *button == crate::PointerButton::Primary && self.interaction.pressed =>
            {
                self.set_from_pointer(position.x, ctx);
                self.interaction.pressed = false;
                ctx.release_pointer();
                ctx.set_handled();
                ctx.request_paint();
            }
            Event::KeyDown(Key::ArrowLeft | Key::ArrowDown) if self.interaction.focused => {
                self.set_value(self.value - self.keyboard_step(), ctx);
            }
            Event::KeyDown(Key::ArrowRight | Key::ArrowUp) if self.interaction.focused => {
                self.set_value(self.value + self.keyboard_step(), ctx);
            }
            Event::KeyDown(Key::Home) if self.interaction.focused => self.set_value(self.min, ctx),
            Event::KeyDown(Key::End) if self.interaction.focused => self.set_value(self.max, ctx),
            Event::AccessibilityAction { target, action } if *target == self.id => match action {
                SemanticAction::Click | SemanticAction::Focus => {
                    ctx.request_focus(self.id);
                    ctx.set_handled();
                }
                SemanticAction::Increment => {
                    self.set_value(self.value + self.keyboard_step(), ctx);
                }
                SemanticAction::Decrement => {
                    self.set_value(self.value - self.keyboard_step(), ctx);
                }
            },
            Event::FocusChanged(focused) => {
                self.interaction.focused = *focused == Some(self.id);
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
        let muted = if self.disabled {
            ctx.theme().colors.disabled
        } else {
            ctx.theme().colors.secondary
        };
        paint_text_layout(
            ctx,
            &self.label_layout,
            Point::new(self.bounds.x, self.bounds.y),
            if self.disabled {
                ctx.theme().colors.text_muted
            } else {
                ctx.theme().text
            },
            14.0,
        );
        paint_text_layout(
            ctx,
            &self.value_layout,
            Point::new(
                self.bounds.x + self.bounds.width - self.value_layout.size.width,
                self.bounds.y,
            ),
            ctx.theme().colors.text_muted,
            13.0,
        );
        ctx.push(PaintOp::FillRect {
            rect: self.track,
            color: muted,
            radius: 3.0,
        });
        let filled = Rect::new(
            self.track.x,
            self.track.y,
            self.track.width * self.fraction(),
            6.0,
        );
        ctx.push(PaintOp::FillRect {
            rect: filled,
            color: if self.disabled {
                muted
            } else {
                ctx.theme().colors.primary
            },
            radius: 3.0,
        });
        let thumb_x = self.track.x + self.track.width * self.fraction();
        let thumb = Rect::new(thumb_x - 7.0, self.track.y - 4.0, 14.0, 14.0);
        ctx.push(PaintOp::FillRect {
            rect: thumb,
            color: if self.disabled {
                muted
            } else {
                ctx.theme().colors.primary
            },
            radius: 7.0,
        });
        if self.interaction.focused {
            ctx.push(PaintOp::StrokeRect {
                rect: thumb,
                color: ctx.theme().focus,
                width: 2.0,
                radius: 7.0,
            });
        }
    }

    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        ctx.push(SemanticNode {
            id: self.id,
            role: SemanticRole::Slider,
            bounds: self.bounds,
            label: self.label.clone(),
            disabled: self.disabled,
            focused: self.interaction.focused,
            checked: None,
            value: Some(self.value.to_string()),
            ime_cursor_area: None,
        });
    }
}

type BoolCallback = dyn FnMut(bool);

/// A compact binary switch intended for system settings.
pub struct Switch {
    id: WidgetId,
    label: String,
    checked: bool,
    disabled: bool,
    bounds: Rect,
    interaction: ControlInteraction,
    layout: TextLayout,
    on_change: Box<BoolCallback>,
}

impl Switch {
    #[must_use]
    pub fn new(
        label: impl Into<String>,
        checked: bool,
        on_change: impl FnMut(bool) + 'static,
    ) -> Self {
        Self {
            id: WidgetId::next(),
            label: label.into(),
            checked,
            disabled: false,
            bounds: Rect::default(),
            interaction: ControlInteraction::default(),
            layout: TextLayout::default(),
            on_change: Box::new(on_change),
        }
    }

    #[must_use]
    pub const fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    fn toggle(&mut self, ctx: &mut EventCtx) {
        if self.disabled {
            return;
        }
        self.checked = !self.checked;
        (self.on_change)(self.checked);
        ctx.set_handled();
        ctx.request_paint();
    }
}

impl Widget for Switch {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.layout = ctx.layout_text(&self.label, 15.0, None);
        let size = constraints.constrain(Size::new(self.layout.size.width + 56.0, 30.0));
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
                self.interaction.hovered = self.bounds.contains(*position);
                ctx.request_paint();
            }
            Event::PointerLeft => {
                self.interaction.hovered = false;
                ctx.request_paint();
            }
            Event::PointerDown {
                position, button, ..
            } if *button == crate::PointerButton::Primary && self.bounds.contains(*position) => {
                self.interaction.pressed = true;
                ctx.capture_pointer(self.id);
                ctx.request_focus(self.id);
                ctx.set_handled();
            }
            Event::PointerUp { position, button }
                if *button == crate::PointerButton::Primary && self.interaction.pressed =>
            {
                self.interaction.pressed = false;
                ctx.release_pointer();
                if self.bounds.contains(*position) {
                    self.toggle(ctx);
                }
            }
            Event::KeyDown(Key::Enter | Key::Space) if self.interaction.focused => self.toggle(ctx),
            Event::AccessibilityAction { target, action } if *target == self.id => match action {
                SemanticAction::Click => self.toggle(ctx),
                SemanticAction::Focus => {
                    ctx.request_focus(self.id);
                    ctx.set_handled();
                }
                SemanticAction::Increment | SemanticAction::Decrement => {}
            },
            Event::FocusChanged(focused) => {
                self.interaction.focused = *focused == Some(self.id);
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
        let track = Rect::new(self.bounds.x, self.bounds.y + 3.0, 44.0, 24.0);
        let color = if self.disabled {
            ctx.theme().colors.disabled
        } else if self.checked {
            ctx.theme().colors.primary
        } else if self.interaction.hovered {
            ctx.theme().colors.secondary_hover
        } else {
            ctx.theme().colors.secondary
        };
        ctx.push(PaintOp::FillRect {
            rect: track,
            color,
            radius: 12.0,
        });
        let knob_x = if self.checked {
            track.x + 23.0
        } else {
            track.x + 3.0
        };
        ctx.push(PaintOp::FillRect {
            rect: Rect::new(knob_x, track.y + 3.0, 18.0, 18.0),
            color: Color::WHITE,
            radius: 9.0,
        });
        if self.interaction.focused {
            ctx.push(PaintOp::StrokeRect {
                rect: track,
                color: ctx.theme().focus,
                width: 2.0,
                radius: 12.0,
            });
        }
        paint_text_layout(
            ctx,
            &self.layout,
            Point::new(self.bounds.x + 54.0, self.bounds.y + 5.0),
            if self.disabled {
                ctx.theme().colors.text_muted
            } else {
                ctx.theme().text
            },
            15.0,
        );
    }
    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        ctx.push(SemanticNode {
            id: self.id,
            role: SemanticRole::Switch,
            bounds: self.bounds,
            label: self.label.clone(),
            disabled: self.disabled,
            focused: self.interaction.focused,
            checked: Some(self.checked),
            value: None,
            ime_cursor_area: None,
        });
    }
}

/// One mutually-exclusive option. Selection ownership remains with the caller.
pub struct RadioButton {
    inner: Switch,
}

impl RadioButton {
    #[must_use]
    pub fn new(
        label: impl Into<String>,
        selected: bool,
        mut on_select: impl FnMut() + 'static,
    ) -> Self {
        Self {
            inner: Switch::new(label, selected, move |checked| {
                if checked {
                    on_select();
                }
            }),
        }
    }
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.inner = self.inner.disabled(disabled);
        self
    }
}

impl Widget for RadioButton {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.inner.layout(ctx, constraints)
    }
    fn set_origin(&mut self, origin: Point) {
        self.inner.set_origin(origin);
    }
    fn bounds(&self) -> Rect {
        self.inner.bounds()
    }
    fn event(&mut self, ctx: &mut EventCtx, event: &Event) {
        if self.inner.checked
            && matches!(
                event,
                Event::PointerDown { .. }
                    | Event::KeyDown(Key::Enter | Key::Space)
                    | Event::AccessibilityAction {
                        action: SemanticAction::Click,
                        ..
                    }
            )
        {
            return;
        }
        self.inner.event(ctx, event);
    }
    fn widget_id(&self) -> Option<WidgetId> {
        self.inner.widget_id()
    }
    fn event_target(&mut self, ctx: &mut EventCtx, event: &Event, target: WidgetId) -> bool {
        self.inner.event_target(ctx, event, target)
    }
    fn reconcile(&mut self, next: &mut dyn Widget) {
        if let Some(next) = next.as_any_mut().downcast_mut::<Self>() {
            self.inner.reconcile(&mut next.inner);
        }
    }
    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let circle = Rect::new(self.inner.bounds.x, self.inner.bounds.y + 5.0, 20.0, 20.0);
        ctx.push(PaintOp::FillRect {
            rect: circle,
            color: ctx.theme().colors.secondary,
            radius: 10.0,
        });
        if self.inner.checked {
            ctx.push(PaintOp::FillRect {
                rect: Rect::new(circle.x + 5.0, circle.y + 5.0, 10.0, 10.0),
                color: ctx.theme().colors.primary,
                radius: 5.0,
            });
        }
        if self.inner.interaction.focused {
            ctx.push(PaintOp::StrokeRect {
                rect: circle,
                color: ctx.theme().focus,
                width: 2.0,
                radius: 10.0,
            });
        }
        paint_text_layout(
            ctx,
            &self.inner.layout,
            Point::new(self.inner.bounds.x + 30.0, self.inner.bounds.y + 5.0),
            if self.inner.disabled {
                ctx.theme().colors.text_muted
            } else {
                ctx.theme().text
            },
            15.0,
        );
    }
    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        ctx.push(SemanticNode {
            id: self.inner.id,
            role: SemanticRole::RadioButton,
            bounds: self.inner.bounds,
            label: self.inner.label.clone(),
            disabled: self.inner.disabled,
            focused: self.inner.interaction.focused,
            checked: Some(self.inner.checked),
            value: None,
            ime_cursor_area: None,
        });
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectOption {
    pub label: String,
    pub disabled: bool,
}

impl SelectOption {
    #[must_use]
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            disabled: false,
        }
    }
    #[must_use]
    pub const fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

type SelectCallback = Rc<RefCell<Box<dyn FnMut(usize)>>>;

/// A button-backed single-selection popup.
pub struct Select {
    id: WidgetId,
    label: String,
    options: Vec<SelectOption>,
    selected: Rc<Cell<usize>>,
    disabled: bool,
    bounds: Rect,
    interaction: ControlInteraction,
    layout: TextLayout,
    on_change: SelectCallback,
}

impl Select {
    #[must_use]
    pub fn new(
        label: impl Into<String>,
        options: impl IntoIterator<Item = SelectOption>,
        selected: usize,
        on_change: impl FnMut(usize) + 'static,
    ) -> Self {
        let options: Vec<_> = options.into_iter().collect();
        let selected = selected.min(options.len().saturating_sub(1));
        Self {
            id: WidgetId::next(),
            label: label.into(),
            options,
            selected: Rc::new(Cell::new(selected)),
            disabled: false,
            bounds: Rect::default(),
            interaction: ControlInteraction::default(),
            layout: TextLayout::default(),
            on_change: Rc::new(RefCell::new(Box::new(on_change))),
        }
    }
    #[must_use]
    pub const fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    #[must_use]
    pub fn selected_index(&self) -> usize {
        self.selected.get()
    }
    fn show_menu(&mut self, ctx: &mut EventCtx) {
        if self.options.is_empty() {
            return;
        }
        let selected = self.selected.clone();
        let callback = self.on_change.clone();
        let items = self
            .options
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, option)| {
                let state = selected.clone();
                let callback = callback.clone();
                MenuItem::radio_action(option.label, index == selected.get(), move |ctx| {
                    state.set(index);
                    (callback.borrow_mut())(index);
                    ctx.request_layout();
                })
                .disabled(option.disabled)
            });
        ctx.show_overlay(self.id, OverlayPlacement::Below, Menu::new(items));
        ctx.set_handled();
    }
}

impl Widget for Select {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let selected = self
            .options
            .get(self.selected.get())
            .map_or("", |option| option.label.as_str());
        self.layout = ctx.layout_text(selected, 15.0, None);
        let size =
            constraints.constrain(Size::new((self.layout.size.width + 44.0).max(140.0), 40.0));
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
                self.interaction.hovered = self.bounds.contains(*position);
                ctx.request_paint();
            }
            Event::PointerLeft => {
                self.interaction.hovered = false;
                ctx.request_paint();
            }
            Event::PointerDown {
                position, button, ..
            } if *button == crate::PointerButton::Primary && self.bounds.contains(*position) => {
                self.interaction.pressed = true;
                ctx.capture_pointer(self.id);
                ctx.request_focus(self.id);
                ctx.set_handled();
            }
            Event::PointerUp { position, button }
                if *button == crate::PointerButton::Primary && self.interaction.pressed =>
            {
                self.interaction.pressed = false;
                ctx.release_pointer();
                if self.bounds.contains(*position) {
                    self.show_menu(ctx);
                }
            }
            Event::KeyDown(Key::Enter | Key::Space | Key::ArrowDown)
                if self.interaction.focused =>
            {
                self.show_menu(ctx);
            }
            Event::AccessibilityAction { target, action } if *target == self.id => match action {
                SemanticAction::Click => self.show_menu(ctx),
                SemanticAction::Focus => {
                    ctx.request_focus(self.id);
                    ctx.set_handled();
                }
                SemanticAction::Increment | SemanticAction::Decrement => {}
            },
            Event::FocusChanged(focused) => {
                self.interaction.focused = *focused == Some(self.id);
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
        let color = if self.disabled {
            ctx.theme().colors.disabled
        } else if self.interaction.pressed {
            ctx.theme().colors.primary_pressed
        } else if self.interaction.hovered {
            ctx.theme().colors.secondary_hover
        } else {
            ctx.theme().colors.secondary
        };
        ctx.push(PaintOp::FillRect {
            rect: self.bounds,
            color,
            radius: ctx.theme().radii.md,
        });
        ctx.push(PaintOp::StrokeRect {
            rect: self.bounds,
            color: if self.interaction.focused {
                ctx.theme().focus
            } else {
                ctx.theme().colors.border
            },
            width: if self.interaction.focused { 2.0 } else { 1.0 },
            radius: ctx.theme().radii.md,
        });
        paint_text_layout(
            ctx,
            &self.layout,
            Point::new(
                self.bounds.x + 12.0,
                self.bounds.y + (self.bounds.height - self.layout.size.height) * 0.5,
            ),
            if self.disabled {
                ctx.theme().colors.text_muted
            } else {
                ctx.theme().text
            },
            15.0,
        );
        let cx = self.bounds.x + self.bounds.width - 18.0;
        ctx.push(PaintOp::FillPath {
            commands: vec![
                crate::PathCommand::MoveTo(Point::new(cx - 5.0, self.bounds.y + 16.0)),
                crate::PathCommand::LineTo(Point::new(cx, self.bounds.y + 22.0)),
                crate::PathCommand::LineTo(Point::new(cx + 5.0, self.bounds.y + 16.0)),
            ],
            color: ctx.theme().text,
        });
    }
    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        let value = self
            .options
            .get(self.selected.get())
            .map(|option| option.label.clone());
        ctx.push(SemanticNode {
            id: self.id,
            role: SemanticRole::ComboBox,
            bounds: self.bounds,
            label: self.label.clone(),
            disabled: self.disabled,
            focused: self.interaction.focused,
            checked: None,
            value,
            ime_cursor_area: None,
        });
    }
}

/// An indeterminate activity indicator driven by animation frames.
pub struct Spinner {
    id: WidgetId,
    label: String,
    phase: f64,
    size: f64,
    bounds: Rect,
}

impl Spinner {
    #[must_use]
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            id: WidgetId::next(),
            label: label.into(),
            phase: 0.0,
            size: 24.0,
            bounds: Rect::default(),
        }
    }
    #[must_use]
    pub fn size(mut self, size: f64) -> Self {
        self.size = size.max(8.0);
        self
    }
}

impl Widget for Spinner {
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
    fn event(&mut self, ctx: &mut EventCtx, event: &Event) {
        if let Event::AnimationFrame(frame) = event {
            self.phase = (self.phase + frame.delta.as_secs_f64() * 1.5) % 1.0;
            ctx.request_paint();
        }
        ctx.request_animation_frame();
    }
    fn widget_id(&self) -> Option<WidgetId> {
        Some(self.id)
    }
    fn reconcile(&mut self, next: &mut dyn Widget) {
        if let Some(next) = next.as_any_mut().downcast_mut::<Self>() {
            next.id = self.id;
            next.phase = self.phase;
        }
    }
    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let cx = self.bounds.x + self.bounds.width * 0.5;
        let cy = self.bounds.y + self.bounds.height * 0.5;
        let radius = self.bounds.width.min(self.bounds.height) * 0.35;
        for segment in 0_u32..8 {
            let angle = TAU * (f64::from(segment) / 8.0 + self.phase);
            let segment_start = f64::from(segment) / 8.0;
            let active = self.phase >= segment_start && self.phase < segment_start + 0.125;
            ctx.push(PaintOp::FillRect {
                rect: Rect::new(
                    cx + angle.cos() * radius - 2.0,
                    cy + angle.sin() * radius - 2.0,
                    4.0,
                    4.0,
                ),
                color: if active {
                    ctx.theme().colors.primary
                } else {
                    ctx.theme().colors.secondary
                },
                radius: 2.0,
            });
        }
    }
    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        ctx.push(SemanticNode {
            id: self.id,
            role: SemanticRole::ProgressIndicator,
            bounds: self.bounds,
            label: self.label.clone(),
            disabled: false,
            focused: false,
            checked: None,
            value: None,
            ime_cursor_area: None,
        });
    }
}

/// Shows a small text bubble while its child is hovered or focused.
pub struct Tooltip {
    child: Box<dyn Widget>,
    text: String,
    hovered: bool,
    focused: bool,
    layout: TextLayout,
    bounds: Rect,
}

impl Tooltip {
    #[must_use]
    pub fn new(text: impl Into<String>, child: impl Widget + 'static) -> Self {
        Self {
            child: Box::new(child),
            text: text.into(),
            hovered: false,
            focused: false,
            layout: TextLayout::default(),
            bounds: Rect::default(),
        }
    }
}

impl Widget for Tooltip {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let size = self.child.layout(ctx, constraints);
        self.layout = ctx.layout_text(&self.text, 13.0, None);
        self.bounds.width = size.width;
        self.bounds.height = size.height;
        self.child
            .set_origin(Point::new(self.bounds.x, self.bounds.y));
        size
    }
    fn set_origin(&mut self, origin: Point) {
        self.bounds.x = origin.x;
        self.bounds.y = origin.y;
        self.child.set_origin(origin);
    }
    fn bounds(&self) -> Rect {
        self.bounds
    }
    fn event(&mut self, ctx: &mut EventCtx, event: &Event) {
        match event {
            Event::PointerMoved(position) => {
                let hovered = self.bounds.contains(*position);
                if hovered != self.hovered {
                    self.hovered = hovered;
                    ctx.request_paint();
                }
                if hovered {
                    self.child.event(ctx, event);
                }
            }
            Event::PointerLeft => {
                self.hovered = false;
                self.child.event(ctx, event);
                ctx.request_paint();
            }
            Event::FocusChanged(_) => {
                self.child.event(ctx, event);
                let mut nodes = Vec::new();
                self.child.semantics(&mut SemanticsCtx::new(&mut nodes));
                let focused = nodes.iter().any(|node| node.focused);
                if focused != self.focused {
                    self.focused = focused;
                    ctx.request_paint();
                }
            }
            _ => self.child.event(ctx, event),
        }
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
            next.hovered = self.hovered;
            next.focused = self.focused;
        }
    }
    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        self.child.paint(ctx);
        if self.hovered || self.focused {
            let width = self.layout.size.width + 16.0;
            let height = self.layout.size.height + 10.0;
            let x = self.bounds.x + (self.bounds.width - width) * 0.5;
            let y = (self.bounds.y - height - 6.0).max(0.0);
            let bubble = Rect::new(x, y, width, height);
            ctx.push(PaintOp::FillRect {
                rect: bubble,
                color: ctx.theme().colors.surface_raised,
                radius: ctx.theme().radii.sm,
            });
            ctx.push(PaintOp::StrokeRect {
                rect: bubble,
                color: ctx.theme().colors.border,
                width: 1.0,
                radius: ctx.theme().radii.sm,
            });
            paint_text_layout(
                ctx,
                &self.layout,
                Point::new(x + 8.0, y + 5.0),
                ctx.theme().text,
                13.0,
            );
        }
    }
    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        self.child.semantics(ctx);
    }
}

/// A uniform-column layout for calendar and settings surfaces.
pub struct Grid {
    children: Vec<Box<dyn Widget>>,
    columns: usize,
    column_gap: f64,
    row_gap: f64,
    row_heights: Vec<f64>,
    bounds: Rect,
    hovered: Option<usize>,
}

impl Grid {
    #[must_use]
    pub fn new(columns: usize, children: impl IntoIterator<Item = Box<dyn Widget>>) -> Self {
        Self {
            children: children.into_iter().collect(),
            columns: columns.max(1),
            column_gap: 8.0,
            row_gap: 8.0,
            row_heights: Vec::new(),
            bounds: Rect::default(),
            hovered: None,
        }
    }
    #[must_use]
    pub fn gaps(mut self, column: f64, row: f64) -> Self {
        self.column_gap = column.max(0.0);
        self.row_gap = row.max(0.0);
        self
    }
}

impl Widget for Grid {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let columns_f = f64::from(u32::try_from(self.columns).unwrap_or(u32::MAX));
        let gap_width = self.column_gap * columns_f.saturating_sub(1.0);
        let width = if constraints.max.width.is_finite() {
            constraints.max.width
        } else {
            400.0
        };
        let cell_width = ((width - gap_width) / columns_f).max(0.0);
        let rows = self.children.len().div_ceil(self.columns);
        self.row_heights = vec![0.0; rows];
        for (index, child) in self.children.iter_mut().enumerate() {
            let child_size = child.layout(
                ctx,
                BoxConstraints::loose(Size::new(cell_width, constraints.max.height)),
            );
            self.row_heights[index / self.columns] =
                self.row_heights[index / self.columns].max(child_size.height);
        }
        let height = self.row_heights.iter().sum::<f64>()
            + self.row_gap * f64::from(u32::try_from(rows.saturating_sub(1)).unwrap_or(u32::MAX));
        let size = constraints.constrain(Size::new(width, height));
        self.bounds.width = size.width;
        self.bounds.height = size.height;
        let mut row_y = self.bounds.y;
        for (index, child) in self.children.iter_mut().enumerate() {
            let row = index / self.columns;
            if index % self.columns == 0 && row > 0 {
                row_y += self.row_heights[row - 1] + self.row_gap;
            }
            let column = index % self.columns;
            let x = self.bounds.x
                + f64::from(u32::try_from(column).unwrap_or(u32::MAX))
                    * (cell_width + self.column_gap);
            child.set_origin(Point::new(x, row_y));
        }
        size
    }
    fn set_origin(&mut self, origin: Point) {
        let dx = origin.x - self.bounds.x;
        let dy = origin.y - self.bounds.y;
        self.bounds.x = origin.x;
        self.bounds.y = origin.y;
        for child in &mut self.children {
            let bounds = child.bounds();
            child.set_origin(Point::new(bounds.x + dx, bounds.y + dy));
        }
    }
    fn bounds(&self) -> Rect {
        self.bounds
    }
    fn event(&mut self, ctx: &mut EventCtx, event: &Event) {
        match event {
            Event::PointerMoved(position) => {
                let next = self
                    .children
                    .iter()
                    .rposition(|child| child.bounds().contains(*position));
                if self.hovered != next {
                    if let Some(previous) = self.hovered.take() {
                        self.children[previous].event(ctx, &Event::PointerLeft);
                    }
                    self.hovered = next;
                }
                if let Some(index) = next {
                    self.children[index].event(ctx, event);
                }
            }
            Event::PointerLeft => {
                if let Some(previous) = self.hovered.take() {
                    self.children[previous].event(ctx, event);
                }
            }
            Event::PointerDown { position, .. }
            | Event::PointerUp { position, .. }
            | Event::Scroll { position, .. } => {
                if let Some(child) = self
                    .children
                    .iter_mut()
                    .rfind(|child| child.bounds().contains(*position))
                {
                    child.event(ctx, event);
                }
            }
            _ => {
                for child in &mut self.children {
                    child.event(ctx, event);
                    if ctx.is_handled() {
                        break;
                    }
                }
            }
        }
    }
    fn event_target(&mut self, ctx: &mut EventCtx, event: &Event, target: WidgetId) -> bool {
        self.children
            .iter_mut()
            .rev()
            .any(|child| child.event_target(ctx, event, target))
    }
    fn apply_command(&mut self, mut command: UiCommand) -> Result<(), UiCommand> {
        for child in self.children.iter_mut().rev() {
            match child.apply_command(command) {
                Ok(()) => return Ok(()),
                Err(returned) => command = returned,
            }
        }
        Err(command)
    }
    fn reconcile(&mut self, next: &mut dyn Widget) {
        if let Some(next) = next.as_any_mut().downcast_mut::<Self>() {
            for (current, next_child) in self.children.iter_mut().zip(&mut next.children) {
                current.reconcile(next_child.as_mut());
            }
            next.hovered = self.hovered.filter(|index| *index < next.children.len());
        }
    }
    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        for child in &self.children {
            child.paint(ctx);
        }
    }
    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        for child in &self.children {
            child.semantics(ctx);
        }
    }
}

trait SaturatingSubF64 {
    fn saturating_sub(self, rhs: f64) -> f64;
}
impl SaturatingSubF64 for f64 {
    fn saturating_sub(self, rhs: f64) -> f64 {
        (self - rhs).max(0.0)
    }
}
