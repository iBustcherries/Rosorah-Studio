# 0.8.0-hotfix1 validation

The user's Fedora diagnostics showed Rosorah's OpenGL loader in two native Dead Cells shell launchers but neither the loader nor capture backend in the game executable. The 0.8.0 observer detached after the first qualifying exec, relying on preload inheritance. This failed across that launcher chain. The evidence does not by itself establish exactly which script or runtime removed the preload.

The hotfix recognizes shell interpreters by their actual executable basename, independently of the process's script-name comm field. It leaves their environment intact and continues watching their exec/fork events; it also follows the env utility. The native executable then uses the existing bounded stack planner and detachment path. The existing ten-second wrapper timeout, same-user checks, secure-execution refusal and helper shutdown behavior remain. This is a focused shell-launcher fix, not universal launcher support.

Validation performed for this hotfix:

- Production helper and native test program compiled with C++17, -Wall -Wextra -Werror.
- Native 32-bit and 64-bit simulated stack tests passed, including argv/auxv preservation, bounded writes, inherited/old preload handling, game filtering, opt-out and secure execution.
- Wrapper classification checks passed, including negative cases for native game names containing shell/utility prefixes.
- A live regression fixture was added for two shells that explicitly clear LD_PRELOAD before execing the native graphics fixture. Both the direct live test and wrapper live test exit 77 here: PTRACE_TRACEME returns EPERM. Live attachment and GPU capture through this launcher chain are NOT validated here.
- The packaged Rust app and all six rendering libraries are unchanged from the saved 0.8.0 release. The earlier Rust and graphics test results in validation-0.8.0.md are baseline results, not new hotfix test runs.

Fedora check: close old Rosorah; keep Steam/KDE open; launch this archive's run-studio.sh; wait for Steam observation; launch/restart native Dead Cells. Confirm an actual game image. If still blank, compare loaded libraries in the game executable and collect Capture details diagnostics. No Steam launch options are required by this approach. Standalone OpenGL emulator activation is still deferred.
