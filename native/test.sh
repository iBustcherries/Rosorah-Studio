#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
. ./compiler-flags.sh
blocked=0
run_checked() {
    local status=0
    "$@" || status=$?
    if [[ $status == 77 ]]; then blocked=1; elif [[ $status != 0 ]]; then exit "$status"; fi
}
mkdir -p build
"${CXX:-c++}" -std=c++17 -O2 -Wall -Wextra -Werror test_launch.cpp -o build/test-launch
build/test-launch
for arch in ${ROSORAH_TEST_ARCHES:-64}; do
    compiler_flags "$arch"
    dir="build/test-$arch"
    mkdir -p "$dir/manifest"
    cxx=("${CXX:-c++}" -std=c++17 -O2 -foptimize-sibling-calls -g -Wall -Wextra -Werror -Wno-missing-field-initializers ${CPPFLAGS:-} "${flags[@]}")
    for api in vulkan gl-capture; do
        source=vulkan_layer.cpp;[[ $api != gl-capture ]] || source=gl_capture.cpp
        "${cxx[@]}" -fPIC -shared -fvisibility=hidden -DROSORAH_TESTING "$source" -o "$dir/librosorah-$api-test.so" -pthread -ldl -Wl,-Bsymbolic -Wl,-z,defs
    done
    mv "$dir/librosorah-gl-capture-test.so" "$dir/librosorah-gl-capture.so"
    "${CC:-cc}" -std=c11 -O2 -foptimize-sibling-calls -fPIC -shared -fvisibility=hidden -DROSORAH_TESTING -Wall -Wextra -Werror ${CPPFLAGS:-} "${flags[@]}" gl_loader.c -o "$dir/librosorah-gl-test.so" -Wl,--no-as-needed -l:libdl.so.2 -Wl,--as-needed -Wl,-Bsymbolic -Wl,-z,defs
    "${cxx[@]}" test_layer.cpp -o "$dir/test-layer" -ldl
    "${cxx[@]}" test_color.cpp -o "$dir/test-color"
    "${cxx[@]}" test_transport.cpp -o "$dir/test-transport" -pthread
    test_libs=();runner=()
    if [[ $arch == 32 ]]; then
        [[ -z ${ROSORAH_TEST_LIBDIR32:-} ]] || test_libs+=(-L"$ROSORAH_TEST_LIBDIR32" -Wl,-rpath-link,"$ROSORAH_TEST_LIBDIR32")
        [[ -z ${ROSORAH_TEST_LOADER32:-} ]] || runner=("$ROSORAH_TEST_LOADER32" --library-path "${ROSORAH_TEST_LIBDIR32:?}")
    fi
    "${cxx[@]}" test_gl.cpp -o "$dir/test-gl" "${test_libs[@]}" -l:libEGL.so.1 -l:libGL.so.1 -ldl
    "${cxx[@]}" -DROSORAH_LOCAL_LOAD test_gl.cpp -o "$dir/test-gl-local" -ldl
    "${cxx[@]}" -fPIC -shared -DFIRST test_lookup.cpp -o "$dir/lookup-first.so" -ldl
    "${cxx[@]}" -fPIC -shared -DSECOND test_lookup.cpp -o "$dir/lookup-second.so"
    "${cxx[@]}" test_lookup.cpp -o "$dir/test-lookup" -ldl
    python3 - "$dir" <<'PY'
