# Browser sources and avatar backgrounds — 0.26 preview

## Add an overlay

1. In a scene, choose **Add layer → Browser source**.
2. Select that layer and open its properties. Paste the overlay URL into Browser source. The field masks the address because overlay URLs often contain tokens.
3. Set the webpage's size and frame rate, choose whether its audio should play, and click **Apply browser settings**.
4. The existing private browser runtime is prepared if needed. Users who already have browser chat working reuse the same installation. If preparation fails, use Prepare browser runtime to retry.
5. Resize/place the layer on Canvas 1 or Canvas 2 as usual. Reusing this source in multiple scenes/canvases does not start another browser for each canvas.
6. Use **Refresh page** to reload. **Interact** opens the page for clicking and typing; closing that interaction window keeps the source running while it remains visible on a canvas.

HTTP and HTTPS URLs are supported, including local web servers. Local file paths, embedded HTML editing, custom CSS, popup windows and camera/microphone permission grants are not included in this preview. A webpage must supply a transparent background to act as a transparent overlay; opaque webpages remain opaque.

The viewport supports up to 1920×1080 or 1080×1920, with a maximum dimension of 1920 and a 2,073,600-pixel budget. Set dimensions to the overlay's intended design size. Frame rate is 1–60; the default is 30 to reduce browser capture overhead. Canvas and game capture defaults remain unchanged at 60 FPS.

## Browser audio

Enable Browser audio and Apply. Rosorah creates a normal mixer channel for this source. Use its gain/mute/monitoring controls and existing track/destination routing. The browser is routed into a dedicated temporary PulseAudio-compatible sink; it does not record all desktop audio. Monitoring remains off by default, so you may not hear it locally until you enable monitoring, even when recording/streaming receives it.

On Fedora, `pactl` and `parec` are supplied by `pulseaudio-utils`. The normal PipeWire Pulse service must be available. If the audio connection cannot be created, Rosorah mutes the page and reports the problem rather than playing it through the desktop and capturing unrelated applications. Audio sinks are removed on normal source shutdown. Killing the browser with SIGKILL bypasses cleanup and may leave a temporary `rosorah_browser_…` sink until the audio server restarts.

The existing media mix remains shared; this update does not create isolated per-canvas audio mixes. Standard scene/mixer routing still applies.

## Avatar capture and chroma key

Rosorah does not load Live2D/VRM models or perform face tracking. Run your avatar in its own application, then use Rosorah's Window/Screen source or a compatible Linux virtual camera. Choose a solid green or blue background in the avatar app.

Select the capture layer, open its properties, and enable **Chroma key · remove a background color**:

- Choose Green, Blue or a custom key color.
- Increase Similarity until the background disappears.
- Adjust Edge softness to smooth the boundary.
- Use Spill reduction to desaturate background-colored edges.

The filter executes in the GPU compositor and applies to preview, recording and streaming. It is a source setting shared across its layers. Disabling it restores the original image without reopening capture. Skin, hair or clothing matching the chosen key color can also disappear, so use a contrasting background.

This is a conventional color key, not AI background removal. Transparent capture from arbitrary avatar applications remains dependent on their output/capture path and has not been verified universally.

## Lifecycle and resource use

Only enabled sources visible in an active canvas scene run. Hiding the last visible instance shuts its browser down; showing it reloads the URL. Website session state is temporary and is lost on shutdown. Overlay pages do not share account-chat cookies. This is suitable for provider overlay URLs; it is not persistent account sign-in across browser source restarts.

Each source has an isolated browser process and its own audio sink. Frames use the same latest-frame capture engine as other live sources, with bounded pending data. Unchanged page frames avoid IPC/upload work, and chroma key does not require CPU pixel processing. Page rendering/capture still consumes CPU/GPU resources; complex animation and 1080p60 cost more than small or static overlays. This is not a zero-copy browser renderer and no universal overhead figure is claimed.

Browser availability and codecs depend on the Qt WebEngine runtime and host libraries. Linux KDE/Wayland hardware compatibility should be tested on the target system. An automatic retry follows unexpected source-process exits; Refresh is available for page load errors.
