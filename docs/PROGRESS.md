# Progress

Native Windows screen recorder built with GPUI, gpui-component, and
`windows-capture`. Design spec: `docs/superpowers/specs/2026-08-24-minimal-screen-recorder-design.md`.

## Completed

### Capture and encoding

- Windows Graphics Capture backed by D3D11 (BGRA8), GPU surface passed straight
  to Media Foundation with no CPU frame copies or fallback path.
- Hardware-accelerated H.264 encoding at 60 FPS into an MP4 container, audio
  disabled.
- Dedicated encoder pump thread with a bounded frame queue; overflow drops
  frames under backpressure and the drop count is reported in the session
  manifest.
- Native monitor resolution capture; monitors enumerated with name and current
  dimensions, multi-monitor selection.
- Shared monitor/window capture source selection; visible top-level windows are
  enumerated with title, process name when available, and current dimensions.
  Window targets use the same GPU-backed capture worker and encoder.
- Capture targets use the exact Windows Graphics Capture item dimensions, keeping
  mixed-DPI windows aligned with the encoder and cursor telemetry. A target resize
  stops the recording cleanly before the backend's fixed-size surface path can
  introduce stale bands.
- Window capture requests full dirty-region rendering so each GPU surface handed
  to the encoder contains a complete frame instead of stale compositor rows.
- Native cursor excluded from capture (`WithoutCursor`); the cursor is
  reconstructed during playback from telemetry instead.
- Worker lifecycle: dedicated capture thread, stop-request channel,
  unexpected-stop detection for both capture and encoder threads, panic
  isolation around worker and encoder work, and errors surfaced to the UI.
- Graceful shutdown: quitting the app or closing the recorder window stops an
  active recording and waits for finalization before exiting.

### Sessions and projects

- Each recording gets its own folder under `recordings/<UTC-timestamp>` with
  unique-suffix collision handling.
- `session.json` manifest per recording: schema version, status
  (`recording` / `completed` / `failed`), created/finished timestamps,
  timebase descriptions, monitor name and dimensions, file names, dropped
  frame count, failure reason, and tagged monitor/window source metadata. Window
  metadata stores title, process name when available, and captured dimensions;
  raw window handles are runtime-only.
- Project save file per recording: `<folder>.recproj` (JSON) is created when a
  recording starts and holds every editor-changeable setting (schema version,
  cursor style/size/smoothing/visibility, canvas zoom and pan, zoom regions).
  Writes are atomic (temp file plus replace) and normalized on save.
- Legacy recordings keep their old `project.json`; the loader prefers
  `<folder>.recproj` and falls back so existing projects do not lose settings.
- Home screen lists saved projects (completed recordings only), newest first,
  with a count, refresh action, and one-click open into the playback window.

### Input and cursor telemetry

- Shared QPC recording clock zeroed at the first accepted video frame, so video
  and telemetry share one timebase.
- Cursor tracker thread sampling at ~120 Hz, gated until video starts:
  timestamped samples with monitor-relative, screen, and normalized
  coordinates, cursor identity, visibility flag, and mouse button states.
- Derived mouse button down/up events (left, right, middle).
- JSONL telemetry file with validated header, flat sample/event records, and a
  footer carrying record counts and initial button state; buffered writing with
  periodic flush and final sync.

### Recorder home UI

- Themed GPUI/gpui-component interface following system light/dark appearance.
- Live status badge (Ready / Starting / Recording / Finishing) plus a detailed
  status line that reports errors and the output location.
- Monitor/window source selector showing the selected resolution; selection
  locked while a recording is active. Window lists can be refreshed before
  recording.
- Start/Stop controls with correct enabled/disabled states per recorder state.

### Recording overlay

- Always-on-top draggable overlay on the recorded monitor with an elapsed-time
  clock and a Stop button.
- Overlay is excluded from the capture itself via `WDA_EXCLUDEFROMCAPTURE`;
  if exclusion fails the overlay closes instead of appearing in the video.

### Playback

