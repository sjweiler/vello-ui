use crate::{
    BoxConstraints, DragOperation, DragOperations, DragPayload, DragPreview, Event, EventCtx, Key,
    LayoutCtx, Modifiers, PaintCtx, PaintOp, Point, Rect, SemanticAction, SemanticNode,
    SemanticRole, SemanticsCtx, Size, TextLayout, UiCommand, Widget, WidgetId, WidgetKey,
};
use std::{cell::RefCell, rc::Rc};

type ItemFactory = dyn FnMut(usize) -> ListItem;
type SelectionCallback = dyn FnMut(Vec<WidgetKey>);
type ActivationCallback = dyn FnMut(WidgetKey);
type DropCallback = dyn FnMut(CollectionDrop);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropPosition {
    Before,
    After,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectionDrop {
    pub dragged: WidgetKey,
    pub target: WidgetKey,
    pub position: DropPosition,
    pub operation: DragOperation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectionDrag {
    pub key: WidgetKey,
}

#[derive(Clone, Copy)]
struct DragCandidate {
    index: usize,
    origin: Point,
}

#[derive(Clone)]
struct DropIndicator {
    index: usize,
    target: WidgetKey,
    position: DropPosition,
}

#[derive(Clone, Copy)]
enum SemanticsEmission {
    None,
    Container,
    Children,
    Both,
}

impl SemanticsEmission {
    const fn container(self) -> bool {
        matches!(self, Self::Container | Self::Both)
    }

    const fn children(self) -> bool {
        matches!(self, Self::Children | Self::Both)
    }
}

fn index_position(index: usize) -> f64 {
    f64::from(u32::try_from(index).unwrap_or(u32::MAX))
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn nonnegative_index(value: f64) -> usize {
    value.max(0.0).min(usize::MAX as f64) as usize
}

pub struct ListItem {
    key: WidgetKey,
    label: String,
    child: Box<dyn Widget>,
    disabled: bool,
}

impl ListItem {
    #[must_use]
    pub fn new(
        key: impl Into<WidgetKey>,
        label: impl Into<String>,
        child: impl Widget + 'static,
    ) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            child: Box::new(child),
            disabled: false,
        }
    }

    #[must_use]
    pub const fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

struct MaterializedRow {
    index: usize,
    key: WidgetKey,
    label: String,
    child: Box<dyn Widget>,
    disabled: bool,
    id: WidgetId,
    bounds: Rect,
}

/// A fixed-height, vertically virtualized collection with keyed row identity.
pub struct ListView {
    id: WidgetId,
    item_count: usize,
    item_factory: Box<ItemFactory>,
    rows: Vec<MaterializedRow>,
    row_height: f64,
    overscan: usize,
    viewport_height: Option<f64>,
    bounds: Rect,
    scroll_y: f64,
    pending_scroll_to: Option<usize>,
    selected: Vec<WidgetKey>,
    active: Option<usize>,
    anchor: Option<usize>,
    hovered: Option<usize>,
    focused: bool,
    multi_select: bool,
    modifiers: Modifiers,
    on_selection_change: Box<SelectionCallback>,
    on_activate: Box<ActivationCallback>,
    semantic_role: SemanticRole,
    item_semantic_role: SemanticRole,
    semantic_label: String,
    content_inset: f64,
    semantics_emission: SemanticsEmission,
    drag_enabled: bool,
    drag_candidate: Option<DragCandidate>,
    drop_indicator: Option<DropIndicator>,
    on_drop: Box<DropCallback>,
}

impl ListView {
    #[must_use]
    pub fn new(item_count: usize, item_factory: impl FnMut(usize) -> ListItem + 'static) -> Self {
        Self {
            id: WidgetId::next(),
            item_count,
            item_factory: Box::new(item_factory),
            rows: Vec::new(),
            row_height: 36.0,
            overscan: 2,
            viewport_height: None,
            bounds: Rect::default(),
            scroll_y: 0.0,
            pending_scroll_to: None,
            selected: Vec::new(),
            active: None,
            anchor: None,
            hovered: None,
            focused: false,
            multi_select: true,
            modifiers: Modifiers::default(),
            on_selection_change: Box::new(|_| {}),
            on_activate: Box::new(|_| {}),
            semantic_role: SemanticRole::List,
            item_semantic_role: SemanticRole::ListItem,
            semantic_label: "List".to_owned(),
            content_inset: 12.0,
            semantics_emission: SemanticsEmission::Container,
            drag_enabled: false,
            drag_candidate: None,
            drop_indicator: None,
            on_drop: Box::new(|_| {}),
        }
    }

    #[must_use]
    pub fn row_height(mut self, row_height: f64) -> Self {
        self.row_height = row_height.max(1.0);
        self
    }

    #[must_use]
    pub const fn overscan(mut self, rows: usize) -> Self {
        self.overscan = rows;
        self
    }

    #[must_use]
    pub fn viewport_height(mut self, height: f64) -> Self {
        self.viewport_height = Some(height.max(0.0));
        self
    }

    #[must_use]
    pub const fn multi_select(mut self, enabled: bool) -> Self {
        self.multi_select = enabled;
        self
    }

    #[must_use]
    pub const fn scroll_to_item(mut self, index: usize) -> Self {
        self.pending_scroll_to = Some(index);
        self
    }

    #[must_use]
    pub fn on_selection_change(mut self, callback: impl FnMut(Vec<WidgetKey>) + 'static) -> Self {
        self.on_selection_change = Box::new(callback);
        self
    }

    #[must_use]
    pub fn on_activate(mut self, callback: impl FnMut(WidgetKey) + 'static) -> Self {
        self.on_activate = Box::new(callback);
        self
    }

    #[must_use]
    pub const fn draggable(mut self, enabled: bool) -> Self {
        self.drag_enabled = enabled;
        self
    }

    #[must_use]
    pub fn on_drop(mut self, callback: impl FnMut(CollectionDrop) + 'static) -> Self {
        self.on_drop = Box::new(callback);
        self
    }

    #[must_use]
    pub fn selected_keys(&self) -> &[WidgetKey] {
        &self.selected
    }

    #[must_use]
    pub const fn active_index(&self) -> Option<usize> {
        self.active
    }

    #[must_use]
    pub const fn is_focused(&self) -> bool {
        self.focused
    }

    #[must_use]
    pub fn row_index_at(&self, point: Point) -> Option<usize> {
        self.row_at(point)
    }

    pub fn focus_item(&mut self, index: usize, ctx: &mut EventCtx) {
        self.move_active(index, ctx);
    }

    pub(crate) fn semantic_roles(
        mut self,
        container: SemanticRole,
        item: SemanticRole,
        label: impl Into<String>,
    ) -> Self {
        self.semantic_role = container;
        self.item_semantic_role = item;
        self.semantic_label = label.into();
        self
    }

    pub(crate) const fn content_inset(mut self, inset: f64) -> Self {
        self.content_inset = inset;
        self
    }

    pub(crate) const fn include_child_semantics(mut self, include: bool) -> Self {
        self.semantics_emission = match (self.semantics_emission.container(), include) {
            (true, true) => SemanticsEmission::Both,
            (true, false) => SemanticsEmission::Container,
            (false, true) => SemanticsEmission::Children,
            (false, false) => SemanticsEmission::None,
        };
        self
    }

    pub(crate) const fn container_semantics(mut self, include: bool) -> Self {
        self.semantics_emission = match (include, self.semantics_emission.children()) {
            (true, true) => SemanticsEmission::Both,
            (true, false) => SemanticsEmission::Container,
            (false, true) => SemanticsEmission::Children,
            (false, false) => SemanticsEmission::None,
        };
        self
    }

    #[must_use]
    pub fn visible_range(&self) -> std::ops::Range<usize> {
        let first = nonnegative_index((self.scroll_y / self.row_height).floor());
        let visible = nonnegative_index((self.bounds.height / self.row_height).ceil());
        first.saturating_sub(self.overscan)..(first + visible + self.overscan).min(self.item_count)
    }

    fn content_height(&self) -> f64 {
        self.row_height * index_position(self.item_count)
    }

    fn max_scroll(&self) -> f64 {
        (self.content_height() - self.bounds.height).max(0.0)
    }

    fn row_at(&self, point: Point) -> Option<usize> {
        if !self.bounds.contains(point) {
            return None;
        }
        let index = nonnegative_index(
            ((point.y - self.bounds.y + self.scroll_y) / self.row_height).floor(),
        );
        (index < self.item_count).then_some(index)
    }

    fn ensure_visible(&mut self, index: usize) {
        let top = index_position(index) * self.row_height;
        let bottom = top + self.row_height;
        if top < self.scroll_y {
            self.scroll_y = top;
        } else if bottom > self.scroll_y + self.bounds.height {
            self.scroll_y = bottom - self.bounds.height;
        }
        self.scroll_y = self.scroll_y.clamp(0.0, self.max_scroll());
    }

    fn item_key_if_enabled(&mut self, index: usize) -> Option<WidgetKey> {
        let item = (self.item_factory)(index);
        (!item.disabled).then_some(item.key)
    }

    fn replace_selection(&mut self, keys: Vec<WidgetKey>) {
        if self.selected != keys {
            self.selected = keys;
            (self.on_selection_change)(self.selected.clone());
        }
    }

    fn select(&mut self, index: usize) {
        let shift = self.multi_select && self.modifiers.shift();
        let toggle = self.multi_select && self.modifiers.command();
        if shift {
            let anchor = self.anchor.unwrap_or(index);
            let start = anchor.min(index);
            let end = anchor.max(index);
            let keys = (start..=end)
                .filter_map(|row| self.item_key_if_enabled(row))
                .collect();
            self.replace_selection(keys);
        } else if toggle {
            if let Some(key) = self.item_key_if_enabled(index) {
                let mut keys = self.selected.clone();
                if let Some(position) = keys.iter().position(|selected| *selected == key) {
                    keys.remove(position);
                } else {
                    keys.push(key);
                }
                self.replace_selection(keys);
                self.anchor = Some(index);
            }
        } else if let Some(key) = self.item_key_if_enabled(index) {
            self.replace_selection(vec![key]);
            self.anchor = Some(index);
        }
        self.active = Some(index);
    }

    fn move_active(&mut self, target: usize, ctx: &mut EventCtx) {
        if self.item_count == 0 {
            return;
        }
        let target = target.min(self.item_count - 1);
        self.select(target);
        self.ensure_visible(target);
        ctx.request_layout();
        ctx.set_handled();
    }

    fn activate(&mut self, index: usize, ctx: &mut EventCtx) {
        if let Some(key) = self.item_key_if_enabled(index) {
            (self.on_activate)(key);
            ctx.set_handled();
        }
    }

    fn handle_key(&mut self, key: &Key, ctx: &mut EventCtx) -> bool {
        if !self.focused {
            return false;
        }
        match key {
            Key::ArrowDown => self.move_active(self.active.map_or(0, |index| index + 1), ctx),
            Key::ArrowUp => self.move_active(self.active.unwrap_or(1).saturating_sub(1), ctx),
            Key::Home => self.move_active(0, ctx),
            Key::End if self.item_count > 0 => self.move_active(self.item_count - 1, ctx),
            Key::PageDown | Key::PageUp => {
                let page =
                    nonnegative_index((self.bounds.height / self.row_height).floor().max(1.0));
                let current = self.active.unwrap_or(0);
                let target = if *key == Key::PageDown {
                    current.saturating_add(page)
                } else {
                    current.saturating_sub(page)
                };
                self.move_active(target, ctx);
            }
            Key::Space => {
                if let Some(index) = self.active {
                    self.select(index);
                    ctx.request_paint();
                    ctx.set_handled();
                }
            }
            Key::Enter => {
                if let Some(index) = self.active {
                    self.activate(index, ctx);
                }
            }
            _ => return false,
        }
        true
    }

    fn position_rows(&mut self) {
        for row in &mut self.rows {
            row.bounds = Rect::new(
                self.bounds.x,
                self.bounds.y + index_position(row.index) * self.row_height - self.scroll_y,
                self.bounds.width,
                self.row_height,
            );
            row.child
                .set_origin(Point::new(row.bounds.x + self.content_inset, row.bounds.y));
        }
    }

    fn refresh_rows(&mut self, ctx: &mut LayoutCtx<'_>) {
        let mut previous = std::mem::take(&mut self.rows);
        let mut rows = Vec::with_capacity(self.visible_range().len());
        for index in self.visible_range() {
            let mut item = (self.item_factory)(index);
            let existing = previous.iter().position(|row| row.key == item.key);
            let id = existing.map_or_else(WidgetId::next, |position| {
                let mut old = previous.swap_remove(position);
                old.child.reconcile(item.child.as_mut());
                old.id
            });
            let width = (self.bounds.width - self.content_inset * 2.0).max(0.0);
            item.child.layout(
                ctx,
                BoxConstraints::tight(Size::new(width, self.row_height)),
            );
            rows.push(MaterializedRow {
                index,
                key: item.key,
                label: item.label,
                child: item.child,
                disabled: item.disabled,
                id,
                bounds: Rect::default(),
            });
        }
        self.rows = rows;
        self.position_rows();
    }

    fn paint_scrollbar(&self, ctx: &mut PaintCtx<'_>) {
        let content_height = self.content_height();
        if content_height <= self.bounds.height || self.bounds.height <= 0.0 {
            return;
        }
        let track_height = (self.bounds.height - 8.0).max(0.0);
        let thumb_height = (self.bounds.height / content_height * track_height)
            .clamp(20.0_f64.min(track_height), track_height);
        let travel = track_height - thumb_height;
        let y = self.bounds.y + 4.0 + self.scroll_y / self.max_scroll() * travel;
        ctx.push(PaintOp::FillRect {
            rect: Rect::new(
                self.bounds.x + self.bounds.width - 6.0,
                y,
                3.0,
                thumb_height,
            ),
            color: ctx.theme().colors.border,
            radius: 1.5,
        });
    }

    fn begin_drag_if_needed(&mut self, point: Point, ctx: &mut EventCtx) {
        let Some(candidate) = self.drag_candidate else {
            return;
        };
        let distance = (point.x - candidate.origin.x).hypot(point.y - candidate.origin.y);
        if distance < 6.0 {
            return;
        }
        let item = (self.item_factory)(candidate.index);
        self.drag_candidate = None;
        if item.disabled {
            return;
        }
        ctx.start_drag(
            self.id,
            DragPayload::new(CollectionDrag {
                key: item.key.clone(),
            }),
            DragOperations::MOVE,
            DragPreview::label(item.label),
            point,
        );
    }

    fn update_drop_indicator(&mut self, point: Point, payload: &DragPayload, ctx: &mut EventCtx) {
        if !self.drag_enabled || !payload.is::<CollectionDrag>() || !self.bounds.contains(point) {
            if self.drop_indicator.take().is_some() {
                ctx.request_paint();
            }
            return;
        }
        self.auto_scroll_drag(point, ctx);
        let Some(index) = self.row_at(point) else {
            return;
        };
        let item = (self.item_factory)(index);
        if item.disabled {
            return;
        }
        let row_top = self.bounds.y + index_position(index) * self.row_height - self.scroll_y;
        let position = if point.y < row_top + self.row_height * 0.5 {
            DropPosition::Before
        } else {
            DropPosition::After
        };
        self.drop_indicator = Some(DropIndicator {
            index,
            target: item.key,
            position,
        });
        ctx.accept_drag(self.id, DragOperation::Move);
        ctx.request_paint();
    }

    fn auto_scroll_drag(&mut self, point: Point, ctx: &mut EventCtx) {
        const EDGE: f64 = 24.0;
        let delta = if point.y < self.bounds.y + EDGE {
            -self.row_height
        } else if point.y > self.bounds.y + self.bounds.height - EDGE {
            self.row_height
        } else {
            return;
        };
        let next = (self.scroll_y + delta).clamp(0.0, self.max_scroll());
        if (next - self.scroll_y).abs() > f64::EPSILON {
            self.scroll_y = next;
            ctx.request_layout();
        }
    }

    fn perform_drop(
        &mut self,
        payload: &DragPayload,
        operation: DragOperation,
        ctx: &mut EventCtx,
    ) {
        let (Some(drag), Some(indicator)) = (
            payload.downcast_ref::<CollectionDrag>(),
            self.drop_indicator.take(),
        ) else {
            return;
        };
        (self.on_drop)(CollectionDrop {
            dragged: drag.key.clone(),
            target: indicator.target,
            position: indicator.position,
            operation,
        });
        ctx.request_paint();
        ctx.set_handled();
    }

    fn keyboard_reorder(&mut self, key: &Key, ctx: &mut EventCtx) -> bool {
        if !self.drag_enabled || !self.focused || !self.modifiers.alt() {
            return false;
        }
        let Some(source) = self.active else {
            return false;
        };
        let target = match key {
            Key::ArrowUp if source > 0 => source - 1,
            Key::ArrowDown if source + 1 < self.item_count => source + 1,
            _ => return false,
        };
        let source_item = (self.item_factory)(source);
        let target_item = (self.item_factory)(target);
        if source_item.disabled || target_item.disabled {
            return false;
        }
        (self.on_drop)(CollectionDrop {
            dragged: source_item.key,
            target: target_item.key,
            position: if target < source {
                DropPosition::Before
            } else {
                DropPosition::After
            },
            operation: DragOperation::Move,
        });
        self.move_active(target, ctx);
        true
    }
}

impl Widget for ListView {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let width = if constraints.max.width.is_finite() {
            constraints.max.width
        } else {
            240.0
        };
        let desired_height = self
            .viewport_height
            .unwrap_or_else(|| self.content_height());
        let size = constraints.constrain(Size::new(width, desired_height));
        self.bounds.width = size.width;
        self.bounds.height = size.height;
        if let Some(index) = self
            .pending_scroll_to
            .take()
            .filter(|_| self.item_count > 0)
        {
            self.ensure_visible(index.min(self.item_count - 1));
        }
        self.scroll_y = self.scroll_y.clamp(0.0, self.max_scroll());
        self.refresh_rows(ctx);
        size
    }

    fn set_origin(&mut self, origin: Point) {
        self.bounds.x = origin.x;
        self.bounds.y = origin.y;
        self.position_rows();
    }

    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &Event) {
        match event {
            Event::ModifiersChanged(modifiers) => self.modifiers = *modifiers,
            Event::PointerMoved(position) => {
                self.begin_drag_if_needed(*position, ctx);
                let hovered = self.row_at(*position);
                if hovered != self.hovered {
                    self.hovered = hovered;
                    ctx.request_paint();
                }
            }
            Event::PointerLeft => {
                self.drag_candidate = None;
                if self.hovered.take().is_some() {
                    ctx.request_paint();
                }
            }
            Event::PointerDown {
                position,
                button: crate::PointerButton::Primary,
                click_count,
            } => {
                if let Some(index) = self.row_at(*position) {
                    self.select(index);
                    ctx.request_focus(self.id);
                    ctx.request_paint();
                    ctx.set_handled();
                    if self.drag_enabled {
                        self.drag_candidate = Some(DragCandidate {
                            index,
                            origin: *position,
                        });
                    }
                    if *click_count >= 2 {
                        self.activate(index, ctx);
                    }
                }
            }
            Event::PointerUp { .. } => self.drag_candidate = None,
            Event::DragMoved { position, payload } => {
                self.update_drop_indicator(*position, payload, ctx);
            }
            Event::DragLeft { .. } | Event::DragCancelled { .. } | Event::DragCompleted { .. } => {
                if self.drop_indicator.take().is_some() {
                    ctx.request_paint();
                }
                self.drag_candidate = None;
            }
            Event::Drop {
                payload, operation, ..
            } => self.perform_drop(payload, *operation, ctx),
            Event::Scroll {
                position, delta_y, ..
            } if self.bounds.contains(*position) => {
                let next = (self.scroll_y + delta_y).clamp(0.0, self.max_scroll());
                if (next - self.scroll_y).abs() > f64::EPSILON {
                    self.scroll_y = next;
                    ctx.request_layout();
                    ctx.set_handled();
                }
            }
            Event::KeyDown(key) if self.keyboard_reorder(key, ctx) => {}
            Event::KeyDown(key) if self.handle_key(key, ctx) => {}
            Event::AccessibilityAction { target, action } if *target == self.id => {
                if *action == SemanticAction::Focus {
                    ctx.request_focus(self.id);
                    ctx.set_handled();
                }
            }
            Event::AccessibilityAction { target, action } => {
                if let Some(index) = self
                    .rows
                    .iter()
                    .find_map(|row| (row.id == *target && !row.disabled).then_some(row.index))
                {
                    self.select(index);
                    self.ensure_visible(index);
                    ctx.request_focus(self.id);
                    ctx.request_layout();
                    ctx.set_handled();
                    if *action == SemanticAction::Click {
                        self.activate(index, ctx);
                    }
                }
            }
            Event::FocusChanged(focused) => {
                let next = *focused == Some(self.id);
                if next != self.focused {
                    self.focused = next;
                    ctx.request_paint();
                }
            }
            _ => {}
        }
    }

    fn widget_id(&self) -> Option<WidgetId> {
        Some(self.id)
    }

    fn event_target(&mut self, ctx: &mut EventCtx, event: &Event, target: WidgetId) -> bool {
        if target == self.id || self.rows.iter().any(|row| row.id == target) {
            self.event(ctx, event);
            true
        } else {
            self.rows
                .iter_mut()
                .any(|row| row.child.event_target(ctx, event, target))
        }
    }

    fn apply_command(&mut self, mut command: UiCommand) -> Result<(), UiCommand> {
        for row in self.rows.iter_mut().rev() {
            match row.child.apply_command(command) {
                Ok(()) => return Ok(()),
                Err(returned) => command = returned,
            }
        }
        Err(command)
    }

    fn reconcile(&mut self, next: &mut dyn Widget) {
        if let Some(next) = next.as_any_mut().downcast_mut::<Self>() {
            next.id = self.id;
            next.rows = std::mem::take(&mut self.rows);
            next.scroll_y = self.scroll_y;
            next.selected.clone_from(&self.selected);
            next.active = self.active.filter(|index| *index < next.item_count);
            next.anchor = self.anchor.filter(|index| *index < next.item_count);
            next.hovered = self.hovered.filter(|index| *index < next.item_count);
            next.focused = self.focused;
            next.modifiers = self.modifiers;
            next.drag_candidate = None;
            next.drop_indicator.clone_from(&self.drop_indicator);
        }
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.push(PaintOp::PushClip {
            rect: self.bounds,
            radius: 0.0,
        });
        for row in &self.rows {
            let selected = self.selected.contains(&row.key);
            if selected || self.hovered == Some(row.index) {
                ctx.push(PaintOp::FillRect {
                    rect: row.bounds,
                    color: if selected {
                        ctx.theme().colors.secondary
                    } else {
                        ctx.theme().components.ghost_button.hovered.background
                    },
                    radius: 0.0,
                });
            }
            if self.focused && self.active == Some(row.index) {
                ctx.push(PaintOp::StrokeRect {
                    rect: Rect::new(
                        row.bounds.x + 1.0,
                        row.bounds.y + 1.0,
                        (row.bounds.width - 2.0).max(0.0),
                        (row.bounds.height - 2.0).max(0.0),
                    ),
                    color: ctx.theme().colors.focus,
                    width: 1.0,
                    radius: ctx.theme().radii.sm,
                });
            }
            row.child.paint(ctx);
        }
        if let Some(indicator) = &self.drop_indicator {
            let row_y =
                self.bounds.y + index_position(indicator.index) * self.row_height - self.scroll_y;
            let y = if indicator.position == DropPosition::Before {
                row_y
            } else {
                row_y + self.row_height
            };
            ctx.push(PaintOp::FillRect {
                rect: Rect::new(self.bounds.x + 4.0, y - 1.0, self.bounds.width - 8.0, 2.0),
                color: ctx.theme().colors.primary,
                radius: 1.0,
            });
        }
        ctx.push(PaintOp::PopClip);
        self.paint_scrollbar(ctx);
    }

    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        if self.semantics_emission.container() {
            ctx.push(SemanticNode {
                id: self.id,
                role: self.semantic_role,
                bounds: self.bounds,
                label: self.semantic_label.clone(),
                disabled: false,
                focused: self.focused,
                checked: None,
                value: self
                    .active
                    .map(|index| format!("Item {} of {}", index + 1, self.item_count)),
                ime_cursor_area: None,
            });
        }
        for row in &self.rows {
            ctx.push(SemanticNode {
                id: row.id,
                role: self.item_semantic_role,
                bounds: row.bounds,
                label: row.label.clone(),
                disabled: row.disabled,
                focused: self.focused && self.active == Some(row.index),
                checked: Some(self.selected.contains(&row.key)),
                value: Some(format!("{} of {}", row.index + 1, self.item_count)),
                ime_cursor_area: None,
            });
            if self.semantics_emission.children() {
                row.child.semantics(ctx);
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeChildren {
    None,
    Unloaded,
    Loading,
    Error(String),
    Loaded(Vec<TreeNode>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeNode {
    key: WidgetKey,
    label: String,
    children: TreeChildren,
    disabled: bool,
}

impl TreeNode {
    #[must_use]
    pub fn leaf(key: impl Into<WidgetKey>, label: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            children: TreeChildren::None,
            disabled: false,
        }
    }

    #[must_use]
    pub fn branch(
        key: impl Into<WidgetKey>,
        label: impl Into<String>,
        children: impl IntoIterator<Item = Self>,
    ) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            children: TreeChildren::Loaded(children.into_iter().collect()),
            disabled: false,
        }
    }

    #[must_use]
    pub fn unloaded(key: impl Into<WidgetKey>, label: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            children: TreeChildren::Unloaded,
            disabled: false,
        }
    }

    #[must_use]
    pub fn loading(key: impl Into<WidgetKey>, label: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            children: TreeChildren::Loading,
            disabled: false,
        }
    }

    #[must_use]
    pub fn error(
        key: impl Into<WidgetKey>,
        label: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            children: TreeChildren::Error(message.into()),
            disabled: false,
        }
    }

    #[must_use]
    pub const fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    #[must_use]
    pub const fn key(&self) -> &WidgetKey {
        &self.key
    }
}

