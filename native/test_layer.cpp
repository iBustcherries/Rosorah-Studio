#include <vulkan/vulkan.h>
#include "transport.hpp"
#include <dlfcn.h>
#include <cstdio>
#include <cassert>
#include <vector>
#define CHECK(call) do{ VkResult result=(call); if(result!=VK_SUCCESS){fprintf(stderr,"%s: %d\n",#call,result);return 1;} }while(0)
int main(int argc,char** argv) {
    assert(argc>=2);
    VkFormat format=VK_FORMAT_R8G8B8A8_UNORM;VkColorSpaceKHR space=VK_COLOR_SPACE_SRGB_NONLINEAR_KHR;
    const char* kind=argc>2?argv[2]:"sdr";
    if(strcmp(kind,"sdr10")==0)format=VK_FORMAT_A2B10G10R10_UNORM_PACK32;
    if(strcmp(kind,"pq")==0){format=VK_FORMAT_A2B10G10R10_UNORM_PACK32;space=VK_COLOR_SPACE_HDR10_ST2084_EXT;}
    if(strcmp(kind,"pq-bgr")==0){format=VK_FORMAT_A2R10G10B10_UNORM_PACK32;space=VK_COLOR_SPACE_HDR10_ST2084_EXT;}
    if(strcmp(kind,"hlg")==0){format=VK_FORMAT_A2B10G10R10_UNORM_PACK32;space=VK_COLOR_SPACE_HDR10_HLG_EXT;}
    if(strcmp(kind,"scrgb")==0){format=VK_FORMAT_R16G16B16A16_SFLOAT;space=VK_COLOR_SPACE_EXTENDED_SRGB_LINEAR_EXT;}
    void* loader=dlopen("libvulkan.so.1",RTLD_NOW|RTLD_LOCAL);assert(loader);
    auto gipa=(PFN_vkGetInstanceProcAddr)dlsym(loader,"vkGetInstanceProcAddr");assert(gipa);
    auto create=(PFN_vkCreateInstance)gipa(nullptr,"vkCreateInstance");
    VkApplicationInfo app{VK_STRUCTURE_TYPE_APPLICATION_INFO};app.pApplicationName="Rosorah layer validation";app.apiVersion=VK_API_VERSION_1_1;
    const char* layers[]={"VK_LAYER_ROSORAH_capture","VK_LAYER_KHRONOS_validation"};
    VkInstanceCreateInfo ci{VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO};ci.pApplicationInfo=&app;ci.enabledLayerCount=getenv("ROSORAH_IMPLICIT_TEST")?0:(getenv("ROSORAH_VALIDATE")?2:1);ci.ppEnabledLayerNames=layers;
    VkInstance instance{};CHECK(create(&ci,nullptr,&instance));
#define INSTANCE(n) auto n=(PFN_vk##n)gipa(instance,"vk" #n);assert(n)
    INSTANCE(GetPhysicalDeviceFormatProperties);INSTANCE(EnumeratePhysicalDevices);INSTANCE(GetPhysicalDeviceQueueFamilyProperties);INSTANCE(GetPhysicalDeviceMemoryProperties);INSTANCE(CreateDevice);INSTANCE(DestroyInstance);
    uint32_t count=0;CHECK(EnumeratePhysicalDevices(instance,&count,nullptr));assert(count);
    std::vector<VkPhysicalDevice> physical(count);CHECK(EnumeratePhysicalDevices(instance,&count,physical.data()));
    VkFormatProperties format_props{};GetPhysicalDeviceFormatProperties(physical[0],format,&format_props);
    VkFormatFeatureFlags blit=VK_FORMAT_FEATURE_BLIT_SRC_BIT|VK_FORMAT_FEATURE_BLIT_DST_BIT;
    if((format_props.optimalTilingFeatures&blit)!=blit){printf("SKIPPED: software driver does not support blitting format %s (%u)\n",kind,format);DestroyInstance(instance,nullptr);return 77;}
    uint32_t families=0;GetPhysicalDeviceQueueFamilyProperties(physical[0],&families,nullptr);std::vector<VkQueueFamilyProperties> props(families);GetPhysicalDeviceQueueFamilyProperties(physical[0],&families,props.data());
    uint32_t family=0;while(family<families && !(props[family].queueFlags&VK_QUEUE_GRAPHICS_BIT))family++;assert(family<families);
    float priority=1;VkDeviceQueueCreateInfo qi{VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO};qi.queueFamilyIndex=family;qi.queueCount=1;qi.pQueuePriorities=&priority;
    VkDeviceCreateInfo di{VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO};di.queueCreateInfoCount=1;di.pQueueCreateInfos=&qi;
    VkDevice device{};CHECK(CreateDevice(physical[0],&di,nullptr,&device));
    auto gdpa=(PFN_vkGetDeviceProcAddr)gipa(instance,"vkGetDeviceProcAddr");assert(gdpa);
#define DEVICE(n) auto n=(PFN_vk##n)gdpa(device,"vk" #n);assert(n)
    DEVICE(GetDeviceQueue);DEVICE(CreateImage);DEVICE(GetImageMemoryRequirements);DEVICE(AllocateMemory);DEVICE(BindImageMemory);DEVICE(CreateCommandPool);DEVICE(AllocateCommandBuffers);DEVICE(BeginCommandBuffer);DEVICE(EndCommandBuffer);DEVICE(CmdPipelineBarrier);DEVICE(CmdClearColorImage);DEVICE(CreateSemaphore);DEVICE(QueueSubmit);DEVICE(DeviceWaitIdle);DEVICE(DestroySemaphore);DEVICE(DestroyCommandPool);DEVICE(DestroyImage);DEVICE(FreeMemory);DEVICE(DestroyDevice);
    VkQueue queue{};GetDeviceQueue(device,family,0,&queue);
    VkImageCreateInfo ii{VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO};ii.imageType=VK_IMAGE_TYPE_2D;ii.format=format;ii.extent={3840,2160,1};ii.mipLevels=1;ii.arrayLayers=1;ii.samples=VK_SAMPLE_COUNT_1_BIT;ii.tiling=VK_IMAGE_TILING_OPTIMAL;ii.usage=VK_IMAGE_USAGE_TRANSFER_SRC_BIT|VK_IMAGE_USAGE_TRANSFER_DST_BIT;ii.sharingMode=VK_SHARING_MODE_EXCLUSIVE;
    VkImage image{};CHECK(CreateImage(device,&ii,nullptr,&image));VkMemoryRequirements req{};GetImageMemoryRequirements(device,image,&req);VkPhysicalDeviceMemoryProperties memory{};GetPhysicalDeviceMemoryProperties(physical[0],&memory);
    uint32_t index=0;while(index<memory.memoryTypeCount && !(req.memoryTypeBits&(1u<<index)))index++;assert(index<memory.memoryTypeCount);
    VkMemoryAllocateInfo ai{VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO};ai.allocationSize=req.size;ai.memoryTypeIndex=index;VkDeviceMemory allocation{};CHECK(AllocateMemory(device,&ai,nullptr,&allocation));CHECK(BindImageMemory(device,image,allocation,0));
    VkCommandPoolCreateInfo pi{VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO};pi.queueFamilyIndex=family;VkCommandPool pool{};CHECK(CreateCommandPool(device,&pi,nullptr,&pool));
    VkCommandBufferAllocateInfo bi{VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO};bi.commandPool=pool;bi.level=VK_COMMAND_BUFFER_LEVEL_PRIMARY;bi.commandBufferCount=1;VkCommandBuffer command{};CHECK(AllocateCommandBuffers(device,&bi,&command));
    VkCommandBufferBeginInfo begin{VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO};CHECK(BeginCommandBuffer(command,&begin));
    VkImageMemoryBarrier barrier{VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER};barrier.oldLayout=VK_IMAGE_LAYOUT_UNDEFINED;barrier.newLayout=VK_IMAGE_LAYOUT_GENERAL;barrier.srcQueueFamilyIndex=VK_QUEUE_FAMILY_IGNORED;barrier.dstQueueFamilyIndex=VK_QUEUE_FAMILY_IGNORED;barrier.dstAccessMask=VK_ACCESS_TRANSFER_WRITE_BIT;barrier.image=image;barrier.subresourceRange={VK_IMAGE_ASPECT_COLOR_BIT,0,1,0,1};
    CmdPipelineBarrier(command,VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,VK_PIPELINE_STAGE_TRANSFER_BIT,0,0,nullptr,0,nullptr,1,&barrier);VkClearColorValue color{{1.f,.25f,.5f,1.f}};
    if(space==VK_COLOR_SPACE_HDR10_ST2084_EXT)color={{.508078f,.508078f,.508078f,1.f}};
    if(space==VK_COLOR_SPACE_HDR10_HLG_EXT)color={{.5f,.5f,.5f,1.f}};
    CmdClearColorImage(command,image,VK_IMAGE_LAYOUT_GENERAL,&color,1,&barrier.subresourceRange);CHECK(EndCommandBuffer(command));
    void* library=dlopen(argv[1],RTLD_NOW|RTLD_NOLOAD);assert(library);
    using Test=int(*)(VkDevice,VkQueue,VkImage,VkSemaphore,uint32_t,VkFormat,VkColorSpaceKHR,unsigned char*);
    auto test=(Test)dlsym(library,"rosorah_test_readback");assert(test);
    for(uint32_t quality=0;quality<5;quality++)for(uint32_t mode: {0u,1u,2u}) {
        VkSemaphoreCreateInfo si{VK_STRUCTURE_TYPE_SEMAPHORE_CREATE_INFO};VkSemaphore semaphore{};CHECK(CreateSemaphore(device,&si,nullptr,&semaphore));
        VkSubmitInfo submit{VK_STRUCTURE_TYPE_SUBMIT_INFO};if(mode==0){submit.commandBufferCount=1;submit.pCommandBuffers=&command;}submit.signalSemaphoreCount=1;submit.pSignalSemaphores=&semaphore;CHECK(QueueSubmit(queue,1,&submit,VK_NULL_HANDLE));
        unsigned char pixel[4]{};int result=test(device,queue,image,semaphore,mode|(quality<<8),format,space,pixel);if(result){fprintf(stderr,"capture test mode %u failed %d\n",mode,result);return 1;}
        if(mode==0){printf("Readback pixel: %u %u %u %u\n",pixel[0],pixel[1],pixel[2],pixel[3]);assert(pixel[3]==255);
            if(space==VK_COLOR_SPACE_SRGB_NONLINEAR_KHR)assert(pixel[0]==255 && pixel[1]==64 && pixel[2]==128);
            if(space==VK_COLOR_SPACE_HDR10_ST2084_EXT)assert(pixel[0]>=185 && pixel[0]<=187 && pixel[1]==pixel[0] && pixel[2]==pixel[0]);
            if(space==VK_COLOR_SPACE_HDR10_HLG_EXT)assert(pixel[0]>=137 && pixel[0]<=141 && pixel[1]==pixel[0] && pixel[2]==pixel[0]);
            if(space==VK_COLOR_SPACE_EXTENDED_SRGB_LINEAR_EXT)assert(pixel[0]>=168 && pixel[0]<=169 && pixel[1]>=88 && pixel[1]<=89 && pixel[2]>=122 && pixel[2]<=124);}
        DestroySemaphore(device,semaphore,nullptr);
    }
    CHECK(DeviceWaitIdle(device));DestroyCommandPool(device,pool,nullptr);DestroyImage(device,image,nullptr);FreeMemory(device,allocation,nullptr);DestroyDevice(device,nullptr);DestroyInstance(instance,nullptr);
    puts("PASS: real loader chain, Vulkan GPU image readback, inactive passthrough, nonblocking pending-copy handling, resource teardown");
}
