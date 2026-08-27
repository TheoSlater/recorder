# Native playback performance notes

## Current Windows path

The current public GPUI path is CPU-backed:

`Media Foundation SourceReader → CPU NV12 → parallel NV12→BGRA → Vec<u8> → ImageBuffer/RenderImage → GPUI DirectX atlas → canvas`

Media Foundation is explicitly asked for NV12 output, but the decoder keeps the
sample in a CPU-readable `IMFMediaBuffer`. `copy_nv12_buffer` allocates one
BGRA byte vector for each generated frame. At 3440 × 1440 that vector is about
19.8 MB. `RenderImage` owns the `Frame` and its bytes, so a conversion buffer
cannot be returned to a pool while GPUI or a scene may still reference the
image. A bounded pool would need an ownership/lifetime callback that the public
API does not provide; unsafe reuse is not justified. The current safe behavior
therefore retains one allocation per generated frame.

The playback event channel is bounded to four entries. If a GPUI update finds
more than one frame waiting, the event listener coalesces that batch to the
newest timestamped frame; obsolete frames are counted as dropped rather than
being presented rapidly to catch up. The worker also retains at most one future
Media Foundation sample while dropping obsolete sequential samples before BGRA
conversion.

Each generated `RenderImage` still has a unique GPUI image ID. The old image is
released directly from the playback window during the next render pass, avoiding
`App::drop_image`'s all-window traversal on every frame. This bounds atlas
ownership, but it does not make the DirectX atlas resource reusable: GPUI's
public Windows API has no operation to upload new bytes into an existing image
ID. A unique frame therefore still incurs one atlas insertion/upload and the
corresponding texture allocation/free cycle.

## Scheduling and measurement

## Stable release measurements

The release scheduler probe used the same 3440 × 1440 row workload as the
playback conversion path. It was repeated after the persistent worker change:

| Scheduler | Frame time | Throughput |
| --- | ---: | ---: |
| New OS threads per frame | 6.56 ms | 152.4 FPS |
| Persistent Rayon pool | 5.51 ms | 181.5 FPS |

Across three uncontended release invocations, the per-frame-thread path ranged
from 6.37–6.81 ms and the persistent pool ranged from 5.32–5.87 ms. The
persistent pool reduced the measured probe time by about 16% (about 19%
higher probe throughput). This is a scheduler improvement, not an end-to-end
playback rate; the release player still has to allocate BGRA data, construct a
`RenderImage`, submit the image to GPUI, and paint the composition.

The longer optimized playback run was measured against the 3440 × 1440
recording, with a second run at 3456 × 1408. The stable observed ranges were:

| Stage or result | Release result |
| --- | ---: |
| Media Foundation decoded | about 60 FPS |
| Visible video submissions | about 45–53 FPS at 3440 × 1440; about 43–54 FPS at 3456 × 1408 |
| NV12 → BGRA conversion | about 8–13 ms/frame |
| BGRA allocation/zeroing | about 1.5–3.5 ms/frame |
| `paint_image`/atlas submission boundary | about 4–9 ms/frame |
| Non-video composition paint | about 0–0.03 ms/frame |
| Queue/encoder-style frame drops | 0 in the run |
| Late presentation reports | about 46–54 per second |
| BGRA allocations and `RenderImage` values | about 60 per second each |
| BGRA output | about 19.8 MB/frame |
| Active-playback CPU | about 2.8 CPU-seconds/second, roughly 47% of six logical CPUs |

The values remained in the same band across the longer run rather than
improving after warm-up. Decode is therefore not the limiting stage. The
largest measured application-side costs are the conversion and the
CPU-backed GPUI image submission; ordinary composition paint is negligible.
The 4–9 ms value is deliberately named a submission boundary: GPUI does not
expose the D3D11 `UpdateSubresource` or swap-chain present timestamp to the
application, so this number must not be presented as an isolated GPU upload
duration.

