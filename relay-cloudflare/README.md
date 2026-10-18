# Rosorah Ko-fi shared service · 0.24 preview

This replaces per-streamer receiver codes with a Ko-fi verification token. The desktop app establishes one secure WebSocket while it is open. Closing the app closes that connection; no local receiver, tunnel or background daemon is installed. The shared cloud receiver stays online to collect missed activity.

## Streamer setup, once the operator has deployed the service

1. Open Rosorah Settings → Donations → Ko-fi → Connect.
2. Follow the link to Ko-fi's webhook settings, copy the verification token and paste it into Rosorah. Choose Connect Ko-fi. Remember stores only the derived access credential and service address in your desktop wallet, not your project.
3. Copy Rosorah's webhook address into Ko-fi and save it. Send a test notification from Ko-fi.
4. When Rosorah confirms receipt, enable live donations.

Pause before sending further test notifications. Ko-fi has no guaranteed test flag. Payments received in test mode are deliberately discarded. Shop orders, commissions, refunds and cancellations are not imported; donation and membership payments use the Donation event.

Both live and missed Ko-fi donations appear in the shared Activity panel. On connection, up to 1,000 retained donations from the last seven days are delivered as history. New missed donations increase existing matching-currency goals once but do not play media, sounds or other alert actions. Already-seen IDs are suppressed by Rosorah's saved activity history (30 days, 5,000 total events). This is bounded deduplication, not an accounting ledger or a guarantee against duplicates after clearing history or a crash between saves. Goal progress is saved with the project. A newly connected account may import its retained history; it is not restricted to payments since this computer's last session.

StreamElements/Streamlabs live events continue working independently. Their existing direct connections do not import donations received while Rosorah was closed. Offline Twitch/YouTube events are not added by this update.

## One-time deployment by the Rosorah distributor

End users do not need Cloudflare accounts. The distributor needs one free Cloudflare account, and deploys the shared receiver once. No account, public hostname or deployment has been configured in the supplied build.

1. Install a supported Node.js LTS version (22 or newer) on the operator's computer. Extract this folder and open a terminal in it.
2. Run `npm ci`. Run `npm run login` and sign into your own Cloudflare account in the browser. Do not paste Cloudflare credentials into Rosorah or a support conversation.
3. Confirm the Cloudflare Workers account is on the **Free** plan, then run `npm run deploy`. This creates the Worker and its SQLite-backed Durable Object binding, with a free `workers.dev` address. Do not upgrade to a paid plan as part of these instructions.
4. Copy the HTTPS address printed by Wrangler. Verify its `/health` endpoint reports `rosorah-kofi`, version 2.
5. Place `kofi-service.json` beside `rosorah-studio` with `{"url":"https://YOUR-ACTUAL-SERVICE.workers.dev"}`. Package that same non-secret file with future Linux downloads. Alternatively, compile with `ROSORAH_KOFI_SERVICE_URL` set to that origin. The Advanced service field is available for initial operator testing; regular users only need the Ko-fi token once this file is included.
6. Complete the streamer test above with your own account. Public deployment, actual Cloudflare quotas and real Ko-fi delivery still need validation.

The package deliberately contains no invented public service address. The app shows a clear service-not-configured message until this step is complete. A legacy receiver code can still be entered under Advanced for 0.23 servers, but those servers retain their older replay behavior.

## Why Durable Objects instead of D1?

SQLite-backed Durable Objects support a WebSocket that can hibernate while idle. The cloud receiver therefore pushes notifications without repeatedly querying D1 or keeping a polling loop running. SQLite-backed Durable Objects are available on the Workers Free plan, with usage limits. Free is not unlimited: reaching Cloudflare quotas can interrupt delivery until they reset. This deployment does not purchase hosting or enable paid usage.

Official references: [pricing](https://developers.cloudflare.com/durable-objects/platform/pricing/), [WebSocket hibernation](https://developers.cloudflare.com/durable-objects/best-practices/websockets/), [rate limits](https://developers.cloudflare.com/workers/runtime-apis/bindings/rate-limit/), [Ko-fi webhooks](https://help.ko-fi.com/hc/en-us/articles/360004162298-Does-Ko-fi-have-an-API-or-webhook).

## Data, limits and operation

The verification token derives a private access key using SHA-256 with a fixed namespace prefix; the access key's hash routes to the account object. Treat the token and derived key as secrets. Raw tokens, email, shipping addresses and entire webhook bodies are not stored. Public donor names/messages, amount, currency and timestamp are kept for history. Private donations use Anonymous and an empty message. The service does not process money.

No console logging or Worker observability is enabled. HTTP bodies are limited to 64 KiB, history to 1,000 events/seven days of delivery eligibility, duplicate IDs to 5,000 per account, and one app WebSocket per account. Duplicate IDs older than 30 days and expired event payloads are removed on subsequent notifications; this is not a scheduled deletion guarantee for idle accounts. The operator can revoke/delete stored account data using authenticated DELETE `/v1/account`; remove the webhook in Ko-fi as well. On deletion, reconnect with the original token can register again, so rotate the verification token in Ko-fi if it was exposed.

Rate limits are 10 configure requests and 300 overall API/webhook requests per minute per source IP at each Cloudflare location. This is a small-rollout default, not distributed abuse protection. Larger public distribution needs measured capacity, operator monitoring and account recovery/privacy procedures. Offline history depends on Ko-fi successfully delivering to this service; it cannot reconstruct payments that Ko-fi never delivered. Close Rosorah before opening another instance for the same Ko-fi account.

## Validation

`npm test` exercises the actual local Workers runtime via Miniflare: token registration, setup verification, live and missed delivery, privacy, amount rejection, wrong token, duplicate suppression, account isolation, reconnect, persistent live mode and deletion. `npx wrangler deploy --dry-run` validates bundling and bindings without deploying. Live deployment and production free-tier behavior remain unverified.
