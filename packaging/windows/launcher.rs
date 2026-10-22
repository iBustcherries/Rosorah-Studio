#![windows_subsystem = "windows"]
use std::{env, fs, os::windows::process::CommandExt, path::PathBuf, process::{Command, Stdio}};
#[link(name="user32")]
unsafe extern "system" { fn MessageBoxW(hwnd: *mut std::ffi::c_void, text: *const u16, title: *const u16, flags: u32) -> i32; }
fn error(text: &str) {
    let text: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    let title: Vec<u16> = "Rosorah Studio".encode_utf16().chain(Some(0)).collect();
    unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), 0x10); }
}
#[link(name="advapi32")]
unsafe extern "system" { fn RegGetValueW(key: isize, subkey: *const u16, value: *const u16, flags: u32, kind: *mut u32, data: *mut std::ffi::c_void, bytes: *mut u32) -> i32; }
const GST_VAR: &str = "GSTREAMER_1_0_ROOT_MSVC_X86_64";
fn machine_string(key: &str, name: &str) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    let key: Vec<u16> = key.encode_utf16().chain(Some(0)).collect();
    let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let mut data=[0u16; 1024];
    let mut bytes=(data.len()*2) as u32;
    let status=unsafe { RegGetValueW(0x8000_0002u32 as i32 as isize, key.as_ptr(), name.as_ptr(), 0x2, std::ptr::null_mut(), data.as_mut_ptr().cast(), &mut bytes) };
    let len=data.iter().position(|&c| c==0).unwrap_or(data.len());
    (status==0 && len>0).then(|| std::ffi::OsString::from_wide(&data[..len]).into())
}
// GStreamer's MSI installs into <INSTALLDIR>\1.0\msvc_x86_64 and sets a machine-wide variable that a process
// started before it (such as from the installer's last page) doesn't see, so the registry is checked too.
fn gstreamer_root() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf>=env::var_os(GST_VAR).map(PathBuf::from).into_iter().collect();
    candidates.extend(machine_string(r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment", GST_VAR));
    candidates.extend(machine_string(r"SOFTWARE\GStreamer1.0\x86_64", "InstallDir").map(|d| d.join(r"1.0\msvc_x86_64")));
    candidates.push(PathBuf::from(r"C:\gstreamer\1.0\msvc_x86_64"));
    candidates.into_iter().find(|g| g.join(r"bin\gstreamer-1.0-0.dll").is_file())
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let root=env::current_exe()?.parent().ok_or("Missing application directory")?.to_owned();
    let gst=gstreamer_root().ok_or("Media runtime is missing. Run the Rosorah installer again to repair setup.")?;
    let mut paths=vec![root.join("runtime/ffmpeg-9.0.2-essentials_build/bin"),gst.join("bin")];
    paths.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    let logs=PathBuf::from(env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is unavailable")?).join("RosorahStudio/logs");
    fs::create_dir_all(&logs)?;
    let log=logs.join("startup.log");
    let output=fs::File::create(&log)?;
    let status=Command::new(root.join("rosorah-studio.exe"))
        .args(env::args_os().skip(1)).current_dir(&root)
        .env("PATH",env::join_paths(paths)?)
        .env(GST_VAR,&gst)
        .env("GST_PLUGIN_PATH",gst.join("lib/gstreamer-1.0"))
        .stdin(Stdio::null()).stdout(output.try_clone()?).stderr(output)
        .creation_flags(0x08000000).status()?;
    if !status.success() { return Err(format!("Rosorah exited unexpectedly ({status}). Details: {}",log.display()).into()); }
    Ok(())
}
fn main(){if let Err(e)=run(){error(&e.to_string());std::process::exit(1)}}
