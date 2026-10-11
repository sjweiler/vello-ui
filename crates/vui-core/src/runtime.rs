use crate::event::PointerCaptureRequest;
use crate::text::BootstrapTextSystem;
use crate::{
    AnimationFrame, BoxConstraints, ClipboardRequest, DragOperation, DragOperations, DragPayload,
    DragPreview, Event, EventCtx, Key, LayoutCtx, PaintCtx, PaintList, PaintOp, Point, Rect,
    SemanticNode, SemanticRole, SemanticsCtx, Size, TaskId, TaskRequest, TaskUpdate, TextSystem,
    Theme, UiCommand, Widget, WidgetId, drag::DragRequest, task::TaskCommand,
};
use std::{collections::HashMap, time::Duration};

#[derive(Clone, Debug, PartialEq)]
pub struct UiFrame {
    pub viewport: Size,
    pub damage: Vec<Rect>,
    pub paint: PaintList,
    pub semantics: Vec<SemanticNode>,
}

pub struct UiRuntime {
    window_id: Option<crate::WindowId>,
    root: Box<dyn Widget>,
    viewport: Size,
    theme: Theme,
    text: Box<dyn TextSystem>,
    focused: Option<WidgetId>,
    needs_layout: bool,
    needs_paint: bool,
    clipboard_request: Option<ClipboardRequest>,
    pointer_capture: Option<WidgetId>,
    overlay_focus_stack: Vec<Option<WidgetId>>,
    drag: Option<DragSession>,
    now: Duration,
    last_animation_frame: Option<Duration>,
    animation_deadline: Option<Duration>,
    reduced_motion: bool,
    tasks: HashMap<TaskId, ActiveTask>,
    pending_tasks: Vec<TaskRequest>,
    platform_commands: Vec<crate::PlatformCommand>,
}

struct ActiveTask {
    owner: WidgetId,
    cancellation: crate::CancellationToken,
}

struct DragSession {
    source: WidgetId,
    payload: DragPayload,
    allowed: DragOperations,
    preview: DragPreview,
    position: Point,
    accepted: Option<(WidgetId, DragOperation)>,
}

impl UiRuntime {
    #[must_use]
    pub fn new(root: impl Widget + 'static) -> Self {
        Self::from_boxed(Box::new(root))
    }

