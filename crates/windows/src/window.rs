//! Windows Graphics Capture for explicit windows and opted-in fullscreen applications.
//! No process injection and no whole-monitor fallback for a window request.
use anyhow::{Context, Result, bail, ensure};
use rosorah_capture::{Backend, GameProcess, Options, Producer};
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::watch;
use windows::{
    Foundation::TypedEventHandler,
    Graphics::{
        Capture::*,
        DirectX::{Direct3D11::IDirect3DDevice, DirectXPixelFormat},
    },
    Win32::{
        Foundation::*,
        Graphics::{
            Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_UNKNOWN},
            Direct3D11::*,
            Dxgi::{Common::*, *},
            Gdi::*,
        },
        System::{
            Diagnostics::ToolHelp::*,
            Threading::*,
            WinRT::{Direct3D11::*, Graphics::Capture::IGraphicsCaptureItemInterop, *},
        },
        UI::WindowsAndMessaging::*,
    },
    core::*,
};
#[derive(Clone, Debug)]
pub struct Window {
    pub handle: usize,
    pub title: String,
    pub process: GameProcess,
}
struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
pub fn process(pid: u32) -> Option<GameProcess> {
    unsafe {
        let h = Handle(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?);
        let (mut created, mut exit, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        GetProcessTimes(h.0, &mut created, &mut exit, &mut kernel, &mut user).ok()?;
        if exit.dwHighDateTime!=0 || exit.dwLowDateTime!=0 { return None; }
        let mut name = vec![0u16; 32768];
        let mut len = name.len() as u32;
        QueryFullProcessImageNameW(h.0, PROCESS_NAME_WIN32, PWSTR(name.as_mut_ptr()), &mut len)
            .ok()?;
        Some(GameProcess {
            pid,
            start_time: (created.dwHighDateTime as u64) << 32 | created.dwLowDateTime as u64,
            name: String::from_utf16_lossy(&name[..len as usize]),
        })
    }
}
pub fn alive(game: &GameProcess) -> bool {
    process(game.pid).is_some_and(|g| g.start_time == game.start_time)
}
fn inspect(hwnd: HWND) -> Option<Window> {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || hwnd == GetShellWindow() {
            return None;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == std::process::id() || pid == 0 {
            return None;
        }
        let mut title = [0u16; 1024];
        let n = GetWindowTextW(hwnd, &mut title);
        if n <= 0 {
            return None;
        }
        Some(Window {
            handle: hwnd.0 as usize,
            title: String::from_utf16_lossy(&title[..n as usize]),
            process: process(pid)?,
        })
    }
}
pub fn windows() -> Result<Vec<Window>> {
    unsafe {
        unsafe extern "system" fn enumerate(hwnd: HWND, param: LPARAM) -> BOOL {
            if let Some(w) = inspect(hwnd) {
                unsafe {
                    (*(param.0 as *mut Vec<Window>)).push(w);
                }
            }
            BOOL(1)
        }
        let mut list = Vec::<Window>::new();
        EnumWindows(Some(enumerate), LPARAM(&mut list as *mut _ as isize))?;
        list.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
        Ok(list)
    }
}
fn graphics_process(pid: u32) -> bool {
    unsafe {
        let Ok(h) = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid) else {
            return false;
        };
        let h = Handle(h);
        let mut entry = MODULEENTRY32W {
            dwSize: std::mem::size_of::<MODULEENTRY32W>() as u32,
            ..Default::default()
        };
        if Module32FirstW(h.0, &mut entry).is_err() {
            return false;
        }
        loop {
            let n = String::from_utf16_lossy(
                &entry.szModule[..entry
                    .szModule
                    .iter()
                    .position(|c| *c == 0)
                    .unwrap_or(entry.szModule.len())],
            )
            .to_lowercase();
            if crate::capture_policy::renderer(&n) { return true; }
            if Module32NextW(h.0, &mut entry).is_err() {
                return false;
            }
        }
    }
}
fn automatic() -> Option<Window> {
    unsafe {
        let hwnd = GetForegroundWindow();
        let w = inspect(hwnd)?;
        if crate::capture_policy::excluded(&w.process.name) { return None; }
        if IsIconic(hwnd).as_bool() {
            return None;
        }
        let mut rect = RECT::default();
        GetWindowRect(hwnd, &mut rect).ok()?;
        let mut monitor = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(
            MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
            &mut monitor,
        )
        .as_bool()
        {
            return None;
        }
        let m = monitor.rcMonitor;
        if !crate::capture_policy::fullscreen([rect.left,rect.top,rect.right,rect.bottom],[m.left,m.top,m.right,m.bottom]) { return None; }
        graphics_process(w.process.pid).then_some(w)
    }
}
struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            RoUninitialize();
        }
    }
}
/// The graphics adapter the studio draws with (its LUID), when it can draw pictures handed over
/// on the graphics card; asked again for every frame, since the studio may stop accepting them.
pub type Adapter = fn() -> Option<[u8; 8]>;

