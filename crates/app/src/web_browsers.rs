//! The Chromium browsers installed on this computer, so a co-stream invite can be accepted in one
//! of them rather than the default browser (Twitch's Stream Together needs camera and mic access
//! that some browsers handle better than others).
use std::{path::PathBuf, process::Command, sync::OnceLock};

#[derive(Clone, Debug)]
pub struct Browser {
    pub name: &'static str,
    program: PathBuf,
    /// Arguments before the address (`flatpak run <app id>`).
    args: Vec<String>,
}

/// Installed browsers, looked for once.
pub fn installed() -> &'static [Browser] {
    static FOUND: OnceLock<Vec<Browser>> = OnceLock::new();
    FOUND.get_or_init(find)
}

/// Opens `url` in the browser named `choice`, or in the default browser when it's empty or gone.
pub fn open(choice: &str, url: &str) -> std::io::Result<()> {
    match installed().iter().find(|b| b.name == choice) {
        Some(b) => Command::new(&b.program).args(&b.args).arg(url).spawn().map(|_| ()),
        None => crate::desktop::open(url).map(|_| ()),
    }
}

#[cfg(windows)]
fn find() -> Vec<Browser> {
    let env = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let (local, files, files86) = (env("LOCALAPPDATA"), env("ProgramFiles"), env("ProgramFiles(x86)"));
    let places: [(&str, &str); 7] = [
        ("Chrome", r"Google\Chrome\Application\chrome.exe"),
        ("Edge", r"Microsoft\Edge\Application\msedge.exe"),
        ("Brave", r"BraveSoftware\Brave-Browser\Application\brave.exe"),
        ("Vivaldi", r"Vivaldi\Application\vivaldi.exe"),
        ("Opera", r"Programs\Opera\opera.exe"),
        ("Opera GX", r"Programs\Opera GX\opera.exe"),
        ("Chromium", r"Chromium\Application\chrome.exe"),
    ];
    places
        .into_iter()
        .filter_map(|(name, path)| {
            let program = [&files, &files86, &local].into_iter().flatten().map(|root| root.join(path)).find(|p| p.is_file())?;
            Some(Browser { name, program, args: Vec::new() })
        })
        .collect()
}

#[cfg(not(windows))]
fn find() -> Vec<Browser> {
    let browsers: [(&str, &[&str], &str); 6] = [
        ("Chrome", &["google-chrome-stable", "google-chrome"], "com.google.Chrome"),
        ("Edge", &["microsoft-edge-stable", "microsoft-edge"], "com.microsoft.Edge"),
        ("Brave", &["brave-browser", "brave"], "com.brave.Browser"),
        ("Vivaldi", &["vivaldi-stable", "vivaldi"], "com.vivaldi.Vivaldi"),
        ("Opera", &["opera"], "com.opera.Opera"),
        ("Chromium", &["chromium-browser", "chromium"], "org.chromium.Chromium"),
    ];
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    browsers
        .into_iter()
        .filter_map(|(name, commands, flatpak)| {
            if let Some(command) = commands.iter().find(|c| crate::desktop::command_available(c)) {
                return Some(Browser { name, program: command.into(), args: Vec::new() });
            }
            let installed = [PathBuf::from("/var/lib/flatpak/app"), home.join(".local/share/flatpak/app")].iter().any(|d| d.join(flatpak).is_dir());
            installed.then(|| Browser { name, program: "flatpak".into(), args: vec!["run".into(), flatpak.into()] })
        })
        .collect()
}
