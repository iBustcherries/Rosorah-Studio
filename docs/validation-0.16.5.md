# Rosorah Studio 0.16.5 validation

## Changes

One central OAuth account powers broadcast controls, activity, stream-key discovery and explicit outgoing chat. Renewable tokens use the desktop Secret Service through dynamically loaded libsecret; no plaintext token fallback. A non-secret client/provider marker controls automatic restore. Cancellation and account replacement serialize worker lifetimes so old sign-out cleanup cannot erase a later login. Transient restore/identity network failures retry without launching authorization windows. Explicit sign-out removes remembered credentials and account-managed destination keys.

The browser remains responsible for incoming chat and emote images. Both composers route explicit sends through the account worker. Requests are bounded, acknowledgement-correlated and never retried automatically. A changed YouTube broadcast rejects an already queued send; failed/unknown sends preserve the draft. Tokens are never sent into browser IPC. Obsolete website input automation was removed.

YouTube metadata discovery follows the only active broadcast and preserves a valid choice when several are live. It runs at 60 seconds active / 120 seconds idle, with a short 10-second burst after encoder start, exponential failure backoff and a one-hour quota-error pause. No liveChatMessages.list polling was added. Outgoing YouTube messages use insert (20 quota units per send). New explicit broadcast creation automatically creates/binds an encoder key when none was selected; automatic creation/publishing and retrying creation are not performed.

Browser runtime verification/setup happens automatically, reuses an installed pinned runtime, and retries failures. The native panel has an empty-state account shortcut, messages, composer and overflow menu. Details/recovery controls live in Accounts. A bundled accounts-defaults.json is supported, but no publisher registration values or user tokens are included in these archives.

## Verification

- Release workspace suite: **94 passed, 0 failed, 9 environment-dependent tests ignored**. Includes shared-account send routing, cancel/no-retry classification, stale broadcast selection, ambiguity, quota backoff, composer acknowledgement/draft handling, hidden child launch/recovery, source switching, camera/GStreamer and existing canvas/stream routing regressions.
- Strict release workspace/all-target Clippy passes.
- Seven Python tests pass for companion lifecycle, isolated bridge delivery, stale-source rejection, shared-account send forwarding, input rejection, and real PNG/GIF/WebP decoding and URL restrictions.
- JavaScript fixtures pass for Twitch/YouTube parsing, emote fragments, paid events, deletes, deduplication, safe rendering, composer routing, target availability, draft preservation and acknowledgements. A 350-message DOM burst reaches the sink without browser timers; this fixture is not a platform latency measurement.
- Python syntax checked. Native Studio and Accounts layouts rendered locally with Xvfb/software Vulkan. The normal disconnected Chat panel offers a single account shortcut; the connected composer uses a compact input/Send row.
- Linux binary ZIP, tar.gz and source ZIP verified member-by-member for exact file contents and permissions. Capture/launch helpers retained byte-for-byte from the working 0.16.4 package.

## Verification limits

No signed-in Twitch/Google session was available here, no real messages or streams were sent, and Fedora KWallet token save/restore was not integration-tested. Secret Service errors remain visible and allow session-only operation. Publisher client registration values are absent, so the package uses the existing user's accounts.json; a genuinely clean distributed installation needs the publisher defaults first. Provider approval, account permissions, quota and private-chat website restrictions remain external requirements.

The Qt companion was tested with real image decoders and mocked native lifecycle widgets plus JavaScript fixtures; live platform pages and a full QtWebEngine session were not tested. No Chromium security sandbox was disabled. Website markup and platform delivery timing can change. Camera/game/streaming hardware was not available; existing hardware-dependent tests remain explicitly ignored. This build changes no kernel, graphics driver, native renderer hook, or Steam launch configuration.
