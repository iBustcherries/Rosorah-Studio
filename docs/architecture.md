# Engine boundaries

## Current data flow

Synthetic/image/desktop-capture sources → shared Vulkan source textures → per-canvas scene
layers → reusable RGBA render targets → staging/readback → native preview or
FFmpeg software H.264 encoders (synthetic sources only).

One pattern frame is generated and uploaded per source per clock tick. Every
canvas references that same texture. Each canvas necessarily has its own output
render and encoding pass when its pixels/layout differ. “Shared input” does not
mean different layouts can use a single encoded video bitstream.

The core crate owns serializable projects, globally unique numeric IDs, normalized
layer geometry, sources, scenes, canvases, six-track PCM routing, chat messages,
capability-aware connector contracts, event matching, cooldowns and actions.

The GPU crate owns Vulkan setup, source textures, sampling, straight-alpha
compositing in linear light using sRGB source/target formats, scratch render
targets, and row-aligned readback. Textures are reused when dimensions match.
The scratch target cache is bounded to eight sizes, avoiding unbounded growth
while resizing windows. Frame upload and readback dimensions are validated.

The app crate owns the native eframe UI, image decoding with limits, project
loading/saving, resource measurements, the initial FFmpeg subprocess encoder,
CLI checks, preview-only alert display and a background recording worker.

The preview is demand-driven when paused; synthetic animation is capped at
about 15 updates per second. This is a prototype tradeoff, not a measured
production efficiency result. The output FPS setting controls recorded frames.

## Custom capture engine in 0.4

`rosorah-capture` owns the backend contract, per-source worker lifecycle, typed
Choosing/Connecting/Live/Stopping/Stopped/Failed states, bounded latest-frame
mailboxes, frame validation and received/replaced counts. It has no GStreamer,
Vulkan, egui or desktop portal dependency. A backend receives a Producer, explicit
capture options and cancellation receiver. Only the app's desktop adapter needs
portal/GStreamer APIs. Future game backends can implement the same contract.

Stop makes publication terminal for that session and immediately queues a clear.
Reconnect cannot overlap backend cleanup. Each new session gets independent
state so an old producer cannot inject stale frames into a new session.
Worker errors/panics become terminal states. The compositor drains once per
source, independently of how many canvases reference that texture.

## Desktop backend

Each capture source owns one async portal session and PipeWire remote file descriptor.
A dedicated worker runs an in-process GStreamer pipewiresrc → videoconvert →
videoscale → RGBA appsink pipeline. The appsink drops old frames, and a mutex
holds only the latest pending frame. UI polling uploads each consumed frame once
into the existing source texture; both canvases reference that texture.

Mapped video stride is respected and output dimensions are bounded to 4096 per
axis. Cancellation drops the playing pipeline, sets it to NULL, then closes the
portal session. Portal closure, bus errors and EOS clear the stale texture.
No permission tokens or captured pixels are stored in projects; sharing always
requires an explicit start. A closed or denied session can be started again.

## Next capture boundary

A capture adapter should publish timestamped frames and metadata to a shared
source registry. Linux portal/PipeWire adapters must honor compositor permission
flows. Automatic selection should remember allowed sessions where supported;
it must not represent denied access as success. Game detection and process-audio
matching are policies above capture adapters, not repeated captures per canvas.

GPU frames should later carry imported DMA-BUF/external texture ownership and
synchronization. CPU RGBA must remain an explicitly slower fallback. Capture,
audio, rendering and encoding must use bounded queues with defined overload
policies. Live pacing should use a monotonic clock and report dropped frames.

## Browser boundary

CEF should run in an isolated adapter/process with no OBS/obs-browser code.
Offscreen rendering must be proved independently of native Wayland windowing.
Frame transport, input events, scale factor, resizing, popup behavior and
browser/audio lifetime need separate integration tests. GPU sharing is desirable,
but should only be advertised after an actual Wayland prototype passes them.

## Platform boundary

Platform connectors normalize chat and events but retain platform message IDs,
user IDs, scopes and actual capabilities. Per-target sends need individual
outcomes rather than claiming all succeeded after one request. Unsupported
moderation actions must remain unavailable. User-initiated OAuth credentials
belong in an OS keyring, not project JSON, screenshots, or logs.

Chat and event buffers are bounded. Chat orders by receive time and deduplicates
by platform and message ID. Alert events deduplicate by platform/event ID and
have per-rule cooldowns. This avoids duplicate actions on reconnect, within the
bounded deduplication window. Persistent event replay protections come later.

## Persistence

Schema version 1 rejects unsupported versions and invalid references/dimensions.
Writes sync a sibling temporary file and rename it into place. Image files remain
external references. Legacy AuroraCast project/layout copying is implemented. Cross-process save locking, asset
embedding, portable project bundles and crash recovery remain upcoming work.


## Modular workspace (0.2)

The Studio page uses egui_dock 0.18 with egui 0.33. The six panel variants
render views over one Studio state; moving or closing views never copies or
deletes the underlying project, chat feed or recorder. A separate versioned
layout file stores the dock tree, active tabs, split fractions and floating
window geometry. Save operations use a sibling temporary file and rename.
Layout writes are throttled to once per second and skipped while dragging.
Non-finite initial widget rectangles are normalized before JSON serialization.
On load, floating-window position and size are restored explicitly because
egui's in-memory window state does not survive application restarts.

### Edge snapping (0.3.0)

The highlighted-area overlay uses a central 40% rectangle for tab grouping.
Outside it, the dominant pointer direction chooses the corresponding half of
the hovered panel. Center and window coverage match, eliminating the accidental
floating band. Double-click actions are queued until after DockArea finishes
rendering, then float or redock the existing panel. No view state is recreated.
