use crate::widgets::paint_text_layout;
use crate::{
    BoxConstraints, Color, Event, EventCtx, Insets, Key, LayoutCtx, OverlayHost, OverlayPlacement,
    PaintCtx, PaintOp, PathCommand, Point, Rect, SemanticAction, SemanticNode, SemanticRole,
    SemanticsCtx, Size, Surface, SurfaceTone, TextField, TextLayout, UiCommand, Widget, WidgetId,
    WindowCommand,
};

/// An anchored, viewport-aware floating surface.
pub struct Popover {
    inner: OverlayHost,
}

impl Popover {
    #[must_use]
    pub fn new(
        base: impl Widget + 'static,
        anchor: Rect,
        placement: OverlayPlacement,
        content: impl Widget + 'static,
    ) -> Self {
        let surface = Surface::new(crate::Padding::new(Insets::all(12.0), content))
            .tone(SurfaceTone::Raised)
            .elevation(2)
            .radius(10.0);
        Self {
            inner: OverlayHost::new(base).overlay(anchor, placement, surface),
        }
    }

    #[must_use]
    pub fn modal(mut self, modal: bool) -> Self {
        self.inner = self.inner.modal(modal);
        self
    }

    #[must_use]
    pub fn dismiss_on_outside_click(mut self, dismiss: bool) -> Self {
        self.inner = self.inner.dismiss_on_outside_click(dismiss);
        self
    }

    #[must_use]
    pub fn on_dismiss(mut self, callback: impl FnMut() + 'static) -> Self {
        self.inner = self.inner.on_dismiss(callback);
        self
    }
}

impl Widget for Popover {
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
        self.inner.event(ctx, event);
    }
    fn event_target(&mut self, ctx: &mut EventCtx, event: &Event, target: WidgetId) -> bool {
        self.inner.event_target(ctx, event, target)
    }
    fn apply_command(&mut self, command: UiCommand) -> Result<(), UiCommand> {
        self.inner.apply_command(command)
    }
    fn reconcile(&mut self, next: &mut dyn Widget) {
        if let Some(next) = next.as_any_mut().downcast_mut::<Self>() {
            self.inner.reconcile(&mut next.inner);
        }
    }
    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        self.inner.paint(ctx);
    }
    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        self.inner.semantics(ctx);
    }
}

/// A text field specialized for incremental search with an accessible clear control.
pub struct SearchField {
    id: WidgetId,
    clear_id: WidgetId,
    field: TextField,
    bounds: Rect,
    clear_bounds: Rect,
    clear_pressed: bool,
    clear_focused: bool,
}

impl SearchField {
    #[must_use]
    pub fn new(value: impl Into<String>, on_change: impl FnMut(&str) + 'static) -> Self {
        Self {
            id: WidgetId::next(),
            clear_id: WidgetId::next(),
            field: TextField::new(value, on_change).placeholder("Search"),
            bounds: Rect::default(),
            clear_bounds: Rect::default(),
            clear_pressed: false,
            clear_focused: false,
        }
    }

    #[must_use]
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.field = self.field.placeholder(placeholder);
        self
    }

    #[must_use]
    pub fn value(&self) -> &str {
        self.field.value()
    }

    fn clear(&mut self, ctx: &mut EventCtx) {
        self.field.replace_value(String::new(), ctx);
        ctx.request_focus(self.field.widget_id().expect("text fields have stable IDs"));
        ctx.set_handled();
    }
}

