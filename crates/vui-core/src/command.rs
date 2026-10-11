use crate::{
    BoxConstraints, Button, ButtonVariant, Event, EventCtx, IconButton, IconName, Key, LayoutCtx,
    Modifiers, PaintCtx, Point, Rect, SemanticsCtx, Size, UiCommand, Widget, WidgetId,
};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CommandId(String);

impl CommandId {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl From<&str> for CommandId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}
impl From<String> for CommandId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum CommandScope {
    #[default]
    Global,
    Window,
    Panel,
    Focused,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub key: Key,
    pub primary: bool,
    pub shift: bool,
    pub alt: bool,
}

impl KeyChord {
    #[must_use]
    pub fn new(key: Key) -> Self {
        Self {
            key: normalized_key(key),
            primary: false,
            shift: false,
            alt: false,
        }
    }
    #[must_use]
    pub fn primary(key: Key) -> Self {
        Self {
            primary: true,
            ..Self::new(key)
        }
    }
    #[must_use]
    pub const fn shift(mut self) -> Self {
        self.shift = true;
        self
    }
    #[must_use]
    pub const fn alt(mut self) -> Self {
        self.alt = true;
        self
    }
    #[must_use]
    pub fn matches(&self, modifiers: Modifiers, key: &Key) -> bool {
        self.primary == modifiers.command()
            && self.shift == modifiers.shift()
            && self.alt == modifiers.alt()
            && self.key == normalized_key(key.clone())
    }
    #[must_use]
    pub fn label(&self) -> String {
        let mut parts = Vec::new();
        if self.primary {
            parts.push("Ctrl/Cmd".to_owned());
        }
        if self.alt {
            parts.push("Alt".to_owned());
        }
        if self.shift {
            parts.push("Shift".to_owned());
        }
        parts.push(key_label(&self.key));
        parts.join("+")
    }
}

fn normalized_key(key: Key) -> Key {
    match key {
        Key::Character(value) => Key::Character(value.to_lowercase()),
        key => key,
    }
}

