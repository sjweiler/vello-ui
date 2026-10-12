use crate::{
    BoxConstraints, ButtonVariant, Color, ControlState, Easing, Event, EventCtx, Insets,
    Interpolate, LayoutCtx, OverlayAnchor, PaintCtx, PaintOp, Point, Rect, SemanticNode,
    SemanticRole, SemanticsCtx, Size, TextLayout, Tween, UiCommand, Widget, WidgetId, WidgetKey,
};
use std::time::Duration;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Default)]
struct Interaction {
    hovered: bool,
    pressed: bool,
    focused: bool,
}

fn reconcile_children(current: &mut [Box<dyn Widget>], next: &mut [Box<dyn Widget>]) {
    for (index, next_child) in next.iter_mut().enumerate() {
        let key = next_child.widget_key().cloned();
        let current_child = if let Some(key) = key {
            current
                .iter_mut()
                .find(|child| child.widget_key() == Some(&key))
        } else {
            current
                .get_mut(index)
                .filter(|child| child.widget_key().is_none())
        };
        if let Some(current_child) = current_child {
            current_child.reconcile(next_child.as_mut());
        }
    }
}

fn route_children(
    children: &mut [Box<dyn Widget>],
    hovered_child: &mut Option<usize>,
    ctx: &mut EventCtx,
    event: &Event,
) {
    match event {
        Event::PointerMoved(position) => {
            let next = children
                .iter()
                .rposition(|child| child.bounds().contains(*position));
            if *hovered_child != next {
                if let Some(previous) = hovered_child.take() {
                    children[previous].event(ctx, &Event::PointerLeft);
                }
                *hovered_child = next;
            }
            if let Some(index) = next {
                children[index].event(ctx, event);
            }
        }
        Event::PointerLeft => {
            if let Some(previous) = hovered_child.take() {
                children[previous].event(ctx, event);
            }
        }
        Event::PointerDown { position, .. }
        | Event::PointerUp { position, .. }
        | Event::Scroll { position, .. } => {
            if let Some(child) = children
                .iter_mut()
                .rfind(|child| child.bounds().contains(*position))
            {
                child.event(ctx, event);
            }
        }
        _ => {
            for child in children {
                child.event(ctx, event);
                if ctx.is_handled() {
                    break;
                }
            }
        }
    }
}

fn event_target_children(
    children: &mut [Box<dyn Widget>],
    ctx: &mut EventCtx,
    event: &Event,
    target: WidgetId,
) -> bool {
    children
        .iter_mut()
        .rev()
        .any(|child| child.event_target(ctx, event, target))
}

fn apply_command_children(
    children: &mut [Box<dyn Widget>],
    mut command: UiCommand,
) -> Result<(), UiCommand> {
    for child in children.iter_mut().rev() {
        match child.apply_command(command) {
            Ok(()) => return Ok(()),
            Err(returned) => command = returned,
        }
    }
    Err(command)
}

pub struct Label {
    id: WidgetId,
    text: String,
    size: Option<f32>,
    resolved_size: f32,
    color: Option<Color>,
    bounds: Rect,
    layout: TextLayout,
}

impl Label {
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            id: WidgetId::next(),
            text: text.into(),
            size: None,
            resolved_size: 16.0,
            color: None,
            bounds: Rect::default(),
            layout: TextLayout::default(),
        }
    }

    #[must_use]
    pub fn text_size(mut self, size: f32) -> Self {
        self.size = Some(size);
        self
    }
}

impl Widget for Label {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.resolved_size = self.size.unwrap_or(ctx.theme().typography.body);
        self.layout = ctx.layout_text(&self.text, self.resolved_size, None);
        let size = constraints.constrain(self.layout.size);
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

    fn reconcile(&mut self, next: &mut dyn Widget) {
        if let Some(next) = next.as_any_mut().downcast_mut::<Self>() {
            next.id = self.id;
        }
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        paint_text_layout(
            ctx,
            &self.layout,
            Point::new(self.bounds.x, self.bounds.y),
            self.color.unwrap_or(ctx.theme().text),
            self.resolved_size,
        );
    }

    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        ctx.push(SemanticNode {
            id: self.id,
            role: SemanticRole::Label,
            bounds: self.bounds,
            label: self.text.clone(),
            disabled: false,
            focused: false,
            checked: None,
            value: None,
            ime_cursor_area: None,
        });
    }
}

pub struct Button {
    id: WidgetId,
    label: String,
    bounds: Rect,
    interaction: Interaction,
    disabled: bool,
    selected: bool,
    variant: ButtonVariant,
    text_layout: TextLayout,
    on_click: Box<ButtonAction>,
    hover_animation: Tween<f64>,
}

type ButtonAction = dyn FnMut(WidgetId, &mut EventCtx);

impl Button {
    #[must_use]
    pub fn new(label: impl Into<String>, on_click: impl FnMut() + 'static) -> Self {
        let mut on_click = on_click;
        Self {
            id: WidgetId::next(),
            label: label.into(),
            bounds: Rect::default(),
            interaction: Interaction::default(),
            disabled: false,
            selected: false,
            variant: ButtonVariant::Primary,
            text_layout: TextLayout::default(),
            on_click: Box::new(move |_, _| on_click()),
            hover_animation: Tween::new(0.0, 0.0, Duration::ZERO).easing(Easing::EaseOut),
        }
    }

    #[must_use]
    pub fn action(
        label: impl Into<String>,
        on_click: impl FnMut(WidgetId, &mut EventCtx) + 'static,
    ) -> Self {
        Self {
            id: WidgetId::next(),
            label: label.into(),
            bounds: Rect::default(),
            interaction: Interaction::default(),
            disabled: false,
            selected: false,
            variant: ButtonVariant::Primary,
            text_layout: TextLayout::default(),
            on_click: Box::new(on_click),
            hover_animation: Tween::new(0.0, 0.0, Duration::ZERO).easing(Easing::EaseOut),
        }
    }

    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    #[must_use]
    pub const fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    #[must_use]
    pub const fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    fn activate(&mut self, ctx: &mut EventCtx) {
        if self.disabled {
            return;
        }
        (self.on_click)(self.id, ctx);
        ctx.set_handled();
        ctx.request_paint();
    }

    fn set_hovered(&mut self, hovered: bool, ctx: &mut EventCtx) {
        if self.interaction.hovered == hovered {
            return;
        }
        self.interaction.hovered = hovered;
        self.hover_animation
            .retarget(if hovered { 1.0 } else { 0.0 }, Duration::from_millis(120));
        ctx.request_animation_frame();
    }
}

impl Widget for Button {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let style = ctx.theme().button_style(self.variant);
        self.text_layout = ctx.layout_text(&self.label, style.text_size, None);
        let width = style.padding_x * 2.0 + self.text_layout.size.width;
        let size = constraints.constrain(Size::new(width.max(style.min_width), style.height));
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
                self.set_hovered(hovered, ctx);
            }
            Event::PointerLeft => {
                self.set_hovered(false, ctx);
            }
            Event::PointerDown {
                position, button, ..
            } if *button == crate::PointerButton::Primary && self.bounds.contains(*position) => {
                self.interaction.pressed = true;
                ctx.capture_pointer(self.id);
                ctx.request_focus(self.id);
                ctx.request_paint();
                ctx.set_handled();
            }
            Event::PointerUp { position, button } if *button == crate::PointerButton::Primary => {
                let activate = self.interaction.pressed && self.bounds.contains(*position);
                self.interaction.pressed = false;
                ctx.release_pointer();
                if activate {
                    self.activate(ctx);
                } else {
                    ctx.request_paint();
                }
            }
            Event::KeyDown(crate::Key::Enter | crate::Key::Space) if self.interaction.focused => {
                self.activate(ctx);
            }
            Event::AccessibilityAction { target, action } if *target == self.id => match action {
                crate::SemanticAction::Click => self.activate(ctx),
                crate::SemanticAction::Focus => {
                    ctx.request_focus(self.id);
                    ctx.set_handled();
                }
                crate::SemanticAction::Increment | crate::SemanticAction::Decrement => {}
            },
            Event::FocusChanged(focused) => {
                let next = *focused == Some(self.id);
                if next != self.interaction.focused {
                    self.interaction.focused = next;
                    ctx.request_paint();
                }
            }
            Event::AnimationFrame(frame) => {
                let active = self
                    .hover_animation
                    .advance(frame.delta, frame.reduced_motion);
                ctx.request_paint();
                if active {
                    ctx.request_animation_frame();
                }
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
            next.hover_animation.clone_from(&self.hover_animation);
        }
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let style = ctx.theme().button_style(self.variant);
        let resting = style.resolve(if self.selected {
            ControlState::Selected
        } else {
            ControlState::Normal
        });
        let hovered = style.resolve(ControlState::Hovered);
        let progress = *self.hover_animation.value();
        let state = if self.disabled {
            style.resolve(ControlState::Disabled)
        } else if self.interaction.pressed {
            style.resolve(ControlState::Pressed)
        } else {
            crate::ControlStateStyle {
                background: resting
                    .background
                    .interpolate(&hovered.background, progress),
                foreground: resting
                    .foreground
                    .interpolate(&hovered.foreground, progress),
                border: resting.border.interpolate(&hovered.border, progress),
            }
        };
        ctx.push(PaintOp::FillRect {
            rect: self.bounds,
            color: state.background,
            radius: style.radius,
        });
        if style.border_width > 0.0 && state.border.a > 0 {
            ctx.push(PaintOp::StrokeRect {
                rect: self.bounds,
                color: state.border,
                width: style.border_width,
                radius: style.radius,
            });
        }
        if self.interaction.focused {
            ctx.push(PaintOp::StrokeRect {
                rect: self.bounds,
                color: ctx.theme().focus,
                width: 2.0,
                radius: style.radius,
            });
        }
        let text_y = self.bounds.y + (self.bounds.height - self.text_layout.size.height) * 0.5;
        paint_text_layout(
            ctx,
            &self.text_layout,
            Point::new(self.bounds.x + style.padding_x, text_y),
            state.foreground,
            style.text_size,
        );
    }

    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        ctx.push(SemanticNode {
            id: self.id,
            role: SemanticRole::Button,
            bounds: self.bounds,
            label: self.label.clone(),
            disabled: self.disabled,
            focused: self.interaction.focused,
            checked: None,
            value: None,
            ime_cursor_area: None,
        });
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EditSnapshot {
    text: String,
    cursor: usize,
    anchor: usize,
}

#[derive(Clone, Copy, Debug)]
enum ScrollDrag {
    Horizontal { grab: f64 },
    Vertical { grab: f64 },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum TextFieldMode {
    #[default]
    Plain,
    Password,
}

pub struct TextField {
    id: WidgetId,
    text: String,
    placeholder: String,
    cursor: usize,
    anchor: usize,
    preedit: String,
    preedit_cursor: Option<(usize, usize)>,
    modifiers: crate::Modifiers,
    scroll_x: f64,
    scroll_y: f64,
    multiline: bool,
    mode: TextFieldMode,
    min_lines: u32,
    bounds: Rect,
    interaction: Interaction,
    text_layout: TextLayout,
    showing_placeholder: bool,
    undo: Vec<EditSnapshot>,
    redo: Vec<EditSnapshot>,
    typing_active: bool,
    scroll_drag: Option<ScrollDrag>,
    on_change: Box<dyn FnMut(&str)>,
}

impl TextField {
    #[must_use]
    pub fn new(value: impl Into<String>, on_change: impl FnMut(&str) + 'static) -> Self {
        let text = value.into();
        let cursor = text.len();
        Self {
            id: WidgetId::next(),
            text,
            placeholder: String::new(),
            cursor,
            anchor: cursor,
            preedit: String::new(),
            preedit_cursor: None,
            modifiers: crate::Modifiers::default(),
            scroll_x: 0.0,
            scroll_y: 0.0,
            multiline: false,
            mode: TextFieldMode::Plain,
            min_lines: 1,
            bounds: Rect::default(),
            interaction: Interaction::default(),
            text_layout: TextLayout::default(),
            showing_placeholder: false,
            undo: Vec::new(),
            redo: Vec::new(),
            typing_active: false,
            scroll_drag: None,
            on_change: Box::new(on_change),
        }
    }

    #[must_use]
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    #[must_use]
    pub fn value(&self) -> &str {
        &self.text
    }

    pub(crate) fn replace_value(&mut self, value: impl Into<String>, ctx: &mut EventCtx) {
        let value = value.into();
        if self.text == value {
            return;
        }
        self.record_edit(false);
        self.text = value;
        self.cursor = self.text.len();
        self.anchor = self.cursor;
        self.clear_preedit();
        self.change(ctx);
    }

