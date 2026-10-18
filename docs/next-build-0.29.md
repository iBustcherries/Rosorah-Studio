# Rosorah Studio 0.29.0-preview.1 — canvas editing and stream reliability

This preview carries forward the approved next-build list. Existing projects load with no crop, no groups and output-enabled audio by default. Save a backup before testing a preview; an older app will not understand the new editing settings.

## Included scope and controls

| Item | Where / behavior |
| --- | --- |
| Delete widgets | Right-click the layer → Delete widget, or right-click an existing widget in Widgets. Removes its layer from the current scene; other scenes keep their shared source. Undo restores it. |
| Crop sources | Right-click a layer → Edit crop. Drag gold corner/edge handles or enter edge trims in pixels when source dimensions are available, otherwise percent. Reset crop restores the original bounds. Crop belongs to the layer, not the shared source. |
| Middle-edge resize | Select a source and drag the middle of any edge. Corners follow the Keep proportions setting; middle edges change one dimension. |
| Compact sports | New sports cards have a shorter default box; cards, boards and tickers fill the chosen widget area. Empty schedules use a readable message. Boards page through five games; provider configuration is unchanged. |
| Undo / redo | Edit menu or Ctrl+Z / Ctrl+Shift+Z. Covers crop, resize, group, placement and deletion. Live goal counters remain outside editing history. |
| Multiple selection | Shift-click sources or layer rows; drag empty canvas space to box-select. Move or resize the selection together. |
| Groups | Select layers → Arrange → Group / Ungroup. Clicking one group member selects its visible partners. A locked member prevents dragging the group. Groups are scene-local selection groups, not nested folders. |
| Alignment and spacing | Arrange aligns selected items to their collective bounds; three or more items can have equal gaps. Layer context menus also align to the canvas. Teal guides show snapping; hold Ctrl to bypass it. |
| Copy between canvases | Arrange → Copy to canvas → Share source settings or Make independent copies. Copies to that canvas’s active scene using normalized placement; review its framing afterward. Independent copies have their own source settings and media-audio channel. |
| Responsive widgets | Chat, music, goals, sports and information cards reflow when their box changes. QR codes and credits retain their source proportions. Widget rendering is bounded to 960 pixels on the longest side. |
| Canvas lock | Lock canvas under its preview prevents mouse selection/transforms there. Layer and settings controls remain available. |
| Stream recovery | Each destination has its own bounded queue/reconnect worker. Broadcast shows state and offers Retry this destination now. Backoff resets after a sustained connection. A destination outage does not stop other destinations or the local recording. |
| Monitoring and sync | Mixer → Monitoring & sync, or Edit menu. Select headphones, Output only / Monitor & output / Monitor only, and 0–2000 ms audio delay. Volume/mute continue to follow the active scene. |
| Recording recovery / MP4 | Edit → Project & recording tools. Incomplete MKVs in the recording directory are offered for recovery; the tools open on startup if these are found. Convert to MP4 copies compatible video/audio without re-encoding. Originals and existing outputs are never overwritten. |
| Performance details | Edit → Performance details. Shows render time, missed deadlines, queued-frame drops, estimated encoder backlog and destination errors. GPU busy appears when the Linux driver exposes it (including AMD); it is whole-device usage, not Rosorah-only usage. Multi-canvas totals show summed drops/deadlines and worst render time/backlog. |
| Account recovery | Existing token renewal is retained. Permanent refresh failures now mark the account disconnected and direct you to Reconnect; transient failures keep retrying. |
| Portable projects | Edit → Project & recording tools → Export / Import. ZIP includes project settings and local image/video/audio files referenced as media sources. Import extracts into a new folder; Locate repairs missing media. Accounts and stream keys are excluded. Browser URLs remain present; check them before sharing a bundle. |
| Guided first run | New projects open a five-step setup guide; reopen from Edit → Setup guide. Covers accounts, capture, microphone, optional camera and an actual ten-second local recording to play back. |

## Important behavior

- Shared widget sources have one rendering size across their placements. Use independent copies for different responsive layouts on Canvas 1 and Canvas 2. Crop and placement are always per-layer.
- Recovery rebuilds the container from readable data. Data that never reached disk cannot be restored. MP4 conversion requires compatible codecs (the normal H.264 + AAC output is supported).
- Project bundles include only local media referenced by the project. Remote browser content, fonts installed on another computer, camera devices, provider credentials and account sessions are not bundled.
- Output retries handle transport interruptions; they cannot revive a broadcast that a platform has ended or fix an invalid stream key. A shared encoder failure remains distinct from an individual destination failure.
- GPU statistics require a supported kernel counter. Unsupported systems show an unavailable indicator rather than a guessed number. Encoder backlog is an estimate based on output progress, not a hardware encoder utilization percentage.
- Portable project operations and recording conversion run off the UI thread. Recording/remuxing needs FFmpeg; project bundles use Python 3. Close the file picker to cancel. Let a conversion finish before exiting the app.
- This release does not install, replace or update kernel/graphics drivers. Existing capture hooks and browser support are retained.

## Efficiency

No additional cloud service or always-running background daemon is introduced. Monitoring starts only when configured, project/archive operations run on demand, source frames remain shared, destination queues remain bounded, widget frames retain change-driven updates, and GPU counters follow the existing resource-sampling interval.

## Test boundary

Validated in this build environment using software Vulkan, local audio/video fixtures, local RTMP receivers, and native UI screenshots. This does not establish Fedora Wayland game-hook, AMD hardware encoder, real microphone/camera, live OAuth, or public-platform outage behavior on your computer. Those still need testing with this preview.