- Dedicated playback window that opens automatically after a successful
  recording, and reopens any saved project later.
- Native Windows Media Foundation source-reader playback runs on a worker
  thread and sends decoded frames, timing, and transport state to GPUI.
- Decoded frames are published as GPUI `RenderImage` values and painted by a
  custom GPUI canvas element. The preview stays in the GPUI render tree, so
  inspectors, toolbars, overlays, and other GPUI UI can render above it.
- Canvas-style preview stage: zoom in/out (0.25x–4x), middle-mouse panning, and
  fit/reset view controls on a responsive 16:9 layout.
- Transport controls: play/pause, jump-to-start/end, and elapsed/duration
  readout formatted to centiseconds.
- Reconstructed cursor overlay driven by telemetry: positions interpolate
  between samples at the playhead, hide when the cursor was invisible or out of
  bounds, and stay disabled unless the manifest confirms the native cursor was
  excluded (preventing a double cursor). Problems surface as a diagnostic line
  in the toolbar.
- Playback window now provides the initial editor shell: compact project
  toolbar, dominant responsive preview, right-side inspector structure, compact
  transport row, and a custom-rendered native Video/Cursor/Zoom timeline.
- Editor auto-save: every change made in the playback editor persists to the
  project's `.recproj` immediately - cursor controls save on each adjustment,
  and native GPUI canvas zoom/pan is saved as it changes; the view restores the
  saved zoom/pan when a project reopens. Save failures surface in the toolbar
  and as dismissible in-app alerts showing the full message.
- Playback errors surface as dismissible in-app alerts; the compact toolbar
  diagnostic may truncate, but the alert retains the complete error message.
  The playback worker survives transient seek or decode failures -
  playback stops with a clear message instead of killing the worker, so replay
  can be retried immediately. Recorder, overlay, project-opening, and
  playback-worker failures are sent through the same tracing-backed error path.
- In-app alerts now use the themed gpui-component `Alert` directly in each GPUI
  view, so they remain visible without OS notification delivery or a separate
  notification layer. Errors and warnings are bounded, deduplicated, logged,
  dismissible, and rendered above the editor/home content.
- Alerts use a compact extra-small layout with an opaque themed popover surface,
  restrained theme border, no redundant title row, and a narrower stack so
  diagnostics stay visible without dominating the editor or home screen.
- Empty auto-zoom generation reports the analyzed duration, telemetry state,
  qualifying click count, candidate count, and whether existing zoom regions
  protected every candidate; it explicitly explains missing clicks, unavailable
  telemetry, recordings too short for a usable region, and overlap protection.
- Replay at end-of-media is explicit: the UI invalidates the old seek generation,
  seeks to zero, and starts playback; the worker also guards the end-of-media
  play command so a delayed state event cannot turn replay into a no-op.
- Reopening a project always reads its settings from disk at open time, so
  edits saved by a previous editor session show up without refreshing the
  project list or restarting the app.
- The playback window title shows the project's name (its recording folder).
- Existing viewport navigation is owned by the GPUI canvas: wheel zoom,
  middle-button pan, fit/reset, aspect-ratio preservation, and cursor overlay
  transforms stay in the same render tree as the editor.
- Canvas composition editing now supports recording selection, pointer movement,
  proportional resize, normalized canvas padding, corner radius, shadow, and
  five aspect-ratio presets. Shared geometry drives both rendering and
  hit-testing, and changes persist through `.recproj`.
- Canvas backgrounds now support themed solid colours, two-stop gradients, and
  native-picked images. Picker file reads run in the GPUI background task,
  stale picker results are ignored, and unavailable saved images surface a
  warning without breaking playback.
- Reconstructed cursor overlays remain clipped to the export canvas; the video
  rectangle itself stays square while the surrounding canvas remains rounded.
- The Screen Recording Layer is a distinct object inside the Video Composition:
  time-based zoom transforms the whole layer and its cursor, then clips the
  layer to the export canvas boundary. The cursor may overflow the recording
  rectangle into the composition padding, but never leaves the composition.
  Canvas hit-testing also ignores the cropped area outside that boundary.