impl Widget for SearchField {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let size = constraints.constrain(Size::new(280.0, 38.0));
        self.bounds.width = size.width;
        self.bounds.height = size.height;
        let field_width = (size.width - 58.0).max(0.0);
        self.field.layout(
            ctx,
            BoxConstraints::tight(Size::new(field_width, size.height)),
        );
        self.set_origin(Point::new(self.bounds.x, self.bounds.y));
        size
    }

    fn set_origin(&mut self, origin: Point) {
        self.bounds.x = origin.x;
        self.bounds.y = origin.y;
        self.field.set_origin(Point::new(origin.x + 29.0, origin.y));
        self.clear_bounds = Rect::new(origin.x + self.bounds.width - 29.0, origin.y, 29.0, 38.0);
    }

    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn widget_id(&self) -> Option<WidgetId> {
        Some(self.id)
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &Event) {
        match event {
            Event::PointerDown {
                position,
                button: crate::PointerButton::Primary,
                ..
            } if !self.field.value().is_empty() && self.clear_bounds.contains(*position) => {
                self.clear_pressed = true;
                ctx.capture_pointer(self.id);
                ctx.request_focus(self.clear_id);
                ctx.set_handled();
                ctx.request_paint();
            }
            Event::PointerUp {
                position,
                button: crate::PointerButton::Primary,
            } if self.clear_pressed => {
                let activate = self.clear_bounds.contains(*position);
                self.clear_pressed = false;
                ctx.release_pointer();
                ctx.set_handled();
                ctx.request_paint();
                if activate {
                    self.clear(ctx);
                }
            }
            Event::KeyDown(Key::Enter | Key::Space) if self.clear_focused => self.clear(ctx),
            Event::FocusChanged(focused) => {
                self.clear_focused = *focused == Some(self.clear_id);
                self.field.event(ctx, event);
            }
            Event::AccessibilityAction { target, action }
                if *target == self.clear_id && *action == SemanticAction::Click =>
            {
                self.clear(ctx);
            }
            Event::PointerMoved(position) if self.field.bounds().contains(*position) => {
                self.field.event(ctx, event);
            }
            Event::PointerDown { position, .. }
            | Event::PointerUp { position, .. }
            | Event::Scroll { position, .. }
                if self.field.bounds().contains(*position) =>
            {
                self.field.event(ctx, event);
            }
            Event::PointerMoved(_) | Event::PointerDown { .. } | Event::PointerUp { .. } => {}
            _ => self.field.event(ctx, event),
        }
    }

    fn event_target(&mut self, ctx: &mut EventCtx, event: &Event, target: WidgetId) -> bool {
        if target == self.id || target == self.clear_id {
            self.event(ctx, event);
            true
        } else {
            self.field.event_target(ctx, event, target)
        }
    }

    fn reconcile(&mut self, next: &mut dyn Widget) {
        if let Some(next) = next.as_any_mut().downcast_mut::<Self>() {
            next.id = self.id;
            next.clear_id = self.clear_id;
            next.clear_focused = self.clear_focused;
            self.field.reconcile(&mut next.field);
        }
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        self.field.paint(ctx);
        let icon = ctx.theme().colors.text_muted;
        ctx.push(PaintOp::StrokePath {
            commands: vec![
                PathCommand::MoveTo(Point::new(self.bounds.x + 10.0, self.bounds.y + 13.0)),
                PathCommand::LineTo(Point::new(self.bounds.x + 15.0, self.bounds.y + 18.0)),
                PathCommand::LineTo(Point::new(self.bounds.x + 10.0, self.bounds.y + 23.0)),
                PathCommand::LineTo(Point::new(self.bounds.x + 5.0, self.bounds.y + 18.0)),
                PathCommand::LineTo(Point::new(self.bounds.x + 10.0, self.bounds.y + 13.0)),
                PathCommand::MoveTo(Point::new(self.bounds.x + 14.0, self.bounds.y + 22.0)),
                PathCommand::LineTo(Point::new(self.bounds.x + 20.0, self.bounds.y + 28.0)),
            ],
            color: icon,
            width: 1.8,
        });
        if !self.field.value().is_empty() {
            let color = if self.clear_pressed || self.clear_focused {
                ctx.theme().focus
            } else {
                icon
            };
            let center = Point::new(self.clear_bounds.x + 14.0, self.clear_bounds.y + 19.0);
            ctx.push(PaintOp::StrokePath {
                commands: vec![
                    PathCommand::MoveTo(Point::new(center.x - 4.0, center.y - 4.0)),
                    PathCommand::LineTo(Point::new(center.x + 4.0, center.y + 4.0)),
                    PathCommand::MoveTo(Point::new(center.x + 4.0, center.y - 4.0)),
                    PathCommand::LineTo(Point::new(center.x - 4.0, center.y + 4.0)),
                ],
                color,
                width: 1.8,
            });
        }
    }

    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        self.field.semantics(ctx);
        if !self.field.value().is_empty() {
            ctx.push(SemanticNode {
                id: self.clear_id,
                role: SemanticRole::Button,
                bounds: self.clear_bounds,
                label: "Clear search".to_owned(),
                disabled: false,
                focused: self.clear_focused,
                checked: None,
                value: None,
                ime_cursor_area: None,
            });
        }
    }
}

type ToastCallback = Box<dyn FnMut()>;

/// One alert in a [`ToastStack`].
pub struct Toast {
    title: String,
    message: String,
    alert_id: WidgetId,
    close_id: WidgetId,
    action_id: WidgetId,
    action_label: Option<String>,
    on_action: Option<ToastCallback>,
    on_dismiss: Option<ToastCallback>,
    bounds: Rect,
    close_bounds: Rect,
    action_bounds: Rect,
    title_layout: TextLayout,
    message_layout: TextLayout,
    action_layout: TextLayout,
}