    /// Masks the rendered value and keeps it out of the semantic value tree.
    #[must_use]
    pub const fn password(mut self, password: bool) -> Self {
        self.mode = if password {
            TextFieldMode::Password
        } else {
            TextFieldMode::Plain
        };
        self
    }

    fn into_multiline(mut self, min_lines: u32) -> Self {
        self.multiline = true;
        self.min_lines = min_lines.max(2);
        self
    }

    fn display_text(&self) -> String {
        let display = if self.preedit.is_empty() {
            self.text.clone()
        } else {
            let mut display = String::with_capacity(self.text.len() + self.preedit.len());
            display.push_str(&self.text[..self.cursor]);
            display.push_str(&self.preedit);
            display.push_str(&self.text[self.cursor..]);
            display
        };
        if self.mode == TextFieldMode::Password {
            // One ASCII mask byte per source byte keeps the existing byte-indexed
            // caret and selection geometry valid for all UTF-8 input.
            "*".repeat(display.len())
        } else {
            display
        }
    }

    fn clear_preedit(&mut self) {
        self.preedit.clear();
        self.preedit_cursor = None;
    }

    fn display_caret_index(&self) -> usize {
        self.cursor
            + self
                .preedit_cursor
                .map_or(self.preedit.len(), |(_, end)| end.min(self.preedit.len()))
    }

    fn change(&mut self, ctx: &mut EventCtx) {
        (self.on_change)(&self.text);
        ctx.set_handled();
        ctx.request_layout();
    }

    fn snapshot(&self) -> EditSnapshot {
        EditSnapshot {
            text: self.text.clone(),
            cursor: self.cursor,
            anchor: self.anchor,
        }
    }

