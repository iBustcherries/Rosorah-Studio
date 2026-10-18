# 0.5.0 validation — 2026-10-09

## Completed

- Rust 1.99.0, Linux x86_64 release build.
- `cargo fmt --all` and strict Clippy (`--workspace --all-targets -- -D warnings`).
- 19 unit/integration tests: 7 core, 3 capture lifecycle, 9 application tests.
- New checks cover audio-node filtering, explicit app-port selection excluding another app, WAV headers/length, and fullscreen selection/minimize/exit transitions.
- Existing tests cover six-track gain/mute/clipping, chat and alerts, source sharing, project migration, dock persistence, stale capture generations, and actual GStreamer test-frame delivery/shutdown.
- Release compositor self-check: two canvas shapes, source sharing, placement, visibility, alpha, padded rows and empty scenes. Software Vulkan (llvmpipe); this does not measure AMD performance.

## Not verified here

A local D-Bus session failed to start because this sandbox prohibits Unix socket creation. Therefore no live PipeWire server, KDE session, microphone, game audio isolation, or GUI mouse/visual check was possible. The fullscreen backend is experimental until tested on Fedora KDE. No claim is made that end-to-end capture passed.

## Fedora acceptance checks

1. Open an existing 0.4 project; verify the dock layout survives. Add a capture source with the default fullscreen mode, and fullscreen a game. Confirm preview follows it.
2. Minimize or close that game: preview must clear rather than expose the desktop. Return to fullscreen; verify reacquisition. With two fullscreen apps, verify active-window preference.
3. Click Choose window and use KDE's picker. Cancel, retry, stop and reconnect. Confirm switching modes is responsive.
4. Play game audio and an unrelated browser stream. In Audio select only the game's Application entry; record a short audio test. Track 1 should include the game and exclude the browser. Test microphone separately and together.
5. During an audio test, change volume/mute and track routing. Stop recording and inspect all six WAV files. No speaker monitoring is enabled, to avoid feedback.
6. Restart the game/device, refresh inputs and reselect. Verify failed inputs report an ended state and do not silently capture a different app.
7. Resize each dock to narrow and wide widths. Verify mixer cards wrap, advanced controls expand, menus remain reachable, and snapping remains usable.

## Current limitations

Audio inputs require manual selection and are not automatically paired to captured windows. Audio input selection and capture mode are session-local. The audio recorder is a development test recorder using buffered PCM and a worker clock, not the final synchronized A/V pipeline. Refreshing/starting an input may interrupt the worker; do not use this build for production recording. CEF, platform accounts, streaming and live A/V recording remain pending.

## Dependency references

- PipeWire client options: https://docs.pipewire.org/page_man_pw-cat_1.html
- PipeWire node properties: https://docs.pipewire.org/page_man_pipewire-props_7.html
- Fedora utilities: https://packages.fedoraproject.org/pkgs/pipewire/pipewire-utils/fedora-44-updates.html
- Desktop portal: https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html
- KDE protocol XML is supplied by `wayland-protocols-plasma`; no OBS code is used.
