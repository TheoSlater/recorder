# Minimal Windows Screen Recorder Design

Date: 2026-08-24  
Status: Approved for implementation

## Goal

Build the smallest working native Windows screen recorder in the existing Rust
binary. The first version must open a GPUI desktop window, allow a monitor to
be selected, record that monitor, stop recording, and write an MP4 named
`recording.mp4` in the current working directory. A later editor is explicitly
out of scope.

## Scope

Included:

- GPUI application initialized with the current `gpui-component` installation
  pattern and bundled assets.
- Monitor enumeration and selection.
- Start and stop controls.
- Windows Graphics Capture backed by D3D11.
- Native monitor width and height.
- H.264, 60 FPS output in an MP4 container.
- Hardware acceleration requested through the `windows-capture` encoder.
- Background capture, encoding, and finalization work.
- User-visible capture and encoding errors.

Excluded:

- Timeline, editing, zoom, cursor customization, webcam, microphone, effects,
  backgrounds, and trimming.
- File dialogs and output configuration.
- CPU frame-buffer processing or a CPU fallback path.

## Chosen approach

Use `windows-capture` end-to-end for the capture and encoder boundary. Its
`GraphicsCaptureApiHandler::start_free_threaded` API supplies a dedicated
capture thread, while `VideoEncoder::send_frame` accepts the captured
Direct3D surface directly. The encoder is configured for H.264, 60 FPS, the
selected monitor's current dimensions, and an MP4 container with audio
disabled.

This keeps the implementation small and avoids introducing a custom COM,
Media Foundation transform, or D3D11 copy pipeline. A custom encoder would
provide more hardware-device control but would be substantially larger. An
FFmpeg subprocess would add an external runtime and weaken the intended native
GPU path.

## UI and state

The implementation remains in `src/main.rs` rather than introducing a
workspace or module hierarchy.

At startup, the application enumerates `windows-capture::monitor::Monitor`
values and records a display label and current width/height for each one. The
view owns the monitor list, selected index, current recorder state, status
message, and optional stop sender.

The window uses `gpui-component`'s `Root` and `Button` components. It shows one
button per monitor, the selected monitor's dimensions, `Start Recording`,
`Stop Recording`, and a status line. Monitor selection is disabled while a
recording is starting, running, or stopping. Start is disabled while active;
Stop is disabled while idle.

The fixed output path is `recording.mp4` in the process working directory.
Starting another recording overwrites that file.

## Worker lifecycle

When Start is pressed, the view creates:

- a bounded stop channel from the UI to the worker; and
- an event channel from the worker to the UI.

The worker thread receives a copy of the selected `Monitor` and its dimensions,
builds capture settings, and calls `Capture::start_free_threaded`. The worker
owns the returned `CaptureControl` for the entire session. It sends `Started`,
waits for the stop message, finalizes the encoder, requests capture shutdown,
joins the capture thread, and sends `Finished` or `Failed`.

The UI never waits on the worker, capture control, or encoder. A GPUI
background task waits for worker events and applies them through the
foreground entity context. This keeps GPUI input, redraw, and window messages
responsive during both recording and finalization.

## Capture and encoding data flow

The worker uses the selected monitor as the Windows Graphics Capture item and
configures:

- `ColorFormat::Bgra8`, matching the encoder's Direct3D surface input;
- `MinimumUpdateIntervalSettings::Custom(Duration::from_nanos(16_666_667))`;
- no cursor or audio customization; and
- the selected monitor's current width and height as encoder dimensions.

The capture handler owns `Option<VideoEncoder>`. In `new`, it constructs the
encoder with `VideoSettingsBuilder::new(width, height)`, H.264, 60 FPS, the
default target bitrate, disabled audio, and the default MPEG-4 container.

Each `on_frame_arrived` callback calls `send_frame(frame)`. It does not call
`Frame::buffer`, `buffer_crop`, or any other CPU-readable frame API. The
captured GPU surface therefore stays on the Direct3D path into Media
Foundation. Hardware acceleration is explicitly enabled by the
`windows-capture` encoder. If capture, D3D11, Media Foundation, or encoder
finalization fails, the error is sent to the UI; no CPU frame-processing
fallback is attempted.

Windows Graphics Capture is update-driven. The implementation requests a
60-FPS update interval and writes a 60-FPS H.264 stream, but it does not invent
duplicate GPU frames when Windows emits no update for an unchanged desktop.
Strict constant-frame duplication is deferred because it would require a
separate GPU-surface pacing stage.

## Shutdown and error handling

The handler uses `Option<VideoEncoder>` so finalization happens at most once.
Normal stop finalizes the encoder before the worker stops and joins capture.
If the capture item closes, `on_closed` attempts the same finalization path.
Errors from startup, frame delivery, shutdown, and finalization are converted
to status events and displayed in the window.

If no monitors can be enumerated, the window still opens and shows the
enumeration error with recording disabled. The application reports that the
recorder is Windows-only outside the supported capture target rather than
silently substituting CPU capture.

## Verification

After implementation, run:

```text
cargo fmt --check
cargo check
```

The runtime acceptance check on Windows is to launch the binary, select each
available monitor, start recording, stop recording, and verify that
`recording.mp4` is created or overwritten and contains the selected monitor at
its current dimensions.
