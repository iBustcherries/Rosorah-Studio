# Account sign-in update for Windows and Linux

Status: preview.9 has the publisher Kick client ID and deployed HTTPS token
service built in. Native end-to-end sign-in validation is still required.

- Twitch retains device authorization and wallet-backed session restoration.
- YouTube retains Google desktop OAuth and PKCE. Randomness now uses the platform
  secure random generator instead of opening `/dev/urandom`, fixing the Windows
  authorization path. Callback parsing rejects empty codes and duplicate parameters.
- Kick uses browser OAuth/PKCE, a publisher-hosted token broker, the existing
  platform credential vault, and automatic token renewal. A third account card
  appears in Settings on both platforms. Missing registration/service setup is
  shown honestly and disables Connect.
- Successful Kick authorization retrieves the channel and ingest server/key.
  A Kick destination is created if absent. Account-managed destinations refresh
  from the session; existing manually entered keys are retained unless the user
  chooses Use connected Kick account. Stream routes and canvas assignments remain
  intact. Sign-out removes saved local tokens and clears account-managed keys.
- The public desktop defaults contain the broker URL and public Kick client ID,
  never the Kick client secret. Publisher setup is in `auth-broker/README.md`.

The existing Windows and Linux account vaults are reused. Sign-in does not imply
Kick chat/activity support, which remains unfinished. Browser chat website cookies
are separate from OAuth tokens and may still require website sign-in.

Validation: Linux all-target cargo check; actual OAuth helper tests through a
standalone harness; Node token-broker tests. The deployed service returned the expected 405 for GET /token and 400 for an
incomplete authorization request. Windows and Linux desktop sign-in still need
end-to-end validation. See release-0.31-preview9.md for final build validation.
