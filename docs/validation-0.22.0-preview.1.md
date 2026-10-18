# Validation — Rosorah Studio 0.22.0-preview.1

- Rust workspace compiled successfully with Rust 1.96 and the locked dependencies.
- 128 automated tests passed; 15 hardware/environment-dependent tests remain ignored.
- Donation fixtures checked exact decimal handling, rejected/pending/moderated/test events, privacy filtering, persisted duplicate protection and currency-specific goal updates.
- Service-name persistence and preservation through per-canvas encoder grouping checked.
- Local chat timestamps checked in an isolated America/New_York process across winter/summer offsets, with both 12/24-hour formatting.
- Native UI rendered for Studio, the centered service chooser, and donation connection setup. Canvas 1/2 labels visible; existing project IDs and dimensions retained.
- GPU self-check passed on software Vulkan: two canvas sizes, textures, layer placement/stacking, visibility, alpha, row padding, unused assets, empty scene.

The initial test environment mixed local Debian libraries with Ubuntu FFmpeg; tests were rerun successfully with isolated test-only FFmpeg wrappers using system libraries. These wrappers and build dependencies are not shipped.

No signed-in StreamElements/Streamlabs account, wallet-unlock interaction, real donation, hardware GPU, or Fedora game capture session was available for live validation. Provider delivery must be checked on the user's account. Direct Ko-fi/PayPal are unavailable pending a hosted receiver; no server was deployed. Hooks and browser companion are preserved from 0.21.0-preview.1.
