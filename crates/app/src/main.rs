#![cfg_attr(windows, windows_subsystem = "windows")]
mod system_monitor;
mod accounts;
mod activity;
mod audio;
mod audio_fx;
mod automations;
mod automation_preview;
mod backups;
mod profiles;
mod alert_guide;
mod alert_samples;
mod announce;
mod app_audio;
mod appearance;
mod fonts;
mod rewards_ui;
mod browser_chat;
mod browser_dock;
mod browser_source;
mod camera;
mod camera_layout;
mod canvas_edit;
mod capture;
mod capture_log;
mod avatars;
mod chat_render;
mod twemoji;
mod emote_picker;
mod title_bar;
#[cfg(not(windows))]
mod desktop;
#[cfg(windows)]
#[path = "platform_windows/desktop.rs"]
mod desktop;
mod donations;
mod donations_live;
mod edit_history;
#[cfg(not(windows))]
mod game_audio;
mod hotkeys;
#[cfg(windows)]
#[path = "platform_windows/game_audio.rs"]
mod game_audio;
mod kofi;
#[cfg(not(windows))]
mod launch_hook;
#[cfg(windows)]
#[path = "platform_windows/launch_hook.rs"]
mod launch_hook;
mod material;
mod media;
mod music_service;
mod ndi;
#[cfg(not(windows))]
mod music;
#[cfg(windows)]
#[path = "platform_windows/music.rs"]
mod music;
mod utilities;
mod external_import;
mod recording;
mod scripts;
mod remote;
#[cfg(not(windows))]
mod renderer_capture;
#[cfg(windows)]
#[path = "platform_windows/renderer_capture.rs"]
mod renderer_capture;
mod snapping;
mod demo;
mod speech;
mod sports;
mod streaming;
mod updates;
mod studio;
mod video;
mod virtual_camera;
mod widgets;
mod window_memory;
mod workspace;

use anyhow::{Context, Result, bail};
use auroracast_core::Project;
use std::path::PathBuf;

