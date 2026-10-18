#define ROSORAH_LAUNCH_TESTING
#include "launch_watch.cpp"
#include <cassert>
using namespace rosorah::launch;
const std::string hook="/home/test/.local/share/rosorah-studio/renderer-0.8.0/$LIB/librosorah-gl.so";
struct Stack {
    static constexpr uint64_t base=0x100000,sp=base+65536;
    std::vector<uint8_t> memory=std::vector<uint8_t>(131072,0);
    unsigned word;
    uint64_t next=sp+32768;
    explicit Stack(unsigned w,const std::vector<std::string>& env,bool secure=false):word(w) {
        std::vector<uint64_t> table={2,string("/game"),string("argument with spaces"),0};
        for(const auto& value:env)table.push_back(string(value));
        table.insert(table.end(),{0,6,4096,23,secure?1u:0u,25,string("random bytes"),0,0});
        for(size_t i=0;i<table.size();i++)memcpy(memory.data()+sp-base+i*word,&table[i],word);
    }
    uint64_t string(const std::string& value) {
        auto p=next;memcpy(memory.data()+p-base,value.c_str(),value.size()+1);next+=value.size()+1;return p;
    }
    void read(uint64_t address,void* out,size_t size) const {
        if(address<base || size>memory.size() || address-base>memory.size()-size)throw std::runtime_error("invalid fixture read");
        memcpy(out,memory.data()+address-base,size);
    }
    uint64_t pointer(uint64_t address) const {uint64_t out=0;read(address,&out,word);return out;}
    std::string text(uint64_t address) const {
        std::string out;char c;while((read(address++,&c,1),c))out.push_back(c);return out;
    }
    Plan plan(uint64_t lower=base) const {return plan_stack([&](auto a,auto b,auto c){read(a,b,c);},sp,word,lower,hook);}
    std::vector<std::string> env(uint64_t address) const {
        address+=(pointer(address)+2)*word;std::vector<std::string> out;
        while(auto p=pointer(address)){out.push_back(text(p));address+=word;}return out;
    }
};
int unit() {
    // Replay the reported executable chain. Only the final game is an
    // injection candidate; similarly named games must remain candidates.
    const std::vector<std::string> reported={
        "/home/moe/.local/share/Steam/ubuntu12_32/reaper",
        "/run/media/moe/Games/SteamLibrary/steamapps/common/SteamLinuxRuntime_soldier/pressure-vessel/libexec/steam-runtime-tools-0/srt-bwrap",
        "/usr/lib/pressure-vessel/from-host/libexec/steam-runtime-tools-0/pv-adverb",
        "/usr/bin/bash","/usr/bin/bash",
        "/run/media/moe/Games/SteamLibrary/steamapps/common/Dead Cells/deadcells"};
    for(size_t i=0;i<reported.size();i++)assert(launch_wrapper(reported[i])==(i+1<reported.size()));
    assert(!launch_wrapper("/games/reaper") && !launch_wrapper("/games/pv-adverb"));
    for(const char* path:{"/bin/sh","/usr/bin/bash","/bin/dash","/usr/bin/zsh","/bin/ksh","/bin/ksh93","/usr/bin/fish","/usr/bin/env"})
        assert(shell_launcher(path));
    for(const char* path:{"/games/deadcells","/games/bash-game","/games/env-game","/usr/bin/wine64",""})
        assert(!shell_launcher(path));
    for(unsigned word:{4,8}) {
        Stack stack(word,{"SteamAppId=424242","KEEP=unchanged", "LD_PRELOAD=/opt/overlay.so:/home/test/.local/share/rosorah-studio/renderer-0.7.0/$LIB/librosorah-gl.so", "ROSORAH_GL_HOOK_VERSION=0.7.0"});
        auto original=stack.memory;auto plan=stack.plan();assert(plan.decision==Decision::inject);
        assert(stack.memory==original && plan.sp%16==0 && plan.sp+plan.bytes.size()<=Stack::sp);
        memcpy(stack.memory.data()+plan.sp-Stack::base,plan.bytes.data(),plan.bytes.size());
        assert(!memcmp(stack.memory.data()+Stack::sp-Stack::base,original.data()+Stack::sp-Stack::base,original.size()-(Stack::sp-Stack::base)));
        assert(stack.pointer(plan.sp)==2);
        assert(stack.text(stack.pointer(plan.sp+word))=="/game");
        assert(stack.text(stack.pointer(plan.sp+2*word))=="argument with spaces");
        auto env=stack.env(plan.sp);
        assert(env==std::vector<std::string>({"SteamAppId=424242","KEEP=unchanged","LD_PRELOAD="+hook+":/opt/overlay.so","ROSORAH_GL_HOOK_VERSION=0.8.0"}));
        auto aux=plan.sp+(2+2+env.size()+1)*word;
        assert(stack.pointer(aux)==6 && stack.pointer(aux+word)==4096);
        assert(stack.pointer(aux+4*word)==25 && stack.text(stack.pointer(aux+5*word))=="random bytes");
        assert(Stack(word,{"SteamAppId=0"}).plan().decision==Decision::not_game);
        assert(Stack(word,{"SteamGameId=abc"}).plan().decision==Decision::not_game);
        assert(Stack(word,{"SteamGameId=42","ROSORAH_DISABLE_CAPTURE=0"}).plan().decision==Decision::disabled);
        assert(Stack(word,{"SteamGameId=42"},true).plan().decision==Decision::secure);
        assert(Stack(word,{"SteamGameId=42","LD_PRELOAD="+hook}).plan().decision==Decision::inherited);
        assert(Stack(word,{"STEAM_COMPAT_APP_ID=42"}).plan().decision==Decision::inject);
        bool refused=false;try{stack.plan(Stack::sp-32);}catch(...){refused=true;}assert(refused);
        Stack malformed(word,{"SteamAppId=42"});uint64_t oversized=9000;memcpy(malformed.memory.data()+Stack::sp-Stack::base,&oversized,word);
        refused=false;try{malformed.plan();}catch(...){refused=true;}assert(refused);
    }
    assert(game_id("10") && !game_id("00") && !game_id("-1"));
    assert(!own_preload("/opt/another/librosorah-gl.so"));
    assert(own_preload(hook));
    std::string stat="123 (game (name)) S";
    for(unsigned field=4;field<=21;field++)stat+=" 0";
    stat+=" 987654321";
    assert(parse_start_time(stat)==987654321);
    assert(parse_start_time("bad stat")==0);
    puts("PASS: 32/64-bit launch stacks, argv/auxv preservation, bounded staging, preload upgrade, opt-out, secure mode, game filtering and idempotence");
    return 0;
}
int main(int argc,char** argv) {
    if(argc==1)return unit();
    if(argc==3 && !strcmp(argv[1],"--unknown-wrapper")) {
        unsetenv("LD_PRELOAD");
        auto game=fork();assert(game>=0);
        if(game==0) {
            execl("/bin/sh","sh","-c",
                "unset LD_PRELOAD; exec /bin/sh -c 'unset LD_PRELOAD; exec \"$1\" core' rosorah-test \"$1\"",
                "rosorah-test",argv[2],nullptr);_exit(127);
        }
        int status=0;assert(waitpid(game,&status,0)==game);
        return WIFEXITED(status)?WEXITSTATUS(status):1;
    }
    if(argc==4 && !strcmp(argv[1],"--live-chain")) {
        // Exercise the actual Watcher, including fork events, across an
        // unrecognized native wrapper that clears its injected preload.
        int gate[2];assert(pipe(gate)==0);
        auto child=fork();assert(child>=0);
        if(child==0) {
            close(gate[1]);char go=0;
            if(read(gate[0],&go,1)!=1 || go!=1)_exit(0);
            close(gate[0]);setenv("SteamAppId","424242",1);unsetenv("LD_PRELOAD");
            execl(argv[0],argv[0],"--unknown-wrapper",argv[3],nullptr);_exit(127);
        }
        close(gate[0]);int status=0;
        const unsigned long options=PTRACE_O_TRACEEXEC|PTRACE_O_TRACEFORK|PTRACE_O_TRACEVFORK|PTRACE_O_TRACECLONE;
        if(ptrace(PTRACE_SEIZE,child,nullptr,reinterpret_cast<void*>(options))<0) {
            perror("BLOCKED: live launch chain");close(gate[1]);waitpid(child,&status,0);return 77;
        }
        Watcher watcher(getuid(),argv[2]);watcher.adopt_test_trace(child);
        const char go=1;assert(write(gate[1],&go,1)==1);close(gate[1]);
        auto deadline=Clock::now()+std::chrono::seconds(15);
        bool exited=false;
        while(watcher.healthy() && Clock::now()<deadline) {
            watcher.pump();
            if(watcher.test_exit(child,status)){exited=true;break;}
            poll(nullptr,0,5);
        }
        watcher.shutdown();
        if(!exited || !WIFEXITED(status) || WEXITSTATUS(status))return 1;
        puts("PASS: actual observer follows native fork plus two preload-clearing shell execs to graphics fixture");return 0;
    }
    if(argc!=4 || (strcmp(argv[1],"--live") && strcmp(argv[1],"--live-wrappers")))return 2;
    const bool wrappers=!strcmp(argv[1],"--live-wrappers");
    auto child=fork();assert(child>=0);
    if(child==0) {
        if(ptrace(PTRACE_TRACEME,0,nullptr,nullptr)<0){perror("BLOCKED: live launch attachment");_exit(77);}
        raise(SIGSTOP);setenv("SteamAppId","424242",1);unsetenv("LD_PRELOAD");
        if(wrappers) {
            // Both shells explicitly erase any preload before the next exec.
            // The observer must wait for the native graphics fixture.
            execl("/bin/sh","sh","-c",
                "unset LD_PRELOAD; exec /bin/sh -c 'unset LD_PRELOAD; exec \"$1\" core' rosorah-test \"$1\"",
                "rosorah-test",argv[3],nullptr);_exit(127);
        }
        execl(argv[3],argv[3],"core",nullptr);_exit(127);
    }
    int status=0;assert(waitpid(child,&status,0)==child);
    if(WIFEXITED(status))return WEXITSTATUS(status);
    assert(WIFSTOPPED(status));assert(ptrace(PTRACE_SETOPTIONS,child,nullptr,reinterpret_cast<void*>(PTRACE_O_TRACEEXEC))==0);
    int result=-1;
    const unsigned expected_execs=wrappers?3:1;
    for(unsigned step=0;step<expected_execs;step++) {
        assert(ptrace(PTRACE_CONT,child,nullptr,nullptr)==0);assert(waitpid(child,&status,0)==child);
        assert(unsigned(status)>>16==PTRACE_EVENT_EXEC);
        try{result=prepare_launch(child,argv[2]);}catch(const std::exception& e){fprintf(stderr,"%s\n",e.what());result=-1;}
        if(step+1<expected_execs)assert(result==0);
    }
    assert(ptrace(PTRACE_DETACH,child,nullptr,nullptr)==0);
    assert(waitpid(child,&status,0)==child);
    if(result!=1 || !WIFEXITED(status) || WEXITSTATUS(status))return 1;
    puts(wrappers?"PASS: two preload-clearing shells followed to native graphics fixture":"PASS: real exec-boundary attachment and graphics fixture");return 0;
}
