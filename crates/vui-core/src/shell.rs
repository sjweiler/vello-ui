use crate::{
    BoxConstraints, Button, ButtonVariant, Column, CommandDescriptor, DragOperation,
    DragOperations, DragPayload, DragPreview, Event, EventCtx, Flex, FlexChild, IconButton,
    IconName, Insets, Key, Label, LayoutCtx, Modifiers, PaintCtx, PaintOp, PathCommand, Point,
    Rect, SemanticNode, SemanticRole, SemanticsCtx, Size, Spacer, Surface, SurfaceTone, TextLayout,
    UiCommand, Widget, WidgetId,
};

fn move_child(bounds: &mut Rect, child: &mut dyn Widget, origin: Point) {
    let dx = origin.x - bounds.x;
    let dy = origin.y - bounds.y;
    bounds.x = origin.x;
    bounds.y = origin.y;
    let child_bounds = child.bounds();
    child.set_origin(Point::new(child_bounds.x + dx, child_bounds.y + dy));
}

struct Region {
    id: WidgetId,
    role: SemanticRole,
    label: String,
    child: Box<dyn Widget>,
    bounds: Rect,
}

impl Region {
    fn new(role: SemanticRole, label: impl Into<String>, child: impl Widget + 'static) -> Self {
        Self {
            id: WidgetId::next(),
            role,
            label: label.into(),
            child: Box::new(child),
            bounds: Rect::default(),
        }
    }
}

impl Widget for Region {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let size = self.child.layout(ctx, constraints);
        self.child
            .set_origin(Point::new(self.bounds.x, self.bounds.y));
        self.bounds.width = size.width;
        self.bounds.height = size.height;
        size
    }
    fn set_origin(&mut self, origin: Point) {
        move_child(&mut self.bounds, self.child.as_mut(), origin);
    }
    fn bounds(&self) -> Rect {
        self.bounds
    }
    fn widget_id(&self) -> Option<WidgetId> {
        Some(self.id)
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
            next.id = self.id;
            self.child.reconcile(next.child.as_mut());
        }
    }
    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        self.child.paint(ctx);
    }
    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        ctx.push(SemanticNode {
            id: self.id,
            role: self.role,
            bounds: self.bounds,
            label: self.label.clone(),
            disabled: false,
            focused: false,
            checked: None,
            value: None,
            ime_cursor_area: None,
        });
        self.child.semantics(ctx);
    }
}

