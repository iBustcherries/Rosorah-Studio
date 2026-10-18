# Rosorah Studio 0.18.0-preview.1

Linux preview build — local alerts, native widgets and donation setup preparation.

Extract this entire folder, close the previous Rosorah instance, and run `run-studio.sh`.
The launcher and game capture hooks are unchanged from 0.17.1. No driver or kernel changes are needed for this update.

Back up your project before trying this preview. New widgets/actions are not readable by older builds.

- **Alerts & automation:** live event rules; word/user/reward filters; media and widget actions.
- **Studio → Add layer → Widget:** chat text, goals, countdowns, counters and shoutouts.
- **Settings → Donations & PayPal setup:** a resumable, guided preparation wizard.

**Live donation connections are not implemented yet.** No hosted service is deployed. The wizard cannot currently connect PayPal, StreamElements, Streamlabs or Ko-fi, and donation goals are manual. This is not the complete alerts release.

Read `docs/alerts-preview.md` for setup, supported behavior and the remaining work.
Read `docs/validation-0.18.0-preview.1.md` for the checks performed.

Existing streaming/capture setup and runtime dependency guidance follow:

# Rosorah Studio 0.17.1 — efficiency update

Extract this complete folder and run `./run-studio.sh` from **this folder**. The app version is 0.17.1. Old desktop shortcuts may still launch an earlier copy. This build includes the 0.16.6 account-wallet crash fix; remembered accounts and settings retain their existing locations.

## New in 0.17.1

All existing features and the 60 FPS capture default are retained. Resolution, frame rate, audio processing, chat delivery, alerts and transitions keep their existing controls and behavior.

- Static images and colors stay on the GPU across unrelated edits. Replacing an image file is detected on the next project/preview refresh using its modification time and size; reopening the project also reloads it.
- Each canvas reuses its drawing buffer. Unchanged layer geometry needs no additional vertex upload; transitions and layer edits update immediately.
- Resizing a preview releases its old compositor target instead of accumulating multiple full-size surfaces.
- Recording and streaming upload live frames only for sources visible on that output's scene. Revealing a source picks up its latest available frame immediately.
- Audio workers share immutable channel settings, and each output caches its own scene mix until the project changes. Scene volumes, mute, media visibility, monitoring and noise suppression remain supported.
- Unchanged chat history is no longer rescanned on every UI frame. Incoming messages/events continue through the existing immediate delivery path; no polling interval was increased.

See `docs/validation-0.17.1.md` for checks and the focused performance comparison. Savings depend on scene complexity and which outputs are running; the comparison is not a whole-app CPU/GPU percentage.

## New in 0.17.0

- **Activity** sits next to Chat and is available in Panels. One newest-first timeline follows the connected Twitch and YouTube accounts, with no platform tabs or extra sign-in. It keeps up to 5,000 received events from the last 30 days locally, separated by account identity and deduplicated across restarts. It does not backfill every event while the app was closed.
- Twitch activity includes follows, subscriptions/resubscriptions, gifted subscriptions, bits, raids, channel-point redemptions, Hype Trains, polls and predictions through EventSub. YouTube Super Chats, Super Stickers, memberships and gifts come from the active broadcast's browser chat. Ordinary YouTube subscriber entries are public subscribers only: one 1-unit request every five minutes with a selected broadcast, or thirty minutes without one. The latest fifty available records are checked; bursts beyond that window can be missed. Errors back off for an hour. No live-chat API polling is added.
- **Layers → + Add layer → Video file** imports MP4, WebM, MOV and other formats supported by the installed GStreamer codecs. Files remain at their chosen location. Video loops by default, fits its detected proportions, and is decoded once for both canvases and all outputs. Select the layer to enable sound, stop/restart, or choose **Play as an event alert** and its platform/event. Alerts play once and disappear at the end, queue up to sixteen occurrences per source, and only fire in a visible scene. Incoming events restored from saved history or initial page history do not replay alerts. The first five seconds of a newly loaded chat page are treated as history to avoid replaying old events. Video sound appears as its own mixer channel when enabled; it is off initially for backgrounds. Transparent video requires a format and installed decoder that preserve alpha.
- **Scenes → Scene transition** offers Instant, Fade and Wipe with an adjustable 100–2,000 ms duration. Preview, stream and recording all use the same GPU transition implementation. Each canvas transitions independently, using a snapshot of the outgoing composition.
- **Audio → Options → Add Desktop audio** adds an opt-in desktop-output input. Choose the speakers/headphones whose mix you want to capture. It includes all sound on that output; avoid duplicating game sound by muting the separate game channel when appropriate.
- Mixer **volume and mute are saved per scene**. Select a canvas and scene before adjusting them. Main and vertical streams use their own scene's mix even when both run together. Input choice, track routing, delay, monitoring and noise-suppression settings remain shared per audio channel. A video file's sound is silent on canvases where its layer is absent or hidden.
- Under **Microphone noise suppression**, the microphone has **Off, Gentle and Strong** options. Gentle reduces steady fan/hiss; Strong also reduces low-level room noise between speech. Processing uses FFmpeg and adds a small buffer. It is not speech isolation and cannot guarantee removal of keyboard clicks or nearby voices. Start with Gentle and check your microphone level. Noise suppression applies before scene volume, recording, streaming and monitoring.
- Broadcast keeps Start/Stop and destination choices visible. Title/category, YouTube broadcast editing, recording details and diagnostics are expandable. Layers uses readable Lock/Unlock controls, a clearly labeled Add layer menu, and collapsed numeric position/size controls. Existing panel arrangements are retained; the Activity tab is added once and stays hidden if you close it.

