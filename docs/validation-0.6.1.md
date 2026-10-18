# 0.6.1 validation — 2026-10-09

- Optimized release build and compositor self-check passed on software Vulkan.
- Rust formatting and strict Clippy passed; 25 workspace tests passed.
- New lock-contention regression holds the producer's exclusive lock for 100 receiver polls. The receiver keeps its source and resumes that same source after unlock.
- New resize-gap test retains the last frame, clears on a real timeout and reconnects to a new mailbox.
- Real Vulkan offscreen GPU readback passed with Khronos validation enabled and no validation errors. A test override detects any call to WaitForFences in capture; none occurred. Simulated NOT_READY fence status retains pending resources and skips delivery. Completed pixels and presentation semaphore consumption remain correct.
- 2560×1440 mailbox input is sampled to 1280×720; dimensions, corner pixels and exact payload length passed.

This is not an AMD game performance benchmark. The actual preview flicker and slowdown reported by the user require retesting on their desktop. Actual window-system presentation was not exercised here. CPU sampling and original-size GPU staging copies remain; capture can still affect frame rate. OpenGL, 32-bit, HDR and Flatpak limitations from 0.6.0 remain.

Keep the Steam runtime directory launch option that was confirmed to make Octopath capture. Close the game before installing the updated hook; already-running games retain the old library.
