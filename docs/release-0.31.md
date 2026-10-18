# Rosorah Studio 0.31.0-preview.1

## Alerts and automation

Open **Alerts & automation → Create alert or automation**. Choose a template/event, add media and text, choose a canvas, then create and test or enable it. The guide reuses a guided alert box on the selected scene, or adds one if needed; resize and position it like another widget. The rule remains editable in the automation list and its context menu.

- Text can sit above, below, or over media. Text variables use the existing event expansion.
- PNG/JPEG still images, video/GIF via installed GStreamer decoders, and separate sound files use existing shared media playback. This is not a promise that every codec is installed.
- Alert duration is explicit (1–60 seconds); it is not auto-detected from the media file.
- An optional chat reply uses the triggering platform by default, or explicitly selected Twitch/YouTube accounts. Donation services have no chat endpoint: choose destinations for donation replies. Outgoing messages use the shared authenticated account worker, not a new poller or bot login.
- Chat post results appear in automation history. Test and replay remove chat-posting, counter/goal, and scene-switch actions. Offline tests may play media; live recording/streaming blocks media previews.
- Incoming messages from the connected account are not used to trigger chat-command/phrase automations, preventing simple self-reply loops.
- Advanced conditions, roles and cooldowns are collapsed by default. Existing rule queues and context menus remain available.

The guide shares an alert box and its text position across guided alerts on the same scene. It does not yet provide a reusable shared design library, visual timeline, automatic clip-duration discovery, or a nested action builder. Existing legacy alerts retain their behavior.

## Twitch rewards

Open **Alerts & automation → Twitch rewards**. Use **Grant rewards access** once to approve the new `channel:manage:redemptions` permission when needed. Existing chat/broadcast sign-ins continue working without this optional permission. The permission button renews the shared Twitch account; it does not create a second login.

List rewards, create/edit reward details and limits, pause/enable, delete with confirmation, link to a new automation without copying an ID, and view/fulfill/cancel pending redemptions. Canceling a pending redemption refunds points through Twitch. The queue displays the oldest 50; resolve and refresh to retrieve subsequent requests.

Twitch only permits the app/client ID that created a reward to modify it or its redemptions. Other rewards are shown as read-only but can be linked to event automations. Existing Firebot-created rewards are not taken over. The editor checks ownership before mutations. Reward requests occur on explicit user actions, not continuous polling. Existing Twitch EventSub handles live events.

Live reward mutations and chat posting require account testing by the user; development verification does not create or send anything on a real account.

API reference: https://dev.twitch.tv/docs/api/reference/#create-custom-rewards

## Import preview

Open **Edit → Project & recording tools → Preview OBS / Firebot import**. Select a file or enter a path. You can also enter a replacement media folder; unique matching basenames reconnect missing local files. Review the report, then create a new project file. Existing project files and original backups are not overwritten. Imported Firebot rules are disabled for review.

Supported first-pass inputs:

- OBS scene collection JSON: basic scenes, image/video/color/text sources, HTTP(S) browser URLs, source order, visibility, estimated placement, and media volume/mute.
- OBS `basic.ini` profile: base canvas dimensions and integer FPS. A ZIP containing a scene collection and `basic.ini` combines these.
- Firebot JSON/ZIP event and command definitions: recognized follow/raid/bits/sub/redemption/donation triggers and commands; chat, local sound, local video/overlay alerts and delays.

This is a partial converter, not a complete Firebot or OBS backup restore. OBS nested scenes, groups, plugin sources, device bindings, filters, transitions, detailed text styling, non-integer FPS and encoder/output profiles need manual work. Source sizes without explicit bounds are estimates. Firebot scripts, nested effects, unsupported variables, role/condition logic, remote media, timers and counter definitions are not fully translated. Every supported rule starts disabled; inspect its conditions before enabling. Unsupported items appear in the report. No scripts are run; archives are read in memory without extraction, with size/entry limits and path checks. Auth/token/service files are excluded. Credentials and stream keys are never mapped into the new project.

An actual user OBS export and Firebot backup are still needed for broader compatibility testing. Fixtures do not establish compatibility with every backup version.

Schema references (read for interoperability, no upstream application code copied):
- https://github.com/obsproject/obs-studio/blob/master/libobs/obs-scene.c
- https://github.com/crowbartools/Firebot/blob/master/src/backend/backup-manager.ts
- https://github.com/crowbartools/Firebot/blob/master/src/types/events.ts
- https://github.com/crowbartools/Firebot/tree/master/src/backend/effects/builtin

## Windows groundwork

The shared project/automation model now has Windows-aware configuration locations, and the shared compositor enables Direct3D 12 on Windows with Vulkan also available. A Windows CI workflow checks the shared core, capture contracts and compositor. This is the start of the port, not a Windows application release.

The full app still has Linux-only integrations: capture portals and Unix renderer transport, PipeWire audio, V4L2 camera discovery, Secret Service, launch hooks, browser-runtime setup and process diagnostics. WASAPI, Windows capture/camera and credentials, renderer hooks, packaging and real-device validation remain necessary. No Windows installer is included in this preview.
