//! Live PCM capture through PipeWire's CLI clients. No default-device fallback
//! for app capture: explicit output-port links keep other applications private.
pub const DEFAULT_MONITOR: &str = "rosorah:default-output";
use anyhow::{Context, Result, bail, ensure};
use auroracast_core::{AudioChannel, Id, mix_audio};
use rosorah_capture::GameProcess;
use serde_json::Value;
use std::{
    collections::{BTreeMap, VecDeque},
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
const SAMPLES: usize = 960; // stereo, 10 ms, 48 kHz
#[derive(Clone, Debug)]
pub struct Device {
    pub serial: String,
    pub target: String,
    pub label: String,
    pub kind: String,
    pub id: u64,
    pub pid: Option<u32>,
    pub channels: Vec<String>,
    /// For app streams: which app it is, stable across restarts (program or app id, lowercase).
    pub app: String,
    /// For app streams: what the stream calls itself (Firefox uses the page title) and its role.
    pub title: String,
    pub role: String,
}
#[derive(Clone, Default)]
pub struct View {
    pub devices: Vec<Device>,
    pub status: BTreeMap<Id, String>,
    pub peaks: BTreeMap<Id, f32>,
    pub stereo_peaks: BTreeMap<Id, [f32; 2]>,
    pub error: String,
    pub capture_error: String,
    pub recording: bool,
    pub saved: String,
    pub capturing: bool,
    pub monitor_status: String,
}
#[derive(Clone, Default)]
struct Control {
    channels: Arc<Vec<AudioChannel>>,
    preview_channels: Arc<Vec<AudioChannel>>,
    media: crate::video::AudioBus,
    monitor_device: String,
    targets: Arc<BTreeMap<Id, String>>,
    /// Apps the user added to the automatic music list.
    music_apps: Arc<Vec<String>>,
    refresh: bool,
    record: Option<bool>,
    automatic: bool,
    game: Option<GameProcess>,
    frames: Option<rosorah_capture::FrameReader>,
    outlet: Option<crate::recording::AudioOutlet>,
    streaming_outlets: Arc<Vec<crate::recording::AudioOutlet>>,
}
impl Control {
    fn snapshot(&self) -> Self {
        let mut c = self.clone();
        if let Some(reader) = &c.frames {
            c.game = reader.game();
        }
        c
    }
}
pub struct Audio {
    control: Arc<Mutex<Control>>,
    view: Arc<Mutex<View>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    pub selected: BTreeMap<Id, String>,
    pub enabled: BTreeMap<Id, bool>,
}
impl Default for Audio {
    fn default() -> Self {
        let control = Arc::new(Mutex::new(Control {
            refresh: true,
            ..Default::default()
        }));
        let view = Arc::new(Mutex::new(View::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (c, v, s) = (control.clone(), view.clone(), stop.clone());
        let worker = thread::spawn(move || worker(c, v, s));
        Self {
            control,
            view,
            stop,
            worker: Some(worker),
            selected: Default::default(),
            enabled: Default::default(),
        }
    }
}
impl Audio {
    pub fn media_bus(&self, bus: crate::video::AudioBus) {
        self.control.lock().unwrap().media = bus;
    }
    pub fn scene(&self, project: &auroracast_core::Project, scene: Id) {
        self.control.lock().unwrap().preview_channels = Arc::new(project.audio_for_scene(scene));
    }

    pub fn follow_capture(&self, reader: rosorah_capture::FrameReader) {
        self.control.lock().unwrap().frames = Some(reader);
    }
    pub fn recording_outlet(&self, outlet: Option<crate::recording::AudioOutlet>) {
        self.control.lock().unwrap().outlet = outlet;
    }
    pub fn streaming_outlets(&self, outlets: Vec<crate::recording::AudioOutlet>) {
        self.control.lock().unwrap().streaming_outlets = Arc::new(outlets);
    }
    pub fn view(&self) -> View {
        self.view.lock().unwrap().clone()
    }
    pub fn sync(&self, channels: &[AudioChannel], automatic: bool, game: Option<GameProcess>) {
        let mut c = self.control.lock().unwrap();
        if c.channels.as_slice() != channels {
            c.channels = Arc::new(channels.to_vec());
        }
        c.automatic = automatic && channels.iter().any(|c| c.id == crate::game_audio::CHANNEL);
        c.game = game;
        // Only strips that still exist: a deleted strip keeps its choice (for Undo) but captures nothing.
        let selected = self
            .selected
            .iter()
            .filter(|(id, _)| self.enabled.get(id) == Some(&true) && channels.iter().any(|c| c.id == **id));
        if !c.targets.iter().eq(selected.clone()) {
            c.targets = Arc::new(selected.map(|(id, target)| (*id, target.clone())).collect());
        }
    }
    pub fn music_apps(&self, apps: &[String]) {
        let mut c = self.control.lock().unwrap();
        if c.music_apps.as_slice() != apps {
            c.music_apps = Arc::new(apps.to_vec());
        }
    }
    pub fn monitor_device(&self, target: &str) {
        let mut c = self.control.lock().unwrap();
        if c.monitor_device != target {
            c.monitor_device = target.into();
        }
    }
    pub fn refresh(&self) {
        self.control.lock().unwrap().refresh = true;
    }
    pub fn record(&self, start: bool) {
        self.control.lock().unwrap().record = Some(start);
    }
    pub fn active(&self) -> bool {
        self.enabled.values().any(|v| *v)
            || self.control.lock().unwrap().automatic
            || self.view.lock().unwrap().recording
    }
}
impl Drop for Audio {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}
// Bound command duration. Capturing output on another thread avoids pipe-buffer deadlocks.
pub(crate) fn command(program: &str, args: &[&str]) -> Result<Vec<u8>> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("Cannot run {program}"))?;
    let mut out = child.stdout.take().unwrap();
    let reader = thread::spawn(move || {
        let mut b = Vec::new();
        out.by_ref()
            .take(8 * 1024 * 1024)
            .read_to_end(&mut b)
            .map(|_| b)
    });
    let start = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if start.elapsed() > Duration::from_secs(3) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            bail!("{program} timed out");
        }
        thread::sleep(Duration::from_millis(10));
    };
    let bytes = reader
        .join()
        .map_err(|_| anyhow::anyhow!("Reader failed"))??;
    ensure!(status.success(), "{program} exited unsuccessfully");
    Ok(bytes)
}
/// This computer's audio devices and app streams right now (for checks and tests).
#[cfg(all(test, not(windows)))]
pub fn live_devices() -> Result<Vec<Device>> {
    Ok(devices(&graph()?))
}
#[cfg(not(windows))]
fn graph() -> Result<Value> {
    Ok(serde_json::from_slice(&command("pw-dump", &[])?)?)
}
fn prop(v: &Value, k: &str) -> String {
    match &v["info"]["props"][k] {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}
#[cfg(not(windows))]
fn devices(g: &Value) -> Vec<Device> {
    g.as_array()
        .into_iter()
        .flatten()
        .filter_map(|n| {
            if n["type"].as_str() != Some("PipeWire:Interface:Node") {
                return None;
            }
            let kind = prop(n, "media.class");
            if !["Audio/Source", "Audio/Sink", "Stream/Output/Audio"].contains(&kind.as_str()) {
                return None;
            }
            let name = prop(n, "node.name");
            if name.starts_with("rosorah-") {
                return None;
            }
            let serial = prop(n, "object.serial");
            if serial.is_empty() {
                return None;
            }
            let mut label = prop(n, "node.description");
            if label.is_empty() {
                label = prop(n, "application.name");
            }
            if label.is_empty() {
                label = name.clone();
            }
            let prefix = match kind.as_str() {
                "Audio/Source" => "Microphone / input",
                "Audio/Sink" => "Desktop output",
                _ => "Application",
            };
            // The first identity the app gives: its program, its (Flatpak) app id, then its name.
            let app = ["application.process.binary", "pipewire.access.portal.app_id", "application.id", "application.name", "node.name"]
                .iter()
                .map(|k| prop(n, k))
                .find(|v| !v.trim().is_empty())
                .unwrap_or_default()
                .to_lowercase();
            Some(Device {
                target: name,
                serial,
                label: format!("{prefix}: {label}"),
                id: n["id"].as_u64()?,
                pid: node_pid(g, n),
                channels: output_channels(g, n["id"].as_u64()?),
                app: if kind == "Stream/Output/Audio" { app } else { String::new() },
                title: prop(n, "media.name"),
                role: prop(n, "media.role"),
                kind,
            })
        })
        .collect()
}
fn node_pid(g: &Value, node: &Value) -> Option<u32> {
    // PulseAudio supplies the game's PID on the stream node. Its PipeWire
    // client's secured PID belongs to pipewire-pulse, not to the game.
    if let Ok(pid) = prop(node, "application.process.id").parse::<u32>() {
        return (pid > 0).then_some(pid);
    }
    let client_id = prop(node, "client.id");
    let client = g.as_array()?.iter().find(|n| {
        n["type"].as_str() == Some("PipeWire:Interface:Client")
            && n["id"]
                .as_u64()
                .is_some_and(|id| id.to_string() == client_id)
    })?;
    prop(client, "application.process.id")
        .parse::<u32>()
        .ok()
        .filter(|p| *p > 0)
}
fn output_channels(g: &Value, node: u64) -> Vec<String> {
    let mut ports: Vec<_> = g
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| {
            p["type"].as_str() == Some("PipeWire:Interface:Port")
                && prop(p, "node.id") == node.to_string()
                && prop(p, "port.direction") == "out"
        })
        .collect();
    ports.sort_by_key(|p| p["id"].as_u64());
    ports.iter().map(|p| prop(p, "audio.channel")).collect()
}
struct Buffer {
    samples: VecDeque<f32>,
    ended: bool,
}
#[cfg(not(windows))]
struct Input {
    child: Child,
    filter: Option<Child>,
    reader: Option<thread::JoinHandle<()>>,
    buffer: Arc<Mutex<Buffer>>,
}
#[cfg(not(windows))]
impl Input {
    fn start(d: &Device, id: Id, noise: auroracast_core::NoiseSuppression) -> Result<Self> {
        // App streams are explicitly linked; devices use the session manager's target.
        let app = d.kind == "Stream/Output/Audio";
        let name = format!("rosorah-audio-{}-{id}-{}", std::process::id(), d.serial);
        let layout = if app {
            d.channels.clone()
        } else {
            vec!["FL".into(), "FR".into()]
        };
        ensure!(
            !layout.is_empty() && layout.len() <= 8 && layout.iter().all(|c| !c.is_empty()),
            "Waiting for the application's audio ports"
        );
        ensure!(
            layout.iter().all(|c| matches!(
                c.as_str(),
                "FL" | "FR" | "FC" | "LFE" | "RL" | "RR" | "SL" | "SR" | "RC" | "MONO"
            ) || (layout.len() <= 2
                && c.strip_prefix("AUX")
                    .is_some_and(|s| s.parse::<u32>().is_ok()))),
            "Unsupported audio channel layout; choose stereo output in the game"
        );
        let props=serde_json::json!({"node.name":name,"application.name":"Rosorah Studio","node.dont-reconnect":true,"node.dont-fallback":true,"node.autoconnect":!app,"node.passive":app,"stream.monitor":app,"stream.capture.sink":d.kind!="Audio/Source"}).to_string();
        let mut command_builder = Command::new("pw-record");
        // Never interpret an AU/WAV container header as float PCM.
        ensure!(
            String::from_utf8_lossy(&command("pw-record", &["--help"])?).contains("--raw"),
            "Update pipewire-utils: pw-record must support --raw audio capture"
        );
        command_builder.arg("--raw");
        let count = layout.len().to_string();
        let channel_map = layout.join(",");
        let child = command_builder
            .args([
                "--format=f32",
                "--rate=48000",
                "--channels",
                &count,
                "--channel-map",
                &channel_map,
                "--latency=20ms",
                "--target",
                if app { "0" } else { &d.serial },
                "--properties",
                &props,
                "-",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("Cannot start pw-record; install pipewire-utils")?;
        let buffer = Arc::new(Mutex::new(Buffer {
            samples: VecDeque::new(),
            ended: false,
        }));
        let mut input = Self {
            child,
            filter: None,
            reader: None,
            buffer: buffer.clone(),
        };
        let stdout = input.child.stdout.take().unwrap();
        let mut stdout = if noise != auroracast_core::NoiseSuppression::Off {
            let mut child = Command::new("ffmpeg")
                .args([
                    "-nostdin",
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-f",
                    "f32le",
                    "-ar",
                    "48000",
                    "-ac",
                    &count,
                    "-probesize",
                    "32",
                    "-analyzeduration",
                    "0",
                    "-i",
                    "pipe:0",
                    "-af",
                    noise_filter(noise),
                    "-f",
                    "f32le",
                    "-ar",
                    "48000",
                    "-ac",
                    &count,
                    "-flush_packets",
                    "1",
                    "pipe:1",
                ])
                .stdin(Stdio::from(stdout))
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .context("Noise suppression needs FFmpeg")?;
            let out = child.stdout.take().context("Noise filter output missing")?;
            input.filter = Some(child);
            out
        } else {
            stdout
        };
        input.reader = Some(thread::spawn(move || {
            let mut bytes = vec![0u8; SAMPLES / 2 * layout.len() * 4];
            while stdout.read_exact(&mut bytes).is_ok() {
                let mut b = buffer.lock().unwrap();
                b.samples.extend(stereo(&bytes, &layout));
                while b.samples.len() > SAMPLES * 10 {
                    b.samples.pop_front();
                }
            }
            buffer.lock().unwrap().ended = true;
        }));
        if app {
            let started = Instant::now();
            loop {
                let g = graph()?;
                ensure!(
                    devices(&g).iter().any(|current| current.id == d.id
                        && current.serial == d.serial
                        && current.pid == d.pid),
                    "Application audio stream changed; reconnecting"
                );
                let node = g
                    .as_array()
                    .context("Invalid graph")?
                    .iter()
                    .find(|n| prop(n, "node.name") == name)
                    .and_then(|n| n["id"].as_u64());
                if let Some(node) = node {
                    let pairs = port_pairs(&g, d.id, node);
                    if !pairs.is_empty() {
                        for (out, inp) in pairs {
                            command("pw-link", &["-L", "-P", &out.to_string(), &inp.to_string()])?;
                        }
                        break;
                    }
                }
                ensure!(
                    started.elapsed() < Duration::from_secs(3),
                    "Application audio ports are unavailable; reconnecting automatically"
                );
                thread::sleep(Duration::from_millis(50));
            }
        }
        Ok(input)
    }
}
fn stereo(bytes: &[u8], layout: &[String]) -> Vec<f32> {
    let mut output = Vec::with_capacity(bytes.len() / layout.len() / 2);
    for frame in bytes.chunks_exact(layout.len() * 4) {
        let mut lr = [0.; 2];
        for (i, (sample, ch)) in frame.as_chunks::<4>().0.iter().zip(layout).enumerate() {
            let x = f32::from_ne_bytes(*sample);
            let x = if x.is_finite() { x } else { 0. };
            let gain = match ch.as_str() {
                "FL" => [1., 0.],
                "FR" => [0., 1.],
                "MONO" => [1., 1.],
                "FC" => [std::f32::consts::FRAC_1_SQRT_2; 2],
                "RL" | "SL" => [std::f32::consts::FRAC_1_SQRT_2, 0.],
                "RR" | "SR" => [0., std::f32::consts::FRAC_1_SQRT_2],
                "LFE" | "RC" => [0.5; 2],
                _ if layout.len() == 1 => [1., 1.],
                _ if i == 0 => [1., 0.],
                _ if i == 1 => [0., 1.],
                _ => [0., 0.],
            };
            lr[0] += x * gain[0];
            lr[1] += x * gain[1];
        }
        output.extend(lr);
    }
    output
}
#[cfg(not(windows))]
impl Drop for Input {
    fn drop(&mut self) {
        if let Some(filter) = &mut self.filter {
            let _ = filter.kill();
            let _ = filter.wait();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(t) = self.reader.take() {
            let _ = t.join();
        }
    }
}
fn port_pairs(g: &Value, source: u64, dest: u64) -> Vec<(u64, u64)> {
    let ports: Vec<_> = g
        .as_array()
        .into_iter()
        .flatten()
        .filter(|n| n["type"].as_str() == Some("PipeWire:Interface:Port"))
        .collect();
    let outs: Vec<_> = ports
        .iter()
        .filter(|p| prop(p, "node.id") == source.to_string() && prop(p, "port.direction") == "out")
        .collect();
    let ins: Vec<_> = ports
        .iter()
        .filter(|p| prop(p, "node.id") == dest.to_string() && prop(p, "port.direction") == "in")
        .collect();
    ins.iter()
        .filter_map(|i| {
            let ch = prop(i, "audio.channel");
            let o = outs
                .iter()
                .find(|o| prop(o, "audio.channel") == ch)
                .or_else(|| if outs.len() == 1 { outs.first() } else { None })?;
            Some((o["id"].as_u64()?, i["id"].as_u64()?))
        })
        .collect()
}
struct Wav {
    file: File,
    bytes: u32,
}
impl Wav {
    fn new(path: PathBuf) -> Result<Self> {
        let file = OpenOptions::new().write(true).create_new(true).open(path)?;
        let mut s = Self { file, bytes: 0 };
        s.header()?;
        Ok(s)
    }
    fn header(&mut self) -> Result<()> {
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(b"RIFF")?;
        self.file.write_all(&(36 + self.bytes).to_le_bytes())?;
        self.file.write_all(b"WAVEfmt ")?;
        for b in [
            16u32.to_le_bytes().to_vec(),
            1u16.to_le_bytes().to_vec(),
            2u16.to_le_bytes().to_vec(),
            48000u32.to_le_bytes().to_vec(),
            192000u32.to_le_bytes().to_vec(),
            4u16.to_le_bytes().to_vec(),
            16u16.to_le_bytes().to_vec(),
            b"data".to_vec(),
            self.bytes.to_le_bytes().to_vec(),
        ] {
            self.file.write_all(&b)?;
        }
        Ok(())
    }
    fn write(&mut self, pcm: &[f32]) -> Result<()> {
        ensure!(
            self.bytes < 3_900_000_000,
            "Audio test recording reached its size limit"
        );
        let b: Vec<u8> = pcm
            .iter()
            .flat_map(|x| ((x.clamp(-1., 1.) * 32767.).round() as i16).to_le_bytes())
            .collect();
        self.file.write_all(&b)?;
        self.bytes += b.len() as u32;
        Ok(())
    }
    fn finish(&mut self) -> Result<()> {
        self.header()?;
        self.file.flush()?;
        Ok(())
    }
}
impl Drop for Wav {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Selection {
    Manual(String),
    Game(GameProcess),
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Binding {
    channel: Id,
    selection: Selection,
}
impl Binding {
    fn current(&self, c: &Control) -> bool {
        match &self.selection {
            Selection::Game(game) => {
                c.automatic
                    && self.channel == crate::game_audio::CHANNEL
                    && c.frames
                        .as_ref()
                        .map_or_else(|| c.game.clone(), |r| r.game())
                        .as_ref()
                        == Some(game)
            }
            Selection::Manual(serial) => {
                !(c.automatic && self.channel == crate::game_audio::CHANNEL)
                    && c.targets.get(&self.channel) == Some(serial)
            }
        }
    }
}
#[derive(Clone)]
struct LiveInput {
    binding: Binding,
    buffer: Arc<Mutex<Buffer>>,
}
fn noise_filter(noise: auroracast_core::NoiseSuppression) -> &'static str {
    match noise {
        auroracast_core::NoiseSuppression::Off => "anull",
        auroracast_core::NoiseSuppression::Gentle => "highpass=f=70,afftdn=nr=12:nf=-45:tn=1",
        auroracast_core::NoiseSuppression::Strong => {
            "highpass=f=90,afftdn=nr=24:nf=-35:tn=1,agate=threshold=0.008:ratio=2:attack=10:release=180:range=0.1"
        }
    }
}
fn channel_noise(c: &Control, id: Id) -> auroracast_core::NoiseSuppression {
    c.channels
        .iter()
        .find(|v| v.id == id)
        .map(|v| v.noise_suppression)
        .unwrap_or_default()
}
struct ManagedInput {
    input: Input,
    binding: Binding,
    device: Device,
    noise: auroracast_core::NoiseSuppression,
}
type InputKey = (Id, String);

// Graph discovery and linking may take seconds when an audio server is busy.
// Keep every such operation off the 10 ms PCM mixing/recording clock.
fn capture_worker(
    control: Arc<Mutex<Control>>,
    view: Arc<Mutex<View>>,
    live: Arc<Mutex<Vec<LiveInput>>>,
    stop: Arc<AtomicBool>,
) {
    let mut inputs: BTreeMap<InputKey, ManagedInput> = BTreeMap::new();
    let mut failed: BTreeMap<InputKey, (Instant, String)> = BTreeMap::new();
    let mut known = Vec::new();
    let mut last_graph = Instant::now() - Duration::from_secs(5);
    let mut previous_game = None;
    let mut family = std::collections::BTreeSet::new();
    while !stop.load(Ordering::Relaxed) {
        let (mut c, refresh) = {
            let mut c = control.lock().unwrap();
            let refresh = std::mem::take(&mut c.refresh);
            (c.snapshot(), refresh)
        };
        let changed_game = c.game != previous_game;
        if refresh
            || changed_game
            || (((c.automatic && c.game.is_some()) || !c.targets.is_empty())
                && last_graph.elapsed() >= Duration::from_secs(1))
        {
            match graph() {
                Ok(g) => {
                    known = devices(&g);
                    let mut v = view.lock().unwrap();
                    v.devices = known.clone();
                    v.capture_error.clear();
                }
                Err(e) => {
                    known.clear();
                    let mut v = view.lock().unwrap();
                    v.devices.clear();
                    v.capture_error = e.to_string();
                }
            }
            last_graph = Instant::now();
            c = control.lock().unwrap().snapshot();
            family = c
                .game
                .as_ref()
                .filter(|_| c.automatic)
                .map(crate::game_audio::family)
                .unwrap_or_default();
            previous_game = c.game.clone();
            if refresh {
                failed.clear();
            }
        }
        if c.game
            .as_ref()
            .is_some_and(|g| !crate::game_audio::alive(g))
        {
            family.clear();
        }
        let mut desired: BTreeMap<InputKey, (Device, Binding)> = BTreeMap::new();
        // Apps a strip follows by name; Music leaves those to their own strips.
        let claimed: Vec<String> = c.targets.values().filter_map(|t| crate::app_audio::followed_app(t)).map(str::to_owned).collect();
        for (id, serial) in c.targets.iter() {
            if c.automatic && *id == crate::game_audio::CHANNEL {
                continue;
            }
            if crate::app_audio::follows(serial) {
                // One capture per matching app sound, picked again whenever apps come and go.
                for d in crate::app_audio::resolve(serial, &known, &c.music_apps, &claimed, &family) {
                    desired.insert((*id, d.serial.clone()), (d.clone(), Binding { channel: *id, selection: Selection::Manual(serial.clone()) }));
                }
                continue;
            }
            if let Some(d) = known.iter().find(|d| &d.serial == serial) {
                desired.insert(
                    (*id, serial.clone()),
                    (
                        d.clone(),
                        Binding {
                            channel: *id,
                            selection: Selection::Manual(serial.clone()),
                        },
                    ),
                );
            }
        }
        if c.automatic
            && let Some(game) = &c.game
        {
            for d in known
                .iter()
                .filter(|d| {
                    d.kind == "Stream/Output/Audio"
                        && d.pid.is_some_and(|pid| family.contains(&pid))
                })
                .take(16)
            {
                desired.insert(
                    (crate::game_audio::CHANNEL, d.serial.clone()),
                    (
                        d.clone(),
                        Binding {
                            channel: crate::game_audio::CHANNEL,
                            selection: Selection::Game(game.clone()),
                        },
                    ),
                );
            }
        }
        inputs.retain(|key, managed| {
            let wanted = desired.get(key).is_some_and(|(d, b)| {
                b == &managed.binding
                    && d.id == managed.device.id
                    && d.pid == managed.device.pid
                    && d.channels == managed.device.channels
                    && managed.noise == channel_noise(&c, key.0)
            });
            if wanted && managed.input.buffer.lock().unwrap().ended {
                failed.insert(
                    key.clone(),
                    (
                        Instant::now() + Duration::from_secs(2),
                        "Audio input ended; reconnecting automatically".into(),
                    ),
                );
                return false;
            }
            wanted
        });
        failed.retain(|key, _| desired.contains_key(key));
        for (key, (d, binding)) in &desired {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            if inputs.contains_key(key)
                || failed
                    .get(key)
                    .is_some_and(|(next, _)| Instant::now() < *next)
            {
                continue;
            }
            if !binding.current(&control.lock().unwrap()) {
                continue;
            }
            let noise = channel_noise(&c, key.0);
            match Input::start(d, key.0, noise) {
                Ok(input) => {
                    if binding.current(&control.lock().unwrap()) {
                        inputs.insert(
                            key.clone(),
                            ManagedInput {
                                input,
                                binding: binding.clone(),
                                device: d.clone(),
                                noise,
                            },
                        );
                        failed.remove(key);
                    }
                }
                Err(e) => {
                    failed.insert(
                        key.clone(),
                        (Instant::now() + Duration::from_secs(2), e.to_string()),
                    );
                }
            }
        }
        // A source switch can arrive during a command. Publish only bindings
        // still requested; the mixer independently applies this check each tick.
        let c = control.lock().unwrap().snapshot();
        let active: Vec<_> = inputs
            .values()
            .filter(|m| m.binding.current(&c))
            .map(|m| LiveInput {
                binding: m.binding.clone(),
                buffer: m.input.buffer.clone(),
            })
            .collect();
        {
            let mut v = view.lock().unwrap();
            v.capturing = !active.is_empty();
            v.status.clear();
            for channel in c.channels.iter() {
                let id = channel.id;
                let count = active.iter().filter(|s| s.binding.channel == id).count();
                let problem = failed
                    .iter()
                    .find(|((channel, _), _)| *channel == id)
                    .map(|(_, (_, e))| e.as_str());
                if c.automatic && id == crate::game_audio::CHANNEL {
                    let status = match (&c.game, count, problem) {
                        (None, _, _) => "Waiting for a captured game".into(),
                        (Some(game), n, _) if n > 0 => format!(
                            "Capturing {} · {n} audio stream{}",
                            game.name,
                            if n == 1 { "" } else { "s" }
                        ),
                        (_, _, Some(error)) => error.into(),
                        (Some(game), _, _) => format!("Waiting for audio from {}", game.name),
                    };
                    v.status.insert(id, status);
                } else if c.targets.contains_key(&id) {
                    v.status.insert(
                        id,
                        if count > 0 {
                            "Capturing".into()
                        } else {
                            problem
                                .unwrap_or("Waiting for audio source; reconnecting automatically")
                                .into()
                        },
                    );
                }
            }
        }
        *live.lock().unwrap() = active;
        thread::sleep(Duration::from_millis(50));
    }
    live.lock().unwrap().clear();
}

fn read_inputs(c: &Control, live: &[LiveInput]) -> BTreeMap<Id, Vec<f32>> {
    let mut pcms: BTreeMap<Id, Vec<f32>> = BTreeMap::new();
    for input in live.iter().filter(|s| s.binding.current(c)) {
        let mut buffer = input.buffer.lock().unwrap();
        let pcm = pcms
            .entry(input.binding.channel)
            .or_insert_with(|| vec![0.; SAMPLES]);
        for sample in pcm {
            *sample += buffer.samples.pop_front().unwrap_or(0.);
        }
    }
    pcms
}

fn worker(control: Arc<Mutex<Control>>, view: Arc<Mutex<View>>, stop: Arc<AtomicBool>) {
    let live = Arc::new(Mutex::new(Vec::new()));
    let manager = {
        let (c, v, l, s) = (control.clone(), view.clone(), live.clone(), stop.clone());
        thread::spawn(move || capture_worker(c, v, l, s))
    };
    let mut writers: Vec<Wav> = Vec::new();
    let mut next = Instant::now();
    let mut delays = BTreeMap::new();
    let mut monitor: Option<Monitor> = None;
    let mut monitor_retry = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        let block_start = next;
        let (c, record) = {
            let mut c = control.lock().unwrap();
            let record = c.record.take();
            (c.snapshot(), record)
        };
        let sources = live.lock().unwrap().clone();
        if let Some(start) = record {
            if !start {
                for w in &mut writers {
                    if let Err(e) = w.finish() {
                        view.lock().unwrap().error = e.to_string();
                    }
                }
                writers.clear();
            } else if writers.is_empty() {
                let result = (|| -> Result<(Vec<Wav>, PathBuf)> {
                    ensure!(
                        sources.iter().any(|s| s.binding.current(&c)),
                        "Wait for game audio or choose an audio input before recording"
                    );
                    let stamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)?
                        .as_millis();
                    let base = std::env::var_os("HOME")
                        .map(PathBuf::from)
                        .context("Home directory unavailable")?
                        .join("Music")
                        .join(format!("Rosorah-audio-{stamp}"));
                    std::fs::create_dir_all(&base)?;
                    let mut ws = Vec::new();
                    for t in 1..=6 {
                        ws.push(Wav::new(base.join(format!("track-{t}.wav")))?);
                    }
                    Ok((ws, base))
                })();
                match result {
                    Ok((ws, path)) => {
                        writers = ws;
                        view.lock().unwrap().saved = path.display().to_string();
                    }
                    Err(e) => view.lock().unwrap().error = e.to_string(),
                }
            }
            view.lock().unwrap().recording = !writers.is_empty();
        }
        let mut pcms = read_inputs(&c, &sources);
        let media = c.media.read();
        for channel in c.channels.iter() {
            if let Some(source) = channel.media_source
                && let Some(pcm) = media.get(&source)
            {
                pcms.insert(channel.id, pcm.clone());
            }
        }
        delays.retain(|id, _| pcms.contains_key(id));
        for (id, pcm) in &mut pcms {
            let frames = c
                .channels
                .iter()
                .find(|ch| ch.id == *id)
                .map_or(0, |ch| ch.delay_ms.min(2000) as usize * 48 * 2);
            delay_pcm(
                delays.entry(*id).or_insert_with(|| (0, VecDeque::new())),
                frames,
                pcm,
            );
        }
        let stereo_peaks: BTreeMap<Id, [f32; 2]> = pcms
            .iter()
            .map(|(id, pcm)| {
                let gain = c
                    .preview_channels
                    .iter()
                    .find(|c| c.id == *id)
                    .map_or(0., |c| c.gain());
                let mut peaks = [0.0f32; 2];
                for frame in pcm.chunks_exact(2) {
                    for channel in 0..2 {
                        peaks[channel] = peaks[channel].max((frame[channel] * gain).abs());
                    }
                }
                (*id, peaks)
            })
            .collect();
        let peaks = stereo_peaks
            .iter()
            .map(|(id, p)| (*id, p[0].max(p[1])))
            .collect();
        let refs: Vec<_> = pcms.iter().map(|(id, p)| (*id, p.as_slice())).collect();
        if let Some(outlet) = &c.outlet {
            let channels = outlet.channels();
            let tracks: Vec<_> = (0..outlet.tracks())
                .map(|t| {
                    mix_audio(&channels, &refs, t, SAMPLES).unwrap_or_else(|_| vec![0.; SAMPLES])
                })
                .collect();
            outlet.push_tracks(block_start, &tracks);
        }
        for outlet in c.streaming_outlets.iter() {
            if let Ok(pcm) = mix_audio(&outlet.channels(), &refs, 0, SAMPLES) {
                outlet.push(block_start, pcm);
            }
        }
        let monitor_on = !c.monitor_device.is_empty() && c.channels.iter().any(|ch| ch.monitor);
        if monitor.as_mut().is_some_and(|m| {
            m.target != c.monitor_device
                || !monitor_on
                || m.ended()
        }) {
            monitor = None;
        }
        if monitor_on && monitor.is_none() && Instant::now() >= monitor_retry {
            monitor_retry = Instant::now() + Duration::from_secs(5);
            match Monitor::start(&c.monitor_device) {
                Ok(m) => {
                    monitor = Some(m);
                    view.lock().unwrap().monitor_status = "Monitoring active".into();
                }
                Err(e) => {
                    view.lock().unwrap().monitor_status = format!("Monitor: {e}");
                    monitor_retry = Instant::now() + Duration::from_secs(5);
                }
            }
        }
        if !monitor_on {
            let mut v = view.lock().unwrap();
            if v.monitor_status != "Monitoring off" {
                v.monitor_status = "Monitoring off".into();
            }
        }
        if let Some(m) = &monitor {
            let mut channels: Vec<_> = c
                .preview_channels
                .iter()
                .filter(|ch| ch.monitor)
                .cloned()
                .collect();
            for ch in &mut channels {
                ch.tracks[0] = true;
                ch.monitor_only=false;
            }
            if let Ok(pcm) = mix_audio(&channels, &refs, 0, SAMPLES) {
                let _ = m.tx.try_send(pcm);
            }
        }
        let mut write_error = None;
        for (t, w) in writers.iter_mut().enumerate() {
            if let Err(e) = mix_audio(&c.channels, &refs, t, SAMPLES).and_then(|pcm| w.write(&pcm))
            {
                write_error = Some(e.to_string());
                break;
            }
        }
        if let Some(e) = write_error {
            writers.clear();
            let mut v = view.lock().unwrap();
            v.recording = false;
            v.error = e;
        }
        {
            let mut view = view.lock().unwrap();
            view.peaks = peaks;
            view.stereo_peaks = stereo_peaks;
        }
        next += Duration::from_millis(10);
        let now = Instant::now();
        if next > now {
            thread::sleep(next - now);
        } else {
            next = now;
        }
    }
    let _ = manager.join();
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selects_only_audio_and_excludes_self() {
        let g = serde_json::json!([{"type":"PipeWire:Interface:Node","id":5,"info":{"props":{"media.class":"Stream/Output/Audio","node.name":"game","object.serial":10,"application.process.id":123}}},{"type":"PipeWire:Interface:Node","id":6,"info":{"props":{"media.class":"Audio/Source","node.name":"rosorah-audio-test","object.serial":11}}}]);
        let d = devices(&g);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].serial, "10");
        assert_eq!(d[0].pid, Some(123));
    }
    #[test]
    fn pulse_stream_pid_takes_precedence_over_audio_server_pid() {
        let g = serde_json::json!([
            {"type":"PipeWire:Interface:Client","id":9,"info":{"props":{"application.process.id":100,"pipewire.sec.pid":100}}},
            {"type":"PipeWire:Interface:Node","id":10,"info":{"props":{"client.id":9,"application.process.id":"200"}}},
            {"type":"PipeWire:Interface:Node","id":11,"info":{"props":{"client.id":9}}}
        ]);
        assert_eq!(node_pid(&g, &g[1]), Some(200));
        assert_eq!(node_pid(&g, &g[2]), Some(100));
    }
    #[test]
    fn multiple_game_streams_mix_and_old_game_stops_immediately() {
        let game = GameProcess {
            pid: 42,
            start_time: 10,
            name: "game".into(),
        };
        let make = |selection, value| LiveInput {
            binding: Binding {
                channel: crate::game_audio::CHANNEL,
                selection,
            },
            buffer: Arc::new(Mutex::new(Buffer {
                samples: vec![value; SAMPLES].into(),
                ended: false,
            })),
        };
        let sources = vec![
            make(Selection::Game(game.clone()), 0.1),
            make(Selection::Game(game.clone()), 0.2),
            make(Selection::Manual("desktop".into()), 0.7),
        ];
        let mut c = Control {
            automatic: true,
            game: Some(game.clone()),
            ..Default::default()
        };
        Arc::make_mut(&mut c.targets).insert(crate::game_audio::CHANNEL, "desktop".into());
        let pcm = read_inputs(&c, &sources);
        assert!((pcm[&crate::game_audio::CHANNEL][0] - 0.3).abs() < 0.0001);
        let mut channels = auroracast_core::Project::default().audio;
        channels[0].gain_db = 0.;
        let refs = [(
            crate::game_audio::CHANNEL,
            pcm[&crate::game_audio::CHANNEL].as_slice(),
        )];
        assert!((mix_audio(&channels, &refs, 0, SAMPLES).unwrap()[0] - 0.3).abs() < 0.0001);
        assert!(
            mix_audio(&channels, &refs, 5, SAMPLES)
                .unwrap()
                .iter()
                .all(|x| *x == 0.)
        );
        channels[0].muted = true;
        assert!(
            mix_audio(&channels, &refs, 0, SAMPLES)
                .unwrap()
                .iter()
                .all(|x| *x == 0.)
        );
        channels[0].muted = false;
        channels[0].gain_db = -6.;
        assert!((mix_audio(&channels, &refs, 0, SAMPLES).unwrap()[0] - 0.150356).abs() < 0.00001);
        c.game = Some(GameProcess {
            start_time: 20,
            ..game
        });
        assert!(
            read_inputs(&c, &sources).is_empty(),
            "Reused PID is a different game"
        );
        c.game = None;
        assert!(
            read_inputs(&c, &sources).is_empty(),
            "Paused video must not keep its audio"
        );
        c.automatic = false;
        assert!((read_inputs(&c, &sources)[&crate::game_audio::CHANNEL][0] - 0.7).abs() < 0.0001);
    }
    #[test]
    fn mono_and_surround_downmix_preserve_center_and_reject_nan() {
        let bytes = |v: &[f32]| v.iter().flat_map(|x| x.to_ne_bytes()).collect::<Vec<_>>();
        assert_eq!(stereo(&bytes(&[0.25]), &["MONO".into()]), vec![0.25, 0.25]);
        let layout = ["FL", "FR", "FC", "LFE", "RL", "RR"].map(String::from);
        let center = stereo(&bytes(&[0., 0., 1., 0., 0., 0.]), &layout);
        assert!((center[0] - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.0001);
        assert_eq!(center[0], center[1]);
        assert_eq!(
            stereo(
                &bytes(&[f32::NAN, f32::INFINITY]),
                &["FL".into(), "FR".into()]
            ),
            vec![0., 0.]
        );
    }
    #[test]
    fn explicit_ports_do_not_capture_other_apps() {
        let mut nodes = Vec::new();
        for (id, node, ch, dir) in [
            (1, 10, "FL", "out"),
            (2, 10, "FR", "out"),
            (3, 11, "FL", "out"),
            (4, 20, "FL", "in"),
            (5, 20, "FR", "in"),
        ] {
            nodes.push(serde_json::json!({"type":"PipeWire:Interface:Port","id":id,"info":{"props":{"node.id":node,"audio.channel":ch,"port.direction":dir}}}));
        }
        assert_eq!(
            port_pairs(&Value::Array(nodes), 10, 20),
            vec![(1, 4), (2, 5)]
        );
    }
    #[test]
    fn wav_header_and_routing() -> Result<()> {
        let path = std::env::temp_dir().join(format!("rosorah-wav-{}.wav", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let mut w = Wav::new(path.clone())?;
            w.write(&[0., 1., -1., 0.5])?;
            w.finish()?;
        }
        let b = std::fs::read(&path)?;
        assert_eq!(&b[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(b[40..44].try_into()?), 8);
        assert_eq!(b.len(), 52);
        std::fs::remove_file(path)?;
        Ok(())
    }
}

fn delay_pcm(state: &mut (usize, VecDeque<f32>), samples: usize, pcm: &mut [f32]) {
    if state.0 != samples {
        state.0 = samples;
        state.1 = std::iter::repeat_n(0., samples).collect();
    }
    if samples == 0 {
        return;
    }
    for value in pcm {
        state.1.push_back(*value);
        *value = state.1.pop_front().unwrap_or(0.);
    }
}
#[cfg(not(windows))]
struct Monitor {
    target: String,
    child: Child,
    tx: std::sync::mpsc::SyncSender<Vec<f32>>,
}
#[cfg(not(windows))]
impl Monitor {
    fn ended(&mut self) -> bool { self.child.try_wait().ok().flatten().is_some() }
    fn start(target: &str) -> Result<Self> {
        let mut command = Command::new("pw-cat");
        if target != DEFAULT_MONITOR { command.args(["--target", target]); }
        let mut child = command
            .args([
                "--playback",
                "--raw",
                "--format",
                "f32",
                "--rate",
                "48000",
                "--channels",
                "2",
                "--latency",
                "60ms",
                "--properties",
                "{ node.name = rosorah-monitor node.description = Rosorah-Monitor }",
                "-",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("Could not start pw-cat playback")?;
        let mut stdin = child.stdin.take().unwrap();
        let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(3);
        thread::spawn(move || {
            for pcm in rx {
                let bytes: Vec<_> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
                if stdin.write_all(&bytes).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            target: target.into(),
            child,
            tx,
        })
    }
}
#[cfg(not(windows))]
impl Drop for Monitor {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
#[cfg(test)]
mod delay_tests {
    use super::*;
    #[test]
    fn delay_preserves_samples_and_resets() {
        let mut d = (0, VecDeque::new());
        let mut a = [1., 2., 3., 4.];
        delay_pcm(&mut d, 2, &mut a);
        assert_eq!(a, [0., 0., 1., 2.]);
        let mut b = [5., 6.];
        delay_pcm(&mut d, 2, &mut b);
        assert_eq!(b, [3., 4.]);
        delay_pcm(&mut d, 0, &mut b);
        assert!(d.1.is_empty());
    }
}

#[cfg(test)]
mod suppression_tests {
    use super::*;
    #[test]
    fn real_filter_reduces_background_noise_without_losing_voice_band() -> Result<()> {
        let run = |signal: &str, mode| -> Result<f32> {
            let out = Command::new("ffmpeg")
                .args([
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    signal,
                    "-af",
                    noise_filter(mode),
                    "-ar",
                    "48000",
                    "-ac",
                    "2",
                    "-f",
                    "f32le",
                    "pipe:1",
                ])
                .output()?;
            ensure!(
                out.status.success(),
                "Noise filter: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            let pcm: Vec<_> = out
                .stdout
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .skip(48000)
                .collect();
            ensure!(!pcm.is_empty(), "Filter returned no sound");
            Ok((pcm.iter().map(|v| v * v).sum::<f32>() / pcm.len() as f32).sqrt())
        };
        use auroracast_core::NoiseSuppression::*;
        let noise = "anoisesrc=amplitude=0.005:duration=2:sample_rate=48000:seed=42";
        let raw = run(noise, Off)?;
        let reduced = run(noise, Strong)?;
        ensure!(
            reduced < raw * 0.65,
            "Noise not reduced: {raw} to {reduced}"
        );
        let tone = run("sine=frequency=1000:duration=2:sample_rate=48000", Gentle)?;
        ensure!(tone > 0.04, "Voice band attenuated too much: {tone}");
        Ok(())
    }
}

#[cfg(test)]
mod immutable_control_tests {
    use super::*;
    #[test]
    fn worker_snapshots_share_settings_without_mutating_existing_mix() {
        let mut c = Control {
            channels: Arc::new(auroracast_core::Project::default().audio),
            targets: Arc::new([(1, "microphone".into())].into()),
            ..Default::default()
        };
        let old = c.snapshot();
        let original = old.channels[0].gain_db;
        for _ in 0..1000 {
            let next = c.snapshot();
            assert!(Arc::ptr_eq(&old.channels, &next.channels));
            assert!(Arc::ptr_eq(&old.targets, &next.targets));
        }
        let mut changed = c.channels.as_ref().clone();
        changed[0].gain_db = -30.;
        c.channels = Arc::new(changed);
        c.targets = Arc::new([(1, "desktop".into())].into());
        assert_eq!(old.channels[0].gain_db, original);
        assert_eq!(old.targets[&1], "microphone");
        assert_eq!(c.snapshot().channels[0].gain_db, -30.);
    }
}

#[cfg(windows)]
include!("platform_windows/audio.rs");
