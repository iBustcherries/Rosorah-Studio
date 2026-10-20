//! Rosorah's output as a webcam other apps can pick (Discord, Zoom, browsers). On Linux it's a
//! v4l2loopback device named "Rosorah Virtual Camera"; on Windows 11 the Rosorah camera component
//! the installer registers reads frames from a pipe.
#[cfg(not(windows))]
pub const NAME: &str = "Rosorah Virtual Camera";

/// The device path when Rosorah's camera exists.
#[cfg(not(windows))]
pub fn device() -> Option<String> {
    std::fs::read_dir("/sys/class/video4linux").ok()?.flatten().find_map(|entry| {
        let name = std::fs::read_to_string(entry.path().join("name")).ok()?;
        (name.trim() == NAME).then(|| format!("/dev/{}", entry.file_name().to_string_lossy()))
    })
}
/// Whether the loopback kernel module is available to add a camera with.
#[cfg(not(windows))]
pub fn supported() -> bool {
    std::path::Path::new("/usr/bin/v4l2loopback-ctl").exists() || std::path::Path::new("/usr/local/bin/v4l2loopback-ctl").exists()
}
/// Adds Rosorah's camera (asks for the password once through the system prompt). It lasts until
/// the computer restarts; other loopback cameras (OBS's, DroidCam's) are left alone.
#[cfg(not(windows))]
pub fn set_up() -> anyhow::Result<()> {
    // The module may need loading first (it's usually loaded already when DroidCam or OBS ran).
    let script = format!("modprobe v4l2loopback 2>/dev/null; v4l2loopback-ctl add -n '{NAME}' -x 1 -b 2");
    let status = std::process::Command::new("pkexec").args(["sh", "-c", &script]).status()?;
    anyhow::ensure!(status.success(), "The camera wasn't added (the password prompt was cancelled or v4l2loopback isn't installed)");
    anyhow::ensure!(device().is_some(), "The camera was added but didn't appear");
    Ok(())
}
/// On Windows the camera is "there" once its component is set up; it only shows in apps while on.
#[cfg(windows)]
pub fn device() -> Option<String> {
    rosorah_windows::virtual_camera::installed().then(|| rosorah_windows::virtual_camera::PIPE.to_owned())
}
#[cfg(windows)]
pub fn supported() -> bool {
    rosorah_windows::virtual_camera::supported()
}
/// Whether the set-up camera component needs replacing with the one shipped with this Rosorah.
#[cfg(windows)]
pub fn outdated() -> bool {
    rosorah_windows::virtual_camera::installed() && !rosorah_windows::virtual_camera::up_to_date()
}
#[cfg(not(windows))]
pub fn outdated() -> bool {
    false
}
/// Copies the camera component into Program Files and registers it (Windows asks permission once).
#[cfg(windows)]
pub fn set_up() -> anyhow::Result<()> {
    rosorah_windows::virtual_camera::set_up()
}
