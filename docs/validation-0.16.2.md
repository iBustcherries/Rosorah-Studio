# Rosorah Studio 0.16.2 validation

Resize fixes: preview-specific drag ownership (also scoped by canvas ID); retain selection changes in the active preview; 10-point handle hit margin clipped to its own panel.

Regression tests exercise both preview render orders during a multi-frame vertical corner drag, verify the main scene remains unchanged, preserve anchor and aspect, release the drag cleanly, and grab a handle from outside the image edge. Existing lock, snapping, Ctrl bypass and corner transform tests are retained.

Camera sizing: default-on automatic aspect option; use actual frame dimensions, including crop. Tests cover landscape and portrait canvases with 4:3, 16:9 and portrait camera input; input changes, unchanged input, position/width preservation, locked layers and disabling automatic fitting. Native camera hardware and user Fedora desktop still require testing. Reused scenes continue to share transforms across canvases.

No changes to Twitch/YouTube output protocols or authentication. Twitch Enhanced Broadcasting and Dual Format are not included. Capture hooks, camera drivers and system packages remain unchanged.

Executed: release workspace suite passed 87 tests (9 integration tests excluded from the default suite); strict Clippy for all targets passed. New resize regressions and camera-layout checks ran successfully.
