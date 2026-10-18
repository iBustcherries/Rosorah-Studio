# 0.7.0 validation — 2026-10-10

## Passed

- Rust format check, Clippy across all targets with warnings denied, and optimized release build (Rust 1.99).
- 25 Rust tests passed. One cross-language socket integration test remains explicitly ignored. Tests include actual GStreamer sample frames/shutdown, independent mailbox locks, sealed memory, stale/malformed frames, source selection and renderer/bitness metadata.
- Release compositor `--self-check` passed: two canvas sizes, shared textures, placement, visibility, alpha, padded rows, unused assets and empty scene. Software Vulkan/llvmpipe; not an AMD benchmark.
- Both x86-64 and i386 Vulkan/OpenGL shared libraries compiled with warnings treated as errors. The 32-bit color, lookup, EGL and Vulkan test executables also compiled and linked. Header/message ABI sizes are asserted in both builds.
- 64-bit real surfaceless EGL/OpenGL and GLES readback: GPU downscale from 2560 × 1440, asynchronous PBO copy, correct 255/64/128/255 pixels, preservation of pixel pack settings, pack/unpack PBOs, texture/framebuffer bindings, scissor and sRGB state.
- 64-bit MSAA renderbuffer fixture: resolve, downscale and asynchronous readback through the same capture code produced the expected pixels. This is an offscreen framebuffer test, not a game window presentation test.
- The minimal preload library has no C++ dependency; versioned libdl imports and its dynamic dependencies were inspected. Missing-backend fallback preserved ordinary EGL rendering and teardown.
- RTLD_LOCAL graphics-library loading and function-pointer interception passed. A two-library preload-chain test verified that dlsym(RTLD_NEXT) retains the original caller scope. The release passthrough tail-call shape was inspected in x86-64 and i386 disassembly.
- Real Vulkan loader chain and offscreen GPU copy passed for RGBA8, packed RGB10 SDR, packed RGB10 PQ, packed RGB10 HLG and linear RGBA16F/scRGB. Expected SDR pixel vectors were verified. The baseline inactive, unfinished-copy, busy-slot and resource teardown checks still pass. No capture fence wait is permitted by the test override.
- Color conversion tests passed for 10-bit channel packing, half floats, the PQ 100-nit reference, scRGB 80-nit reference, and neutral monotonic PQ/HLG output.
- Isolated installer checks passed: architecture-specific manifests and bytes, host `$LIB` expansion, preservation of existing preload entries, repeat sourcing/installation, scoped uninstall, mapped-library retention and refusal to overwrite a foreign session file. Shell syntax and release exports checked; test helpers excluded from release libraries.

## Blocked or not exercised

- This host rejects execution of i386 ELF programs with `Exec format error`. 32-bit runtime/driver behavior is unverified; successful compilation is not a runtime test.
- The build sandbox denies Unix sockets. Native and cross-language hook-to-studio IPC integration could not run. This build retains the 0.6.2 socket/memfd path that the user reported working with Octopath Traveler, with backward-compatible metadata bits.
- The software Vulkan driver lacks required blit features for A2R10G10B10/BGR10. The GPU test reports a skip; CPU channel-order conversion is covered. Real GPU capture requires the format's transfer/blit features.
- The software driver's multisampled default EGL pbuffer resolved to zero even in a separate GL-only fixture without Rosorah. MSAA validation therefore used a real multisample renderbuffer fixture. Actual window-system multisample presentation remains untested.
- No native X11/GLX window, Wayland game surface, real HDR display, Steam container, AMD GPU, anti-cheat game or Windows capture was exercised. HDR conversion uses fixed reference luminance and needs visual confirmation on actual HDR games. No current-release Khronos validation-layer run was available.
- No measurements of game FPS, CPU cost, end-to-end latency or A/V synchronization. No claim that all games/renderers work.

The retained validation documents describe their historical versions. See the current README for installation and supported formats.
