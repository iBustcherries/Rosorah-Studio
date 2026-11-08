#!/usr/bin/env bash
# Starts a desktop session with no screen and runs browser/test_session.py in it.
# Usage: scripts/browser-session-test.sh wayland|x11 [scale] [python]
#   wayland: sway (headless, no XWayland, so nothing can fall back to X11)
#   x11:     Xvfb, the baseline the Wayland results are compared with
# python is the browser runtime's interpreter (PySide6 installed); default python3.
set -euo pipefail
cd "$(dirname "$0")/.."
session=${1:?wayland or x11}; scale=${2:-1}; python=${3:-python3}
export ROSORAH_SESSION_ARTIFACTS=${ROSORAH_SESSION_ARTIFACTS:-$PWD/session-artifacts}
mkdir -p "$ROSORAH_SESSION_ARTIFACTS"
export XDG_RUNTIME_DIR=$(mktemp -d); chmod 700 "$XDG_RUNTIME_DIR"
case $session in
wayland)
    unset DISPLAY
    cat >"$XDG_RUNTIME_DIR/sway.conf" <<EOF
xwayland disable
output HEADLESS-1 resolution 1920x1080 scale $scale
EOF
    WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 \
        sway -c "$XDG_RUNTIME_DIR/sway.conf" >"$ROSORAH_SESSION_ARTIFACTS/sway.log" 2>&1 &
    for _ in $(seq 50); do [ -S "$XDG_RUNTIME_DIR/wayland-1" ] && break; sleep 0.2; done
    export WAYLAND_DISPLAY=wayland-1
    export SWAYSOCK=$(ls "$XDG_RUNTIME_DIR"/sway-ipc.*.sock)
    swaymsg -t get_outputs -r | "$python" -c 'import json,sys;[print("output",o["name"],o["current_mode"]["width"],"x",o["current_mode"]["height"],"scale",o["scale"]) for o in json.load(sys.stdin)]'
    ;;
x11)
    unset WAYLAND_DISPLAY
    Xvfb :99 -screen 0 1920x1080x24 >"$ROSORAH_SESSION_ARTIFACTS/xvfb.log" 2>&1 &
    export DISPLAY=:99
    for _ in $(seq 50); do [ -e /tmp/.X11-unix/X99 ] && break; sleep 0.2; done
    ;;
*) echo "unknown session $session" >&2; exit 2 ;;
esac
"$python" -c 'from PySide6.QtWidgets import QApplication;a=QApplication([]);print("Qt platform:",a.platformName())'
exec "$python" browser/test_session.py -v
