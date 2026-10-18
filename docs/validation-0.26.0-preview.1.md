# Validation — 0.26.0-preview.1

- Workspace suite: 144 passed, zero failed. Eighteen environment-dependent tests are excluded from the default run.
- Explicit Vulkan suite: four tests passed using llvmpipe. New chroma test verifies green removal, foreground preservation and immediate filter toggling without re-uploading the source. Existing canvas routing, transition and geometry reuse tests pass.
- Native Qt WebEngine fixture: an animated HTTP webpage produced opaque red pixels and transparent background pixels, and its generated tone was captured through its isolated sink. Graceful exit removed that sink.
- Additional browser integration test: passed with the actual Python companion, Rust capture engine, Vulkan compositor and FFmpeg recorder. Decoded recording contained the red webpage pixels, the blue layer beneath transparent areas, and nonzero browser audio. Source cancellation completed within five seconds.
- Sidebar regression: 180 consecutive redraws with a long rule name and a 1400-pixel saved sidebar width remain bounded to 360 pixels. The native automation page was visually inspected after correcting row height/alignment.
- Application self-check passed on software Vulkan: both canvas sizes, shared textures, placement/stacking, visibility, alpha, padding, unused assets and empty scene.
- Release build uses locked dependencies. One existing unused Activity helper warning remains.
- Package ZIP/TAR.GZ contents are checked against staging; source ZIP integrity checked. Existing capture hooks, launch helper and account chat browser scripts retain their previous bytes.

Tests used an isolated local HTTP fixture, temporary PulseAudio test server, Xvfb and Qt's software rendering backend. The root-only test environment required disabling Chromium's sandbox for QA; the shipped application does not set that override. No host GPU drivers, kernel packages or user display configuration were changed. Downloaded dependency packages were extracted into the build workspace.

Remaining checks: real Fedora KDE/Wayland hardware behavior, specific VTuber applications, third-party overlay services, network streaming and high-resolution performance. The browser test used a 320×240 fixture; it does not establish a performance guarantee for 1080p60. Chroma key is GPU based, but browser capture is not zero-copy. Browser audio follows the shared media mix, and overlay sessions reset when their sources shut down.

See browser-sources.md for setup and current limitations. This update supports external avatar capture with chroma key; it does not add Live2D/VRM loading or face tracking.