/// Textures shared with the studio as NT handles: each captured frame is copied into a free
/// one on the graphics card, and the studio draws from there instead of from memory.
struct Ring {
    export: u64,
    size: (u32, u32),
    textures: Vec<ID3D11Texture2D>,
    handles: Arc<Vec<OwnedHandle>>,
    /// Slots the studio may still be drawing from; set free again when it lets go of a frame.
    busy: Arc<Vec<AtomicBool>>,
    /// When every slot was last found busy.
    full_since: Option<Instant>,
}
/// Every ring gets its own number, so the studio opens each set of textures once.
static EXPORTS: AtomicU64 = AtomicU64::new(1);
impl Ring {
    fn new(device: &ID3D11Device, width: u32, height: u32) -> Result<Self> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: (D3D11_RESOURCE_MISC_SHARED.0 | D3D11_RESOURCE_MISC_SHARED_NTHANDLE.0) as u32,
        };
        let (mut textures, mut handles) = (Vec::new(), Vec::new());
        for _ in 0..4 {
            unsafe {
                let mut texture = None;
                device.CreateTexture2D(&desc, None, Some(&mut texture)).context("Couldn't make a shared texture")?;
                let texture = texture.context("Couldn't make a shared texture")?;
                let handle = texture
                    .cast::<IDXGIResource1>()?
                    .CreateSharedHandle(None, (DXGI_SHARED_RESOURCE_READ | DXGI_SHARED_RESOURCE_WRITE).0, PCWSTR::null())
                    .context("Couldn't share a texture")?;
                handles.push(OwnedHandle::from_raw_handle(handle.0));
                textures.push(texture);
            }
        }
        Ok(Self {
            export: EXPORTS.fetch_add(1, Ordering::Relaxed),
            size: (width, height),
            textures,
            handles: Arc::new(handles),
            busy: Arc::new((0..4).map(|_| AtomicBool::new(false)).collect()),
            full_since: None,
        })
    }
}
/// A capture device on the adapter with this LUID.
fn device_on(luid: [u8; 8]) -> Result<(ID3D11Device, ID3D11DeviceContext)> {
    unsafe {
        let factory: IDXGIFactory4 = CreateDXGIFactory1()?;
        let luid = LUID {
            LowPart: u32::from_le_bytes(luid[..4].try_into().unwrap()),
            HighPart: i32::from_le_bytes(luid[4..].try_into().unwrap()),
        };
        let adapter: IDXGIAdapter = factory.EnumAdapterByLuid(luid)?;
        let (mut device, mut context) = (None, None);
        D3D11CreateDevice(
            &adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )?;
        Ok((device.context("No D3D11 capture device")?, context.context("No D3D11 context")?))
    }
}
struct Session {
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    winrt: IDirect3DDevice,
    size: windows::Graphics::SizeInt32,
    staging: Option<ID3D11Texture2D>,
    closed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Set by Windows' FrameArrived callback, so the capture loop wakes as a frame is ready.
    arrived: std::sync::Arc<Arrived>,
    /// Where the studio draws, and the adapter this device is on when it's the same one.
    gpu: Option<Adapter>,
    adapter: Option<[u8; 8]>,
    ring: Option<Ring>,
    copied: Option<ID3D11Query>,
    /// Whether the last frame went to the studio on the graphics card, or why it can't.
    on_gpu: bool,
    problem: Option<String>,
    _item: GraphicsCaptureItem,
}
/// "A new frame is waiting", from Windows' capture thread to the capture loop.
#[derive(Default)]
struct Arrived {
    ready: std::sync::Mutex<bool>,
    signal: std::sync::Condvar,
}
impl Arrived {
    fn notify(&self) {
        *self.ready.lock().unwrap() = true;
        self.signal.notify_one();
    }
    /// Waits up to `limit` for a frame; true when one arrived.
    fn wait(&self, limit: Duration) -> bool {
        let ready = self.ready.lock().unwrap();
        let (mut ready, _) = self.signal.wait_timeout_while(ready, limit, |r| !*r).unwrap();
        std::mem::take(&mut *ready)
    }
}
/// Frames are taken as soon as Windows delivers them (no waiting for a timer), at most twice the
/// capture rate: the preview and the output clock then always get the newest picture, and a frame
/// never waits a whole interval for the next poll.
fn min_gap(fps: u32) -> Duration {
    Duration::from_secs_f64(0.5 / fps.max(1) as f64)
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}
impl Session {
    fn open(window: &Window, cursor: bool, gpu: Option<Adapter>) -> Result<Self> {
        unsafe {
            ensure!(
                GraphicsCaptureSession::IsSupported()?,
                "Windows Graphics Capture is unavailable on this system"
            );
            let hwnd = HWND(window.handle as *mut _);
            ensure!(
                inspect(hwnd).is_some_and(|w| w.process == window.process),
                "Selected window closed or changed"
            );
            let interop: IGraphicsCaptureItemInterop =
                windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
            let item: GraphicsCaptureItem = interop.CreateForWindow(hwnd)?;
            Self::open_item(item, cursor, gpu)
        }
    }
    fn open_monitor(id: &str, cursor: bool, gpu: Option<Adapter>) -> Result<Self> {
        unsafe {
            let interop: IGraphicsCaptureItemInterop =
                windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
            let item: GraphicsCaptureItem =
                interop.CreateForMonitor(crate::capture::monitor_handle(id)?)?;
            Self::open_item(item, cursor, gpu)
        }
    }
    fn open_item(item: GraphicsCaptureItem, cursor: bool, gpu: Option<Adapter>) -> Result<Self> {
        unsafe {
            // On the studio's adapter when it can take frames on the graphics card, so they can
            // be shared with it; otherwise the default one.
            let mut problem = None;
            let on_adapter = gpu.and_then(|f| f()).and_then(|luid| match device_on(luid) {
                Ok(made) => Some((made, luid)),
                Err(e) => {
                    problem = Some(format!("no capture device on the studio's graphics card ({e})"));
                    None
                }
            });
            let (device, context, adapter) = match on_adapter {
                Some(((device, context), luid)) => (device, context, Some(luid)),
                None => {
                    let (mut device, mut context) = (None, None);
                    D3D11CreateDevice(
                        None,
                        D3D_DRIVER_TYPE_HARDWARE,
                        HMODULE::default(),
                        D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                        None,
                        D3D11_SDK_VERSION,
                        Some(&mut device),
                        None,
                        Some(&mut context),
                    )?;
                    (device.context("No D3D11 capture device")?, context.context("No D3D11 context")?, None)
                }
            };
            let dxgi: IDXGIDevice = device.cast()?;
            let winrt: IDirect3DDevice = CreateDirect3D11DeviceFromDXGIDevice(&dxgi)?.cast()?;
            let size = item.Size()?;
            ensure!(
                size.Width > 0 && size.Height > 0,
                "Selected window has no drawable content"
            );
            let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
                &winrt,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                2,
                size,
            )?;
            let arrived = std::sync::Arc::new(Arrived::default());
            let wake = arrived.clone();
            pool.FrameArrived(&TypedEventHandler::new(move |_, _| {
                wake.notify();
                Ok(())
            }))?;
            let session = pool.CreateCaptureSession(&item)?;
            session
                .SetIsCursorCaptureEnabled(cursor)
                .context("Cursor control requires a current Windows 10/11 release")?;
            let closed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let signal = closed.clone();
            item.Closed(&TypedEventHandler::new(move |_, _| {
                signal.store(true, std::sync::atomic::Ordering::Relaxed);
                Ok(())
            }))?;
            session.StartCapture()?;
            Ok(Self {
                pool,
                session,
                device,
                context,
                winrt,
                size,
                staging: None,
                closed,
                arrived,
                gpu,
                adapter,
                ring: None,
                copied: None,
                on_gpu: false,
                problem,
                _item: item,
            })
        }
    }
    fn next(
        &mut self,
        resolution: u32,
        cancel: &watch::Receiver<bool>,
    ) -> Result<Option<rosorah_capture::Frame>> {
        unsafe {
            let Ok(mut frame) = self.pool.TryGetNextFrame() else {
                return Ok(None);
            };
            // Consume only the newest available frame; never accumulate a preview queue.
            for _ in 0..2 {
                if let Ok(new) = self.pool.TryGetNextFrame() {
                    let _ = frame.Close();
                    frame = new;
                } else {
                    break;
                }
            }
            struct FrameGuard(Direct3D11CaptureFrame);
            impl Drop for FrameGuard {
                fn drop(&mut self) {
                    let _ = self.0.Close();
                }
            }
            let frame = FrameGuard(frame);
            let size = frame.0.ContentSize()?;
            if size.Width <= 0 || size.Height <= 0 {
                return Ok(None);
            }
            if size != self.size {
                self.size = size;
                self.staging = None;
                drop(frame);
                self.pool.Recreate(
                    &self.winrt,
                    DirectXPixelFormat::B8G8R8A8UIntNormalized,
                    2,
                    size,
                )?;
                return Ok(None);
            }
            let access: IDirect3DDxgiInterfaceAccess = frame.0.Surface()?.cast()?;
            let texture: ID3D11Texture2D = access.GetInterface()?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut desc);
            ensure!(
                desc.Width <= 16384 && desc.Height <= 16384,
                "Window dimensions exceed capture limits"
            );
            let width = (size.Width as u32).min(desc.Width);
            let height = (size.Height as u32).min(desc.Height);
            if let Some(shared) = self.share(&texture, &desc, width, height, cancel) {
                return Ok(shared);
            }
            if self.staging.is_none() {
                desc.Usage = D3D11_USAGE_STAGING;
                desc.BindFlags = 0;
                desc.MiscFlags = 0;
                desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
                let mut t = None;
                self.device.CreateTexture2D(&desc, None, Some(&mut t))?;
                self.staging = t;
            }
            let staging = self
                .staging
                .as_ref()
                .context("Cannot allocate window capture staging texture")?;
            self.context.CopyResource(staging, &texture);
            self.context.Flush();
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            let started = Instant::now();
            loop {
                match self.context.Map(
                    staging,
                    0,
                    D3D11_MAP_READ,
                    D3D11_MAP_FLAG_DO_NOT_WAIT.0 as u32,
                    Some(&mut mapped),
                ) {
                    Ok(()) => break,
                    Err(e) if e.code() == DXGI_ERROR_WAS_STILL_DRAWING => {
                        if *cancel.borrow() || cancel.has_changed().is_err() {
                            return Ok(None);
                        }
                        ensure!(
                            started.elapsed() < Duration::from_secs(2),
                            "GPU window readback stalled"
                        );
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            struct Unmap<'a>(&'a ID3D11DeviceContext, &'a ID3D11Texture2D);
            impl Drop for Unmap<'_> {
                fn drop(&mut self) {
                    unsafe { self.0.Unmap(self.1, 0) }
                }
            }
            let _unmap = Unmap(&self.context, staging);
            let bytes = std::slice::from_raw_parts(
                mapped.pData.cast::<u8>(),
                mapped.RowPitch as usize * height as usize,
            );
            Ok(Some(crate::pixels::convert(
                bytes,
                width,
                height,
                mapped.RowPitch as usize,
                crate::pixels::Rotation::Identity,
                resolution,
            )?))
        }
    }
}
impl Session {
    /// Hands a frame to the studio on the graphics card when it draws on this device's adapter.
    /// None sends it through memory instead; Some(None) skips it (no free slot, or stopping).
    fn share(&mut self, texture: &ID3D11Texture2D, desc: &D3D11_TEXTURE2D_DESC, width: u32, height: u32, cancel: &watch::Receiver<bool>) -> Option<Option<rosorah_capture::Frame>> {
        self.on_gpu = false;
        let luid = self.gpu.and_then(|f| f())?;
        if self.adapter != Some(luid) {
            return None;
        }
        if desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM || width > 4096 || height > 4096 {
            self.problem = Some("the picture is too large or in an unusual format".into());
            return None;
        }
        let stop = || *cancel.borrow() || cancel.has_changed().is_err();
        match unsafe { copy_shared(&self.device, &self.context, &mut self.ring, &mut self.copied, texture, width, height, &stop) } {
            Ok(frame) => {
                self.on_gpu = true;
                self.problem = None;
                Some(frame)
            }
            Err(e) => {
                // Through memory from now on in this session.
                self.gpu = None;
                self.ring = None;
                self.problem = Some(format!("{e:#}"));
                None
            }
        }
    }
    /// How frames reach the studio, for the capture details.
    fn route(&self) -> String {
        match (&self.on_gpu, &self.problem) {
            (true, _) => "on the graphics card".into(),
            (false, Some(problem)) => format!("through memory ({problem})"),
            (false, None) => "through memory".into(),
        }
    }
}
/// Copies a frame into a free texture of the ring and waits for the copy: the frame to hand to the
/// studio, or None when every texture is still in use or `stop` says to stop.
#[allow(clippy::too_many_arguments)]
unsafe fn copy_shared(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    ring: &mut Option<Ring>,
    copied: &mut Option<ID3D11Query>,
    texture: &ID3D11Texture2D,
    width: u32,
    height: u32,
    stop: &dyn Fn() -> bool,
) -> Result<Option<rosorah_capture::Frame>> {
    unsafe {
        if ring.as_ref().is_none_or(|r| r.size != (width, height)) {
            *ring = Some(Ring::new(device, width, height)?);
        }
        let ring = ring.as_mut().unwrap();
        let Some(slot) = ring.busy.iter().position(|b| !b.load(Ordering::Acquire)) else {
            if ring.full_since.get_or_insert_with(Instant::now).elapsed() > Duration::from_secs(2) {
                bail!("the studio stopped handing shared pictures back");
            }
            return Ok(None);
        };
        ring.full_since = None;
        let area = D3D11_BOX { left: 0, top: 0, front: 0, right: width, bottom: height, back: 1 };
        context.CopySubresourceRegion(&ring.textures[slot], 0, 0, 0, 0, texture, 0, Some(&area));
        // The studio draws with another device, so wait here until the copy is done: it
        // never sees half a picture.
        if copied.is_none() {
            let mut query = None;
            device.CreateQuery(&D3D11_QUERY_DESC { Query: D3D11_QUERY_EVENT, MiscFlags: 0 }, Some(&mut query))?;
            *copied = query;
        }
        let copied = copied.as_ref().context("No copy query")?;
        context.End(copied);
        context.Flush();
        let started = Instant::now();
        loop {
            let mut done = BOOL(0);
            context.GetData(copied, Some(&mut done as *mut BOOL as *mut _), std::mem::size_of::<BOOL>() as u32, 0)?;
            if done.as_bool() {
                break;
            }
            if stop() {
                return Ok(None);
            }
            ensure!(started.elapsed() < Duration::from_secs(2), "the copy on the graphics card stalled");
            std::thread::sleep(Duration::from_micros(500));
        }
        ring.busy[slot].store(true, Ordering::Release);
        let busy = ring.busy.clone();
        Ok(Some(rosorah_capture::Frame {
            width,
            height,
            rgba: Vec::new(),
            shared: Some(Arc::new(rosorah_capture::SharedImage {
                export: ring.export,
                slot,
                width,
                height,
                // VK_FORMAT_B8G8R8A8_UNORM, the same as DXGI_FORMAT_B8G8R8A8_UNORM.
                vk_format: 44,
                sizes: Vec::new(),
                memory: ring.handles.clone(),
                release: Box::new(move || busy[slot].store(false, Ordering::Release)),
            })),
        }))
    }
}

