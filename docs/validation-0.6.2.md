# 0.6.2 validation — 2026-10-09

Passed:

- C++17 optimized release hook build with warnings as errors.
- Rust release build, formatting and strict Clippy across all targets.
- 24 Rust workspace tests. The shared-memory regression uses a real sealed memfd, reopens it for an independent lock owner, confirms 100 contended reads hold the previous frame, validates BGRA conversion and rejects resize attempts. Parser tests reject stale/future/oversized/old-protocol packets. Source selection prefers Steam, pins the current Steam source and ignores pending handshakes.
- Offscreen native Vulkan test through the real loader with Khronos validation: 2560 × 1440 source downscaled on the GPU to 1280 × 720, expected RGBA pixel 255/64/128/255, original/replacement semaphore handling, inactive passthrough through the capture entry point, simulated unfinished fence, both readback slots busy, teardown. No validation errors. Capture does not call WaitForFences.
- Compositor self-check on llvmpipe: two canvas sizes, shared textures, placement, visibility, linear-light alpha, padding, unused assets and empty scenes. The shared composition routine now returns an Unorm display view of the sRGB render target. Export still reads back explicitly.

- Isolated per-user installation and scoped uninstall passed: correct 0.6.2 manifest/library bytes, unrelated layer retained, versioned library retained for running games. Release exports contain the loader entry points and exclude the test helper. Shell syntax checks passed.

Blocked / not measured:

- The sandbox denies AF_UNIX socket creation with EPERM. The compiled native IPC test reports BLOCKED and exits 77. It is not counted as passed. The C++ producer → Rust receiver integration test is present and explicitly ignored by default; it has not run here.
- No real Fedora desktop, Steam container, Proton game, AMD GPU benchmark or WSI present/acquire cycle was available. End-to-end latency and game FPS need measurement on the target machine. GPU-only egui canvas presentation has been compiled, but not visually exercised on a desktop here.

The shared-home rendezvous follows Valve's documented native Steam runtime filesystem behavior. It is an implemented compatibility change, not a demonstrated Steam integration result from this environment. Remove the previous Rosorah launch-option workaround and restart the game after installing the new hook. No special Steam launcher or replacement command is supplied.

The retained validation documents for older versions describe their historical behavior, not the 0.6.2 transport.