Video playback requires GStreamer playback/codec plugins. Audio capture uses PipeWire utilities; noise suppression, recording and streaming use FFmpeg. No kernel or graphics-driver changes are needed for these features.

Connect Twitch and YouTube in **Settings → Accounts**. The same account is used for broadcast details, outgoing chat, activity, and automatic stream-key setup. An empty Chat panel has a **Connect accounts** shortcut to that one settings page. The normal panel is messages, a composer, and a small overflow menu.

## What happens automatically

- Chat starts in the background after connecting. The private PySide6 6.10.2 browser runtime is checked and, if missing, downloaded automatically (about 250 MB). An existing working runtime is reused. Failed setup retries with backoff. No separate browser-window or setup button is needed in normal use.
- Renewable account credentials are stored using the desktop Secret Service through libsecret (KWallet or GNOME Keyring). Opening Rosorah restores remembered accounts and renews their tokens. Transient connection failures retry automatically. The desktop wallet must be available and unlocked; otherwise the account works for this session and Accounts explains why it cannot be remembered. Expired/revoked permissions still require reconnecting.
- Twitch chat follows your connected channel. YouTube finds the sole active broadcast, switches away from an old scheduled selection, and stops following an ended broadcast. It checks broadcast metadata every 60 seconds while active or 120 seconds otherwise. Starting Rosorah's encoder triggers a short faster discovery period. When more than one broadcast is live, select the intended one by title in **Broadcast → Stream details**; Rosorah preserves that valid selection. Lists cover up to 50 broadcasts.
- Twitch's stream key and a selected YouTube broadcast's linked key populate the primary destinations automatically. Existing canvas assignments and enabled destinations remain your choices. YouTube Vertical still needs its separate key for dual-format output. A running encoder keeps its current route; newly discovered settings apply after streaming stops. Signing out removes automatically supplied keys from future settings, but does not stop an already running encoder or erase manually supplied keys.
- When you explicitly create a YouTube broadcast, the default **Automatic stream key** option creates and links a key, then follows that broadcast's chat. You can still choose an existing key. Creation does not start streaming; the optional go-live-on-encoder-start control remains off by default. A partially completed creation retains its ID and is never retried automatically.

## Chat and sending

Type into the native Chat panel or the combined browser dock. Both send through the central Rosorah account, so ordinary sending does not require a second website login. Twitch accepts up to 500 Unicode characters and YouTube up to 200. Messages are sent only after an explicit Send/Enter action, and success requires a platform acknowledgement. A failed or unanswered send retains the draft and is never automatically repeated; check the chat before resending after a timeout.

Reading remains browser-based. There is **no YouTube liveChatMessages.list polling**. Automatic discovery uses occasional broadcast metadata requests, with exponential backoff and a one-hour pause for reported quota exhaustion. Sending a YouTube message uses the official insert endpoint and currently costs 20 quota units per message. Broadcast creation, edits, thumbnails, and key binding also use the API. A publisher distributing to many users must budget the shared application's quota accordingly.

Incoming Unicode emoji, platform emote images, animations, and Twitch/YouTube chat-visible activity are retained. Recent emotes and an emoji shortcut remain in the composer. The browser collector pushes messages as the platform page receives them; platform/network delays still apply. A private/restricted YouTube chat may require its separate website session to **read** it. OAuth does not transfer website cookies. Full platform emote pickers remain in the optional browser platform tabs; API text sending cannot promise insertion of every account-specific YouTube custom emote. No 7TV/BTTV/FFZ extension is installed.

The browser window can stay closed. Page/process failures retry automatically. Detailed recovery controls are under **Settings → Accounts → Troubleshooting**. First connection needs Python 3 with venv/pip, internet access, and normal Qt/Chromium desktop libraries. No kernel or graphics-driver changes are made.

## Upgrading and first-time distribution