#[derive(Clone)]
enum FlatTreeKind {
    Leaf,
    Collapsed,
    Expanded,
    Status,
}

#[derive(Clone)]
struct FlatTreeNode {
    key: WidgetKey,
    label: String,
    depth: usize,
    parent: Option<WidgetKey>,
    kind: FlatTreeKind,
    disabled: bool,
}

impl FlatTreeNode {
    const fn expandable(&self) -> bool {
        matches!(self.kind, FlatTreeKind::Collapsed | FlatTreeKind::Expanded)
    }

    const fn expanded(&self) -> bool {
        matches!(self.kind, FlatTreeKind::Expanded)
    }

    const fn status(&self) -> bool {
        matches!(self.kind, FlatTreeKind::Status)
    }
}

type SharedKeyCallback = Rc<RefCell<Box<dyn FnMut(WidgetKey)>>>;
type SharedSelectionCallback = Rc<RefCell<Box<dyn FnMut(Vec<WidgetKey>)>>>;
type SharedExpansionCallback = Rc<RefCell<Box<dyn FnMut(WidgetKey, bool)>>>;
type SharedDropCallback = Rc<RefCell<Box<dyn FnMut(CollectionDrop)>>>;

fn with_shared_drop(list: ListView, enabled: bool, callback: &SharedDropCallback) -> ListView {
    let callback = callback.clone();
    list.draggable(enabled)
        .on_drop(move |drop| (callback.borrow_mut())(drop))
}

