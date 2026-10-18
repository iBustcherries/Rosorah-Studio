#![windows_subsystem = "windows"]
use std::{env, fs, os::windows::process::CommandExt, path::PathBuf, process::{Command, Stdio}};
#[link(name="user32")]
unsafe extern "system" { fn MessageBoxW(hwnd: *mut std::ffi::c_void, text: *const u16, title: *const u16, flags: u32) -> i32; }
fn error(text: &str) {
    let text: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    let title: Vec<u16> = "Rosorah Studio".encode_utf16().chain(Some(0)).collect();
    unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), 0x10); }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let root=env::current_exe()?.parent().ok_or("Missing application directory")?.to_owned();
    let gst=env::var_os("GSTREAMER_1_0_ROOT_MSVC_X86_64").map(PathBuf::from).unwrap_or_else(||PathBuf::from(r"C:\gstreamer\1.0\msvc_x86_64"));
    if !gst.join("bin/gstreamer-1.0-0.dll").is_file() { return Err("Media runtime is missing. Run the Rosorah installer again to repair setup.".into()); }
    let mut paths=vec![root.join("runtime/ffmpeg-9.0.2-essentials_build/bin"),gst.join("bin")];
    paths.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    let logs=PathBuf::from(env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is unavailable")?).join("RosorahStudio/logs");
    fs::create_dir_all(&logs)?;
    let log=logs.join("startup.log");
    let output=fs::File::create(&log)?;
    let status=Command::new(root.join("rosorah-studio.exe"))
        .args(env::args_os().skip(1)).current_dir(&root)
        .env("PATH",env::join_paths(paths)?)
        .env("GST_PLUGIN_PATH",gst.join("lib/gstreamer-1.0"))
        .stdin(Stdio::null()).stdout(output.try_clone()?).stderr(output)
        .creation_flags(0x08000000).status()?;
    if !status.success() { return Err(format!("Rosorah exited unexpectedly ({status}). Details: {}",log.display()).into()); }
    Ok(())
}
fn main(){if let Err(e)=run(){error(&e.to_string());std::process::exit(1)}}