fn key_label(key: &Key) -> String {
    match key {
        Key::Enter => "Enter".to_owned(),
        Key::Space => "Space".to_owned(),
        Key::Tab => "Tab".to_owned(),
        Key::Escape => "Esc".to_owned(),
        Key::Backspace => "Backspace".to_owned(),
        Key::Delete => "Delete".to_owned(),
        Key::ArrowLeft => "Left".to_owned(),
        Key::ArrowRight => "Right".to_owned(),
        Key::ArrowUp => "Up".to_owned(),
        Key::ArrowDown => "Down".to_owned(),
        Key::Home => "Home".to_owned(),
        Key::End => "End".to_owned(),
        Key::PageUp => "PageUp".to_owned(),
        Key::PageDown => "PageDown".to_owned(),
        Key::Character(value) => value.to_uppercase(),
        Key::Other => "Key".to_owned(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandDescriptor {
    pub id: CommandId,
    pub label: String,
    pub description: String,
    pub keywords: String,
    pub icon: Option<IconName>,
    pub shortcut: Option<KeyChord>,
    pub scope: CommandScope,
    pub enabled: bool,
    pub checked: Option<bool>,
    pub selected: bool,
}

type CommandAction = dyn FnMut();

pub struct Command {
    descriptor: CommandDescriptor,
    action: Box<CommandAction>,
}

impl Command {
    #[must_use]
    pub fn new(label: impl Into<String>, action: impl FnMut() + 'static) -> Self {
        let label = label.into();
        let id = label
            .to_lowercase()
            .chars()
            .map(|character| {
                if character.is_alphanumeric() {
                    character
                } else {
                    '.'
                }
            })
            .collect::<String>();
        Self::action(id, label, action)
    }
    #[must_use]
    pub fn action(
        id: impl Into<CommandId>,
        label: impl Into<String>,
        action: impl FnMut() + 'static,
    ) -> Self {
        let label = label.into();
        Self {
            descriptor: CommandDescriptor {
                id: id.into(),
                keywords: label.to_lowercase(),
                label,
                description: String::new(),
                icon: None,
                shortcut: None,
                scope: CommandScope::Global,
                enabled: true,
                checked: None,
                selected: false,
            },
            action: Box::new(action),
        }
    }
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.descriptor.description = description.into();
        self
    }
    #[must_use]
    pub fn keywords(mut self, keywords: impl Into<String>) -> Self {
        self.descriptor.keywords.push(' ');
        self.descriptor
            .keywords
            .push_str(&keywords.into().to_lowercase());
        self
    }
    #[must_use]
    pub const fn icon(mut self, icon: IconName) -> Self {
        self.descriptor.icon = Some(icon);
        self
    }
    #[must_use]
    pub fn shortcut(mut self, shortcut: KeyChord) -> Self {
        self.descriptor.shortcut = Some(shortcut);
        self
    }
    #[must_use]
    pub const fn scope(mut self, scope: CommandScope) -> Self {
        self.descriptor.scope = scope;
        self
    }
    #[must_use]
    pub const fn enabled(mut self, enabled: bool) -> Self {
        self.descriptor.enabled = enabled;
        self
    }
    #[must_use]
    pub const fn checked(mut self, checked: bool) -> Self {
        self.descriptor.checked = Some(checked);
        self
    }
    #[must_use]
    pub const fn selected(mut self, selected: bool) -> Self {
        self.descriptor.selected = selected;
        self
    }
    #[must_use]
    pub const fn descriptor(&self) -> &CommandDescriptor {
        &self.descriptor
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShortcutConflict {
    pub chord: KeyChord,
    pub commands: Vec<CommandId>,
}

pub struct CommandRegistry {
    commands: Vec<Command>,
}
impl CommandRegistry {
    #[must_use]
    pub fn new(commands: impl IntoIterator<Item = Command>) -> Self {
        Self {
            commands: commands.into_iter().collect(),
        }
    }
    #[must_use]
    pub fn descriptors(&self) -> Vec<CommandDescriptor> {
        self.commands
            .iter()
            .map(|command| command.descriptor.clone())
            .collect()
    }
    #[must_use]
    pub fn descriptor(&self, id: &CommandId) -> Option<CommandDescriptor> {
        self.commands
            .iter()
            .find(|command| command.descriptor.id == *id)
            .map(|command| command.descriptor.clone())
    }
    #[must_use]
    pub fn shortcut_conflicts(&self) -> Vec<ShortcutConflict> {
        let mut grouped: HashMap<KeyChord, Vec<CommandId>> = HashMap::new();
        for command in &self.commands {
            if let Some(chord) = &command.descriptor.shortcut {
                grouped
                    .entry(chord.clone())
                    .or_default()
                    .push(command.descriptor.id.clone());
            }
        }
        grouped
            .into_iter()
            .filter_map(|(chord, commands)| {
                (commands.len() > 1).then_some(ShortcutConflict { chord, commands })
            })
            .collect()
    }
    fn invoke(&mut self, id: &CommandId, active_scope: CommandScope) -> bool {
        let Some(command) = self.commands.iter_mut().find(|command| {
            command.descriptor.id == *id
                && command.descriptor.enabled
                && command.descriptor.scope <= active_scope
        }) else {
            return false;
        };
        (command.action)();
        true
    }
    fn shortcut(
        &self,
        modifiers: Modifiers,
        key: &Key,
        active_scope: CommandScope,
    ) -> Option<CommandId> {
        self.commands
            .iter()
            .filter(|command| {
                command.descriptor.enabled
                    && command.descriptor.scope <= active_scope
                    && command
                        .descriptor
                        .shortcut
                        .as_ref()
                        .is_some_and(|chord| chord.matches(modifiers, key))
            })
            .max_by_key(|command| command.descriptor.scope)
            .map(|command| command.descriptor.id.clone())
    }
}

pub struct CommandHost {
    registry: CommandRegistry,
    child: Box<dyn Widget>,
    modifiers: Modifiers,
    active_scope: CommandScope,
}
impl CommandHost {
    #[must_use]
    pub fn new(commands: impl IntoIterator<Item = Command>, child: impl Widget + 'static) -> Self {
        Self {
            registry: CommandRegistry::new(commands),
            child: Box::new(child),
            modifiers: Modifiers::default(),
            active_scope: CommandScope::Focused,
        }
    }
    #[must_use]
    pub const fn active_scope(mut self, scope: CommandScope) -> Self {
        self.active_scope = scope;
        self
    }
    #[must_use]
    pub fn descriptors(&self) -> Vec<CommandDescriptor> {
        self.registry.descriptors()
    }
    #[must_use]
    pub fn shortcut_conflicts(&self) -> Vec<ShortcutConflict> {
        self.registry.shortcut_conflicts()
    }
}
impl Widget for CommandHost {
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
        if let Event::ModifiersChanged(modifiers) = event {
            self.modifiers = *modifiers;
        }
        if let Event::CommandInvoked(id) = event
            && self.registry.invoke(id, self.active_scope)
        {
            ctx.set_handled();
            ctx.request_layout();
            return;
        }
        if let Event::KeyDown(key) = event
            && let Some(id) = self
                .registry
                .shortcut(self.modifiers, key, self.active_scope)
            && self.registry.invoke(&id, self.active_scope)
        {
            ctx.set_handled();
            ctx.request_layout();
            return;
        }
        self.child.event(ctx, event);
    }
    fn event_target(&mut self, ctx: &mut EventCtx, event: &Event, target: WidgetId) -> bool {
        self.child.event_target(ctx, event, target)
    }
    fn apply_command(&mut self, command: UiCommand) -> Result<(), UiCommand> {
        match command {
            UiCommand::InvokeCommand(id) if self.registry.invoke(&id, self.active_scope) => Ok(()),
            UiCommand::InvokeCommand(id) => Err(UiCommand::InvokeCommand(id)),
            command => self.child.apply_command(command),
        }
    }
    fn reconcile(&mut self, next: &mut dyn Widget) {
        if let Some(next) = next.as_any_mut().downcast_mut::<Self>() {
            next.modifiers = self.modifiers;
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

pub struct CommandButton {
    button: Button,
}
impl CommandButton {
    #[must_use]
    pub fn new(command: CommandDescriptor) -> Self {
        let id = command.id.clone();
        Self {
            button: Button::action(command.label, move |_, ctx| ctx.invoke_command(id.clone()))
                .disabled(!command.enabled)
                .selected(command.selected),
        }
    }
    #[must_use]
    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.button = self.button.variant(variant);
        self
    }
}
impl Widget for CommandButton {
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

pub struct CommandIconButton {
    button: IconButton,
}
impl CommandIconButton {
    #[must_use]
    pub fn new(command: CommandDescriptor) -> Self {
        let id = command.id.clone();
        let tooltip = command.shortcut.as_ref().map_or_else(
            || command.description.clone(),
            |shortcut| format!("{} ({})", command.description, shortcut.label()),
        );
        let button = IconButton::action(
            command.icon.unwrap_or(IconName::Command),
            command.label,
            move |_, ctx| ctx.invoke_command(id.clone()),
        )
        .tooltip(tooltip)
        .disabled(!command.enabled);
        Self { button }
    }
}
impl Widget for CommandIconButton {
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