fn tree_status_key(parent: &WidgetKey, suffix: &str) -> WidgetKey {
    WidgetKey::new(format!("\0vui.tree.{}.{}", parent.as_str(), suffix))
}

fn flatten_tree(
    nodes: &[TreeNode],
    depth: usize,
    parent: Option<&WidgetKey>,
    expanded: &[WidgetKey],
    output: &mut Vec<FlatTreeNode>,
) {
    for node in nodes {
        let is_expanded = expanded.contains(&node.key);
        let expandable = !matches!(node.children, TreeChildren::None)
            && !matches!(&node.children, TreeChildren::Loaded(children) if children.is_empty());
        output.push(FlatTreeNode {
            key: node.key.clone(),
            label: node.label.clone(),
            depth,
            parent: parent.cloned(),
            kind: if expandable {
                if is_expanded {
                    FlatTreeKind::Expanded
                } else {
                    FlatTreeKind::Collapsed
                }
            } else {
                FlatTreeKind::Leaf
            },
            disabled: node.disabled,
        });
        if !is_expanded {
            continue;
        }
        match &node.children {
            TreeChildren::Loaded(children) => {
                flatten_tree(children, depth + 1, Some(&node.key), expanded, output);
            }
            TreeChildren::Unloaded => output.push(FlatTreeNode {
                key: tree_status_key(&node.key, "unloaded"),
                label: "Load children…".to_owned(),
                depth: depth + 1,
                parent: Some(node.key.clone()),
                kind: FlatTreeKind::Status,
                disabled: true,
            }),
            TreeChildren::Loading => output.push(FlatTreeNode {
                key: tree_status_key(&node.key, "loading"),
                label: "Loading…".to_owned(),
                depth: depth + 1,
                parent: Some(node.key.clone()),
                kind: FlatTreeKind::Status,
                disabled: true,
            }),
            TreeChildren::Error(message) => output.push(FlatTreeNode {
                key: tree_status_key(&node.key, "error"),
                label: format!("Error: {message}"),
                depth: depth + 1,
                parent: Some(node.key.clone()),
                kind: FlatTreeKind::Status,
                disabled: true,
            }),
            TreeChildren::None => {}
        }
    }
}

