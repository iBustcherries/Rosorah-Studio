# Validation · 0.23.0-preview.1

## Changes

Independent Ko-fi receiver connection alongside the existing StreamElements/Streamlabs connection. Guided webhook setup with test/live modes, desktop-wallet secret storage, normalized donation events, shared alerts/activity/goals and connection retries. The accompanying single-instance Python receiver uses SQLite for bounded durable delivery and duplicate suppression, private account codes, verification-token checks and minimal donor data.

## Executed checks

- Locked optimized Linux x86_64 release build completed.
- Workspace tests: 130 passed, zero failed, 15 explicitly ignored integration/hardware tests.
- Receiver HTTP integration tests: six passed. Checks cover auth/account isolation, malformed amounts, size limits, setup mode, private donations, duplicate IDs, live/pause behavior, acknowledgements, reconnect replay, process restart persistence, bounded queues, revocation and held-request wakeup.
- Native settings screenshot rendered and inspected: Ko-fi setup, app version and connection controls.
- Native compositor self-check passed with software Vulkan: two canvas sizes, shared textures, placement, stacking, visibility, alpha, padding and empty scenes. Hardware performance is unverified.
- Linux archive integrity and preserved capture/browser/helper bytes are checked during packaging.

Initial local media tests hit a stale GStreamer plugin registry and the build environment's HTTP proxy. With a fresh registry and proxies removed for loopback tests, the full suite passed. No user-system drivers or kernel packages were changed.

## Not yet verified

No public receiver is deployed, no domain or hosting is configured, and no real Ko-fi payment has been received in this environment. Docker/Caddy deployment, a real desktop wallet, Fedora hardware and live provider delivery need testing on the target systems. Existing capture/browser helper binaries are preserved from 0.22. This build does not change GPU drivers, game hooks, audio pipelines or encoding behavior.

Ko-fi test notifications without an explicit test flag are indistinguishable from live payments: pause before testing. The receiver starts in test mode and only enables live mode after a verified notification. It keeps live undelivered events for seven days, so reconnecting may play older alerts. Desktop history is bounded and not an exactly-once financial ledger. Membership payments are Donation events; shop orders, commissions, cancellations, refunds and chargebacks are outside this preview.

See `../relay/README.md` for setup, deployment, retention and operational limits.
