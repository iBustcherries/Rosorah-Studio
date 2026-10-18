# Validation — 0.27.0-preview.1

- Rust workspace checks passed.
- Automated tests: 152 passed, 0 failed, 18 pre-existing hardware/service-dependent tests ignored. Includes native widget rendering across themes/goal styles, legacy widget settings, weekly resets, period-scoped historical donations, no-goal-mutation preview, transparent idle alerts, media fit and sports score parsing.
- GPU composition self-check passed on software Vulkan (llvmpipe): two canvases, shared textures, placement, stacking, visibility, alpha, row padding, unused assets and empty scenes.
- Native app opened, rendered both canvases, saved its project and closed successfully in a virtual X11 desktop. Additional native screenshot checked goal/sports widgets on both canvases and the vertical mixer. Theme and goal output images were visually inspected.
- Release uses the prior validated native capture hooks, launch observer and browser helpers. Those helpers were not rewritten in this update.
- Archive contents and executable permissions are verified during packaging; ZIP CRCs and packaged executable startup/self-check are checked.

## Not verified here

No authenticated BALLDONTLIE key was available: no live NFL/NBA score freshness or account quota guarantee is claimed. Endpoint response parsing is fixture-tested; runtime connection failures retain old scores and show status. No live Twitch/YouTube/donation account or real game/camera session was available for this build's end-to-end testing. No Fedora/KDE/Wayland desktop or hardware GPU performance benchmark was available. The virtual desktop does not have PipeWire devices; its mixer correctly reports that audio discovery is unavailable there.

Calendar goals use the streamer's system timezone. Live source/stream integrations retain their existing requirements. See widgets-0.27.md for initial-preview limitations, manual utility widgets and supported data sources.
