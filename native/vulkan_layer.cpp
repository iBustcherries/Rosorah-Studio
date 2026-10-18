// Independent Vulkan presentation capture layer. No OBS code or dependency.
#include <vulkan/vulkan.h>
#include <vulkan/vk_layer.h>
#include "transport.hpp"
#include "color.hpp"
#include <algorithm>
#include <array>
#include <atomic>
#include <thread>
#include <memory>
#include <mutex>
#include <unordered_map>
#include <vector>

namespace {
std::recursive_mutex gate;
void* key(const void* h) { return h?*static_cast<void* const*>(h):nullptr; }
struct Instance {
    VkInstance handle{}; PFN_vkGetInstanceProcAddr gipa{};
    PFN_GetPhysicalDeviceProcAddr gpdpa{};
};
struct ReadSlot {
    VkBuffer buffer{}; VkDeviceMemory memory{}; void* mapped=nullptr;
    VkImage scaled{}; VkDeviceMemory image_memory{};
    VkCommandPool pool{}; VkCommandBuffer command{}; VkFence fence{};
    bool coherent=false;
    uint64_t timestamp=0;
    // Release/acquire transfers ownership between presentation and delivery.
    std::atomic<bool> pending{false};
};
struct Readback {
    std::array<ReadSlot,2> slots;
    uint32_t family=UINT32_MAX;
    std::vector<VkSemaphore> ready;
};
struct Swap {
    VkExtent2D size{}, output{}; VkFormat format{}; rosorah::Color color; bool supported=false;
    VkFilter filter=VK_FILTER_NEAREST;
    std::vector<VkImage> images;
    Readback read;
    rosorah::Mailbox mailbox;
    std::thread worker;
    std::atomic<bool> stop{false};
    std::atomic<uint32_t> fps{0};
    std::atomic<uint32_t> requested_quality{1};uint32_t quality=1;
    uint64_t next_capture=0;
    ~Swap(){stop.store(true,std::memory_order_release);if(worker.joinable())worker.join();}
};
#define DEVICE_FUNCTIONS(F) \
 F(DestroyDevice) F(GetDeviceQueue) F(GetDeviceQueue2) F(CreateSwapchainKHR) F(DestroySwapchainKHR) F(GetSwapchainImagesKHR) F(QueuePresentKHR) \
 F(CreateImage) F(DestroyImage) F(GetImageMemoryRequirements) F(BindImageMemory) F(CmdBlitImage) \
 F(CreateBuffer) F(DestroyBuffer) F(GetBufferMemoryRequirements) F(AllocateMemory) F(FreeMemory) F(BindBufferMemory) F(MapMemory) F(UnmapMemory) F(InvalidateMappedMemoryRanges) \
 F(CreateCommandPool) F(DestroyCommandPool) F(AllocateCommandBuffers) F(ResetCommandPool) F(BeginCommandBuffer) F(EndCommandBuffer) F(CmdPipelineBarrier) F(CmdCopyImageToBuffer) \
 F(CreateFence) F(DestroyFence) F(ResetFences) F(WaitForFences) F(GetFenceStatus) F(CreateSemaphore) F(DestroySemaphore) F(QueueSubmit) F(DeviceWaitIdle)
struct Device {
    VkDevice handle{}; VkPhysicalDevice physical{}; Instance instance;
    PFN_vkGetDeviceProcAddr gdpa{}; PFN_vkSetDeviceLoaderData set_data{};
    VkPhysicalDeviceMemoryProperties memory{};
    std::vector<VkQueueFamilyProperties> families;
    std::unordered_map<VkQueue,uint32_t> queues;
    std::unordered_map<VkSwapchainKHR,std::unique_ptr<Swap>> swaps;
#define DECLARE(n) PFN_vk##n n{};
    DEVICE_FUNCTIONS(DECLARE)
#undef DECLARE
};
std::unordered_map<void*,Instance> instances;
std::unordered_map<void*,std::shared_ptr<Device>> devices;
std::shared_ptr<Device> device(const void* handle) { auto it=devices.find(key(handle));return it==devices.end()?nullptr:it->second; }
bool color_format(VkFormat f,VkColorSpaceKHR space,rosorah::Color& color) {
    using P=rosorah::Pixel;using T=rosorah::Transfer;
    if(space==VK_COLOR_SPACE_SRGB_NONLINEAR_KHR) {
        color.transfer=T::Srgb;
        switch(f) {
            case VK_FORMAT_R8G8B8A8_UNORM:case VK_FORMAT_R8G8B8A8_SRGB:color.pixel=P::Rgba8;return true;
            case VK_FORMAT_B8G8R8A8_UNORM:case VK_FORMAT_B8G8R8A8_SRGB:color.pixel=P::Bgra8;return true;
            case VK_FORMAT_A2B10G10R10_UNORM_PACK32:color.pixel=P::Rgb10;return true;
            case VK_FORMAT_A2R10G10B10_UNORM_PACK32:color.pixel=P::Bgr10;return true;
            default:return false;
        }
    }
    if(space==VK_COLOR_SPACE_EXTENDED_SRGB_LINEAR_EXT && f==VK_FORMAT_R16G16B16A16_SFLOAT){color={P::Rgba16f,T::Linear709};return true;}
    if((space==VK_COLOR_SPACE_HDR10_ST2084_EXT || space==VK_COLOR_SPACE_HDR10_HLG_EXT) &&
       (f==VK_FORMAT_A2B10G10R10_UNORM_PACK32 || f==VK_FORMAT_A2R10G10B10_UNORM_PACK32)) {
        color.pixel=f==VK_FORMAT_A2B10G10R10_UNORM_PACK32?P::Rgb10:P::Bgr10;
        color.transfer=space==VK_COLOR_SPACE_HDR10_ST2084_EXT?T::Pq2020:T::Hlg2020;return true;
    }
    return false;
}

void stop_worker(Swap& s) {
    s.stop.store(true,std::memory_order_release);
    if(s.worker.joinable())s.worker.join();
    s.fps.store(0,std::memory_order_relaxed);
}
void cleanup(Device& d,Swap& s) {
    stop_worker(s);
    auto& r=s.read;
    bool pending=false;for(auto& slot:r.slots)pending|=slot.pending.load();
    if(pending)d.DeviceWaitIdle(d.handle);
    for(auto sem:r.ready)if(sem)d.DestroySemaphore(d.handle,sem,nullptr);
    r.ready.clear();
    for(auto& slot:r.slots) {
        if(slot.fence)d.DestroyFence(d.handle,slot.fence,nullptr);
        if(slot.pool)d.DestroyCommandPool(d.handle,slot.pool,nullptr);
        if(slot.scaled)d.DestroyImage(d.handle,slot.scaled,nullptr);
        if(slot.image_memory)d.FreeMemory(d.handle,slot.image_memory,nullptr);
        if(slot.mapped)d.UnmapMemory(d.handle,slot.memory);
        if(slot.buffer)d.DestroyBuffer(d.handle,slot.buffer,nullptr);
        if(slot.memory)d.FreeMemory(d.handle,slot.memory,nullptr);
        slot.fence=VK_NULL_HANDLE;slot.pool=VK_NULL_HANDLE;slot.scaled=VK_NULL_HANDLE;
        slot.image_memory=VK_NULL_HANDLE;slot.buffer=VK_NULL_HANDLE;slot.memory=VK_NULL_HANDLE;
        slot.mapped=nullptr;slot.pending.store(false);
    }
    r.family=UINT32_MAX;
}
uint32_t memory_type(Device& d,uint32_t bits,bool host) {
    uint32_t best=UINT32_MAX;int score=-1;
    for(uint32_t i=0;i<d.memory.memoryTypeCount;i++) {
        auto f=d.memory.memoryTypes[i].propertyFlags;
        if(!(bits&(1u<<i)) || (host && !(f&VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT)))continue;
        // CPU readback needs cached memory. Coherency alone can select extremely
        // slow write-combined VRAM/BAR memory on discrete GPUs.
        int rank=host ? ((f&VK_MEMORY_PROPERTY_HOST_CACHED_BIT)?4:0)+((f&VK_MEMORY_PROPERTY_HOST_COHERENT_BIT)?1:0)
                      : ((f&VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT)?1:0);
        if(rank>score){best=i;score=rank;}
    }
    return best;
}
bool configure_output(Device& d,Swap& s) {
    rosorah::capture_size(s.size.width,s.size.height,s.quality,s.output.width,s.output.height);
    if(s.output.width==s.size.width && s.output.height==s.size.height)return true;
    auto get=(PFN_vkGetPhysicalDeviceFormatProperties)d.instance.gipa(d.instance.handle,"vkGetPhysicalDeviceFormatProperties");
    VkFormatProperties props{};get(d.physical,s.format,&props);
    auto f=props.optimalTilingFeatures;
    s.filter=(f&VK_FORMAT_FEATURE_SAMPLED_IMAGE_FILTER_LINEAR_BIT)?VK_FILTER_LINEAR:VK_FILTER_NEAREST;
    return (f&(VK_FORMAT_FEATURE_BLIT_SRC_BIT|VK_FORMAT_FEATURE_BLIT_DST_BIT))==(VK_FORMAT_FEATURE_BLIT_SRC_BIT|VK_FORMAT_FEATURE_BLIT_DST_BIT);
}
bool prepare(Device& d,Swap& s,uint32_t family) {
    auto& r=s.read;
    if(r.family==family)return true;
    // A different presentation family is uncommon. Tear down before rebuilding.
    if(r.family!=UINT32_MAX){cleanup(d,s);return false;}
    for(auto& slot:r.slots) {
        VkBufferCreateInfo bi{VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO};
        bi.size=VkDeviceSize(s.output.width)*s.output.height*s.color.bytes();bi.usage=VK_BUFFER_USAGE_TRANSFER_DST_BIT;bi.sharingMode=VK_SHARING_MODE_EXCLUSIVE;
        if(d.CreateBuffer(d.handle,&bi,nullptr,&slot.buffer)!=VK_SUCCESS)goto fail;
        VkMemoryRequirements req{};d.GetBufferMemoryRequirements(d.handle,slot.buffer,&req);
        uint32_t index=memory_type(d,req.memoryTypeBits,true);if(index==UINT32_MAX)goto fail;
        slot.coherent=(d.memory.memoryTypes[index].propertyFlags&VK_MEMORY_PROPERTY_HOST_COHERENT_BIT)!=0;
        VkMemoryAllocateInfo ai{VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO};ai.allocationSize=req.size;ai.memoryTypeIndex=index;
        if(d.AllocateMemory(d.handle,&ai,nullptr,&slot.memory)!=VK_SUCCESS || d.BindBufferMemory(d.handle,slot.buffer,slot.memory,0)!=VK_SUCCESS || d.MapMemory(d.handle,slot.memory,0,VK_WHOLE_SIZE,0,&slot.mapped)!=VK_SUCCESS)goto fail;
        if(s.output.width!=s.size.width || s.output.height!=s.size.height) {
            VkImageCreateInfo ii{VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO};ii.imageType=VK_IMAGE_TYPE_2D;ii.format=s.format;ii.extent={s.output.width,s.output.height,1};ii.mipLevels=1;ii.arrayLayers=1;ii.samples=VK_SAMPLE_COUNT_1_BIT;ii.tiling=VK_IMAGE_TILING_OPTIMAL;ii.usage=VK_IMAGE_USAGE_TRANSFER_DST_BIT|VK_IMAGE_USAGE_TRANSFER_SRC_BIT;ii.sharingMode=VK_SHARING_MODE_EXCLUSIVE;
            if(d.CreateImage(d.handle,&ii,nullptr,&slot.scaled)!=VK_SUCCESS)goto fail;
            d.GetImageMemoryRequirements(d.handle,slot.scaled,&req);ai.allocationSize=req.size;ai.memoryTypeIndex=memory_type(d,req.memoryTypeBits,false);
            if(ai.memoryTypeIndex==UINT32_MAX || d.AllocateMemory(d.handle,&ai,nullptr,&slot.image_memory)!=VK_SUCCESS || d.BindImageMemory(d.handle,slot.scaled,slot.image_memory,0)!=VK_SUCCESS)goto fail;
        }
        VkCommandPoolCreateInfo pi{VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO};pi.queueFamilyIndex=family;pi.flags=VK_COMMAND_POOL_CREATE_TRANSIENT_BIT;
        if(d.CreateCommandPool(d.handle,&pi,nullptr,&slot.pool)!=VK_SUCCESS)goto fail;
        VkCommandBufferAllocateInfo ci{VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO};ci.commandPool=slot.pool;ci.level=VK_COMMAND_BUFFER_LEVEL_PRIMARY;ci.commandBufferCount=1;
        if(d.AllocateCommandBuffers(d.handle,&ci,&slot.command)!=VK_SUCCESS || !d.set_data || d.set_data(d.handle,slot.command)!=VK_SUCCESS)goto fail;
        VkFenceCreateInfo fi{VK_STRUCTURE_TYPE_FENCE_CREATE_INFO};if(d.CreateFence(d.handle,&fi,nullptr,&slot.fence)!=VK_SUCCESS)goto fail;
    }
    {
        r.ready.resize(s.images.size());VkSemaphoreCreateInfo si{VK_STRUCTURE_TYPE_SEMAPHORE_CREATE_INFO};
        for(auto& sem:r.ready)if(d.CreateSemaphore(d.handle,&si,nullptr,&sem)!=VK_SUCCESS)goto fail;
    }
    r.family=family;return true;
fail: cleanup(d,s);s.supported=false;return false;
}
bool completed(Device& d,ReadSlot& slot) {
    if(!slot.pending.load(std::memory_order_acquire) || d.GetFenceStatus(d.handle,slot.fence)!=VK_SUCCESS)return false;
    VkMappedMemoryRange range{VK_STRUCTURE_TYPE_MAPPED_MEMORY_RANGE};range.memory=slot.memory;range.size=VK_WHOLE_SIZE;
    return slot.coherent || d.InvalidateMappedMemoryRanges(d.handle,1,&range)==VK_SUCCESS;
}
void start_worker(Device& d,Swap& s) {
    if(s.worker.joinable())return;
    s.stop.store(false,std::memory_order_release);
    s.mailbox.configure(s.output.width,s.output.height,s.color.pixel==rosorah::Pixel::Bgra8,s.color.hdr()?4u:0u);
    s.worker=std::thread([&d,&s] {
        uint64_t delivered=0;rosorah::ColorConverter converter;std::vector<uint8_t> rgba;
        while(!s.stop.load(std::memory_order_acquire)) {
            uint32_t fps=0;
            try {fps=s.mailbox.service();}catch(...){/* A transport failure must not terminate the game. */}
            s.requested_quality.store(s.mailbox.resolution(),std::memory_order_release);
            s.fps.store(fps,std::memory_order_relaxed);
            ReadSlot* latest=nullptr;
            for(auto& slot:s.read.slots)if(completed(d,slot)) {
                if(slot.timestamp<=delivered){slot.pending.store(false,std::memory_order_release);continue;}
                if(!latest || slot.timestamp>latest->timestamp){if(latest)latest->pending.store(false,std::memory_order_release);latest=&slot;}
                else slot.pending.store(false,std::memory_order_release);
            }
            if(latest) {
                delivered=latest->timestamp;
                if(fps)try{
                    const void* pixels=latest->mapped;
                    if(s.color.convert()){converter.convert(pixels,size_t(s.output.width)*s.output.height,s.color,rgba);pixels=rgba.data();}
                    s.mailbox.publish(pixels,latest->timestamp);
                }catch(...){}
                latest->pending.store(false,std::memory_order_release);
            }
            std::this_thread::sleep_for(std::chrono::milliseconds(fps?1:20));
        }
    });
}
// Never wait for a readback slot. Under pressure, skip capture and present normally.
VkSemaphore submit_copy(Device& d,VkQueue queue,Swap& s,uint32_t image,const VkPresentInfoKHR& present,VkImageLayout return_layout) {
    ReadSlot* free=nullptr;
    for(auto& slot:s.read.slots)if(!slot.pending.load(std::memory_order_acquire)){free=&slot;break;}
    if(!free)return VK_NULL_HANDLE;
    auto& r=*free;
    if(d.ResetCommandPool(d.handle,r.pool,0)!=VK_SUCCESS)return VK_NULL_HANDLE;
    VkCommandBufferBeginInfo begin{VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO};begin.flags=VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT;
    if(d.BeginCommandBuffer(r.command,&begin)!=VK_SUCCESS)return VK_NULL_HANDLE;
    VkImageMemoryBarrier barrier{VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER};
    barrier.srcAccessMask=VK_ACCESS_MEMORY_WRITE_BIT;barrier.dstAccessMask=VK_ACCESS_TRANSFER_READ_BIT;barrier.oldLayout=return_layout;barrier.newLayout=VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL;
    barrier.srcQueueFamilyIndex=VK_QUEUE_FAMILY_IGNORED;barrier.dstQueueFamilyIndex=VK_QUEUE_FAMILY_IGNORED;barrier.image=s.images[image];barrier.subresourceRange={VK_IMAGE_ASPECT_COLOR_BIT,0,1,0,1};
    d.CmdPipelineBarrier(r.command,VK_PIPELINE_STAGE_ALL_COMMANDS_BIT,VK_PIPELINE_STAGE_TRANSFER_BIT,0,0,nullptr,0,nullptr,1,&barrier);
    VkImage source=s.images[image];
    if(r.scaled) {
        VkImageMemoryBarrier scale=barrier;scale.image=r.scaled;scale.srcAccessMask=0;scale.dstAccessMask=VK_ACCESS_TRANSFER_WRITE_BIT;scale.oldLayout=VK_IMAGE_LAYOUT_UNDEFINED;scale.newLayout=VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL;
        d.CmdPipelineBarrier(r.command,VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,VK_PIPELINE_STAGE_TRANSFER_BIT,0,0,nullptr,0,nullptr,1,&scale);
        VkImageBlit blit{};blit.srcSubresource={VK_IMAGE_ASPECT_COLOR_BIT,0,0,1};blit.dstSubresource=blit.srcSubresource;blit.srcOffsets[1]={int32_t(s.size.width),int32_t(s.size.height),1};blit.dstOffsets[1]={int32_t(s.output.width),int32_t(s.output.height),1};
        d.CmdBlitImage(r.command,source,VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,r.scaled,VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,1,&blit,s.filter);
        scale.srcAccessMask=VK_ACCESS_TRANSFER_WRITE_BIT;scale.dstAccessMask=VK_ACCESS_TRANSFER_READ_BIT;scale.oldLayout=VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL;scale.newLayout=VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL;
        d.CmdPipelineBarrier(r.command,VK_PIPELINE_STAGE_TRANSFER_BIT,VK_PIPELINE_STAGE_TRANSFER_BIT,0,0,nullptr,0,nullptr,1,&scale);source=r.scaled;
    }
    VkBufferImageCopy region{};region.imageSubresource={VK_IMAGE_ASPECT_COLOR_BIT,0,0,1};region.imageExtent={s.output.width,s.output.height,1};
    d.CmdCopyImageToBuffer(r.command,source,VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,r.buffer,1,&region);
    barrier.srcAccessMask=VK_ACCESS_TRANSFER_READ_BIT;barrier.dstAccessMask=0;barrier.oldLayout=VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL;barrier.newLayout=return_layout;
    VkBufferMemoryBarrier host{VK_STRUCTURE_TYPE_BUFFER_MEMORY_BARRIER};host.srcAccessMask=VK_ACCESS_TRANSFER_WRITE_BIT;host.dstAccessMask=VK_ACCESS_HOST_READ_BIT;host.srcQueueFamilyIndex=VK_QUEUE_FAMILY_IGNORED;host.dstQueueFamilyIndex=VK_QUEUE_FAMILY_IGNORED;host.buffer=r.buffer;host.size=VK_WHOLE_SIZE;
    d.CmdPipelineBarrier(r.command,VK_PIPELINE_STAGE_TRANSFER_BIT,VK_PIPELINE_STAGE_HOST_BIT|VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT,0,0,nullptr,1,&host,1,&barrier);
    if(d.EndCommandBuffer(r.command)!=VK_SUCCESS)return VK_NULL_HANDLE;
    std::vector<VkPipelineStageFlags> stages(present.waitSemaphoreCount,VK_PIPELINE_STAGE_ALL_COMMANDS_BIT);
    VkSemaphore signal=s.read.ready[image];
    VkSubmitInfo submit{VK_STRUCTURE_TYPE_SUBMIT_INFO};submit.waitSemaphoreCount=present.waitSemaphoreCount;submit.pWaitSemaphores=present.pWaitSemaphores;submit.pWaitDstStageMask=stages.data();submit.commandBufferCount=1;submit.pCommandBuffers=&r.command;submit.signalSemaphoreCount=1;submit.pSignalSemaphores=&signal;
    if(d.ResetFences(d.handle,1,&r.fence)!=VK_SUCCESS)return VK_NULL_HANDLE;
    r.timestamp=rosorah::now_ns();
    if(d.QueueSubmit(queue,1,&submit,r.fence)!=VK_SUCCESS)return VK_NULL_HANDLE;
    // Nothing that can throw or fail may occur after consuming the game's waits.
    r.pending.store(true,std::memory_order_release);return signal;
}
VkSemaphore capture(Device& d,VkQueue queue,Swap& s,uint32_t image,const VkPresentInfoKHR& present) {
    auto qi=d.queues.find(queue);
    if(qi==d.queues.end() || qi->second>=d.families.size() || !(d.families[qi->second].queueFlags&VK_QUEUE_GRAPHICS_BIT) || image>=s.images.size())return VK_NULL_HANDLE;
    const uint32_t quality=s.requested_quality.load(std::memory_order_acquire);
    if(quality!=s.quality){d.DeviceWaitIdle(d.handle);cleanup(d,s);s.quality=quality;s.output={};s.next_capture=0;}
    if(!s.output.width && !configure_output(d,s)){s.supported=false;return VK_NULL_HANDLE;}
    start_worker(d,s);
    uint32_t fps=s.fps.load(std::memory_order_relaxed);if(!fps)return VK_NULL_HANDLE;
    const uint64_t now=rosorah::now_ns(), interval=1000000000ull/fps;
    if(now<s.next_capture)return VK_NULL_HANDLE;
    if(!prepare(d,s,qi->second))return VK_NULL_HANDLE;
    auto ready=submit_copy(d,queue,s,image,present,VK_IMAGE_LAYOUT_PRESENT_SRC_KHR);
    if(ready)s.next_capture=(now-s.next_capture>interval)?now+interval:s.next_capture+interval;
    return ready;
}

}

