# AGENTS.md

## Architecture

* Keep the codebase modular. Avoid monolithic files, components, or modules.
* Follow Rust best practices and prefer simple, readable, maintainable code.
* Keep source files under 300 lines where practical.
* A small exception up to roughly 350 lines is acceptable when splitting the file would make the code less clear.
* If existing code becomes too large or mixes unrelated responsibilities, refactor it into smaller focused modules.
* Prefer clear separation between UI, recording, input tracking, media, timeline, and project logic.
* Avoid unnecessary abstractions. Introduce them only when they simplify the codebase or reduce duplication.

## Code Quality

* Keep functions focused on a single responsibility.
* Prefer descriptive names over excessive comments.
* Add documentation where behavior, architecture, or non-obvious decisions need explanation.
* Do not add comments that simply restate the code.
* Remove dead code, unused imports, temporary debugging code, and unnecessary dependencies.
* Reuse existing utilities and patterns before introducing new ones.

## Styling
- Never hardcode colours.
- Use themes.

## Naming

- Use short, natural, descriptive names for functions, tests, variables, and modules.
- Avoid overly verbose names such as `tracker_writes_samples_after_video_start`.
- Prefer normal names such as `writes_samples`, `tracks_cursor`, `starts_recording`, or `cursor_tracking`.
- Names should describe purpose clearly without reading like a full sentence or test specification.
- If you find any overly verbose names, refactor them to be more concise.

## Changes

* Keep changes scoped to the task being implemented.
* Do not perform unrelated rewrites unless required to keep the architecture clean.
* When touching problematic or oversized code, improve its structure where reasonable without expanding the task unnecessarily.
* Preserve existing behavior unless the task explicitly requires changing it.

## Validation

Before considering a change complete:

* Run `cargo fmt`.
* Run `cargo check`.
* Run relevant tests.
* Run Clippy when practical and resolve warnings introduced by the change.


## Recorder Editor Coordinate Spaces

The recorder editor has three separate visual/coordinate spaces. Do not conflate them.

### 1. Editor Canvas / Workspace

The large editor area containing the video composition.

- The user can pan around it.
- The user can zoom the editor viewport in/out.
- Viewport zoom is an editor/navigation feature only.
- It must NEVER affect video rendering, recording transforms, timeline data, zoom regions, or exported output.
- Controls such as `125%`, Fit, and Recenter operate on this space.

Call this **viewport zoom** or **canvas zoom**.

### 2. Video Composition

The fixed-aspect-ratio rectangle displayed on the canvas, typically 16:9.

This represents the final exported video frame.

The composition contains:
- Background/wallpaper
- Screen recording
- Cursor
- Captions/text
- Other visual overlays

Anything outside the composition is editor workspace and must not appear in the export.

### 3. Screen Recording Layer

The actual captured desktop/window rendered inside the composition.

It can have its own:
- Position
- Scale
- Crop
- Corner radius
- Shadow
- Zoom/pan animation

Automatic and manual timeline zoom regions affect the **recording/content presentation inside the composition**, NOT the editor canvas.

Call this **recording zoom** or **content zoom**.

### Required Hierarchy

Editor Viewport
└── Canvas / Workspace
    └── Video Composition
        ├── Background Layer
        ├── Screen Recording Layer
        ├── Cursor Layer
        └── Overlay Layers

### Critical Rule

Never implement a generic "zoom" without first identifying which coordinate space it belongs to.

`viewport zoom != recording zoom`

Changing editor viewport zoom must only change how large the composition appears in the editor.

It must not modify:
- exported output
- composition dimensions
- recording scale
- recording crop
- recording zoom regions
- cursor coordinates
- timeline timing
- persisted recording transforms

When modifying rendering, zooming, panning, cropping, preview, or timeline behavior, preserve this separation.


## Documentation help:
https://gpui.rs/
https://longbridge.github.io/gpui-component/docs/installation

Always record new features you've added to PROGRESS.md and then add next features to do, based on context, in PROGRESS.md.

