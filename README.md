# Rosorah Studio 0.32.0-preview.7 — reliable account reconnection

- A saved sign-in that is still valid is reused at start instead of being renewed every time.
- A renewed sign-in is always saved to the wallet, even if Rosorah is closing or switching profile,
  so Kick and Twitch (which replace the refresh token on renewal) are never left signed out.
- Network trouble or a platform outage at start keeps retrying in the background instead of
  showing "Sign-in expired"; only a sign-in the platform actually rejects asks you to reconnect.

# Rosorah Studio 0.32.0-preview.6 — profiles, loading screen, start-up backups

- Settings > Profiles: separate projects (scenes, sources, mixer, alerts, automations, layout and
  backups) with shared accounts and stream keys. Create from a copy or empty, rename, delete, and
  Switch, which saves and restarts Rosorah into the chosen profile. The existing project is "Default".
- Settings > Backups: "Open the newest backup at start if it's newer than the saved project" (on by
  default). A project that can't be read also falls back to its newest readable backup.
- A loading screen covers the studio while fonts, the media engine, audio devices and accounts get
  ready (at least 0.8 s, at most 6 s), then fades into the already-drawn studio.

# Rosorah Studio 0.32.0-preview.5 — scene deletion fix

- Scenes can be deleted in projects imported from OBS/Firebot, restored from a backup or recovered.
  Those projects now get their per-canvas scene lists when they are opened, not only at startup.

# Rosorah Studio 0.32.0-preview.4 — alert sound fix

- Alert videos and sounds are heard again: they play whenever an alert triggers them, in any scene,
  instead of only when their video is a visible layer. Old per-scene mutes no longer silence them;
  the Alerts mixer strip controls them all.
- Alert media with sound that had no mixer channel (for example a video played by an automation)
  gets one automatically, with monitoring on.
- Changing the Alerts strip only copies the setting you changed to each alert sound.

# Rosorah Studio 0.32.0-preview.3 — text, backups, docking memory, importers

- Text layers: Layers > + Add layer > Text. Type a note, pick from 68 bundled open-licence font
  families plus every font installed on the computer (searchable), size or "Fit to box",
  alignment, colour, background, outline and drop shadow.
- Settings > Backups replaces the header's Save project button: Back up now, automatic backups
  every hour (when something changed) and/or every time Rosorah closes, how many to keep, and
  restore with an automatic safety backup first. Ctrl+S still saves.
- Docking remembers where a panel was: Dock in workspace (from a floating panel or its own
  window) and re-showing a hidden panel put it back in the same place and size.
- Drag a panel's tab past the edge of the main window to give it its own window. Drag a
  window's "⠿" handle out of it and let go to dock it back.
- The audio mixer shows one "Alerts" strip for every alert sound; its volume and mute apply to all.
- OBS import: crop, nested scenes and groups, cameras and DroidCam, screen/window/game capture,
  styled text (font matched to bundled/installed fonts), chroma key, crop and opacity filters,
  mic/desktop/application audio with gain and noise suppression, transitions including stingers,
  whole settings folders and multiple scene collections.
- Firebot import: events from Twitch, Kick, YouTube and donation services, commands with
  permissions and cooldowns, channel point rewards matched to their Twitch reward, counters,
  scene switches, effect lists and more variables. OBS and Firebot are found automatically.

# Rosorah Studio 0.32.0-preview.2 — panel windows, per-destination bitrate, keyring

- Right-click a panel's tab and choose "Open in its own window" to move it into a native window
  (for example onto another monitor). Close that window, or press "Dock in workspace", to dock it again.
  Canvas 1 always stays in the workspace. Popped-out panels are remembered.
- Each streaming connection can set its own video bitrate (Broadcast > connection > "Own video bitrate").
  Connections that share a canvas and bitrate still share one encoder; a different bitrate adds an encoder.
  Twitch is limited to 6,000 kbps on its own, so other destinations can use more.
- Stream keys are stored in the system keyring (Secret Service on Linux, Credential Manager on Windows).
  Keys already saved are moved there on first start. Without a keyring they stay in the private file.
- Automation "Send to chat" no longer offers TikTok, which can't receive chat from Rosorah.

# Rosorah Studio 0.32.0-preview.1 — Kick activity

- Kick follows, subs, resubs, gifted subs, Kicks and reward redemptions in activity, alerts and automations.
- Delete message for Kick chat; automation "Send to chat" can post to Kick.
- Audio mixer channels can be reordered by dragging; mixer bars stay inside the panel.
- Now Playing appears only for music apps and sites.
- Kick activity needs the updated sign-in broker and webhooks turned on for the Kick app
  (auth-broker/README.md). Sign out of Kick and connect again once after updating.

# Rosorah Studio 0.31.0-preview.9 — Kick sign-in preview