    fn record_edit(&mut self, coalesce_typing: bool) {
        const HISTORY_LIMIT: usize = 100;
        if coalesce_typing && self.typing_active {
            return;
        }
        let snapshot = self.snapshot();
        if self.undo.last() != Some(&snapshot) {
            self.undo.push(snapshot);
            if self.undo.len() > HISTORY_LIMIT {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.typing_active = coalesce_typing;
    }

    fn finish_typing(&mut self) {
        self.typing_active = false;
    }

    fn restore(&mut self, snapshot: EditSnapshot, ctx: &mut EventCtx) {
        self.text = snapshot.text;
        self.cursor = snapshot.cursor;
        self.anchor = snapshot.anchor;
        self.clear_preedit();
        self.change(ctx);
    }

    fn undo(&mut self, ctx: &mut EventCtx) {
        self.finish_typing();
        if let Some(snapshot) = self.undo.pop() {
            self.redo.push(self.snapshot());
            self.restore(snapshot, ctx);
        } else {
            ctx.set_handled();
        }
    }

    fn redo(&mut self, ctx: &mut EventCtx) {
        self.finish_typing();
        if let Some(snapshot) = self.redo.pop() {
            self.undo.push(self.snapshot());
            self.restore(snapshot, ctx);
        } else {
            ctx.set_handled();
        }
    }

    fn selection(&self) -> Option<std::ops::Range<usize>> {
        (self.anchor != self.cursor).then(|| {
            let start = self.anchor.min(self.cursor);
            let end = self.anchor.max(self.cursor);
            start..end
        })
    }

    fn delete_selection(&mut self) -> bool {
        let Some(selection) = self.selection() else {
            return false;
        };
        self.text.drain(selection.clone());
        self.cursor = selection.start;
        self.anchor = self.cursor;
        true
    }

    fn replace_selection(&mut self, input: &str) {
        self.delete_selection();
        self.text.insert_str(self.cursor, input);
        self.cursor += input.len();
        self.anchor = self.cursor;
    }

    fn move_cursor(&mut self, cursor: usize, extend: bool, ctx: &mut EventCtx) {
        let changed =
            cursor != self.cursor || !self.preedit.is_empty() || (!extend && self.anchor != cursor);
        if !extend {
            self.anchor = cursor;
        }
        if changed {
            self.cursor = cursor;
            self.clear_preedit();
            ctx.set_handled();
            ctx.request_layout();
        }
    }

    fn caret_geometry(&self, index: usize) -> crate::CaretPosition {
        self.text_layout
            .carets
            .iter()
            .find(|caret| caret.index == index)
            .copied()
            .unwrap_or_default()
    }

    fn vertical_cursor(&self, down: bool) -> usize {
        let Some(current) = self
            .text_layout
            .carets
            .iter()
            .find(|caret| caret.index == self.cursor)
        else {
            return self.logical_vertical_cursor(down);
        };
        let target_y = self
            .text_layout
            .carets
            .iter()
            .filter(|caret| {
                if down {
                    caret.y > current.y + 0.01
                } else {
                    caret.y < current.y - 0.01
                }
            })
            .map(|caret| caret.y)
            .min_by(|left, right| {
                (left - current.y)
                    .abs()
                    .total_cmp(&(right - current.y).abs())
            });
        let Some(target_y) = target_y else {
            return self.cursor;
        };
        self.text_layout
            .carets
            .iter()
            .filter(|caret| (caret.y - target_y).abs() < 0.01)
            .min_by(|left, right| {
                (left.x - current.x)
                    .abs()
                    .total_cmp(&(right.x - current.x).abs())
            })
            .map_or(self.cursor, |caret| caret.index.min(self.text.len()))
    }

    fn logical_vertical_cursor(&self, down: bool) -> usize {
        let line_start = self.text[..self.cursor]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        let column = self.text[line_start..self.cursor].chars().count();
        let target = if down {
            let Some(line_break) = self.text[self.cursor..].find('\n') else {
                return self.cursor;
            };
            let start = self.cursor + line_break + 1;
            let end = self.text[start..]
                .find('\n')
                .map_or(self.text.len(), |index| start + index);
            start..end
        } else {
            if line_start == 0 {
                return self.cursor;
            }
            let end = line_start - 1;
            let start = self.text[..end].rfind('\n').map_or(0, |index| index + 1);
            start..end
        };
        self.text[target.clone()]
            .char_indices()
            .nth(column)
            .map_or(target.end, |(index, _)| target.start + index)
    }

    fn page_cursor(&self, down: bool) -> usize {
        let Some(current) = self
            .text_layout
            .carets
            .iter()
            .find(|caret| caret.index == self.cursor)
        else {
            return if down { self.text.len() } else { 0 };
        };
        let page = (self.bounds.height - 20.0).max(current.height);
        let target_y = if down {
            current.y + page
        } else {
            (current.y - page).max(0.0)
        };
        self.text_layout
            .carets
            .iter()
            .min_by(|left, right| {
                let left_distance = (left.y - target_y).abs() * 4.0 + (left.x - current.x).abs();
                let right_distance = (right.y - target_y).abs() * 4.0 + (right.x - current.x).abs();
                left_distance.total_cmp(&right_distance)
            })
            .map_or(self.cursor, |caret| caret.index.min(self.text.len()))
    }

    fn visual_line_edge(&self, start: bool) -> usize {
        let Some(current) = self
            .text_layout
            .carets
            .iter()
            .find(|caret| caret.index == self.cursor)
        else {
            return if start {
                self.text[..self.cursor]
                    .rfind('\n')
                    .map_or(0, |index| index + 1)
            } else {
                self.text[self.cursor..]
                    .find('\n')
                    .map_or(self.text.len(), |index| self.cursor + index)
            };
        };
        let line = self
            .text_layout
            .carets
            .iter()
            .filter(|caret| (caret.y - current.y).abs() < 0.01);
        if start {
            line.min_by(|left, right| left.x.total_cmp(&right.x))
                .map_or(self.cursor, |caret| caret.index.min(self.text.len()))
        } else {
            line.max_by(|left, right| left.x.total_cmp(&right.x))
                .map_or(self.cursor, |caret| caret.index.min(self.text.len()))
        }
    }

    fn hit_test(&self, position: Point) -> usize {
        if self.text.is_empty() {
            return 0;
        }
        let target_x = position.x - self.bounds.x - 12.0 + self.scroll_x;
        let target_y = position.y - self.bounds.y - 10.0 + self.scroll_y;
        self.text_layout
            .carets
            .iter()
            .filter(|caret| caret.index <= self.text.len())
            .min_by(|left, right| {
                let left_distance = (left.y - target_y).abs() * 4.0 + (left.x - target_x).abs();
                let right_distance = (right.y - target_y).abs() * 4.0 + (right.x - target_x).abs();
                left_distance.total_cmp(&right_distance)
            })
            .map_or(self.text.len(), |caret| caret.index)
    }

    fn select_word_at(&mut self, index: usize) {
        if let Some((start, word)) = self
            .text
            .unicode_word_indices()
            .find(|(start, word)| index >= *start && index <= *start + word.len())
        {
            self.anchor = start;
            self.cursor = start + word.len();
        } else {
            self.anchor = index;
            self.cursor = index;
        }
    }

    fn select_line_at(&mut self, index: usize) {
        let start = self.text[..index]
            .rfind('\n')
            .map_or(0, |offset| offset + 1);
        let end = self.text[index..]
            .find('\n')
            .map_or(self.text.len(), |offset| index + offset + 1);
        self.anchor = start;
        self.cursor = end;
    }

    fn scroll(&mut self, delta_x: f64, delta_y: f64, ctx: &mut EventCtx) {
        let (max_x, max_y) = self.scroll_extents();
        let next_x = (self.scroll_x + delta_x).clamp(0.0, max_x);
        let next_y = (self.scroll_y + delta_y).clamp(0.0, max_y);
        if (next_x - self.scroll_x).abs() > f64::EPSILON
            || (next_y - self.scroll_y).abs() > f64::EPSILON
        {
            self.scroll_x = next_x;
            self.scroll_y = next_y;
            ctx.request_paint();
            ctx.set_handled();
        }
    }

    fn scroll_extents(&self) -> (f64, f64) {
        (
            (self.text_layout.size.width - (self.bounds.width - 24.0)).max(0.0),
            (self.text_layout.size.height - (self.bounds.height - 20.0)).max(0.0),
        )
    }

    fn vertical_scroll_track(&self) -> Option<Rect> {
        self.vertical_scrollbar().map(|_| {
            Rect::new(
                self.bounds.x + self.bounds.width - 8.0,
                self.bounds.y + 6.0,
                7.0,
                (self.bounds.height - 12.0).max(0.0),
            )
        })
    }

    fn horizontal_scroll_track(&self) -> Option<Rect> {
        self.horizontal_scrollbar().map(|_| {
            Rect::new(
                self.bounds.x + 8.0,
                self.bounds.y + self.bounds.height - 8.0,
                (self.bounds.width - 16.0).max(0.0),
                7.0,
            )
        })
    }

    fn vertical_scrollbar(&self) -> Option<Rect> {
        let (_, max_y) = self.scroll_extents();
        if max_y <= f64::EPSILON {
            return None;
        }
        let inner_height = (self.bounds.height - 20.0).max(0.0);
        let track_height = (self.bounds.height - 12.0).max(0.0);
        let thumb_height = (inner_height / self.text_layout.size.height * track_height)
            .clamp(20.0_f64.min(track_height), track_height);
        let travel = track_height - thumb_height;
        let thumb_y = self.bounds.y + 6.0 + self.scroll_y / max_y * travel;
        Some(Rect::new(
            self.bounds.x + self.bounds.width - 8.0,
            thumb_y,
            7.0,
            thumb_height,
        ))
    }

    fn horizontal_scrollbar(&self) -> Option<Rect> {
        let (max_x, _) = self.scroll_extents();
        if max_x <= f64::EPSILON {
            return None;
        }
        let inner_width = (self.bounds.width - 24.0).max(0.0);
        let track_width = (self.bounds.width - 16.0).max(0.0);
        let thumb_width = (inner_width / self.text_layout.size.width * track_width)
            .clamp(20.0_f64.min(track_width), track_width);
        let travel = track_width - thumb_width;
        let thumb_x = self.bounds.x + 8.0 + self.scroll_x / max_x * travel;
        Some(Rect::new(
            thumb_x,
            self.bounds.y + self.bounds.height - 8.0,
            thumb_width,
            7.0,
        ))
    }

    fn drag_scrollbar(&mut self, position: Point, drag: ScrollDrag, ctx: &mut EventCtx) {
        let (max_x, max_y) = self.scroll_extents();
        match drag {
            ScrollDrag::Vertical { grab } => {
                let Some(thumb) = self.vertical_scrollbar() else {
                    return;
                };
                let track_start = self.bounds.y + 6.0;
                let travel = (self.bounds.height - 12.0 - thumb.height).max(0.0);
                if travel > f64::EPSILON {
                    self.scroll_y =
                        ((position.y - grab - track_start) / travel * max_y).clamp(0.0, max_y);
                }
            }
            ScrollDrag::Horizontal { grab } => {
                let Some(thumb) = self.horizontal_scrollbar() else {
                    return;
                };
                let track_start = self.bounds.x + 8.0;
                let travel = (self.bounds.width - 16.0 - thumb.width).max(0.0);
                if travel > f64::EPSILON {
                    self.scroll_x =
                        ((position.x - grab - track_start) / travel * max_x).clamp(0.0, max_x);
                }
            }
        }
        ctx.set_handled();
        ctx.request_paint();
    }

    fn auto_scroll_selection(&mut self, position: Point, ctx: &mut EventCtx) {
        let left = self.bounds.x + 18.0;
        let right = self.bounds.x + self.bounds.width - 18.0;
        let top = self.bounds.y + 18.0;
        let bottom = self.bounds.y + self.bounds.height - 18.0;
        let delta_x = if position.x < left {
            (position.x - left).max(-32.0)
        } else if position.x > right {
            (position.x - right).min(32.0)
        } else {
            0.0
        };
        let delta_y = if self.multiline && position.y < top {
            (position.y - top).max(-32.0)
        } else if self.multiline && position.y > bottom {
            (position.y - bottom).min(32.0)
        } else {
            0.0
        };
        if delta_x.abs() > f64::EPSILON || delta_y.abs() > f64::EPSILON {
            self.scroll(delta_x, delta_y, ctx);
        }
    }

    fn paint_scrollbars(&self, ctx: &mut PaintCtx<'_>) {
        let color = if self.interaction.hovered || self.interaction.focused {
            ctx.theme().button_hover
        } else {
            ctx.theme().button
        };
        if let Some(thumb) = self.vertical_scrollbar() {
            ctx.push(PaintOp::FillRect {
                rect: Rect::new(thumb.x + 2.0, thumb.y, 3.0, thumb.height),
                color,
                radius: 1.5,
            });
        }
        if let Some(thumb) = self.horizontal_scrollbar() {
            ctx.push(PaintOp::FillRect {
                rect: Rect::new(thumb.x, thumb.y + 2.0, thumb.width, 3.0),
                color,
                radius: 1.5,
            });
        }
    }

    fn paint_preedit(&self, ctx: &mut PaintCtx<'_>, text_y: f64) {
        if self.preedit.is_empty() {
            return;
        }
        let composition = self.cursor..self.cursor + self.preedit.len();
        let selected = self
            .preedit_cursor
            .map(|(start, end)| self.cursor + start.min(end)..self.cursor + start.max(end));
        for pair in self.text_layout.carets.windows(2) {
            let start = pair[0];
            let end = pair[1];
            if start.index < composition.start
                || start.index >= composition.end
                || (start.y - end.y).abs() >= 0.01
            {
                continue;
            }
            let emphasized = selected
                .as_ref()
                .is_some_and(|range| start.index >= range.start && start.index < range.end);
            let thickness = if emphasized { 3.0 } else { 1.5 };
            ctx.push(PaintOp::FillRect {
                rect: Rect::new(
                    self.bounds.x + 12.0 + start.x.min(end.x) - self.scroll_x,
                    text_y + start.y + start.height - thickness,
                    (end.x - start.x).abs().max(2.0),
                    thickness,
                ),
                color: if emphasized {
                    ctx.theme().text
                } else {
                    ctx.theme().focus
                },
                radius: thickness * 0.5,
            });
        }
    }
}

impl Widget for TextField {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        const FONT_SIZE: f32 = 15.0;
        const HORIZONTAL_PADDING: f64 = 12.0;
        let display = self.display_text();
        self.showing_placeholder = display.is_empty();
        let visible = if self.showing_placeholder {
            &self.placeholder
        } else {
            &display
        };
        let wrap_width = self.multiline.then(|| {
            let available = (constraints.max.width.min(360.0) - HORIZONTAL_PADDING * 2.0).max(1.0);
            width_as_f32(available)
        });
        self.text_layout = ctx.layout_text(visible, FONT_SIZE, wrap_width);

        let width = (self.text_layout.size.width + HORIZONTAL_PADDING * 2.0).max(240.0);
        let desired_height = if self.multiline {
            self.text_layout
                .size
                .height
                .max(f64::from(self.min_lines) * f64::from(FONT_SIZE) * 1.35)
                + 20.0
        } else {
            42.0
        };
        let desired_width = if self.multiline {
            width.min(360.0)
        } else {
            width
        };
        let size = constraints.constrain(Size::new(desired_width, desired_height));
        self.bounds.width = size.width;
        self.bounds.height = size.height;
        let caret_index = self.display_caret_index();
        let caret = if self.showing_placeholder {
            crate::CaretPosition::default()
        } else {
            self.caret_geometry(caret_index)
        };
        let inner_width = (size.width - HORIZONTAL_PADDING * 2.0).max(0.0);
        if caret.x < self.scroll_x {
            self.scroll_x = caret.x;
        } else if caret.x > self.scroll_x + inner_width {
            self.scroll_x = caret.x - inner_width;
        }
        self.scroll_x = self
            .scroll_x
            .min((self.text_layout.size.width - inner_width).max(0.0));
        let inner_height = (size.height - 20.0).max(0.0);
        if caret.y < self.scroll_y {
            self.scroll_y = caret.y;
        } else if caret.y + caret.height > self.scroll_y + inner_height {
            self.scroll_y = caret.y + caret.height - inner_height;
        }
        self.scroll_y = self
            .scroll_y
            .min((self.text_layout.size.height - inner_height).max(0.0));
        size
    }

    fn set_origin(&mut self, origin: Point) {
        self.bounds.x = origin.x;
        self.bounds.y = origin.y;
    }

    fn bounds(&self) -> Rect {
        self.bounds
    }

    #[allow(clippy::too_many_lines)]
    fn event(&mut self, ctx: &mut EventCtx, event: &Event) {
        match event {
            Event::PointerMoved(position) => {
                let hovered = self.bounds.contains(*position);
                if hovered != self.interaction.hovered {
                    self.interaction.hovered = hovered;
                    ctx.request_paint();
                }
                if let Some(drag) = self.scroll_drag {
                    self.drag_scrollbar(*position, drag, ctx);
                } else if self.interaction.pressed {
                    self.auto_scroll_selection(*position, ctx);
                    self.cursor = self.hit_test(*position);
                    self.clear_preedit();
                    ctx.set_handled();
                    ctx.request_layout();
                }
            }
            Event::PointerLeft => {
                if self.interaction.hovered {
                    self.interaction.hovered = false;
                    ctx.request_paint();
                }
            }
            Event::PointerDown {
                position,
                button,
                click_count,
            } if *button == crate::PointerButton::Primary && self.bounds.contains(*position) => {
                self.finish_typing();
                if let Some(thumb) = self.vertical_scrollbar()
                    && thumb.contains(*position)
                {
                    self.scroll_drag = Some(ScrollDrag::Vertical {
                        grab: position.y - thumb.y,
                    });
                    ctx.capture_pointer(self.id);
                    ctx.set_handled();
                    ctx.request_paint();
                    return;
                }
                if let Some(thumb) = self.horizontal_scrollbar()
                    && thumb.contains(*position)
                {
                    self.scroll_drag = Some(ScrollDrag::Horizontal {
                        grab: position.x - thumb.x,
                    });
                    ctx.capture_pointer(self.id);
                    ctx.set_handled();
                    ctx.request_paint();
                    return;
                }
                if let Some(track) = self.vertical_scroll_track()
                    && track.contains(*position)
                {
                    let thumb = self.vertical_scrollbar().expect("track requires thumb");
                    let page = (self.bounds.height - 20.0).max(0.0);
                    let delta = if position.y < thumb.y { -page } else { page };
                    self.scroll(0.0, delta, ctx);
                    return;
                }
                if let Some(track) = self.horizontal_scroll_track()
                    && track.contains(*position)
                {
                    let thumb = self.horizontal_scrollbar().expect("track requires thumb");
                    let page = (self.bounds.width - 24.0).max(0.0);
                    let delta = if position.x < thumb.x { -page } else { page };
                    self.scroll(delta, 0.0, ctx);
                    return;
                }
                let cursor = self.hit_test(*position);
                if *click_count >= 3 && self.multiline {
                    self.select_line_at(cursor);
                } else if *click_count >= 2 {
                    self.select_word_at(cursor);
                } else {
                    self.cursor = cursor;
                    self.anchor = cursor;
                }
                self.clear_preedit();
                self.interaction.pressed = *click_count == 1;
                if self.interaction.pressed {
                    ctx.capture_pointer(self.id);
                }
                ctx.request_focus(self.id);
                ctx.request_layout();
                ctx.set_handled();
            }
            Event::PointerUp { button, .. } if *button == crate::PointerButton::Primary => {
                if self.scroll_drag.take().is_some() {
                    ctx.release_pointer();
                    ctx.set_handled();
                    ctx.request_paint();
                    return;
                }
                if self.interaction.pressed {
                    self.interaction.pressed = false;
                    ctx.release_pointer();
                    ctx.set_handled();
                    ctx.request_paint();
                }
            }
            Event::Scroll {
                position,
                delta_x,
                delta_y,
            } if self.bounds.contains(*position) => {
                self.scroll(*delta_x, *delta_y, ctx);
            }
            Event::TextInput(input) if self.interaction.focused && !input.is_empty() => {
                self.clear_preedit();
                self.record_edit(true);
                self.replace_selection(input);
                self.change(ctx);
            }
            Event::ImePreedit { text, cursor } if self.interaction.focused => {
                if self.preedit != *text || self.preedit_cursor != *cursor {
                    self.preedit.clone_from(text);
                    self.preedit_cursor =
                        cursor.map(|(start, end)| (start.min(text.len()), end.min(text.len())));
                    ctx.set_handled();
                    ctx.request_layout();
                }
            }
            Event::KeyDown(crate::Key::Backspace) if self.interaction.focused => {
                self.clear_preedit();
                if self.selection().is_some() {
                    self.record_edit(false);
                    self.delete_selection();
                    self.change(ctx);
                    return;
                }
                let previous = if self.modifiers.command() {
                    previous_word_boundary(&self.text, self.cursor)
                } else {
                    previous_char_boundary(&self.text, self.cursor)
                };
                if previous == self.cursor {
                    ctx.set_handled();
                } else {
                    self.record_edit(false);
                    self.text.drain(previous..self.cursor);
                    self.cursor = previous;
                    self.anchor = previous;
                    self.change(ctx);
                }
            }
            Event::KeyDown(crate::Key::Delete) if self.interaction.focused => {
                self.clear_preedit();
                if self.selection().is_some() {
                    self.record_edit(false);
                    self.delete_selection();
                    self.change(ctx);
                    return;
                }
                let next = if self.modifiers.command() {
                    next_word_boundary(&self.text, self.cursor)
                } else {
                    next_char_boundary(&self.text, self.cursor)
                };
                if next == self.cursor {
                    ctx.set_handled();
                } else {
                    self.record_edit(false);
                    self.text.drain(self.cursor..next);
                    self.change(ctx);
                }
            }
            Event::KeyDown(crate::Key::ArrowLeft) if self.interaction.focused => {
                self.finish_typing();
                let cursor = if self.modifiers.shift() {
                    if self.modifiers.command() {
                        previous_word_boundary(&self.text, self.cursor)
                    } else {
                        previous_char_boundary(&self.text, self.cursor)
                    }
                } else {
                    self.selection().map_or_else(
                        || {
                            if self.modifiers.command() {
                                previous_word_boundary(&self.text, self.cursor)
                            } else {
                                previous_char_boundary(&self.text, self.cursor)
                            }
                        },
                        |selection| selection.start,
                    )
                };
                self.move_cursor(cursor, self.modifiers.shift(), ctx);
                ctx.set_handled();
            }
            Event::KeyDown(crate::Key::ArrowRight) if self.interaction.focused => {
                self.finish_typing();
                let cursor = if self.modifiers.shift() {
                    if self.modifiers.command() {
                        next_word_boundary(&self.text, self.cursor)
                    } else {
                        next_char_boundary(&self.text, self.cursor)
                    }
                } else {
                    self.selection().map_or_else(
                        || {
                            if self.modifiers.command() {
                                next_word_boundary(&self.text, self.cursor)
                            } else {
                                next_char_boundary(&self.text, self.cursor)
                            }
                        },
                        |selection| selection.end,
                    )
                };
                self.move_cursor(cursor, self.modifiers.shift(), ctx);
                ctx.set_handled();
            }
            Event::KeyDown(crate::Key::Home) if self.interaction.focused => {
                self.finish_typing();
                let cursor = if self.multiline && !self.modifiers.command() {
                    self.visual_line_edge(true)
                } else {
                    0
                };
                self.move_cursor(cursor, self.modifiers.shift(), ctx);
                ctx.set_handled();
            }
            Event::KeyDown(crate::Key::End) if self.interaction.focused => {
                self.finish_typing();
                let cursor = if self.multiline && !self.modifiers.command() {
                    self.visual_line_edge(false)
                } else {
                    self.text.len()
                };
                self.move_cursor(cursor, self.modifiers.shift(), ctx);
                ctx.set_handled();
            }
            Event::KeyDown(crate::Key::ArrowUp) if self.interaction.focused && self.multiline => {
                self.finish_typing();
                self.move_cursor(self.vertical_cursor(false), self.modifiers.shift(), ctx);
                ctx.set_handled();
            }
            Event::KeyDown(crate::Key::ArrowDown) if self.interaction.focused && self.multiline => {
                self.finish_typing();
                self.move_cursor(self.vertical_cursor(true), self.modifiers.shift(), ctx);
                ctx.set_handled();
            }
            Event::KeyDown(crate::Key::PageUp) if self.interaction.focused && self.multiline => {
                self.finish_typing();
                self.move_cursor(self.page_cursor(false), self.modifiers.shift(), ctx);
                ctx.set_handled();
            }
            Event::KeyDown(crate::Key::PageDown) if self.interaction.focused && self.multiline => {
                self.finish_typing();
                self.move_cursor(self.page_cursor(true), self.modifiers.shift(), ctx);
                ctx.set_handled();
            }
            Event::KeyDown(crate::Key::Enter) if self.interaction.focused && self.multiline => {
                self.clear_preedit();
                self.record_edit(false);
                self.replace_selection("\n");
                self.change(ctx);
            }
            Event::KeyDown(crate::Key::Character(character))
                if self.interaction.focused && self.modifiers.command() =>
            {
                self.finish_typing();
                match character.to_lowercase().as_str() {
                    "a" => {
                        self.anchor = 0;
                        self.cursor = self.text.len();
                        ctx.set_handled();
                        ctx.request_layout();
                    }
                    "c" => {
                        if self.mode != TextFieldMode::Password
                            && let Some(selection) = self.selection()
                        {
                            ctx.request_clipboard(crate::ClipboardRequest::Write(
                                self.text[selection].to_owned(),
                            ));
                        }
                        ctx.set_handled();
                    }
                    "x" => {
                        if let Some(selection) = self.selection() {
                            if self.mode != TextFieldMode::Password {
                                ctx.request_clipboard(crate::ClipboardRequest::Write(
                                    self.text[selection.clone()].to_owned(),
                                ));
                            }
                            self.record_edit(false);
                            self.delete_selection();
                            self.change(ctx);
                        } else {
                            ctx.set_handled();
                        }
                    }
                    "v" => {
                        ctx.request_clipboard(crate::ClipboardRequest::Read);
                        ctx.set_handled();
                    }
                    "z" if self.modifiers.shift() => self.redo(ctx),
                    "z" => self.undo(ctx),
                    "y" => self.redo(ctx),
                    _ => {}
                }
            }
            Event::ModifiersChanged(modifiers) => self.modifiers = *modifiers,
            Event::AccessibilityAction { target, action } if *target == self.id => {
                if *action == crate::SemanticAction::Focus {
                    ctx.request_focus(self.id);
                    ctx.set_handled();
                }
            }
            Event::FocusChanged(focused) => {
                let next = *focused == Some(self.id);
                if next != self.interaction.focused {
                    self.interaction.focused = next;
                    if !next {
                        self.finish_typing();
                        self.clear_preedit();
                        self.interaction.pressed = false;
                        self.scroll_drag = None;
                    }
                    ctx.request_layout();
                }
            }
            _ => {}
        }
    }

    fn widget_id(&self) -> Option<WidgetId> {
        Some(self.id)
    }

    fn reconcile(&mut self, next: &mut dyn Widget) {
        let Some(next) = next.as_any_mut().downcast_mut::<Self>() else {
            return;
        };
        next.id = self.id;
        next.interaction = self.interaction;
        next.modifiers = self.modifiers;
        next.scroll_x = self.scroll_x;
        next.scroll_y = self.scroll_y;
        if next.text == self.text {
            next.cursor = self.cursor;
            next.anchor = self.anchor;
            next.preedit.clone_from(&self.preedit);
            next.preedit_cursor = self.preedit_cursor;
            next.undo.clone_from(&self.undo);
            next.redo.clone_from(&self.redo);
            next.typing_active = self.typing_active;
            next.scroll_drag = self.scroll_drag;
        } else {
            next.cursor = next.text.len();
            next.anchor = next.cursor;
        }
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.push(PaintOp::FillRect {
            rect: self.bounds,
            color: ctx.theme().button_disabled,
            radius: 8.0,
        });
        ctx.push(PaintOp::StrokeRect {
            rect: self.bounds,
            color: if self.interaction.focused {
                ctx.theme().focus
            } else {
                ctx.theme().button
            },
            width: if self.interaction.focused { 2.0 } else { 1.0 },
            radius: 8.0,
        });
        let content = Rect::new(
            self.bounds.x + 4.0,
            self.bounds.y + 4.0,
            (self.bounds.width - 8.0).max(0.0),
            (self.bounds.height - 8.0).max(0.0),
        );
        ctx.push(PaintOp::PushClip {
            rect: content,
            radius: 4.0,
        });
        let text_y = if self.multiline {
            self.bounds.y + 10.0 - self.scroll_y
        } else {
            self.bounds.y + (self.bounds.height - self.text_layout.size.height) * 0.5
        };
        if let Some(selection) = self.selection() {
            for pair in self.text_layout.carets.windows(2) {
                let start = pair[0];
                let end = pair[1];
                if start.index < selection.start
                    || start.index >= selection.end
                    || (start.y - end.y).abs() >= 0.01
                {
                    continue;
                }
                ctx.push(PaintOp::FillRect {
                    rect: Rect::new(
                        self.bounds.x + 12.0 + start.x.min(end.x) - self.scroll_x,
                        text_y + start.y,
                        (end.x - start.x).abs().max(2.0),
                        start.height,
                    ),
                    color: Color::rgb(49, 110, 158),
                    radius: 2.0,
                });
            }
        }
        paint_text_layout(
            ctx,
            &self.text_layout,
            Point::new(self.bounds.x + 12.0 - self.scroll_x, text_y),
            if self.showing_placeholder {
                Color::rgb(157, 166, 181)
            } else {
                ctx.theme().text
            },
            15.0,
        );
        self.paint_preedit(ctx, text_y);
        if self.interaction.focused && (self.preedit.is_empty() || self.preedit_cursor.is_some()) {
            let cursor_index = self.display_caret_index();
            let caret = self.caret_geometry(cursor_index);
            let cursor_x = self.bounds.x + 12.0 + caret.x - self.scroll_x;
            ctx.push(PaintOp::FillRect {
                rect: Rect::new(cursor_x, text_y + caret.y, 1.5, caret.height.max(20.0)),
                color: ctx.theme().text,
                radius: 0.75,
            });
        }
        ctx.push(PaintOp::PopClip);
        self.paint_scrollbars(ctx);
    }

    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        ctx.push(SemanticNode {
            id: self.id,
            role: if self.mode == TextFieldMode::Password {
                SemanticRole::PasswordInput
            } else if self.multiline {
                SemanticRole::MultilineTextInput
            } else {
                SemanticRole::TextInput
            },
            bounds: self.bounds,
            label: self.placeholder.clone(),
            disabled: false,
            focused: self.interaction.focused,
            checked: None,
            value: (self.mode != TextFieldMode::Password).then(|| self.text.clone()),
            ime_cursor_area: self.interaction.focused.then(|| {
                let caret = self.caret_geometry(self.display_caret_index());
                let text_y = if self.multiline {
                    self.bounds.y + 10.0 - self.scroll_y
                } else {
                    self.bounds.y + (self.bounds.height - self.text_layout.size.height) * 0.5
                };
                Rect::new(
                    self.bounds.x + 12.0 + caret.x - self.scroll_x,
                    text_y + caret.y,
                    1.5,
                    caret.height.max(20.0),
                )
            }),
        });
    }
}

pub struct TextArea {
    field: TextField,
}

impl TextArea {
    #[must_use]
    pub fn new(value: impl Into<String>, on_change: impl FnMut(&str) + 'static) -> Self {
        Self {
            field: TextField::new(value, on_change).into_multiline(4),
        }
    }

