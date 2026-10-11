//! Linux XDG portal and D-Bus implementation of VUI desktop services.

use ashpd::{
    Uri,
    desktop::{
        inhibit::{CreateMonitorOptions, InhibitFlags, InhibitOptions, InhibitProxy},
        notification::{Button, Notification, NotificationProxy},
        open_uri::{OpenDirectoryRequest, OpenFileRequest},
        power_profile_monitor::PowerProfileMonitor,
        settings::{ColorScheme, Contrast, ReducedMotion, Settings},
    },
};
use enumflags2::make_bitflags;
use futures_util::StreamExt;
use std::{
    collections::{BTreeSet, VecDeque},
    fmt::Write,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use vui_core::{
    Color, DesktopNotification, DesktopSessionEvent, PlatformEvent, PlatformRequest, PowerEvent,
    SystemAppearance, SystemPreferences, WindowId,
};
use vui_platform::{PlatformError, PlatformResponder, PlatformServices};
use vui_winit::{DesktopPlatformServices, RunOptions};
use zbus::fdo::{RequestNameFlags, RequestNameReply};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinuxDisplayServer {
    Wayland,
    X11,
    WaylandAndX11,
    Headless,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinuxCapabilities {
    available: BTreeSet<LinuxCapability>,
    pub display_server: LinuxDisplayServer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LinuxCapability {
    SessionBus,
    PortalSettings,
    PortalOpenUri,
    PortalNotifications,
    PortalInhibit,
    PortalPowerProfiles,
    NativeTray,
    OutboundDrag,
}

impl LinuxCapabilities {
    #[must_use]
    pub fn detect() -> Self {
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
        let x11 = std::env::var_os("DISPLAY").is_some();
        let display_server = match (wayland, x11) {
            (true, true) => LinuxDisplayServer::WaylandAndX11,
            (true, false) => LinuxDisplayServer::Wayland,
            (false, true) => LinuxDisplayServer::X11,
            (false, false) => LinuxDisplayServer::Headless,
        };
        let mut available = BTreeSet::from([LinuxCapability::NativeTray]);
        let session = zbus::blocking::Connection::session().ok();
        if session.is_some() {
            available.insert(LinuxCapability::SessionBus);
        }
        let portal = session.as_ref().is_some_and(|connection| {
            zbus::blocking::Proxy::new(
                connection,
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
            )
            .and_then(|proxy| {
                proxy.call::<_, _, bool>("NameHasOwner", &("org.freedesktop.portal.Desktop",))
            })
            .unwrap_or(false)
        });
        if portal {
            available.extend([
                LinuxCapability::PortalSettings,
                LinuxCapability::PortalOpenUri,
                LinuxCapability::PortalNotifications,
                LinuxCapability::PortalInhibit,
                LinuxCapability::PortalPowerProfiles,
            ]);
        }
        Self {
            available,
            display_server,
        }
    }

    #[must_use]
    pub fn supports(&self, capability: LinuxCapability) -> bool {
        self.available.contains(&capability)
    }

    pub fn available(&self) -> impl Iterator<Item = LinuxCapability> + '_ {
        self.available.iter().copied()
    }
}

struct ActivationEndpoint {
    target: Arc<Mutex<Option<(WindowId, PlatformResponder)>>>,
}

#[zbus::interface(name = "dev.vui.Application")]
impl ActivationEndpoint {
    fn activate(&self, arguments: Vec<String>) {
        let target = self
            .target
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some((window, respond)) = target {
            respond(window, PlatformEvent::SecondInstance(arguments));
        }
    }
}

/// Portal-first Linux services with the portable Winit backend as fallback.
pub struct LinuxPlatformServices {
    capabilities: LinuxCapabilities,
    fallback: DesktopPlatformServices,
    preferences: Arc<Mutex<SystemPreferences>>,
    events: Arc<Mutex<VecDeque<PlatformEvent>>>,
    origin: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    inhibition: Option<ashpd::desktop::Request<()>>,
    activation_connection: Option<zbus::blocking::Connection>,
    activation_target: Arc<Mutex<Option<(WindowId, PlatformResponder)>>>,
}

impl LinuxPlatformServices {
    /// Creates a portal-backed service and records unavailable capabilities.
    ///
    /// # Errors
    ///
    /// Reserved for initialization failures that prevent all fallback service.
    pub fn new() -> Result<Self, PlatformError> {
        Self::with_options(RunOptions::default())
    }

    /// Creates a service with explicit accessibility preferences.
    ///
    /// # Errors
    ///
    /// Reserved for initialization failures that prevent all fallback service.
    pub fn with_options(options: RunOptions) -> Result<Self, PlatformError> {
        let capabilities = LinuxCapabilities::detect();
        let preferences = Arc::new(Mutex::new(SystemPreferences {
            reduced_motion: options.reduced_motion,
            ..SystemPreferences::default()
        }));
        let events = Arc::new(Mutex::new(VecDeque::new()));
        let stop = Arc::new(AtomicBool::new(false));
        if capabilities.supports(LinuxCapability::PortalSettings) {
            watch_settings(preferences.clone(), events.clone(), stop.clone());
        }
        if capabilities.supports(LinuxCapability::PortalPowerProfiles) {
            watch_power_profiles(events.clone(), stop.clone());
        }
        if capabilities.supports(LinuxCapability::PortalInhibit) {
            watch_session(events.clone(), stop.clone());
        }
        Ok(Self {
            capabilities,
            fallback: DesktopPlatformServices::new(options),
            preferences,
            events,
            origin: Arc::new(AtomicU64::new(0)),
            stop,
            inhibition: None,
            activation_connection: None,
            activation_target: Arc::new(Mutex::new(None)),
        })
    }

    #[must_use]
    pub const fn capabilities(&self) -> &LinuxCapabilities {
        &self.capabilities
    }

    fn open_uri(
        origin: WindowId,
        uri: &str,
        respond: PlatformResponder,
    ) -> Result<(), PlatformError> {
        let parsed =
            Uri::parse(uri).map_err(|error| PlatformError::new(format!("invalid URI: {error}")))?;
        std::thread::spawn(move || {
            let result = async_io::block_on(async {
                OpenFileRequest::default()
                    .send_uri(&parsed)
                    .await?
                    .response()
            });
            if let Err(error) = result {
                respond(origin, PlatformEvent::Failed(format!("open URI: {error}")));
            }
        });
        Ok(())
    }

    fn reveal_path(origin: WindowId, path: PathBuf, respond: PlatformResponder) {
        let directory = if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or(&path).to_path_buf()
        };
        std::thread::spawn(move || {
            let result = fs::File::open(&directory)
                .map_err(ashpd::Error::IO)
                .and_then(|directory| {
                    async_io::block_on(async {
                        OpenDirectoryRequest::default()
                            .send(&directory)
                            .await?
                            .response()
                    })
                });
            if let Err(error) = result {
                respond(
                    origin,
                    PlatformEvent::Failed(format!("reveal path: {error}")),
                );
            }
        });
    }

    fn notify(origin: WindowId, notification: DesktopNotification, respond: PlatformResponder) {
        std::thread::spawn(move || {
            let result = async_io::block_on(async {
                let proxy = NotificationProxy::new().await?;
                let mut native =
                    Notification::new(&notification.title).body(notification.body.as_str());
                for action in &notification.actions {
                    native = native.button(Button::new(&action.label, &action.id));
                }
                if notification.actions.is_empty() {
                    proxy.add_notification(&notification.id, native).await?;
                    return Ok::<(), ashpd::Error>(());
                }
                let mut actions = proxy.receive_action_invoked().await?;
                proxy.add_notification(&notification.id, native).await?;
                while let Some(action) = actions.next().await {
                    if action.id() == notification.id {
                        respond(
                            origin,
                            PlatformEvent::NotificationAction {
                                notification: notification.id.clone(),
                                action: action.name().to_owned(),
                            },
                        );
                        break;
                    }
                }
                Ok::<(), ashpd::Error>(())
            });
            if let Err(error) = result {
                respond(
                    origin,
                    PlatformEvent::Failed(format!("notification: {error}")),
                );
            }
        });
    }

    fn set_inhibition(&mut self, inhibited: bool) -> Result<(), PlatformError> {
        if inhibited && self.inhibition.is_none() {
            let request = async_io::block_on(async {
                InhibitProxy::new()
                    .await?
                    .inhibit(
                        None,
                        make_bitflags!(InhibitFlags::{Suspend | Idle}),
                        InhibitOptions::default()
                            .set_reason(Some("VUI application is performing active work")),
                    )
                    .await
            })
            .map_err(|error| PlatformError::new(format!("power inhibition: {error}")))?;
            request
                .response()
                .map_err(|error| PlatformError::new(format!("power inhibition: {error}")))?;
            self.inhibition = Some(request);
        } else if !inhibited && let Some(request) = self.inhibition.take() {
            async_io::block_on(request.close())
                .map_err(|error| PlatformError::new(format!("release inhibition: {error}")))?;
        }
        Ok(())
    }

    fn register_dbus_instance(
        &mut self,
        origin: WindowId,
        application_id: &str,
        arguments: &[String],
        respond: &PlatformResponder,
    ) -> Result<(), PlatformError> {
        *self
            .activation_target
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((origin, respond.clone()));
        let endpoint = ActivationEndpoint {
            target: self.activation_target.clone(),
        };
        let connection = zbus::blocking::Connection::session()
            .map_err(|error| PlatformError::new(format!("D-Bus activation: {error}")))?;
        connection
            .object_server()
            .at("/dev/vui/Application", endpoint)
            .map_err(|error| PlatformError::new(format!("D-Bus activation: {error}")))?;
        let ownership = connection.request_name_with_flags(
            application_id,
            make_bitflags!(RequestNameFlags::{DoNotQueue}),
        );
        match ownership {
            Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => {
                self.activation_connection = Some(connection);
                respond(
                    origin,
                    PlatformEvent::SingleInstanceStatus { primary: true },
                );
                Ok(())
            }
            Ok(RequestNameReply::Exists | RequestNameReply::InQueue)
            | Err(zbus::Error::NameTaken) => {
                let proxy = zbus::blocking::Proxy::new(
                    &connection,
                    application_id,
                    "/dev/vui/Application",
                    "dev.vui.Application",
                )
                .map_err(|error| PlatformError::new(format!("D-Bus activation: {error}")))?;
                let _: () = proxy
                    .call("Activate", &(arguments,))
                    .map_err(|error| PlatformError::new(format!("D-Bus activation: {error}")))?;
                respond(
                    origin,
                    PlatformEvent::SingleInstanceStatus { primary: false },
                );
                Ok(())
            }
            Err(error) => Err(PlatformError::new(format!("D-Bus activation: {error}"))),
        }
    }
}

impl Default for LinuxPlatformServices {
    fn default() -> Self {
        Self::new().expect("portable Linux fallback is always available")
    }
}

impl Drop for LinuxPlatformServices {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(request) = self.inhibition.take() {
            let _result = async_io::block_on(request.close());
        }
    }
}