Extract the complete folder and run `./run-studio.sh`. Your projects, camera settings, canvas destinations, browser profile, and existing `accounts.json` remain in their current user directories. Connect once in this build to establish remembered sessions; Twitch will request chat-send and stream-key permissions in addition to its existing activity permissions. Refresh tokens are not written to projects or plaintext configuration. The small remembered-account marker contains only provider/client identifiers.

This package supports the user's existing application registrations. It does **not** contain the publisher's Twitch/Google registration values. A publisher-ready release must include `accounts-defaults.json` beside the executable, using the schema in `accounts-defaults.example.json` (under `packaging/` in the source archive), with a public Twitch device-flow client and the Google Desktop-app OAuth registration. Never bundle user access/refresh tokens or a confidential Twitch application secret. Google publishing/test-user status, API enablement, and quota must allow the intended users. The user's per-provider `accounts.json` overrides the defaults. Without either configuration, Accounts reports that a publisher-configured build is needed; new users are not asked to configure developer credentials in the chat panel.

Automatic capture defaults to **60 FPS**, independently of the game's render rate. Existing saved canvases retain their FPS. Capture, camera aspect handling, layers/snapping, per-canvas output, game audio, recording, and the native Vulkan/OpenGL hooks are retained.

## Resize and camera fixes (0.16.2)

Main and Vertical previews now retain their own resize interaction identity. Drawing the other canvas no longer cancels a drag, and source selection is retained after clicks and drags on the active preview. The resize hit area extends 10 screen pixels beyond the canvas image, within its panel, so corner handles at the canvas edge are easier to grab. Snapping, proportional resizing, locked layers and the existing Ctrl snap bypass remain supported.

Camera settings now default to **Match camera aspect ratio automatically**. Once the camera supplies frames, Rosorah uses their actual dimensions (including crop) to adjust the camera layer height while preserving its position and width. It follows input format changes, including 4:3, 16:9 and portrait DroidCam feeds, and respects locked layers. It does not continually override manual resizing when the input size is unchanged. Uncheck the option and Apply camera settings for an intentionally stretched layout. This default also applies when older camera settings do not contain the new option. Scenes still share their layout wherever reused; use separate scenes for independently arranged Main and Vertical compositions.

Twitch output remains standard RTMP/RTMPS with one chosen canvas. Enhanced Broadcasting / Twitch Dual Format is not implemented by this update.

## Canvas destinations (0.16.1)

In **Broadcast**, enable Twitch, YouTube Main, and/or YouTube Vertical and choose a canvas beneath each destination. Stream keys remain in **Streaming settings**. Canvas assignments use stable canvas IDs and are saved with streaming settings. Existing destinations default to Main; YouTube Vertical defaults to the first portrait canvas. If a saved canvas is missing, start is blocked until you choose another. Preview clicks do not alter routing. Stop streaming before editing routes.

For Twitch Main + YouTube Main + YouTube Vertical, use two canvases and three destinations. Main destinations share one encoder; Vertical uses a second encoder. Each destination consumes upload bandwidth. All outputs use Track 1, with volume and mute taken from the scene assigned to that canvas. Record while streaming writes one MKV per active canvas. The standalone Record button continues to use the selected preview canvas.

For one YouTube broadcast in two formats, open YouTube Studio → Live Control Room → Stream, set the vertical preview from Auto to Encoder, and choose a **second stream key** for the vertical output. Enter that key under YouTube Vertical, and the horizontal key under YouTube Main. Configure this before going live. Rosorah rejects duplicate YouTube keys. YouTube's dual-format broadcast has one shared chat, so the automatic chat source remains the selected broadcast. Rosorah does not set YouTube's dual-stream layout option through the API. Platform instructions: https://support.google.com/youtube/answer/2474026

Chat follows the central account and automatically discovered/selected broadcast as described above. YouTube dual-format output has one shared chat. Independent simultaneous broadcasts require choosing the intended chat by title.

## New in this build

### Automatic camera mode (0.15.2)

**Webcam / USB → Auto detect resolution and FPS** is on by default, including camera sources saved by older builds. Select **DroidCam Virtual Camera**, then connect. Rosorah reads its current output instead of imposing the saved manual resolution/FPS. The live status shows the negotiated input size and device-reported frame rate, for example **Auto detected: 1920 × 1080 · 60 FPS**. This is the advertised input cadence, not a measured performance counter. If the source supplies no frame rate, Rosorah says so.

Change DroidCam's quality in its desktop client. Rosorah checks an active loopback device once per second and reopens capture when its reported output mode changes. On device failure or disconnect it uses the existing automatic retry. A client may require stopping/restarting its own virtual camera to publish a changed mode. A loopback device without a producer reports that it is waiting for video. New modes must be within 16–4096 pixels in each dimension.