- Mouse smoothing in the inspector Cursor section: the slider (0–100%) persists
  with project settings and applies a centered, weighted nine-tap temporal
  filter across a window that grows from 40ms to 300ms with the slider. The
  center-weighted filter damps hand jitter and rounding direction changes while
  staying responsive and in sync with the video.
- Cursor button-down events now drive a deterministic 240ms scale bounce on the
  reconstructed cursor, with a damped overshoot that stays inside the composition
  clip and follows the playhead during playback and scrubbing.
- Preview reserves a definite canvas-relative video frame before metadata arrives,
  so recordings remain visible while their aspect ratio is being resolved.
- Preview displays a small top-right FPS badge measured from frame updates reaching
  the native GPUI canvas, resetting to zero while paused.
- Native GPUI timeline editing now owns the session playhead and duration in
  microseconds. Its custom element draws the ruler, bounded track lanes, video
  extent, and playhead; click and drag use the existing latest-request-wins seek
  path, while cursor reconstruction follows the same playhead timestamp.
- Timeline seeks retain the requested playhead while Media Foundation resolves
  a keyframe seek, so a pre-target decoded frame cannot make a scrub visibly
  jump backward; normal playback releases that hold when its media clock catches
  the requested timestamp.
- Transport controls now jump to recording start/end with terminal-bar arrow
  glyphs and use the same native seek path as timeline scrubbing.
- Manual zoom regions now share a typed timestamp-based model with the Zoom
  track: regions can be added, selected, moved, resized, deleted, targeted at
  the cursor or canvas center, eased live in the native preview with a quintic
  smootherstep envelope, and persisted through `.recproj`.
- Auto-zoom generation normalizes telemetry, derives explicit clicks from button
  transitions, rejects cursor-only motion and drags, infers double-click and
  context-click strength, clusters clicks across a 2.5-second gap, and selects
  the strongest click as the focus. It adds 500ms padding, uses depth-2 (1.5×)
  `ZoomRegion` values with explicit 1.522575s/1.01505s entry and exit windows,
  keeps separate click clusters as separate editable regions, trims overlapping
  transition windows, and bridges short gaps without crossing manual regions.
  Newly completed recordings run this generation once after the editor receives
  the video's duration; saved projects keep their existing regions unchanged
  when reopened. A generated region clipped by the recording end keeps its final
  zoomed frame because no exit interval can be displayed. The toolbar action
  remains available for intentional manual regeneration of unprotected areas.
- The Zoom track is now compact and manipulation-focused: selected regions show
  distinct zoom-in, hold, and zoom-out sections; outer edges resize total
  duration; transition handles adjust easing duration; body drags move the
  region; Delete/Backspace removes the selection; and small grab handles use
  contextual cursors, hover disclosure, and subtle snapping to timeline points.
- Cursor size keyframes now have a dedicated Cursor track: keyframe regions show
  eased keyframe diamonds, can be added at the playhead, moved or extended by
  dragging, adjusted from the inspector, deleted with the keyboard or button,
  and persist through the project settings.
- Timeline viewport navigation is session-only: horizontal trackpad deltas scroll
  the visible time range and vertical wheel deltas zoom around the pointer. The
  preview canvas keeps its independent wheel zoom and middle-button pan behavior.
- Timeline rendering now has an explicit fixed label gutter and clipped,
  viewport-relative ruler/tracks/zoom/playhead content. Ruler intervals and
  labels adapt to scale without overlap, and pointer-anchored timeline zoom
  preserves the timestamp under the pointer while scrolling remains bounded.
- Playback performance audit: Media Foundation NV12 conversion now runs in parallel
  across bounded worker rows, keeping decoding and conversion off the GPUI thread.
  A replaced `RenderImage` is released from the playback window during its render
  pass, avoiding an all-window atlas walk for every frame while keeping atlas
  ownership bounded.
- Fixed the editor’s green/stale bottom band by locating the NV12 chroma plane after
  the decoder’s aligned luma height rather than after only the visible frame rows.
  Added layout and chroma-offset regression tests for padded NV12 buffers.