macro_rules! region_widget {
    ($name:ident) => {
        impl Widget for $name {
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
            fn event_target(
                &mut self,
                ctx: &mut EventCtx,
                event: &Event,
                target: WidgetId,
            ) -> bool {
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
    };
}

pub struct Toolbar {
    inner: Region,
}
impl Toolbar {
    #[must_use]
    pub fn new(items: impl IntoIterator<Item = Box<dyn Widget>>) -> Self {
        let flex = Flex::row(items.into_iter().map(FlexChild::from)).gap(6.0);
        let surface =
            Surface::new(crate::Padding::new(Insets::all(6.0), flex)).tone(SurfaceTone::Raised);
        Self {
            inner: Region::new(SemanticRole::Toolbar, "Toolbar", surface),
        }
    }
}
region_widget!(Toolbar);

pub struct Sidebar {
    inner: Region,
}
impl Sidebar {
    #[must_use]
    pub fn new(items: impl IntoIterator<Item = Box<dyn Widget>>) -> Self {
        let column = Column::new(items).gap(6.0);
        let surface =
            Surface::new(crate::Padding::new(Insets::all(10.0), column)).tone(SurfaceTone::Raised);
        Self {
            inner: Region::new(SemanticRole::Navigation, "Sidebar", surface),
        }
    }
}
region_widget!(Sidebar);

pub struct StatusBar {
    inner: Region,
}
impl StatusBar {
    #[must_use]
    pub fn new(items: impl IntoIterator<Item = Box<dyn Widget>>) -> Self {
        let flex = Flex::row(items.into_iter().map(FlexChild::from)).gap(10.0);
        let surface = Surface::new(crate::Padding::new(
            Insets {
                left: 10.0,
                top: 4.0,
                right: 10.0,
                bottom: 4.0,
            },
            flex,
        ));
        Self {
            inner: Region::new(SemanticRole::StatusBar, "Status", surface),
        }
    }
}
region_widget!(StatusBar);

/// A raised compositor/application panel with an optional overlay close control.
pub struct Panel {
    inner: Region,
}

impl Panel {
    #[must_use]
    pub fn new(title: impl Into<String>, body: impl Widget + 'static) -> Self {
        Self::build(title.into(), body, false)
    }

    #[must_use]
    pub fn dismissible(title: impl Into<String>, body: impl Widget + 'static) -> Self {
        Self::build(title.into(), body, true)
    }

    fn build(title: String, body: impl Widget + 'static, dismissible: bool) -> Self {
        let header: Box<dyn Widget> = if dismissible {
            Box::new(
                Flex::row([
                    FlexChild::fixed(Label::new(title.clone()).text_size(20.0)),
                    FlexChild::flexible(1.0, Spacer::new(0.0, 0.0)),
                    FlexChild::fixed(IconButton::action(
                        IconName::Close,
                        "Close panel",
                        |_, ctx| ctx.dismiss_overlay(),
                    )),
                ])
                .gap(8.0),
            )
        } else {
            Box::new(Label::new(title.clone()).text_size(20.0))
        };
        let content = Column::new([header, Box::new(body) as Box<dyn Widget>]).gap(14.0);
        let surface = Surface::new(crate::Padding::new(Insets::all(18.0), content))
            .tone(SurfaceTone::Raised)
            .elevation(2)
            .radius(12.0);
        Self {
            inner: Region::new(SemanticRole::Group, title, surface),
        }
    }
}
region_widget!(Panel);

/// A dismissible alert card with optional action buttons.
pub struct NotificationCard {
    inner: Region,
}

impl NotificationCard {
    #[must_use]
    pub fn new(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self::with_actions(title, message, std::iter::empty::<Button>())
    }

    #[must_use]
    pub fn with_actions(
        title: impl Into<String>,
        message: impl Into<String>,
        actions: impl IntoIterator<Item = Button>,
    ) -> Self {
        let title = title.into();
        let header = Flex::row([
            FlexChild::fixed(Label::new(title.clone()).text_size(17.0)),
            FlexChild::flexible(1.0, Spacer::new(0.0, 0.0)),
            FlexChild::fixed(IconButton::action(
                IconName::Close,
                "Dismiss notification",
                |_, ctx| ctx.dismiss_overlay(),
            )),
        ])
        .gap(8.0);
        let mut children = vec![
            Box::new(header) as Box<dyn Widget>,
            Box::new(Label::new(message)) as Box<dyn Widget>,
        ];
        let actions: Vec<_> = actions.into_iter().map(FlexChild::fixed).collect();
        if !actions.is_empty() {
            children.push(Box::new(Flex::row(actions).gap(8.0)));
        }
        let content = Column::new(children).gap(10.0);
        let surface = Surface::new(crate::Padding::new(Insets::all(14.0), content))
            .tone(SurfaceTone::Raised)
            .elevation(2)
            .radius(10.0);
        Self {
            inner: Region::new(SemanticRole::Alert, title, surface),
        }
    }
}
region_widget!(NotificationCard);

pub struct NavigationItem {
    button: Button,
}
impl NavigationItem {
    #[must_use]
    pub fn new(label: impl Into<String>, on_activate: impl FnMut() + 'static) -> Self {
        Self {
            button: Button::new(label, on_activate).variant(ButtonVariant::Ghost),
        }
    }
    #[must_use]
    pub fn selected(mut self, selected: bool) -> Self {
        self.button = self.button.selected(selected);
        self
    }

    #[must_use]
    pub fn command(command: CommandDescriptor) -> Self {
        let id = command.id.clone();
        Self {
            button: Button::action(command.label, move |_, ctx| {
                ctx.invoke_command(id.clone());
            })
            .variant(ButtonVariant::Ghost)
            .disabled(!command.enabled)
            .selected(command.selected),
        }
    }
}
impl Widget for NavigationItem {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.button.layout(ctx, constraints)
    }
    fn set_origin(&mut self, origin: Point) {
        self.button.set_origin(origin);
    }
    fn bounds(&self) -> Rect {
        self.button.bounds()
    }
    fn event(&mut self, ctx: &mut EventCtx, event: &Event) {
        self.button.event(ctx, event);
    }
    fn event_target(&mut self, ctx: &mut EventCtx, event: &Event, target: WidgetId) -> bool {
        self.button.event_target(ctx, event, target)
    }
    fn reconcile(&mut self, next: &mut dyn Widget) {
        if let Some(next) = next.as_any_mut().downcast_mut::<Self>() {
            self.button.reconcile(&mut next.button);
        }
    }
    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        self.button.paint(ctx);
    }
    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        self.button.semantics(ctx);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tab {
    pub label: String,
    pub closable: bool,
}
impl Tab {
    #[must_use]
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            closable: false,
        }
    }
    #[must_use]
    pub const fn closable(mut self, closable: bool) -> Self {
        self.closable = closable;
        self
    }
}

