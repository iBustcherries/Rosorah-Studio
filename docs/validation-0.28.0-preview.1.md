# Validation — 0.28.0-preview.1

- Rust workspace compilation and optimized Linux release build completed.
- Automated workspace tests: **161 passed, 0 failed**. Twenty tests are excluded from the default run: eighteen existing hardware/service checks and two explicit media/design checks.
- The new media check was run separately against an isolated TCP D-Bus session and passed: player discovery, metadata decoding, playing state, position, duration and progress. This uses a synthetic MPRIS player, not a real Spotify/YouTube Music account.
- The design-export check was run separately and passed. Actual native music, unified chat, Windows 98, Windows XP, phone-bubble and goal output was rendered and visually reviewed. These are sample messages and an illustrative music track, not connected-account output.
- Regression tests cover older manual-music settings, style/content separation, undo/redo, preservation of live widget totals, drag coalescing, project backup/recovery-file lifecycle, safe unknown music duration, transparent unavailable music, themes and goal styles. Existing capture, audio, recording, automation, browser and donation tests remain in the workspace suite.
- GPU self-check passed on software Vulkan (llvmpipe): two canvas sizes, shared textures, placement/stacking, visibility, alpha, row padding, unused assets and empty scenes.
- The native editor opened in a virtual X11 desktop, rendered its sample preview and controls within the window, saved its project and closed successfully.
- Packaging verifies ZIP CRCs, retained helper bytes, executable permissions, the packaged application hash and startup. The source ZIP includes Cargo.lock and the new modules/tests.

## Practical limits

No real Fedora/KDE/Wayland desktop, AMD hardware encoding session, game, USB camera or authenticated streaming/donation account was available for end-to-end testing. No percentage reduction in CPU/GPU usage is claimed. Capture hooks and browser helpers are retained from 0.27; they were not rewritten for this update.

Music services are not individually certified: discovery requires the installed desktop app/browser to expose MPRIS metadata. The implementation shares local discovery, caches bounded artwork, idles when unused, renders stable playback progress once per second and uses short 20-FPS transitions. These are implementation limits, not measured hardware benchmarks.

Full canvas multi-selection/grouping/equal-spacing and inline canvas-chat emotes are outside this preview. The optional readiness screen is a local configuration/status check, not a remote ingest test. See widgets-0.28.md for user-facing behavior and setup.
