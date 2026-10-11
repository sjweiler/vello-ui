//! Renderer- and platform-neutral UI primitives.

mod animation;
mod app;
mod collection;
mod command;
mod desktop;
mod drag;
mod event;
mod geometry;
mod icon;
mod layout;
mod paint;
mod platform;
mod raster;
mod runtime;
mod semantics;
mod shell;
mod style;
mod task;
mod text;
mod theme;
mod widget;
mod widgets;
mod window;

pub use animation::{AnimationFrame, Easing, Interpolate, Spring, Tween};
pub use app::{App, AppSender};
pub use collection::{
    CollectionDrag, CollectionDrop, DropPosition, ListItem, ListView, SortDirection,
    TableAlignment, TableColumn, TableView, TreeChildren, TreeNode, TreeView,
};
pub use command::{
    Command, CommandButton, CommandDescriptor, CommandHost, CommandIconButton, CommandId,
    CommandRegistry, CommandScope, KeyChord, ShortcutConflict,
};
pub use desktop::{DesktopApp, DesktopSender, DesktopWindow};
pub use drag::{DragOperation, DragOperations, DragPayload, DragPreview};
pub use event::{
    ClipboardRequest, Event, EventCtx, Key, Modifier, Modifiers, OverlayAnchor, PointerButton,
    SemanticAction, UiCommand, WidgetId,
};
pub use geometry::{BoxConstraints, Insets, Point, Rect, Size};
pub use icon::{Icon, IconButton, IconData, IconName};
pub use layout::{
    Axis, ConstrainedBox, CrossAxisAlignment, Flex, FlexChild, FlexWrap, MainAxisAlignment,
    Responsive, SizedBox, Spacer, SplitPane, Stack,
};
pub use paint::{Color, PaintList, PaintOp, PathCommand};
pub use platform::{
    DesktopNotification, DesktopSessionEvent, NativeMenu, NativeMenuItem, NotificationAction,
    OutboundDragData, PlatformEvent, PlatformRequest, PowerEvent, SystemAppearance,
    SystemPreferences, TrayDescriptor,
};
pub use raster::{ImageFit, ImageView, RasterImage};
pub use runtime::{UiFrame, UiRuntime};
pub use semantics::{SemanticNode, SemanticRole};
pub use shell::{CommandPalette, Dialog, NavigationItem, Sidebar, StatusBar, Tab, TabBar, Toolbar};
pub use style::{Border, Surface, SurfaceTone, ThemeScope};
pub use task::{
    CancellationToken, Resource, ResourceState, TaskContext, TaskId, TaskPayload, TaskRequest,
    TaskUpdate,
};
pub use text::{
    BOOTSTRAP_ADVANCE_EM, CaretPosition, ShapedGlyph, ShapedGlyphRun, TextLayout, TextSystem,
};
pub use theme::{
    BorderTokens, ButtonStyle, ButtonVariant, ColorTokens, ComponentStyles, ControlState,
    ControlStateStyle, DensityTokens, ElevationTokens, RadiusTokens, SpacingTokens, Theme,
    ThemeMode, TypographyTokens,
};
pub use widget::{LayoutCtx, PaintCtx, SemanticsCtx, Widget, WidgetKey};
pub use widgets::{
    Button, Checkbox, Column, ContextMenu, Keyed, Label, Menu, MenuButton, MenuItem, OverlayHost,
    OverlayPlacement, Padding, ProgressBar, Row, ScrollAxes, ScrollView, TextArea, TextField,
};
pub use window::{
    FileDialogFilter, FileDialogKind, FileDialogRequest, MonitorInfo, Notification,
    PlatformCommand, PlatformRequestId, WindowCommand, WindowDescriptor, WindowId,
};
