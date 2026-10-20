//! "I'm live" posts to Discord (a channel webhook), Bluesky (handle + app password) and Threads
//! (an access token from the user's own Meta app), sent once per stream when it starts sending.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

pub const DEFAULT_MESSAGE: &str = "🔴 I'm live! {title}\n{links}";
/// Bluesky counts graphemes; characters are a close, slightly cautious stand-in.
pub const BLUESKY_LIMIT: usize = 300;
const DISCORD_LIMIT: usize = 2000;
pub const THREADS_LIMIT: usize = 500;
const THREADS_API: &str = "https://graph.threads.net";

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub message: String,
    pub discord: bool,
    /// Secret: kept in the keyring with the stream keys.
    pub discord_webhook: String,
    pub bluesky: bool,
    pub bluesky_handle: String,
    /// Secret: a Bluesky app password, kept in the keyring with the stream keys.
    pub bluesky_password: String,
    pub threads: bool,
    /// Secret: a Threads access token, kept in the keyring with the stream keys. Renewed on each
    /// post, since a token unused for 60 days can't be renewed any more.
    pub threads_token: String,
    /// When the token runs out (Unix seconds); 0 until Rosorah has renewed it once.
    pub threads_expires: u64,
    /// The account the token posts as, learned when it's first used.
    pub threads_username: String,
    /// An optional picture (PNG or JPEG) posted with the announcement.
    pub image: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            message: DEFAULT_MESSAGE.into(),
            discord: false,
            discord_webhook: String::new(),
            bluesky: false,
            bluesky_handle: String::new(),
            bluesky_password: String::new(),
            threads: false,
            threads_token: String::new(),
            threads_expires: 0,
            threads_username: String::new(),
            image: String::new(),
        }
    }
}
impl Settings {
    pub fn discord_ready(&self) -> bool {
        self.discord && valid_webhook(&self.discord_webhook)
    }
    pub fn bluesky_ready(&self) -> bool {
        self.bluesky && !handle(&self.bluesky_handle).is_empty() && !self.bluesky_password.trim().is_empty()
    }
    pub fn threads_ready(&self) -> bool {
        self.threads && !self.threads_token.trim().is_empty()
    }
}
/// Bluesky refuses images over about 1 MB; Discord allows far more, so one size suits both.
const IMAGE_LIMIT: usize = 950_000;
/// An image ready to post: its bytes, type and file name.
pub struct Picture {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    pub name: String,
}
/// Reads the announcement image, shrinking it (as a JPEG) when it's too big to post.
pub fn load_picture(path: &str) -> Result<Picture> {
    let path = std::path::Path::new(path.trim());
    let extension = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    ensure!(matches!(extension.as_str(), "png" | "jpg" | "jpeg"), "Use a PNG or JPEG image");
    let bytes = std::fs::read(path).with_context(|| format!("Couldn't read {}", path.display()))?;
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "announcement".into());
    if bytes.len() <= IMAGE_LIMIT {
        let mime = if extension == "png" { "image/png" } else { "image/jpeg" };
        return Ok(Picture { bytes, mime, name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(stem) });
    }
    let image = image::load_from_memory(&bytes).context("That image couldn't be opened")?;
    let mut image = image.resize(1600, 1600, image::imageops::FilterType::Triangle).to_rgb8();
    for quality in [85u8, 75, 65, 55, 45] {
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality).encode_image(&image)?;
        if out.len() <= IMAGE_LIMIT {
            return Ok(Picture { bytes: out, mime: "image/jpeg", name: format!("{stem}.jpg") });
        }
        if quality == 55 {
            image = image::imageops::resize(&image, image.width() * 3 / 4, image.height() * 3 / 4, image::imageops::FilterType::Triangle);
        }
    }
    bail!("That image is too large to post; try a smaller one")
}
/// A multipart form body (for Discord's file upload): JSON settings plus one file.
fn multipart(boundary: &str, json: &Value, picture: &Picture) -> Vec<u8> {
    let mut body = Vec::with_capacity(picture.bytes.len() + 1024);
    let name = picture.name.replace(['"', '\r', '\n'], "_");
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"payload_json\"\r\nContent-Type: application/json\r\n\r\n{json}\r\n").as_bytes());
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"files[0]\"; filename=\"{name}\"\r\nContent-Type: {}\r\n\r\n", picture.mime).as_bytes());
    body.extend_from_slice(&picture.bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}
