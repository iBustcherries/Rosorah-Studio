//! Rosorah's Windows 11 virtual camera. The camera component (crates/camera) is copied to Program
//! Files and registered once, with the user's permission. While the camera is on, Rosorah asks
//! Windows to show it (it disappears again when switched off), FFmpeg writes finished frames into
//! a private pipe, and a relay hands the newest frame to the component whenever it's reading.
use anyhow::{Context, Result, bail, ensure};
use std::{
    fs::File,
    io::{Read, Write},
    os::windows::io::{FromRawHandle, OwnedHandle},
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use windows::{
    Win32::{
        Foundation::*,
        Media::MediaFoundation::*,
        Security::{Authorization::*, *},
        Storage::FileSystem::*,
        System::{Com::*, LibraryLoader::*, Pipes::*, Registry::*, Threading::*},
        UI::{Shell::*, WindowsAndMessaging::SW_HIDE},
    },
    core::{HSTRING, Interface, PCSTR, PCWSTR},
};

pub const NAME: &str = "Rosorah Virtual Camera";
/// The camera component's class id (matches crates/camera).
pub const CLSID: &str = "{b6ac0fba-5752-439c-ad7b-9bbc8d093265}";
/// The picture size and rate the component offers; Rosorah scales its canvas to fit.
pub const WIDTH: u32 = 1920;
pub const HEIGHT: u32 = 1080;
pub const FPS: u32 = 30;
const FRAME_BYTES: usize = (WIDTH * HEIGHT * 3 / 2) as usize;
/// The pipe the camera component reads NV12 frames from.
pub const PIPE: &str = r"\\.\pipe\RosorahVirtualCamera";
const FILE_NAME: &str = "rosorah-camera.dll";

fn key() -> String {
    format!(r"SOFTWARE\Classes\CLSID\{CLSID}\InprocServer32")
}
/// Where the component is installed for Windows' camera service to load.
fn installed_path() -> PathBuf {
    PathBuf::from(std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into())).join("Rosorah Studio Camera").join(FILE_NAME)
}
/// The copy that ships next to Rosorah.
fn bundled_path() -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join(FILE_NAME))
}
/// Windows 11 has virtual cameras; Windows 10 doesn't.
fn windows_has_virtual_cameras() -> bool {
    unsafe {
        let Ok(library) = LoadLibraryW(&HSTRING::from("mfsensorgroup.dll")) else { return false };
        let found = GetProcAddress(library, PCSTR(c"MFCreateVirtualCamera".as_ptr().cast())).is_some();
        let _ = FreeLibrary(library);
        found
    }
}
/// Whether this computer can have the camera (Windows 11, and the component shipped with Rosorah).
pub fn supported() -> bool {
    windows_has_virtual_cameras() && bundled_path().is_some_and(|p| p.exists())
}
/// Whether the component is registered and in place.
pub fn installed() -> bool {
    let mut buffer = [0u16; 1024];
    let mut size = (buffer.len() * 2) as u32;
    let found = unsafe {
        RegGetValueW(HKEY_LOCAL_MACHINE, &HSTRING::from(key()), PCWSTR::null(), RRF_RT_REG_SZ, None, Some(buffer.as_mut_ptr().cast()), Some(&mut size))
    };
    if found.is_err() {
        return false;
    }
    let length = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
    let path = PathBuf::from(String::from_utf16_lossy(&buffer[..length]));
    windows_has_virtual_cameras() && path.exists()
}
/// Whether the installed component matches the one shipped with this Rosorah (false after an update).
pub fn up_to_date() -> bool {
    let read = |p: Option<PathBuf>| p.and_then(|p| std::fs::read(p).ok());
    read(bundled_path()) == read(Some(installed_path()))
}
/// Copies the component into Program Files and registers it. Windows asks for permission once.
pub fn set_up() -> Result<()> {
    ensure!(windows_has_virtual_cameras(), "Windows' virtual cameras need Windows 11");
    let source = bundled_path().filter(|p| p.exists()).context("The camera component is missing from Rosorah's folder; reinstall Rosorah")?;
    let target = installed_path();
    let folder = target.parent().context("No Program Files folder")?;
    let (source, target, folder, key) = (source.display(), target.display(), folder.display(), key());
    let script = format!(
        r#"/s /c "mkdir "{folder}" 2>nul & copy /y "{source}" "{target}" >nul && reg add "HKLM\SOFTWARE\Classes\CLSID\{CLSID}" /ve /d "{NAME}" /f >nul && reg add "HKLM\{key}" /ve /d "{target}" /f >nul && reg add "HKLM\{key}" /v ThreadingModel /d Both /f >nul""#
    );
    run_elevated(&script)?;
    ensure!(installed(), "The camera wasn't set up");
    Ok(())
}
/// Runs a cmd.exe line as administrator (Windows shows its permission prompt) and waits for it.
fn run_elevated(arguments: &str) -> Result<()> {
    let (verb, file, parameters) = (HSTRING::from("runas"), HSTRING::from("cmd.exe"), HSTRING::from(arguments));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    unsafe {
        if ShellExecuteExW(&mut info).is_err() {
            bail!("Setting up the camera needs administrator permission (the prompt was cancelled)");
        }
        let process = owned(info.hProcess);
        WaitForSingleObject(raw(&process), 60_000);
        let mut code = 1u32;
        let _ = GetExitCodeProcess(raw(&process), &mut code);
        ensure!(code == 0, "The camera setup didn't finish (code {code})");
    }
    Ok(())
}
fn owned(handle: HANDLE) -> OwnedHandle {
    unsafe { OwnedHandle::from_raw_handle(handle.0) }
}
fn raw(handle: &OwnedHandle) -> HANDLE {
    use std::os::windows::io::AsRawHandle;
    HANDLE(handle.as_raw_handle())
}

