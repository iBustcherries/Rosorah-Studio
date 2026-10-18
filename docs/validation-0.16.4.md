# Rosorah Studio 0.16.4 validation

## Changes

The browser collector uses a DocumentReady script in Qt's isolated ApplicationWorld. MutationObserver sends messages through QWebChannel without setTimeout, polling or requestAnimationFrame on the primary delivery path. The parent Rust reader requests repaint on incoming events and uses a bounded channel with backpressure. A five-second local DOM scan is recovery only. Chat pages stay Active and visible to Chromium when their widget is hidden; background timer and renderer throttling are disabled. No Chromium security sandbox changes were made.

Messages carry bounded text/image fragments. Native emotes are fetched asynchronously from a restricted platform CDN allowlist, decoded using Qt, then sent as bounded PNG animation frames to Rust. HTTPS, restricted YouTube emoji paths, no credentials in URLs, no redirects in the native image downloader, download/size/frame/cache limits and text fallbacks are applied. Browser images retain their original animation; the native view uses 64px thumbnails and up to 120 frames. Unicode text and emoji are preserved. No third-party emote extensions/catalogs are installed.

Both the native dock and combined browser window have a per-platform composer. Sending invokes the existing website editor and Send button only after an explicit user action. Character limits, sign-in/missing editor, disabled inputs, unfinished platform drafts and failed/unknown results preserve drafts. Requests are not automatically retried. Submission is distinguished from server-confirmed delivery. Recent emotes are available locally; full account-specific pickers remain available in platform tabs, particularly for YouTube custom emotes that need native website selection.

The native dock moves detailed connection/status text into its existing dropdown to leave space for its composer. Prior automatic startup/restart, canvas, camera, capture, audio and streaming features remain included. No kernel/system package/capture hook changes.

## Checks

- Release workspace Rust suite: 91 passed, 9 environment-dependent tests ignored. Includes real local child-process background launch/restart/IPC tests, source switching, composer routing, acknowledgement correlation, draft preservation, character limits, and restricted/bounded rich fragments.
- Strict release Clippy for the workspace and all targets passes.
- Five Python lifecycle/bridge tests pass: hidden launch/close, recovery, immediate bridge delivery, stale-source rejection, current-platform sending and no automatic retries.
- Two real Qt image tests pass, decoding PNG, animated GIF and animated WebP with frame timing; URL restrictions tested. Installed Qt decoder list also includes SVG.
- JavaScript fixtures pass for Twitch/YouTube parsing, emoji/emote fragments, paid events, deletes, deduplication, image updates, safe DOM rendering, routing, editor restrictions and composer acknowledgements.
- A 350-message DOM mutation burst reaches the event sink without any browser timers. The jsdom fixture completes in about 0.3–0.6 seconds including constructing/parsing the entire synthetic DOM. This is not a measurement of platform or real browser end-to-end latency.
- Python syntax and Qt's bundled WebChannel resource availability checked.
- Native studio UI rendered locally using the software Vulkan device for layout inspection.
- Binary ZIP, tar.gz and source ZIP checked member-by-member for content hashes and file permissions. Existing capture/launch helpers compared byte-for-byte with the known working baseline.

## Limits of verification

No signed-in Twitch/YouTube session was used and no real chat messages or streams were sent. Browser behavior uses fixture/mocked-widget tests, not a live QtWebEngine renderer session. Website editor selectors, authentication, custom-emote insertion, hidden-page behavior and end-to-end delivery latency need testing on the user's Fedora system. Platform markup can change. Platform and network delays cannot be removed by Rosorah. Native image limits may truncate unusually long animations; third-party 7TV/BTTV/FFZ catalogs are outside this build.
