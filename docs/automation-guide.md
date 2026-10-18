# Alerts and automations — 0.25 preview

## Create and manage rules

Open Alerts & automation. Choose a starting template and Add automation. Select a rule to edit its name, trigger, conditions and actions. Changes save with the project. Right-click its row or open the three-dot menu for editing, disabling, duplication, testing, manual execution, movement, export and deletion. Undo delete restores the last removed rule while the app remains open. Copies start disabled with no hotkey.

Blank, raid welcome, first-message, donation and death-counter templates are starting points. Text alerts are app-preview notices; for content in the actual broadcast use a media source or a canvas widget. The death-counter template uses an existing widget if available; choose the intended counter before enabling it.

## Conditions

All configured conditions must match. Empty filters impose no restriction, and zero minimum/cooldown values disable those restrictions. Select a platform or Any platform, then configure a user, word/phrase, reward ID, minimum donation amount, currency, event count, permissions or active scene. Scene scope matches when either canvas uses that scene.

Donation amounts display with two decimals and retain the event's original currency. Use an explicit currency when setting an amount threshold; no currency conversion occurs. Numeric donation data comes from the existing connected donation providers. Twitch bits, raid viewer counts and gift totals use structured event data. Browser-collected YouTube events may lack numeric or role data; conditions requiring unavailable values do not match. Moderator/subscriber permissions currently use trusted Twitch chat metadata. Selected users are comma-separated display names and are not an account-verification mechanism.

Global cooldown applies to a rule; per-viewer cooldown also includes platform and normalized viewer name. First message means first message this stream/session, not silent viewer presence. First-ever chatter is a separate Twitch event.

## Actions and playback

Drag a numbered action header onto another header, or use Up/Down. Actions issue in list order. Add Wait to delay subsequent actions. A media action starts playback; add an explicit wait if a later action must start after a particular point in that clip. Queue mode holds the next automation until current media finishes and text/wait durations expire.

- Queue runs one automation at a time.
- Overlap allows multiple sequences. Different media sources can play together; repeated triggers of the same source use its existing bounded playback queue. Preview text has one notice slot.
- Interrupt cancels the previous alert media and sequences when another automation starts.
- Pause holds future actions; already playing media continues. Elapsed waits continue counting while paused.
- Skip cancels the first queued/running sequence and its media. A media source shared with another overlapping sequence is a single player, so stopping that source also stops its shared playback.
- Clear queue drops scheduled sequences; already playing media can finish. Stop all alerts cancels media and queued actions and pauses future processing. Uncheck Pause alerts to resume.

Use variables such as `{user}`, `{message}`, `{amount}`, `{currency}`, `{count}`, `{platform}`, `{reward}`, `{bits}` and `{viewers}`. Viewer-provided text is substituted literally once; it cannot introduce extra variable expansion. Donation display privacy remains respected.

## Media and canvases

1. Add a Media file source in Studio and keep its layer visible in each target scene.
2. Add a Media action and select that source.
3. Enable Controlled by automations. Background media should use a separate source.
4. Set audio and volume, then choose a maximum duration. Zero plays the whole file.
5. Select Canvas 1, Canvas 2 or both. This filters video visibility; audio continues through the existing shared media mix.
6. Adjust each scene layer's normalized X/Y/Width/Height fields, or position it in Studio.

Locate file repairs a missing path using KDE's file chooser. Preview media opens the canvas preview and triggers the source. Media tests are blocked while recording or streaming because the preview shares the existing media player. A hidden source or one outside the active scenes will not play visibly. Decoder support remains dependent on installed GStreamer codecs; this update does not add universal format support.

Media settings belong to the source, so other rules using that source share its volume, duration and canvas choices. Timed-out or completed alert media releases the existing decoder pipeline; inactive alerts do not continuously decode.

## Tests, replay and missed donations

Test preview bypasses trigger conditions using sample values and skips widget/counter changes and scene switches. Media is allowed only when neither recording nor streaming. Run manually executes configured actions directly, including changes to counters and scenes; it bypasses trigger conditions and cooldowns. Ctrl+Alt+F1 through F12 can run enabled rules while Rosorah has focus and no text field is being edited. These are not system-wide shortcuts.

Right-click an Activity entry and choose Replay matching alerts. Replay matches current enabled rules, uses a fresh cooldown/dedup context, and excludes widget updates and scene switches. It never re-ingests the donation or increments donation goals. It does not resend platform events or messages.

Recovered missed donations have three independent controls:

- Show missed activity controls visibility in Activity; hidden history is retained for deduplication.
- Count missed donations toward goals is enabled by default. Newly recovered unique donations update matching-currency goals once.
- Also play this automation for newly recovered missed events is off by default and applies to an individual rule. Enabling it permits that rule's configured actions when new history arrives; it does not reprocess already handled history.

Recovery is only available where the connected provider/receiver supplies history. This update cannot reconstruct events the provider never delivered or retained.

## History, backup and efficiency

Run history and rule explanations are session-local and bounded to 200 entries each. Scheduler capacity is 100 sequences with at most 64 actions per rule; full queues report skipped work. Long waits are non-blocking. Duplicate event IDs are bounded, and actions cannot emit input events or execute scripts, preventing self-triggering action loops. Warnings highlight enabled rules with potentially overlapping triggers; conditions may still make those rules exclusive.

Export from a rule's menu, then save the JSON file or copy it. Import supports a file chooser and pasted JSON. Bundles contain rules plus source/scene/canvas names, without account configuration, credentials, stream keys or media files. Imports resolve references by unique names and start disabled without hotkeys. Create the named sources/scenes first if an import reports missing or ambiguous references. File dialogs require `kdialog`; copy/paste remains available elsewhere.

Rules, source settings and missed-event preferences save with the project. Queue mode, pause state, run history and pending actions are session controls and reset when Rosorah closes. There is no additional polling loop or browser runtime for this editor.
