//! Winit desktop host for VUI.

use anyhow::{Context, Result};
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet, VecDeque},
    hash::{DefaultHasher, Hash, Hasher},
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};
use tray_icon::{
    Icon as TrayIconImage, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem, MenuItemKind, PredefinedMenuItem, Submenu},
};
use vello::{
    AaConfig, RenderParams, Renderer, RendererOptions,
    peniko::Color as VelloColor,
    util::{RenderContext, RenderSurface},
    wgpu,
};
use vui_core::{
    ClipboardRequest, CommandId, DesktopNotification, DragOperation, DragPayload, Event,
    FileDialogKind, FileDialogRequest, Key, Modifier, Modifiers, MonitorInfo, NativeMenu,
    NativeMenuItem, PlatformCommand, PlatformEvent, PlatformRequest, Point, PointerButton,
    PowerEvent, Size, SystemAppearance, SystemPreferences, TaskUpdate, Theme, TrayDescriptor,
    UiRuntime, Widget, WindowCommand, WindowDescriptor, WindowId as UiWindowId,
};
use vui_platform::{PlatformError, PlatformResponder, PlatformServices};
use vui_session::SessionStore;
#[cfg(feature = "system-fonts")]
use vui_text::ParleyTextSystem;
use vui_vello::SceneBuilder;
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalPosition, LogicalSize},
    event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    keyboard::{Key as WinitKey, ModifiersState, NamedKey},
    monitor::MonitorHandle,
    window::{
        Fullscreen, Theme as WinitTheme, UserAttentionType, Window, WindowAttributes, WindowId,
    },
};

/// Runs a widget tree in a native desktop window.
///
/// Widgets may create additional independent windows with
/// [`vui_core::EventCtx::open_window`]. Embedders may instead drive
/// [`UiRuntime`] and `vui_vello::SceneBuilder` directly.
///
/// # Errors
///
/// Returns an error if the event loop, a native window, GPU surface, Vello
/// renderer, or a rendered frame cannot be created or presented.
pub fn run(title: impl Into<String>, root: impl Widget + 'static) -> Result<()> {
    run_with_options(title, root, RunOptions::default())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RunOptions {
    pub reduced_motion: bool,
}

/// Configures and runs a complete desktop application lifecycle.
pub struct DesktopApplication {
    descriptor: WindowDescriptor,
    root: Box<dyn Widget>,
    options: RunOptions,
    session_path: Option<PathBuf>,
    services: Option<Box<dyn PlatformServices>>,
    lifecycle: DesktopLifecycleOptions,
}

#[derive(Default)]
struct DesktopLifecycleOptions {
    application_id: Option<String>,
    arguments: Vec<String>,
    single_instance: bool,
    tray: Option<TrayDescriptor>,
    menu_bar: Option<NativeMenu>,
    close_to_tray: bool,
    follow_system_theme: bool,
}

impl DesktopApplication {
    #[must_use]
    pub fn new(application_id: impl Into<String>, root: impl Widget + 'static) -> Self {
        let application_id = application_id.into();
        Self {
            descriptor: WindowDescriptor::new(&application_id, Size::new(640.0, 360.0))
                .min_size(Size::new(320.0, 180.0))
                .key("main"),
            root: Box::new(root),
            options: RunOptions::default(),
            session_path: None,
            services: None,
            lifecycle: DesktopLifecycleOptions {
                application_id: Some(application_id),
                arguments: std::env::args().collect(),
                follow_system_theme: true,
                ..DesktopLifecycleOptions::default()
            },
        }
    }

    #[must_use]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.descriptor.title = title.into();
        self
    }

    #[must_use]
    pub fn window(mut self, descriptor: WindowDescriptor) -> Self {
        self.descriptor = descriptor;
        self
    }

    #[must_use]
    pub const fn options(mut self, options: RunOptions) -> Self {
        self.options = options;
        self
    }

    #[must_use]
    pub fn session_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.session_path = Some(path.into());
        self
    }

    #[must_use]
    pub fn platform_services(mut self, services: impl PlatformServices + 'static) -> Self {
        self.services = Some(Box::new(services));
        self
    }

    #[must_use]
    pub const fn single_instance(mut self, enabled: bool) -> Self {
        self.lifecycle.single_instance = enabled;
        self
    }

    #[must_use]
    pub fn arguments(mut self, arguments: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.lifecycle.arguments = arguments.into_iter().map(Into::into).collect();
        self
    }

    #[must_use]
    pub fn tray(mut self, tray: TrayDescriptor) -> Self {
        self.lifecycle.tray = Some(tray);
        self
    }

    #[must_use]
    pub fn menu_bar(mut self, menu: NativeMenu) -> Self {
        self.lifecycle.menu_bar = Some(menu);
        self
    }

    #[must_use]
    pub const fn close_to_tray(mut self, enabled: bool) -> Self {
        self.lifecycle.close_to_tray = enabled;
        self
    }

    #[must_use]
    pub const fn follow_system_theme(mut self, enabled: bool) -> Self {
        self.lifecycle.follow_system_theme = enabled;
        self
    }

    /// Starts the native application and blocks until shutdown.
    ///
    /// # Errors
    ///
    /// Returns an error if configuration, session loading, or native hosting fails.
    pub fn run(self) -> Result<()> {
        if self.lifecycle.close_to_tray && self.lifecycle.tray.is_none() {
            anyhow::bail!("close-to-tray requires a tray descriptor");
        }
        let session = self
            .session_path
            .map(SessionStore::open)
            .transpose()
            .context("load VUI session")?;
        run_configured(
            self.descriptor,
            self.root,
            self.options,
            session,
            self.services,
            self.lifecycle,
        )
    }
}

/// Runs a widget tree with explicit desktop accessibility options.
///
/// # Errors
///
/// Returns an error under the same conditions as [`run`].
pub fn run_with_options(
    title: impl Into<String>,
    root: impl Widget + 'static,
    options: RunOptions,
) -> Result<()> {
    run_internal(title.into(), Box::new(root), options, None, None)
}

/// Runs a widget tree with a replaceable desktop-services implementation.
///
/// This is the integration seam for deterministic tests, application-specific
/// policy, and platform backends that provide richer shell integration.
///
/// # Errors
///
/// Returns an error under the same conditions as [`run`].
pub fn run_with_platform_services(
    title: impl Into<String>,
    root: impl Widget + 'static,
    options: RunOptions,
    services: impl PlatformServices + 'static,
) -> Result<()> {
    run_internal(
        title.into(),
        Box::new(root),
        options,
        None,
        Some(Box::new(services)),
    )
}

/// Runs a widget tree and persists keyed window geometry between launches.
///
/// # Errors
///
/// Returns an error if the session cannot be loaded or the native host fails.
pub fn run_with_session(
    title: impl Into<String>,
    root: impl Widget + 'static,
    options: RunOptions,
    session_path: impl Into<PathBuf>,
) -> Result<()> {
    let session = SessionStore::open(session_path.into()).context("load VUI session")?;
    run_internal(title.into(), Box::new(root), options, Some(session), None)
}

/// Runs with both session persistence and custom desktop services.
///
/// # Errors
///
/// Returns an error if the session cannot be loaded or the native host fails.
pub fn run_with_session_and_platform_services(
    title: impl Into<String>,
    root: impl Widget + 'static,
    options: RunOptions,
    session_path: impl Into<PathBuf>,
    services: impl PlatformServices + 'static,
) -> Result<()> {
    let session = SessionStore::open(session_path.into()).context("load VUI session")?;
    run_internal(
        title.into(),
        Box::new(root),
        options,
        Some(session),
        Some(Box::new(services)),
    )
}

