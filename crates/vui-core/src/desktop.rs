use crate::{
    BoxConstraints, Event, EventCtx, LayoutCtx, PaintCtx, Point, Rect, SemanticsCtx, Size,
    UiCommand, Widget, WidgetId, WindowCommand,
};
use std::{cell::RefCell, rc::Rc};

type DesktopUpdate<Model, Message> = dyn FnMut(&mut Model, Message);

struct DesktopState<Model, Message> {
    model: Model,
    update: Box<DesktopUpdate<Model, Message>>,
    revision: u64,
}

pub struct DesktopSender<Message> {
    queue: Rc<RefCell<Vec<Message>>>,
}

impl<Message> Clone for DesktopSender<Message> {
    fn clone(&self) -> Self {
        Self {
            queue: self.queue.clone(),
        }
    }
}

impl<Message> DesktopSender<Message> {
    pub fn emit(&self, message: Message) {
        self.queue.borrow_mut().push(message);
    }

    pub fn send(&self, message: Message, ctx: &mut EventCtx) {
        self.emit(message);
        ctx.request_layout();
    }
}

/// Shared typed model and reducer used by independent window runtimes.
pub struct DesktopApp<Model, Message> {
    state: Rc<RefCell<DesktopState<Model, Message>>>,
    sender: DesktopSender<Message>,
}

impl<Model, Message> Clone for DesktopApp<Model, Message> {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
            sender: self.sender.clone(),
        }
    }
}

impl<Model: 'static, Message: 'static> DesktopApp<Model, Message> {
    #[must_use]
    pub fn new(model: Model, update: impl FnMut(&mut Model, Message) + 'static) -> Self {
        Self {
            state: Rc::new(RefCell::new(DesktopState {
                model,
                update: Box::new(update),
                revision: 0,
            })),
            sender: DesktopSender {
                queue: Rc::new(RefCell::new(Vec::new())),
            },
        }
    }

    #[must_use]
    pub fn sender(&self) -> DesktopSender<Message> {
        self.sender.clone()
    }

    #[must_use]
    pub fn window<Child: Widget + 'static>(
        &self,
        view: impl Fn(&Model, DesktopSender<Message>) -> Child + 'static,
    ) -> DesktopWindow<Model, Message> {
        let boxed_view: Box<WindowView<Model, Message>> =
            Box::new(move |model, sender| Box::new(view(model, sender)));
        let state = self.state.borrow();
        let child = boxed_view(&state.model, self.sender.clone());
        DesktopWindow {
            state: self.state.clone(),
            sender: self.sender.clone(),
            view: boxed_view,
            child,
            revision: state.revision,
        }
    }
}

type WindowView<Model, Message> = dyn Fn(&Model, DesktopSender<Message>) -> Box<dyn Widget>;

pub struct DesktopWindow<Model, Message> {
    state: Rc<RefCell<DesktopState<Model, Message>>>,
    sender: DesktopSender<Message>,
    view: Box<WindowView<Model, Message>>,
    child: Box<dyn Widget>,
    revision: u64,
}

impl<Model: 'static, Message: 'static> DesktopWindow<Model, Message> {
    fn rebuild(&mut self, ctx: &mut EventCtx) {
        let state = self.state.borrow();
        if state.revision == self.revision {
            return;
        }
        let mut next = (self.view)(&state.model, self.sender.clone());
        self.child.reconcile(next.as_mut());
        self.child = next;
        self.revision = state.revision;
        ctx.request_layout();
    }

    fn reduce_messages(&mut self, ctx: &mut EventCtx) {
        let messages = self.sender.queue.borrow_mut().drain(..).collect::<Vec<_>>();
        if messages.is_empty() {
            self.rebuild(ctx);
            return;
        }
        {
            let mut state = self.state.borrow_mut();
            let DesktopState {
                model,
                update,
                revision,
            } = &mut *state;
            for message in messages {
                update(model, message);
            }
            *revision = revision.wrapping_add(1);
        }
        self.rebuild(ctx);
        ctx.window_command(WindowCommand::RefreshAll);
    }
}

impl<Model: 'static, Message: 'static> Widget for DesktopWindow<Model, Message> {
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
        if matches!(event, Event::AppStateChanged) {
            self.rebuild(ctx);
        } else {
            self.child.event(ctx, event);
            self.reduce_messages(ctx);
        }
    }

    fn event_target(&mut self, ctx: &mut EventCtx, event: &Event, target: WidgetId) -> bool {
        let handled = self.child.event_target(ctx, event, target);
        self.reduce_messages(ctx);
        handled
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
