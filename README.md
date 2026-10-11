# VUI

VUI is an experimental, embeddable Rust UI toolkit with a renderer-neutral
core and a Vello rendering backend.

The project is deliberately independent of any desktop environment. Windowed
applications can use `vui-winit`; compositors and other hosts can drive
`vui-core` directly and provide their own render target.

## Current status

The initial vertical slice includes:

- a retained widget runtime;
- a deterministic animation clock with interruptible tweens, springs, easing, delayed frames, and reduced-motion support;
- typed background tasks with progress, cooperative cancellation, stale-result protection, and resource states;
- PNG/JPEG decoding, weak resource caching, directory/tree loaders, and accessible raster image widgets;
- logical geometry and box-constraint layout;
- pointer, keyboard, and IME event routing;
- topmost positional hit testing with runtime pointer capture for reliable drags;
- anchored popup and modal overlay composition with viewport-aware placement;
- keyboard-accessible menus, menu buttons, and pointer-position context menus;
- a typed application model/update/view loop with keyed retained-state reconciliation;
- typed design tokens, light/dark themes, subtree theme scopes, and state-aware component styles;
- renderer-neutral vector paths, accessible icons, tooltips, and desktop-shell components;
- a scoped command registry shared by shortcuts, buttons, menus, navigation, toolbars, and the command palette;
- a keyed, fixed-row-height virtualized list with large-data scrolling and desktop selection behavior;
- a virtualized tree with retained expansion, lazy child states, and desktop tree navigation;
- a virtualized data table with sortable, resizable, reorderable, and hideable columns;
- Tab focus traversal with disabled-control filtering;
- label, button, checkbox, editable text field and text area, row, column, flex, stack, split-pane, sizing, padding, and generic scroll-view widgets;
- renderer-neutral paint operations and damage reporting;
- Parley/Fontique shaping with system-font fallback and bidirectional text;
- Vello scene translation;
- live AccessKit integration with semantic updates and action routing;
- a multi-window winit/Vello desktop host with per-window runtimes;
- typed window and platform-service commands, native file dialogs, URI/file reveal, attention requests, image clipboard transfer, and external file drop;
- deterministic single- and multi-window headless testing harnesses.
- deterministic offscreen Vello snapshots with PNG baselines, tolerance-aware comparison, and failure diff artifacts.

Run the example with:

```sh
cargo run -p vui-hello
```

## Using VUI

The workspace is intentionally modular. A renderer-neutral embedder can depend
only on `vui-core`; Vello translation lives in `vui-vello`, native windows in
`vui-winit`, Linux portal integration in `vui-platform-linux`, deterministic
headless interaction tests in `vui-testing`, and GPU visual regression tooling
in `vui-snapshot`.

For a native application:

```toml
[dependencies]
vui-core = "0.1.0"
vui-winit = "0.1.0"
```

`vui-winit` enables `accessibility` and `system-fonts` by default. Embedded
hosts can disable both and opt back into either independently:

```toml
vui-winit = { version = "0.1.0", default-features = false, features = ["accessibility"] }
```

The smallest recommended application is in `examples/minimal`; the larger
`vui-hello` program exercises the component, lifecycle, session, and Linux
service layers.

Visual regression tests live in `vui-snapshot`. The checked-in gallery fixes the
viewport, scale, theme, bootstrap font metrics, animation preference, and Vello
antialiasing mode. A mismatch writes expected, actual, and amplified-difference
PNGs under `snapshot-artifacts/`.

The shaped-text gallery uses checked-in Noto Sans and Noto Sans Arabic variable
fonts through `ParleyTextSystem::with_embedded_fonts`. That constructor disables
host font discovery and installs an explicit in-memory fallback order. The OFL
license, upstream URLs, and SHA-256 hashes live beside the fixtures. Baselines
cover ligatures, combining marks, Arabic RTL, mixed bidi, wrapping, selection,
IME preedit, and 1×/1.25×/2× scale factors.

Verify the gallery with:

```sh
cargo test -p vui-snapshot --test gallery
```

Approve intentional visual changes with:

