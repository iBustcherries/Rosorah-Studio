//! Windows Graphics Capture for explicit windows and opted-in fullscreen applications.
//! No process injection and no whole-monitor fallback for a window request.
use anyhow::{Context, Result, ensure};
use rosorah_capture::{Backend, GameProcess, Options, Producer};
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
        Graphics::{Direct3D::D3D_DRIVER_TYPE_HARDWARE, Direct3D11::*, Dxgi::*, Gdi::*},
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
struct Session {
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    winrt: IDirect3DDevice,
    size: windows::Graphics::SizeInt32,
    staging: Option<ID3D11Texture2D>,
    closed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    _item: GraphicsCaptureItem,
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}
impl Session {
    fn open(window: &Window, cursor: bool) -> Result<Self> {
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
            Self::open_item(item, cursor)
        }
    }
    fn open_monitor(id: &str, cursor: bool) -> Result<Self> {
        unsafe {
            let interop: IGraphicsCaptureItemInterop =
                windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
            let item: GraphicsCaptureItem =
                interop.CreateForMonitor(crate::capture::monitor_handle(id)?)?;
            Self::open_item(item, cursor)
        }
    }
    fn open_item(item: GraphicsCaptureItem, cursor: bool) -> Result<Self> {
        unsafe {
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
            let device = device.context("No D3D11 capture device")?;
            let context = context.context("No D3D11 context")?;
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
            let width = (size.Width as u32).min(desc.Width);
            let height = (size.Height as u32).min(desc.Height);
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
pub struct WindowBackend {
    pub window: Option<Window>,
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
        let interval = Duration::from_secs_f64(1. / options.fps.max(1) as f64);
        let mut last_frame = Instant::now();
        let mut received = false;
        let mut notice = Instant::now();
        output.connecting();
        while !*cancel.borrow() && cancel.has_changed().is_ok() {
            let start = Instant::now();
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
                    session = Some(Session::open(w, options.cursor)?);
                    output.game(Some(w.process.clone()));
                    last_frame = Instant::now();
                }
                if let Some(frame) = session
                    .as_mut()
                    .unwrap()
                    .next(options.resolution, &cancel)?
                {
                    last_frame = Instant::now();
                    received = true;
                    output.publish_with_detail(
                        frame,
                        Some(&format!(
                            "Live · Windows window capture · {} · cursor {}",
                            w.title,
                            if options.cursor { "on" } else { "off" }
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
            }
            if let Some(wait) = interval.checked_sub(start.elapsed()) {
                std::thread::sleep(wait.min(Duration::from_millis(50)));
            }
        }
        output.game(None);
        Ok(())
    }
}

pub struct MonitorBackend {
    pub id: String,
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
        let mut session = Session::open_monitor(&self.id, options.cursor)?;
        let interval = Duration::from_secs_f64(1. / options.fps.max(1) as f64);
        output.connecting();
        while !*cancel.borrow() && cancel.has_changed().is_ok() {
            let start = Instant::now();
            ensure!(
                !session.closed.load(std::sync::atomic::Ordering::Relaxed),
                "Selected monitor disconnected"
            );
            if let Some(frame) = session.next(options.resolution, &cancel)? {
                output.publish_with_detail(
                    frame,
                    Some("Live · Windows monitor capture · cursor enabled"),
                )?;
            }
            if let Some(wait) = interval.checked_sub(start.elapsed()) {
                std::thread::sleep(wait);
            }
        }
        Ok(())
    }
}