/// What the stream is about, for filling in the message.
#[derive(Clone, Default)]
pub struct Stream {
    pub title: String,
    pub category: String,
    /// Where to watch, as (platform, address), in the order they're listed.
    pub links: Vec<(String, String)>,
}
pub fn fill(template: &str, stream: &Stream) -> String {
    let links = stream.links.iter().map(|(_, url)| url.as_str()).collect::<Vec<_>>().join("\n");
    let mut text = template
        .replace("{title}", stream.title.trim())
        .replace("{category}", stream.category.trim())
        .replace("{links}", &links);
    for (platform, url) in &stream.links {
        text = text.replace(&format!("{{{}}}", platform.to_lowercase()), url);
    }
    text.trim().to_owned()
}
pub fn valid_webhook(url: &str) -> bool {
    reqwest::Url::parse(url.trim()).is_ok_and(|u| {
        u.scheme() == "https"
            && matches!(u.host_str(), Some("discord.com" | "discordapp.com" | "canary.discord.com" | "ptb.discord.com"))
            && u.path().starts_with("/api/webhooks/")
    })
}
fn handle(text: &str) -> String {
    text.trim().trim_start_matches('@').to_lowercase()
}
fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(20))
        .user_agent(concat!("RosorahStudio/", env!("CARGO_PKG_VERSION")))
        .build()?)
}
/// Discord's error text, or the status when it sends none.
fn failure(response: reqwest::blocking::Response) -> anyhow::Error {
    let status = response.status();
    let body: Value = response.json().unwrap_or_default();
    let message = body["message"].as_str().filter(|m| !m.is_empty()).map(str::to_owned);
    anyhow::anyhow!(message.unwrap_or_else(|| format!("HTTP {status}")))
}
fn post_discord(webhook: &str, text: &str, picture: Option<&Picture>) -> Result<()> {
    ensure!(valid_webhook(webhook), "That isn't a Discord webhook address");
    let text: String = text.chars().take(DISCORD_LIMIT).collect();
    // @everyone and role mentions work when the message includes them.
    let payload = json!({"content": text, "allowed_mentions": {"parse": ["everyone", "roles", "users"]}});
    let request = client()?.post(webhook.trim());
    let request = match picture {
        Some(picture) => {
            let boundary = format!("rosorah{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
            request.header("Content-Type", format!("multipart/form-data; boundary={boundary}")).body(multipart(&boundary, &payload, picture))
        }
        None => request.json(&payload),
    };
    let response = request.send().context("Couldn't reach Discord")?;
    if !response.status().is_success() {
        return Err(failure(response));
    }
    Ok(())
}
/// Clickable links: Bluesky needs each URL's byte range marked in the post.
fn link_facets(text: &str) -> Vec<Value> {
    let mut facets = Vec::new();
    let mut at = 0;
    while let Some(found) = text[at..].find("https://").or_else(|| text[at..].find("http://")) {
        let start = at + found;
        let end = text[start..].find(char::is_whitespace).map_or(text.len(), |n| start + n);
        let url = text[start..end].trim_end_matches(['.', ',', ')', '!', '?']);
        let end = start + url.len();
        facets.push(json!({
            "index": {"byteStart": start, "byteEnd": end},
            "features": [{"$type": "app.bsky.richtext.facet#link", "uri": url}],
        }));
        at = end.max(start + 1);
    }
    facets
}
fn post_bluesky(name: &str, password: &str, text: &str, stream: &Stream, picture: Option<&Picture>) -> Result<()> {
    let handle = handle(name);
    ensure!(!handle.is_empty(), "Enter your Bluesky handle");
    ensure!(text.chars().count() <= BLUESKY_LIMIT, "The message is longer than Bluesky's {BLUESKY_LIMIT} characters");
    let client = client()?;
    let session = client
        .post("https://bsky.social/xrpc/com.atproto.server.createSession")
        .json(&json!({"identifier": handle, "password": password.trim()}))
        .send()
        .context("Couldn't reach Bluesky")?;
    if session.status().as_u16() == 401 {
        bail!("Bluesky didn't accept the handle or app password");
    }
    if !session.status().is_success() {
        return Err(failure(session));
    }
    let session: Value = session.json()?;
    let (token, did) = (session["accessJwt"].as_str().context("Bluesky sign-in failed")?, session["did"].as_str().context("Bluesky sign-in failed")?);
    // The picture is uploaded first; the post then refers to it.
    let blob = match picture {
        Some(picture) => {
            let uploaded = client
                .post("https://bsky.social/xrpc/com.atproto.repo.uploadBlob")
                .bearer_auth(token)
                .header("Content-Type", picture.mime)
                .body(picture.bytes.clone())
                .send()
                .context("Couldn't upload the image to Bluesky")?;
            if !uploaded.status().is_success() {
                return Err(failure(uploaded));
            }
            Some(uploaded.json::<Value>()?["blob"].clone())
        }
        None => None,
    };
    let mut record = json!({
        "$type": "app.bsky.feed.post",
        "text": text,
        "createdAt": chrono_now(),
        "facets": link_facets(text),
    });
    // A link card for the first place to watch, with the picture as its thumbnail; without a
    // link the picture is attached on its own.
    if let Some((platform, url)) = stream.links.first() {
        record["embed"] = json!({
            "$type": "app.bsky.embed.external",
            "external": {
                "uri": url,
                "title": if stream.title.trim().is_empty() { format!("Live on {platform}") } else { stream.title.trim().to_owned() },
                "description": if stream.category.trim().is_empty() { format!("Live now on {platform}") } else { format!("{} · live now on {platform}", stream.category.trim()) },
            },
        });
        if let Some(blob) = &blob {
            record["embed"]["external"]["thumb"] = blob.clone();
        }
    } else if let Some(blob) = blob {
        record["embed"] = json!({"$type": "app.bsky.embed.images", "images": [{"image": blob, "alt": stream.title.trim()}]});
    }
    let response = client
        .post("https://bsky.social/xrpc/com.atproto.repo.createRecord")
        .bearer_auth(token)
        .json(&json!({"repo": did, "collection": "app.bsky.feed.post", "record": record}))
        .send()?;
    if !response.status().is_success() {
        return Err(failure(response));
    }
    Ok(())
}
/// Length as Threads counts it toward its limit: emoji and other symbols count as their UTF-8
/// bytes. Letters outside ASCII are counted the same way, which errs on the safe side.
pub fn threads_length(text: &str) -> usize {
    text.chars().map(|c| if c.is_ascii() { 1 } else { c.len_utf8() }).sum()
}
/// What Rosorah learned about the Threads account while posting.
pub struct ThreadsAccount {
    /// A renewed token and when it runs out, when renewing worked.
    pub renewed: Option<(String, u64)>,
    pub username: String,
}
/// Meta's error text; an expired or revoked token gets a plain explanation.
fn threads_failure(response: reqwest::blocking::Response) -> anyhow::Error {
    let status = response.status();
    let body: Value = response.json().unwrap_or_default();
    if body["error"]["code"].as_i64() == Some(190) {
        return anyhow::anyhow!("the Threads token has expired or was revoked; paste a new one in Settings");
    }
    let message = body["error"]["message"].as_str().filter(|m| !m.is_empty()).map(str::to_owned);
    anyhow::anyhow!(message.unwrap_or_else(|| format!("HTTP {status}")))
}
fn post_threads(token: &str, text: &str, stream: &Stream) -> Result<ThreadsAccount> {
    ensure!(threads_length(text) <= THREADS_LIMIT, "The message is longer than Threads' {THREADS_LIMIT} characters");
    let client = client()?;
    let mut token = token.trim().to_owned();
    // Renewing gives another 60 days. Meta refuses tokens under a day old, which is fine: the
    // current one is then still good.
    let renewed = client
        .get(format!("{THREADS_API}/refresh_access_token"))
        .query(&[("grant_type", "th_refresh_token"), ("access_token", token.as_str())])
        .send()
        .ok()
        .filter(|r| r.status().is_success())
        .and_then(|r| r.json::<Value>().ok())
        .and_then(|v| {
            let fresh = v["access_token"].as_str()?.to_owned();
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
            Some((fresh, now + v["expires_in"].as_u64()?))
        });
    if let Some((fresh, _)) = &renewed {
        token = fresh.clone();
    }
    let me = client.get(format!("{THREADS_API}/v1.0/me")).query(&[("fields", "username"), ("access_token", token.as_str())]).send().context("Couldn't reach Threads")?;
    if !me.status().is_success() {
        return Err(threads_failure(me));
    }
    let username = me.json::<Value>()?["username"].as_str().unwrap_or_default().to_owned();
    // A text post, published as soon as it's made, with a preview card for the first place to watch.
    let mut form = vec![("media_type", "TEXT"), ("text", text), ("auto_publish_text", "true"), ("access_token", token.as_str())];
    if let Some((_, url)) = stream.links.first() {
        form.push(("link_attachment", url.as_str()));
    }
    let response = client.post(format!("{THREADS_API}/v1.0/me/threads")).form(&form).send().context("Couldn't reach Threads")?;
    if !response.status().is_success() {
        return Err(threads_failure(response));
    }
    Ok(ThreadsAccount { renewed, username })
}
/// The current time as Bluesky expects it (RFC 3339, UTC).
fn chrono_now() -> String {
    let now = gst::glib::DateTime::now_utc().ok();
    now.and_then(|d| d.format("%Y-%m-%dT%H:%M:%S.000Z").ok()).map(|d| d.to_string()).unwrap_or_else(|| "1970-01-01T00:00:00Z".into())
}

/// Sends announcements off the UI thread; results are collected for the studio to show.
#[derive(Default)]
pub struct Announcer {
    results: Arc<Mutex<Vec<String>>>,
    threads: Arc<Mutex<Option<ThreadsAccount>>>,
}
impl Announcer {
    /// Posts to the places switched on in `which` ([Discord, Bluesky, Threads]).
    pub fn send(&self, settings: &Settings, stream: Stream, which: [bool; 3], ctx: &eframe::egui::Context) {
        let (settings, results, threads, ctx) = (settings.clone(), self.results.clone(), self.threads.clone(), ctx.clone());
        std::thread::spawn(move || {
            let text = fill(&settings.message, &stream);
            let mut out = Vec::new();
            // A missing or broken picture shouldn't stop the announcement itself.
            let picture = if settings.image.trim().is_empty() {
                None
            } else {
                match load_picture(&settings.image) {
                    Ok(p) => Some(p),
                    Err(e) => {
                        out.push(format!("The announcement image was left out: {e:#}"));
                        None
                    }
                }
            };
            if which[0] {
                out.push(match post_discord(&settings.discord_webhook, &text, picture.as_ref()) {
                    Ok(()) => "Announced on Discord".to_owned(),
                    Err(e) => format!("Discord announcement failed: {e:#}"),
                });
            }
            if which[1] {
                out.push(match post_bluesky(&settings.bluesky_handle, &settings.bluesky_password, &text, &stream, picture.as_ref()) {
                    Ok(()) => "Announced on Bluesky".to_owned(),
                    Err(e) => format!("Bluesky announcement failed: {e:#}"),
                });
            }
            if which[2] {
                out.push(match post_threads(&settings.threads_token, &text, &stream) {
                    Ok(account) => {
                        *threads.lock().unwrap() = Some(account);
                        "Announced on Threads".to_owned()
                    }
                    Err(e) => format!("Threads announcement failed: {e:#}"),
                });
            }
            results.lock().unwrap().extend(out);
            ctx.request_repaint();
        });
    }
    pub fn drain(&self) -> Vec<String> {
        std::mem::take(&mut *self.results.lock().unwrap())
    }
    /// The Threads account details from the last successful post, once.
    pub fn threads_account(&self) -> Option<ThreadsAccount> {
        self.threads.lock().unwrap().take()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fills_title_category_and_links() {
        let stream = Stream {
            title: "Ranked grind ".into(),
            category: "Valorant".into(),
            links: vec![("Twitch".into(), "https://twitch.tv/moe".into()), ("Kick".into(), "https://kick.com/moe".into())],
        };
        assert_eq!(fill(DEFAULT_MESSAGE, &stream), "🔴 I'm live! Ranked grind\nhttps://twitch.tv/moe\nhttps://kick.com/moe");
        assert_eq!(fill("{category}: {kick}", &stream), "Valorant: https://kick.com/moe");
    }
    #[test]
    fn link_ranges_are_utf8_bytes() {
        let text = "🔴 live! https://twitch.tv/moe.\nhttps://kick.com/moe";
        let facets = link_facets(text);
        assert_eq!(facets.len(), 2);
        let range = |f: &Value| (f["index"]["byteStart"].as_u64().unwrap() as usize, f["index"]["byteEnd"].as_u64().unwrap() as usize);
        let (s, e) = range(&facets[0]);
        assert_eq!(&text[s..e], "https://twitch.tv/moe");
        let (s, e) = range(&facets[1]);
        assert_eq!(&text[s..e], "https://kick.com/moe");
    }
    #[test]
    fn multipart_body_carries_json_and_file() {
        let picture = Picture { bytes: vec![1, 2, 3], mime: "image/png", name: "live.png".into() };
        let body = multipart("XYZ", &json!({"content": "hi"}), &picture);
        let text = String::from_utf8_lossy(&body);
        assert!(text.starts_with("--XYZ\r\nContent-Disposition: form-data; name=\"payload_json\""));
        assert!(text.contains("{\"content\":\"hi\"}"));
        assert!(text.contains("name=\"files[0]\"; filename=\"live.png\"\r\nContent-Type: image/png\r\n\r\n\u{1}\u{2}\u{3}\r\n--XYZ--"));
    }
    #[test]
    fn big_pictures_shrink_below_the_limit() {
        let dir = std::env::temp_dir().join(format!("rosorah-announce-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Noise doesn't compress, so this PNG is well over the limit.
        let mut seed = 12345u32;
        let noisy = image::RgbImage::from_fn(1400, 1000, |_, _| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            image::Rgb([(seed >> 24) as u8, (seed >> 16) as u8, (seed >> 8) as u8])
        });
        let path = dir.join("big.png");
        noisy.save(&path).unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() as usize > IMAGE_LIMIT);
        let picture = load_picture(path.to_str().unwrap()).unwrap();
        assert!(picture.bytes.len() <= IMAGE_LIMIT && picture.mime == "image/jpeg" && picture.name == "big.jpg");
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn threads_counts_emoji_as_bytes() {
        assert_eq!(threads_length("live now"), 8);
        assert_eq!(threads_length("🔴 live"), 4 + 5);
    }
    #[test]
    fn only_discord_webhooks_are_accepted() {
        assert!(valid_webhook("https://discord.com/api/webhooks/1/abc"));
        assert!(!valid_webhook("http://discord.com/api/webhooks/1/abc"));
        assert!(!valid_webhook("https://example.com/api/webhooks/1/abc"));
        assert!(!valid_webhook("https://discord.com/channels/1"));
    }
}
