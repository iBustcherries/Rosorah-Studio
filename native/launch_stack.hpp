// Linux x86 initial-stack planner. No target memory is changed by this module.
// Keep argv and auxv pointers intact; place a new environment table BELOW the
// original stack. The only commit is changing SP after the complete write.
#pragma once
#include <algorithm>
#include <cstdint>
#include <cstring>
#include <functional>
#include <stdexcept>
#include <string>
#include <vector>

namespace rosorah::launch {
using Read = std::function<void(uint64_t, void*, size_t)>;
enum class Decision { inject, inherited, not_game, disabled, secure };
struct Plan {
    Decision decision = Decision::not_game;
    uint64_t sp = 0;
    std::vector<uint8_t> bytes;
};
inline bool game_id(const std::string& value) {
    return !value.empty() && value.size() <= 20 &&
        value.find_first_not_of("0123456789") == std::string::npos &&
        value.find_first_not_of('0') != std::string::npos;
}
inline bool own_preload(const std::string& value) {
    const std::string suffix = "/librosorah-gl.so";
    return value.find("/rosorah-studio/renderer-") != std::string::npos &&
        value.size() >= suffix.size() && value.compare(value.size()-suffix.size(),suffix.size(),suffix)==0;
}
inline std::vector<std::string> preload_tokens(const std::string& text) {
    std::vector<std::string> out;
    size_t at = 0;
    while (at < text.size()) {
        at = text.find_first_not_of(" :\t\n", at);
        if (at == std::string::npos) break;
        auto end = text.find_first_of(" :\t\n", at);
        if (end == std::string::npos) end = text.size();
        out.push_back(text.substr(at,end-at)); at=end;
    }
    return out;
}
inline Plan plan_stack(const Read& read, uint64_t old_sp, unsigned word,
                       uint64_t lower_bound, const std::string& hook) {
    if ((word != 4 && word != 8) || old_sp < lower_bound ||
        hook.empty() || hook.front() != '/' || hook.size() > 3000 ||
        hook.find_first_of(" :\t\n\r") != std::string::npos)
        throw std::runtime_error("unsupported stack or hook path");
    auto pointer = [&](uint64_t address) {
        uint64_t v=0; read(address,&v,word); return v;
    };
    size_t budget=1024*1024;
    auto string_at = [&](uint64_t address) {
        std::string out;
        for (unsigned i=0;i<65536 && budget;i++,budget--) {
            char c=0; read(address+i,&c,1);
            if (!c) return out;
            out.push_back(c);
        }
        throw std::runtime_error("environment exceeds bounded input");
    };
    uint64_t argc=pointer(old_sp);
    if (argc > 4096) throw std::runtime_error("argument count exceeds bound");
    std::vector<uint64_t> prefix;
    uint64_t cursor=old_sp;
    for (uint64_t i=0;i<argc+2;i++,cursor+=word) prefix.push_back(pointer(cursor));
    if (prefix.back()) throw std::runtime_error("argument terminator missing");
    struct Env { uint64_t pointer; std::string key,value; };
    std::vector<Env> env;
    bool game=false,disabled=false,current=false;
    std::vector<std::string> preloads;
    for (unsigned i=0;;i++,cursor+=word) {
        if (i>=4096) throw std::runtime_error("environment count exceeds bound");
        auto p=pointer(cursor); if (!p) {cursor+=word;break;}
        auto text=string_at(p); auto split=text.find('=');
        std::string key=text.substr(0,split);
        std::string value=split==std::string::npos?"":text.substr(split+1);
        if (key=="SteamAppId" || key=="SteamGameId" || key=="STEAM_COMPAT_APP_ID") game|=game_id(value);
        disabled|=key=="ROSORAH_DISABLE_CAPTURE";
        if (key=="LD_PRELOAD") for (auto& token:preload_tokens(value)) {
            current|=token==hook;
            if (!own_preload(token) && token!=hook) preloads.push_back(token);
        }
        env.push_back({p,std::move(key),std::move(value)});
    }
    std::vector<uint64_t> aux;
    bool secure=false;
    for (unsigned i=0;;i++,cursor+=word*2) {
        if (i>=128) throw std::runtime_error("auxiliary vector exceeds bound");
        auto type=pointer(cursor), value=pointer(cursor+word);
        aux.push_back(type);aux.push_back(value);
        if(type==23 && value)secure=true; // AT_SECURE
        if(!type)break;
    }
    if(secure)return {Decision::secure,0,{}};
    if(disabled)return {Decision::disabled,0,{}};
    if(!game)return {};
    if(current)return {Decision::inherited,0,{}};
    std::string preload="LD_PRELOAD="+hook;
    for(auto& token:preloads)preload+=":"+token;
    const std::string marker="ROSORAH_GL_HOOK_VERSION=0.8.0";
    std::vector<uint64_t> table=prefix;
    for(const auto& e:env)if(e.key!="LD_PRELOAD" && e.key!="ROSORAH_GL_HOOK_VERSION")table.push_back(e.pointer);
    size_t preload_index=table.size();table.push_back(0);table.push_back(0);table.push_back(0);
    table.insert(table.end(),aux.begin(),aux.end());
    size_t table_bytes=table.size()*word;
    size_t total=(table_bytes+preload.size()+1+marker.size()+1+15)&~size_t(15);
    if(total>65536 || old_sp-lower_bound<total+128)
        throw std::runtime_error("insufficient mapped initial stack space");
    uint64_t sp=(old_sp-total)&~uint64_t(15);
    if(sp<lower_bound || (word==4 && old_sp>UINT32_MAX))
        throw std::runtime_error("invalid stack address");
    table[preload_index]=sp+table_bytes;
    table[preload_index+1]=sp+table_bytes+preload.size()+1;
    Plan plan{Decision::inject,sp,std::vector<uint8_t>(size_t(old_sp-sp),0)};
    for(size_t i=0;i<table.size();i++) {
        if(word==4 && table[i]>UINT32_MAX)throw std::runtime_error("32-bit pointer overflow");
        memcpy(plan.bytes.data()+i*word,&table[i],word);
    }
    memcpy(plan.bytes.data()+table_bytes,preload.c_str(),preload.size()+1);
    memcpy(plan.bytes.data()+table_bytes+preload.size()+1,marker.c_str(),marker.size()+1);
    return plan;
}
}