import json,pathlib,sys
root=pathlib.Path(sys.argv[1]).resolve()
(root/'manifest/rosorah-test.json').write_text(json.dumps({'file_format_version':'1.2.0','layer':{'name':'VK_LAYER_ROSORAH_capture','type':'GLOBAL','library_path':str(root/'librosorah-vulkan-test.so'),'api_version':'1.3.0','implementation_version':'1','description':'Rosorah test layer'}}))
PY
    color_status=0
    "${runner[@]}" "$dir/test-color" || color_status=$?
    if [[ $arch == 32 && $color_status == 126 ]]; then
        echo 'BLOCKED: this host cannot execute 32-bit ELF programs; 32-bit libraries and tests compiled successfully.'
        blocked=1
        continue
    elif [[ $color_status != 0 ]]; then
        exit "$color_status"
    fi
    for mode in core es msaa; do
        if [[ ${#runner[@]} == 0 ]]; then
            run_checked env LD_PRELOAD="$PWD/$dir/librosorah-gl-test.so" LIBGL_ALWAYS_SOFTWARE=1 "$dir/test-gl" "$mode"
        else
            # Apply the 32-bit preload through its loader, not a 64-bit env binary.
            run_checked env LIBGL_ALWAYS_SOFTWARE=1 LIBGL_DRIVERS_PATH="${ROSORAH_TEST_LIBDIR32:?}/dri" "${runner[@]}" --preload "$PWD/$dir/librosorah-gl-test.so" "$dir/test-gl" "$mode"
        fi
    done
    mkdir -p "$dir/fallback"
    cp "$dir/librosorah-gl-test.so" "$dir/fallback/librosorah-gl.so"
    if [[ ${#runner[@]} == 0 ]]; then
        run_checked env LD_PRELOAD="$PWD/$dir/fallback/librosorah-gl.so" LIBGL_ALWAYS_SOFTWARE=1 "$dir/test-gl" passthrough
    else
        run_checked env LIBGL_ALWAYS_SOFTWARE=1 LIBGL_DRIVERS_PATH="${ROSORAH_TEST_LIBDIR32:?}/dri" "${runner[@]}" --preload "$PWD/$dir/fallback/librosorah-gl.so" "$dir/test-gl" passthrough
    fi
    preloads="$PWD/$dir/librosorah-gl-test.so:$PWD/$dir/lookup-first.so:$PWD/$dir/lookup-second.so"
    if [[ ${#runner[@]} == 0 ]]; then
        run_checked env LD_PRELOAD="$PWD/$dir/librosorah-gl-test.so" LIBGL_ALWAYS_SOFTWARE=1 "$dir/test-gl-local" core
        run_checked env LD_PRELOAD="$preloads" "$dir/test-lookup"
    else
        run_checked env LIBGL_ALWAYS_SOFTWARE=1 LIBGL_DRIVERS_PATH="${ROSORAH_TEST_LIBDIR32:?}/dri" "${runner[@]}" --preload "$PWD/$dir/librosorah-gl-test.so" "$dir/test-gl-local" core
        run_checked "${runner[@]}" --preload "$preloads" "$dir/test-lookup"
    fi
    icd=${VK_ICD_FILENAMES:-}
    [[ $arch != 32 ]] || icd=${ROSORAH_TEST_ICD32:-$icd}
    for format in sdr sdr10 pq pq-bgr hlg scrgb; do
        run_checked env ${icd:+VK_ICD_FILENAMES=$icd} VK_LAYER_PATH="$PWD/$dir/manifest${VK_LAYER_PATH:+:$VK_LAYER_PATH}" "${runner[@]}" "$dir/test-layer" "$PWD/$dir/librosorah-vulkan-test.so" "$format"
    done
    run_checked "${runner[@]}" "$dir/test-transport"
    if [[ $arch == 64 ]]; then
        # A real game process started WITHOUT LD_PRELOAD. The observer installs
        # it at the exec stop; the fixture then verifies actual GL pixels.
        fixture="$PWD/build/launch-fixture/rosorah-studio/renderer-0.8.0"
        for libdir in lib64 lib/x86_64-linux-gnu; do
            mkdir -p "$fixture/$libdir"
            cp "$dir/librosorah-gl-test.so" "$fixture/$libdir/librosorah-gl.so"
            cp "$dir/librosorah-gl-capture.so" "$fixture/$libdir/librosorah-gl-capture.so"
        done
        run_checked env LIBGL_ALWAYS_SOFTWARE=1 build/test-launch --live "$fixture/\$LIB/librosorah-gl.so" "$PWD/$dir/test-gl"
        run_checked env LIBGL_ALWAYS_SOFTWARE=1 build/test-launch --live-wrappers "$fixture/\$LIB/librosorah-gl.so" "$PWD/$dir/test-gl"
        run_checked env LIBGL_ALWAYS_SOFTWARE=1 build/test-launch --live-chain "$fixture/\$LIB/librosorah-gl.so" "$PWD/$dir/test-gl"
    fi
done
exit "$((blocked*77))"
