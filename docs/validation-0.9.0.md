# Rosorah Studio 0.9.0 validation

Built 2026-10-10 UTC for Fedora x86-64, starting from the user-confirmed working
0.8.0-hotfix3 package. This release adds automatic game audio capture.

## Changes

- Renderer connections retain the same-user host PID from `SO_PEERCRED` and its
  `/proc` start time. Selected process metadata reaches the audio controller
  through capture snapshots without changing the native wire protocol or pixels.
- The Game audio channel follows that PID and same-user descendants. It does not
  match by application title, select a Steam sibling, or fall back to desktop
  audio. Pause, stop, game changes and PID reuse invalidate old mixer bindings.
- PipeWire node discovery refreshes once per second while an input is requested.
  Newly appearing/recreated streams reconnect automatically; failed starts back
  off for two seconds. A changed channel layout causes reconnection.
- Multiple matching playback streams are summed before the existing channel gain,
  mute and six-track routing. Mono, stereo and common surround positions produce
  48 kHz stereo PCM. Unknown/unavailable formats report a connection error.
- Graph queries and linking run on a separate worker from the 10 ms mixing clock.
  Per-input PCM buffers are bounded to 100 ms. Raw PCM support is explicitly
  checked to avoid interpreting AU/WAV headers as audio samples.
- Include game audio automatically is enabled by the existing default preference.
  The mixer exposes status, level, mute and volume. Audio diagnostics are included
  in Copy diagnostics. Automatic capture does not require an extra Start action.
- Existing normal game playback is left in place. Audio is not replayed to the
  user's speakers. Files are created only by the explicit audio test recorder.

## Completed checks

- Rust 1.99 optimized build completed; CLI identifies version 0.9.0.
- `cargo fmt --all --check` passed.
- `cargo clippy --workspace --all-targets --release --locked -- -D warnings` passed.
- `cargo test --workspace --release --locked`: **32 passed, 0 failed, 1 ignored**.
  Coverage includes host process start-time parsing, same-game descendants versus
  Steam siblings, PID reuse, multiple audio streams, immediate suppression of old
  bindings, mute/gain/track routing, mono/surround downmix, non-finite samples,
  PulseAudio stream PID precedence, explicit port selection, WAV headers, and
  capture cancellation/reconnection. The real GStreamer synthetic-frame test
  also passed.
- The release executable's GPU self-check passed on software Vulkan (llvmpipe):
  both canvas sizes, shared textures, layer placement/visibility, alpha, row
  padding, unused assets, and empty scene behavior.
- The native launch helper, all six renderer libraries, installer and launch
  script are retained from hotfix3 and compared byte-for-byte. No hook rebuild,
  launch-option change, login script change or Steam restart is introduced.

## Limits of this validation

**Live game-to-PipeWire capture has not been tested here.** This execution host
rejects AF_UNIX stream and sequenced-packet sockets with EPERM, preventing a live
audio server and complete renderer connection test. The existing native-to-Rust
socket test remains explicitly ignored. Unit tests do not establish live audio
delivery, reconnect timing, real game performance, or latency on Fedora hardware.

The available test-host PipeWire 1.0.5 client does not support `pw-record --raw`;
the app deliberately rejects that version instead of reading a file header as
samples. Fedora needs pipewire-utils with raw capture support. No audio daemon,
system security setting, or privileged service was changed on the user's PC.

Native Dead Cells capture was confirmed by the user on hotfix3. Vulkan was
previously confirmed with Octopath Traveler. Neither game, Steam, KDE, AMD
hardware or an actual audio device was available for testing this release.
This build does not add speaker monitoring, synchronized live video/audio
recording, live broadcasting, Windows support, or standalone OpenGL activation.

Matching is conservative: isolated PID namespaces and sound delegated to an
unrelated process can need manual application selection. Up to 16 matching
streams and up to eight channel positions per stream are supported. Normal
audio metadata is used for selection, not as a security authentication mechanism.

## Fedora acceptance check

1. Close Rosorah, extract this complete package into a fresh directory and run
   `run-studio.sh`. Keep Steam and KDE open; launch the game after Rosorah is ready.
2. Leave Include game audio automatically enabled. Play a sound and confirm the
   Game audio meter responds and its status names the captured game.
3. Play unrelated browser/music audio and stop the game's sound. The game channel
   should not respond to that other application. Leave other mixer channels off
   for this isolation check.
4. Use Record audio test briefly, then stop it and play `track-1.wav` from the
   displayed Music/Rosorah-audio folder. Check volume, mute and track routing.
5. Pause capture and verify game audio stops in the mixer. Resume, restart the
   game, and check that audio reconnects automatically. Repeat with the working
   Vulkan game. Compare game performance against hotfix3.
6. If the meter stays silent, use Copy diagnostics and include the game name and
   native/Proton mode. Manual app selection is available by turning off the
   automatic game-audio checkbox.

## API references

- https://docs.pipewire.org/page_man_pw-cat_1.html — explicit target, raw PCM,
  format, sample rate and channel mapping.
- https://docs.pipewire.org/page_man_pw-link_1.html — explicit passive port links.
- https://docs.pipewire.org/page_man_pipewire-props_7.html — process properties,
  PulseAudio server PID distinction, monitor and connection policies.
