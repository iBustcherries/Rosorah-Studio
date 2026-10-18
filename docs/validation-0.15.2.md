# Rosorah Studio 0.15.2 validation

## Automatic camera input

- Webcam / USB defaults to Auto detect resolution and FPS. Older saved camera sources gain this default while keeping their manual size/FPS values for an explicit manual override.
- Active V4L2 loopback devices (including the user's DroidCam camera) are queried with read-only QUERYCAP, G_FMT and G_PARM ioctls. Capture negotiates the producer's current size and rational frame rate instead of imposing Rosorah's saved manual mode. An inactive output-only loopback device reports that the desktop client must start sending video.
- Active loopback mode changes are checked once per second. A changed mode releases the old pipeline and opens a new one; failures use the existing automatic retry. A producer may need to stop/restart its own virtual camera before a new mode becomes available.
- Ordinary webcams negotiate an available raw-video/MJPEG mode. Manual mode continues to request the selected size/FPS exactly. Supported input dimensions are 16–4096 pixels per axis.
- Live status shows the negotiated input dimensions and advertised FPS, including fractional rates, before crop. It does not claim to measure the delivered frame cadence. Sources without a reported FPS say so.
- Manual size/FPS controls are hidden when Auto is selected. Direct Wi-Fi controls and the network frame cap remain independent. Game capture, canvas output and recording/stream FPS settings are unchanged.
- Camera errors now include the underlying GStreamer diagnostic text. Auto-mode errors no longer ask the user to match manual settings.

## Validation

- Locked release build, rustfmt and all-target Clippy with warnings denied.
- 69 ordinary tests passed: 12 core, 4 capture, 53 app. Eight existing environment-dependent tests remain ignored.
- A real GStreamer appsrc input in YUY2 (V4L2 YUYV) changed from 1280×720/30 to 1920×1080/60 and then 640×480 at 30000/1001 FPS. The same automatic pipeline decoded all modes, checked dimensions/pixels and reported the actual negotiated rate despite intentionally conflicting saved manual values.
- Tests also cover rational device mode constraints, manual overrides, older project migration, exact manual negotiation, crop/flip and local HTTP multipart/raw JPEG decoding, shared readers and shutdown.
- The x86_64 ioctl sizes and structure offsets were checked against the installed Linux UAPI headers with a compiled C program.
- Native UI opened under Xvfb to inspect the Auto checkbox and hidden manual controls. No physical camera is connected in this environment.
- Release archives are fully decompressed and checked for member hashes, executable permissions and version. Fresh saved downloads are compared byte-for-byte before delivery.

## Scope and limits

The user confirmed their physical DroidCam output reaches 1080p/60 before this release. This build's physical V4L2 probing and mode-change reopen behavior still need testing on that device; the local dynamic-format test uses appsrc, not a real loopback kernel device. A brief interruption may occur while the client restarts or Rosorah reconnects. No kernel modules are loaded or installed by Rosorah.

The capture transport adds an optional atomic frame description so camera mode status matches its frame; normal game frame publishing is unchanged. Native hooks, observer, installer, launcher, audio selection, recording and streaming implementation code are unchanged from 0.15.1. Prior camera/account limits are documented in validation-0.15.0.md.

## Upgrade

Close Rosorah, extract the entire archive into a fresh folder and run run-studio.sh. Keep the existing working kernel and DroidCam driver setup. Steam, KDE and games using 0.13.0-or-later hooks can remain open.
