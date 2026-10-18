# Rosorah Studio 0.28.0-preview.1

## Widgets and editing

Choose **Widgets** in the top bar to add a widget to the active canvas. The new source opens in **Widget studio**. Existing widget layers stay in Layers, where you can right-click **Edit Widget…**. The old widget creation submenu in Add layer has been removed.

The editor provides a live preview and an explicit sample-data preview. Samples do not post chat, play an alert, or change a goal. Preview images use the same native renderer as the canvas. Hidden sources use a static preview. Alert preview buttons in the controls are separate, intentional actions that can display a test alert.

Copy/paste an appearance or save a named style in the project. Applying a style preserves text, goals, counts, account selection and music-player selection. Use Apply this style to scene widgets for a consistent appearance. Styles are currently saved per project.

Chat cards, retro chat windows, phone bubbles, music, goals, counters and utility cards have revised typography, spacing and rounded-edge rendering. Chat cards show up to four recent messages. Canvas chat still renders text rather than inline emote images; the separate chat dock is unchanged. Sports retains its existing score layouts and provider requirements.

## Desktop music

Add **Widgets → Now playing**, start music, and keep **Detect desktop music automatically** enabled. Automatic selection prefers a playing desktop player. Use the player selector when multiple music or video apps expose media controls. A browser player may identify itself by browser name rather than music service.

The Linux implementation reads the session's standard MPRIS media interface. Spotify and other desktop apps, and browser tabs for services such as YouTube Music, Apple Music, Amazon Music and Tidal, can appear **when their particular app/browser publishes that information**. Support for each service is not independently certified. If a service exposes no media information, Rosorah cannot infer the track from its audio. Flatpak/sandbox session-bus permissions can also affect discovery.

The card supports cover art, artist, title, optional progress, automatic artwork colors, compact/large-artwork layouts and brief track-change fades. Long titles are ellipsized. Missing artwork uses an abstract fallback. Unknown duration hides the progress bar. Hide when paused/stopped is enabled by default; unavailable players produce a transparent source. Manual title/artist entry remains available, and older manual music widgets remain manual when opened.

No separate music authorization or music-service API polling is used. A single shared worker queries local desktop media information approximately every two seconds while requested. Artwork may be downloaded from the URL supplied by the player; it is bounded and cached. Hidden widgets stop their render workers, and media discovery idles after demand stops. Stable playback progress redraws at most once per second; track-change fades run briefly at 20 FPS. These limits do not cap game or stream FPS.

## Editing and recovery

Use **Edit → Undo / Redo** or Ctrl+Z / Ctrl+Shift+Z (Ctrl+Y also works) for project edits. Typing fields retain their own text undo behavior. Continuous drags are coalesced, history is bounded to 40 snapshots / approximately 16 MB, and history is not saved across restarts. Live widget totals are preserved through project undo. Account authorization, external messages, recording/stream actions and activity history are outside project undo.

Layer context menus add alignment to the six canvas edges/centers and **Restore source proportions**. Sources without known dimensions must start first. Full multi-selection, grouping and equal-spacing tools are not included in this preview.

Unsaved project edits are checkpointed approximately every 15 seconds. If a newer recovery copy exists at startup, choose Recover changes or Keep saved project. The saved project is not overwritten before that choice. Successful project saves retain up to five prior valid copies in a neighboring `.backups` folder. **Edit → Stream readiness** includes restore buttons that load a backup as unsaved changes. Normal exit uses the same backup-aware save path.

## Readiness

**Edit → Stream readiness** is an optional local check: account connection state, enabled destinations with missing setup, missing local media, microphone enabled/muted state, and capture/browser status. It provides links to account and broadcast settings. It does not start a stream, test a remote ingest server, verify rights to music, or certify the sound/video viewers receive.

## Install

Extract the complete Linux ZIP, close the previous Rosorah instance, and launch `run-studio.sh` from the new folder. Existing preferences and service connections remain in their existing locations. Keep your prior build and a project backup while testing this preview. No kernel, display-driver, Steam-launch or KDE session changes are needed for this update.

Existing capture hooks, recording/streaming pipeline, browser helper and donation hosting requirements are retained. Default canvas/capture frame rate remains 60 FPS. Windows is not included.