pub struct TabBar {
    id: WidgetId,
    tabs: Vec<Tab>,
    ids: Vec<WidgetId>,
    layouts: Vec<TextLayout>,
    tab_bounds: Vec<Rect>,
    overflow_bounds: Option<Rect>,
    active: usize,
    hovered: Option<usize>,
    focused: Option<usize>,
    bounds: Rect,
    on_select: Box<dyn FnMut(usize)>,
    on_close: Box<dyn FnMut(usize)>,
    on_reorder: Box<dyn FnMut(usize, usize)>,
    reorderable: bool,
    drag_candidate: Option<(usize, Point)>,
    dragging: bool,
    drop_index: Option<usize>,
    modifiers: Modifiers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TabDrag {
    bar: WidgetId,
    index: usize,
}

impl TabBar {
    #[must_use]
    pub fn new(
        tabs: impl IntoIterator<Item = Tab>,
        on_select: impl FnMut(usize) + 'static,
    ) -> Self {
        let tabs: Vec<_> = tabs.into_iter().collect();
        let ids = tabs.iter().map(|_| WidgetId::next()).collect();
        Self {
            id: WidgetId::next(),
            tabs,
            ids,
            layouts: Vec::new(),
            tab_bounds: Vec::new(),
            overflow_bounds: None,
            active: 0,
            hovered: None,
            focused: None,
            bounds: Rect::default(),
            on_select: Box::new(on_select),
            on_close: Box::new(|_| {}),
            on_reorder: Box::new(|_, _| {}),
            reorderable: false,
            drag_candidate: None,
            dragging: false,
            drop_index: None,
            modifiers: Modifiers::default(),
        }
    }
    #[must_use]
    pub fn active(mut self, active: usize) -> Self {
        self.active = active.min(self.tabs.len().saturating_sub(1));
        self
    }
    #[must_use]
    pub fn on_close(mut self, on_close: impl FnMut(usize) + 'static) -> Self {
        self.on_close = Box::new(on_close);
        self
    }

    #[must_use]
    pub fn reorderable(mut self, reorderable: bool) -> Self {
        self.reorderable = reorderable;
        self
    }

    #[must_use]
    pub fn on_reorder(mut self, on_reorder: impl FnMut(usize, usize) + 'static) -> Self {
        self.on_reorder = Box::new(on_reorder);
        self
    }

    fn select(&mut self, index: usize, ctx: &mut EventCtx) {
        if index < self.tabs.len() {
            self.active = index;
            (self.on_select)(index);
            ctx.request_paint();
            ctx.set_handled();
        }
    }
    fn close_hit(&self, index: usize, point: Point) -> bool {
        self.tabs[index].closable
            && point.x >= self.tab_bounds[index].x + self.tab_bounds[index].width - 28.0
    }

    fn begin_tab_drag(&mut self, point: Point, ctx: &mut EventCtx) {
        let Some((index, origin)) = self.drag_candidate else {
            return;
        };
        if (point.x - origin.x).hypot(point.y - origin.y) < 6.0 {
            return;
        }
        self.drag_candidate = None;
        self.dragging = true;
        ctx.start_drag(
            self.id,
            DragPayload::new(TabDrag {
                bar: self.id,
                index,
            }),
            DragOperations::MOVE,
            DragPreview::label(self.tabs[index].label.clone()),
            point,
        );
    }