extern "C" {
__attribute__((visibility("default"))) PFN_vkVoidFunction VKAPI_CALL vkGetInstanceProcAddr(VkInstance,const char*);
__attribute__((visibility("default"))) PFN_vkVoidFunction VKAPI_CALL vkGetDeviceProcAddr(VkDevice,const char*);
__attribute__((visibility("default"))) PFN_vkVoidFunction VKAPI_CALL rosorahGetPhysicalDeviceProcAddr(VkInstance,const char*);
__attribute__((visibility("default"))) VkResult VKAPI_CALL vkCreateInstance(const VkInstanceCreateInfo* ci,const VkAllocationCallbacks* a,VkInstance* out) {
    auto* link=(VkLayerInstanceCreateInfo*)ci->pNext;
    while(link && !(link->sType==VK_STRUCTURE_TYPE_LOADER_INSTANCE_CREATE_INFO && link->function==VK_LAYER_LINK_INFO))link=(VkLayerInstanceCreateInfo*)link->pNext;
    if(!link || !link->u.pLayerInfo)return VK_ERROR_INITIALIZATION_FAILED;
    auto gipa=link->u.pLayerInfo->pfnNextGetInstanceProcAddr;auto gpdpa=link->u.pLayerInfo->pfnNextGetPhysicalDeviceProcAddr;
    auto create=(PFN_vkCreateInstance)gipa(VK_NULL_HANDLE,"vkCreateInstance");link->u.pLayerInfo=link->u.pLayerInfo->pNext;
    VkResult result=create(ci,a,out);
    if(result==VK_SUCCESS){std::lock_guard<std::recursive_mutex> lock(gate);instances[key(*out)]={*out,gipa,gpdpa};}
    return result;
}
__attribute__((visibility("default"))) void VKAPI_CALL vkDestroyInstance(VkInstance h,const VkAllocationCallbacks* a) {
    std::lock_guard<std::recursive_mutex> lock(gate);auto it=instances.find(key(h));if(it==instances.end())return;
    auto destroy=(PFN_vkDestroyInstance)it->second.gipa(h,"vkDestroyInstance");instances.erase(it);destroy(h,a);
}
__attribute__((visibility("default"))) VkResult VKAPI_CALL vkCreateDevice(VkPhysicalDevice physical,const VkDeviceCreateInfo* ci,const VkAllocationCallbacks* a,VkDevice* out) {
    Instance instance;
    {std::lock_guard<std::recursive_mutex> lock(gate);auto it=instances.find(key(physical));if(it==instances.end())return VK_ERROR_INITIALIZATION_FAILED;instance=it->second;}
    auto* link=(VkLayerDeviceCreateInfo*)ci->pNext; PFN_vkSetDeviceLoaderData set_data=nullptr;
    for(auto* p=link;p;p=(VkLayerDeviceCreateInfo*)p->pNext)if(p->sType==VK_STRUCTURE_TYPE_LOADER_DEVICE_CREATE_INFO && p->function==VK_LOADER_DATA_CALLBACK)set_data=p->u.pfnSetDeviceLoaderData;
    while(link && !(link->sType==VK_STRUCTURE_TYPE_LOADER_DEVICE_CREATE_INFO && link->function==VK_LAYER_LINK_INFO))link=(VkLayerDeviceCreateInfo*)link->pNext;
    if(!link || !link->u.pLayerInfo)return VK_ERROR_INITIALIZATION_FAILED;
    auto gipa=link->u.pLayerInfo->pfnNextGetInstanceProcAddr;auto gdpa=link->u.pLayerInfo->pfnNextGetDeviceProcAddr;
    auto create=(PFN_vkCreateDevice)gipa(instance.handle,"vkCreateDevice");link->u.pLayerInfo=link->u.pLayerInfo->pNext;
    VkResult result=create(physical,ci,a,out);if(result!=VK_SUCCESS)return result;
    auto d=std::make_shared<Device>();d->handle=*out;d->physical=physical;d->instance=instance;d->gdpa=gdpa;d->set_data=set_data;
#define LOAD(n) d->n=(PFN_vk##n)gdpa(*out,"vk" #n);
    DEVICE_FUNCTIONS(LOAD)
#undef LOAD
    ((PFN_vkGetPhysicalDeviceMemoryProperties)gipa(instance.handle,"vkGetPhysicalDeviceMemoryProperties"))(physical,&d->memory);
    auto props=(PFN_vkGetPhysicalDeviceQueueFamilyProperties)gipa(instance.handle,"vkGetPhysicalDeviceQueueFamilyProperties");uint32_t count=0;props(physical,&count,nullptr);d->families.resize(count);props(physical,&count,d->families.data());
    // Multi-device and protected operation are deliberately excluded from capture.
    bool group=false;for(auto* p=(const VkBaseInStructure*)ci->pNext;p;p=p->pNext)if(p->sType==VK_STRUCTURE_TYPE_DEVICE_GROUP_DEVICE_CREATE_INFO && ((const VkDeviceGroupDeviceCreateInfo*)p)->physicalDeviceCount>1)group=true;
    if(!group)for(uint32_t i=0;i<ci->queueCreateInfoCount;i++)if(ci->pQueueCreateInfos[i].flags==0)for(uint32_t n=0;n<ci->pQueueCreateInfos[i].queueCount;n++){VkQueue q{};d->GetDeviceQueue(*out,ci->pQueueCreateInfos[i].queueFamilyIndex,n,&q);d->queues[q]=ci->pQueueCreateInfos[i].queueFamilyIndex;}
    std::lock_guard<std::recursive_mutex> lock(gate);devices[key(*out)]=d;return result;
}
__attribute__((visibility("default"))) void VKAPI_CALL vkDestroyDevice(VkDevice h,const VkAllocationCallbacks* a) {
    std::lock_guard<std::recursive_mutex> lock(gate);auto d=device(h);if(!d)return;
    d->DeviceWaitIdle(h);for(auto& [id,s]:d->swaps){(void)id;cleanup(*d,*s);}d->swaps.clear();devices.erase(key(h));d->DestroyDevice(h,a);
}
__attribute__((visibility("default"))) VkResult VKAPI_CALL vkCreateSwapchainKHR(VkDevice h,const VkSwapchainCreateInfoKHR* ci,const VkAllocationCallbacks* a,VkSwapchainKHR* out) {
    std::lock_guard<std::recursive_mutex> lock(gate);auto d=device(h);if(!d || !d->CreateSwapchainKHR)return VK_ERROR_INITIALIZATION_FAILED;
    auto copy=*ci;rosorah::Color color;bool supported=!rosorah::self_excluded() && (ci->flags & ~VK_SWAPCHAIN_CREATE_MUTABLE_FORMAT_BIT_KHR)==0 && ci->imageArrayLayers==1 && color_format(ci->imageFormat,ci->imageColorSpace,color) && ci->imageExtent.width>=160 && ci->imageExtent.height>=90 && ci->imageExtent.width<=4096 && ci->imageExtent.height<=4096;
    auto caps=(PFN_vkGetPhysicalDeviceSurfaceCapabilitiesKHR)d->instance.gipa(d->instance.handle,"vkGetPhysicalDeviceSurfaceCapabilitiesKHR");VkSurfaceCapabilitiesKHR surface{};
    supported=supported && caps && caps(d->physical,ci->surface,&surface)==VK_SUCCESS && (surface.supportedUsageFlags&VK_IMAGE_USAGE_TRANSFER_SRC_BIT);
    if(supported)copy.imageUsage|=VK_IMAGE_USAGE_TRANSFER_SRC_BIT;
    VkResult result=d->CreateSwapchainKHR(h,&copy,a,out);
    if(result!=VK_SUCCESS && ci->oldSwapchain==VK_NULL_HANDLE && copy.imageUsage!=ci->imageUsage){supported=false;result=d->CreateSwapchainKHR(h,ci,a,out);}
    if(result==VK_SUCCESS){auto s=std::make_unique<Swap>();s->size=ci->imageExtent;s->format=ci->imageFormat;s->color=color;s->supported=supported;uint32_t n=0;if(d->GetSwapchainImagesKHR(h,*out,&n,nullptr)==VK_SUCCESS){s->images.resize(n);if(d->GetSwapchainImagesKHR(h,*out,&n,s->images.data())!=VK_SUCCESS)s->supported=false;}else s->supported=false;d->swaps[*out]=std::move(s);}
    return result;
}
__attribute__((visibility("default"))) void VKAPI_CALL vkDestroySwapchainKHR(VkDevice h,VkSwapchainKHR swap,const VkAllocationCallbacks* a) {
    std::lock_guard<std::recursive_mutex> lock(gate);auto d=device(h);if(!d || !d->DestroySwapchainKHR)return;
    auto it=d->swaps.find(swap);if(it!=d->swaps.end()){if(it->second->read.family!=UINT32_MAX)d->DeviceWaitIdle(h);cleanup(*d,*it->second);d->swaps.erase(it);}d->DestroySwapchainKHR(h,swap,a);
}
__attribute__((visibility("default"))) VkResult VKAPI_CALL vkQueuePresentKHR(VkQueue q,const VkPresentInfoKHR* info) {
    std::lock_guard<std::recursive_mutex> lock(gate);auto d=device(q);if(!d || !d->QueuePresentKHR)return VK_ERROR_INITIALIZATION_FAILED;
    VkSemaphore ready{};
    for(uint32_t i=0;i<info->swapchainCount;i++){auto it=d->swaps.find(info->pSwapchains[i]);if(it!=d->swaps.end() && it->second->supported){try{ready=capture(*d,q,*it->second,info->pImageIndices[i],*info);}catch(...){ready=VK_NULL_HANDLE;}if(ready)break;}}
    auto copy=*info;if(ready){copy.waitSemaphoreCount=1;copy.pWaitSemaphores=&ready;}
    return d->QueuePresentKHR(q,&copy);
}
#define INTERCEPT(n) if(strcmp(name,"vk" #n)==0)return (PFN_vkVoidFunction)vk##n;
__attribute__((visibility("default"))) PFN_vkVoidFunction VKAPI_CALL vkGetDeviceProcAddr(VkDevice h,const char* name) {
    if(!name)return nullptr;
    INTERCEPT(GetDeviceProcAddr) INTERCEPT(DestroyDevice)
    std::lock_guard<std::recursive_mutex> lock(gate);auto d=device(h);if(!d)return nullptr;
    auto next=d->gdpa(h,name);if(!next)return nullptr;
    INTERCEPT(CreateSwapchainKHR) INTERCEPT(DestroySwapchainKHR) INTERCEPT(QueuePresentKHR)
    return next;
}
__attribute__((visibility("default"))) PFN_vkVoidFunction VKAPI_CALL vkGetInstanceProcAddr(VkInstance h,const char* name) {
    if(!name)return nullptr;
    INTERCEPT(GetInstanceProcAddr) INTERCEPT(GetDeviceProcAddr) INTERCEPT(CreateInstance)
    if(!h)return nullptr;
    INTERCEPT(DestroyInstance) INTERCEPT(CreateDevice)
    INTERCEPT(DestroyDevice) INTERCEPT(CreateSwapchainKHR) INTERCEPT(DestroySwapchainKHR) INTERCEPT(QueuePresentKHR)
    std::lock_guard<std::recursive_mutex> lock(gate);auto it=instances.find(key(h));return it==instances.end()?nullptr:it->second.gipa(h,name);
}
__attribute__((visibility("default"))) PFN_vkVoidFunction VKAPI_CALL rosorahGetPhysicalDeviceProcAddr(VkInstance h,const char* name) {
    std::lock_guard<std::recursive_mutex> lock(gate);auto it=instances.find(key(h));if(it==instances.end())return nullptr;
    return it->second.gpdpa?it->second.gpdpa(h,name):it->second.gipa(h,name);
}
__attribute__((visibility("default"))) VkResult VKAPI_CALL vkNegotiateLoaderLayerInterfaceVersion(VkNegotiateLayerInterface* info) {
    if(!info || info->loaderLayerInterfaceVersion<2)return VK_ERROR_INITIALIZATION_FAILED;
    info->loaderLayerInterfaceVersion=2;info->pfnGetInstanceProcAddr=vkGetInstanceProcAddr;info->pfnGetDeviceProcAddr=vkGetDeviceProcAddr;info->pfnGetPhysicalDeviceProcAddr=rosorahGetPhysicalDeviceProcAddr;return VK_SUCCESS;
}
}

