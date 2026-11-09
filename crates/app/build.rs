// On Windows, GStreamer's libraries are loaded only when first used, so Rosorah can find them
// itself (see windows_runtime in main.rs) instead of failing to start when they aren't on PATH.
const GSTREAMER: [&str; 7] = [
    "glib-2.0-0.dll",
    "gobject-2.0-0.dll",
    "gio-2.0-0.dll",
    "gstreamer-1.0-0.dll",
    "gstbase-1.0-0.dll",
    "gstapp-1.0-0.dll",
    "gstvideo-1.0-0.dll",
];

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        for dll in GSTREAMER {
            println!("cargo:rustc-link-arg-bin=rosorah-studio=/DELAYLOAD:{dll}");
        }
        println!("cargo:rustc-link-arg-bin=rosorah-studio=delayimp.lib");
        icon();
    }
}

/// Builds the app icon into rosorah-studio.exe, which the taskbar and Alt+Tab show for its window.
/// Needs the Windows SDK's rc.exe; the Linux cross build links the icon itself (build-cross.sh).
fn icon() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/windows");
    println!("cargo:rerun-if-changed={}", dir.join("rosorah.rc").display());
    println!("cargo:rerun-if-changed={}", dir.join("rosorah.ico").display());
    if !cfg!(windows) {
        return;
    }
    let kits = std::path::PathBuf::from(std::env::var_os("ProgramFiles(x86)").unwrap_or_default()).join(r"Windows Kits\10\bin");
    let mut found: Vec<_> = std::fs::read_dir(&kits)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path().join(r"x64\rc.exe"))
        .filter(|rc| rc.is_file())
        .collect();
    found.sort();
    let Some(rc) = found.pop() else {
        println!("cargo:warning=rc.exe not found: rosorah-studio.exe will have no icon");
        return;
    };
    let res = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("rosorah.res");
    let built = std::process::Command::new(rc)
        .current_dir(&dir)
        .args(["/nologo", "/fo"])
        .arg(&res)
        .arg("rosorah.rc")
        .status()
        .is_ok_and(|s| s.success());
    if built {
        println!("cargo:rustc-link-arg-bin=rosorah-studio={}", res.display());
    } else {
        println!("cargo:warning=The icon resource did not compile: rosorah-studio.exe will have no icon");
    }
}
