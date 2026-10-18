# Ko-fi simplified setup in 0.24

Use the shared receiver described in `../relay-cloudflare/README.md`. Once its public address is bundled by the distributor, users paste only their Ko-fi verification token, click Connect, and copy the generated webhook address into Ko-fi once. A test verifies the connection before live mode is enabled.

Live donations and missed Ko-fi donations appear in Activity. Missed donations are marked as history, preserve their timestamps, increase matching-currency goals once, and do not replay sounds/media/actions. History is bounded to the most recent 1,000 received payments within seven days at the server. Rosorah's local history suppresses duplicates while retained (5,000 total events/30 days). Do not use this as a financial ledger.

The hosted receiver runs independently; Rosorah opens its connection only while running. StreamElements/Streamlabs continue using their existing direct connection without offline history imports. Older deployment notes below apply only to earlier receivers.

---

# Ko-fi addition in 0.23

Ko-fi connects independently alongside StreamElements/Streamlabs. Open Settings → Donations → Ko-fi → Connect. You need a private connection code from a deployed Rosorah receiver. No hosted receiver is included with the desktop download. See `../relay/README.md` for guided setup and server deployment.

A setup test verifies delivery without increasing goals. Enable live donations afterward. Pause before any further test notifications. Private donor names/messages are hidden, amounts retain their original currency, and duplicate message IDs are suppressed. Membership payments use the Donation event. Live events can be replayed for up to seven days after a disconnect; refunds and shop orders are not imported.

The earlier StreamElements/Streamlabs instructions below remain valid; references to Ko-fi being unavailable describe the previous build.

---

# Donation setup — 0.22.0 preview

## What connects now

StreamElements and Streamlabs can send live donation events directly to Rosorah. One provider is active at a time. This does not require a browser dock or a Rosorah-hosted server. An event token is required; this build does not provide one-click provider OAuth.

Open **Settings → Donations & PayPal setup** and follow Choose → Prepare → Connect → Customize → Preview.

### StreamElements

1. Sign in on the official StreamElements website and finish your tipping page setup.
2. Select the channel that owns that page. In the account/channel settings, copy its channel ID and JWT token.
3. Paste those into Rosorah. Choose **Remember on this computer** to save the connection in your desktop wallet; leave it off for this session only.
4. Click **Connect donation events**. Wait for **Connected to StreamElements**. An open website or a sample preview is not a connected account.
5. Rosorah accepts completed, moderation-approved tip events. Pending and blocked tips do not trigger an alert. Review or approve held tips in the provider dashboard; this build does not subscribe to later moderation changes.

### Streamlabs

1. Sign in and finish your tipping page setup on Streamlabs.
2. Open account settings → API Settings → API Tokens. Copy the **Socket API Token**.
3. Paste it in Rosorah and connect. This is not the stream key, widget URL, or desktop remote-control token.
4. Wait for **Connected to Streamlabs**. Donation events then arrive over the provider's socket connection.

### PayPal, step by step

For this build, use PayPal through your StreamElements or Streamlabs donation page:

1. Open PayPal directly and sign in. Complete any email confirmation or account checks PayPal shows.
2. Pick the donation-page provider you prefer. Open its tipping/payment settings and choose its PayPal connection option where available for your account and region.
3. Follow the provider's sign-in flow. Review the permissions before accepting. Your PayPal password belongs only on PayPal's website; Rosorah does not need it.
4. Finish the donation page: choose the public name, currency, and other available tipping preferences. Follow any remaining requirements shown by the provider.
5. Return to Rosorah and choose that provider, then connect its event token using the steps above.
6. Publish/share that provider's donation-page link with viewers. Payments sent elsewhere, including personal transfers and unrelated PayPal.me links, will not automatically appear.
7. Review receipts and payment status in your payment provider. A sample alert never proves that money changed hands.

Direct PayPal and Ko-fi connections are **not implemented in this build**. They require a public webhook receiver and provider configuration that Rosorah does not currently have. The app keeps them clearly unavailable instead of claiming a successful connection. No hosting service has been deployed.

## Alerts and goals

- Add a video/GIF alert source and select the **Donation** trigger. Leave its platform set to Any, or choose Donations.
- For more control, create a **Donation** rule in Alerts & automation and add a Play media or Update widget action.
- Add a **Donation goal** widget. It increases only for donations in its configured currency; there is no automatic currency conversion. Values are stored in hundredths (1234 means 12.34).
- Customize donor name, amount and message visibility in donation setup. Messages are hidden by default. Donation text supports `{user}`, `{amount}`, `{currency}`, `{message}`. A rule's `{message}` receives that formatted, privacy-filtered donation text.
- Text preview alerts currently appear only in the app. Use media sources or goal widgets for output included in the stream and recording.
- Received donations appear in the combined Activity panel. Provider event IDs are deduplicated through the existing local history, including across normal app restarts, within its 30-day/5,000-event retention limits.

## Connection behavior and limits

Rosorah reconnects automatically with a backoff up to 30 seconds. Remembered connections restore from the desktop wallet on launch. Tokens are not saved in shareable projects or diagnostic logs. Disconnect stops the connection and removes the remembered token when the wallet is available.

Only new events received while connected are processed. Missed donations are not backfilled, and refunds/chargebacks are not automatically deducted. Marked provider test events are ignored; the local sample preview does not increase goals. Provider events are notification data, not an independently verified financial ledger.

If a saved connection will not restore, unlock the desktop wallet or paste a fresh provider token. If remembering fails, you can connect for this session with Remember turned off. Verify that your tipping page and event token belong to the same channel.

## Implementation references

- https://docs.streamelements.com/websockets
- https://docs.streamelements.com/websockets/topics/channel-tips
- https://dev.streamlabs.com/docs/socket-api
- https://help.ko-fi.com/hc/en-us/articles/360004162298-Does-Ko-fi-have-an-API-or-webhook
- https://developer.paypal.com/api/rest/webhooks/
