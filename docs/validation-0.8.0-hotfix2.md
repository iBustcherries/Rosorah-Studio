# 0.8.0-hotfix2 validation

The user confirmed hotfix1 was running on Fedora KDE Wayland. Its INJECTED PID 122516 was Steam's ubuntu12_32/reaper. The game ancestry continued through steam-runtime-tools-0/srt-bwrap, steam-runtime-tools-0/pv-adverb, two bash scripts and native deadcells. The OpenGL loader was present in reaper and the shells but absent from deadcells. This identifies an early-detachment gap before the shell handling added in hotfix1.

The updated helper keeps observing the three reported runtime launcher path suffixes and the previously supported shell interpreters. Their initial environments are not modified. The existing final-executable stack preparation, same-user restrictions, secure-execution check, ten-second wrapper timeout and shutdown behavior remain. Runtime paths are matched narrowly to avoid treating a game merely named reaper or pv-adverb as a launcher. No system security policy, Steam file or launch option is changed.

Completed for this hotfix:

- Helper and native tests compiled with C++17 and -Wall -Wextra -Werror.
- The reported executable sequence was checked against the launch decision: all intermediate stages remain observed; the final deadcells executable is an injection candidate. Unrelated same-named executables remain candidates.
- Existing simulated 32-bit/64-bit initial-stack and argument/environment/auxiliary-vector integrity tests passed.
- The live wrapper fixture was attempted and exited 77 because PTRACE_TRACEME returned EPERM. Actual tracing through Steam's container, its fork/exec behavior and Dead Cells frame delivery remain unverified here.

The Rust app and all six rendering libraries are unchanged from 0.8.0; its earlier graphics/Rust test results are baseline evidence, not new tests of this handoff. The native helper is the only changed executable. Vulkan capture is unchanged. Standalone OpenGL emulator activation remains deferred.

Fedora acceptance: close Rosorah and Dead Cells, leave Steam/KDE open, launch this package's run-studio.sh, wait for Steam observation, then launch native Dead Cells. Check actual image delivery. If still blank, collect the same executable/parent/loaded-library diagnostic and Capture details log; determine whether INJECTED now refers to the game and whether both OpenGL libraries are mapped there.