fn make_tree_list(
    entries: &[FlatTreeNode],
    row_height: f64,
    overscan: usize,
    viewport_height: Option<f64>,
    multi_select: bool,
    selection_callback: &SharedSelectionCallback,
    activation_callback: &SharedKeyCallback,
) -> ListView {
    let rows = Rc::new(entries.to_vec());
    let factory_rows = rows.clone();
    let selection = selection_callback.clone();
    let activation = activation_callback.clone();
    let mut list = ListView::new(rows.len(), move |index| {
        let entry = &factory_rows[index];
        let indent = index_position(entry.depth) * 16.0;
        let disclosure = if entry.expandable() {
            if entry.expanded() { "▾" } else { "▸" }
        } else if entry.status() {
            "·"
        } else {
            " "
        };
        let text = format!("{disclosure}  {}", entry.label);
        ListItem::new(
            entry.key.clone(),
            entry.label.clone(),
            crate::Padding::new(
                crate::Insets {
                    left: indent,
                    top: 0.0,
                    right: 0.0,
                    bottom: 0.0,
                },
                crate::Label::new(text),
            ),
        )
        .disabled(entry.disabled || entry.status())
    })
    .row_height(row_height)
    .overscan(overscan)
    .multi_select(multi_select)
    .on_selection_change(move |keys| (selection.borrow_mut())(keys))
    .on_activate(move |key| (activation.borrow_mut())(key))
    .semantic_roles(SemanticRole::Tree, SemanticRole::TreeItem, "Tree");
    if let Some(height) = viewport_height {
        list = list.viewport_height(height);
    }
    list
}

/// A virtualized hierarchical collection with retained expansion state.
pub struct TreeView {
    nodes: Vec<TreeNode>,
    entries: Vec<FlatTreeNode>,
    expanded: Vec<WidgetKey>,
    controlled_expansion: bool,
    list: ListView,
    row_height: f64,
    overscan: usize,
    viewport_height: Option<f64>,
    multi_select: bool,
    on_selection_change: SharedSelectionCallback,
    on_activate: SharedKeyCallback,
    on_expansion_change: SharedExpansionCallback,
    drag_enabled: bool,
    on_drop: SharedDropCallback,
}

