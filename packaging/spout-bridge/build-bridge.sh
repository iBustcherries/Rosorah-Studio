#!/usr/bin/env bash
# Builds rosorah-spout-bridge.exe, the small Windows helper the Linux packages use to bring in
# Spout pictures from Windows apps running in Wine or Proton. Needs cargo-xwin (see
# packaging/windows/build-cross.sh); the C runtime is linked in so Wine needs no extra DLLs.
# Usage: build-bridge.sh <output dir>
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
out="$1"
export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR="$root/target-windows/bridge" XWIN_ACCEPT_LICENSE=true
export RUSTFLAGS="-C target-feature=+crt-static"
(cd "$root" && cargo xwin build --locked --release --target x86_64-pc-windows-msvc -p rosorah-windows --bin rosorah-spout-bridge)
mkdir -p "$out"
install -m 0644 "$CARGO_TARGET_DIR/x86_64-pc-windows-msvc/release/rosorah-spout-bridge.exe" "$out/"
