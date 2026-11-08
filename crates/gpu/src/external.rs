//! Pictures another process left on the graphics card, imported into this device once per set
//! of slots and drawn into a source's texture, so a game frame never goes through main memory.
//! On Linux they're Vulkan memory from Rosorah's game layer, passed as opaque file descriptors;
//! opaque memory only works between the same GPU and driver, so both identities must match.
//! On Windows they're Direct3D 11 textures from Windows Graphics Capture, shared as NT handles,
//! which Direct3D 12 and Vulkan can open on the same graphics adapter.
use anyhow::{Context, Result, bail, ensure};
use ash::vk;
#[cfg(target_os = "linux")]
use std::os::fd::{FromRawFd, IntoRawFd, OwnedFd};
#[cfg(windows)]
use std::os::windows::io::{AsRawHandle, OwnedHandle};
use std::sync::Mutex;

/// A GPU and driver, by their Vulkan UUIDs.
#[cfg(target_os = "linux")]
pub type Identity = ([u8; 16], [u8; 16]);
/// A graphics adapter, by its locally unique identifier (LUID).
#[cfg(windows)]
pub type Identity = [u8; 8];

/// Every compositor device made in this process, and whether it can import shared memory.
static DEVICES: Mutex<Vec<Option<Identity>>> = Mutex::new(Vec::new());

/// Whether every compositor here can draw pictures shared from this GPU and driver.
pub fn accepts(identity: Identity) -> bool {
    let devices = DEVICES.lock().unwrap();
    !devices.is_empty() && devices.iter().all(|d| *d == Some(identity))
}

/// Whether a game could share with every compositor here: they all can import, and all use
/// one GPU and driver (the game's must then match too, checked when it offers).
pub fn sharing_possible() -> bool {
    let devices = DEVICES.lock().unwrap();
    devices.first().is_some_and(|first| first.is_some() && devices.iter().all(|d| d == first))
}

/// The adapter every compositor here draws with, when pictures shared from it can be drawn.
#[cfg(windows)]
pub fn adapter() -> Option<Identity> {
    sharing_possible().then(|| DEVICES.lock().unwrap().first().copied().flatten()).flatten()
}

/// Stops sharing for good in this run (an import failed): games go back to sharing through memory.
pub fn give_up() {
    DEVICES.lock().unwrap().push(None);
}

/// Notes a compositor's device: its identity, or None when it can't import.
pub(crate) fn register(device: &wgpu::Device) {
    let identity = identity(device);
    let mut devices = DEVICES.lock().unwrap();
    if !devices.contains(&identity) {
        devices.push(identity);
    }
}

#[cfg(target_os = "linux")]
fn identity(device: &wgpu::Device) -> Option<Identity> {
    let hal = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }?;
    if !hal.enabled_device_extensions().contains(&ash::khr::external_memory_fd::NAME) {
        return None;
    }
    let instance = hal.shared_instance();
    if instance.instance_api_version() < vk::API_VERSION_1_1 {
        return None;
    }
    let mut id = vk::PhysicalDeviceIDProperties::default();
    let mut properties = vk::PhysicalDeviceProperties2::default().push_next(&mut id);
    unsafe { instance.raw_instance().get_physical_device_properties2(hal.raw_physical_device(), &mut properties) };
    Some((id.device_uuid, id.driver_uuid))
}

#[cfg(windows)]
fn identity(device: &wgpu::Device) -> Option<Identity> {
    if let Some(hal) = unsafe { device.as_hal::<wgpu::hal::api::Dx12>() } {
        let luid = unsafe { hal.raw_device().GetAdapterLuid() };
        let mut identity = [0; 8];
        identity[..4].copy_from_slice(&luid.LowPart.to_le_bytes());
        identity[4..].copy_from_slice(&luid.HighPart.to_le_bytes());
        return Some(identity);
    }
    let hal = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }?;
    if !hal.enabled_device_extensions().contains(&ash::khr::external_memory_win32::NAME) {
        return None;
    }
    let instance = hal.shared_instance();
    if instance.instance_api_version() < vk::API_VERSION_1_1 {
        return None;
    }
    let mut id = vk::PhysicalDeviceIDProperties::default();
    let mut properties = vk::PhysicalDeviceProperties2::default().push_next(&mut id);
    unsafe { instance.raw_instance().get_physical_device_properties2(hal.raw_physical_device(), &mut properties) };
    (id.device_luid_valid == vk::TRUE).then_some(id.device_luid)
}

/// One shared picture to draw: which set of slots, which slot, and how to import them.
pub struct SharedFrame<'a> {
    pub export: u64,
    pub slot: usize,
    pub width: u32,
    pub height: u32,
    pub vk_format: i32,
    /// Each slot's allocation size (Linux only).
    pub sizes: &'a [u64],
    #[cfg(target_os = "linux")]
    pub memory: &'a [OwnedFd],
    #[cfg(windows)]
    pub memory: &'a [OwnedHandle],
}

