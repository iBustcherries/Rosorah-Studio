use anyhow::{Context, Result};
use auroracast_core::{ChatFeed, ChatMessage, Platform};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
#[derive(Default)]
pub struct BrowserChat {
    pub rich: crate::chat_render::RichChat,
    pub draft: String,
    #[cfg(test)]
    pub target: String,
    pub post_status: String,
    pending: Option<(String, String, Instant)>,
    next_post: u64,
    post_queue: std::collections::VecDeque<String>,
    batch_draft: String,
    batch_ok: bool,
    batch_results: Vec<String>,
    pending_platform: String,
    pub outgoing: std::collections::VecDeque<Value>,
    pub activity: Vec<crate::activity::Item>,
    pub activity_deleted: Vec<(Platform, String, String)>,
    youtube_owner: String,
    pub open_accounts: bool,
    setup_attempted: bool,
    /// A browser source needs the runtime even when no account is connected.
    pub runtime_wanted: bool,
    setup_retry: Retry,
    accounts_connected: bool,
    context: Option<eframe::egui::Context>,
    child: Option<Child>,
    tx: Option<mpsc::SyncSender<Value>>,
    rx: Option<mpsc::Receiver<Value>>,
    setup: Option<mpsc::Receiver<Result<(), String>>>,
    pub status: String,
    twitch: String,
    youtube: String,
    kick: String,
    tiktok: String,
    /// Co-streamers' Twitch channels whose chat pages are read alongside yours.
    costream: Vec<String>,
    /// Your Twitch login while invites are watched for on your Stream Manager page.
    dashboard: String,
    /// A co-stream invite showing on your Stream Manager, offered under Chat.
    pub invite: Option<Invite>,
    /// The Stream Manager page wants you signed in to Twitch in the chat browser.
    pub dashboard_signin: bool,
    /// Last page-structure report requested from the chat browser (no message text).
    pub diagnostic: Option<String>,
    page_results: Vec<Value>,
    last_sources: Option<Value>,
    retry: Retry,
    started: Option<Instant>,
    /// Emotes listed on the signed-in YouTube chat page, for the emote picker.
    pub youtube_emotes: Vec<crate::chat_render::Emote>,
    /// Picker pictures asked of the chat browser, and when.
    image_requests: std::collections::HashMap<String, Instant>,
}
/// A co-stream invite read from the Stream Manager page.
#[derive(Clone, Debug, PartialEq)]
pub struct Invite {
    /// What the invite says, as Twitch shows it ("Alice invited you to Stream Together").
    pub text: String,
    pub can_decline: bool,
    /// Twitch's pop-up only has Close, which puts the invite off rather than declining it.
    pub close_only: bool,
    /// Twitch's Stream Together page for this invite, where accepting takes you.
    pub link: String,
    /// When Twitch's pop-up went away unanswered; it can then only be answered on Twitch.
    pub gone: Option<std::time::Instant>,
}
#[derive(Default)]
struct Retry {
    failures: u32,
    at: Option<Instant>,
}
impl Retry {
    fn ready(&self, now: Instant) -> bool {
        self.at.is_none_or(|at| now >= at)
    }
    fn failed(&mut self, now: Instant) -> u64 {
        let delay = (5_u64 << self.failures.min(4)).min(60);
        self.failures = self.failures.saturating_add(1);
        self.at = Some(now + Duration::from_secs(delay));
        delay
    }
}
pub(crate) fn base() -> PathBuf {
    std::env::current_exe()
        .unwrap_or_default()
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join("browser")
}
pub(crate) fn venv() -> PathBuf {
    #[cfg(windows)]
    { auroracast_core::platform::config_root().join("rosorah-studio/browser-runtime") }
    #[cfg(not(windows))]
    { std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(||PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")).join("rosorah-studio/browser-runtime") }
}
pub(crate) fn python_relative() -> &'static str { if cfg!(windows) { "Scripts/python.exe" } else { "bin/python" } }
pub(crate) fn python() -> PathBuf { venv().join(python_relative()) }
/// Python used to create the chat runtime. On Windows the installer puts Python 3.13 in the
/// user's Programs folder; using it directly avoids the Microsoft Store "python" placeholder
/// and PATH changes that only apply after signing out.
pub(crate) fn system_python() -> PathBuf {
    #[cfg(windows)]
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        for version in ["Python313", "Python312", "Python311", "Python310"] {
            let candidate = PathBuf::from(&local).join("Programs/Python").join(version).join("python.exe");
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    PathBuf::from(if cfg!(windows) { "python" } else { "python3" })
}

impl BrowserChat {
    /// Loads emote pictures for the picker (through the chat browser, like chat's own emotes).
    pub fn want_images(&mut self, urls: Vec<String>) {
        let now = Instant::now();
        let urls: Vec<_> = urls
            .into_iter()
            .filter(|u| !self.rich.loaded(u) && self.image_requests.get(u).is_none_or(|t| now.duration_since(*t) > Duration::from_secs(60)))
            .take(64)
            .collect();
        if urls.is_empty() || self.tx.is_none() {
            return;
        }
        if self.image_requests.len() > 4000 {
            self.image_requests.clear();
        }
        for u in &urls {
            self.image_requests.insert(u.clone(), now);
        }
        self.send(json!({"type": "images", "urls": urls}));
    }
    pub fn set_context(&mut self, ctx: &eframe::egui::Context) {
        self.context = Some(ctx.clone());
    }
    pub fn posting(&self) -> bool {
        self.pending.is_some()
    }
    #[cfg(test)]
    pub fn post(&mut self) {
        let target = if self.target.is_empty() {
            "Twitch".to_owned()
        } else {
            self.target.clone()
        };
        self.post_to(&[target]);
    }
    pub fn post_to(&mut self, targets: &[String]) {
        if self.posting() || targets.is_empty() {
            return;
        }
        let limit = if targets.iter().any(|t| t == "TikTok") {
            150
        } else if targets.iter().any(|t| t == "YouTube") {
            200
        } else {
            500
        };
        if self.draft.trim().is_empty() || self.draft.chars().count() > limit {
            self.post_status =
                format!("Message must be 1–{limit} characters for the selected platforms.");
            return;
        }
        self.post_queue = targets.iter().cloned().collect();
        self.batch_draft = self.draft.clone();
        self.batch_ok = true;
        self.batch_results.clear();
        self.next_target();
    }
    fn next_target(&mut self) {
        if let Some(target) = self.post_queue.pop_front() {
            self.next_post += 1;
            let request = format!("native-{}", self.next_post);
            self.outgoing.push_back(json!({"type":"post_request","platform":target,"text":self.batch_draft,"request":request}));
            self.pending = Some((request, self.batch_draft.clone(), Instant::now()));
            self.pending_platform = target;
            self.post_status = format!("Sending to {}…", self.pending_platform);
        } else {
            if self.batch_ok && self.draft == self.batch_draft {
                self.draft.clear();
            }
            self.post_status = self.batch_results.join(" · ");
            if !self.batch_ok {
                self.post_status
                    .push_str(". Draft kept; retry only destinations that failed.");
            }
        }
    }
    pub fn finish_post(&mut self, v: Value) {
        if self
            .pending
            .as_ref()
            .is_some_and(|(id, _, _)| v["request"].as_str() == Some(id))
        {
            self.pending = None;
            let ok = v["ok"] == true;
            self.batch_ok &= ok;
            let text = v["text"]
                .as_str()
                .unwrap_or(if ok { "Sent" } else { "Failed" });
            self.batch_results.push(format!(
                "{}: {}",
                self.pending_platform,
                text.chars().take(500).collect::<String>()
            ));
            self.next_target();
        }
        self.send(v);
    }
    pub fn running(&self) -> bool {
        self.child.is_some()
    }
    pub fn installing(&self) -> bool {
        self.setup.is_some()
    }
    pub fn setup(&mut self) {
        if self.setup.is_some() {
            return;
        }
        let (tx, rx) = mpsc::sync_channel(1);
        self.setup = Some(rx);
        self.setup_attempted = true;
        self.status = "Preparing chat for the first time…".into();
        std::thread::spawn(move || {
            let result = (|| -> Result<()> {
                let p = venv();
                std::fs::create_dir_all(p.parent().unwrap())?;
                let ready = p.join("rosorah-runtime-6.10.2.ready");
                let valid = || {
                    Command::new(p.join(python_relative())).args(["-c", "import PySide6; from PySide6.QtWebEngineWidgets import QWebEngineView; assert PySide6.__version__ == '6.10.2'"]).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
                };
                if valid() {
                    std::fs::write(ready, b"6.10.2\n")?;
                    return Ok(());
                }
                anyhow::ensure!(
                    Command::new(system_python())
                        .args(["-m", "venv"])
                        .arg(&p)
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status()?
                        .success(),
                    if cfg!(windows) {
                        "Cannot create Python environment. Run the Rosorah installer again to install Python."
                    } else {
                        "Cannot create Python environment. Install python3 and python3-pip."
                    }
                );
                anyhow::ensure!(
                    Command::new(p.join(python_relative()))
                        .args([
                            "-m",
                            "pip",
                            "install",
                            "--disable-pip-version-check",
                            "PySide6==6.10.2"
                        ])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status()?
                        .success(),
                    "Browser runtime download failed. Check internet access and retry setup."
                );
                anyhow::ensure!(
                    valid(),
                    "Chat runtime cannot load. Check its desktop libraries in Accounts > Troubleshooting."
                );
                std::fs::write(ready, b"6.10.2\n")?;
                Ok(())
            })();
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });
    }
    pub fn sync_accounts(
        &mut self,
        twitch: &crate::accounts::Account,
        youtube: &crate::accounts::Account,
        kick: &crate::accounts::Account,
    ) {
        self.accounts_connected = twitch.connected || youtube.connected || kick.connected;
        self.kick = if kick.connected {
            kick.name.clone()
        } else {
            String::new()
        };
        self.youtube_owner = if youtube.connected {
            youtube.user_id.clone()
        } else {
            String::new()
        };
        self.twitch = if twitch.connected {
            twitch.name.clone()
        } else {
            String::new()
        };
        self.youtube = if youtube.connected {
            youtube.broadcast.clone()
        } else {
            String::new()
        };
        self.sources();
    }
    /// Co-streaming on Twitch: the channels typed in Settings, except those already in Twitch's
    /// shared chat with you (your own chat page shows theirs then), and whether invites are watched for.
    pub fn set_costream(&mut self, typed: &str, shared: &[(String, String)], invites: bool) {
        let mut channels: Vec<String> = Vec::new();
        for name in typed.split(|c: char| c == ',' || c.is_whitespace()) {
            let name = name.trim().trim_start_matches('@').to_lowercase();
            if valid_login(&name) && name != self.twitch.to_lowercase() && !shared.iter().any(|(login, _)| *login == name) && !channels.contains(&name) {
                channels.push(name);
            }
        }
        channels.truncate(6);
        let dashboard = if invites { self.twitch.clone() } else { String::new() };
        if channels != self.costream || dashboard != self.dashboard {
            if dashboard.is_empty() {
                self.invite = None;
                self.dashboard_signin = false;
            }
            self.costream = channels;
            self.dashboard = dashboard;
            self.sources();
        }
    }
    /// Presses Accept or Decline on the invite, on the Stream Manager page.
    pub fn answer_invite(&mut self, accept: bool) {
        self.send(json!({"type":"invite_action","action":if accept {"accept"} else {"decline"}}));
    }
    /// Takes the card away without answering, once Twitch's pop-up has gone.
    pub fn forget_invite(&mut self) {
        self.invite = None;
        self.send(json!({"type":"invite_action","action":"forget"}));
    }
    /// TikTok has no sign-in here: chat is read from this username's LIVE page.
    pub fn set_tiktok(&mut self, username: &str) {
        let name = username.trim().trim_start_matches('@');
        if self.tiktok != name {
            self.tiktok = name.to_owned();
            self.sources();
        }
    }
    pub fn start(&mut self) -> Result<()> {
        self.launch(true)
    }
    fn launch(&mut self, visible: bool) -> Result<()> {
        self.launch_paths(
            visible,
            &python(),
            &base().join("chat_browser.py"),
        )
    }
    fn launch_paths(
        &mut self,
        visible: bool,
        python: &std::path::Path,
        script: &std::path::Path,
    ) -> Result<()> {
        if self.running() {
            if visible {
                self.send(json!({"type":"show"}));
            }
            return Ok(());
        }
        anyhow::ensure!(
            python.is_file(),
            "Chat is preparing automatically. Check Settings > Accounts > Chat troubleshooting if it cannot start."
        );
        let mut command = Command::new(python);
        command.arg(script);
        if !visible {
            command.arg("--background");
        }
        let mut child = command
            .env_remove("LD_LIBRARY_PATH")
            .env_remove("QT_PLUGIN_PATH")
            .env_remove("QT_QPA_PLATFORM_PLUGIN_PATH")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("Start browser chat")?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, commands) = mpsc::sync_channel::<Value>(256);
        let (events, rx) = mpsc::sync_channel(512);
        std::thread::spawn(move || {
            let mut input = stdin;
            for c in commands {
                if writeln!(input, "{c}").and_then(|_| input.flush()).is_err() {
                    break;
                }
            }
        });
        let repaint = self.context.clone();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    _ => (),
                };
                if line.len() > 131072 {
                    break;
                }
                if let Ok(v) = serde_json::from_str(&line) {
                    if events.send(v).is_err() {
                        break;
                    }
                    if let Some(ctx) = &repaint {
                        ctx.request_repaint();
                    }
                }
            }
        });
        self.child = Some(child);
        self.started = Some(Instant::now());
        self.tx = Some(tx);
        self.rx = Some(rx);
        self.last_sources = None;
        self.status = "Starting chat in the background…".into();
        self.sources();
        Ok(())
    }
    pub fn send(&self, v: Value) {
        if let Some(tx) = &self.tx {
            let _ = tx.try_send(v);
        }
    }
    fn sources(&mut self) {
        let mut next = json!({"type":"sources","twitch":self.twitch.trim(),"youtube":self.youtube.trim(),"kick":self.kick.trim(),"tiktok":self.tiktok.trim()});
        // Only sent when used, so the message stays the same for everyone else.
        if !self.costream.is_empty() {
            next["costream"] = json!(self.costream);
        }
        if !self.dashboard.is_empty() {
            next["dashboard"] = json!(self.dashboard);
        }
        if self.last_sources.as_ref() != Some(&next)
            && let Some(tx) = &self.tx
            && tx.try_send(next.clone()).is_ok()
        {
            self.last_sources = Some(next);
        }
    }
    pub fn relay(&self, m: &ChatMessage) {
        if m.id.starts_with("event-") {
            self.send(json!({"type":"event","event":{"type":"message","platform":m.platform.name(),"id":m.id,"author":m.author,"text":m.text,"kind":"activity"}}));
        }
    }
    pub fn tick(&mut self, feed: &mut ChatFeed) {
        // Results of messages typed into the TikTok page, received last frame.
        for v in std::mem::take(&mut self.page_results) {
            self.finish_post(v);
        }
        if let Some(rx) = &self.setup
            && let Ok(result) = rx.try_recv()
        {
            self.status = match result {
                Ok(()) => {
                    self.retry = Retry::default();
                    "Chat runtime ready. Chat starts automatically for connected accounts.".into()
                }
                Err(e) => {
                    self.setup_attempted = false;
                    let delay = self.setup_retry.failed(Instant::now());
                    format!("{e} Retrying chat setup automatically in {delay}s.")
                }
            };
            self.setup = None;
        }
        if (self.accounts_connected || self.runtime_wanted)
            && (!python().is_file()
                || !venv().join("rosorah-runtime-6.10.2.ready").is_file())
            && !self.setup_attempted
            && self.setup_retry.ready(Instant::now())
        {
            self.setup();
        }
        self.supervise(&python(), &base().join("chat_browser.py"));
        self.receive(feed);
    }
    fn supervise(&mut self, python: &std::path::Path, script: &std::path::Path) {
        if self
            .child
            .as_mut()
            .is_some_and(|c| c.try_wait().ok().flatten().is_some())
        {
            self.child = None;
            self.tx = None;
            self.rx = None;
            self.started = None;
            let delay = self.retry.failed(Instant::now());
            self.status =
                format!("Chat service stopped. Restarting automatically in {delay} seconds…");
        }
        if self.running()
            && self
                .started
                .is_some_and(|at| at.elapsed() >= Duration::from_secs(60))
        {
            self.retry = Retry::default();
        }
        if (!self.twitch.is_empty() || !self.youtube.is_empty() || !self.kick.is_empty())
            && !self.running()
            && !self.installing()
            && self.retry.ready(Instant::now())
        {
            if python.is_file() {
                if let Err(error) = self.launch_paths(false, python, script) {
                    let delay = self.retry.failed(Instant::now());
                    self.status = format!("{error}. Retrying automatically in {delay} seconds…");
                }
            } else {
                self.status =
                    "Chat setup needs attention. Open Settings > Accounts > Chat troubleshooting."
                        .into();
                self.retry.at = Some(Instant::now() + Duration::from_secs(5));
            }
        }
    }
    fn receive(&mut self, feed: &mut ChatFeed) {
        if self
            .pending
            .as_ref()
            .is_some_and(|(_, _, time)| time.elapsed() > Duration::from_secs(30))
        {
            self.pending = None;
            self.batch_ok = false;
            self.batch_results.push(format!(
                "{}: no response; check chat before retrying",
                self.pending_platform
            ));
            self.next_target();
        }
        if let Some(rx) = &self.rx {
            for v in rx.try_iter() {
                match v["type"].as_str() {
                    Some("post_result") if v["platform"] == "TikTok" => self.page_results.push(v),
                    Some("diagnostic") => {
                        self.diagnostic = Some(v["text"].as_str().unwrap_or("").chars().take(60000).collect());
                    }
                    Some("emote") => {
                        if let Some(ctx) = &self.context {
                            self.rich.image(ctx, &v);
                        }
                    }
                    Some("post_request") => {
                        if self.outgoing.len() < 16
                            && matches!(v["platform"].as_str(), Some("Twitch" | "YouTube" | "Kick" | "TikTok"))
                            && v["request"].as_str().is_some_and(|s| s.len() < 100)
                            && v["text"].as_str().is_some_and(|s| s.chars().count() <= 500)
                        {
                            self.outgoing.push_back(v);
                        }
                    }
                    Some("open_accounts") => self.open_accounts = true,
                    Some("emote_list") if v["platform"] == "YouTube" => {
                        self.youtube_emotes = v["items"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .take(300)
                            .filter_map(|e| {
                                let name = e["name"].as_str()?.chars().take(100).collect::<String>();
                                let src = e["src"].as_str()?.to_string();
                                (!name.is_empty() && crate::chat_render::safe_image(&src)).then(|| crate::chat_render::Emote {
                                    group: "YouTube".into(),
                                    insert: name.clone(),
                                    name,
                                    src,
                                })
                            })
                            .collect();
                    }
                    Some("invite") => {
                        self.dashboard_signin = v["signin"].as_bool().unwrap_or(false);
                        let text: String = v["text"].as_str().unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" ").chars().take(300).collect();
                        let gone = v["gone"].as_bool().unwrap_or(false);
                        let was = self.invite.take().filter(|old| old.text == text);
                        self.invite = (!text.is_empty() && !self.dashboard.is_empty()).then(|| Invite {
                            // Gone keeps the buttons it had, and the time it went.
                            can_decline: if gone { was.as_ref().is_some_and(|o| o.can_decline) } else { v["can_decline"].as_bool().unwrap_or(false) },
                            close_only: if gone { was.as_ref().is_some_and(|o| o.close_only) } else { v["close_only"].as_bool().unwrap_or(false) },
                            link: v["link"].as_str().filter(|l| l.starts_with("https://www.twitch.tv/popout/")).unwrap_or("").to_owned(),
                            gone: gone.then(|| was.and_then(|o| o.gone).unwrap_or_else(std::time::Instant::now)),
                            text,
                        });
                    }
                    Some("status") => {
                        self.status = v["text"]
                            .as_str()
                            .unwrap_or("")
                            .chars()
                            .take(1000)
                            .collect()
                    }
                    Some("message" | "delete") => {
                        let platform = match v["platform"].as_str() {
                            Some("Twitch") => Platform::Twitch,
                            Some("YouTube") => Platform::YouTube,
                            Some("Kick") => Platform::Kick,
                            Some("TikTok") => Platform::TikTok,
                            _ => continue,
                        };
                        let id = v["id"].as_str().unwrap_or("");
                        if id.is_empty() {
                            continue;
                        }
                        let id = format!("web-{}", id.chars().take(200).collect::<String>());
                        if v["type"] == "delete" {
                            feed.remove(platform, &id);
                            if platform == Platform::YouTube
                                && v["source"].as_str() == Some(self.youtube.as_str())
                            {
                                self.activity_deleted.push((
                                    platform,
                                    self.youtube_owner.clone(),
                                    id.clone(),
                                ));
                            }
                        } else {
                            self.rich
                                .message(format!("{}:{id}", platform.name()), &v["parts"]);
                            let author: String = v["author"]
                                .as_str()
                                .unwrap_or("Viewer")
                                .chars()
                                .take(100)
                                .collect();
                            crate::avatars::note(platform.name(), &author, v["author_photo"].as_str().unwrap_or(""));
                            let mut text = v["text"]
                                .as_str()
                                .unwrap_or("")
                                .chars()
                                .take(5000)
                                .collect::<String>();
                            if v["kind"] == "activity" {
                                text = format!("★ {text}");
                            }
                            let received_ms = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis() as u64;
                            if platform == Platform::YouTube
                                && !self.youtube_owner.is_empty()
                                && v["source"].as_str() == Some(self.youtube.as_str())
                                && let Some(kind) = crate::activity::youtube_kind(
                                    v["activity_kind"].as_str().unwrap_or(""),
                                )
                            {
                                self.activity.push(crate::activity::Item {
                                    facts: Default::default(),
                                    reward_id: String::new(),
                                    goal_count: 0,
                                    platform,
                                    owner: self.youtube_owner.clone(),
                                    id: id.clone(),
                                    kind,
                                    author: author.clone(),
                                    text: text.trim_start_matches("★ ").into(),
                                    at_ms: received_ms,
                                    historical: v["historical"].as_bool().unwrap_or(false),
                                });
                            }
                            if platform == Platform::TikTok
                                && !self.tiktok.is_empty()
                                && v["source"].as_str() == Some(self.tiktok.as_str())
                                && let Some(kind) = crate::activity::tiktok_kind(v["activity_kind"].as_str().unwrap_or(""))
                            {
                                self.activity.push(crate::activity::Item {
                                    facts: Default::default(),
                                    reward_id: String::new(),
                                    goal_count: 0,
                                    platform,
                                    owner: self.tiktok.clone(),
                                    id: id.clone(),
                                    kind,
                                    author: author.clone(),
                                    text: text.trim_start_matches("★ ").into(),
                                    at_ms: received_ms,
                                    historical: v["historical"].as_bool().unwrap_or(false),
                                });
                            }
                            feed.push(ChatMessage {
                                metadata: auroracast_core::alert_filters::ChatMetadata {
                                    moderator: false,
                                    subscriber: false,
                                    user_id: v["author_id"].as_str().unwrap_or("").into(),
                                    first_ever: false,
                                    historical: v["historical"].as_bool().unwrap_or(false)
                                        || v["kind"] == "activity",
                                    color: v["color"].as_str().filter(|c| valid_color(c)).unwrap_or("").into(),
                                    badges: v["badges"]
                                        .as_array()
                                        .into_iter()
                                        .flatten()
                                        .filter_map(|b| b.as_str())
                                        .filter(|b| valid_badge(b))
                                        .take(6)
                                        .map(Into::into)
                                        .collect(),
                                    highlight: v["highlight"]
                                        .as_str()
                                        .filter(|h| matches!(*h, "paid" | "first" | "highlight"))
                                        .unwrap_or("")
                                        .into(),
                                    channel: v["channel"]
                                        .as_str()
                                        .map(|c| c.trim().chars().filter(|c| !c.is_control()).take(40).collect::<String>())
                                        .filter(|c| platform == Platform::Twitch && !c.is_empty() && !c.eq_ignore_ascii_case(&self.twitch))
                                        .unwrap_or_default(),
                                },
                                platform,
                                id,
                                author,
                                text,
                                received_ms,
                                demo: false,
                            });
                        }
                    }
                    _ => (),
                }
            }
        }
    }
}
/// A Twitch login: 1-25 letters, digits or underscores.
fn valid_login(name: &str) -> bool {
    (1..=25).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}
