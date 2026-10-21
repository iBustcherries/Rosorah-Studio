//! Looks for a newer Rosorah on the public releases page and fetches the installer that fits
//! this computer (Windows .exe, Fedora .rpm or Ubuntu/Debian .deb).
use anyhow::{Context, Result, bail, ensure};
use eframe::egui;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

/// Public, installers only; the source lives in a private repository.
pub const REPO: &str = "iBustcherries/RosorahStudio-releases";

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub page: String,
    pub notes: String,
    /// The installer for this computer: file name, download address and size.
    pub asset: Option<(String, String, u64)>,
    pub sums: Option<String>,
}
#[derive(Clone, Debug, Default)]
pub enum State {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Downloading(Release, u64, u64),
    Downloaded(Release, PathBuf),
    /// Linux: the package manager is installing it, after the system password prompt.
    Installing(Release),
    /// Linux: installed; a restart starts the new version.
    Installed(Release),
    /// Linux: the install didn't happen (declined or failed); the download is kept to retry.
    InstallFailed(Release, PathBuf, String),
    Failed(String),
}
#[derive(Default)]
pub struct Updates {
    state: Arc<Mutex<State>>,
}

/// 0.32.0-preview.10 → (0, 32, 0, 10); a full release sorts after all of its previews.
fn version_key(text: &str) -> Option<(u64, u64, u64, u64)> {
    let text = text.trim().trim_start_matches('v');
    let (main, pre) = text.split_once('-').map_or((text, None), |(m, p)| (m, Some(p)));
    let mut parts = main.split('.').map(|p| p.parse::<u64>().ok());
    let key = (parts.next()??, parts.next()??, parts.next()??);
    let pre = match pre {
        None => u64::MAX,
        Some(p) => p.rsplit('.').next()?.parse().ok()?,
    };
    Some((key.0, key.1, key.2, pre))
}
pub fn newer(candidate: &str, current: &str) -> bool {
    matches!((version_key(candidate), version_key(current)), (Some(a), Some(b)) if a > b)
}
/// Which installer suits this computer: its file name ending.
fn wanted() -> Option<&'static str> {
    if cfg!(windows) {
        return Some("-Setup.exe");
    }
    let release = std::fs::read_to_string("/etc/os-release").unwrap_or_default().to_lowercase();
    let family: String = release.lines().filter(|l| l.starts_with("id=") || l.starts_with("id_like=")).collect::<Vec<_>>().join(" ");
    if ["fedora", "rhel", "centos"].iter().any(|d| family.contains(d)) {
        Some(".rpm")
    } else if ["debian", "ubuntu"].iter().any(|d| family.contains(d)) {
        Some(".deb")
    } else {
        None
    }
}
fn client(timeout: u64) -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(timeout))
        .user_agent(concat!("RosorahStudio/", env!("CARGO_PKG_VERSION")))
        .build()?)
}
fn latest(current: &str) -> Result<Option<Release>> {
    let list: Value = client(20)?
        .get(format!("https://api.github.com/repos/{REPO}/releases?per_page=20"))
        .header("Accept", "application/vnd.github+json")
        .send()?
        .error_for_status()
        .context("The releases page didn't answer")?
        .json()?;
    let best = list
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["draft"] != true)
        .filter_map(|r| Some((version_key(r["tag_name"].as_str()?)?, r)))
        .max_by_key(|(key, _)| *key);
    let Some((_, release)) = best else { return Ok(None) };
    let version = release["tag_name"].as_str().unwrap_or_default().trim_start_matches('v').to_owned();
    if !newer(&version, current) {
        return Ok(None);
    }
    let assets: Vec<&Value> = release["assets"].as_array().into_iter().flatten().collect();
    let url = |a: &Value| a["browser_download_url"].as_str().unwrap_or_default().to_owned();
    let asset = wanted().and_then(|end| {
        assets
            .iter()
            .find(|a| a["name"].as_str().is_some_and(|n| n.ends_with(end)))
            .map(|a| (a["name"].as_str().unwrap_or_default().to_owned(), url(a), a["size"].as_u64().unwrap_or(0)))
    });
    Ok(Some(Release {
        version,
        page: release["html_url"].as_str().unwrap_or_default().to_owned(),
        notes: release["body"].as_str().unwrap_or_default().chars().take(6000).collect(),
        asset,
        sums: assets.iter().find(|a| a["name"] == "SHA256SUMS.txt").map(|a| url(a)),
    }))
}
fn downloads_folder() -> PathBuf {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    home.map(|h| h.join("Downloads")).filter(|d| d.is_dir()).unwrap_or_else(std::env::temp_dir)
}
fn download(release: &Release, state: &Mutex<State>, ctx: &egui::Context) -> Result<PathBuf> {
    let Some((name, url, size)) = &release.asset else { bail!("There's no installer for this system in this release") };
    ensure!(!name.contains(['/', '\\']) && !name.starts_with('.'), "Unexpected installer name");
    let client = client(1800)?;
    let expected = match &release.sums {
        Some(sums) => {
            let text = client.get(sums).send()?.error_for_status()?.text()?;
            text.lines().find_map(|l| l.split_once("  ").filter(|(_, n)| n.trim() == name).map(|(h, _)| h.trim().to_lowercase()))
        }
        None => None,
    };
    let folder = downloads_folder();
    let path = folder.join(name);
    let part = folder.join(format!("{name}.part"));
    let mut response = client.get(url).send()?.error_for_status()?;
    let total = response.content_length().unwrap_or(*size);
    let mut file = std::fs::File::create(&part)?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 256 * 1024];
    let mut done = 0u64;
    loop {
        let n = response.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        file.write_all(&buffer[..n])?;
        hash.update(&buffer[..n]);
        done += n as u64;
        *state.lock().unwrap() = State::Downloading(release.clone(), done, total);
        ctx.request_repaint();
    }
    file.sync_all()?;
    drop(file);
    let actual: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if let Some(expected) = expected
        && expected != actual
    {
        let _ = std::fs::remove_file(&part);
        bail!("The download was damaged (its checksum doesn't match). Try again.");
    }
    std::fs::rename(&part, &path)?;
    Ok(path)
}
impl Updates {
    pub fn state(&self) -> State {
        self.state.lock().unwrap().clone()
    }
    pub fn check(&self, ctx: &egui::Context) {
        self.check_as(ctx, env!("CARGO_PKG_VERSION"));
    }
    /// Checks as if this version were installed (`--page update` uses an old one for screenshots).
    pub fn check_as(&self, ctx: &egui::Context, current: &'static str) {
        if matches!(self.state(), State::Checking | State::Downloading(..)) {
            return;
        }
        *self.state.lock().unwrap() = State::Checking;
        let (state, ctx) = (self.state.clone(), ctx.clone());
        std::thread::spawn(move || {
            let result = latest(current);
            *state.lock().unwrap() = match result {
                Ok(Some(release)) => State::Available(release),
                Ok(None) => State::UpToDate,
                Err(e) => State::Failed(format!("Couldn't check for updates: {e:#}")),
            };
            ctx.request_repaint();
        });
    }
    pub fn download(&self, ctx: &egui::Context, release: Release) {
        *self.state.lock().unwrap() = State::Downloading(release.clone(), 0, 0);
        let (state, ctx) = (self.state.clone(), ctx.clone());
        std::thread::spawn(move || {
            let result = download(&release, &state, &ctx);
            *state.lock().unwrap() = match result {
                Ok(path) => State::Downloaded(release, path),
                Err(e) => State::Failed(format!("The update didn't download: {e:#}")),
            };
            ctx.request_repaint();
        });
    }
}
impl Updates {
    /// Installs a downloaded .rpm or .deb with the system's package manager, asking for the
    /// password through the desktop's usual prompt (pkexec).
    pub fn install(&self, ctx: &egui::Context, release: Release, path: PathBuf) {
        *self.state.lock().unwrap() = State::Installing(release.clone());
        let (state, ctx) = (self.state.clone(), ctx.clone());
        std::thread::spawn(move || {
            let tool: &[&str] = if path.extension().is_some_and(|e| e == "deb") { &["apt", "install", "-y"] } else { &["dnf", "install", "-y"] };
            let status = std::process::Command::new("pkexec").args(tool).arg(&path).status();
            *state.lock().unwrap() = match status {
                Ok(s) if s.success() => State::Installed(release),
                // pkexec: 126 the prompt was dismissed, 127 not authorized.
                Ok(s) if matches!(s.code(), Some(126 | 127)) => State::InstallFailed(release, path, "The install was cancelled at the password prompt.".into()),
                Ok(s) => State::InstallFailed(release, path, format!("The package manager stopped with code {}.", s.code().unwrap_or(-1))),
                Err(e) => State::InstallFailed(release, path, format!("Couldn't ask for permission to install ({e}).")),
            };
            ctx.request_repaint();
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn previews_sort_by_number_and_before_the_release() {
        assert!(newer("0.32.0-preview.10", "0.32.0-preview.9"));
        assert!(newer("v0.32.0", "0.32.0-preview.10"));
        assert!(newer("0.33.0-preview.1", "0.32.0"));
        assert!(!newer("0.32.0-preview.9", "0.32.0-preview.10"));
        assert!(!newer("0.32.0-preview.10", "0.32.0-preview.10"));
        assert!(!newer("nonsense", "0.32.0"));
    }
    /// Talks to GitHub: cargo test -- --ignored live_release_check
    #[test]
    #[ignore]
    fn live_release_check() {
        let release = latest("0.32.0-preview.9").unwrap().expect("a newer release");
        assert!(newer(&release.version, "0.32.0-preview.9"));
        let (name, url, size) = release.asset.expect("an installer for this system");
        assert!(size > 1_000_000 && url.starts_with("https://github.com/"), "{name} {url}");
        assert!(release.sums.is_some());
        println!("found {} → {name}", release.version);
        assert!(latest(env!("CARGO_PKG_VERSION")).unwrap().is_none_or(|r| newer(&r.version, env!("CARGO_PKG_VERSION"))));
    }
}