impl Toast {
    #[must_use]
    pub fn new(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            alert_id: WidgetId::next(),
            close_id: WidgetId::next(),
            action_id: WidgetId::next(),
            action_label: None,
            on_action: None,
            on_dismiss: None,
            bounds: Rect::default(),
            close_bounds: Rect::default(),
            action_bounds: Rect::default(),
            title_layout: TextLayout::default(),
            message_layout: TextLayout::default(),
            action_layout: TextLayout::default(),
        }
    }

    #[must_use]
    pub fn action(mut self, label: impl Into<String>, callback: impl FnMut() + 'static) -> Self {
        self.action_label = Some(label.into());
        self.on_action = Some(Box::new(callback));
        self
    }

    #[must_use]
    pub fn on_dismiss(mut self, callback: impl FnMut() + 'static) -> Self {
        self.on_dismiss = Some(Box::new(callback));
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToastHit {
    Close,
    Action,
}

/// A compact stack of actionable, individually dismissible notifications.
pub struct ToastStack {
    id: WidgetId,
    toasts: Vec<Toast>,
    bounds: Rect,
    pressed: Option<(usize, ToastHit)>,
    focused: Option<WidgetId>,
}

impl ToastStack {
    #[must_use]
    pub fn new(toasts: impl IntoIterator<Item = Toast>) -> Self {
        Self {
            id: WidgetId::next(),
            toasts: toasts.into_iter().collect(),
            bounds: Rect::default(),
            pressed: None,
            focused: None,
        }
    }

    pub fn push(&mut self, toast: Toast) {
        self.toasts.push(toast);
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.toasts.is_empty()
    }

    fn hit(&self, point: Point) -> Option<(usize, ToastHit)> {
        self.toasts.iter().enumerate().find_map(|(index, toast)| {
            if toast.close_bounds.contains(point) {
                Some((index, ToastHit::Close))
            } else if toast.action_label.is_some() && toast.action_bounds.contains(point) {
                Some((index, ToastHit::Action))
            } else {
                None
            }
        })
    }

    fn activate(&mut self, index: usize, hit: ToastHit, ctx: &mut EventCtx) {
        if index >= self.toasts.len() {
            return;
        }
        match hit {
            ToastHit::Close => {
                let mut toast = self.toasts.remove(index);
                if let Some(callback) = toast.on_dismiss.as_mut() {
                    callback();
                }
                self.focused = None;
                ctx.request_layout();
            }
            ToastHit::Action => {
                if let Some(callback) = self.toasts[index].on_action.as_mut() {
                    callback();
                }
            }
        }
        ctx.set_handled();
        ctx.request_paint();
    }
}

impl Widget for ToastStack {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let width = constraints.constrain(Size::new(360.0, 0.0)).width;
        let mut y = 0.0;
        for toast in &mut self.toasts {
            toast.title_layout = ctx.layout_text(&toast.title, 16.0, Some(288.0));
            toast.message_layout = ctx.layout_text(&toast.message, 14.0, Some(328.0));
            toast.action_layout = toast
                .action_label
                .as_ref()
                .map_or_else(TextLayout::default, |label| {
                    ctx.layout_text(label, 14.0, None)
                });
            let action_height = if toast.action_label.is_some() {
                32.0
            } else {
                0.0
            };
            let height = (toast.title_layout.size.height
                + toast.message_layout.size.height
                + action_height
                + 32.0)
                .max(76.0);
            toast.bounds = Rect::new(self.bounds.x, self.bounds.y + y, width, height);
            toast.close_bounds = Rect::new(
                toast.bounds.x + width - 42.0,
                toast.bounds.y + 8.0,
                34.0,
                34.0,
            );
            toast.action_bounds = Rect::new(
                toast.bounds.x + 12.0,
                toast.bounds.y + height - 36.0,
                (toast.action_layout.size.width + 24.0).max(76.0),
                28.0,
            );
            y += height + 10.0;
        }
        if !self.toasts.is_empty() {
            y -= 10.0;
        }
        let size = constraints.constrain(Size::new(width, y));
        self.bounds.width = size.width;
        self.bounds.height = size.height;
        size
    }

    fn set_origin(&mut self, origin: Point) {
        let dx = origin.x - self.bounds.x;
        let dy = origin.y - self.bounds.y;
        self.bounds.x = origin.x;
        self.bounds.y = origin.y;
        for toast in &mut self.toasts {
            toast.bounds.x += dx;
            toast.bounds.y += dy;
            toast.close_bounds.x += dx;
            toast.close_bounds.y += dy;
            toast.action_bounds.x += dx;
            toast.action_bounds.y += dy;
        }
    }

    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn widget_id(&self) -> Option<WidgetId> {
        Some(self.id)
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &Event) {
        match event {
            Event::PointerDown {
                position,
                button: crate::PointerButton::Primary,
                ..
            } => {
                if let Some(hit) = self.hit(*position) {
                    self.pressed = Some(hit);
                    let toast = &self.toasts[hit.0];
                    let id = match hit.1 {
                        ToastHit::Close => toast.close_id,
                        ToastHit::Action => toast.action_id,
                    };
                    ctx.request_focus(id);
                    ctx.capture_pointer(self.id);
                    ctx.set_handled();
                    ctx.request_paint();
                }
            }
            Event::PointerUp {
                position,
                button: crate::PointerButton::Primary,
            } => {
                let pressed = self.pressed.take();
                ctx.release_pointer();
                if pressed.is_some() {
                    ctx.set_handled();
                    ctx.request_paint();
                }
                if pressed == self.hit(*position)
                    && let Some((index, hit)) = pressed
                {
                    self.activate(index, hit, ctx);
                }
            }
            Event::FocusChanged(focused) => {
                self.focused = *focused;
                ctx.request_paint();
            }
            Event::KeyDown(Key::Enter | Key::Space) => {
                let hit = self.toasts.iter().enumerate().find_map(|(index, toast)| {
                    if self.focused == Some(toast.close_id) {
                        Some((index, ToastHit::Close))
                    } else if self.focused == Some(toast.action_id) {
                        Some((index, ToastHit::Action))
                    } else {
                        None
                    }
                });
                if let Some((index, hit)) = hit {
                    self.activate(index, hit, ctx);
                }
            }
            Event::AccessibilityAction { target, action } if *action == SemanticAction::Click => {
                let hit = self.toasts.iter().enumerate().find_map(|(index, toast)| {
                    if *target == toast.close_id {
                        Some((index, ToastHit::Close))
                    } else if *target == toast.action_id {
                        Some((index, ToastHit::Action))
                    } else {
                        None
                    }
                });
                if let Some((index, hit)) = hit {
                    self.activate(index, hit, ctx);
                }
            }
            _ => {}
        }
    }

    fn event_target(&mut self, ctx: &mut EventCtx, event: &Event, target: WidgetId) -> bool {
        if target == self.id
            || self
                .toasts
                .iter()
                .any(|toast| target == toast.close_id || target == toast.action_id)
        {
            self.event(ctx, event);
            true
        } else {
            false
        }
    }

    fn reconcile(&mut self, next: &mut dyn Widget) {
        if let Some(next) = next.as_any_mut().downcast_mut::<Self>() {
            next.id = self.id;
            for next_toast in &mut next.toasts {
                if let Some(current) = self
                    .toasts
                    .iter()
                    .find(|toast| toast.title == next_toast.title)
                {
                    next_toast.alert_id = current.alert_id;
                    next_toast.close_id = current.close_id;
                    next_toast.action_id = current.action_id;
                }
            }
        }
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        for (index, toast) in self.toasts.iter().enumerate() {
            ctx.push(PaintOp::FillRect {
                rect: toast.bounds,
                color: ctx.theme().colors.surface,
                radius: 10.0,
            });
            ctx.push(PaintOp::StrokeRect {
                rect: toast.bounds,
                color: ctx.theme().colors.border,
                width: 1.0,
                radius: 10.0,
            });
            paint_text_layout(
                ctx,
                &toast.title_layout,
                Point::new(toast.bounds.x + 14.0, toast.bounds.y + 12.0),
                ctx.theme().text,
                16.0,
            );
            paint_text_layout(
                ctx,
                &toast.message_layout,
                Point::new(toast.bounds.x + 14.0, toast.bounds.y + 36.0),
                ctx.theme().colors.text_muted,
                14.0,
            );
            let close_color = if self.focused == Some(toast.close_id)
                || self.pressed == Some((index, ToastHit::Close))
            {
                ctx.theme().focus
            } else {
                ctx.theme().colors.text_muted
            };
            ctx.push(PaintOp::StrokePath {
                commands: vec![
                    PathCommand::MoveTo(Point::new(
                        toast.close_bounds.x + 11.0,
                        toast.close_bounds.y + 11.0,
                    )),
                    PathCommand::LineTo(Point::new(
                        toast.close_bounds.x + 23.0,
                        toast.close_bounds.y + 23.0,
                    )),
                    PathCommand::MoveTo(Point::new(
                        toast.close_bounds.x + 23.0,
                        toast.close_bounds.y + 11.0,
                    )),
                    PathCommand::LineTo(Point::new(
                        toast.close_bounds.x + 11.0,
                        toast.close_bounds.y + 23.0,
                    )),
                ],
                color: close_color,
                width: 1.8,
            });
            if toast.action_label.is_some() {
                ctx.push(PaintOp::FillRect {
                    rect: toast.action_bounds,
                    color: ctx.theme().button,
                    radius: 6.0,
                });
                paint_text_layout(
                    ctx,
                    &toast.action_layout,
                    Point::new(toast.action_bounds.x + 12.0, toast.action_bounds.y + 5.0),
                    ctx.theme().text,
                    14.0,
                );
            }
        }
    }

    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        for toast in &self.toasts {
            ctx.push(SemanticNode {
                id: toast.alert_id,
                role: SemanticRole::Alert,
                bounds: toast.bounds,
                label: toast.title.clone(),
                disabled: false,
                focused: false,
                checked: None,
                value: Some(toast.message.clone()),
                ime_cursor_area: None,
            });
            ctx.push(SemanticNode {
                id: toast.close_id,
                role: SemanticRole::Button,
                bounds: toast.close_bounds,
                label: format!("Dismiss {}", toast.title),
                disabled: false,
                focused: self.focused == Some(toast.close_id),
                checked: None,
                value: None,
                ime_cursor_area: None,
            });
            if let Some(label) = &toast.action_label {
                ctx.push(SemanticNode {
                    id: toast.action_id,
                    role: SemanticRole::Button,
                    bounds: toast.action_bounds,
                    label: label.clone(),
                    disabled: false,
                    focused: self.focused == Some(toast.action_id),
                    checked: None,
                    value: None,
                    ime_cursor_area: None,
                });
            }
        }
    }
}

