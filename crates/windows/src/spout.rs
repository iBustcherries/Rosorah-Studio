//! Spout: apps on the same PC sharing pictures as DirectX textures. Spout's names list
//! ("SpoutSenderNames"), each sender's info block and its access lock are plain Windows shared
//! memory and mutexes, so Rosorah speaks the protocol directly instead of bundling the library.
use anyhow::{Context, Result, bail, ensure};
use rosorah_capture::{Backend, Frame, Options, Producer};
use std::time::{Duration, Instant};
use tokio::sync::watch;
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, HMODULE, WAIT_ABANDONED, WAIT_OBJECT_0},
        Graphics::{
            Direct3D::D3D_DRIVER_TYPE_HARDWARE,
            Direct3D11::*,
            Dxgi::{Common::*, IDXGIResource},
        },
        System::{
            Memory::{CreateFileMappingA, FILE_MAP_ALL_ACCESS, MEMORY_BASIC_INFORMATION, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile, OpenFileMappingA, PAGE_READWRITE, UnmapViewOfFile, VirtualQuery},
            Threading::{CreateMutexA, ReleaseMutex, WaitForSingleObject},
        },
    },
    core::{Interface, PCSTR},
};

const NAME_LEN: usize = 256;
/// Spout's default limit; a list someone else made first keeps its own size.
const MAX_SENDERS: usize = 64;
/// shareHandle, width, height, format, usage, description (128 wide chars), partnerId.
const INFO_LEN: usize = 280;
/// How long Spout apps wait for each other's locks.
const LOCK_MS: u32 = 67;

fn cstr(name: &str) -> Vec<u8> {
    name.bytes().chain([0]).collect()
}
/// A named shared memory block, kept open for as long as this value lives.
struct Shared {
    handle: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
    len: usize,
}
impl Shared {
    fn open(name: &str) -> Option<Self> {
        let name = cstr(name);
        unsafe {
            let handle = OpenFileMappingA(FILE_MAP_ALL_ACCESS.0, false, PCSTR(name.as_ptr())).ok()?;
            Self::map(handle)
        }
    }
    fn create(name: &str, len: usize) -> Result<Self> {
        let name = cstr(name);
        unsafe {
            let handle = CreateFileMappingA(HANDLE(-1isize as _), None, PAGE_READWRITE, 0, len as u32, PCSTR(name.as_ptr()))?;
            Self::map(handle).context("Windows couldn't share memory for Spout")
        }
    }
    unsafe fn map(handle: HANDLE) -> Option<Self> {
        unsafe {
            let view = MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, 0);
            if view.Value.is_null() {
                let _ = CloseHandle(handle);
                return None;
            }
            // A block opened by name doesn't say its size; the mapped region does.
            let mut info = MEMORY_BASIC_INFORMATION::default();
            VirtualQuery(Some(view.Value), &mut info, size_of::<MEMORY_BASIC_INFORMATION>());
            Some(Self { handle, view, len: info.RegionSize })
        }
    }
    fn bytes(&self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.view.Value.cast(), self.len) }
    }
}
impl Drop for Shared {
    fn drop(&mut self) {
        unsafe {
            let _ = UnmapViewOfFile(self.view);
            let _ = CloseHandle(self.handle);
        }
    }
}
/// A named Windows mutex, created if no other app made it yet.
struct Lock(HANDLE);
impl Lock {
    fn new(name: &str) -> Result<Self> {
        let name = cstr(name);
        Ok(Self(unsafe { CreateMutexA(None, false, PCSTR(name.as_ptr()))? }))
    }
    /// Runs `f` holding the lock; an app that never lets go isn't waited for forever.
    fn with<T>(&self, f: impl FnOnce() -> T) -> T {
        let held = matches!(unsafe { WaitForSingleObject(self.0, LOCK_MS) }, WAIT_OBJECT_0 | WAIT_ABANDONED);
        let result = f();
        if held {
            let _ = unsafe { ReleaseMutex(self.0) };
        }
        result
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}
fn read_names(list: &[u8]) -> Vec<String> {
    list.chunks_exact(NAME_LEN)
        .map(|slot| String::from_utf8_lossy(&slot[..slot.iter().position(|b| *b == 0).unwrap_or(NAME_LEN)]).into_owned())
        .take_while(|name| !name.is_empty())
        .collect()
}
fn write_names(list: &mut [u8], names: &[String]) {
    list.fill(0);
    for (slot, name) in list.chunks_exact_mut(NAME_LEN).zip(names) {
        let n = name.len().min(NAME_LEN - 1);
        slot[..n].copy_from_slice(&name.as_bytes()[..n]);
    }
}
/// The Spout senders running now, as their apps named them.
pub fn senders() -> Vec<String> {
    let Some(list) = Shared::open("SpoutSenderNames") else { return Vec::new() };
    let lock = Lock::new("SpoutSenderNames_mutex").ok();
    let names = match &lock {
        Some(lock) => lock.with(|| read_names(list.bytes())),
        None => read_names(list.bytes()),
    };
    // A crashed app can leave its name behind; only senders whose info still exists count.
    names.into_iter().filter(|n| Shared::open(n).is_some()).collect()
}
#[derive(Clone, Copy, PartialEq, Debug)]
struct Info {
    handle: u32,
    width: u32,
    height: u32,
    format: u32,
}
impl Info {
    fn read(bytes: &[u8]) -> Self {
        let word = |i: usize| u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap());
        Self { handle: word(0), width: word(1), height: word(2), format: word(3) }
    }
    fn write(&self, bytes: &mut [u8]) {
        bytes[..INFO_LEN].fill(0);
        for (i, v) in [self.handle, self.width, self.height, self.format].into_iter().enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        // The description holds the sending program's path, as wide characters.
        let path = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
        for (i, c) in path.encode_utf16().take(127).enumerate() {
            bytes[20 + i * 2..22 + i * 2].copy_from_slice(&c.to_le_bytes());
        }
    }
}
fn device() -> Result<(ID3D11Device, ID3D11DeviceContext)> {
    let (mut device, mut context) = (None, None);
    unsafe {
        D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut context))
            .context("Spout needs DirectX 11")?;
    }
    Ok((device.context("No DirectX device")?, context.context("No DirectX context")?))
}

