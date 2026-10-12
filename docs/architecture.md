# Architecture

VUI separates application behavior, platform integration, and rendering.

```text
host event
    -> retained widget runtime
    -> event / update / layout / paint / semantics
    -> paint list + damage
    -> renderer backend
    -> host render target
```

## Dependency rule

`vui-core` must not depend on Ash, Vulkan, Vello, wgpu, winit, Wayland, Smithay, an async
runtime, or any operating-system UI API. Platform and rendering crates depend
inward on the core.

`vui-resources` is an optional outward dependency on `vui-core`. It owns file
I/O and encoded image formats, returning core `RasterImage`, `TreeNode`, and
task-compatible values. This keeps decoders and filesystem policy out of the
widget/runtime crate.

## Ownership

The core owns widget state, focus, layout, invalidation, semantic information,
and paint intent. A host owns its event loop, surfaces, clipboard, timers,
accessibility bridge, and render targets. A renderer translates paint intent
into pixels.

`vui-ash` is the native desktop backend. Its safe scene stage tessellates the
complete `PaintList`, including shaped glyph outlines, paths, images, and nested
clips. Ash pipelines rasterize that geometry on the GPU; stencil operations
implement clip nesting and a per-frame texture atlas supplies raster images. A
small audited unsafe boundary owns the Vulkan instance, device, pipelines,
buffers, swapchain, synchronization, and presentation. The backend selects up
to 4x MSAA, retains unchanged tessellation and image-atlas uploads, uses two
frames in flight, and embeds build-time-compiled SPIR-V. Set
`VUI_VULKAN_VALIDATION=1` to require the Khronos validation layer during local
diagnostics. `VUI_VULKAN_SAMPLES=1`, `2`, or `4` requests an MSAA count; a
valid unsupported count falls back to the highest supported lower count, while
the unset default remains the highest supported count up to 4x. Device telemetry
reports both requested and selected counts. Compatible adapters are selected deterministically (discrete,
integrated, virtual, then CPU, with name and enumeration index tie-breakers).
Set `VUI_VULKAN_DEVICE` to an enumeration index, exact device name, or unique
case-insensitive name fragment to override that choice. Both renderers expose
their selected `VulkanDeviceInfo`. The native host has no CPU paint-list
fallback: scene-construction failures are reported instead of silently changing
renderers. `vui-winit` depends on this backend and has no Vello or wgpu
dependency. Vertex and index data live in device-local buffers; per-frame
host-visible staging buffers feed revision-gated copies with explicit
transfer-to-vertex-input barriers. `VulkanUploadStats` reports the geometry and
atlas bytes actually transferred by the latest frame. The image atlas retains
stable slots across scenes, uploads only newly allocated rectangles, reuses
freed space, and evicts the least-recently-used inactive images under pressure.
It repacks only when growth is required. The default 64 MiB ceiling can be
changed with `VUI_VULKAN_ATLAS_MB` (clamped to 1–256 MiB); scene statistics
expose occupancy, dirty bytes, evictions, and rebuilds.

The Winit host suspends rendering while a native surface is zero-sized and
recreates the presenter once after Vulkan reports `ERROR_DEVICE_LOST`, retrying
the same retained scene on the replacement device. Swapchain out-of-date and
suboptimal results continue through the resize path. For deterministic recovery
testing, set `VUI_VULKAN_FAULT` to `acquire-device-lost-once`,
`submit-device-lost-once`, or `present-device-lost-once`.

The same Ash pipelines render into an offscreen RGBA target for native-backend
visual regression tests. The checked-in component and typography galleries
cover light, dark, high-contrast, and fractional-scale output. Scene statistics
report geometry, index, draw-command, and atlas sizes; the windowed presenter
reports GPU duration and actual upload bytes. The reproducible renderer
benchmark and deterministic CI budgets are documented in
[`performance.md`](performance.md).

`vui-vello` remains an optional compatibility backend for compositor-owned wgpu
textures. It is not used by the native desktop host or visual-regression suite.