    #[must_use]
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.field = self.field.placeholder(placeholder);
        self
    }

    #[must_use]
    pub fn min_lines(mut self, lines: u32) -> Self {
        self.field.min_lines = lines.max(2);
        self
    }
}

impl Widget for TextArea {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.field.layout(ctx, constraints)
    }

    fn set_origin(&mut self, origin: Point) {
        self.field.set_origin(origin);
    }

    fn bounds(&self) -> Rect {
        self.field.bounds()
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &Event) {
        self.field.event(ctx, event);
    }

    fn event_target(&mut self, ctx: &mut EventCtx, event: &Event, target: WidgetId) -> bool {
        self.field.event_target(ctx, event, target)
    }

    fn apply_command(&mut self, command: UiCommand) -> Result<(), UiCommand> {
        self.field.apply_command(command)
    }

    fn reconcile(&mut self, next: &mut dyn Widget) {
        if let Some(next) = next.as_any_mut().downcast_mut::<Self>() {
            self.field.reconcile(&mut next.field);
        }
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        self.field.paint(ctx);
    }

    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        self.field.semantics(ctx);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MenuItemKind {
    Action,
    Checkbox,
    Radio,
    Separator,
}

type MenuAction = dyn FnMut(bool, &mut EventCtx);

pub struct MenuItem {
    id: WidgetId,
    label: String,
    kind: MenuItemKind,
    enabled: bool,
    checked: Option<bool>,
    bounds: Rect,
    layout: TextLayout,
    shortcut: Option<String>,
    shortcut_layout: TextLayout,
    on_activate: Box<MenuAction>,
}

impl MenuItem {
    #[must_use]
    pub fn new(label: impl Into<String>, on_activate: impl FnMut() + 'static) -> Self {
        let mut on_activate = on_activate;
        Self::action(label, move |_| on_activate())
    }

    #[must_use]
    pub fn action(
        label: impl Into<String>,
        on_activate: impl FnMut(&mut EventCtx) + 'static,
    ) -> Self {
        let mut on_activate = on_activate;
        Self::with_kind(label, MenuItemKind::Action, None, move |_, ctx| {
            on_activate(ctx);
        })
    }

    #[must_use]
    pub fn command(command: crate::CommandDescriptor) -> Self {
        let id = command.id.clone();
        let kind = if command.checked.is_some() {
            MenuItemKind::Checkbox
        } else if command.selected {
            MenuItemKind::Radio
        } else {
            MenuItemKind::Action
        };
        let checked = command.checked.or(command.selected.then_some(true));
        let shortcut = command.shortcut.as_ref().map(crate::KeyChord::label);
        Self::with_kind(command.label, kind, checked, move |_, ctx| {
            ctx.invoke_command(id.clone());
        })
        .disabled(!command.enabled)
        .shortcut_label(shortcut)
    }

    #[must_use]
    pub fn checkbox(
        label: impl Into<String>,
        checked: bool,
        on_change: impl FnMut(bool) + 'static,
    ) -> Self {
        let mut on_change = on_change;
        Self::with_kind(
            label,
            MenuItemKind::Checkbox,
            Some(checked),
            move |checked, _| on_change(checked),
        )
    }

    #[must_use]
    pub fn radio(
        label: impl Into<String>,
        selected: bool,
        on_select: impl FnMut() + 'static,
    ) -> Self {
        let mut on_select = on_select;
        Self::radio_action(label, selected, move |_| on_select())
    }