The allocation and image stages have also been separated in the instrumentation:
`ImageBuffer::from_raw` wraps the converted `Vec<u8>` without a second pixel
copy, and `RenderImage::new` wraps that frame without copying its bytes. The
bytes cannot be safely reused while GPUI may retain the `RenderImage` or its
scene reference. Reusing the vector would require a renderer-owned release
callback or an equivalent ownership contract that this GPUI revision does not
provide.

Normal playback reads sequentially and uses Media Foundation timestamps against
one monotonic media clock. It does not seek per frame. If the worker is behind,
it reads ahead and converts only the newest sample whose timestamp is already due;
one future sample is retained for the next deadline. The four-entry event queue
and seek-generation checks provide a second bounded stale-frame guard. A seek
clears queued frame events, and GPUI rejects any frame from an older seek
generation, preserving latest-request-wins behavior.

Playback metrics are aggregated and logged once per second. They include decoded
and submitted/presented rates, dropped and late frames, p50/p95/p99/worst frame
intervals, presentation latency, decode/read, contiguous-buffer, conversion,
image construction, event delivery, GPUI update, invalidation-to-paint,
`paint_image`/atlas insertion, atlas release, cursor, seek, and queue-depth
timings. The visible FPS badge counts unique successful video `paint_image`
submissions while playing; repainting the same frame during zoom, resize, or
overlay changes does not inflate it.

During timeline scrubbing, the playhead and reconstructed cursor are updated on
the GPUI event immediately. Decoder requests are coalesced at a one-frame
(`16.667 ms`) timestamp step, with the final drag position always published.
Pending seek targets use atomics rather than a mutex, so the GPUI thread does
not wait on the decoder. The worker checks the request generation before the
Media Foundation seek, before conversion, between conversion row groups, and
before emitting a frame. A newer target therefore cancels obsolete conversion
work and prevents its `RenderImage` from reaching the atlas. Seek `Time`
events carry the same generation, preventing an older seek from snapping the
logical playhead backward.

After a keyframe seek, the decoded image may precede the requested timestamp.
The worker therefore anchors its playback clock to the clamped requested target
rather than the keyframe timestamp; resuming from a scrub does not replay the
keyframe-to-target interval.

GPUI's public Windows API has no application callback at the swapchain present.
The closest application-level boundary is the successful video
`Window::paint_image` call, which synchronously calls the DirectX atlas
`get_or_insert_with` path. In the current GPUI source this path locks the atlas,
allocates a tile when the `RenderImage` ID is new, and calls D3D11
`UpdateSubresource` with the CPU bytes. The metric is therefore named
canvas-submitted/presented in the app, not a hardware-present timestamp.

## GPUI preview limitation and future playback path

The ideal Windows architecture is:

`Media Foundation hardware decode → D3D11 NV12 texture → GPU video processor/shader conversion → native GPUI rendering`

That would remove the CPU NV12→BGRA conversion, the large CPU BGRA buffer,
`RenderImage` creation, and the per-frame CPU-byte atlas upload. The current
GPUI source exposes `Window::paint_surface` only on macOS, and its public
Windows `Window::paint_image` accepts only CPU-backed `RenderImage` bytes.
The exact pinned source is GPUI 0.2.2 at Zed revision `1475887f`:

* `crates/gpui/src/window.rs:4493` builds `RenderImageParams`, asks the
  platform atlas for a tile, and inserts a `PolychromeSprite`.
* `crates/gpui/src/platform.rs:816` exposes `PlatformWindow::draw` and
  `sprite_atlas`, but those are consumed inside GPUI; the application receives
  neither the renderer nor its D3D11 device.
* `crates/gpui/src/scene.rs:222` only has the existing primitive variants.
  `PolychromeSprite` stores an atlas tile, not an external shader resource.
* `crates/gpui_windows/src/directx_atlas.rs:74` inserts new image keys and
  `:304` uploads CPU bytes with `ID3D11DeviceContext::UpdateSubresource`.