Raw Vulkan embedders use `vui-ash::EmbeddedRenderer`. Its unsafe constructor
borrows the host's instance, physical device, logical device, graphics queue,
and queue-family identity without assuming ownership. VUI owns all child
pipelines, buffers, attachments, and its persistent RGBA target. `render`
returns the target image and view in `SHADER_READ_ONLY_OPTIMAL` after GPU
completion, without readback; the borrow prevents target recreation while the
host inspects those handles. The host externally synchronizes queue access and
must drop the renderer before destroying its Vulkan context.

Layout uses finite-or-unbounded box constraints. Simple `Row` and `Column`
containers remain convenient defaults; `Flex` adds proportional allocation,
wrapping, and alignment for application shells, `Stack` supplies layered
composition, and `SplitPane` retains divider state while its child trees
reconcile. `Responsive` keeps compact and wide branches retained and activates
one from the current width constraint. Layout widgets route events and deferred
commands through the same widget boundaries as the rest of the retained tree.

## Styling

Themes are value-type design-token collections rather than renderer resources.
Typography and density enter the layout context, while colors, borders, radii,
and elevation enter paint. `ThemeScope` creates matching nested layout and paint
contexts so subtree overrides cannot produce geometry from one theme and pixels
from another. Component styles resolve a typed interaction state before emitting
renderer-neutral paint operations. Surface and border decorators remain ordinary
widgets, so they participate in layout, event forwarding, reconciliation, and
composition without special host support.

## Icons and shell components

Vector icon geometry enters the paint list as renderer-neutral path commands;
each renderer converts them into its native path representation. This keeps
icon data, custom paths, and shell widgets out of renderer-specific crates. Decorative
icons omit semantics, while labeled icons and icon buttons publish image or
button nodes.

Raster images follow the same split. Core stores validated shared RGBA8 pixels
and emits an image plus logical destination rectangle. `ImageView` resolves
contain, cover, or fill geometry and clipping. Renderers consume that shared
pixel buffer without exposing their image resources to core. Encoded PNG/JPEG
bytes never enter the paint or renderer interfaces.

Desktop-shell components are retained compositions or widgets in `vui-core`.
Toolbars, navigation regions, status regions, tabs, dialogs, tooltips, and the
command palette use existing focus, overlay, and reconciliation paths. Tabs
preserve selection and IDs across rebuilds, expose overflow, and support arrow
navigation. Dialogs and the command palette are modal overlay content, so focus
trapping and restoration remain centralized in `OverlayHost` and `UiRuntime`.

Application actions are represented by stable `CommandId` values. A
`CommandRegistry` owns callbacks and dynamic descriptors, while bound widgets
only enqueue an ID. `CommandHost` is the single execution boundary for widget
activation, palette selection, and normalized Ctrl/Cmd key chords. Scope
precedence runs from global through window and panel to focused context, and
conflicting default chords remain inspectable instead of being silently lost.
Deferred IDs are reinjected as command events by the runtime, allowing an outer
`App` to drain messages and reconcile model-derived command state in the same
interaction.

Text shaping lives in `vui-text`. It converts Parley layouts into
renderer-neutral glyph runs containing shared font data, normalized variation
coordinates, glyph identifiers, and positions. `vui-ash` and `vui-vello`
consume those runs without shaping text again.

Accessibility semantics remain platform neutral in `vui-core`.
`vui-accesskit` converts each frame's semantic tree to an AccessKit tree update;
window-system and compositor hosts decide how to publish those updates. The
Winit host installs `accesskit_winit` before showing its window, publishes
active tree updates, and routes supported focus/click actions back into neutral
core events.

Text editing follows the same boundary: `vui-core::TextField` owns committed
text, composition preedit, and UTF-8 cursor movement, while hosts translate
platform IME events into neutral `TextInput` and `ImePreedit` events. Clipboard
integration remains a host service rather than a core dependency: widgets emit
neutral read/write requests, and the Winit host fulfills them through the
platform clipboard.

Single-line and multiline editors share the same editing state machine.
`vui-text` supplies two-dimensional caret geometry from Parley; widgets use it
for pointer hit testing, vertical navigation, selection painting, and viewport
scrolling without depending on Parley directly.

Multiline layout supplies a width constraint to Parley, so soft line breaks and
their caret geometry remain shaping-aware. Word commands use Unicode word
boundaries in the platform-neutral editor rather than ASCII heuristics.