/// A stand-in for a captured window, for checking that pictures handed over on the graphics card
/// arrive intact (`rosorah-studio --gpu-share-check`).
pub struct ShareCheck {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    picture: ID3D11Texture2D,
    size: (u32, u32),
    ring: Option<Ring>,
    copied: Option<ID3D11Query>,
}
impl ShareCheck {
    pub fn new(luid: [u8; 8], width: u32, height: u32) -> Result<Self> {
        let (device, context) = device_on(luid)?;
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut picture = None;
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut picture))? };
        Ok(Self { device, context, picture: picture.context("No test picture")?, size: (width, height), ring: None, copied: None })
    }
    /// A frame filled with one RGBA colour, as capture would hand it over.
    pub fn frame(&mut self, rgba: [u8; 4]) -> Result<Option<rosorah_capture::Frame>> {
        let (width, height) = self.size;
        let pixels: Vec<u8> = (0..width * height).flat_map(|_| [rgba[2], rgba[1], rgba[0], rgba[3]]).collect();
        unsafe {
            self.context.UpdateSubresource(&self.picture, 0, None, pixels.as_ptr().cast(), width * 4, 0);
            copy_shared(&self.device, &self.context, &mut self.ring, &mut self.copied, &self.picture, width, height, &|| false)
        }
    }
}
pub struct WindowBackend {
    pub window: Option<Window>,
    pub gpu: Option<Adapter>,
}
impl Backend for WindowBackend {
    fn run(
        self: Box<Self>,
        output: Producer,
        cancel: watch::Receiver<bool>,
        options: Options,
    ) -> Result<()> {
        unsafe {
            RoInitialize(RO_INIT_MULTITHREADED)?;
        }
        let _apartment = Apartment;
        let auto = options.target == rosorah_capture::Target::AutoFullscreen;
        ensure!(
            auto || self.window.is_some(),
            "Select a window before starting capture"
        );
        let mut selected = self.window.clone();
        let mut session: Option<Session> = None;
        let mut checked = Instant::now() - Duration::from_secs(1);
        let gap = min_gap(options.fps);
        let mut last_frame = Instant::now();
        let mut received = false;
        let mut notice = Instant::now();
        output.connecting();
        while !*cancel.borrow() && cancel.has_changed().is_ok() {
            if auto && checked.elapsed() >= Duration::from_millis(500) {
                checked = Instant::now();
                if let Some(w) = automatic() {
                    if selected
                        .as_ref()
                        .is_none_or(|old| old.handle != w.handle || old.process != w.process)
                    {
                        session = None;
                        selected = Some(w);
                        received = false;
                        output.clear("Connecting to fullscreen application");
                    }
                }
            }
            if selected.as_ref().is_some_and(|w| {
                !alive(&w.process) || !unsafe { IsWindow(HWND(w.handle as *mut _)) }.as_bool()
            }) || session
                .as_ref()
                .is_some_and(|s| s.closed.load(std::sync::atomic::Ordering::Relaxed))
            {
                session = None;
                selected = None;
                received = false;
                output.clear(
                    "Captured application closed; waiting for another fullscreen application",
                );
                if !auto {
                    anyhow::bail!("Selected window closed; choose another window")
                }
            }
            if let Some(w) = &selected {
                if session.is_none() {
                    session = Some(Session::open(w, options.cursor, self.gpu)?);
                    output.game(Some(w.process.clone()));
                    last_frame = Instant::now();
                }
                let s = session.as_mut().unwrap();
                // Sleep until Windows has a frame (or 50 ms, to look after the window), keeping
                // at least `gap` since the last one.
                if let Some(wait) = gap.checked_sub(last_frame.elapsed()) {
                    std::thread::sleep(wait);
                }
                s.arrived.wait(Duration::from_millis(50));
                if let Some(frame) = s.next(options.resolution, &cancel)? {
                    last_frame = Instant::now();
                    received = true;
                    output.publish_with_detail(
                        frame,
                        Some(&format!(
                            "Live · Windows window capture · {} · cursor {} · {}",
                            w.title,
                            if options.cursor { "on" } else { "off" },
                            s.route()
                        )),
                    )?;
                } else if last_frame.elapsed() > Duration::from_secs(3)
                    && notice.elapsed() > Duration::from_secs(3)
                {
                    notice = Instant::now();
                    if received {
                        output.note("Windows window capture · no new image (static or minimized); application audio remains connected");
                    } else {
                        output.waiting(
                            "Waiting for window frames (minimized, protected, or not rendering)",
                        );
                    }
                }
            } else {
                output.waiting("Waiting for a fullscreen graphics application. Window capture is also available.");
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        output.game(None);
        Ok(())
    }
}

pub struct MonitorBackend {
    pub id: String,
    pub gpu: Option<Adapter>,
}
impl Backend for MonitorBackend {
    fn run(
        self: Box<Self>,
        output: Producer,
        cancel: watch::Receiver<bool>,
        options: Options,
    ) -> Result<()> {
        unsafe {
            RoInitialize(RO_INIT_MULTITHREADED)?;
        }
        let _apartment = Apartment;
        let mut session = Session::open_monitor(&self.id, options.cursor, self.gpu)?;
        let gap = min_gap(options.fps);
        let mut last = Instant::now() - gap;
        output.connecting();
        while !*cancel.borrow() && cancel.has_changed().is_ok() {
            ensure!(
                !session.closed.load(std::sync::atomic::Ordering::Relaxed),
                "Selected monitor disconnected"
            );
            if let Some(wait) = gap.checked_sub(last.elapsed()) {
                std::thread::sleep(wait);
            }
            session.arrived.wait(Duration::from_millis(50));
            if let Some(frame) = session.next(options.resolution, &cancel)? {
                last = Instant::now();
                output.publish_with_detail(
                    frame,
                    Some(&format!("Live · Windows monitor capture · cursor enabled · {}", session.route())),
                )?;
            }
        }
        Ok(())
    }
}
