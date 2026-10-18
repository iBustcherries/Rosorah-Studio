# Validation — 0.21.0-preview.1

- Linux x86_64 optimized native release built from included source.
- 122 automated tests passed (20 core, 4 capture, 98 app); 15 hardware/network tests remain ignored. New coverage: backward-compatible streaming settings, additional-service serialization, per-canvas encoder grouping, missing canvases, duplicate destinations, secret-safe validation and connection count limits.
- Strict workspace/all-target Clippy with warnings denied.
- Native rendering inspected under isolated Xvfb/software Vulkan: visible Broadcast destination rows, canvas preview controls removed, compact adjacent mixer strips, layer-row drag affordance.
- GPU renderer self-check passed: both canvas sizes, shared textures, stacking/placement, alpha, visibility, row padding, unused assets and empty scenes.
- Existing capture hooks and browser companion preserved. Archive CRC/content and executable-mode checks performed during packaging.
- Existing private streaming settings file (0600) holds keys outside shareable projects. Credentials were not printed or added to source/package.

Not verified: actual Kick/TikTok/custom RTMP ingestion, authenticated Twitch/YouTube live sessions, signed-in chat composer behavior, Fedora/KDE/AMD performance, or long streams. No Kick/TikTok OAuth or chat/activity integration is claimed. TikTok streaming is conditional on the platform issuing an encoder server/key to the account.