Editor history stores bounded snapshots in the core, coalescing uninterrupted
typing while keeping deletion, cuts, pastes, and newline insertion as explicit
transactions. Hosts provide click counts and normalized scroll deltas;
selection, paging, scrollbar geometry, and viewport state remain widget-owned.

IME preedit events preserve the platform cursor/selection range. The editor
uses that metadata for caret placement and composition decoration while keeping
uncommitted text out of its semantic value and undo history.

Focused text semantics also carry a logical caret rectangle. Native hosts use
it for IME candidate placement, while embedded hosts may translate it into
their own surface coordinate system. Selection auto-scroll and scrollbar-track
paging remain core behaviors and therefore work identically in every host.

Generic scrolling follows the same rule. `ScrollView` lays out its child with
an unbounded constraint only on enabled axes, composes the child at a translated
origin, and emits a renderer-neutral clip around the viewport. Wheel events are
offered to descendants first; a viewport marks them handled only when its
offset changes, allowing a nested scroller at its boundary to bubble naturally.

Virtualized collections do not create an intermediate widget for every model
item. `ListView` converts its scroll offset, viewport height, row height, and
overscan into a visible index range, then invokes a lazy item factory only for
that range. Materialized rows reconcile by stable `WidgetKey`; scroll position,
active index, range anchor, and selected keys survive view reconstruction.
Painting and semantics are clipped to materialized rows, while keyboard
navigation can move to any logical index and request a fresh layout around it.

`TreeView` derives a flattened descriptor sequence from expanded `TreeNode`
branches and feeds that sequence into `ListView`; only descriptors, not row
widgets, exist outside the viewport. Its explicit leaf/collapsed/expanded/status
row state drives disclosure rendering and Left/Right navigation. Expansion is
retained internally by default or can be controlled by the application model,
which lets focused command callbacks update the same state used by pointer and
keyboard disclosure actions. Unloaded, loading, and error children appear as
disabled status rows until the application supplies loaded nodes.

`TableView` uses `ListView` for vertical virtualization and selection while a
separate frozen header owns column interaction. Visible row factories read a
shared column layout snapshot and lazily create only their visible cells.
Horizontal offset, column widths, order, visibility, and sort metadata remain
table state; resizing and reordering request a fresh row layout without
materializing offscreen data. Applications provide stable row keys and receive
selection, activation, sort, and column-order callbacks, allowing focused
commands and model-controlled data ordering without coupling the core to a
specific data source.

Positional input is hit-tested from the last-painted child toward the back,
maintaining one hover path rather than broadcasting events across siblings.
Widgets request pointer capture through `EventCtx`; `UiRuntime` then routes
move, leave, and release events directly to that widget ID through composite
boundaries. Release clears capture even if a widget disappears or declines the
event, preventing a stale drag from taking over later input.

Drag-and-drop is a runtime interaction rather than a widget-specific pointer
gesture. A source starts a typed `DragPayload` with allowed copy, move, and link
operations; targets accept one operation during `DragMoved`. The runtime owns
enter/leave transitions, targeted drop delivery, completion or cancellation,
Escape and window-leave cancellation, plus the renderer-neutral preview.
Collections layer stable-key insertion feedback and edge auto-scroll on that
contract, while keyboard reorder gestures invoke the same model callback.

Animation uses the same event path. `EventCtx::request_animation_frame` asks for
the earliest possible frame, while `request_animation_after` records a precise
deadline for delayed work. `UiRuntime` owns monotonic elapsed time, emits an
`AnimationFrame` containing elapsed and delta durations, and retains only the
earliest pending deadline. Winit maps that deadline to `ControlFlow::WaitUntil`;
without a deadline it returns to `Wait`, so an idle UI does not poll or render
continuously. Headless tests advance the clock explicitly.

Interpolation remains renderer-neutral. `Tween<T>` works for every
`Interpolate` value and retargets from its current presentation value, making
rapid reversals continuous. `Spring` integrates a damped scalar with a bounded
time step. Both accept the frame's reduced-motion flag and snap to their target
without requiring separate widget logic.