fn option(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// Finds GStreamer's MSVC x86_64 folder. Its MSI installs into `<INSTALLDIR>\1.0\msvc_x86_64` and sets a
/// machine-wide variable, but a process started before that (such as from the installer's last page)
/// doesn't see it, so the registry is checked as well, then the default folder.
#[cfg(windows)]
fn gstreamer_root() -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegGetValueW(key: isize, subkey: *const u16, value: *const u16, flags: u32, kind: *mut u32, data: *mut std::ffi::c_void, bytes: *mut u32) -> i32;
    }
    const HKEY_LOCAL_MACHINE: isize = 0x8000_0002u32 as i32 as isize;
    const RRF_RT_REG_SZ: u32 = 0x2;
    const VAR: &str = "GSTREAMER_1_0_ROOT_MSVC_X86_64";
    let machine = |key: &str, name: &str| -> Option<PathBuf> {
        let key: Vec<u16> = key.encode_utf16().chain(Some(0)).collect();
        let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let mut data = [0u16; 1024];
        let mut bytes = (data.len() * 2) as u32;
        let status = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), data.as_mut_ptr().cast(), &mut bytes) };
        let len = data.iter().position(|&c| c == 0).unwrap_or(data.len());
        (status == 0 && len > 0).then(|| std::ffi::OsString::from_wide(&data[..len]).into())
    };
    let mut candidates: Vec<PathBuf> = std::env::var_os(VAR).map(PathBuf::from).into_iter().collect();
    candidates.extend(machine(r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment", VAR));
    candidates.extend(machine(r"SOFTWARE\GStreamer1.0\x86_64", "InstallDir").map(|d| d.join(r"1.0\msvc_x86_64")));
    candidates.push(PathBuf::from(r"C:\gstreamer\1.0\msvc_x86_64"));
    candidates.into_iter().find(|g| g.join(r"bin\gstreamer-1.0-0.dll").is_file())
}

/// Puts GStreamer and the bundled FFmpeg on the search path, so starting rosorah-studio.exe directly
/// works as well as starting it from the Rosorah launcher. Without GStreamer it explains and stops.
#[cfg(windows)]
fn windows_runtime() {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBoxW(hwnd: *mut std::ffi::c_void, text: *const u16, title: *const u16, flags: u32) -> i32;
        fn ShowWindow(hwnd: *mut std::ffi::c_void, show: i32) -> i32;
        fn IsWindowVisible(hwnd: *mut std::ffi::c_void) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetConsoleWindow() -> *mut std::ffi::c_void;
        fn GetConsoleProcessList(list: *mut u32, count: u32) -> u32;
        fn AttachConsole(process: u32) -> i32;
        fn AllocConsole() -> i32;
        fn GetStdHandle(which: u32) -> *mut std::ffi::c_void;
        fn SetStdHandle(which: u32, handle: *mut std::ffi::c_void) -> i32;
        fn GetModuleHandleW(name: *const u16) -> *mut std::ffi::c_void;
        fn GetProcAddress(module: *mut std::ffi::c_void, name: *const u8) -> *mut std::ffi::c_void;
    }
    // Rosorah is a windowed program, so Windows gives it no console. It still needs one: the FFmpeg
    // and Python helpers it starts would each open a console window without it, and so does
    // NVIDIA Broadcast's camera driver, which prints a log when the cameras are listed. So it makes
    // a console with no window (Windows 11 24H2 and later), or uses the command prompt it was
    // started from. Output the launcher sends to startup.log keeps going there.
    #[repr(C)]
    struct AllocConsoleOptions {
        mode: i32,
        use_show_window: i32,
        show_window: u16,
    }
    const NO_WINDOW: i32 = 2;
    const STD_HANDLES: [u32; 3] = [-10i32 as u32, -11i32 as u32, -12i32 as u32];
    fn hide_own_console() {
        unsafe {
            let mut processes = [0u32; 2];
            let window = GetConsoleWindow();
            if !window.is_null() && IsWindowVisible(window) != 0 && GetConsoleProcessList(processes.as_mut_ptr(), 2) == 1 {
                ShowWindow(window, 0);
            }
        }
    }
    unsafe {
        if AttachConsole(u32::MAX) == 0 {
            let saved = STD_HANDLES.map(|h| GetStdHandle(h));
            let kernel32: Vec<u16> = "kernel32.dll".encode_utf16().chain(Some(0)).collect();
            let with_options = GetProcAddress(GetModuleHandleW(kernel32.as_ptr()), c"AllocConsoleWithOptions".as_ptr().cast());
            if with_options.is_null() {
                // Older Windows: the console has a window, hidden straight away.
                AllocConsole();
                hide_own_console();
            } else {
                let alloc: unsafe extern "system" fn(*const AllocConsoleOptions, *mut i32) -> i32 = std::mem::transmute(with_options);
                let mut result = 0;
                alloc(&AllocConsoleOptions { mode: NO_WINDOW, use_show_window: 0, show_window: 0 }, &mut result);
            }
            for (which, handle) in STD_HANDLES.into_iter().zip(saved) {
                if !handle.is_null() && handle as isize != -1 {
                    SetStdHandle(which, handle);
                }
            }
        }
    }
    let Some(gst) = gstreamer_root() else {
        let text: Vec<u16> = "Rosorah Studio can't start because its media runtime (GStreamer) is missing.\n\nRun the Rosorah Studio installer again to repair it."
            .encode_utf16().chain(Some(0)).collect();
        let title: Vec<u16> = "Rosorah Studio".encode_utf16().chain(Some(0)).collect();
        unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), 0x10) };
        std::process::exit(1);
    };
    let mut paths = vec![gst.join("bin")];
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("runtime/ffmpeg-9.0.2-essentials_build/bin"))) {
        paths.push(dir);
    }
    let current = std::env::var_os("PATH").unwrap_or_default();
    let existing: Vec<PathBuf> = std::env::split_paths(&current).collect();
    paths.retain(|p| !existing.contains(p));
    paths.extend(existing);
    // SAFETY: called first thing in main, before any other thread exists.
    unsafe {
        if let Ok(joined) = std::env::join_paths(paths) {
            std::env::set_var("PATH", joined);
        }
        if std::env::var_os("GST_PLUGIN_PATH").is_none() {
            std::env::set_var("GST_PLUGIN_PATH", gst.join("lib/gstreamer-1.0"));
        }
    }
    // In case anything still opens a console window of its own inside Rosorah.
    std::thread::spawn(|| loop {
        std::thread::sleep(std::time::Duration::from_millis(100));
        hide_own_console();
    });
}