fn run_internal(
    title: String,
    root: Box<dyn Widget>,
    options: RunOptions,
    session: Option<SessionStore>,
    services: Option<Box<dyn PlatformServices>>,
) -> Result<()> {
    let descriptor = WindowDescriptor::new(title, Size::new(640.0, 360.0))
        .min_size(Size::new(320.0, 180.0))
        .key("main");
    run_configured(
        descriptor,
        root,
        options,
        session,
        services,
        DesktopLifecycleOptions::default(),
    )
}

fn run_configured(
    descriptor: WindowDescriptor,
    root: Box<dyn Widget>,
    options: RunOptions,
    session: Option<SessionStore>,
    services: Option<Box<dyn PlatformServices>>,
    lifecycle: DesktopLifecycleOptions,
) -> Result<()> {
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .context("create event loop")?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::new(
        descriptor,
        root,
        event_loop.create_proxy(),
        options,
        session,
        services.unwrap_or_else(|| Box::new(DesktopPlatformServices::new(options))),
        lifecycle,
    );
    event_loop.run_app(&mut app).context("run event loop")?;
    if let Some(error) = app.fatal_error {
        Err(error)
    } else {
        Ok(())
    }
}

#[derive(Debug)]
enum UserEvent {
    #[cfg(feature = "accessibility")]
    AccessKit(accesskit_winit::Event),
    Task {
        window: UiWindowId,
        update: TaskUpdate,
    },
    Platform {
        window: UiWindowId,
        event: PlatformEvent,
    },
}

#[cfg(feature = "accessibility")]
impl From<accesskit_winit::Event> for UserEvent {
    fn from(event: accesskit_winit::Event) -> Self {
        Self::AccessKit(event)
    }
}

/// Default cross-platform implementation of VUI's desktop-service boundary.
pub struct DesktopPlatformServices {
    preferences: SystemPreferences,
    registered_instances: HashSet<String>,
    trays: HashMap<String, TrayIcon>,
    tray_origins: HashMap<String, (UiWindowId, String)>,
    menu_commands: HashMap<String, (UiWindowId, CommandId)>,
}

impl DesktopPlatformServices {
    #[must_use]
    pub fn new(options: RunOptions) -> Self {
        Self {
            preferences: SystemPreferences {
                reduced_motion: options.reduced_motion,
                ..SystemPreferences::default()
            },
            registered_instances: HashSet::new(),
            trays: HashMap::new(),
            tray_origins: HashMap::new(),
            menu_commands: HashMap::new(),
        }
    }

    fn show_file_dialog(request: &FileDialogRequest) -> Option<Vec<PathBuf>> {
        let mut dialog = rfd::FileDialog::new();
        if let Some(title) = &request.title {
            dialog = dialog.set_title(title);
        }
        if let Some(directory) = &request.starting_directory {
            dialog = dialog.set_directory(directory);
        }
        for filter in &request.filters {
            dialog = dialog.add_filter(&filter.label, &filter.extensions);
        }
        match request.kind {
            FileDialogKind::OpenFile if request.allow_multiple => dialog.pick_files(),
            FileDialogKind::OpenFile => dialog.pick_file().map(|path| vec![path]),
            FileDialogKind::SaveFile => dialog.save_file().map(|path| vec![path]),
            FileDialogKind::SelectFolder if request.allow_multiple => dialog.pick_folders(),
            FileDialogKind::SelectFolder => dialog.pick_folder().map(|path| vec![path]),
        }
    }

    fn register_single_instance(
        &mut self,
        origin: UiWindowId,
        application_id: &str,
        arguments: &[String],
        respond: PlatformResponder,
    ) -> Result<(), PlatformError> {
        if !self.registered_instances.insert(application_id.to_owned()) {
            return Ok(());
        }
        let address = format!("127.0.0.1:{}", instance_port(application_id));
        match TcpListener::bind(&address) {
            Ok(listener) => {
                respond(
                    origin,
                    PlatformEvent::SingleInstanceStatus { primary: true },
                );
                std::thread::spawn(move || {
                    for mut stream in listener.incoming().flatten() {
                        let mut bytes = Vec::new();
                        if stream.read_to_end(&mut bytes).is_ok() {
                            let arguments = decode_arguments(&bytes);
                            respond(origin, PlatformEvent::SecondInstance(arguments));
                        }
                    }
                });
                Ok(())
            }
            Err(bind_error) => {
                let mut stream = TcpStream::connect(&address).map_err(|connect_error| {
                    PlatformError::new(format!(
                        "single-instance endpoint failed: {bind_error}; {connect_error}"
                    ))
                })?;
                stream
                    .write_all(&encode_arguments(arguments))
                    .map_err(|error| PlatformError::new(error.to_string()))?;
                respond(
                    origin,
                    PlatformEvent::SingleInstanceStatus { primary: false },
                );
                Ok(())
            }
        }
    }

    fn set_tray(
        &mut self,
        origin: UiWindowId,
        descriptor: TrayDescriptor,
    ) -> Result<(), PlatformError> {
        let tray_key = format!("{}:{}", origin.raw(), descriptor.id);
        let native_id = format!("vui-tray:{tray_key}");
        let menu_prefix = format!("vui-menu:{tray_key}:");
        self.menu_commands
            .retain(|id, _| !id.starts_with(&menu_prefix));
        let menu = build_native_menu(
            origin,
            &descriptor.menu,
            &menu_prefix,
            &mut self.menu_commands,
        )?;
        let mut builder = TrayIconBuilder::new()
            .with_id(native_id.clone())
            .with_menu(Box::new(menu))
            .with_tooltip(&descriptor.tooltip);
        if let Some(icon) = descriptor.icon {
            let icon =
                TrayIconImage::from_rgba(icon.pixels().to_vec(), icon.width(), icon.height())
                    .map_err(|error| PlatformError::new(format!("invalid tray icon: {error}")))?;
            builder = builder.with_icon(icon);
        }
        let tray = builder
            .build()
            .map_err(|error| PlatformError::new(format!("create tray icon: {error}")))?;
        self.tray_origins.insert(native_id, (origin, descriptor.id));
        self.trays.insert(tray_key, tray);
        Ok(())
    }
}

impl Default for DesktopPlatformServices {
    fn default() -> Self {
        Self::new(RunOptions::default())
    }
}

impl PlatformServices for DesktopPlatformServices {
    fn request(
        &mut self,
        origin: UiWindowId,
        request: PlatformRequest,
        respond: PlatformResponder,
    ) -> Result<(), PlatformError> {
        match request {
            PlatformRequest::OpenUri(uri) => {
                launch_uri(&uri).map_err(|error| PlatformError::new(format!("open URI: {error}")))
            }
            PlatformRequest::RevealPath(path) => reveal_path(&path)
                .map_err(|error| PlatformError::new(format!("reveal path: {error}"))),
            PlatformRequest::FileDialog(request) => {
                let event = Self::show_file_dialog(&request).map_or(
                    PlatformEvent::FileDialogCancelled { id: request.id },
                    |paths| PlatformEvent::FileDialogCompleted {
                        id: request.id,
                        paths,
                    },
                );
                respond(origin, event);
                Ok(())
            }
            PlatformRequest::Notification(notification) => {
                show_notification(origin, notification, respond);
                Ok(())
            }
            PlatformRequest::RegisterSingleInstance {
                application_id,
                arguments,
            } => self.register_single_instance(origin, &application_id, &arguments, respond),
            PlatformRequest::SetMenuBar { .. } => Err(PlatformError::new(
                "native menu bars require a host-specific PlatformServices backend",
            )),
            PlatformRequest::SetTray(descriptor) => self.set_tray(origin, descriptor),
            PlatformRequest::RemoveTray(id) => {
                let tray_key = format!("{}:{id}", origin.raw());
                self.trays.remove(&tray_key);
                self.tray_origins.remove(&format!("vui-tray:{tray_key}"));
                Ok(())
            }
            PlatformRequest::StartDrag { .. } => Err(PlatformError::new(
                "outbound native drag is unavailable in the default Winit backend",
            )),
            PlatformRequest::SetPowerInhibition(_) => Err(PlatformError::new(
                "power inhibition requires a host-specific PlatformServices backend",
            )),
        }
    }

