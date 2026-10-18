//! One live encoder per canvas feeds independent, bounded destination workers.
//! Keys are kept outside shareable projects and never included in diagnostics. They are
//! stored in the system keyring when one is available, otherwise in a private settings file.
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
mod relay;
use anyhow::{Context, Result, ensure};
use auroracast_core::Project;
pub use relay::Hub;
use rosorah_capture::FrameReader;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

/// The keys last read from or written to the keyring; None when no keyring is available.
static KEYRING: Mutex<Option<std::collections::BTreeMap<String, String>>> = Mutex::new(None);

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Destination {
    pub enabled: bool,
    pub server: String,
    pub key: String,
    pub managed_by_account: bool,
    pub canvas: Option<auroracast_core::Id>,
    /// This destination's own video bitrate; None uses the shared Output bitrate.
    pub bitrate: Option<u32>,
}
#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider {
    Kick,
    TikTok,
    #[default]
    Custom,
}
impl Provider {
    pub fn name(self) -> &'static str {
        match self {
            Self::Kick => "Kick",
            Self::TikTok => "TikTok",
            Self::Custom => "Custom RTMP",
        }
    }
    pub fn dashboard(self) -> &'static str {
        match self {
            Self::Kick => "https://dashboard.kick.com/",
            Self::TikTok => "https://www.tiktok.com/studio",
            Self::Custom => "",
        }
    }
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Service {
    pub id: u64,
    pub provider: Provider,
    pub name: String,
    pub destination: Destination,
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub connection_names: [String; 3],
    pub twitch: Destination,
    pub youtube: Destination,
    pub youtube_vertical: Destination,
    pub services: Vec<Service>,
    pub bitrate: u32,
    pub record: bool,
    /// TikTok username whose LIVE page chat is read from (no sign-in).
    pub tiktok_username: String,
    /// Opt-in: type outgoing messages into TikTok's chat box.
    pub tiktok_send: bool,
    /// "I'm live" posts to Discord and Bluesky; its two secrets live in the keyring.
    pub announce: crate::announce::Settings,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            connection_names: ["Twitch".into(), "YouTube".into(), "YouTube 2".into()],
            twitch: Destination {
                server: "rtmp://ingest.global-contribute.live-video.net/app".into(),
                ..Default::default()
            },
            youtube: Destination {
                server: "rtmps://a.rtmps.youtube.com:443/live2".into(),
                ..Default::default()
            },
            youtube_vertical: Destination {
                server: "rtmps://a.rtmps.youtube.com:443/live2".into(),
                ..Default::default()
            },
            services: Vec::new(),
            bitrate: 6000,
            record: false,
            tiktok_username: String::new(),
            tiktok_send: false,
            announce: Default::default(),
        }
    }
}
impl Config {
    pub fn connection_name(&self, slot: usize) -> String {
        if slot < 3 {
            self.connection_names[slot].clone()
        } else {
            self.services
                .get(slot - 3)
                .map(|s| s.name.clone())
                .unwrap_or_default()
        }
    }
    pub fn connection_name_mut(&mut self, slot: usize) -> Option<&mut String> {
        if slot < 3 {
            self.connection_names.get_mut(slot)
        } else {
            self.services.get_mut(slot - 3).map(|s| &mut s.name)
        }
    }