    fn update_tab_drop(&mut self, point: Point, payload: &DragPayload, ctx: &mut EventCtx) {
        let Some(drag) = payload.downcast_ref::<TabDrag>() else {
            return;
        };
        if drag.bar != self.id || !self.bounds.contains(point) {
            self.drop_index = None;
            return;
        }
        let target = self
            .tab_bounds
            .iter()
            .position(|bounds| bounds.contains(point));
        if let Some(target) = target {
            self.drop_index = Some(target);
            ctx.accept_drag(self.id, DragOperation::Move);
            ctx.request_paint();
        }
    }

    fn keyboard_reorder(&mut self, key: &Key, ctx: &mut EventCtx) -> bool {
        if !self.reorderable || !self.modifiers.alt() {
            return false;
        }
        let Some(from) = self.focused else {
            return false;
        };
        let to = match key {
            Key::ArrowLeft if from > 0 => from - 1,
            Key::ArrowRight if from + 1 < self.tabs.len() => from + 1,
            _ => return false,
        };
        (self.on_reorder)(from, to);
        self.focused = Some(to);
        ctx.request_paint();
        ctx.set_handled();
        true
    }

    fn handle_drag_event(&mut self, event: &Event, ctx: &mut EventCtx) -> bool {
        match event {
            Event::ModifiersChanged(modifiers) => self.modifiers = *modifiers,
            Event::DragMoved { position, payload } => {
                self.update_tab_drop(*position, payload, ctx);
            }
            Event::DragLeft { .. } => {
                self.drop_index = None;
                ctx.request_paint();
            }
            Event::Drop { payload, .. } => {
                if let (Some(drag), Some(to)) =
                    (payload.downcast_ref::<TabDrag>(), self.drop_index.take())
                    && drag.bar == self.id
                    && drag.index != to
                {
                    (self.on_reorder)(drag.index, to);
                    ctx.set_handled();
                }
                ctx.request_paint();
            }
            Event::DragCancelled { .. } | Event::DragCompleted { .. } => {
                self.drag_candidate = None;
                self.dragging = false;
                self.drop_index = None;
                ctx.request_paint();
            }
            Event::KeyDown(key) if self.keyboard_reorder(key, ctx) => {}
            _ => return false,
        }
        true
    }
}

impl Widget for TabBar {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.layouts = self
            .tabs
            .iter()
            .map(|tab| ctx.layout_text(&tab.label, ctx.theme().typography.label, None))
            .collect();
        let desired: Vec<f64> = self
            .layouts
            .iter()
            .zip(&self.tabs)
            .map(|(layout, tab)| {
                (layout.size.width + if tab.closable { 52.0 } else { 28.0 }).max(88.0)
            })
            .collect();
        let available = constraints.max.width;
        let mut visible = desired.len();
        if available.is_finite() && desired.iter().sum::<f64>() > available {
            visible = 0;
            let mut used = 48.0;
            for width in &desired {
                if visible > 0 && used + width > available {
                    break;
                }
                used += width;
                visible += 1;
            }
        }
        self.tab_bounds.clear();
        let mut x = self.bounds.x;
        for width in desired.into_iter().take(visible) {
            self.tab_bounds
                .push(Rect::new(x, self.bounds.y, width, 38.0));
            x += width;
        }
        self.overflow_bounds =
            (visible < self.tabs.len()).then(|| Rect::new(x, self.bounds.y, 48.0, 38.0));
        if self.overflow_bounds.is_some() {
            x += 48.0;
        }
        let size = constraints.constrain(Size::new(x - self.bounds.x, 38.0));
        self.bounds.width = size.width;
        self.bounds.height = size.height;
        size
    }
    fn set_origin(&mut self, origin: Point) {
        let dx = origin.x - self.bounds.x;
        let dy = origin.y - self.bounds.y;
        self.bounds.x = origin.x;
        self.bounds.y = origin.y;
        for bounds in &mut self.tab_bounds {
            bounds.x += dx;
            bounds.y += dy;
        }
        if let Some(bounds) = &mut self.overflow_bounds {
            bounds.x += dx;
            bounds.y += dy;
        }
    }
    fn bounds(&self) -> Rect {
        self.bounds
    }
    fn widget_id(&self) -> Option<WidgetId> {
        Some(self.id)
    }
    fn event(&mut self, ctx: &mut EventCtx, event: &Event) {
        if self.handle_drag_event(event, ctx) {
            return;
        }
        match event {
            Event::PointerMoved(point) => {
                if self.reorderable {
                    self.begin_tab_drag(*point, ctx);
                }
                let next = self
                    .tab_bounds
                    .iter()
                    .position(|bounds| bounds.contains(*point));
                if next != self.hovered {
                    self.hovered = next;
                    ctx.request_paint();
                }
            }
            Event::PointerLeft => {
                self.hovered = None;
                ctx.request_paint();
            }
            Event::PointerDown {
                position,
                button: crate::PointerButton::Primary,
                ..
            } => {
                if let Some(index) = self
                    .tab_bounds
                    .iter()
                    .position(|bounds| bounds.contains(*position))
                {
                    self.focused = Some(index);
                    if self.reorderable && !self.close_hit(index, *position) {
                        self.drag_candidate = Some((index, *position));
                    }
                    ctx.request_focus(self.ids[index]);
                    ctx.set_handled();
                }
            }
            Event::PointerUp {
                position,
                button: crate::PointerButton::Primary,
            } => {
                self.drag_candidate = None;
                if self.dragging {
                    return;
                }
                if let Some(index) = self
                    .tab_bounds
                    .iter()
                    .position(|bounds| bounds.contains(*position))
                {
                    if self.close_hit(index, *position) {
                        (self.on_close)(index);
                        ctx.set_handled();
                    } else {
                        self.select(index, ctx);
                    }
                }
            }
            Event::KeyDown(Key::ArrowLeft) if self.focused.is_some() => {
                let index = self.active.saturating_sub(1);
                self.select(index, ctx);
            }
            Event::KeyDown(Key::ArrowRight) if self.focused.is_some() => {
                let index = (self.active + 1).min(self.tabs.len().saturating_sub(1));
                self.select(index, ctx);
            }
            Event::KeyDown(Key::Enter | Key::Space) if self.focused.is_some() => {
                if let Some(index) = self.focused {
                    self.select(index, ctx);
                }
            }
            Event::FocusChanged(id) => {
                self.focused =
                    id.and_then(|id| self.ids.iter().position(|candidate| *candidate == id));
                ctx.request_paint();
            }
            Event::AccessibilityAction { target, action } => {
                if let Some(index) = self.ids.iter().position(|id| id == target) {
                    match action {
                        crate::SemanticAction::Click => self.select(index, ctx),
                        crate::SemanticAction::Focus => {
                            ctx.request_focus(*target);
                            ctx.set_handled();
                        }
                        crate::SemanticAction::Increment | crate::SemanticAction::Decrement => {}
                    }
                }
            }
            _ => {}
        }
    }
    fn event_target(&mut self, ctx: &mut EventCtx, event: &Event, target: WidgetId) -> bool {
        if target == self.id || self.ids.contains(&target) {
            self.event(ctx, event);
            true
        } else {
            false
        }
    }
    fn reconcile(&mut self, next: &mut dyn Widget) {
        if let Some(next) = next.as_any_mut().downcast_mut::<Self>() {
            next.id = self.id;
            for (next_index, next_tab) in next.tabs.iter().enumerate() {
                if let Some(index) = self.tabs.iter().position(|tab| tab.label == next_tab.label) {
                    next.ids[next_index] = self.ids[index];
                }
            }
            next.active = self.active.min(next.tabs.len().saturating_sub(1));
            next.focused = self.focused.filter(|index| *index < next.tabs.len());
            next.hovered = self.hovered.filter(|index| *index < next.tabs.len());
            next.drag_candidate = None;
            next.dragging = false;
            next.drop_index = self.drop_index.filter(|index| *index < next.tabs.len());
            next.modifiers = self.modifiers;
        }
    }
    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.push(PaintOp::FillRect {
            rect: self.bounds,
            color: ctx.theme().colors.surface_raised,
            radius: 0.0,
        });
        for (index, bounds) in self.tab_bounds.iter().enumerate() {
            if index == self.active || self.hovered == Some(index) {
                ctx.push(PaintOp::FillRect {
                    rect: *bounds,
                    color: if index == self.active {
                        ctx.theme().colors.surface
                    } else {
                        ctx.theme().colors.secondary
                    },
                    radius: 6.0,
                });
            }
            if self.focused == Some(index) {
                ctx.push(PaintOp::StrokeRect {
                    rect: *bounds,
                    color: ctx.theme().colors.focus,
                    width: 2.0,
                    radius: 6.0,
                });
            }
            let text_y = bounds.y + (bounds.height - self.layouts[index].size.height) * 0.5;
            crate::widgets::paint_text_layout(
                ctx,
                &self.layouts[index],
                Point::new(bounds.x + 14.0, text_y),
                ctx.theme().colors.text,
                ctx.theme().typography.label,
            );
            if self.tabs[index].closable {
                let x = bounds.x + bounds.width - 18.0;
                let y = bounds.y + bounds.height * 0.5;
                ctx.push(PaintOp::StrokePath {
                    commands: vec![
                        PathCommand::MoveTo(Point::new(x - 4.0, y - 4.0)),
                        PathCommand::LineTo(Point::new(x + 4.0, y + 4.0)),
                        PathCommand::MoveTo(Point::new(x + 4.0, y - 4.0)),
                        PathCommand::LineTo(Point::new(x - 4.0, y + 4.0)),
                    ],
                    color: ctx.theme().colors.text_muted,
                    width: 1.5,
                });
            }
        }
        if let Some(bounds) = self.overflow_bounds {
            let remaining = self.tabs.len() - self.tab_bounds.len();
            ctx.push(PaintOp::Text {
                origin: Point::new(bounds.x + 10.0, bounds.y + 24.0),
                text: format!("+{remaining}"),
                color: ctx.theme().colors.text_muted,
                size: 13.0,
            });
        }
        if let Some(index) = self.drop_index
            && let Some(bounds) = self.tab_bounds.get(index)
        {
            ctx.push(PaintOp::FillRect {
                rect: Rect::new(bounds.x - 1.0, bounds.y + 3.0, 2.0, bounds.height - 6.0),
                color: ctx.theme().colors.primary,
                radius: 1.0,
            });
        }
    }
    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        for (index, bounds) in self.tab_bounds.iter().enumerate() {
            ctx.push(SemanticNode {
                id: self.ids[index],
                role: SemanticRole::Tab,
                bounds: *bounds,
                label: self.tabs[index].label.clone(),
                disabled: false,
                focused: self.focused == Some(index),
                checked: Some(self.active == index),
                value: None,
                ime_cursor_area: None,
            });
        }
    }
}

