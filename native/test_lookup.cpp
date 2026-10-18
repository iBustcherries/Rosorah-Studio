#include <dlfcn.h>
#include <cassert>
#include <cstdio>
#if defined(FIRST)
extern "C" int rosorah_lookup_fixture() {
    auto next=reinterpret_cast<int(*)()>(dlsym(RTLD_NEXT,"rosorah_lookup_fixture"));
    return next?next()+1:-1;
}
#elif defined(SECOND)
extern "C" int rosorah_lookup_fixture(){return 7;}
#else
int main() {
    auto first=reinterpret_cast<int(*)()>(dlsym(RTLD_DEFAULT,"rosorah_lookup_fixture"));
    assert(first && first()==8);
    puts("PASS: dlsym RTLD_NEXT preserves caller scope with other preload libraries");
}
#endif