/// Reads one Spout sender's pictures.
pub struct Receiver {
    name: String,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    lock: Lock,
    opened: Option<(Info, ID3D11Texture2D, ID3D11Texture2D)>,
}
impl Receiver {
    pub fn new(name: &str) -> Result<Self> {
        ensure!(!name.is_empty(), "Choose a Spout sender in Layer Properties");
        let (device, context) = device()?;
        Ok(Self { name: name.into(), device, context, lock: Lock::new(&format!("{name}_SpoutAccessMutex"))?, opened: None })
    }
    /// The sender's current picture as RGBA, or None while it isn't sending.
    pub fn frame(&mut self) -> Result<Option<Frame>> {
        let Some(block) = Shared::open(&self.name) else {
            self.opened = None;
            return Ok(None);
        };
        let info = Info::read(block.bytes());
        drop(block);
        if info.handle == 0 || info.width == 0 || info.height == 0 {
            return Ok(None);
        }
        if self.opened.as_ref().is_none_or(|(old, ..)| *old != info) {
            drop(self.opened.take());
            // DirectX 9 senders use the same 32-bit BGRA as DirectX 11 ones.
            ensure!(
                matches!(info.format, 0 | 21 | 28 | 87),
                "“{}” shares pictures in a format Rosorah can't show yet (format {}); set it to 8-bit RGBA or BGRA in that app",
                self.name,
                info.format
            );
            let mut texture: Option<ID3D11Texture2D> = None;
            unsafe { self.device.OpenSharedResource(HANDLE(info.handle as usize as _), &mut texture) }
                .with_context(|| format!("Couldn't open “{}”'s picture; both apps must use the same graphics card", self.name))?;
            let texture = texture.context("Spout sender gave no picture")?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            unsafe { texture.GetDesc(&mut desc) };
            desc.Usage = D3D11_USAGE_STAGING;
            desc.BindFlags = 0;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            desc.MiscFlags = 0;
            let mut staging = None;
            unsafe { self.device.CreateTexture2D(&desc, None, Some(&mut staging))? };
            self.opened = Some((info, texture, staging.context("No staging picture")?));
        }
        let Some((info, texture, staging)) = &self.opened else { return Ok(None) };
        self.lock.with(|| unsafe { self.context.CopyResource(staging, texture) });
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe { self.context.Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))? };
        let (w, h) = (info.width as usize, info.height as usize);
        let mut rgba = vec![0u8; w * h * 4];
        unsafe {
            let src = std::slice::from_raw_parts(mapped.pData.cast::<u8>(), mapped.RowPitch as usize * h);
            for (row, out) in rgba.chunks_exact_mut(w * 4).enumerate() {
                out.copy_from_slice(&src[row * mapped.RowPitch as usize..][..w * 4]);
            }
            self.context.Unmap(staging, 0);
        }
        if info.format != 28 {
            for px in rgba.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
        }
        Ok(Some(Frame { shared: None, width: info.width, height: info.height, rgba }))
    }
}
/// Shows one Spout sender's picture as a source.
pub struct SpoutBackend(pub String);
impl Backend for SpoutBackend {
    fn run(self: Box<Self>, output: Producer, cancel: watch::Receiver<bool>, _: Options) -> Result<()> {
        let mut receiver = Receiver::new(&self.0)?;
        let mut next = Instant::now();
        while !*cancel.borrow() {
            std::thread::sleep(next.saturating_duration_since(Instant::now()));
            next = Instant::now() + Duration::from_millis(16);
            match receiver.frame()? {
                Some(frame) => {
                    let detail = format!("Spout · {} · {}×{}", self.0, frame.width, frame.height);
                    output.publish_with_detail(frame, Some(&detail))?;
                }
                None => {
                    output.waiting(&format!("Waiting for “{}” to start sending", self.0));
                    next += Duration::from_millis(500);
                }
            }
        }
        Ok(())
    }
}

