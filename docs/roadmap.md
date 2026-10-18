# Delivery sequence

All milestones use original Rosorah Studio code and independent upstream libraries.
Full OBS feature parity is a long-term objective, not the scope of these development builds.

| Stage | Deliverable | Completion evidence |
| --- | --- | --- |
| 0 — foundation | Native workspace, scene model, shared-source compositor, recording proof, audio/chat/rule cores | Unit checks, actual rendered pixels, horizontal/vertical MP4 verification, UI screenshots |
| 1 — Linux capture | Portal/PipeWire desktop/window capture and application audio; practical game selection | Fedora KDE Wayland test, permission/reconnect handling, automatic selection tests, game/audio pairing |
| 2 — live media | GPU frame import, audio clock/resampling, muxing, hardware encoders, RTMPS output | Sustained real game stream with A/V sync, multiple audio tracks, reconnect behavior and measured resources |
| 3 — connected studio | Explicit account sign-in, platform metadata, native multichat, events and moderation | Real accounts/scopes, token refresh/keyring, rate limits, per-platform send/action outcomes |
| 4 — browser and alerts | Independent CEF browser adapter, embedded docks, on-output alerts, sound/media actions | Native Wayland resizing/input/HiDPI tests, popup/process recovery, correct recorded overlays |
| 5 — full workflows | Transitions, hotkeys, media/camera inputs, monitoring, replay, virtual camera, separate native panel windows, extensibility | Device matrix, long-stream soak tests, output/canvas changes during streaming, recovery checks |
| 6 — distribution | Fedora packages, installer/update flow and release process | Install/upgrade/uninstall tests, dependency notices, chosen project license and signed artifacts |

Docking, tab grouping, in-application floating panels and saved layouts shipped
in the 0.2 workspace update. Separate native windows remain future work.

## Current 0.16.0 update

Automatic camera modes were user-tested in 0.15.2. The 0.16.0 test build adds broadcast creation/thumbnail controls, a Qt WebEngine companion combined chat window, pushed Twitch events, dual canvas previews, positive audio sync delays, monitoring and standalone multitrack MKV. Browser collection and live account/event workflows need user verification; ordinary YouTube subscriber alerts remain unavailable. See validation-0.16.0.md for the actual test boundary.

## Earlier milestone notes (historical)

Automatic game capture/audio, recording, and AMD hardware recording are user-confirmed working through 0.11.0. 0.12.0 adds standard Twitch/YouTube streaming using stream keys, per-destination status and reconnect, a shared encoded MKV copy, and 60 FPS defaults for capture and new canvases.

Local two-destination RTMP delivery, forced disconnect/reconnect, shared recording, A/V timing, cancellation and RTMPS certificate handling are tested. A real Twitch/YouTube broadcast and physical-GPU streaming performance remain user validation. Stage 2 remains in progress: zero-copy frame import, multitrack video recording and long-session performance measurement remain future work. Standalone OpenGL emulator activation remains deferred until requested.

The next connected-studio milestone is explicit account sign-in and broadcast metadata editing, followed by live chat/activity. OAuth registrations and credentials must be supplied before real account integration; this release does not launch sign-in automatically.

## Open decisions that do not block the current foundation

- Public/open-source versus proprietary distribution license.
- OAuth application registrations and service-specific API permissions.
- Actual TikTok/Kick feature availability for the user's account and approved APIs.
- CEF version/build distribution and the texture-sharing approach after a prototype.
- A measured resource budget on the user's AMD GPU and i7-13700K system.

## Important current limits

Desktop/renderer capture, application audio, the mixer, WAV tests, selected-canvas recording and standard RTMP/RTMPS streaming are implemented. Stream destinations share one H.264/AAC profile; different encodes per platform are future work. Keys are saved in a separate private local file, not a keyring. Camera capture, CEF, online chat, account sign-in, moderation and output notifications remain future work. Test messages/events do not contact services.
