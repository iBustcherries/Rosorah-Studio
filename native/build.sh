#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
# Fedora: gcc-c++ glibc-devel.i686 libstdc++-devel.i686 vulkan-headers libglvnd-devel
# ROSORAH_MULTILIB_ROOT is only needed for a locally staged multilib toolchain.
. ./compiler-flags.sh
mkdir -p build
"${CXX:-c++}" -std=c++17 -O2 -Wall -Wextra -Werror launch_watch.cpp -o build/rosorah-launch-watch
for arch in 64 32; do
    compiler_flags "$arch"
    mkdir -p "build/$arch"
    for api in vulkan gl-capture; do
        source=vulkan_layer.cpp
        [[ $api != gl-capture ]] || source=gl_capture.cpp
        "${CXX:-c++}" -std=c++17 -O2 -foptimize-sibling-calls -fPIC -shared -fvisibility=hidden \
            -Wall -Wextra -Werror -Wno-missing-field-initializers ${CPPFLAGS:-} "${flags[@]}" \
            "$source" -o "build/$arch/librosorah-$api.so" -ldl -pthread -Wl,-Bsymbolic -Wl,-z,defs
    done
    "${CC:-cc}" -std=c11 -O2 -foptimize-sibling-calls -fPIC -shared -fvisibility=hidden -Wall -Wextra -Werror ${CPPFLAGS:-} "${flags[@]}" \
        gl_loader.c -o "build/$arch/librosorah-gl.so" -Wl,--no-as-needed -l:libdl.so.2 -Wl,--as-needed -Wl,-Bsymbolic -Wl,-z,defs
done