/// Shares Rosorah's picture with other apps on this PC as a Spout sender.
pub struct Sender {
    name: String,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    texture: Option<(u32, u32, ID3D11Texture2D)>,
    info: Shared,
    lock: Lock,
    // Held open so the names list survives while Rosorah is in it.
    names: Shared,
    names_lock: Lock,
    _active: Option<Shared>,
    bgra: Vec<u8>,
}
// DirectX 11 devices are free-threaded; the sender is only used from one thread at a time.
unsafe impl Send for Sender {}
impl Sender {
    pub fn new(name: &str) -> Result<Self> {
        ensure!(!name.is_empty() && name.len() < NAME_LEN, "Give the Spout output a name");
        let (device, context) = device()?;
        let names = match Shared::open("SpoutSenderNames") {
            Some(list) => list,
            None => Shared::create("SpoutSenderNames", NAME_LEN * MAX_SENDERS)?,
        };
        let names_lock = Lock::new("SpoutSenderNames_mutex")?;
        let added = names_lock.with(|| {
            let mut list = read_names(names.bytes());
            if list.iter().any(|n| n == name) {
                return Ok(false);
            }
            if list.len() >= names.len / NAME_LEN {
                bail!("Spout already has as many senders as it allows");
            }
            list.push(name.into());
            list.sort();
            write_names(names.bytes(), &list);
            Ok(true)
        })?;
        ensure!(added || Shared::open(name).is_none(), "Another app is already sending to Spout as “{name}”");
        // Receivers that just follow "the active sender" pick Rosorah when nothing else is on.
        let active = Shared::open("ActiveSenderName").map(Ok).unwrap_or_else(|| Shared::create("ActiveSenderName", NAME_LEN)).ok();
        if let Some(active) = &active
            && !read_names(names.bytes()).contains(&read_names(&active.bytes()[..NAME_LEN]).into_iter().next().unwrap_or_default())
        {
            write_names(&mut active.bytes()[..NAME_LEN], &[name.into()]);
        }
        Ok(Self {
            name: name.into(),
            device,
            context,
            texture: None,
            info: Shared::create(name, INFO_LEN)?,
            lock: Lock::new(&format!("{name}_SpoutAccessMutex"))?,
            names,
            names_lock,
            _active: active,
            bgra: Vec::new(),
        })
    }
    /// Sends one RGBA picture, tightly packed.
    pub fn send(&mut self, rgba: &[u8], width: u32, height: u32) -> Result<()> {
        ensure!(rgba.len() == width as usize * height as usize * 4, "Spout picture size mismatch");
        if self.texture.as_ref().is_none_or(|(w, h, _)| (*w, *h) != (width, height)) {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
                CPUAccessFlags: 0,
                MiscFlags: D3D11_RESOURCE_MISC_SHARED.0 as u32,
            };
            let mut texture = None;
            unsafe { self.device.CreateTexture2D(&desc, None, Some(&mut texture))? };
            let texture: ID3D11Texture2D = texture.context("No Spout picture")?;
            let handle = unsafe { texture.cast::<IDXGIResource>()?.GetSharedHandle()? };
            let info = Info { handle: handle.0 as usize as u32, width, height, format: DXGI_FORMAT_B8G8R8A8_UNORM.0 as u32 };
            self.lock.with(|| info.write(self.info.bytes()));
            self.texture = Some((width, height, texture));
        }
        let Some((_, _, texture)) = &self.texture else { return Ok(()) };
        self.bgra.clear();
        self.bgra.extend_from_slice(rgba);
        for px in self.bgra.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        self.lock.with(|| unsafe {
            self.context.UpdateSubresource(texture, 0, None, self.bgra.as_ptr().cast(), width * 4, 0);
            self.context.Flush();
        });
        Ok(())
    }
}
impl Drop for Sender {
    fn drop(&mut self) {
        self.names_lock.with(|| {
            let list: Vec<String> = read_names(self.names.bytes()).into_iter().filter(|n| *n != self.name).collect();
            write_names(self.names.bytes(), &list);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_list_round_trips_and_ends_at_the_first_empty_slot() {
        let mut list = vec![0u8; NAME_LEN * 4];
        write_names(&mut list, &["Rosorah".into(), "VTube Studio".into()]);
        assert_eq!(read_names(&list), ["Rosorah", "VTube Studio"]);
        write_names(&mut list, &[]);
        assert!(read_names(&list).is_empty());
    }
    #[test]
    fn sender_info_layout_matches_spout() {
        let mut bytes = vec![0u8; INFO_LEN];
        let info = Info { handle: 0x1234, width: 1920, height: 1080, format: 87 };
        info.write(&mut bytes);
        assert_eq!(Info::read(&bytes), info);
        assert_eq!(&bytes[4..8], &1920u32.to_le_bytes());
    }
}
