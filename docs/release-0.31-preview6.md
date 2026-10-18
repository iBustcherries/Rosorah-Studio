# Windows capture performance preview — 0.31.0-preview.6

This preview targets the approximately one-second canvas delay reported on
Windows before recording. The cause has not been conclusively reproduced.

## Changes

- Unscaled BGRA-to-RGBA conversion uses contiguous row loops instead of divisions
  and rotation calculations per pixel. Scaled/rotated conversion precomputes
  horizontal coordinates and calculates row mapping once per row. Pixel output,
  capture resolution, opaque alpha and aspect-ratio behavior are unchanged.
- DXGI staging readback uses nonblocking Map calls and cancellation checks.
  Only one GPU copy may be pending. A 16 ms poll budget yields to the capture
  loop while retaining that copy, rather than submitting another queued copy.
  This does not make the driver itself cancellable or guarantee a latency bound.
- The acquired DXGI frame is retained through processing/pacing and released
  immediately before acquiring the next frame, or on backend destruction.
- Pointer-only desktop events skip readback after the first image, because this
  backend does not yet composite the cursor.
- Capture details show readback age and CPU conversion time. Readback age includes
  scheduling/polling time; it is not a GPU profiler measurement or end-to-end delay.

The shared capture engine already has a latest-frame mailbox and wakes the UI on
publication. This change does not add frame queues, reduce recording quality,
change the default capture FPS, or implement Windows automatic game hooks.

## Validation

Ten portable tests passed (four capture engine, six Windows pixel conversion).
Full release linking of Studio and the Windows diagnostic also succeeded with
the Microsoft SDK and GStreamer 1.26.10 import libraries.
The new reference comparison covers odd dimensions, padded rows, scaling and all
four rotations. Windows backend/test targets pass MSVC cross-target checking.

A 40-iteration release-mode Linux CPU benchmark against preview.5 measured:

| Input/output | Before | After |
| --- | ---: | ---: |
| 1080p / 1080p | 5.076 ms | 1.463 ms |
| 4K / 4K | 21.016 ms | 6.965 ms |
| 4K / 1080p | 5.884 ms | 4.266 ms |

These are component measurements on the builder, not Windows hardware results.
Windows GPU readback, cancellation, display changes and end-to-end preview delay
still require a real Windows test. Do not claim the reported delay is resolved
until the new build has been tested on the user's PC.

## Windows test

Close the old instance and extract this build into a new folder. Existing
GStreamer/Visual C++ installations can be reused. The launcher can reuse the
previous folder's runtime/ffmpeg-9.0.2-essentials_build directory if you copy that
folder into this build's runtime folder; alternatively run setup.cmd again.

Run run.cmd, select the same monitor and resolution, then compare a moving image
with the canvas before recording. Check Capture details for readback/conversion
numbers. Test stopping capture, restarting it, and a short recording. Report any
remaining visible delay and those numbers. Keep the old build available.

API reference: https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgioutputduplication-releaseframe
