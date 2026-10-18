# Validation — 0.18.0-preview.1

This preview contains local automation/widget features and donation onboarding preparation. It does not include live donation integrations or a deployed hosted relay.

- Workspace tests: 113 passed, 15 ignored by default. Includes legacy-project migration, phrase boundaries/case/user/reward matching, first-message reconnect and stream reset behavior, multiple rule kinds from one message, deduplication, privacy-aware sample templates, scoped subscriber counts, counter saturation, widget RGBA rendering, shared widget frame publication/update and stopping hidden renderers.
- Strict workspace/all-target Clippy passed with `-D warnings`.
- Release native UI opened under Xvfb/software Vulkan. Inspected donation chooser and PayPal preparation screens, and native widgets on both canvas previews. This is not a hardware performance benchmark.
- Native `--self-check` passed shared-texture, canvas-size, placement, alpha, visibility and stacking checks.
- Explicit real media output integration test passed: video file sound, per-scene audio mixing and transitions reach the MKV recording.
- Two localhost camera tests initially failed because the environment routed them through a proxy. Both passed with proxy variables unset, as did the full suite.
- A damaged LLVM graphics-library extraction in the isolated build environment was repaired before UI validation. No operating-system packages, kernels or drivers on the user's Fedora machine were changed.
- Existing packaged native Vulkan/OpenGL hooks and Steam launch observer are retained byte-for-byte from 0.17.1. Browser companion files are unchanged.
- ZIP contents/CRC and TAR file hashes verified during packaging; executable permissions preserved.

Not validated here: live Twitch/YouTube account events on a signed-in account, Fedora AMD hardware performance, PayPal payments, donation-provider authentication/webhooks, a public hosted service, all media codecs or long streaming sessions. The user-facing guide lists specific preview limitations.