impl TreeView {
    #[must_use]
    pub fn new(nodes: impl IntoIterator<Item = TreeNode>) -> Self {
        let nodes = nodes.into_iter().collect::<Vec<_>>();
        let mut entries = Vec::new();
        flatten_tree(&nodes, 0, None, &[], &mut entries);
        let selection: SharedSelectionCallback = Rc::new(RefCell::new(Box::new(|_| {})));
        let activation: SharedKeyCallback = Rc::new(RefCell::new(Box::new(|_| {})));
        let expansion: SharedExpansionCallback = Rc::new(RefCell::new(Box::new(|_, _| {})));
        let drop_callback: SharedDropCallback = Rc::new(RefCell::new(Box::new(|_| {})));
        let list = with_shared_drop(
            make_tree_list(&entries, 36.0, 2, None, true, &selection, &activation),
            false,
            &drop_callback,
        );
        Self {
            nodes,
            entries,
            expanded: Vec::new(),
            controlled_expansion: false,
            list,
            row_height: 36.0,
            overscan: 2,
            viewport_height: None,
            multi_select: true,
            on_selection_change: selection,
            on_activate: activation,
            on_expansion_change: expansion,
            drag_enabled: false,
            on_drop: drop_callback,
        }
    }

    #[must_use]
    pub fn row_height(mut self, height: f64) -> Self {
        self.row_height = height.max(1.0);
        self.rebuild(false);
        self
    }

    #[must_use]
    pub fn viewport_height(mut self, height: f64) -> Self {
        self.viewport_height = Some(height.max(0.0));
        self.rebuild(false);
        self
    }

    #[must_use]
    pub fn overscan(mut self, rows: usize) -> Self {
        self.overscan = rows;
        self.rebuild(false);
        self
    }

    #[must_use]
    pub fn multi_select(mut self, enabled: bool) -> Self {
        self.multi_select = enabled;
        self.rebuild(false);
        self
    }

    #[must_use]
    pub fn expanded(mut self, keys: impl IntoIterator<Item = WidgetKey>) -> Self {
        self.expanded = keys.into_iter().collect();
        self.controlled_expansion = true;
        self.rebuild(false);
        self
    }

    #[must_use]
    pub fn on_selection_change(self, callback: impl FnMut(Vec<WidgetKey>) + 'static) -> Self {
        *self.on_selection_change.borrow_mut() = Box::new(callback);
        self
    }

    #[must_use]
    pub fn on_activate(self, callback: impl FnMut(WidgetKey) + 'static) -> Self {
        *self.on_activate.borrow_mut() = Box::new(callback);
        self
    }

    #[must_use]
    pub fn on_expansion_change(self, callback: impl FnMut(WidgetKey, bool) + 'static) -> Self {
        *self.on_expansion_change.borrow_mut() = Box::new(callback);
        self
    }

    #[must_use]
    pub fn draggable(mut self, enabled: bool) -> Self {
        self.drag_enabled = enabled;
        self.list.drag_enabled = enabled;
        self
    }

    #[must_use]
    pub fn on_drop(self, callback: impl FnMut(CollectionDrop) + 'static) -> Self {
        *self.on_drop.borrow_mut() = Box::new(callback);
        self
    }

    #[must_use]
    pub fn expanded_keys(&self) -> &[WidgetKey] {
        &self.expanded
    }

    fn rebuild(&mut self, preserve_list_state: bool) {
        self.entries.clear();
        flatten_tree(&self.nodes, 0, None, &self.expanded, &mut self.entries);
        let mut next = with_shared_drop(
            make_tree_list(
                &self.entries,
                self.row_height,
                self.overscan,
                self.viewport_height,
                self.multi_select,
                &self.on_selection_change,
                &self.on_activate,
            ),
            self.drag_enabled,
            &self.on_drop,
        );
        if preserve_list_state {
            self.list.reconcile(&mut next);
        }
        self.list = next;
    }

    fn set_expanded(&mut self, index: usize, expanded: bool, ctx: &mut EventCtx) {
        let Some(entry) = self.entries.get(index).filter(|entry| entry.expandable()) else {
            return;
        };
        let key = entry.key.clone();
        if expanded {
            if self.expanded.contains(&key) {
                return;
            }
            self.expanded.push(key.clone());
        } else if let Some(position) = self.expanded.iter().position(|item| *item == key) {
            self.expanded.remove(position);
        } else {
            return;
        }
        (self.on_expansion_change.borrow_mut())(key, expanded);
        self.rebuild(true);
        ctx.request_layout();
        ctx.set_handled();
    }

    fn handle_horizontal_key(&mut self, key: &Key, ctx: &mut EventCtx) -> bool {
        if !self.list.is_focused() || !matches!(key, Key::ArrowLeft | Key::ArrowRight) {
            return false;
        }
        let Some(index) = self.list.active_index() else {
            return false;
        };
        let Some(entry) = self.entries.get(index).cloned() else {
            return false;
        };
        match key {
            Key::ArrowRight if entry.expandable() && !entry.expanded() => {
                self.set_expanded(index, true, ctx);
            }
            Key::ArrowRight if entry.expanded() => {
                if self
                    .entries
                    .get(index + 1)
                    .is_some_and(|next| next.depth > entry.depth)
                {
                    self.list.focus_item(index + 1, ctx);
                }
            }
            Key::ArrowLeft if entry.expanded() => self.set_expanded(index, false, ctx),
            Key::ArrowLeft => {
                if let Some(parent) = entry.parent
                    && let Some(parent_index) =
                        self.entries.iter().position(|item| item.key == parent)
                {
                    self.list.focus_item(parent_index, ctx);
                }
            }
            _ => {}
        }
        ctx.set_handled();
        true
    }
}