- Added compact 24 FPS, 30 FPS, and 60 FPS preview controls to the editor header.
  Preview-only frame presentation is capped without changing the 60 FPS recording,
  media timing, or timeline data; 60 FPS is the default.
- Playback can be paused, resumed, or replayed from the end with the Spacebar
  anywhere in the editor shell; the shortcut is surfaced in the play button
  tooltip and avoids double-activating the focused button.
- End-to-end native playback instrumentation now measures decoded versus actually
  submitted frames, stage and frame-time percentiles, queue depth, late/stale/queue
  drops, seek latency, atlas release, cursor work, and the GPUI invalidation-to-
  `paint_image` path. The FPS badge counts unique successful video submissions
  while playing rather than decoder throughput.
- Normal playback now remains sequential and timestamp-driven while dropping
  obsolete samples before BGRA conversion. A single future sample is retained;
  seek requests are generation-tagged, queued frame events are invalidated on seek,
  and older-generation frames are rejected by GPUI.
- Playback control delivery is non-blocking on the GPUI thread; rapid seek requests
  collapse to the newest target while normal playback remains sequential.
- Timeline scrubbing now keeps logical playhead/cursor updates independent from
  decoder delivery, publishes at most one seek per 60 FPS timestamp step, and
  flushes the final drag target through the same seek path.
- Pending seek publication is atomic and generation-checked. Obsolete Media
  Foundation seeks, conversion rows, decoded frames, and seek errors are discarded
  before preview upload; stale `Time` events cannot move the playhead backward.
- Seek resume timing is anchored to the requested media timestamp instead of the
  pre-target keyframe, so resuming after a scrub does not replay obsolete frames.
- Scrub diagnostics aggregate pointer moves, published requests, pending replacements,
  stale skips/cancellations, pointer-to-request latency, and the existing
  decode/event/update/upload timing percentiles without logging every pointer event
  at info level.
- GPUI event delivery coalesces a queued batch to the newest timestamped frame, so
  a slow render pass does not rapidly present obsolete frames to catch up.
- A 3440 × 1440 probe improved from roughly 3–6 FPS in the scalar path to roughly
  75 FPS in an optimized build; that is isolated conversion throughput, not the
  user-visible playback rate. The end-to-end submitted-FPS metrics are required
  before claiming the current roughly 20 FPS in-app baseline is resolved. GPUI
  still performs one CPU-byte upload per frame because its public Windows API does
  not accept a shared D3D11 video surface.
- Completed three uncontended optimized release scheduler probes: per-frame
  thread creation averaged 6.56 ms/frame (152.4 FPS), while the persistent
  Rayon pool averaged 5.51 ms/frame (181.5 FPS), about 16% lower probe time.
  This is a modest isolated scheduler gain, not an end-to-end playback rate.
- Completed the longer release playback run at 3440 × 1440 and a 3456 × 1408
  run. Decode stayed near 60 FPS, visible submissions stayed around 45–53 FPS
  (43–54 FPS on the second size), conversion was about 8–13 ms/frame, BGRA
  allocation about 1.5–3.5 ms/frame, and the GPUI `paint_image` submission
  boundary about 4–9 ms/frame. No frame drops were observed; the full stable
  breakdown and its measurement boundaries are in
  `docs/native-playback-performance.md`.
- Traced the exact pinned GPUI 0.2.2 / Zed `1475887f` Windows backend for a
  native-texture prototype. It has no public external-texture scene primitive,
  renderer device access, or shared-surface lifetime API. The prototype is
  intentionally stopped at that boundary; native D3D11 import is now a focused
  upstream GPUI patch milestone, with the CPU `RenderImage` path retained as
  fallback.

### Codebase health

- Modular architecture (~25 focused modules): capture, encoder, input tracking,
  telemetry, sessions/projects, playback, overlay, and UI separated.
- Unit tests cover session paths/timestamps, telemetry records, queue
  backpressure, shutdown coordination, cursor interpolation, playback time
  formatting, and project loading.