```sh
VUI_UPDATE_SNAPSHOTS=1 cargo test -p vui-snapshot --test gallery
```

Set `VUI_REQUIRE_GPU_SNAPSHOTS=1` to fail instead of skip when no compatible
Vello/wgpu device is available. CI enables this flag and uses software Vulkan.
Cross-platform CI sets `VUI_SKIP_GPU_SNAPSHOTS=1`; Linux is the canonical
baseline renderer while Windows and macOS still compile and test platform-neutral code.

Animation is runtime-driven rather than tied to the renderer. Widgets receive
deterministic `AnimationFrame` events and request the next frame or a delayed
wake-up through `EventCtx`. Scalars, colors, points, sizes, and rectangles can
be interpolated. The Winit host sleeps while idle and wakes at the earliest
requested deadline; `run_with_options` accepts `RunOptions` for reduced motion.
Button hover colors use this path as the first built-in transition.

Background work follows the same ownership model. `EventCtx::spawn_task`
creates a typed task owned by a stable `WidgetId`; workers can publish progress
through `TaskContext` and return typed results. Winit executes jobs off the UI
thread and returns updates through its event-loop proxy. `Resource<T>` converts
those events into idle/loading/ready/error/cancelled state, while the runtime
cancels work whose owner disappears. The headless harness executes pending jobs
deterministically. Tasks produce events and never mutate widgets directly.

Window management follows an equally explicit boundary. `WindowDescriptor`
and stable `WindowId` values describe independent native windows, while
`EventCtx::open_window` and `WindowCommand` request open, close, focus,
minimize, maximize, fullscreen, and attention operations. Each window owns a
separate `UiRuntime`, Vello surface, accessibility adapter, focus/overlay/task
state, animation clock, and scale factor; applications can still share model
state explicitly. Modal descriptors infer the current window as their parent,
and closing a parent closes its owned children.

`DesktopApp<Model, Message>` provides the optional shared layer above those
independent runtimes. A message emitted in any `DesktopWindow` runs through one
typed reducer, increments a shared revision, and rebuilds every open window
from the same model. Keyed singleton descriptors focus an existing tool window
instead of opening a duplicate. Close and application-exit events are
cancellable when a widget handles them.

Platform services use typed requests and events instead of backend calls in
widgets. The replaceable `PlatformServices` boundary has a deterministic mock;
the Winit implementation supplies native open/save/folder dialogs, actionable
notifications, system-tray menus, URI/file reveal, and local single-instance
argument forwarding. It also reports system appearance and suspend/resume
changes. Text and RGBA images use the clipboard, while external file hover/drop
becomes typed drag events. Menu-bar attachment, outbound native drag, and power
inhibition remain explicit host-backend capabilities, so an embedder can add
them without coupling `vui-core` to an OS toolkit. `MultiWindowHarness` applies
the same window protocol without an OS event loop and retains requests for
assertions.

On Linux, `vui-platform-linux` layers XDG Desktop Portal and D-Bus behavior
over the portable backend. It detects available portal interfaces, uses the
portal for URI/file reveal and actionable notifications, reads and monitors
appearance/accent/reduced-motion settings, reports power-saver and lock state,
holds portal power-inhibition requests, and uses a well-known D-Bus name for
single-instance activation. `LinuxDesktopEntry` renders and atomically installs
`.desktop` metadata for MIME types and URL schemes. Wayland/X11 outbound drag
is reported as unavailable until the Winit host can provide the required
compositor/source-window handle.

`vui-session` stores keyed window geometry and open/maximized/fullscreen state
in versioned JSON. Writes are debounced, synchronized, and atomically renamed;
unknown schema versions fall back safely. Restoration prefers the saved
monitor and clamps offscreen positions onto an available display. The native
showcase uses this layer and shares live settings with its detached inspector.

`vui_winit::DesktopApplication` composes those pieces into an application
lifecycle. It performs single-instance arbitration before showing the primary
window, installs configured tray and menu services, translates initial and
forwarded arguments into file/URL events, supports close-to-tray with tray
reactivation, follows system light/dark and reduced-motion preferences, and
flushes session state during explicit shutdown. `DesktopLifecycleHarness`
models the same startup, forwarding, visibility, notification-action, and
shutdown transitions without an operating-system event loop.

