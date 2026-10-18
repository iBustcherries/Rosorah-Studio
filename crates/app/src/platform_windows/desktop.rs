//! Windows dialogs; no shell interpretation of user paths or URLs.
use anyhow::Result;
use std::path::PathBuf;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Session {
    Windows,
}
pub fn session() -> Session {
    Session::Windows
}
pub fn command_available(name: &str) -> bool {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|v| std::env::split_paths(&v).collect::<Vec<_>>())
        .any(|p| p.join(name).is_file() || p.join(format!("{name}.exe")).is_file())
}
#[derive(Clone, Debug)]
pub struct Report {
    pub desktop: String,
    pub session: Session,
    pub sources: Option<u32>,
    pub cursors: Option<u32>,
    pub portal_error: String,
    pub file_chooser: bool,
    pub wallet: bool,
    pub commands: Vec<(&'static str, bool)>,
    pub checking: bool,
}
pub fn report() -> Report {
    Report {
        desktop: "Windows".into(),
        session: session(),
        sources: Some(3),
        cursors: Some(3),
        portal_error: String::new(),
        file_chooser: true,
        wallet: true,
        commands: vec![("ffmpeg", command_available("ffmpeg"))],
        checking: false,
    }
}
static WINDOWS:std::sync::OnceLock<std::sync::Mutex<Vec<rosorah_windows::window::Window>>>=std::sync::OnceLock::new();
pub fn windows()->Vec<rosorah_windows::window::Window>{WINDOWS.get_or_init(Default::default).lock().unwrap().clone()}
type Monitors = (Vec<rosorah_windows::capture::Monitor>, String);
static MONITORS: std::sync::OnceLock<std::sync::Mutex<Monitors>> = std::sync::OnceLock::new();
static PROBING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub fn probing() -> bool { PROBING.load(std::sync::atomic::Ordering::Relaxed) }
pub fn monitors() -> Monitors {
    MONITORS
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .clone()
}
pub fn refresh() {
    use std::sync::atomic::Ordering;
    if PROBING.swap(true, Ordering::Relaxed) {
        return;
    }
    std::thread::spawn(|| {
        *WINDOWS.get_or_init(Default::default).lock().unwrap()=rosorah_windows::window::windows().unwrap_or_default();
        let result = match rosorah_windows::capture::monitors() {
            Ok(m) => (m, String::new()),
            Err(e) => (vec![], e.to_string()),
        };
        *MONITORS.get_or_init(Default::default).lock().unwrap() = result;
        PROBING.store(false, Ordering::Relaxed);
    });
}
#[derive(Clone, Copy)]
pub enum Pick {
    Open,
    Save,
    Folder,
}
pub fn pick(mode: Pick, title: &str, suggested: &str) -> Result<Option<PathBuf>> {
    let dialog = rfd::FileDialog::new()
        .set_title(title)
        .set_file_name(suggested);
    Ok(match mode {
        Pick::Open => dialog.pick_file(),
        Pick::Save => dialog.save_file(),
        Pick::Folder => dialog.pick_folder(),
    })
}
pub struct Opened;
impl Opened {
    pub fn wait(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub fn open(target: impl AsRef<std::ffi::OsStr>) -> std::io::Result<Opened> {
    rosorah_windows::shell::open(target.as_ref())
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(Opened)
}
