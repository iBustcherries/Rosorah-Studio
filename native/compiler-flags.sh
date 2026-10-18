# Sourced by build/test scripts. Standard multilib compiler unless a prefix is supplied.
compiler_flags() {
    flags=()
    if [[ $1 == 32 ]]; then
        flags+=(-m32)
        if [[ -n ${ROSORAH_MULTILIB_ROOT:-} ]]; then
            local root=$ROSORAH_MULTILIB_ROOT
            flags+=(-B"$root/usr/lib/gcc/x86_64-linux-gnu/13/32/" -B"$root/usr/lib32/"
                -I"$root/usr/include" -I"$root/usr/include/x86_64-linux-gnu"
                -I"$root/usr/include/x86_64-linux-gnu/c++/13/32" -I/usr/include/x86_64-linux-gnu
                -L"$root/usr/lib32" -Wl,--sysroot="$root")
        fi
    fi
}