Background tasks cross the platform boundary as owned requests and typed
updates. `EventCtx::spawn_task` records the initiating widget ID and a `Send`
job. The host runs that job away from the UI thread; `TaskContext` exposes only
cooperative cancellation and a thread-safe progress reporter. Completion,
failure, progress, and cancellation return through targeted widget events.
They never grant worker code access to retained widget or application state.

The runtime retains cancellation tokens for active task IDs. Before painting it
probes each owner through normal targeted routing; disappearing owners cancel
their jobs, and updates for inactive IDs are discarded. `Resource<T>` is the
small state machine used by widgets to translate task events into
idle/loading/ready/error/cancelled presentation. Tests use the same task
requests but execute them synchronously, making progress and stale-result cases
deterministic.

`OverlayHost` is a composition boundary above ordinary layout. It lays out the
base normally, measures the popup against the host viewport, resolves anchored
placement with flip-and-clamp behavior, then paints and hit-tests the popup
last. Modal overlays suppress base semantics and input while visible; Escape
and configured outside clicks dismiss them without introducing a platform API
into the core.

Tree-changing UI work is deferred. Widget actions enqueue `UiCommand` values
in `EventCtx`; the runtime applies them only after event routing and requests a
fresh layout. Overlay commands carry owned widget trees plus an optional live
anchor ID. The host resolves that ID from current semantics after layout and
after scroll events, so placement follows composed geometry rather than a stale
rectangle. The runtime keeps an overlay focus stack, focuses the first newly
introduced control, constrains modal Tab traversal through visible semantics,
and restores the previous valid focus target on dismissal.

Menus are retained overlay widgets rather than host-native menus. Menu entries
own stable semantic IDs and roving focus state; the runtime sees menu-item,
checkbox-item, and radio-item roles as focusable, while AccessKit receives the
corresponding native roles and actions. Context menus use an absolute logical
pointer rectangle as their overlay anchor, preserving the same placement and
input rules as widget-anchored menus.

Application updates are message-driven. `App<Model, Message>` owns the model,
collects typed messages from `AppSender`, runs the reducer after event dispatch,
and rebuilds its view once per batch. Before replacing the tree, widgets
reconcile transient retained state into their successors. Unkeyed children
match by position; `Keyed` children match by logical key across insertion and
reordering. Model values remain authoritative, while widget IDs, focus/press
state, text selection and history, and scroll offsets survive compatible
rebuilds. This keeps the core synchronous and deterministic without requiring
a global store or async runtime.

The convenience winit runner owns a window because the caller explicitly asks
it to. Embedded hosts drive `UiRuntime` directly.

## Windows and platform services

Multi-window applications are coordinated by renderer-neutral commands. A
stable `WindowId` identifies a `WindowDescriptor` and its one `UiRuntime`. No
focus, overlay, animation, task, theme, accessibility, or renderer state is
shared implicitly between windows. Application model state may be shared
explicitly outside those runtimes.

Widgets enqueue `WindowCommand` and typed `PlatformRequest` values through
`EventCtx`; hosts drain them after event delivery. Window commands cover
creation, close, focus, minimize, maximize, fullscreen, and attention. The
separate `vui-platform` crate defines the replaceable `PlatformServices` trait,
its callback channel, and a deterministic mock. Platform requests cover file
dialogs, URI opening, file reveal, actionable notifications, menu bars, tray
icons and menus, outbound drag, single-instance forwarding, and power
inhibition. Responses return as `PlatformEvent` values to the originating
runtime. A modal descriptor automatically adopts that runtime's window as its
parent. Native move, resize, focus, scale-factor, close-request, appearance,
power lifecycle, and file-drop events flow back through the same event path.

`vui-winit::DesktopPlatformServices` implements the portable subset: dialogs,
URI and file reveal, notifications, tray menus, system preferences, and a
loopback single-instance channel. Native menu-bar attachment, outbound drag,
and power inhibition are intentionally backend capabilities because their
window-handle and toolkit requirements differ by platform. Applications can
install another implementation with `run_with_platform_services` without
changing widget code. Both the Winit host and deterministic
`MultiWindowHarness` preserve the renderer-neutral request protocol.

