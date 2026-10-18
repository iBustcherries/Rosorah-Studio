# Rosorah Studio 0.16.1 validation

Changes: automatic account-linked chat sources; YouTube selection on sign-in/Refresh; per-destination canvas routing, including a second YouTube output; one streaming encoder per distinct canvas; audio fan-out; per-canvas recording and combined output lifecycle/status.

Focused checks cover unique/ambiguous YouTube discovery, explicit selection, account switching and sign-out, browser restart/queue retry, old streaming settings, stable canvas IDs, missing canvas rejection, duplicate YouTube key rejection, and grouping multiple destinations on one encoder. A local dual-output integration check renders separate landscape/portrait video, writes both through the streaming muxer/relay to local MKVs, and decodes both audio tracks. No platform broadcast is started by tests.

Platform OAuth, private-chat sign-in, YouTube dual-stream configuration, and simultaneous hardware encoding on the user's Fedora AMD GPU require user testing. Browser chat retains its separate website session; automatic source configuration does not turn OAuth tokens into website cookies. No new recurring YouTube discovery polling. Stream keys must still match the platform's output keys.

Start/stop controls operate all enabled destinations. A failed canvas output is reported while other outputs may continue until Stop. Destination retries remain independent. Saved routes refer to canvas IDs in the current project; invalid IDs must be reassigned. Existing recording/streaming limits remain 1920x1080 or portrait, up to 60 FPS for streaming.

No capture hooks, kernel packages, desktop configuration, camera driver or browser runtime version were changed.

## Executed checks

- Release workspace suite: 84 tests passed, 9 environment-dependent tests excluded from the default suite.
- The new dual-canvas test was additionally run explicitly and passed: 160x90 and 90x160, stereo audio decoded with nonzero samples in both MKVs. No external platform contacted.
- Strict release Clippy for all workspace targets passed.
- Native UI launched under Xvfb; Main/Vertical previews and all three destination canvas selectors rendered successfully.
