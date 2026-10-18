#!/usr/bin/env bash
# Runs inside the Ubuntu container started by build-deb.sh.
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
rm -f /etc/apt/apt.conf.d/docker-clean
apt-get update -qq
apt-get install -y -qq --no-install-recommends ca-certificates curl build-essential pkg-config \
    libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev libdrm-dev \
    g++-multilib libvulkan-dev libgl-dev rsync >/dev/null
export PATH="/root/.cargo/bin:$PATH" CARGO_TARGET_DIR=/work/target
if ! command -v cargo >/dev/null; then
    curl -sSf https://sh.rustup.rs | sh -s -- -y -q --profile minimal --default-toolchain 1.99.0 --no-modify-path
fi
rustup default 1.99.0 >/dev/null 2>&1 || true

# Build from a copy so nothing is written into the source folder.
mkdir -p /work/src
rsync -a --delete --exclude target --exclude target-windows --exclude dist --exclude .git /src/ /work/src/
cd /work/src
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
# Debian versions sort "~" before release, like the RPM: 0.32.0~preview.10.
debversion="${version/-/\~}"
echo "== Compiling Rosorah Studio $version for Ubuntu/Debian (this can take a long time)"
cargo build --release --locked --offline

pkg=/work/pkg
rm -rf "$pkg"
stage="$pkg/opt/rosorah-studio"
mkdir -p "$stage/browser" "$pkg/usr/bin" "$pkg/usr/share/applications" "$pkg/usr/share/icons/hicolor/scalable/apps" "$pkg/DEBIAN"
cp "$CARGO_TARGET_DIR/release/rosorah-studio" "$stage/"
cp -r browser/. "$stage/browser/"
rm -rf "$stage/browser/__pycache__"
cp -r fonts "$stage/fonts"
cp packaging/install-renderer.py "$stage/"
install -m 0755 packaging/rpm/rosorah-studio-wrapper "$pkg/usr/bin/rosorah-studio"
install -m 0644 packaging/io.rosorah.Studio.desktop "$pkg/usr/share/applications/"
install -m 0644 packaging/io.rosorah.Studio.svg "$pkg/usr/share/icons/hicolor/scalable/apps/"

echo "== Building optional game-capture hooks (skipped if they don't build here)"
if (cd native && rm -rf build && bash ./build.sh); then
    mkdir -p "$stage/hooks"
    cp -r native/build/64 native/build/32 "$stage/hooks/"
    cp native/build/rosorah-launch-watch "$stage/"
else
    echo "WARNING: hooks were not built. The package will work, but without automatic game capture." >&2
fi

size="$(du -sk "$pkg" | cut -f1)"
cat > "$pkg/DEBIAN/control" <<CONTROL
Package: rosorah-studio
Version: ${debversion}-1
Architecture: amd64
Maintainer: iBustcherries <102161061+iBustcherries@users.noreply.github.com>
Installed-Size: ${size}
Depends: libc6 (>= 2.39), libgstreamer1.0-0, libgstreamer-plugins-base1.0-0, libdrm2, gstreamer1.0-plugins-base, gstreamer1.0-plugins-good, gstreamer1.0-libav, gstreamer1.0-pipewire, pipewire-bin, python3, python3-venv, ffmpeg
Recommends: gstreamer1.0-plugins-bad, gstreamer1.0-vaapi, xdg-utils
Section: video
Priority: optional
Homepage: https://github.com/iBustcherries/RosorahStudio
Description: Streaming and recording studio
 Rosorah Studio is a streaming and recording application with scenes, two canvases,
 multistreaming, unified chat, alerts and automations. This is an unsigned
 development package.
CONTROL
out="/dist/rosorah-studio_${debversion}-1_amd64.deb"
dpkg-deb --root-owner-group --build "$pkg" "$out" >/dev/null
echo "Built $out"
