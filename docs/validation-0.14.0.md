# Rosorah Studio 0.14.0 validation

## Implemented

- Snapping while moving or resizing sources: canvas edges, canvas center and other visible source edges/centers. Cyan guides are editor-only. Ctrl temporarily bypasses snapping without accumulated offsets.
- Default 8 px preview-space threshold, adjustable 2–24 px. Each target category and the global switch are configurable and saved with the project. Older projects default to enabled snapping.
- Hidden/fully transparent sources and the moving source itself are excluded. Locked sources remain valid alignment targets. Proportional resizing keeps its opposite corner and aspect ratio; free resizing can snap both axes independently.
- Layers panel places the stack first, top layer in front. Move up/down and Order → Bring to front/Send to back retain the selected source and its properties. Locked layers cannot be reordered. Existing per-layer size, position, visibility, opacity and lock controls remain available.
- Game capture controls are grouped under Layers → Game capture. Capture defaults remain 1080p/60, with adjustable resolution and independent canvas output dimensions.

## Verified

- Locked release build, rustfmt and Clippy with warnings denied.
- 55 ordinary tests passed (10 core, 4 capture, 41 app); eight environment-dependent integration tests remain ignored in the normal suite.
- Tests cover preview zoom-independent thresholds; canvas edge and center alignment; adjacent source alignment; hidden, locked and self target rules; snap release without drift; all four proportional resize corners; free resize bounds; layer ordering, selection and property preservation; legacy defaults and saved settings/order round trips.
- Actual egui pointer events verify resize/lock behavior, snapping, holding/releasing Ctrl while dragging, moving away to release a snap, and guides disappearing after release.
- GPU compositor self-check passed on Mesa llvmpipe. Bringing the background to the front changed the actual rendered overlay pixels, then sending it back restored the stack. Existing composition, visibility, alpha, shared texture, padding and empty-scene checks also passed.
- Native studio opened under X11/Xvfb; the Layers stack, ordering buttons and snapping controls were visually inspected.
- All six native capture libraries, launch observer, installer and launcher are byte-for-byte copies of the working 0.13.0 release. Capture, recording and streaming implementation source is unchanged; no capture protocol upgrade occurs.
- Download packages are checked for complete bytes, archive integrity, member hashes, executable modes, extraction and executable version.

## Scope and upgrade

The user confirmed streaming works in 0.13.0. This release changes scene editing and layer composition controls; external-platform streaming and Fedora game-hook integration were not rerun in the build environment. Physical-GPU performance is not benchmarked here.

From 0.13.0, close Rosorah, extract the complete package into a fresh folder and run run-studio.sh. Games, Steam and KDE can stay open. Direct upgrades from 0.12.0 or earlier still need one game restart for the protocol-3 capture hooks introduced in 0.13.0.