With Auto enabled, manual resolution/FPS controls are hidden. Uncheck Auto to use them, then click **Apply camera settings**. Saved manual choices are retained for this override. Ordinary webcams negotiate an available raw-video or MJPEG mode; automatic following of a producer's mode specifically applies to V4L2 loopback devices. This change does not change the direct Wi-Fi URL/resolution controls or network frame cap.

Input settings do not change game capture, canvas dimensions or the recording/streaming frame rate. A 30 FPS camera can appear in a 60 FPS output; a 1080p camera retains that input detail before layer composition. Cropping continues to operate on the negotiated source pixels, and match-aspect uses the cropped frame.

No kernel, driver packages, boot configuration or desktop settings are changed by this release.

### Camera hotfix (0.15.1)

- Direct Wi-Fi input detects the incoming format instead of forcing every HTTP stream through a multipart demuxer. Live multipart MJPEG and raw JPEG input are covered by local integration tests.
- Known V4L2 loopback devices remain selectable before their video producer starts. Metadata-only camera nodes remain excluded.
- Camera failures identify the input type and failing component, with separate USB and Wi-Fi guidance.

For DroidCam through its Linux client, the `v4l2loopback` kernel module must be loaded and the client must publish to its virtual camera. Seeing video in the DroidCam client alone does not establish that a Linux camera device exists. If the module is installed but unloaded, run `sudo modprobe v4l2loopback exclusive_caps=1 card_label="DroidCam"`, reconnect the client and refresh **Webcam / USB** devices. If loading fails, retain the exact error to diagnose the module installation. Rosorah does not install or load kernel modules.

The Wi-Fi changes are verified against local test streams; a real DroidCam phone has not been tested here.

### Camera and DroidCam

1. In Layers, choose **Add source → Camera / DroidCam device** for a USB webcam or a DroidCam virtual webcam provided by its Linux desktop client. For a direct phone connection choose **DroidCam over Wi-Fi**.
2. Select the new layer. **Camera settings** appears above Size & position. Choose a device, or enter the Wi-Fi IP and port shown by the DroidCam phone app. Connect the phone to the same network as the computer. No OBS plugin is required.
3. Leave **Auto detect resolution and FPS** enabled for Webcam / USB, or choose a supported manual mode, then **Connect camera**. The canvas is the live preview and the camera is included in recordings and streams. Game capture defaults to 60 FPS; automatic camera input follows its own negotiated rate.
4. **Flip horizontally**, crop margins, and resolution/FPS changes take effect with **Apply camera settings**. **Match camera aspect ratio** adjusts the selected unlocked layer to the actual cropped video. Existing drag/resize, layer order, opacity, visibility and snapping work normally.
5. **Disconnect** releases the camera. Enabled camera sources reopen when their project opens, and reconnect after a device/network failure. Removing the last layer referencing a camera stops it. Hidden layers may keep their camera connected; use Disconnect to release it.

DroidCam direct Wi-Fi uses the officially documented HTTP video endpoint `/video/WIDTHxHEIGHT`. The documented modes are 640×480, 1280×720 and 1920×1080. **Use legacy DroidCam /video address** supports the older endpoint, whose input resolution is controlled by the phone. The FPS control caps network frames; the phone determines its own capture rate. Direct USB phone discovery and the proprietary DroidCam OBS plugin are not included: USB works through a DroidCam virtual webcam supplied by the Linux client. Camera capture is video-only; existing game audio continues unchanged.

On Fedora, camera/video decoding needs `gstreamer1-plugins-base` and `gstreamer1-plugins-good`. If missing, install them with `sudo dnf install gstreamer1-plugins-base gstreamer1-plugins-good`. Physical cameras need permission to access their `/dev/video*` device. Refresh cameras after plugging in a new device. Unsupported manual resolution/FPS modes report an error; Rosorah does not silently claim the requested mode worked.

### Broadcast controls and activity

**Broadcast → Stream details** contains Twitch title/category editing and YouTube broadcast selection, creation and thumbnail upload. Provide a title, description, visibility, category, optional local schedule, and explicit audience choice for new broadcasts. JPEG/PNG thumbnails are validated and limited to 2 MiB. Existing descriptions/tags are preserved when editing title/category. Refresh is automatic; account connection lives only in Settings → Accounts.

Twitch activity uses EventSub WebSocket, with REST calls to establish subscriptions. Supported events include follows, subscriptions, resubs, gifted subs, Bits, incoming raids, custom channel-point redemptions, Hype Trains, polls and predictions. YouTube browser events include chat-visible Super Chats, Super Stickers, memberships, and gift membership announcements. Ordinary/private YouTube subscriber alerts and external donation services are not included. Platform permissions and markup may limit availability.

Browser collection remains experimental and depends on website markup. It runs in a Qt WebEngine companion, mirrored into the native dock. The current build's automated/local validation is listed in `docs/validation-0.17.1.md`; signed-in provider sessions and Fedora KWallet persistence require testing on the target machine.

