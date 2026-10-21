#include "transport.hpp"
#include <cassert>
#include <cstdio>
#include <vector>
#include <thread>
#include <tuple>
int main(int argc,char** argv) {
    if(argc==3 && std::string(argv[1])=="--produce") {
        rosorah::Mailbox producer(argv[2]);producer.configure(2,1,true);
        const uint32_t pixels[2]={0x00010203,0x00040506};
        bool delivered=false;
        for(unsigned i=0;i<500;i++) {
            if(producer.service()){producer.publish(pixels,rosorah::now_ns());delivered=true;}
            std::this_thread::sleep_for(std::chrono::milliseconds(2));
        }
        return delivered?0:1;
    }
    unsigned w=0,h=0;
    rosorah::capture_size(3440,1440,1,w,h);assert(w==1920 && h==803);
    rosorah::capture_size(640,480,3,w,h);assert(w==640 && h==480);
    rosorah::capture_size(1440,2560,4,w,h);assert(w==1440 && h==2560);
    puts("PASS: ultrawide aspect ratio, original portrait size, no upscaling");
    char root[]="/tmp/rosorah-ipc-XXXXXX";assert(mkdtemp(root));
    std::string path=std::string(root)+"/receiver.sock";
    int server=socket(AF_UNIX,SOCK_SEQPACKET|SOCK_NONBLOCK|SOCK_CLOEXEC,0);
    if(server<0 && (errno==EPERM || errno==EACCES)){fprintf(stderr,"BLOCKED: sandbox denies Unix sockets; IPC integration was not run.\n");rmdir(root);return 77;}
    assert(server>=0);
    sockaddr_un address{};address.sun_family=AF_UNIX;strcpy(address.sun_path,path.c_str());
    assert(bind(server,reinterpret_cast<sockaddr*>(&address),sizeof(address))==0);assert(listen(server,4)==0);
    rosorah::Mailbox producer(path);producer.configure(2,1,true);
    assert(producer.service()==0);
    int client=accept4(server,nullptr,nullptr,SOCK_CLOEXEC|SOCK_NONBLOCK);assert(client>=0);
    rosorah::Header header{};iovec iov{&header,sizeof(header)};
    alignas(cmsghdr) char control[CMSG_SPACE(sizeof(int))]{};
    msghdr msg{};msg.msg_iov=&iov;msg.msg_iovlen=1;msg.msg_control=control;msg.msg_controllen=sizeof(control);
    assert(recvmsg(client,&msg,MSG_CMSG_CLOEXEC)==sizeof(header));
    auto* c=CMSG_FIRSTHDR(&msg);assert(c && c->cmsg_level==SOL_SOCKET && c->cmsg_type==SCM_RIGHTS);
    int passed=-1;memcpy(&passed,CMSG_DATA(c),sizeof(passed));assert(passed>=0);
    std::string fdpath="/proc/self/fd/"+std::to_string(passed);
    int memory=open(fdpath.c_str(),O_RDONLY|O_CLOEXEC);assert(memory>=0);close(passed);
    struct stat st{};assert(fstat(memory,&st)==0 && st.st_nlink==0 && st.st_size==rosorah::CAPACITY);
    assert((fcntl(memory,F_GET_SEALS)&(F_SEAL_GROW|F_SEAL_SHRINK))==(F_SEAL_GROW|F_SEAL_SHRINK));
    rosorah::Message active;active.value=60;assert(send(client,&active,sizeof(active),MSG_NOSIGNAL)==sizeof(active));assert(producer.service()==60);
    rosorah::Message quality;quality.kind=3;quality.value=4;
    assert(send(client,&quality,sizeof(quality),MSG_NOSIGNAL)==sizeof(quality));
    assert(producer.service()==60 && producer.resolution()==4);
    uint32_t pixels[2]={0xff030201,0xff060504};
    assert(producer.publish(pixels,rosorah::now_ns()));
    rosorah::Message ready;assert(recv(client,&ready,sizeof(ready),0)==sizeof(ready) && ready.kind==2);
    assert(flock(memory,LOCK_SH|LOCK_NB)==0);
    assert(!producer.publish(pixels,rosorah::now_ns())); // independent lock owner
    assert(pread(memory,&header,sizeof(header),0)==sizeof(header) && header.width==2 && header.height==1 && header.sequence==1);
    uint32_t copied[2]{};assert(pread(memory,copied,sizeof(copied),sizeof(header))==sizeof(copied));assert(memcmp(copied,pixels,sizeof(pixels))==0);
    flock(memory,LOCK_UN);
    // A slow consumer cannot build an unbounded frame queue. Latest data wins.
    for(uint32_t n=0;n<1000;n++){pixels[0]=n;assert(producer.publish(pixels,rosorah::now_ns()));}
    assert(pread(memory,copied,sizeof(copied),sizeof(header))==sizeof(copied) && copied[0]==999);
    close(client);assert(producer.service()==0);
    close(memory);close(server);unlink(path.c_str());rmdir(root);
    // Pacing: a game presenting at the capture rate with jitter keeps every frame; a faster one is
    // thinned to the capture rate.
    for(auto [present_hz,jitter_ns,expected]:{std::tuple{60.,1500000.,600},std::tuple{144.,300000.,600},std::tuple{30.,1000000.,300}}) {
        const uint64_t interval=1000000000ull/60;uint64_t next=0;int captured=0;uint32_t seed=1;
        for(int i=0;i<int(present_hz*10);i++) {
            seed=seed*1103515245u+12345u;
            const uint64_t now=uint64_t(1e9+i*1e9/present_hz+(double(seed>>16)/65535.-0.5)*2*jitter_ns);
            if(rosorah::capture_due(now,next,interval)){captured++;next=rosorah::following_capture(now,next,interval);}
        }
        if(std::abs(captured-expected)>3){fprintf(stderr,"pacing at %.0f Hz captured %d of %d\n",present_hz,captured,expected);return 1;}
    }
    puts("PASS: descriptor passing, anonymous sealed memory, independent locks, drop on contention, latest-frame coalescing, studio disconnect");
}
