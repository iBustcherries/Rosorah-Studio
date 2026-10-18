# Rosorah Studio 0.16.0 validation

Built from the saved, user-tested 0.15.2 source. Native hooks and installer remain unchanged. No kernel/desktop/boot changes.

## Implemented and checked

- Saved accounts load from the previous path; registration fields removed from normal UI. Stream metadata moved into Broadcast.
- New YouTube broadcast request validation: title, description, privacy, explicit audience, future local date/time (round-trip rejects invalid dates and DST gaps). Creation retains the ID before follow-up binding/category/thumbnail work, to avoid automatic duplicate creation.
- JPEG/PNG thumbnail content/type/size/decode limits; preview; media upload for new/existing broadcast. Stream resource selector/binding is explicit. No automatic encoder start.
- YouTube chat-list polling removed from the account loop. Browser collection uses a separate Qt WebEngine profile and parent-owned JSON pipes (no listening HTTP server). Source changes require Apply, preventing repeated reloads while typing.
- Browser fixture tests: text, emote text, Super Chat display, duplicate suppression, explicit deletion markers, and escaping remote content with DOM textContent.
- Twitch EventSub event mapping, bounded duplicate history, keepalive deadline, connection retry/backoff, reconnect URL host validation, transferred-session handling, and explicit retry. New permissions requested on sign-in. Actual Twitch service reconnection is not exercised locally.
- Per-input positive audio delay tested across block boundaries and parameter reset. Monitor is opt-in and has an independent bounded writer; actual PipeWire headphone playback needs user testing.
- Six-track FFmpeg integration test writes a real MKV and decodes every stereo track, checking channel separation and sample counts. Stream/shared-output mode remains one stereo track.
- Saved workspace tests cover vertical panel close/restore/persistence, duplicate prevention and existing panel arrangements. Native UI screenshots show both canvases simultaneously and enlarged center workspace. GPU targets are keyed by canvas ID and dimensions; source texture uploads remain shared.
- Formatting, strict Clippy, locked release build and workspace tests run in this environment (75 passed; 8 environment-specific tests ignored). Existing HTTP camera tests require proxy variables removed only from the test subprocess; the app's network settings are not changed.
- Packages are CRC/member/hash verified before upload and checked again after download.

## Not claimed as tested

No live account logins, broadcasts, thumbnails, chat messages, subscriptions, or other mutations were sent to Twitch/YouTube from the build environment. Browser engine imports and Python syntax pass, but its sandboxed renderer was not end-to-end exercised in this environment. Live website selectors, Google/Twitch embedded sign-in acceptance, KDE Wayland browser input and long-session recovery remain user tests. Browser collection is labeled experimental. Website updates can break collection; ordinary YouTube subscriber notifications and complete deletion coverage are not supported. Image emotes render as text names.

No physical Fedora AMD GPU, game, camera, or PipeWire output is available here. Keep the working 0.15.2 build for comparison. The new package does not upgrade system packages. Browser runtime setup is explicit and installs only a private Python environment.

## Suggested user check

1. Extract the whole directory, close the old Rosorah, run run-studio.sh. Confirm game/DroidCam still capture.
2. Confirm main/vertical previews side by side, close Vertical Canvas and restore it from Panels. Verify changing preview selection does not change an active output.
3. Sign in to accounts, open Broadcast → Stream details. Create an unlisted test broadcast, select the same YouTube stream key as the encoder, choose a thumbnail and verify the resulting broadcast on YouTube.
4. Chat → Browser chat connections → Set up browser chat (once), then Open browser chat dock. Use platform tabs if sign-in is needed. Confirm messages/events in Combined and in the native Chat panel. Browser activity may be visible while broadcast authorization is disconnected; Twitch EventSub needs the account connection.
5. Record a short clip using the separate-track preset, verify lip-sync and tracks in an editor; enable only the desired microphone monitor through headphones.
