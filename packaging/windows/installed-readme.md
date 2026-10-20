# Rosorah Studio for Windows

Start Rosorah Studio from the Start menu (or the desktop shortcut, if you chose one).

## Updating

Run the new Setup EXE. Close Rosorah first; the installer asks you to if it is still open.
Your projects, settings and signed-in accounts are kept, because they are stored apart
from the application.

## What setup installs

The installer includes GStreamer. If they are missing, it also installs the Microsoft
Visual C++ Runtime, FFmpeg, and Python 3.13 for your account (browser chat and web page
sources use Python). These downloads need internet access, and the official installers
may ask for administrator permission. Your graphics drivers are not changed.

If setup stops part way, `setup.log` in this folder says why. Run the Setup EXE again to retry.

## Virtual camera

Windows 11 can show Rosorah as a webcam in Discord, Zoom or a browser. Press
**Set up virtual camera** in the Stream panel once (Windows asks for permission).
After an update, the button may read **Update virtual camera**; press it once more.

## If something goes wrong

- Rosorah's log from the last start is in `%LOCALAPPDATA%\RosorahStudio\logs\startup.log`.
- Report problems at https://github.com/iBustcherries/Rosorah-Studio/issues.

## Uninstalling

Use Windows Settings → Apps → Installed apps → Rosorah Studio. Your projects and
settings are left in place.