pub struct Dialog {
    inner: Region,
}
impl Dialog {
    #[must_use]
    pub fn new(title: impl Into<String>, body: impl Widget + 'static) -> Self {
        let title = title.into();
        let content = Column::new([
            Box::new(Label::new(title.clone()).text_size(22.0)) as Box<dyn Widget>,
            Box::new(body) as Box<dyn Widget>,
        ])
        .gap(14.0);
        let surface = Surface::new(crate::Padding::new(Insets::all(20.0), content))
            .tone(SurfaceTone::Raised)
            .elevation(2)
            .radius(12.0);
        Self {
            inner: Region::new(SemanticRole::Dialog, title, surface),
        }
    }

    #[must_use]
    pub fn confirmation(
        title: impl Into<String>,
        message: impl Into<String>,
        confirm_label: impl Into<String>,
        mut on_confirm: impl FnMut() + 'static,
    ) -> Self {
        let confirm = Button::action(confirm_label, move |_, ctx| {
            on_confirm();
            ctx.dismiss_overlay();
        })
        .variant(ButtonVariant::Primary);
        let cancel = Button::action("Cancel", |_, ctx| ctx.dismiss_overlay())
            .variant(ButtonVariant::Secondary);
        let actions = Flex::row([FlexChild::fixed(cancel), FlexChild::fixed(confirm)]).gap(8.0);
        let body = Column::new([
            Box::new(Label::new(message)) as Box<dyn Widget>,
            Box::new(actions) as Box<dyn Widget>,
        ])
        .gap(18.0);
        Self::new(title, body)
    }
}
region_widget!(Dialog);