* `crates/gpui_windows/src/directx_renderer.rs:330` renders the scene and
  `:786` binds atlas shader-resource views; `:251` recreates the private device
  and all GPU resources after device loss.

This is the stop condition for the isolated prototype. There is no safe
application-only extension point at which a continuously changing D3D11
texture can enter a normal GPUI scene. A child HWND, DirectComposition visual,
second swap chain, or `unsafe` access to private fields would lose normal GPUI
z-order/clipping/invalidation or create an unverified device/lifetime race.

The smallest upstream GPUI change is a Windows-native external-texture
primitive, kept separate from the atlas. It would need to provide:

1. a renderer-owned D3D11 device/context identity (or a supported shared
   resource import) so Media Foundation can use the same device or a verified
   shared handle;
2. an opaque, reference-counted texture/SRV handle with an explicit
   device-loss invalidation/recreation contract;
3. a `Scene` primitive and batch that carry source bounds, destination bounds,
   content mask, corner radii, opacity, and transform while preserving z-order;
4. a DirectX shader path that binds the external SRV without entering the
   atlas, plus synchronization so the decoder never recycles a texture while
   GPUI samples it.

That is a focused maintained GPUI patch, but it is not a change that can be
implemented and proven in this application repository without forking or
vendoring the pinned GPUI backend. Media Foundation's
`IMFDXGIDeviceManager`/`MF_SOURCE_READER_D3D_MANAGER` work should follow that
boundary: verify hardware-backed samples on the renderer device, then use a
small GPU conversion surface (video processor or shader) and a bounded
decoder-owned texture ring. It must not copy NV12 back to CPU.

The current playback path does not set `MF_SOURCE_READER_D3D_MANAGER`, does
not create an `IMFDXGIDeviceManager`, and does not inspect `IMFDXGIBuffer`; its
Source Reader output is therefore CPU-readable NV12. The independent export
path does use those D3D-aware Media Foundation interfaces and obtains
GPU-backed ARGB32 surfaces for native composition. No zero-copy claim is made
for the GPUI preview.

The practical alternatives are a small maintained GPUI fork/patch, an
upstream GPUI external-texture API, or a separate native video child/visual
with the known loss of normal GPUI scene composition. D3D11 shared handles and
D3D11On12 are transport mechanisms, not an application-level solution here:
they still require the GPUI renderer to import and synchronize the resource.
The recommended next step is a focused GPUI Windows patch and an independent
texture smoke test against that patch before connecting Media Foundation.

The CPU `RenderImage` path remains the known-good GPUI preview path. Native
D3D11 import for interactive playback is recorded as a future GPUI/upstream
milestone; it is not needed by, or shared with, the native export worker.

## Native editor export

Export is intentionally independent from the GPUI preview path:

`Media Foundation Source Reader (D3D11 ARGB32) → CompositionFrame evaluation → D3D11 render target → Media Foundation H.264 Sink Writer`

The export worker owns COM, Media Foundation, the D3D11 device, decoder
surfaces, composition shaders, and encoder samples. It evaluates one output
frame at each original recording timestamp and waits for each render/write
operation, so slow rendering cannot drop export frames or change the result.
The output target is finalized atomically only after `IMFSinkWriter::Finalize`
succeeds. No `RenderImage`, GPUI image atlas, or `Window::paint_image` is used.

Source video and composition pixels stay GPU-backed through decode and render.
The selected background image is the one deliberate CPU boundary: it is read
and decoded once with the `image` crate, uploaded to a D3D11 shader resource,
and reused for all frames. Cursor telemetry and project JSON are also loaded
once on the worker before frame evaluation.

The first renderer supports solid, gradient, and cover-cropped image
backgrounds; recording transforms; timestamped zoom; reconstructed cursor
shapes; cursor sizing/bounce; and rounded recording clipping. Shadow rendering
and future overlay layers are still pending. The renderer is separate from
playback and does not change the documented GPUI D3D11 preview limitation.