    #[must_use]
    pub fn radio_action(
        label: impl Into<String>,
        selected: bool,
        on_select: impl FnMut(&mut EventCtx) + 'static,
    ) -> Self {
        let mut on_select = on_select;
        Self::with_kind(
            label,
            MenuItemKind::Radio,
            Some(selected),
            move |selected, ctx| {
                if selected {
                    on_select(ctx);
                }
            },
        )
    }

    #[must_use]
    pub fn separator() -> Self {
        Self::with_kind("", MenuItemKind::Separator, None, |_, _| {})
    }

    #[must_use]
    pub const fn disabled(mut self, disabled: bool) -> Self {
        self.enabled = !disabled;
        self
    }

    #[must_use]
    pub fn shortcut_label(mut self, shortcut: Option<String>) -> Self {
        self.shortcut = shortcut;
        self
    }

    fn with_kind(
        label: impl Into<String>,
        kind: MenuItemKind,
        checked: Option<bool>,
        on_activate: impl FnMut(bool, &mut EventCtx) + 'static,
    ) -> Self {
        Self {
            id: WidgetId::next(),
            label: label.into(),
            kind,
            enabled: kind != MenuItemKind::Separator,
            checked,
            bounds: Rect::default(),
            layout: TextLayout::default(),
            shortcut: None,
            shortcut_layout: TextLayout::default(),
            on_activate: Box::new(on_activate),
        }
    }

    const fn height(&self) -> f64 {
        if matches!(self.kind, MenuItemKind::Separator) {
            9.0
        } else {
            34.0
        }
    }
}

pub struct Menu {
    id: WidgetId,
    items: Vec<MenuItem>,
    bounds: Rect,
    active: Option<usize>,
    pressed: Option<usize>,
    typeahead: String,
}

impl Menu {
    #[must_use]
    pub fn new(items: impl IntoIterator<Item = MenuItem>) -> Self {
        Self {
            id: WidgetId::next(),
            items: items.into_iter().collect(),
            bounds: Rect::default(),
            active: None,
            pressed: None,
            typeahead: String::new(),
        }
    }

    fn selectable(&self, index: usize) -> bool {
        self.items
            .get(index)
            .is_some_and(|item| item.enabled && item.kind != MenuItemKind::Separator)
    }

    fn item_at(&self, position: Point) -> Option<usize> {
        self.items
            .iter()
            .position(|item| item.bounds.contains(position) && item.kind != MenuItemKind::Separator)
    }

    fn move_active(&mut self, forward: bool, ctx: &mut EventCtx) {
        if self.items.is_empty() {
            return;
        }
        let start = self
            .active
            .unwrap_or(if forward { self.items.len() - 1 } else { 0 });
        for offset in 1..=self.items.len() {
            let index = if forward {
                (start + offset) % self.items.len()
            } else {
                (start + self.items.len() - offset % self.items.len()) % self.items.len()
            };
            if self.selectable(index) {
                self.set_active(index, ctx);
                return;
            }
        }
    }

    fn edge_active(&mut self, first: bool, ctx: &mut EventCtx) {
        let index = if first {
            (0..self.items.len()).find(|index| self.selectable(*index))
        } else {
            (0..self.items.len())
                .rev()
                .find(|index| self.selectable(*index))
        };
        if let Some(index) = index {
            self.set_active(index, ctx);
        }
    }

    fn set_active(&mut self, index: usize, ctx: &mut EventCtx) {
        if self.active != Some(index) && self.selectable(index) {
            self.active = Some(index);
            ctx.request_focus(self.items[index].id);
            ctx.request_paint();
        }
    }

    fn activate(&mut self, index: usize, ctx: &mut EventCtx) {
        if !self.selectable(index) {
            return;
        }
        let item = &mut self.items[index];
        let checked = match item.kind {
            MenuItemKind::Checkbox => {
                let checked = !item.checked.unwrap_or(false);
                item.checked = Some(checked);
                checked
            }
            MenuItemKind::Radio => {
                item.checked = Some(true);
                true
            }
            MenuItemKind::Action | MenuItemKind::Separator => false,
        };
        (item.on_activate)(checked, ctx);
        ctx.dismiss_overlay();
        ctx.set_handled();
        ctx.request_paint();
    }

    fn typeahead(&mut self, text: &str, ctx: &mut EventCtx) {
        if text.chars().any(char::is_control) {
            return;
        }
        self.typeahead.push_str(&text.to_lowercase());
        let mut found = self.items.iter().enumerate().find_map(|(index, item)| {
            (self.selectable(index) && item.label.to_lowercase().starts_with(&self.typeahead))
                .then_some(index)
        });
        if found.is_none() {
            self.typeahead = text.to_lowercase();
            found = self.items.iter().enumerate().find_map(|(index, item)| {
                (self.selectable(index) && item.label.to_lowercase().starts_with(&self.typeahead))
                    .then_some(index)
            });
        }
        if let Some(index) = found {
            self.set_active(index, ctx);
            ctx.set_handled();
        }
    }
}

impl Widget for Menu {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let mut width: f64 = 160.0;
        let mut height = 12.0;
        for item in &mut self.items {
            if item.kind != MenuItemKind::Separator {
                item.layout = ctx.layout_text(&item.label, 14.0, None);
                item.shortcut_layout = item
                    .shortcut
                    .as_ref()
                    .map_or_else(TextLayout::default, |shortcut| {
                        ctx.layout_text(shortcut, 13.0, None)
                    });
                width = width.max(item.layout.size.width + item.shortcut_layout.size.width + 80.0);
            }
            height += item.height();
        }
        let size = constraints.constrain(Size::new(width, height));
        self.bounds.width = size.width;
        self.bounds.height = size.height;
        self.set_origin(Point::new(self.bounds.x, self.bounds.y));
        size
    }

    fn set_origin(&mut self, origin: Point) {
        self.bounds.x = origin.x;
        self.bounds.y = origin.y;
        let mut y = origin.y + 6.0;
        for item in &mut self.items {
            item.bounds = Rect::new(
                origin.x + 6.0,
                y,
                (self.bounds.width - 12.0).max(0.0),
                item.height(),
            );
            y += item.height();
        }
    }

    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &Event) {
        match event {
            Event::PointerMoved(position) => {
                if let Some(index) = self.item_at(*position) {
                    self.set_active(index, ctx);
                }
            }
            Event::PointerLeft => {
                if self.active.take().is_some() {
                    ctx.request_paint();
                }
            }
            Event::PointerDown {
                position, button, ..
            } if *button == crate::PointerButton::Primary => {
                if let Some(index) = self.item_at(*position) {
                    self.pressed = Some(index);
                    self.set_active(index, ctx);
                    ctx.capture_pointer(self.id);
                    ctx.set_handled();
                }
            }
            Event::PointerUp { position, button } if *button == crate::PointerButton::Primary => {
                let released = self.item_at(*position);
                let pressed = self.pressed.take();
                ctx.release_pointer();
                if pressed.is_some() {
                    ctx.set_handled();
                }
                if pressed == released
                    && let Some(index) = released
                {
                    self.activate(index, ctx);
                }
            }
            Event::KeyDown(crate::Key::ArrowDown) => {
                self.typeahead.clear();
                self.move_active(true, ctx);
                ctx.set_handled();
            }
            Event::KeyDown(crate::Key::ArrowUp) => {
                self.typeahead.clear();
                self.move_active(false, ctx);
                ctx.set_handled();
            }
            Event::KeyDown(crate::Key::Home) => {
                self.typeahead.clear();
                self.edge_active(true, ctx);
                ctx.set_handled();
            }
            Event::KeyDown(crate::Key::End) => {
                self.typeahead.clear();
                self.edge_active(false, ctx);
                ctx.set_handled();
            }
            Event::KeyDown(crate::Key::Enter | crate::Key::Space) => {
                if let Some(index) = self.active {
                    self.activate(index, ctx);
                }
            }
            Event::KeyDown(crate::Key::Character(text)) => self.typeahead(text, ctx),
            Event::AccessibilityAction { target, action } => {
                if let Some(index) = self.items.iter().position(|item| item.id == *target) {
                    match action {
                        crate::SemanticAction::Click => self.activate(index, ctx),
                        crate::SemanticAction::Focus => {
                            self.set_active(index, ctx);
                            ctx.set_handled();
                        }
                        crate::SemanticAction::Increment | crate::SemanticAction::Decrement => {}
                    }
                }
            }
            Event::FocusChanged(focused) => {
                if let Some(index) = self.items.iter().position(|item| Some(item.id) == *focused)
                    && self.active != Some(index)
                {
                    self.active = Some(index);
                    ctx.request_paint();
                }
            }
            _ => {}
        }
    }

    fn widget_id(&self) -> Option<WidgetId> {
        Some(self.id)
    }

    fn reconcile(&mut self, next: &mut dyn Widget) {
        let Some(next) = next.as_any_mut().downcast_mut::<Self>() else {
            return;
        };
        next.id = self.id;
        next.active = self.active.filter(|index| *index < next.items.len());
        next.pressed = self.pressed.filter(|index| *index < next.items.len());
        next.typeahead.clone_from(&self.typeahead);
        for (current, next) in self.items.iter().zip(&mut next.items) {
            if current.kind == next.kind && current.label == next.label {
                next.id = current.id;
            }
        }
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.push(PaintOp::FillRect {
            rect: self.bounds,
            color: ctx.theme().button_disabled,
            radius: 8.0,
        });
        ctx.push(PaintOp::StrokeRect {
            rect: self.bounds,
            color: ctx.theme().button,
            width: 1.0,
            radius: 8.0,
        });
        for (index, item) in self.items.iter().enumerate() {
            if item.kind == MenuItemKind::Separator {
                ctx.push(PaintOp::FillRect {
                    rect: Rect::new(
                        item.bounds.x + 8.0,
                        item.bounds.y + 4.0,
                        (item.bounds.width - 16.0).max(0.0),
                        1.0,
                    ),
                    color: ctx.theme().button,
                    radius: 0.5,
                });
                continue;
            }
            if self.active == Some(index) && item.enabled {
                ctx.push(PaintOp::FillRect {
                    rect: item.bounds,
                    color: ctx.theme().button_hover,
                    radius: 5.0,
                });
            }
            if item.checked == Some(true) {
                ctx.push(PaintOp::FillRect {
                    rect: Rect::new(item.bounds.x + 10.0, item.bounds.y + 12.0, 10.0, 10.0),
                    color: ctx.theme().focus,
                    radius: if item.kind == MenuItemKind::Radio {
                        5.0
                    } else {
                        2.0
                    },
                });
            }
            paint_text_layout(
                ctx,
                &item.layout,
                Point::new(item.bounds.x + 30.0, item.bounds.y + 7.0),
                if item.enabled {
                    ctx.theme().text
                } else {
                    Color::rgb(157, 166, 181)
                },
                14.0,
            );
            if item.shortcut.is_some() {
                paint_text_layout(
                    ctx,
                    &item.shortcut_layout,
                    Point::new(
                        item.bounds.x + item.bounds.width - item.shortcut_layout.size.width - 12.0,
                        item.bounds.y + 8.0,
                    ),
                    ctx.theme().colors.text_muted,
                    13.0,
                );
            }
        }
    }

    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        ctx.push(SemanticNode {
            id: self.id,
            role: SemanticRole::Menu,
            bounds: self.bounds,
            label: String::new(),
            disabled: false,
            focused: false,
            checked: None,
            value: None,
            ime_cursor_area: None,
        });
        for (index, item) in self.items.iter().enumerate() {
            if item.kind == MenuItemKind::Separator {
                continue;
            }
            let role = match item.kind {
                MenuItemKind::Action => SemanticRole::MenuItem,
                MenuItemKind::Checkbox => SemanticRole::MenuItemCheckbox,
                MenuItemKind::Radio => SemanticRole::MenuItemRadio,
                MenuItemKind::Separator => continue,
            };
            ctx.push(SemanticNode {
                id: item.id,
                role,
                bounds: item.bounds,
                label: item.shortcut.as_ref().map_or_else(
                    || item.label.clone(),
                    |shortcut| format!("{} ({shortcut})", item.label),
                ),
                disabled: !item.enabled,
                focused: self.active == Some(index),
                checked: item.checked,
                value: None,
                ime_cursor_area: None,
            });
        }
    }
}

pub struct MenuButton {
    button: Button,
}

impl MenuButton {
    #[must_use]
    pub fn new(label: impl Into<String>, factory: impl FnMut() -> Menu + 'static) -> Self {
        let mut factory = factory;
        Self {
            button: Button::action(label, move |anchor, ctx| {
                ctx.show_overlay(anchor, OverlayPlacement::Below, factory());
            }),
        }
    }
}

