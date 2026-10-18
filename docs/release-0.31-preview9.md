# Rosorah Studio 0.31.0-preview.9 — Kick account connection

## What changed

- Kick browser sign-in is available in Settings → Accounts on Windows and Linux.
- The public publisher client ID and deployed HTTPS token broker are built in.
  The Kick client secret is never shipped in Rosorah or the source package.
- Sign-in uses PKCE and a validated localhost callback. Tokens are renewed and
  saved in Windows Credential Manager or the Linux desktop secret service.
- A successful connection supplies the Kick streaming server and stream key.
  A Kick service is added if absent; manual keys are retained unless the user
  chooses "Use connected Kick account". Choose its canvas and enable streaming
  through the existing Broadcast/Streaming services controls.
- Sign-out clears local saved tokens and account-managed Kick stream keys.
- YouTube OAuth uses the OS random generator on Windows instead of /dev/urandom.
  Callback parsing also rejects duplicate parameters and empty authorization codes.

## Try it

1. Close Rosorah before updating. Install the Windows Setup EXE, or extract the
   complete Linux archive and run run-studio.sh from the new folder.
2. Open Settings → Accounts → Kick → Connect.
3. Approve access in the browser and return to Rosorah.
4. Check that your Kick channel appears connected and its streaming service has
   the account-provided server/key. Choose Canvas 1 or Canvas 2 as desired.
5. Close and reopen Rosorah to check sign-in restoration. Do not share tokens,
   stream keys, or the full OAuth callback address when reporting an error.

Existing projects and user account configuration are stored outside the install
folder and are retained. Linux needs a working desktop secret service for saved
sign-ins; a locked or missing wallet is reported by the account panel.

## Scope

This is a sign-in and streaming-connection preview. Kick chat, activity events,
and broadcast editing are not implemented in this update. Existing Twitch and
YouTube functionality is retained. Website chat cookies remain separate from
OAuth, and the existing Python/PySide6 chat runtime requirement remains.

## Validation

- Windows x64 optimized release application, probe and launcher cross-builds passed.
- Linux all-target compilation check and optimized release build passed.
  The release executable ran --help successfully and resolved all dynamic libraries
  in the build environment. No live desktop/GPU session was available.
- Three tests of the actual OAuth helper passed via the isolated harness.
- Six broker tests passed, including PKCE forwarding, refresh, request rejection,
  response sanitization, size/rate limits, and missing publisher configuration.
- Both 32-bit and 64-bit Linux native capture hooks compiled; hook installer tests passed.
- The deployed broker returned 405 for GET /token and 400 invalid_request for an
  incomplete authorization request. These checks do not prove the secret is
  correct or replace a live account login.
- Native Windows installer execution and Windows/Linux browser-to-app Kick login
  have not been run here. The Windows installer is unsigned, as in prior previews.
