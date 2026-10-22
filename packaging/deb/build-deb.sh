#!/usr/bin/env bash
# Builds Rosorah Studio as a .deb for Ubuntu 24.04+ and Debian 13+.
# It compiles inside an Ubuntu 24.04 container (podman), because a program built on a newer
# distribution needs a newer C library than Ubuntu and Debian have.
# Run from anywhere:  packaging/deb/build-deb.sh
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
command -v podman >/dev/null || { echo "Missing 'podman': sudo dnf install podman" >&2; exit 1; }
mkdir -p "$root/dist" "${CARGO_HOME:-$HOME/.cargo}/registry"
# The Spout bridge is a Windows program, so it's built here (cargo-xwin) and handed in.
bridge="$root/target-windows/bridge-out"
rm -rf "$bridge" && mkdir -p "$bridge"
bash "$here/../spout-bridge/build-bridge.sh" "$bridge" || echo "WARNING: the Spout bridge was not built. The package will work, but without Spout." >&2
# Build tools and the compiled output are kept in podman volumes, so later builds are quicker.
podman run --rm \
    -v "$root:/src:ro,z" \
    -v "$root/dist:/dist:z" \
    -v "$bridge:/bridge:ro,z" \
    -v "${CARGO_HOME:-$HOME/.cargo}/registry:/root/.cargo/registry:z" \
    -v rosorah-deb-rustup:/root/.rustup \
    -v rosorah-deb-target:/work/target \
    -v rosorah-deb-apt:/var/cache/apt/archives \
    docker.io/library/ubuntu:24.04 bash /src/packaging/deb/inside.sh
echo
echo "Done. Your package:"
ls -1 "$root/dist/"*.deb
echo "Install it with:  sudo apt install ./<file>.deb"
