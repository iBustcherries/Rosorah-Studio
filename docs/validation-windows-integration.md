# Windows preview.5 validation — 2026-10-12

Completed in a Linux x86_64 build container with Rust 1.96:

- Full Windows release build of rosorah-studio.exe and rosorah-windows-probe.exe
  succeeded for x86_64-pc-windows-msvc. LLVM 18 lld-link used Microsoft CRT/SDK
  libraries and official GStreamer 1.26.10 MSVC x64 import libraries.
- Full workspace Windows `cargo check --locked --offline --workspace --all-targets`
  passed after the transport and AMD AMF changes, including native test targets.
- Both outputs are PE32+ x86-64 executables. Each directly imported GStreamer/GLib
  DLL is present in the selected official runtime. This is dependency inspection,
  not a successful Windows launch or a complete runtime loader test.
- Full Linux workspace/all-targets compiler check passed.
- 39 portable tests passed: 30 core, 4 capture, 5 pixel-conversion tests.
- Seven application recording tests passed on Linux with FFmpeg 6.1.1, including
  real hardware-probe failure/software recovery, timeout cleanup, and six distinct
  stereo tracks written to and decoded from MKV. Zero failures. These exercise
  shared command construction and Linux transport, not Windows named pipes.
- Official GStreamer runtime/development installer hashes matched the setup pins.
  FFmpeg 9.0.2 ZIP hash, ZIP integrity and expected executable path were verified.
- Native pipe tests cover multi-buffer transfer/EOF, cancellation, stalled readers
  and unexpected peer rejection. They were compiled, not executed.

Not performed: Windows EXE launch, setup.cmd / PowerShell execution, native pipe
or device tests, GitHub Actions execution, AMD/NVIDIA hardware encoding,
account/browser login, Windows stream reconnect/stop, performance measurement,
installer signing or end-to-end Windows testing. Compilation is not runtime
validation. Test the supplied preview on a Windows PC before relying on it.

The previously delivered Linux executable remains unchanged. No new Linux
binary is included in this Windows development package.