/// One expandable section in an [`Accordion`].
pub struct AccordionSection {
    title: String,
    header_id: WidgetId,
    expanded: bool,
    body: Box<dyn Widget>,
    header_bounds: Rect,
    title_layout: TextLayout,
}

impl AccordionSection {
    #[must_use]
    pub fn new(title: impl Into<String>, body: impl Widget + 'static) -> Self {
        Self {
            title: title.into(),
            header_id: WidgetId::next(),
            expanded: false,
            body: Box::new(body),
            header_bounds: Rect::default(),
            title_layout: TextLayout::default(),
        }
    }

    #[must_use]
    pub const fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = expanded;
        self
    }
}

/// Vertically stacked disclosure sections with retained expansion state.
pub struct Accordion {
    id: WidgetId,
    sections: Vec<AccordionSection>,
    bounds: Rect,
    focused: Option<usize>,
    pressed: Option<usize>,
}

impl Accordion {
    #[must_use]
    pub fn new(sections: impl IntoIterator<Item = AccordionSection>) -> Self {
        Self {
            id: WidgetId::next(),
            sections: sections.into_iter().collect(),
            bounds: Rect::default(),
            focused: None,
            pressed: None,
        }
    }

    fn toggle(&mut self, index: usize, ctx: &mut EventCtx) {
        if let Some(section) = self.sections.get_mut(index) {
            section.expanded = !section.expanded;
            ctx.request_focus(section.header_id);
            ctx.set_handled();
            ctx.request_layout();
        }
    }
}

