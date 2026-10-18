# Rosorah Kick sign-in service — publisher setup

This is a prepared implementation, not a deployed service. No registered client
ID or secret is supplied. It supports both Windows and Linux Rosorah clients.
Regular app users will click Connect and approve Kick; they do not create developer
applications or enter developer tokens.

## One-time publisher setup

1. Create the Rosorah application in Kick Developer settings. Register the exact
   redirect URL `http://localhost:17653/oauth/callback`.
2. Request the `user:read`, `channel:read`, `streamkey:read`, `chat:write`,
   `channel:write`, `moderation:ban`, `moderation:chat_message:manage` and
   `events:subscribe` permissions.
3. Deploy `worker.mjs` using `wrangler.jsonc` to a publisher-owned Cloudflare Worker.
   Add `KICK_CLIENT_ID` and `KICK_CLIENT_SECRET` as Worker secrets, through the
   Cloudflare dashboard or `wrangler secret put`. Never put the secret in the desktop
   package, a project, a source commit, a command-line argument, or a chat message.
4. Add the public ID and deployed HTTPS base URL as `kick.id` and `kick.broker_url`
   in the publisher's `accounts-defaults.json` beside both app executables. Preserve
   the existing Twitch and Google registrations. Do not distribute the example
   placeholders as real defaults.
5. For Kick activity (follows, subs, gifts, Kicks, reward redemptions and Delete message),
   edit the Kick app, turn on **Enable Webhooks** and enter the deployed Worker's
   address followed by `/webhook`, for example
   `https://rosorah-kick-auth.YOUR-ACCOUNT.workers.dev/webhook`.
6. Sign in on Windows and Linux; test cancellation, wallet restore, key retrieval,
   token renewal and sign-out before releasing installers to users.

The Worker forwards only code/PKCE or refresh grants to the fixed Kick token
endpoint. It does not store tokens, log bodies, accept a caller-specified redirect,
or follow HTTP redirects. The desktop keeps refresh tokens in Windows Credential
Manager or Linux Secret Service. The Worker has a per-IP rate limit; users sharing
an address may share that allowance. OAuth refreshes are sparse, not chat polling.
Requests have bounded sizes and an upstream timeout. Deployment and provider
authorization have not been tested live.

## Activity relay

Kick sends events only as webhooks to one public address per app. `/webhook` checks
each delivery's `Kick-Event-Signature` against Kick's public key
(https://api.kick.com/public/v1/public-key, refetched once if Kick rotates it), refuses
deliveries older than an hour, and passes the event to a `KickEvents` Durable Object
named after the channel's broadcaster id. The desktop app opens a WebSocket to `/events`
with its Kick access token; the Worker confirms the token with Kick (active, issued to
this app) and joins it only to its owner's channel. Events are relayed, not stored, so
anything sent while the app is closed is not replayed. The app subscribes the channel
to events itself the first time it connects.

## Local checks

Run `node --test test.mjs` for request validation, PKCE forwarding, refresh handling,
response filtering and rate limiting tests. The application's actual OAuth helper
tests run through `cargo test --manifest-path ../validation/oauth-harness/Cargo.toml`.

## Scope

Kick sign-in, ingest setup and the activity relay. Chat is read through the chat
window; sending chat and broadcast editing use the user's own token directly. OAuth authorization does
not transfer a browser website's login cookies into the chat browser.

Official contracts:
- https://docs.kick.com/getting-started/generating-tokens-oauth2-flow
- https://docs.kick.com/apis/channels
- https://docs.kick.com/events/webhook-security
- https://docs.kick.com/events/event-types
- https://developers.cloudflare.com/workers/runtime-apis/bindings/rate-limit/