    fn preferences(&self) -> SystemPreferences {
        self.preferences.clone()
    }

    fn poll_events(&mut self) -> Vec<(UiWindowId, PlatformEvent)> {
        let mut events = Vec::new();
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if let Some((window, command)) = self.menu_commands.get(event.id().as_ref()) {
                events.push((*window, PlatformEvent::CommandInvoked(command.clone())));
            }
        }
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if let Some((window, id)) = self.tray_origins.get(event.id().as_ref()) {
                events.push((*window, PlatformEvent::TrayActivated(id.clone())));
            }
        }
        events
    }

    fn poll_interval(&self) -> Option<Duration> {
        (!self.trays.is_empty()).then_some(Duration::from_millis(100))
    }
}

fn build_native_menu(
    origin: UiWindowId,
    menu: &NativeMenu,
    prefix: &str,
    commands: &mut HashMap<String, (UiWindowId, CommandId)>,
) -> Result<Menu, PlatformError> {
    let native = Menu::new();
    for (index, item) in menu.items.iter().enumerate() {
        let item = build_native_menu_item(origin, item, prefix, index, commands)?;
        append_menu_item(&native, &item)?;
    }
    Ok(native)
}

fn build_native_menu_item(
    origin: UiWindowId,
    item: &NativeMenuItem,
    prefix: &str,
    index: usize,
    commands: &mut HashMap<String, (UiWindowId, CommandId)>,
) -> Result<MenuItemKind, PlatformError> {
    Ok(match item {
        NativeMenuItem::Command(command) => {
            let id = format!("{prefix}{index}:{}", command.id.as_str());
            commands.insert(id.clone(), (origin, command.id.clone()));
            MenuItemKind::MenuItem(MenuItem::with_id(id, &command.label, command.enabled, None))
        }
        NativeMenuItem::Separator => MenuItemKind::Predefined(PredefinedMenuItem::separator()),
        NativeMenuItem::Submenu { label, menu } => {
            let submenu = Submenu::new(label, true);
            for (child_index, child) in menu.items.iter().enumerate() {
                let child_prefix = format!("{prefix}{index}:");
                let child =
                    build_native_menu_item(origin, child, &child_prefix, child_index, commands)?;
                append_submenu_item(&submenu, &child)?;
            }
            MenuItemKind::Submenu(submenu)
        }
    })
}

fn append_menu_item(menu: &Menu, item: &MenuItemKind) -> Result<(), PlatformError> {
    let result = match item {
        MenuItemKind::MenuItem(item) => menu.append(item),
        MenuItemKind::Submenu(item) => menu.append(item),
        MenuItemKind::Predefined(item) => menu.append(item),
        MenuItemKind::Check(item) => menu.append(item),
        MenuItemKind::Icon(item) => menu.append(item),
    };
    result.map_err(|error| PlatformError::new(format!("build tray menu: {error}")))
}

fn append_submenu_item(submenu: &Submenu, item: &MenuItemKind) -> Result<(), PlatformError> {
    let result = match item {
        MenuItemKind::MenuItem(item) => submenu.append(item),
        MenuItemKind::Submenu(item) => submenu.append(item),
        MenuItemKind::Predefined(item) => submenu.append(item),
        MenuItemKind::Check(item) => submenu.append(item),
        MenuItemKind::Icon(item) => submenu.append(item),
    };
    result.map_err(|error| PlatformError::new(format!("build tray submenu: {error}")))
}

fn instance_port(application_id: &str) -> u16 {
    let mut hasher = DefaultHasher::new();
    application_id.hash(&mut hasher);
    40_000 + u16::try_from(hasher.finish() % 20_000).unwrap_or(0)
}

fn encode_arguments(arguments: &[String]) -> Vec<u8> {
    arguments.join("\0").into_bytes()
}

fn decode_arguments(bytes: &[u8]) -> Vec<String> {
    if bytes.is_empty() {
        return Vec::new();
    }
    String::from_utf8_lossy(bytes)
        .split('\0')
        .map(ToOwned::to_owned)
        .collect()
}