impl Widget for Accordion {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let width = constraints.max.width;
        let width = if width.is_finite() { width } else { 320.0 };
        let mut y = 0.0;
        for section in &mut self.sections {
            section.title_layout = ctx.layout_text(&section.title, 15.0, None);
            section.header_bounds = Rect::new(self.bounds.x, self.bounds.y + y, width, 40.0);
            y += 40.0;
            if section.expanded {
                let available = Size::new((width - 24.0).max(0.0), constraints.max.height);
                let body_size = section.body.layout(ctx, BoxConstraints::loose(available));
                section
                    .body
                    .set_origin(Point::new(self.bounds.x + 12.0, self.bounds.y + y + 8.0));
                y += body_size.height + 16.0;
            }
        }
        let size = constraints.constrain(Size::new(width, y));
        self.bounds.width = size.width;
        self.bounds.height = size.height;
        size
    }

    fn set_origin(&mut self, origin: Point) {
        let dx = origin.x - self.bounds.x;
        let dy = origin.y - self.bounds.y;
        self.bounds.x = origin.x;
        self.bounds.y = origin.y;
        for section in &mut self.sections {
            section.header_bounds.x += dx;
            section.header_bounds.y += dy;
            let body = section.body.bounds();
            section
                .body
                .set_origin(Point::new(body.x + dx, body.y + dy));
        }
    }

    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn widget_id(&self) -> Option<WidgetId> {
        Some(self.id)
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &Event) {
        match event {
            Event::PointerDown {
                position,
                button: crate::PointerButton::Primary,
                ..
            } => {
                if let Some(index) = self
                    .sections
                    .iter()
                    .position(|section| section.header_bounds.contains(*position))
                {
                    self.pressed = Some(index);
                    ctx.request_focus(self.sections[index].header_id);
                    ctx.capture_pointer(self.id);
                    ctx.set_handled();
                    ctx.request_paint();
                } else if let Some(section) = self
                    .sections
                    .iter_mut()
                    .find(|section| section.expanded && section.body.bounds().contains(*position))
                {
                    section.body.event(ctx, event);
                }
            }
            Event::PointerUp {
                position,
                button: crate::PointerButton::Primary,
            } if self.pressed.is_some() => {
                let pressed = self.pressed.take();
                let released = self
                    .sections
                    .iter()
                    .position(|section| section.header_bounds.contains(*position));
                ctx.release_pointer();
                ctx.set_handled();
                ctx.request_paint();
                if pressed == released
                    && let Some(index) = pressed
                {
                    self.toggle(index, ctx);
                }
            }
            Event::PointerMoved(position)
            | Event::PointerUp { position, .. }
            | Event::Scroll { position, .. } => {
                if let Some(section) = self
                    .sections
                    .iter_mut()
                    .find(|section| section.expanded && section.body.bounds().contains(*position))
                {
                    section.body.event(ctx, event);
                }
            }
            Event::FocusChanged(focused) => {
                self.focused = focused.and_then(|id| {
                    self.sections
                        .iter()
                        .position(|section| section.header_id == id)
                });
                for section in &mut self.sections {
                    if section.expanded {
                        section.body.event(ctx, event);
                    }
                }
                ctx.request_paint();
            }
            Event::KeyDown(Key::Enter | Key::Space) => {
                if let Some(index) = self.focused {
                    self.toggle(index, ctx);
                }
            }
            Event::AccessibilityAction { target, action } if *action == SemanticAction::Click => {
                if let Some(index) = self
                    .sections
                    .iter()
                    .position(|section| section.header_id == *target)
                {
                    self.toggle(index, ctx);
                }
            }
            _ => {
                for section in &mut self.sections {
                    if section.expanded {
                        section.body.event(ctx, event);
                        if ctx.is_handled() {
                            break;
                        }
                    }
                }
            }
        }
    }

    fn event_target(&mut self, ctx: &mut EventCtx, event: &Event, target: WidgetId) -> bool {
        if target == self.id
            || self
                .sections
                .iter()
                .any(|section| section.header_id == target)
        {
            self.event(ctx, event);
            true
        } else {
            self.sections
                .iter_mut()
                .any(|section| section.expanded && section.body.event_target(ctx, event, target))
        }
    }

    fn apply_command(&mut self, mut command: UiCommand) -> Result<(), UiCommand> {
        for section in &mut self.sections {
            match section.body.apply_command(command) {
                Ok(()) => return Ok(()),
                Err(returned) => command = returned,
            }
        }
        Err(command)
    }

    fn reconcile(&mut self, next: &mut dyn Widget) {
        if let Some(next) = next.as_any_mut().downcast_mut::<Self>() {
            next.id = self.id;
            next.pressed = None;
            for next_section in &mut next.sections {
                if let Some(current) = self
                    .sections
                    .iter_mut()
                    .find(|section| section.title == next_section.title)
                {
                    next_section.header_id = current.header_id;
                    next_section.expanded = current.expanded;
                    current.body.reconcile(next_section.body.as_mut());
                }
            }
        }
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        for (index, section) in self.sections.iter().enumerate() {
            ctx.push(PaintOp::FillRect {
                rect: section.header_bounds,
                color: if self.focused == Some(index) {
                    ctx.theme().button_hover
                } else {
                    ctx.theme().button_disabled
                },
                radius: 6.0,
            });
            let arrow_x = section.header_bounds.x + 15.0;
            let arrow_y = section.header_bounds.y + 20.0;
            let commands = if section.expanded {
                vec![
                    PathCommand::MoveTo(Point::new(arrow_x - 4.0, arrow_y - 2.0)),
                    PathCommand::LineTo(Point::new(arrow_x, arrow_y + 2.0)),
                    PathCommand::LineTo(Point::new(arrow_x + 4.0, arrow_y - 2.0)),
                ]
            } else {
                vec![
                    PathCommand::MoveTo(Point::new(arrow_x - 2.0, arrow_y - 4.0)),
                    PathCommand::LineTo(Point::new(arrow_x + 2.0, arrow_y)),
                    PathCommand::LineTo(Point::new(arrow_x - 2.0, arrow_y + 4.0)),
                ]
            };
            ctx.push(PaintOp::StrokePath {
                commands,
                color: ctx.theme().colors.text_muted,
                width: 1.8,
            });
            paint_text_layout(
                ctx,
                &section.title_layout,
                Point::new(
                    section.header_bounds.x + 30.0,
                    section.header_bounds.y + 10.0,
                ),
                ctx.theme().text,
                15.0,
            );
            if section.expanded {
                section.body.paint(ctx);
            }
        }
    }

    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        ctx.push(SemanticNode {
            id: self.id,
            role: SemanticRole::Group,
            bounds: self.bounds,
            label: "Accordion".to_owned(),
            disabled: false,
            focused: false,
            checked: None,
            value: None,
            ime_cursor_area: None,
        });
        for (index, section) in self.sections.iter().enumerate() {
            ctx.push(SemanticNode {
                id: section.header_id,
                role: SemanticRole::Button,
                bounds: section.header_bounds,
                label: section.title.clone(),
                disabled: false,
                focused: self.focused == Some(index),
                checked: Some(section.expanded),
                value: None,
                ime_cursor_area: None,
            });
            if section.expanded {
                section.body.semantics(ctx);
            }
        }
    }
}

