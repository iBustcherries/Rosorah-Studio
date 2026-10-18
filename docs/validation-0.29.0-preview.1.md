# Validation — 0.29.0-preview.1

- Workspace Rust tests: 168 passed, no failures; 21 environment/design tests excluded from the default run.
- All 5 Vulkan compositor tests passed on llvmpipe, including crop UVs and isolation between canvases, transition behavior, chroma key, reusable geometry and widget proportions.
- Real FFmpeg local RTMP test passed: two outputs, one interrupted and reconnected, with a continuous shared local recording. No public streaming service was contacted.
- Portable project export/import round trip preserved media bytes, crop/group settings and avoided overwriting an existing archive.
- Unsafe ZIP paths and external media references were rejected; partial imports were cleaned up.
- FFmpeg recovery and MP4 conversion passed using a real H.264 MKV fixture; original bytes were unchanged and existing output was protected.
- Audio tests cover monitor-only exclusion from output alongside existing gain/mute/track routing.
- Canvas tests cover multiple linked previews, corner/edge resizing, grouped movement, canvas lock and snap bypass.
- Responsive widget frames validated at wide, landscape and portrait sizes; design review images inspected.

Native UI, final release/archive checks and exact package hashes are recorded in the bundled VALIDATION.md and SHA256SUMS.

These are local software-rendering/loopback checks, not tests of Fedora game capture, AMD VAAPI, physical audio/video devices, real account renewal or public stream ingest.
