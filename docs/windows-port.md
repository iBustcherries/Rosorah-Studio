# Current integration checkpoint

See [windows-integration-checkpoint.md](windows-integration-checkpoint.md) for the new full-app integration status and remaining release gates. The earlier backend checkpoint is retained below for history.

# Windows port checkpoint — 2026-10-12

This is implemented backend source, not a Windows Studio release or installer.
The Linux 0.31.0-preview.2 build includes separate alert-preview/audio fixes.

Implemented in crates/windows:
- Native DXGI monitor enumeration across adapters and selected-monitor desktop duplication.
- Captured frames enter Rosorah's existing bounded capture engine; 30/60 FPS scheduling, cancellation, access-loss/device-reset recovery, row-pitch conversion, rotation and output-size limits.
- The first version reads a staging texture back to CPU RGBA; zero-copy GPU sharing is still future optimization. Resizing currently uses nearest-neighbor sampling.
- Window-only and automatic-game requests are explicitly rejected by this monitor backend. Cursor embedding is not implemented; the diagnostic sets cursor=false.
- Native event-driven WASAPI default microphone and desktop loopback capture, shared-mode conversion to 48 kHz stereo float. Worker-owned COM initialization, finite event waits, silence/discontinuity/timestamp handling, packet release and stream cleanup. No internal audio queue.
- Windows Credential Manager read/write/delete, user-scoped persistence, missing credentials handled normally, 2560-byte storage limit reported rather than truncating. A Windows-specific app account-vault adapter is selected via cfg(windows). It has not yet been compiled as part of the complete app.
- An opt-in rosorah-windows-probe diagnostic lists displays, captures a chosen display for five seconds, or reads five seconds of microphone/desktop audio. It saves no media.

Validation:
- Windows MSVC cross-target cargo check of rosorah-windows --all-targets passes on the Linux builder. This type-checks the Windows API bindings, backend, tests and diagnostic; it does not link/run an EXE.
- Five Linux-runnable pixel conversion tests cover padded rows, BGRA/alpha, rotations, malformed input, engine bounds and selected resolution.
- Windows CI now builds the diagnostic and uploads it as an artifact, in addition to shared-engine tests. The workflow was added locally; it has not run on GitHub.
- Credential round-trip test is opt-in and uses a unique temporary credential with cleanup.
- No Windows runtime/hardware test has been performed.

Build on a Windows development machine with Rust/MSVC installed:

    cargo test --locked -p auroracast-core -p rosorah-capture -p rosorah-windows
    cargo build --locked --release -p rosorah-windows --bin rosorah-windows-probe
    target\release\rosorah-windows-probe.exe
    target\release\rosorah-windows-probe.exe --monitor DISPLAY_ID
    target\release\rosorah-windows-probe.exe --audio desktop
    target\release\rosorah-windows-probe.exe --audio microphone

Use exactly a display ID printed by the first command. Desktop loopback can have no packets when no sound is playing. Run hardware tests in an interactive unlocked desktop, with a moving image/audio playing, and test rotation, multiple GPUs, display reconnect and stop/restart. Test device loss/default-device changes separately; audio reconnection is not implemented yet.

Remaining before a Windows Studio preview:
- Connect native capture/audio to the shared Studio UI and mixer; monitor selection, window capture and pointer rendering.
- Port cameras/device discovery, monitoring playback, browser/runtime helpers, donation relay helpers, media/codecs, secure configuration writes, file dialogs, time parsing and platform-specific diagnostics.
- Replace Linux renderer/launch-hook integration; automatic game hooking is a separate backend.
- Complete app compilation, Windows dependencies/installer, actual Windows capture/recording/streaming and account tests.

Primary API references:
- https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/desktop-dup-api
- https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgioutputduplication-acquirenextframe
- https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording
- https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-getbuffer
- https://learn.microsoft.com/en-us/windows/win32/api/wincred/nf-wincred-credwritew
