# Rosorah Studio 0.8.0 prototype validation

Build date: 2026-10-10 UTC. Target: Fedora x86-64, native/RPM Steam.

## What changed

The Rust app starts `rosorah-launch-watch` with the studio's process identity and installed hook path. The C++ helper watches the current user's Steam and Steam Runtime launcher threads for process-creation/exec events. A qualifying new process receives the existing architecture-dispatched GL preload in a staged initial stack, then the helper detaches. The original argument strings, argument pointers and auxiliary vector are retained. A failed plan or incomplete staging does not change the original stack pointer. A same-version preload is left alone; older Rosorah preload entries are replaced while other preloads are preserved.

This targets newly launched games, not an OpenGL game whose renderer already initialized. Steam and KDE are not restarted. No per-game launch configuration is written. The existing Vulkan implicit layer, GLX/EGL capture code and v2 frame transport are unchanged. All six shipped renderer libraries are the unchanged 0.7.0 release binaries, checked against that package; only the app and launch observer are newly compiled for 0.8.0.

The observer starts without elevated rights. An explicit UI action can run the helper through pkexec if permissions block observation. The helper restricts its target user to the invoking account, refuses a root studio, validates the owner PID/start time and stops when its parent control pipe closes or the owner disappears. It does not install a persistent privileged service or weaken SELinux/ptrace settings. The default installer replaces the old owned KDE login-script route with launch observation; an explicit session-hook fallback remains available.

## Completed checks

- 27 Rust tests passed, including the new status/identity tests and the real GStreamer synthetic-frame/shutdown test. One existing native-to-Rust socket integration test remains ignored.
- `cargo fmt --all --check` and Clippy for the workspace/all targets with warnings denied passed.
- The optimized release build completed. Its `--self-check` passed both canvas sizes, shared textures, placement, visibility, alpha, row padding, unused assets and empty-scene checks on software Vulkan (llvmpipe).
- All six production renderer libraries match the 0.7.0 package by SHA-256 and contain no `rosorah_test` exports.
- Native helper compiled with C++17 and warnings treated as errors.
- Simulated 32-bit and 64-bit initial-stack tests passed: argument/auxv integrity, other preload preservation, replacing an old Rosorah preload, idempotence, game-id filtering, explicit capture opt-out, secure-execution refusal, malformed input and insufficient stack space.
- Those same native tests passed with AddressSanitizer and UndefinedBehaviorSanitizer. Leak checking was disabled for the tracing-restricted test host.
- Installer tests passed in an isolated directory: both renderer architectures, default setup without a KDE login script, optional session fallback, removal of the owned legacy script, repeated install, existing preload preservation, scoped uninstall and foreign-file protection.
- Real 64-bit software EGL/OpenGL 4.5 and OpenGL ES 3.2 readback checks passed, including GPU downscale, PBO/fence readback, state restoration, an MSAA renderbuffer fixture, optional-backend fallback, RTLD_LOCAL lookup and chaining with other preloads.
- Real 64-bit software Vulkan loader/readback checks passed for SDR8, SDR10, PQ RGB10, HLG and scRGB. Inactive passthrough, busy-copy behavior and teardown checks passed.

- Binary tar and source ZIP contents were read back and checked against the staged/source files, including executable modes. Test binaries and build caches are excluded; locked third-party dependency entries are unchanged from 0.7.0.

## Blocked or not established

**The new live process-attachment path has NOT been verified on this host.** Even attaching to a test child with PTRACE_TRACEME returns EPERM. The live exec-to-OpenGL test exits 77 before attachment. The host also exposes a procfs view that does not match its emulated process ids; its owner/launcher discovery cannot serve as a Fedora integration test.

Unix socket creation is denied here, so complete hook-to-studio frame transport could not be retested. The software Vulkan driver lacks the required BGR10 blit features; that format's GPU test was skipped. The full native test runner exits 77 to report these gaps, not a complete integration pass.

No real Steam session, pkexec dialog, SELinux configuration, fork/vfork/clone launcher tree, Steam container, 32-bit exec attachment, actual game window, console emulator, AMD hardware, anti-cheat/protected game, HDR display, end-to-end latency or game FPS was tested. Privileged executable transitions and launchers that remove preloads may be incompatible. Flatpak launchers are skipped. Automatic OpenGL activation outside qualifying Steam launches is not implemented. Windows is deferred.

## Fedora acceptance check

1. Keep the existing Steam and KDE session open. Close the previous Rosorah build and launch this package's run-studio.sh.
2. In Capture details, wait for Watching new Steam games. If blocked, use the explicit permission button. Do not disable system security policies if it remains blocked; collect diagnostics instead.
3. Start an ordinary single-player OpenGL game after the observer is ready. Native Dead Cells is a candidate if your installed build uses OpenGL. Confirm received OpenGL frames, not merely a Hook prepared message.
4. Close that game and launch the previously working Vulkan/Proton Octopath Traveler. Confirm capture still works and compare game FPS with Rosorah closed/open.
5. Close Rosorah. Steam should remain responsive. Relaunch Rosorah and repeat an OpenGL launch to verify cleanup/reconnection.
6. If anything fails, provide Copy diagnostics, the game and native/Proton mode. Keep 0.7.0 available for comparison. This is an experimental prototype, not a validated all-game release.
