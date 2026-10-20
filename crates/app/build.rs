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
    }
}
