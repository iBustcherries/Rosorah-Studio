//! One renewable account session powers broadcast controls, activity and outgoing chat.
mod events;
pub mod rewards;
mod oauth;
mod kick;
mod kick_events;
mod services;
#[cfg(not(windows))]
mod vault;
#[cfg(windows)]
#[path = "accounts/vault_windows.rs"]
mod vault;
use anyhow::{Context, Result, ensure};
use auroracast_core::{ChatMessage, Platform};
use reqwest::blocking::{Client, RequestBuilder};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json as j};
pub use services::Draft;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ClientConfig {
    pub id: String,
    pub secret: String,
    pub broker_url: String,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub twitch: ClientConfig,
    pub youtube: ClientConfig,
    pub kick: ClientConfig,
}
impl Config {
    fn path() -> std::path::PathBuf {
        crate::streaming::Config::path().with_file_name("accounts.json")
    }
    pub fn load() -> Self {
        let read = |path| {
            std::fs::read(path)
                .ok()
                .and_then(|b| serde_json::from_slice::<Self>(&b).ok())
                .unwrap_or_default()
        };
        let mut config = read(Self::path());
        let defaults = read(
            std::env::current_exe()
                .unwrap_or_default()
                .with_file_name("accounts-defaults.json"),
        );
        if config.twitch.id.is_empty() {
            config.twitch = defaults.twitch;
        }
        if config.youtube.id.is_empty() {
            config.youtube = defaults.youtube;
        }
        if config.kick.id.is_empty() {
            config.kick = defaults.kick;
        }
        if config.kick.id.is_empty() {
            config.kick = ClientConfig {
                id: "01M4YA7GYFWH7BWR999CPG5YH4".into(),
                broker_url: "https://rosorah-kick-auth.justmoeislive.workers.dev".into(),
                secret: String::new(),
            };
        }
        config
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Token {
    access: String,
    refresh: String,
    expiry: u64,
}
#[derive(Clone, Default)]
pub struct Choice {
    pub id: String,
    pub name: String,
}
/// A moderation action from the chat panel. `user` is the platform login/username the
/// chat page exposed; `message` is the platform message id where one is known (YouTube).
#[derive(Clone, Debug, PartialEq)]
pub enum ModAction {
    Delete,
    Timeout(u32),
    Ban,
    Unban,
    ClearChat,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Moderation {
    pub action: ModAction,
    pub user: String,
    pub author: String,
    pub message: String,
    /// The message's text, used to find its Kick message id.
    pub text: String,
}
impl Moderation {
    /// Past-tense summary used for the result line, e.g. "Timed out Rose for 10 minutes".
    pub fn done(&self) -> String {
        match &self.action {
            ModAction::Delete => format!("Deleted {}'s message", self.author),
            ModAction::Timeout(s) => format!("Timed out {} for {}", self.author, duration_label(*s)),
            ModAction::Ban => format!("Banned {}", self.author),
            ModAction::Unban => format!("Unbanned {}", self.author),
            ModAction::ClearChat => "Cleared chat for everyone".into(),
        }
    }
}
pub fn duration_label(seconds: u32) -> String {
    match seconds {
        s if s % 86400 == 0 => format!("{} day{}", s / 86400, if s == 86400 { "" } else { "s" }),
        s if s % 3600 == 0 => format!("{} hour{}", s / 3600, if s == 3600 { "" } else { "s" }),
        s if s % 60 == 0 => format!("{} minute{}", s / 60, if s == 60 { "" } else { "s" }),
        s => format!("{s} seconds"),
    }
}
pub enum Command {
    Moderate(Moderation),
    Rewards(rewards::Request),
    Refresh,
    Discover,
    Post {
        request: String,
        text: String,
        broadcast: String,
    },
    Reconnect,
    Create(Draft),
    Thumbnail(String),
    Bind(String),
    Search(String),
    Select(String),
    Update {
        title: String,
        category: String,
    },
    /// Rosorah started (true) or stopped (false) streaming: viewers are only checked while live.
    Live(bool),
}
pub enum Event {
    Moderated(String),
    Rewards(std::result::Result<Value,String>),
    Status(String),
    Remembered(String),
    PostResult(Value),
    Ingest(String, String),
    OperationDone,
    Selected(String),
    Streams(Vec<Choice>),
    Connected(String, String),
    Activity(crate::activity::Item),
    Authorize(String, String),
    Metadata(String, String),
    Categories(Vec<Choice>),
    Broadcasts(Vec<Choice>),
    Chat(ChatMessage),
    /// People watching now; None while the channel isn't live.
    Viewers(Option<u64>),
    #[cfg(test)]
    Delete(String),
    #[cfg(test)]
    Clear(String),
    Failed(String),
}
pub struct Account {
    pub reward_result: Option<std::result::Result<Value,String>>,
    pub reward_busy: bool,
    pub platform: Platform,
    pub status: String,
    pub connected: bool,
    pub busy: bool,
    pub auth_url: String,
    pub title: String,
    pub category: String,
    pub category_query: String,
    pub categories: Vec<Choice>,
    pub broadcasts: Vec<Choice>,
    pub broadcast: String,
    pub name: String,
    pub user_id: String,
    pub activity: Vec<crate::activity::Item>,
    pub draft: Draft,
    pub streams: Vec<Choice>,
    pub operation_busy: bool,
    pub remembered_status: String,
    pub ingest: Option<(String, String)>,
    pub forgotten: bool,
    pub post_results: Vec<Value>,
    pub moderation_results: Vec<String>,
    /// People watching now (None when offline or unknown).
    pub viewers: Option<u64>,
    /// Whether this account's worker was last told Rosorah is streaming.
    live_sent: Option<bool>,
    worker: Option<std::thread::JoinHandle<()>>,
    saved_config: Option<ClientConfig>,
    tx: Option<mpsc::SyncSender<Command>>,
    rx: Option<mpsc::Receiver<Event>>,
    cancel: Arc<AtomicBool>,
}
impl Account {
    pub fn new(platform: Platform) -> Self {
        Self {
            reward_result:None,reward_busy:false,
            platform,
            status: "Disconnected".into(),
            connected: false,
            busy: false,
            auth_url: String::new(),
            title: String::new(),
            category: String::new(),
            category_query: String::new(),
            categories: vec![],
            broadcasts: vec![],
            broadcast: String::new(),
            name: String::new(),
            user_id: String::new(),
            activity: Vec::new(),
            draft: Draft::default(),
            streams: vec![],
            operation_busy: false,
            remembered_status: String::new(),
            ingest: None,
            forgotten: false,
            post_results: Vec::new(),
            moderation_results: Vec::new(),
            viewers: None,
            live_sent: None,
            worker: None,
            saved_config: None,
            tx: None,
            rx: None,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
    pub fn connect(&mut self, config: ClientConfig) {
        self.begin(config, true);
    }
    pub fn restore(&mut self, config: ClientConfig) {
        if !config.id.is_empty() && vault::remembered(self.platform, &config.id) {
            self.begin(config, false);
        }
    }
    fn begin(&mut self, config: ClientConfig, interactive: bool) {
        self.stop(false);
        let prior = self.worker.take();
        self.saved_config = Some(config.clone());
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        let platform = self.platform;
        let (tx, commands) = mpsc::sync_channel(16);
        let (events, rx) = mpsc::sync_channel(512);
        self.tx = Some(tx);
        self.rx = Some(rx);
        self.busy = true;
        self.status = if interactive {
            "Opening account sign-in…"
        } else {
            "Reconnecting saved account…"
        }
        .into();
        self.worker = Some(std::thread::spawn(move || {
            if let Some(prior) = prior {
                let _ = prior.join();
            }
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let result = run(
                platform,
                config.clone(),
                &events,
                commands,
                &cancel,
                interactive,
            );
            if let Err(e) = result {
                let _ = events.send(Event::Failed(format!("{e:#}")));
            }
        }));
    }
    pub fn can_disconnect(&self) -> bool {
        self.saved_config.is_some() || self.busy || self.connected
    }
    pub fn disconnect(&mut self) {
        self.stop(true);
        self.forgotten = true;
    }
    fn stop(&mut self, forget: bool) {
        self.cancel.store(true, Ordering::Relaxed);
        if forget && let Some(config) = self.saved_config.take() {
            vault::forget_marker(self.platform, &config);
            let prior = self.worker.take();
            let platform = self.platform;
            self.worker = Some(std::thread::spawn(move || {
                if let Some(prior) = prior {
                    let _ = prior.join();
                }
                vault::forget(platform, &config);
            }));
        }
        self.tx = None;
        self.rx = None;
        self.connected = false;
        self.viewers = None;
        self.live_sent = None;
        self.busy = false;
        self.auth_url.clear();
        self.status = "Disconnected".into();
        self.operation_busy = false;
        self.ingest = None;
        self.post_results.clear();
        self.moderation_results.clear();
        self.remembered_status.clear();
        self.reward_busy=false;self.reward_result=None;
        self.name.clear();
        self.user_id.clear();
        self.activity.clear();
        self.broadcast.clear();
        self.broadcasts.clear();
        self.categories.clear();
        self.title.clear();
        self.category.clear();
    }
    pub fn grant_rewards(&mut self){if let Some(config)=self.saved_config.clone(){self.begin(config,true);}}
    pub fn request_rewards(&mut self,op:rewards::Request){
        if self.reward_busy{return;}
        if self.connected&&self.tx.as_ref().is_some_and(|tx|tx.try_send(Command::Rewards(op)).is_ok()){self.reward_busy=true;}else{self.reward_result=Some(Err("Connect Twitch, then try again.".into()));}
    }
    pub fn post(&mut self, request: String, text: String) -> bool {
        self.connected
            && self.tx.as_ref().is_some_and(|tx| {
                tx.try_send(Command::Post {
                    request,
                    text,
                    broadcast: self.broadcast.clone(),
                })
                .is_ok()
            })
    }
    pub fn moderate(&mut self, action: Moderation) -> bool {
        self.connected
            && self
                .tx
                .as_ref()
                .is_some_and(|tx| tx.try_send(Command::Moderate(action)).is_ok())
    }
    /// Tells the account whether Rosorah is streaming (called every frame; sends only changes).
    pub fn set_live(&mut self, live: bool) {
        if !self.connected || self.live_sent == Some(live) {
            return;
        }
        if self.tx.as_ref().is_some_and(|tx| tx.try_send(Command::Live(live)).is_ok()) {
            self.live_sent = Some(live);
            if !live {
                self.viewers = None;
            }
        }
    }
    pub fn send(&mut self, command: Command) {
        let operation = matches!(
            command,
            Command::Create(_) | Command::Thumbnail(_) | Command::Bind(_)
        );
        if operation && self.operation_busy {
            return;
        }
        self.operation_busy |= operation;
        if self
            .tx
            .as_ref()
            .is_none_or(|tx| tx.try_send(command).is_err())
        {
            self.operation_busy = false;
            self.status = "Connection busy or unavailable; try again".into();
        }
    }
    pub fn tick(&mut self, feed: &mut auroracast_core::ChatFeed) {
        let events: Vec<_> = self
            .rx
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for event in events {
            match event {
                Event::Moderated(s) => self.moderation_results.push(s),
                Event::Rewards(result)=>{self.reward_busy=false;self.reward_result=Some(result);},
                Event::Status(s) => self.status = s,
                Event::Remembered(s) => self.remembered_status = s,
                Event::PostResult(v) => self.post_results.push(v),
                Event::Ingest(server, key) => self.ingest = Some((server, key)),
                Event::OperationDone => self.operation_busy = false,
                Event::Selected(id) => self.broadcast = id,
                Event::Streams(v) => self.streams = v,
                Event::Activity(item) => self.activity.push(item),
                Event::Connected(name, id) => {
                    self.user_id = id;
                    self.connected = true;
                    self.busy = false;
                    self.auth_url.clear();
                    self.name = name.clone();
                    self.status = format!("Connected as {name}");
                }
                Event::Authorize(url, s) => {
                    if let Ok(mut child)=crate::desktop::open(&url){std::thread::spawn(move||{let _=child.wait();});}
                    self.auth_url = url;
                    self.status = s;
                }
                Event::Metadata(title, category) => {
                    self.title = title;
                    self.category = category;
                }
                Event::Categories(v) => self.categories = v,
                Event::Broadcasts(v) => self.broadcasts = v,
                Event::Chat(m) => {
                    feed.push(m);
                }
                Event::Viewers(n) => self.viewers = n,
                #[cfg(test)]
                Event::Delete(id) => feed.remove(self.platform, &id),
                #[cfg(test)]
                Event::Clear(author) => feed.clear_author(self.platform, &author),
                Event::Failed(s) => {
                    self.status = s;
                    self.connected = false;
                    self.viewers = None;
                    self.operation_busy = false;
                    self.busy = false;
                    self.auth_url.clear();
                }
            }
        }
    }
}
impl Drop for Account {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn wait(cancel: &AtomicBool, d: Duration) -> bool {
    let end = Instant::now() + d;
    while Instant::now() < end {
        if cancel.load(Ordering::Relaxed) {
            return true;
        }
        std::thread::sleep(
            Duration::from_millis(100).min(end.saturating_duration_since(Instant::now())),
        );
    }
    cancel.load(Ordering::Relaxed)
}
fn field<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .with_context(|| format!("Platform response is missing {key}"))
}
/// When to ask a platform how many people are watching. Only while Rosorah is streaming: every
/// 15 seconds for the first two minutes (until the platform shows the stream as live), then once a
/// minute, about as often as the platforms refresh the number. After errors it waits longer, so
/// the count never eats into a platform's request limits.
pub(super) struct ViewerCheck {
    next: Instant,
    failures: u32,
    live: bool,
    /// Quick checks left while waiting for the platform to show the stream.
    early: u8,
}
impl Default for ViewerCheck {
    fn default() -> Self {
        Self { next: Instant::now(), failures: 0, live: false, early: 0 }
    }
}
impl ViewerCheck {
    pub fn due(&self) -> bool {
        self.live && Instant::now() >= self.next
    }
    /// Rosorah went live: the first check is 15 seconds in, as platforms take a moment to notice.
    pub fn start(&mut self) {
        (self.live, self.failures, self.early) = (true, 0, 8);
        self.next = Instant::now() + Duration::from_secs(15);
    }
    /// Rosorah stopped streaming: no more checks, and the count goes away.
    pub fn stop(&mut self, tx: &mpsc::SyncSender<Event>) {
        self.live = false;
        let _ = tx.try_send(Event::Viewers(None));
    }
    pub fn done(&mut self, result: Result<Option<u64>>, tx: &mpsc::SyncSender<Event>) {
        let wait = match result {
            Ok(viewers) => {
                self.failures = 0;
                let _ = tx.try_send(Event::Viewers(viewers));
                if viewers.is_some() {
                    self.early = 0;
                    60
                } else if self.early > 0 {
                    self.early -= 1;
                    15
                } else {
                    // Not shown as live on this platform (perhaps not streaming there).
                    180
                }
            }
            Err(_) => {
                // Doubles each time (2, 4, 8 minutes...), at most 15 minutes; the last count stays shown.
                self.failures = self.failures.saturating_add(1);
                (60u64 << self.failures.min(4)).min(900)
            }
        };
        self.next = Instant::now() + Duration::from_secs(wait);
    }
}

fn json(request: RequestBuilder) -> Result<Value> {
    let response = request.send().context("Could not reach platform")?;
    let status = response.status();
    if status.as_u16() == 204 {
        return Ok(Value::Null);
    }
    let value: Value = response.json().context("Invalid platform response")?;
    if !status.is_success() {
        let reason = value["error"]["errors"][0]["reason"]
            .as_str()
            .or(value["error"].as_str())
            // Kick reports failures as {"message": "..."}.
            .or(value["message"].as_str())
            .unwrap_or("request rejected");
        // Do not echo response bodies, OAuth tokens or arbitrary remote content into diagnostics.
        let reason: String = reason
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
            .take(80)
            .collect();
        anyhow::bail!(
            "Platform request failed (HTTP {}, {}). Check account permissions or reconnect.",
            status.as_u16(),
            reason
        );
    }
    Ok(value)
}
/// Network trouble and server errors: worth retrying, never a reason to drop a sign-in.
fn transient(error: &anyhow::Error) -> bool {
    error.chain().any(|e| e.downcast_ref::<reqwest::Error>().is_some())
        || ["HTTP 429", "HTTP 500", "HTTP 502", "HTTP 503", "HTTP 504", "Invalid platform response", "Could not reach platform"]
            .iter()
            .any(|s| error.chain().any(|e| e.to_string().contains(s)))
}
/// The platform rejected the saved sign-in itself (revoked, expired or replaced).
fn revoked(error: &anyhow::Error) -> bool {
    ["HTTP 400", "HTTP 401"].iter().any(|s| error.to_string().contains(s))
}
/// Saves a renewed sign-in even while Rosorah is closing: the platform has already
/// replaced the old refresh token, so losing this save would lose the account.
fn keep(platform: Platform, config: &ClientConfig, token: &Token) -> Result<()> {
    vault::save(platform, config, token, &AtomicBool::new(false))
}
fn retry_network<T>(
    cancel: &AtomicBool,
    tx: &mpsc::SyncSender<Event>,
    mut action: impl FnMut() -> Result<T>,
) -> Result<T> {
    let mut attempt = 0_u32;
    loop {
        ensure!(!cancel.load(Ordering::Relaxed), "Connection cancelled");
        match action() {
            Ok(value) => return Ok(value),
            Err(error) if transient(&error) => {
                let delay = (5_u64 << attempt.min(6)).min(300);
                attempt = attempt.saturating_add(1);
                let _ = tx.try_send(Event::Status(format!(
                    "Connection interrupted. Retrying automatically in {delay}s…"
                )));
                ensure!(
                    !wait(cancel, Duration::from_secs(delay)),
                    "Connection cancelled"
                );
            }
            Err(error) => return Err(error),
        }
    }
}
fn run(
    platform: Platform,
    config: ClientConfig,
    tx: &mpsc::SyncSender<Event>,
    commands: mpsc::Receiver<Command>,
    cancel: &AtomicBool,
    interactive: bool,
) -> Result<()> {
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .connect_timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let remembered = vault::remembered(platform, &config.id);
    let saved = if remembered && !interactive {
        match vault::load(platform, &config, cancel) {
            Ok(v) => v,
            Err(_) if interactive => None,
            Err(e) => return Err(e),
        }
    } else {
        None
    };
    let had_saved = saved.is_some();
    let mut token = match saved {
        // A saved sign-in with time left is used as is; renewing at every start replaces
        // the refresh token each time for no benefit.
        Some(old) if old.expiry > now() / 1000 + 300 => old,
        Some(old) => match retry_network(cancel, tx, || {
            oauth::refresh(&client, &config, platform, &old)
        }) {
            Ok(next) => next,
            Err(_) if interactive => oauth::login(&client, &config, platform, tx, cancel)?,
            Err(e) if revoked(&e) => {
                anyhow::bail!("Sign-in expired. Reconnect this account in Settings > Accounts.")
            }
            Err(e) => return Err(e.context("Couldn't renew the saved sign-in; it is kept and tried again next time")),
        },
        None if interactive => oauth::login(&client, &config, platform, tx, cancel)?,
        None => anyhow::bail!("Saved account unavailable. Reconnect in Settings > Accounts."),
    };
    if had_saved
        && interactive
        && platform == Platform::Twitch
        && services::validate_twitch(&client, &config, &token).is_err()
    {
        token = oauth::login(&client, &config, platform, tx, cancel)?;
    }
    let _ = tx.send(Event::Remembered(
        match keep(platform, &config, &token) {
            Ok(()) => "Sign-in remembered in your desktop wallet".into(),
            Err(_) => {
                "Connected for this session. Unlock your desktop wallet to remember this account."
                    .into()
            }
        },
    ));
    if cancel.load(Ordering::Relaxed) {
        return Ok(());
    }
    if platform == Platform::Kick {
        return kick::session(&client, &config, token, tx, commands, cancel);
    }
    let mut session = retry_network(cancel, tx, || {
        services::Session::new(platform, &client, &config, &token, tx)
    })?;
    let mut validate = Instant::now();
    let mut refresh_retry = Instant::now();
    let mut events = events::Events::default();
    while !cancel.load(Ordering::Relaxed) {
        if token.expiry <= now() / 1000 + 90 && Instant::now() >= refresh_retry {
            match oauth::refresh(&client, &config, platform, &token) {
                Ok(next) => {
                    token = next;
                    let _ = keep(platform, &config, &token);
                    events.reset();
                    session.reset_chat();
                }
                Err(error) if revoked(&error) => {anyhow::bail!("Authorization expired or was revoked. Click Reconnect in Settings > Accounts.");}
                Err(_) => {
                    refresh_retry = Instant::now() + Duration::from_secs(30);
                    let _ = tx.try_send(Event::Status("Account refresh failed; retrying in 30 seconds. Sign in again if it persists.".into()));
                }
            }
        }
        if platform == Platform::Twitch && validate.elapsed() > Duration::from_secs(3000) {
            if services::validate_twitch(&client, &config, &token).is_err() {
                let _ = tx.try_send(Event::Status(
                    "Account validation failed; use Sign out / Sign in if it persists".into(),
                ));
            }
            validate = Instant::now();
        }
        match commands.recv_timeout(Duration::from_millis(50)) {
            Ok(Command::Moderate(action)) => {
                let text = match session.moderate(&action, &client, &config, &token) {
                    Ok(()) => action.done(),
                    Err(e) => format!("{} moderation failed: {e}", platform.name()),
                };
                let _ = tx.send(Event::Moderated(text));
            }
            Ok(Command::Live(live)) => session.watch(live, tx),
            Ok(Command::Rewards(op))=>{
                let result=if platform==Platform::Twitch{rewards::run(op,&client,&config,&token,session.user_id()).map_err(|e|format!("{e}. If permission is missing, reconnect Twitch in Settings > Accounts."))}else{Err("Rewards require Twitch".into())};
                let _=tx.send(Event::Rewards(result));
            },
            Ok(command) => {
                let operation = matches!(
                    command,
                    Command::Create(_) | Command::Thumbnail(_) | Command::Bind(_)
                );
                if matches!(command, Command::Reconnect) {
                    events.reset();
                }
                let post = if let Command::Post { request, .. } = &command {
                    Some(request.clone())
                } else {
                    None
                };
                if let Err(e) = session.command(command, &client, &config, &token, tx) {
                    if let Some(request) = post {
                        let _=tx.send(Event::PostResult(j!({"type":"post_result","request":request,"platform":platform.name(),"ok":false,"text":format!("{e}. Your draft was kept; messages are never automatically resent.")})));
                    } else {
                        let _ = tx.send(Event::Status(e.to_string()));
                    }
                }
                if operation {
                    let _ = tx.send(Event::OperationDone);
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => (),
        }
        if platform == Platform::Twitch {
            events.poll(&client, &config, &token, session.user_id(), tx);
        }
        session.discover_if_due(&client, &config, &token, tx);
        session.subscribers_if_due(&client, &token, tx);
        session.viewers_if_due(&client, &config, &token, tx);
        // Browser chat owns message delivery. No YouTube message polling or duplicate IRC reader.
    }
    Ok(())
}

pub fn thumbnail_preview(path: &str) -> Result<image::DynamicImage> {
    let (bytes, _) = services::thumbnail_bytes(path)?;
    Ok(image::load_from_memory(&bytes)?)
}

#[cfg(test)]
mod viewer_tests {
    use super::*;
    #[test]
    fn checks_every_minute_live_less_often_offline_and_backs_off() {
        let (tx, rx) = mpsc::sync_channel(8);
        let mut check = ViewerCheck::default();
        // Nothing is checked until Rosorah goes live.
        assert!(!check.due());
        let wait = |c: &ViewerCheck| (c.next - Instant::now()).as_secs_f64().round() as u64;
        check.start();
        assert_eq!(wait(&check), 15);
        // Quick checks until the platform shows the stream...
        check.done(Ok(None), &tx);
        assert_eq!(wait(&check), 15);
        // ...then once a minute.
        check.done(Ok(Some(69)), &tx);
        assert_eq!(wait(&check), 60);
        check.done(Ok(None), &tx);
        assert_eq!(wait(&check), 180);
        while rx.try_recv().is_ok() {}
        for expected in [120, 240, 480, 900, 900] {
            check.done(Err(anyhow::anyhow!("HTTP 429")), &tx);
            assert_eq!(wait(&check), expected);
        }
        // Errors keep the last count on screen (no event); stopping clears it and the checks.
        assert!(rx.try_recv().is_err());
        check.stop(&tx);
        assert!(matches!(rx.try_recv(), Ok(Event::Viewers(None))) && !check.due());
    }
}

#[cfg(test)]
mod session_tests {
    #[test]
    fn only_a_platform_rejection_counts_as_signed_out() {
        let offline = anyhow::anyhow!("dns failure").context("Could not reach platform");
        let busy = anyhow::anyhow!("Platform request failed (HTTP 503, unavailable). Check account permissions or reconnect.");
        let garbled = anyhow::anyhow!("eof").context("Invalid platform response");
        let rejected = anyhow::anyhow!("Platform request failed (HTTP 400, invalid_grant). Check account permissions or reconnect.");
        for e in [&offline, &busy, &garbled] {
            assert!(super::transient(e) && !super::revoked(e), "{e:#}");
        }
        assert!(super::revoked(&rejected) && !super::transient(&rejected));
    }
    use super::*;
    #[test]
    fn automatic_reconnect_only_retries_transient_failures() {
        assert!(transient(&anyhow::anyhow!(
            "Platform request failed (HTTP 503, unavailable)"
        )));
        assert!(!transient(&anyhow::anyhow!(
            "Platform request failed (HTTP 401, invalid_grant)"
        )));
        assert!(!transient(&anyhow::anyhow!(
            "Platform request failed (HTTP 403, quotaExceeded)"
        )));
        let (tx, _) = mpsc::sync_channel(4);
        let mut calls = 0;
        let result: Result<()> = retry_network(&AtomicBool::new(true), &tx, || {
            calls += 1;
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(calls, 0);
    }
    #[test]
    fn moderation_summaries_name_the_user_and_duration() {
        let m = |action| Moderation { action, user: "rose".into(), author: "Rose".into(), message: String::new(), text: String::new() };
        assert_eq!(m(ModAction::Timeout(600)).done(), "Timed out Rose for 10 minutes");
        assert_eq!(m(ModAction::Timeout(86400)).done(), "Timed out Rose for 1 day");
        assert_eq!(m(ModAction::Timeout(3600)).done(), "Timed out Rose for 1 hour");
        assert_eq!(m(ModAction::Ban).done(), "Banned Rose");
        let (tx, commands) = mpsc::sync_channel(2);
        let mut account = Account::new(Platform::Kick);
        account.tx = Some(tx);
        assert!(!account.moderate(m(ModAction::Ban)), "disconnected accounts must not moderate");
        account.connected = true;
        assert!(account.moderate(m(ModAction::Unban)));
        assert!(matches!(commands.try_recv(), Ok(Command::Moderate(Moderation { action: ModAction::Unban, .. }))));
    }
    #[test]
    fn posts_use_the_shared_account_and_snapshot_its_broadcast() {
        let (tx, commands) = mpsc::sync_channel(2);
        let mut account = Account::new(Platform::YouTube);
        account.tx = Some(tx);
        assert!(!account.post("1".into(), "hi".into()));
        account.connected = true;
        account.broadcast = "current".into();
        assert!(account.post("2".into(), "hello".into()));
        let Command::Post {
            request,
            text,
            broadcast,
        } = commands.try_recv().unwrap()
        else {
            panic!("Expected shared session post")
        };
        assert_eq!(
            (request.as_str(), text.as_str(), broadcast.as_str()),
            ("2", "hello", "current")
        );
        account.disconnect();
        assert!(!account.post("3".into(), "no".into()));
        assert!(account.forgotten);
    }
}

pub(crate) fn donation_secret(
    value: Option<&str>,
    remove: bool,
    cancel: &AtomicBool,
) -> Result<Option<String>> {
    vault::donation_secret(value, remove, cancel)
}

pub(crate) fn stream_keys_secret(
    value: Option<&str>,
    remove: bool,
    cancel: &AtomicBool,
) -> Result<Option<String>> {
    vault::stream_keys_secret(value, remove, cancel)
}

pub(crate) fn kofi_secret(
    value: Option<&str>,
    remove: bool,
    cancel: &AtomicBool,
) -> Result<Option<String>> {
    vault::kofi_secret(value, remove, cancel)
}