- Windows-only entry point with a clear message on other platforms.

### Recorder home UI redesign

- Rebuilt the launcher as a compact native-tool layout: slim header with inline
  status dot, segmented Monitor/Window control, single primary record action
  that flips between Start and Stop, and a one-line status row showing the
  output path; the large grey status card is gone.
- Monitor sources render as selectable cards (name plus resolution, accent fill
  and check when selected); window mode keeps a compact dropdown with refresh.
- Saved recordings are dense clickable rows ("Recording N", relative age,
  `source · width × height`) inside an internally scrolling list so recording
  controls stay visible without whole-window scrolling.
- Project rows read the human-readable `source` field already stored in each
  session manifest (`project.rs`), falling back to legacy monitor names, and
  derive relative ages from `created_at_utc` via a new epoch parser.
- Status errors render in the danger color while alerts keep surfacing
  details (`RecorderView::status_error`, `set_status`).

## Next

- Resolve the in-flight `CaptureItem`/`Send` break around
  `start_free_threaded` in the capture worker (concurrent capture work, not UI).
- Show a live elapsed-time readout next to the record action while recording.
- Add hover tooltips to the record and refresh actions.
- Populate the remaining inspector sections with editor controls as those
  systems mature (each new control should persist through `.recproj`).
- Add an export workflow after the editor state has a stable representation.
- Verify and tune cursor edge behavior across multi-monitor recordings.
- Add fixture-backed decoder tests for frame orientation, duration, and seeking.
- Add fixture-backed playback performance coverage for the bounded conversion path,
  frame drops, and preview frame rate.
- Extend the release playback matrix with 1080p60 and 1440p60 fixtures, and
  capture RAM/GPU counters when a repeatable Windows measurement harness is
  available; the 3440 × 1440 and 3456 × 1408 stable runs are recorded.
- Re-run the aggressive-scrubbing matrix with the new `scrub_moves`, `scrub_seeks`,
  `seek_replaced`, `seek_skipped`, and `seek_cancelled` metrics, confirming no GPUI
  freeze and stable memory during 30–60 seconds of drag input.
- Prepare a small maintained GPUI upstream patch for a Windows external-texture
  scene primitive, device identity/import, device-loss handling, and bounded
  decoder texture ownership; do not attempt application-side interop until that
  API exists.
- Verify window capture across DPI changes, minimized windows, and the clean
  resize-stop behavior on representative Windows configurations.
- Verify zoom-region manipulation at minimum timeline widths, including
  overlapping regions, short transitions, snapping, keyboard focus, and
  reopening saved transition points.
- Tune the center-weighted cursor tracking window and quintic zoom envelope
  against fixture-backed recordings at normal and fast cursor movement.
- Tune cursor click-bounce amplitude and duration against rapid repeated clicks
  and intervals where the cursor is hidden.
- Verify cursor-size keyframe easing, track diamonds, edge dragging, keyboard
  deletion, overlapping regions, and reopening saved ranges and peak sizes.
- Verify the timeline viewport matrix at minimum, medium, and maximum zoom,
  including repeated pointer-anchored zoom, middle/end scrolling, and playhead
  alignment at the gutter boundary.
- Tune click-cluster thresholds against fixture-backed real recordings while
  preserving automatic generation and manual-region protection.
- Add explicit reset and clear actions for canvas composition settings and
  background images.
- Verify compact alert layout at narrow home and playback window sizes.
- Show an autosave indicator ("All changes saved") in the editor toolbar.
- Surface additional user-actionable diagnostics only where they have a clear
  recovery action, keeping decoder telemetry in the log instead of flooding
  the alert stack.
- Consider persisting the preview-rate preference once its project-versus-user scope
  is defined.
- Add focused keyboard transport tests when the GPUI playback-window test harness
  covers key event bubbling and capture.

## Later

- Let users rename projects (updates `<folder>.recproj` naming and home list
  labels) when the project workflow expands beyond the current dedicated
  playback window.
