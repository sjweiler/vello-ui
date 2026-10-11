//! Headless harness for deterministic event, layout, paint, and semantics tests.

use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use vui_core::{
    ClipboardRequest, Event, NativeMenu, PlatformCommand, PlatformEvent, PlatformRequest, Size,
    SystemPreferences, Theme, TrayDescriptor, UiFrame, UiRuntime, Widget, WindowCommand,
    WindowDescriptor, WindowId,
};

pub struct TestHarness {
    runtime: UiRuntime,
    now: Duration,
}

/// Deterministic model of the native desktop application lifecycle.
pub struct DesktopLifecycleHarness {
    window: WindowId,
    application_id: String,
    arguments: Vec<String>,
    single_instance: bool,
    tray: Option<TrayDescriptor>,
    menu_bar: Option<NativeMenu>,
    close_to_tray: bool,
    state: HarnessLifecycleState,
    primary: Option<bool>,
    open_files: Vec<PathBuf>,
    open_urls: Vec<String>,
    notification_actions: Vec<(String, String)>,
    preferences: SystemPreferences,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum HarnessLifecycleState {
    #[default]
    Ready,
    Waiting,
    Running,
    Hidden,
    Exited,
}

impl DesktopLifecycleHarness {
    #[must_use]
    pub fn new(window: WindowId, application_id: impl Into<String>) -> Self {
        Self {
            window,
            application_id: application_id.into(),
            arguments: Vec::new(),
            single_instance: false,
            tray: None,
            menu_bar: None,
            close_to_tray: false,
            state: HarnessLifecycleState::Ready,
            primary: None,
            open_files: Vec::new(),
            open_urls: Vec::new(),
            notification_actions: Vec::new(),
            preferences: SystemPreferences::default(),
        }
    }

    #[must_use]
    pub fn arguments(mut self, arguments: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.arguments = arguments.into_iter().map(Into::into).collect();
        self
    }

    #[must_use]
    pub const fn single_instance(mut self, enabled: bool) -> Self {
        self.single_instance = enabled;
        self
    }

    #[must_use]
    pub fn tray(mut self, tray: TrayDescriptor) -> Self {
        self.tray = Some(tray);
        self
    }

    #[must_use]
    pub fn menu_bar(mut self, menu: NativeMenu) -> Self {
        self.menu_bar = Some(menu);
        self
    }

    #[must_use]
    pub const fn close_to_tray(mut self, enabled: bool) -> Self {
        self.close_to_tray = enabled;
        self
    }

    pub fn start(&mut self) -> Vec<PlatformRequest> {
        if self.state != HarnessLifecycleState::Ready {
            return Vec::new();
        }
        if self.single_instance {
            self.state = HarnessLifecycleState::Waiting;
            vec![PlatformRequest::RegisterSingleInstance {
                application_id: self.application_id.clone(),
                arguments: self.arguments.clone(),
            }]
        } else {
            self.primary = Some(true);
            self.state = HarnessLifecycleState::Running;
            let arguments = self.arguments.clone();
            self.record_arguments(&arguments);
            self.install_requests()
        }
    }