`vui-resources` keeps file I/O and image decoding outside the platform-neutral
core. It provides PNG/JPEG-to-RGBA decoding, cancellable image and directory
loaders, conversion of directory entries into lazy `TreeNode` children, and a
weak shared `ResourceCache`. `ImageView` emits a renderer-neutral image paint
operation with contain, cover, or fill fitting; Vello retains uploads by stable
image ID and releases entries no longer referenced by the current scene.

See [`docs/architecture.md`](docs/architecture.md) for the dependency rules and
planned runtime passes.

The desktop example includes Latin and Arabic text, an IME-aware editable text
field, interactive checkbox and button controls, keyboard focus, and a disabled
control. The bootstrap bitmap font is retained only as a deterministic fallback
when the host has no usable system font.

The text field supports committed text, composition preedit, Unicode-safe
backspace/delete, left/right/home/end cursor movement, Shift selection,
select-all, native copy/cut/paste, mouse-based caret placement and drag
selection, horizontal caret scrolling, placeholder text, and an accessible
value.

`TextArea` adds hard-line and soft-wrapped multiline editing, Up/Down caret navigation,
two-dimensional pointer hit testing, multiline selection painting, and vertical
caret scrolling. Both editors support Unicode word-wise movement, selection,
and deletion through Ctrl/Cmd plus the arrow, Backspace, or Delete keys.
They also provide bounded, typing-coalesced undo/redo history; double-click word
and triple-click line selection; Page Up/Page Down navigation; visible overflow
indicators; and pointer-local wheel/trackpad scrolling.
Scrollbar thumbs are draggable, Home/End operate on visual wrapped lines while
Ctrl/Cmd+Home/End target the document, and IME composition ranges are rendered
with selected-subrange emphasis.
Dragging a selection beyond an editor edge auto-scrolls the viewport, clicking
a scrollbar track pages by one viewport, and the focused caret rectangle is
reported to the platform so IME candidate windows follow the insertion point.

`ScrollView` clips and scrolls arbitrary widget trees on either or both axes.
It supports wheel and touchpad deltas, draggable scrollbar thumbs, track paging,
and nested propagation: an inner viewport consumes motion only while it can
move, then leaves the remaining interaction available to its parent viewport.

`ListView` virtualizes large fixed-height collections by building only visible
rows plus configurable overscan. Rows retain identity through `WidgetKey`;
selection supports pointer activation, Ctrl/Cmd toggling, Shift ranges, and
Arrow/Home/End/Page navigation. Keyboard movement keeps the active row visible,
and list/list-item semantics expose visible rows without constructing the full
data set. Optional row dragging reports stable source and target keys, paints an
insertion marker, and auto-scrolls near viewport edges. Alt+Up/Down exposes the
same reorder callback without a pointer. The native example exercises the
widget with 10,000 rows.

`TreeView` layers hierarchy over the same keyed virtualization engine. It
flattens expanded branches into logical rows, renders depth and disclosure
state, and retains expansion across uncontrolled view reconstruction. Passing
`.expanded(...)` makes expansion model-controlled for command-driven apps.
Left/Right navigate parent-child structure or collapse/expand branches;
Up/Down, ranges, activation, scrolling, and selection come from `ListView`.
`TreeNode` supports loaded, unloaded, loading, and error child states, while
native tree/tree-item semantics expose only materialized rows.
Tree and table rows opt into the same collection drop contract with
`.draggable(true).on_drop(...)`.

`TableView` completes the collection layer with lazy cell construction,
application-defined row keys, a frozen header, horizontal scrolling, and the
same keyboard and range selection used by `ListView`. `TableColumn` supplies a
stable ID, width limits, alignment, visibility, and sortable state. Header
clicks cycle ascending, descending, and unsorted states; divider drags resize
within constraints, and header drags reorder columns. Width, order, sort, and
scroll state reconcile across application view rebuilds. Table, row,
column-header, and cell semantics are translated through AccessKit.