impl Widget for TreeView {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.list.layout(ctx, constraints)
    }

    fn set_origin(&mut self, origin: Point) {
        self.list.set_origin(origin);
    }

    fn bounds(&self) -> Rect {
        self.list.bounds()
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &Event) {
        if let Event::KeyDown(key) = event
            && self.handle_horizontal_key(key, ctx)
        {
            return;
        }
        if let Event::PointerDown {
            position,
            button: crate::PointerButton::Primary,
            ..
        } = event
            && let Some(index) = self.list.row_index_at(*position)
            && let Some(entry) = self.entries.get(index)
            && entry.expandable()
        {
            let disclosure_right = self.list.bounds().x + 32.0 + index_position(entry.depth) * 16.0;
            if position.x <= disclosure_right {
                let expanded = !entry.expanded();
                self.list.event(ctx, event);
                self.set_expanded(index, expanded, ctx);
                return;
            }
        }
        self.list.event(ctx, event);
    }

    fn widget_id(&self) -> Option<WidgetId> {
        self.list.widget_id()
    }

    fn event_target(&mut self, ctx: &mut EventCtx, event: &Event, target: WidgetId) -> bool {
        self.list.event_target(ctx, event, target)
    }

    fn apply_command(&mut self, command: UiCommand) -> Result<(), UiCommand> {
        self.list.apply_command(command)
    }

    fn reconcile(&mut self, next: &mut dyn Widget) {
        if let Some(next) = next.as_any_mut().downcast_mut::<Self>() {
            if !next.controlled_expansion {
                next.expanded.clone_from(&self.expanded);
            }
            next.rebuild(false);
            self.list.reconcile(&mut next.list);
        }
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        self.list.paint(ctx);
    }

    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        self.list.semantics(ctx);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TableAlignment {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TableColumn {
    pub id: WidgetKey,
    pub header: String,
    pub width: f64,
    pub min_width: f64,
    pub max_width: f64,
    pub alignment: TableAlignment,
    pub sortable: bool,
    pub visible: bool,
}

impl TableColumn {
    #[must_use]
    pub fn new(id: impl Into<WidgetKey>, header: impl Into<String>, width: f64) -> Self {
        let width = width.max(24.0);
        Self {
            id: id.into(),
            header: header.into(),
            width,
            min_width: 48.0_f64.min(width),
            max_width: f64::INFINITY,
            alignment: TableAlignment::Left,
            sortable: true,
            visible: true,
        }
    }

    #[must_use]
    pub fn width_range(mut self, min: f64, max: f64) -> Self {
        self.min_width = min.max(24.0);
        self.max_width = max.max(self.min_width);
        self.width = self.width.clamp(self.min_width, self.max_width);
        self
    }

    #[must_use]
    pub const fn alignment(mut self, alignment: TableAlignment) -> Self {
        self.alignment = alignment;
        self
    }

    #[must_use]
    pub const fn sortable(mut self, sortable: bool) -> Self {
        self.sortable = sortable;
        self
    }

    #[must_use]
    pub const fn hidden(mut self, hidden: bool) -> Self {
        self.visible = !hidden;
        self
    }
}

#[derive(Clone)]
struct TableLayoutState {
    columns: Vec<TableColumn>,
    scroll_x: f64,
    sort: Option<(WidgetKey, SortDirection)>,
}

struct TableCell {
    column: WidgetKey,
    label: String,
    width: f64,
    x: f64,
    alignment: TableAlignment,
    id: WidgetId,
    bounds: Rect,
    layout: TextLayout,
}

struct TableRowCells {
    row: usize,
    cells: Vec<TableCell>,
    bounds: Rect,
}

impl Widget for TableRowCells {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        for cell in &mut self.cells {
            cell.layout = ctx.layout_text(&cell.label, ctx.theme().typography.body, None);
        }
        self.bounds.width = constraints.max.width;
        self.bounds.height = constraints.max.height;
        Size::new(self.bounds.width, self.bounds.height)
    }

    fn set_origin(&mut self, origin: Point) {
        self.bounds.x = origin.x;
        self.bounds.y = origin.y;
        for cell in &mut self.cells {
            cell.bounds = Rect::new(origin.x + cell.x, origin.y, cell.width, self.bounds.height);
        }
    }

    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn event(&mut self, _ctx: &mut EventCtx, _event: &Event) {}

    fn reconcile(&mut self, next: &mut dyn Widget) {
        if let Some(next) = next.as_any_mut().downcast_mut::<Self>() {
            for cell in &mut next.cells {
                if let Some(current) = self.cells.iter().find(|item| item.column == cell.column) {
                    cell.id = current.id;
                }
            }
        }
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        for cell in &self.cells {
            ctx.push(PaintOp::PushClip {
                rect: cell.bounds,
                radius: 0.0,
            });
            let x = match cell.alignment {
                TableAlignment::Left => cell.bounds.x + 8.0,
                TableAlignment::Center => {
                    cell.bounds.x + (cell.bounds.width - cell.layout.size.width) * 0.5
                }
                TableAlignment::Right => {
                    cell.bounds.x + cell.bounds.width - cell.layout.size.width - 8.0
                }
            };
            let y = cell.bounds.y + (cell.bounds.height - cell.layout.size.height) * 0.5;
            crate::widgets::paint_text_layout(
                ctx,
                &cell.layout,
                Point::new(x, y),
                ctx.theme().colors.text,
                ctx.theme().typography.body,
            );
            ctx.push(PaintOp::PopClip);
        }
    }

    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        for cell in &self.cells {
            ctx.push(SemanticNode {
                id: cell.id,
                role: SemanticRole::Cell,
                bounds: cell.bounds,
                label: cell.label.clone(),
                disabled: false,
                focused: false,
                checked: None,
                value: Some(format!(
                    "Row {}, column {}",
                    self.row + 1,
                    cell.column.as_str()
                )),
                ime_cursor_area: None,
            });
        }
    }
}

type SharedCellFactory = Rc<RefCell<Box<dyn FnMut(usize, &WidgetKey) -> String>>>;
type SharedRowKeyFactory = Rc<dyn Fn(usize) -> WidgetKey>;
type SharedSortCallback = Rc<RefCell<Box<dyn FnMut(WidgetKey, Option<SortDirection>)>>>;
type SharedOrderCallback = Rc<RefCell<Box<dyn FnMut(Vec<WidgetKey>)>>>;

fn make_table_list(
    row_count: usize,
    state: &Rc<RefCell<TableLayoutState>>,
    cell_factory: &SharedCellFactory,
    row_key_factory: &SharedRowKeyFactory,
    row_height: f64,
    selection_callback: &SharedSelectionCallback,
    activation_callback: &SharedKeyCallback,
) -> ListView {
    let row_state = state.clone();
    let cells = cell_factory.clone();
    let row_keys = row_key_factory.clone();
    let selection = selection_callback.clone();
    let activation = activation_callback.clone();
    ListView::new(row_count, move |row| {
        let state = row_state.borrow();
        let columns = state
            .columns
            .iter()
            .filter(|column| column.visible)
            .cloned()
            .collect::<Vec<_>>();
        let mut x = -state.scroll_x;
        drop(state);
        let mut materialized = Vec::with_capacity(columns.len());
        for column in columns {
            let label = (cells.borrow_mut())(row, &column.id);
            materialized.push(TableCell {
                column: column.id,
                label,
                width: column.width,
                x,
                alignment: column.alignment,
                id: WidgetId::next(),
                bounds: Rect::default(),
                layout: TextLayout::default(),
            });
            x += column.width;
        }
        ListItem::new(
            row_keys(row),
            format!("Row {}", row + 1),
            TableRowCells {
                row,
                cells: materialized,
                bounds: Rect::default(),
            },
        )
    })
    .row_height(row_height)
    .content_inset(0.0)
    .container_semantics(false)
    .include_child_semantics(true)
    .semantic_roles(SemanticRole::Table, SemanticRole::TableRow, "Table")
    .on_selection_change(move |keys| (selection.borrow_mut())(keys))
    .on_activate(move |key| (activation.borrow_mut())(key))
}

struct TableHeaderCell {
    column: WidgetKey,
    id: WidgetId,
    bounds: Rect,
    layout: TextLayout,
}

enum TablePointerDrag {
    Resize {
        column: WidgetKey,
        start_x: f64,
        start_width: f64,
    },
    Reorder {
        column: WidgetKey,
        start_x: f64,
        moved: bool,
    },
}

/// A virtualized data table with a frozen, interactive header.
pub struct TableView {
    id: WidgetId,
    row_count: usize,
    state: Rc<RefCell<TableLayoutState>>,
    cell_factory: SharedCellFactory,
    row_key_factory: SharedRowKeyFactory,
    list: ListView,
    row_height: f64,
    header_height: f64,
    viewport_height: Option<f64>,
    bounds: Rect,
    header_cells: Vec<TableHeaderCell>,
    pointer_drag: Option<TablePointerDrag>,
    on_selection_change: SharedSelectionCallback,
    on_activate: SharedKeyCallback,
    on_sort_change: SharedSortCallback,
    on_column_order_change: SharedOrderCallback,
    controlled_sort: bool,
    drag_enabled: bool,
    on_drop: SharedDropCallback,
}

