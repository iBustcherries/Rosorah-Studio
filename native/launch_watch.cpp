// Experimental, launch-only OpenGL activation. The rendering hooks are unchanged.
// No late dlopen, code patching, global preload, Steam file edits or syscall tracing.
#include "launch_stack.hpp"
#include <elf.h>
#include <sys/ptrace.h>
#include <sys/signalfd.h>
#include <sys/stat.h>
#include <sys/uio.h>
#include <sys/user.h>
#include <sys/wait.h>
#include <syslog.h>
#include <poll.h>
#include <signal.h>
#include <unistd.h>
#include <dirent.h>
#include <fcntl.h>
#include <cerrno>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <map>
#include <set>
#include <sstream>
#include <array>

namespace rosorah::launch {
using Clock=std::chrono::steady_clock;
inline std::string proc(pid_t pid,const char* part) {return "/proc/"+std::to_string(pid)+"/"+part;}
std::string contents(const std::string& path,size_t limit=65536) {
    std::ifstream f(path);std::string out;char buffer[4096];
    while(f && out.size()<limit){f.read(buffer,std::min(sizeof(buffer),limit-out.size()));out.append(buffer,size_t(f.gcount()));}
    return out;
}
std::string executable(pid_t pid) {
    char path[4096];auto n=readlink(proc(pid,"exe").c_str(),path,sizeof(path));
    return n>0 && n<ssize_t(sizeof(path))?std::string(path,size_t(n)):std::string{};
}
std::string basename(const std::string& path) {return path.substr(path.find_last_of('/')+1);}
bool shell_launcher(const std::string& path) {
    // Use the executable, not comm/argv: a shell running deadcells.sh may
    // adopt the script's name. These wrappers can clear LD_PRELOAD before
    // exec, so leave their initial environment intact and follow their exec
    // and fork events until the actual game starts. The existing timeout
    // bounds observation of wrappers that never launch a game.
    const auto name=basename(path);
    return name=="sh" || name=="bash" || name=="dash" || name=="zsh" ||
        name=="ksh" || name=="ksh93" || name=="fish" || name=="env";
}
bool launch_wrapper(const std::string& path) {
    if(shell_launcher(path))return true;
    // Reported Fedora launch chain: reaper -> srt-bwrap -> pv-adverb ->
    // bash scripts -> deadcells. Each runtime stage must stay observed;
    // adding a preload to reaper and detaching loses the later game exec.
    // Match runtime path suffixes so a game merely named reaper is not
    // mistaken for Steam's launcher. Do not modify the runtime environment.
    const auto ends=[&](const std::string& suffix) {
        return path.size()>=suffix.size() &&
            path.compare(path.size()-suffix.size(),suffix.size(),suffix)==0;
    };
    return ends("/ubuntu12_32/reaper") ||
        ends("/steam-runtime-tools-0/srt-bwrap") ||
        ends("/steam-runtime-tools-0/pv-adverb");
}
uint64_t parse_start_time(const std::string& text) {
    auto pos=text.rfind(')');
    if(pos==std::string::npos)return 0;
    std::istringstream fields(text.substr(pos+2));std::string value;
    for(unsigned n=3;n<=22;n++)if(!(fields>>value))return 0;
    try{return std::stoull(value);}catch(...){return 0;}
}
uint64_t start_time(pid_t pid) {return parse_start_time(contents(proc(pid,"stat")));}
bool same_user(pid_t pid,uid_t uid) {
    auto text=contents(proc(pid,"status"));auto at=text.find("\nUid:\t");
    if(at==std::string::npos)return false;
    std::istringstream fields(text.substr(at+6));uint64_t value;
    for(unsigned i=0;i<4;i++)if(!(fields>>value) || value!=uid)return false;
    return true;
}
pid_t group(pid_t tid) {
    auto text=contents(proc(tid,"status"));auto at=text.find("\nTgid:\t");
    if(at==std::string::npos)return -1;
    return pid_t(strtol(text.c_str()+at+7,nullptr,10));
}
std::vector<pid_t> ids(const std::string& path) {
    std::vector<pid_t> out;DIR* dir=opendir(path.c_str());if(!dir)return out;
    while(auto* entry=readdir(dir)) {
        char* end=nullptr;long id=strtol(entry->d_name,&end,10);
        if(end && !*end && id>0 && id<=INT32_MAX)out.push_back(pid_t(id));
    }
    closedir(dir);return out;
}
bool launcher(pid_t pid) {
    if(access(proc(pid,"root/.flatpak-info").c_str(),F_OK)==0)return false;
    auto name=basename(executable(pid));
    return name=="steam" || name=="steam-runtime-launcher-service";
}
void event(const char* kind,pid_t pid=0,int value=0) {
    printf("%s %d %d\n",kind,pid,value);fflush(stdout);
    auto name=pid>0?basename(executable(pid)):std::string{};
    for(char& c:name)if(static_cast<unsigned char>(c)<32 || c==127)c='_';
    syslog(LOG_INFO,"hotfix3 %s pid=%d detail=%d exe=%s",kind,pid,value,name.c_str());
}
class Memory {
    pid_t pid;
    std::map<uint64_t,std::array<uint8_t,4096>> pages;
public:
    explicit Memory(pid_t p):pid(p){}
    void read(uint64_t address,void* result,size_t size) {
        auto* out=static_cast<uint8_t*>(result);
        while(size) {
            if(address>0x7fffffffffffULL)throw std::runtime_error("invalid target address");
            auto base=address&~uint64_t(4095);auto it=pages.find(base);
            if(it==pages.end()) {
                std::array<uint8_t,4096> bytes{};
                iovec local{bytes.data(),bytes.size()},remote{reinterpret_cast<void*>(base),bytes.size()};
                if(process_vm_readv(pid,&local,1,&remote,1,0)!=ssize_t(bytes.size()))
                    throw std::runtime_error("cannot read initial stack");
                it=pages.emplace(base,bytes).first;
            }
            auto offset=size_t(address-base),n=std::min(size,size_t(4096)-offset);
            memcpy(out,it->second.data()+offset,n);address+=n;out+=n;size-=n;
        }
    }
};
uint64_t stack_lower(pid_t pid,uint64_t sp) {
    std::istringstream maps(contents(proc(pid,"maps"),1024*1024));std::string line;
    while(std::getline(maps,line)) {
        unsigned long long begin=0,end=0;char perms[5]{};
        if(sscanf(line.c_str(),"%llx-%llx %4s",&begin,&end,perms)==3 &&
           sp>=begin && sp<end && perms[0]=='r' && perms[1]=='w')return begin;
    }
    throw std::runtime_error("initial stack mapping unavailable");
}
unsigned image_word(pid_t pid) {
    std::ifstream f(proc(pid,"exe"),std::ios::binary);
    Elf64_Ehdr h{};f.read(reinterpret_cast<char*>(&h),sizeof(h));
    if(!f || memcmp(h.e_ident,ELFMAG,SELFMAG) || h.e_ident[EI_DATA]!=ELFDATA2LSB)return 0;
    if(h.e_ident[EI_CLASS]==ELFCLASS64 && h.e_machine==EM_X86_64)return 8;
    if(h.e_ident[EI_CLASS]==ELFCLASS32 && h.e_machine==EM_386)return 4;
    return 0;
}
bool hook_visible(pid_t pid,unsigned word,const std::string& hook) {
    auto token=hook.find("$LIB");
    if(token==std::string::npos)return false;
    const std::vector<std::string> dirs=word==8?std::vector<std::string>{"lib64","lib/x86_64-linux-gnu"}:
        std::vector<std::string>{"lib","lib32","lib/i386-linux-gnu"};
    for(const auto& dir:dirs) {
        auto path=hook;path.replace(token,4,dir);
        std::ifstream f(proc(pid,"root")+path,std::ios::binary);unsigned char header[5]{};
        if(f.read(reinterpret_cast<char*>(header),sizeof(header)) &&
           !memcmp(header,ELFMAG,SELFMAG) && header[EI_CLASS]==(word==8?ELFCLASS64:ELFCLASS32))return true;
    }
    return false;
}
// Returns 1 for committed insertion, 2 for an inherited hook, 0 for a wrapper
// without a game id, -1 for an unsupported/disabled process. Caller detaches
// on unsupported/disabled results. Successful preparation is followed across
// further execs until the capture backend loads or the startup timeout expires.
int inject(pid_t pid,const std::string& hook) {
    unsigned word=image_word(pid);if(!word)return -1;
    std::array<uint8_t,sizeof(user_regs_struct)> raw{};iovec regset{raw.data(),raw.size()};
    if(ptrace(PTRACE_GETREGSET,pid,reinterpret_cast<void*>(NT_PRSTATUS),&regset)<0)
        throw std::runtime_error("register read failed");
    const size_t expected=word==8?sizeof(user_regs_struct):17*sizeof(uint32_t);
    if(regset.iov_len!=expected)throw std::runtime_error("unsupported register layout");
    const size_t sp_offset=word==8?offsetof(user_regs_struct,rsp):15*sizeof(uint32_t);
    uint64_t old_sp=0;memcpy(&old_sp,raw.data()+sp_offset,word);
    Memory memory(pid);
    auto plan=plan_stack([&](uint64_t a,void* b,size_t n){memory.read(a,b,n);},old_sp,word,stack_lower(pid,old_sp),hook);
    if(plan.decision==Decision::not_game)return 0;
    if(plan.decision==Decision::inherited)return 2;
    if(plan.decision!=Decision::inject)return -1;
    if(!hook_visible(pid,word,hook)){event("HOOK_UNAVAILABLE",pid,word*8);return -1;}
    // The original stack is untouched. Partial write/verification failures leave
    // original SP and the complete original environment valid.
    iovec local{plan.bytes.data(),plan.bytes.size()},remote{reinterpret_cast<void*>(plan.sp),plan.bytes.size()};
    if(process_vm_writev(pid,&local,1,&remote,1,0)!=ssize_t(plan.bytes.size()))
        throw std::runtime_error("stack staging failed");
    std::vector<uint8_t> verify(plan.bytes.size());local={verify.data(),verify.size()};
    if(process_vm_readv(pid,&local,1,&remote,1,0)!=ssize_t(verify.size()) || verify!=plan.bytes)
        throw std::runtime_error("stack verification failed");
    memcpy(raw.data()+sp_offset,&plan.sp,word);
    if(ptrace(PTRACE_SETREGSET,pid,reinterpret_cast<void*>(NT_PRSTATUS),&regset)<0)
        throw std::runtime_error("stack commit failed");
    event("INJECTED",pid,word*8);return 1;
}
int prepare_launch(pid_t pid,const std::string& hook) {
    return launch_wrapper(executable(pid))?0:inject(pid,hook);
}
struct Trace {bool root=false;bool release=false;Clock::time_point since=Clock::now();};
class Watcher {
    uid_t uid;
    std::string hook;
    std::map<pid_t,Trace> traced;
    std::map<pid_t,uint64_t> refused;
    bool draining=false;
    bool broken=false;
#ifdef ROSORAH_LAUNCH_TESTING
    std::map<pid_t,int> test_exits;
#endif
    static constexpr unsigned long options=PTRACE_O_TRACEFORK|PTRACE_O_TRACEVFORK|PTRACE_O_TRACECLONE|PTRACE_O_TRACEEXEC;
    void resume(pid_t tid,int signal=0) {
        if(ptrace(PTRACE_CONT,tid,nullptr,reinterpret_cast<void*>(uintptr_t(signal)))<0) {
            if(errno==ESRCH)traced.erase(tid);
            else {event("FAILED",tid,errno);broken=true;}
        }
    }
    void detach(pid_t tid,int signal=0) {
        if(ptrace(PTRACE_DETACH,tid,nullptr,reinterpret_cast<void*>(uintptr_t(signal)))==0 || errno==ESRCH)traced.erase(tid);
        else {traced[tid].release=true;event("DETACH_RETRY",tid,errno);broken=true;}
    }
public:
    Watcher(uid_t u,std::string h):uid(u),hook(std::move(h)){}
    bool healthy() const {return !broken;}
#ifdef ROSORAH_LAUNCH_TESTING
    void adopt_test_trace(pid_t pid) {traced[pid]={false,false,Clock::now()};}
    bool test_exit(pid_t pid,int& status) const {
        auto it=test_exits.find(pid);if(it==test_exits.end())return false;
        status=it->second;return true;
    }
#endif
    void discover() {
        for(auto pid:ids("/proc")) {
            if(!same_user(pid,uid) || !launcher(pid))continue;
            auto stamp=start_time(pid);
            if(refused.count(pid) && refused[pid]==stamp)continue;
            bool added=false,failed=false;
            // Repeat scans also catch a thread born during the initial attach.
            for(auto tid:ids(proc(pid,"task"))) {
                if(traced.count(tid))continue;
                if(ptrace(PTRACE_SEIZE,tid,nullptr,reinterpret_cast<void*>(options))<0) {
                    if(errno!=ESRCH){event("BLOCKED",pid,errno);failed=true;}
                    continue;
                }
                traced[tid]={true,false,Clock::now()};added=true;
            }
            if(added && !failed)event("WATCHING",pid);
            if(failed) {
                refused[pid]=stamp;
                // Do not leave a partially watched launcher running. A later
                // authorized helper can attach all of its threads together.
                for(auto& [tid,t]:traced)if(t.root && group(tid)==pid) {
                    t.release=true;ptrace(PTRACE_INTERRUPT,tid,nullptr,nullptr);
                }
            }
        }
        auto now=Clock::now();
        std::map<pid_t,bool> ready;
        std::set<pid_t> reported;
        for(auto& [tid,t]:traced)if(!t.root && !t.release) {
            auto pid=group(tid);if(pid<=0)pid=tid;
            if(!ready.count(pid))ready[pid]=contents(proc(pid,"maps"),1024*1024).find("/librosorah-gl-capture.so")!=std::string::npos;
            if(ready[pid] || now-t.since>std::chrono::seconds(10)) {
                if(reported.insert(pid).second)event(ready[pid]?"CAPTURE_LOADED":"STARTUP_TIMEOUT",pid);
                t.release=true;ptrace(PTRACE_INTERRUPT,tid,nullptr,nullptr);
            }
        }
        // Only ids and start times are retained; no process environment is logged.
        for(auto it=refused.begin();it!=refused.end();) {
            if(start_time(it->first)!=it->second)it=refused.erase(it);else ++it;
        }
    }
    void stopped(pid_t tid,int status) {
        if(WIFEXITED(status)||WIFSIGNALED(status)){
#ifdef ROSORAH_LAUNCH_TESTING
            test_exits[tid]=status;
#endif
            traced.erase(tid);return;
        }
        if(!WIFSTOPPED(status))return;
        int sig=WSTOPSIG(status);unsigned ev=unsigned(status)>>16;
        if(!traced.count(tid))traced[tid]={false,false,Clock::now()};
        if(ev==PTRACE_EVENT_EXEC) {
            unsigned long old=0;
            if(ptrace(PTRACE_GETEVENTMSG,tid,nullptr,&old)==0 && old && old!=unsigned(tid))traced.erase(pid_t(old));
        }
        if(draining || traced[tid].release || !same_user(tid,uid)) {
            if(!draining && !traced[tid].release)event("OWNER_CHANGED",tid);
            detach(tid,ev?0:sig);return;
        }
        if(ev==PTRACE_EVENT_FORK || ev==PTRACE_EVENT_VFORK || ev==PTRACE_EVENT_CLONE) {
            unsigned long child=0;
            if(ptrace(PTRACE_GETEVENTMSG,tid,nullptr,&child)==0 && child) {
                bool root=traced[tid].root && group(pid_t(child))==group(tid);
                traced[pid_t(child)]={root,false,Clock::now()};
            }
            resume(tid);return;
        }
        if(ev==PTRACE_EVENT_EXEC) {
            event("EXEC",tid);
            if(launcher(tid)){traced[tid].root=true;resume(tid);return;}
            traced[tid].root=false;
            try {
                int result=prepare_launch(tid,hook);
                if(result<0){event("SKIPPED",tid);detach(tid);return;}
                // A prepared preload can still be cleared by a later exec.
                // Keep fork/exec observation active even after result 1 or 2.
                if(result>0)event("FOLLOWING",tid,result);
            } catch(const std::exception& e) {
                syslog(LOG_INFO,"hotfix3 preparation failed pid=%d reason=%s",tid,e.what());
                event("SKIPPED",tid);detach(tid);return;
            }
            resume(tid);return;
        }
        if(ev==PTRACE_EVENT_STOP) {
            if(sig==SIGTRAP)resume(tid);
            else if(ptrace(PTRACE_LISTEN,tid,nullptr,nullptr)<0)detach(tid,sig);
            return;
        }
        // Preserve application signal delivery, including real SIGTRAPs.
        resume(tid,sig);
    }
    void pump() {
        int status=0;pid_t tid;
        while((tid=waitpid(-1,&status,__WALL|WNOHANG))>0)stopped(tid,status);
    }
    void shutdown() {
        draining=true;
        for(const auto& [tid,t]:traced){(void)t;ptrace(PTRACE_INTERRUPT,tid,nullptr,nullptr);}
        auto deadline=Clock::now()+std::chrono::seconds(2);
        while(!traced.empty() && Clock::now()<deadline) {
            pump();poll(nullptr,0,5);
        }
        // Deliberately no EXITKILL, SIGKILL or global SIGCONT. Linux detaches
        // remaining tracees when this helper exits, preserving application life.
        event("STOPPED");
    }
};
}