/// The newest frame from FFmpeg, numbered so the relay sends each one once.
struct Latest {
    frame: Mutex<(u64, Vec<u8>)>,
    arrived: Condvar,
}
/// The camera while it's on: Windows shows it to apps until this is dropped.
pub struct Camera {
    /// The pipe FFmpeg writes frames into.
    pub input: String,
    stop: Arc<AtomicBool>,
    latest: Arc<Latest>,
}
impl Camera {
    pub fn start() -> Result<Self> {
        ensure!(installed(), "Set up the virtual camera first");
        let stop = Arc::new(AtomicBool::new(false));
        let latest = Arc::new(Latest { frame: Mutex::new((0, vec![0; FRAME_BYTES])), arrived: Condvar::new() });
        let input = format!(r"\\.\pipe\RosorahCameraIn-{}-{:x}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos() as u64);
        // FFmpeg (running as this user) writes in; the camera service reads out.
        let incoming = pipe(&input, PIPE_ACCESS_INBOUND, "D:P(A;;GA;;;OW)(A;;GA;;;SY)")?;
        let (s, l) = (stop.clone(), latest.clone());
        std::thread::spawn(move || receive(incoming, &s, &l));
        let (s, l) = (stop.clone(), latest.clone());
        std::thread::spawn(move || send(&s, &l));
        let (ready_tx, ready) = std::sync::mpsc::channel();
        let s = stop.clone();
        std::thread::spawn(move || {
            let result = show_camera(&s, &ready_tx);
            let _ = ready_tx.send(result);
        });
        let camera = Self { input, stop, latest };
        match ready.recv_timeout(Duration::from_secs(20)) {
            Ok(Ok(())) => Ok(camera),
            Ok(Err(e)) => Err(e),
            Err(_) => bail!("Windows' camera service didn't answer"),
        }
    }
}
impl Drop for Camera {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.latest.arrived.notify_all();
        // Wake the pipes if they're still waiting for someone to connect.
        let _ = std::fs::OpenOptions::new().write(true).open(&self.input);
        let _ = File::open(PIPE);
    }
}
/// Creates a pipe end that waits for one connection.
fn pipe(name: &str, direction: FILE_FLAGS_AND_ATTRIBUTES, sddl: &str) -> Result<OwnedHandle> {
    unsafe {
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        ConvertStringSecurityDescriptorToSecurityDescriptorW(&HSTRING::from(sddl), SDDL_REVISION_1, &mut descriptor, None)?;
        let attributes = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: descriptor.0, bInheritHandle: false.into() };
        let handle = CreateNamedPipeW(&HSTRING::from(name), direction, PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS, 1, 4 << 20, 4 << 20, 0, Some(&attributes));
        let _ = LocalFree(HLOCAL(descriptor.0));
        ensure!(handle != INVALID_HANDLE_VALUE, "Couldn't make the camera pipe: {}", std::io::Error::last_os_error());
        Ok(owned(handle))
    }
}
/// Waits for the other end; a client that got in first counts as connected.
fn accept(handle: &OwnedHandle) -> bool {
    match unsafe { ConnectNamedPipe(raw(handle), None) } {
        Ok(()) => true,
        Err(e) => e.code() == ERROR_PIPE_CONNECTED.to_hresult(),
    }
}
fn receive(incoming: OwnedHandle, stop: &AtomicBool, latest: &Latest) {
    if !accept(&incoming) || stop.load(Ordering::Relaxed) {
        return;
    }
    let mut file = File::from(incoming);
    let mut buffer = vec![0u8; FRAME_BYTES];
    while !stop.load(Ordering::Relaxed) && file.read_exact(&mut buffer).is_ok() {
        let mut slot = latest.frame.lock().unwrap();
        slot.0 += 1;
        std::mem::swap(&mut slot.1, &mut buffer);
        latest.arrived.notify_all();
    }
}
/// Serves frames to the camera component each time it connects.
fn send(stop: &AtomicBool, latest: &Latest) {
    // The camera service runs as Local Service; it may only read.
    let access = "D:P(A;;GA;;;OW)(A;;GA;;;SY)(A;;GR;;;LS)(A;;GR;;;NS)";
    let mut frame = vec![0u8; FRAME_BYTES];
    while !stop.load(Ordering::Relaxed) {
        let Ok(handle) = pipe(PIPE, PIPE_ACCESS_OUTBOUND, access) else {
            std::thread::sleep(Duration::from_millis(500));
            continue;
        };
        if !accept(&handle) || stop.load(Ordering::Relaxed) {
            continue;
        }
        let mut file = File::from(handle);
        let mut sent = 0;
        while !stop.load(Ordering::Relaxed) {
            {
                let slot = latest.frame.lock().unwrap();
                let (slot, _) = latest.arrived.wait_timeout_while(slot, Duration::from_millis(250), |s| s.0 == sent && !stop.load(Ordering::Relaxed)).unwrap();
                if slot.0 == sent {
                    continue;
                }
                sent = slot.0;
                frame.copy_from_slice(&slot.1);
            }
            if file.write_all(&frame).is_err() {
                break;
            }
        }
    }
}
/// Registers the camera with Windows for this session and keeps it until `stop`.
fn show_camera(stop: &AtomicBool, ready: &std::sync::mpsc::Sender<Result<()>>) -> Result<()> {
    type Startup = unsafe extern "system" fn(u32, u32) -> windows::core::HRESULT;
    type Create = unsafe extern "system" fn(MFVirtualCameraType, MFVirtualCameraLifetime, MFVirtualCameraAccess, PCWSTR, PCWSTR, *const windows::core::GUID, u32, *mut *mut core::ffi::c_void) -> windows::core::HRESULT;
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        // Both are looked up at run time so Rosorah still starts where they don't exist.
        let mfplat = LoadLibraryW(&HSTRING::from("mfplat.dll")).context("Media Foundation isn't installed")?;
        let sensors = LoadLibraryW(&HSTRING::from("mfsensorgroup.dll")).context("Virtual cameras need Windows 11")?;
        let startup: Startup = std::mem::transmute(GetProcAddress(mfplat, PCSTR(c"MFStartup".as_ptr().cast())).context("Media Foundation isn't installed")?);
        let shutdown: unsafe extern "system" fn() -> windows::core::HRESULT = std::mem::transmute(GetProcAddress(mfplat, PCSTR(c"MFShutdown".as_ptr().cast())).context("Media Foundation isn't installed")?);
        let create: Create = std::mem::transmute(GetProcAddress(sensors, PCSTR(c"MFCreateVirtualCamera".as_ptr().cast())).context("Virtual cameras need Windows 11")?);
        startup(MF_VERSION, MFSTARTUP_LITE).ok()?;
        let (name, clsid) = (HSTRING::from(NAME), HSTRING::from(CLSID));
        let mut pointer = std::ptr::null_mut();
        let created = create(MFVirtualCameraType_SoftwareCameraSource, MFVirtualCameraLifetime_Session, MFVirtualCameraAccess_CurrentUser, PCWSTR(name.as_ptr()), PCWSTR(clsid.as_ptr()), std::ptr::null(), 0, &mut pointer).ok();
        if let Err(e) = created {
            let _ = shutdown();
            return Err(e).context("Windows didn't accept the camera");
        }
        let camera = IMFVirtualCamera::from_raw(pointer);
        let vtable = Interface::vtable(&camera);
        if let Err(e) = (vtable.Start)(camera.as_raw(), std::ptr::null_mut()).ok() {
            let _ = (vtable.Shutdown)(camera.as_raw());
            let _ = shutdown();
            return Err(e).context("Windows couldn't start the camera");
        }
        let _ = ready.send(Ok(()));
        while !stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = (vtable.Remove)(camera.as_raw());
        let _ = (vtable.Shutdown)(camera.as_raw());
        drop(camera);
        let _ = shutdown();
    }
    Ok(())
}
