# Validation — Rosorah Studio 0.10.0

## Scope

Adds one-canvas live MKV recording with the existing Track 1 audio mix. Source
frames are shared by reference through an independent bounded reader; the new
background compositor does not depend on UI redraws. Capture pause/reconnect
clears/replaces the shared reader safely. Audio selection also follows that
reader independently of the UI. The native hooks, observer, installer and
renderer IPC protocol remain unchanged from the user-confirmed working build.

Output has a shared monotonic origin and a fixed video/audio duration. Late
video is repeated; missing audio is silence. A 60 ms scheduling allowance,
2–8 completed video buffers, and at most 100 ten-millisecond PCM blocks keep
queues bounded. Raw format probing is explicitly restricted so FFmpeg does not
hold back startup video while collecting audio. Encoder threads are limited;
the game/capture/mixer never writes to an encoder pipe. A sustained encoder
stall or failure is reported, with the partial MKV retained.

## Completed checks

- Rust 1.99.0, Cargo locked release build; formatting and Clippy with warnings
  denied. No new third-party Rust dependencies. Workspace version 0.10.0.
- 36 ordinary tests passed (8 core, 4 capture, 24 application). Four integration
  tests are normally ignored; the three new recording integrations were run
  explicitly and passed. The existing native receiver/socket test remains
  blocked by the environment.
- The actual compositor + FFmpeg recording path was driven by live synthetic
  source updates and timestamped PCM, without a UI. Horizontal 160 × 90 at
  30 FPS and vertical 90 × 160 at 60 FPS produced two-stream MKVs. Decoded
  flash/tone onsets aligned at 1.000 s in the final small-frame runs. Three-second
  files had container durations 3.054 s / 3.038 s, including frame rounding and
  AAC priming. Decoded audio/video endpoints differed by less than 50 ms.
- Five-second 1920 × 1080 and 1080 × 1920 recordings at 30 FPS passed through
  Mesa llvmpipe; both reported a 5.054 s container duration and aligned 1.000 s
  flash/tone onset. These are generated frames on a software Vulkan adapter,
  not a game-performance benchmark or a measurement of PipeWire capture latency.
- A deliberate 300 ms missing-audio interval remained silent without shortening
  the soundtrack. Editing another canvas did not change the recorded dimensions.
  The preview draining its mailbox did not consume the recorder's frames.
- A short silent recording finalized when the recorder was dropped (application
  exit path); ffprobe saw both streams and the decoded audio was silent.
- MPEG-4/AAC and VP8/FLAC fallback files were actually encoded. Existing file
  protection preserved a sentinel file. Codec selection accepts restricted
  FFmpeg builds and rejects missing audio encoders. FFmpeg error output is
  checked even when that FFmpeg build returns a successful exit status.
- Capture reader stop/reconnect and late-old-generation rejection were tested.
  Legacy project preferences load without a recording-directory field.

## Release checks

The release executable's GPU composition self-check and installer tests are run
before packaging. Packaging compares all six native hook libraries and the
launch observer to the working baseline. ZIP and TAR archives are fully read
back, including every member hash and executable mode. Saved download copies
are checked against the originals and extracted with standard unzip/tar tools.

## Not established here

This environment has no Fedora KDE session, Steam game, usable PipeWire server
or AMD hardware, and blocks AF_UNIX and process tracing. The user confirmed the
0.9.0 capture/audio build works; this new recorder still needs a real game test
for capture-to-sound offset, long-session stability, minimize/restore, window
close behavior, and CPU/GPU/game FPS impact. Hardware encoding, zero-copy frame
import, multitrack video output and streaming are not part of this release.

The existing camera placeholder and preview alert banner remain placeholders;
preview alert text is not burned into recorded output. Recording one selected
canvas does not simultaneously export every canvas.

## Reproduce

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --release --locked -- -D warnings
cargo test --workspace --release --locked
cargo test --release --locked -p rosorah-studio recording::tests:: -- --ignored --nocapture --test-threads=1
ROSORAH_RECORDING_TEST_LARGE=1 ROSORAH_RECORDING_TEST_SECONDS=5 cargo test --release --locked -p rosorah-studio recording::tests::live_recording_keeps_audio_video_timing_without_ui -- --ignored --nocapture --test-threads=1
cargo build --release --locked
./target/release/rosorah-studio --self-check
./native/build.sh  # native build prerequisites are listed in README.md
python3 packaging/test-install.py
```

The recording integration tests require FFmpeg, ffprobe and Vulkan. Set
`ROSORAH_RECORDING_TEST_DIR` to retain test recordings in a chosen directory;
otherwise they use the system temporary directory. `VK_ICD_FILENAMES` can select
a software driver where no GPU is available.

References: [FFmpeg command-line options](https://ffmpeg.org/ffmpeg.html),
[FFmpeg pipe protocol](https://ffmpeg.org/ffmpeg-protocols.html#pipe), and
[Fedora's ffmpeg-free package](https://packages.fedoraproject.org/pkgs/ffmpeg/ffmpeg-free/).