/// A set of slots imported into this device, as textures to draw from.
pub(crate) struct Imported {
    pub export: u64,
    pub slots: Vec<wgpu::BindGroup>,
    /// Windows: the handles, kept open while the device uses what they share.
    #[cfg(windows)]
    _handles: Vec<OwnedHandle>,
}

fn format(vk_format: i32) -> Result<wgpu::TextureFormat> {
    Ok(match vk::Format::from_raw(vk_format) {
        vk::Format::B8G8R8A8_UNORM => wgpu::TextureFormat::Bgra8Unorm,
        vk::Format::R8G8B8A8_UNORM => wgpu::TextureFormat::Rgba8Unorm,
        vk::Format::A2B10G10R10_UNORM_PACK32 => wgpu::TextureFormat::Rgb10a2Unorm,
        other => bail!("Shared pictures in {other:?} aren't supported"),
    })
}

/// Imports every slot of a shared set. The images are made exactly as the game layer made
/// them (same format, size, tiling and usage), as opaque memory requires.
pub(crate) fn import(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, frame: &SharedFrame) -> Result<Imported> {
    ensure!(
        frame.width > 0 && frame.height > 0 && frame.width <= 4096 && frame.height <= 4096 && !frame.memory.is_empty() && frame.memory.len() <= 8,
        "Invalid shared picture"
    );
    #[cfg(target_os = "linux")]
    ensure!(frame.sizes.len() == frame.memory.len(), "Invalid shared picture");
    let texture_format = format(frame.vk_format)?;
    let mut slots = Vec::new();
    for (i, memory) in frame.memory.iter().enumerate() {
        #[cfg(target_os = "linux")]
        let texture = unsafe { import_one(device, frame, texture_format, frame.sizes[i], memory)? };
        #[cfg(windows)]
        let texture = {
            let _ = i;
            unsafe { import_handle(device, frame, texture_format, memory)? }
        };
        let view = texture.create_view(&Default::default());
        slots.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Shared picture"),
            layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }],
        }));
    }
    Ok(Imported {
        export: frame.export,
        slots,
        #[cfg(windows)]
        _handles: frame.memory.iter().map(OwnedHandle::try_clone).collect::<std::io::Result<_>>()?,
    })
}