## Final notes
REMEMBER THE 300 LINES OF CODE RULE. FIND ANY CODE LONGER THAN THAT AND REFACTOR IT.

## Repository-specific architecture

This repository is a single Rust 2024 binary, not a workspace or a library. It
is a native Windows desktop screen recorder built on GPUI, gpui-component,
windows-capture, Windows Graphics Capture, and Media Foundation. The code in
the repository is the source of truth; the initial design in
`docs/superpowers/specs/2026-08-24-minimal-screen-recorder-design.md` predates
the editor and playback work.

### Module ownership

- `src/main.rs` owns the platform gate and the process allocator. On Windows it
  calls `app::run`; elsewhere it only reports that the recorder is unsupported.
- `src/app.rs` owns application bootstrap: tracing, initial monitor/window
  enumeration, GPUI and gpui-component initialization, bundled assets, system
  appearance synchronization, the main window, and shutdown hooks.
- `src/recorder.rs` is the private recorder module root and re-exports only the
  application-facing view, monitor/window enumeration, and shutdown coordinator.
- `src/recorder/model.rs` owns shared capture constants, source wrappers,
  monitor/window metadata, recorder states, worker events, and the documented
  `CaptureItem` thread-safety boundary. Do not move UI or media policy into it.
- `monitors.rs` and `windows.rs` enumerate selectable sources. Window
  enumeration filters invalid, untitled, zero-sized, and self-owned windows;
  source handles are runtime values and are never persisted.
- `ui.rs` owns `RecorderView`, the home-screen state machine, start/stop
  orchestration, worker-event application, status/error state, and coordination
  with the overlay and playback windows. `home_ui.rs`, `project_ui.rs`, and
  `components.rs` render the home screen and reusable home controls.
- `alerts.rs` owns the bounded, deduplicated, dismissible in-app alert queue.
  `hooks.rs` adapts worker channels to GPUI entity updates. `lifecycle.rs`
  keeps the active recording control reachable during window or app teardown.
- `capture.rs` owns the Windows Graphics Capture worker and its shutdown
  sequence. `encoder.rs` owns the H.264 encoder construction, bounded frame
  handoff, backpressure statistics, and encoder-thread join.
- `input/clock.rs` owns the shared Windows QPC clock. `input/tracker.rs` owns
  cursor/button sampling on its dedicated thread, and `input/telemetry.rs`
  owns buffered JSONL writes. `input/model.rs` owns telemetry record types and
  schema constants.
- `session.rs` owns recording directories, session manifests, artifact paths,
  and completion/failure metadata. `project.rs` discovers completed projects
  and resolves their settings files. `project_settings.rs` owns the editable
  JSON schema, normalization, and atomic saves.
- `cursor_settings.rs` owns cursor appearance settings and built-in cursor
  assets. `cursor.rs` loads/validates telemetry, interpolates and smooths
  cursor positions, and exposes the reconstructed cursor frame.
- `zoom.rs` owns the typed microsecond-based zoom and cursor-size region models,
  transition points, easing, normalization, and overlap precedence.
  `auto_zoom.rs` plus `auto_zoom/` is a pure telemetry-to-region pipeline; it
  extracts completed clicks, clusters them, protects existing regions, and
  returns ordinary editable `ZoomRegion` values.
- `media.rs` defines playback events and bounded event-queue helpers.
  `media/native.rs` owns the Media Foundation/COM playback worker, command
  delivery, latest-wins seeks, sequential playback clock, and worker errors.
  `media/native_decoder.rs` owns the Source Reader and NV12 frame decoding;
  `media/native_decoder/conversion.rs` owns bounded parallel NV12-to-BGRA
  conversion. `media/metrics.rs` owns aggregated playback instrumentation.
- `playback.rs` composes the playback modules and opens the editor window.
  `playback/view.rs` is the editor state/action coordinator. Rendering and
  focused interaction code belongs in the adjacent `editor_*` modules:
  `editor_shell` composes the layout, `editor_toolbar` and `playback_ui` render
  controls, `editor_preview` wires the preview, `editor_canvas*` owns canvas
  geometry/painting/controls, `editor_timeline*` owns timeline math/painting,
  `editor_cursor` and `editor_zoom` render inspector sections, and
  `preview_rate` owns preview-rate values.
