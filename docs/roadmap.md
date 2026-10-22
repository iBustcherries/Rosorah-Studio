# Rosorah Studio roadmap

Updated for 0.48.0 (October 2026). Rosorah Studio is original code built on
independent libraries (Rust, egui/wgpu, GStreamer, Qt WebEngine). It aims to be an easier
all-in-one alternative to running OBS, a chat bot and an alert service side by side,
not a feature-for-feature copy of OBS.

## Where things stand

| Area | Status | What's there |
| --- | --- | --- |
| Studio and layout | Shipped | GNOME-style title bar on GNOME (other desktops keep their own), Simple and Advanced modes, first-run guide, docking with tab groups, floating panels and separate native windows, saved layouts, web page panels, themes and backgrounds, undo/redo |
| Scenes and layers | Shipped | An Add dialog that also reuses layers from other scenes with live previews, scenes named in place, drag to reorder with sliding rows (also the mixer and automation steps), square/rounded/circle layer frames with faded edges, a green-screen key (experimental), main and vertical videos with linked scenes, crop/align/group tools, text layers with 68 bundled font families, widgets (chat, music, goals, utilities), scenes inside scenes, image slideshows and video playlists, copy and paste layers between scenes and videos, right-click menus on the canvas |
| Layer effects | Shipped | Rotate and flip, blend modes, colour correction, LUTs, sharpen, scrolling, image masks, luma key, per-layer delay, show/hide animations |
| Studio Mode and projectors | Shipped | Prepare a scene before sending it live, per-scene transitions (slide, swipe, pattern wipe, fade to colour), projector windows for the program, a scene or Multiview |
| Capture (Linux) | Shipped | Automatic game capture (Vulkan/OpenGL hooks) with game audio, PipeWire desktop/window capture, cameras and DroidCam, browser sources (web addresses or local HTML files, custom CSS, OBS overlay API, saved sign-ins, keep running or refresh on show) |
| Capture (Windows) | Shipped | Automatic game capture (finds the game's window and captures it with Windows Graphics Capture), monitor and window capture, application audio and cursor |
| Sound | Shipped | Music strip that follows music apps automatically (never browser video), per-app sound strips that survive app restarts, right-click rename/hide/delete, mixer with per-scene volumes, Sound settings window (device, live level, noise suppression, gate, compressor, limiter, expander, EQ, mono, balance), monitoring, delay, six recording tracks |
| Recording and streaming | Shipped | Hardware encoders (AMD/NVIDIA, software fallback), multistreaming to Twitch, YouTube, YouTube vertical, Kick, TikTok and custom RTMP, per-destination bitrate, reconnect, Go live checklist, replay buffer (saves the last seconds as an MP4) |
| Virtual camera | Shipped | Rosorah's video as a webcam for Discord, Zoom and browsers (Linux: v4l2loopback; Windows 11: Rosorah's own camera component) |
| NDI and Spout | Shipped | NDI sources (with sound) and an NDI output on Windows and Linux (GStreamer's NDI plugin; Linux packages ship it, users install the free NDI runtime). Spout 2 sources and output on Windows; on Linux, Spout to and from Windows apps in Wine/Proton through rosorah-spout-bridge.exe run in the app's own Wine session (needs DXVK, which Proton, Lutris and Bottles include). Under Add → Capture devices and in the Stream panel |
| Hotkeys, remote control and scripts | Shipped | System-wide hotkeys for scenes, mute, push-to-talk and outputs; remote control for Stream Deck, Touch Portal and phone apps (Rosorah's protocol, plus OBS's for tools made for OBS); sandboxed Lua scripts |
| Accounts | Shipped | Twitch, YouTube and Kick sign-in with renewable tokens in the system keyring, titles/categories, automatic stream keys |
| Chat and activity | Shipped | Unified Twitch/YouTube/Kick/TikTok chat with moderation, colour emoji (Twemoji), an emoji and emote picker with your own Twitch, YouTube and Kick emotes, emotes and emoji as pictures in the on-canvas chat widget, which also shows profile pictures, badges, name colours and highlighted paid and first-time messages, sliding and fading in and out, leaving out bots and chatters you hide (right-click a message), Activity history for every connected account |
| Alerts and automations | Shipped | One list grouped by event (follows, subs, bits & donations, raids, channel points, chat, timers), chat timers, bot accounts on Twitch and Kick that send automation chat, guided alert setup, themed alert cards with the event, the viewer's picture and an entrance animation, one shared alert box plus optional boxes for chosen alerts, paid messages read aloud with a free offline voice (Piper), built-in sounds and animations, triggers from follows, subs, gifts, bits/Kicks, raids, redemptions, donations and chat; Twitch rewards editor |
| Donations | Shipped | StreamElements, Streamlabs and Ko-fi (Settings → Accounts → Donations), donation goals |
| Transitions | Shipped | Cut, fade, wipe, Move (shared layers glide to their new place), stinger videos and transition screens |
| Projects | Shipped | Profiles, hourly and on-close backups, recording recovery, OBS and Firebot import |
| Announcements | Shipped | "I'm live" posts to Discord (webhook), Bluesky (app password) and Threads, once per stream, with a message template and per-stream skip |
| Distribution | Shipped | Windows installer, Fedora .rpm and Ubuntu/Debian .deb on GitHub Releases; in-app updates (checks at start and every few hours, Help > Check for updates, a window or a bottom-bar reminder, one-click install on Windows and Linux) |

## Next

Nothing is scheduled right now. Rosorah updates itself from GitHub Releases, so it isn't listed in Discover or GNOME Software.

## Later

- X (Twitter) announcements (on hold): X's API is pay-per-use only, at $0.20 per post with a link (October 2026).
- Signed installers (Windows code signing).

## Known limits

- Builds are unsigned.
- Rosorah is licensed under the GNU GPL version 3; bundled components keep their own licences.
- YouTube doesn't offer events for ordinary (non-member) subscribers, so those alerts aren't possible.
- Kick activity needs webhooks turned on for the Kick app (see `auth-broker/README.md`).
- TikTok chat is read-only; Rosorah can't post to it.
- Web page panels and browser sources use the browser runtime, which is downloaded the first time it's needed.
