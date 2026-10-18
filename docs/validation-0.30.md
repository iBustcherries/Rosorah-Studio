# Rosorah Studio 0.30.0-preview.1 validation

## Completed

- Locked release build using Rust 1.96.0.
- Workspace tests: 174 passed, zero failed; 23 environment-dependent tests ignored by default.
- Additional isolated D-Bus portal test: file selection, second-dialog cancellation, and capability report passed. Each operation uses a fresh connection.
- Additional real X11 capture test under Xvfb: delivered frames and stopped cleanly.
- Five additional GPU compositor tests passed using software Vulkan (llvmpipe).
- Packaged release self-check passed: two canvas sizes, shared textures, placement, stacking, visibility, alpha, row padding, unused assets and empty scene.
- Release GUI launched under Xvfb; Edit menu and Desktop compatibility panel inspected at 1440 x 900. This is a virtual X11 server, not a real XFCE session.
- ZIP CRC and byte-for-byte content validation performed during packaging. SHA-256 checksums included.

## Not verified in this environment

Real KDE/GNOME/COSMIC/Xfce/MATE/Cinnamon or standalone Wayland compositor sessions; physical camera/audio/GPU drivers; hardware encoding performance; live provider accounts; real game hooks and Steam containers. Existing hooks and browser/relay bundles are retained from 0.29.0-preview.1.

This is a compatibility preview, not a guarantee that every desktop exposes every feature. See docs/desktop-compatibility-0.30.md for required interfaces, fallback behavior and limitations. No system kernel, graphics driver or desktop package changes are made by this release.

## Quick check on your desktop

1. Close the previous Rosorah instance, extract the complete ZIP, and run run-studio.sh.
2. Open Edit → Desktop compatibility and check the detected desktop/session and available tools.
3. Try a media file chooser, cancel it, then open it again.
4. Try screen and window capture separately, if advertised by your desktop.
5. Confirm game capture, audio, camera, recording and account persistence with your existing setup.
