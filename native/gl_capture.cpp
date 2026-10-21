// Independent GLX/EGL capture. No OBS code or dependency.
#include <GL/gl.h>
#include <GL/glext.h>
#include <GL/glx.h>
#include <EGL/egl.h>
#include <EGL/eglext.h>
#include <dlfcn.h>
#include "transport.hpp"
#include "color.hpp"
#include <array>
#include <atomic>
#include <thread>
#include <mutex>
#include <memory>
#include <unordered_map>
#include <vector>
#define EXPORT __attribute__((visibility("default")))
namespace {
using Symbol=void*(*)(void*,const char*) noexcept;
Symbol symbol() noexcept {
#if defined(__x86_64__)
    static auto fn=reinterpret_cast<Symbol>(dlvsym(RTLD_NEXT,"dlsym","GLIBC_2.2.5"));
#else
    static auto fn=reinterpret_cast<Symbol>(dlvsym(RTLD_NEXT,"dlsym","GLIBC_2.0"));
#endif
    return fn;
}
void* next(const char* name) noexcept {
    auto fn=symbol();if(!fn)return nullptr;
    if(auto p=fn(RTLD_NEXT,name))return p;
    // SDL and other engines can dlopen graphics libraries with RTLD_LOCAL.
    // RTLD_NOLOAD only queries libraries that the application already loaded.
    for(const char* library:{"libGL.so.1","libEGL.so.1","libGLX.so.0","libGLESv2.so.2"}) {
        void* handle=dlopen(library,RTLD_LAZY|RTLD_NOLOAD);if(!handle)continue;
        void* p=fn(handle,name);dlclose(handle);if(p)return p;
    }
    return nullptr;
}
void* gl_proc(const char* name) {
    using Proc=void*(*)(const char*);
    // OpenGL dispatch pointers remain context-specific through GLVND.
    auto egl=reinterpret_cast<Proc>(next("eglGetProcAddress"));
    auto glx=reinterpret_cast<Proc>(next("glXGetProcAddressARB"));
    if(void* p=next(name))return p;
    if(egl)if(void* p=egl(name))return p;
    return glx?glx(name):nullptr;
}
#define GL_FUNCTIONS(F) \
 F(const GLubyte*,GetString,(GLenum)) F(void,GetIntegerv,(GLenum,GLint*)) F(GLboolean,IsEnabled,(GLenum)) \
 F(void,Enable,(GLenum)) F(void,Disable,(GLenum)) F(void,PixelStorei,(GLenum,GLint)) \
 F(void,GenTextures,(GLsizei,GLuint*)) F(void,DeleteTextures,(GLsizei,const GLuint*)) F(void,BindTexture,(GLenum,GLuint)) \
 F(void,TexImage2D,(GLenum,GLint,GLint,GLsizei,GLsizei,GLint,GLenum,GLenum,const void*)) F(void,TexParameteri,(GLenum,GLenum,GLint)) \
 F(void,GenFramebuffers,(GLsizei,GLuint*)) F(void,DeleteFramebuffers,(GLsizei,const GLuint*)) F(void,BindFramebuffer,(GLenum,GLuint)) \
 F(void,FramebufferTexture2D,(GLenum,GLenum,GLenum,GLuint,GLint)) F(GLenum,CheckFramebufferStatus,(GLenum)) \
 F(void,GetFramebufferAttachmentParameteriv,(GLenum,GLenum,GLenum,GLint*)) \
 F(void,BlitFramebuffer,(GLint,GLint,GLint,GLint,GLint,GLint,GLint,GLint,GLbitfield,GLenum)) F(void,ReadBuffer,(GLenum)) \
 F(void,GenBuffers,(GLsizei,GLuint*)) F(void,DeleteBuffers,(GLsizei,const GLuint*)) F(void,BindBuffer,(GLenum,GLuint)) \
 F(void,BufferData,(GLenum,GLsizeiptr,const void*,GLenum)) F(void*,MapBufferRange,(GLenum,GLintptr,GLsizeiptr,GLbitfield)) F(GLboolean,UnmapBuffer,(GLenum)) \
 F(void,ReadPixels,(GLint,GLint,GLsizei,GLsizei,GLenum,GLenum,void*)) F(GLsync,FenceSync,(GLenum,GLbitfield)) \
 F(GLenum,ClientWaitSync,(GLsync,GLbitfield,GLuint64)) F(void,DeleteSync,(GLsync)) F(void,Flush,())
struct Gl {
#define DECL(ret,name,args) using Fn##name=ret(APIENTRY*)args;Fn##name name{};
    GL_FUNCTIONS(DECL)
#undef DECL
    bool es=false,srgb_control=true;
    bool load() {
#define LOAD(ret,name,args) name=reinterpret_cast<Fn##name>(gl_proc("gl" #name));if(!name)return false;
        GL_FUNCTIONS(LOAD)
#undef LOAD
        const char* version=reinterpret_cast<const char*>(GetString(GL_VERSION));if(!version)return false;
        es=strstr(version,"OpenGL ES")!=nullptr;
        const char* number=version;while(*number && (*number<'0' || *number>'9'))number++;
        const int major=*number?number[0]-'0':0,minor=number[0] && number[1]=='.' && number[2]?number[2]-'0':0;
        // Sync objects and framebuffer blits: desktop 3.2+, or ES 3.0+.
        if(es?(major<3):(major<3 || (major==3 && minor<2)))return false;
        srgb_control=!es;return true;
    }
};
struct CpuSlot {
    std::vector<uint8_t> pixels;uint64_t timestamp=0;
    std::atomic<unsigned> state{0}; // 0 free, 1 ready, 2 exclusively owned
};
class Delivery {
    std::array<CpuSlot,2> slots;
    rosorah::Mailbox mailbox;
    std::atomic<bool> stop{false};
    std::thread worker;
public:
    std::atomic<uint32_t> fps{0},requested_quality{1};
    Delivery(unsigned w,unsigned h,rosorah::Color color,uint32_t quality):mailbox() {
        mailbox.resolution(quality);requested_quality.store(quality);
        mailbox.configure(w,h,false,2u|(color.hdr()?4u:0u));
        worker=std::thread([this,w,h,color] {
            std::vector<uint8_t> flipped,rgba;
            rosorah::ColorConverter converter;uint64_t last=0;
            while(!stop.load(std::memory_order_acquire)) {
                uint32_t rate=0;try{rate=mailbox.service();}catch(...){}
                if(rate && flipped.empty()) {
                    const size_t bytes=size_t(w)*h*color.bytes();
                    try{flipped.resize(bytes);for(auto& slot:slots)slot.pixels.resize(bytes);}
                    catch(...){fps.store(0,std::memory_order_release);return;}
                }
                requested_quality.store(mailbox.resolution(),std::memory_order_release);
                fps.store(rate,std::memory_order_release);
                CpuSlot* latest=nullptr;
                for(auto& slot:slots) {
                    unsigned expected=1;
                    if(!slot.state.compare_exchange_strong(expected,2,std::memory_order_acquire))continue;
                    if(slot.timestamp<=last || (latest && slot.timestamp<latest->timestamp)){slot.state.store(0,std::memory_order_release);continue;}
                    if(latest)latest->state.store(0,std::memory_order_release);
                    latest=&slot;
                }
                if(latest) {
                    last=latest->timestamp;
                    if(rate)try{
                        const size_t stride=size_t(w)*color.bytes();
                        for(unsigned y=0;y<h;y++)memcpy(flipped.data()+size_t(y)*stride,latest->pixels.data()+size_t(h-1-y)*stride,stride);
                        const void* pixels=flipped.data();
                        if(color.convert()){converter.convert(pixels,size_t(w)*h,color,rgba);pixels=rgba.data();}
                        mailbox.publish(pixels,last);
                    }catch(...){}
                    latest->state.store(0,std::memory_order_release);
                }
                std::this_thread::sleep_for(std::chrono::milliseconds(rate?1:20));
            }
        });
    }
    ~Delivery(){stop.store(true,std::memory_order_release);if(worker.joinable())worker.join();}
    void submit(const void* pixels,uint64_t stamp) {
        for(auto& slot:slots) {
            unsigned expected=0;
            if(!slot.state.compare_exchange_strong(expected,2,std::memory_order_acquire))continue;
            memcpy(slot.pixels.data(),pixels,slot.pixels.size());slot.timestamp=stamp;
            slot.state.store(1,std::memory_order_release);return;
        }
    }
};
struct Saved {
    Gl& gl;GLint read=0,draw=0,pack=0,unpack=0,texture=0,alignment=0,row=0,skip_rows=0,skip_pixels=0,swap_bytes=0;
    GLuint source=0;GLint default_read=0;GLboolean scissor=0,srgb=0;
    explicit Saved(Gl& g,GLuint input=0):gl(g),source(input) {
        gl.GetIntegerv(GL_READ_FRAMEBUFFER_BINDING,&read);gl.GetIntegerv(GL_DRAW_FRAMEBUFFER_BINDING,&draw);
        gl.GetIntegerv(GL_PIXEL_UNPACK_BUFFER_BINDING,&unpack);
        if(!gl.es)gl.GetIntegerv(GL_PACK_SWAP_BYTES,&swap_bytes);
        gl.GetIntegerv(GL_PIXEL_PACK_BUFFER_BINDING,&pack);gl.GetIntegerv(GL_TEXTURE_BINDING_2D,&texture);
        gl.GetIntegerv(GL_PACK_ALIGNMENT,&alignment);gl.GetIntegerv(GL_PACK_ROW_LENGTH,&row);
        gl.GetIntegerv(GL_PACK_SKIP_ROWS,&skip_rows);gl.GetIntegerv(GL_PACK_SKIP_PIXELS,&skip_pixels);
        scissor=gl.IsEnabled(GL_SCISSOR_TEST);if(gl.srgb_control)srgb=gl.IsEnabled(GL_FRAMEBUFFER_SRGB);
        gl.BindFramebuffer(GL_READ_FRAMEBUFFER,source);gl.GetIntegerv(GL_READ_BUFFER,&default_read);
    }
    ~Saved() {
        gl.BindFramebuffer(GL_READ_FRAMEBUFFER,source);gl.ReadBuffer(GLenum(default_read));
        gl.BindFramebuffer(GL_READ_FRAMEBUFFER,GLuint(read));gl.BindFramebuffer(GL_DRAW_FRAMEBUFFER,GLuint(draw));
        gl.BindBuffer(GL_PIXEL_UNPACK_BUFFER,GLuint(unpack));
        if(!gl.es)gl.PixelStorei(GL_PACK_SWAP_BYTES,swap_bytes);
        gl.BindBuffer(GL_PIXEL_PACK_BUFFER,GLuint(pack));gl.BindTexture(GL_TEXTURE_2D,GLuint(texture));
        gl.PixelStorei(GL_PACK_ALIGNMENT,alignment);gl.PixelStorei(GL_PACK_ROW_LENGTH,row);
        gl.PixelStorei(GL_PACK_SKIP_ROWS,skip_rows);gl.PixelStorei(GL_PACK_SKIP_PIXELS,skip_pixels);
        if(scissor)gl.Enable(GL_SCISSOR_TEST);else gl.Disable(GL_SCISSOR_TEST);
        if(gl.srgb_control){if(srgb)gl.Enable(GL_FRAMEBUFFER_SRGB);else gl.Disable(GL_FRAMEBUFFER_SRGB);}
    }
};
struct Slot {GLuint pbo=0;GLsync fence=nullptr;uint64_t timestamp=0;};
struct Capture {
    Gl gl;bool valid=false,surface_valid=false;unsigned width=0,height=0,out_w=0,out_h=0;
    GLuint texture=0,fbo=0,resolve_texture=0,resolve_fbo=0;
#ifdef ROSORAH_TESTING
    GLuint input_framebuffer=0; // Offscreen MSAA fixture; real swap capture always reads framebuffer 0.
    GLuint input() const {return input_framebuffer;}
#else
    GLuint input() const {return 0;}
#endif
    GLint samples=0,encoding=GL_LINEAR;
    rosorah::Color color;
    std::array<Slot,2> slots;
    std::unique_ptr<Delivery> delivery;
    uint64_t due=0,dimension_check=0;uint32_t quality=1;
    Capture():valid(gl.load()){}
    // Must be called while the owning GL context is current. Deleting in-flight
    // GL objects is deferred by the driver; no glFinish/client fence wait.
    void release_gpu() {
        for(auto& slot:slots){if(slot.fence)gl.DeleteSync(slot.fence);if(slot.pbo)gl.DeleteBuffers(1,&slot.pbo);slot=Slot{};}
        for(auto object:{fbo,resolve_fbo})if(object)gl.DeleteFramebuffers(1,&object);
        for(auto object:{texture,resolve_texture})if(object)gl.DeleteTextures(1,&object);
        texture=fbo=resolve_texture=resolve_fbo=0;
    }
    GLenum data_type() const {return color.pixel==rosorah::Pixel::Rgba16f?GL_HALF_FLOAT:(color.pixel==rosorah::Pixel::Rgb10?GL_UNSIGNED_INT_2_10_10_10_REV:GL_UNSIGNED_BYTE);}
    GLint internal_format() const {return color.pixel==rosorah::Pixel::Rgba16f?GL_RGBA16F:(color.pixel==rosorah::Pixel::Rgb10?GL_RGB10_A2:(encoding==GL_SRGB?GL_SRGB8_ALPHA8:GL_RGBA8));}
    bool make_target(GLuint& tex,GLuint& frame,unsigned w,unsigned h) {
        gl.GenTextures(1,&tex);gl.BindTexture(GL_TEXTURE_2D,tex);
        gl.TexParameteri(GL_TEXTURE_2D,GL_TEXTURE_MIN_FILTER,GL_LINEAR);gl.TexParameteri(GL_TEXTURE_2D,GL_TEXTURE_MAG_FILTER,GL_LINEAR);
        gl.BindBuffer(GL_PIXEL_UNPACK_BUFFER,0);
        gl.TexImage2D(GL_TEXTURE_2D,0,internal_format(),GLsizei(w),GLsizei(h),0,GL_RGBA,data_type(),nullptr);
        gl.GenFramebuffers(1,&frame);gl.BindFramebuffer(GL_DRAW_FRAMEBUFFER,frame);
        gl.FramebufferTexture2D(GL_DRAW_FRAMEBUFFER,GL_COLOR_ATTACHMENT0,GL_TEXTURE_2D,tex,0);
        return gl.CheckFramebufferStatus(GL_DRAW_FRAMEBUFFER)==GL_FRAMEBUFFER_COMPLETE;
    }
    void configure(unsigned w,unsigned h,rosorah::Color input,bool enable_delivery=true) {
        release_gpu();delivery.reset();width=w;height=h;color=input;
        rosorah::capture_size(w,h,quality,out_w,out_h);
        if(enable_delivery)delivery=std::make_unique<Delivery>(out_w,out_h,color,quality);
    }
    GLenum source_buffer() {
        if(input())return GL_COLOR_ATTACHMENT0;
        if(gl.es)return GL_BACK;
        gl.BindFramebuffer(GL_DRAW_FRAMEBUFFER,0);
        GLint draw=GL_BACK;gl.GetIntegerv(GL_DRAW_BUFFER0,&draw);
        // EGL pbuffers can report !DOUBLEBUFFER yet draw into BACK. Respect the
        // actual default framebuffer draw target, including stereo left/right.
        if(draw>=GL_FRONT_LEFT && draw<=GL_BACK)return GLenum(draw);
        GLint double_buffer=0;gl.GetIntegerv(GL_DOUBLEBUFFER,&double_buffer);
        return double_buffer?GL_BACK:GL_FRONT;
    }
    bool prepare() {
        Saved saved(gl,input());
        gl.BindFramebuffer(GL_DRAW_FRAMEBUFFER,input());gl.GetIntegerv(GL_SAMPLES,&samples);
        GLenum source=source_buffer();gl.ReadBuffer(source);
        GLenum attachment=input()?GL_COLOR_ATTACHMENT0:(gl.es?GL_BACK:(source==GL_BACK?GL_BACK_LEFT:(source==GL_FRONT?GL_FRONT_LEFT:source)));
        gl.GetFramebufferAttachmentParameteriv(GL_READ_FRAMEBUFFER,attachment,GL_FRAMEBUFFER_ATTACHMENT_COLOR_ENCODING,&encoding);
        if(!make_target(texture,fbo,out_w,out_h))return false;
        if(samples>1 && !make_target(resolve_texture,resolve_fbo,width,height))return false;
        for(auto& slot:slots){gl.GenBuffers(1,&slot.pbo);gl.BindBuffer(GL_PIXEL_PACK_BUFFER,slot.pbo);gl.BufferData(GL_PIXEL_PACK_BUFFER,GLsizeiptr(size_t(out_w)*out_h*color.bytes()),nullptr,GL_STREAM_READ);}
        return true;
    }
    bool drain(std::vector<uint8_t>* test_pixels=nullptr) {
        Slot* latest=nullptr;
        for(auto& slot:slots)if(slot.fence) {
            GLenum result=gl.ClientWaitSync(slot.fence,0,0);
            if(result!=GL_ALREADY_SIGNALED && result!=GL_CONDITION_SATISFIED)continue;
            gl.DeleteSync(slot.fence);slot.fence=nullptr;
            if(!latest || slot.timestamp>latest->timestamp)latest=&slot;
        }
        if(!latest)return false;
        GLint previous=0;gl.GetIntegerv(GL_PIXEL_PACK_BUFFER_BINDING,&previous);gl.BindBuffer(GL_PIXEL_PACK_BUFFER,latest->pbo);
        const size_t bytes=size_t(out_w)*out_h*color.bytes();
        void* mapped=gl.MapBufferRange(GL_PIXEL_PACK_BUFFER,0,GLsizeiptr(bytes),GL_MAP_READ_BIT);
        if(mapped){if(test_pixels)test_pixels->assign(static_cast<uint8_t*>(mapped),static_cast<uint8_t*>(mapped)+bytes);else if(delivery)delivery->submit(mapped,latest->timestamp);gl.UnmapBuffer(GL_PIXEL_PACK_BUFFER);}
        gl.BindBuffer(GL_PIXEL_PACK_BUFFER,GLuint(previous));return mapped!=nullptr;
    }
    bool enqueue() {
        Slot* free=nullptr;for(auto& slot:slots)if(!slot.fence){free=&slot;break;}if(!free)return false;
        Saved saved(gl,input());
        gl.Disable(GL_SCISSOR_TEST);
        if(gl.srgb_control){if(encoding==GL_SRGB)gl.Enable(GL_FRAMEBUFFER_SRGB);else gl.Disable(GL_FRAMEBUFFER_SRGB);}
        gl.ReadBuffer(source_buffer());
        if(resolve_fbo){gl.BindFramebuffer(GL_DRAW_FRAMEBUFFER,resolve_fbo);gl.BlitFramebuffer(0,0,GLint(width),GLint(height),0,0,GLint(width),GLint(height),GL_COLOR_BUFFER_BIT,GL_NEAREST);gl.BindFramebuffer(GL_READ_FRAMEBUFFER,resolve_fbo);gl.ReadBuffer(GL_COLOR_ATTACHMENT0);

        }
        gl.BindFramebuffer(GL_DRAW_FRAMEBUFFER,fbo);
        gl.BlitFramebuffer(0,0,GLint(width),GLint(height),0,0,GLint(out_w),GLint(out_h),GL_COLOR_BUFFER_BIT,samples>1 && !resolve_fbo?GL_NEAREST:GL_LINEAR);
        gl.BindFramebuffer(GL_READ_FRAMEBUFFER,fbo);gl.ReadBuffer(GL_COLOR_ATTACHMENT0);
        gl.BindBuffer(GL_PIXEL_PACK_BUFFER,free->pbo);gl.PixelStorei(GL_PACK_ALIGNMENT,1);gl.PixelStorei(GL_PACK_ROW_LENGTH,0);gl.PixelStorei(GL_PACK_SKIP_ROWS,0);gl.PixelStorei(GL_PACK_SKIP_PIXELS,0);
        if(!gl.es)gl.PixelStorei(GL_PACK_SWAP_BYTES,GL_FALSE);
        gl.ReadPixels(0,0,GLsizei(out_w),GLsizei(out_h),GL_RGBA,data_type(),nullptr);
        free->timestamp=rosorah::now_ns();free->fence=gl.FenceSync(GL_SYNC_GPU_COMMANDS_COMPLETE,0);
        // Flush commands, never finish/wait. Swap will then present normally.
        gl.Flush();return free->fence!=nullptr;
    }
};
struct Key {uintptr_t context=0,surface=0;bool egl=false;bool operator==(const Key& b)const{return context==b.context && surface==b.surface && egl==b.egl;}};
struct Hash {size_t operator()(const Key& k)const{return std::hash<uintptr_t>{}(k.context)^(std::hash<uintptr_t>{}(k.surface)<<1)^size_t(k.egl);}};
std::recursive_mutex& gate(){static auto* lock=new std::recursive_mutex;return *lock;}
// Intentionally retained until process exit if a program omits API teardown.
// Explicit context/surface destroy interceptors remove normal live entries.
auto& captures(){static auto* map=new std::unordered_map<Key,std::unique_ptr<Capture>,Hash>;return *map;}
thread_local bool inside=false;
struct Enter {bool active;Enter():active(!inside){if(active)inside=true;}~Enter(){if(active)inside=false;}};
rosorah::Color egl_color(EGLDisplay display,EGLSurface surface,bool& supported) {
    rosorah::Color color;
    auto query=reinterpret_cast<decltype(&eglQueryString)>(next("eglQueryString"));
    auto surface_query=reinterpret_cast<decltype(&eglQuerySurface)>(next("eglQuerySurface"));
    const char* extensions=query?query(display,EGL_EXTENSIONS):nullptr;
    if(!extensions || !strstr(extensions,"EGL_KHR_gl_colorspace") || !surface_query)return color;
    EGLint space=EGL_GL_COLORSPACE_LINEAR_KHR;
    if(!surface_query(display,surface,EGL_GL_COLORSPACE_KHR,&space))return color;
    if(space==EGL_GL_COLORSPACE_LINEAR_KHR || space==EGL_GL_COLORSPACE_SRGB_KHR)return color;
    if(space==EGL_GL_COLORSPACE_SCRGB_LINEAR_EXT)color={rosorah::Pixel::Rgba16f,rosorah::Transfer::Linear709};
    else if(space==EGL_GL_COLORSPACE_BT2020_PQ_EXT)color={rosorah::Pixel::Rgb10,rosorah::Transfer::Pq2020};
#ifdef EGL_GL_COLORSPACE_BT2020_HLG_EXT
    else if(space==EGL_GL_COLORSPACE_BT2020_HLG_EXT)color={rosorah::Pixel::Rgb10,rosorah::Transfer::Hlg2020};
#endif
    else supported=false;
    return color;
}
void capture(Key key,void* display) {
    static const bool excluded=rosorah::self_excluded();if(excluded || !key.context || !key.surface)return;
    Enter enter;if(!enter.active)return;
    std::lock_guard<std::recursive_mutex> lock(gate());
    auto& all=captures();auto found=all.find(key);
    if(found==all.end()){if(all.size()>=32)return;found=all.emplace(key,std::make_unique<Capture>()).first;}
    auto& c=*found->second;if(!c.valid)return;
    uint64_t now=rosorah::now_ns();
    if(!c.width || now>=c.dimension_check) {
        unsigned w=0,h=0;rosorah::Color color;bool color_supported=true;
        if(key.egl) {
            auto query=reinterpret_cast<decltype(&eglQuerySurface)>(next("eglQuerySurface"));EGLint ew=0,eh=0;
            if(!query || !query(display,reinterpret_cast<EGLSurface>(key.surface),EGL_WIDTH,&ew) || !query(display,reinterpret_cast<EGLSurface>(key.surface),EGL_HEIGHT,&eh))return;
            w=unsigned(ew);h=unsigned(eh);color=egl_color(display,reinterpret_cast<EGLSurface>(key.surface),color_supported);
        } else {
            auto query=reinterpret_cast<decltype(&glXQueryDrawable)>(next("glXQueryDrawable"));if(!query)return;
            query(static_cast<Display*>(display),GLXDrawable(key.surface),GLX_WIDTH,&w);query(static_cast<Display*>(display),GLXDrawable(key.surface),GLX_HEIGHT,&h);
        }
        c.dimension_check=now+250000000ull;
        c.surface_valid=color_supported && w>=160 && h>=90 && w<=4096 && h<=4096;
        if(!c.surface_valid)return;
        if(w!=c.width || h!=c.height || color.pixel!=c.color.pixel || color.transfer!=c.color.transfer) {
            if(c.gl.es && color.hdr())return; // GLES SDR path is supported; HDR readback needs desktop GL.
            c.configure(w,h,color);
        }
    }
    if(!c.surface_valid)return;
    if(c.delivery){uint32_t quality=c.delivery->requested_quality.load(std::memory_order_acquire);
        if(quality!=c.quality){c.quality=quality;c.configure(c.width,c.height,c.color);c.due=0;}}
    if(c.fbo)c.drain();
    uint32_t fps=c.delivery?c.delivery->fps.load(std::memory_order_acquire):0;
    // Twice the studio's rate, with early presents on time, as in the Vulkan layer.
    const uint64_t interval=fps?1000000000ull/(fps*2):0;
    if(!fps || !rosorah::capture_due(now,c.due,interval))return;
    if(!c.fbo && !c.prepare()){c.valid=false;c.delivery.reset();return;}
    if(c.enqueue())c.due=rosorah::following_capture(now,c.due,interval);
}
void remove_surface(uintptr_t surface,bool egl,uintptr_t current) {
    std::lock_guard<std::recursive_mutex> lock(gate());
    auto& all=captures();
    for(auto it=all.begin();it!=all.end();) {
        if(it->first.surface!=surface || it->first.egl!=egl){++it;continue;}
        if(current==it->first.context && it->second->valid)it->second->release_gpu();
        it=all.erase(it);
    }
}
void remove_context(uintptr_t context,bool egl,bool current) {
    std::lock_guard<std::recursive_mutex> lock(gate());
    auto& all=captures();
    for(auto it=all.begin();it!=all.end();) {
        if(it->first.context!=context || it->first.egl!=egl){++it;continue;}
        if(current && it->second->valid)it->second->release_gpu();
        // If non-current, the context's resources are left to driver teardown.
        it=all.erase(it);
    }
}
}
extern "C" {
EXPORT void glXSwapBuffers(Display* display,GLXDrawable drawable) {
    static auto real=reinterpret_cast<decltype(&glXSwapBuffers)>(next("glXSwapBuffers"));
    auto current=reinterpret_cast<decltype(&glXGetCurrentContext)>(next("glXGetCurrentContext"));
    try{if(current)capture({reinterpret_cast<uintptr_t>(current()),uintptr_t(drawable),false},display);}catch(...){}
    if(real)real(display,drawable);
}
EXPORT int64_t glXSwapBuffersMscOML(Display* display,GLXDrawable drawable,int64_t target,int64_t divisor,int64_t remainder) {
    using Fn=int64_t(*)(Display*,GLXDrawable,int64_t,int64_t,int64_t);
    static auto real=reinterpret_cast<Fn>(next("glXSwapBuffersMscOML"));
    auto current=reinterpret_cast<decltype(&glXGetCurrentContext)>(next("glXGetCurrentContext"));
    try{if(current)capture({reinterpret_cast<uintptr_t>(current()),uintptr_t(drawable),false},display);}catch(...){}
    return real?real(display,drawable,target,divisor,remainder):0;
}
void capture_egl(EGLDisplay display,EGLSurface surface) {
    auto current=reinterpret_cast<decltype(&eglGetCurrentContext)>(next("eglGetCurrentContext"));
    auto draw=reinterpret_cast<decltype(&eglGetCurrentSurface)>(next("eglGetCurrentSurface"));
    if(current && draw && draw(EGL_DRAW)==surface)capture({reinterpret_cast<uintptr_t>(current()),reinterpret_cast<uintptr_t>(surface),true},display);
}
EXPORT EGLBoolean eglSwapBuffers(EGLDisplay display,EGLSurface surface) {
    static auto real=reinterpret_cast<decltype(&eglSwapBuffers)>(next("eglSwapBuffers"));
    try{capture_egl(display,surface);}catch(...){}return real?real(display,surface):EGL_FALSE;
}
EXPORT EGLBoolean eglSwapBuffersWithDamageKHR(EGLDisplay display,EGLSurface surface,const EGLint* rects,EGLint count) {
    using Fn=EGLBoolean(*)(EGLDisplay,EGLSurface,const EGLint*,EGLint);
    static auto real=reinterpret_cast<Fn>(gl_proc("eglSwapBuffersWithDamageKHR"));
    try{capture_egl(display,surface);}catch(...){}return real?real(display,surface,rects,count):EGL_FALSE;
}
EXPORT EGLBoolean eglSwapBuffersWithDamageEXT(EGLDisplay display,EGLSurface surface,const EGLint* rects,EGLint count) {
    using Fn=EGLBoolean(*)(EGLDisplay,EGLSurface,const EGLint*,EGLint);
    static auto real=reinterpret_cast<Fn>(gl_proc("eglSwapBuffersWithDamageEXT"));
    try{capture_egl(display,surface);}catch(...){}return real?real(display,surface,rects,count):EGL_FALSE;
}
EXPORT void glXDestroyContext(Display* display,GLXContext context) {
    static auto real=reinterpret_cast<decltype(&glXDestroyContext)>(next("glXDestroyContext"));
    auto current=reinterpret_cast<decltype(&glXGetCurrentContext)>(next("glXGetCurrentContext"));
    try{remove_context(reinterpret_cast<uintptr_t>(context),false,current && current()==context);}catch(...){}
    if(real)real(display,context);
}
EXPORT EGLBoolean eglDestroyContext(EGLDisplay display,EGLContext context) {
    static auto real=reinterpret_cast<decltype(&eglDestroyContext)>(next("eglDestroyContext"));
    auto current=reinterpret_cast<decltype(&eglGetCurrentContext)>(next("eglGetCurrentContext"));
    try{remove_context(reinterpret_cast<uintptr_t>(context),true,current && current()==context);}catch(...){}
    return real?real(display,context):EGL_FALSE;
}
EXPORT EGLBoolean eglDestroySurface(EGLDisplay display,EGLSurface surface) {
    static auto real=reinterpret_cast<decltype(&eglDestroySurface)>(next("eglDestroySurface"));
    auto current=reinterpret_cast<decltype(&eglGetCurrentContext)>(next("eglGetCurrentContext"));
    try{remove_surface(reinterpret_cast<uintptr_t>(surface),true,current?reinterpret_cast<uintptr_t>(current()):0);}catch(...){}
    return real?real(display,surface):EGL_FALSE;
}
EXPORT void glXDestroyWindow(Display* display,GLXWindow window) {
    static auto real=reinterpret_cast<decltype(&glXDestroyWindow)>(next("glXDestroyWindow"));
    auto current=reinterpret_cast<decltype(&glXGetCurrentContext)>(next("glXGetCurrentContext"));
    try{remove_surface(uintptr_t(window),false,current?reinterpret_cast<uintptr_t>(current()):0);}catch(...){}
    if(real)real(display,window);
}
EXPORT void glXDestroyPbuffer(Display* display,GLXPbuffer buffer) {
    static auto real=reinterpret_cast<decltype(&glXDestroyPbuffer)>(next("glXDestroyPbuffer"));
    auto current=reinterpret_cast<decltype(&glXGetCurrentContext)>(next("glXGetCurrentContext"));
    try{remove_surface(uintptr_t(buffer),false,current?reinterpret_cast<uintptr_t>(current()):0);}catch(...){}
    if(real)real(display,buffer);
}
EXPORT __GLXextFuncPtr glXGetProcAddress(const GLubyte*);
EXPORT __GLXextFuncPtr glXGetProcAddressARB(const GLubyte*);
EXPORT __eglMustCastToProperFunctionPointerType eglGetProcAddress(const char*);
}
namespace {
void* intercept(const char* name,void* original) noexcept {
    if(!original || !name)return original;
#define HOOK(n) if(strcmp(name,#n)==0)return reinterpret_cast<void*>(&n);
    HOOK(glXSwapBuffers) HOOK(glXSwapBuffersMscOML) HOOK(eglSwapBuffers)
    HOOK(eglSwapBuffersWithDamageKHR) HOOK(eglSwapBuffersWithDamageEXT)
    HOOK(glXGetProcAddress) HOOK(glXGetProcAddressARB) HOOK(eglGetProcAddress)
    HOOK(glXDestroyContext) HOOK(eglDestroyContext) HOOK(eglDestroySurface) HOOK(glXDestroyWindow) HOOK(glXDestroyPbuffer)
#undef HOOK
    return original;
}
}
extern "C" {
EXPORT __GLXextFuncPtr glXGetProcAddress(const GLubyte* name) {
    static auto real=reinterpret_cast<decltype(&glXGetProcAddress)>(next("glXGetProcAddress"));
    return reinterpret_cast<__GLXextFuncPtr>(intercept(reinterpret_cast<const char*>(name),real?reinterpret_cast<void*>(real(name)):nullptr));
}
EXPORT __GLXextFuncPtr glXGetProcAddressARB(const GLubyte* name) {
    static auto real=reinterpret_cast<decltype(&glXGetProcAddressARB)>(next("glXGetProcAddressARB"));
    return reinterpret_cast<__GLXextFuncPtr>(intercept(reinterpret_cast<const char*>(name),real?reinterpret_cast<void*>(real(name)):nullptr));
}
EXPORT __eglMustCastToProperFunctionPointerType eglGetProcAddress(const char* name) {
    static auto real=reinterpret_cast<decltype(&eglGetProcAddress)>(next("eglGetProcAddress"));
    return reinterpret_cast<__eglMustCastToProperFunctionPointerType>(intercept(name,real?reinterpret_cast<void*>(real(name)):nullptr));
}
EXPORT void* dlsym(void* handle,const char* name) noexcept {
    auto real=symbol();if(!real)return nullptr;
    if(handle==RTLD_NEXT || intercept(name,reinterpret_cast<void*>(1))==reinterpret_cast<void*>(1))
        return real(handle,name); // Keep this a tail call (verified in both release architectures).
    return intercept(name,real(handle,name));
}
}
#ifdef ROSORAH_TESTING
extern "C" EXPORT int rosorah_test_gl(unsigned w,unsigned h,unsigned input,unsigned char* pixel) {
    Capture c;if(!c.valid)return -1;
    const unsigned widths[]={1280,1920,2560,3840,3840,1280};
    const unsigned heights[]={720,1080,1440,2160,2160,720};
    for(unsigned n=0;n<6;n++) {
        c.quality=n==5?0:n;
        c.input_framebuffer=input;c.configure(w,h,{},false);
        if(c.out_w!=widths[n] || c.out_h!=heights[n] || !c.prepare())return -1;
        if(!c.enqueue())return -2;
        std::vector<uint8_t> pixels;bool got=false;
        for(unsigned i=0;i<500 && !got;i++){got=c.drain(&pixels);if(!got)std::this_thread::sleep_for(std::chrono::milliseconds(1));}
        if(!got)return -3;
        if(pixels.size()!=size_t(widths[n])*heights[n]*4)return -4;
        memcpy(pixel,pixels.data(),4);
        c.release_gpu();
    }
    return 0;
}
#endif
