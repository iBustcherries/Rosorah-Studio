# Rosorah Studio 0.31.0-preview.2

## Automation preview
The floating preview now reserves a bounded drawing area. Canvas content can no longer enlarge the window on each redraw. The window remains draggable and resizable, and Canvas 1/2 switching and canvas controls remain available. The old expanded window geometry is reset once; projects and panel layouts are preserved.

## Alert audio
The guided setup has a “Hear this alert on my speakers / headphones” checkbox, enabled for new alerts. It enables the alert media's monitoring and chooses the system default output if no monitoring device is already configured. It preserves an existing chosen output and stream/recording track routing.

For an existing automation: edit its Display alert or Play sound action and enable “Hear on my speakers / headphones.” For video, also enable “Use media audio” and check its Volume. The mixer lets you choose the monitoring device or turn monitoring off globally. Existing alerts are not silently changed.

If Desktop audio captures the output used for monitoring, the stream may receive the same alert twice. Choose a separate monitoring output when using both paths. Separate sound files intentionally replace the video's soundtrack in the guided setup.

This addresses local monitoring configuration. Files still require a supported audio codec and an audio track. Real-device playback must be checked on the user's machine.

## Windows
Native monitor capture, WASAPI audio and Credential Manager source are included as the first Windows backend checkpoint. The included runnable Studio bundle remains Linux-only. See windows-port.md for implemented scope, validation and remaining work.

## One shared Alerts box
The alert guide now reuses the project's existing Alerts box, including a box added manually. Adding Alerts from Widgets also reuses it. Adding another alert does not create another alert box. On another scene/canvas, Rosorah adds a layer referring to the same source so placement can differ. Existing duplicate boxes from older projects are preserved rather than deleting their layouts/settings automatically; new alerts use the first existing box.
