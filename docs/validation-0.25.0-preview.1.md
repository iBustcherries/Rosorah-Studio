# Validation — Rosorah Studio 0.25.0-preview.1

## Automated verification

- Workspace tests: 140 passed, 0 failed; 16 environment-dependent tests excluded from the normal run.
- Explicit GPU run: all 3 Vulkan tests passed using llvmpipe. These check transitions/canvas isolation, geometry reuse and target lifetime, and the new alert media canvas filter, including restoring visibility on both canvases.
- New automation coverage: numeric/permission matching, per-viewer cooldowns, deduplication, literal variable expansion and privacy, legacy defaults, waits/pause, queue limits, overlap/interrupt, media completion before the next queued alert, skip targeting, import reference remapping/disabled defaults, and safe test/replay action filtering.
- Release build completed with the locked dependency set. The existing test-only Activity helper emits one dead-code warning in release builds.
- Native Alerts & automation editor rendered under Xvfb and was visually inspected. Basic controls, saved rule loading, input labels and scroll layout were checked.
- Application compositor self-check passed: two canvas sizes, shared textures, layer placement/stacking, visibility, alpha, padded rows, unused assets and empty scene.
- Linux ZIP and TAR.GZ contents were checked byte-for-byte against staging; source ZIP CRC integrity checked. Existing game-capture/browser helper bytes were preserved from 0.24.

Tests used a temporary project without real account credentials. Local camera HTTP tests used a fresh GStreamer registry and bypassed the test environment's external proxy. No system GPU drivers or kernels were changed.

## Remaining real-system checks

Live provider events, streaming output, hardware GPU performance and global desktop integration require testing on the target Fedora system. Software Vulkan validates rendering correctness, not hardware performance or resource savings. No quantified efficiency improvement is claimed.

Hotkeys require Rosorah focus. Test preview skips widgets and scene switches and blocks media while streaming/recording. Media canvas selection affects video visibility; media audio remains in the shared mix. Role and numeric filters only match when the event adapter supplies the required data. Browser YouTube collection does not currently provide trusted role or numeric values for all event types.

Rule replay intentionally excludes widget updates and scene changes, so it cannot increase goals/counters. Missed donation history depends on the connected receiver/provider. Existing Ko-fi hosting requirements are unchanged; this release does not deploy a receiver.

Read automation-guide.md for queue semantics, media requirements, import reference matching and persistence boundaries.