### Audio sync, monitoring and recording tracks (0.16.0)

Open **Audio mixer → Advanced audio** to choose a headphone monitoring output and enable separate MKV tracks. Each source's **Advanced audio & tracks** has a 0–2000 ms positive audio delay, a monitor checkbox and track routing. Delay affects both streaming and recording; use it to align a microphone with a delayed camera. There is no automatic lip-sync calibration or negative/video delay in this build. Monitoring is off until explicitly configured. Use headphones; feeding a captured desktop mix back into its own output can produce feedback.

**Separate game, microphone and music tracks** enables six-track recording and assigns Track 1 to the combined stream mix, Track 2 to game sound, Track 3 to the microphone, and Track 4 to other channels. Tracks 5–6 remain available for custom routing. Existing projects keep their current routing until you choose the preset. Standalone MKV recordings store six independent stereo tracks when enabled. RTMP and the optional shared stream recording continue to contain only Track 1; a standalone multitrack recording may run alongside streaming using a separate encoder. Missing inputs are silent. Monitoring uses bounded queues and never blocks the mixer/encoder.

### Larger main and vertical canvas panels (0.16.0)

The center workspace displays **Main Canvas** and **Vertical Canvas** side by side. Close Vertical Canvas using its tab's × and reopen it with **Panels → Vertical Canvas**. Visibility and placement are saved. Version-1 layouts gain the vertical pane once; default-sized splits are enlarged. Dividers remain draggable. The 960-pixel preview width cap and extra top padding are removed; source aspect ratios are preserved. Click a preview to choose which canvas the layer editor controls. An already-running recording/stream retains its chosen output canvas. Source textures are shared between previews, with separate canvas render targets.


## Existing scene editing features

- **Snapping** is enabled below the preview. Sources snap to canvas edges/center and the edges/centers of other visible layers while moving or resizing. Cyan guides show the active alignment and are never included in recordings or streams.
- Hold **Ctrl** to bypass snapping temporarily. **Snap settings** lets you toggle canvas edges, canvas center and other layers, or choose a 2–24 px snap distance (8 px by default). Distance is measured in the preview, keeping the feel consistent across canvas sizes. Settings save with the project; old projects get sensible defaults.
- **Layers** replaces the Sources panel title. The top row is the front layer. Select a row and use **Move up**, **Move down**, or **Order → Bring to front / Send to back**. The selected layer stays selected after reordering.
- Each layer retains independent position, size, opacity, visibility and lock state. Drag it in the preview or edit pixel dimensions in **Layers → Size & position**. Locked layers cannot be moved, resized or reordered, but can act as snap targets. Hidden or fully transparent layers do not attract snaps. Proportional resizing keeps the opposite corner fixed.
- The layer list and adjustment controls appear first. Capture options are under **Layers → Game capture**, including the saved 720p/1080p/1440p/4K/original-size choice. Default capture remains **1080p at 60 FPS**. Recording dimensions remain independent in **Settings → Canvases & recording**.

**Upgrade from 0.13.0:** close Rosorah, extract the complete archive into a fresh folder and run `run-studio.sh`. Your games, Steam and KDE can stay open. The six native capture libraries, launch observer and installer remain unchanged; recording and streaming have since gained the features described above. The capture transport remains protocol 3. If upgrading directly from 0.12.0 or earlier, restart an already-running game once to load protocol-3 hooks.

## Stream a game

1. Close the old Rosorah build, extract the complete archive into a fresh folder, and run `run-studio.sh`.
2. Open **Settings → Streaming services** (or **Broadcast → Streaming settings**). Enable Twitch, YouTube Main, and/or YouTube Vertical, choose their canvases, and paste the matching stream keys. Dashboard links are provided. Click **Save streaming settings**.
3. For YouTube, create or select a stream in Live Control Room and use its key. The default server uses RTMPS; change **Server address** if your dashboard supplies another address. Twitch defaults to its global RTMP ingest endpoint.
4. Check each destination’s canvas assignment in Broadcast, the captured game, and Game audio meter. Enable **Record while streaming** for one MKV per streamed canvas, using its streaming encoder.
5. Click **Start streaming**. Each enabled destination changes from Preparing/Connecting to **Sending** when FFmpeg reports media delivery. Open the platform dashboard to check visibility and stream health. YouTube may require its own Go live action depending on your auto-start setting.
6. Click **Stop streaming**. Rosorah stops network output and finishes the shared recording. **Open recording** opens the saved MKV. Closing Rosorah also stops the outputs and waits for recording finalization.

The session timer measures time since encoder startup. Each destination timer measures delivered media for its current connection and resets after reconnecting. A disconnect retries automatically with bounded backoff, discards outdated network frames, and restarts on a fresh video keyframe. A local recording write failure is reported separately while healthy network outputs continue. Files are never overwritten. A separate recording started before streaming keeps its own encoder; the stream does not create a duplicate recording. If you start a standalone recording during a stream without a shared recording, it uses its own encoder and chosen canvas.