fn classify_launch_arguments(arguments: &[String]) -> (Vec<PathBuf>, Vec<String>) {
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

fn show_notification(
    origin: UiWindowId,
    notification: DesktopNotification,
    respond: PlatformResponder,
) {
    std::thread::spawn(move || {
        let mut native = notify_rust::Notification::new();
        native.summary(&notification.title).body(&notification.body);
        for action in &notification.actions {
            native.action(&action.id, &action.label);
        }
        match native.show() {
            Ok(handle) => handle.wait_for_action(|action| {
                if action == "__closed" {
                    respond(
                        origin,
                        PlatformEvent::NotificationClosed(notification.id.clone()),
                    );
                } else {
                    respond(
                        origin,
                        PlatformEvent::NotificationAction {
                            notification: notification.id.clone(),
                            action: action.to_owned(),
                        },
                    );
                }
            }),
            Err(error) => respond(origin, PlatformEvent::Failed(error.to_string())),
        }
    });
}

struct PendingWindow {
    id: UiWindowId,
    descriptor: WindowDescriptor,
    root: Box<dyn Widget>,
}

struct ClickState {
    when: Instant,
    position: Point,
    count: u8,
}

struct NativeWindow {
    descriptor: WindowDescriptor,
    runtime: UiRuntime,
    renderer: Renderer,
    surface: RenderSurface<'static>,
    window: Arc<Window>,
    scene_builder: SceneBuilder,
    pointer: Point,
    scale_factor: f64,
    #[cfg(feature = "accessibility")]
    accesskit: accesskit_winit::Adapter,
    last_click: Option<ClickState>,
    monitor: Option<MonitorInfo>,
}

struct App {
    pending_primary: Option<PendingWindow>,
    primary_id: UiWindowId,
    windows: HashMap<UiWindowId, NativeWindow>,
    native_ids: HashMap<WindowId, UiWindowId>,
    render_context: RenderContext,
    fatal_error: Option<anyhow::Error>,
    clipboard: Option<arboard::Clipboard>,
    event_proxy: EventLoopProxy<UserEvent>,
    options: RunOptions,
    started_at: Instant,
    session: Option<SessionStore>,
    services: Box<dyn PlatformServices>,
    lifecycle: DesktopLifecycleOptions,
    startup_started: bool,
    waiting_for_instance: bool,
}

impl App {
    fn new(
        descriptor: WindowDescriptor,
        root: Box<dyn Widget>,
        event_proxy: EventLoopProxy<UserEvent>,
        options: RunOptions,
        session: Option<SessionStore>,
        services: Box<dyn PlatformServices>,
        lifecycle: DesktopLifecycleOptions,
    ) -> Self {
        let primary_id = UiWindowId::next();
        Self {
            pending_primary: Some(PendingWindow {
                id: primary_id,
                descriptor,
                root,
            }),
            primary_id,
            windows: HashMap::new(),
            native_ids: HashMap::new(),
            render_context: RenderContext::new(),
            fatal_error: None,
            clipboard: None,
            event_proxy,
            options,
            started_at: Instant::now(),
            session,
            services,
            lifecycle,
            startup_started: false,
            waiting_for_instance: false,
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: impl Into<anyhow::Error>) {
        self.fatal_error = Some(error.into());
        event_loop.exit();
    }

    fn begin_startup(&mut self, event_loop: &ActiveEventLoop) -> bool {
        if self.startup_started {
            if !self.waiting_for_instance {
                return self.ensure_primary_window(event_loop);
            }
            return false;
        }
        self.startup_started = true;
        if self.lifecycle.single_instance
            && let Some(application_id) = self.lifecycle.application_id.clone()
        {
            self.waiting_for_instance = true;
            self.request_platform(
                self.primary_id,
                PlatformRequest::RegisterSingleInstance {
                    application_id,
                    arguments: self.lifecycle.arguments.clone(),
                },
            );
        } else {
            return self.ensure_primary_window(event_loop);
        }
        false
    }

    fn ensure_primary_window(&mut self, event_loop: &ActiveEventLoop) -> bool {
        let Some(pending) = self.pending_primary.take() else {
            return false;
        };
        if let Err(error) = self.create_window(event_loop, pending) {
            self.fail(event_loop, error);
            return false;
        }
        self.install_desktop_services(event_loop);
        true
    }

    fn install_desktop_services(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(menu) = self.lifecycle.menu_bar.clone() {
            self.request_platform(
                self.primary_id,
                PlatformRequest::SetMenuBar {
                    window: self.primary_id,
                    menu,
                },
            );
        }
        if let Some(tray) = self.lifecycle.tray.clone() {
            self.request_platform(self.primary_id, PlatformRequest::SetTray(tray));
        }
        let (files, urls) = classify_launch_arguments(&self.lifecycle.arguments);
        if !files.is_empty() {
            self.dispatch(
                self.primary_id,
                &Event::Platform(PlatformEvent::OpenFiles(files)),
                event_loop,
            );
        }
        if !urls.is_empty() {
            self.dispatch(
                self.primary_id,
                &Event::Platform(PlatformEvent::OpenUrls(urls)),
                event_loop,
            );
        }
    }

    fn show_primary_window(&mut self) {
        if let Some(window) = self.windows.get(&self.primary_id) {
            window.window.set_visible(true);
            window.window.focus_window();
            window.window.request_redraw();
        }
    }

    fn hide_primary_window(&mut self) {
        if let Some(window) = self.windows.get(&self.primary_id) {
            window.window.set_visible(false);
        }
    }

    fn shutdown(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(tray) = &self.lifecycle.tray {
            self.request_platform(
                self.primary_id,
                PlatformRequest::RemoveTray(tray.id.clone()),
            );
        }
        if let Some(session) = &mut self.session
            && let Err(error) = session.save_now()
        {
            self.fail(event_loop, error);
            return;
        }
        event_loop.exit();
    }

    fn apply_preferences(&mut self, preferences: &SystemPreferences) {
        for window in self.windows.values_mut() {
            window
                .runtime
                .set_reduced_motion(preferences.reduced_motion);
            if self.lifecycle.follow_system_theme {
                match preferences.appearance {
                    SystemAppearance::Light => window.runtime.set_theme(Theme::light()),
                    SystemAppearance::Dark => window.runtime.set_theme(Theme::dark()),
                    SystemAppearance::HighContrast | SystemAppearance::Unknown => {}
                }
            }
            window.window.request_redraw();
        }
    }

    fn deliver_environment(&mut self, event_loop: &ActiveEventLoop) {
        let preferences = self.services.preferences();
        self.apply_preferences(&preferences);
        let ids = self.windows.keys().copied().collect::<Vec<_>>();
        for id in ids {
            self.dispatch(
                id,
                &Event::Platform(PlatformEvent::PreferencesChanged(preferences.clone())),
                event_loop,
            );
            self.dispatch(
                id,
                &Event::Platform(PlatformEvent::Power(PowerEvent::Resume)),
                event_loop,
            );
        }
    }

    fn restored_descriptor(
        &self,
        event_loop: &ActiveEventLoop,
        mut descriptor: WindowDescriptor,
    ) -> (WindowDescriptor, Option<(bool, bool)>) {
        let monitors = event_loop
            .available_monitors()
            .map(|monitor| monitor_handle_info(&monitor))
            .collect::<Vec<_>>();
        let saved_state = descriptor.key.as_deref().and_then(|key| {
            self.session.as_ref().and_then(|session| {
                session
                    .window(key)
                    .map(|window| (window.maximized, window.fullscreen))
            })
        });
        if let (Some(session), Some(key)) = (&self.session, descriptor.key.as_deref()) {
            descriptor = session.restore_descriptor(key, descriptor.clone(), &monitors);
        }
        (descriptor, saved_state)
    }

    fn create_window(
        &mut self,
        event_loop: &ActiveEventLoop,
        pending: PendingWindow,
    ) -> Result<()> {
        let (descriptor, saved_state) = self.restored_descriptor(event_loop, pending.descriptor);
        let session_key = descriptor.key.clone();
        let modal_parent = descriptor.is_modal().then_some(descriptor.parent).flatten();
        let mut attributes: WindowAttributes = Window::default_attributes()
            .with_title(descriptor.title.clone())
            .with_inner_size(logical_winit_size(descriptor.size))
            .with_resizable(descriptor.is_resizable())
            .with_transparent(descriptor.is_transparent())
            .with_visible(false);
        if let Some(size) = descriptor.min_size {
            attributes = attributes.with_min_inner_size(logical_winit_size(size));
        }
        if let Some(size) = descriptor.max_size {
            attributes = attributes.with_max_inner_size(logical_winit_size(size));
        }
        if let Some(position) = descriptor.position {
            attributes = attributes.with_position(LogicalPosition::new(position.x, position.y));
        }
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .context("create native window")?,
        );
        let physical = window.inner_size();
        #[cfg(feature = "accessibility")]
        let accesskit = accesskit_winit::Adapter::with_event_loop_proxy(
            event_loop,
            window.as_ref(),
            self.event_proxy.clone(),
        );
        window.set_ime_allowed(true);
        self.clipboard = self
            .clipboard
            .take()
            .or_else(|| arboard::Clipboard::new().ok());
        let scale_factor = window.scale_factor();
        let surface = pollster::block_on(self.render_context.create_surface(
            window.clone(),
            physical.width.max(1),
            physical.height.max(1),
            wgpu::PresentMode::AutoVsync,
        ))
        .context("create Vello render surface")?;
        let device = &self.render_context.devices[surface.dev_id].device;
        let renderer =
            Renderer::new(device, RendererOptions::default()).context("create Vello renderer")?;
        let runtime = UiRuntime::from_boxed(pending.root).with_window_id(pending.id);
        #[cfg(feature = "system-fonts")]
        let runtime = runtime.with_text_system(ParleyTextSystem::new());
        let mut runtime = runtime;
        runtime.set_reduced_motion(self.options.reduced_motion);
        runtime.set_viewport(logical_size(physical.width, physical.height, scale_factor));
        let native_id = window.id();
        let monitor = monitor_info(&window);
        if let Some(info) = monitor.clone() {
            runtime.handle_event(&Event::MonitorChanged(info));
        }
        self.native_ids.insert(native_id, pending.id);
        self.windows.insert(
            pending.id,
            NativeWindow {
                descriptor,
                runtime,
                renderer,
                surface,
                window: window.clone(),
                scene_builder: SceneBuilder::new(),
                pointer: Point::default(),
                scale_factor,
                #[cfg(feature = "accessibility")]
                accesskit,
                last_click: None,
                monitor: monitor.clone(),
            },
        );
        if let Some(info) = monitor {
            self.record_session_event(pending.id, &Event::MonitorChanged(info));
        }
        window.set_visible(true);
        if let Some((maximized, fullscreen)) = saved_state {
            window.set_maximized(maximized);
            window.set_fullscreen(fullscreen.then_some(Fullscreen::Borderless(None)));
        }
        if modal_parent.is_some() {
            window.focus_window();
        }
        if let (Some(session), Some(key)) = (&mut self.session, session_key.as_deref()) {
            session.set_open(key, true, Instant::now());
        }
        window.request_redraw();
        Ok(())
    }

    fn modal_child(&self, parent: UiWindowId) -> Option<&NativeWindow> {
        self.windows
            .values()
            .find(|window| window.descriptor.is_modal() && window.descriptor.parent == Some(parent))
    }

    fn remove_window(&mut self, id: UiWindowId, event_loop: &ActiveEventLoop) {
        let children = self
            .windows
            .iter()
            .filter_map(|(child_id, window)| {
                (window.descriptor.parent == Some(id)).then_some(*child_id)
            })
            .collect::<Vec<_>>();
        for child in children {
            self.remove_window(child, event_loop);
        }
        if let Some(window) = self.windows.remove(&id) {
            self.native_ids.remove(&window.window.id());
            if let (Some(session), Some(key)) =
                (&mut self.session, window.descriptor.key.as_deref())
            {
                session.set_open(key, false, Instant::now());
            }
        }
        if self.windows.is_empty() && self.pending_primary.is_none() {
            if let Some(session) = &mut self.session
                && let Err(error) = session.save_now()
            {
                self.fail(event_loop, error);
                return;
            }
            event_loop.exit();
        }
    }

    fn service_platform_commands(&mut self, source: UiWindowId, event_loop: &ActiveEventLoop) {
        let initial = self
            .windows
            .get_mut(&source)
            .map_or_else(Vec::new, |window| window.runtime.take_platform_commands());
        let mut commands = initial
            .into_iter()
            .map(|command| (source, command))
            .collect::<VecDeque<_>>();
        while let Some((origin, command)) = commands.pop_front() {
            match command {
                PlatformCommand::Window(command) => {
                    if !self.service_window_command(command, event_loop) {
                        return;
                    }
                }
                PlatformCommand::OpenUri(uri) => {
                    self.request_platform(origin, PlatformRequest::OpenUri(uri));
                }
                PlatformCommand::RevealPath(path) => {
                    self.request_platform(origin, PlatformRequest::RevealPath(path));
                }
                PlatformCommand::ShowNotification(notification) => self.request_platform(
                    origin,
                    PlatformRequest::Notification(DesktopNotification::new(
                        format!("legacy-{}", self.started_at.elapsed().as_nanos()),
                        notification.title,
                        notification.body,
                    )),
                ),
                PlatformCommand::FileDialog(request) => {
                    self.request_platform(origin, PlatformRequest::FileDialog(request));
                }
                PlatformCommand::Request(request) => self.request_platform(origin, request),
            }
            if let Some(window) = self.windows.get_mut(&origin) {
                commands.extend(
                    window
                        .runtime
                        .take_platform_commands()
                        .into_iter()
                        .map(|command| (origin, command)),
                );
            }
        }
    }

    fn request_platform(&mut self, origin: UiWindowId, request: PlatformRequest) {
        let proxy = self.event_proxy.clone();
        let respond: PlatformResponder = Arc::new(move |window, event| {
            let _result = proxy.send_event(UserEvent::Platform { window, event });
        });
        if let Err(error) = self.services.request(origin, request, respond) {
            let _result = self.event_proxy.send_event(UserEvent::Platform {
                window: origin,
                event: PlatformEvent::Failed(error.to_string()),
            });
        }
    }

    fn service_window_command(
        &mut self,
        command: WindowCommand,
        event_loop: &ActiveEventLoop,
    ) -> bool {
        match command {
            WindowCommand::Open {
                id,
                descriptor,
                root,
            } => {
                if descriptor.is_singleton()
                    && let Some(key) = descriptor.key.as_deref()
                    && let Some(existing) = self
                        .windows
                        .values()
                        .find(|window| window.descriptor.key.as_deref() == Some(key))
                {
                    existing.window.focus_window();
                    return true;
                }
                if let Err(error) = self.create_window(
                    event_loop,
                    PendingWindow {
                        id,
                        descriptor,
                        root,
                    },
                ) {
                    self.fail(event_loop, error);
                    return false;
                }
            }
            WindowCommand::Close(id) => self.remove_window(id, event_loop),
            WindowCommand::Focus(id) => {
                if let Some(window) = self.windows.get(&id) {
                    window.window.focus_window();
                }
            }
            WindowCommand::Minimize(id, value) => {
                if let Some(window) = self.windows.get(&id) {
                    window.window.set_minimized(value);
                }
            }
            WindowCommand::Maximize(id, value) => {
                if let Some(window) = self.windows.get(&id) {
                    window.window.set_maximized(value);
                }
                self.record_window_state(id, Some(value), None);
            }
            WindowCommand::Fullscreen(id, value) => {
                if let Some(window) = self.windows.get(&id) {
                    window
                        .window
                        .set_fullscreen(value.then_some(Fullscreen::Borderless(None)));
                }
                self.record_window_state(id, None, Some(value));
            }
            WindowCommand::RequestAttention(id) => {
                if let Some(window) = self.windows.get(&id) {
                    window
                        .window
                        .request_user_attention(Some(UserAttentionType::Critical));
                }
            }
            WindowCommand::RefreshAll => {
                for window in self.windows.values_mut() {
                    window.runtime.handle_event(&Event::AppStateChanged);
                    window.window.request_redraw();
                }
            }
            WindowCommand::QuitApplication => {
                let cancelled = self.windows.values_mut().any(|window| {
                    window
                        .runtime
                        .handle_event(&Event::ApplicationExitRequested)
                });
                if !cancelled {
                    self.shutdown(event_loop);
                }
            }
        }
        true
    }

    fn service_tasks(&mut self, id: UiWindowId) {
        let Some(window) = self.windows.get_mut(&id) else {
            return;
        };
        for request in window.runtime.take_task_requests() {
            let proxy = self.event_proxy.clone();
            std::thread::spawn(move || {
                request.run(move |update| {
                    let _result = proxy.send_event(UserEvent::Task { window: id, update });
                });
            });
        }
    }

    fn service_clipboard(&mut self, id: UiWindowId) {
        let request = self
            .windows
            .get_mut(&id)
            .and_then(|window| window.runtime.take_clipboard_request());
        match request {
            Some(ClipboardRequest::Write(text)) => {
                if let Some(clipboard) = &mut self.clipboard {
                    let _result = clipboard.set_text(text);
                }
            }
            Some(ClipboardRequest::Read) => {
                let text = self
                    .clipboard
                    .as_mut()
                    .and_then(|clipboard| clipboard.get_text().ok());
                if let (Some(text), Some(window)) = (text, self.windows.get_mut(&id)) {
                    window.runtime.handle_event(&Event::TextInput(text));
                }
            }
            Some(ClipboardRequest::WriteImage(image)) => {
                if let (Ok(width), Ok(height), Some(clipboard)) = (
                    usize::try_from(image.width()),
                    usize::try_from(image.height()),
                    self.clipboard.as_mut(),
                ) {
                    let data = arboard::ImageData {
                        width,
                        height,
                        bytes: Cow::Owned(image.pixels().to_vec()),
                    };
                    let _result = clipboard.set_image(data);
                }
            }
            Some(ClipboardRequest::ReadImage) => {
                let image = self.clipboard.as_mut().and_then(|clipboard| {
                    let image = clipboard.get_image().ok()?;
                    let width = u32::try_from(image.width).ok()?;
                    let height = u32::try_from(image.height).ok()?;
                    vui_core::RasterImage::new(width, height, image.bytes.into_owned()).ok()
                });
                if let (Some(image), Some(window)) = (image, self.windows.get_mut(&id)) {
                    window.runtime.handle_event(&Event::ClipboardImage(image));
                }
            }
            None => {}
        }
    }

    fn dispatch(&mut self, id: UiWindowId, event: &Event, event_loop: &ActiveEventLoop) -> bool {
        let handled = if let Some(window) = self.windows.get_mut(&id) {
            window.runtime.advance_time(self.started_at.elapsed());
            let handled = window.runtime.handle_event(event);
            window.window.request_redraw();
            handled
        } else {
            false
        };
        self.service_tasks(id);
        self.service_clipboard(id);
        self.service_platform_commands(id, event_loop);
        handled
    }

    fn resize(&mut self, id: UiWindowId, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        if let Some(window) = self.windows.get_mut(&id) {
            self.render_context
                .resize_surface(&mut window.surface, width, height);
            window
                .runtime
                .set_viewport(logical_size(width, height, window.scale_factor));
            window.window.request_redraw();
        }
    }

    fn render(&mut self, id: UiWindowId, event_loop: &ActiveEventLoop) -> Result<()> {
        let Some(window) = self.windows.get_mut(&id) else {
            return Ok(());
        };
        window.runtime.advance_time(self.started_at.elapsed());
        for request in window.runtime.take_task_requests() {
            let proxy = self.event_proxy.clone();
            std::thread::spawn(move || {
                request.run(move |update| {
                    let _result = proxy.send_event(UserEvent::Task { window: id, update });
                });
            });
        }
        let Some(frame) = window.runtime.render_frame() else {
            return Ok(());
        };
        if let Some(area) = frame.semantics.iter().find_map(|node| node.ime_cursor_area) {
            window.window.set_ime_cursor_area(
                LogicalPosition::new(area.x, area.y),
                LogicalSize::new(area.width, area.height),
            );
        }
        #[cfg(feature = "accessibility")]
        window
            .accesskit
            .update_if_active(|| vui_accesskit::tree_update(&frame.semantics));
        let scene = window
            .scene_builder
            .build(&frame.paint, window.scale_factor);
        let device_handle = &self.render_context.devices[window.surface.dev_id];
        window
            .renderer
            .render_to_texture(
                &device_handle.device,
                &device_handle.queue,
                scene,
                &window.surface.target_view,
                &RenderParams {
                    base_color: VelloColor::from_rgb8(0, 0, 0),
                    width: window.surface.config.width,
                    height: window.surface.config.height,
                    antialiasing_method: AaConfig::Area,
                },
            )
            .context("render Vello scene")?;

        let surface_texture = match window.surface.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture) => texture,
            wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
                self.render_context.configure_surface(&window.surface);
                texture
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.render_context.configure_surface(&window.surface);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Lost => anyhow::bail!("GPU surface was lost"),
            wgpu::CurrentSurfaceTexture::Validation => {
                event_loop.exit();
                anyhow::bail!("GPU surface validation failed");
            }
        };
        let surface_view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder =
            device_handle
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("vui surface blit"),
                });
        window.surface.blitter.copy(
            &device_handle.device,
            &mut encoder,
            &window.surface.target_view,
            &surface_view,
        );
        device_handle.queue.submit([encoder.finish()]);
        device_handle.queue.present(surface_texture);
        Ok(())
    }

    fn handle_window_state_event(
        &mut self,
        id: UiWindowId,
        event: &WindowEvent,
        event_loop: &ActiveEventLoop,
    ) -> bool {
        match event {
            WindowEvent::CloseRequested => {
                if !self.dispatch(id, &Event::WindowCloseRequested, event_loop) {
                    if id == self.primary_id && self.lifecycle.close_to_tray {
                        self.hide_primary_window();
                    } else {
                        self.remove_window(id, event_loop);
                    }
                }
            }
            WindowEvent::Resized(size) => {
                self.resize(id, size.width, size.height);
                if let Some(window) = self.windows.get(&id) {
                    let size = logical_size(size.width, size.height, window.scale_factor);
                    let event = Event::WindowResized(size);
                    self.record_session_event(id, &event);
                    self.dispatch(id, &event, event_loop);
                }
            }
            WindowEvent::Moved(position) => {
                if let Some(window) = self.windows.get(&id) {
                    let position = Point::new(
                        f64::from(position.x) / window.scale_factor,
                        f64::from(position.y) / window.scale_factor,
                    );
                    let event = Event::WindowMoved(position);
                    self.record_session_event(id, &event);
                    self.dispatch(id, &event, event_loop);
                }
                self.sync_monitor(id, event_loop);
            }
            WindowEvent::Focused(focused) => {
                self.dispatch(id, &Event::WindowFocused(*focused), event_loop);
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if let Some(window) = self.windows.get_mut(&id) {
                    window.scale_factor = *scale_factor;
                    let size = window.window.inner_size();
                    self.resize(id, size.width, size.height);
                }
                self.dispatch(
                    id,
                    &Event::WindowScaleFactorChanged(*scale_factor),
                    event_loop,
                );
                self.sync_monitor(id, event_loop);
            }
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.render(id, event_loop) {
                    self.fail(event_loop, error);
                } else {
                    self.service_platform_commands(id, event_loop);
                }
            }
            _ => return false,
        }
        true
    }

    fn handle_pointer_event(
        &mut self,
        id: UiWindowId,
        event: &WindowEvent,
        event_loop: &ActiveEventLoop,
    ) -> bool {
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                let point = self.windows.get_mut(&id).map(|window| {
                    window.pointer = Point::new(
                        position.x / window.scale_factor,
                        position.y / window.scale_factor,
                    );
                    window.pointer
                });
                if let Some(point) = point {
                    self.dispatch(id, &Event::PointerMoved(point), event_loop);
                }
            }
            WindowEvent::CursorLeft { .. } => {
                self.dispatch(id, &Event::PointerLeft, event_loop);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if let Some(button) = pointer_button(*button)
                    && let Some(window) = self.windows.get_mut(&id)
                {
                    let event = match state {
                        ElementState::Pressed => Event::PointerDown {
                            position: window.pointer,
                            button,
                            click_count: window.click_count(button),
                        },
                        ElementState::Released => Event::PointerUp {
                            position: window.pointer,
                            button,
                        },
                    };
                    self.dispatch(id, &event, event_loop);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if let Some(window) = self.windows.get(&id) {
                    let (delta_x, delta_y) = match delta {
                        MouseScrollDelta::LineDelta(x, y) => {
                            (-f64::from(*x) * 40.0, -f64::from(*y) * 40.0)
                        }
                        MouseScrollDelta::PixelDelta(position) => (
                            -position.x / window.scale_factor,
                            -position.y / window.scale_factor,
                        ),
                    };
                    let event = Event::Scroll {
                        position: window.pointer,
                        delta_x,
                        delta_y,
                    };
                    self.dispatch(id, &event, event_loop);
                }
            }
            _ => return false,
        }
        true
    }

    fn handle_keyboard_and_file_event(
        &mut self,
        id: UiWindowId,
        event: &WindowEvent,
        event_loop: &ActiveEventLoop,
    ) {
        match event {
            WindowEvent::KeyboardInput { event, .. } => {
                let key = key(event.logical_key.clone());
                let event = match event.state {
                    ElementState::Pressed => Event::KeyDown(key),
                    ElementState::Released => Event::KeyUp(key),
                };
                self.dispatch(id, &event, event_loop);
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.dispatch(
                    id,
                    &Event::ModifiersChanged(modifiers_state(modifiers.state())),
                    event_loop,
                );
            }
            WindowEvent::Ime(Ime::Commit(text)) => {
                self.dispatch(id, &Event::TextInput(text.clone()), event_loop);
            }
            WindowEvent::Ime(Ime::Preedit(text, cursor)) => {
                self.dispatch(
                    id,
                    &Event::ImePreedit {
                        text: text.clone(),
                        cursor: *cursor,
                    },
                    event_loop,
                );
            }
            WindowEvent::Ime(Ime::Disabled) => {
                self.dispatch(
                    id,
                    &Event::ImePreedit {
                        text: String::new(),
                        cursor: None,
                    },
                    event_loop,
                );
            }
            WindowEvent::HoveredFile(path) => self.dispatch_file_drag(id, path, false, event_loop),
            WindowEvent::DroppedFile(path) => self.dispatch_file_drag(id, path, true, event_loop),
            WindowEvent::HoveredFileCancelled => {
                self.dispatch(
                    id,
                    &Event::DragLeft {
                        payload: DragPayload::new(Vec::<std::path::PathBuf>::new()),
                    },
                    event_loop,
                );
            }
            _ => {}
        }
    }

    fn dispatch_file_drag(
        &mut self,
        id: UiWindowId,
        path: &Path,
        dropped: bool,
        event_loop: &ActiveEventLoop,
    ) {
        let Some(position) = self.windows.get(&id).map(|window| window.pointer) else {
            return;
        };
        let payload = DragPayload::new(vec![path.to_path_buf()]);
        let event = if dropped {
            Event::Drop {
                position,
                payload,
                operation: DragOperation::Copy,
            }
        } else {
            Event::DragEntered { position, payload }
        };
        self.dispatch(id, &event, event_loop);
    }

    fn sync_monitor(&mut self, id: UiWindowId, event_loop: &ActiveEventLoop) {
        let changed = self.windows.get_mut(&id).and_then(|window| {
            let current = monitor_info(&window.window);
            if current == window.monitor {
                return None;
            }
            window.monitor.clone_from(&current);
            current
        });
        if let Some(monitor) = changed {
            self.record_session_event(id, &Event::MonitorChanged(monitor.clone()));
            self.dispatch(id, &Event::MonitorChanged(monitor), event_loop);
        }
    }

    fn record_session_event(&mut self, id: UiWindowId, event: &Event) {
        let key = self
            .windows
            .get(&id)
            .and_then(|window| window.descriptor.key.clone());
        if let (Some(session), Some(key)) = (&mut self.session, key) {
            session.record_event(&key, event, Instant::now());
        }
    }

    fn record_window_state(
        &mut self,
        id: UiWindowId,
        maximized: Option<bool>,
        fullscreen: Option<bool>,
    ) {
        let key = self
            .windows
            .get(&id)
            .and_then(|window| window.descriptor.key.clone());
        let Some(key) = key else {
            return;
        };
        let current = self
            .session
            .as_ref()
            .and_then(|session| session.window(&key));
        let maximized = maximized.or_else(|| current.map(|window| window.maximized));
        let fullscreen = fullscreen.or_else(|| current.map(|window| window.fullscreen));
        if let (Some(session), Some(maximized), Some(fullscreen)) =
            (&mut self.session, maximized, fullscreen)
        {
            session.set_window_state(&key, maximized, fullscreen, Instant::now());
        }
    }
}

