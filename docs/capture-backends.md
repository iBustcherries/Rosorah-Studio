# Capture backends in 0.8.0

Automatic: Vulkan implicit layer or GLX/EGL swap interception → bounded GPU downscale/readback → background delivery → anonymous memfd → event-driven Rust receiver → bounded capture engine → shared-device GPU canvas.

Manual: user-invoked desktop portal → PipeWire → GStreamer → capture engine → the same GPU canvas. No automatic path invokes KDE or the desktop portal. Windows is deferred.

## Native synchronization

The hook adds TRANSFER_SRC to supported swapchains. Capture waits on the application's presentation semaphores, transitions the image, scales on the GPU when needed, copies the selected capture resolution into a persistently mapped buffer, restores the present layout and signals one replacement semaphore per swapchain image. The game's original waits are consumed exactly once. After a successful submission, the replacement semaphore is always forwarded to presentation.

Two readback slots have independently owned command pools, fences, memory and optional scaled images. Atomic release/acquire transfers ownership between the presentation thread and a delivery thread. The worker polls fences at 1 ms while active, invalidates non-coherent mapped memory as necessary, publishes the newest completed timestamp and releases slots. It never submits to the application's queue. The presentation thread skips capture if both slots are busy. It never waits for a fence or sends/copies frame payloads. Swapchain/device teardown joins the delivery worker and waits for GPU completion before releasing resources.

Host-visible memory selection prefers HOST_CACHED over HOST_COHERENT. Buffer memory is mapped once. The output fits the selected 720p/1080p/1440p/2160p box, or the original game size (up to 4096 per axis), with aspect ratio preserved and no upscaling. Unsupported blit formats at larger dimensions are skipped rather than falling back to full-size CPU sampling.

## OpenGL and color

The default 0.8.0 activation route is the experimental Steam launch observer in `native/launch_watch.cpp`. It catches exec events, inserts an architecture-dispatched preload into a new game process, and detaches. An optional per-user KDE session script provides the old login-based fallback. The installer supplies Fedora and Debian/Steam `$LIB` layouts and separate 32-bit/64-bit Vulkan manifests. It never writes `/etc/ld.so.preload`, changes Steam launch options or relaunches Steam.

The preload entry is a small C library with no C++ runtime dependency. It uses older versioned libdl symbols and loads `librosorah-gl-capture.so` lazily with RTLD_LOCAL. Missing/incompatible capture backend dependencies leave the application on its original graphics calls.

GLX/EGL interception includes regular and damage-based swap calls, API function-pointer queries and explicit dlsym lookups used by engines loading graphics libraries locally. Passthrough dlsym uses a tail call to preserve RTLD_NEXT's original caller scope; release builds enforce optimization and native tests check composition with other preload libraries. Desktop GL 3.2 or ES 3.0 is required. The hook restores framebuffer bindings, texture binding, pack/unpack PBO bindings, pixel pack settings, scissor and framebuffer sRGB state. It resolves MSAA before downscaling. Fences are polled with a zero timeout; completed PBOs are mapped on the owning GL context and copied into bounded delivery buffers. A worker flips rows and converts color. Context/surface teardown retires capture connections; GPU deletion requires the owning context to be current, otherwise driver context teardown owns cleanup. Shared contexts destroyed while non-current can retain object storage until the share group ends.

HDR conversion occurs after the bounded GPU copy. PQ uses the ST2084 inverse transfer and BT.2020-to-BT.709 matrix; HLG uses a 1000-nit reference display with luminance-based system gamma 1.2. Linear scRGB uses 80 nits per unit. The fixed SDR reference is 203 nits, with a soft highlight knee and clipped out-of-gamut colors. PQ and output gamma use lookup tables. This is an approximate SDR preview conversion, not an HDR mastering pipeline. Unknown Vulkan color spaces are skipped. The game render target and display metadata are not modified.

## Connection and protocol

The receiver owns a private 0700 `~/.rosorah-capture` directory, an exclusive owner lock and a Unix SOCK_SEQPACKET listener. The socket is only a rendezvous; pixels never go into a regular home-directory file. Native Steam shares the home directory in its normal container setup, removing reliance on mounting `$XDG_RUNTIME_DIR/rosorah-renderer` into individual games.

