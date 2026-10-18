#define GL_GLEXT_PROTOTYPES
#include <GL/gl.h>
#include <GL/glext.h>
#include <EGL/egl.h>
#include <EGL/eglext.h>
#include <dlfcn.h>
#include <cstdio>
#include <cassert>
#include <cstring>
int main(int argc,char** argv) {
    setvbuf(stdout,nullptr,_IONBF,0);

#ifdef ROSORAH_LOCAL_LOAD
    void* egl_library=dlopen("libEGL.so.1",RTLD_NOW|RTLD_LOCAL);
    void* gl_library=dlopen("libGL.so.1",RTLD_NOW|RTLD_LOCAL);
    assert(egl_library && gl_library);
    auto eglBindAPI=reinterpret_cast<decltype(&::eglBindAPI)>(dlsym(egl_library,"eglBindAPI"));assert(eglBindAPI);
    auto eglChooseConfig=reinterpret_cast<decltype(&::eglChooseConfig)>(dlsym(egl_library,"eglChooseConfig"));assert(eglChooseConfig);
    auto eglCreateContext=reinterpret_cast<decltype(&::eglCreateContext)>(dlsym(egl_library,"eglCreateContext"));assert(eglCreateContext);
    auto eglCreatePbufferSurface=reinterpret_cast<decltype(&::eglCreatePbufferSurface)>(dlsym(egl_library,"eglCreatePbufferSurface"));assert(eglCreatePbufferSurface);
    auto eglDestroyContext=reinterpret_cast<decltype(&::eglDestroyContext)>(dlsym(egl_library,"eglDestroyContext"));assert(eglDestroyContext);
    auto eglDestroySurface=reinterpret_cast<decltype(&::eglDestroySurface)>(dlsym(egl_library,"eglDestroySurface"));assert(eglDestroySurface);
    auto eglGetError=reinterpret_cast<decltype(&::eglGetError)>(dlsym(egl_library,"eglGetError"));assert(eglGetError);
    auto eglGetProcAddress=reinterpret_cast<decltype(&::eglGetProcAddress)>(dlsym(egl_library,"eglGetProcAddress"));assert(eglGetProcAddress);
    auto eglInitialize=reinterpret_cast<decltype(&::eglInitialize)>(dlsym(egl_library,"eglInitialize"));assert(eglInitialize);
    auto eglMakeCurrent=reinterpret_cast<decltype(&::eglMakeCurrent)>(dlsym(egl_library,"eglMakeCurrent"));assert(eglMakeCurrent);
    auto eglSwapBuffers=reinterpret_cast<decltype(&::eglSwapBuffers)>(dlsym(egl_library,"eglSwapBuffers"));assert(eglSwapBuffers);
    auto eglTerminate=reinterpret_cast<decltype(&::eglTerminate)>(dlsym(egl_library,"eglTerminate"));assert(eglTerminate);
    auto glBindBuffer=reinterpret_cast<decltype(&::glBindBuffer)>(dlsym(gl_library,"glBindBuffer"));assert(glBindBuffer);
    auto glBindFramebuffer=reinterpret_cast<decltype(&::glBindFramebuffer)>(dlsym(gl_library,"glBindFramebuffer"));assert(glBindFramebuffer);
    auto glBindRenderbuffer=reinterpret_cast<decltype(&::glBindRenderbuffer)>(dlsym(gl_library,"glBindRenderbuffer"));assert(glBindRenderbuffer);
    auto glBindTexture=reinterpret_cast<decltype(&::glBindTexture)>(dlsym(gl_library,"glBindTexture"));assert(glBindTexture);
    auto glBufferData=reinterpret_cast<decltype(&::glBufferData)>(dlsym(gl_library,"glBufferData"));assert(glBufferData);
    auto glCheckFramebufferStatus=reinterpret_cast<decltype(&::glCheckFramebufferStatus)>(dlsym(gl_library,"glCheckFramebufferStatus"));assert(glCheckFramebufferStatus);
    auto glClear=reinterpret_cast<decltype(&::glClear)>(dlsym(gl_library,"glClear"));assert(glClear);
    auto glClearColor=reinterpret_cast<decltype(&::glClearColor)>(dlsym(gl_library,"glClearColor"));assert(glClearColor);
    auto glDeleteBuffers=reinterpret_cast<decltype(&::glDeleteBuffers)>(dlsym(gl_library,"glDeleteBuffers"));assert(glDeleteBuffers);
    auto glDeleteFramebuffers=reinterpret_cast<decltype(&::glDeleteFramebuffers)>(dlsym(gl_library,"glDeleteFramebuffers"));assert(glDeleteFramebuffers);
    auto glDeleteRenderbuffers=reinterpret_cast<decltype(&::glDeleteRenderbuffers)>(dlsym(gl_library,"glDeleteRenderbuffers"));assert(glDeleteRenderbuffers);
    auto glDeleteTextures=reinterpret_cast<decltype(&::glDeleteTextures)>(dlsym(gl_library,"glDeleteTextures"));assert(glDeleteTextures);
    auto glEnable=reinterpret_cast<decltype(&::glEnable)>(dlsym(gl_library,"glEnable"));assert(glEnable);
    auto glFramebufferRenderbuffer=reinterpret_cast<decltype(&::glFramebufferRenderbuffer)>(dlsym(gl_library,"glFramebufferRenderbuffer"));assert(glFramebufferRenderbuffer);
    auto glGenBuffers=reinterpret_cast<decltype(&::glGenBuffers)>(dlsym(gl_library,"glGenBuffers"));assert(glGenBuffers);
    auto glGenFramebuffers=reinterpret_cast<decltype(&::glGenFramebuffers)>(dlsym(gl_library,"glGenFramebuffers"));assert(glGenFramebuffers);
    auto glGenRenderbuffers=reinterpret_cast<decltype(&::glGenRenderbuffers)>(dlsym(gl_library,"glGenRenderbuffers"));assert(glGenRenderbuffers);
    auto glGenTextures=reinterpret_cast<decltype(&::glGenTextures)>(dlsym(gl_library,"glGenTextures"));assert(glGenTextures);
    auto glGetError=reinterpret_cast<decltype(&::glGetError)>(dlsym(gl_library,"glGetError"));assert(glGetError);
    auto glGetIntegerv=reinterpret_cast<decltype(&::glGetIntegerv)>(dlsym(gl_library,"glGetIntegerv"));assert(glGetIntegerv);
    auto glGetString=reinterpret_cast<decltype(&::glGetString)>(dlsym(gl_library,"glGetString"));assert(glGetString);
    auto glIsEnabled=reinterpret_cast<decltype(&::glIsEnabled)>(dlsym(gl_library,"glIsEnabled"));assert(glIsEnabled);
    auto glPixelStorei=reinterpret_cast<decltype(&::glPixelStorei)>(dlsym(gl_library,"glPixelStorei"));assert(glPixelStorei);
    auto glRenderbufferStorageMultisample=reinterpret_cast<decltype(&::glRenderbufferStorageMultisample)>(dlsym(gl_library,"glRenderbufferStorageMultisample"));assert(glRenderbufferStorageMultisample);
    auto glScissor=reinterpret_cast<decltype(&::glScissor)>(dlsym(gl_library,"glScissor"));assert(glScissor);
#endif
    const bool es=argc>1 && strcmp(argv[1],"es")==0;
    const bool msaa=argc>1 && strcmp(argv[1],"msaa")==0;
    auto platform=(PFNEGLGETPLATFORMDISPLAYEXTPROC)eglGetProcAddress("eglGetPlatformDisplayEXT");assert(platform);
    EGLDisplay display=platform(EGL_PLATFORM_SURFACELESS_MESA,EGL_DEFAULT_DISPLAY,nullptr);assert(display!=EGL_NO_DISPLAY);
    EGLint major=0,minor=0;if(!eglInitialize(display,&major,&minor)){fprintf(stderr,"BLOCKED: EGL initialize 0x%x\n",eglGetError());return 77;}
    assert(eglBindAPI(es?EGL_OPENGL_ES_API:EGL_OPENGL_API));
    EGLint config_attr[]={EGL_SURFACE_TYPE,EGL_PBUFFER_BIT,EGL_RENDERABLE_TYPE,es?EGL_OPENGL_ES3_BIT:EGL_OPENGL_BIT,EGL_RED_SIZE,8,EGL_GREEN_SIZE,8,EGL_BLUE_SIZE,8,EGL_ALPHA_SIZE,8,EGL_SAMPLES,0,EGL_NONE};
    EGLConfig config{};EGLint count=0;assert(eglChooseConfig(display,config_attr,&config,1,&count) && count);
    EGLint context_attr[]={EGL_CONTEXT_MAJOR_VERSION,3,EGL_CONTEXT_MINOR_VERSION,es?0:3,EGL_NONE};
    EGLContext context=eglCreateContext(display,config,EGL_NO_CONTEXT,context_attr);assert(context!=EGL_NO_CONTEXT);
    EGLint surface_attr[]={EGL_WIDTH,3840,EGL_HEIGHT,2160,EGL_NONE};
    EGLSurface surface=eglCreatePbufferSurface(display,config,surface_attr);assert(surface!=EGL_NO_SURFACE);
    assert(eglMakeCurrent(display,surface,surface,context));
    GLint samples=0;glGetIntegerv(GL_SAMPLES,&samples);printf("GL: %s; samples %d\n",glGetString(GL_VERSION),samples);glClearColor(1,.25,.5,1);glClear(GL_COLOR_BUFFER_BIT);
    GLint draw=0,read=0,db=0;if(!es)glGetIntegerv(GL_DRAW_BUFFER,&draw);glGetIntegerv(GL_READ_BUFFER,&read);if(!es)glGetIntegerv(GL_DOUBLEBUFFER,&db);printf("draw %x read %x double %d\n",draw,read,db);

    if(argc>1 && strcmp(argv[1],"passthrough")==0) {
        assert(eglSwapBuffers(display,surface));
        assert(eglMakeCurrent(display,EGL_NO_SURFACE,EGL_NO_SURFACE,EGL_NO_CONTEXT));
        assert(eglDestroyContext(display,context));assert(eglDestroySurface(display,surface));assert(eglTerminate(display));
        puts("PASS: missing capture backend leaves normal EGL rendering/teardown working");return 0;
    }
    // Test multisample resolve with a real MSAA renderbuffer. The surfaceless
    // software driver's multisample default pbuffer resolves to zero even in an
    // independent GL-only fixture, so it cannot validate window presentation.
    GLuint input=0,renderbuffer=0;
    if(msaa){
        glGenFramebuffers(1,&input);glBindFramebuffer(GL_FRAMEBUFFER,input);
        glGenRenderbuffers(1,&renderbuffer);glBindRenderbuffer(GL_RENDERBUFFER,renderbuffer);
        glRenderbufferStorageMultisample(GL_RENDERBUFFER,4,GL_RGBA8,3840,2160);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER,GL_COLOR_ATTACHMENT0,GL_RENDERBUFFER,renderbuffer);
        assert(glCheckFramebufferStatus(GL_FRAMEBUFFER)==GL_FRAMEBUFFER_COMPLETE);
        glClear(GL_COLOR_BUFFER_BIT);glBindFramebuffer(GL_FRAMEBUFFER,0);
    }
    // Deliberately hostile state: capture must temporarily reset and then restore it.
    GLuint pack=0,unpack=0,texture=0;glGenBuffers(1,&pack);glBindBuffer(GL_PIXEL_PACK_BUFFER,pack);glBufferData(GL_PIXEL_PACK_BUFFER,64,nullptr,GL_STREAM_READ);
    glGenBuffers(1,&unpack);glBindBuffer(GL_PIXEL_UNPACK_BUFFER,unpack);glBufferData(GL_PIXEL_UNPACK_BUFFER,64,nullptr,GL_STREAM_DRAW);
    glGenTextures(1,&texture);glBindTexture(GL_TEXTURE_2D,texture);
    glPixelStorei(GL_PACK_ROW_LENGTH,17);glPixelStorei(GL_PACK_SKIP_ROWS,2);glPixelStorei(GL_PACK_SKIP_PIXELS,3);glPixelStorei(GL_PACK_ALIGNMENT,8);
    if(!es){glEnable(GL_FRAMEBUFFER_SRGB);glPixelStorei(GL_PACK_SWAP_BYTES,GL_TRUE);}
    glEnable(GL_SCISSOR_TEST);glScissor(2,2,1,1);
    using Test=int(*)(unsigned,unsigned,unsigned,unsigned char*);
    auto test=(Test)dlsym(RTLD_DEFAULT,"rosorah_test_gl");assert(test);unsigned char pixel[4]{};
    int result=test(3840,2160,input,pixel);printf("Capture result %d: %u %u %u %u\n",result,pixel[0],pixel[1],pixel[2],pixel[3]);assert(!result);
    if(pixel[0]!=255 || pixel[1]!=64 || pixel[2]!=128 || pixel[3]!=255){fprintf(stderr,"Capture GL error: 0x%x\n",glGetError());return 1;}
    GLint value=0;