`Flex` supports horizontal and vertical flow, grow/shrink/basis sizing, wrapping,
run gaps, main-axis distribution, and container- or child-level cross-axis
alignment. `SizedBox`, `ConstrainedBox`, and `Spacer` provide explicit,
minimum/maximum, aspect-ratio, and flexible sizing. `Stack` layers children with
topmost-first pointer routing. `SplitPane` provides horizontal or vertical panes
with a captured draggable divider, minimum pane sizes, and a split ratio that
survives application view reconciliation. `Responsive` selects compact or wide
widget trees from the available width. In debug builds, overflowing flex layouts
receive a red outline.

`Theme` groups color, typography, spacing, radius, border, elevation, density,
and component tokens. Buttons support primary, secondary, ghost, destructive,
and icon variants with normal, hovered, pressed, selected, focused, and disabled
presentation. `ThemeScope` overrides both layout and paint for a subtree;
`Surface` and `Border` provide composable backgrounds, outlines, padding, and
elevation. A runtime theme can also be replaced live with `UiRuntime::set_theme`.

Icons are expressed as renderer-neutral move, line, quadratic, cubic, and close
path commands; Vello translates those commands into native paths. `IconButton`
provides focus, activation, accessible labeling, and optional anchored tooltips.
The shell layer includes `Toolbar`, `Sidebar`, `NavigationItem`, `StatusBar`,
keyboard-accessible retained `TabBar`, modal `Dialog` helpers, and a searchable
`CommandPalette`. These remain ordinary widgets and use the same overlay,
reconciliation, theming, semantics, and event-routing infrastructure.
`TabBar::reorderable` uses the shared drag lifecycle and also supports
Alt+Left/Right reordering.

`Command` gives an action a stable ID, presentation metadata, dynamic enabled,
checked, and selected state, an optional icon, a default `KeyChord`, and a
global/window/panel/focused scope. `CommandHost` resolves Ctrl/Cmd-normalized
shortcuts and command IDs through one registry. `CommandButton`,
`CommandIconButton`, `NavigationItem::command`, and `MenuItem::command` bind to
the same descriptors; menus and tooltips display the registered shortcut, and
`CommandPalette` is populated directly from registry descriptors. Duplicate
shortcuts are available through `CommandRegistry::shortcut_conflicts`.

Pointer presses may capture the pointer by widget ID. Captured move and release
events are routed directly through composite widgets even after the pointer
leaves the original bounds; buttons, checkboxes, editor selections, and
scrollbar thumbs use this path.

`OverlayHost` paints a popup after the base tree and routes input to it first.
Anchored overlays support above, below, left, right, and centered placement;
they flip away from constrained edges and clamp to the host viewport. Modal
overlays add a scrim, hide background semantics, block background input, and
can dismiss on Escape or an outside click.

Widget actions can enqueue overlay commands through `EventCtx`. Commands are
applied after dispatch, so callbacks can safely create or dismiss widget trees
without re-entrant mutation. Dynamic popups anchor to a live `WidgetId`, follow
that widget through scrolling and relayout, move focus into their controls, and
restore the invoking control when dismissed. Modal semantics naturally limit
Tab traversal to the active dialog.

Menus provide action, disabled, separator, checkbox, and radio entries. They
support pointer activation, Up/Down and Home/End navigation, Enter/Space,
type-ahead matching, and semantic menu roles. `MenuButton` opens from a live
widget anchor, while `ContextMenu` opens from the secondary-button position and
uses the same viewport flip-and-clamp placement.

`App<Model, Message>` provides an Elm-style update cycle without an async
runtime. `AppSender` queues typed messages during callbacks; after dispatch the
model is reduced and the view rebuilt. Reconciliation keeps retained widget
IDs and transient interaction state while accepting new model-derived values.
Wrap conditionally reordered stateful controls in `Keyed` so editor focus,
selection/history, and scroll identity follow the logical control rather than
its temporary child index.
