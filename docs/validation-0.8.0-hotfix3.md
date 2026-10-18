# 0.8.0-hotfix3 validation

The user's running hotfix2 helper was confirmed by its executable path. Native deadcells still had neither OpenGL library mapped. The app log contained two INJECTED events without a later error; it did not identify the exec-time executable names. The precise missed stage was therefore not established.

Hotfix3 changes the observer lifecycle: successful preload preparation and inherited preloads no longer cause immediate detachment. The observer continues following exec/fork events, including through an unrecognized native launcher. Known shell and runtime wrappers remain unmodified. Once the OpenGL capture backend appears in a process's maps, its observed tasks are interrupted and detached using the existing cleanup path. The existing per-task ten-second startup timeout also triggers detachment. Unsupported, opted-out, secure or failed preparation still detaches; same-user checks and permission restrictions are retained.

Only process-creation/exec events are traced. No syscall or per-frame tracing is added. The impact of observing game startup for up to the timeout has not been measured on Fedora. Actual backend detection/retirement, container transitions and frame delivery require desktop verification.

Diagnostic lifecycle events now also go to syslog under rosorah-launch-watch, tagged hotfix3. EXEC, FOLLOWING, CAPTURE_LOADED, STARTUP_TIMEOUT and OWNER_CHANGED distinguish previously silent stages. Existing preparation exceptions are logged with their fixed reason strings. Logs include executable basenames and numeric identifiers, without process arguments, environment values or pixels. The unchanged app ignores the extra event codes in its stdout protocol, so use the journal command in README for these details. Fedora journal delivery was not tested here.

Completed checks:

- Production helper and native tests compiled with C++17 and -Wall -Wextra -Werror.
- Existing 32-bit and 64-bit initial-stack, environment/argv/auxv, bounds, opt-out, secure-execution and wrapper-path checks passed.
- Test script passed bash syntax validation.
- Added a live regression that drives the actual Watcher across an unrecognized native wrapper, a fork, and two shells that clear LD_PRELOAD, ending in the existing graphics fixture. It uses PTRACE_SEIZE to match the production observer. The test was attempted and returned 77 because PTRACE_SEIZE is denied with EPERM. This is NOT a passed live regression.

The packaged app and six renderer libraries remain the 0.8.0 binaries. Earlier graphics/Rust results are baseline evidence only. The helper is the only changed executable. Vulkan rendering remains unchanged; standalone OpenGL emulator activation remains deferred.

Fedora acceptance: close Rosorah and Dead Cells, keep Steam/KDE open, run this package, wait for launch observation and start native Dead Cells. Confirm the actual canvas image. If blank, collect `journalctl -b -t rosorah-launch-watch --since '5 minutes ago' --no-pager` promptly and verify loaded libraries in the actual game. Backend-loaded or hook-prepared messages alone are not a capture success.