impl Widget for MenuButton {
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

    fn apply_command(&mut self, command: UiCommand) -> Result<(), UiCommand> {
        self.button.apply_command(command)
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

pub struct ContextMenu {
    child: Box<dyn Widget>,
    factory: Box<dyn FnMut() -> Menu>,
}

impl ContextMenu {
    #[must_use]
    pub fn new(child: impl Widget + 'static, factory: impl FnMut() -> Menu + 'static) -> Self {
        Self {
            child: Box::new(child),
            factory: Box::new(factory),
        }
    }
}

impl Widget for ContextMenu {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.child.layout(ctx, constraints)
    }

    fn set_origin(&mut self, origin: Point) {
        self.child.set_origin(origin);
    }

    fn bounds(&self) -> Rect {
        self.child.bounds()
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &Event) {
        if let Event::PointerDown {
            position, button, ..
        } = event
            && *button == crate::PointerButton::Secondary
            && self.bounds().contains(*position)
        {
            ctx.show_context_menu(*position, (self.factory)());
            ctx.set_handled();
            return;
        }
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
    }

    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        self.child.semantics(ctx);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OverlayPlacement {
    Above,
    #[default]
    Below,
    Left,
    Right,
    Center,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OverlayModality {
    NonModal,
    Modal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OutsideDismissal {
    Disabled,
    Enabled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OverlayVisibility {
    Hidden,
    Visible,
}

/// Paints and routes an optional popup above a normal widget tree.
pub struct OverlayHost {
    base: Box<dyn Widget>,
    overlay: Option<Box<dyn Widget>>,
    anchor: Rect,
    anchor_id: Option<WidgetId>,
    anchor_is_absolute: bool,
    placement: OverlayPlacement,
    gap: f64,
    modality: OverlayModality,
    outside_dismissal: OutsideDismissal,
    visibility: OverlayVisibility,
    bounds: Rect,
    overlay_hovered: bool,
    on_dismiss: Box<dyn FnMut()>,
}

impl OverlayHost {
    #[must_use]
    pub fn new(base: impl Widget + 'static) -> Self {
        Self {
            base: Box::new(base),
            overlay: None,
            anchor: Rect::default(),
            anchor_id: None,
            anchor_is_absolute: false,
            placement: OverlayPlacement::Below,
            gap: 6.0,
            modality: OverlayModality::NonModal,
            outside_dismissal: OutsideDismissal::Enabled,
            visibility: OverlayVisibility::Hidden,
            bounds: Rect::default(),
            overlay_hovered: false,
            on_dismiss: Box::new(|| {}),
        }
    }

    #[must_use]
    pub fn overlay(
        mut self,
        anchor: Rect,
        placement: OverlayPlacement,
        overlay: impl Widget + 'static,
    ) -> Self {
        self.overlay = Some(Box::new(overlay));
        self.anchor = anchor;
        self.anchor_id = None;
        self.anchor_is_absolute = false;
        self.placement = placement;
        self.visibility = OverlayVisibility::Visible;
        self
    }

    #[must_use]
    pub fn centered_modal(mut self, overlay: impl Widget + 'static) -> Self {
        self.overlay = Some(Box::new(overlay));
        self.anchor_id = None;
        self.anchor_is_absolute = false;
        self.placement = OverlayPlacement::Center;
        self.modality = OverlayModality::Modal;
        self.visibility = OverlayVisibility::Visible;
        self
    }

    #[must_use]
    pub const fn modal(mut self, modal: bool) -> Self {
        self.modality = if modal {
            OverlayModality::Modal
        } else {
            OverlayModality::NonModal
        };
        self
    }

    #[must_use]
    pub const fn dismiss_on_outside_click(mut self, dismiss: bool) -> Self {
        self.outside_dismissal = if dismiss {
            OutsideDismissal::Enabled
        } else {
            OutsideDismissal::Disabled
        };
        self
    }

    #[must_use]
    pub fn gap(mut self, gap: f64) -> Self {
        self.gap = gap.max(0.0);
        self
    }

    #[must_use]
    pub fn on_dismiss(mut self, callback: impl FnMut() + 'static) -> Self {
        self.on_dismiss = Box::new(callback);
        self
    }

    fn dismiss(&mut self, ctx: &mut EventCtx) {
        if self.visibility == OverlayVisibility::Visible {
            self.visibility = OverlayVisibility::Hidden;
            self.overlay_hovered = false;
            (self.on_dismiss)();
            ctx.request_focus_restore();
            ctx.set_handled();
            ctx.request_paint();
        }
    }

    fn position_overlay(&mut self) {
        let anchor = self.anchor_id.and_then(|id| {
            let mut nodes = Vec::new();
            self.base.semantics(&mut SemanticsCtx::new(&mut nodes));
            nodes
                .into_iter()
                .find(|node| node.id == id)
                .map(|node| node.bounds)
        });
        let Some(overlay) = &mut self.overlay else {
            return;
        };
        let overlay_bounds = overlay.bounds();
        let anchor = anchor.unwrap_or_else(|| {
            if self.anchor_is_absolute {
                self.anchor
            } else {
                Rect::new(
                    self.bounds.x + self.anchor.x,
                    self.bounds.y + self.anchor.y,
                    self.anchor.width,
                    self.anchor.height,
                )
            }
        });
        let left = self.bounds.x;
        let top = self.bounds.y;
        let right = self.bounds.x + self.bounds.width;
        let bottom = self.bounds.y + self.bounds.height;
        let width = overlay_bounds.width;
        let height = overlay_bounds.height;
        let (mut x, mut y) = match self.placement {
            OverlayPlacement::Below => {
                let below = anchor.y + anchor.height + self.gap;
                let above = anchor.y - self.gap - height;
                let y = if below + height <= bottom || above < top {
                    below
                } else {
                    above
                };
                (anchor.x, y)
            }
            OverlayPlacement::Above => {
                let above = anchor.y - self.gap - height;
                let below = anchor.y + anchor.height + self.gap;
                let y = if above >= top || below + height > bottom {
                    above
                } else {
                    below
                };
                (anchor.x, y)
            }
            OverlayPlacement::Left => {
                let left_side = anchor.x - self.gap - width;
                let right_side = anchor.x + anchor.width + self.gap;
                let x = if left_side >= left || right_side + width > right {
                    left_side
                } else {
                    right_side
                };
                (x, anchor.y)
            }
            OverlayPlacement::Right => {
                let right_side = anchor.x + anchor.width + self.gap;
                let left_side = anchor.x - self.gap - width;
                let x = if right_side + width <= right || left_side < left {
                    right_side
                } else {
                    left_side
                };
                (x, anchor.y)
            }
            OverlayPlacement::Center => (
                self.bounds.x + (self.bounds.width - width) * 0.5,
                self.bounds.y + (self.bounds.height - height) * 0.5,
            ),
        };
        x = x.clamp(left, (right - width).max(left));
        y = y.clamp(top, (bottom - height).max(top));
        overlay.set_origin(Point::new(x, y));
    }

    fn route_visible_event(&mut self, ctx: &mut EventCtx, event: &Event) {
        let overlay = self.overlay.as_mut().expect("visible overlay");
        match event {
            Event::PointerMoved(position) if overlay.bounds().contains(*position) => {
                if !self.overlay_hovered {
                    self.overlay_hovered = true;
                    self.base.event(ctx, &Event::PointerLeft);
                }
                overlay.event(ctx, event);
            }
            Event::PointerMoved(_) => {
                if self.overlay_hovered {
                    self.overlay_hovered = false;
                    overlay.event(ctx, &Event::PointerLeft);
                }
                if self.modality == OverlayModality::NonModal {
                    self.base.event(ctx, event);
                }
            }
            Event::PointerLeft => {
                self.overlay_hovered = false;
                overlay.event(ctx, event);
                self.base.event(ctx, event);
            }
            Event::PointerDown { position, .. } if overlay.bounds().contains(*position) => {
                overlay.event(ctx, event);
            }
            Event::PointerDown { .. } if self.outside_dismissal == OutsideDismissal::Enabled => {
                self.dismiss(ctx);
            }
            Event::PointerDown { .. } | Event::PointerUp { .. } | Event::Scroll { .. }
                if self.modality == OverlayModality::Modal =>
            {
                ctx.set_handled();
            }
            Event::PointerUp { position, .. } | Event::Scroll { position, .. }
                if overlay.bounds().contains(*position) =>
            {
                overlay.event(ctx, event);
            }
            Event::KeyDown(crate::Key::Escape) => self.dismiss(ctx),
            Event::FocusChanged(_) => {
                self.base.event(ctx, event);
                overlay.event(ctx, event);
            }
            _ => {
                overlay.event(ctx, event);
                if !ctx.is_handled() && self.modality == OverlayModality::NonModal {
                    self.base.event(ctx, event);
                }
            }
        }
    }
}

impl Widget for OverlayHost {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let size = self.base.layout(ctx, constraints);
        self.bounds.width = size.width;
        self.bounds.height = size.height;
        if let Some(overlay) = &mut self.overlay {
            overlay.layout(ctx, BoxConstraints::loose(size));
        }
        self.position_overlay();
        size
    }

    fn set_origin(&mut self, origin: Point) {
        self.bounds.x = origin.x;
        self.bounds.y = origin.y;
        self.base.set_origin(origin);
        self.position_overlay();
    }

    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &Event) {
        if self.visibility == OverlayVisibility::Visible && self.overlay.is_some() {
            self.route_visible_event(ctx, event);
        } else {
            self.base.event(ctx, event);
        }
        self.position_overlay();
    }

    fn event_target(&mut self, ctx: &mut EventCtx, event: &Event, target: WidgetId) -> bool {
        if self.visibility == OverlayVisibility::Visible
            && let Some(overlay) = &mut self.overlay
            && overlay.event_target(ctx, event, target)
        {
            return true;
        }
        self.base.event_target(ctx, event, target)
    }

    fn apply_command(&mut self, command: UiCommand) -> Result<(), UiCommand> {
        match command {
            UiCommand::ShowOverlay {
                anchor,
                placement,
                overlay,
                modal,
            } => {
                self.overlay = Some(overlay);
                match anchor {
                    Some(OverlayAnchor::Widget(id)) => {
                        self.anchor_id = Some(id);
                        self.anchor_is_absolute = false;
                    }
                    Some(OverlayAnchor::Rect(rect)) => {
                        self.anchor = rect;
                        self.anchor_id = None;
                        self.anchor_is_absolute = true;
                    }
                    None => {
                        self.anchor_id = None;
                        self.anchor_is_absolute = false;
                    }
                }
                self.placement = placement;
                self.modality = if modal {
                    OverlayModality::Modal
                } else {
                    OverlayModality::NonModal
                };
                self.visibility = OverlayVisibility::Visible;
                self.overlay_hovered = false;
                Ok(())
            }
            UiCommand::DismissOverlay => {
                if self.visibility == OverlayVisibility::Visible {
                    self.visibility = OverlayVisibility::Hidden;
                    self.overlay_hovered = false;
                    (self.on_dismiss)();
                }
                Ok(())
            }
            UiCommand::InvokeCommand(id) => self.base.apply_command(UiCommand::InvokeCommand(id)),
        }
    }

    fn reconcile(&mut self, next: &mut dyn Widget) {
        let Some(next) = next.as_any_mut().downcast_mut::<Self>() else {
            return;
        };
        self.base.reconcile(next.base.as_mut());
        if let (Some(current), Some(next_overlay)) = (&mut self.overlay, &mut next.overlay) {
            current.reconcile(next_overlay.as_mut());
        } else if next.overlay.is_none() {
            next.overlay = self.overlay.take();
        }
        next.anchor = self.anchor;
        next.anchor_id = self.anchor_id;
        next.anchor_is_absolute = self.anchor_is_absolute;
        next.placement = self.placement;
        next.modality = self.modality;
        next.outside_dismissal = self.outside_dismissal;
        next.visibility = self.visibility;
        next.overlay_hovered = self.overlay_hovered;
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        self.base.paint(ctx);
        if self.visibility == OverlayVisibility::Hidden {
            return;
        }
        if self.modality == OverlayModality::Modal {
            ctx.push(PaintOp::FillRect {
                rect: self.bounds,
                color: Color::rgba(0, 0, 0, 140),
                radius: 0.0,
            });
        }
        if let Some(overlay) = &self.overlay {
            overlay.paint(ctx);
        }
    }

    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        if self.visibility == OverlayVisibility::Hidden
            || self.modality == OverlayModality::NonModal
        {
            self.base.semantics(ctx);
        }
        if self.visibility == OverlayVisibility::Visible
            && let Some(overlay) = &self.overlay
        {
            overlay.semantics(ctx);
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScrollAxes {
    Horizontal,
    #[default]
    Vertical,
    Both,
}

impl ScrollAxes {
    const fn horizontal(self) -> bool {
        matches!(self, Self::Horizontal | Self::Both)
    }

    const fn vertical(self) -> bool {
        matches!(self, Self::Vertical | Self::Both)
    }
}

/// A clipped viewport that can scroll any child widget.
pub struct ScrollView {
    id: WidgetId,
    child: Box<dyn Widget>,
    axes: ScrollAxes,
    bounds: Rect,
    content_size: Size,
    scroll_x: f64,
    scroll_y: f64,
    viewport_width: Option<f64>,
    viewport_height: Option<f64>,
    scroll_drag: Option<ScrollDrag>,
    hovered: bool,
}

impl ScrollView {
    #[must_use]
    pub fn new(child: impl Widget + 'static) -> Self {
        Self {
            id: WidgetId::next(),
            child: Box::new(child),
            axes: ScrollAxes::Vertical,
            bounds: Rect::default(),
            content_size: Size::ZERO,
            scroll_x: 0.0,
            scroll_y: 0.0,
            viewport_width: None,
            viewport_height: None,
            scroll_drag: None,
            hovered: false,
        }
    }

    #[must_use]
    pub const fn axes(mut self, axes: ScrollAxes) -> Self {
        self.axes = axes;
        self
    }

    #[must_use]
    pub fn viewport_width(mut self, width: f64) -> Self {
        self.viewport_width = Some(width.max(0.0));
        self
    }

    #[must_use]
    pub fn viewport_height(mut self, height: f64) -> Self {
        self.viewport_height = Some(height.max(0.0));
        self
    }

    fn scroll_extents(&self) -> (f64, f64) {
        (
            if self.axes.horizontal() {
                (self.content_size.width - self.bounds.width).max(0.0)
            } else {
                0.0
            },
            if self.axes.vertical() {
                (self.content_size.height - self.bounds.height).max(0.0)
            } else {
                0.0
            },
        )
    }

    fn position_child(&mut self) {
        self.child.set_origin(Point::new(
            self.bounds.x - self.scroll_x,
            self.bounds.y - self.scroll_y,
        ));
    }

    fn scroll_by(&mut self, delta_x: f64, delta_y: f64, ctx: &mut EventCtx) {
        let (max_x, max_y) = self.scroll_extents();
        let next_x = (self.scroll_x + delta_x).clamp(0.0, max_x);
        let next_y = (self.scroll_y + delta_y).clamp(0.0, max_y);
        if (next_x - self.scroll_x).abs() <= f64::EPSILON
            && (next_y - self.scroll_y).abs() <= f64::EPSILON
        {
            return;
        }
        self.scroll_x = next_x;
        self.scroll_y = next_y;
        self.position_child();
        ctx.set_handled();
        ctx.request_paint();
    }

    fn vertical_track(&self) -> Option<Rect> {
        self.vertical_thumb().map(|_| {
            Rect::new(
                self.bounds.x + self.bounds.width - 10.0,
                self.bounds.y + 4.0,
                8.0,
                (self.bounds.height - 8.0).max(0.0),
            )
        })
    }

    fn horizontal_track(&self) -> Option<Rect> {
        self.horizontal_thumb().map(|_| {
            Rect::new(
                self.bounds.x + 4.0,
                self.bounds.y + self.bounds.height - 10.0,
                (self.bounds.width - 8.0).max(0.0),
                8.0,
            )
        })
    }

    fn vertical_thumb(&self) -> Option<Rect> {
        let (_, max_y) = self.scroll_extents();
        if max_y <= f64::EPSILON {
            return None;
        }
        let track = Rect::new(
            self.bounds.x + self.bounds.width - 10.0,
            self.bounds.y + 4.0,
            8.0,
            (self.bounds.height - 8.0).max(0.0),
        );
        let height = (self.bounds.height / self.content_size.height * track.height)
            .clamp(20.0_f64.min(track.height), track.height);
        let travel = track.height - height;
        Some(Rect::new(
            track.x,
            track.y + self.scroll_y / max_y * travel,
            track.width,
            height,
        ))
    }

    fn horizontal_thumb(&self) -> Option<Rect> {
        let (max_x, _) = self.scroll_extents();
        if max_x <= f64::EPSILON {
            return None;
        }
        let track = Rect::new(
            self.bounds.x + 4.0,
            self.bounds.y + self.bounds.height - 10.0,
            (self.bounds.width - 8.0).max(0.0),
            8.0,
        );
        let width = (self.bounds.width / self.content_size.width * track.width)
            .clamp(20.0_f64.min(track.width), track.width);
        let travel = track.width - width;
        Some(Rect::new(
            track.x + self.scroll_x / max_x * travel,
            track.y,
            width,
            track.height,
        ))
    }

    fn drag_scrollbar(&mut self, position: Point, drag: ScrollDrag, ctx: &mut EventCtx) {
        let (max_x, max_y) = self.scroll_extents();
        match drag {
            ScrollDrag::Vertical { grab } => {
                let Some(thumb) = self.vertical_thumb() else {
                    return;
                };
                let track_start = self.bounds.y + 4.0;
                let travel = (self.bounds.height - 8.0 - thumb.height).max(0.0);
                if travel > f64::EPSILON {
                    self.scroll_y =
                        ((position.y - grab - track_start) / travel * max_y).clamp(0.0, max_y);
                }
            }
            ScrollDrag::Horizontal { grab } => {
                let Some(thumb) = self.horizontal_thumb() else {
                    return;
                };
                let track_start = self.bounds.x + 4.0;
                let travel = (self.bounds.width - 8.0 - thumb.width).max(0.0);
                if travel > f64::EPSILON {
                    self.scroll_x =
                        ((position.x - grab - track_start) / travel * max_x).clamp(0.0, max_x);
                }
            }
        }
        self.position_child();
        ctx.set_handled();
        ctx.request_paint();
    }

    fn handle_scrollbar_down(&mut self, position: Point, ctx: &mut EventCtx) -> bool {
        if let Some(thumb) = self.vertical_thumb()
            && thumb.contains(position)
        {
            self.scroll_drag = Some(ScrollDrag::Vertical {
                grab: position.y - thumb.y,
            });
            ctx.capture_pointer(self.id);
            ctx.set_handled();
            ctx.request_paint();
            return true;
        }
        if let Some(thumb) = self.horizontal_thumb()
            && thumb.contains(position)
        {
            self.scroll_drag = Some(ScrollDrag::Horizontal {
                grab: position.x - thumb.x,
            });
            ctx.capture_pointer(self.id);
            ctx.set_handled();
            ctx.request_paint();
            return true;
        }
        if let Some(track) = self.vertical_track()
            && track.contains(position)
        {
            if let Some(thumb) = self.vertical_thumb() {
                let delta = if position.y < thumb.y {
                    -self.bounds.height
                } else {
                    self.bounds.height
                };
                self.scroll_by(0.0, delta, ctx);
            }
            return true;
        }
        if let Some(track) = self.horizontal_track()
            && track.contains(position)
        {
            if let Some(thumb) = self.horizontal_thumb() {
                let delta = if position.x < thumb.x {
                    -self.bounds.width
                } else {
                    self.bounds.width
                };
                self.scroll_by(delta, 0.0, ctx);
            }
            return true;
        }
        false
    }

    fn paint_scrollbars(&self, ctx: &mut PaintCtx<'_>) {
        let color = if self.hovered || self.scroll_drag.is_some() {
            ctx.theme().button_hover
        } else {
            ctx.theme().button
        };
        if let Some(thumb) = self.vertical_thumb() {
            ctx.push(PaintOp::FillRect {
                rect: Rect::new(thumb.x + 2.0, thumb.y, 4.0, thumb.height),
                color,
                radius: 2.0,
            });
        }
        if let Some(thumb) = self.horizontal_thumb() {
            ctx.push(PaintOp::FillRect {
                rect: Rect::new(thumb.x, thumb.y + 2.0, thumb.width, 4.0),
                color,
                radius: 2.0,
            });
        }
    }
}

impl Widget for ScrollView {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let child_max = Size::new(
            if self.axes.horizontal() {
                f64::INFINITY
            } else {
                constraints.max.width
            },
            if self.axes.vertical() {
                f64::INFINITY
            } else {
                constraints.max.height
            },
        );
        self.content_size = self.child.layout(ctx, BoxConstraints::loose(child_max));
        let desired = Size::new(
            self.viewport_width.unwrap_or(self.content_size.width),
            self.viewport_height.unwrap_or(self.content_size.height),
        );
        let size = constraints.constrain(desired);
        self.bounds.width = size.width;
        self.bounds.height = size.height;
        let (max_x, max_y) = self.scroll_extents();
        self.scroll_x = self.scroll_x.clamp(0.0, max_x);
        self.scroll_y = self.scroll_y.clamp(0.0, max_y);
        self.position_child();
        size
    }

    fn set_origin(&mut self, origin: Point) {
        self.bounds.x = origin.x;
        self.bounds.y = origin.y;
        self.position_child();
    }

    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &Event) {
        match event {
            Event::PointerMoved(position) if self.scroll_drag.is_some() => {
                self.drag_scrollbar(*position, self.scroll_drag.expect("checked drag"), ctx);
            }
            Event::PointerMoved(position) => {
                let hovered = self.bounds.contains(*position);
                if hovered != self.hovered {
                    self.hovered = hovered;
                    ctx.request_paint();
                }
                self.child.event(ctx, event);
            }
            Event::PointerLeft => {
                if self.hovered {
                    self.hovered = false;
                    ctx.request_paint();
                }
                self.child.event(ctx, event);
            }
            Event::PointerDown {
                position, button, ..
            } if *button == crate::PointerButton::Primary && self.bounds.contains(*position) => {
                if !self.handle_scrollbar_down(*position, ctx) {
                    self.child.event(ctx, event);
                }
            }
            Event::PointerUp { button, .. }
                if *button == crate::PointerButton::Primary && self.scroll_drag.is_some() =>
            {
                self.scroll_drag = None;
                ctx.release_pointer();
                ctx.set_handled();
                ctx.request_paint();
            }
            Event::Scroll {
                position,
                delta_x,
                delta_y,
            } if self.bounds.contains(*position) => {
                self.child.event(ctx, event);
                if !ctx.is_handled() {
                    self.scroll_by(*delta_x, *delta_y, ctx);
                }
            }
            Event::PointerDown { .. } | Event::Scroll { .. } => {}
            _ => self.child.event(ctx, event),
        }
    }

    fn widget_id(&self) -> Option<WidgetId> {
        Some(self.id)
    }

    fn event_target(&mut self, ctx: &mut EventCtx, event: &Event, target: WidgetId) -> bool {
        if target == self.id {
            self.event(ctx, event);
            true
        } else {
            self.child.event_target(ctx, event, target)
        }
    }

    fn apply_command(&mut self, command: UiCommand) -> Result<(), UiCommand> {
        self.child.apply_command(command)
    }

    fn reconcile(&mut self, next: &mut dyn Widget) {
        if let Some(next) = next.as_any_mut().downcast_mut::<Self>() {
            next.id = self.id;
            next.scroll_x = self.scroll_x;
            next.scroll_y = self.scroll_y;
            next.scroll_drag = self.scroll_drag;
            next.hovered = self.hovered;
            self.child.reconcile(next.child.as_mut());
        }
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.push(PaintOp::PushClip {
            rect: self.bounds,
            radius: 0.0,
        });
        self.child.paint(ctx);
        ctx.push(PaintOp::PopClip);
        self.paint_scrollbars(ctx);
    }

    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        self.child.semantics(ctx);
    }
}

pub struct Column {
    children: Vec<Box<dyn Widget>>,
    gap: f64,
    bounds: Rect,
    hovered_child: Option<usize>,
}

impl Column {
    #[must_use]
    pub fn new(children: impl IntoIterator<Item = Box<dyn Widget>>) -> Self {
        Self {
            children: children.into_iter().collect(),
            gap: 8.0,
            bounds: Rect::default(),
            hovered_child: None,
        }
    }

    #[must_use]
    pub fn gap(mut self, gap: f64) -> Self {
        self.gap = gap.max(0.0);
        self
    }
}

impl Widget for Column {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let mut y = 0.0;
        let mut width: f64 = 0.0;
        let count = self.children.len();
        for child in &mut self.children {
            let child_size = child.layout(ctx, BoxConstraints::loose(constraints.max));
            child.set_origin(Point::new(self.bounds.x, self.bounds.y + y));
            y += child_size.height + self.gap;
            width = width.max(child_size.width);
        }
        if count > 0 {
            y -= self.gap;
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
        for child in &mut self.children {
            let bounds = child.bounds();
            child.set_origin(Point::new(bounds.x + dx, bounds.y + dy));
        }
    }

    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &Event) {
        route_children(&mut self.children, &mut self.hovered_child, ctx, event);
    }

    fn event_target(&mut self, ctx: &mut EventCtx, event: &Event, target: WidgetId) -> bool {
        event_target_children(&mut self.children, ctx, event, target)
    }

    fn apply_command(&mut self, command: UiCommand) -> Result<(), UiCommand> {
        apply_command_children(&mut self.children, command)
    }

    fn reconcile(&mut self, next: &mut dyn Widget) {
        if let Some(next) = next.as_any_mut().downcast_mut::<Self>() {
            reconcile_children(&mut self.children, &mut next.children);
            next.hovered_child = self
                .hovered_child
                .filter(|index| *index < next.children.len());
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

pub struct Row {
    children: Vec<Box<dyn Widget>>,
    gap: f64,
    bounds: Rect,
    hovered_child: Option<usize>,
}

impl Row {
    #[must_use]
    pub fn new(children: impl IntoIterator<Item = Box<dyn Widget>>) -> Self {
        Self {
            children: children.into_iter().collect(),
            gap: 8.0,
            bounds: Rect::default(),
            hovered_child: None,
        }
    }

    #[must_use]
    pub fn gap(mut self, gap: f64) -> Self {
        self.gap = gap.max(0.0);
        self
    }
}

impl Widget for Row {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let mut x = 0.0;
        let mut height: f64 = 0.0;
        let count = self.children.len();
        for child in &mut self.children {
            let remaining = (constraints.max.width - x).max(0.0);
            let child_size = child.layout(
                ctx,
                BoxConstraints::loose(Size::new(remaining, constraints.max.height)),
            );
            child.set_origin(Point::new(self.bounds.x + x, self.bounds.y));
            x += child_size.width + self.gap;
            height = height.max(child_size.height);
        }
        if count > 0 {
            x -= self.gap;
        }
        let size = constraints.constrain(Size::new(x, height));
        self.bounds.width = size.width;
        self.bounds.height = size.height;
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
        route_children(&mut self.children, &mut self.hovered_child, ctx, event);
    }

    fn event_target(&mut self, ctx: &mut EventCtx, event: &Event, target: WidgetId) -> bool {
        event_target_children(&mut self.children, ctx, event, target)
    }

    fn apply_command(&mut self, command: UiCommand) -> Result<(), UiCommand> {
        apply_command_children(&mut self.children, command)
    }

    fn reconcile(&mut self, next: &mut dyn Widget) {
        if let Some(next) = next.as_any_mut().downcast_mut::<Self>() {
            reconcile_children(&mut self.children, &mut next.children);
            next.hovered_child = self
                .hovered_child
                .filter(|index| *index < next.children.len());
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

pub struct Checkbox {
    id: WidgetId,
    label: String,
    checked: bool,
    bounds: Rect,
    interaction: Interaction,
    text_layout: TextLayout,
    on_change: Box<dyn FnMut(bool)>,
}

impl Checkbox {
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
            bounds: Rect::default(),
            interaction: Interaction::default(),
            text_layout: TextLayout::default(),
            on_change: Box::new(on_change),
        }
    }

    fn toggle(&mut self, ctx: &mut EventCtx) {
        self.checked = !self.checked;
        (self.on_change)(self.checked);
        ctx.set_handled();
        ctx.request_paint();
    }
}

impl Widget for Checkbox {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.text_layout = ctx.layout_text(&self.label, 15.0, None);
        let size = constraints.constrain(Size::new(
            32.0 + self.text_layout.size.width,
            self.text_layout.size.height.max(24.0),
        ));
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
        match event {
            Event::PointerMoved(position) => {
                let hovered = self.bounds.contains(*position);
                if hovered != self.interaction.hovered {
                    self.interaction.hovered = hovered;
                    ctx.request_paint();
                }
            }
            Event::PointerLeft => {
                if self.interaction.hovered {
                    self.interaction.hovered = false;
                    ctx.request_paint();
                }
            }
            Event::PointerDown {
                position, button, ..
            } if *button == crate::PointerButton::Primary && self.bounds.contains(*position) => {
                self.interaction.pressed = true;
                ctx.capture_pointer(self.id);
                ctx.request_focus(self.id);
                ctx.request_paint();
                ctx.set_handled();
            }
            Event::PointerUp { position, button } if *button == crate::PointerButton::Primary => {
                let toggle = self.interaction.pressed && self.bounds.contains(*position);
                self.interaction.pressed = false;
                ctx.release_pointer();
                if toggle {
                    self.toggle(ctx);
                } else {
                    ctx.request_paint();
                }
            }
            Event::KeyDown(crate::Key::Enter | crate::Key::Space) if self.interaction.focused => {
                self.toggle(ctx);
            }
            Event::AccessibilityAction { target, action } if *target == self.id => match action {
                crate::SemanticAction::Click => self.toggle(ctx),
                crate::SemanticAction::Focus => {
                    ctx.request_focus(self.id);
                    ctx.set_handled();
                }
                crate::SemanticAction::Increment | crate::SemanticAction::Decrement => {}
            },
            Event::FocusChanged(focused) => {
                let next = *focused == Some(self.id);
                if next != self.interaction.focused {
                    self.interaction.focused = next;
                    ctx.request_paint();
                }
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
        let box_rect = Rect::new(
            self.bounds.x,
            self.bounds.y + (self.bounds.height - 20.0) * 0.5,
            20.0,
            20.0,
        );
        ctx.push(PaintOp::FillRect {
            rect: box_rect,
            color: if self.interaction.hovered {
                ctx.theme().button_hover
            } else {
                ctx.theme().button
            },
            radius: 5.0,
        });
        if self.checked {
            ctx.push(PaintOp::FillRect {
                rect: Rect::new(box_rect.x + 5.0, box_rect.y + 5.0, 10.0, 10.0),
                color: Color::WHITE,
                radius: 2.0,
            });
        }
        if self.interaction.focused {
            ctx.push(PaintOp::StrokeRect {
                rect: box_rect,
                color: ctx.theme().focus,
                width: 2.0,
                radius: 5.0,
            });
        }
        let text_y = self.bounds.y + (self.bounds.height - self.text_layout.size.height) * 0.5;
        paint_text_layout(
            ctx,
            &self.text_layout,
            Point::new(self.bounds.x + 30.0, text_y),
            ctx.theme().text,
            15.0,
        );
    }

    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        ctx.push(SemanticNode {
            id: self.id,
            role: SemanticRole::Checkbox,
            bounds: self.bounds,
            label: self.label.clone(),
            disabled: false,
            focused: self.interaction.focused,
            checked: Some(self.checked),
            value: None,
            ime_cursor_area: None,
        });
    }
}

pub struct Keyed {
    key: WidgetKey,
    child: Box<dyn Widget>,
}

impl Keyed {
    #[must_use]
    pub fn new(key: impl Into<WidgetKey>, child: impl Widget + 'static) -> Self {
        Self {
            key: key.into(),
            child: Box::new(child),
        }
    }
}

impl Widget for Keyed {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.child.layout(ctx, constraints)
    }

    fn set_origin(&mut self, origin: Point) {
        self.child.set_origin(origin);
    }

    fn bounds(&self) -> Rect {
        self.child.bounds()
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

    fn widget_key(&self) -> Option<&WidgetKey> {
        Some(&self.key)
    }

    fn reconcile(&mut self, next: &mut dyn Widget) {
        if let Some(next) = next.as_any_mut().downcast_mut::<Self>()
            && self.key == next.key
        {
            self.child.reconcile(next.child.as_mut());
        }
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        self.child.paint(ctx);
    }

    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        self.child.semantics(ctx);
    }
}

pub struct Padding {
    insets: Insets,
    child: Box<dyn Widget>,
    bounds: Rect,
}

impl Padding {
    #[must_use]
    pub fn new(insets: Insets, child: impl Widget + 'static) -> Self {
        Self {
            insets,
            child: Box::new(child),
            bounds: Rect::default(),
        }
    }
}

impl Widget for Padding {
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
        let dx = origin.x - self.bounds.x;
        let dy = origin.y - self.bounds.y;
        self.bounds.x = origin.x;
        self.bounds.y = origin.y;
        let child = self.child.bounds();
        self.child
            .set_origin(Point::new(child.x + dx, child.y + dy));
    }

    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &Event) {
        match event {
            Event::PointerMoved(position) if self.child.bounds().contains(*position) => {
                self.child.event(ctx, event);
            }
            Event::PointerMoved(_) | Event::PointerLeft => {
                self.child.event(ctx, &Event::PointerLeft);
            }
            Event::PointerDown { position, .. }
            | Event::PointerUp { position, .. }
            | Event::Scroll { position, .. }
                if self.child.bounds().contains(*position) =>
            {
                self.child.event(ctx, event);
            }
            Event::PointerDown { .. } | Event::PointerUp { .. } | Event::Scroll { .. } => {}
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
        }
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        self.child.paint(ctx);
    }

    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        self.child.semantics(ctx);
    }
}

