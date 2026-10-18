# Rosorah Studio 0.12.0 validation

Built October 10, 2026 from the saved, user-tested 0.11.0 source. This is a Linux x86-64 development build; the native observer/installer and six 32/64-bit Vulkan/OpenGL libraries are retained unchanged.

## Implemented

- Twitch and YouTube RTMP/RTMPS destinations, explicit Start/Stop, independent delivery status/timers, reconnect with backoff and keyframe restart.
- A single actual H.264/AAC encoder and canvas compositor shared by both destinations and the optional MKV stream recording. Network and recording outputs remux packets without encoding again.
- Bounded independent destination queues. Network failure does not block healthy outputs. Recording write failure is reported without stopping the streams.
- Actual CBR encoder probe at the selected size/FPS, Auto VAAPI/NVENC with software H.264 fallback, two-second keyframes, 160 kbps AAC stereo.
- Separate mode-0600 stream settings, masked key fields, atomic save, secret redaction before diagnostic truncation. Projects do not contain stream keys.
- Capture defaults to 60 FPS. Both default project canvases and newly added canvases use 60 FPS. Existing saved canvas FPS values are loaded unchanged. The native hooks receive capture cadence via the existing control protocol, without changing game presentation cadence.

## Completed checks

- Locked release build, formatting, and Clippy with warnings denied.
- 44 ordinary tests: 8 core, 4 capture, 32 app. Eight environment-dependent tests are normally ignored; seven recording/streaming integration tests were explicitly run successfully. The remaining native Unix-socket test is blocked by the build environment, and those native bits are unchanged.
- Real background Vulkan compositor plus H.264/AAC streaming at 320×180/60 FPS to **two local RTMP receivers**, with an MKV copy from the same encoder. A paired flash/tone at 1.000 s stayed aligned in both the saved MKV and the healthy network receiver.
- Sixteen-second shared recording: 960 decoded frames at 60 FPS. Healthy receiver: 950 decoded frames, with only the bounded stop tail omitted. After forcibly terminating and restarting the second receiver at four seconds, its independent worker reconnected and delivered 592 decodable frames; the first receiver stayed Sending and the recording remained continuous.
- Cancellation during a deliberately stalled RTMP handshake finished within the three-second bound (complete test runtime 2.01 s), without claiming Sending or creating a recording.
- RTMPS protocol check using a local TLS-to-RTMP test receiver: trusted test certificate delivered H.264/AAC; an untrusted test certificate was rejected with verification enabled. The explicit test CA was used only for this test, not installed globally or shipped.
- Existing real recording tests passed: horizontal 160×90/30 and portrait 90×160/60 A/V synchronization, missing-input silence, independent preview consumption, short recording/Drop finalization, no-overwrite behavior, MPEG-4/VP8 fallback, invalid VAAPI device recovery, and cancelled encoder probes.
- Bounded queue overflow isolation, malformed/oversized packet rejection, diagnostics redaction, settings persistence/permissions, and endpoint validation tested.
- Native application screenshots inspected under Xvfb/X11: streaming settings and the main studio. This is UI layout evidence, not a KDE Wayland interaction test.

## Validation limits

No Twitch/YouTube account or stream key was used, and no public broadcast was started. Actual service authentication, stream visibility, long-session reconnect behavior, and the user's upload route need a platform test. Sending reports media delivered by the encoder, not public audience visibility.

The build environment has software Vulkan but no physical AMD/Intel/NVIDIA GPU. The user confirmed AMD recording on Fedora with 0.11.0; the new streaming CBR profile, 60 FPS game capture performance, KDE minimizing/closing behavior, and real game audio routing need the user's physical-device test. The previous recording profile remains unchanged.

Capture still scales to fit 1280×720 before transfer. This is not a zero-copy capture/encode pipeline. Full-size canvases upscale that image. Audio uses Track 1 only; account sign-in, broadcast editing, chat, CEF, HDR, Enhanced Broadcasting, and per-platform encode settings remain future milestones.

## Primary implementation references

- Twitch video broadcast URLs and keys: https://dev.twitch.tv/docs/video-broadcast/
- Twitch broadcast settings: https://help.twitch.tv/s/article/broadcasting-guidelines
- Twitch global ingest: https://help.twitch.tv/s/twitch-ingest-recommendation?language=en_US
- YouTube encoder guidance: https://support.google.com/youtube/answer/2853702?hl=en
- YouTube RTMPS guidance: https://support.google.com/youtube/answer/10364924
- FFmpeg protocols and network timeouts: https://ffmpeg.org/ffmpeg-protocols.html
- FFmpeg stream copy/progress: https://ffmpeg.org/ffmpeg.html
