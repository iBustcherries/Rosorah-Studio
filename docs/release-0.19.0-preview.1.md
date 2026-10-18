# Rosorah Studio 0.19.0-preview.1

Extract this entire folder and run `run-studio.sh`. Close the previous Rosorah instance first. Keep the hooks and browser folders beside the application.

## This build
- Destinations beneath each canvas; compact Broadcast controls with separate stream-detail and quality windows.
- Chat destination toggles for connected Twitch/YouTube accounts. Send to either or both. The composer wraps/grows to five lines, then scrolls. Enter sends; Shift+Enter adds a newline. Delivery is tracked per platform; failed batches retain the draft. Retry only failed destinations to avoid duplicate messages.
- Main/Vertical scene tabs. Right-click a scene (or use its ... button) for duplicate, rename, delete, reorder and a Link scene submenu. Main switches its linked Vertical scene; switching Vertical independently does not change Main. Links can be disabled globally or removed per scene.
- Each canvas owns its scene list. Existing active vertical layouts are retained. New vertical scene alternatives share source assets while keeping independent layer transforms and scene volumes. Duplicates begin unlinked.
- Drag layers by their :: handle to reorder. Right-click offers ordering and lock actions. Locked layers cannot be dragged.
- Horizontal/vertical audio controls with slim handles; detailed controls remain in each channel's Settings menu.
- On-demand 160 ms layer/panel movement. Settings > Appearance & chat > Smooth panel and layer movement disables it. Preview textures are reused while panels settle to avoid repeated resize allocations. Capture and encoding are not animated.
- The prior version-3 dock layout is backed up with a .pre-019.json extension. Hidden panels stay hidden. Panels > Reset layout restores the new default.

## Scope and validation
This remains a preview. The alert/donation preparation limitations from 0.18 remain: live donation-provider connections and a hosted donation service are not implemented here. Existing native game hooks and browser companion are retained unchanged.

See VALIDATION.md for tested behavior and limits. Actual Fedora/AMD performance and signed-in multi-platform posting need user testing.
