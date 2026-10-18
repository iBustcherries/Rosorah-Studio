#!/usr/bin/env bash
set -eu
cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
if ldd ./rosorah-studio 2>/dev/null | grep -q 'not found'; then
    echo 'Missing runtime libraries:' >&2
    ldd ./rosorah-studio | grep 'not found' >&2
    echo 'On Fedora: sudo dnf install gstreamer1 gstreamer1-plugins-base pipewire-gstreamer pipewire-utils' >&2
    exit 1
fi
for tool in pw-record pw-dump pw-link; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo 'Audio capture needs: sudo dnf install pipewire-utils' >&2
        break
    fi
done
if ! command -v ffmpeg >/dev/null 2>&1; then
    echo 'Recording and streaming need FFmpeg. On Fedora: sudo dnf install ffmpeg-free' >&2
fi
python3 ./install-renderer.py --quiet
exec ./rosorah-studio "$@"
