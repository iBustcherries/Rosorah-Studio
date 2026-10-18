# Validation — 0.31.0-preview.1

- Linux release build and locked workspace tests: 181 passed, zero failed; 23 environment-dependent tests ignored by default.
- New tests cover guided-rule creation, missing-file rollback, targeted alert routing, reserved text placement, reward field validation, Windows/Linux configuration paths, sound-only PCM playback, and test/replay suppression of chat posting.
- Existing media, audio processing, recording and project tests continue to pass.
- Four external-import tests pass: Firebot rules imported disabled and scripts excluded, OBS placement/missing-file handling, ZIP traversal rejection, and credential-file exclusion.
- `cargo check` for `x86_64-pc-windows-msvc` passed for the shared project core, capture contracts and GPU compositor. This is compile validation, not a Windows application/runtime test. The included GitHub workflow has not been run on a hosted Windows machine in this session.
- Native UI interaction check passed: two guided alerts were created and saved while reusing one alert box.
- Native Linux GUI screenshots of the three guided-editor steps and a converted OBS fixture were inspected under a virtual X11 display at 1440 × 900. Software Vulkan composition self-check passed.

Live Twitch mutations, account reauthorization and outgoing chat were not exercised against real accounts. No messages were sent and no rewards were created during development. These need user account testing. Real user OBS/Firebot backups are still needed; supported fixtures do not certify every backup version. Windows capture/audio/camera/credential backends and a Windows installer remain future work.

Sound files enter the existing audio mixer/output tracks. Enable mixer monitoring if you want to hear them locally; monitoring is not enabled automatically.

See release-0.31.md for the exact feature scope and importer limitations.
