//! Windows system media sessions, sampled on demand off the UI thread.
use rosorah_capture::Frame;
use std::{
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};
use windows::{Media::Control::*, Storage::Streams::DataReader};
#[derive(Clone)]
pub struct Track {
    pub player: String,
    pub identity: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub playing: bool,
    pub position: f64,
    pub length: f64,
    pub rate: f64,
    pub sampled: Instant,
    pub artwork: Option<Arc<Frame>>,
    pub accent: [u8; 4],
    /// The music service playing this track; `None` for videos and other media.
    pub service: Option<&'static str>,
}
impl Track {
    pub fn progress(&self) -> f32 {
        if self.length <= 0. {
            return 0.;
        }
        ((self.position
            + if self.playing {
                self.sampled.elapsed().as_secs_f64() * self.rate
            } else {
                0.
            })
            / self.length)
            .clamp(0., 1.) as f32
    }
}
struct Cache {
    tracks: Vec<Track>,
    status: String,
    demand: Instant,
}
fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| {
        std::thread::spawn(worker);
        Mutex::new(Cache {
            tracks: Vec::new(),
            status: "Looking for Windows music sessions…".into(),
            demand: Instant::now(),
        })
    })
}
pub fn players() -> (Vec<(String, String)>, String) {
    let mut c = cache().lock().unwrap();
    c.demand = Instant::now();
    (
        c.tracks
            .iter()
            .map(|t| (t.player.clone(), t.identity.clone()))
            .collect(),
        c.status.clone(),
    )
}
pub fn current(settings: &auroracast_core::widgets::Music) -> Option<Track> {
    let mut c = cache().lock().unwrap();
    c.demand = Instant::now();
    let t = if settings.player.is_empty() {
        // Automatic mode only follows music apps and sites unless the widget
        // allows any media player.
        let mut music = c
            .tracks
            .iter()
            .filter(|t| !settings.music_apps_only || t.service.is_some());
        let first = music.clone().next();
        music
            .find(|t| t.playing && !t.title.is_empty())
            .or(first)
    } else {
        c.tracks.iter().find(|t| t.player == settings.player)
    };
    t.filter(|t| !t.title.is_empty() && (!settings.hide_paused || t.playing))
        .cloned()
}
pub fn reconnect() {
    let mut c = cache().lock().unwrap();
    c.demand = Instant::now();
    c.status = "Checking Windows music sessions…".into();
}
fn wait<T: windows::core::RuntimeType>(
    operation: windows::Foundation::IAsyncOperation<T>,
) -> windows::core::Result<T> {
    let start = Instant::now();
    while operation.Status()? == windows::Foundation::AsyncStatus::Started {
        if start.elapsed() > Duration::from_secs(2) {
            let _ = operation.Cancel();
            return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                0x800705b4u32 as i32,
            )));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    operation.GetResults()
}
fn artwork(props: &GlobalSystemMediaTransportControlsSessionMediaProperties) -> Option<Arc<Frame>> {
    let stream = wait(props.Thumbnail().ok()?.OpenReadAsync().ok()?).ok()?;
    let size = stream.Size().ok()?;
    if size == 0 || size > 8 * 1024 * 1024 {
        return None;
    }
    let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0).ok()?).ok()?;
    let load = reader.LoadAsync(size as u32).ok()?;
    let started = Instant::now();
    while load.Status().ok()? == windows::Foundation::AsyncStatus::Started {
        if started.elapsed() > Duration::from_secs(2) {
            let _ = load.Cancel();
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if load.GetResults().ok()? != size as u32 {
        return None;
    }
    let mut bytes = vec![0; size as usize];
    reader.ReadBytes(&mut bytes).ok()?;
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().ok()?.thumbnail(512, 512).into_rgba8();
    Some(Arc::new(Frame { shared: None,
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    }))
}
fn worker() {
    unsafe {
        if windows::Win32::System::WinRT::RoInitialize(
            windows::Win32::System::WinRT::RO_INIT_MULTITHREADED,
        )
        .is_err()
        {
            return;
        }
    }
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                windows::Win32::System::WinRT::RoUninitialize();
            }
        }
    }
    let _apartment = Apartment;
    let mut manager = None;
    let mut art = std::collections::BTreeMap::<String, Option<Arc<Frame>>>::new();
    loop {
        if cache().lock().unwrap().demand.elapsed() < Duration::from_secs(10) {
            if manager.is_none() {
                manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()
                    .ok()
                    .and_then(|op| wait(op).ok());
            }
            let mut tracks = Vec::new();
            if let Some(m) = &manager {
                if let Ok(sessions) = m.GetSessions() {
                    for session in sessions.into_iter().take(16) {
                        let Ok(player) = session.SourceAppUserModelId() else {
                            continue;
                        };
                        let Some(props) = session
                            .TryGetMediaPropertiesAsync()
                            .ok()
                            .and_then(|op| wait(op).ok())
                        else {
                            continue;
                        };
                        let title = props.Title().map(|v| v.to_string()).unwrap_or_default();
                        let artist = props.Artist().map(|v| v.to_string()).unwrap_or_default();
                        let album = props
                            .AlbumTitle()
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        let player = player.to_string();
                        let key = format!("{player}\0{title}\0{artist}\0{album}");
                        if !art.contains_key(&key) {
                            if art.len() >= 32 {
                                art.clear();
                            }
                            art.insert(key.clone(), artwork(&props));
                        }
                        let playback = session.GetPlaybackInfo().ok();
                        let playing = playback.as_ref().and_then(|p| p.PlaybackStatus().ok())
                            == Some(
                                GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing,
                            );
                        let rate = playback
                            .as_ref()
                            .and_then(|p| p.PlaybackRate().ok())
                            .and_then(|r| r.Value().ok())
                            .filter(|r| r.is_finite())
                            .unwrap_or(1.);
                        let timeline = session.GetTimelineProperties().ok();
                        let start = timeline
                            .as_ref()
                            .and_then(|t| t.StartTime().ok())
                            .map(|v| v.Duration)
                            .unwrap_or(0);
                        let length = timeline
                            .as_ref()
                            .and_then(|t| t.EndTime().ok())
                            .map(|v| v.Duration.saturating_sub(start).max(0) as f64 / 1e7)
                            .unwrap_or(0.);
                        let position = timeline
                            .as_ref()
                            .and_then(|t| t.Position().ok())
                            .map(|v| v.Duration.saturating_sub(start).max(0) as f64 / 1e7)
                            .unwrap_or(0.);
                        let artwork = art.get(&key).cloned().flatten();
                        let accent = artwork
                            .as_ref()
                            .map(|f| {
                                let mut sum = [0u64; 3];
                                let mut n = 0u64;
                                for p in f.rgba.chunks_exact(4).step_by(64) {
                                    if p[3] > 128 {
                                        for i in 0..3 {
                                            sum[i] += p[i] as u64;
                                        }
                                        n += 1;
                                    }
                                }
                                if n > 0 {
                                    [
                                        (sum[0] / n) as u8,
                                        (sum[1] / n) as u8,
                                        (sum[2] / n) as u8,
                                        255,
                                    ]
                                } else {
                                    [144, 183, 198, 255]
                                }
                            })
                            .unwrap_or([144, 183, 198, 255]);
                        let service = crate::music_service::service(
                            &[&player],
                            &[],
                            &title,
                            &artist,
                            &album,
                        );
                        tracks.push(Track {
                            service,
                            identity: player.clone(),
                            player,
                            title,
                            artist,
                            album,
                            playing,
                            position,
                            length,
                            rate,
                            sampled: Instant::now(),
                            artwork,
                            accent,
                        });
                    }
                } else {
                    manager = None
                }
            }
            tracks.sort_by(|a, b| a.player.cmp(&b.player));
            let mut c = cache().lock().unwrap();
            c.status = if tracks.is_empty() {
                "No music session found. Open a player that publishes Windows media controls."
                    .into()
            } else {
                format!("{} Windows media player(s) connected", tracks.len())
            };
            c.tracks = tracks;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}
pub fn sample() -> Track {
    Track {
        player: "sample".into(),
        identity: "SAMPLE TRACK".into(),
        title: "After the rain".into(),
        artist: "Luna & The Satellites".into(),
        album: "Midnight landscapes".into(),
        playing: true,
        position: 78.,
        length: 224.,
        rate: 0.,
        sampled: Instant::now(),
        artwork: None,
        accent: [144, 183, 198, 255],
        service: Some("Spotify"),
    }
}
