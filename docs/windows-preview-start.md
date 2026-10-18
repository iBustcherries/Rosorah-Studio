# Rosorah Studio 0.31.0-preview.6 — Windows test preview

See release-0.31-preview6.md for this build’s capture changes and timing details.

Upgrading from preview.5: close the old app, extract into a new folder, and copy
the old folder’s runtime folder into this one. Then run run.cmd; existing media
dependencies can be reused. Keep the old build available for comparison.

This ZIP contains a real Windows x64 Studio executable and diagnostic. It is an
unsigned development build. It has been compiled and linked on Linux, but has
not been launched on Windows. Keep your existing Linux installation and back up
projects before opening them in this preview.

## Start

1. Extract the entire ZIP into a writable folder, for example Documents\Rosorah-test.
2. Double-click **setup.cmd** once while online. It downloads Microsoft's Visual
   C++ runtime, GStreamer 1.26.10 (official installer), and FFmpeg 9.0.2 (Gyan build).
   Follow the official installer prompts; administrator approval may be required.
   Allow GStreamer to install at C:\gstreamer\1.0\msvc_x86_64. Setup verifies the
   downloaded files before use. It does not install or update GPU drivers.
3. Double-click **run.cmd**, rather than the EXE, so Rosorah can find its media DLLs.
4. Select a display in the capture source and start that selected display. This
   preview uses monitor capture; automatic game hooking is not implemented yet.
5. Select the system microphone/desktop audio as needed. Make a short local
   recording first and play it back to check video, audio and stop behavior.

Setup requires roughly 200 MB of downloads and additional extraction space.
If it reports an error, save that text; do not continue with partial dependencies.
The executable is not code-signed. If Windows blocks it, report the exact message;
this preview does not require disabling Windows security protections.

## Chat/browser setup

Setup installs Python 3.13 for your account when it is missing; Rosorah uses it to
download its private PySide6 runtime the first time browser chat starts.
Account authorization and browser website sign-in may both be needed.

## First Windows test

- Record 20 seconds with a moving image and desktop sound, then stop and replay.
- Check the encoder label. AMD AMF / NVIDIA NVENC use a real encode probe; software
  fallback is expected if the Windows driver/encoder is unavailable.
- Test camera selection, microphone, alert video/sound, save/reopen and normal exit.
- Only after local recording works, test an unlisted/private stream, stop, and
  reconnect. Never include stream keys or account tokens in diagnostic reports.
- Send the exact error and whether it happened during setup, startup, capture,
  recording or streaming. `rosorah-windows-probe.exe` is an optional terminal
  diagnostic for display and audio capture (see windows-port.md in the source).

Known gaps: individual-window/automatic game capture, cursor composition,
per-game audio, Windows microphone noise suppression, Intel QSV, music-session
integration, full GPU telemetry and native Windows performance validation.
Desktop capture may include other windows on the selected monitor.

## Dependencies and source

Media dependencies are downloaded directly from their distributors and retain
those distributors' licenses. No FFmpeg/GStreamer runtime binaries are bundled
in this ZIP. FFmpeg build: https://www.gyan.dev/ffmpeg/builds/
GStreamer: https://gstreamer.freedesktop.org/download/
Microsoft runtime: https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist
The matching Rosorah source ZIP is supplied alongside this preview.