impl PlatformServices for LinuxPlatformServices {
    fn request(
        &mut self,
        origin: WindowId,
        request: PlatformRequest,
        respond: PlatformResponder,
    ) -> Result<(), PlatformError> {
        self.origin.store(origin.raw(), Ordering::Relaxed);
        match request {
            PlatformRequest::OpenUri(uri)
                if self.capabilities.supports(LinuxCapability::PortalOpenUri) =>
            {
                Self::open_uri(origin, &uri, respond)
            }
            PlatformRequest::RevealPath(path)
                if self.capabilities.supports(LinuxCapability::PortalOpenUri) =>
            {
                Self::reveal_path(origin, path, respond);
                Ok(())
            }
            PlatformRequest::Notification(notification)
                if self
                    .capabilities
                    .supports(LinuxCapability::PortalNotifications) =>
            {
                Self::notify(origin, notification, respond);
                Ok(())
            }
            PlatformRequest::SetPowerInhibition(inhibited)
                if self.capabilities.supports(LinuxCapability::PortalInhibit) =>
            {
                self.set_inhibition(inhibited)
            }
            PlatformRequest::RegisterSingleInstance {
                application_id,
                arguments,
            } if self.capabilities.supports(LinuxCapability::SessionBus) => self
                .register_dbus_instance(origin, &application_id, &arguments, &respond)
                .or_else(|_| {
                    self.fallback.request(
                        origin,
                        PlatformRequest::RegisterSingleInstance {
                            application_id,
                            arguments,
                        },
                        respond,
                    )
                }),
            PlatformRequest::StartDrag { .. } => {
                let message = match self.capabilities.display_server {
                    LinuxDisplayServer::Wayland | LinuxDisplayServer::WaylandAndX11 => {
                        "Wayland outbound drag requires a compositor data-device handle"
                    }
                    LinuxDisplayServer::X11 => {
                        "X11 outbound drag requires an XDND source window handle"
                    }
                    LinuxDisplayServer::Headless => {
                        "outbound drag is unavailable without a display server"
                    }
                };
                respond(origin, PlatformEvent::Failed(message.to_owned()));
                Ok(())
            }
            request => self.fallback.request(origin, request, respond),
        }
    }

