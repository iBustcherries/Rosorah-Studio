# Rosorah Studio 0.31.0-preview.7

## Preview latency and idle rendering

- Request a one-frame presentation queue instead of the graphics library default. VSync stays enabled to avoid tearing. This is a buffering reduction, not a claim of measured end-to-end latency.
- Remove redundant 30 Hz capture UI polling; capture producers already request a repaint when they publish a frame.
- Suppress capture-driven preview wakeups and GPU uploads when minimized or when no preview panel is open, or while viewing settings/automations without their preview. Recording and streaming keep their independent rendering and latest-frame readers. Capture itself continues.
- On Windows, capture diagnostics include desktop-present-to-publication age (QPC), readback, and conversion. Preview diagnostics on both platforms include mailbox wait, CPU upload-call duration, and upload-to-first-visible-preview submission. These are CPU-side measurements, not GPU execution or screen scanout latency. Desktop age can grow for a static desktop and is not itself proof of queuing.
- Rename Windows Game capture section to Monitor capture and hide Linux launch-hook guidance there.

## System monitors

Footer CPU and RAM now mean whole-system utilization, not Rosorah-only utilization. One background worker samples once per second and publishes one replaceable snapshot; it is stopped on app exit. No shell subprocess is launched for monitoring.

Windows uses GetSystemTimes, GlobalMemoryStatusEx, and the English PDH GPU Engine counters. Process contributions are summed per hardware engine; the busiest reported engine/device supplies the footer percentage. GPU availability depends on driver counter support.

Linux uses /proc/stat, MemAvailable from /proc/meminfo, and DRM gpu_busy_percent (including supported AMD drivers). Drivers without this counter display an unavailable dash; this build does not claim universal NVIDIA/Intel Linux utilization support. Unsupported readings are never fabricated as zero.

## Validation and limitations

Windows is cross-compiled on Linux. Native Windows display timing, PDH readings, installer execution, and minimized streaming must still be tested on Windows. No claim of a specific latency reduction is made. This build keeps the existing media dependency installer and does not add game hooks.

Validation completed: Windows release cross-build; standalone Windows monitor test binary linked; Linux CPU/RAM live sample; three system-monitor tests; four capture-engine tests and six Windows pixel-conversion tests run on Linux; Windows installer extracted and all 555 payload files compared byte-for-byte. Linux capture helper sources match preview.3 exactly (16 native source files), so its tested 32/64-bit helper binaries are preserved. No Windows runtime/GPU measurement was available in the build environment.
