# Validation — 0.31.0-preview.2

- Linux release workspace tests: 189 passed, 0 failed; hardware/desktop opt-in tests remain ignored (23 total).
- New floating-window regression: 80 repeated frames at 1440×900 and 800×600 with intentionally oversized content; preview stays inside the screen and stops changing size after initialization.
- New alert audio regression: guided media enables local monitoring, preserves stream track 1, remains audible in scene routing, preserves an already chosen monitoring device, and respects opting out.
- New shared-box regression: follow, raid and donation automations across scenes reuse one manually created alert box without adding duplicate layers.
- Existing generated-WAV alert playback test still passes, verifying nonzero decoded PCM and clean cancellation. Actual user speakers, codec availability and PipeWire routing require on-device verification.
- Five native Windows pixel conversion tests pass on Linux.
- Windows MSVC cross-target check of rosorah-windows --all-targets passes. No Windows EXE has been linked/run here, and the full Studio app is not yet Windows-compatible. See windows-port.md.
- Native Linux release launched successfully under Xvfb/Mesa for the automation-preview and alert-guide-media pages; both initial and settled UI screenshots inspected. The preview remains a bounded floating window; the guide's audio control, shared-box explanation and navigation are visible.
- GPU compositor self-check passed.
