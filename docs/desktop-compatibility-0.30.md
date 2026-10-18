# Desktop compatibility — 0.30.0-preview.1

This release removes desktop-specific assumptions and adds capability-based integration. It is a compatibility preview, not certification that every feature works on every compositor.

## Changes

- One file chooser path for media, automation import/export, YouTube thumbnails, project transfer and recording tools. The XDG FileChooser portal is preferred. If unavailable, Rosorah tries Zenity first outside KDE and KDialog first on KDE. Cancelling a working chooser ends the operation rather than opening another dialog. Local file URI decoding preserves spaces and Unicode; remote file URIs are rejected.
- Screen/window capture reads the portal's advertised source types and pointer modes before requesting a session. A window-only request never silently becomes a whole-screen capture. Screen-only desktops get an explanatory status. Pointer embedding may fall back to hidden with a notice; a request to hide the pointer never silently embeds it.
- X11 sessions can use GStreamer's ximagesrc if the portal is unavailable or cannot satisfy the request. Screen capture uses the X11 desktop; window selection uses xwininfo and supports cancellation. X11 fallback is never used on a Wayland session, even when XWayland supplies DISPLAY.
- The renderer receiver excludes desktop compositors/shells, including KDE, GNOME, Cinnamon, Xfce, COSMIC, Sway, Hyprland, Weston, river, niri, labwc and Wayfire, before admitting a renderer client. Existing game hook libraries and their activation model are retained.
- Edit → Desktop compatibility shows this session's screen/window/pointer capabilities, file chooser, account wallet and installed audio/capture tools. Checks run once at startup and on Refresh, on a worker thread, with a bounded portal timeout.
- External browser/file opening falls back to GIO when xdg-open is absent. Account sign-in still uses the existing shared account connections and Secret Service storage.

## Desktop targets

| Desktop/session | Integration route | Remaining validation |
| --- | --- | --- |
| KDE Plasma / Wayland | Desktop portal for sharing and dialogs; PipeWire audio; renderer hooks | Real-session regression test |
| GNOME / Wayland | Desktop portal for sharing and dialogs; PipeWire audio; renderer hooks | Real-session test |
| Cinnamon, Xfce, MATE / X11 | Portal where offered; native X11 fallback; GTK/KDE dialog fallback | Real desktop/window-picker test |
| COSMIC / Wayland | Advertised desktop portal capabilities; PipeWire audio; renderer hooks | Real-session test; capabilities vary by installed backend |
| Sway, Hyprland, niri, river, labwc, Wayfire / Wayland | Installed portal backend's advertised sharing capabilities; FileChooser backend may be separate | Real-session test for each compositor/backend |
| Other X11 desktops | Generic X11 fallback and desktop services | Real-session test |

These rows describe how Rosorah selects an interface, not a blanket claim that every listed desktop provides every interface. The capability panel is authoritative for the current session.

## Requirements and limits

- Screen sharing on Wayland requires xdg-desktop-portal and a working backend for the current desktop. A backend that exposes only monitors cannot supply individual windows. Rosorah does not replace the compositor, modify portal configuration, or bypass its permission prompts.
- X11 fallback needs the GStreamer good plugins (ximagesrc). Window selection additionally needs xwininfo. An X11 window may stop producing useful pixels when minimized or obscured. This fallback is not a renderer hook and can use more CPU.
- Existing game capture remains renderer/runtime dependent, not guaranteed for every game or anti-cheat configuration. The native installer does not support Flatpak packaging; this pass does not add Flatpak support.
- Audio features require PipeWire and its command-line tools. PulseAudio-only installations do not gain per-game/desktop audio support from this release.
- Hardware encoding depends on the GPU, drivers and installed FFmpeg codecs. Switching desktop does not install or repair them. GPU counters remain driver-dependent.
- Persistent account login needs libsecret and an available, unlocked Secret Service provider. Without one, existing behavior permits session-only login; Rosorah does not write OAuth tokens into project files.
- Automation shortcuts remain application-focused. Desktop-reserved shortcuts may be intercepted before Rosorah receives them. This release does not add system-wide shortcut registration.
- Browser sources/chat continue to use the existing private browser runtime. This release does not change their platform event coverage or provider limits.
- No desktop environment, kernel, graphics driver or system service is installed or changed by this package. The legacy optional KDE login hook remains explicitly KDE-specific; normal automatic game capture does not require it.

## Protocol references

- https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html
- https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.FileChooser.html

The ScreenCast interface requires clients to request advertised cursor/source modes. FileChooser is a shared interface whose backend supplies the desktop's dialog.