    pub fn add_service(&mut self, provider: Provider) -> Option<usize> {
        if self.services.len() >= 16 {
            return None;
        }
        let id = self
            .services
            .iter()
            .map(|s| s.id)
            .max()
            .unwrap_or(0)
            .checked_add(1)?;
        self.services.push(Service {
            id,
            provider,
            name: provider.name().into(),
            ..Default::default()
        });
        Some(self.services.len() + 2)
    }
    pub fn destination_mut(&mut self, slot: usize) -> Option<&mut Destination> {
        match slot {
            0 => Some(&mut self.twitch),
            1 => Some(&mut self.youtube),
            2 => Some(&mut self.youtube_vertical),
            _ => self.services.get_mut(slot - 3).map(|s| &mut s.destination),
        }
    }
    pub fn path() -> PathBuf {
        auroracast_core::platform::config_root()
            .join("rosorah-studio/streaming.json")
    }
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let bytes = fs::read(path).context("Read streaming settings")?;
        ensure!(bytes.len() < 64 * 1024, "Streaming settings are too large");
        serde_json::from_slice(&bytes).context("Streaming settings could not be read")
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        let parent = path.parent().context("Streaming settings need a folder")?;
        fs::create_dir_all(parent)?;
        let temp = path.with_extension(format!("{}.tmp", std::process::id()));
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)] options.mode(0o600);
        let mut file = options.open(&temp)
            .context("Save streaming settings")?;
        let result = (|| -> Result<()> {
            file.write_all(&serde_json::to_vec_pretty(self)?)?;
            file.sync_all()?;
            #[cfg(unix)]
            fs::set_permissions(&temp, fs::Permissions::from_mode(0o600))?;
            fs::rename(&temp, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result
    }
    /// Every non-empty stream key, by a name that stays stable when services are reordered.
    fn keys(&self) -> std::collections::BTreeMap<String, String> {
        let mut keys = std::collections::BTreeMap::new();
        for (name, d) in [("twitch", &self.twitch), ("youtube", &self.youtube), ("youtube_vertical", &self.youtube_vertical)] {
            if !d.key.is_empty() {
                keys.insert(name.to_string(), d.key.clone());
            }
        }
        for s in &self.services {
            if !s.destination.key.is_empty() {
                keys.insert(format!("service-{}", s.id), s.destination.key.clone());
            }
        }
        for (name, secret) in [("discord-webhook", &self.announce.discord_webhook), ("bluesky-password", &self.announce.bluesky_password)] {
            if !secret.is_empty() {
                keys.insert(name.to_string(), secret.clone());
            }
        }
        keys
    }
    fn fill_keys(&mut self, keys: &std::collections::BTreeMap<String, String>) {
        for (name, d) in [("twitch", &mut self.twitch), ("youtube", &mut self.youtube), ("youtube_vertical", &mut self.youtube_vertical)] {
            if d.key.is_empty() && let Some(k) = keys.get(name) {
                d.key = k.clone();
            }
        }
        for s in &mut self.services {
            if s.destination.key.is_empty() && let Some(k) = keys.get(&format!("service-{}", s.id)) {
                s.destination.key = k.clone();
            }
        }
        for (name, secret) in [("discord-webhook", &mut self.announce.discord_webhook), ("bluesky-password", &mut self.announce.bluesky_password)] {
            if secret.is_empty() && let Some(k) = keys.get(name) {
                *secret = k.clone();
            }
        }
    }
    fn without_keys(&self) -> Self {
        let mut c = self.clone();
        for d in [&mut c.twitch, &mut c.youtube, &mut c.youtube_vertical] {
            d.key.clear();
        }
        for s in &mut c.services {
            s.destination.key.clear();
        }
        c.announce.discord_webhook.clear();
        c.announce.bluesky_password.clear();
        c
    }
    /// Loads settings and their keys from the keyring. Keys found in the file (from older
    /// versions, or saved without a keyring) are moved into the keyring when it works.
    pub fn load_with_keyring(path: &Path) -> Result<Self> {
        let mut config = Self::load(path)?;
        let cancel = AtomicBool::new(false);
        let Ok(stored) = crate::accounts::stream_keys_secret(None, false, &cancel) else {
            *KEYRING.lock().unwrap() = None;
            return Ok(config);
        };
        let stored: std::collections::BTreeMap<String, String> =
            stored.and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default();
        let in_file = !config.keys().is_empty();
        config.fill_keys(&stored);
        *KEYRING.lock().unwrap() = Some(stored);
        if in_file {
            let _ = config.save_with_keyring(path);
        }
        Ok(config)
    }
    /// Saves settings; returns whether the keys went to the keyring rather than the file.
    pub fn save_with_keyring(&self, path: &Path) -> Result<bool> {
        let keys = self.keys();
        let mut stored = KEYRING.lock().unwrap();
        let in_keyring = match stored.as_ref() {
            Some(previous) if *previous == keys => true,
            Some(_) => {
                let cancel = AtomicBool::new(false);
                let written = if keys.is_empty() {
                    crate::accounts::stream_keys_secret(None, true, &cancel)
                } else {
                    crate::accounts::stream_keys_secret(Some(&serde_json::to_string(&keys)?), false, &cancel)
                };
                written.is_ok()
            }
            None => false,
        };
        if in_keyring {
            *stored = Some(keys);
            self.without_keys().save(path)?;
        } else {
            self.save(path)?;
        }
        Ok(in_keyring)
    }
    fn targets(&self) -> Result<Vec<Target>> {
        ensure!(
            (1000..=20_000).contains(&self.bitrate),
            "Choose a bitrate between 1,000 and 20,000 kbps"
        );
        if self.twitch.enabled {
            ensure!(
                self.twitch.bitrate.unwrap_or(self.bitrate) <= 6000,
                "Use 6,000 kbps or less for Twitch"
            );
        }
        let mut targets = Vec::new();
        ensure!(
            self.services.len() <= 16,
            "A maximum of 16 additional services is supported"
        );
        let mut entries = vec![
            (self.connection_name(0), &self.twitch),
            (self.connection_name(1), &self.youtube),
            (self.connection_name(2), &self.youtube_vertical),
        ];
        for service in &self.services {
            if !service.destination.enabled {
                continue;
            }
            ensure!(
                service.name.chars().count() <= 64
                    && !service.name.trim().is_empty()
                    && !service.name.chars().any(char::is_control),
                "Use a service name of 1–64 characters"
            );
            entries.push((
                format!("{} · {}", service.name, service.id),
                &service.destination,
            ));
        }
        for (name, dest) in entries {
            if !dest.enabled {
                continue;
            }
            ensure!(
                !name.trim().is_empty()
                    && name.chars().count() <= 64
                    && !name.chars().any(char::is_control),
                "Use a connection name of 1–64 characters"
            );
            ensure!(
                dest.bitrate.is_none_or(|b| (1000..=20_000).contains(&b)),
                "Choose a {name} bitrate between 1,000 and 20,000 kbps"
            );
            let key = dest.key.trim();
            ensure!(
                !key.is_empty(),
                "Enter your {name} stream key in Streaming settings"
            );
            ensure!(
                key.len() <= 1024
                    && key
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-?=&/%.:+~".contains(&b)),
                "{name} stream key contains invalid characters"
            );
            let server = dest.server.trim().trim_end_matches('/');
            let rest = server
                .strip_prefix("rtmps://")
                .or_else(|| server.strip_prefix("rtmp://"))
                .context(format!("{name} server must start with rtmp:// or rtmps://"))?;
            let (host, app) = rest
                .split_once('/')
                .context(format!("{name} server needs an application path"))?;
            ensure!(
                !host.is_empty()
                    && !app.is_empty()
                    && server.len() <= 2048
                    && server
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b":/.-_[]".contains(&b)),
                "Invalid {name} server address"
            );
            targets.push(Target {
                name,
                destination: Location::Network(format!("{server}/{key}")),
                secret: key.into(),
            });
        }
        ensure!(
            !targets.is_empty(),
            "Choose where to stream: switch on a platform in Settings > Accounts"
        );
        for (index, target) in targets.iter().enumerate() {
            if let Location::Network(url) = &target.destination {
                ensure!(!targets[..index].iter().any(|other|matches!(&other.destination,Location::Network(previous) if previous==url)),"Two enabled services use the same stream endpoint. Use separate stream keys.");
            }
        }
        if self.youtube.enabled && self.youtube_vertical.enabled {
            ensure!(
                self.youtube.key.trim() != self.youtube_vertical.key.trim(),
                "The two YouTube connections need different stream keys. Choose the second key in YouTube Studio's vertical Encoder settings."
            );
        }
        Ok(targets)
    }

    fn canvas_groups(&self, project: &Project, fallback: usize) -> Result<Vec<(usize, Config)>> {
        // Validate every destination and canvas before any encoder or network worker starts.
        self.targets()?;
        let main = project
            .canvases
            .iter()
            .position(|c| c.width >= c.height)
            .unwrap_or(fallback);
        let vertical = project.canvases.iter().position(|c| c.height > c.width);
        let mut groups: Vec<(usize, Config)> = Vec::new();
        for (slot, dest) in [&self.twitch, &self.youtube, &self.youtube_vertical]
            .into_iter()
            .chain(self.services.iter().map(|s| &s.destination))
            .enumerate()
        {
            if !dest.enabled {
                continue;
            }
            let index = if let Some(id) = dest.canvas {
                project.canvases.iter().position(|c| c.id == id).context("A streaming destination's canvas is missing. Choose its video again in Settings > Accounts.")?
            } else if slot == 2 {
                vertical.context("Choose an existing canvas for the second YouTube connection")?
            } else {
                main
            };
            let c = project
                .canvases
                .get(index)
                .context("Choose an output canvas")?;
            ensure!(
                c.fps <= 60
                    && c.width <= 1920
                    && c.height <= 1920
                    && c.width.min(c.height) <= 1080
                    && c.width.is_multiple_of(2)
                    && c.height.is_multiple_of(2),
                "Canvas {} needs even dimensions, up to 1920×1080 (or portrait) at 60 FPS",
                c.name
            );
            // Destinations share an encoder when they use the same canvas and bitrate.
            let bitrate = dest.bitrate.unwrap_or(self.bitrate);
            let group = if let Some(i) = groups.iter().position(|(i, g)| *i == index && g.bitrate == bitrate) {
                i
            } else {
                groups.push((
                    index,
                    Config {
                        connection_names: self.connection_names.clone(),
                        bitrate,
                        // One recording per canvas, from its first encoder.
                        record: self.record && !groups.iter().any(|(i, _)| *i == index),
                        ..Default::default()
                    },
                ));
                groups.len() - 1
            };
            if slot < 3 {
                *groups[group].1.destination_mut(slot).unwrap() = dest.clone();
            } else {
                groups[group]
                    .1
                    .services
                    .push(self.services[slot - 3].clone());
            }
        }
        Ok(groups)
    }
}
#[derive(Clone)]
pub(super) enum Location {
    Network(String),
    File(PathBuf),
}
#[derive(Clone)]
pub(super) struct Target {
    pub name: String,
    pub destination: Location,
    pub secret: String,
}
impl Target {
    fn redact(&self, text: &str) -> String {
        let mut clean = text.to_owned();
        if let Location::Network(url) = &self.destination {
            clean = clean.replace(url, "[stream address]");
        }
        if !self.secret.is_empty() {
            clean = clean.replace(&self.secret, "[stream key]");
        }
        clean
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Preparing,
    Connecting,
    Sending,
    Reconnecting,
    Stopped,
    Failed,
}
impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Preparing => "Preparing",
            Self::Connecting => "Connecting",
            Self::Sending => "Sending",
            Self::Reconnecting => "Reconnecting",
            Self::Stopped => "Stopped",
            Self::Failed => "Failed",
        }
    }
}
#[derive(Clone)]
pub struct DestinationView {
    pub name: String,
    pub phase: Phase,
    pub attempts: u32,
    pub elapsed: std::time::Duration,
    pub detail: String,
    pub path: Option<PathBuf>,
}
pub struct State {
    pub destinations: Mutex<Vec<DestinationView>>,
    pub stopping: AtomicBool,
    pub retry_now:Mutex<std::collections::BTreeSet<usize>>,
}
impl State {
    pub fn view(&self) -> Vec<DestinationView> {
        self.destinations.lock().unwrap().clone()
    }
    pub fn stop(&self) {
        self.stopping.store(true, Ordering::Relaxed);
    }
}
#[derive(Clone)]
pub struct Setup {
    pub(super) targets: Vec<Target>,
    pub bitrate: u32,
    pub state: Arc<State>,
    pub recording: Option<PathBuf>,
}
impl Setup {
    fn new(config: &Config, folder: &Path) -> Result<Self> {
        let mut targets = config.targets()?;
        let recording = if config.record {
            let path = crate::recording::output_path(folder)?;
            targets.push(Target {
                name: "Recording".into(),
                destination: Location::File(path.clone()),
                secret: String::new(),
            });
            Some(path)
        } else {
            None
        };
        let destinations = targets
            .iter()
            .map(|t| DestinationView {
                name: t.name.clone(),
                phase: Phase::Preparing,
                attempts: 0,
                elapsed: Default::default(),
                detail: String::new(),
                path: match &t.destination {
                    Location::File(p) => Some(p.clone()),
                    _ => None,
                },
            })
            .collect();
        Ok(Self {
            targets,
            bitrate: config.bitrate,
            state: Arc::new(State {
                destinations: Mutex::new(destinations),
                stopping: AtomicBool::new(false),
                retry_now:Mutex::default(),
            }),
            recording,
        })
    }
}
pub struct Stream {
    pub engine: crate::recording::Recording,
    pub state: Arc<State>,
    others: Vec<(crate::recording::Recording, Arc<State>)>,
    rates: Vec<u32>,
}
impl Stream {
    pub fn start(
        project: &Project,
        canvas: usize,
        frames: FrameReader,
        config: &Config,
        folder: &Path,
    ) -> Result<Self> {
        let groups = config.canvas_groups(project, canvas)?;
        let setups = groups
            .into_iter()
            .map(|(index, group)| Setup::new(&group, folder).map(|setup| (index, setup)))
            .collect::<Result<Vec<_>>>()?;
        Self::start_prepared(project, frames, setups)
    }
    fn start_prepared(
        project: &Project,
        frames: FrameReader,
        setups: Vec<(usize, Setup)>,
    ) -> Result<Self> {
        let mut stream: Option<Self> = None;
        for (index, setup) in setups {
            let state = setup.state.clone();
            let rate = setup.bitrate;
            let engine =
                crate::recording::Recording::start_stream(project, index, frames.clone(), setup)?;
            if let Some(stream) = &mut stream {
                stream.others.push((engine, state));
                stream.rates.push(rate);
            } else {
                stream = Some(Self {
                    engine,
                    state,
                    others: Vec::new(),
                    rates: vec![rate],
                });
            }
        }
        stream.context("Enable a streaming destination")
    }
    fn engines(&self) -> impl Iterator<Item = &crate::recording::Recording> {
        std::iter::once(&self.engine).chain(self.others.iter().map(|(engine, _)| engine))
    }
    pub fn outlets(&self) -> Vec<crate::recording::AudioOutlet> {
        self.engines().map(|e| e.outlet()).collect()
    }
    pub fn update_project(&self, project: &Project) {
        for engine in self.engines() {
            engine.update_project(project);
        }
    }
    pub fn retry_destination(&self,mut index:usize){for state in std::iter::once(&self.state).chain(self.others.iter().map(|(_,s)|s)){let n=state.destinations.lock().unwrap().len();if index<n{state.retry_now.lock().unwrap().insert(index);return}index-=n;}}
    pub fn bitrates(&self) -> String {
        let mut rates = self.rates.clone();
        rates.sort_unstable();
        rates.dedup();
        rates.iter().map(|b| format!("{b} kbps")).collect::<Vec<_>>().join(" + ")
    }
    pub fn canvas_names(&self) -> String {
        self.engines()
            .map(|e| e.canvas.name.as_str())
            .collect::<Vec<_>>()
            .join(" + ")
    }
    pub fn destinations(&self) -> Vec<DestinationView> {
        std::iter::once((&self.engine, &self.state))
            .chain(self.others.iter().map(|(e, s)| (e, s)))
            .flat_map(|(engine, state)| {
                state
                    .view()
                    .into_iter()
                    .map(|mut v| {
                        v.name = format!("{} > {}", v.name, engine.canvas.name);
                        if let Some(Err(error)) = engine.view().result {
                            v.phase = Phase::Failed;
                            v.detail = error;
                        }
                        v
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }
    pub fn view(&self) -> crate::recording::View {
        let views: Vec<_> = self.engines().map(|e| e.view()).collect();
        let mut view = views
            .iter()
            .find(|v| v.result.is_none())
            .unwrap_or(&views[0])
            .clone();
        view.render_ms=views.iter().map(|v|v.render_ms).fold(0.,f32::max);
        view.missed_frames=views.iter().map(|v|v.missed_frames).sum();
        view.queue_drops=views.iter().map(|v|v.queue_drops).sum();
        view.encoder_lag_ms=views.iter().map(|v|v.encoder_lag_ms).max().unwrap_or(0);
        view.details = self
            .engines()
            .zip(&views)
            .map(|(e, v)| format!("{}: {}", e.canvas.name, v.details))
            .collect::<Vec<_>>()
            .join("\n");
        view.encoder = self
            .engines()
            .zip(&views)
            .map(|(e, v)| {
                format!(
                    "{}: {}",
                    e.canvas.name,
                    if v.encoder.is_empty() {
                        "Preparing encoder…"
                    } else {
                        &v.encoder
                    }
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        if views.iter().all(|v| v.result.is_some()) {
            view.result = Some(
                views
                    .iter()
                    .find_map(|v| v.result.as_ref().and_then(|r| r.as_ref().err()).cloned())
                    .map_or_else(|| Ok(view.path.clone()), Err),
            );
        } else {
            view.result = None;
        }
        view
    }
    pub fn stop(&self) {
        self.state.stop();
        self.engine.stop();
        for (engine, state) in &self.others {
            state.stop();
            engine.stop();
        }
    }
}
impl Drop for Stream {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_destinations_and_keeps_secrets_out_of_diagnostics() {
        let mut c = Config::default();
        assert!(c.targets().is_err());
        c.twitch.enabled = true;
        assert!(c.targets().is_err());
        c.twitch.key = "live_private_key".into();
        let t = c.targets().unwrap().remove(0);
        let Location::Network(url) = &t.destination else {
            panic!()
        };
        assert_eq!(
            t.redact(&format!("Failed {url}: live_private_key")),
            "Failed [stream address]: [stream key]"
        );
        c.twitch.server = "file:///tmp/overwrite".into();
        assert!(c.targets().is_err());
        c.twitch.server = "rtmp://example/app|file:/tmp/output".into();
        assert!(c.targets().is_err());
        c.twitch = Config::default().twitch;
        c.twitch.enabled = true;
        c.twitch.key = "live_bad\nkey".into();
        assert!(c.targets().is_err());
    }
    #[test]
    fn destination_bitrates_get_their_own_encoder_and_one_recording() {
        let p = Project::default();
        let mut c = Config::default();
        c.record = true;
        c.bitrate = 12_000;
        c.youtube.enabled = true;
        c.youtube.key = "y".into();
        c.twitch.enabled = true;
        c.twitch.key = "t".into();
        assert!(c.canvas_groups(&p, 0).is_err(), "Twitch above 6,000 kbps");
        c.twitch.bitrate = Some(6000);
        let groups = c.canvas_groups(&p, 0).unwrap();
        assert_eq!(groups.iter().map(|(i, g)| (*i, g.bitrate, g.record)).collect::<Vec<_>>(), [(0, 6000, true), (0, 12_000, false)]);
        assert!(groups[0].1.twitch.enabled && groups[1].1.youtube.enabled);
        c.twitch.bitrate = Some(12_000);
        assert!(c.canvas_groups(&p, 0).is_err());
        c.twitch.bitrate = None;
        c.bitrate = 6000;
        assert_eq!(c.canvas_groups(&p, 0).unwrap().len(), 1);
    }
    #[test]
    fn keyring_split_keeps_keys_by_service_id() {
        let mut c = Config::default();
        c.twitch.key = "t".into();
        c.add_service(Provider::Kick);
        c.add_service(Provider::Custom);
        c.services[1].destination.key = "custom".into();
        let keys = c.keys();
        assert_eq!(keys.keys().collect::<Vec<_>>(), ["service-2", "twitch"]);
        let mut stripped = c.without_keys();
        assert!(stripped.keys().is_empty());
        stripped.services.swap(0, 1);
        stripped.fill_keys(&keys);
        assert_eq!(stripped.services[0].destination.key, "custom");
        assert_eq!(stripped.twitch.key, "t");
    }
    #[test]
    fn private_settings_roundtrip_and_atomic_replacement() {
        let dir =
            std::env::temp_dir().join(format!("rosorah-stream-settings-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("settings.json");
        let mut c = Config::default();
        c.youtube.key = "private-key".into();
        c.save(&p).unwrap();
        assert_eq!(Config::load(&p).unwrap().youtube.key, "private-key");
        #[cfg(unix)]
        assert_eq!(p.metadata().unwrap().permissions().mode() & 0o777, 0o600);
        c.youtube.key.clear();
        c.save(&p).unwrap();
        assert!(!fs::read_to_string(&p).unwrap().contains("private-key"));
        fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod canvas_tests {
    use super::*;
    fn config() -> Config {
        let mut c = Config::default();
        c.twitch.enabled = true;
        c.twitch.key = "twitch-key".into();
        c.youtube.enabled = true;
        c.youtube.key = "youtube-main-key".into();
        c.youtube_vertical.enabled = true;
        c.youtube_vertical.key = "youtube-vertical-key".into();
        c
    }
    #[test]
    fn shares_one_encoder_per_canvas_and_ignores_preview_selection() {
        let p = Project::default();
        let c = config();
        let groups = c.canvas_groups(&p, 1).unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0, 0);
        assert!(groups[0].1.twitch.enabled && groups[0].1.youtube.enabled);
        assert!(!groups[0].1.youtube_vertical.enabled);
        assert_eq!(groups[1].0, 1);
        assert!(groups[1].1.youtube_vertical.enabled);
    }
    #[test]
    fn routes_by_stable_canvas_id_and_rejects_missing_canvas_or_duplicate_keys() {
        let mut p = Project::default();
        let mut c = config();
        c.twitch.canvas = Some(p.canvases[1].id);
        p.canvases.swap(0, 1);
        let groups = c.canvas_groups(&p, 1).unwrap();
        assert_eq!(groups.len(), 2);
        assert!(groups[0].1.twitch.enabled && groups[0].1.youtube_vertical.enabled);
        c.twitch.canvas = Some(u64::MAX);
        assert!(c.canvas_groups(&p, 0).is_err());
        c.twitch.canvas = None;
        c.youtube_vertical.key = c.youtube.key.clone();
        assert!(c.canvas_groups(&p, 0).is_err());
    }
    #[test]
    fn old_settings_keep_keys_and_default_to_main() {
        let c: Config=serde_json::from_value(serde_json::json!({"youtube":{"enabled":true,"server":"rtmp://example/live","key":"existing"}})).unwrap();
        assert!(!c.youtube_vertical.enabled);
        assert_eq!(c.youtube.key, "existing");
        assert_eq!(c.canvas_groups(&Project::default(), 1).unwrap()[0].0, 0);
    }
    #[test]
    #[ignore = "Requires FFmpeg/ffprobe and Vulkan; only local files, no streaming service"]
    fn dual_canvas_pipeline_writes_landscape_portrait_and_audio() -> Result<()> {
        use std::time::{Duration, Instant};
        let mut p = Project::default();
        p.preferences.recording_encoder = auroracast_core::RecordingEncoder::Software;
        p.canvases[0].width = 160;
        p.canvases[0].height = 90;
        p.canvases[0].fps = 30;
        p.canvases[1].width = 90;
        p.canvases[1].height = 160;
        p.canvases[1].fps = 30;
        let folder =
            std::env::temp_dir().join(format!("rosorah-dual-canvas-{}", std::process::id()));
        let mut c = config();
        c.record = true;
        let setups = c
            .canvas_groups(&p, 0)?
            .into_iter()
            .map(|(index, c)| {
                let mut setup = Setup::new(&c, &folder)?;
                setup
                    .targets
                    .retain(|t| matches!(t.destination, Location::File(_)));
                setup
                    .state
                    .destinations
                    .lock()
                    .unwrap()
                    .retain(|d| d.path.is_some());
                Ok((index, setup))
            })
            .collect::<Result<Vec<_>>>()?;
        let stream = Stream::start_prepared(&p, FrameReader::default(), setups)?;
        assert_eq!(stream.outlets().len(), 2);
        let deadline = Instant::now() + Duration::from_secs(20);
        while stream
            .engines()
            .any(|e| e.view().phase != crate::recording::Phase::Recording)
        {
            for e in stream.engines() {
                if let Some(Err(error)) = e.view().result {
                    anyhow::bail!("{error}");
                }
            }
            ensure!(Instant::now() < deadline, "Encoders failed to start");
            std::thread::sleep(Duration::from_millis(10));
        }
        let outlets = stream.outlets();
        for _ in 0..60 {
            let at = Instant::now();
            for outlet in &outlets {
                outlet.push(at, vec![0.15; 960]);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        stream.stop();
        while stream.view().result.is_none() {
            ensure!(Instant::now() < deadline, "Finalization timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
        stream.view().result.unwrap().map_err(anyhow::Error::msg)?;
        for engine in stream.engines() {
            let path = engine.view().path;
            let probe = std::process::Command::new("ffprobe")
                .args(["-v", "error", "-show_streams", "-of", "json"])
                .arg(&path)
                .output()?;
            ensure!(probe.status.success(), "ffprobe failed");
            let v: serde_json::Value = serde_json::from_slice(&probe.stdout)?;
            let tracks = v["streams"].as_array().unwrap();
            let video = tracks.iter().find(|v| v["codec_type"] == "video").unwrap();
            assert_eq!(video["width"], engine.canvas.width);
            assert_eq!(video["height"], engine.canvas.height);
            assert!(
                tracks
                    .iter()
                    .any(|v| v["codec_type"] == "audio" && v["channels"] == 2)
            );
            let audio = std::process::Command::new("ffmpeg")
                .args(["-v", "error", "-i"])
                .arg(&path)
                .args(["-map", "0:a:0", "-f", "f32le", "pipe:1"])
                .output()?;
            ensure!(audio.status.success(), "Audio decoding failed");
            assert!(
                audio
                    .stdout
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| f32::from_le_bytes(*b))
                    .any(|s| s.abs() > 0.05)
            );
            println!(
                "Verified {}: {}x{} with stereo audio",
                path.display(),
                engine.canvas.width,
                engine.canvas.height
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod additional_service_tests {
    use super::*;
    fn extra(
        config: &mut Config,
        provider: Provider,
        key: &str,
        canvas: Option<auroracast_core::Id>,
    ) {
        let slot = config.add_service(provider).unwrap();
        *config.destination_mut(slot).unwrap() = Destination {
            enabled: true,
            server: "rtmps://example.com/live".into(),
            key: key.into(),
            canvas,
            ..Default::default()
        };
    }
    #[test]
    fn old_settings_and_added_connections_round_trip() {
        let mut c: Config =
            serde_json::from_str(r#"{"bitrate":4500,"twitch":{"enabled":true,"server":"rtmps://example.com/twitch","key":"old-key"}}"#)
                .unwrap();
        assert!(c.services.is_empty());
        assert_eq!(c.twitch.key, "old-key");
        extra(&mut c, Provider::Kick, "kick-key", None);
        extra(&mut c, Provider::TikTok, "tik+tok.key?auth=abc%20", None);
        let decoded: Config = serde_json::from_slice(&serde_json::to_vec(&c).unwrap()).unwrap();
        assert!(decoded == c);
        assert_eq!(decoded.targets().unwrap().len(), 3);
    }
    #[test]
    fn additional_services_share_only_their_assigned_canvas_encoder() {
        let p = Project::default();
        let main = p.canvases[0].id;
        let vertical = p.canvases[1].id;
        let mut c = Config::default();
        extra(&mut c, Provider::Kick, "kick", Some(main));
        extra(&mut c, Provider::TikTok, "tiktok", Some(vertical));
        extra(&mut c, Provider::Custom, "custom", Some(main));
        let groups = c.canvas_groups(&p, 0).unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].1.services.len(), 2);
        assert_eq!(groups[1].1.services.len(), 1);
        c.services[1].destination.canvas = Some(u64::MAX);
        assert!(c.canvas_groups(&p, 0).is_err());
    }
    #[test]
    fn duplicate_targets_and_invalid_secrets_fail_without_echoing_keys() {
        let mut c = Config::default();
        extra(&mut c, Provider::Kick, "private-key", None);
        extra(&mut c, Provider::TikTok, "private-key", None);
        let err = c.targets().err().unwrap().to_string();
        assert!(!err.contains("private-key"));
        c.services[1].destination.key = "private\nkey".into();
        let err = c.targets().err().unwrap().to_string();
        assert!(!err.contains("private"));
        c.services[1].destination.enabled = false;
        c.services[1].name.clear();
        assert_eq!(c.targets().unwrap().len(), 1);
    }
    #[test]
    fn additional_service_count_is_bounded() {
        let mut c = Config::default();
        for _ in 0..16 {
            assert!(c.add_service(Provider::Custom).is_some());
        }
        assert!(c.add_service(Provider::Custom).is_none());
        c.services.remove(5);
        let slot = c.add_service(Provider::Kick).unwrap();
        assert_eq!(c.services[slot - 3].id, 17);
    }
}

#[cfg(test)]
mod naming_tests {
    use super::*;
    #[test]
    fn legacy_defaults_and_renamed_connections_survive_encoder_grouping() {
        let mut config: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(config.connection_name(1), "YouTube");
        *config.connection_name_mut(0).unwrap() = "My gaming channel".into();
        config.twitch.enabled = true;
        config.twitch.key = "test-key".into();
        let restored: Config =
            serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        let groups = restored.canvas_groups(&Project::default(), 0).unwrap();
        assert_eq!(groups[0].1.targets().unwrap()[0].name, "My gaming channel");
    }
}