impl NativeWindow {
    fn click_count(&mut self, button: PointerButton) -> u8 {
        if button != PointerButton::Primary {
            return 1;
        }
        let now = Instant::now();
        let count = self.last_click.as_ref().map_or(1, |last| {
            let dx = last.position.x - self.pointer.x;
            let dy = last.position.y - self.pointer.y;
            if now.duration_since(last.when) <= Duration::from_millis(500)
                && dx * dx + dy * dy <= 25.0
            {
                last.count.saturating_add(1).min(3)
            } else {
                1
            }
        });
        self.last_click = Some(ClickState {
            when: now,
            position: self.pointer,
            count,
        });
        count
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.begin_startup(event_loop) || !self.windows.is_empty() {
            self.deliver_environment(event_loop);
        }
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        let ids = self.windows.keys().copied().collect::<Vec<_>>();
        for id in ids {
            self.dispatch(
                id,
                &Event::Platform(PlatformEvent::Power(PowerEvent::Suspend)),
                event_loop,
            );
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        native_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(id) = self.native_ids.get(&native_id).copied() else {
            return;
        };
        if is_input_event(&event)
            && let Some(modal) = self.modal_child(id)
        {
            modal.window.focus_window();
            modal
                .window
                .request_user_attention(Some(UserAttentionType::Informational));
            return;
        }
        #[cfg(feature = "accessibility")]
        if let Some(window) = self.windows.get_mut(&id) {
            window.accesskit.process_event(&window.window, &event);
        }
        if let WindowEvent::ThemeChanged(theme) = event {
            let mut preferences = self.services.preferences();
            preferences.appearance = match theme {
                WinitTheme::Light => SystemAppearance::Light,
                WinitTheme::Dark => SystemAppearance::Dark,
            };
            self.dispatch(
                id,
                &Event::Platform(PlatformEvent::PreferencesChanged(preferences)),
                event_loop,
            );
            return;
        }
        if !self.handle_window_state_event(id, &event, event_loop)
            && !self.handle_pointer_event(id, &event, event_loop)
        {
            self.handle_keyboard_and_file_event(id, &event, event_loop);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            #[cfg(feature = "accessibility")]
            UserEvent::AccessKit(event) => {
                let Some(id) = self.native_ids.get(&event.window_id).copied() else {
                    return;
                };
                match event.window_event {
                    accesskit_winit::WindowEvent::InitialTreeRequested => {
                        if let Some(window) = self.windows.get_mut(&id) {
                            window.runtime.request_paint();
                            window.window.request_redraw();
                        }
                    }
                    accesskit_winit::WindowEvent::ActionRequested(request) => {
                        if let Some(event) = vui_accesskit::event_for_action(&request) {
                            self.dispatch(id, &event, event_loop);
                        }
                    }
                    accesskit_winit::WindowEvent::AccessibilityDeactivated => {}
                }
            }
            UserEvent::Task { window: id, update } => {
                let handled = self
                    .windows
                    .get_mut(&id)
                    .is_some_and(|window| window.runtime.handle_task_update(update));
                if handled {
                    self.service_tasks(id);
                    self.service_platform_commands(id, event_loop);
                    if let Some(window) = self.windows.get(&id) {
                        window.window.request_redraw();
                    }
                }
            }
            UserEvent::Platform { window: id, event } => {
                if self.waiting_for_instance {
                    match &event {
                        PlatformEvent::SingleInstanceStatus { primary: false } => {
                            event_loop.exit();
                            return;
                        }
                        PlatformEvent::SingleInstanceStatus { primary: true }
                        | PlatformEvent::Failed(_) => {
                            self.waiting_for_instance = false;
                            if self.ensure_primary_window(event_loop) {
                                self.deliver_environment(event_loop);
                            }
                        }
                        _ => {}
                    }
                }
                if matches!(&event, PlatformEvent::TrayActivated(_)) {
                    self.show_primary_window();
                }
                if let PlatformEvent::PreferencesChanged(preferences) = &event {
                    self.apply_preferences(preferences);
                }
                if let PlatformEvent::SecondInstance(arguments) = &event {
                    self.show_primary_window();
                    let (files, urls) = classify_launch_arguments(arguments);
                    if !files.is_empty() {
                        self.dispatch(
                            id,
                            &Event::Platform(PlatformEvent::OpenFiles(files)),
                            event_loop,
                        );
                    }
                    if !urls.is_empty() {
                        self.dispatch(
                            id,
                            &Event::Platform(PlatformEvent::OpenUrls(urls)),
                            event_loop,
                        );
                    }
                }
                let event = match event {
                    PlatformEvent::CommandInvoked(command) => Event::CommandInvoked(command),
                    PlatformEvent::FileDialogCompleted { id, paths } => {
                        Event::FileDialogCompleted { id, paths }
                    }
                    PlatformEvent::FileDialogCancelled { id } => Event::FileDialogCancelled { id },
                    event => Event::Platform(event),
                };
                self.dispatch(id, &event, event_loop);
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let platform_events = self.services.poll_events();
        for (id, event) in platform_events {
            let event = match event {
                PlatformEvent::CommandInvoked(command) => Event::CommandInvoked(command),
                event => Event::Platform(event),
            };
            self.dispatch(id, &event, event_loop);
        }
        if let Some(session) = &mut self.session
            && let Err(error) = session.flush_if_due(Instant::now())
        {
            self.fail(event_loop, error);
            return;
        }
        let now = self.started_at.elapsed();
        let deadline = self
            .windows
            .values()
            .filter_map(|window| window.runtime.next_animation_deadline())
            .min();
        let platform_deadline = self
            .services
            .poll_interval()
            .map(|interval| Instant::now() + interval);
        let Some(deadline) = deadline else {
            event_loop.set_control_flow(
                platform_deadline.map_or(ControlFlow::Wait, ControlFlow::WaitUntil),
            );
            return;
        };
        if deadline <= now {
            for window in self.windows.values() {
                if window
                    .runtime
                    .next_animation_deadline()
                    .is_some_and(|value| value <= now)
                {
                    window.window.request_redraw();
                }
            }
            event_loop.set_control_flow(ControlFlow::Wait);
        } else {
            let animation_deadline = self.started_at + deadline;
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                platform_deadline.map_or(animation_deadline, |platform| {
                    animation_deadline.min(platform)
                }),
            ));
        }
    }
}

fn launch_uri(uri: &str) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    let _child = Command::new("xdg-open").arg(uri).spawn()?;
    #[cfg(target_os = "macos")]
    let _child = Command::new("open").arg(uri).spawn()?;
    #[cfg(target_os = "windows")]
    let _child = Command::new("cmd").args(["/C", "start", "", uri]).spawn()?;
    Ok(())
}

