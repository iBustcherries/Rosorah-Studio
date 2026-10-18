#!/usr/bin/env bash
# Builds Rosorah Studio from this source tree and packages it as an .rpm.
# Run on Fedora from anywhere:  packaging/rpm/build-rpm.sh
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
cd "$root"

for tool in cargo rpmbuild pkg-config python3; do
    command -v "$tool" >/dev/null || { echo "Missing '$tool'. See packaging/rpm/README.md for the packages to install." >&2; exit 1; }
done
pkg-config --exists gstreamer-1.0 || { echo "GStreamer development files are missing: sudo dnf install gstreamer1-devel gstreamer1-plugins-base-devel" >&2; exit 1; }

# RPM versions cannot contain '-', so 0.31.0-preview.9 becomes 0.31.0~preview.9 (sorts before 0.31.0).
cargo_version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
[ -n "$cargo_version" ] || { echo "Could not read the version from Cargo.toml" >&2; exit 1; }
tilde='~'
pkgversion="${cargo_version/-/$tilde}"

echo "== Compiling Rosorah Studio $cargo_version (this can take a long time)"
cargo build --release --locked

work="$root/target/rpm-work"
rm -rf "$work"
stage="$work/stage"
mkdir -p "$stage/browser"
cp target/release/rosorah-studio "$stage/"
cp -r browser/. "$stage/browser/"
# Open-licence fonts for text layers; each family folder carries its licence.
cp -r fonts "$stage/fonts"
cp packaging/install-renderer.py "$stage/"
cp packaging/rpm/rosorah-studio-wrapper "$work/"
cp packaging/io.rosorah.Studio.desktop "$work/"
cp packaging/io.rosorah.Studio.svg "$work/"

echo "== Building optional game-capture hooks (skipped if the 32-bit tools are missing)"
if (cd native && bash ./build.sh); then
    mkdir -p "$stage/hooks"
    cp -r native/build/64 native/build/32 "$stage/hooks/"
    cp native/build/rosorah-launch-watch "$stage/"
else
    echo "WARNING: hooks were not built. The package will work, but without automatic game capture." >&2
    echo "         To include them: sudo dnf install gcc-c++ glibc-devel.i686 libstdc++-devel.i686 vulkan-headers libglvnd-devel" >&2
fi

echo "== Packaging"
rpmbuild -bb "$here/rosorah-studio.spec" \
    --define "_topdir $work/rpmbuild" \
    --define "stagedir $stage" \
    --define "pkgversion $pkgversion"

mkdir -p "$root/dist"
cp "$work"/rpmbuild/RPMS/*/*.rpm "$root/dist/"
echo
echo "Done. Your package:"
ls -1 "$root/dist/"*.rpm
echo "Install it with:  sudo dnf install ./dist/<file>.rpm"