- `overlay.rs` owns the always-on-top recording overlay, its timer and Stop
  action, and the Win32 display-affinity exclusion from capture.

Keep new behavior in the owner that already holds its invariant. Do not add a
second source of truth in a renderer, duplicate playback math in hit-testing,
or make a child UI module own recorder/editor state that belongs to its view.

### Application and recording flow

The normal recording path is:

1. `app::run` enumerates sources, creates the main `RecorderView`, initializes
   the theme, and registers both the app-quit and window-close shutdown paths.
2. While idle, the home view selects a monitor or window. Selection, refresh,
   and opening a project are rejected while a recording is starting, recording,
   or finishing.
3. Start creates a unique `recordings/<UTC timestamp[_suffix]>` directory,
   writes default project settings, writes a `session.json` manifest with
   `status: "recording"`, and registers a bounded stop channel with the
   `ShutdownCoordinator` before spawning the worker.
4. The worker converts the selected source to the exact Windows Graphics
   Capture item dimensions, updates the session source dimensions, creates the
   encoder handoff, starts the cursor tracker, and sends worker events back to
   the GPUI view. Capture dimensions, not a DPI-virtualized window rectangle,
   are authoritative for the encoded video and telemetry scaling.
5. A successful first frame submission marks the shared QPC zero and releases
   the cursor tracker from its start gate. Do not timestamp telemetry before
   that signal or introduce a second recording clock.
6. Stop, capture closure, a target resize, a worker/encoder failure, app quit,
   and window close all converge on the same cleanup path. Capture, telemetry,
   and encoder resources are stopped/joined, errors are combined, dropped
   frames are recorded, and the manifest is finalized as `completed` or
   `failed` exactly once. App/window teardown waits for the worker completion
   signal instead of abandoning an active encoder.

### Capture, encoding, and input invariants

- Recording capture is Windows Graphics Capture backed by D3D11. It uses
  `ColorFormat::Bgra8`, `CursorCaptureSettings::WithoutCursor`, a custom
  roughly 60 FPS update interval, and an H.264/60 FPS MP4 encoder with audio
  disabled. The update-driven capture path does not invent duplicate frames
  when Windows reports no update.
- Monitor capture uses the default dirty-region behavior. Window capture uses
  full dirty-region rendering so a surface handed to the encoder is complete.
  A frame-size change is an error/clean stop; do not pad stale rows or silently
  continue with an old encoder size.
- The recording path must remain GPU-backed. Do not call CPU frame-buffer APIs,
  copy captured pixels, or add a CPU fallback between `Frame` and
  `VideoEncoder::send_frame`.
- The encoder queue capacity is intentionally two. `FrameTask` contains a
  borrowed frame pointer plus a one-shot completion channel; the encoder reads
  it only while the capture callback waits for completion. It must never retain
  the pointer after that callback returns. A full queue drops the frame and
  increments the session's dropped-frame count.
- The `unsafe impl Send` boundaries for `CaptureItem` and `FrameTask` are
  narrow, documented contracts. Do not widen them, make the borrowed frame
  owned, or move WinRT/Direct3D objects across threads without re-checking
  their lifetime and apartment requirements.
- Cursor telemetry uses one Windows QPC clock, zeroed at the first accepted
  video frame. The video timestamp is relative to that zero; telemetry uses
  microseconds from the same zero. The tracker samples at about 120 Hz, stores
  monitor-relative/screen/normalized coordinates, visibility, cursor identity,
  and button state, then derives left/right/middle down/up events.
- Telemetry is a flat JSONL stream with a validated header, sample/event
  records, and a footer containing counts and initial button state. It flushes
  periodically and syncs on finish. Preserve the timebase, zero, coordinate
  space, and schema checks when changing the format.

### Session and project persistence