fn reveal_path(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    let _child = Command::new("xdg-open")
        .arg(path.parent().unwrap_or(path))
        .spawn()?;
    #[cfg(target_os = "macos")]
    let _child = Command::new("open").arg("-R").arg(path).spawn()?;
    #[cfg(target_os = "windows")]
    let _child = Command::new("explorer")
        .arg(format!("/select,{}", path.display()))
        .spawn()?;
    Ok(())
}

fn modifiers_state(state: ModifiersState) -> Modifiers {
    let mut modifiers = Modifiers::default();
    modifiers.set(Modifier::Shift, state.shift_key());
    modifiers.set(Modifier::Control, state.control_key());
    modifiers.set(Modifier::Alt, state.alt_key());
    modifiers.set(Modifier::Meta, state.super_key());
    modifiers
}

fn logical_winit_size(size: Size) -> LogicalSize<f64> {
    LogicalSize::new(size.width, size.height)
}

fn logical_size(width: u32, height: u32, scale: f64) -> Size {
    Size::new(f64::from(width) / scale, f64::from(height) / scale)
}

fn monitor_info(window: &Window) -> Option<MonitorInfo> {
    let monitor = window.current_monitor()?;
    Some(monitor_handle_info(&monitor))
}

fn monitor_handle_info(monitor: &MonitorHandle) -> MonitorInfo {
    let scale_factor = monitor.scale_factor();
    let position = monitor.position();
    let size = monitor.size();
    MonitorInfo {
        name: monitor.name(),
        position: Point::new(
            f64::from(position.x) / scale_factor,
            f64::from(position.y) / scale_factor,
        ),
        size: logical_size(size.width, size.height, scale_factor),
        scale_factor,
    }
}

