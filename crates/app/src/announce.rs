//! "I'm live" posts to Discord (a channel webhook) and Bluesky (handle + app password),
//! sent once per stream when it starts sending.
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
fn post_discord(webhook: &str, text: &str) -> Result<()> {
    ensure!(valid_webhook(webhook), "That isn't a Discord webhook address");
    let text: String = text.chars().take(DISCORD_LIMIT).collect();
    let response = client()?
        .post(webhook.trim())
        // @everyone and role mentions work when the message includes them.
        .json(&json!({"content": text, "allowed_mentions": {"parse": ["everyone", "roles", "users"]}}))
        .send()
        .context("Couldn't reach Discord")?;
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
fn post_bluesky(name: &str, password: &str, text: &str, stream: &Stream) -> Result<()> {
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
    let mut record = json!({
        "$type": "app.bsky.feed.post",
        "text": text,
        "createdAt": chrono_now(),
        "facets": link_facets(text),
    });
    // A link card for the first place to watch.
    if let Some((platform, url)) = stream.links.first() {
        record["embed"] = json!({
            "$type": "app.bsky.embed.external",
            "external": {
                "uri": url,
                "title": if stream.title.trim().is_empty() { format!("Live on {platform}") } else { stream.title.trim().to_owned() },
                "description": if stream.category.trim().is_empty() { format!("Live now on {platform}") } else { format!("{} · live now on {platform}", stream.category.trim()) },
            },
        });
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
/// The current time as Bluesky expects it (RFC 3339, UTC).
fn chrono_now() -> String {
    let now = gst::glib::DateTime::now_utc().ok();
    now.and_then(|d| d.format("%Y-%m-%dT%H:%M:%S.000Z").ok()).map(|d| d.to_string()).unwrap_or_else(|| "1970-01-01T00:00:00Z".into())
}

/// Sends announcements off the UI thread; results are collected for the studio to show.
#[derive(Default)]
pub struct Announcer {
    results: Arc<Mutex<Vec<String>>>,
}
impl Announcer {
    /// Posts to the places switched on in `which` ([Discord, Bluesky]).
    pub fn send(&self, settings: &Settings, stream: Stream, which: [bool; 2], ctx: &eframe::egui::Context) {
        let (settings, results, ctx) = (settings.clone(), self.results.clone(), ctx.clone());
        std::thread::spawn(move || {
            let text = fill(&settings.message, &stream);
            let mut out = Vec::new();
            if which[0] {
                out.push(match post_discord(&settings.discord_webhook, &text) {
                    Ok(()) => "Announced on Discord".to_owned(),
                    Err(e) => format!("Discord announcement failed: {e:#}"),
                });
            }
            if which[1] {
                out.push(match post_bluesky(&settings.bluesky_handle, &settings.bluesky_password, &text, &stream) {
                    Ok(()) => "Announced on Bluesky".to_owned(),
                    Err(e) => format!("Bluesky announcement failed: {e:#}"),
                });
            }
            results.lock().unwrap().extend(out);
            ctx.request_repaint();
        });
    }
    pub fn drain(&self) -> Vec<String> {
        std::mem::take(&mut *self.results.lock().unwrap())
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
    fn only_discord_webhooks_are_accepted() {
        assert!(valid_webhook("https://discord.com/api/webhooks/1/abc"));
        assert!(!valid_webhook("http://discord.com/api/webhooks/1/abc"));
        assert!(!valid_webhook("https://example.com/api/webhooks/1/abc"));
        assert!(!valid_webhook("https://discord.com/channels/1"));
    }
}