    pub fn platform_event(&mut self, event: PlatformEvent) -> Vec<PlatformRequest> {
        match event {
            PlatformEvent::SingleInstanceStatus { primary: false } => {
                self.primary = Some(false);
                self.state = HarnessLifecycleState::Exited;
                Vec::new()
            }
            PlatformEvent::SingleInstanceStatus { primary: true } => {
                self.primary = Some(true);
                self.state = HarnessLifecycleState::Running;
                let arguments = self.arguments.clone();
                self.record_arguments(&arguments);
                self.install_requests()
            }
            PlatformEvent::TrayActivated(_) => {
                self.state = HarnessLifecycleState::Running;
                Vec::new()
            }
            PlatformEvent::SecondInstance(arguments) => {
                self.state = HarnessLifecycleState::Running;
                self.record_arguments(&arguments);
                Vec::new()
            }
            PlatformEvent::OpenFiles(paths) => {
                self.open_files.extend(paths);
                Vec::new()
            }
            PlatformEvent::OpenUrls(urls) => {
                self.open_urls.extend(urls);
                Vec::new()
            }
            PlatformEvent::NotificationAction {
                notification,
                action,
            } => {
                self.notification_actions.push((notification, action));
                Vec::new()
            }
            PlatformEvent::PreferencesChanged(preferences) => {
                self.preferences = preferences;
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    pub fn request_close(&mut self) {
        if self.close_to_tray {
            self.state = HarnessLifecycleState::Hidden;
        } else {
            self.state = HarnessLifecycleState::Exited;
        }
    }

    pub fn quit(&mut self) -> Vec<PlatformRequest> {
        self.state = HarnessLifecycleState::Exited;
        self.tray.as_ref().map_or_else(Vec::new, |tray| {
            vec![PlatformRequest::RemoveTray(tray.id.clone())]
        })
    }

    #[must_use]
    pub const fn is_visible(&self) -> bool {
        matches!(self.state, HarnessLifecycleState::Running)
    }

    #[must_use]
    pub const fn has_exited(&self) -> bool {
        matches!(self.state, HarnessLifecycleState::Exited)
    }

    #[must_use]
    pub const fn primary_status(&self) -> Option<bool> {
        self.primary
    }

    #[must_use]
    pub fn open_files(&self) -> &[PathBuf] {
        &self.open_files
    }

    #[must_use]
    pub fn open_urls(&self) -> &[String] {
        &self.open_urls
    }

    #[must_use]
    pub fn notification_actions(&self) -> &[(String, String)] {
        &self.notification_actions
    }

    #[must_use]
    pub const fn preferences(&self) -> &SystemPreferences {
        &self.preferences
    }

    fn install_requests(&self) -> Vec<PlatformRequest> {
        let mut requests = Vec::new();
        if let Some(menu) = &self.menu_bar {
            requests.push(PlatformRequest::SetMenuBar {
                window: self.window,
                menu: menu.clone(),
            });
        }
        if let Some(tray) = &self.tray {
            requests.push(PlatformRequest::SetTray(tray.clone()));
        }
        requests
    }

    fn record_arguments(&mut self, arguments: &[String]) {
        let (files, urls) = classify_arguments(arguments);
        self.open_files.extend(files);
        self.open_urls.extend(urls);
    }
}

fn classify_arguments(arguments: &[String]) -> (Vec<PathBuf>, Vec<String>) {
    let mut files = Vec::new();
    let mut urls = Vec::new();
    for argument in arguments.iter().skip(1) {
        if argument.contains("://") {
            urls.push(argument.clone());
        } else if !argument.starts_with('-') {
            files.push(PathBuf::from(argument));
        }
    }
    (files, urls)
}

/// Deterministic host for applications that own more than one UI runtime.
///
/// Every window keeps independent focus, overlays, tasks, animation time, and
/// theme state. Non-window platform requests are retained for assertions or a
/// caller-provided fake service.
pub struct MultiWindowHarness {
    windows: BTreeMap<WindowId, HarnessWindow>,
    platform_requests: Vec<(WindowId, PlatformCommand)>,
    quit_requested: bool,
}

struct HarnessWindow {
    descriptor: WindowDescriptor,
    runtime: UiRuntime,
    now: Duration,
    state: u8,
}

impl MultiWindowHarness {
    #[must_use]
    pub fn new(descriptor: WindowDescriptor, root: impl Widget + 'static) -> (Self, WindowId) {
        let id = WindowId::next();
        let window = HarnessWindow::new(id, descriptor, Box::new(root));
        let mut windows = BTreeMap::new();
        windows.insert(id, window);
        (
            Self {
                windows,
                platform_requests: Vec::new(),
                quit_requested: false,
            },
            id,
        )
    }

    #[must_use]
    pub fn contains(&self, id: WindowId) -> bool {
        self.windows.contains_key(&id)
    }

    #[must_use]
    pub fn window_count(&self) -> usize {
        self.windows.len()
    }

    #[must_use]
    pub fn window_ids(&self) -> Vec<WindowId> {
        self.windows.keys().copied().collect()
    }

    #[must_use]
    pub fn descriptor(&self, id: WindowId) -> Option<&WindowDescriptor> {
        self.windows.get(&id).map(|window| &window.descriptor)
    }

    pub fn event(&mut self, id: WindowId, event: &Event) -> bool {
        let handled = self
            .windows
            .get_mut(&id)
            .is_some_and(|window| window.runtime.handle_event(event));
        self.service_window_commands(id);
        handled
    }

    pub fn frame(&mut self, id: WindowId) -> Option<UiFrame> {
        let window = self.windows.get_mut(&id)?;
        window.runtime.request_paint();
        window.runtime.render_frame()
    }

    pub fn open_window(
        &mut self,
        descriptor: WindowDescriptor,
        root: impl Widget + 'static,
    ) -> WindowId {
        let id = WindowId::next();
        self.windows
            .insert(id, HarnessWindow::new(id, descriptor, Box::new(root)));
        id
    }

    pub fn advance_by(&mut self, id: WindowId, duration: Duration) -> bool {
        let Some(window) = self.windows.get_mut(&id) else {
            return false;
        };
        window.now = window.now.saturating_add(duration);
        let advanced = window.runtime.advance_time(window.now);
        self.service_window_commands(id);
        advanced
    }

    pub fn run_pending_tasks(&mut self, id: WindowId) -> usize {
        let Some(window) = self.windows.get_mut(&id) else {
            return 0;
        };
        let requests = window.runtime.take_task_requests();
        let count = requests.len();
        for request in requests {
            let updates = Arc::new(Mutex::new(Vec::new()));
            let reported = updates.clone();
            request.run(move |update| {
                reported
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(update);
            });
            let pending = {
                let mut updates = updates
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                std::mem::take(&mut *updates)
            };
            for update in pending {
                window.runtime.handle_task_update(update);
            }
        }
        self.service_window_commands(id);
        count
    }

    pub fn take_platform_requests(&mut self) -> Vec<(WindowId, PlatformCommand)> {
        std::mem::take(&mut self.platform_requests)
    }

    pub fn request_close(&mut self, id: WindowId) -> bool {
        let cancelled = self
            .windows
            .get_mut(&id)
            .is_some_and(|window| window.runtime.handle_event(&Event::WindowCloseRequested));
        if !cancelled {
            self.remove_window(id);
        }
        cancelled
    }

    #[must_use]
    pub const fn quit_requested(&self) -> bool {
        self.quit_requested
    }

    #[must_use]
    pub fn is_focused(&self, id: WindowId) -> bool {
        self.has_state(id, HarnessWindow::FOCUSED)
    }

    #[must_use]
    pub fn is_minimized(&self, id: WindowId) -> bool {
        self.has_state(id, HarnessWindow::MINIMIZED)
    }

    #[must_use]
    pub fn is_maximized(&self, id: WindowId) -> bool {
        self.has_state(id, HarnessWindow::MAXIMIZED)
    }

    #[must_use]
    pub fn is_fullscreen(&self, id: WindowId) -> bool {
        self.has_state(id, HarnessWindow::FULLSCREEN)
    }

    #[must_use]
    pub fn attention_requested(&self, id: WindowId) -> bool {
        self.has_state(id, HarnessWindow::ATTENTION)
    }

    fn service_window_commands(&mut self, source: WindowId) {
        let commands = self
            .windows
            .get_mut(&source)
            .map_or_else(Vec::new, |window| window.runtime.take_platform_commands());
        for command in commands {
            match command {
                PlatformCommand::Window(WindowCommand::Open {
                    id,
                    descriptor,
                    root,
                }) => {
                    if descriptor.is_singleton()
                        && let Some(key) = descriptor.key.as_deref()
                        && let Some(existing) = self.windows.iter().find_map(|(id, window)| {
                            (window.descriptor.key.as_deref() == Some(key)).then_some(*id)
                        })
                    {
                        for window in self.windows.values_mut() {
                            window.set_state(HarnessWindow::FOCUSED, false);
                        }
                        if let Some(window) = self.windows.get_mut(&existing) {
                            window.set_state(HarnessWindow::FOCUSED, true);
                        }
                        continue;
                    }
                    self.windows
                        .insert(id, HarnessWindow::new(id, descriptor, root));
                }
                PlatformCommand::Window(WindowCommand::Close(id)) => {
                    self.remove_window(id);
                }
                PlatformCommand::Window(WindowCommand::Focus(id)) => {
                    for window in self.windows.values_mut() {
                        window.set_state(HarnessWindow::FOCUSED, false);
                    }
                    if let Some(window) = self.windows.get_mut(&id) {
                        window.set_state(HarnessWindow::FOCUSED, true);
                    }
                }
                PlatformCommand::Window(WindowCommand::Minimize(id, value)) => {
                    if let Some(window) = self.windows.get_mut(&id) {
                        window.set_state(HarnessWindow::MINIMIZED, value);
                    }
                }
                PlatformCommand::Window(WindowCommand::Maximize(id, value)) => {
                    if let Some(window) = self.windows.get_mut(&id) {
                        window.set_state(HarnessWindow::MAXIMIZED, value);
                    }
                }
                PlatformCommand::Window(WindowCommand::Fullscreen(id, value)) => {
                    if let Some(window) = self.windows.get_mut(&id) {
                        window.set_state(HarnessWindow::FULLSCREEN, value);
                    }
                }
                PlatformCommand::Window(WindowCommand::RequestAttention(id)) => {
                    if let Some(window) = self.windows.get_mut(&id) {
                        window.set_state(HarnessWindow::ATTENTION, true);
                    }
                }
                PlatformCommand::Window(WindowCommand::RefreshAll) => {
                    for window in self.windows.values_mut() {
                        window.runtime.handle_event(&Event::AppStateChanged);
                    }
                }
                PlatformCommand::Window(WindowCommand::QuitApplication) => {
                    let cancelled = self.windows.values_mut().any(|window| {
                        window
                            .runtime
                            .handle_event(&Event::ApplicationExitRequested)
                    });
                    if !cancelled {
                        self.quit_requested = true;
                    }
                }
                request => self.platform_requests.push((source, request)),
            }
        }
    }

    fn has_state(&self, id: WindowId, state: u8) -> bool {
        self.windows
            .get(&id)
            .is_some_and(|window| window.state & state != 0)
    }

    fn remove_window(&mut self, id: WindowId) {
        let children = self
            .windows
            .iter()
            .filter_map(|(child_id, window)| {
                (window.descriptor.parent == Some(id)).then_some(*child_id)
            })
            .collect::<Vec<_>>();
        for child in children {
            self.remove_window(child);
        }
        self.windows.remove(&id);
    }
}

impl HarnessWindow {
    const FOCUSED: u8 = 1;
    const MINIMIZED: u8 = 1 << 1;
    const MAXIMIZED: u8 = 1 << 2;
    const FULLSCREEN: u8 = 1 << 3;
    const ATTENTION: u8 = 1 << 4;

    fn new(id: WindowId, descriptor: WindowDescriptor, root: Box<dyn Widget>) -> Self {
        let viewport = descriptor.size;
        let mut runtime = UiRuntime::from_boxed(root).with_window_id(id);
        runtime.set_viewport(viewport);
        runtime.set_reduced_motion(true);
        Self {
            descriptor,
            runtime,
            now: Duration::ZERO,
            state: 0,
        }
    }

    fn set_state(&mut self, state: u8, enabled: bool) {
        if enabled {
            self.state |= state;
        } else {
            self.state &= !state;
        }
    }
}

impl TestHarness {
    #[must_use]
    pub fn new(root: impl Widget + 'static, viewport: Size) -> Self {
        let mut runtime = UiRuntime::new(root);
        runtime.set_viewport(viewport);
        runtime.set_reduced_motion(true);
        Self {
            runtime,
            now: Duration::ZERO,
        }
    }

    #[must_use]
    pub fn with_text_system(
        root: impl Widget + 'static,
        viewport: Size,
        text: impl vui_core::TextSystem + 'static,
    ) -> Self {
        let mut runtime = UiRuntime::new(root).with_text_system(text);
        runtime.set_viewport(viewport);
        runtime.set_reduced_motion(true);
        Self {
            runtime,
            now: Duration::ZERO,
        }
    }

    pub fn event(&mut self, event: &Event) -> bool {
        self.runtime.handle_event(event)
    }

    /// Produces a fresh paint and semantics frame.
    ///
    /// # Panics
    ///
    /// Panics when the harness was constructed with an empty viewport.
    pub fn frame(&mut self) -> UiFrame {
        self.runtime.request_paint();
        self.runtime
            .render_frame()
            .expect("test harness has a non-empty viewport")
    }

    pub fn take_clipboard_request(&mut self) -> Option<ClipboardRequest> {
        self.runtime.take_clipboard_request()
    }

    pub fn resize(&mut self, viewport: Size) {
        self.runtime.set_viewport(viewport);
    }

    pub fn set_theme(&mut self, theme: Theme) {
        self.runtime.set_theme(theme);
    }

    #[must_use]
    pub const fn drag_active(&self) -> bool {
        self.runtime.drag_active()
    }

    pub fn set_reduced_motion(&mut self, reduced_motion: bool) {
        self.runtime.set_reduced_motion(reduced_motion);
    }

    pub fn advance_by(&mut self, duration: Duration) -> bool {
        self.now = self.now.saturating_add(duration);
        self.runtime.advance_time(self.now)
    }

    #[must_use]
    pub const fn animation_pending(&self) -> bool {
        self.runtime.animation_pending()
    }

    pub fn run_pending_tasks(&mut self) -> usize {
        let requests = self.runtime.take_task_requests();
        let count = requests.len();
        for request in requests {
            let updates = Arc::new(Mutex::new(Vec::new()));
            let reported = updates.clone();
            request.run(move |update| {
                reported
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(update);
            });
            let pending = {
                let mut updates = updates
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                std::mem::take(&mut *updates)
            };
            for update in pending {
                self.runtime.handle_task_update(update);
            }
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::{cell::Cell, rc::Rc};
    use vui_core::{
        App, Button, ButtonVariant, Checkbox, CollectionDrop, Column, Command, CommandButton,
        CommandHost, CommandPalette, CommandRegistry, CommandScope, ConstrainedBox, ContextMenu,
        ControlState, CrossAxisAlignment, DesktopApp, Dialog, DropPosition, Flex, FlexChild,
        FlexWrap, Icon, IconButton, IconName, Insets, Key, KeyChord, Keyed, Label, ListItem,
        ListView, Menu, MenuButton, MenuItem, Modifier, Modifiers, NavigationItem, OverlayHost,
        OverlayPlacement, PaintOp, Point, PointerButton, ProgressBar, Rect, Resource,
        ResourceState, Responsive, Row, ScrollAxes, ScrollView, SemanticAction, SemanticRole,
        Sidebar, SizedBox, SortDirection, SplitPane, Stack, StatusBar, Surface, SurfaceTone, Tab,
        TabBar, TableAlignment, TableColumn, TableView, TaskId, TextArea, TextField, ThemeScope,
        Toolbar, TreeNode, TreeView, Widget, WidgetId, WidgetKey,
    };

    struct DelayedFrameWidget {
        frames: Rc<Cell<u32>>,
        bounds: Rect,
    }

    impl Widget for DelayedFrameWidget {
        fn layout(
            &mut self,
            _ctx: &mut vui_core::LayoutCtx<'_>,
            constraints: vui_core::BoxConstraints,
        ) -> Size {
            let size = constraints.constrain(Size::new(40.0, 40.0));
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

        fn event(&mut self, ctx: &mut vui_core::EventCtx, event: &Event) {
            match event {
                Event::PointerDown { .. } => {
                    ctx.request_animation_after(Duration::from_millis(100));
                    ctx.set_handled();
                }
                Event::AnimationFrame(_) => self.frames.set(self.frames.get() + 1),
                _ => {}
            }
        }

        fn paint(&self, _ctx: &mut vui_core::PaintCtx<'_>) {}

        fn semantics(&self, _ctx: &mut vui_core::SemanticsCtx<'_>) {}
    }

    struct TaskProbeWidget {
        id: WidgetId,
        resource: Resource<String>,
        task: Option<TaskId>,
        status: Rc<std::cell::RefCell<String>>,
        started: Arc<AtomicBool>,
        fail: bool,
        bounds: Rect,
    }

    impl TaskProbeWidget {
        fn new(
            status: Rc<std::cell::RefCell<String>>,
            started: Arc<AtomicBool>,
            fail: bool,
        ) -> Self {
            Self {
                id: WidgetId::next(),
                resource: Resource::default(),
                task: None,
                status,
                started,
                fail,
                bounds: Rect::default(),
            }
        }
    }

    impl Widget for TaskProbeWidget {
        fn layout(
            &mut self,
            _ctx: &mut vui_core::LayoutCtx<'_>,
            constraints: vui_core::BoxConstraints,
        ) -> Size {
            let size = constraints.constrain(Size::new(120.0, 40.0));
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

        fn event(&mut self, ctx: &mut vui_core::EventCtx, event: &Event) {
            match event {
                Event::PointerDown { .. } if self.task.is_none() => {
                    let started = self.started.clone();
                    let fail = self.fail;
                    let task = ctx.spawn_task(self.id, move |task| {
                        started.store(true, Ordering::Release);
                        task.report(Some(0.5), "halfway".to_owned());
                        if fail {
                            Err("offline".to_owned())
                        } else {
                            Ok("ready".to_owned())
                        }
                    });
                    self.task = Some(task);
                    self.resource.begin(task);
                    *self.status.borrow_mut() = "loading".to_owned();
                    ctx.set_handled();
                }
                Event::KeyDown(Key::Escape) => {
                    self.resource.cancel(ctx);
                    *self.status.borrow_mut() = "cancelled".to_owned();
                    self.task = None;
                }
                Event::TaskProgress { fraction, .. } => {
                    if self.resource.handle_event(event) {
                        *self.status.borrow_mut() =
                            format!("{}%", fraction.unwrap_or_default() * 100.0);
                    }
                }
                Event::TaskCompleted { .. }
                | Event::TaskFailed { .. }
                | Event::TaskCancelled { .. } => {
                    if self.resource.handle_event(event) {
                        let status = match self.resource.state() {
                            ResourceState::Ready(value) => value.as_str().to_owned(),
                            ResourceState::Error(error) => error.clone(),
                            ResourceState::Cancelled => "cancelled".to_owned(),
                            ResourceState::Idle | ResourceState::Loading { .. } => return,
                        };
                        *self.status.borrow_mut() = status;
                        self.task = None;
                        ctx.request_paint();
                    }
                }
                _ => {}
            }
        }

        fn widget_id(&self) -> Option<WidgetId> {
            Some(self.id)
        }

        fn paint(&self, _ctx: &mut vui_core::PaintCtx<'_>) {}

        fn semantics(&self, ctx: &mut vui_core::SemanticsCtx<'_>) {
            ctx.push(vui_core::SemanticNode {
                id: self.id,
                role: SemanticRole::StatusBar,
                bounds: self.bounds,
                label: "Task probe".to_owned(),
                disabled: false,
                focused: false,
                checked: None,
                value: Some(self.status.borrow().clone()),
                ime_cursor_area: None,
            });
        }
    }

    struct RemovableTaskOwner {
        child: Option<TaskProbeWidget>,
        bounds: Rect,
    }

    impl Widget for RemovableTaskOwner {
        fn layout(
            &mut self,
            ctx: &mut vui_core::LayoutCtx<'_>,
            constraints: vui_core::BoxConstraints,
        ) -> Size {
            let size = constraints.constrain(Size::new(160.0, 60.0));
            if let Some(child) = &mut self.child {
                child.layout(ctx, vui_core::BoxConstraints::tight(size));
                child.set_origin(Point::new(self.bounds.x, self.bounds.y));
            }
            self.bounds.width = size.width;
            self.bounds.height = size.height;
            size
        }

        fn set_origin(&mut self, origin: Point) {
            self.bounds.x = origin.x;
            self.bounds.y = origin.y;
            if let Some(child) = &mut self.child {
                child.set_origin(origin);
            }
        }

        fn bounds(&self) -> Rect {
            self.bounds
        }

        fn event(&mut self, ctx: &mut vui_core::EventCtx, event: &Event) {
            if matches!(event, Event::KeyDown(Key::Delete)) {
                self.child = None;
                ctx.request_layout();
            } else if let Some(child) = &mut self.child {
                child.event(ctx, event);
            }
        }

        fn event_target(
            &mut self,
            ctx: &mut vui_core::EventCtx,
            event: &Event,
            target: WidgetId,
        ) -> bool {
            self.child
                .as_mut()
                .is_some_and(|child| child.event_target(ctx, event, target))
        }

        fn paint(&self, ctx: &mut vui_core::PaintCtx<'_>) {
            if let Some(child) = &self.child {
                child.paint(ctx);
            }
        }

        fn semantics(&self, ctx: &mut vui_core::SemanticsCtx<'_>) {
            if let Some(child) = &self.child {
                child.semantics(ctx);
            }
        }
    }

    #[test]
    fn click_activates_and_focuses_button() {
        let activations = Rc::new(Cell::new(0));
        let callback_count = activations.clone();
        let button = Button::new("Run", move || {
            callback_count.set(callback_count.get() + 1);
        });
        let root = vui_core::Padding::new(Insets::all(10.0), button);
        let mut harness = TestHarness::new(root, Size::new(200.0, 100.0));

        let initial = harness.frame();
        assert!(initial.paint.operations().len() >= 3);
        assert!(
            initial
                .semantics
                .iter()
                .any(|node| node.role == SemanticRole::Button && !node.focused)
        );

        let position = Point::new(20.0, 20.0);
        assert!(harness.event(&Event::PointerDown {
            position,
            button: PointerButton::Primary,
            click_count: 1,
        }));
        assert!(harness.event(&Event::PointerUp {
            position,
            button: PointerButton::Primary,
        }));
        assert_eq!(activations.get(), 1);

        let focused = harness.frame();
        assert!(
            focused
                .semantics
                .iter()
                .any(|node| node.role == SemanticRole::Button && node.focused)
        );
    }

    #[test]
    fn button_paints_normal_hovered_pressed_selected_and_disabled_states() {
        let root = Column::new([
            Box::new(Button::new("Interactive", || {})) as Box<dyn Widget>,
            Box::new(Button::new("Selected", || {}).selected(true)) as Box<dyn Widget>,
            Box::new(Button::new("Disabled", || {}).disabled(true)) as Box<dyn Widget>,
        ]);
        let theme = Theme::dark();
        let mut harness = TestHarness::new(root, Size::new(220.0, 160.0));
        let initial = harness.frame();
        assert_eq!(
            button_fill(&initial, "Interactive"),
            Some(theme.components.primary_button.normal.background)
        );
        assert_eq!(
            button_fill(&initial, "Selected"),
            Some(theme.components.primary_button.selected.background)
        );
        assert_eq!(
            button_fill(&initial, "Disabled"),
            Some(theme.components.primary_button.disabled.background)
        );

        harness.event(&Event::PointerMoved(Point::new(20.0, 20.0)));
        let hovered = harness.frame();
        assert_eq!(
            button_fill(&hovered, "Interactive"),
            Some(theme.components.primary_button.hovered.background)
        );

        harness.event(&Event::PointerDown {
            position: Point::new(20.0, 20.0),
            button: PointerButton::Primary,
            click_count: 1,
        });
        let pressed = harness.frame();
        assert_eq!(
            button_fill(&pressed, "Interactive"),
            Some(theme.components.primary_button.pressed.background)
        );
        assert!(pressed.paint.operations().iter().any(|operation| matches!(
            operation,
            PaintOp::StrokeRect { color, width, .. }
                if *color == theme.colors.focus
                    && (*width - theme.borders.strong).abs() < f64::EPSILON
        )));
    }

    #[test]
    fn button_hover_animation_uses_the_manual_test_clock() {
        let theme = Theme::dark();
        let mut harness = TestHarness::new(Button::new("Animated", || {}), Size::new(200.0, 60.0));
        harness.set_reduced_motion(false);
        let initial = harness.frame();
        let normal = button_fill(&initial, "Animated").expect("normal button fill");

        harness.event(&Event::PointerMoved(Point::new(20.0, 20.0)));
        assert!(harness.animation_pending());
        assert!(harness.advance_by(Duration::from_millis(60)));
        let halfway = button_fill(&harness.frame(), "Animated").expect("animated button fill");
        assert_ne!(halfway, normal);
        assert_ne!(halfway, theme.components.primary_button.hovered.background);

        assert!(harness.advance_by(Duration::from_millis(60)));
        let finished = button_fill(&harness.frame(), "Animated").expect("hovered button fill");
        assert_eq!(finished, theme.components.primary_button.hovered.background);
        assert!(!harness.animation_pending());
    }

    #[test]
    fn delayed_animation_frame_does_not_wake_early() {
        let frames = Rc::new(Cell::new(0));
        let mut harness = TestHarness::new(
            DelayedFrameWidget {
                frames: frames.clone(),
                bounds: Rect::default(),
            },
            Size::new(80.0, 80.0),
        );
        harness.set_reduced_motion(false);
        harness.event(&Event::PointerDown {
            position: Point::new(10.0, 10.0),
            button: PointerButton::Primary,
            click_count: 1,
        });
        assert!(harness.animation_pending());
        assert!(!harness.advance_by(Duration::from_millis(99)));
        assert_eq!(frames.get(), 0);
        assert!(harness.advance_by(Duration::from_millis(1)));
        assert_eq!(frames.get(), 1);
        assert!(!harness.animation_pending());
    }

    #[test]
    fn typed_task_reports_progress_and_completes_resource() {
        let status = Rc::new(std::cell::RefCell::new(String::new()));
        let started = Arc::new(AtomicBool::new(false));
        let widget = TaskProbeWidget::new(status.clone(), started.clone(), false);
        let mut harness = TestHarness::new(widget, Size::new(160.0, 60.0));
        let _initial = harness.frame();

        assert!(harness.event(&Event::PointerDown {
            position: Point::new(10.0, 10.0),
            button: PointerButton::Primary,
            click_count: 1,
        }));
        assert_eq!(status.borrow().as_str(), "loading");
        assert_eq!(harness.run_pending_tasks(), 1);
        assert!(started.load(Ordering::Acquire));
        assert_eq!(status.borrow().as_str(), "ready");
        assert_eq!(
            semantic_value(&harness.frame(), "Task probe").as_deref(),
            Some("ready")
        );
    }

    #[test]
    fn task_failure_reaches_resource_error_state() {
        let status = Rc::new(std::cell::RefCell::new(String::new()));
        let started = Arc::new(AtomicBool::new(false));
        let widget = TaskProbeWidget::new(status.clone(), started, true);
        let mut harness = TestHarness::new(widget, Size::new(160.0, 60.0));
        let _initial = harness.frame();
        harness.event(&Event::PointerDown {
            position: Point::new(10.0, 10.0),
            button: PointerButton::Primary,
            click_count: 1,
        });

        assert_eq!(harness.run_pending_tasks(), 1);
        assert_eq!(status.borrow().as_str(), "offline");
    }

    #[test]
    fn progress_bar_clamps_visual_and_accessible_value() {
        let mut harness = TestHarness::new(
            ProgressBar::new(1.5).label("Indexing"),
            Size::new(200.0, 20.0),
        );
        let frame = harness.frame();
        assert_eq!(semantic_value(&frame, "Indexing").as_deref(), Some("100%"));
        assert!(frame.paint.operations().iter().any(|operation| matches!(
            operation,
            PaintOp::FillRect { rect, .. } if (rect.width - 200.0).abs() < f64::EPSILON
        )));
    }

    #[test]
    fn cancelling_before_execution_prevents_task_job_from_running() {
        let status = Rc::new(std::cell::RefCell::new(String::new()));
        let started = Arc::new(AtomicBool::new(false));
        let widget = TaskProbeWidget::new(status.clone(), started.clone(), false);
        let mut harness = TestHarness::new(widget, Size::new(160.0, 60.0));
        let _initial = harness.frame();
        harness.event(&Event::PointerDown {
            position: Point::new(10.0, 10.0),
            button: PointerButton::Primary,
            click_count: 1,
        });
        harness.event(&Event::KeyDown(Key::Escape));

        assert_eq!(harness.run_pending_tasks(), 1);
        assert!(!started.load(Ordering::Acquire));
        assert_eq!(status.borrow().as_str(), "cancelled");
    }

    #[test]
    fn removing_task_owner_cancels_queued_work_and_discards_stale_result() {
        let status = Rc::new(std::cell::RefCell::new(String::new()));
        let started = Arc::new(AtomicBool::new(false));
        let root = RemovableTaskOwner {
            child: Some(TaskProbeWidget::new(status, started.clone(), false)),
            bounds: Rect::default(),
        };
        let mut harness = TestHarness::new(root, Size::new(160.0, 60.0));
        let _initial = harness.frame();
        harness.event(&Event::PointerDown {
            position: Point::new(10.0, 10.0),
            button: PointerButton::Primary,
            click_count: 1,
        });
        harness.event(&Event::KeyDown(Key::Delete));
        let removed = harness.frame();
        assert!(semantic_bounds(&removed, "Task probe").is_none());

        assert_eq!(harness.run_pending_tasks(), 1);
        assert!(!started.load(Ordering::Acquire));
    }

    #[test]
    fn button_variants_resolve_their_token_palettes() {
        let theme = Theme::dark();
        let variants = [
            ButtonVariant::Primary,
            ButtonVariant::Secondary,
            ButtonVariant::Ghost,
            ButtonVariant::Destructive,
            ButtonVariant::Icon,
        ];
        for variant in variants {
            let style = theme.button_style(variant);
            assert_eq!(style.resolve(ControlState::Hovered), style.hovered);
            assert_eq!(style.resolve(ControlState::Pressed), style.pressed);
            assert_eq!(style.resolve(ControlState::Selected), style.selected);
            assert_eq!(style.resolve(ControlState::Disabled), style.disabled);
        }
        assert_ne!(
            theme.button_style(ButtonVariant::Primary).normal,
            theme.button_style(ButtonVariant::Secondary).normal
        );
        assert_ne!(
            theme.button_style(ButtonVariant::Destructive).normal,
            theme.button_style(ButtonVariant::Primary).normal
        );
    }

    #[test]
    fn theme_scope_surface_border_and_elevation_use_local_tokens() {
        let light = Theme::light();
        let root = ThemeScope::new(
            light,
            Surface::new(Label::new("Card"))
                .tone(SurfaceTone::Raised)
                .padding(Insets::all(12.0))
                .border(light.colors.border, light.borders.thin)
                .elevation(1),
        );
        let mut harness = TestHarness::new(root, Size::new(180.0, 80.0));
        let frame = harness.frame();
        assert!(frame.paint.operations().iter().any(|operation| matches!(
            operation,
            PaintOp::FillRect { color, .. } if *color == light.colors.surface_raised
        )));
        assert!(frame.paint.operations().iter().any(|operation| matches!(
            operation,
            PaintOp::FillRect { color, .. } if *color == light.elevation.shadow
        )));
        assert!(frame.paint.operations().iter().any(|operation| matches!(
            operation,
            PaintOp::StrokeRect { color, .. } if *color == light.colors.border
        )));
    }

    #[test]
    fn runtime_theme_can_switch_live() {
        let mut harness = TestHarness::new(Button::new("Theme", || {}), Size::new(180.0, 80.0));
        let dark = harness.frame();
        assert_eq!(
            button_fill(&dark, "Theme"),
            Some(Theme::dark().colors.primary)
        );

        harness.set_theme(Theme::light());
        let light = harness.frame();
        assert_eq!(
            button_fill(&light, "Theme"),
            Some(Theme::light().colors.primary)
        );
    }

    #[test]
    fn flex_distributes_space_by_grow_factor() {
        let root = Flex::row([
            FlexChild::flexible(1.0, Label::new("One")),
            FlexChild::flexible(2.0, Label::new("Two")),
        ])
        .gap(6.0)
        .cross_alignment(CrossAxisAlignment::Stretch);
        let mut harness = TestHarness::new(root, Size::new(306.0, 80.0));
        let frame = harness.frame();
        let one = semantic_bounds(&frame, "One").expect("first label");
        let two = semantic_bounds(&frame, "Two").expect("second label");

        assert!((one.width - 100.0).abs() < 0.01);
        assert!((two.width - 200.0).abs() < 0.01);
        assert!((two.x - 106.0).abs() < 0.01);
        assert!((one.height - 80.0).abs() < 0.01);
    }

    #[test]
    fn sized_box_enforces_explicit_dimensions() {
        let root = SizedBox::new(Label::new("Sized")).width(120.0).height(44.0);
        let mut harness = TestHarness::new(root, Size::new(300.0, 100.0));
        let bounds = semantic_bounds(&harness.frame(), "Sized").expect("label");
        assert!((bounds.width - 120.0).abs() < 0.01);
        assert!((bounds.height - 44.0).abs() < 0.01);
    }

    #[test]
    fn stack_routes_pointer_to_topmost_child() {
        let bottom = Rc::new(Cell::new(0));
        let top = Rc::new(Cell::new(0));
        let bottom_callback = bottom.clone();
        let top_callback = top.clone();
        let root = Stack::new([
            Box::new(
                SizedBox::new(Button::new("Bottom", move || {
                    bottom_callback.set(bottom_callback.get() + 1);
                }))
                .width(120.0)
                .height(50.0),
            ) as Box<dyn Widget>,
            Box::new(
                SizedBox::new(Button::new("Top", move || {
                    top_callback.set(top_callback.get() + 1);
                }))
                .width(120.0)
                .height(50.0),
            ) as Box<dyn Widget>,
        ]);
        let mut harness = TestHarness::new(root, Size::new(120.0, 50.0));
        let _ = harness.frame();
        click(&mut harness, Point::new(20.0, 20.0));
        assert_eq!(top.get(), 1);
        assert_eq!(bottom.get(), 0);
    }

    #[test]
    fn split_pane_drag_resizes_both_children() {
        let root = SplitPane::horizontal(Label::new("Left"), Label::new("Right"))
            .ratio(0.25)
            .divider_width(6.0)
            .min_pane(20.0);
        let mut harness = TestHarness::new(root, Size::new(406.0, 100.0));
        let initial = harness.frame();
        assert!((semantic_bounds(&initial, "Left").expect("left").width - 100.0).abs() < 0.01);

        assert!(harness.event(&Event::PointerDown {
            position: Point::new(103.0, 50.0),
            button: PointerButton::Primary,
            click_count: 1,
        }));
        assert!(harness.event(&Event::PointerMoved(Point::new(243.0, 50.0))));
        assert!(harness.event(&Event::PointerUp {
            position: Point::new(243.0, 50.0),
            button: PointerButton::Primary,
        }));

        let resized = harness.frame();
        let left = semantic_bounds(&resized, "Left").expect("left");
        let right = semantic_bounds(&resized, "Right").expect("right");
        assert!((left.width - 243.0).abs() < 0.01);
        assert!((right.x - 249.0).abs() < 0.01);
        assert!((right.width - 157.0).abs() < 0.01);
    }

    #[test]
    fn constrained_box_applies_limits_and_aspect_ratio() {
        let root = ConstrainedBox::new(Label::new("Preview"))
            .min_width(80.0)
            .max_width(160.0)
            .max_height(100.0)
            .aspect_ratio(2.0);
        let mut harness = TestHarness::new(root, Size::new(500.0, 300.0));
        let bounds = semantic_bounds(&harness.frame(), "Preview").expect("preview");
        assert!((bounds.width - 160.0).abs() < 0.01);
        assert!((bounds.height - 80.0).abs() < 0.01);
    }

    #[test]
    fn flex_wraps_children_into_runs_and_honors_align_self() {
        let root = Flex::row([
            FlexChild::fixed(SizedBox::new(Label::new("One")).width(80.0).height(40.0)),
            FlexChild::fixed(SizedBox::new(Label::new("Two")).width(80.0).height(20.0))
                .align_self(CrossAxisAlignment::End),
            FlexChild::fixed(SizedBox::new(Label::new("Three")).width(80.0).height(30.0)),
        ])
        .gap(10.0)
        .run_gap(7.0)
        .wrap(FlexWrap::Wrap);
        let mut harness = TestHarness::new(root, Size::new(170.0, 200.0));
        let frame = harness.frame();
        let one = semantic_bounds(&frame, "One").expect("one");
        let two = semantic_bounds(&frame, "Two").expect("two");
        let three = semantic_bounds(&frame, "Three").expect("three");
        assert!((two.y - 20.0).abs() < 0.01);
        assert!((three.y - 47.0).abs() < 0.01);
        assert!((one.x - 0.0).abs() < 0.01);
    }

    #[test]
    fn responsive_switches_branch_after_resize() {
        let root = Responsive::new(300.0, Label::new("Compact"), Label::new("Wide"));
        let mut harness = TestHarness::new(root, Size::new(240.0, 100.0));
        let compact = harness.frame();
        assert!(semantic_bounds(&compact, "Compact").is_some());
        assert!(semantic_bounds(&compact, "Wide").is_none());

        harness.resize(Size::new(500.0, 100.0));
        let wide = harness.frame();
        assert!(semantic_bounds(&wide, "Wide").is_some());
        assert!(semantic_bounds(&wide, "Compact").is_none());
    }

    #[test]
    fn overflowing_flex_draws_debug_indicator() {
        let root = Flex::row([
            FlexChild::fixed(SizedBox::new(Label::new("One")).width(100.0)),
            FlexChild::fixed(SizedBox::new(Label::new("Two")).width(100.0)),
        ]);
        let mut harness = TestHarness::new(root, Size::new(150.0, 60.0));
        let frame = harness.frame();
        assert!(frame.paint.operations().iter().any(|operation| matches!(
            operation,
            PaintOp::StrokeRect { color, width, .. }
                if *color == vui_core::Color::rgb(255, 64, 64)
                    && (*width - 2.0).abs() < f64::EPSILON
        )));
    }

    #[test]
    fn split_pane_handles_tiny_viewports() {
        let root = SplitPane::horizontal(Label::new("Left"), Label::new("Right")).min_pane(80.0);
        let mut harness = TestHarness::new(root, Size::new(20.0, 12.0));
        let frame = harness.frame();
        assert!(frame.semantics.iter().all(|node| {
            node.bounds.width.is_finite()
                && node.bounds.height.is_finite()
                && node.bounds.width >= 0.0
                && node.bounds.height >= 0.0
        }));
    }

    #[test]
    fn split_pane_resolves_unbounded_scroll_constraints_to_finite_content() {
        let root = ScrollView::new(SplitPane::horizontal(
            Label::new("Left"),
            Label::new("Right"),
        ))
        .axes(ScrollAxes::Both)
        .viewport_width(160.0)
        .viewport_height(80.0);
        let mut harness = TestHarness::new(root, Size::new(160.0, 80.0));
        let frame = harness.frame();
        assert!(frame.semantics.iter().all(|node| {
            node.bounds.x.is_finite()
                && node.bounds.y.is_finite()
                && node.bounds.width.is_finite()
                && node.bounds.height.is_finite()
        }));
    }

    #[test]
    fn icon_emits_vector_path_and_optional_image_semantics() {
        let root = Icon::named(IconName::Search)
            .size(28.0)
            .accessible_label("Search symbol");
        let mut harness = TestHarness::new(root, Size::new(80.0, 80.0));
        let frame = harness.frame();
        assert!(frame.paint.operations().iter().any(|operation| matches!(
            operation,
            PaintOp::StrokePath { commands, .. } if !commands.is_empty()
        )));
        assert!(
            frame
                .semantics
                .iter()
                .any(|node| { node.role == SemanticRole::Image && node.label == "Search symbol" })
        );
    }

    #[test]
    fn icon_button_is_accessible_activates_and_opens_tooltip() {
        let activations = Rc::new(Cell::new(0));
        let callback = activations.clone();
        let root = OverlayHost::new(
            IconButton::new(IconName::Settings, "Settings", move || {
                callback.set(callback.get() + 1);
            })
            .tooltip("Open settings"),
        );
        let mut harness = TestHarness::new(root, Size::new(180.0, 100.0));
        let initial = harness.frame();
        let bounds = semantic_bounds(&initial, "Settings").expect("icon button");
        harness.event(&Event::PointerMoved(Point::new(
            bounds.x + 10.0,
            bounds.y + 10.0,
        )));
        assert!(
            harness
                .frame()
                .semantics
                .iter()
                .any(|node| node.label == "Open settings")
        );

        harness.event(&Event::PointerLeft);
        let _ = harness.frame();
        click(&mut harness, Point::new(bounds.x + 10.0, bounds.y + 10.0));
        assert_eq!(activations.get(), 1);
    }

    #[test]
    fn shell_regions_publish_accessible_roles() {
        let root = Column::new([
            Box::new(Toolbar::new([
                Box::new(Icon::named(IconName::Add)) as Box<dyn Widget>
            ])) as Box<dyn Widget>,
            Box::new(Sidebar::new([
                Box::new(NavigationItem::new("Files", || {})) as Box<dyn Widget>,
            ])) as Box<dyn Widget>,
            Box::new(StatusBar::new([
                Box::new(Label::new("Ready")) as Box<dyn Widget>
            ])) as Box<dyn Widget>,
        ]);
        let mut harness = TestHarness::new(root, Size::new(300.0, 240.0));
        let frame = harness.frame();
        assert!(
            frame
                .semantics
                .iter()
                .any(|node| node.role == SemanticRole::Toolbar)
        );
        assert!(
            frame
                .semantics
                .iter()
                .any(|node| node.role == SemanticRole::Navigation)
        );
        assert!(
            frame
                .semantics
                .iter()
                .any(|node| node.role == SemanticRole::StatusBar)
        );
    }

    #[test]
    fn tabs_select_close_navigate_and_report_overflow() {
        let selected = Rc::new(Cell::new(usize::MAX));
        let closed = Rc::new(Cell::new(usize::MAX));
        let selected_callback = selected.clone();
        let closed_callback = closed.clone();
        let root = TabBar::new(
            [
                Tab::new("One"),
                Tab::new("Two").closable(true),
                Tab::new("Three").closable(true),
                Tab::new("Four").closable(true),
            ],
            move |index| selected_callback.set(index),
        )
        .on_close(move |index| closed_callback.set(index));
        let mut harness = TestHarness::new(root, Size::new(250.0, 60.0));
        let initial = harness.frame();
        let two = semantic_bounds(&initial, "Two").expect("second tab");
        click(&mut harness, Point::new(two.x + 20.0, two.y + 15.0));
        assert_eq!(selected.get(), 1);

        assert!(harness.event(&Event::KeyDown(Key::ArrowLeft)));
        assert_eq!(selected.get(), 0);
        click(
            &mut harness,
            Point::new(two.x + two.width - 8.0, two.y + 15.0),
        );
        assert_eq!(closed.get(), 1);
        assert!(initial.paint.operations().iter().any(|operation| matches!(
            operation,
            PaintOp::Text { text, .. } if text.starts_with('+')
        )));
    }

    #[test]
    fn tabs_reorder_with_pointer_drag_and_alt_arrow() {
        use std::cell::RefCell;

        let reorders = Rc::new(RefCell::new(Vec::new()));
        let reported = reorders.clone();
        let root = TabBar::new(
            [Tab::new("One"), Tab::new("Two"), Tab::new("Three")],
            |_| {},
        )
        .reorderable(true)
        .on_reorder(move |from, to| reported.borrow_mut().push((from, to)));
        let mut harness = TestHarness::new(root, Size::new(300.0, 60.0));
        let frame = harness.frame();
        let one = semantic_bounds(&frame, "One").expect("first tab");
        let two = semantic_bounds(&frame, "Two").expect("second tab");

        harness.event(&Event::PointerDown {
            position: Point::new(one.x + 20.0, one.y + 16.0),
            button: PointerButton::Primary,
            click_count: 1,
        });
        assert!(harness.event(&Event::PointerMoved(
            Point::new(two.x + 20.0, two.y + 16.0,)
        )));
        assert!(harness.drag_active());
        assert!(harness.event(&Event::PointerUp {
            position: Point::new(two.x + 20.0, two.y + 16.0),
            button: PointerButton::Primary,
        }));
        assert_eq!(reorders.borrow().as_slice(), &[(0, 1)]);

        let mut alt = Modifiers::default();
        alt.set(Modifier::Alt, true);
        harness.event(&Event::ModifiersChanged(alt));
        assert!(harness.event(&Event::KeyDown(Key::ArrowRight)));
        assert_eq!(reorders.borrow().as_slice(), &[(0, 1), (0, 1)]);
    }

    #[test]
    fn dialog_helper_opens_as_a_modal_overlay() {
        let root = OverlayHost::new(Button::action("Open dialog", |_, ctx| {
            ctx.show_modal(Dialog::confirmation(
                "Delete item?",
                "This cannot be undone.",
                "Delete",
                || {},
            ));
        }));
        let mut harness = TestHarness::new(root, Size::new(420.0, 240.0));
        let initial = harness.frame();
        let button = semantic_bounds(&initial, "Open dialog").expect("button");
        click(&mut harness, Point::new(button.x + 12.0, button.y + 12.0));
        let modal = harness.frame();
        assert!(
            modal
                .semantics
                .iter()
                .any(|node| { node.role == SemanticRole::Dialog && node.label == "Delete item?" })
        );
        assert!(
            !modal
                .semantics
                .iter()
                .any(|node| node.label == "Open dialog")
        );
    }

    #[test]
    fn command_palette_filters_and_activates_with_keyboard() {
        let activated = Rc::new(Cell::new(0));
        let callback = activated.clone();
        let commands = vec![
            Command::new("Open file", || {}),
            Command::new("Toggle theme", move || {
                callback.set(callback.get() + 1);
            })
            .keywords("appearance dark light"),
        ];
        let descriptors = commands
            .iter()
            .map(|command| command.descriptor().clone())
            .collect::<Vec<_>>();
        let palette = CommandPalette::new(descriptors);
        let base = SizedBox::new(Label::new("Workspace"))
            .width(600.0)
            .height(400.0);
        let root = CommandHost::new(commands, OverlayHost::new(base).centered_modal(palette));
        let mut harness = TestHarness::new(root, Size::new(600.0, 400.0));
        let initial = harness.frame();
        assert!(semantic_bounds(&initial, "Open file").is_some());
        assert!(semantic_bounds(&initial, "Toggle theme").is_some());

        assert!(harness.event(&Event::TextInput("theme".to_owned())));
        let filtered = harness.frame();
        assert!(semantic_bounds(&filtered, "Open file").is_none());
        assert!(semantic_bounds(&filtered, "Toggle theme").is_some());
        assert!(harness.event(&Event::KeyDown(Key::Enter)));
        assert_eq!(activated.get(), 1);
    }

    #[test]
    fn command_shortcuts_normalize_control_and_meta_and_prefer_focused_scope() {
        let global_count = Rc::new(Cell::new(0));
        let focused_count = Rc::new(Cell::new(0));
        let global_callback = global_count.clone();
        let focused_callback = focused_count.clone();
        let chord = KeyChord::primary(Key::Character("k".to_owned()));
        let commands = vec![
            Command::action("global.search", "Global search", move || {
                global_callback.set(global_callback.get() + 1);
            })
            .shortcut(chord.clone()),
            Command::action("focused.search", "Focused search", move || {
                focused_callback.set(focused_callback.get() + 1);
            })
            .shortcut(chord)
            .scope(CommandScope::Focused),
        ];
        let root = CommandHost::new(commands, Label::new("Workspace"));
        let mut harness = TestHarness::new(root, Size::new(300.0, 100.0));

        let mut control = Modifiers::default();
        control.set(Modifier::Control, true);
        harness.event(&Event::ModifiersChanged(control));
        assert!(harness.event(&Event::KeyDown(Key::Character("K".to_owned()))));

        let mut meta = Modifiers::default();
        meta.set(Modifier::Meta, true);
        harness.event(&Event::ModifiersChanged(meta));
        assert!(harness.event(&Event::KeyDown(Key::Character("k".to_owned()))));

        assert_eq!(global_count.get(), 0);
        assert_eq!(focused_count.get(), 2);
    }

    #[test]
    fn command_registry_reports_shortcut_conflicts() {
        let chord = KeyChord::primary(Key::Character("p".to_owned()));
        let registry = CommandRegistry::new([
            Command::action("file.print", "Print", || {}).shortcut(chord.clone()),
            Command::action("palette.open", "Open palette", || {}).shortcut(chord),
        ]);
        let conflicts = registry.shortcut_conflicts();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].chord.label(), "Ctrl/Cmd+P");
        assert_eq!(conflicts[0].commands.len(), 2);
    }

    #[test]
    fn command_bound_button_and_menu_share_registry_state() {
        let activations = Rc::new(Cell::new(0));
        let callback = activations.clone();
        let command = Command::action("settings.save", "Save", move || {
            callback.set(callback.get() + 1);
        })
        .shortcut(KeyChord::primary(Key::Character("s".to_owned())))
        .checked(true);
        let descriptor = command.descriptor().clone();
        let root = CommandHost::new(
            [command],
            Column::new([
                Box::new(CommandButton::new(descriptor.clone())) as Box<dyn Widget>,
                Box::new(Menu::new([MenuItem::command(descriptor)])) as Box<dyn Widget>,
            ]),
        );
        let mut harness = TestHarness::new(root, Size::new(300.0, 140.0));
        let frame = harness.frame();
        let button = semantic_bounds(&frame, "Save").expect("command button");
        assert!(frame.semantics.iter().any(|node| {
            node.label == "Save (Ctrl/Cmd+S)"
                && node.role == SemanticRole::MenuItemCheckbox
                && node.checked == Some(true)
        }));

        click(&mut harness, Point::new(button.x + 8.0, button.y + 8.0));
        assert_eq!(activations.get(), 1);
    }

    #[test]
    fn deferred_command_updates_app_model_in_the_same_interaction() {
        enum Message {
            Increment,
        }

        let root = App::new(
            0_u32,
            |count, Message::Increment| *count += 1,
            |count, sender| {
                let command_sender = sender.clone();
                let command = Command::action("counter.increment", "Increment", move || {
                    command_sender.emit(Message::Increment);
                });
                let descriptor = command.descriptor().clone();
                CommandHost::new(
                    [command],
                    Column::new([
                        Box::new(Label::new(format!("Count: {count}"))) as Box<dyn Widget>,
                        Box::new(CommandButton::new(descriptor)) as Box<dyn Widget>,
                    ]),
                )
            },
        );
        let mut harness = TestHarness::new(root, Size::new(240.0, 100.0));
        let initial = harness.frame();
        let button = semantic_bounds(&initial, "Increment").expect("command button");
        click(&mut harness, Point::new(button.x + 8.0, button.y + 8.0));

        let updated = harness.frame();
        assert!(semantic_bounds(&updated, "Count: 1").is_some());
    }

    #[test]
    fn list_view_only_materializes_visible_rows_from_ten_thousand_items() {
        let builds = Rc::new(Cell::new(0));
        let build_count = builds.clone();
        let list = ListView::new(10_000, move |index| {
            build_count.set(build_count.get() + 1);
            ListItem::new(
                format!("item-{index}"),
                format!("Item {index}"),
                Label::new(format!("Item {index}")),
            )
        })
        .row_height(24.0)
        .overscan(2)
        .viewport_height(240.0);
        let mut harness = TestHarness::new(list, Size::new(320.0, 240.0));
        let initial = harness.frame();
        let visible_rows = initial
            .semantics
            .iter()
            .filter(|node| node.role == SemanticRole::ListItem)
            .count();

        assert_eq!(visible_rows, 12);
        assert!(builds.get() < 20);
        assert!(semantic_bounds(&initial, "Item 0").is_some());
        assert!(semantic_bounds(&initial, "Item 100").is_none());
    }

    #[test]
    fn list_view_supports_keyboard_range_selection_and_scrolls_active_row() {
        use std::cell::RefCell;

        let selections = Rc::new(RefCell::new(Vec::<Vec<String>>::new()));
        let reported = selections.clone();
        let list = ListView::new(100, |index| {
            ListItem::new(
                format!("item-{index}"),
                format!("Item {index}"),
                Label::new(format!("Item {index}")),
            )
        })
        .row_height(20.0)
        .viewport_height(100.0)
        .on_selection_change(move |keys| {
            reported
                .borrow_mut()
                .push(keys.iter().map(|key| key.as_str().to_owned()).collect());
        });
        let mut harness = TestHarness::new(list, Size::new(260.0, 100.0));
        let _initial = harness.frame();
        harness.event(&Event::PointerDown {
            position: Point::new(20.0, 30.0),
            button: PointerButton::Primary,
            click_count: 1,
        });
        let mut shift = Modifiers::default();
        shift.set(Modifier::Shift, true);
        harness.event(&Event::ModifiersChanged(shift));
        assert!(harness.event(&Event::KeyDown(Key::ArrowDown)));
        assert_eq!(
            selections.borrow().last().cloned(),
            Some(vec!["item-1".to_owned(), "item-2".to_owned()])
        );

        assert!(harness.event(&Event::KeyDown(Key::End)));
        let end = harness.frame();
        assert!(semantic_bounds(&end, "Item 99").is_some());
        assert!(semantic_bounds(&end, "Item 0").is_none());
    }

    #[test]
    fn list_view_scroll_to_item_and_accessibility_activation_work() {
        let activated = Rc::new(Cell::new(usize::MAX));
        let activation = activated.clone();
        let list = ListView::new(10_000, |index| {
            ListItem::new(
                format!("item-{index}"),
                format!("Item {index}"),
                Label::new(format!("Item {index}")),
            )
        })
        .row_height(25.0)
        .viewport_height(125.0)
        .scroll_to_item(9_999)
        .on_activate(move |key| {
            let index = key
                .as_str()
                .trim_start_matches("item-")
                .parse()
                .expect("numeric item key");
            activation.set(index);
        });
        let mut harness = TestHarness::new(list, Size::new(300.0, 125.0));
        let frame = harness.frame();
        let item = frame
            .semantics
            .iter()
            .find(|node| node.role == SemanticRole::ListItem && node.label == "Item 9999")
            .expect("last row is visible");
        assert!(harness.event(&Event::AccessibilityAction {
            target: item.id,
            action: SemanticAction::Click,
        }));
        assert_eq!(activated.get(), 9_999);
    }

    #[test]
    fn list_drag_negotiates_move_and_reports_stable_keys() {
        use std::cell::RefCell;

        let drops = Rc::new(RefCell::new(Vec::<CollectionDrop>::new()));
        let reported = drops.clone();
        let list = ListView::new(4, |index| {
            ListItem::new(
                format!("item-{index}"),
                format!("Item {index}"),
                Label::new(format!("Item {index}")),
            )
        })
        .row_height(24.0)
        .viewport_height(96.0)
        .draggable(true)
        .on_drop(move |drop| reported.borrow_mut().push(drop));
        let mut harness = TestHarness::new(list, Size::new(240.0, 96.0));
        let _initial = harness.frame();

        harness.event(&Event::PointerDown {
            position: Point::new(12.0, 12.0),
            button: PointerButton::Primary,
            click_count: 1,
        });
        assert!(harness.event(&Event::PointerMoved(Point::new(12.0, 42.0))));
        assert!(harness.drag_active());
        let dragging = harness.frame();
        assert!(dragging.paint.operations().iter().any(|operation| matches!(
            operation,
            PaintOp::Text { text, .. } if text == "Item 0"
        )));

        assert!(harness.event(&Event::PointerUp {
            position: Point::new(12.0, 42.0),
            button: PointerButton::Primary,
        }));
        assert!(!harness.drag_active());
        assert_eq!(
            drops.borrow().as_slice(),
            &[CollectionDrop {
                dragged: WidgetKey::new("item-0"),
                target: WidgetKey::new("item-1"),
                position: DropPosition::After,
                operation: vui_core::DragOperation::Move,
            }]
        );
    }

    #[test]
    fn list_drag_cancels_on_escape_and_supports_keyboard_reorder() {
        use std::cell::RefCell;

        let drops = Rc::new(RefCell::new(Vec::<CollectionDrop>::new()));
        let reported = drops.clone();
        let list = ListView::new(3, |index| {
            ListItem::new(
                format!("item-{index}"),
                format!("Item {index}"),
                Label::new(format!("Item {index}")),
            )
        })
        .row_height(24.0)
        .draggable(true)
        .on_drop(move |drop| reported.borrow_mut().push(drop));
        let mut harness = TestHarness::new(list, Size::new(240.0, 72.0));
        let _initial = harness.frame();

        harness.event(&Event::PointerDown {
            position: Point::new(12.0, 12.0),
            button: PointerButton::Primary,
            click_count: 1,
        });
        harness.event(&Event::PointerMoved(Point::new(12.0, 36.0)));
        assert!(harness.drag_active());
        assert!(harness.event(&Event::KeyDown(Key::Escape)));
        assert!(!harness.drag_active());
        assert!(drops.borrow().is_empty());

        let mut alt = Modifiers::default();
        alt.set(Modifier::Alt, true);
        harness.event(&Event::ModifiersChanged(alt));
        assert!(harness.event(&Event::KeyDown(Key::ArrowDown)));
        assert_eq!(drops.borrow().len(), 1);
        assert_eq!(drops.borrow()[0].dragged.as_str(), "item-0");
        assert_eq!(drops.borrow()[0].target.as_str(), "item-1");
    }

    #[test]
    fn tree_view_expands_and_navigates_between_parent_and_child() {
        let expansions = Rc::new(Cell::new(0));
        let expansion_count = expansions.clone();
        let tree = TreeView::new([
            TreeNode::branch(
                "src",
                "src",
                [
                    TreeNode::leaf("src/lib.rs", "lib.rs"),
                    TreeNode::leaf("src/main.rs", "main.rs"),
                ],
            ),
            TreeNode::leaf("readme", "README.md"),
        ])
        .row_height(24.0)
        .viewport_height(120.0)
        .on_expansion_change(move |_, expanded| {
            if expanded {
                expansion_count.set(expansion_count.get() + 1);
            }
        });
        let mut harness = TestHarness::new(tree, Size::new(280.0, 120.0));
        let collapsed = harness.frame();
        assert!(semantic_bounds(&collapsed, "lib.rs").is_none());

        assert!(harness.event(&Event::PointerDown {
            position: Point::new(12.0, 12.0),
            button: PointerButton::Primary,
            click_count: 1,
        }));
        let expanded = harness.frame();
        assert_eq!(expansions.get(), 1);
        assert!(semantic_bounds(&expanded, "lib.rs").is_some());

        assert!(harness.event(&Event::KeyDown(Key::ArrowRight)));
        let child = harness.frame();
        assert!(child.semantics.iter().any(|node| {
            node.role == SemanticRole::TreeItem && node.label == "lib.rs" && node.focused
        }));
        assert!(harness.event(&Event::KeyDown(Key::ArrowLeft)));
        let parent = harness.frame();
        assert!(parent.semantics.iter().any(|node| {
            node.role == SemanticRole::TreeItem && node.label == "src" && node.focused
        }));
        assert!(harness.event(&Event::KeyDown(Key::ArrowLeft)));
        let recollapsed = harness.frame();
        assert!(semantic_bounds(&recollapsed, "lib.rs").is_none());
    }

    #[test]
    fn tree_view_exposes_lazy_loading_and_error_rows() {
        let tree = TreeView::new([
            TreeNode::unloaded("remote", "Remote"),
            TreeNode::loading("packages", "Packages"),
            TreeNode::error("broken", "Broken", "permission denied"),
        ])
        .expanded([
            WidgetKey::new("remote"),
            WidgetKey::new("packages"),
            WidgetKey::new("broken"),
        ])
        .viewport_height(260.0);
        let mut harness = TestHarness::new(tree, Size::new(320.0, 260.0));
        let frame = harness.frame();

        assert!(frame.semantics.iter().any(|node| {
            node.role == SemanticRole::TreeItem && node.label == "Load children…" && node.disabled
        }));
        assert!(frame.semantics.iter().any(|node| {
            node.role == SemanticRole::TreeItem && node.label == "Loading…" && node.disabled
        }));
        assert!(frame.semantics.iter().any(|node| {
            node.role == SemanticRole::TreeItem
                && node.label == "Error: permission denied"
                && node.disabled
        }));
    }

    #[test]
    fn tree_view_virtualizes_ten_thousand_root_nodes() {
        let nodes = (0..10_000)
            .map(|index| TreeNode::leaf(format!("node-{index}"), format!("Node {index}")))
            .collect::<Vec<_>>();
        let tree = TreeView::new(nodes)
            .row_height(24.0)
            .overscan(2)
            .viewport_height(240.0);
        let mut harness = TestHarness::new(tree, Size::new(320.0, 240.0));
        let frame = harness.frame();
        let materialized = frame
            .semantics
            .iter()
            .filter(|node| node.role == SemanticRole::TreeItem)
            .count();

        assert_eq!(materialized, 12);
        assert!(semantic_bounds(&frame, "Node 0").is_some());
        assert!(semantic_bounds(&frame, "Node 100").is_none());
    }

    #[test]
    fn table_view_virtualizes_cells_and_uses_stable_row_keys_for_selection() {
        use std::cell::RefCell;

        let builds = Rc::new(Cell::new(0));
        let build_count = builds.clone();
        let selections = Rc::new(RefCell::new(Vec::<String>::new()));
        let reported = selections.clone();
        let table = TableView::new(
            10_000,
            [
                TableColumn::new("name", "Name", 140.0),
                TableColumn::new("kind", "Kind", 90.0),
                TableColumn::new("size", "Size", 80.0).alignment(TableAlignment::Right),
            ],
            move |row, column| {
                build_count.set(build_count.get() + 1);
                format!("{}-{row}", column.as_str())
            },
        )
        .row_height(24.0)
        .viewport_height(180.0)
        .row_keys(|row| WidgetKey::new(format!("file-{row}")))
        .on_selection_change(move |keys| {
            reported
                .borrow_mut()
                .extend(keys.into_iter().map(|key| key.as_str().to_owned()));
        });
        let mut harness = TestHarness::new(table, Size::new(310.0, 180.0));
        let frame = harness.frame();
        let rows = frame
            .semantics
            .iter()
            .filter(|node| node.role == SemanticRole::TableRow)
            .count();
        let cells = frame
            .semantics
            .iter()
            .filter(|node| node.role == SemanticRole::Cell)
            .count();

        assert_eq!(rows, 9);
        assert_eq!(cells, 27);
        assert!(builds.get() < 40);
        assert!(harness.event(&Event::PointerDown {
            position: Point::new(20.0, 48.0),
            button: PointerButton::Primary,
            click_count: 1,
        }));
        assert_eq!(
            selections.borrow().last().map(String::as_str),
            Some("file-0")
        );
    }

    #[test]
    fn table_headers_sort_resize_and_reorder() {
        use std::cell::RefCell;

        let sorts = Rc::new(RefCell::new(Vec::new()));
        let sort_events = sorts.clone();
        let orders = Rc::new(RefCell::new(Vec::<Vec<String>>::new()));
        let order_events = orders.clone();
        let table = TableView::new(
            20,
            [
                TableColumn::new("name", "Name", 120.0).width_range(70.0, 240.0),
                TableColumn::new("size", "Size", 100.0),
                TableColumn::new("hidden", "Hidden", 80.0).hidden(true),
            ],
            |row, column| format!("{}-{row}", column.as_str()),
        )
        .viewport_height(160.0)
        .on_sort_change(move |column, direction| {
            sort_events
                .borrow_mut()
                .push((column.as_str().to_owned(), direction));
        })
        .on_column_order_change(move |columns| {
            order_events.borrow_mut().push(
                columns
                    .iter()
                    .map(|column| column.as_str().to_owned())
                    .collect(),
            );
        });
        let mut harness = TestHarness::new(table, Size::new(260.0, 160.0));
        let initial = harness.frame();
        assert!(initial.semantics.iter().all(|node| node.label != "Hidden"));

        click(&mut harness, Point::new(24.0, 16.0));
        assert_eq!(
            sorts.borrow().last().cloned(),
            Some(("name".to_owned(), Some(SortDirection::Ascending)))
        );

        harness.event(&Event::PointerDown {
            position: Point::new(120.0, 16.0),
            button: PointerButton::Primary,
            click_count: 1,
        });
        harness.event(&Event::PointerMoved(Point::new(160.0, 16.0)));
        harness.event(&Event::PointerUp {
            position: Point::new(160.0, 16.0),
            button: PointerButton::Primary,
        });
        let resized = harness.frame();
        let name_header = resized
            .semantics
            .iter()
            .find(|node| node.role == SemanticRole::ColumnHeader && node.label == "Name")
            .expect("name header");
        assert!((name_header.bounds.width - 160.0).abs() < f64::EPSILON);

        harness.event(&Event::PointerDown {
            position: Point::new(40.0, 16.0),
            button: PointerButton::Primary,
            click_count: 1,
        });
        harness.event(&Event::PointerMoved(Point::new(190.0, 16.0)));
        harness.event(&Event::PointerUp {
            position: Point::new(190.0, 16.0),
            button: PointerButton::Primary,
        });
        assert_eq!(
            orders.borrow().last().cloned(),
            Some(vec![
                "size".to_owned(),
                "name".to_owned(),
                "hidden".to_owned()
            ])
        );
    }

    #[test]
    fn table_view_scrolls_wide_cells_under_frozen_headers() {
        let table = TableView::new(
            100,
            [
                TableColumn::new("alpha", "Alpha", 180.0),
                TableColumn::new("beta", "Beta", 180.0),
                TableColumn::new("gamma", "Gamma", 180.0),
            ],
            |row, column| format!("{}-{row}", column.as_str()),
        )
        .viewport_height(160.0);
        let mut harness = TestHarness::new(table, Size::new(250.0, 160.0));
        let initial = harness.frame();
        let initial_cell = initial
            .semantics
            .iter()
            .find(|node| node.role == SemanticRole::Cell && node.label == "alpha-0")
            .expect("first cell");
        let header_y = semantic_bounds(&initial, "Alpha").expect("header").y;

        assert!(harness.event(&Event::Scroll {
            position: Point::new(100.0, 80.0),
            delta_x: 120.0,
            delta_y: 0.0,
        }));
        let scrolled = harness.frame();
        let scrolled_cell = scrolled
            .semantics
            .iter()
            .find(|node| node.role == SemanticRole::Cell && node.label == "alpha-0")
            .expect("scrolled first cell");
        let scrolled_header = semantic_bounds(&scrolled, "Alpha").expect("header");
        assert!((scrolled_cell.bounds.x - initial_cell.bounds.x + 120.0).abs() < f64::EPSILON);
        assert!((scrolled_header.y - header_y).abs() < f64::EPSILON);
    }

    #[test]
    fn pointer_capture_returns_release_to_the_pressed_widget() {
        let first_count = Rc::new(Cell::new(0));
        let second_count = Rc::new(Cell::new(0));
        let first_callback = first_count.clone();
        let second_callback = second_count.clone();
        let root = Row::new([
            Box::new(Button::new("First", move || {
                first_callback.set(first_callback.get() + 1);
            })) as Box<dyn Widget>,
            Box::new(Button::new("Second", move || {
                second_callback.set(second_callback.get() + 1);
            })) as Box<dyn Widget>,
        ]);
        let mut harness = TestHarness::new(root, Size::new(240.0, 80.0));
        let _initial = harness.frame();

        assert!(harness.event(&Event::PointerDown {
            position: Point::new(20.0, 20.0),
            button: PointerButton::Primary,
            click_count: 1,
        }));
        harness.event(&Event::PointerMoved(Point::new(120.0, 20.0)));
        assert!(!harness.event(&Event::PointerUp {
            position: Point::new(120.0, 20.0),
            button: PointerButton::Primary,
        }));
        assert_eq!(first_count.get(), 0);
        assert_eq!(second_count.get(), 0);

        assert!(harness.event(&Event::PointerDown {
            position: Point::new(120.0, 20.0),
            button: PointerButton::Primary,
            click_count: 1,
        }));
        assert!(harness.event(&Event::PointerUp {
            position: Point::new(120.0, 20.0),
            button: PointerButton::Primary,
        }));
        assert_eq!(second_count.get(), 1);
    }

    #[test]
    fn overlay_receives_pointer_events_before_overlapping_base() {
        let base_count = Rc::new(Cell::new(0));
        let overlay_count = Rc::new(Cell::new(0));
        let base_callback = base_count.clone();
        let overlay_callback = overlay_count.clone();
        let base = ScrollView::new(Button::new("Base", move || {
            base_callback.set(base_callback.get() + 1);
        }))
        .viewport_width(240.0)
        .viewport_height(120.0);
        let root = OverlayHost::new(base).gap(0.0).overlay(
            Rect::new(0.0, 0.0, 88.0, 0.0),
            OverlayPlacement::Below,
            Button::new("Popup", move || {
                overlay_callback.set(overlay_callback.get() + 1);
            }),
        );
        let mut harness = TestHarness::new(root, Size::new(240.0, 120.0));
        let _initial = harness.frame();

        assert!(harness.event(&Event::PointerDown {
            position: Point::new(20.0, 20.0),
            button: PointerButton::Primary,
            click_count: 1,
        }));
        assert!(harness.event(&Event::PointerUp {
            position: Point::new(20.0, 20.0),
            button: PointerButton::Primary,
        }));
        assert_eq!(overlay_count.get(), 1);
        assert_eq!(base_count.get(), 0);
    }

    #[test]
    fn modal_overlay_dismisses_outside_and_restores_base_routing() {
        let base_count = Rc::new(Cell::new(0));
        let dismiss_count = Rc::new(Cell::new(0));
        let base_callback = base_count.clone();
        let dismiss_callback = dismiss_count.clone();
        let base = ScrollView::new(Button::new("Base", move || {
            base_callback.set(base_callback.get() + 1);
        }))
        .viewport_width(240.0)
        .viewport_height(120.0);
        let root = OverlayHost::new(base)
            .centered_modal(Button::new("Dialog", || {}))
            .on_dismiss(move || dismiss_callback.set(dismiss_callback.get() + 1));
        let mut harness = TestHarness::new(root, Size::new(240.0, 120.0));
        let modal = harness.frame();
        assert!(modal.semantics.iter().any(|node| node.label == "Dialog"));
        assert!(!modal.semantics.iter().any(|node| node.label == "Base"));

        assert!(harness.event(&Event::PointerDown {
            position: Point::new(10.0, 10.0),
            button: PointerButton::Primary,
            click_count: 1,
        }));
        assert_eq!(dismiss_count.get(), 1);
        assert!(
            harness
                .frame()
                .semantics
                .iter()
                .any(|node| node.label == "Base")
        );

        assert!(harness.event(&Event::PointerDown {
            position: Point::new(20.0, 20.0),
            button: PointerButton::Primary,
            click_count: 1,
        }));
        assert!(harness.event(&Event::PointerUp {
            position: Point::new(20.0, 20.0),
            button: PointerButton::Primary,
        }));
        assert_eq!(base_count.get(), 1);
    }

    #[test]
    fn overlay_flips_away_from_viewport_edge() {
        let base = ScrollView::new(Label::new("Base"))
            .viewport_width(200.0)
            .viewport_height(100.0);
        let root = OverlayHost::new(base).overlay(
            Rect::new(20.0, 90.0, 80.0, 10.0),
            OverlayPlacement::Below,
            Button::new("Popup", || {}),
        );
        let mut harness = TestHarness::new(root, Size::new(200.0, 100.0));
        let popup = harness
            .frame()
            .semantics
            .into_iter()
            .find(|node| node.label == "Popup")
            .expect("popup semantics");
        assert!(popup.bounds.y < 90.0);
        assert!(popup.bounds.y >= 0.0);
        assert!(popup.bounds.y + popup.bounds.height <= 100.0);
    }

    #[test]
    fn button_command_opens_live_anchored_overlay_and_restores_focus() {
        let opener = Button::action("Open", |anchor, ctx| {
            ctx.show_overlay(
                anchor,
                OverlayPlacement::Below,
                Button::new("Popup action", || {}),
            );
        });
        let base = ScrollView::new(opener)
            .viewport_width(240.0)
            .viewport_height(120.0);
        let mut harness = TestHarness::new(OverlayHost::new(base), Size::new(240.0, 120.0));
        let _initial = harness.frame();

        assert!(harness.event(&Event::PointerDown {
            position: Point::new(20.0, 20.0),
            button: PointerButton::Primary,
            click_count: 1,
        }));
        assert!(harness.event(&Event::PointerUp {
            position: Point::new(20.0, 20.0),
            button: PointerButton::Primary,
        }));
        let open = harness.frame();
        let anchor = open
            .semantics
            .iter()
            .find(|node| node.label == "Open")
            .expect("open button");
        let popup = open
            .semantics
            .iter()
            .find(|node| node.label == "Popup action")
            .expect("popup button");
        assert!(popup.focused);
        assert!(popup.bounds.y >= anchor.bounds.y + anchor.bounds.height);

        assert!(harness.event(&Event::KeyDown(Key::Escape)));
        let dismissed = harness.frame();
        assert!(
            !dismissed
                .semantics
                .iter()
                .any(|node| node.label == "Popup action")
        );
        assert!(
            dismissed
                .semantics
                .iter()
                .any(|node| node.label == "Open" && node.focused)
        );
    }

    #[test]
    fn live_overlay_anchor_tracks_scrolled_widget() {
        let opener = Button::action("Open", |anchor, ctx| {
            ctx.show_overlay(
                anchor,
                OverlayPlacement::Right,
                Button::new("Following popup", || {}),
            );
        });
        let content = Column::new([
            Box::new(Label::new("Spacer one")) as Box<dyn Widget>,
            Box::new(Label::new("Spacer two")) as Box<dyn Widget>,
            Box::new(opener) as Box<dyn Widget>,
            Box::new(Label::new("Trailing one")) as Box<dyn Widget>,
            Box::new(Label::new("Trailing two")) as Box<dyn Widget>,
            Box::new(Label::new("Trailing three")) as Box<dyn Widget>,
        ]);
        let base = ScrollView::new(content)
            .viewport_width(260.0)
            .viewport_height(100.0);
        let mut harness = TestHarness::new(OverlayHost::new(base), Size::new(260.0, 100.0));
        let initial = harness.frame();
        let opener_bounds = initial
            .semantics
            .iter()
            .find(|node| node.label == "Open")
            .expect("open button")
            .bounds;
        let click = Point::new(opener_bounds.x + 10.0, opener_bounds.y + 10.0);
        assert!(harness.event(&Event::PointerDown {
            position: click,
            button: PointerButton::Primary,
            click_count: 1,
        }));
        assert!(harness.event(&Event::PointerUp {
            position: click,
            button: PointerButton::Primary,
        }));
        let before = semantic_label_y(&harness.frame(), "Following popup").expect("popup");

        assert!(harness.event(&Event::Scroll {
            position: Point::new(10.0, 90.0),
            delta_x: 0.0,
            delta_y: 30.0,
        }));
        let after = semantic_label_y(&harness.frame(), "Following popup").expect("popup");
        assert!(after < before);
    }

    #[test]
    fn modal_command_traps_tab_and_close_command_restores_focus() {
        let opener = Button::action("Open dialog", |_, ctx| {
            ctx.show_modal(Column::new([
                Box::new(Button::new("Confirm", || {})) as Box<dyn Widget>,
                Box::new(Button::action("Close", |_, ctx| ctx.dismiss_overlay()))
                    as Box<dyn Widget>,
            ]));
        });
        let base = ScrollView::new(opener)
            .viewport_width(240.0)
            .viewport_height(160.0);
        let mut harness = TestHarness::new(OverlayHost::new(base), Size::new(240.0, 160.0));
        let _initial = harness.frame();
        assert!(harness.event(&Event::PointerDown {
            position: Point::new(20.0, 20.0),
            button: PointerButton::Primary,
            click_count: 1,
        }));
        assert!(harness.event(&Event::PointerUp {
            position: Point::new(20.0, 20.0),
            button: PointerButton::Primary,
        }));
        assert_eq!(focused_label(&harness.frame()).as_deref(), Some("Confirm"));
        assert!(harness.event(&Event::KeyDown(Key::Tab)));
        assert_eq!(focused_label(&harness.frame()).as_deref(), Some("Close"));
        assert!(harness.event(&Event::KeyDown(Key::Enter)));
        assert_eq!(
            focused_label(&harness.frame()).as_deref(),
            Some("Open dialog")
        );
    }

    #[test]
    fn menu_button_supports_roving_keys_typeahead_and_activation() {
        let activations = Rc::new(Cell::new(0));
        let callback_count = activations.clone();
        let button = MenuButton::new("Open menu", move || {
            let alpha_count = callback_count.clone();
            let beta_count = callback_count.clone();
            Menu::new([
                MenuItem::new("Unavailable", || {}).disabled(true),
                MenuItem::separator(),
                MenuItem::new("Alpha", move || alpha_count.set(alpha_count.get() + 1)),
                MenuItem::new("Beta", move || beta_count.set(beta_count.get() + 10)),
            ])
        });
        let base = ScrollView::new(button)
            .viewport_width(260.0)
            .viewport_height(180.0);
        let mut harness = TestHarness::new(OverlayHost::new(base), Size::new(260.0, 180.0));
        let _initial = harness.frame();

        click(&mut harness, Point::new(20.0, 20.0));
        assert_eq!(focused_label(&harness.frame()).as_deref(), Some("Alpha"));
        assert!(harness.event(&Event::KeyDown(Key::ArrowDown)));
        assert_eq!(focused_label(&harness.frame()).as_deref(), Some("Beta"));
        assert!(harness.event(&Event::KeyDown(Key::Enter)));
        assert_eq!(activations.get(), 10);
        assert_eq!(
            focused_label(&harness.frame()).as_deref(),
            Some("Open menu")
        );

        click(&mut harness, Point::new(20.0, 20.0));
        assert!(harness.event(&Event::KeyDown(Key::Character("b".to_owned()))));
        assert_eq!(focused_label(&harness.frame()).as_deref(), Some("Beta"));
        assert!(harness.event(&Event::KeyDown(Key::Enter)));
        assert_eq!(activations.get(), 20);
    }

    #[test]
    fn checked_and_radio_menu_items_publish_state() {
        let checked_value = Rc::new(Cell::new(true));
        let callback_value = checked_value.clone();
        let button = MenuButton::new("Options", move || {
            let callback_value = callback_value.clone();
            Menu::new([
                MenuItem::checkbox("Show panel", true, move |checked| {
                    callback_value.set(checked);
                }),
                MenuItem::radio("Compact", false, || {}),
            ])
        });
        let base = ScrollView::new(button)
            .viewport_width(240.0)
            .viewport_height(160.0);
        let mut harness = TestHarness::new(OverlayHost::new(base), Size::new(240.0, 160.0));
        let _initial = harness.frame();
        click(&mut harness, Point::new(20.0, 20.0));

        let open = harness.frame();
        assert!(open.semantics.iter().any(|node| {
            node.role == SemanticRole::MenuItemCheckbox
                && node.label == "Show panel"
                && node.checked == Some(true)
        }));
        assert!(open.semantics.iter().any(|node| {
            node.role == SemanticRole::MenuItemRadio
                && node.label == "Compact"
                && node.checked == Some(false)
        }));
        assert!(harness.event(&Event::KeyDown(Key::Enter)));
        assert!(!checked_value.get());
    }

    #[test]
    fn context_menu_opens_at_pointer_and_stays_in_viewport() {
        let base = ScrollView::new(Label::new("Right-click surface"))
            .viewport_width(240.0)
            .viewport_height(120.0);
        let contextual =
            ContextMenu::new(base, || Menu::new([MenuItem::new("Context action", || {})]));
        let mut harness = TestHarness::new(OverlayHost::new(contextual), Size::new(240.0, 120.0));
        let _initial = harness.frame();
        assert!(harness.event(&Event::PointerDown {
            position: Point::new(230.0, 110.0),
            button: PointerButton::Secondary,
            click_count: 1,
        }));
        let frame = harness.frame();
        let item = frame
            .semantics
            .iter()
            .find(|node| node.label == "Context action")
            .expect("context menu item");
        assert!(item.focused);
        assert!(item.bounds.x >= 0.0);
        assert!(item.bounds.y >= 0.0);
        assert!(item.bounds.x + item.bounds.width <= 240.0);
        assert!(item.bounds.y + item.bounds.height <= 120.0);
    }

    #[test]
    fn app_messages_update_model_and_rebuild_visible_state() {
        #[derive(Clone, Copy)]
        enum Message {
            Increment,
        }
        let app = App::new(
            0_u32,
            |count, Message::Increment| *count += 1,
            |count, sender| {
                Column::new([
                    Box::new(Label::new(format!("Count: {count}"))) as Box<dyn Widget>,
                    Box::new(Button::action("Increment", move |_, ctx| {
                        sender.send(Message::Increment, ctx);
                    })) as Box<dyn Widget>,
                ])
            },
        );
        let mut harness = TestHarness::new(app, Size::new(240.0, 100.0));
        assert!(
            harness
                .frame()
                .semantics
                .iter()
                .any(|node| node.label == "Count: 0")
        );
        click(&mut harness, Point::new(20.0, 45.0));
        assert!(
            harness
                .frame()
                .semantics
                .iter()
                .any(|node| node.label == "Count: 1")
        );
    }

    #[test]
    fn keyed_editor_survives_conditional_reordering() {
        #[derive(Default)]
        struct Model {
            text: String,
            show_status: bool,
        }
        enum Message {
            Edited(String),
        }
        let app = App::new(
            Model::default(),
            |model, message| match message {
                Message::Edited(text) => {
                    model.text = text;
                    model.show_status = true;
                }
            },
            |model, sender| {
                let editor_sender = sender.clone();
                let editor = Box::new(Keyed::new(
                    "editor",
                    TextField::new(model.text.clone(), move |value| {
                        editor_sender.emit(Message::Edited(value.to_owned()));
                    }),
                )) as Box<dyn Widget>;
                let children = if model.show_status {
                    vec![Box::new(Label::new("Saved")) as Box<dyn Widget>, editor]
                } else {
                    vec![editor]
                };
                Column::new(children)
            },
        );
        let mut harness = TestHarness::new(app, Size::new(320.0, 140.0));
        let _initial = harness.frame();
        assert!(harness.event(&Event::KeyDown(Key::Tab)));
        assert!(harness.event(&Event::TextInput("é".to_owned())));
        let rebuilt = harness.frame();
        assert!(rebuilt.semantics.iter().any(|node| node.label == "Saved"));
        assert!(rebuilt.semantics.iter().any(|node| {
            node.role == SemanticRole::TextInput
                && node.focused
                && node.value.as_deref() == Some("é")
        }));
        assert!(harness.event(&Event::TextInput("!".to_owned())));
        assert_eq!(text_input_value(&harness.frame()).as_deref(), Some("é!"));
    }

    #[test]
    fn app_rebuild_preserves_scroll_offset() {
        #[derive(Clone, Copy)]
        enum Message {
            Refresh,
        }
        let app = App::new(
            0_u32,
            |version, Message::Refresh| *version += 1,
            |version, sender| {
                let mut children = (0..7)
                    .map(|index| {
                        Box::new(Label::new(format!("Row {index} v{version}"))) as Box<dyn Widget>
                    })
                    .collect::<Vec<_>>();
                children.push(Box::new(Button::action("Refresh", move |_, ctx| {
                    sender.send(Message::Refresh, ctx);
                })));
                ScrollView::new(Column::new(children))
                    .viewport_width(240.0)
                    .viewport_height(100.0)
            },
        );
        let mut harness = TestHarness::new(app, Size::new(240.0, 100.0));
        let _initial = harness.frame();
        assert!(harness.event(&Event::Scroll {
            position: Point::new(30.0, 50.0),
            delta_x: 0.0,
            delta_y: 1_000.0,
        }));
        let scrolled = harness.frame();
        let before = semantic_label_y(&scrolled, "Refresh").expect("refresh button");
        let refresh = scrolled
            .semantics
            .iter()
            .find(|node| node.label == "Refresh")
            .expect("refresh button")
            .bounds;
        click(&mut harness, Point::new(refresh.x + 10.0, refresh.y + 10.0));
        let rebuilt = harness.frame();
        let after = semantic_label_y(&rebuilt, "Refresh").expect("refresh button");
        assert!((after - before).abs() < f64::EPSILON);
        assert!(
            rebuilt
                .semantics
                .iter()
                .any(|node| node.label == "Row 0 v1")
        );
    }

    #[test]
    fn accessibility_action_activates_button() {
        let activations = Rc::new(Cell::new(0));
        let callback_count = activations.clone();
        let button = Button::new("Run", move || {
            callback_count.set(callback_count.get() + 1);
        });
        let mut harness = TestHarness::new(button, Size::new(200.0, 100.0));
        let frame = harness.frame();
        let target = frame
            .semantics
            .iter()
            .find(|node| node.role == SemanticRole::Button)
            .expect("button semantics")
            .id;

        assert!(harness.event(&Event::AccessibilityAction {
            target,
            action: SemanticAction::Click,
        }));
        assert_eq!(activations.get(), 1);
    }

    #[test]
    fn tab_cycles_focus_and_skips_disabled_controls() {
        let root = Column::new([
            Box::new(Button::new("First", || {})) as Box<dyn Widget>,
            Box::new(Button::new("Disabled", || {}).disabled(true)) as Box<dyn Widget>,
            Box::new(Checkbox::new("Third", false, |_| {})) as Box<dyn Widget>,
        ]);
        let mut harness = TestHarness::new(root, Size::new(300.0, 200.0));
        let _initial = harness.frame();

        assert!(harness.event(&Event::KeyDown(Key::Tab)));
        assert_eq!(focused_label(&harness.frame()).as_deref(), Some("First"));

        assert!(harness.event(&Event::KeyDown(Key::Tab)));
        assert_eq!(focused_label(&harness.frame()).as_deref(), Some("Third"));

        assert!(harness.event(&Event::KeyDown(Key::Tab)));
        assert_eq!(focused_label(&harness.frame()).as_deref(), Some("First"));
    }

    #[test]
    fn text_field_handles_unicode_editing_and_preedit() {
        let root = TextField::new("A", |_| {}).placeholder("Name");
        let mut harness = TestHarness::new(root, Size::new(300.0, 100.0));
        let _initial = harness.frame();

        assert!(harness.event(&Event::KeyDown(Key::Tab)));
        assert!(harness.event(&Event::TextInput("é".to_owned())));
        assert_eq!(text_input_value(&harness.frame()).as_deref(), Some("Aé"));

        assert!(harness.event(&Event::ImePreedit {
            text: "界".to_owned(),
            cursor: Some((0, 3)),
        }));
        let composing = harness.frame();
        assert_eq!(text_input_value(&composing).as_deref(), Some("Aé"));
        assert!(
            composing
                .semantics
                .iter()
                .any(|node| node.ime_cursor_area.is_some())
        );
        assert!(composing.paint.operations().iter().any(|operation| {
            matches!(operation, PaintOp::FillRect { rect, .. } if (rect.height - 3.0).abs() < f64::EPSILON)
        }));

        assert!(harness.event(&Event::KeyDown(Key::Backspace)));
        assert_eq!(text_input_value(&harness.frame()).as_deref(), Some("A"));
    }

    #[test]
    fn text_field_selection_drives_clipboard_commands() {
        let root = TextField::new("Hello", |_| {});
        let mut harness = TestHarness::new(root, Size::new(300.0, 100.0));
        let _initial = harness.frame();
        assert!(harness.event(&Event::KeyDown(Key::Tab)));
        let mut modifiers = Modifiers::default();
        modifiers.set(Modifier::Control, true);
        harness.event(&Event::ModifiersChanged(modifiers));

        assert!(harness.event(&Event::KeyDown(Key::Character("a".to_owned()))));
        assert!(harness.event(&Event::KeyDown(Key::Character("c".to_owned()))));
        assert_eq!(
            harness.take_clipboard_request(),
            Some(ClipboardRequest::Write("Hello".to_owned()))
        );

        assert!(harness.event(&Event::KeyDown(Key::Character("x".to_owned()))));
        assert_eq!(
            harness.take_clipboard_request(),
            Some(ClipboardRequest::Write("Hello".to_owned()))
        );
        assert_eq!(text_input_value(&harness.frame()).as_deref(), Some(""));

        assert!(harness.event(&Event::KeyDown(Key::Character("v".to_owned()))));
        assert_eq!(
            harness.take_clipboard_request(),
            Some(ClipboardRequest::Read)
        );
        assert!(harness.event(&Event::TextInput("Restored".to_owned())));
        assert_eq!(
            text_input_value(&harness.frame()).as_deref(),
            Some("Restored")
        );
    }

    #[test]
    fn text_area_edits_and_navigates_multiple_lines() {
        let root = TextArea::new("one\ntwo", |_| {}).min_lines(3);
        let mut harness = TestHarness::new(root, Size::new(400.0, 220.0));
        let _initial = harness.frame();

        assert!(harness.event(&Event::KeyDown(Key::Tab)));
        assert!(harness.event(&Event::KeyDown(Key::Enter)));
        assert!(harness.event(&Event::TextInput("three".to_owned())));
        assert!(harness.event(&Event::KeyDown(Key::ArrowUp)));
        assert!(harness.event(&Event::TextInput("X".to_owned())));

        let frame = harness.frame();
        let area = frame
            .semantics
            .iter()
            .find(|node| node.role == SemanticRole::MultilineTextInput)
            .expect("text area semantics");
        assert_eq!(area.value.as_deref(), Some("one\ntwoX\nthree"));
        assert!(area.focused);
    }

    #[test]
    fn command_navigation_and_deletion_use_word_boundaries() {
        let root = TextField::new("alpha beta", |_| {});
        let mut harness = TestHarness::new(root, Size::new(300.0, 100.0));
        let _initial = harness.frame();
        assert!(harness.event(&Event::KeyDown(Key::Tab)));
        let mut modifiers = Modifiers::default();
        modifiers.set(Modifier::Control, true);
        harness.event(&Event::ModifiersChanged(modifiers));

        assert!(harness.event(&Event::KeyDown(Key::ArrowLeft)));
        assert!(harness.event(&Event::TextInput("X".to_owned())));
        assert_eq!(
            text_input_value(&harness.frame()).as_deref(),
            Some("alpha Xbeta")
        );

        assert!(harness.event(&Event::KeyDown(Key::Delete)));
        assert_eq!(
            text_input_value(&harness.frame()).as_deref(),
            Some("alpha X")
        );
    }

    #[test]
    fn undo_and_redo_restore_editor_state() {
        let root = TextField::new("alpha", |_| {});
        let mut harness = TestHarness::new(root, Size::new(300.0, 100.0));
        let _initial = harness.frame();
        assert!(harness.event(&Event::KeyDown(Key::Tab)));
        assert!(harness.event(&Event::TextInput("X".to_owned())));
        assert!(harness.event(&Event::TextInput("Y".to_owned())));

        let mut modifiers = Modifiers::default();
        modifiers.set(Modifier::Control, true);
        harness.event(&Event::ModifiersChanged(modifiers));
        assert!(harness.event(&Event::KeyDown(Key::Character("z".to_owned()))));
        assert_eq!(text_input_value(&harness.frame()).as_deref(), Some("alpha"));

        assert!(harness.event(&Event::KeyDown(Key::Character("y".to_owned()))));
        assert_eq!(
            text_input_value(&harness.frame()).as_deref(),
            Some("alphaXY")
        );
    }

    #[test]
    fn double_click_selects_a_unicode_word() {
        let root = TextField::new("alpha beta", |_| {});
        let mut harness = TestHarness::new(root, Size::new(300.0, 100.0));
        let _initial = harness.frame();
        assert!(harness.event(&Event::PointerDown {
            position: Point::new(32.0, 20.0),
            button: PointerButton::Primary,
            click_count: 2,
        }));
        assert!(harness.event(&Event::TextInput("X".to_owned())));
        assert_eq!(
            text_input_value(&harness.frame()).as_deref(),
            Some("X beta")
        );
    }

    #[test]
    fn text_area_consumes_scroll_inside_its_bounds() {
        let root = TextArea::new("one\ntwo\nthree\nfour\nfive\nsix", |_| {});
        let mut harness = TestHarness::new(root, Size::new(300.0, 100.0));
        let initial = harness.frame();
        assert!(initial.paint.operations().iter().any(|operation| {
            matches!(operation, PaintOp::FillRect { rect, .. } if (rect.width - 3.0).abs() < f64::EPSILON)
        }));
        assert!(harness.event(&Event::Scroll {
            position: Point::new(20.0, 20.0),
            delta_x: 0.0,
            delta_y: -40.0,
        }));
    }

    #[test]
    fn triple_click_selects_a_hard_line() {
        let root = TextArea::new("one\ntwo\nthree", |_| {});
        let mut harness = TestHarness::new(root, Size::new(300.0, 160.0));
        let _initial = harness.frame();
        assert!(harness.event(&Event::PointerDown {
            position: Point::new(24.0, 34.0),
            button: PointerButton::Primary,
            click_count: 3,
        }));
        assert!(harness.event(&Event::TextInput("X".to_owned())));
        assert_eq!(
            text_area_value(&harness.frame()).as_deref(),
            Some("one\nXthree")
        );
    }

    #[test]
    fn page_navigation_moves_within_multiline_content() {
        let original = "one\ntwo\nthree\nfour\nfive\nsix";
        let root = TextArea::new(original, |_| {});
        let mut harness = TestHarness::new(root, Size::new(300.0, 90.0));
        let _initial = harness.frame();
        assert!(harness.event(&Event::KeyDown(Key::Tab)));
        assert!(harness.event(&Event::KeyDown(Key::PageUp)));
        assert!(harness.event(&Event::TextInput("X".to_owned())));
        let value = text_area_value(&harness.frame()).expect("text area value");
        assert_ne!(value, format!("{original}X"));
        assert!(value.contains('X'));
    }

    #[test]
    fn visual_home_and_command_home_use_different_edges() {
        let root = TextArea::new("alpha beta gamma delta", |_| {});
        let mut harness = TestHarness::new(root, Size::new(110.0, 140.0));
        let _initial = harness.frame();
        assert!(harness.event(&Event::KeyDown(Key::Tab)));

        assert!(harness.event(&Event::KeyDown(Key::Home)));
        assert!(harness.event(&Event::TextInput("X".to_owned())));
        let visual_home = text_area_value(&harness.frame()).expect("text area value");
        assert!(!visual_home.starts_with('X'));

        let mut modifiers = Modifiers::default();
        modifiers.set(Modifier::Control, true);
        harness.event(&Event::ModifiersChanged(modifiers));
        assert!(harness.event(&Event::KeyDown(Key::Home)));
        harness.event(&Event::ModifiersChanged(Modifiers::default()));
        assert!(harness.event(&Event::TextInput("Y".to_owned())));
        assert!(
            text_area_value(&harness.frame())
                .expect("text area value")
                .starts_with('Y')
        );
    }

    #[test]
    fn vertical_scrollbar_thumb_can_be_dragged() {
        let root = TextArea::new("one\ntwo\nthree\nfour\nfive\nsix", |_| {});
        let mut harness = TestHarness::new(root, Size::new(300.0, 100.0));
        let initial = harness.frame();
        let thumb = vertical_thumb(&initial).expect("vertical scrollbar thumb");
        let position = Point::new(thumb.x + thumb.width * 0.5, thumb.y + thumb.height * 0.5);
        assert!(harness.event(&Event::PointerDown {
            position,
            button: PointerButton::Primary,
            click_count: 1,
        }));
        assert!(harness.event(&Event::PointerMoved(Point::new(
            position.x,
            position.y - 24.0,
        ))));
        assert!(harness.event(&Event::PointerUp {
            position: Point::new(position.x, position.y - 24.0),
            button: PointerButton::Primary,
        }));
        let moved = vertical_thumb(&harness.frame()).expect("vertical scrollbar thumb");
        assert!(moved.y < thumb.y);
    }

    #[test]
    fn scrollbar_track_click_pages_the_viewport() {
        let root = TextArea::new("one\ntwo\nthree\nfour\nfive\nsix", |_| {});
        let mut harness = TestHarness::new(root, Size::new(300.0, 100.0));
        let initial = harness.frame();
        let thumb = vertical_thumb(&initial).expect("vertical scrollbar thumb");
        assert!(harness.event(&Event::PointerDown {
            position: Point::new(thumb.x + 1.0, 7.0),
            button: PointerButton::Primary,
            click_count: 1,
        }));
        let paged = vertical_thumb(&harness.frame()).expect("vertical scrollbar thumb");
        assert!(paged.y < thumb.y);
    }

    #[test]
    fn drag_selection_auto_scrolls_past_the_viewport_edge() {
        let root = TextArea::new("one\ntwo\nthree\nfour\nfive\nsix", |_| {});
        let mut harness = TestHarness::new(root, Size::new(300.0, 100.0));
        let initial = harness.frame();
        let thumb = vertical_thumb(&initial).expect("vertical scrollbar thumb");
        assert!(harness.event(&Event::PointerDown {
            position: Point::new(40.0, 70.0),
            button: PointerButton::Primary,
            click_count: 1,
        }));
        assert!(harness.event(&Event::PointerMoved(Point::new(40.0, -20.0))));
        let scrolled = vertical_thumb(&harness.frame()).expect("vertical scrollbar thumb");
        assert!(scrolled.y < thumb.y);
    }

    #[test]
    fn scroll_view_clips_and_scrolls_arbitrary_content() {
        let content = Column::new((0..8).map(|index| {
            Box::new(Button::new(format!("Action {index}"), || {})) as Box<dyn Widget>
        }));
        let root = ScrollView::new(content)
            .viewport_width(240.0)
            .viewport_height(100.0);
        let mut harness = TestHarness::new(root, Size::new(240.0, 100.0));
        let initial = harness.frame();
        let initial_y = semantic_label_y(&initial, "Action 0").expect("first action");
        assert!(
            initial
                .paint
                .operations()
                .iter()
                .any(|operation| matches!(operation, PaintOp::PushClip { .. }))
        );

        assert!(harness.event(&Event::Scroll {
            position: Point::new(40.0, 40.0),
            delta_x: 0.0,
            delta_y: 60.0,
        }));
        let scrolled_y = semantic_label_y(&harness.frame(), "Action 0").expect("first action");
        assert!(scrolled_y < initial_y);
    }

    #[test]
    fn scroll_view_track_pages_and_thumb_drags() {
        let content =
            Column::new((0..8).map(|index| {
                Box::new(Button::new(format!("Item {index}"), || {})) as Box<dyn Widget>
            }));
        let root = ScrollView::new(content)
            .viewport_width(240.0)
            .viewport_height(100.0);
        let mut harness = TestHarness::new(root, Size::new(240.0, 100.0));
        let initial_thumb = scroll_view_vertical_thumb(&harness.frame()).expect("scroll thumb");

        assert!(harness.event(&Event::PointerDown {
            position: Point::new(initial_thumb.x + 2.0, 90.0),
            button: PointerButton::Primary,
            click_count: 1,
        }));
        let paged_thumb = scroll_view_vertical_thumb(&harness.frame()).expect("scroll thumb");
        assert!(paged_thumb.y > initial_thumb.y);

        let grab = Point::new(
            paged_thumb.x + paged_thumb.width * 0.5,
            paged_thumb.y + paged_thumb.height * 0.5,
        );
        assert!(harness.event(&Event::PointerDown {
            position: grab,
            button: PointerButton::Primary,
            click_count: 1,
        }));
        assert!(harness.event(&Event::PointerMoved(Point::new(grab.x, grab.y - 20.0,))));
        assert!(harness.event(&Event::PointerUp {
            position: Point::new(grab.x, grab.y - 20.0),
            button: PointerButton::Primary,
        }));
        let dragged_thumb = scroll_view_vertical_thumb(&harness.frame()).expect("scroll thumb");
        assert!(dragged_thumb.y < paged_thumb.y);
    }

    #[test]
    fn nested_scroll_view_bubbles_wheel_at_its_boundary() {
        let inner = ScrollView::new(Column::new((0..6).map(|index| {
            Box::new(Button::new(format!("Inner {index}"), || {})) as Box<dyn Widget>
        })))
        .viewport_height(80.0);
        let outer_content = Column::new([
            Box::new(Label::new("Before")) as Box<dyn Widget>,
            Box::new(inner) as Box<dyn Widget>,
            Box::new(Label::new("After")) as Box<dyn Widget>,
            Box::new(Label::new("More one")) as Box<dyn Widget>,
            Box::new(Label::new("More two")) as Box<dyn Widget>,
            Box::new(Label::new("More three")) as Box<dyn Widget>,
        ]);
        let root = ScrollView::new(outer_content).viewport_height(120.0);
        let mut harness = TestHarness::new(root, Size::new(260.0, 120.0));
        let initial = harness.frame();
        let after_y = semantic_label_y(&initial, "After").expect("after label");

        assert!(harness.event(&Event::Scroll {
            position: Point::new(30.0, 50.0),
            delta_x: 0.0,
            delta_y: 1_000.0,
        }));
        let after_inner_scroll = semantic_label_y(&harness.frame(), "After").expect("after label");
        assert!((after_inner_scroll - after_y).abs() < f64::EPSILON);

        assert!(harness.event(&Event::Scroll {
            position: Point::new(30.0, 50.0),
            delta_x: 0.0,
            delta_y: 40.0,
        }));
        assert!(semantic_label_y(&harness.frame(), "After").expect("after label") < after_y);
    }

    struct WindowCommandProbe {
        child: Rc<Cell<Option<vui_core::WindowId>>>,
        bounds: Rect,
    }

    impl Widget for WindowCommandProbe {
        fn layout(
            &mut self,
            _ctx: &mut vui_core::LayoutCtx<'_>,
            constraints: vui_core::BoxConstraints,
        ) -> Size {
            let size = constraints.constrain(Size::new(100.0, 40.0));
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

        fn event(&mut self, ctx: &mut vui_core::EventCtx, event: &Event) {
            if matches!(event, Event::WindowCloseRequested) {
                ctx.set_handled();
                return;
            }
            let Event::KeyDown(Key::Character(key)) = event else {
                return;
            };
            match key.as_str() {
                "open" => {
                    let descriptor =
                        vui_core::WindowDescriptor::new("Inspector", Size::new(320.0, 240.0))
                            .modal(true);
                    let id = ctx.open_window(descriptor, Label::new("Detached inspector"));
                    self.child.set(Some(id));
                }
                "focus" => {
                    if let Some(id) = self.child.get() {
                        ctx.window_command(vui_core::WindowCommand::Focus(id));
                    }
                }
                "state" => {
                    if let Some(id) = self.child.get() {
                        ctx.window_command(vui_core::WindowCommand::Minimize(id, true));
                        ctx.window_command(vui_core::WindowCommand::Maximize(id, true));
                        ctx.window_command(vui_core::WindowCommand::Fullscreen(id, true));
                        ctx.window_command(vui_core::WindowCommand::RequestAttention(id));
                    }
                }
                "close" => {
                    if let Some(id) = self.child.get() {
                        ctx.window_command(vui_core::WindowCommand::Close(id));
                    }
                }
                "services" => {
                    ctx.open_uri("https://example.com");
                    ctx.reveal_path("/tmp/example.txt");
                    ctx.show_notification("Done", "Export finished");
                    ctx.request_file_dialog(vui_core::FileDialogRequest::new(
                        vui_core::FileDialogKind::OpenFile,
                    ));
                }
                "singleton" => {
                    ctx.open_window(
                        vui_core::WindowDescriptor::new("Singleton", Size::new(300.0, 200.0))
                            .key("singleton")
                            .singleton(true),
                        Label::new("Only one"),
                    );
                }
                _ => return,
            }
            ctx.set_handled();
        }

        fn paint(&self, _ctx: &mut vui_core::PaintCtx<'_>) {}

        fn semantics(&self, _ctx: &mut vui_core::SemanticsCtx<'_>) {}
    }

    #[test]
    fn multi_window_harness_owns_independent_runtimes_and_window_state() {
        let child = Rc::new(Cell::new(None));
        let descriptor = vui_core::WindowDescriptor::new("Workspace", Size::new(800.0, 600.0));
        let (mut harness, primary) = MultiWindowHarness::new(
            descriptor,
            WindowCommandProbe {
                child: child.clone(),
                bounds: Rect::default(),
            },
        );

        assert!(harness.event(primary, &Event::KeyDown(Key::Character("open".into()))));
        let inspector = child.get().expect("child window ID");
        assert_eq!(harness.window_count(), 2);
        assert_eq!(
            harness
                .descriptor(inspector)
                .map(|value| value.title.as_str()),
            Some("Inspector")
        );
        assert_eq!(
            harness.descriptor(inspector).and_then(|value| value.parent),
            Some(primary)
        );
        assert!(harness.frame(primary).is_some());
        assert!(harness.frame(inspector).is_some());

        harness.event(primary, &Event::KeyDown(Key::Character("focus".into())));
        harness.event(primary, &Event::KeyDown(Key::Character("state".into())));
        assert!(harness.is_focused(inspector));
        assert!(harness.is_minimized(inspector));
        assert!(harness.is_maximized(inspector));
        assert!(harness.is_fullscreen(inspector));
        assert!(harness.attention_requested(inspector));

        harness.event(primary, &Event::KeyDown(Key::Character("close".into())));
        assert!(!harness.contains(inspector));
        assert_eq!(harness.window_count(), 1);

        harness.event(primary, &Event::KeyDown(Key::Character("singleton".into())));
        harness.event(primary, &Event::KeyDown(Key::Character("singleton".into())));
        assert_eq!(harness.window_count(), 2);
        assert!(harness.request_close(primary));
        assert!(harness.contains(primary));
    }

    #[test]
    fn multi_window_harness_retains_typed_platform_requests() {
        let descriptor = vui_core::WindowDescriptor::new("Workspace", Size::new(800.0, 600.0));
        let (mut harness, primary) = MultiWindowHarness::new(
            descriptor,
            WindowCommandProbe {
                child: Rc::new(Cell::new(None)),
                bounds: Rect::default(),
            },
        );
        harness.event(primary, &Event::KeyDown(Key::Character("services".into())));
        let requests = harness.take_platform_requests();
        assert_eq!(requests.len(), 4);
        assert!(requests.iter().all(|(source, _)| *source == primary));
        assert!(matches!(
            requests[0].1,
            vui_core::PlatformCommand::OpenUri(_)
        ));
        assert!(matches!(
            requests[3].1,
            vui_core::PlatformCommand::FileDialog(_)
        ));
    }

    #[test]
    fn desktop_app_updates_every_window_from_one_shared_model() {
        let desktop = DesktopApp::new(0_u32, |count, ()| *count += 1);
        let primary_view = desktop
            .window(|count, sender| Button::new(format!("Count {count}"), move || sender.emit(())));
        let (mut harness, primary) = MultiWindowHarness::new(
            vui_core::WindowDescriptor::new("Primary", Size::new(320.0, 200.0)),
            primary_view,
        );
        let secondary = harness.open_window(
            vui_core::WindowDescriptor::new("Inspector", Size::new(320.0, 200.0)),
            desktop.window(|count, _sender| Label::new(format!("Count {count}"))),
        );
        harness.frame(primary).expect("primary frame");
        harness.frame(secondary).expect("secondary frame");

        assert!(harness.event(
            primary,
            &Event::PointerDown {
                position: Point::new(10.0, 10.0),
                button: PointerButton::Primary,
                click_count: 1,
            },
        ));
        assert!(harness.event(
            primary,
            &Event::PointerUp {
                position: Point::new(10.0, 10.0),
                button: PointerButton::Primary,
            },
        ));

        let frame = harness.frame(secondary).expect("updated secondary frame");
        assert!(frame.semantics.iter().any(|node| node.label == "Count 1"));
    }

    fn focused_label(frame: &UiFrame) -> Option<String> {
        frame
            .semantics
            .iter()
            .find(|node| node.focused)
            .map(|node| node.label.clone())
    }

    fn click(harness: &mut TestHarness, position: Point) {
        assert!(harness.event(&Event::PointerDown {
            position,
            button: PointerButton::Primary,
            click_count: 1,
        }));
        assert!(harness.event(&Event::PointerUp {
            position,
            button: PointerButton::Primary,
        }));
    }

    fn text_input_value(frame: &UiFrame) -> Option<String> {
        frame
            .semantics
            .iter()
            .find(|node| node.role == SemanticRole::TextInput)
            .and_then(|node| node.value.clone())
    }

    fn text_area_value(frame: &UiFrame) -> Option<String> {
        frame
            .semantics
            .iter()
            .find(|node| node.role == SemanticRole::MultilineTextInput)
            .and_then(|node| node.value.clone())
    }

    fn vertical_thumb(frame: &UiFrame) -> Option<vui_core::Rect> {
        frame
            .paint
            .operations()
            .iter()
            .find_map(|operation| match operation {
                PaintOp::FillRect { rect, .. }
                    if (rect.width - 3.0).abs() < f64::EPSILON && rect.height >= 20.0 =>
                {
                    Some(*rect)
                }
                _ => None,
            })
    }

    fn semantic_label_y(frame: &UiFrame, label: &str) -> Option<f64> {
        frame
            .semantics
            .iter()
            .find(|node| node.label == label)
            .map(|node| node.bounds.y)
    }

    fn semantic_bounds(frame: &UiFrame, label: &str) -> Option<Rect> {
        frame
            .semantics
            .iter()
            .find(|node| node.label == label)
            .map(|node| node.bounds)
    }

    fn semantic_value(frame: &UiFrame, label: &str) -> Option<String> {
        frame
            .semantics
            .iter()
            .find(|node| node.label == label)
            .and_then(|node| node.value.clone())
    }

    fn button_fill(frame: &UiFrame, label: &str) -> Option<vui_core::Color> {
        let bounds = semantic_bounds(frame, label)?;
        frame
            .paint
            .operations()
            .iter()
            .find_map(|operation| match operation {
                PaintOp::FillRect { rect, color, .. }
                    if (rect.x - bounds.x).abs() < f64::EPSILON
                        && (rect.y - bounds.y).abs() < f64::EPSILON
                        && (rect.width - bounds.width).abs() < f64::EPSILON
                        && (rect.height - bounds.height).abs() < f64::EPSILON =>
                {
                    Some(*color)
                }
                _ => None,
            })
    }

    fn scroll_view_vertical_thumb(frame: &UiFrame) -> Option<vui_core::Rect> {
        frame
            .paint
            .operations()
            .iter()
            .find_map(|operation| match operation {
                PaintOp::FillRect { rect, .. }
                    if (rect.width - 4.0).abs() < f64::EPSILON && rect.height >= 20.0 =>
                {
                    Some(*rect)
                }
                _ => None,
            })
    }

    #[test]
    fn desktop_lifecycle_defers_install_until_primary_instance_wins() {
        let window = WindowId::next();
        let tray = TrayDescriptor::new("app", "Application", NativeMenu::new([]));
        let mut lifecycle = DesktopLifecycleHarness::new(window, "dev.vui.test")
            .arguments(["vui", "document.txt", "vui://settings", "--verbose"])
            .single_instance(true)
            .tray(tray)
            .close_to_tray(true);
        assert!(matches!(
            lifecycle.start().as_slice(),
            [PlatformRequest::RegisterSingleInstance { .. }]
        ));
        assert!(!lifecycle.is_visible());
        assert!(matches!(
            lifecycle
                .platform_event(PlatformEvent::SingleInstanceStatus { primary: true })
                .as_slice(),
            [PlatformRequest::SetTray(_)]
        ));
        assert!(lifecycle.is_visible());
        lifecycle.request_close();
        assert!(!lifecycle.is_visible());
        lifecycle.platform_event(PlatformEvent::TrayActivated("app".to_owned()));
        assert!(lifecycle.is_visible());
        lifecycle.platform_event(PlatformEvent::SecondInstance(vec![
            "vui".to_owned(),
            "other.txt".to_owned(),
            "vui://about".to_owned(),
        ]));
        assert_eq!(
            lifecycle.open_files(),
            [PathBuf::from("document.txt"), PathBuf::from("other.txt")]
        );
        assert_eq!(lifecycle.open_urls(), ["vui://settings", "vui://about"]);
        assert!(matches!(
            lifecycle.quit().as_slice(),
            [PlatformRequest::RemoveTray(id)] if id == "app"
        ));
        assert!(lifecycle.has_exited());
    }

    #[test]
    fn secondary_desktop_instance_exits_without_installing_services() {
        let mut lifecycle =
            DesktopLifecycleHarness::new(WindowId::next(), "dev.vui.test").single_instance(true);
        lifecycle.start();
        assert!(
            lifecycle
                .platform_event(PlatformEvent::SingleInstanceStatus { primary: false })
                .is_empty()
        );
        assert_eq!(lifecycle.primary_status(), Some(false));
        assert!(lifecycle.has_exited());
    }
}
