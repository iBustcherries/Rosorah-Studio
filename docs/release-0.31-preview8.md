# Rosorah Studio 0.31.0-preview.8 — Windows capture and audio

## Added

- Automatic fullscreen application capture and a specific-window selector using
  Windows Graphics Capture. Automatic selection checks the foreground window,
  fullscreen bounds and loaded graphics modules, retaining a selected application
  across Alt-Tab. Browsers and common launcher/shell windows are excluded.
- Optional cursor capture for windows and monitors. Cursor-off monitor capture
  retains the existing DXGI path. Change the cursor option before starting capture.
- Application-only audio through Windows process loopback, including child
  processes. Automatic game audio follows the captured application's identity.
  This never falls back silently to desktop audio.
- Named microphone and output devices, including monitoring output selection.
  Default capture devices reconnect when Windows changes the default endpoint.
- Gentle/Strong microphone noise suppression through GStreamer's WebRTC DSP.
  Audio queues are bounded; no separate filtering process is launched.
- Music discovery from applications publishing Windows media controls, including
  playback status, title, artist, album and bounded artwork. Discovery runs on a
  background worker only while requested; it does not poll a music-service API.

## Try it

1. Close Rosorah and install this Setup EXE. Open your existing project.
2. Select a capture layer and choose Automatic fullscreen application, or choose
   Specific window and select a window. Use Refresh windows / monitors if needed.
   Adding a Capture application layer explicitly enables automatic mode.
3. Enable Include game audio automatically to follow the captured application.
   Mute Desktop audio when testing, so audio is not captured twice.
4. Select named devices in mixer channel Settings. Microphone suppression is
   available for input devices, including renamed microphone channels.
5. Open your music widget and play music in an application that exposes Windows
   media controls. Only participating applications can be discovered.

Old projects do not automatically opt into fullscreen application capture.
Capture waits for an eligible application and never falls back to a whole monitor.
The capture engine clears stale frames when switching or losing a target. A
static captured window retains its audio connection. Selected window handles
are not persisted across application launches.

## Remaining limitations

- Injected DirectX/Vulkan/OpenGL renderer hooks are NOT included. Windows Graphics
  Capture can capture application windows independently of their renderer, but
  protected content, restricted processes, minimized windows and some exclusive
  fullscreen games may not produce frames. Manual selection is available when
  automatic detection cannot recognize an application.
- Process audio requires Windows build 20348 or newer. Windows 11 is recommended;
  common Windows 10 client builds do not meet this requirement. Default/named
  desktop audio remains separately selectable on supported older systems.
- Capture still uses GPU-to-CPU readback; this is not a zero-copy capture pipeline.
  No new latency or GPU-use measurement is claimed without Windows testing.
- Music discovery depends on each player's Windows media integration. It cannot
  identify every browser tab or every streaming service automatically.
- Browser chat keeps its existing separate Python/PySide6 setup requirement.

## Validation

Windows release cross-compilation and all-target checks; Linux application check;
capture mailbox, image conversion and automatic-target policy tests; project
preference migration test. Installer contents are extracted and compared with
the staged payload; source archive integrity is checked. This environment has no
Windows desktop, so native capture/audio/music/installer execution is unverified.
