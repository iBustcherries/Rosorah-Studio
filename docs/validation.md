# Rosorah Studio 0.4.0 validation

Build environment: Ubuntu 24.04 x86_64, Rust 1.99.0, GStreamer 1.24.2.
Release builds use normal optimization with LTO disabled.

## Automated checks

- `cargo fmt --all --check`: passed.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
- `cargo test --workspace --locked`: 15 passed.
- New tests cover actual GStreamer videotestsrc → conversion/scaling → RGBA
  appsink frame delivery and pipeline transition to NULL; padded row copying
  and truncated frame rejection; capture source JSON roundtrip/shared identity;
  legacy project/layout migration without overwriting newer files.
- Existing core audio/chat/automation and docking persistence tests still pass.

## Scope of capture validation

The GStreamer test uses a generated video source, not a real desktop capture.
This environment cannot validate the KDE permission picker or an actual Wayland
PipeWire remote session. Real Fedora KDE Wayland acceptance is still required:

1. Allow a monitor, then a window; verify moving content and cursor.
2. Use Add existing capture in the other canvas; verify shared content.
3. Cancel the picker; retry. Stop capture; reconnect. Close the selected window.
4. Revoke sharing using KDE; verify stale pixels clear and status changes.
5. Resize the captured window; verify dimensions/content update.
6. Restart the app; verify capture is stopped until explicitly selected again.

Live audio, capture recording, streaming, GPU zero-copy and hardware performance
are not claimed by this milestone. Recording/export commands reject projects
containing live capture sources rather than silently exporting empty frames.

## Native UI and compositor checks

- Native X11 window exercised under Xvfb with software Vulkan. Renamed header,
  existing docks and capture controls render correctly.
- Desktop portal failure is reported in Sources. Retrying stays responsive;
  closing and reopening preserves the capture source without auto-starting it.
- Stopped/reopened capture textures are transparent, so other scene layers remain
  visible. The GPU self-check also verifies that placeholder initialization does
  not overwrite an already uploaded capture frame.
- `--self-check` validates both canvas sizes, source reuse, composition, opacity,
  visibility and pixel readback with llvmpipe. This is not AMD hardware validation.
- Live-source export rejection verified before output files are created.

## Custom engine checks

Three new tests exercise the backend contract with a controlled backend:
- 100 publications retain only the latest frame, count 99 replacements and drain once.
- Stop rejects late frames; reconnect isolates a new producer from the old session.
- Duplicate starts do not create another session; invalid RGBA frames are rejected.

Desktop end-to-end limitations above still apply. Game hooks, automatic game
selection and game audio are future backends, not included in this build.