The shared Windows/Linux account implementation includes Kick browser sign-in,
renewable tokens in the OS credential vault, and automatic streaming-server/key
setup. The publisher client ID and HTTPS token service are built in; no Kick
client secret is distributed. Existing local account configurations take priority.

Kick chat, activity events, and broadcast editing are not part of this update.
See [preview.9 notes](docs/release-0.31-preview9.md) for validation and testing.

# Rosorah Studio 0.31.0-preview.8 — Windows capture and audio

Automatic fullscreen application/window capture, cursor capture, application
audio, named devices, microphone suppression and Windows media discovery.
See [preview.8 notes, setup and limitations](docs/release-0.31-preview8.md).
Windows Graphics Capture is used; injected renderer hooks are not included.
The Windows installer is cross-compiled and needs native Windows testing.

Preview.7 added system monitoring and reduced preview buffering/idle work:
[preview.7 notes](docs/release-0.31-preview7.md).

# Rosorah Studio 0.31.0-preview.6 — Windows capture performance

This preview optimizes Windows capture conversion and GPU readback handling.
See [preview.6 notes and validation](docs/release-0.31-preview6.md).
The reported one-second preview delay still needs a Windows retest; component
benchmarks are not an end-to-end latency measurement. The previous checkpoint
history follows below.

# Rosorah Studio 0.31.0-preview.5 — Windows test build

The full Windows application now links as an x64 EXE. Recording and streaming use cancellable Windows named pipes, with AMD AMF and NVIDIA NVENC probing plus software fallback. This is an unsigned, hardware-untested development preview, not a finished Windows release. See [start here](docs/windows-preview-start.md), [integration status](docs/windows-integration-checkpoint.md) and [validation](docs/validation-windows-integration.md).

# 0.31.0-preview.3 update

Layers now has a direct Alerts option and Rename/Delete context-menu actions. See [release notes](docs/release-0.31-preview3.md).

# 0.31.0-preview.2 update

Automation preview sizing, alert audio monitoring, and one shared Alerts box: see [release notes](docs/release-0.31-preview2.md). Windows backend source progress is documented in [Windows port](docs/windows-port.md); no Windows Studio installer is included.

# Rosorah Studio 0.31.0-preview.1

Desktop compatibility preview: shared portal file dialogs, capability-aware Wayland capture, an explicit X11 fallback, compositor exclusions and a Desktop compatibility panel. See **docs/desktop-compatibility-0.30.md** for routes, requirements and the real-desktop testing checklist.

This build adds crop and edge handles, multiple selection/groups, responsive widget layouts, project transfer, recording recovery/conversion and clearer stream/audio diagnostics. See **docs/next-build-0.29.md** for the complete approved scope and controls, and **docs/validation-0.29.0-preview.1.md** for testing.

Extract the complete Linux ZIP, close the previous Rosorah instance, then run `run-studio.sh` in the new folder. Back up your project before opening a preview build.

## Retained from 0.28

- Top-level Widgets menu and a dedicated editor with live/sample previews.
- Refined native chat, music, goal and utility cards; saved appearance styles and copy/paste.
- Automatic Linux desktop music discovery through MPRIS, artwork, progress and track fades.
- Project undo/redo, layer alignment and restore-source-proportions actions.
- Recovery checkpoints, five saved-project backups and optional stream readiness checks.

See **docs/widgets-0.28.md** for setup and limitations and **docs/validation-0.28.0-preview.1.md** for verification. Music discovery depends on the app/browser exposing desktop media controls. This is not a claim of tested compatibility with every music service. Chat canvas widgets remain text-only. Sports retains its existing provider/key requirements; schedule and poll cards remain manual/automation-driven.

Existing capture hooks, browser-source and chat helpers are retained. Main canvas/capture defaults remain 60 FPS. Animated widgets render at 20 FPS to limit overhead; stable music progress updates once per second. Existing donation hosting requirements are unchanged. Scene-local grouping and equal-spacing tools are now available under Arrange.

## Build

Rust 1.90+, GStreamer/base plugin and GLib/GIO development packages, pkg-config and a native toolchain. Run `cargo build --release --locked`. Native capture source is under `native/` and `packaging/`. Older documents describe their named releases.

## 0.31 preview: guided alerts, rewards and imports

See [release notes](docs/release-0.31.md) for the new guided alert setup, Twitch rewards editor, OBS/Firebot import preview, and the exact limits of the initial Windows groundwork. The runnable package remains Linux x86_64.

## License

Rosorah Studio's original project code is licensed under the [GNU General Public License, version 3 only](LICENSE) (`GPL-3.0-only`), except where a file or component carries a different license notice. You may use, modify, and redistribute the covered code under that license. Distributed binaries must be accompanied by corresponding source as required by GPLv3.

Third-party dependencies, vendored code, fonts, and media retain their respective licenses and notices. This license does not grant rights to third-party content you capture or stream, and does not require your recordings or broadcasts to be GPL-licensed merely because you created them with Rosorah.