Each recording directory contains `recording.mp4`, `telemetry.jsonl`,
`session.json`, and the new `<directory-name>.recproj` project file. The
legacy `project.json` is read only as a fallback for older recordings. Paths
are relative to the process working directory; `recordings/` and `target/` are
runtime/build directories and are ignored by Git.

- `session.json` is manifest schema version 2. It records recording status,
  timestamps, timebase descriptions, cursor exclusion, monitor/source metadata,
  artifact names, dropped frames, and an optional failure reason. New source
  metadata is tagged as `monitor` or `window`; window handles never enter JSON.
- `telemetry.jsonl` is schema version 2. Its loader rejects missing/unsupported
  timebase metadata and unknown record kinds, ignores invalid normalized samples
  with a warning, and sorts valid samples/events by timestamp.
- Project settings are schema version 4 and currently contain cursor settings,
  viewport `CanvasView`, `CanvasComposition`, zoom regions, and cursor-size
  regions. Loads and saves normalize the complete value: invalid/non-finite
  values are replaced, ranges are clamped, invalid regions are removed, and
  colors are restricted to normalized hex values.
- Project saves use a temporary file followed by replacement with write-through
  semantics on Windows. Preserve this atomic-save behavior and surface save
  failures through the owning GPUI view rather than silently losing edits.
- The home list shows completed sessions with a video, newest first. Opening a
  project reloads settings from disk at open time; do not rely on the stale
  project-list snapshot for editor settings.

### Playback and editor invariants

- Native playback runs off the GPUI thread. The worker initializes COM and
  Media Foundation, reads the MP4 with an `IMFSourceReader`, requests NV12,
  converts frames to CPU-backed `RenderImage` values, and sends bounded
  `PlaybackEvent` values back to the view. The public Windows GPUI API does not
  currently provide D3D11 texture import, so do not claim or implement a
  zero-copy preview without an explicit GPUI API change.
- Normal playback is sequential and timestamp-driven. The decoder may retain
  one future sample and drops obsolete due samples before conversion; it does
  not seek for every frame. The event queue is bounded and GPUI coalesces a
  queued batch to the newest frame rather than presenting stale frames rapidly.
- Seeks are non-blocking from GPUI and latest-wins. The atomic pending-seek
  request carries a generation and target; the worker checks that generation
  before/after seek and during conversion. Queued frames, time events, decode
  work, and seek errors from an older generation must not move the playhead
  backward or reach the preview.
- A keyframe seek can decode a frame before the requested timestamp. The
  logical playhead remains at the requested time and the resume clock is
  anchored there. End-of-media play explicitly seeks to zero before replaying.
- Seek/decode failures stop playback and report an error while keeping the
  worker alive for a retry. Do not turn a transient media error into a worker
  exit unless the worker itself is shutting down.
- Old video images are released from the active playback window during its
  render pass. Avoid an all-window image-drop traversal for every frame and do
  not retain unbounded `RenderImage` values.
- The editor shell is composed from a top toolbar, preview, inspector,
  transport row, and native-rendered timeline. Cursor controls, canvas controls,
  zoom regions, cursor-size keyframes, and chosen background images update
  `PlaybackView` and auto-save the project settings. Timeline playhead and
  viewport navigation are editor-only state; timeline region edits auto-save.
  The export action is intentionally disabled until an export representation
  exists.
- The native cursor is excluded during capture. Playback reconstructs it only
  when the manifest confirms `cursor_capture: "excluded"`, preventing a double
  cursor; missing or incompatible telemetry disables the overlay with a
  diagnostic. Valid cursor positions interpolate in normalized coordinates,
  and the smoothing setting averages a bounded window around the playhead.
- New recordings may run auto-zoom generation once after duration metadata is
  ready. Reopening a saved project does not regenerate or replace its existing
  regions; generated candidates protect existing regions and remain ordinary
  editable timeline regions.
- Timeline playhead, duration, region bounds, and transition points are
  microseconds. Timeline viewport scroll/scale is interaction state; persisted
  zoom/cursor-size regions are project state. Keep duration normalization and
  selection indices valid whenever media metadata or regions change.