## Streaming settings and limits

- Each assigned canvas supports up to 1920×1080 (or 1080×1920) at 60 FPS, H.264 + 48 kHz stereo AAC, Track 1 mixer gain/mute, CBR target bitrate, and a two-second keyframe interval. This is standard RTMP/RTMPS output, not Twitch Enhanced Broadcasting.
- Default video bitrate is 6,000 kbps, shared by all stream destinations. Twitch-enabled sessions allow 1,000–6,000 kbps; YouTube-only sessions allow up to 20,000 kbps. Each destination needs its own full upload bandwidth. Audio adds 160 kbps per destination.
- **Settings → Canvases & recording → Output encoder** selects Auto (prefer GPU) or Software. Streaming checks the actual H.264 CBR pipeline at the selected size/FPS; it can fall back to `libx264`. Streaming requires AAC and a working H.264 encoder; recording's MPEG-4/VP8/FLAC fallbacks cannot be sent to these destinations. Recording retains its existing quality-oriented encoder settings.
- RTMPS certificate verification is enabled. Network setup/write stalls are bounded and cancellation can stop a pending connection. Internet failures do not accumulate unlimited memory or stop capture.
- Keys are masked in the UI and saved in `~/.config/rosorah-studio/streaming.json` (or under `XDG_CONFIG_HOME`), separate from exported projects, in a mode-0600 file. They are not encrypted or stored in the desktop keyring yet. Error messages redact configured stream keys/URLs. FFmpeg receives the destination as an argument, so the same local user can inspect it in process information.
- Streaming begins only after your Start action. Account connections and metadata controls are in Accounts/Broadcast; browser chat is described above. Full moderation and YouTube dual-stream layout configuration remain platform-dashboard tasks.

## Record a game

1. Close the old Rosorah build, extract the complete archive into a fresh folder, and run `run-studio.sh`.
2. Keep **Include game audio automatically** checked. Check that the game is visible and the **Game audio** meter responds.
3. Select the canvas you want to save and click **Start recording** in **Broadcast**. Wait for the timer to start. Recording continues in a background worker while the preview is hidden or Rosorah is minimized.
4. Click **Stop recording**, wait for saving to finish, and click **Open recording**. A normal application close also waits for the recorder to finish.

The selected canvas stays fixed for that recording, even if you preview another canvas. Scene/layer changes on the recorded canvas are included. Size and FPS are locked until Stop. Track 1 includes every channel routed to it, with its current gain and mute. Microphone and Music still require selecting their inputs; they are not enabled automatically. The game continues playing through your normal speakers/headphones without Rosorah replaying it unless you explicitly enable monitoring.

## Recording requirements and limits

Recording needs the `ffmpeg` command. If it is missing on Fedora, install `ffmpeg-free`. Hardware encoding also requires an accessible GPU and an installed driver that exposes H.264 encoding; the encoder being listed by FFmpeg is not enough. Rosorah checks a short actual video/audio encode before creating your recording. Some Fedora FFmpeg or graphics-driver packages omit codec support. If that applies, **Recording details** contains the driver/encoder error and Rosorah uses software. No packages or driver settings are changed automatically.

**Settings → Canvases & recording → Output encoder** has two choices:

- **Auto (prefer GPU)** — the default, including for older saved projects. Checks VAAPI render devices (including the alternate low-power entrypoint) and NVIDIA NVENC, then tries software H.264 (`libx264`), MPEG-4 and VP8 as available.
- **Software** — bypasses GPU checks and uses a tested software encoder.

The selected setting is saved with the project. Recording uses an MKV container and 48 kHz stereo AAC sound (FLAC if AAC is unavailable). The active encoder stays fixed for the recording. If a driver fails after recording starts, Rosorah reports the error and keeps the partial MKV; it does not silently change codecs in the middle of a file. Select Software before retrying if a GPU driver repeatedly fails during a session.

Video rendering and encoding run separately from the UI and game capture. Input frames are shared in bounded memory, and the preview and recorder consume them independently. Both recording streams use the same monotonic timeline. A slow render repeats the last completed frame; missing audio fills with silence. A persistently stalled encoder stops with an error instead of making the game wait or building an unlimited queue. Existing output files are never overwritten. An interrupted or failed recording may leave a partial MKV; Rosorah reports its path rather than claiming it saved successfully.

Capture defaults to 1080p at 60 FPS. Choose a higher capture resolution to retain more source detail before canvas composition. The 30 FPS capture option remains available. Recording FPS never limits the game's FPS. Hardware encoding moves video compression to the GPU encoder. Capture transport, canvas readback, pixel conversion, audio and file writing still use resources; this is not a zero-copy pipeline. Actual game performance on the user's AMD hardware has not been measured here.