A background hook thread connects and validates SO_PEERCRED's UID. It creates a 0600 anonymous memfd, fixes its size to 112 + 4096 × 4096 × 4 bytes, seals resizing/seal changes, maps it and passes its descriptor with a version-3 header via SCM_RIGHTS. The receiver validates the peer UID, exact capacity, anonymity, permissions, seals and metadata, and reopens `/proc/self/fd/N` read-only. Reopening is essential: SCM_RIGHTS alone shares the producer's open-file description and flock owner.

The producer and receiver use nonblocking exclusive/shared locks on independent file descriptions. Contention drops a capture or retains the last complete frame. The writer commits a complete header and pixel payload while locked, unlocks, and sends a small readiness notification. Notifications are coalesced; the receiver reads only the latest sequence. A slow consumer cannot accumulate full frames. Invalid packets/ancillary data close the client, with all received descriptors closed.

Header: little-endian u32 magic/version/width/height/format/pid/flags/reserved; u64 sequence/capture timestamp; 64-byte process name. Size 112. Version 2, magic 0x52535246. Format 1 RGBA or 2 BGRA. Flags: bit 0 Steam, bit 1 OpenGL, bit 2 HDR converted to SDR, bit 3 32-bit producer. Both architectures preserve the same 112-byte ABI. A 16-byte control/notification consists of magic, version, kind and value. Kind 1 selects rate 0/30/60; kind 2 notifies new pixels. Only the selected source receives a nonzero rate.

The receiver pins its source, can promote a Steam application over a non-Steam application, and holds the last image for two seconds when no connection remains. A replacement swapchain can reconnect sooner. Multiple automatic sources in Rosorah share the listener and latest frame; a second Rosorah process cannot own it simultaneously.

## Preview

The compositor reuses eframe's GPU device and queue. `render_gpu` submits composition and returns a view; egui samples it directly. The target remains sRGB for correct blending, with an Unorm view exposing gamma-encoded texels to egui. Staging buffers and blocking readback are allocated only by explicit snapshot/export `render` calls. New capture frames request an egui repaint immediately, with periodic polling retained for status and retry handling.

## Limits and references

This is readback plus CPU memory transport, not zero-copy/DMA-BUF. Actual AMD game performance, WSI presentation, and Steam container IPC were not exercised in the build sandbox. The automatic receiver does not inspect fullscreen/focus state. Vulkan and OpenGL hook paths are built for x86-64 and i386. See the README for API/format requirements. GLX window-system presentation, real game MSAA and real HDR output remain desktop validation items.

- https://github.com/KhronosGroup/Vulkan-Loader/blob/main/docs/LoaderLayerInterface.md
- https://docs.vulkan.org/guide/latest/swapchain_semaphore_reuse.html
- https://docs.vulkan.org/spec/latest/chapters/memory.html
- https://github.com/ValveSoftware/steam-runtime/blob/master/doc/steamlinuxruntime-known-issues.md
- https://man7.org/linux/man-pages/man2/memfd_create.2.html

- https://userbase.kde.org/Session_Environment_Variables
- https://man7.org/linux/man-pages/man8/ld.so.8.html
- https://docs.vulkan.org/refpages/latest/refpages/source/VkColorSpaceKHR.html

## Resolution changes (0.13.0)

Control kind 3 carries a quality index 0–4; kind 1 retains 0/30/60 FPS. Resolution control precedes activation. Anonymous memory has a fixed 64 MiB maximum pixel capacity; only written pages consume physical memory. The selected dimensions determine the actual pixel copy and GPU/CPU slot allocations. The receiver retains exact-size, seal, ownership and dimension validation.

Background threads publish requested quality atomically. Vulkan performs a one-time device-idle/worker join on quality changes before destroying/reallocating readback resources; normal capture remains nonblocking. OpenGL rebuilds its buffers on the owning context and recreates delivery with the requested quality, avoiding reset loops. Neither path adds a game-frame-rate cap. Old protocol-2 hooks require one game restart after upgrading.