pub struct CommandPalette {
    id: WidgetId,
    commands: Vec<CommandDescriptor>,
    command_ids: Vec<WidgetId>,
    query: String,
    filtered: Vec<usize>,
    active: usize,
    layouts: Vec<TextLayout>,
    bounds: Rect,
    row_bounds: Vec<Rect>,
}

impl CommandPalette {
    #[must_use]
    pub fn new(commands: impl IntoIterator<Item = CommandDescriptor>) -> Self {
        let commands: Vec<_> = commands.into_iter().collect();
        let command_ids = commands.iter().map(|_| WidgetId::next()).collect();
        let filtered = (0..commands.len()).collect();
        Self {
            id: WidgetId::next(),
            commands,
            command_ids,
            query: String::new(),
            filtered,
            active: 0,
            layouts: Vec::new(),
            bounds: Rect::default(),
            row_bounds: Vec::new(),
        }
    }
    fn update_filter(&mut self) {
        let query = self.query.to_lowercase();
        self.filtered = self
            .commands
            .iter()
            .enumerate()
            .filter(|(_, command)| {
                command.keywords.contains(&query)
                    || command.description.to_lowercase().contains(&query)
            })
            .map(|(index, _)| index)
            .collect();
        self.active = self.active.min(self.filtered.len().saturating_sub(1));
    }
    fn activate(&mut self, ctx: &mut EventCtx) {
        if let Some(index) = self.filtered.get(self.active).copied()
            && self.commands[index].enabled
        {
            ctx.invoke_command(self.commands[index].id.clone());
            ctx.dismiss_overlay();
            ctx.set_handled();
        }
    }
}

