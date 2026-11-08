"""How the browser helpers fit into the desktop session."""
import os
# Rosorah's own app id, so the desktop shows helper windows as Rosorah with its icon.
APP_ID='io.rosorah.Studio'
def prepare():
    """Call before Qt starts. On Wayland, Qt WebEngine sets EGL_PLATFORM from its GPU thread while
    Chromium's font threads read the environment; adding a variable then moves the environment
    and can crash them. Set beforehand, Qt's write only replaces the value."""
    if os.environ.get('WAYLAND_DISPLAY') and os.environ.get('QT_QPA_PLATFORM','wayland').startswith('wayland'):
        os.environ.setdefault('EGL_PLATFORM','wayland')
def bring_forward(window,token=None):
    """Wayland lets a window take focus only with an activation token from the app the user is in,
    so Rosorah sends one with each command that opens a window; Qt takes it from
    XDG_ACTIVATION_TOKEN when the window is shown or raised, and clears it once used."""
    if isinstance(token,str) and 0<len(token)<1024:os.environ['XDG_ACTIVATION_TOKEN']=token
    window.showNormal();window.raise_();window.activateWindow()
