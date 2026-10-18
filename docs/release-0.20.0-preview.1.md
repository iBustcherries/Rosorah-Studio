# Rosorah Studio 0.20.0-preview.1

Close Rosorah, extract the entire ZIP, then launch run-studio.sh. Keep the hooks and browser directories beside the application.

## Material Studio
- Option 1: charcoal surfaces, lavender controls, rounded panels, Roboto bundled offline (SIL Open Font License included).
- Main and Vertical canvases with destinations underneath; Broadcast above Layers; Scenes beside a full-width horizontal mixer. Activity shares the Chat tab group and remains accessible from Panels.
- Existing version-4 dock layout backed up as .pre-020.json before adopting this default. Hidden panels remain hidden; all panels remain movable. Panels > Reset layout restores the default.
- OBS-style mixer: real stereo input/output peaks after scene gain/mute, logarithmic dB scales, green/yellow/red zones, separate slim volume faders, mute and channel settings. Horizontal by default for new projects; saved orientation is retained. Vertical is still available.
- Per-scene gains, input selection, noise suppression, monitoring and track routing retained.
- Existing linked scenes, drag layers, canvas routing, chat destination selection and movement animations retained.

Meters reuse existing PCM blocks; no extra capture stream or animation worker. The design does not introduce a web UI runtime. Existing browser-based chat still uses its companion.

This is a preview. Live donation-provider connections and hosted donation service are still not implemented. Existing native hooks and browser companion are unchanged. See VALIDATION.md for testing and limitations.
