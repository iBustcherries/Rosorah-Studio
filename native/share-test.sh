#!/usr/bin/env bash
# Game capture on the graphics card, end to end: vkcube runs with a freshly built layer in an
# off-screen KDE session, and Rosorah's receiver test draws its shared frames through the real
# compositor. Its own HOME keeps it away from a Rosorah that's already open.
# Needs kwin_wayland, vkcube and a GPU whose driver has VK_KHR_external_memory_fd.
# Options (environment): ROSORAH_SHARE_TEST_MEMORY=1 uses the memory path instead, to compare;
# ROSORAH_SHARE_TEST_PNG=<file> saves a drawn frame; ROSORAH_SHARE_TEST_WSI=xcb runs vkcube through
# XWayland (8-bit BGRA rather than KWin's 10-bit); as a benchmark, ROSORAH_SHARE_TEST_SECONDS,
# ROSORAH_SHARE_TEST_RESOLUTION (0 720p … 4 original), ROSORAH_SHARE_TEST_WIDTH/HEIGHT (screen) and
# ROSORAH_SHARE_TEST_CUBE (vkcube options, such as "--width 2560 --height 1440").
set -euo pipefail
cd "$(dirname "$0")"
root=$(cd .. && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/home" "$work/layer"
"${CXX:-c++}" -std=c++17 -O2 -fPIC -shared -fvisibility=hidden -Wall -Wextra -Werror -Wno-missing-field-initializers \
    vulkan_layer.cpp -o "$work/layer/librosorah-vulkan.so" -ldl -pthread -Wl,-Bsymbolic -Wl,-z,defs
cat > "$work/layer/rosorah-test.json" <<EOF
{"file_format_version":"1.2.0","layer":{"name":"VK_LAYER_ROSORAH_capture","type":"GLOBAL","library_path":"$work/layer/librosorah-vulkan.so","api_version":"1.3.0","implementation_version":"1","description":"Rosorah share test"}}
EOF
test=$(cd "$root" && cargo test -p rosorah-studio --no-run --message-format=json 2>/dev/null | python3 -c '
import json,sys
for line in sys.stdin:
    m=json.loads(line)
    if m.get("reason")=="compiler-artifact" and m.get("executable") and m["target"]["name"]=="rosorah-studio" and m["profile"]["test"]:
        print(m["executable"])')
cat > "$work/session.sh" <<EOF
#!/bin/sh
# Other implicit layers (an installed Rosorah, OBS's) stay out of the way.
HOME="$work/home" VK_LOADER_LAYERS_DISABLE='~implicit~' VK_LAYER_PATH="$work/layer" VK_INSTANCE_LAYERS=VK_LAYER_ROSORAH_capture \
    vkcube --wsi ${ROSORAH_SHARE_TEST_WSI:-wayland} ${ROSORAH_SHARE_TEST_CUBE:-} >"$work/cube.txt" 2>&1 &
cube=\$!
ROSORAH_SHARE_TEST_PNG="${ROSORAH_SHARE_TEST_PNG:-}" ROSORAH_SHARE_TEST_MEMORY="${ROSORAH_SHARE_TEST_MEMORY:-}" ROSORAH_SHARE_TEST_SECONDS="${ROSORAH_SHARE_TEST_SECONDS:-}" ROSORAH_SHARE_TEST_RESOLUTION="${ROSORAH_SHARE_TEST_RESOLUTION:-}" HOME="$work/home" /usr/bin/time -f "studio cpu %U user %S system" "$test" a_vulkan_game_shares_its_frames_on_the_graphics_card --ignored --nocapture > "$work/result.txt" 2>&1
echo "exit \$?" >> "$work/result.txt"
# The game's CPU time (user + system, in clock ticks) for the benchmark.
echo "game cpu ticks \$(awk '{print \$14+\$15}' /proc/\$cube/stat)" >> "$work/result.txt"
kill \$cube
EOF
chmod +x "$work/session.sh"
kwin_wayland --virtual --xwayland --width ${ROSORAH_SHARE_TEST_WIDTH:-1280} --height ${ROSORAH_SHARE_TEST_HEIGHT:-720} --no-lockscreen --exit-with-session "$work/session.sh" >/dev/null 2>&1 || true
cat "$work/cube.txt" "$work/result.txt"
grep -q "^exit 0" "$work/result.txt"