    fn preferences(&self) -> SystemPreferences {
        self.preferences
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn poll_events(&mut self) -> Vec<(WindowId, PlatformEvent)> {
        let mut events = self.fallback.poll_events();
        let origin = self.origin.load(Ordering::Relaxed);
        if origin != 0 {
            events.extend(
                self.events
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .drain(..)
                    .map(|event| (WindowId::from_raw(origin), event)),
            );
        }
        events
    }

    fn poll_interval(&self) -> Option<Duration> {
        self.fallback
            .poll_interval()
            .or(Some(Duration::from_millis(250)))
    }
}

fn read_preferences(default_reduced_motion: bool) -> SystemPreferences {
    async_io::block_on(async {
        let Ok(settings) = Settings::new().await else {
            return SystemPreferences {
                reduced_motion: default_reduced_motion,
                ..SystemPreferences::default()
            };
        };
        let appearance = match settings.color_scheme().await.unwrap_or_default() {
            ColorScheme::PreferDark => SystemAppearance::Dark,
            ColorScheme::PreferLight => SystemAppearance::Light,
            ColorScheme::NoPreference => SystemAppearance::Unknown,
        };
        let appearance = if settings.contrast().await.unwrap_or_default() == Contrast::High {
            SystemAppearance::HighContrast
        } else {
            appearance
        };
        let accent = settings.accent_color().await.ok().map(|color| {
            Color::rgb(
                channel(color.red()),
                channel(color.green()),
                channel(color.blue()),
            )
        });
        let reduced_motion = settings
            .reduced_motion()
            .await
            .map_or(default_reduced_motion, |preference| {
                preference == ReducedMotion::ReducedMotion
            });
        SystemPreferences {
            appearance,
            accent,
            reduced_motion,
        }
    })
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn channel(value: f64) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn watch_settings(
    preferences: Arc<Mutex<SystemPreferences>>,
    events: Arc<Mutex<VecDeque<PlatformEvent>>>,
    stop: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        async_io::block_on(async move {
            let Ok(settings) = Settings::new().await else {
                return;
            };
            let initial = read_preferences(false);
            *preferences
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = initial.clone();
            events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push_back(PlatformEvent::PreferencesChanged(initial));
            let Ok(changes) = settings.receive_setting_changed().await else {
                return;
            };
            futures_util::pin_mut!(changes);
            while changes.next().await.is_some() && !stop.load(Ordering::Relaxed) {
                let updated = read_preferences(false);
                *preferences
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = updated.clone();
                events
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push_back(PlatformEvent::PreferencesChanged(updated));
            }
        });
    });
}

