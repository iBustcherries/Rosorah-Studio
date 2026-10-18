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
# Build tools and the compiled output are kept in podman volumes, so later builds are quicker.
podman run --rm \
    -v "$root:/src:ro,z" \
    -v "$root/dist:/dist:z" \
    -v "${CARGO_HOME:-$HOME/.cargo}/registry:/root/.cargo/registry:z" \
    -v rosorah-deb-rustup:/root/.rustup \
    -v rosorah-deb-target:/work/target \
    -v rosorah-deb-apt:/var/cache/apt/archives \
    docker.io/library/ubuntu:24.04 bash /src/packaging/deb/inside.sh
echo
echo "Done. Your package:"
ls -1 "$root/dist/"*.deb
echo "Install it with:  sudo apt install ./<file>.deb"