type ChromeCallback = Box<dyn FnMut(&mut EventCtx)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChromeAction {
    Minimize,
    Maximize,
    Close,
}

/// Compositor-friendly title bar with minimize, maximize, close, and drag hooks.
pub struct WindowChrome {
    id: WidgetId,
    title_id: WidgetId,
    button_ids: [WidgetId; 3],
    title: String,
    title_layout: TextLayout,
    bounds: Rect,
    button_bounds: [Rect; 3],
    pressed: Option<ChromeAction>,
    focused: Option<ChromeAction>,
    maximized: bool,
    callbacks: [Option<ChromeCallback>; 3],
    on_drag: Option<ChromeCallback>,
}

impl WindowChrome {
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            id: WidgetId::next(),
            title_id: WidgetId::next(),
            button_ids: [WidgetId::next(), WidgetId::next(), WidgetId::next()],
            title: title.into(),
            title_layout: TextLayout::default(),
            bounds: Rect::default(),
            button_bounds: [Rect::default(); 3],
            pressed: None,
            focused: None,
            maximized: false,
            callbacks: [None, None, None],
            on_drag: None,
        }
    }

    #[must_use]
    pub const fn maximized(mut self, maximized: bool) -> Self {
        self.maximized = maximized;
        self
    }

    #[must_use]
    pub fn on_minimize(mut self, callback: impl FnMut(&mut EventCtx) + 'static) -> Self {
        self.callbacks[0] = Some(Box::new(callback));
        self
    }

    #[must_use]
    pub fn on_maximize(mut self, callback: impl FnMut(&mut EventCtx) + 'static) -> Self {
        self.callbacks[1] = Some(Box::new(callback));
        self
    }

    #[must_use]
    pub fn on_close(mut self, callback: impl FnMut(&mut EventCtx) + 'static) -> Self {
        self.callbacks[2] = Some(Box::new(callback));
        self
    }

    #[must_use]
    pub fn on_drag(mut self, callback: impl FnMut(&mut EventCtx) + 'static) -> Self {
        self.on_drag = Some(Box::new(callback));
        self
    }

    fn hit(&self, point: Point) -> Option<ChromeAction> {
        self.button_bounds
            .iter()
            .position(|bounds| bounds.contains(point))
            .map(action_for_index)
    }

    fn activate(&mut self, action: ChromeAction, ctx: &mut EventCtx) {
        let index = action_index(action);
        if let Some(callback) = self.callbacks[index].as_mut() {
            callback(ctx);
        } else if let Some(window) = ctx.current_window() {
            match action {
                ChromeAction::Minimize => ctx.window_command(WindowCommand::Minimize(window, true)),
                ChromeAction::Maximize => {
                    self.maximized = !self.maximized;
                    ctx.window_command(WindowCommand::Maximize(window, self.maximized));
                }
                ChromeAction::Close => ctx.close_window(window),
            }
        }
        ctx.set_handled();
        ctx.request_paint();
    }
}

