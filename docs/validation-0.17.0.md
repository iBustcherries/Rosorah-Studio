# Rosorah Studio 0.17.0 validation

## Changes

Combined account-scoped Activity history; typed Twitch/YouTube events; low-frequency public-subscriber discovery; video-file backgrounds and queued event alerts with optional sound; GPU fade/wipe transitions; simpler Broadcast/Layers controls; opt-in desktop output capture; per-scene volume/mute applied separately to each canvas output; FFmpeg microphone denoising presets.

The process-lifetime libsecret library handle from 0.16.6 remains in place. The crash log supplied on October 10 points to an executable launched from the 0.16.5 directory and the known `secret_schema_new` / `g_intern_static_string` crash path. This build does not modify GPU drivers or the kernel.

## Validation performed

- Release workspace suite: 103 tests passed. Eleven environment-dependent tests are ignored in the default suite; two relevant ones were run explicitly as described below.
- Explicit GPU test on software Vulkan: fade and wipe pixel results pass; changing one canvas does not affect the other. Transitions snapshot the prior fully composited frame and affect exported pixels.
- Explicit media-output integration: a real WebM file with audio is decoded, recorded to MKV, and followed by a scene switch. Decoding the resulting MKV verifies the new scene's pixels, audible file sound before the switch, and scene mute afterward. The test passes through the real GStreamer backend, shared frame reader, compositor, FFmpeg recorder, and per-output audio-channel selection.
- Real GStreamer video looping and cancellation test passes. Audio queues are bounded and cleared on stop. Test audio confirms the Strong filter reduces quiet background noise while Gentle preserves a 1 kHz voice-band signal.
- Activity tests cover persistence, account-specific identities, duplicates, moderation redaction, public-subscriber identity, typed browser events, rejection of stale broadcast activity, and one-time layout migration that preserves a closed Activity panel.
- The wallet regression passes with the real libsecret/GObject libraries, including repeated and concurrent wallet-handle lifetimes.
- Seven Python browser-companion tests pass; both JavaScript collector/composer suites pass, including a 350-message push burst, emotes, deletion, stale callbacks and draft preservation.
- Strict release workspace/all-target Clippy passes. The optimized Linux binary builds successfully. Native studio/Activity screenshots were inspected under Xvfb/software Vulkan, with an actual looping video on the canvases.
- Release ZIP and TAR members, contents, executable permissions and checksums are verified during packaging. The native Vulkan/OpenGL hook binaries and launch observer are unchanged from 0.16.6.

## Limits of local testing

No signed-in Twitch or YouTube account, real Fedora PipeWire input, physical microphone, or AMD hardware encoder is available in this validation environment. Provider delivery, account permissions, actual microphone sound quality, installed video-codec coverage, and hardware performance need target-machine checks. Existing game capture and streaming implementations are retained.

YouTube browser activity depends on its page markup. Public subscribers are checked at most once per five minutes with a selected broadcast (thirty minutes otherwise), with an hour backoff on error. This is not a push feed or complete historical archive, and private subscribers are not exposed. The latest fifty returned subscribers are considered. Initial browser history is displayed but does not trigger video alerts; the first five seconds after loading a chat page are conservatively treated as history. Received activity is capped at 5,000 entries and thirty days.

Scene audio changes volume and mute; other channel settings remain shared. A video source shared across scenes/canvases has one playback clock. Scene transitions use a still snapshot of the outgoing composition. Noise suppression is spectral denoising/high-pass filtering with an additional downward gate in Strong, not neural voice isolation.
