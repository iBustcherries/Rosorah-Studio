# Rosorah Studio Windows preview.9

Close Rosorah, run the Setup EXE, then reopen it from the Start menu.
The desktop shortcut is optional. Existing projects and settings are retained.
Internet access is needed for the existing media dependencies; official dependency
installers may request administrator approval. Setup also installs Python 3.13 for
your account (if missing) so browser chat can download its PySide6 runtime on first use.

This build adds Kick account sign-in and automatic stream-key setup. Open
Settings → Accounts → Kick → Connect. Kick chat and activity events are not
included. Windows YouTube sign-in now uses the operating system random generator.

The previous build added automatic fullscreen application and individual-window capture,
cursor capture, application audio, named audio devices, microphone suppression,
and music discovery through Windows media controls. See release-0.31-preview8.md.
Existing projects retain their capture settings. Select Automatic fullscreen
application explicitly to enable automatic capture; it does not capture a whole
monitor as a fallback. Windows 11 is recommended for application audio.

The Windows application and installer are cross-compiled. Native Windows
installation, capture, audio and music need user testing. Injected renderer
hooks are not included; automatic/window capture uses Windows Graphics Capture.
