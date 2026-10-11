use crate::{Size, Widget};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(u64);

impl WindowId {
    #[must_use]
    pub fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }

    #[must_use]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowDescriptor {
    pub key: Option<String>,
    pub title: String,
    pub size: Size,
    pub min_size: Option<Size>,
    pub max_size: Option<Size>,
    pub position: Option<crate::Point>,
    pub parent: Option<WindowId>,
    flags: u8,
}

impl WindowDescriptor {
    #[must_use]
    pub fn new(title: impl Into<String>, size: Size) -> Self {
        Self {
            key: None,
            title: title.into(),
            size,
            min_size: None,
            max_size: None,
            position: None,
            parent: None,
            flags: Self::RESIZABLE,
        }
    }

    #[must_use]
    pub const fn parent(mut self, parent: WindowId) -> Self {
        self.parent = Some(parent);
        self
    }

    #[must_use]
    pub const fn modal(mut self, modal: bool) -> Self {
        self.set_flag(Self::MODAL, modal);
        self
    }

    #[must_use]
    pub const fn min_size(mut self, size: Size) -> Self {
        self.min_size = Some(size);
        self
    }

    #[must_use]
    pub const fn max_size(mut self, size: Size) -> Self {
        self.max_size = Some(size);
        self
    }

    #[must_use]
    pub const fn position(mut self, position: crate::Point) -> Self {
        self.position = Some(position);
        self
    }

    #[must_use]
    pub const fn resizable(mut self, resizable: bool) -> Self {
        self.set_flag(Self::RESIZABLE, resizable);
        self
    }

    #[must_use]
    pub const fn transparent(mut self, transparent: bool) -> Self {
        self.set_flag(Self::TRANSPARENT, transparent);
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    #[must_use]
    pub const fn singleton(mut self, singleton: bool) -> Self {
        self.set_flag(Self::SINGLETON, singleton);
        self
    }

    #[must_use]
    pub const fn is_modal(&self) -> bool {
        self.flags & Self::MODAL != 0
    }

    #[must_use]
    pub const fn is_resizable(&self) -> bool {
        self.flags & Self::RESIZABLE != 0
    }

    #[must_use]
    pub const fn is_transparent(&self) -> bool {
        self.flags & Self::TRANSPARENT != 0
    }

    #[must_use]
    pub const fn is_singleton(&self) -> bool {
        self.flags & Self::SINGLETON != 0
    }

    const MODAL: u8 = 1;
    const RESIZABLE: u8 = 1 << 1;
    const TRANSPARENT: u8 = 1 << 2;
    const SINGLETON: u8 = 1 << 3;

    const fn set_flag(&mut self, flag: u8, enabled: bool) {
        if enabled {
            self.flags |= flag;
        } else {
            self.flags &= !flag;
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MonitorInfo {
    pub name: Option<String>,
    pub position: crate::Point,
    pub size: Size,
    pub scale_factor: f64,
}

pub enum WindowCommand {
    Open {
        id: WindowId,
        descriptor: WindowDescriptor,
        root: Box<dyn Widget>,
    },
    Close(WindowId),
    Focus(WindowId),
    Minimize(WindowId, bool),
    Maximize(WindowId, bool),
    Fullscreen(WindowId, bool),
    RequestAttention(WindowId),
    RefreshAll,
    QuitApplication,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PlatformRequestId(u64);

impl PlatformRequestId {
    #[must_use]
    pub fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }

    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileDialogKind {
    OpenFile,
    SaveFile,
    SelectFolder,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDialogRequest {
    pub id: PlatformRequestId,
    pub kind: FileDialogKind,
    pub title: Option<String>,
    pub starting_directory: Option<PathBuf>,
    pub filters: Vec<FileDialogFilter>,
    pub allow_multiple: bool,
}

impl FileDialogRequest {
    #[must_use]
    pub fn new(kind: FileDialogKind) -> Self {
        Self {
            id: PlatformRequestId::next(),
            kind,
            title: None,
            starting_directory: None,
            filters: Vec::new(),
            allow_multiple: false,
        }
    }

    #[must_use]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    #[must_use]
    pub fn starting_directory(mut self, directory: impl Into<PathBuf>) -> Self {
        self.starting_directory = Some(directory.into());
        self
    }

    #[must_use]
    pub fn filter(
        mut self,
        label: impl Into<String>,
        extensions: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.filters.push(FileDialogFilter {
            label: label.into(),
            extensions: extensions.into_iter().map(Into::into).collect(),
        });
        self
    }

    #[must_use]
    pub const fn allow_multiple(mut self, allow_multiple: bool) -> Self {
        self.allow_multiple = allow_multiple;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDialogFilter {
    pub label: String,
    pub extensions: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
    pub title: String,
    pub body: String,
}

pub enum PlatformCommand {
    Window(WindowCommand),
    OpenUri(String),
    RevealPath(PathBuf),
    ShowNotification(Notification),
    FileDialog(FileDialogRequest),
    Request(crate::PlatformRequest),
}
