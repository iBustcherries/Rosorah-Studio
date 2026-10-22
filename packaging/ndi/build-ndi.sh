#!/usr/bin/env bash
# Builds GStreamer's NDI plugin (gst-plugin-ndi from gst-plugins-rs, MPL-2.0) for the Linux
# packages; Fedora and Ubuntu don't ship it. Rosorah loads it from /opt/rosorah-studio/gstreamer.
# The NDI runtime itself (libndi) is not included: users install it from ndi.video.
# Usage: build-ndi.sh <work dir> <output dir>   (add --offline to cargo with CARGO_NET_OFFLINE=true)
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
work="$1" out="$2"
version=0.15.4
sum=d31649d4056f9aa3a7b2ddfc81ef2dc3156a214f50361e88226d2ebdf2e6a415
crate="$here/gst-plugin-ndi-$version.crate"
if [ ! -f "$crate" ]; then
    curl -sSfL -o "$crate.part" "https://static.crates.io/crates/gst-plugin-ndi/gst-plugin-ndi-$version.crate"
    mv "$crate.part" "$crate"
fi
echo "$sum  $crate" | sha256sum -c --quiet - || { echo "gst-plugin-ndi download doesn't match its checksum" >&2; rm -f "$crate"; exit 1; }
rm -rf "$work/ndi-src" && mkdir -p "$work/ndi-src"
tar xzf "$crate" -C "$work/ndi-src"
src="$work/ndi-src/gst-plugin-ndi-$version"
# Built on its own, not as part of Rosorah's workspace.
printf '\n[workspace]\n' >> "$src/Cargo.toml"
(cd "$src" && CARGO_TARGET_DIR="$work/ndi-build" cargo build --release --locked)
mkdir -p "$out"
install -m 0755 "$work/ndi-build/release/libgstndi.so" "$out/"
install -m 0644 "$src/LICENSE-MPL-2.0" "$out/"
echo "gst-plugin-ndi $version, source: https://crates.io/crates/gst-plugin-ndi and https://gitlab.freedesktop.org/gstreamer/gst-plugins-rs" > "$out/README-NDI.txt"