fn pointer_button(button: MouseButton) -> Option<PointerButton> {
    match button {
        MouseButton::Left => Some(PointerButton::Primary),
        MouseButton::Right => Some(PointerButton::Secondary),
        MouseButton::Middle => Some(PointerButton::Middle),
        MouseButton::Back | MouseButton::Forward | MouseButton::Other(_) => None,
    }
}

fn is_input_event(event: &WindowEvent) -> bool {
    matches!(
        event,
        WindowEvent::CursorMoved { .. }
            | WindowEvent::CursorLeft { .. }
            | WindowEvent::MouseInput { .. }
            | WindowEvent::MouseWheel { .. }
            | WindowEvent::KeyboardInput { .. }
            | WindowEvent::ModifiersChanged(_)
            | WindowEvent::Ime(_)
            | WindowEvent::Touch(_)
    )
}

fn key(key: WinitKey) -> Key {
    match key {
        WinitKey::Named(NamedKey::Enter) => Key::Enter,
        WinitKey::Named(NamedKey::Space) => Key::Space,
        WinitKey::Named(NamedKey::Tab) => Key::Tab,
        WinitKey::Named(NamedKey::Escape) => Key::Escape,
        WinitKey::Named(NamedKey::Backspace) => Key::Backspace,
        WinitKey::Named(NamedKey::Delete) => Key::Delete,
        WinitKey::Named(NamedKey::ArrowLeft) => Key::ArrowLeft,
        WinitKey::Named(NamedKey::ArrowRight) => Key::ArrowRight,
        WinitKey::Named(NamedKey::ArrowUp) => Key::ArrowUp,
        WinitKey::Named(NamedKey::ArrowDown) => Key::ArrowDown,
        WinitKey::Named(NamedKey::Home) => Key::Home,
        WinitKey::Named(NamedKey::End) => Key::End,
        WinitKey::Named(NamedKey::PageUp) => Key::PageUp,
        WinitKey::Named(NamedKey::PageDown) => Key::PageDown,
        WinitKey::Character(value) => Key::Character(value.to_string()),
        _ => Key::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::{decode_arguments, encode_arguments, instance_port};

    #[test]
    fn single_instance_arguments_round_trip() {
        let arguments = vec!["vui".to_owned(), "document one.txt".to_owned()];
        assert_eq!(decode_arguments(&encode_arguments(&arguments)), arguments);
        assert!(decode_arguments(&[]).is_empty());
    }

    #[test]
    fn application_id_maps_to_stable_private_port_range() {
        let first = instance_port("dev.vui.showcase");
        assert_eq!(first, instance_port("dev.vui.showcase"));
        assert!((40_000..60_000).contains(&first));
    }
}