#[cfg(target_os = "linux")]
unsafe fn import_one(device: &wgpu::Device, frame: &SharedFrame, texture_format: wgpu::TextureFormat, size: u64, memory: &OwnedFd) -> Result<wgpu::Texture> {
    let hal = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }.context("Shared pictures need Vulkan")?;
    let raw = hal.raw_device().clone();
    let instance = hal.shared_instance().raw_instance();
    let mut external = vk::ExternalMemoryImageCreateInfo::default().handle_types(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);
    let info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(vk::Format::from_raw(frame.vk_format))
        .extent(vk::Extent3D { width: frame.width, height: frame.height, depth: 1 })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .push_next(&mut external);
    let image = unsafe { raw.create_image(&info, None) }?;
    let requirements = unsafe { raw.get_image_memory_requirements(image) };
    let types = unsafe { instance.get_physical_device_memory_properties(hal.raw_physical_device()) };
    let index = (0..types.memory_type_count).find(|&i| {
        requirements.memory_type_bits & (1 << i) != 0 && types.memory_types[i as usize].property_flags.contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
    });
    let Some(index) = index.filter(|_| requirements.size <= size) else {
        unsafe { raw.destroy_image(image, None) };
        bail!("The shared picture doesn't fit this device's memory");
    };
    // Vulkan takes the descriptor over when the import succeeds, so it gets its own copy.
    let fd = memory.try_clone()?.into_raw_fd();
    let mut import = vk::ImportMemoryFdInfoKHR::default().handle_type(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD).fd(fd);
    let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
    let allocate = vk::MemoryAllocateInfo::default().allocation_size(size).memory_type_index(index).push_next(&mut import).push_next(&mut dedicated);
    let memory = match unsafe { raw.allocate_memory(&allocate, None) } {
        Ok(m) => m,
        Err(e) => {
            unsafe {
                drop(OwnedFd::from_raw_fd(fd));
                raw.destroy_image(image, None);
            }
            bail!("Couldn't import the shared picture: {e}");
        }
    };
    if let Err(e) = unsafe { raw.bind_image_memory(image, memory, 0) } {
        unsafe {
            raw.free_memory(memory, None);
            raw.destroy_image(image, None);
        }
        bail!("Couldn't use the shared picture: {e}");
    }
    let extent = wgpu::Extent3d { width: frame.width, height: frame.height, depth_or_array_layers: 1 };
    let desc = wgpu::hal::TextureDescriptor {
        label: Some("Shared picture"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: texture_format,
        usage: wgpu::TextureUses::RESOURCE | wgpu::TextureUses::COPY_DST,
        memory_flags: wgpu::hal::MemoryFlags::empty(),
        view_formats: Vec::new(),
    };
    let release: wgpu::hal::DropCallback = Box::new(move || unsafe {
        raw.destroy_image(image, None);
        raw.free_memory(memory, None);
    });
    let texture = unsafe { hal.texture_from_raw(image, &desc, Some(release)) };
    drop(hal);
    Ok(unsafe {
        device.create_texture_from_hal::<wgpu::hal::api::Vulkan>(
            texture,
            &wgpu::TextureDescriptor {
                label: Some("Shared picture"),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: texture_format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
        )
    })
}

/// Opens one shared Direct3D 11 texture on this device: through Direct3D 12 directly, or as
/// Vulkan memory imported from the handle.
#[cfg(windows)]
unsafe fn import_handle(device: &wgpu::Device, frame: &SharedFrame, texture_format: wgpu::TextureFormat, handle: &OwnedHandle) -> Result<wgpu::Texture> {
    use windows::Win32::{Foundation::HANDLE, Graphics::Direct3D12::ID3D12Resource};
    let extent = wgpu::Extent3d { width: frame.width, height: frame.height, depth_or_array_layers: 1 };
    let descriptor = wgpu::TextureDescriptor {
        label: Some("Shared picture"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: texture_format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    };
    if let Some(hal) = unsafe { device.as_hal::<wgpu::hal::api::Dx12>() } {
        let mut resource: Option<ID3D12Resource> = None;
        unsafe { hal.raw_device().OpenSharedHandle(HANDLE(handle.as_raw_handle()), &mut resource) }.context("Couldn't open the shared picture")?;
        let resource = resource.context("Couldn't open the shared picture")?;
        drop(hal);
        let texture = unsafe { wgpu::hal::dx12::Device::texture_from_raw(resource, texture_format, wgpu::TextureDimension::D2, extent, 1, 1) };
        return Ok(unsafe { device.create_texture_from_hal::<wgpu::hal::api::Dx12>(texture, &descriptor) });
    }
    let hal = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }.context("Shared pictures need Direct3D 12 or Vulkan")?;
    let raw = hal.raw_device().clone();
    let instance = hal.shared_instance().raw_instance();
    let mut external = vk::ExternalMemoryImageCreateInfo::default().handle_types(vk::ExternalMemoryHandleTypeFlags::D3D11_TEXTURE);
    let info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(vk::Format::from_raw(frame.vk_format))
        .extent(vk::Extent3D { width: frame.width, height: frame.height, depth: 1 })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .push_next(&mut external);
    let image = unsafe { raw.create_image(&info, None) }?;
    let requirements = unsafe { raw.get_image_memory_requirements(image) };
    let types = unsafe { instance.get_physical_device_memory_properties(hal.raw_physical_device()) };
    let index = (0..types.memory_type_count).find(|&i| {
        requirements.memory_type_bits & (1 << i) != 0 && types.memory_types[i as usize].property_flags.contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
    });
    let Some(index) = index else {
        unsafe { raw.destroy_image(image, None) };
        bail!("The shared picture doesn't fit this device's memory");
    };
    // An NT handle stays ours: the memory keeps its own reference to the texture.
    let mut import = vk::ImportMemoryWin32HandleInfoKHR::default().handle_type(vk::ExternalMemoryHandleTypeFlags::D3D11_TEXTURE).handle(handle.as_raw_handle() as vk::HANDLE);
    let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
    let allocate = vk::MemoryAllocateInfo::default().allocation_size(requirements.size).memory_type_index(index).push_next(&mut import).push_next(&mut dedicated);
    let memory = match unsafe { raw.allocate_memory(&allocate, None) } {
        Ok(m) => m,
        Err(e) => {
            unsafe { raw.destroy_image(image, None) };
            bail!("Couldn't import the shared picture: {e}");
        }
    };
    if let Err(e) = unsafe { raw.bind_image_memory(image, memory, 0) } {
        unsafe {
            raw.free_memory(memory, None);
            raw.destroy_image(image, None);
        }
        bail!("Couldn't use the shared picture: {e}");
    }
    let desc = wgpu::hal::TextureDescriptor {
        label: Some("Shared picture"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: texture_format,
        usage: wgpu::TextureUses::RESOURCE | wgpu::TextureUses::COPY_DST,
        memory_flags: wgpu::hal::MemoryFlags::empty(),
        view_formats: Vec::new(),
    };
    let release: wgpu::hal::DropCallback = Box::new(move || unsafe {
        raw.destroy_image(image, None);
        raw.free_memory(memory, None);
    });
    let texture = unsafe { hal.texture_from_raw(image, &desc, Some(release)) };
    drop(hal);
    Ok(unsafe { device.create_texture_from_hal::<wgpu::hal::api::Vulkan>(texture, &descriptor) })
}

/// Draws a shared picture into a source texture, as opaque colour: a game's alpha is
/// whatever its renderer left there.
pub(crate) const SHADER: &str = r#"
@group(0) @binding(0) var picture: texture_2d<f32>;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}
@fragment fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    return vec4<f32>(textureLoad(picture, vec2<i32>(at.xy), 0).rgb, 1.0);
}
"#;