impl Widget for WindowChrome {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.title_layout = ctx.layout_text(&self.title, 15.0, None);
        let width = if constraints.max.width.is_finite() {
            constraints.max.width
        } else {
            self.title_layout.size.width + 150.0
        };
        let size = constraints.constrain(Size::new(width, 42.0));
        self.bounds.width = size.width;
        self.bounds.height = size.height;
        self.set_origin(Point::new(self.bounds.x, self.bounds.y));
        size
    }

    fn set_origin(&mut self, origin: Point) {
        self.bounds.x = origin.x;
        self.bounds.y = origin.y;
        let right = origin.x + self.bounds.width;
        for (bounds, offset) in self.button_bounds.iter_mut().zip([0.0, 42.0, 84.0]) {
            *bounds = Rect::new(right - 126.0 + offset, origin.y, 42.0, 42.0);
        }
    }

    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn widget_id(&self) -> Option<WidgetId> {
        Some(self.id)
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &Event) {
        match event {
            Event::PointerDown {
                position,
                button: crate::PointerButton::Primary,
                click_count,
            } if self.bounds.contains(*position) => {
                if let Some(action) = self.hit(*position) {
                    self.pressed = Some(action);
                    ctx.request_focus(self.button_ids[action_index(action)]);
                    ctx.capture_pointer(self.id);
                    ctx.set_handled();
                } else if *click_count >= 2 {
                    self.activate(ChromeAction::Maximize, ctx);
                } else {
                    if let Some(callback) = self.on_drag.as_mut() {
                        callback(ctx);
                    }
                    ctx.set_handled();
                }
                ctx.request_paint();
            }
            Event::PointerUp {
                position,
                button: crate::PointerButton::Primary,
            } if self.pressed.is_some() => {
                let pressed = self.pressed.take();
                ctx.release_pointer();
                ctx.set_handled();
                if pressed == self.hit(*position)
                    && let Some(action) = pressed
                {
                    self.activate(action, ctx);
                }
            }
            Event::FocusChanged(focused) => {
                self.focused = focused.and_then(|id| {
                    self.button_ids
                        .iter()
                        .position(|candidate| *candidate == id)
                        .map(action_for_index)
                });
                ctx.request_paint();
            }
            Event::KeyDown(Key::Enter | Key::Space) => {
                if let Some(action) = self.focused {
                    self.activate(action, ctx);
                }
            }
            Event::AccessibilityAction { target, action } if *action == SemanticAction::Click => {
                if let Some(index) = self
                    .button_ids
                    .iter()
                    .position(|candidate| candidate == target)
                {
                    self.activate(action_for_index(index), ctx);
                }
            }
            _ => {}
        }
    }

    fn event_target(&mut self, ctx: &mut EventCtx, event: &Event, target: WidgetId) -> bool {
        if target == self.id || self.button_ids.contains(&target) {
            self.event(ctx, event);
            true
        } else {
            false
        }
    }

    fn reconcile(&mut self, next: &mut dyn Widget) {
        if let Some(next) = next.as_any_mut().downcast_mut::<Self>() {
            next.id = self.id;
            next.title_id = self.title_id;
            next.button_ids = self.button_ids;
            next.maximized = self.maximized;
            next.focused = self.focused;
        }
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.push(PaintOp::FillRect {
            rect: self.bounds,
            color: ctx.theme().colors.surface,
            radius: 0.0,
        });
        paint_text_layout(
            ctx,
            &self.title_layout,
            Point::new(self.bounds.x + 14.0, self.bounds.y + 11.0),
            ctx.theme().text,
            15.0,
        );
        for (index, bounds) in self.button_bounds.iter().enumerate() {
            let action = action_for_index(index);
            if self.focused == Some(action) || self.pressed == Some(action) {
                ctx.push(PaintOp::FillRect {
                    rect: *bounds,
                    color: if action == ChromeAction::Close {
                        Color::rgb(190, 55, 65)
                    } else {
                        ctx.theme().button_hover
                    },
                    radius: 0.0,
                });
            }
            let center = Point::new(bounds.x + 21.0, bounds.y + 21.0);
            let commands = match action {
                ChromeAction::Minimize => vec![
                    PathCommand::MoveTo(Point::new(center.x - 6.0, center.y + 4.0)),
                    PathCommand::LineTo(Point::new(center.x + 6.0, center.y + 4.0)),
                ],
                ChromeAction::Maximize => vec![
                    PathCommand::MoveTo(Point::new(center.x - 6.0, center.y - 5.0)),
                    PathCommand::LineTo(Point::new(center.x + 6.0, center.y - 5.0)),
                    PathCommand::LineTo(Point::new(center.x + 6.0, center.y + 6.0)),
                    PathCommand::LineTo(Point::new(center.x - 6.0, center.y + 6.0)),
                    PathCommand::LineTo(Point::new(center.x - 6.0, center.y - 5.0)),
                ],
                ChromeAction::Close => vec![
                    PathCommand::MoveTo(Point::new(center.x - 5.0, center.y - 5.0)),
                    PathCommand::LineTo(Point::new(center.x + 5.0, center.y + 5.0)),
                    PathCommand::MoveTo(Point::new(center.x + 5.0, center.y - 5.0)),
                    PathCommand::LineTo(Point::new(center.x - 5.0, center.y + 5.0)),
                ],
            };
            ctx.push(PaintOp::StrokePath {
                commands,
                color: ctx.theme().text,
                width: 1.5,
            });
        }
    }

    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        ctx.push(SemanticNode {
            id: self.id,
            role: SemanticRole::Toolbar,
            bounds: self.bounds,
            label: "Window controls".to_owned(),
            disabled: false,
            focused: false,
            checked: None,
            value: None,
            ime_cursor_area: None,
        });
        ctx.push(SemanticNode {
            id: self.title_id,
            role: SemanticRole::Label,
            bounds: self.bounds,
            label: self.title.clone(),
            disabled: false,
            focused: false,
            checked: None,
            value: None,
            ime_cursor_area: None,
        });
        let labels = ["Minimize window", "Maximize window", "Close window"];
        for (index, id) in self.button_ids.iter().enumerate() {
            ctx.push(SemanticNode {
                id: *id,
                role: SemanticRole::Button,
                bounds: self.button_bounds[index],
                label: labels[index].to_owned(),
                disabled: false,
                focused: self.focused == Some(action_for_index(index)),
                checked: (index == 1).then_some(self.maximized),
                value: None,
                ime_cursor_area: None,
            });
        }
    }
}

const fn action_index(action: ChromeAction) -> usize {
    match action {
        ChromeAction::Minimize => 0,
        ChromeAction::Maximize => 1,
        ChromeAction::Close => 2,
    }
}

const fn action_for_index(index: usize) -> ChromeAction {
    match index {
        0 => ChromeAction::Minimize,
        1 => ChromeAction::Maximize,
        _ => ChromeAction::Close,
    }
}