pub(crate) fn paint_text_layout(
    ctx: &mut PaintCtx<'_>,
    layout: &TextLayout,
    origin: Point,
    color: Color,
    fallback_size: f32,
) {
    if let Some(text) = &layout.fallback_text {
        ctx.push(PaintOp::Text {
            origin: Point::new(origin.x, origin.y + f64::from(fallback_size)),
            text: text.clone(),
            color,
            size: fallback_size,
        });
    }
    for run in &layout.runs {
        ctx.push(PaintOp::GlyphRun {
            origin,
            font: run.font.clone(),
            font_size: run.font_size,
            normalized_coords: run.normalized_coords.clone(),
            glyphs: run.glyphs.clone(),
            color,
        });
    }
}

fn previous_char_boundary(text: &str, cursor: usize) -> usize {
    text[..cursor]
        .char_indices()
        .next_back()
        .map_or(cursor, |(index, _)| index)
}

fn next_char_boundary(text: &str, cursor: usize) -> usize {
    text[cursor..]
        .char_indices()
        .nth(1)
        .map_or(text.len(), |(index, _)| cursor + index)
}

fn previous_word_boundary(text: &str, cursor: usize) -> usize {
    text[..cursor]
        .unicode_word_indices()
        .next_back()
        .map_or(0, |(index, _)| index)
}