/// A name colour from a chat page: "#rrggbb".
fn valid_color(c: &str) -> bool {
    c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|c| c.is_ascii_hexdigit())
}
/// A badge from a chat page: a picture from a known host, or a role name.
fn valid_badge(b: &str) -> bool {
    crate::avatars::safe(b) && b.len() <= 512
        || matches!(b, "broadcaster" | "moderator" | "vip" | "subscriber" | "member" | "verified" | "founder" | "og" | "staff")
}
impl Drop for BrowserChat {
    fn drop(&mut self) {
        self.send(json!({"type":"quit"}));
        self.tx = None;
        if let Some(mut child) = self.child.take() {
            let deadline = Instant::now() + Duration::from_millis(500);
            while Instant::now() < deadline {
                if child.try_wait().ok().flatten().is_some() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::Account;

    #[test]
    fn composer_routes_once_preserves_failures_and_matches_acknowledgements() {
        let mut chat = BrowserChat::default();
        let (tx, commands) = mpsc::sync_channel(8);
        chat.tx = Some(tx);
        chat.target = "YouTube".into();
        chat.draft = "Hello 👋".into();
        chat.post();
        chat.post();
        let command = chat.outgoing.pop_front().unwrap();
        assert_eq!(command["platform"], "YouTube");
        assert!(chat.outgoing.is_empty());
        assert!(
            commands.try_recv().is_err(),
            "Tokens and sending belong to the account worker, not the browser"
        );
        chat.finish_post(json!({"type":"post_result","request":"stale","ok":true}));
        assert_eq!(chat.draft, "Hello 👋");
        assert!(chat.posting());
        chat.finish_post(json!({"type":"post_result","request":command["request"],"ok":false,"text":"Unavailable"}));
        assert_eq!(chat.draft, "Hello 👋");
        assert!(!chat.posting());
        assert!(chat.outgoing.is_empty());
        chat.post();
        let command = chat.outgoing.pop_front().unwrap();
        chat.finish_post(
            json!({"type":"post_result","request":command["request"],"ok":true,"text":"Sent"}),
        );
        assert!(chat.draft.is_empty());
        chat.draft = "x".repeat(201);
        chat.post();
        assert!(chat.outgoing.is_empty());
        assert!(!chat.posting());
    }

    #[test]
    fn automatically_launches_hidden_collects_and_recovers_without_open_button() {
        let folder = std::env::temp_dir().join(format!(
            "rosorah-chat-auto-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&folder).unwrap();
        let script = folder.join("worker.py");
        std::fs::write(&script, r#"import sys,json
assert '--background' in sys.argv
for line in sys.stdin:
    command=json.loads(line)
    if command['type']=='quit':break
    if command['type']=='sources' and command['twitch']:
        print(json.dumps({'type':'message','platform':'Twitch','id':'background-message','author':'Viewer','text':'Received without opening a window'}),flush=True)
"#).unwrap();
        let python = std::path::Path::new("/usr/bin/python3");
        let mut browser = BrowserChat::default();
        let mut twitch = Account::new(Platform::Twitch);
        let youtube = Account::new(Platform::YouTube);
        let kick = Account::new(Platform::Kick);
        browser.supervise(python, &script);
        assert!(!browser.running(), "Must wait for a connected chat source");
        twitch.connected = true;
        twitch.name = "moe".into();
        browser.sync_accounts(&twitch, &youtube, &kick);
        browser.supervise(python, &script);
        let pid = browser.child.as_ref().unwrap().id();
        browser.supervise(python, &script);
        assert_eq!(
            browser.child.as_ref().unwrap().id(),
            pid,
            "Must reuse the running service"
        );
        let mut feed = ChatFeed::new(10);
        let deadline = Instant::now() + Duration::from_secs(3);
        while feed.messages().next().is_none() {
            browser.receive(&mut feed);
            assert!(
                Instant::now() < deadline,
                "Background message never arrived"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            feed.messages().next().unwrap().text,
            "Received without opening a window"
        );
        browser.child.as_mut().unwrap().kill().unwrap();
        browser.child.as_mut().unwrap().wait().unwrap();
        browser.supervise(python, &script);
        assert!(!browser.running());
        assert!(
            !browser.retry.ready(Instant::now()),
            "Crash restart must back off"
        );
        browser.retry.at = Some(Instant::now());
        browser.supervise(python, &script);
        assert!(browser.running());
        assert_ne!(browser.child.as_ref().unwrap().id(), pid);
        drop(browser);
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn repeated_crashes_back_off_and_missing_runtime_does_not_install_itself() {
        let now = Instant::now();
        let mut retry = Retry::default();
        assert!(retry.ready(now));
        for delay in [5, 10, 20, 40, 60, 60] {
            assert_eq!(retry.failed(now), delay);
            assert!(!retry.ready(now + Duration::from_secs(delay - 1)));
            assert!(retry.ready(now + Duration::from_secs(delay)));
        }
        let mut browser = BrowserChat::default();
        browser.twitch = "moe".into();
        browser.supervise(
            std::path::Path::new("/no-such-runtime/bin/python"),
            std::path::Path::new("/no-such-script"),
        );
        assert!(!browser.running());
        assert!(!browser.installing());
        assert!(browser.status.contains("Chat troubleshooting"));
    }

    #[test]
    fn follows_connections_switches_broadcasts_and_clears_on_sign_out() {
        let (tx, rx) = mpsc::sync_channel(8);
        let mut browser = BrowserChat::default();
        browser.tx = Some(tx);
        let mut twitch = Account::new(Platform::Twitch);
        let mut youtube = Account::new(Platform::YouTube);
        let mut kick = Account::new(Platform::Kick);
        kick.connected = true;
        kick.name = "moe-kick".into();
        twitch.connected = true;
        twitch.name = "moe".into();
        youtube.connected = true;
        youtube.broadcast = "first-video".into();
        browser.sync_accounts(&twitch, &youtube, &kick);
        assert_eq!(
            rx.try_recv().unwrap(),
            json!({"type":"sources","twitch":"moe","youtube":"first-video","kick":"moe-kick","tiktok":""})
        );
        browser.sync_accounts(&twitch, &youtube, &kick);
        assert!(
            rx.try_recv().is_err(),
            "unchanged accounts must not reload pages"
        );
        twitch.name = "other_channel".into();
        youtube.broadcast = "next--video".into();
        browser.sync_accounts(&twitch, &youtube, &kick);
        assert_eq!(rx.try_recv().unwrap()["youtube"], "next--video");
        twitch.disconnect();
        youtube.disconnect();
        kick.disconnect();
        browser.sync_accounts(&twitch, &youtube, &kick);
        assert_eq!(
            rx.try_recv().unwrap(),
            json!({"type":"sources","twitch":"","youtube":"","kick":"","tiktok":""})
        );
    }

    #[test]
    fn costreamers_get_their_own_pages_unless_twitch_already_shares_their_chat() {
        let (tx, rx) = mpsc::sync_channel(8);
        let mut browser = BrowserChat::default();
        browser.tx = Some(tx);
        browser.twitch = "moe".into();
        browser.set_costream("@Alice, bob  moe bad-name!", &[], false);
        assert_eq!(rx.try_recv().unwrap()["costream"], json!(["alice", "bob"]));
        browser.set_costream("alice bob", &[("alice".into(), "Alice".into())], true);
        let next = rx.try_recv().unwrap();
        assert_eq!((next["costream"].clone(), next["dashboard"].clone()), (json!(["bob"]), json!("moe")));
        browser.set_costream("alice bob", &[("alice".into(), "Alice".into())], true);
        assert!(rx.try_recv().is_err(), "unchanged co-streams must not reload pages");
        let mut feed = ChatFeed::new(10);
        let (events, incoming) = mpsc::sync_channel(8);
        browser.rx = Some(incoming);
        events.send(json!({"type":"invite","signin":false,"text":"Alice  invited you\nto Stream Together","can_decline":true})).unwrap();
        events.send(json!({"type":"message","platform":"Twitch","id":"1","author":"Ann","text":"hi","channel":"bob"})).unwrap();
        browser.receive(&mut feed);
        assert_eq!(browser.invite.as_ref().unwrap().text, "Alice invited you to Stream Together");
        assert_eq!(feed.messages().next().unwrap().metadata.channel, "bob");
        events.send(json!({"type":"invite","signin":false,"text":""})).unwrap();
        browser.receive(&mut feed);
        assert!(browser.invite.is_none());
    }

    #[test]
    fn sources_retry_after_full_queue_and_on_browser_restart() {
        let (tx, rx) = mpsc::sync_channel(1);
        tx.try_send(json!({"type":"show"})).unwrap();
        let mut browser = BrowserChat::default();
        browser.tx = Some(tx);
        browser.sources();
        assert!(browser.last_sources.is_none());
        rx.try_recv().unwrap();
        browser.sources();
        assert_eq!(rx.try_recv().unwrap()["type"], "sources");
        browser.last_sources = None;
        browser.sources();
        assert_eq!(rx.try_recv().unwrap()["type"], "sources");
    }
}

#[cfg(test)]
mod activity_tests {
    use super::*;
    #[test]
    fn activity_requires_typed_event_and_current_broadcast() {
        let mut b = BrowserChat::default();
        b.youtube = "abcdefghijk".into();
        b.youtube_owner = "owner".into();
        let (tx, rx) = mpsc::channel();
        b.rx = Some(rx);
        let mut feed = ChatFeed::new(10);
        for (id, source, kind) in [
            ("1", "oldvideo123", "super_chat"),
            ("2", "abcdefghijk", ""),
            ("3", "abcdefghijk", "super_chat"),
        ] {
            tx.send(json!({"type":"message","platform":"YouTube","source":source,"id":id,"author":"Moe","text":"★ $5","kind":"activity","activity_kind":kind})).unwrap();
        }
        b.receive(&mut feed);
        assert_eq!(b.activity.len(), 1);
        assert_eq!(b.activity[0].id, "web-3");
        assert_eq!(b.activity[0].owner, "owner");
        tx.send(json!({"type":"delete","platform":"YouTube","source":"abcdefghijk","id":"3"}))
            .unwrap();
        b.receive(&mut feed);
        assert_eq!(b.activity_deleted.len(), 1);
    }
    #[test]
    fn tiktok_page_events_become_alert_kinds_for_the_set_username() {
        let mut b = BrowserChat::default();
        b.set_tiktok("@moe.live");
        let (tx, rx) = mpsc::channel();
        b.rx = Some(rx);
        let mut feed = ChatFeed::new(10);
        for (id, source, kind) in [
            ("tt-1", "moe.live", "gift"),
            ("tt-2", "moe.live", "follow"),
            ("tt-3", "moe.live", "share"),
            ("tt-4", "someone.else", "gift"),
        ] {
            tx.send(json!({"type":"message","platform":"TikTok","source":source,"id":id,"author":"Rose","text":"sent Rose","kind":"activity","activity_kind":kind})).unwrap();
        }
        tx.send(json!({"type":"message","platform":"TikTok","source":"moe.live","id":"tt-5","author":"Ann","text":"hi","kind":"chat"})).unwrap();
        b.receive(&mut feed);
        let kinds: Vec<_> = b.activity.iter().map(|a| (a.kind, a.owner.as_str())).collect();
        assert_eq!(kinds, [(auroracast_core::EventKind::Gift, "moe.live"), (auroracast_core::EventKind::Follow, "moe.live")]);
        assert_eq!(feed.messages().filter(|m| m.platform == Platform::TikTok).count(), 5);
    }
}

#[cfg(test)]
mod multi_destination_tests {
    use super::*;
    #[test]
    fn sends_each_platform_once_and_keeps_draft_on_partial_failure() {
        let mut c = BrowserChat::default();
        c.draft = "Hello 👋".into();
        c.post_to(&["Twitch".into(), "YouTube".into()]);
        let a = c.outgoing.pop_front().unwrap();
        assert_eq!(a["platform"], "Twitch");
        c.finish_post(json!({"request":a["request"],"ok":true,"text":"Sent"}));
        assert!(!c.draft.is_empty());
        let b = c.outgoing.pop_front().unwrap();
        assert_eq!(b["platform"], "YouTube");
        c.finish_post(json!({"request":a["request"],"ok":true}));
        assert!(c.posting());
        c.finish_post(json!({"request":b["request"],"ok":false,"text":"Unavailable"}));
        assert!(!c.posting());
        assert_eq!(c.draft, "Hello 👋");
        assert!(c.post_status.contains("Twitch: Sent"));
        assert!(c.post_status.contains("YouTube: Unavailable"));
        assert!(c.outgoing.is_empty());
    }
    #[test]
    fn validates_both_before_sending_and_preserves_newer_edits() {
        let mut c = BrowserChat::default();
        c.draft = "x".repeat(201);
        c.post_to(&["Twitch".into(), "YouTube".into()]);
        assert!(c.outgoing.is_empty());
        c.draft = "Original".into();
        c.post_to(&["Twitch".into(), "YouTube".into()]);
        let a = c.outgoing.pop_front().unwrap();
        c.draft = "Next message".into();
        c.finish_post(json!({"request":a["request"],"ok":true}));
        let b = c.outgoing.pop_front().unwrap();
        assert_eq!(b["text"], "Original");
        c.finish_post(json!({"request":b["request"],"ok":true}));
        assert_eq!(c.draft, "Next message");
    }
}
