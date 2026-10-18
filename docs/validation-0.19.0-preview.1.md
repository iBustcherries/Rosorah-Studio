# Validation — 0.19.0-preview.1

- Optimized Linux x86_64 release built from the included source.
- 117 tests passed (20 core, 4 capture, 93 application); 15 hardware/network-dependent tests remain ignored by default. Includes scene migration, independent canvas scene ownership, stable links through reorder, duplication/deletion, serialization, multi-platform chat acknowledgements, partial delivery failures, limits, and preservation of newer drafts.
- Strict workspace/all-target Clippy passed with -D warnings.
- Native application opened under isolated Xvfb/software Vulkan; inspected the new default dock layout, canvas destinations area, scene tabs/context entry points, layer handles, audio controls, and compact Broadcast panel.
- Release --self-check passed two canvas sizes, shared textures, layer placement/stacking, visibility, alpha, row padding, unused assets and empty scenes.
- Movement is presentation-only: 160 ms on-demand interpolation. Layer ordering updates the project once on drop; intermediate animation does not mutate the recording/stream layout. Panel movement reuses preview texture dimensions until settling. No additional browser process, capture pipeline, encoder, polling service or animation worker was introduced. These are implementation properties, not a measured hardware performance guarantee.
- Native Vulkan/OpenGL hooks, Steam observer and browser companion retained byte-for-byte from 0.18.0-preview.1.
- ZIP CRC checks, executable modes and TAR content hashes verified when packaged.

Build-environment issues resolved: local dependencies used instead of system graphics-package installation; localhost proxy settings disabled for camera tests; regenerated test executables had their execute bits restored before running; a legacy dock migration fixture was corrected to use finite saved geometry; release sources were staged in a fresh directory after older workspace files were being restored during compilation.

Not validated here: an authenticated Twitch/YouTube live posting session, the user's Fedora/KDE/AMD hardware, game frame-time impact, long streams, donation-provider connections, or all available media codecs. Test the preview before relying on it for a live broadcast. Donation integrations retain the limitations documented in the prior preview.
