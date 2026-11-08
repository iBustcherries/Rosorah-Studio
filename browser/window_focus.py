"""Bringing a helper's window to the front. Wayland lets a window take focus only with an activation
token from the app the user is in, so Rosorah sends one with each command that opens a window; Qt
takes it from XDG_ACTIVATION_TOKEN when the window is shown or raised, and clears it once used."""
import os
# Rosorah's own app id, so the desktop shows helper windows as Rosorah with its icon.
APP_ID='io.rosorah.Studio'
def bring_forward(window,token=None):
    if isinstance(token,str) and 0<len(token)<1024:os.environ['XDG_ACTIVATION_TOKEN']=token
    window.showNormal();window.raise_();window.activateWindow()
