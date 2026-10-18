//! Shared desktop media discovery. No service account, external polling or child browser.
use auroracast_core::widgets::Music;
use rosorah_capture::Frame;
use std::{
    collections::{HashMap, VecDeque},
    io::Read,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};
use zbus::{
    blocking::{Connection, Proxy},
    zvariant::OwnedValue,
};
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
        let position = self.position
            + if self.playing {
                self.sampled.elapsed().as_secs_f64() * self.rate
            } else {
                0.
            };
        (position / self.length).clamp(0., 1.) as f32
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
            status: "Looking for desktop music players…".into(),
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
pub fn current(settings: &Music) -> Option<Track> {
    let mut c = cache().lock().unwrap();
    c.demand = Instant::now();
    let selected = if settings.player.is_empty() {
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
    selected
        .filter(|t| !t.title.is_empty() && (!settings.hide_paused || t.playing))
        .cloned()
}
pub fn reconnect() {
    let mut c = cache().lock().unwrap();
    c.demand = Instant::now();
    c.status = "Checking desktop players…".into();
}
fn string(m: &HashMap<String, OwnedValue>, k: &str) -> String {
    m.get(k)
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| String::try_from(v).ok())
        .unwrap_or_default()
        .chars()
        .take(500)
        .collect()
}
fn number(m: &HashMap<String, OwnedValue>, k: &str) -> i64 {
    m.get(k).and_then(|v| i64::try_from(v).ok()).unwrap_or(0)
}
fn scan(bus: &Connection) -> anyhow::Result<Vec<(Track, String)>> {
    let dbus = Proxy::new(
        bus,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )?;
    let names: Vec<String> = dbus.call("ListNames", &())?;
    let mut tracks = Vec::new();
    for name in names
        .into_iter()
        .filter(|n| n.starts_with("org.mpris.MediaPlayer2."))
        .take(12)
    {
        let Ok(proxy) = Proxy::new(
            bus,
            name.as_str(),
            "/org/mpris/MediaPlayer2",
            "org.freedesktop.DBus.Properties",
        ) else {
            continue;
        };
        let Ok(props): Result<HashMap<String, OwnedValue>, _> =
            proxy.call("GetAll", &("org.mpris.MediaPlayer2.Player",))
        else {
            continue;
        };
        let Some(meta) = props
            .get("Metadata")
            .and_then(|m| m.try_clone().ok())
            .and_then(|m| HashMap::<String, OwnedValue>::try_from(m).ok())
        else {
            continue;
        };
        let artist = meta
            .get("xesam:artist")
            .and_then(|v| v.try_clone().ok())
            .and_then(|v| Vec::<String>::try_from(v).ok())
            .map(|v| v.into_iter().take(5).collect::<Vec<_>>().join(", "))
            .unwrap_or_default();
        let identity = name
            .trim_start_matches("org.mpris.MediaPlayer2.")
            .split('.')
            .next()
            .unwrap_or("Music")
            .to_string();
        let art = string(&meta, "mpris:artUrl");
        let title = string(&meta, "xesam:title");
        let album = string(&meta, "xesam:album");
        let root: HashMap<String, OwnedValue> = proxy
            .call("GetAll", &("org.mpris.MediaPlayer2",))
            .unwrap_or_default();
        let service = crate::music_service::service(
            &[&name, &string(&root, "Identity"), &string(&root, "DesktopEntry")],
            &[&string(&meta, "xesam:url"), &art],
            &title,
            &artist,
            &album,
        );
        tracks.push((
            Track {
                player: name.clone(),
                identity,
                title,
                artist,
                album,
                playing: string(&props, "PlaybackStatus") == "Playing",
                position: number(&props, "Position").max(0) as f64 / 1e6,
                length: number(&meta, "mpris:length").max(0) as f64 / 1e6,
                rate: props
                    .get("Rate")
                    .and_then(|v| f64::try_from(v).ok())
                    .filter(|v| v.is_finite())
                    .unwrap_or(1.),
                sampled: Instant::now(),
                artwork: None,
                accent: [161, 139, 250, 255],
                service,
            },
            art,
        ));
    }
    tracks.sort_by(|a, b| a.0.player.cmp(&b.0.player));
    Ok(tracks)
}
fn artwork(url: &str) -> Option<(Arc<Frame>, [u8; 4])> {
    let url = reqwest::Url::parse(url).ok()?;
    let bytes = match url.scheme() {
        "file" => {
            let p = url.to_file_path().ok()?;
            if std::fs::metadata(&p).ok()?.len() > 8 * 1024 * 1024 {
                return None;
            }
            std::fs::read(p).ok()?
        }
        "https" | "http" => {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(4))
                .redirect(reqwest::redirect::Policy::limited(3))
                .build()
                .ok()?;
            let res = client.get(url).send().ok()?.error_for_status().ok()?;
            if res.content_length().is_some_and(|n| n > 8 * 1024 * 1024) {
                return None;
            }
            let mut bytes = Vec::new();
            res.take(8 * 1024 * 1024 + 1).read_to_end(&mut bytes).ok()?;
            if bytes.len() > 8 * 1024 * 1024 {
                return None;
            }
            bytes
        }
        _ => return None,
    };
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let img = reader
        .decode()
        .ok()?
        .resize_to_fill(320, 320, image::imageops::FilterType::Triangle)
        .into_rgba8();
    let mut sum = [0u64; 3];
    let mut count = 0;
    for p in img.pixels().step_by(31) {
        if p[3] > 128 {
            for c in 0..3 {
                sum[c] += u64::from(p[c]);
            }
            count += 1;
        }
    }
    let mut accent = [161, 139, 250, 255];
    if count > 0 {
        for c in 0..3 {
            accent[c] = ((sum[c] / count) as u8).saturating_add(35).max(70);
        }
    }
    Some((
        Arc::new(Frame {
            width: 320,
            height: 320,
            rgba: img.into_raw(),
        }),
        accent,
    ))
}
fn worker() {
    let mut bus = None;
    let mut art: VecDeque<(String, Option<(Arc<Frame>, [u8; 4])>)> = VecDeque::new();
    loop {
        std::thread::sleep(Duration::from_secs(2));
        if cache().lock().unwrap().demand.elapsed() > Duration::from_secs(4) {
            continue;
        }
        if bus.is_none() {
            bus = zbus::blocking::connection::Builder::session()
                .and_then(|b| b.method_timeout(Duration::from_secs(2)).build())
                .ok();
        }
        let result = bus
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Desktop media connection unavailable"))
            .and_then(scan);
        match result {
            Ok(rows) => {
                let mut tracks = Vec::new();
                for (mut t, url) in rows {
                    if !url.is_empty() {
                        if !art.iter().any(|(key, _)| key == &url) {
                            art.push_back((url.clone(), artwork(&url)));
                            while art.len() > 12 {
                                art.pop_front();
                            }
                        }
                        if let Some((_, Some((frame, accent)))) =
                            art.iter().find(|(key, _)| key == &url)
                        {
                            t.artwork = Some(frame.clone());
                            t.accent = *accent;
                        }
                    }
                    tracks.push(t);
                }
                let mut c = cache().lock().unwrap();
                c.status = if tracks.is_empty() {
                    "No desktop player detected. Start music in an app or browser that publishes media controls.".into()
                } else {
                    format!("{} desktop player(s) detected", tracks.len())
                };
                c.tracks = tracks;
            }
            Err(_) => {
                bus = None;
                let mut c = cache().lock().unwrap();
                c.tracks.clear();
                c.status="Desktop media connection unavailable. Rosorah must run in your desktop session.".into();
            }
        }
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
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paused_position_does_not_advance() {
        let mut t = sample();
        t.playing = false;
        t.position = 50.;
        t.length = 100.;
        assert!((t.progress() - 0.5).abs() < 0.001);
    }
    #[test]
    fn unknown_duration_is_safe() {
        let mut t = sample();
        t.length = 0.;
        assert_eq!(t.progress(), 0.);
    }
    struct FakePlayer;
    #[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
    impl FakePlayer {
        #[zbus(property)]
        fn playback_status(&self) -> String {
            "Playing".into()
        }
        #[zbus(property)]
        fn position(&self) -> i64 {
            30_000_000
        }
        #[zbus(property)]
        fn rate(&self) -> f64 {
            1.0
        }
        #[zbus(property)]
        fn metadata(&self) -> HashMap<String, OwnedValue> {
            let mut m = HashMap::new();
            m.insert(
                "xesam:title".into(),
                OwnedValue::from(zbus::zvariant::Str::from("Verified test track")),
            );
            m.insert("mpris:length".into(), OwnedValue::from(120_000_000i64));
            m
        }
    }
    #[test]
    #[ignore = "Requires an isolated D-Bus session"]
    fn private_bus_media_discovery() {
        let _service = zbus::blocking::connection::Builder::session()
            .unwrap()
            .name("org.mpris.MediaPlayer2.rosorah_test")
            .unwrap()
            .serve_at("/org/mpris/MediaPlayer2", FakePlayer)
            .unwrap()
            .build()
            .unwrap();
        let client = Connection::session().unwrap();
        let tracks = scan(&client).unwrap();
        let (t, _) = tracks
            .iter()
            .find(|t| t.0.player == "org.mpris.MediaPlayer2.rosorah_test")
            .expect("discover registered player");
        assert_eq!(t.title, "Verified test track");
        assert!(t.playing);
        assert_eq!(t.position, 30.);
        assert_eq!(t.length, 120.);
        assert!((t.progress() - 0.25).abs() < 0.01);
    }
}