fn main() -> Result<()> {
    #[cfg(windows)]
    windows_runtime();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "Rosorah Studio 0.32.0-preview.9 live streaming\n\n  rosorah-studio                       Open native studio\n  rosorah-studio --project FILE        Open a project\n  rosorah-studio --self-check          Validate GPU composition\n  rosorah-studio --snapshot --output DIR\n  rosorah-studio --render-demo --output DIR [--seconds 3] [--project FILE]\n\nRender-demo records synthetic video, not your screen. Requires ffmpeg with libx264.\nRecord and Go live are at the top right (FFmpeg required). Auto prefers a tested VAAPI/NVENC GPU encoder with software fallback. Existing output files are never overwritten. Sign in to Twitch, YouTube and Kick in Settings > Accounts."
        );
        return Ok(());
    }
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--self-check" | "--snapshot" | "--render-demo" => index += 1,
            "--project" | "--output" | "--seconds" | "--screenshot" | "--page" => {
                if args.get(index + 1).is_none_or(|v| v.starts_with("--")) {
                    bail!("{} requires a value", args[index]);
                }
                index += 2;
            }
            flag => bail!("Unknown argument {flag}; use --help"),
        }
    }
    let modes = ["--self-check", "--snapshot", "--render-demo"];
    if modes
        .iter()
        .filter(|m| args.iter().any(|a| a == **m))
        .count()
        > 1
    {
        bail!("Choose one engine command at a time");
    }
    if args.iter().any(|a| a == "--self-check") {
        return media::self_check();
    }
    let project_path = option(&args, "--project").map(PathBuf::from);
    if args
        .iter()
        .any(|a| a == "--render-demo" || a == "--snapshot")
    {
        let project = match &project_path {
            Some(path) => Project::load(path)?,
            None => Project::default(),
        };
        let output = PathBuf::from(option(&args, "--output").context("Specify --output DIR")?);
        if args.iter().any(|a| a == "--snapshot") {
            return media::snapshot(&project, &output);
        }
        let seconds: u32 = option(&args, "--seconds")
            .unwrap_or("3".into())
            .parse()
            .context("Seconds must be an integer")?;
        if !(1..=30).contains(&seconds) {
            bail!("Demo duration must be 1-30 seconds");
        }
        return media::render_demo(&project, &output, seconds);
    }
    let project_path = project_path.unwrap_or_else(|| profiles::path(&profiles::current()));
    let (project, notice) = profiles::open(&project_path)?;
    let screenshot = option(&args, "--screenshot").map(PathBuf::from);
    let page = option(&args, "--page").unwrap_or("studio".into());
    let viewport = eframe::egui::ViewportBuilder::default()
        .with_title("Rosorah Studio")
        .with_inner_size([1440., 900.])
        .with_min_inner_size([1100., 700.])
        .with_app_id("io.rosorah.Studio")
        .with_decorations(!title_bar::wanted(project.preferences.workspace.appearance.title_bar))
        .with_icon(eframe::icon_data::from_png_bytes(include_bytes!("../assets/icons/app-256.png")).unwrap_or_default());
    // Open the way it was left (screenshots always use the standard window).
    let viewport = if screenshot.is_none() { window_memory::apply(viewport, &window_memory::load()) } else { viewport };
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            desired_maximum_frame_latency: Some(1),
            ..Default::default()
        },
        ..Default::default()
    };
    let _launch_hook = launch_hook::Guard::start();
    eframe::run_native(
        "Rosorah Studio",
        options,
        Box::new(move |cc| {
            let mut studio = studio::Studio::new(cc, project, project_path, screenshot, page);
            if let Some(notice) = notice {
                studio.notice(notice);
            }
            Ok(Box::new(studio))
        }),
    )
    .map_err(|e| anyhow::anyhow!("Studio failed to open: {e}"))
}
