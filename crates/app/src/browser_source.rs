//! Browser frames use the same latest-frame engine as cameras and recordings.
use anyhow::{Context, Result, ensure};
use auroracast_core::{Id, browser::Settings};
use rosorah_capture::{Backend, Frame, Options, Producer};
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};
use tokio::sync::watch;
pub fn ready() -> bool {
    crate::browser_chat::venv()
        .join("rosorah-runtime-6.10.2.ready")
        .is_file()
}
pub fn valid_url(url: &str) -> bool {
    reqwest::Url::parse(url).is_ok_and(|u| {
        matches!(u.scheme(), "https" | "http")
            && u.host_str().is_some()
            && u.username().is_empty()
            && u.password().is_none()
    })
}
/// A browser source can also show an overlay saved on this PC (`file:///…`), like OBS's
/// "Local file" option.
pub fn valid_source(url: &str) -> bool {
    valid_url(url) || local_file(url).is_some()
}
pub fn local_file(url: &str) -> Option<std::path::PathBuf> {
    reqwest::Url::parse(url)
        .ok()
        .filter(|u| u.scheme() == "file" && u.host_str().is_none_or(|h| h.is_empty() || h == "localhost"))
        .and_then(|u| u.to_file_path().ok())
}
/// Turns what was typed or picked (a web address, a file URL or a plain path) into the address
/// the page loads.
pub fn address(text: &str) -> Option<String> {
    let text = text.trim();
    if valid_source(text) {
        return Some(text.to_owned());
    }
    let path = std::path::Path::new(text);
    (path.is_absolute() && path.is_file())
        .then(|| reqwest::Url::from_file_path(path).ok())
        .flatten()
        .map(String::from)
}
/// Where a browser source keeps its sign-ins and cookies. Each source has its own, apart from
/// Rosorah's account sign-ins.
pub fn storage(id: Id) -> std::path::PathBuf {
    auroracast_core::platform::config_root().join(format!("rosorah-studio/browser-sources/{id}"))
}
pub struct BrowserBackend {
    pub id: Id,
    pub url: String,
    pub settings: Settings,
    pub audio: crate::video::AudioBus,
    pub commands: mpsc::Receiver<serde_json::Value>,
}
impl Backend for BrowserBackend {
    fn run(
        self: Box<Self>,
        output: Producer,
        cancel: watch::Receiver<bool>,
        _: Options,
    ) -> Result<()> {
        self.settings.validate()?;
        ensure!(valid_source(&self.url), "Use an HTTP or HTTPS address or a local file");
        ensure!(
            ready(),
            "Browser runtime is not ready; prepare it in source settings"
        );
        let mut child = Command::new(crate::browser_chat::python())
            .arg(crate::browser_chat::base().join("source_browser.py"))
            .env_remove("LD_LIBRARY_PATH")
            .env_remove("QT_PLUGIN_PATH")
            .env_remove("QT_QPA_PLATFORM_PLUGIN_PATH")
            .env("QT_SCALE_FACTOR", "1")
            .env("QT_AUTO_SCREEN_SCALE_FACTOR", "0")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("Start browser source")?;
        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut config = serde_json::to_value(&self.settings)?;
        config["url"] = self.url.clone().into();
        config["storage"] = storage(self.id).display().to_string().into();
        // On Windows the page plays normally and Rosorah records the browser's own sound.
        config["system_audio"] = cfg!(windows).into();
        #[cfg(windows)]
        let windows_audio = self.settings.audio.then(|| {
            let (pid, bus, id) = (child.id(), self.audio.clone(), self.id);
            let stop = Arc::new(AtomicBool::new(false));
            let flag = stop.clone();
            let worker = std::thread::spawn(move || {
                // Retries while the page starts its sound process; reports only the first failure.
                let mut logged = false;
                while !flag.load(Ordering::Relaxed) {
                    let result = rosorah_windows::audio::run(rosorah_windows::audio::Input::Process(pid), &flag, |packet| {
                        let bytes: Vec<u8> = packet.samples.iter().flat_map(|v| v.to_le_bytes()).collect();
                        bus.push(id, &bytes);
                        Ok(())
                    });
                    if let Err(e) = result
                        && !std::mem::replace(&mut logged, true)
                    {
                        crate::capture_log::event(format!("Browser source sound: {e}"));
                    }
                    for _ in 0..20 {
                        if flag.load(Ordering::Relaxed) {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
            });
            (stop, worker)
        });
        if writeln!(stdin, "{config}").is_err() {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("Browser configuration failed");
        }
        let done = Arc::new(AtomicBool::new(false));
        let finished = done.clone();
        let audio = self.audio.clone();
        let id = self.id;
        let commands = self.commands;
        let controller = std::thread::spawn(move || {
            while !finished.load(Ordering::Relaxed) && !*cancel.borrow() {
                if let Ok(command) = commands.recv_timeout(Duration::from_millis(50))
                    && writeln!(stdin, "{command}").is_err()
                {
                    break;
                }
                if child.try_wait().ok().flatten().is_some() {
                    break;
                }
            }
            let _ = writeln!(stdin, "{{\"type\":\"quit\"}}");
            drop(stdin);
            for _ in 0..60 {
                if child.try_wait().ok().flatten().is_some() {
                    audio.clear(id);
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            let _ = child.kill();
            let _ = child.wait();
            audio.clear(id);
        });
        let result = read_packets(stdout, &output, &self.audio, self.id);
        done.store(true, Ordering::Relaxed);
        let _ = controller.join();
        #[cfg(windows)]
        if let Some((stop, worker)) = windows_audio {
            stop.store(true, Ordering::Relaxed);
            let _ = worker.join();
        }
        self.audio.clear(self.id);
        result
    }
}
fn read_packets(
    mut input: impl Read,
    output: &Producer,
    audio: &crate::video::AudioBus,
    id: Id,
) -> Result<()> {
    let mut detail = "Loading browser…".to_string();
    loop {
        let mut header = [0u8; 20];
        input
            .read_exact(&mut header)
            .context("Browser source exited; refresh to reconnect")?;
        ensure!(&header[..4] == b"RSBW", "Invalid browser packet");
        let number = |i| u32::from_le_bytes(header[i..i + 4].try_into().unwrap());
        let (kind, w, h, n) = (number(4), number(8), number(12), number(16));
        ensure!(
            packet_valid(kind, w, h, n),
            "Invalid browser frame or audio size"
        );
        let mut bytes = vec![0; n as usize];
        input.read_exact(&mut bytes)?;
        match kind {
            1 => {
                output.publish_with_detail(
                    Frame { shared: None,
                        width: w,
                        height: h,
                        rgba: bytes,
                    },
                    Some(&detail),
                )?;
            }
            2 => audio.push(id, &bytes),
            3 => {
                detail = String::from_utf8_lossy(&bytes).into_owned();
                output.note(&detail);
            }
            _ => unreachable!(),
        }
    }
}
fn packet_valid(kind: u32, w: u32, h: u32, n: u32) -> bool {
    match kind {
        1 => {
            (16..=1920).contains(&w)
                && (16..=1920).contains(&h)
                && w * h <= 1920 * 1080
                && n == w * h * 4
        }
        2 => w == 0 && h == 0 && n > 0 && n <= 38400 && n.is_multiple_of(8),
        3 => w == 0 && h == 0 && n <= 4096,
        _ => false,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_packet_before_allocation() {
        assert!(packet_valid(1, 1280, 720, 1280 * 720 * 4));
        assert!(!packet_valid(1, u32::MAX, 4, 16));
        assert!(!packet_valid(2, 0, 0, 7));
        assert!(packet_valid(2, 0, 0, 3840));
        assert!(!packet_valid(3, 0, 0, 5000));
    }
    #[test]
    fn overlay_urls_are_explicit() {
        assert!(valid_url("https://example.com/overlay?token=abc"));
        assert!(valid_url("http://localhost:3000"));
        assert!(!valid_url("file:///etc/passwd"));
        assert!(!valid_url("javascript:alert(1)"));
        assert!(valid_source("file:///home/me/overlay.html"));
        assert!(!valid_source("file://server/share/overlay.html"));
        assert!(!valid_source("javascript:alert(1)"));
    }
    #[test]
    fn typed_paths_become_file_addresses() {
        let dir = std::env::temp_dir().join(format!("rosorah-overlay-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("my overlay.html");
        std::fs::write(&file, "<p>hi</p>").unwrap();
        let url = address(&file.display().to_string()).unwrap();
        assert!(url.starts_with("file:///") && url.ends_with("my%20overlay.html"));
        assert_eq!(local_file(&url).unwrap(), file);
        assert_eq!(address(" https://example.com/a ").as_deref(), Some("https://example.com/a"));
        assert_eq!(address(&dir.join("missing.html").display().to_string()), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    #[test]
    #[ignore = "requires local browser fixture, Qt, PulseAudio, FFmpeg and Vulkan"]
    fn browser_reaches_recording_with_alpha_audio_and_cancellation() -> Result<()> {
        use auroracast_core::{Layer, Project, SourceKind};
        use std::time::Instant;
        let url = std::env::var("ROSORAH_BROWSER_QA_URL")?;
        let dir = std::path::PathBuf::from(std::env::var("ROSORAH_BROWSER_QA_DIR")?);
        let mut p = Project::default();
        p.sources[0].kind = SourceKind::Browser { url: url.clone() };
        p.sources[1].kind = SourceKind::Color([0, 0, 255, 255]);
        p.scenes[0].layers = vec![
            Layer { crop: [0.;4], group: 0, frame: Default::default(), look: Default::default(),
                source: 2,
                rect: [0., 0., 1., 1.],
                visible: true,
                locked: false,
                opacity: 1.,
            },
            Layer { crop: [0.;4], group: 0, frame: Default::default(), look: Default::default(),
                source: 1,
                rect: [0., 0., 1., 1.],
                visible: true,
                locked: false,
                opacity: 1.,
            },
        ];
        p.canvases[0].width = 320;
        p.canvases[0].height = 240;
        p.canvases[0].fps = 15;
        p.preferences.recording_encoder = auroracast_core::RecordingEncoder::Software;
        let bus = crate::video::AudioBus::default();
        let mut engine = rosorah_capture::Engine::default();
        let (_tx, rx) = mpsc::sync_channel(4);
        engine.start(
            1,
            Box::new(BrowserBackend {
                id: 1,
                url,
                settings: Settings {
                    width: 320,
                    height: 240,
                    fps: 15,
                    audio: true,
                    enabled: true,
                    ..Default::default()
                },
                audio: bus.clone(),
                commands: rx,
            }),
            Options::default(),
        )?;
        let limit = Instant::now() + Duration::from_secs(15);
        loop {
            if engine.reader().latest().iter().any(|(_, f)| {
                f.as_ref()
                    .is_some_and(|f| f.rgba[10 * 320 * 4 + 10 * 4] > 240)
            }) {
                break;
            }
            ensure!(
                Instant::now() < limit,
                "Browser never supplied a red frame: {:?}",
                engine.snapshot(1)
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let recording = crate::recording::Recording::start(&p, 0, engine.reader(), &dir)?;
        let limit = Instant::now() + Duration::from_secs(15);
        while recording.view().phase != crate::recording::Phase::Recording {
            ensure!(
                Instant::now() < limit && recording.view().result.is_none(),
                "Recording failed: {:?}",
                recording.view().result
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        for _ in 0..180 {
            let pcm = bus.read().remove(&1).unwrap_or_else(|| vec![0.; 960]);
            recording.outlet().push(Instant::now(), pcm);
            std::thread::sleep(Duration::from_millis(10));
        }
        recording.stop();
        let limit = Instant::now() + Duration::from_secs(15);
        let path = loop {
            if let Some(result) = recording.view().result {
                break result.map_err(anyhow::Error::msg)?;
            }
            ensure!(Instant::now() < limit, "Recording stop timeout");
            std::thread::sleep(Duration::from_millis(20));
        };
        engine.stop(1);
        let limit = Instant::now() + Duration::from_secs(5);
        while engine.running(1) {
            ensure!(Instant::now() < limit, "Browser failed to stop");
            std::thread::sleep(Duration::from_millis(20));
        }
        let pixels = Command::new("ffmpeg")
            .args(["-v", "error", "-ss", "0.5", "-i"])
            .arg(&path)
            .args([
                "-frames:v",
                "1",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgb24",
                "pipe:1",
            ])
            .output()?;
        ensure!(
            pixels.status.success() && pixels.stdout.len() == 320 * 240 * 3,
            "Decode browser recording"
        );
        let at = |x: usize, y: usize| &pixels.stdout[(y * 320 + x) * 3..(y * 320 + x) * 3 + 3];
        ensure!(
            at(10, 10)[0] > 220,
            "Browser red pixel missing from recording"
        );
        ensure!(
            at(300, 200)[2] > 220 && at(300, 200)[0] < 30,
            "Transparent browser background did not reveal blue layer"
        );
        let audio = Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&path)
            .args(["-vn", "-f", "f32le", "-ac", "2", "-ar", "48000", "pipe:1"])
            .output()?;
        ensure!(
            audio.status.success()
                && audio
                    .stdout
                    .chunks_exact(4)
                    .any(|b| f32::from_le_bytes(b.try_into().unwrap()).abs() > 0.01),
            "Browser audio missing from recording"
        );
        Ok(())
    }
}
