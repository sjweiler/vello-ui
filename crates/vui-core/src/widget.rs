use crate::{
    BoxConstraints, Event, EventCtx, PaintList, PaintOp, Point, Rect, SemanticNode, Size,
    TextLayout, TextSystem, Theme, UiCommand, WidgetId,
};
use std::any::Any;

pub trait WidgetAny {
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<T: Any> WidgetAny for T {
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct WidgetKey(String);

impl WidgetKey {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for WidgetKey {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for WidgetKey {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

pub struct LayoutCtx<'a> {
    text: &'a mut dyn TextSystem,
    theme: &'a Theme,
}

impl<'a> LayoutCtx<'a> {
    pub(crate) fn new(text: &'a mut dyn TextSystem, theme: &'a Theme) -> Self {
        Self { text, theme }
    }

    pub fn layout_text(
        &mut self,
        text: &str,
        font_size: f32,
        max_width: Option<f32>,
    ) -> TextLayout {
        self.text.layout(text, font_size, max_width)
    }

    #[must_use]
    pub const fn theme(&self) -> &Theme {
        self.theme
    }

    pub fn with_theme<R>(
        &mut self,
        theme: &Theme,
        operation: impl FnOnce(&mut LayoutCtx<'_>) -> R,
    ) -> R {
        let mut nested = LayoutCtx {
            text: self.text,
            theme,
        };
        operation(&mut nested)
    }
}

pub struct PaintCtx<'a> {
    list: &'a mut PaintList,
    theme: &'a Theme,
}

impl<'a> PaintCtx<'a> {
    pub(crate) const fn new(list: &'a mut PaintList, theme: &'a Theme) -> Self {
        Self { list, theme }
    }

    pub fn push(&mut self, operation: PaintOp) {
        self.list.push(operation);
    }

    #[must_use]
    pub const fn theme(&self) -> &Theme {
        self.theme
    }

    pub fn with_theme<R>(
        &mut self,
        theme: &Theme,
        operation: impl FnOnce(&mut PaintCtx<'_>) -> R,
    ) -> R {
        let mut nested = PaintCtx {
            list: self.list,
            theme,
        };
        operation(&mut nested)
    }
}

pub struct SemanticsCtx<'a> {
    nodes: &'a mut Vec<SemanticNode>,
}

impl<'a> SemanticsCtx<'a> {
    pub(crate) const fn new(nodes: &'a mut Vec<SemanticNode>) -> Self {
        Self { nodes }
    }

    pub fn push(&mut self, node: SemanticNode) {
        self.nodes.push(node);
    }
}

pub trait Widget: WidgetAny {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size;
    fn set_origin(&mut self, origin: Point);
    fn bounds(&self) -> Rect;
    fn event(&mut self, ctx: &mut EventCtx, event: &Event);
    fn widget_id(&self) -> Option<WidgetId> {
        None
    }
    fn event_target(&mut self, ctx: &mut EventCtx, event: &Event, target: WidgetId) -> bool {
        if self.widget_id() == Some(target) {
            self.event(ctx, event);
            true
        } else {
            false
        }
    }
    /// Applies a deferred tree command.
    ///
    /// # Errors
    ///
    /// Returns the unchanged command when this widget has no matching command
    /// target, allowing an ancestor or sibling host to try it.
    fn apply_command(&mut self, command: UiCommand) -> Result<(), UiCommand> {
        Err(command)
    }
    fn widget_key(&self) -> Option<&WidgetKey> {
        None
    }
    fn reconcile(&mut self, _next: &mut dyn Widget) {}
    fn paint(&self, ctx: &mut PaintCtx<'_>);
    fn semantics(&self, ctx: &mut SemanticsCtx<'_>);
}
