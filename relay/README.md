# Rosorah Ko-fi receiver · preview 1

This is the server component for Rosorah Studio 0.23. It is not a hosted service yet. The supplied app cannot receive Ko-fi payments until an operator deploys this receiver and provides a private connection code. StreamElements continues working independently.

## For streamers

1. In Rosorah, open Settings → Donations, choose Ko-fi, and open Connect.
2. Paste the private Rosorah connection code supplied by your receiver operator. Do not share it publicly.
3. Open https://ko-fi.com/manage/webhooks, copy the verification token into Rosorah, and click Connect Ko-fi receiver. Remember uses your desktop wallet, outside scene projects.
4. Copy the webhook address from Rosorah into Ko-fi and save it. Send a Ko-fi test notification.
5. Wait for “Ko-fi test received”, then click Enable live donations. Set your Donation alerts and matching-currency donation goals as usual.

Pause donations before sending subsequent Ko-fi tests. Ko-fi's documented example has no reliable test flag, so an unmarked test sent in live mode can look like a real payment. Test mode discards payments; it is not a holding queue. Reconnecting with a blank verification-token field keeps the receiver configuration; entering a token resets it to test mode.

StreamElements/Streamlabs and Ko-fi can run together. Avoid forwarding the same payment through both. Donation and membership payments use the Donation event; shop orders, commissions, cancellations, refunds and chargebacks are not imported. Private Ko-fi donations display Anonymous with no message. Currencies are kept separate; no conversion takes place.

While live, undelivered events are retained for up to seven days and delivered when Rosorah reconnects. This can play old alerts on reconnect. Use one running Rosorah instance per connection. This preview is an alert feed, not an accounting ledger or an exactly-once financial record.

## Operator deployment

These steps belong on a hosted Linux server, not the streamer's gaming PC. Hosting and a domain have not been selected or purchased. Obtain a server with Docker Engine and the Compose plugin, and a domain/subdomain pointing to it. Ports 80 and 443 must reach the server. See the official [Docker installation guide](https://docs.docker.com/compose/install/) and [Caddy HTTPS guide](https://caddyserver.com/docs/quick-starts/https).

1. Copy this folder to the server. Create `.env` containing `RELAY_DOMAIN=your.actual.domain` (no scheme or slash).
2. Run `docker compose up -d --build`. Caddy terminates HTTPS; the receiver's port is only available inside the Compose network.
3. Check `https://your.actual.domain/health` returns service `rosorah-kofi` and version 1.
4. Create one private connection per streamer with `docker compose exec -T receiver python server.py --create-connection`. Deliver the resulting code privately. The operator does not need the streamer's Ko-fi password or verification token.
5. Complete the streamer setup above, then verify a genuine payment after enabling live mode. No real account/payment test or public deployment was performed in the build environment.

There is no open signup endpoint in this preview: provisioning is operator-only. Do not publish a shared connection code. Before offering a public service, establish account recovery, abuse controls at the proxy, service monitoring, a privacy policy, backups and an operator support process. The Compose configuration is supplied for deployment, but Docker/Caddy deployment has not been exercised in this build environment.

## Data and operations

The SQLite volume holds hashed access/verification tokens, minimal normalized event data, delivery acknowledgements and duplicate IDs. The connection code itself contains a bearer secret: protect it like a password. Names/messages for public donations are retained up to seven days; duplicate IDs up to 30 days. Email, shipping and raw payment payloads are discarded. Access logging is disabled. Protect and expire backups separately; deleting live records does not erase backups.

There is a 64 KiB request limit, 10,000 pending-event limit and 50,000 duplicate-ID limit per account. Full queues return retryable errors rather than silently accepting more events. SQLite is intended for a small single receiver deployment; do not run multiple replicas against this volume. The client uses one held-open request per account, wakes on receipt, and backs off on disconnections; it does not poll Ko-fi's API. Network acknowledgement retries can redeliver events; Rosorah's bounded local event history suppresses previously seen IDs.

Disconnect in Rosorah stops its connection and attempts to pause the receiver. Remove the webhook in Ko-fi to stop forwarding. To delete receiver data and revoke a code, send authenticated `DELETE /v1/account` using the key decoded from the private code; never put it into logs or a support message. After deletion, generate a new code and repeat setup to reconnect. Keep the persistent `relay_data`, `caddy_data` and `caddy_config` volumes across upgrades. Stop the receiver before copying its SQLite database and WAL files for a consistent backup.

## Local tests

Use Python 3.12+ with `pip install -r requirements.txt`, then `python -m unittest discover -v`. The tests use temporary databases and loopback HTTP; the production desktop client requires HTTPS and does not disable certificate validation. Tests cover authentication/isolation, private fields, safe test mode, amount validation, duplicate suppression, acknowledgements, restart persistence, queue limits and immediate held-request wakeup.

Protocol reference: [official Ko-fi webhook documentation](https://help.ko-fi.com/hc/en-us/articles/360004162298-Does-Ko-fi-have-an-API-or-webhook).
