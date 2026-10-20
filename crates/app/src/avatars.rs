//! Chatters' profile pictures for the chat widget. YouTube and TikTok chat pages show them, so
//! their addresses arrive with each message; Twitch's are looked up by the Twitch account worker,
//! and Kick's from the chatter's public channel data. Each is fetched once and kept at 64×64.
use image::RgbaImage;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex, OnceLock, mpsc},
    time::{Duration, Instant},
};

const SIZE: u32 = 64;
const KEEP: usize = 400;
/// A chatter without a picture is asked about again after this long.
const RETRY: Duration = Duration::from_secs(600);
/// How long a new chat message waits for its chatter's picture.
pub const WAIT: Duration = Duration::from_millis(500);

enum Job {
    Download(String, String),
    Kick(String, String),
}
#[derive(Default)]
struct Store {
    /// A picture address or account id seen with each chatter's messages.
    hints: HashMap<String, String>,
    /// None while loading, or when the chatter has no picture.
    pictures: HashMap<String, (Option<Arc<RgbaImage>>, Instant)>,
    order: VecDeque<String>,
    /// Pictures being fetched or looked up.
    loading: HashSet<String>,
    /// Twitch chatters to look up: (key, user id or login).
    twitch: Vec<(String, String)>,
    jobs: Option<mpsc::Sender<Job>>,
}
fn store() -> &'static Mutex<Store> {
    static STORE: OnceLock<Mutex<Store>> = OnceLock::new();
    STORE.get_or_init(Default::default)
}
fn key(platform: &str, author: &str) -> String {
    format!("{platform}:{}", author.to_lowercase())
}
/// Remembers a chatter's picture address (YouTube, TikTok) or account id or login (Twitch).
pub fn note(platform: &str, author: &str, hint: &str) {
    let hint = hint.trim();
    if hint.is_empty() || hint.len() > 2048 || author.is_empty() {
        return;
    }
    let mut s = store().lock().unwrap();
    let key = key(platform, author);
    if s.hints.get(&key).map(String::as_str) == Some(hint) {
        return;
    }
    if s.hints.len() > 4000 {
        s.hints.clear();
    }
    // A new address for someone whose picture failed to load is worth another try.
    if s.pictures.get(&key).is_some_and(|(p, _)| p.is_none()) && hint.starts_with("https://") {
        s.pictures.remove(&key);
    }
    s.hints.insert(key, hint.into());
}
/// The chatter's picture, if it has loaded. Asking starts loading it.
pub fn get(platform: &str, author: &str) -> Option<Arc<RgbaImage>> {
    if author.is_empty() || author == "Activity" {
        return None;
    }
    let key = key(platform, author);
    let mut s = store().lock().unwrap();
    if let Some((picture, at)) = s.pictures.get(&key) {
        if picture.is_some() || at.elapsed() < RETRY {
            return picture.clone();
        }
    }
    s.pictures.insert(key.clone(), (None, Instant::now()));
    s.order.retain(|k| k != &key);
    s.order.push_back(key.clone());
    while s.order.len() > KEEP {
        if let Some(old) = s.order.pop_front() {
            s.pictures.remove(&old);
        }
    }
    let hint = s.hints.get(&key).cloned().unwrap_or_default();
    if hint.starts_with("https://") {
        s.loading.insert(key.clone());
        s.send(Job::Download(key, hint));
    } else if platform == "Twitch" {
        let lookup = if hint.is_empty() { author.to_lowercase() } else { hint };
        if valid_twitch(&lookup) && s.twitch.len() < 300 {
            s.loading.insert(key.clone());
            s.twitch.push((key, lookup));
        }
    } else if platform == "Kick" {
        s.loading.insert(key.clone());
        s.send(Job::Kick(key, author.into()));
    }
    None
}
/// True once the chatter's picture has loaded, turned out not to exist, or has taken longer than
/// WAIT (it then appears when it arrives).
pub fn ready(platform: &str, author: &str) -> bool {
    let key = key(platform, author);
    let s = store().lock().unwrap();
    match s.pictures.get(&key) {
        Some((Some(_), _)) | None => true,
        Some((None, at)) => !s.loading.contains(&key) || at.elapsed() >= WAIT,
    }
}
impl Store {
    fn send(&mut self, job: Job) {
        if cfg!(test) {
            return;
        }
        let jobs = self.jobs.get_or_insert_with(|| {
            let (tx, rx) = mpsc::channel();
            let _ = std::thread::Builder::new().name("chat-avatars".into()).spawn(move || work(rx));
            tx
        });
        let _ = jobs.send(job);
    }
}
fn valid_twitch(lookup: &str) -> bool {
    (1..=25).contains(&lookup.len()) && lookup.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}