This release records one selected canvas with one stereo mix or six independently routed stereo tracks. Shared stream recordings remain one stereo mix. Preview-only alert banners are not included. HDR output is not implemented.

## Automatic game sound

Game audio follows the process selected by renderer capture, using its host PID and start time plus same-user descendants. It does not guess by title or capture desktop sound as a fallback. Multiple game streams mix together (up to 16), including common mono/stereo/surround layouts downmixed to stereo. Pause or game exit clears the audio selection; new/recreated streams reconnect automatically. The audio manager follows capture independently of the UI, including during recording.

Audio requires Fedora's `pipewire-utils` (`pw-dump`, `pw-record`, `pw-link`); `pw-record` must support `--raw`. Games with isolated audio PIDs, unrelated audio helper processes or unsupported channel layouts may need manual selection or stereo output in the game. Manual window capture does not identify a game PID. Volume and mute affect the captured mix, not the game's normal playback.

See `docs/validation-0.15.2.md` for completed checks and limits. The GPU-unavailable path and software recordings are tested with generated live frames and audio through the real background compositor and FFmpeg. There is no physical GPU in the build environment; the user confirmed AMD recording and subsequently confirmed streaming in 0.13.0. High-resolution game performance has not been measured in this build environment. **Capture details → Copy diagnostics** includes capture, audio and recording events.

Independent streaming-studio development build. No OBS code, libraries or plugins.

## Install and run on Fedora KDE

1. Close the old Rosorah build. **Leave Steam and KDE running.** Extract the complete archive into a new folder.
2. Run `./run-studio.sh`. It installs the hooks for your user and opens Rosorah. The launch observer starts automatically.
3. Open **Capture details**. If permissions block activation, click **Allow OpenGL activation without restarting Steam** and approve Fedora's normal authentication dialog. This authorization applies while this Rosorah session is open; the helper exits with Rosorah.
4. Wait for **Watching new Steam games**, then launch the game normally. No per-game Steam command or special Steam launcher is needed. An OpenGL game that was already running may need restarting; **Steam itself does not**.

This is a development build for native/RPM Steam on x86-64 Fedora. Native Dead Cells was confirmed capturing with hotfix3; the build environment cannot repeat the live Steam test because it blocks process tracing. Retain the working build for comparison.

Keep `rosorah-studio`, `rosorah-launch-watch`, the `hooks` folder, installer and launch script together. The installer needs Python 3 and does not need sudo. Only the optional permission action starts an elevated observer; Rosorah and the game remain normal user applications. It does not change SELinux, ptrace policy, global preload settings, Steam files, game files, or launch options. Root-owned programs and other users' processes are excluded. Protected games, unusual launchers and Flatpak Steam are not supported by this prototype.

Vulkan retains the existing implicit layer and needs no launch observer. OpenGL uses the existing GLX/EGL capture libraries with a new activation route: observe Steam/Steam Runtime launcher process creation; at a game launch, add the hook to that new process's initial environment before its loader runs; continue observing through intermediate launchers, then detach after backend loading or the startup timeout. It does not patch a running game's graphics code and cannot retrofit the hook into an already-initialized OpenGL game.

The observer pauses processes briefly for launch events, not for each captured frame. If it cannot prepare the environment, it leaves the original stack pointer intact and reports a skipped launch. Compatibility with Steam's container setup, fork/vfork/clone behavior, sandbox transitions and protected launch paths still needs desktop testing. Renderer support alone does not establish launcher compatibility.

The default installer removes Rosorah's own old KDE login script. A currently running session may still carry its previous preload; future game launches prepared by the observer replace old Rosorah entries and retain other preloads. No logout is needed for the new mechanism. The old login-based route is available explicitly via `python3 ./install-renderer.py --session-hook`; it takes effect at the next normal login.

The small C OpenGL loader loads the C++ capture backend only on graphics calls. GPU readback starts only when Rosorah selects the game's rendering surface. A missing or incompatible backend passes graphics calls through.

## Added capture paths

| Path | Support in this build |
| --- | --- |
| Vulkan | 64-bit and 32-bit hooks |
| Direct3D 8/9/10/11 games on Linux | Through Proton/DXVK's Vulkan output |
| Direct3D 12 games on Linux | Through Proton/vkd3d-proton's Vulkan output |
| Desktop OpenGL 3.2+ | GLX and EGL swap hooks, 64-bit and 32-bit |
| OpenGL ES 3.0+ | EGL SDR capture, 64-bit and 32-bit |
| Vulkan SDR formats | RGBA8, BGRA8 and packed RGB/BGR 10-bit |
| Vulkan HDR formats | Packed 10-bit PQ/HLG BT.2020 and 16-bit-float linear scRGB, converted to SDR |
| Desktop EGL HDR | Linear scRGB and PQ surfaces, converted to SDR; requires driver support |

