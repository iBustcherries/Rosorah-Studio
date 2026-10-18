# Rosorah Studio 0.5.1 validation

Date: 2026-10-09

Completed:
- cargo build --release --locked --offline
- Release --self-check passed: two canvas sizes, shared textures, placement, visibility, alpha, row padding, unused assets and empty scene; llvmpipe software Vulkan. Hardware performance unverified.
- Release --help reports 0.5.1.
- cargo check --workspace --offline
- cargo fmt --all
- cargo clippy --workspace --all-targets --offline -- -D warnings
- cargo test --workspace --offline: 25 tests passed (8 core, 3 capture engine, 14 app).

New regression coverage includes legacy preference loading, manual mode serialization, startup arming without a manual picker, pause disabling retries, one shared default source, scaled second-screen geometry, Steam window identification, minimized/self exclusion, and exact Steam metadata parsing. Existing audio routing, frame conversion, GStreamer synthetic capture and shutdown tests passed.

The automatic backend has not been exercised against a real KDE session, actual Steam game, or microphone here. Native interactive layout has not been visually verified in this environment. Renderer hooks are not implemented. The synthetic GStreamer test verifies frame delivery and shutdown, not real desktop capture. Audio/video recording and streaming limitations from 0.5.0 remain.

## Fedora test

1. Exit the previous Rosorah process. Extract this package to a new folder and run run-studio.sh.
2. Leave automatic game capture selected, then launch a Steam game. No Start click should be needed.
3. Try fullscreen, borderless and windowed modes. Minimize and restore the game.
4. Leave the game on a static menu for at least 20 seconds; it should remain captured.
5. Close/reopen the game. Confirm capture reconnects automatically.
6. Pause capture and confirm it stays paused for the session.
7. Choose a window manually, close Rosorah and reopen. The picker should not appear until you choose a window.
8. If capture is blank, use Capture details → Copy diagnostics and include the game name when reporting it.
