# Rosorah Studio 0.6.0 validation — 2026-10-09

## Completed

- Optimized Linux release build and compositor self-check passed on llvmpipe. Release help reports 0.6.0.
- Release hook exports the presentation/loader entry points and does not export the test helper.

- Rust workspace check, formatting and strict Clippy (all targets, warnings denied).
- 23 Rust tests passed: core/project migration, automatic start and manual-mode behavior, capture lifecycle, audio routing, native GStreamer synthetic delivery, renderer mailbox parsing and BGRA conversion, rejection of malformed/stale/future/oversized packets, and producer/consumer lock behavior.
- C++17 release hook compiled with warnings treated as errors.
- Test-only capture library loaded through the real Vulkan loader using llvmpipe software Vulkan.
- A real offscreen Vulkan image was cleared to a known color, copied through the production readback path, and published through the C++ mailbox. Expected RGBA bytes 255,64,128,255 were verified.
- Inactive heartbeat test left the original semaphore unconsumed; the application's downstream wait completed.
- Simulated readback timeout retained the pending resources and returned a replacement semaphore. Its downstream wait and resource teardown completed.
- The same GPU tests passed with VK_LAYER_KHRONOS_validation enabled beneath the capture layer, with no validation errors emitted.
- Per-user installer was exercised in an isolated data directory. With zero explicit layers requested by the test application, the installed implicit hook loaded automatically and all GPU tests passed. Uninstall removed the manifest afterward.

## Not verified here

No real desktop/game swapchain or native interactive UI was exercised. The Vulkan tests use offscreen images and the same capture submission code with GENERAL return layout; they do not emulate a complete window-system presentation engine. Actual vkQueuePresentKHR swapchain interception, image acquisition/reuse across frames, game compatibility, frame rate impact, AMD hardware performance, Steam/Proton visibility of the installed layer, and audio/video synchronization still require desktop testing.

This is an experimental 64-bit Vulkan capture build. OpenGL, 32-bit, HDR, protected content, multi-device capture and Flatpak deployment are not implemented. No KDE window capture backend remains. Manual portal capture and the existing audio backend remain separate.

## Desktop test

Close the previous app, extract the complete archive, run run-studio.sh, then restart a Vulkan game once. Leave the source on Automatic renderer capture. Confirm frames arrive without Start or a KDE picker. Pause and resume, resize the game, close/relaunch it, and verify capture reconnects. Send Copy diagnostics and the game name if it stays blank. State whether Steam is native/RPM or Flatpak and whether the game is native or Proton.
