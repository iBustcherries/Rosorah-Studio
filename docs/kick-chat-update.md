# Kick chat, sending and broadcast details (applied on top of 0.31.0-preview.9)

NOT COMPILED OR RUN. These edits were made in an environment without Rust packages or
GStreamer, so only syntax parsing and the browser-side collector were checked.

## What changed
- Kick chat appears in unified chat. Rosorah opens https://kick.com/popout/<your-slug>/chat in the
  existing private browser-chat helper (the same method used for Twitch and YouTube).
  The collector reads `[data-chat-entry]` rows, `.chat-entry-username` and `.chat-entry-content`.
- Kick appears in the chat panel's Send-to chips and the platform filter.
- Messages are sent through Kick's public API (POST /public/v1/chat, scope chat:write).
- Broadcast details has a Kick section: title and category search/update
  (PATCH /public/v1/channels, GET /public/v1/categories, scope channel:write).
- Sign-in now requests `chat:write channel:write` in addition to the existing scopes.
  Kick sign-ins made before this change must sign out and connect again once.
- Kick emote images from files.kick.com are allowed in chat rendering.
- Project files stay compatible: a separate `kick_chat` setting was added instead of widening the
  existing Twitch/YouTube pair.

## Added in 0.32.0-preview.1
- Kick follows, new subs, resubs, gifted subs, Kicks and channel reward redemptions appear in
  activity and chat, and trigger alerts and automations (Kicks use the "Bits / Kicks" trigger).
  They arrive through the sign-in broker's webhook relay; see auth-broker/README.md.
- Delete message for Kick, for messages sent while Kick activity is connected. Rosorah matches
  the chat window's message to the id from Kick's chat.message.sent event by sender and text.
- Automation "Send to chat" accepts Kick.
- Sign-in also requests `moderation:chat_message:manage events:subscribe`; sign out of Kick and
  connect again once.

## Still not included
- Clear chat for Kick (Kick has no API for it).
- Kick events that happen while Rosorah is closed are not replayed.

## Must be verified on a real build
1. Kick's developer application must have the chat:write and channel:write scopes enabled.
2. Kick's popout chat page markup was written from knowledge of its structure, not observed
   live. If chat does not appear, the selectors in browser/collector.js need adjusting.
3. kick.com may show an anti-bot check inside the embedded browser.
4. The deprecated /public/v1/categories search endpoint may need replacing.
5. Compile, then run: cargo test -p rosorah-studio (browser_chat tests were updated for the new
   `kick` source), and a live Kick login, chat read, chat send and title update.