#ifndef ROSORAH_LAUNCH_TESTING
int main(int argc,char** argv) {
    using namespace rosorah::launch;
    setvbuf(stdout,nullptr,_IOLBF,0);
    openlog("rosorah-launch-watch",LOG_PID|LOG_NDELAY,LOG_USER);
    if(argc!=7 || strcmp(argv[1],"--owner-pid") || strcmp(argv[3],"--owner-start") || strcmp(argv[5],"--hook")) {
        fprintf(stderr,"Rosorah experimental Steam launch observer. Started by Rosorah Studio.\n");return 2;
    }
    pid_t owner=0;uint64_t stamp=0;uid_t uid=getuid();std::string hook=argv[6];
    try {
        auto number=[](const char* s){size_t n=0;auto v=std::stoull(s,&n);if(n!=strlen(s))throw std::runtime_error("bad number");return v;};
        auto p=number(argv[2]);if(p<2 || p>INT32_MAX)throw std::runtime_error("bad pid");owner=pid_t(p);stamp=number(argv[4]);
        if(geteuid()==0) {
            const char* pk=getenv("PKEXEC_UID");if(!pk)throw std::runtime_error("use the in-app permission button");
            auto value=number(pk);if(!value || value>UINT32_MAX)throw std::runtime_error("bad owner");uid=uid_t(value);
        } else if(getuid()!=geteuid())throw std::runtime_error("setuid execution is unsupported");
        if(!uid || !stamp || start_time(owner)!=stamp || !same_user(owner,uid) || basename(executable(owner))!="rosorah-studio")
            throw std::runtime_error("studio owner does not match");
        if(hook.empty() || hook.front()!='/' || hook.size()>3000 || hook.find_first_of(" :\t\n\r")!=std::string::npos ||
           hook.find("$LIB")==std::string::npos || !own_preload(hook))throw std::runtime_error("invalid hook path");
    }catch(const std::exception& e){fprintf(stderr,"Activation refused: %s\n",e.what());return 2;}
    sigset_t mask;sigemptyset(&mask);sigaddset(&mask,SIGCHLD);sigaddset(&mask,SIGTERM);sigaddset(&mask,SIGINT);
    if(sigprocmask(SIG_BLOCK,&mask,nullptr)<0)return 2;
    int fd=signalfd(-1,&mask,SFD_NONBLOCK|SFD_CLOEXEC);if(fd<0)return 2;
    Watcher watcher(uid,hook);event("STARTED");auto next=Clock::now();bool quit=false;
    try {
        while(!quit && watcher.healthy() && start_time(owner)==stamp && same_user(owner,uid)) {
            auto now=Clock::now();if(now>=next){watcher.discover();next=now+std::chrono::seconds(1);}
            watcher.pump();pollfd p[2]={{fd,POLLIN,0},{STDIN_FILENO,POLLIN|POLLHUP,0}};poll(p,2,250);
            // Closing this private parent pipe also stops an elevated helper;
            // the unprivileged studio never needs to signal a root process.
            if(p[1].revents&(POLLIN|POLLHUP|POLLERR|POLLNVAL))quit=true;
            signalfd_siginfo info{};
            while(read(fd,&info,sizeof(info))==sizeof(info))if(info.ssi_signo==SIGTERM || info.ssi_signo==SIGINT)quit=true;
        }
    } catch(const std::exception&) {event("FAILED");}
    watcher.shutdown();close(fd);return 0;
}
#endif
