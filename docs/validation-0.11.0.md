# Validation — Rosorah Studio 0.11.0

## Scope

Adds H.264 hardware encoding to the user-confirmed 0.10.0 live recorder.
Auto checks VAAPI devices under `/dev/dri/renderD*`, including the alternate
low-power entrypoint, and NVIDIA NVENC. It selects a working software encoder
when GPU checks fail. Software mode skips GPU checks. The preference is saved
with the project; old projects default to Auto.

Each candidate must actually encode four RGBA frames at the selected canvas
size and FPS, plus stereo PCM, through the same filters, codecs and MKV muxer
as the live recording. Probe output goes to `/dev/null`. Success requires a
successful exit, no error-level output, and completed FFmpeg progress with at
least four video frames. Listing an encoder alone does not pass the check.
The recording clock starts after encoder selection and compositor setup.

The check runs in the recording worker, supports cancellation, and has a
three-second limit per candidate and a ten-second total GPU-check budget.
Software candidates have their own three-second limits. Probe pipes are
drained concurrently with bounded diagnostic buffers; failure/cancellation
kills and reaps FFmpeg before joining writers. No user recording is created
by the probe. Real output retains both the existence check and FFmpeg `-n`.

VAAPI uses NV12 upload, CQP 23 and no B-frames. NVENC uses the P1 preset,
low-latency tuning, constant QP 23, no lookahead and no B-frames. The existing
software encoder settings remain unchanged. The active encoder and device
appear in Broadcast; selection errors are retained in Recording details and
Capture diagnostics. If hardware fails during an active recording, the error
and partial file are retained; there is no silent mid-file codec switch.

## Checks

- Formatting, Clippy with warnings denied, locked release build and executable
  GPU self-check are required before packaging. No Rust dependencies added.
- 39 ordinary workspace tests pass (8 core, 4 capture, 27 application).
  Selection tests cover an unavailable first GPU followed by a working second
  GPU, software fallback, explicit Software mode, restricted FFmpeg builds,
  hardware-only builds, and cancellation without starting another candidate.
  Legacy preference loading and saved Software preference roundtripping pass.
- Five recording integration tests pass with real FFmpeg 6.1.1. The missing
  VAAPI-device test reports actual driver errors then succeeds with software;
  MPEG-4/AAC and VP8/FLAC probes also complete. A short probe deadline confirms
  that a terminated child does not leave blocked input writers.
- Generated live frames and PCM pass through the real compositor and recorder.
  Horizontal 160 × 90 at 30 FPS and vertical 90 × 160 at 60 FPS have aligned
  decoded flash/tone onsets at 1.000 seconds. Three-second outputs have container
  durations 3.054 and 3.038 seconds, including frame rounding and AAC priming.
  A deliberate audio gap remains silent, and decoded stream endpoints stay
  within 50 ms. Independent preview consumption does not steal recorded frames.
- Short silent recording/drop finalization and actual MPEG-4/AAC and VP8/FLAC
  output pass. Existing-file protection leaves sentinel bytes unchanged.
- Five-second 1920 × 1080 and 1080 × 1920 recordings at 30 FPS are also checked
  to protect the large-frame startup path after the FFmpeg command refactor.

## Release integrity

Capture, audio routing, the native Steam observer, all six Vulkan/OpenGL hook
libraries and the hook installer are unchanged from the working baseline.
Packaging compares the native binary bytes. ZIP and TAR are written with
bounded writes and read back completely, including gzip trailers and every
member hash and mode. Saved download copies are compared to the originals
and extracted with standard unzip/tar tools before delivery.

## Limits

This environment has no physical AMD, Intel or NVIDIA GPU, Fedora KDE session,
Steam game or usable PipeWire server. It blocks AF_UNIX and process tracing.
Successful VAAPI/NVENC encoding, image quality and performance on actual GPUs
have not been validated here. The user confirmed recording on 0.10.0; 0.11.0
still needs a real hardware recording test.

The short probe establishes basic compatibility, not sustained throughput or
long-session driver stability. H.264 hardware encoding depends on the installed
FFmpeg and driver codec support. Some Fedora packages omit that support; the
app reports the failure and falls back without modifying packages or drivers.
Hardware encoding moves compression to the GPU encoder, but frame readback,
pixel conversion, audio processing and file output still use resources. This
is not zero-copy recording. No game FPS or CPU improvement is claimed from
these software-driver tests. Windows and broader standalone OpenGL activation
remain deferred. Streaming and multitrack video output remain future work.

## Reproduce

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --release --locked -- -D warnings
cargo test --workspace --release --locked
cargo test --release --locked -p rosorah-studio recording:: -- --ignored --nocapture --test-threads=1
ROSORAH_RECORDING_TEST_LARGE=1 ROSORAH_RECORDING_TEST_SECONDS=5 cargo test --release --locked -p rosorah-studio recording::tests::live_recording_keeps_audio_video_timing_without_ui -- --ignored --nocapture --test-threads=1
cargo build --release --locked
./target/release/rosorah-studio --self-check
```

Recording tests require FFmpeg, ffprobe and Vulkan. A software Vulkan adapter
can run them where no GPU is available. Set `ROSORAH_RECORDING_TEST_DIR` to
retain their generated recordings in a chosen directory.

References: [FFmpeg VAAPI encoders](https://ffmpeg.org/ffmpeg-codecs.html#VAAPI-encoders),
[FFmpeg hardware device options](https://ffmpeg.org/ffmpeg.html), and
[NVIDIA FFmpeg guide](https://docs.nvidia.com/video-technologies/video-codec-sdk/13.1/ffmpeg-with-nvidia-gpu/index.html).