Applications that want the complete policy layer use
`vui_winit::DesktopApplication`. Its startup state machine waits for
single-instance status before creating the primary window, then installs the
configured tray/menu resources and delivers launch files and URLs through
`PlatformEvent`. A hidden close-to-tray window retains its `UiRuntime`, so
focus, widget state, tasks, and the shared application model survive tray
reopening. Explicit quit removes tray resources and forces pending session
state to disk. `DesktopLifecycleHarness` is the deterministic counterpart for
testing these transitions and notification actions.

## Linux backend

`vui-platform-linux` is a portal-first `PlatformServices` implementation. A
startup capability set records the session bus, Settings, OpenURI,
Notification, Inhibit, PowerProfileMonitor, tray, and display-server state.
Portal failures fall through to `DesktopPlatformServices`, preserving native
dialogs and tray behavior on desktops with partial portal installations.

The Linux service owns the application's well-known D-Bus name. A second
process invokes its activation interface with the original argument vector;
the lifecycle layer then translates files and URLs and reopens a hidden primary
window. Settings, power-saver, screen-lock, and session signals are queued as
typed `PlatformEvent` values rather than mutating runtimes from D-Bus threads.
Power inhibition retains and explicitly closes the portal request.

`LinuxDesktopEntry` generates validated freedesktop entries containing MIME
and `x-scheme-handler` associations and installs them with temporary-file
replacement. It deliberately emits `DBusActivatable=false`: desktop launch
uses `Exec ... %U`, after which the library's D-Bus single-instance channel
forwards activation. Full `org.freedesktop.Application` activation can be
added later without changing the widget or lifecycle API.

`DesktopApp<Model, Message>` is the optional application coordinator above
the per-window boundary. Each `DesktopWindow` retains its own widget tree but
reads from one shared model and reducer. Reducing a message increments a model
revision and emits `RefreshAll`, which causes every runtime to reconcile its
view. This preserves local interaction state while making model changes from
one window immediately visible in the others.

Keyed singleton descriptors let hosts focus an existing settings, inspector,
or palette window. `WindowCloseRequested` and `ApplicationExitRequested` are
delivered before destruction and cancel the operation when handled.
`vui-session` consumes lifecycle geometry, monitor, and state changes into a
versioned JSON document. It performs debounced temporary-file replacement and
clamps restored geometry to the available monitor set. Persistence remains a
separate crate so embedded or compositor hosts can choose another policy.

## Runtime passes

The first implementation combines some passes, but their contracts remain
separate:

1. Event routing updates interaction state and emits actions.
2. Update resolves application-driven widget changes.
3. Style resolves semantic theme tokens.
4. Layout assigns logical geometry under box constraints.
5. Compose establishes transforms, clips, and stacking.
6. Paint emits renderer-neutral operations and damage.
7. Semantics emits the accessibility tree.

## Visual regression testing

`vui-snapshot` connects the deterministic `TestHarness` frame to
`vui-ash::GpuSceneBuilder`, renders it through the same Ash pipelines used by
the native host into a headless RGBA8 Vulkan target, and encodes the result as
PNG. Snapshot inputs explicitly fix logical size, scale factor, theme,
background, reduced-motion behavior, and Vulkan sample selection. The core harness retains its asset-free
bootstrap text system for component snapshots. Typography snapshots instead use
`ParleyTextSystem::with_embedded_fonts`, which constructs a memory-only Fontique
collection with system discovery disabled. Checked-in OFL Noto Sans fixtures
provide a stable Latin/Arabic fallback stack for ligatures, combining marks,
RTL and mixed bidi layout, wrapping, selection, and IME composition at multiple
scale factors.

Comparisons first apply a per-channel threshold, then enforce both changed-pixel
count and ratio limits. Failed comparisons preserve expected, actual, and
difference images. Baseline updates require the explicit
`VUI_UPDATE_SNAPSHOTS` environment variable; missing baselines fail verification.
CI requires a GPU adapter and uses Mesa's software Vulkan implementation so the
gallery cannot silently skip.

## Non-goals for 0.1

- CSS compatibility
- an HTML-style document model
- a markup language or proc-macro DSL
- a mandatory async runtime
- compositor-specific APIs in the reusable crates
- a broad widget catalog before text, input, and accessibility are correct
