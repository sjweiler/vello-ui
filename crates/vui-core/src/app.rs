use crate::{
    BoxConstraints, Event, EventCtx, LayoutCtx, PaintCtx, Point, Rect, SemanticsCtx, Size,
    UiCommand, Widget, WidgetId,
};
use std::{cell::RefCell, rc::Rc};

pub struct AppSender<Message> {
    queue: Rc<RefCell<Vec<Message>>>,
}

impl<Message> Clone for AppSender<Message> {
    fn clone(&self) -> Self {
        Self {
            queue: self.queue.clone(),
        }
    }
}

impl<Message> AppSender<Message> {
    pub fn emit(&self, message: Message) {
        self.queue.borrow_mut().push(message);
    }

    pub fn send(&self, message: Message, ctx: &mut EventCtx) {
        self.emit(message);
        ctx.request_layout();
    }
}

type Update<Model, Message> = dyn FnMut(&mut Model, Message);
type View<Model, Message> = dyn Fn(&Model, AppSender<Message>) -> Box<dyn Widget>;

/// Owns typed application state and rebuilds its retained view after messages.
pub struct App<Model, Message> {
    model: Model,
    update: Box<Update<Model, Message>>,
    view: Box<View<Model, Message>>,
    sender: AppSender<Message>,
    child: Box<dyn Widget>,
}

impl<Model: 'static, Message: 'static> App<Model, Message> {
    #[must_use]
    pub fn new<Child: Widget + 'static>(
        model: Model,
        update: impl FnMut(&mut Model, Message) + 'static,
        view: impl Fn(&Model, AppSender<Message>) -> Child + 'static,
    ) -> Self {
        let queue = Rc::new(RefCell::new(Vec::new()));
        let sender = AppSender { queue };
        let boxed_view: Box<View<Model, Message>> =
            Box::new(move |model, sender| Box::new(view(model, sender)));
        let child = boxed_view(&model, sender.clone());
        Self {
            model,
            update: Box::new(update),
            view: boxed_view,
            sender,
            child,
        }
    }

    #[must_use]
    pub const fn model(&self) -> &Model {
        &self.model
    }

    fn update_view(&mut self, ctx: &mut EventCtx) {
        let messages = self.sender.queue.borrow_mut().drain(..).collect::<Vec<_>>();
        if messages.is_empty() {
            return;
        }
        for message in messages {
            (self.update)(&mut self.model, message);
        }
        let mut next = (self.view)(&self.model, self.sender.clone());
        self.child.reconcile(next.as_mut());
        self.child = next;
        ctx.request_layout();
    }
}

impl<Model: 'static, Message: 'static> Widget for App<Model, Message> {
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
        self.update_view(ctx);
    }

    fn event_target(&mut self, ctx: &mut EventCtx, event: &Event, target: WidgetId) -> bool {
        let handled = self.child.event_target(ctx, event, target);
        self.update_view(ctx);
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