#define EXPECT_STATE(name,expected) glGetIntegerv(name,&value);assert(value==GLint(expected))
    EXPECT_STATE(GL_PIXEL_PACK_BUFFER_BINDING,pack);EXPECT_STATE(GL_PIXEL_UNPACK_BUFFER_BINDING,unpack);EXPECT_STATE(GL_TEXTURE_BINDING_2D,texture);
    EXPECT_STATE(GL_PACK_ROW_LENGTH,17);EXPECT_STATE(GL_PACK_SKIP_ROWS,2);EXPECT_STATE(GL_PACK_SKIP_PIXELS,3);EXPECT_STATE(GL_PACK_ALIGNMENT,8);
    EXPECT_STATE(GL_READ_FRAMEBUFFER_BINDING,0);EXPECT_STATE(GL_DRAW_FRAMEBUFFER_BINDING,0);
    assert(glIsEnabled(GL_SCISSOR_TEST));if(!es){assert(glIsEnabled(GL_FRAMEBUFFER_SRGB));EXPECT_STATE(GL_PACK_SWAP_BYTES,GL_TRUE);}
    GLenum error=glGetError();if(error)fprintf(stderr,"GL error 0x%x\n",error);assert(error==GL_NO_ERROR);
    // Both lookup routes should return the hook while keeping ordinary GL calls intact.
    assert((void*)eglGetProcAddress("eglSwapBuffers")==dlsym(RTLD_DEFAULT,"eglSwapBuffers"));
    assert(eglSwapBuffers(display,surface));
    glDeleteBuffers(1,&pack);glDeleteBuffers(1,&unpack);glDeleteTextures(1,&texture);
    if(input)glDeleteFramebuffers(1,&input);
    if(renderbuffer)glDeleteRenderbuffers(1,&renderbuffer);
    assert(eglDestroyContext(display,context));assert(eglMakeCurrent(display,EGL_NO_SURFACE,EGL_NO_SURFACE,EGL_NO_CONTEXT));assert(eglDestroySurface(display,surface));assert(eglTerminate(display));
    puts("PASS: surfaceless EGL, GPU downscale, asynchronous PBO readback, GL state restoration, lookup interception and teardown");
}
