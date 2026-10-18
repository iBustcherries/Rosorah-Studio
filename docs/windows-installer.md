# Windows preview.6 EXE installer

The installer wraps the existing preview.6 Windows binary. It does not introduce
another capture change. Start it directly; ZIP extraction and manual commands
are no longer required.

- Installs for the current account under LocalAppData/Programs/Rosorah Studio.
- Bundles the GStreamer 1.26.10 runtime (its download server proved unreliable) and
  downloads the rest over HTTPS with automatic retry and resume (setup.ps1). GStreamer,
  FFmpeg 9.0.2 and Python are SHA-256 checked; Microsoft Visual C++ Runtime has
  its Authenticode signature and Microsoft signer checked before execution.
- Official prerequisite installers may request administrator approval. Rosorah
  itself runs as the ordinary user. Setup records failures in setup.log and stops
  rather than reporting a successful installation with missing dependencies.
- Adds a Start menu shortcut and an optional desktop shortcut. The native launcher
  sets media paths and hides the console. Startup failures appear in a dialog;
  logs are under LocalAppData/RosorahStudio/logs/startup.log.
- Registers an uninstaller in Windows Settings > Apps. Projects, profiles and
  account data outside the install directory are retained. Shared GStreamer and
  Microsoft runtimes remain installed for other applications.
- Installs Python 3.13.16 for the current user when it is missing (SHA-256 and
  Python Software Foundation signature checked, no administrator approval). Browser
  chat uses it to download its private PySide6 runtime on first use. This installer
  is not a complete feature-parity claim.

Build: run packaging/windows/build.ps1, then build-installer.ps1 with NSIS 3 and
Rust/MSVC installed. The generated uninstall list covers exactly the payload.

Validation: the launcher cross-linked for Windows x64; the NSIS installer compiled
with warnings treated as errors. Archive contents are checked against the supplied
payload. Installation/UAC/shortcuts/uninstallation have not been executed on a
Windows computer here. The installer is unsigned; report security-block messages
instead of disabling Windows protections.
