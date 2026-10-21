// Rosorah v3: a private home-directory rendezvous, with pixels in anonymous RAM.
// Steam's normal runtime shares HOME. No per-game environment or on-disk frames.
#pragma once
#include <cstdint>
#include <cstring>
#include <cstdlib>
#include <string>
#include <chrono>
#include <algorithm>
#include <cerrno>
#include <fcntl.h>
#include <unistd.h>
#include <sys/stat.h>
#include <sys/file.h>
#include <sys/mman.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <sys/syscall.h>
#include <linux/memfd.h>
namespace rosorah {
constexpr uint32_t MAGIC=0x52535246, VERSION=3;
constexpr size_t CAPACITY=112+4096*4096*4;
// Quality is a bounding box, never an upscale. Original is bounded by the
// receiver's existing 4096-per-axis safety limit. Pages are committed on write.
inline void capture_size(unsigned w,unsigned h,uint32_t quality,unsigned& ow,unsigned& oh) {
    const unsigned widths[]={1280,1920,2560,3840,4096};
    const unsigned heights[]={720,1080,1440,2160,4096};
    quality=std::min(quality,4u);
    const double factor=std::max({1.,double(w)/widths[quality],double(h)/heights[quality]});
    ow=std::max(1u,unsigned(w/factor));oh=std::max(1u,unsigned(h/factor));
}
inline uint64_t now_ns() {
    timespec t{}; clock_gettime(CLOCK_MONOTONIC, &t);
    return uint64_t(t.tv_sec)*1000000000ull+uint64_t(t.tv_nsec);
}
// Capture pacing on the game's presents. A present up to half an interval early counts as on
// time: a game presenting at the capture rate jitters around each deadline, and skipping the
// early ones turned 60 fps into a mix of 60 and 30.
inline bool capture_due(uint64_t now,uint64_t next,uint64_t interval){return now+interval/2>=next;}
// The deadline after a capture at `now`; the grid restarts only after falling a whole interval behind.
inline uint64_t following_capture(uint64_t now,uint64_t next,uint64_t interval){return now>=next+interval?now+interval:next+interval;}
struct Header {
    uint32_t magic=MAGIC, version=VERSION, width=0, height=0;
    uint32_t format=1, pid=0, flags=0, reserved=0;
    uint64_t sequence=0, timestamp=0;
    char name[64]{};
};
struct Message { uint32_t magic=MAGIC, version=VERSION, kind=1, value=0; };
static_assert(sizeof(Header)==112 && sizeof(Message)==16);
inline bool self_excluded() {
    char exe[4096]{}; ssize_t n=readlink("/proc/self/exe",exe,sizeof(exe)-1);
    if(n>0) { exe[n]=0; const char* name=strrchr(exe,'/'); name=name?name+1:exe;
        for(const char* skip:{"rosorah-studio","kwin_wayland","kwin_x11","plasmashell","Xwayland","gnome-shell"})if(strcmp(name,skip)==0)return true; }
    return getenv("ROSORAH_DISABLE_CAPTURE")!=nullptr;
}
inline bool steam_game() {
    for(const char* k : {"SteamAppId","SteamGameId","STEAM_COMPAT_APP_ID"}) {
        const char* v=getenv(k); if(!v || !*v)continue;
        bool digits=true,nonzero=false;for(const char* p=v;*p;p++){digits&=*p>='0' && *p<='9';nonzero|=*p>='1' && *p<='9';}if(digits && nonzero)return true;
    }
    return false;
}
inline std::string socket_path() {
    const char* home=getenv("HOME");
    return home && home[0]=='/' ? std::string(home)+"/.rosorah-capture/receiver.sock" : std::string{};
}
// Used only by the background delivery thread. The render thread reads atomic
// activation state. Socket operations and mailbox writes stay off the render thread.
class Mailbox {
    int socket=-1, fd=-1;
    void* mapped=MAP_FAILED;
    uint64_t retry=0;
    Header header;
    std::string address_path;
    uint32_t rate=0, quality=1;
    void disconnect() {
        rate=0;
        if(socket>=0)close(socket);
        if(mapped!=MAP_FAILED)munmap(mapped,CAPACITY);
        if(fd>=0)close(fd);
        socket=-1;fd=-1;mapped=MAP_FAILED;
    }
    void connect_receiver() {
        auto path=address_path;
        sockaddr_un address{};address.sun_family=AF_UNIX;
        if(path.empty() || path.size()>=sizeof(address.sun_path))return;
        struct stat st{};auto parent=path.substr(0,path.rfind('/'));
        if(lstat(parent.c_str(),&st)!=0 || !S_ISDIR(st.st_mode) || st.st_uid!=geteuid() || (st.st_mode&077)!=0)return;
        memcpy(address.sun_path,path.c_str(),path.size()+1);
        socket=::socket(AF_UNIX,SOCK_SEQPACKET|SOCK_NONBLOCK|SOCK_CLOEXEC,0);
        if(socket<0)return;
        if(::connect(socket,reinterpret_cast<sockaddr*>(&address),sizeof(address))!=0){disconnect();return;}
        ucred peer{};socklen_t length=sizeof(peer);
        if(getsockopt(socket,SOL_SOCKET,SO_PEERCRED,&peer,&length)!=0 || peer.uid!=geteuid()){disconnect();return;}
        fd=int(syscall(SYS_memfd_create,"rosorah-frame",MFD_CLOEXEC|MFD_ALLOW_SEALING));
        if(fd<0 || fchmod(fd,0600)!=0 || ftruncate(fd,CAPACITY)!=0 ||
           fcntl(fd,F_ADD_SEALS,F_SEAL_SHRINK|F_SEAL_GROW|F_SEAL_SEAL)!=0){disconnect();return;}
        mapped=mmap(nullptr,CAPACITY,PROT_READ|PROT_WRITE,MAP_SHARED,fd,0);
        if(mapped==MAP_FAILED){disconnect();return;}
        header.timestamp=now_ns();
        iovec iov{&header,sizeof(header)};
        alignas(cmsghdr) char control[CMSG_SPACE(sizeof(int))]{};
        msghdr msg{};msg.msg_iov=&iov;msg.msg_iovlen=1;msg.msg_control=control;msg.msg_controllen=sizeof(control);
        auto* c=CMSG_FIRSTHDR(&msg);c->cmsg_level=SOL_SOCKET;c->cmsg_type=SCM_RIGHTS;c->cmsg_len=CMSG_LEN(sizeof(int));
        memcpy(CMSG_DATA(c),&fd,sizeof(fd));
        if(sendmsg(socket,&msg,MSG_NOSIGNAL|MSG_DONTWAIT)!=sizeof(header))disconnect();
    }
public:
    explicit Mailbox(std::string path=socket_path()):address_path(std::move(path)){}
    Mailbox(const Mailbox&)=delete;
    ~Mailbox(){disconnect();}
    void configure(uint32_t w,uint32_t h,bool bgra,uint32_t flags=0) {
        header.width=w;header.height=h;header.format=bgra?2:1;header.pid=uint32_t(getpid());header.flags=(steam_game()?1u:0u)|flags|(sizeof(void*)==4?8u:0u);
        int comm=open("/proc/self/comm",O_RDONLY|O_CLOEXEC);
        if(comm>=0){ssize_t n=read(comm,header.name,sizeof(header.name)-1);close(comm);if(n>0 && header.name[n-1]=='\n')header.name[n-1]=0;}
    }
    uint32_t resolution() const {return quality;}
    void resolution(uint32_t value) {quality=std::min(value,4u);}
    uint32_t service() {
        if(socket<0){uint64_t now=now_ns();if(now>=retry){retry=now+1000000000ull;connect_receiver();}}
        if(socket<0)return 0;
        // Coalesce control changes; a closed studio immediately stops capture.
        for(unsigned i=0;i<16;i++) {
            Message m{};ssize_t n=recv(socket,&m,sizeof(m),MSG_DONTWAIT|MSG_TRUNC);
            if(n<0 && errno==EINTR)continue;
            if(n<0 && (errno==EAGAIN || errno==EWOULDBLOCK))break;
            if(n!=sizeof(m) || m.magic!=MAGIC || m.version!=VERSION || !((m.kind==1 && (m.value==0 || m.value==30 || m.value==60)) || (m.kind==3 && m.value<=4))){disconnect();break;}
            if(m.kind==1)rate=m.value;else quality=m.value;
        }
        return rate;
    }
    bool publish(const void* pixels,uint64_t captured_at) {
        if(!rate || fd<0 || !pixels || !header.width || !header.height || header.width>4096 || header.height>4096)return false;
        // Reader reopens its received memfd to get an independent flock owner. The studio holds
        // the lock only while copying a frame out, so this worker thread (never the game's)
        // waits up to 4 ms for it; dropping at once lost frames whenever the two met.
        for(int tries=0;flock(fd,LOCK_EX|LOCK_NB)!=0;tries++){if(tries==16)return false;usleep(250);}
        header.sequence++;header.timestamp=captured_at;
        memcpy(static_cast<char*>(mapped)+sizeof(header),pixels,size_t(header.width)*header.height*4);
        memcpy(mapped,&header,sizeof(header));
        flock(fd,LOCK_UN);
        Message ready;ready.kind=2;
        ssize_t n=send(socket,&ready,sizeof(ready),MSG_NOSIGNAL|MSG_DONTWAIT);
        if(n<0 && errno!=EAGAIN && errno!=EWOULDBLOCK)disconnect();
        return true;
    }
};
}