- Picker-triggered background image reads happen in a GPUI background task, and
  a monotonically increasing load id discards stale results. Missing saved
  images warn without breaking video playback.

### Coordinate-space and rendering ownership

The existing coordinate-space rules above are mandatory. In the current code,
the concrete ownership is:

- `CanvasView` is the editor camera: it zooms/pans the fitted canvas and is
  independent of media time and recording zoom. It may be persisted for editor
  restoration, but it must never change export composition values or timeline
  regions.
- `CanvasComposition` describes the output canvas and recording layer: aspect
  preset, normalized position, scale, padding, corner radius, shadow, and
  background. Its values are normalized before persistence.
- `ZoomRegion` transforms the recording layer inside the composition around
  the cursor or canvas center. `CursorSizeRegion` changes reconstructed cursor
  scale over time. Neither changes the editor camera.
- `editor_canvas_geometry` is the shared geometry source for preview painting
  and hit-testing. Keep the export-canvas clip, recording transform, cursor
  transform, resize handle, and selection behavior consistent between both.
- The custom GPUI canvas paints the stage, canvas background, recording, and
  reconstructed cursor in that order; the canvas outline is painted last so a
  scaled recording cannot hide the composition boundary.

### GPUI, themes, and error delivery

Use GPUI/gpui-component entities and event subscriptions for controls. Blocking
channel receives, Media Foundation calls, capture shutdown, encoder joins, and
large file reads must not run on the GPUI event/render path. Background tasks
wait for channels and call `cx.update`/`view.update` to mutate entities.

`Theme::sync_system_appearance` is called at startup and when the main window's
appearance changes. UI colors, borders, states, and focus styling come from
`cx.theme()`; keep the existing no-hardcoded-colour rule. Recorder and playback
errors are logged through tracing, reflected in status/diagnostic state, and
shown as bounded dismissible in-app alerts. Keep full error text in alerts even
when a compact toolbar label is truncated.

### Verification and repository workflow

Prefer the existing commands in `justfile`:

- `just fmt` / `cargo fmt` formats Rust.
- `just check` / `cargo check` type-checks the Windows binary.
- `just test` / `cargo test` runs unit tests.
- `just lint` runs Clippy for all targets/features with warnings denied.
- `just verify` runs formatting, linting, and tests.
- `just run` and `just release` launch debug or release builds.

Tests are mostly inline unit tests plus the focused external modules
`auto_zoom_tests.rs` and `project_settings_tests.rs`. Coverage includes session
paths/timestamps, project loading/normalization, telemetry records, queue
backpressure, shutdown coordination, cursor interpolation/smoothing, zoom
normalization/easing, timeline math, playback time formatting, and NV12 layout.
The interactive cursor-tracker test is intentionally ignored unless run on an
interactive Windows desktop. Run relevant tests after behavioral changes and
use the real Windows runtime for capture, overlay exclusion, Media Foundation,
and multi-monitor/DPI behavior.

Record new user-facing features and the next context-based work in
`docs/PROGRESS.md`. Keep `docs/native-playback-performance.md` current when
changing frame conversion, queueing, image lifetime, seeking, or measurement.

### Current size debt

The 300-line rule remains the target, but the current worktree still has large
legacy/coordinator files, notably `src/recorder/playback/view.rs`,
`src/recorder/playback/editor_timeline.rs`, `src/recorder/media/native.rs`,
`src/recorder/media/metrics.rs`, `src/recorder/zoom.rs`, `src/recorder/ui.rs`,
`src/recorder/cursor.rs`, `src/recorder/input/tracker.rs`,
`src/recorder/capture.rs`, `src/recorder/project.rs`, `src/recorder/session.rs`,
`src/recorder/home_ui.rs`, and `src/recorder/playback/editor_timeline_canvas.rs`.
Treat these as refactoring candidates when their ownership is touched; split by
responsibility rather than making unrelated broad rewrites for line count
alone.