#ifdef ROSORAH_TESTING
// Exercise the actual GPU commands independently of the socket integration test.
extern "C" __attribute__((visibility("default"))) int rosorah_test_readback(VkDevice h,VkQueue q,VkImage image,VkSemaphore original,uint32_t mode,VkFormat format,VkColorSpaceKHR space,unsigned char* pixel) {
    std::lock_guard<std::recursive_mutex> lock(gate);auto d=device(h);if(!d)return -1;
    const uint32_t quality=mode>>8;mode&=255;
    Swap s;s.quality=quality;s.requested_quality.store(quality);s.size={3840,2160};s.format=format;if(!color_format(format,space,s.color))return -11;s.supported=true;s.images={image};
    VkPresentInfoKHR info{VK_STRUCTURE_TYPE_PRESENT_INFO_KHR};info.waitSemaphoreCount=1;info.pWaitSemaphores=&original;
    const unsigned widths[]={1280,1920,2560,3840,3840};const unsigned heights[]={720,1080,1440,2160,2160};
    if(quality>4 || !configure_output(*d,s) || s.output.width!=widths[quality] || s.output.height!=heights[quality] || !prepare(*d,s,d->queues[q]))return -2;
    auto wait=d->WaitForFences;static bool waited=false;waited=false;
    d->WaitForFences=[](VkDevice,uint32_t,const VkFence*,VkBool32,uint64_t)->VkResult{waited=true;return VK_TIMEOUT;};
    // Keep the worker handle present with a zero activation rate, so the
    // actual capture entry point can be tested without a live desktop receiver.
    if(mode==1)s.worker=std::thread([&s]{while(!s.stop.load())std::this_thread::sleep_for(std::chrono::milliseconds(1));});
    VkSemaphore replacement=mode==1 ? capture(*d,q,s,0,info) : submit_copy(*d,q,s,0,info,VK_IMAGE_LAYOUT_GENERAL);
    if(mode==1 && replacement){cleanup(*d,s);return -10;}
    d->WaitForFences=wait;int result=waited?-6:0;
    if(mode!=1 && !replacement)result=-3;
    if(mode==2) {
        auto status=d->GetFenceStatus;d->GetFenceStatus=[](VkDevice,VkFence)->VkResult{return VK_NOT_READY;};
        if(completed(*d,s.read.slots[0]) || !s.read.slots[0].pending.load())result=-7;
        d->GetFenceStatus=status;
        // Both busy slots must skip submission, preserving the app's waits.
        s.read.slots[1].pending.store(true);
        if(submit_copy(*d,q,s,0,info,VK_IMAGE_LAYOUT_GENERAL))result=-9;
        s.read.slots[1].pending.store(false);
    }
    VkSemaphore consume=replacement?replacement:original;VkPipelineStageFlags stage=VK_PIPELINE_STAGE_ALL_COMMANDS_BIT;
    VkSubmitInfo submit{VK_STRUCTURE_TYPE_SUBMIT_INFO};submit.waitSemaphoreCount=1;submit.pWaitSemaphores=&consume;submit.pWaitDstStageMask=&stage;
    if(d->QueueSubmit(q,1,&submit,VK_NULL_HANDLE)!=VK_SUCCESS || d->DeviceWaitIdle(h)!=VK_SUCCESS)result=-4;
    if(mode!=1) {
        auto& slot=s.read.slots[0];
        if(!completed(*d,slot))result=-8;else {
            if(s.color.convert()){rosorah::ColorConverter converter;std::vector<uint8_t> rgba;converter.convert(slot.mapped,1,s.color,rgba);memcpy(pixel,rgba.data(),4);}
            else memcpy(pixel,slot.mapped,4);
        }
        slot.pending.store(false);
    }
    cleanup(*d,s);return result;
}
#endif