impl TableView {
    #[must_use]
    pub fn new(
        row_count: usize,
        columns: impl IntoIterator<Item = TableColumn>,
        cell_factory: impl FnMut(usize, &WidgetKey) -> String + 'static,
    ) -> Self {
        let state = Rc::new(RefCell::new(TableLayoutState {
            columns: columns.into_iter().collect(),
            scroll_x: 0.0,
            sort: None,
        }));
        let cells: SharedCellFactory = Rc::new(RefCell::new(Box::new(cell_factory)));
        let row_keys: SharedRowKeyFactory =
            Rc::new(|row| WidgetKey::new(format!("\0vui.table.row.{row}")));
        let selection: SharedSelectionCallback = Rc::new(RefCell::new(Box::new(|_| {})));
        let activation: SharedKeyCallback = Rc::new(RefCell::new(Box::new(|_| {})));
        let sort: SharedSortCallback = Rc::new(RefCell::new(Box::new(|_, _| {})));
        let order: SharedOrderCallback = Rc::new(RefCell::new(Box::new(|_| {})));
        let drop_callback: SharedDropCallback = Rc::new(RefCell::new(Box::new(|_| {})));
        let list = with_shared_drop(
            make_table_list(
                row_count,
                &state,
                &cells,
                &row_keys,
                32.0,
                &selection,
                &activation,
            ),
            false,
            &drop_callback,
        );
        Self {
            id: WidgetId::next(),
            row_count,
            state,
            cell_factory: cells,
            row_key_factory: row_keys,
            list,
            row_height: 32.0,
            header_height: 34.0,
            viewport_height: None,
            bounds: Rect::default(),
            header_cells: Vec::new(),
            pointer_drag: None,
            on_selection_change: selection,
            on_activate: activation,
            on_sort_change: sort,
            on_column_order_change: order,
            controlled_sort: false,
            drag_enabled: false,
            on_drop: drop_callback,
        }
    }

    #[must_use]
    pub fn row_height(mut self, height: f64) -> Self {
        self.row_height = height.max(1.0);
        self.rebuild_list(false);
        self
    }

    #[must_use]
    pub fn header_height(mut self, height: f64) -> Self {
        self.header_height = height.max(1.0);
        self
    }

    #[must_use]
    pub fn viewport_height(mut self, height: f64) -> Self {
        self.viewport_height = Some(height.max(self.header_height));
        self
    }

    #[must_use]
    pub fn sort(self, column: impl Into<WidgetKey>, direction: SortDirection) -> Self {
        self.state.borrow_mut().sort = Some((column.into(), direction));
        Self {
            controlled_sort: true,
            ..self
        }
    }

    #[must_use]
    pub fn row_keys(mut self, factory: impl Fn(usize) -> WidgetKey + 'static) -> Self {
        self.row_key_factory = Rc::new(factory);
        self.rebuild_list(false);
        self
    }

    #[must_use]
    pub fn on_selection_change(self, callback: impl FnMut(Vec<WidgetKey>) + 'static) -> Self {
        *self.on_selection_change.borrow_mut() = Box::new(callback);
        self
    }

    #[must_use]
    pub fn on_activate(self, callback: impl FnMut(WidgetKey) + 'static) -> Self {
        *self.on_activate.borrow_mut() = Box::new(callback);
        self
    }

    #[must_use]
    pub fn on_sort_change(
        self,
        callback: impl FnMut(WidgetKey, Option<SortDirection>) + 'static,
    ) -> Self {
        *self.on_sort_change.borrow_mut() = Box::new(callback);
        self
    }

    #[must_use]
    pub fn on_column_order_change(self, callback: impl FnMut(Vec<WidgetKey>) + 'static) -> Self {
        *self.on_column_order_change.borrow_mut() = Box::new(callback);
        self
    }

    #[must_use]
    pub fn draggable(mut self, enabled: bool) -> Self {
        self.drag_enabled = enabled;
        self.list.drag_enabled = enabled;
        self
    }

    #[must_use]
    pub fn on_drop(self, callback: impl FnMut(CollectionDrop) + 'static) -> Self {
        *self.on_drop.borrow_mut() = Box::new(callback);
        self
    }

    #[must_use]
    pub fn columns(&self) -> Vec<TableColumn> {
        self.state.borrow().columns.clone()
    }

    fn rebuild_list(&mut self, preserve: bool) {
        let mut next = with_shared_drop(
            make_table_list(
                self.row_count,
                &self.state,
                &self.cell_factory,
                &self.row_key_factory,
                self.row_height,
                &self.on_selection_change,
                &self.on_activate,
            ),
            self.drag_enabled,
            &self.on_drop,
        );
        if preserve {
            self.list.reconcile(&mut next);
        }
        self.list = next;
    }

    fn total_width(&self) -> f64 {
        self.state
            .borrow()
            .columns
            .iter()
            .filter(|column| column.visible)
            .map(|column| column.width)
            .sum()
    }

    fn max_scroll_x(&self) -> f64 {
        (self.total_width() - self.bounds.width).max(0.0)
    }

    fn layout_headers(&mut self, ctx: &mut LayoutCtx<'_>) {
        let previous = std::mem::take(&mut self.header_cells);
        let state = self.state.borrow();
        let mut x = self.bounds.x - state.scroll_x;
        for column in state.columns.iter().filter(|column| column.visible) {
            let id = previous
                .iter()
                .find(|header| header.column == column.id)
                .map_or_else(WidgetId::next, |header| header.id);
            let indicator = match &state.sort {
                Some((id, SortDirection::Ascending)) if *id == column.id => " ▲",
                Some((id, SortDirection::Descending)) if *id == column.id => " ▼",
                _ => "",
            };
            self.header_cells.push(TableHeaderCell {
                column: column.id.clone(),
                id,
                bounds: Rect::new(x, self.bounds.y, column.width, self.header_height),
                layout: ctx.layout_text(
                    &format!("{}{indicator}", column.header),
                    ctx.theme().typography.label,
                    None,
                ),
            });
            x += column.width;
        }
    }

    fn header_at(&self, point: Point) -> Option<WidgetKey> {
        self.header_cells
            .iter()
            .find(|header| header.bounds.contains(point))
            .map(|header| header.column.clone())
    }

    fn divider_at(&self, point: Point) -> Option<WidgetKey> {
        self.header_cells
            .iter()
            .find(|header| {
                point.y >= header.bounds.y
                    && point.y < header.bounds.y + header.bounds.height
                    && (point.x - header.bounds.x - header.bounds.width).abs() <= 4.0
            })
            .map(|header| header.column.clone())
    }

    fn sort_column(&mut self, key: &WidgetKey, ctx: &mut EventCtx) {
        let mut state = self.state.borrow_mut();
        let sortable = state
            .columns
            .iter()
            .find(|column| column.id == *key)
            .is_some_and(|column| column.sortable);
        if !sortable {
            return;
        }
        let direction = match &state.sort {
            Some((column, SortDirection::Ascending)) if column == key => {
                Some(SortDirection::Descending)
            }
            Some((column, SortDirection::Descending)) if column == key => None,
            _ => Some(SortDirection::Ascending),
        };
        state.sort = direction.map(|direction| (key.clone(), direction));
        drop(state);
        (self.on_sort_change.borrow_mut())(key.clone(), direction);
        ctx.request_layout();
        ctx.set_handled();
    }

    fn resize_column(&mut self, key: &WidgetKey, width: f64, ctx: &mut EventCtx) {
        {
            let mut state = self.state.borrow_mut();
            let Some(column) = state.columns.iter_mut().find(|column| column.id == *key) else {
                return;
            };
            column.width = width.clamp(column.min_width, column.max_width);
        }
        let max_scroll = self.max_scroll_x();
        let scroll_x = self.state.borrow().scroll_x.min(max_scroll);
        self.state.borrow_mut().scroll_x = scroll_x;
        ctx.request_layout();
        ctx.set_handled();
    }

    fn reorder_column(&mut self, key: &WidgetKey, target: &WidgetKey, ctx: &mut EventCtx) {
        let mut state = self.state.borrow_mut();
        let Some(from) = state.columns.iter().position(|column| column.id == *key) else {
            return;
        };
        let Some(to) = state.columns.iter().position(|column| column.id == *target) else {
            return;
        };
        if from == to {
            return;
        }
        let column = state.columns.remove(from);
        state.columns.insert(to, column);
        let order = state
            .columns
            .iter()
            .map(|column| column.id.clone())
            .collect();
        drop(state);
        (self.on_column_order_change.borrow_mut())(order);
        self.rebuild_list(true);
        ctx.request_layout();
        ctx.set_handled();
    }