fn next_word_boundary(text: &str, cursor: usize) -> usize {
    text[cursor..]
        .unicode_word_indices()
        .next()
        .map_or(text.len(), |(index, word)| cursor + index + word.len())
}

/// A compact determinate progress indicator.
pub struct ProgressBar {
    id: WidgetId,
    fraction: f64,
    label: String,
    bounds: Rect,
}

impl ProgressBar {
    #[must_use]
    pub fn new(fraction: f64) -> Self {
        Self {
            id: WidgetId::next(),
            fraction: fraction.clamp(0.0, 1.0),
            label: "Progress".to_owned(),
            bounds: Rect::default(),
        }
    }

    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }
}

impl Widget for ProgressBar {
    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let width = if constraints.max.width.is_finite() {
            constraints.max.width
        } else {
            160.0
        };
        let size = constraints.constrain(Size::new(width, 8.0));
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
        let radius = self.bounds.height * 0.5;
        ctx.push(PaintOp::FillRect {
            rect: self.bounds,
            color: ctx.theme().colors.secondary,
            radius,
        });
        ctx.push(PaintOp::FillRect {
            rect: Rect::new(
                self.bounds.x,
                self.bounds.y,
                self.bounds.width * self.fraction,
                self.bounds.height,
            ),
            color: ctx.theme().colors.primary,
            radius,
        });
    }

    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        ctx.push(SemanticNode {
            id: self.id,
            role: SemanticRole::StatusBar,
            bounds: self.bounds,
            label: self.label.clone(),
            disabled: false,
            focused: false,
            checked: None,
            value: Some(format!("{:.0}%", self.fraction * 100.0)),
            ime_cursor_area: None,
        });
    }
}

#[allow(clippy::cast_possible_truncation)]
fn width_as_f32(width: f64) -> f32 {
    width.min(f64::from(f32::MAX)) as f32
}
