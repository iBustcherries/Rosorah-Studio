> Current alerts editor: see [automation-guide.md](automation-guide.md) for 0.25 behavior. The text below is historical.

> Update: donation connections changed in 0.22.0. See [donations.md](donations.md). The original preview notes below describe 0.18 behavior.

# Rosorah Studio 0.18.0-preview.1 — local alerts and guided donation preparation

This is a preview of the alerts work, not the complete Firebot-style system. It preserves the working capture, streaming, recording, chat, audio and canvas features from 0.17.1. Existing projects load with defaults for the new fields. Back up your project before opening a preview release; older builds cannot read the new widget/action types.

## What you can use now

- Live Twitch/YouTube activity dispatches enabled automation rules; historical activity does not replay rules.
- Literal chat words/phrases, exact messages, case sensitivity and specific display-name filters.
- A viewer's first message in the current stream/session. Starting a stream resets the session; reconnecting does not. A manual reset is available in Alerts & automation. Twitch's first-ever chatter flag is a separate trigger.
- Channel-point matching by exact reward ID, plus existing raids, follows, subscriptions, gifts, bits, memberships and Super Chat event kinds.
- Play a media source, update a widget/counter/timer, show a preview-only text notice, or switch a scene. Matching rules use the existing cooldown and bounded deduplication.
- Native canvas widgets: chat text, subscriber event goals, manual donation goals, countdowns, death/custom counters and shoutouts. These appear in recordings and streams and can be resized and layered like other sources.
- Widgets share one frame producer per source across canvases and outputs. Unchanged widget images are not rerasterized; hidden widgets stop rendering. Counters are retained when the project is saved or Rosorah closes normally. Countdown clocks reset when reopening Rosorah.
- Donation preparation wizard: choose StreamElements, Streamlabs, Ko-fi or PayPal; read provider-specific steps; open official sites; choose sample display privacy; preview sample text; resume saved progress. Wizard edits save after a short pause and on normal close.

## Create a media alert

1. In Studio, add a **Media file** source to the scene that should show the alert.
2. In its properties, enable **Controlled by automation rules** and enable sound if wanted. Keep its layer visible. The source waits until triggered.
3. Open **Alerts & automation**, create a rule, choose its event and filters, then add **Play media action** and select the source.
4. Use **Trigger test event**. Tests run scene/media/widget actions locally; they do not send chat, submit payments or verify donation delivery.
5. Keep the source in each scene where it should appear. A single source can be shared by both canvases. Each source queues at most 16 media triggers using the existing player.

Media decoding uses the installed GStreamer codecs. MP4/WebM/MKV/MOV/AVI, GIF and common audio extensions can be selected, but support depends on the codec plugins present. Not every format has been tested and no universal codec guarantee is made. The existing preview text action remains preview-only; use a media source or shoutout widget for on-stream content.

## Widgets

Use **Add source → Widget**. Select the resulting layer for its controls. A counter can be changed manually or by an **Update widget** rule. A countdown can be started manually or by a rule. For a shoutout, set the action text to e.g. `Go follow {user}!`; this displays on the canvas and does not post a platform-native shoutout.

Select one platform or both in chat/goal properties. The chat widget keeps six new messages as plain text; inline emote images are not yet supported in this new canvas widget. Existing chat-dock emotes are unchanged.

Subscriber goals count verified Twitch new subscriptions and gifted totals received during this app session, starting at your chosen baseline. Gift-recipient subscription events and resubscription messages do not double-count a gift. Existing account totals are not fetched. YouTube membership quantities are not currently reliable in the browser collector, so YouTube goal values are manual in this preview.

Donation goals are manual until provider connections are implemented. Values use hundredths and display the chosen currency; no currency conversion or cross-provider aggregation runs. Do not interpret the selector as a verified connection.

## PayPal and donation connections: still pending

**No hosted donation relay has been built or deployed, and no provider OAuth/event connection has been implemented in this preview.** The wizard is useful preparation and sample customization, not an operational donation integration. It explicitly displays “Connection is not active” and never treats visiting a provider site or running a sample as successful verification. It stores no passwords, API secrets or payment credentials.

The next donation work requires a hosting/domain decision, service account/application registration for the supported providers, secure account authorization and token storage, verified webhook routing, reconnect/deduplication, cancellation/refund handling, and real sandbox end-to-end tests. PayPal platform onboarding needs PayPal approval before live use. PayPal webhooks cover the configured integration's events, not all arbitrary personal-account transfers.

Publisher references checked 2026-10-11:
- https://developer.paypal.com/platforms/get-started
- https://developer.paypal.com/api/rest/webhooks
- https://docs.streamelements.com/websockets
- https://dev.streamlabs.com/docs/socket-api
- https://help.ko-fi.com/hc/en-us/articles/360004162298-Does-Ko-fi-have-an-API-or-webhook

Other work still pending: full effect sequencing/queues across multiple actions, native chat posting/shoutout actions, automatic donation goal updates, live provider authorization and connection status, and full media/emoji compatibility coverage. The 46-item backlog remains out of scope.
