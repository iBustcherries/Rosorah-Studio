# Widgets in Rosorah Studio 0.27.0-preview.1

Add a source using **Add layer → Widget**, then select it in Layers to edit its settings. Widgets can be resized, positioned, duplicated as layers and locked using the existing canvas tools. A shared source has shared contents; use separate sources for independent settings. New widgets start with their native aspect ratio on either canvas.

## Sports: NFL and NBA

Choose **Sports scores · NFL / NBA**. In its settings, follow **Get a free BALLDONTLIE key**, create an account with that provider, and paste its API key into **Save sports connection**. This is one connection for all sports widgets; it is saved separately from your project. Disconnect removes the saved key contents. Rosorah does not require a paid subscription for the games endpoints used here.

Choose leagues, enter comma-separated team names or abbreviations (leave empty for all), and enable live, upcoming and/or final games. The widget requests yesterday, today and tomorrow in system-local calendar dates. Choose rotating cards, a scoreboard that pages through games, or a scrolling ticker. Change card duration, ticker speed/direction, font, bold text, text/background/accent colors, corner radius and background alpha. The Team Colors preset uses an editable team-style accent; it does not fetch logos or automatically infer each team's colors.

One application-wide cache makes two requests approximately every 65 seconds while at least one sports widget is visible. It backs off for three minutes on service errors. Copies across canvases do not create independent requests. Provider updates can add delay; this is not a real-time game clock. Old scores are marked stale after 150 seconds. No key, unavailable data and empty filters have explicit messages. No synthetic scores are shown as live data. The provider can change plan limits. Up to 100 games per league across the requested dates are supported in this preview.

NFL and NBA are enabled. League settings, score records and rendering are separate, so future leagues can be added through another provider adapter without replacing your widget styles. Other leagues are not enabled yet.

Provider documentation: https://docs.balldontlie.io/ and https://nfl.balldontlie.io/.

## Chat and visual styles

Presets include Unified Chat, Windows 98, Windows XP, Phone Bubbles, Minimal Transparent, Stream Cards, Retro Terminal and Pixel RPG. Broadcast Sports, Team Colors and Retro LED are also available. Presets are starting points: font, size, bold, colors and opacity remain editable. Chat layout can use cards, compact lines or bubbles, with spacing and order controls.

Chat widgets show new messages received by the connected chat session. This canvas renderer is text-only in this preview: it does not yet include avatar images, inline emote images or per-message timestamps. The regular chat dock retains its existing richer rendering. The Unified preset uses platform-colored messages; it is not a pixel-identical reproduction of an external OBS plugin.

## Goals and utility widgets

Subscriber and matching-currency donation goals support manual, per-stream, weekly, monthly and inclusive end-date periods. Weekly reset day and system-local calendar periods are configurable. Stream resets happen at a new stream start, not a destination's internal reconnect. Totals and period keys persist with the project. Historical donations from another week/month do not enter the current calendar goal. Replayed alerts and preview alerts do not increase goals.

Goal styles: horizontal/vertical bar, progress ring, thermometer, milestones, segmented, counter card, pixel hearts and growing garden. Goals remain visible when completed; fill is capped at 100% while the numeric total can keep growing. Community unlock goals offer manual/automation increments or verified follow, bits, gift, subscription, membership and public subscriber event counting. These are event totals, not a fetched lifetime follower/subscriber balance. Private or unavailable platform events cannot be counted. Donation values are in hundredths and currencies are never merged or converted.

Other widgets:
- Recent events, latest supporter and scrolling credits use the connected activity feed received while Rosorah is open.
- Countdown or count-up stopwatch, death/custom counter, and shoutout display.
- Stream labels, schedule cards and now-playing labels accept manual text or Update widget automation. There is no automatic music-player integration in this preview.
- Poll/prediction cards accept text and connected poll/prediction event descriptions. They do not create polls, cast votes or independently query platform results.
- Challenge checklist has editable text and checkboxes in its source controls.
- Rotating announcements display one non-empty line at a time.
- QR codes encode your URL/text locally with a white quiet zone; preserve the source aspect ratio when resizing so the code remains scannable.

## Alert Box

An Alert Box is a resizable widget layer. It displays connected activity and Show alert automation text, with a bounded queue. Choose included platforms and event overrides for text, duration and a media source. Variables: `{user}`, `{event}`, `{message}`. Unconfigured event types use a text alert.

For media, add a Media file source, enable **Controlled by automation**, and select it in an Alert Box event override. Its ordinary canvas layer is optional; remove that layer when the media should only appear in the box. Enable the media source's audio for sound and adjust its mixer channel. A zero-opacity Alert Box background lets media show beneath text. Fit preserves the entire media image; fill crops it to the box. Existing GStreamer codec availability determines which media formats decode.

**Preview** changes no goals. **Clear alert boxes** clears pending and active box visuals. Existing automation pause/skip controls remain available. Box media uses the shared decoder and a native composition frame, not a browser runtime. Multiple simultaneous uses of the same media source share playback; use distinct media sources for independent playback timing. Media automation routed to a box has a ten-second default display duration; event overrides provide explicit durations.

## Audio mixer

Drag a channel's name onto another name to reorder channels. The order is saved with the project. Vertical meters scale with available panel height. Expanded docked and floating mixer panels have a minimum height of 15% of the app's content area; minimized/collapsed panels may still be collapsed intentionally.

## Efficiency and limits

Static widget frames update only when their contents change. Fonts are cached; one source frame is shared by previews, canvases, recordings and streams. Hidden widgets stop their rendering worker. Animated widgets use a bounded 20 FPS render loop; main canvas/capture defaults remain 60 FPS. Sports uses shared cached data and bounded timeouts. This preview does not add a browser process per widget.
