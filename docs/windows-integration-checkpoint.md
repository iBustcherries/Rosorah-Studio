# Windows integration checkpoint 2 — 0.31.0-preview.5

The full Studio application and diagnostic have been cross-linked as Windows x64
EXEs using Rust 1.96, LLVM 18, Microsoft CRT/SDK and real GStreamer 1.26.10 import
libraries. This is an unsigned development preview. No Windows EXE, setup script
or native API test has been executed in this Linux workspace. GitHub Actions has
not been run. See windows-preview-start.md for the test package setup.

## Recording and streaming transport

- Separate outbound Windows named pipes carry raw video and interleaved audio
  to FFmpeg, including multi-track recording. Relay input also uses a named pipe.
- Pipe access is restricted to the current user's SID; remote clients are rejected
  and the connected reader PID must match the spawned FFmpeg process.
- Overlapped writes are bounded to 64 KiB, check cancellation every 20 ms and
  time out after 10 seconds. Pending I/O is cancelled and drained before buffers
  are released. Encoder shutdown cancels both writers. Relay stop interrupts a
  stalled writer; existing bounded queues and reconnection policy are retained.
- Windows encoder probes use NUL for discarded output. AMD H.264 AMF and NVIDIA
  NVENC are attempted when advertised, with a real encode probe before selection
  and the existing software fallback. Intel QSV is not yet implemented.
- Native Windows tests cover multi-buffer payloads, EOF, missing/stalled readers,
  cancellation and rejecting an unexpected client. They are compile-checked;
  execution on Windows is still required.

## Connected to the full app

- DXGI monitor capture feeds the existing capture engine and canvas compositor.
  Choose a monitor in Layers → Game capture, then Start selected monitor.
  Each source remembers its own display ID. Changing displays stops capture;
  opening a project never implicitly starts whole-monitor sharing. Enumeration
  runs off the UI thread on refresh rather than repeatedly every frame.
- WASAPI default microphone and desktop loopback feed the existing mixer,
  per-scene volumes, meters, audio buses and output routing. Input queues retain
  at most 100 ms. Device failures stop and retry through the existing supervisor.
  Select a Windows device explicitly; Linux PipeWire device IDs do not transfer.
- Opt-in media/alert monitoring feeds GStreamer's Windows WASAPI output. Both
  the handoff and media queues are bounded. Choose System default output.
- GStreamer Windows camera providers enumerate device paths and create their
  own capture elements. Identically named cameras retain separate selections.
  Virtual cameras require the vendor's Windows driver and running desktop client.
- Native open/save/folder dialogs, ShellExecute URL/file opening without command
  shell interpolation, and the previously implemented Credential Manager vault
  now participate in the full-app Windows compilation.
- Browser helpers use Windows virtual-environment paths and AppData storage.
  The development setup still requires Python 3 with venv/pip; no embedded
  Python installer is included. PySide6 download/setup remains opt-in existing flow.
- Linux portal/D-Bus, process traversal and renderer helpers are target-isolated.
  Shared broadcast scheduling uses GLib's cross-platform local time conversion.

## Deliberate remaining gaps

- Automatic game hooks, individual-window capture, cursor compositing,
  per-process game audio, and desktop music-session discovery remain pending.
  The app does not substitute whole-desktop audio/capture for those requests.
- Windows microphone noise suppression is not connected; select Off. Explicit
  endpoint selection beyond system defaults and following a changed default
  without invalidating the old endpoint require more work.
- Native file dialogs, camera negotiation, Qt browser login, alert audio,
  reconnect/stop behavior and GPU adapter compatibility have not been hardware
  tested. Windows hardware telemetry still needs platform work. AMD/NVIDIA encoder
  selection is implemented but has not been tested on a Windows GPU. Older Linux-specific guidance elsewhere in the UI needs review.
- The test ZIP downloads media dependencies from their providers via setup.cmd.
  GStreamer/FFmpeg are pinned and hash-checked; Microsoft runtime signatures are
  checked. This setup has not been exercised on Windows. A validated installer,
  code signing and a complete release license inventory remain release gates.

## Reproducible Windows build gate

Use Windows 10/11 x64, Rust 1.96 or newer and Visual Studio C++ build tools with
Windows SDK. In a development/build machine, install matching GStreamer MSVC
x64 runtime and development packages, including media plugins. The provided
`packaging/windows/install-gstreamer.ps1` pins official 1.26.10 installers and
verifies their SHA-256 hashes before running them. This setup changes only the
build machine; it is not an end-user setup flow.

From x64 Developer PowerShell, run:

```powershell
$env:GSTREAMER_1_0_ROOT_MSVC_X86_64 = 'C:\gstreamer\1.0\msvc_x86_64'
./packaging/windows/build.ps1
```

The script checks plugins, checks all targets, runs core/capture/native-service
tests, then links Studio and its diagnostic. It creates a new development folder
only after success, refusing to overwrite an existing output. The folder requires
GStreamer installed on that computer. Launch through `run.cmd`. This script and
`.github/workflows/windows-app.yml` still need a real Windows execution.

## Next release gate

Run the native test/build gate and validate setup, EXE launch, monitor capture,
recording tracks, stream reconnect/stop, devices and account login on an
interactive Windows machine before promoting this preview to a release. Preserve Linux capture while adding game hooks
as a separate Windows backend.
