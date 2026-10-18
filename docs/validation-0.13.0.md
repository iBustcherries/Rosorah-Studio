# Rosorah Studio 0.13.0 validation

## Changes

- Persistent automatic game capture resolution: 720p, 1080p default, 1440p, 2160p, original (4096 per-axis limit). No upscaling; aspect ratio preserved. 60 FPS remains the default. Canvas recording dimensions remain independent.
- Version-3 sealed shared-memory transport with bounded 4096×4096 capacity and quality control before capture activation. GPU readback buffers follow the selected dimensions; changes rebuild once on the proper owning thread/context.
- Canvas selection, corner resizing, dragging to move, aspect toggle, pixel position/size and Fit to canvas. Locked items are protected. Editor handles are not composited into outputs.
- Existing observer, GL activation loader, game audio, recording and streaming mechanisms retained.

## Completed checks

- Release build, rustfmt and Clippy (warnings denied).
- 47 ordinary Rust tests: 8 core, 4 capture, 35 app. Includes actual egui mouse events resizing an item and verifying a locked item remains unchanged, corner anchors/aspect preservation, and minimum-size/move bounds. Eight environment-dependent tests remain ignored in the ordinary suite.
- Real 64-bit OpenGL/EGL core, ES and MSAA readback at all five quality levels from a 3840×2160 surface, followed by a return to 720p. Verified pixel values, allocation sizes, GL state restoration, lookup chaining and missing-backend passthrough.
- Real Vulkan loader/GPU readback from 3840×2160 at all five quality levels, SDR/10-bit SDR/PQ/BGR PQ/HLG/scRGB. Verified output dimensions and pixel values, inactive passthrough, pending-copy handling and cleanup. Mesa llvmpipe software driver.
- Both 32-bit and 64-bit release hook libraries compiled with warnings as errors. 32-bit hardware runtime was not exercised here.
- Ultrawide aspect fit, original portrait dimensions and no upscaling checks passed.
- GPU compositor self-check passed for shared textures, two canvas sizes, transforms, visibility, alpha, row padding and empty scenes.
- Isolated installer regression passed: both architectures, token expansion, repeated installation, scoped uninstall and preservation of unrelated files.
- Native studio opened under X11/Xvfb and its screenshot was inspected. Visible capture resolution selector and selected-item corner handles.

## Limits

The build environment blocks Unix-domain sockets and live process attachment, so renderer IPC/Steam launch integration could not be rerun end-to-end here. GPU readback tests use software rendering, not the user's physical AMD GPU. Fedora/KDE Wayland game performance at high resolutions still needs user testing. Capture remains a bounded readback/copy pipeline, not zero-copy; 4K/60 uses substantially more bandwidth than 720p/60.

After upgrading, restart an already-running game once to load protocol-3 hooks. Steam and KDE can stay open. The existing streaming limit remains 1080p/60; recording output dimensions are chosen in Canvases & recording.