impl Widget for CommandPalette {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.layouts = self
            .filtered
            .iter()
            .map(|index| {
                ctx.layout_text(
                    &self.commands[*index].label,
                    ctx.theme().typography.body,
                    None,
                )
            })
            .collect();
        let width = constraints
            .max
            .width
            .min(520.0)
            .max(constraints.min.width.min(520.0));
        let rows = self.filtered.len().min(8);
        let height = 58.0 + 38.0 * f64::from(u32::try_from(rows).unwrap_or(8));
        let size = constraints.constrain(Size::new(width, height));
        self.bounds.width = size.width;
        self.bounds.height = size.height;
        self.row_bounds = (0..rows)
            .map(|row| {
                Rect::new(
                    self.bounds.x + 8.0,
                    self.bounds.y + 50.0 + 38.0 * f64::from(u32::try_from(row).unwrap_or(0)),
                    (size.width - 16.0).max(0.0),
                    36.0,
                )
            })
            .collect();
        size
    }
    fn set_origin(&mut self, origin: Point) {
        let dx = origin.x - self.bounds.x;
        let dy = origin.y - self.bounds.y;
        self.bounds.x = origin.x;
        self.bounds.y = origin.y;
        for bounds in &mut self.row_bounds {
            bounds.x += dx;
            bounds.y += dy;
        }
    }
    fn bounds(&self) -> Rect {
        self.bounds
    }
    fn event(&mut self, ctx: &mut EventCtx, event: &Event) {
        match event {
            Event::TextInput(text) => {
                self.query.push_str(text);
                self.update_filter();
                ctx.request_layout();
                ctx.set_handled();
            }
            Event::KeyDown(Key::Backspace) => {
                self.query.pop();
                self.update_filter();
                ctx.request_layout();
                ctx.set_handled();
            }
            Event::KeyDown(Key::ArrowDown) => {
                if !self.filtered.is_empty() {
                    self.active = (self.active + 1).min(self.filtered.len() - 1);
                    ctx.request_paint();
                }
                ctx.set_handled();
            }
            Event::KeyDown(Key::ArrowUp) => {
                self.active = self.active.saturating_sub(1);
                ctx.request_paint();
                ctx.set_handled();
            }
            Event::KeyDown(Key::Enter) => self.activate(ctx),
            Event::KeyDown(Key::Escape) => {
                ctx.dismiss_overlay();
                ctx.set_handled();
            }
            Event::PointerMoved(point) => {
                if let Some(index) = self
                    .row_bounds
                    .iter()
                    .position(|bounds| bounds.contains(*point))
                {
                    self.active = index;
                    ctx.request_paint();
                }
            }
            Event::PointerUp {
                position,
                button: crate::PointerButton::Primary,
            } => {
                if let Some(index) = self
                    .row_bounds
                    .iter()
                    .position(|bounds| bounds.contains(*position))
                {
                    self.active = index;
                    self.activate(ctx);
                }
            }
            Event::AccessibilityAction {
                target,
                action: crate::SemanticAction::Click,
            } => {
                if let Some(filtered_index) = self
                    .filtered
                    .iter()
                    .position(|index| self.command_ids[*index] == *target)
                {
                    self.active = filtered_index;
                    self.activate(ctx);
                }
            }
            _ => {}
        }
    }
    fn widget_id(&self) -> Option<WidgetId> {
        Some(self.id)
    }
    fn event_target(&mut self, ctx: &mut EventCtx, event: &Event, target: WidgetId) -> bool {
        if target == self.id || self.command_ids.contains(&target) {
            self.event(ctx, event);
            true
        } else {
            false
        }
    }
    fn reconcile(&mut self, next: &mut dyn Widget) {
        if let Some(next) = next.as_any_mut().downcast_mut::<Self>() {
            next.id = self.id;
            next.query.clone_from(&self.query);
            next.active = self.active;
            next.update_filter();
        }
    }
    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.push(PaintOp::FillRect {
            rect: Rect::new(
                self.bounds.x,
                self.bounds.y + 5.0,
                self.bounds.width,
                self.bounds.height,
            ),
            color: ctx.theme().elevation.shadow,
            radius: 12.0,
        });
        ctx.push(PaintOp::FillRect {
            rect: self.bounds,
            color: ctx.theme().colors.surface_raised,
            radius: 12.0,
        });
        ctx.push(PaintOp::StrokeRect {
            rect: Rect::new(
                self.bounds.x + 8.0,
                self.bounds.y + 8.0,
                (self.bounds.width - 16.0).max(0.0),
                34.0,
            ),
            color: ctx.theme().colors.border,
            width: 1.0,
            radius: 7.0,
        });
        ctx.push(PaintOp::Text {
            origin: Point::new(self.bounds.x + 18.0, self.bounds.y + 31.0),
            text: if self.query.is_empty() {
                "Type a command…".to_owned()
            } else {
                self.query.clone()
            },
            color: if self.query.is_empty() {
                ctx.theme().colors.text_muted
            } else {
                ctx.theme().colors.text
            },
            size: 15.0,
        });
        for (row, bounds) in self.row_bounds.iter().enumerate() {
            if row == self.active {
                ctx.push(PaintOp::FillRect {
                    rect: *bounds,
                    color: ctx.theme().colors.secondary,
                    radius: 6.0,
                });
            }
            crate::widgets::paint_text_layout(
                ctx,
                &self.layouts[row],
                Point::new(bounds.x + 10.0, bounds.y + 8.0),
                ctx.theme().colors.text,
                ctx.theme().typography.body,
            );
        }
    }
    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        ctx.push(SemanticNode {
            id: self.id,
            role: SemanticRole::TextInput,
            bounds: Rect::new(
                self.bounds.x + 8.0,
                self.bounds.y + 8.0,
                (self.bounds.width - 16.0).max(0.0),
                34.0,
            ),
            label: "Command search".to_owned(),
            disabled: false,
            focused: true,
            checked: None,
            value: Some(self.query.clone()),
            ime_cursor_area: None,
        });
        for (row, index) in self
            .filtered
            .iter()
            .copied()
            .take(self.row_bounds.len())
            .enumerate()
        {
            ctx.push(SemanticNode {
                id: self.command_ids[index],
                role: SemanticRole::Button,
                bounds: self.row_bounds[row],
                label: self.commands[index].label.clone(),
                disabled: !self.commands[index].enabled,
                focused: row == self.active,
                checked: None,
                value: None,
                ime_cursor_area: None,
            });
        }
    }
}