    fn handle_header_pointer(&mut self, event: &Event, ctx: &mut EventCtx) -> bool {
        match event {
            Event::PointerDown {
                position,
                button: crate::PointerButton::Primary,
                ..
            } if position.y < self.bounds.y + self.header_height => {
                if let Some(column) = self.divider_at(*position) {
                    let width = self
                        .state
                        .borrow()
                        .columns
                        .iter()
                        .find(|item| item.id == column)
                        .map_or(0.0, |item| item.width);
                    self.pointer_drag = Some(TablePointerDrag::Resize {
                        column,
                        start_x: position.x,
                        start_width: width,
                    });
                } else if let Some(column) = self.header_at(*position) {
                    self.pointer_drag = Some(TablePointerDrag::Reorder {
                        column,
                        start_x: position.x,
                        moved: false,
                    });
                }
                if self.pointer_drag.is_some() {
                    ctx.capture_pointer(self.id);
                    ctx.request_focus(self.list.widget_id().expect("list has an ID"));
                    ctx.set_handled();
                }
                true
            }
            Event::PointerMoved(position) if self.pointer_drag.is_some() => {
                let drag = self.pointer_drag.take().expect("checked table drag");
                match drag {
                    TablePointerDrag::Resize {
                        column,
                        start_x,
                        start_width,
                    } => {
                        self.resize_column(&column, start_width + position.x - start_x, ctx);
                        self.pointer_drag = Some(TablePointerDrag::Resize {
                            column,
                            start_x,
                            start_width,
                        });
                    }
                    TablePointerDrag::Reorder {
                        column,
                        start_x,
                        mut moved,
                    } => {
                        if (position.x - start_x).abs() > 16.0
                            && let Some(target) = self.header_at(*position)
                            && target != column
                        {
                            self.reorder_column(&column, &target, ctx);
                            moved = true;
                        }
                        self.pointer_drag = Some(TablePointerDrag::Reorder {
                            column,
                            start_x,
                            moved,
                        });
                    }
                }
                true
            }
            Event::PointerUp {
                button: crate::PointerButton::Primary,
                ..
            } if self.pointer_drag.is_some() => {
                if let Some(TablePointerDrag::Reorder {
                    column,
                    moved: false,
                    ..
                }) = self.pointer_drag.take()
                {
                    self.sort_column(&column, ctx);
                }
                self.pointer_drag = None;
                ctx.release_pointer();
                ctx.request_layout();
                ctx.set_handled();
                true
            }
            _ => false,
        }
    }
}

impl Widget for TableView {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let width = if constraints.max.width.is_finite() {
            constraints.max.width
        } else {
            self.total_width()
        };
        let content_height = self.header_height + self.row_height * index_position(self.row_count);
        let height = self.viewport_height.unwrap_or(content_height);
        let size = constraints.constrain(Size::new(width, height));
        self.bounds.width = size.width;
        self.bounds.height = size.height;
        let max_scroll = self.max_scroll_x();
        let scroll_x = self.state.borrow().scroll_x.min(max_scroll);
        self.state.borrow_mut().scroll_x = scroll_x;
        let body_height = (size.height - self.header_height).max(0.0);
        self.list.layout(
            ctx,
            BoxConstraints::tight(Size::new(size.width, body_height)),
        );
        self.list.set_origin(Point::new(
            self.bounds.x,
            self.bounds.y + self.header_height,
        ));
        self.layout_headers(ctx);
        size
    }

    fn set_origin(&mut self, origin: Point) {
        let dx = origin.x - self.bounds.x;
        let dy = origin.y - self.bounds.y;
        self.bounds.x = origin.x;
        self.bounds.y = origin.y;
        self.list
            .set_origin(Point::new(origin.x, origin.y + self.header_height));
        for header in &mut self.header_cells {
            header.bounds.x += dx;
            header.bounds.y += dy;
        }
    }

    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &Event) {
        if self.handle_header_pointer(event, ctx) {
            return;
        }
        match event {
            Event::Scroll {
                position, delta_x, ..
            } if self.bounds.contains(*position) && delta_x.abs() > f64::EPSILON => {
                let max_scroll = self.max_scroll_x();
                let mut state = self.state.borrow_mut();
                let next = (state.scroll_x + delta_x).clamp(0.0, max_scroll);
                if (next - state.scroll_x).abs() > f64::EPSILON {
                    state.scroll_x = next;
                    drop(state);
                    self.rebuild_list(true);
                    ctx.request_layout();
                    ctx.set_handled();
                }
                self.list.event(ctx, event);
            }
            Event::AccessibilityAction {
                target,
                action: SemanticAction::Click,
            } => {
                if let Some(column) = self
                    .header_cells
                    .iter()
                    .find(|header| header.id == *target)
                    .map(|header| header.column.clone())
                {
                    self.sort_column(&column, ctx);
                } else {
                    self.list.event(ctx, event);
                }
            }
            _ => self.list.event(ctx, event),
        }
    }

    fn widget_id(&self) -> Option<WidgetId> {
        Some(self.id)
    }

    fn event_target(&mut self, ctx: &mut EventCtx, event: &Event, target: WidgetId) -> bool {
        if target == self.id || self.header_cells.iter().any(|header| header.id == target) {
            self.event(ctx, event);
            true
        } else {
            self.list.event_target(ctx, event, target)
        }
    }

    fn apply_command(&mut self, command: UiCommand) -> Result<(), UiCommand> {
        self.list.apply_command(command)
    }

    fn reconcile(&mut self, next: &mut dyn Widget) {
        if let Some(next) = next.as_any_mut().downcast_mut::<Self>() {
            next.id = self.id;
            let current_state = self.state.borrow();
            let mut next_state = next.state.borrow_mut();
            next_state.scroll_x = current_state.scroll_x;
            if !next.controlled_sort {
                next_state.sort.clone_from(&current_state.sort);
            }
            let mut pending = std::mem::take(&mut next_state.columns);
            let mut ordered = Vec::with_capacity(pending.len());
            for current in &current_state.columns {
                if let Some(position) = pending.iter().position(|column| column.id == current.id) {
                    let mut column = pending.remove(position);
                    column.width = current.width.clamp(column.min_width, column.max_width);
                    ordered.push(column);
                }
            }
            ordered.append(&mut pending);
            next_state.columns = ordered;
            drop(next_state);
            drop(current_state);
            next.pointer_drag = None;
            self.list.reconcile(&mut next.list);
        }
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        self.list.paint(ctx);
        ctx.push(PaintOp::PushClip {
            rect: Rect::new(
                self.bounds.x,
                self.bounds.y,
                self.bounds.width,
                self.header_height,
            ),
            radius: 0.0,
        });
        ctx.push(PaintOp::FillRect {
            rect: Rect::new(
                self.bounds.x,
                self.bounds.y,
                self.bounds.width,
                self.header_height,
            ),
            color: ctx.theme().colors.surface_raised,
            radius: 0.0,
        });
        for header in &self.header_cells {
            crate::widgets::paint_text_layout(
                ctx,
                &header.layout,
                Point::new(header.bounds.x + 8.0, header.bounds.y + 8.0),
                ctx.theme().colors.text,
                ctx.theme().typography.label,
            );
            ctx.push(PaintOp::FillRect {
                rect: Rect::new(
                    header.bounds.x + header.bounds.width - 1.0,
                    header.bounds.y,
                    1.0,
                    header.bounds.height,
                ),
                color: ctx.theme().colors.border,
                radius: 0.0,
            });
        }
        ctx.push(PaintOp::PopClip);
    }

    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        ctx.push(SemanticNode {
            id: self.list.widget_id().expect("list has an ID"),
            role: SemanticRole::Table,
            bounds: self.bounds,
            label: "Table".to_owned(),
            disabled: false,
            focused: self.list.is_focused(),
            checked: None,
            value: Some(format!("{} rows", self.row_count)),
            ime_cursor_area: None,
        });
        for header in &self.header_cells {
            ctx.push(SemanticNode {
                id: header.id,
                role: SemanticRole::ColumnHeader,
                bounds: header.bounds,
                label: self
                    .state
                    .borrow()
                    .columns
                    .iter()
                    .find(|column| column.id == header.column)
                    .map_or_else(String::new, |column| column.header.clone()),
                disabled: false,
                focused: false,
                checked: None,
                value: None,
                ime_cursor_area: None,
            });
        }
        self.list.semantics(ctx);
    }
}
