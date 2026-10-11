use crate::{CommandDescriptor, CommandId, DragOperation, RasterImage, WindowId};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeMenu {
    pub items: Vec<NativeMenuItem>,
}

impl NativeMenu {
    #[must_use]
    pub fn new(items: impl IntoIterator<Item = NativeMenuItem>) -> Self {
        Self {
            items: items.into_iter().collect(),
        }
    }

    #[must_use]
    pub fn commands(commands: impl IntoIterator<Item = CommandDescriptor>) -> Self {
        Self::new(commands.into_iter().map(NativeMenuItem::Command))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeMenuItem {
    Command(CommandDescriptor),
    Submenu { label: String, menu: NativeMenu },
    Separator,
}

impl NativeMenuItem {
    #[must_use]
    pub fn submenu(label: impl Into<String>, menu: NativeMenu) -> Self {
        Self::Submenu {
            label: label.into(),
            menu,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationAction {
    pub id: String,
    pub label: String,
}

impl NotificationAction {
    #[must_use]
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesktopNotification {
    pub id: String,
    pub title: String,
    pub body: String,
    pub actions: Vec<NotificationAction>,
}

impl DesktopNotification {
    #[must_use]
    pub fn new(id: impl Into<String>, title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            body: body.into(),
            actions: Vec::new(),
        }
    }

    #[must_use]
    pub fn action(mut self, id: impl Into<String>, label: impl Into<String>) -> Self {
        self.actions.push(NotificationAction::new(id, label));
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayDescriptor {
    pub id: String,
    pub tooltip: String,
    pub icon: Option<RasterImage>,
    pub menu: NativeMenu,
}

impl TrayDescriptor {
    #[must_use]
    pub fn new(id: impl Into<String>, tooltip: impl Into<String>, menu: NativeMenu) -> Self {
        Self {
            id: id.into(),
            tooltip: tooltip.into(),
            icon: None,
            menu,
        }
    }

    #[must_use]
    pub fn icon(mut self, icon: RasterImage) -> Self {
        self.icon = Some(icon);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutboundDragData {
    Text(String),
    Files(Vec<PathBuf>),
    Image(RasterImage),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SystemAppearance {
    Light,
    Dark,
    HighContrast,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SystemPreferences {
    pub appearance: SystemAppearance,
    pub accent: Option<crate::Color>,
    pub reduced_motion: bool,
}

impl Default for SystemPreferences {
    fn default() -> Self {
        Self {
            appearance: SystemAppearance::Unknown,
            accent: None,
            reduced_motion: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerEvent {
    Suspend,
    Resume,
    LowPowerMode(bool),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopSessionEvent {
    Locked,
    Unlocked,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PlatformRequest {
    OpenUri(String),
    RevealPath(PathBuf),
    FileDialog(crate::FileDialogRequest),
    Notification(DesktopNotification),
    SetMenuBar {
        window: WindowId,
        menu: NativeMenu,
    },
    SetTray(TrayDescriptor),
    RemoveTray(String),
    StartDrag {
        window: WindowId,
        data: OutboundDragData,
        allowed: crate::DragOperations,
    },
    RegisterSingleInstance {
        application_id: String,
        arguments: Vec<String>,
    },
    SetPowerInhibition(bool),
}

#[derive(Clone, Debug, PartialEq)]
pub enum PlatformEvent {
    CommandInvoked(CommandId),
    NotificationAction {
        notification: String,
        action: String,
    },
    NotificationClosed(String),
    TrayActivated(String),
    DragCompleted(DragOperation),
    DragCancelled,
    OpenUrls(Vec<String>),
    OpenFiles(Vec<PathBuf>),
    FileDialogCompleted {
        id: crate::PlatformRequestId,
        paths: Vec<PathBuf>,
    },
    FileDialogCancelled {
        id: crate::PlatformRequestId,
    },
    SingleInstanceStatus {
        primary: bool,
    },
    SecondInstance(Vec<String>),
    PreferencesChanged(SystemPreferences),
    Power(PowerEvent),
    Session(DesktopSessionEvent),
    Failed(String),
}
