# Validation — 0.20.0-preview.1

- Optimized Linux x86_64 build, native egui/wgpu UI with embedded Roboto font and included SIL Open Font License.
- 118 automated tests passed: 20 core, 4 capture, 94 application; 15 hardware/network-dependent tests remain ignored. Includes existing scene links/migration, chat batches and new logarithmic meter scale bounds.
- Strict workspace/all-target Clippy passed with warnings denied after final UI edits.
- Native horizontal and vertical mixer rendering checked in isolated Xvfb with software Vulkan. Real GPU/game performance and live audio-device behavior require Fedora testing.
- Renderer self-check passed shared textures, both canvas sizes, layer placement/stacking, alpha, visibility, row padding, unused assets and empty scene.
- Stereo meter peaks are calculated from the existing interleaved stereo PCM blocks after scene gain/mute. Meter painting reuses the current UI cadence. No additional browser runtime, audio capture stream, polling service or animation worker was introduced. This is not a measured hardware-performance guarantee.
- Version-4 layout is backed up before applying the Material default. Hidden panels remain hidden. Older migrations and movable/restorable panels are retained.
- Native capture hooks, Steam observer and browser companion retain their previous bytes. ZIP CRC checks and executable permissions verified during packaging.

Not tested here: authenticated Twitch/YouTube posting or streaming, the user's Fedora/KDE/AMD hardware, game frame-time impact, long sessions, all camera/codec combinations, or donation provider integrations. This remains a preview; existing donation integration limitations apply.
