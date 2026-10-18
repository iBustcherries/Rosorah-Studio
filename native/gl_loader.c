// Minimal session preload. Loading the C++ capture backend is optional and lazy,
// so an older Steam runtime cannot break unrelated programs at process startup.
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdlib.h>
#include <stdatomic.h>
#include <limits.h>
#include <GL/glx.h>
#include <EGL/egl.h>
#define EXPORT __attribute__((visibility("default")))
extern void* compat_dlopen(const char*,int);
extern int compat_dlclose(void*);
extern char* compat_dlerror(void);
extern int compat_dladdr(const void*,Dl_info*);
extern void* compat_dlvsym(void*,const char*,const char*);
#if defined(__x86_64__)
__asm__(".symver compat_dlopen,dlopen@GLIBC_2.2.5");
__asm__(".symver compat_dlclose,dlclose@GLIBC_2.2.5");
__asm__(".symver compat_dlerror,dlerror@GLIBC_2.2.5");
__asm__(".symver compat_dladdr,dladdr@GLIBC_2.2.5");
__asm__(".symver compat_dlvsym,dlvsym@GLIBC_2.2.5");
#define SYMBOL_VERSION "GLIBC_2.2.5"
#else
__asm__(".symver compat_dlopen,dlopen@GLIBC_2.1");
__asm__(".symver compat_dlclose,dlclose@GLIBC_2.0");
__asm__(".symver compat_dlerror,dlerror@GLIBC_2.0");
__asm__(".symver compat_dladdr,dladdr@GLIBC_2.0");
__asm__(".symver compat_dlvsym,dlvsym@GLIBC_2.1");
#define SYMBOL_VERSION "GLIBC_2.0"
#endif
typedef void*(*Symbol)(void*,const char*);
static Symbol real_symbol(void) {
    return (Symbol)compat_dlvsym(RTLD_NEXT,"dlsym",SYMBOL_VERSION);
}
static _Atomic(uintptr_t) backend_state; // 0 untried, 1 loading, 2 unavailable, otherwise handle.
static void* backend(void) {
    uintptr_t state=atomic_load_explicit(&backend_state,memory_order_acquire);
    if(state>2)return (void*)state;
    if(state || getenv("ROSORAH_DISABLE_CAPTURE"))return NULL;
    uintptr_t expected=0;
    if(!atomic_compare_exchange_strong(&backend_state,&expected,1))return NULL;
    Dl_info info={0};char path[PATH_MAX];void* handle=NULL;
    if(compat_dladdr((const void*)&backend,&info) && info.dli_fname) {
        const char* slash=strrchr(info.dli_fname,'/');
        static const char filename[]="librosorah-gl-capture.so";
        size_t prefix=slash?(size_t)(slash-info.dli_fname)+1:0;
        if(prefix && prefix+sizeof(filename)<=sizeof(path)) {
            memcpy(path,info.dli_fname,prefix);memcpy(path+prefix,filename,sizeof(filename));
            handle=compat_dlopen(path,RTLD_NOW|RTLD_LOCAL);
        }
    }
    if(!handle)compat_dlerror(); // An optional backend failure must not poison a successful app dlsym.
    atomic_store_explicit(&backend_state,handle?(uintptr_t)handle:2,memory_order_release);
    return handle;
}
static int intercepted(const char* name) {
    static const char* names[]={"glXSwapBuffers","glXSwapBuffersMscOML","eglSwapBuffers",
        "eglSwapBuffersWithDamageKHR","eglSwapBuffersWithDamageEXT","glXGetProcAddress",
        "glXGetProcAddressARB","eglGetProcAddress","glXDestroyContext","eglDestroyContext",
        "eglDestroySurface","glXDestroyWindow","glXDestroyPbuffer"};
    for(size_t i=0;i<sizeof(names)/sizeof(names[0]);i++)if(strcmp(name,names[i])==0)return 1;
    return 0;
}
static void* lookup(const char* name) {
    Symbol real=real_symbol();if(!real)return NULL;
    void* capture=backend();
    if(capture){void* fn=real(capture,name);if(fn)return fn;}
    void* fn=real(RTLD_NEXT,name);if(fn)return fn;
    // No new driver is loaded by the fallback; handle RTLD_LOCAL engine loads.
    static const char* libraries[]={"libGL.so.1","libEGL.so.1","libGLX.so.0"};
    for(size_t i=0;i<sizeof(libraries)/sizeof(libraries[0]);i++) {
        void* library=compat_dlopen(libraries[i],RTLD_LAZY|RTLD_NOLOAD);
        if(library){fn=real(library,name);compat_dlclose(library);if(fn)return fn;}
    }
    return NULL;
}
EXPORT void* dlsym(void* handle,const char* name) {
    Symbol real=real_symbol();if(!real)return NULL;
    // Tail call preserves the caller address used by glibc for RTLD_NEXT.
    if(handle==RTLD_NEXT || !intercepted(name))return real(handle,name);
    void* original=real(handle,name);if(!original)return NULL;
    void* capture=backend();return capture?real(capture,name):original;
}
EXPORT void glXSwapBuffers(Display* d,GLXDrawable s) {typedef void(*Fn)(Display*,GLXDrawable);Fn fn=(Fn)lookup("glXSwapBuffers");if(fn)fn(d,s);}
EXPORT int64_t glXSwapBuffersMscOML(Display* d,GLXDrawable s,int64_t a,int64_t b,int64_t c) {typedef int64_t(*Fn)(Display*,GLXDrawable,int64_t,int64_t,int64_t);Fn fn=(Fn)lookup("glXSwapBuffersMscOML");return fn?fn(d,s,a,b,c):0;}
EXPORT EGLBoolean eglSwapBuffers(EGLDisplay d,EGLSurface s) {typedef EGLBoolean(*Fn)(EGLDisplay,EGLSurface);Fn fn=(Fn)lookup("eglSwapBuffers");return fn?fn(d,s):EGL_FALSE;}
EXPORT EGLBoolean eglSwapBuffersWithDamageKHR(EGLDisplay d,EGLSurface s,const EGLint* r,EGLint n) {typedef EGLBoolean(*Fn)(EGLDisplay,EGLSurface,const EGLint*,EGLint);Fn fn=(Fn)lookup("eglSwapBuffersWithDamageKHR");return fn?fn(d,s,r,n):EGL_FALSE;}
EXPORT EGLBoolean eglSwapBuffersWithDamageEXT(EGLDisplay d,EGLSurface s,const EGLint* r,EGLint n) {typedef EGLBoolean(*Fn)(EGLDisplay,EGLSurface,const EGLint*,EGLint);Fn fn=(Fn)lookup("eglSwapBuffersWithDamageEXT");return fn?fn(d,s,r,n):EGL_FALSE;}
EXPORT __GLXextFuncPtr glXGetProcAddress(const GLubyte* n) {typedef __GLXextFuncPtr(*Fn)(const GLubyte*);Fn fn=(Fn)lookup("glXGetProcAddress");return fn?fn(n):NULL;}
EXPORT __GLXextFuncPtr glXGetProcAddressARB(const GLubyte* n) {typedef __GLXextFuncPtr(*Fn)(const GLubyte*);Fn fn=(Fn)lookup("glXGetProcAddressARB");return fn?fn(n):NULL;}
EXPORT __eglMustCastToProperFunctionPointerType eglGetProcAddress(const char* n) {typedef __eglMustCastToProperFunctionPointerType(*Fn)(const char*);Fn fn=(Fn)lookup("eglGetProcAddress");return fn?fn(n):NULL;}
EXPORT void glXDestroyContext(Display* d,GLXContext c) {typedef void(*Fn)(Display*,GLXContext);Fn fn=(Fn)lookup("glXDestroyContext");if(fn)fn(d,c);}
EXPORT EGLBoolean eglDestroyContext(EGLDisplay d,EGLContext c) {typedef EGLBoolean(*Fn)(EGLDisplay,EGLContext);Fn fn=(Fn)lookup("eglDestroyContext");return fn?fn(d,c):EGL_FALSE;}
EXPORT EGLBoolean eglDestroySurface(EGLDisplay d,EGLSurface s) {typedef EGLBoolean(*Fn)(EGLDisplay,EGLSurface);Fn fn=(Fn)lookup("eglDestroySurface");return fn?fn(d,s):EGL_FALSE;}
EXPORT void glXDestroyWindow(Display* d,GLXWindow w) {typedef void(*Fn)(Display*,GLXWindow);Fn fn=(Fn)lookup("glXDestroyWindow");if(fn)fn(d,w);}
EXPORT void glXDestroyPbuffer(Display* d,GLXPbuffer p) {typedef void(*Fn)(Display*,GLXPbuffer);Fn fn=(Fn)lookup("glXDestroyPbuffer");if(fn)fn(d,p);}
#ifdef ROSORAH_TESTING
EXPORT int rosorah_test_gl(unsigned w,unsigned h,unsigned input,unsigned char* pixel) {
    typedef int(*Fn)(unsigned,unsigned,unsigned,unsigned char*);
    Fn fn=(Fn)lookup("rosorah_test_gl");return fn?fn(w,h,input,pixel):-99;
}
#endif
