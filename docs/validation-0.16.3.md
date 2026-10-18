# Rosorah Studio 0.16.3 validation

Automatic hidden launch after connected chat source and installed runtime; Show reuses the process. Close hides the companion without stopping its timer; explicit parent Quit and stdin EOF terminate it. Hidden source pages are kept in Active lifecycle state. Source changes discard obsolete callbacks. A stuck callback schedules renderer reload. Process failure uses bounded 5–60 second retry and resets after a healthy minute. Runtime installation remains a one-time explicit action.

A real child-process integration test uses a local Python fixture: no process before account connection, automatic --background launch, message received through the native JSON pipe without opening a window, process reuse, crash detection, delayed restart, and graceful drop. Other Rust tests cover missing runtime, retry bounds and account/source switching. Python lifecycle unit tests call actual companion methods with mocked native widgets: hidden startup, Show, Close preserving collection, explicit Quit, Active hidden-page collection and stale callback rejection. Python syntax validation passed.

The mocked widget tests do not constitute a live QtWebEngine rendering or Twitch/YouTube session test. Actual platform DOM/background behavior, platform sign-in and long sessions require user testing on Fedora. No security sandbox was disabled to test browser rendering. No real platform chat messages or streams were sent.

Earlier canvas/resize/camera/output features remain included. No system packages or capture hooks changed.

Executed results: 89 release workspace tests passed (9 environment-dependent integration tests excluded from default run); 3 Python lifecycle tests passed; strict Clippy for all targets passed.