    #[must_use]
    pub fn from_boxed(root: Box<dyn Widget>) -> Self {
        Self {
            window_id: None,
            root,
            viewport: Size::ZERO,
            theme: Theme::default(),
            text: Box::<BootstrapTextSystem>::default(),
            focused: None,
            needs_layout: true,
            needs_paint: true,
            clipboard_request: None,
            pointer_capture: None,
            overlay_focus_stack: Vec::new(),
            drag: None,
            now: Duration::ZERO,
            last_animation_frame: None,
            animation_deadline: None,
            reduced_motion: false,
            tasks: HashMap::new(),
            pending_tasks: Vec::new(),
            platform_commands: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    #[must_use]
    pub const fn with_window_id(mut self, window_id: crate::WindowId) -> Self {
        self.window_id = Some(window_id);
        self
    }

    #[must_use]
    pub const fn window_id(&self) -> Option<crate::WindowId> {
        self.window_id
    }

    #[must_use]
    pub fn with_text_system(mut self, text: impl TextSystem + 'static) -> Self {
        self.text = Box::new(text);
        self.needs_layout = true;
        self.needs_paint = true;
        self
    }

    pub fn set_viewport(&mut self, viewport: Size) {
        if self.viewport != viewport {
            self.viewport = viewport;
            self.needs_layout = true;
            self.needs_paint = true;
        }
    }

    pub fn set_theme(&mut self, theme: Theme) {
        if self.theme != theme {
            self.theme = theme;
            self.needs_layout = true;
            self.needs_paint = true;
        }
    }

    pub fn handle_event(&mut self, event: &Event) -> bool {
        if matches!(event, Event::KeyDown(Key::Escape)) && self.drag.is_some() {
            self.cancel_drag();
            return true;
        }
        if matches!(event, Event::KeyDown(Key::Tab)) {
            return self.focus_next();
        }
        let mut ctx = self.event_ctx();
        let capture_event = matches!(
            event,
            Event::PointerMoved(_) | Event::PointerUp { .. } | Event::PointerLeft
        );
        let delivered = if capture_event {
            self.pointer_capture
                .is_some_and(|target| self.root.event_target(&mut ctx, event, target))
        } else {
            false
        };
        if !delivered {
            self.root.event(&mut ctx, event);
        }
        let handled = ctx.is_handled();
        let drag_request = ctx.take_drag_request();
        self.apply_event_ctx(&mut ctx, event);
        self.apply_drag_request(drag_request);
        let drag_handled = match event {
            Event::PointerMoved(position) if self.drag.is_some() => {
                self.update_drag(*position);
                true
            }
            Event::PointerUp { position, .. } if self.drag.is_some() => {
                self.finish_drag(*position);
                true
            }
            Event::PointerLeft if self.drag.is_some() => {
                self.cancel_drag();
                true
            }
            _ => false,
        };
        handled || drag_handled
    }

    #[must_use]
    pub fn render_frame(&mut self) -> Option<UiFrame> {
        if self.reduced_motion
            && self
                .animation_deadline
                .is_some_and(|deadline| deadline <= self.now)
        {
            self.advance_time(self.now);
        }
        if self.viewport.width <= 0.0 || self.viewport.height <= 0.0 {
            return None;
        }

        if self.needs_layout {
            let mut layout_ctx = LayoutCtx::new(self.text.as_mut(), &self.theme);
            let constraints = BoxConstraints::loose(self.viewport);
            self.root.layout(&mut layout_ctx, constraints);
            self.root.set_origin(Point::new(0.0, 0.0));
            self.needs_layout = false;
            self.needs_paint = true;
        }

        self.prune_task_owners();

        if !self.needs_paint {
            return None;
        }

        let mut paint = PaintList::default();
        paint.push(PaintOp::FillRect {
            rect: Rect::from_origin_size(Point::new(0.0, 0.0), self.viewport),
            color: self.theme.window,
            radius: 0.0,
        });
        self.root.paint(&mut PaintCtx::new(&mut paint, &self.theme));
        self.paint_drag_preview(&mut paint);
        let mut semantics = Vec::new();
        self.root.semantics(&mut SemanticsCtx::new(&mut semantics));
        self.needs_paint = false;

        Some(UiFrame {
            viewport: self.viewport,
            damage: vec![Rect::from_origin_size(Point::new(0.0, 0.0), self.viewport)],
            paint,
            semantics,
        })
    }

    pub fn request_paint(&mut self) {
        self.needs_paint = true;
    }

    pub fn take_clipboard_request(&mut self) -> Option<ClipboardRequest> {
        self.clipboard_request.take()
    }

    #[must_use]
    pub const fn drag_active(&self) -> bool {
        self.drag.is_some()
    }

    pub fn set_reduced_motion(&mut self, reduced_motion: bool) {
        if self.reduced_motion != reduced_motion {
            self.reduced_motion = reduced_motion;
            if reduced_motion && self.animation_deadline.is_some() {
                self.animation_deadline = Some(self.now);
            }
        }
    }

    #[must_use]
    pub const fn reduced_motion(&self) -> bool {
        self.reduced_motion
    }

    #[must_use]
    pub const fn animation_pending(&self) -> bool {
        self.animation_deadline.is_some()
    }

    #[must_use]
    pub const fn next_animation_deadline(&self) -> Option<Duration> {
        self.animation_deadline
    }

    pub fn advance_time(&mut self, now: Duration) -> bool {
        self.now = self.now.max(now);
        let Some(deadline) = self.animation_deadline else {
            return false;
        };
        if deadline > self.now {
            return false;
        }
        self.animation_deadline = None;
        let delta = self
            .last_animation_frame
            .map_or(Duration::ZERO, |previous| self.now.saturating_sub(previous));
        self.last_animation_frame = Some(self.now);
        let event = Event::AnimationFrame(AnimationFrame {
            elapsed: self.now,
            delta,
            reduced_motion: self.reduced_motion,
        });
        let mut ctx = self.event_ctx();
        self.root.event(&mut ctx, &event);
        self.apply_event_ctx(&mut ctx, &event);
        self.needs_paint = true;
        true
    }

    pub fn take_task_requests(&mut self) -> Vec<TaskRequest> {
        std::mem::take(&mut self.pending_tasks)
    }

    pub fn take_platform_commands(&mut self) -> Vec<crate::PlatformCommand> {
        std::mem::take(&mut self.platform_commands)
    }

    pub fn handle_task_update(&mut self, update: TaskUpdate) -> bool {
        let id = update.id();
        let Some(owner) = self.tasks.get(&id).map(|task| task.owner) else {
            return false;
        };
        let terminal = update.is_terminal();
        let event = match update {
            TaskUpdate::Progress {
                id,
                fraction,
                payload,
            } => Event::TaskProgress {
                id,
                fraction,
                payload,
            },
            TaskUpdate::Completed { id, payload } => Event::TaskCompleted { id, payload },
            TaskUpdate::Failed { id, error } => Event::TaskFailed { id, error },
            TaskUpdate::Cancelled { id } => Event::TaskCancelled { id },
        };
        let mut ctx = self.event_ctx();
        let delivered = self.root.event_target(&mut ctx, &event, owner);
        self.apply_event_ctx(&mut ctx, &event);
        if terminal || !delivered {
            self.cancel_task(id);
        }
        delivered
    }

    fn apply_event_ctx(&mut self, ctx: &mut EventCtx, event: &Event) {
        self.needs_layout |= ctx.layout_requested();
        self.needs_paint |= ctx.paint_requested();
        match ctx.pointer_capture_request() {
            Some(PointerCaptureRequest::Capture(id)) => self.pointer_capture = Some(id),
            Some(PointerCaptureRequest::Release) => self.pointer_capture = None,
            None if matches!(event, Event::PointerUp { .. }) => self.pointer_capture = None,
            None => {}
        }
        self.clipboard_request = ctx.take_clipboard_request();
        if let Some(id) = ctx.focus_request() {
            self.set_focus(Some(id));
        }
        if ctx.focus_restore_requested() {
            self.restore_overlay_focus();
        }
        for command in ctx.take_commands() {
            self.apply_command(command);
        }
        self.platform_commands.extend(ctx.take_platform_commands());
        if let Some(delay) = ctx.take_animation_delay() {
            let deadline = self.now.saturating_add(delay);
            if self.animation_deadline.is_none() {
                self.last_animation_frame = Some(self.now);
            }
            self.animation_deadline = Some(
                self.animation_deadline
                    .map_or(deadline, |current| current.min(deadline)),
            );
        }
        for command in ctx.take_task_commands() {
            match command {
                TaskCommand::Spawn(request) => {
                    let id = request.id();
                    self.tasks.insert(
                        id,
                        ActiveTask {
                            owner: request.owner(),
                            cancellation: request.cancellation_token(),
                        },
                    );
                    self.pending_tasks.push(request);
                }
                TaskCommand::Cancel(id) => self.cancel_task(id),
            }
        }
    }

    fn apply_drag_request(&mut self, request: Option<DragRequest>) {
        match request {
            Some(DragRequest::Start {
                source,
                payload,
                allowed,
                preview,
                position,
            }) => {
                self.drag = Some(DragSession {
                    source,
                    payload,
                    allowed,
                    preview,
                    position,
                    accepted: None,
                });
                self.needs_paint = true;
            }
            Some(DragRequest::Cancel) => self.cancel_drag(),
            Some(DragRequest::Accept { .. }) | None => {}
        }
    }

    fn dispatch_drag_event(
        &mut self,
        event: &Event,
        target: Option<WidgetId>,
    ) -> Option<DragRequest> {
        let mut ctx = self.event_ctx();
        if let Some(target) = target {
            self.root.event_target(&mut ctx, event, target);
        } else {
            self.root.event(&mut ctx, event);
        }
        let request = ctx.take_drag_request();
        self.apply_event_ctx(&mut ctx, event);
        request
    }

    fn update_drag(&mut self, position: Point) {
        let Some(session) = self.drag.as_ref() else {
            return;
        };
        let payload = session.payload.clone();
        let previous = session.accepted;
        let request = self.dispatch_drag_event(
            &Event::DragMoved {
                position,
                payload: payload.clone(),
            },
            None,
        );
        let accepted = match request {
            Some(DragRequest::Accept { target, operation })
                if self
                    .drag
                    .as_ref()
                    .is_some_and(|drag| drag.allowed.contains(operation)) =>
            {
                Some((target, operation))
            }
            Some(DragRequest::Cancel) => {
                self.cancel_drag();
                return;
            }
            _ => None,
        };
        if previous.map(|entry| entry.0) != accepted.map(|entry| entry.0) {
            if let Some((target, _)) = previous {
                self.dispatch_drag_event(
                    &Event::DragLeft {
                        payload: payload.clone(),
                    },
                    Some(target),
                );
            }
            if let Some((target, _)) = accepted {
                self.dispatch_drag_event(
                    &Event::DragEntered {
                        position,
                        payload: payload.clone(),
                    },
                    Some(target),
                );
            }
        }
        if let Some(session) = self.drag.as_mut() {
            session.position = position;
            session.accepted = accepted;
        }
        self.needs_paint = true;
    }

    fn finish_drag(&mut self, position: Point) {
        let Some(session) = self.drag.take() else {
            return;
        };
        if let Some((target, operation)) = session.accepted {
            self.dispatch_drag_event(
                &Event::Drop {
                    position,
                    payload: session.payload.clone(),
                    operation,
                },
                Some(target),
            );
            self.dispatch_drag_event(
                &Event::DragCompleted {
                    payload: session.payload,
                    operation,
                },
                Some(session.source),
            );
        } else {
            self.dispatch_drag_event(
                &Event::DragCancelled {
                    payload: session.payload,
                },
                Some(session.source),
            );
        }
        self.needs_paint = true;
    }

    fn cancel_drag(&mut self) {
        let Some(session) = self.drag.take() else {
            return;
        };
        if let Some((target, _)) = session.accepted {
            self.dispatch_drag_event(
                &Event::DragLeft {
                    payload: session.payload.clone(),
                },
                Some(target),
            );
        }
        self.dispatch_drag_event(
            &Event::DragCancelled {
                payload: session.payload,
            },
            Some(session.source),
        );
        self.needs_paint = true;
    }

    fn paint_drag_preview(&self, paint: &mut PaintList) {
        let Some(session) = &self.drag else {
            return;
        };
        let character_count =
            u32::try_from(session.preview.label.chars().count()).unwrap_or(u32::MAX);
        let width = (f64::from(character_count) * 7.0 + 24.0).clamp(72.0, 280.0);
        let rect = Rect::new(
            session.position.x + 12.0,
            session.position.y + 12.0,
            width,
            30.0,
        );
        paint.push(PaintOp::FillRect {
            rect,
            color: self.theme.colors.surface_raised,
            radius: self.theme.radii.sm,
        });
        paint.push(PaintOp::StrokeRect {
            rect,
            color: if session.accepted.is_some() {
                self.theme.colors.primary
            } else {
                self.theme.colors.border
            },
            width: 1.0,
            radius: self.theme.radii.sm,
        });
        paint.push(PaintOp::Text {
            origin: Point::new(rect.x + 12.0, rect.y + 20.0),
            text: session.preview.label.clone(),
            color: self.theme.colors.text,
            size: 13.0,
        });
    }

    fn cancel_task(&mut self, id: TaskId) {
        if let Some(task) = self.tasks.remove(&id) {
            task.cancellation.cancel();
        }
    }

    fn prune_task_owners(&mut self) {
        let owners = self
            .tasks
            .iter()
            .map(|(id, task)| (*id, task.owner))
            .collect::<Vec<_>>();
        for (id, owner) in owners {
            let mut ctx = self.event_ctx();
            if !self
                .root
                .event_target(&mut ctx, &Event::TaskOwnerProbe, owner)
            {
                self.cancel_task(id);
            }
        }
    }

    fn apply_command(&mut self, command: UiCommand) {
        if let UiCommand::InvokeCommand(id) = command {
            let mut ctx = self.event_ctx();
            let event = Event::CommandInvoked(id);
            self.root.event(&mut ctx, &event);
            self.apply_event_ctx(&mut ctx, &event);
            return;
        }
        let showing = matches!(&command, UiCommand::ShowOverlay { .. });
        let dismissing = matches!(&command, UiCommand::DismissOverlay);
        let previous_focusable = showing.then(|| self.focusable_ids());
        if self.root.apply_command(command).is_err() {
            return;
        }
        self.needs_layout = true;
        self.needs_paint = true;
        if let Some(previous_focusable) = previous_focusable {
            self.overlay_focus_stack.push(self.focused);
            let next = self
                .focusable_ids()
                .into_iter()
                .find(|id| !previous_focusable.contains(id));
            self.set_focus(next);
        } else if dismissing {
            self.restore_overlay_focus();
        }
    }

    fn focusable_ids(&self) -> Vec<WidgetId> {
        let mut semantics = Vec::new();
        self.root.semantics(&mut SemanticsCtx::new(&mut semantics));
        semantics
            .iter()
            .filter(|node| {
                !node.disabled
                    && matches!(
                        node.role,
                        SemanticRole::Button
                            | SemanticRole::Tab
                            | SemanticRole::Checkbox
                            | SemanticRole::MenuItem
                            | SemanticRole::MenuItemCheckbox
                            | SemanticRole::MenuItemRadio
                            | SemanticRole::List
                            | SemanticRole::Tree
                            | SemanticRole::Table
                            | SemanticRole::TextInput
                            | SemanticRole::MultilineTextInput
                    )
            })
            .map(|node| node.id)
            .collect()
    }

    fn set_focus(&mut self, focused: Option<WidgetId>) {
        if self.focused == focused {
            return;
        }
        self.focused = focused;
        let mut ctx = self.event_ctx();
        self.root
            .event(&mut ctx, &Event::FocusChanged(self.focused));
        self.needs_paint = true;
    }

    fn restore_overlay_focus(&mut self) {
        let Some(previous) = self.overlay_focus_stack.pop() else {
            return;
        };
        let focusable = self.focusable_ids();
        let restored = previous.filter(|id| focusable.contains(id));
        self.set_focus(restored.or_else(|| focusable.first().copied()));
    }

    fn focus_next(&mut self) -> bool {
        let focusable = self.focusable_ids();
        if focusable.is_empty() {
            return false;
        }
        let next_index = self
            .focused
            .and_then(|focused| focusable.iter().position(|id| *id == focused))
            .map_or(0, |index| (index + 1) % focusable.len());
        self.set_focus(Some(focusable[next_index]));
        true
    }

    fn event_ctx(&self) -> EventCtx {
        EventCtx::for_window(self.window_id)
    }
}