/// Twitch chatters waiting to be looked up, for the Twitch account worker: (key, id or login).
pub fn take_twitch() -> Vec<(String, String)> {
    std::mem::take(&mut store().lock().unwrap().twitch)
}
/// The Twitch account worker found a chatter's picture address (or found none).
pub fn found(key: String, url: Option<String>) {
    let mut s = store().lock().unwrap();
    match url {
        Some(url) => s.send(Job::Download(key, url)),
        None => {
            s.loading.remove(&key);
            s.pictures.insert(key, (None, Instant::now()));
        }
    }
}
fn work(rx: mpsc::Receiver<Job>) {
    let Ok(client) = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(3))
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) Rosorah Studio")
        .build()
    else {
        return;
    };
    for job in rx {
        let (key, picture) = match job {
            Job::Download(key, url) => {
                let picture = download(&client, &url);
                (key, picture)
            }
            Job::Kick(key, name) => {
                let picture = kick_picture(&client, &name).and_then(|url| download(&client, &url));
                (key, picture)
            }
        };
        let mut s = store().lock().unwrap();
        s.loading.remove(&key);
        // Forgotten while it loaded: keep it anyway only if someone still wants it.
        if s.pictures.contains_key(&key) {
            s.pictures.insert(key, (picture.map(Arc::new), Instant::now()));
            drop(s);
            crate::chat_render::canvas_changed();
        }
    }
}
/// Kick's public channel data has each user's picture; a channel's address is its owner's name.
fn kick_picture(client: &reqwest::blocking::Client, name: &str) -> Option<String> {
    let slug = name.trim().to_lowercase().replace('_', "-");
    if slug.is_empty() || slug.len() > 50 || !slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    let v: serde_json::Value = client.get(format!("https://kick.com/api/v2/channels/{slug}")).send().ok()?.error_for_status().ok()?.json().ok()?;
    v["user"]["profile_pic"].as_str().filter(|u| !u.is_empty()).map(Into::into)
}
pub fn safe(url: &str) -> bool {
    let Ok(u) = reqwest::Url::parse(url) else { return false };
    let host = u.host_str().unwrap_or("");
    u.scheme() == "https"
        && u.username().is_empty()
        && u.port().is_none()
        && (matches!(host, "static-cdn.jtvnw.net" | "yt3.ggpht.com" | "yt4.ggpht.com" | "yt3.googleusercontent.com" | "files.kick.com")
            || [".tiktokcdn.com", ".tiktokcdn-us.com", ".tiktokcdn-eu.com"].iter().any(|d| host.ends_with(d)))
}
fn download(client: &reqwest::blocking::Client, url: &str) -> Option<RgbaImage> {
    // YouTube's chat shows 32-pixel pictures; ask for a sharper one.
    let url = url.replacen("=s32-", "=s88-", 1).replacen("=s64-", "=s88-", 1);
    if !safe(&url) {
        return None;
    }
    let reply = client.get(&url).send().ok()?.error_for_status().ok()?;
    if reply.content_length().is_some_and(|n| n > 4 << 20) {
        return None;
    }
    let bytes = reply.bytes().ok()?;
    if bytes.len() > 4 << 20 {
        return None;
    }
    let image = image::load_from_memory(&bytes).ok()?;
    if image.width() == 0 || image.height() == 0 || image.width() > 4096 || image.height() > 4096 {
        return None;
    }
    Some(image.resize_to_fill(SIZE, SIZE, image::imageops::FilterType::Triangle).to_rgba8())
}
#[cfg(test)]
pub fn insert(platform: &str, author: &str, picture: RgbaImage) {
    let mut s = store().lock().unwrap();
    s.pictures.insert(key(platform, author), (Some(Arc::new(picture)), Instant::now()));
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_known_picture_hosts_are_fetched() {
        assert!(safe("https://static-cdn.jtvnw.net/jtv_user_pictures/x-profile_image-300x300.png"));
        assert!(safe("https://yt4.ggpht.com/abc=s32-c-k-c0x00ffffff-no-rj"));
        assert!(safe("https://p16-sign-va.tiktokcdn.com/tos-maliva-avt-0068/x.webp"));
        assert!(!safe("http://static-cdn.jtvnw.net/x.png"));
        assert!(!safe("https://evil.example/x.png"));
        assert!(!safe("https://tiktokcdn.com.evil.example/x.png"));
        assert!(!valid_twitch("名前") && valid_twitch("some_viewer") && valid_twitch("12345"));
    }
    #[test]
    fn twitch_chatters_queue_for_lookup_once() {
        note("Twitch", "Queued_Viewer", "998877");
        assert!(get("Twitch", "Queued_Viewer").is_none());
        assert!(get("Twitch", "queued_viewer").is_none());
        let queued: Vec<_> = take_twitch().into_iter().filter(|(k, _)| k == "Twitch:queued_viewer").collect();
        assert_eq!(queued, [("Twitch:queued_viewer".to_string(), "998877".to_string())]);
        // Its messages wait for the picture, but not for long.
        assert!(!ready("Twitch", "Queued_Viewer"));
        std::thread::sleep(WAIT);
        assert!(ready("Twitch", "Queued_Viewer"));
        found("Twitch:queued_viewer".into(), None);
        assert!(ready("Twitch", "Queued_Viewer"));
    }
}
