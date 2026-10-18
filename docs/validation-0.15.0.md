# Rosorah Studio 0.15.0 validation

## Included

- V4L2 webcam selection, persistent device paths where available, metadata-node filtering, resolution/FPS controls and visible connection status.
- DroidCam direct Wi-Fi HTTP MJPEG input plus Linux-client virtual webcam input (including USB connections supplied by that client).
- Explicit connect/disconnect, automatic retry after failure, restoration of enabled camera sources, latest-frame bounded transport shared with recording/streaming, source-wide pixel cropping and horizontal flip, and a per-layer match-aspect control.
- Camera sources work with existing resizing, ordering, opacity, visibility and snapping. Capture/output defaults remain 60 FPS.
- Twitch public-client device-code sign-in, session validation/refresh, TLS IRC chat with reconnect/keepalive, category search and title/category updates.
- Google desktop OAuth with PKCE and state-checked loopback callback, refresh, YouTube broadcast selection, live chat polling/sending, and title/video-category updates preserving existing editable snippet fields.
- Unified chat shows both platforms. Twitch message removals/clears and YouTube message deletion events are applied. Sending requires an explicit per-platform button. Local demonstration chat remains separately labeled.
- Account workers do not run network operations on the render/UI thread. Tokens live only in memory; application configuration is separate from projects in an owner-only file. Sign-out/cancel removes the UI session and cancels its worker.

## Verified

- Release build with a locked Cargo dependency file.
- rustfmt and Clippy across all targets with warnings denied.
- 64 ordinary tests passed: 11 core, 4 capture, 49 app. Eight existing environment-dependent tests remain ignored in the ordinary suite.
- New tests cover camera configuration serialization/validation, cropping/flipping and invalid margins, phone URL validation, actual GStreamer camera-style negotiation, OAuth callback path/state/decline handling, token rotation, IRC message parsing/deletion and safe YouTube snippet updates.
- A real local HTTP multipart MJPEG server exercised souphttpsrc → multipartdemux → decodebin → RGBA → crop/flip → capture engine. The test verified decoded dimensions/pixels, the shared recording reader, clearing on disconnect and bounded shutdown. Local test requests ran without the build environment's HTTP proxy. The test exposed and fixed a decodebin startup wait on infinite multipart input.
- Actual native UI opened under Xvfb with a live 1280×720 HTTP test feed on the canvas. Camera settings and account setup screens were visually inspected.
- GPU composition self-check passed on Mesa llvmpipe, including shared textures, layer order, placement, visibility and opacity.
- Existing native hook sources/libraries, game-process audio selection, recording and stream engine code are unchanged. The capture adapter adds camera workers to the same transport.
- Release ZIP and tar.gz are validated by full decompression/member hashes, permissions and executable version, including fresh downloads after saving.

## Remaining external verification

No physical USB camera, DroidCam phone, Fedora hardware GPU or live Twitch/YouTube account is accessible in this build environment. Camera/device-specific modes, USB driver permissions, real phone connections and platform consent/API operations require user testing. The local HTTP test uses a conforming MJPEG test server, not a real DroidCam phone.

This development build has no registered Rosorah OAuth client IDs. Account sign-in is disabled until the application registrations are configured. Follow README Account setup. Live-account sign-in, chat send/receive and title updates have not been end-to-end verified against Twitch/Google. Existing stream-key streaming remains available independently. No stream titles were changed and no chat messages were sent during development.

DroidCam's documented direct Wi-Fi modes are 640×480, 1280×720 and 1920×1080. Legacy /video input uses phone-controlled dimensions. Generic webcam controls also offer 1440p/4K if the device supports them. Unsupported device/FPS modes report failure rather than silently claiming success. Camera audio and direct proprietary USB/OBS-plugin protocols are not included; USB uses the Linux client's virtual webcam. Cropping affects every layer referencing that source; layer size/order remain independent.

Tokens are not persisted, so sign-in is needed again after closing Rosorah. Google API quota/testing-mode limits apply. This build polls YouTube chat at the server's permitted interval with a minimum 5-second interval and pauses on terminal chat/quota errors. It does not implement account-based stream-key retrieval, broadcast creation/start/end, chat emotes/badges or moderator action buttons.

## Upgrade

Close Rosorah, extract the complete archive into a fresh folder and run run-studio.sh. From 0.13/0.14, Steam, games and KDE can stay open because the native hooks/protocol are unchanged. Existing projects retain their settings; adding a camera introduces a source type older builds do not understand.
