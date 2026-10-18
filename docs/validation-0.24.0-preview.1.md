# Validation · 0.24.0-preview.1

- Locked optimized Linux build.
- Rust workspace: 132 tests passed, zero failed, 15 existing integration/hardware tests ignored. New tests cover token-derived connection setup, HTTPS enforcement, missed timestamp metadata and persisted donation deduplication.
- Cloudflare receiver: one end-to-end test scenario passed in the actual local workerd/Miniflare runtime. It exercises token registration, setup-test gating, live delivery, missed activity, original timestamps, private donor masking, wrong-token rejection, malformed amount rejection, duplicate suppression, account isolation, reconnect and revocation.
- Wrangler dry-run successfully bundled the service and validated Durable Object and rate-limit bindings. No cloud deployment was performed.
- Native donation settings screenshot and compositor self-check are included in local build verification. Tests use software Vulkan; target GPU performance is unverified.
- Packaging checks archive integrity and keeps game-capture/browser/helper binaries byte-for-byte identical to 0.23.

The cloud service has no configured account or public URL in this build. A real Ko-fi account/payment test, Cloudflare deployment, live free-tier quotas and the user's desktop wallet remain unverified. End users get the token-only flow after the distributor bundles the deployed service address. There is no installed local background service.

Missed Ko-fi donations update activity and existing matching-currency goals without firing alert actions. Goal totals save with the project. The history and project are separately saved, so a crash between those saves is not an exactly-once financial transaction. Deduplication is bounded by the saved Activity history (5,000 events, 30 days); clearing it can allow retained server events to be counted again. The server delivers at most 1,000 retained donations from seven days; it does not fetch older Ko-fi payment history. StreamElements/Streamlabs offline history is not provided by their existing direct connection.

The initial local media test failures were caused by the build environment's stale GStreamer registry and loopback HTTP proxy. The full workspace passed using a fresh registry and direct local requests. No user drivers or kernel packages were changed.