fn watch_power_profiles(events: Arc<Mutex<VecDeque<PlatformEvent>>>, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let mut previous = None;
        while !stop.load(Ordering::Relaxed) {
            let current = async_io::block_on(async {
                let proxy = PowerProfileMonitor::new().await.ok()?;
                proxy.is_enabled().await.ok()
            });
            if current.is_some() && current != previous {
                previous = current;
                events
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push_back(PlatformEvent::Power(PowerEvent::LowPowerMode(
                        current.unwrap_or(false),
                    )));
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

fn watch_session(events: Arc<Mutex<VecDeque<PlatformEvent>>>, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        async_io::block_on(async move {
            let Ok(proxy) = InhibitProxy::new().await else {
                return;
            };
            let Ok(_session) = proxy
                .create_monitor(None, CreateMonitorOptions::default())
                .await
            else {
                return;
            };
            let Ok(states) = proxy.receive_state_changed().await else {
                return;
            };
            futures_util::pin_mut!(states);
            while let Some(state) = states.next().await {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let event = if state.screensaver_active() {
                    PlatformEvent::Session(DesktopSessionEvent::Locked)
                } else {
                    PlatformEvent::Session(DesktopSessionEvent::Unlocked)
                };
                events
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push_back(event);
            }
        });
    });
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinuxDesktopEntry {
    pub application_id: String,
    pub name: String,
    pub executable: String,
    pub icon: Option<String>,
    pub mime_types: Vec<String>,
    pub url_schemes: Vec<String>,
    pub categories: Vec<String>,
}

impl LinuxDesktopEntry {
    #[must_use]
    pub fn new(
        application_id: impl Into<String>,
        name: impl Into<String>,
        executable: impl Into<String>,
    ) -> Self {
        Self {
            application_id: application_id.into(),
            name: name.into(),
            executable: executable.into(),
            icon: None,
            mime_types: Vec::new(),
            url_schemes: Vec::new(),
            categories: Vec::new(),
        }
    }

    #[must_use]
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    #[must_use]
    pub fn mime_type(mut self, mime_type: impl Into<String>) -> Self {
        self.mime_types.push(mime_type.into());
        self
    }

    #[must_use]
    pub fn url_scheme(mut self, scheme: impl Into<String>) -> Self {
        self.url_schemes.push(scheme.into());
        self
    }

    #[must_use]
    pub fn category(mut self, category: impl Into<String>) -> Self {
        self.categories.push(category.into());
        self
    }

    /// Renders a freedesktop.org desktop entry.
    ///
    /// # Errors
    ///
    /// Returns an error for values that cannot be safely represented.
    pub fn render(&self) -> Result<String, PlatformError> {
        validate_application_id(&self.application_id)?;
        for value in [&self.name, &self.executable] {
            validate_entry_value(value)?;
        }
        for scheme in &self.url_schemes {
            if scheme.is_empty()
                || !scheme
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || "+-.".contains(character))
            {
                return Err(PlatformError::new(format!("invalid URL scheme `{scheme}`")));
            }
        }
        let mut output = format!(
            "[Desktop Entry]\nType=Application\nName={}\nExec={} %U\nTerminal=false\nStartupNotify=true\nDBusActivatable=false\nX-GNOME-UsesNotifications=true\n",
            escape_entry_value(&self.name),
            escape_entry_value(&self.executable),
        );
        if let Some(icon) = &self.icon {
            validate_entry_value(icon)?;
            writeln!(&mut output, "Icon={}", escape_entry_value(icon))
                .expect("writing to a String cannot fail");
        }
        let mut mime_types = self.mime_types.clone();
        mime_types.extend(
            self.url_schemes
                .iter()
                .map(|scheme| format!("x-scheme-handler/{scheme}")),
        );
        if !mime_types.is_empty() {
            writeln!(&mut output, "MimeType={};", mime_types.join(";"))
                .expect("writing to a String cannot fail");
        }
        if !self.categories.is_empty() {
            writeln!(&mut output, "Categories={};", self.categories.join(";"))
                .expect("writing to a String cannot fail");
        }
        Ok(output)
    }

    /// Atomically installs the desktop entry below an applications directory.
    ///
    /// # Errors
    ///
    /// Returns an error when rendering or filesystem operations fail.
    pub fn install_into(&self, applications_directory: &Path) -> Result<PathBuf, PlatformError> {
        let contents = self.render()?;
        fs::create_dir_all(applications_directory)
            .map_err(|error| PlatformError::new(error.to_string()))?;
        let path = applications_directory.join(format!("{}.desktop", self.application_id));
        let temporary =
            applications_directory.join(format!(".{}.desktop.tmp", self.application_id));
        fs::write(&temporary, contents).map_err(|error| PlatformError::new(error.to_string()))?;
        fs::rename(&temporary, &path).map_err(|error| PlatformError::new(error.to_string()))?;
        Ok(path)
    }
}

fn validate_entry_value(value: &str) -> Result<(), PlatformError> {
    if value.is_empty() || value.contains(['\n', '\r', '\0']) {
        return Err(PlatformError::new("invalid desktop-entry value"));
    }
    Ok(())
}

fn validate_application_id(value: &str) -> Result<(), PlatformError> {
    if value.is_empty()
        || value.starts_with('.')
        || value.ends_with('.')
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".-_".contains(character))
    {
        return Err(PlatformError::new("invalid application ID"));
    }
    Ok(())
}

