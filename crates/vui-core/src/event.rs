use crate::{
    DragOperation, DragOperations, DragPayload, DragPreview, OverlayPlacement, Point, TaskContext,
    TaskId, TaskPayload, TaskRequest, Widget, drag::DragRequest, task::TaskCommand,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WidgetId(u64);

impl WidgetId {
    #[must_use]
    pub fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }

    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SemanticAction {
    Click,
    Focus,
    Increment,
    Decrement,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerButton {
    Primary,
    Secondary,
    Middle,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    bits: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modifier {
    Shift,
    Control,
    Alt,
    Meta,
}

impl Modifiers {
    const SHIFT: u8 = 1;
    const CONTROL: u8 = 1 << 1;
    const ALT: u8 = 1 << 2;
    const META: u8 = 1 << 3;

    pub const fn set(&mut self, modifier: Modifier, pressed: bool) {
        let bit = match modifier {
            Modifier::Shift => Self::SHIFT,
            Modifier::Control => Self::CONTROL,
            Modifier::Alt => Self::ALT,
            Modifier::Meta => Self::META,
        };
        if pressed {
            self.bits |= bit;
        } else {
            self.bits &= !bit;
        }
    }

    #[must_use]
    pub const fn shift(self) -> bool {
        self.bits & Self::SHIFT != 0
    }

    #[must_use]
    pub const fn command(self) -> bool {
        self.bits & (Self::CONTROL | Self::META) != 0
    }

    #[must_use]
    pub const fn alt(self) -> bool {
        self.bits & Self::ALT != 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipboardRequest {
    Read,
    Write(String),
    ReadImage,
    WriteImage(crate::RasterImage),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Enter,
    Space,
    Tab,
    Escape,
    Backspace,
    Delete,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    Home,
    End,
    PageUp,
    PageDown,
    Character(String),
    Other,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    AnimationFrame(crate::AnimationFrame),
    TaskProgress {
        id: TaskId,
        fraction: Option<f64>,
        payload: TaskPayload,
    },
    TaskCompleted {
        id: TaskId,
        payload: TaskPayload,
    },
    TaskFailed {
        id: TaskId,
        error: String,
    },
    TaskCancelled {
        id: TaskId,
    },
    #[doc(hidden)]
    TaskOwnerProbe,
    PointerMoved(Point),
    PointerDown {
        position: Point,
        button: PointerButton,
        click_count: u8,
    },
    PointerUp {
        position: Point,
        button: PointerButton,
    },
    PointerLeft,
    DragEntered {
        position: Point,
        payload: DragPayload,
    },
    DragMoved {
        position: Point,
        payload: DragPayload,
    },
    DragLeft {
        payload: DragPayload,
    },
    Drop {
        position: Point,
        payload: DragPayload,
        operation: DragOperation,
    },
    DragCancelled {
        payload: DragPayload,
    },
    DragCompleted {
        payload: DragPayload,
        operation: DragOperation,
    },
    Scroll {
        position: Point,
        delta_x: f64,
        delta_y: f64,
    },
    KeyDown(Key),
    KeyUp(Key),
    ModifiersChanged(Modifiers),
    TextInput(String),
    ImePreedit {
        text: String,
        cursor: Option<(usize, usize)>,
    },
    AccessibilityAction {
        target: WidgetId,
        action: SemanticAction,
    },
    CommandInvoked(crate::CommandId),
    FocusChanged(Option<WidgetId>),
    WindowFocused(bool),
    WindowMoved(Point),
    WindowResized(crate::Size),
    WindowScaleFactorChanged(f64),
    MonitorChanged(crate::MonitorInfo),
    WindowCloseRequested,
    ClipboardImage(crate::RasterImage),
    AppStateChanged,
    ApplicationExitRequested,
    Platform(crate::PlatformEvent),
    FileDialogCompleted {
        id: crate::PlatformRequestId,
        paths: Vec<PathBuf>,
    },
    FileDialogCancelled {
        id: crate::PlatformRequestId,
    },
}

pub enum UiCommand {
    ShowOverlay {
        anchor: Option<OverlayAnchor>,
        placement: OverlayPlacement,
        overlay: Box<dyn Widget>,
        modal: bool,
    },
    DismissOverlay,
    InvokeCommand(crate::CommandId),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OverlayAnchor {
    Widget(WidgetId),
    Rect(crate::Rect),
}

#[derive(Default)]
pub struct EventCtx {
    current_window: Option<crate::WindowId>,
    handled: bool,
    request_flags: u8,
    focus_request: Option<WidgetId>,
    clipboard_request: Option<ClipboardRequest>,
    pointer_capture_request: Option<PointerCaptureRequest>,
    drag_request: Option<DragRequest>,
    animation_delay: Option<Duration>,
    task_commands: Vec<TaskCommand>,
    commands: Vec<UiCommand>,
    platform_commands: Vec<crate::PlatformCommand>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PointerCaptureRequest {
    Capture(WidgetId),
    Release,
}

impl EventCtx {
    const REQUEST_LAYOUT: u8 = 1;
    const REQUEST_PAINT: u8 = 1 << 1;
    const RESTORE_FOCUS: u8 = 1 << 2;

    pub(crate) const fn for_window(current_window: Option<crate::WindowId>) -> Self {
        Self {
            current_window,
            handled: false,
            request_flags: 0,
            focus_request: None,
            clipboard_request: None,
            pointer_capture_request: None,
            drag_request: None,
            animation_delay: None,
            task_commands: Vec::new(),
            commands: Vec::new(),
            platform_commands: Vec::new(),
        }
    }

    #[must_use]
    pub const fn current_window(&self) -> Option<crate::WindowId> {
        self.current_window
    }

    pub fn set_handled(&mut self) {
        self.handled = true;
    }

    pub fn request_layout(&mut self) {
        self.request_flags |= Self::REQUEST_LAYOUT | Self::REQUEST_PAINT;
    }

    pub fn request_paint(&mut self) {
        self.request_flags |= Self::REQUEST_PAINT;
    }

    pub fn request_focus(&mut self, id: WidgetId) {
        self.focus_request = Some(id);
    }

    pub fn request_clipboard(&mut self, request: ClipboardRequest) {
        self.clipboard_request = Some(request);
    }

    pub fn capture_pointer(&mut self, id: WidgetId) {
        self.pointer_capture_request = Some(PointerCaptureRequest::Capture(id));
    }

    pub fn release_pointer(&mut self) {
        self.pointer_capture_request = Some(PointerCaptureRequest::Release);
    }

    pub fn start_drag(
        &mut self,
        source: WidgetId,
        payload: DragPayload,
        allowed: DragOperations,
        preview: DragPreview,
        position: Point,
    ) {
        self.drag_request = Some(DragRequest::Start {
            source,
            payload,
            allowed,
            preview,
            position,
        });
        self.handled = true;
        self.request_paint();
    }

    pub fn accept_drag(&mut self, target: WidgetId, operation: DragOperation) {
        self.drag_request = Some(DragRequest::Accept { target, operation });
        self.handled = true;
        self.request_paint();
    }

    pub fn cancel_drag(&mut self) {
        self.drag_request = Some(DragRequest::Cancel);
        self.handled = true;
        self.request_paint();
    }

    pub fn request_animation_frame(&mut self) {
        self.request_animation_after(Duration::ZERO);
    }

    pub fn request_animation_after(&mut self, delay: Duration) {
        self.animation_delay = Some(
            self.animation_delay
                .map_or(delay, |current| current.min(delay)),
        );
    }

    pub fn spawn_task<T, F>(&mut self, owner: WidgetId, job: F) -> TaskId
    where
        T: std::any::Any + Send + Sync,
        F: FnOnce(TaskContext) -> Result<T, String> + Send + 'static,
    {
        let request = TaskRequest::new(owner, job);
        let id = request.id();
        self.task_commands.push(TaskCommand::Spawn(request));
        id
    }

    pub fn cancel_task(&mut self, id: TaskId) {
        self.task_commands.push(TaskCommand::Cancel(id));
    }

    pub fn show_overlay(
        &mut self,
        anchor: WidgetId,
        placement: OverlayPlacement,
        overlay: impl Widget + 'static,
    ) {
        self.commands.push(UiCommand::ShowOverlay {
            anchor: Some(OverlayAnchor::Widget(anchor)),
            placement,
            overlay: Box::new(overlay),
            modal: false,
        });
    }

    pub fn show_context_menu(&mut self, position: Point, menu: impl Widget + 'static) {
        self.commands.push(UiCommand::ShowOverlay {
            anchor: Some(OverlayAnchor::Rect(crate::Rect::new(
                position.x, position.y, 0.0, 0.0,
            ))),
            placement: OverlayPlacement::Below,
            overlay: Box::new(menu),
            modal: false,
        });
    }

    pub fn show_modal(&mut self, overlay: impl Widget + 'static) {
        self.commands.push(UiCommand::ShowOverlay {
            anchor: None,
            placement: OverlayPlacement::Center,
            overlay: Box::new(overlay),
            modal: true,
        });
    }

    pub fn dismiss_overlay(&mut self) {
        self.commands.push(UiCommand::DismissOverlay);
    }

    pub fn invoke_command(&mut self, id: impl Into<crate::CommandId>) {
        self.commands.push(UiCommand::InvokeCommand(id.into()));
    }

    pub fn open_window(
        &mut self,
        mut descriptor: crate::WindowDescriptor,
        root: impl Widget + 'static,
    ) -> crate::WindowId {
        if descriptor.is_modal() && descriptor.parent.is_none() {
            descriptor.parent = self.current_window;
        }
        let id = crate::WindowId::next();
        self.platform_commands
            .push(crate::PlatformCommand::Window(crate::WindowCommand::Open {
                id,
                descriptor,
                root: Box::new(root),
            }));
        id
    }

    pub fn window_command(&mut self, command: crate::WindowCommand) {
        self.platform_commands
            .push(crate::PlatformCommand::Window(command));
    }

    pub fn close_window(&mut self, id: crate::WindowId) {
        self.window_command(crate::WindowCommand::Close(id));
    }

    pub fn close_current_window(&mut self) {
        if let Some(id) = self.current_window {
            self.close_window(id);
        }
    }

    pub fn quit_application(&mut self) {
        self.window_command(crate::WindowCommand::QuitApplication);
    }

    pub fn open_uri(&mut self, uri: impl Into<String>) {
        self.platform_commands
            .push(crate::PlatformCommand::OpenUri(uri.into()));
    }

    pub fn reveal_path(&mut self, path: impl Into<PathBuf>) {
        self.platform_commands
            .push(crate::PlatformCommand::RevealPath(path.into()));
    }

    pub fn show_notification(&mut self, title: impl Into<String>, body: impl Into<String>) {
        self.platform_commands
            .push(crate::PlatformCommand::ShowNotification(
                crate::Notification {
                    title: title.into(),
                    body: body.into(),
                },
            ));
    }

    pub fn request_platform(&mut self, request: crate::PlatformRequest) {
        self.platform_commands
            .push(crate::PlatformCommand::Request(request));
    }

    pub fn set_menu_bar(&mut self, menu: crate::NativeMenu) {
        if let Some(window) = self.current_window {
            self.request_platform(crate::PlatformRequest::SetMenuBar { window, menu });
        }
    }

    pub fn set_tray(&mut self, tray: crate::TrayDescriptor) {
        self.request_platform(crate::PlatformRequest::SetTray(tray));
    }

    pub fn remove_tray(&mut self, id: impl Into<String>) {
        self.request_platform(crate::PlatformRequest::RemoveTray(id.into()));
    }

    pub fn notify(&mut self, notification: crate::DesktopNotification) {
        self.request_platform(crate::PlatformRequest::Notification(notification));
    }

    pub fn start_external_drag(
        &mut self,
        data: crate::OutboundDragData,
        allowed: crate::DragOperations,
    ) {
        if let Some(window) = self.current_window {
            self.request_platform(crate::PlatformRequest::StartDrag {
                window,
                data,
                allowed,
            });
        }
    }

    pub fn register_single_instance(
        &mut self,
        application_id: impl Into<String>,
        arguments: impl IntoIterator<Item = impl Into<String>>,
    ) {
        self.request_platform(crate::PlatformRequest::RegisterSingleInstance {
            application_id: application_id.into(),
            arguments: arguments.into_iter().map(Into::into).collect(),
        });
    }

    pub fn set_power_inhibition(&mut self, inhibited: bool) {
        self.request_platform(crate::PlatformRequest::SetPowerInhibition(inhibited));
    }

    pub fn request_file_dialog(
        &mut self,
        request: crate::FileDialogRequest,
    ) -> crate::PlatformRequestId {
        let id = request.id;
        self.platform_commands
            .push(crate::PlatformCommand::FileDialog(request));
        id
    }

    #[must_use]
    pub const fn is_handled(&self) -> bool {
        self.handled
    }

    pub(crate) const fn layout_requested(&self) -> bool {
        self.request_flags & Self::REQUEST_LAYOUT != 0
    }

    pub(crate) const fn paint_requested(&self) -> bool {
        self.request_flags & Self::REQUEST_PAINT != 0
    }

    pub(crate) const fn focus_request(&self) -> Option<WidgetId> {
        self.focus_request
    }

    pub(crate) fn take_clipboard_request(&mut self) -> Option<ClipboardRequest> {
        self.clipboard_request.take()
    }

    pub(crate) const fn pointer_capture_request(&self) -> Option<PointerCaptureRequest> {
        self.pointer_capture_request
    }

    pub(crate) fn take_commands(&mut self) -> Vec<UiCommand> {
        std::mem::take(&mut self.commands)
    }

    pub(crate) fn take_platform_commands(&mut self) -> Vec<crate::PlatformCommand> {
        std::mem::take(&mut self.platform_commands)
    }

    pub(crate) fn take_drag_request(&mut self) -> Option<DragRequest> {
        self.drag_request.take()
    }

    pub(crate) fn take_animation_delay(&mut self) -> Option<Duration> {
        self.animation_delay.take()
    }

    pub(crate) fn take_task_commands(&mut self) -> Vec<TaskCommand> {
        std::mem::take(&mut self.task_commands)
    }

    pub(crate) fn request_focus_restore(&mut self) {
        self.request_flags |= Self::RESTORE_FOCUS;
    }

    pub(crate) const fn focus_restore_requested(&self) -> bool {
        self.request_flags & Self::RESTORE_FOCUS != 0
    }
}
