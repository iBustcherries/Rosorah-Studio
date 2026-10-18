# Rosorah Studio 0.17.1 validation

## Scope

Efficiency changes preserve the existing capture/encoding rates and resolutions, per-canvas outputs, immediate chat/event delivery, media playback and alerts, scene transitions, scene audio, monitoring, and noise suppression. The default capture rate remains 60 FPS. No driver, kernel, desktop, or account setup changes are required by this update.

Changes are limited to avoiding repeated work: per-compositor static-asset caching; reusable GPU vertex buffers and CPU geometry storage; replacement of obsolete resized targets; live-frame uploads restricted to each output's visible sources; shared immutable audio configuration and revision-cached scene mixes; borrowed camera/video settings during synchronization; and chat-event relaying only when the chat feed changes.

## Focused performance measurement

The release-mode `media::cache_tests::measure_repeated_image_edits` test makes 60 layer-transform edits with a 1920 × 1080 PNG and two color sources. It renders and reads back a 64 × 36 target after each edit so GPU work completes. The reference recreates the asset cache for each edit, forcing the repeated decode/upload behavior; the optimized path retains its warmed cache. This compares the asset-update paths, not two complete application processes.

One local software-Vulkan run:

| Path | Total time for 60 edits | Source uploads during edits |
| --- | ---: | ---: |
| Forced asset reload | 461 ms | 180 |
| Reused static assets | 82 ms | 0 |

These numbers are not a prediction of whole-app CPU/GPU usage or AMD hardware performance. File content, scene complexity, active outputs, drivers and encoding configuration determine actual savings.

## Regression checks

- Release workspace suite: 105 tests passed; 15 environment-dependent/performance tests are excluded from the default run. Relevant Vulkan and media tests were run explicitly.
- GPU geometry/resize regression: 120 frames on each of two canvases use only two vertex-buffer allocations and two initial geometry uploads. A transform edit updates pixels without another allocation. An empty scene clears correctly. Twenty subsequent resizes keep two current targets and preserve the other canvas's target.
- Existing GPU fade/wipe and canvas-isolation test passes on software Vulkan.
- Static-asset regression checks unchanged uploads across 120 unrelated edits, image-file replacement, missing-file error/recovery, and image-to-color replacement.
- Per-output frame-upload regression checks that hidden sources are skipped, revealing a source uploads its retained latest frame, a stopped source clears, and static replacements cannot be overwritten by leftover capture frames.
- Audio configuration snapshots share storage across 1,000 reads while later changes leave existing snapshots immutable. Cached scene mixes update on revision/scene changes and remain independent between canvases.
- Real GStreamer/FFmpeg media integration passes: a looping video with sound reaches the recording; a scene change reaches encoded pixels; scene muting silences its sound. The 10 ms audio mixer clock, sampling rate and output frame scheduling are unchanged.
- Strict release workspace/all-target Clippy passes. Native compositor self-check passes. Both previews render correctly in a native UI screenshot under Xvfb/software Vulkan.
- Browser companion and native capture hooks retain the previous release's implementation. The process-lifetime libsecret handle that fixes the previous wallet crash remains unchanged.
- Release ZIP/TAR entries, contents, executable permissions and SHA-256 checksums are checked during packaging. Source is included in a separate archive.

## Limits

Validation uses Linux with software Vulkan, GStreamer 1.22 and FFmpeg 5.1; there is no physical AMD GPU, Fedora PipeWire session, camera, or signed-in Twitch/YouTube account here. End-to-end hardware usage and provider delivery still require a check on the user's machine. Existing platform polling schedules and browser background collection are unchanged.

Static image replacements are checked by file modification time and length on the next project/preview refresh, matching when assets were previously refreshed; reopening starts with an empty cache. The cache holds metadata and existing GPU textures, not an additional decoded CPU copy of every image. Old preview targets are discarded when their canvas dimensions change; in-flight GPU/UI references retire safely through WGPU.