fn escape_entry_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace(';', "\\;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn desktop_entry_contains_mime_and_scheme_handlers() {
        let entry = LinuxDesktopEntry::new("dev.vui.showcase", "VUI Showcase", "vui-showcase")
            .icon("dev.vui.showcase")
            .mime_type("text/plain")
            .url_scheme("vui")
            .category("Utility");
        let rendered = entry.render().expect("valid desktop entry");
        assert!(rendered.contains("DBusActivatable=false"));
        assert!(rendered.contains("MimeType=text/plain;x-scheme-handler/vui;"));
        assert!(rendered.contains("Categories=Utility;"));
    }

    #[test]
    fn desktop_entry_rejects_injection_and_invalid_schemes() {
        assert!(
            LinuxDesktopEntry::new("dev.vui.test", "Bad\nHidden=true", "vui")
                .render()
                .is_err()
        );
        assert!(
            LinuxDesktopEntry::new("dev.vui.test", "VUI", "vui")
                .url_scheme("not a scheme")
                .render()
                .is_err()
        );
        assert!(
            LinuxDesktopEntry::new("../escape", "VUI", "vui")
                .render()
                .is_err()
        );
    }

    #[test]
    fn color_channels_are_clamped_and_rounded() {
        assert_eq!(channel(-1.0), 0);
        assert_eq!(channel(0.5), 128);
        assert_eq!(channel(2.0), 255);
    }

    #[test]
    fn dbus_single_instance_forwards_to_primary() {
        if zbus::blocking::Connection::session().is_err() {
            return;
        }
        let application_id = format!("dev.vui.test.p{}", std::process::id());
        let statuses = Arc::new(Mutex::new(Vec::new()));
        let received = statuses.clone();
        let responder: PlatformResponder = Arc::new(move |_window, event| {
            received
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(event);
        });
        let mut primary = LinuxPlatformServices::new().expect("primary services");
        let mut secondary = LinuxPlatformServices::new().expect("secondary services");
        primary
            .register_dbus_instance(
                WindowId::from_raw(1),
                &application_id,
                &["primary".to_owned()],
                &responder,
            )
            .expect("register primary");
        secondary
            .register_dbus_instance(
                WindowId::from_raw(2),
                &application_id,
                &["secondary".to_owned(), "document.txt".to_owned()],
                &responder,
            )
            .expect("forward secondary");
        let statuses = statuses
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(statuses.contains(&PlatformEvent::SingleInstanceStatus { primary: true }));
        assert!(statuses.contains(&PlatformEvent::SingleInstanceStatus { primary: false }));
        assert!(statuses.contains(&PlatformEvent::SecondInstance(vec![
            "secondary".to_owned(),
            "document.txt".to_owned(),
        ])));
    }
}
