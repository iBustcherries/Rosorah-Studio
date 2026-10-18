# Rosorah Studio roadmap

Updated for 0.32.0-preview.15 (October 2026). Rosorah Studio is original code built on
independent libraries (Rust, egui/wgpu, GStreamer, Qt WebEngine). It aims to be an easier
all-in-one alternative to running OBS, a chat bot and an alert service side by side,
not a feature-for-feature copy of OBS.

## Where things stand

| Area | Status | What's there |
| --- | --- | --- |
| Studio and layout | Shipped | Simple and Advanced modes, first-run guide, docking with tab groups, floating panels and separate native windows, saved layouts, web page panels, themes and backgrounds, undo/redo |
| Scenes and layers | Shipped | Scenes named in place, drag to reorder with sliding rows (also the mixer and automation steps), square/rounded/circle layer frames with faded edges, a green-screen key (experimental), main and vertical videos with linked scenes, crop/align/group tools, text layers with 68 bundled font families, widgets (chat, music, goals, utilities) |
| Capture (Linux) | Shipped | Automatic game capture (Vulkan/OpenGL hooks) with game audio, PipeWire desktop/window capture, cameras and DroidCam, browser sources |
| Capture (Windows) | Preview | Windows Graphics Capture, application audio and cursor; renderer hooks are not included; needs more testing on real Windows PCs |
| Sound | Shipped | Mixer with per-scene volumes, Sound settings window (device, live level, noise suppression), monitoring, delay, six recording tracks |
| Recording and streaming | Shipped | Hardware encoders (AMD/NVIDIA, software fallback), multistreaming to Twitch, YouTube, YouTube vertical, Kick, TikTok and custom RTMP, per-destination bitrate, reconnect, Go live checklist |
| Accounts | Shipped | Twitch, YouTube and Kick sign-in with renewable tokens in the system keyring, titles/categories, automatic stream keys |
| Chat and activity | Shipped | Unified Twitch/YouTube/Kick/TikTok chat with moderation, Activity history for every connected account |
| Alerts and automations | Shipped | Guided alert setup, built-in sounds and animations, triggers from follows, subs, gifts, bits/Kicks, raids, redemptions, donations and chat; Twitch rewards editor |
| Donations | Shipped | StreamElements, Streamlabs and Ko-fi (Settings → Accounts → Donations), donation goals |
| Transitions | Shipped | Cut, fade, wipe, Move (shared layers glide to their new place), stinger videos and transition screens |
| Projects | Shipped | Profiles, hourly and on-close backups, recording recovery, OBS and Firebot import |
| Announcements | Shipped | "I'm live" posts to Discord (webhook) and Bluesky (app password), once per stream, with a message template and per-stream skip |
| Distribution | Shipped | Windows installer, Fedora .rpm and Ubuntu/Debian .deb on GitHub Releases; in-app update check |

## Next

1. **Windows testing pass**: check capture, encoders and the installer on real Windows PCs.
2. **Updates through the software apps** (on hold): a package repository so Discover and GNOME Software can update Rosorah, and later a Flatpak for Bazaar. The game-capture hooks don't fit Flatpak's sandbox, which still needs a plan.

## Later

- Replay buffer and virtual camera.
- A GNOME-style (Adwaita) title bar when Rosorah runs on GNOME.
- A hotkeys page for scenes, mute and recording (automations can already run from Ctrl+Alt+F1–F12).
- X (Twitter) announcements (on hold): X's API is pay-per-use only, at $0.20 per post with a link (October 2026).
- Signed installers (Windows code signing, signed package repositories).
- A chosen licence and public source, if the project goes open.

## Known limits

- Builds are unsigned development previews.
- YouTube doesn't offer events for ordinary (non-member) subscribers, so those alerts aren't possible.
- Kick activity needs webhooks turned on for the Kick app (see `auth-broker/README.md`).
- TikTok chat is read-only; Rosorah can't post to it.
- Web page panels and browser sources use the browser runtime, which is downloaded the first time it's needed.