These are implemented paths, not a guarantee that every game will work. The user confirmed Octopath Traveler on 0.6.2; this build needs game testing on Fedora. Native Dead Cells was confirmed working with hotfix3. Older OpenGL contexts without the required features, nonstandard presentation APIs, capture-blocking games, protected surfaces and isolated sandboxes can remain unsupported. Windows capture is deferred.

Console emulators use the same capture paths when their final image is presented through supported Vulkan or OpenGL. Vulkan activation is independent of Steam. The new OpenGL observer covers qualifying Steam launches only; automatic activation for a standalone OpenGL emulator is not implemented. Software emulation can still produce a capturable image if its display backend presents through a supported graphics API. No emulator has been tested with this build, and sandboxed packages may not see the installed hooks.

HDR capture uses a fixed 203-nit SDR reference and soft highlight compression. The game can stay in HDR; Rosorah's canvas remains SDR. HDR recording/output, Dolby Vision, arbitrary wide-gamut spaces and GLES HDR are not implemented. Actual HDR display presentation and visual calibration were not tested here.

## Performance and automatic selection

Both paths downscale on the GPU to fit the selected capture resolution before readback. Original size skips downscaling, within the 4096-per-axis limit. Default capture is 60 FPS, with 30 FPS available. Two GPU slots and bounded delivery buffers drop busy frames instead of building a queue. Vulkan retains its background fence polling and persistently mapped memory. OpenGL polls fences without waiting, maps completed PBOs on the owning GL context, and copies the reduced frame to a delivery thread for row inversion, color conversion and transport. The canvas still uses the UI's GPU device directly.

This remains GPU readback plus CPU memory transport; it is not zero-copy. Actual AMD game FPS and capture latency have not been measured here. Idle OpenGL surfaces do not allocate capture GPU resources or full-frame CPU buffers before selection.

Steam-identified applications are preferred, then larger frames. A selected Steam game stays selected while connected. Selection does not inspect focus/fullscreen state. Vulkan games may already be open if they loaded the installed layer. For the new OpenGL activation route, open Rosorah first, wait for launch observation to be ready, then start the game. Pause stops that source; pausing every automatic source stops readback. The last complete image is held during brief reconnects. Manual **Choose window…** remains a separate desktop-portal option.

The canvas receives frames through the existing private home-directory socket and anonymous shared memory. No game pixels are written to disk. This preserves the 0.6.2 connection path that worked with Octopath Traveler.

## Troubleshooting and removal

If a game stays blank, use **Capture details → Copy diagnostics** and include the game name, native/Proton mode, and native/Flatpak Steam. Diagnostics identify Vulkan/OpenGL, process bitness and HDR conversion. For OpenGL, check the launch activation status and start the game after the observer is ready. “Hook prepared” confirms only the launch environment was staged; actual capture is confirmed by received OpenGL frames. Automatic capture includes the game's rendered cursor, not a separate desktop cursor.

To remove automatic hooks:

```bash
python3 ./install-renderer.py --uninstall
```

Close Rosorah to stop the observer. Versioned library files are retained for already-running programs. Running `run-studio.sh` installs them again. `ROSORAH_DISABLE_CAPTURE=1` disables capture in a process that inherits it.

Game audio pairs automatically as described above. Other audio inputs remain manual. Live recording and streaming are available in Broadcast. The CLI demo command still renders synthetic video only.

## Source build and tests

Requires a compatible Rust toolchain (tested with 1.99), a C++17 multilib compiler with 32-bit C/C++ development libraries, Vulkan/GL/EGL/X11 headers, and GStreamer/Wayland development dependencies.

```bash
./native/build.sh
cargo build --release --locked
cargo test --locked
./native/test.sh
ROSORAH_TEST_ARCHES='64 32' ./native/test.sh
python3 packaging/test-install.py
```

Native tests use Mesa or another available driver. Exit 77 means a test was blocked or skipped, not that every integration check passed. Set `ROSORAH_VALIDATE=1` with an installed Khronos validation layer to check Vulkan commands. The optional cross-language receiver test uses `ROSORAH_TEST_PRODUCER="$PWD/native/build/test-64/test-transport" cargo test --locked native_producer_connects_and_delivers_to_rust_receiver -- --ignored`.

Package `target/release/rosorah-studio`, `native/build/rosorah-launch-watch`, `packaging/run-studio.sh`, `packaging/install-renderer.py` and `native/build/{64,32}` as `hooks/{64,32}`. Test libraries must not be shipped. See `docs/validation-0.15.2.md` for validation and limits. Use `ROSORAH_DISABLE_LAUNCH_WATCH=1 ./run-studio.sh` to disable only the experimental launch observer while retaining existing capture paths.
