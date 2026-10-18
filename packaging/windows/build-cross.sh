#!/usr/bin/env bash
# Cross-builds the Windows installer on Linux: the equivalent of build.ps1 followed by
# build-installer.ps1. Run from anywhere:
#   packaging/windows/build-cross.sh <gstreamer-devel.msi> <gstreamer-runtime.msi>
# The runtime MSI is bundled so setup never depends on GStreamer's download server.
#
# Needs (Fedora): clang lld llvm msitools mingw32-nsis rustup, a rustup toolchain with the
# x86_64-pc-windows-msvc target, and cargo-xwin. cargo-xwin downloads Microsoft's CRT and
# Windows SDK; set XWIN_ACCEPT_LICENSE=true only after accepting Microsoft's license terms.
# The MSI is the GStreamer 1.26.10 MSVC x86_64 *devel* package, checksum-verified below.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
cd "$root"
msi="${1:?Pass the gstreamer-1.0-devel-msvc-x86_64-1.26.10.msi path}"
runtime_msi="${2:?Pass the gstreamer-1.0-msvc-x86_64-1.26.10.msi (runtime) path}"
echo "8487a115fea3b0b0b4c55a9a413ad1adab0fe78f6681feedc60858a48613a121  $msi" | sha256sum -c --quiet
echo "a863bf3faa49e9f33bd3cc42967b473482d4dc98655ed95cba1ac59f26fb0cfb  $runtime_msi" | sha256sum -c --quiet
[ "${XWIN_ACCEPT_LICENSE:-}" = true ] || { echo "Set XWIN_ACCEPT_LICENSE=true after accepting Microsoft's license terms." >&2; exit 1; }

target=x86_64-pc-windows-msvc
export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR="$root/target-windows"
work="$CARGO_TARGET_DIR/cross"
mkdir -p "$work"

echo "== Unpacking GStreamer development files"
if [ ! -d "$work/msi" ]; then
    msiextract -C "$work/msi" "$msi" >/dev/null
fi
pc="$(find "$work/msi" -name gstreamer-1.0.pc -path '*lib/pkgconfig*' | head -1)"
[ -n "$pc" ] || { echo "gstreamer-1.0.pc not found in the MSI" >&2; exit 1; }
gst="$(cd "$(dirname "$pc")/../.." && pwd)"
# The .pc files carry the Windows install prefix; point them at the unpacked copy.
export PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_PATH= PKG_CONFIG_LIBDIR="$gst/lib/pkgconfig" PKG_CONFIG_SYSROOT_DIR=
export PKG_CONFIG="$work/pkg-config"
cat > "$PKG_CONFIG" <<EOF
#!/bin/sh
exec pkg-config --define-variable=prefix="$gst" "\$@"
EOF
chmod +x "$PKG_CONFIG"
"$PKG_CONFIG" --libs gstreamer-1.0 >/dev/null

echo "== Compiling Rosorah Studio for Windows"
(cd "$here" && llvm-rc -no-preprocess -fo "$work/rosorah.res" rosorah.rc)
cargo xwin rustc --locked --release --target $target -p rosorah-studio --bin rosorah-studio -- -C link-arg="$work/rosorah.res"
cargo xwin build --locked --release --target $target -p rosorah-windows

echo "== Compiling the launcher"
mkdir -p "$work/launcher/src"
cp "$here/launcher.rs" "$work/launcher/src/main.rs"
cat > "$work/launcher/Cargo.toml" <<'EOF'
[package]
name = "rosorah-launcher"
version = "0.0.0"
edition = "2024"
[workspace]
[profile.release]
opt-level = "s"
strip = true
EOF
(cd "$work/launcher" && cargo xwin rustc --release --target $target -- -C link-arg="$work/rosorah.res")

echo "== Staging the application folder"
bundle="$work/bundle"
rm -rf "$bundle"
mkdir -p "$bundle"
out="$CARGO_TARGET_DIR/$target/release"
cp "$out/rosorah-studio.exe" "$out/rosorah-windows-probe.exe" "$bundle/"
cp "$CARGO_TARGET_DIR/$target/release/rosorah-launcher.exe" "$bundle/Rosorah.exe"
cp -r browser "$bundle/browser"
rm -rf "$bundle/browser/__pycache__"
cp -r fonts "$bundle/fonts"
cp docs/windows-integration-checkpoint.md "$bundle/README.md"
cp "$here/run.cmd" "$here/setup.cmd" "$here/setup.ps1" "$here/rosorah.ico" "$bundle/"
mkdir -p "$bundle/runtime"
cp "$runtime_msi" "$bundle/runtime/gstreamer-1.26.10.msi"
cp -r docs/windows-third-party "$bundle/THIRD-PARTY"
cp docs/release-0.31-preview6.md "$bundle/"
cp "$here/installed-readme.md" "$bundle/START-HERE.md"
# cmd.exe misreads batch files without Windows (CRLF) line endings.
for f in "$bundle"/*.cmd; do sed -i 's/\r$//; s/$/\r/' "$f"; done

echo "== Building the installer"
(
    cd "$bundle"
    find . -type f | sed 's|^\./||' | sort | while read -r f; do printf 'Delete "$INSTDIR\\%s"\n' "${f//\//\\}"; done
    find . -mindepth 1 -type d | sed 's|^\./||' | awk '{print length, $0}' | sort -rn | cut -d' ' -f2- | while read -r d; do printf 'RMDir "$INSTDIR\\%s"\n' "${d//\//\\}"; done
) > "$here/uninstall-files.nsh"
mkdir -p dist
setup="$root/dist/Rosorah-Studio-0.32.0-preview.8-Setup.exe"
makensis -WX -V2 "-DPAYLOAD=$bundle" "-DOUTPUT=$setup" "$here/installer.nsi"
sha256sum "$setup"
