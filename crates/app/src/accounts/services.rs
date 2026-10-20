#[path = "broadcast.rs"]
mod broadcast;
use super::*;
pub use broadcast::Draft;
pub(super) use broadcast::thumbnail_bytes;
#[cfg(test)]
use std::collections::HashMap;
const YT: &str = "https://www.googleapis.com/youtube/v3/";
const TW: &str = "https://api.twitch.tv/helix/";

// Broadcast metadata discovery is separate from browser-based chat delivery.
// Follow the only live broadcast; keep an explicit choice when several are live.
fn choose_broadcast(response: &Value, current: &str) -> String {
    let items: Vec<_> = response["items"].as_array().into_iter().flatten().collect();
    let ended = |v: &&Value| {
        matches!(
            v["status"]["lifeCycleStatus"].as_str(),
            Some("complete" | "revoked")
        )
    };
    let candidates: Vec<_> = items
        .into_iter()
        .filter(|v| !ended(v) && v["id"].as_str().is_some_and(|id| !id.is_empty()))
        .collect();
    let live: Vec<_> = candidates
        .iter()
        .filter(|v| {
            matches!(
                v["status"]["lifeCycleStatus"].as_str(),
                Some("live" | "liveStarting")
            )
        })
        .collect();
    if live.len() == 1 {
        return live[0]["id"].as_str().unwrap_or("").into();
    }
    if live.len() > 1 {
        return live
            .iter()
            .find(|v| v["id"].as_str() == Some(current))
            .and_then(|v| v["id"].as_str())
            .unwrap_or("")
            .into();
    }
    if !current.is_empty()
        && !response["items"].as_array().into_iter().flatten().any(|v| {
            v["id"].as_str() == Some(current)
                && matches!(
                    v["status"]["lifeCycleStatus"].as_str(),
                    Some("complete" | "revoked")
                )
        })
    {
        return current.into();
    }
    let choice = if live.len() == 1 {
        Some(*live[0])
    } else if live.is_empty() && candidates.len() == 1 {
        Some(candidates[0])
    } else {
        None
    };
    choice.and_then(|v| v["id"].as_str()).unwrap_or("").into()
}
/// Moderation scopes were added after release; older sign-ins lack them until reconnected.
fn twitch_permission(e: anyhow::Error) -> anyhow::Error {
    if e.to_string().contains("HTTP 401") || e.to_string().contains("HTTP 403") {
        anyhow::anyhow!("{e} Sign out of Twitch and connect again to allow moderation.")
    } else {
        e
    }
}
pub(super) fn validate_twitch(
    client: &Client,
    config: &ClientConfig,
    token: &Token,
) -> Result<Value> {
    let v = json(
        client
            .get("https://id.twitch.tv/oauth2/validate")
            .header("Authorization", format!("OAuth {}", token.access)),
    )?;
    ensure!(
        v["client_id"].as_str() == Some(config.id.as_str()),
        "Twitch client ID does not match the session"
    );
    let scopes = v["scopes"]
        .as_array()
        .context("Twitch session has no scopes")?;
    ensure!(
        oauth::TWITCH_SCOPES
            .split_whitespace()
            .filter(|s|*s!="channel:manage:redemptions" && !s.starts_with("moderator:manage:"))
            .all(|s| scopes.iter().any(|v| v.as_str() == Some(s))),
        "Twitch permissions incomplete; reconnect and approve chat and broadcast access"
    );
    Ok(v)
}
pub(super) struct Session {
    platform: Platform,
    id: String,
    name: String,
    broadcast: String,
    chat_id: String,
    bound_stream: String,
    page: String,
    next_poll: Instant,
    buffer: String,
    next_discovery: Instant,
    discovery_failures: u32,
    discovery_burst: u8,
    metadata_pending: bool,
    next_subscribers: Instant,
    subscribers_seen: std::collections::BTreeSet<String>,
    subscribers_initial: bool,
    viewers: ViewerCheck,
}
impl Session {
    pub fn new(
        platform: Platform,
        client: &Client,
        config: &ClientConfig,
        token: &Token,
        tx: &mpsc::SyncSender<Event>,
    ) -> Result<Self> {
        let (id, name) = if platform == Platform::Twitch {
            let v = validate_twitch(client, config, token)?;
            (field(&v, "user_id")?.into(), field(&v, "login")?.into())
        } else {
            let v = json(
                client
                    .get(format!("{YT}channels"))
                    .bearer_auth(&token.access)
                    .query(&[("part", "snippet"), ("mine", "true")]),
            )?;
            (
                field(&v["items"][0], "id")?.into(),
                field(&v["items"][0]["snippet"], "title")?.into(),
            )
        };
        let mut s = Self {
            platform,
            id,
            name,
            broadcast: String::new(),
            chat_id: String::new(),
            bound_stream: String::new(),
            page: String::new(),
            next_poll: Instant::now(),
            buffer: String::new(),
            next_discovery: Instant::now() + Duration::from_secs(120),
            discovery_failures: 0,
            discovery_burst: 0,
            metadata_pending: false,
            next_subscribers: Instant::now(),
            subscribers_seen: Default::default(),
            subscribers_initial: true,
            viewers: ViewerCheck::default(),
        };
        let _ = tx.send(Event::Connected(s.name.clone(), s.id.clone()));
        if let Err(e) = s.command(Command::Refresh, client, config, token, tx) {
            s.metadata_pending = true;
            s.discovery_failures = 1;
            s.next_discovery =
                Instant::now() + Duration::from_secs(discovery_retry(&e.to_string(), 1));
            let _ = tx.send(Event::Status(e.to_string()));
        }
        Ok(s)
    }
    fn twitch(
        &self,
        client: &Client,
        config: &ClientConfig,
        token: &Token,
        method: reqwest::Method,
        path: &str,
    ) -> RequestBuilder {
        client
            .request(method, format!("{TW}{path}"))
            .header("Client-Id", &config.id)
            .bearer_auth(&token.access)
    }
    fn yt(
        &self,
        client: &Client,
        token: &Token,
        method: reqwest::Method,
        path: &str,
    ) -> RequestBuilder {
        client
            .request(method, format!("{YT}{path}"))
            .bearer_auth(&token.access)
    }
    pub fn user_id(&self) -> &str {
        &self.id
    }
    pub fn moderate(
        &self,
        m: &Moderation,
        client: &Client,
        config: &ClientConfig,
        token: &Token,
    ) -> Result<()> {
        use reqwest::Method as M;
        if self.platform == Platform::Twitch {
            let me = [("broadcaster_id", self.id.as_str()), ("moderator_id", self.id.as_str())];
            if m.action == ModAction::ClearChat {
                json(self.twitch(client, config, token, M::DELETE, "moderation/chat").query(&me))
                    .map_err(twitch_permission)?;
                return Ok(());
            }
            ensure!(
                m.action != ModAction::Delete,
                "Twitch's chat page doesn't include message IDs; use a timeout, which also removes their recent messages"
            );
            ensure!(
                !m.user.is_empty() && m.user.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
                "This message has no Twitch username; moderate it on Twitch"
            );
            let user = json(self.twitch(client, config, token, M::GET, "users").query(&[("login", &m.user)]))?;
            let target = user["data"][0]["id"].as_str().context("Twitch user not found")?.to_string();
            let bans = self.twitch(client, config, token, if m.action == ModAction::Unban { M::DELETE } else { M::POST }, "moderation/bans").query(&me);
            json(match m.action {
                ModAction::Timeout(seconds) => bans.json(&j!({"data":{"user_id":target,"duration":seconds}})),
                ModAction::Unban => bans.query(&[("user_id", &target)]),
                _ => bans.json(&j!({"data":{"user_id":target}})),
            })
            .map_err(twitch_permission)?;
            return Ok(());
        }
        ensure!(!self.chat_id.is_empty(), "YouTube chat is not available yet");
        ensure!(!m.message.is_empty(), "This message has no YouTube message ID");
        match m.action {
            ModAction::Delete => {
                json(self.yt(client, token, M::DELETE, "liveChat/messages").query(&[("id", &m.message)]))?;
            }
            ModAction::Timeout(_) | ModAction::Ban => {
                // The chat page doesn't expose the author's channel; look it up from the message once.
                let list = json(self.yt(client, token, M::GET, "liveChat/messages").query(&[
                    ("liveChatId", self.chat_id.as_str()),
                    ("part", "snippet,authorDetails"),
                    ("maxResults", "2000"),
                ]))?;
                let channel = list["items"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|i| i["id"].as_str() == Some(m.message.as_str()))
                    .and_then(|i| i["authorDetails"]["channelId"].as_str())
                    .context("That message is no longer in YouTube's recent chat; moderate it in YouTube Studio")?;
                let mut snippet = j!({"liveChatId":self.chat_id,"type":"permanent","bannedUserDetails":{"channelId":channel}});
                if let ModAction::Timeout(seconds) = m.action {
                    snippet["type"] = "temporary".into();
                    snippet["banDurationSeconds"] = seconds.into();
                }
                json(self.yt(client, token, M::POST, "liveChat/bans").query(&[("part", "snippet")]).json(&j!({"snippet":snippet})))?;
            }
            ModAction::Unban | ModAction::ClearChat => {
                anyhow::bail!("YouTube doesn't allow this from Rosorah; use YouTube Studio")
            }
        }
        Ok(())
    }
    pub fn subscribers_if_due(
        &mut self,
        client: &Client,
        token: &Token,
        tx: &mpsc::SyncSender<Event>,
    ) {
        if self.platform != Platform::YouTube || Instant::now() < self.next_subscribers {
            return;
        }
        // One quota unit per request; never polls liveChatMessages.
        self.next_subscribers = Instant::now()
            + Duration::from_secs(if self.broadcast.is_empty() { 1800 } else { 300 });
        match json(
            self.yt(client, token, reqwest::Method::GET, "subscriptions")
                .query(&[
                    ("part", "snippet,subscriberSnippet"),
                    ("myRecentSubscribers", "true"),
                    ("maxResults", "50"),
                ]),
        ) {
            Ok(v) => {
                for item in crate::activity::subscribers(&v, &self.id, self.subscribers_initial) {
                    if self.subscribers_seen.insert(item.id.clone()) {
                        let _ = tx.send(Event::Activity(item));
                    }
                }
                self.subscribers_initial = false;
                if self.subscribers_seen.len() > 5000 {
                    self.subscribers_seen.clear();
                }
            }
            Err(e) => {
                self.next_subscribers = Instant::now() + Duration::from_secs(3600);
                let _ = tx.send(Event::Status(format!("YouTube subscriber history temporarily unavailable: {e}. Chat remains connected; retrying later.")));
            }
        }
    }
    /// Starts or stops the viewer checks as Rosorah goes live or stops.
    pub fn watch(&mut self, live: bool, tx: &mpsc::SyncSender<Event>) {
        if live { self.viewers.start() } else { self.viewers.stop(tx) }
    }
    /// Asks how many people are watching, when it's time to (see ViewerCheck).
    pub fn viewers_if_due(&mut self, client: &Client, config: &ClientConfig, token: &Token, tx: &mpsc::SyncSender<Event>) {
        if !self.viewers.due() {
            return;
        }
        let result = if self.platform == Platform::Twitch {
            // An empty list means the channel is offline.
            json(self.twitch(client, config, token, reqwest::Method::GET, "streams").query(&[("user_id", self.id.as_str())]))
                .map(|v| v["data"][0]["viewer_count"].as_u64())
        } else if self.broadcast.is_empty() {
            // Nothing selected or live: no request at all.
            Ok(None)
        } else {
            // One quota unit; YouTube only reports viewers while the broadcast is live.
            json(self.yt(client, token, reqwest::Method::GET, "videos").query(&[("part", "liveStreamingDetails"), ("id", self.broadcast.as_str())])).map(|v| {
                let live = &v["items"][0]["liveStreamingDetails"];
                if live["actualEndTime"].is_string() { None } else { live["concurrentViewers"].as_str().and_then(|n| n.parse().ok()) }
            })
        };
        self.viewers.done(result, tx);
    }
    pub fn reset_chat(&mut self) {
        self.buffer.clear();
        self.next_poll = Instant::now();
    }
    pub fn discover_if_due(
        &mut self,
        client: &Client,
        config: &ClientConfig,
        token: &Token,
        tx: &mpsc::SyncSender<Event>,
    ) {
        if Instant::now() < self.next_discovery
            || (self.platform == Platform::Twitch && !self.metadata_pending)
        {
            return;
        }
        let result = if self.platform == Platform::Twitch {
            self.command(Command::Refresh, client, config, token, tx)
                .map(|_| {
                    self.metadata_pending = false;
                    false
                })
        } else {
            self.discover(client, token, tx)
        };
        match result {
            Ok(live) => {
                self.discovery_failures = 0;
                let delay = if self.discovery_burst > 0 {
                    self.discovery_burst -= 1;
                    10
                } else if live {
                    60
                } else {
                    120
                };
                self.next_discovery = Instant::now() + Duration::from_secs(delay);
            }
            Err(e) => {
                self.discovery_failures = self.discovery_failures.saturating_add(1);
                let delay = discovery_retry(&e.to_string(), self.discovery_failures);
                self.next_discovery = Instant::now() + Duration::from_secs(delay);
                let _ = tx.try_send(Event::Status(format!(
                    "Account details will retry automatically in {delay}s: {e}"
                )));
            }
        }
    }
    fn discover(
        &mut self,
        client: &Client,
        token: &Token,
        tx: &mpsc::SyncSender<Event>,
    ) -> Result<bool> {
        let mut v = json(
            self.yt(client, token, reqwest::Method::GET, "liveBroadcasts")
                .query(&[
                    ("part", "snippet,status,contentDetails"),
                    ("broadcastStatus", "active"),
                    ("broadcastType", "all"),
                    ("maxResults", "50"),
                ]),
        )?;
        let live = v["items"].as_array().is_some_and(|a| !a.is_empty());
        if !live {
            v = json(
                self.yt(client, token, reqwest::Method::GET, "liveBroadcasts")
                    .query(&[
                        ("part", "snippet,status,contentDetails"),
                        ("mine", "true"),
                        ("broadcastType", "all"),
                        ("maxResults", "50"),
                    ]),
            )?;
        }
        self.apply_discovery(&v, client, token, tx)?;
        Ok(live)
    }
    fn apply_discovery(
        &mut self,
        v: &Value,
        client: &Client,
        token: &Token,
        tx: &mpsc::SyncSender<Event>,
    ) -> Result<()> {
        let choices = v["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|v| {
                !matches!(
                    v["status"]["lifeCycleStatus"].as_str(),
                    Some("complete" | "revoked")
                )
            })
            .filter_map(|v| {
                Some(Choice {
                    id: v["id"].as_str()?.into(),
                    name: v["snippet"]["title"].as_str()?.into(),
                })
            })
            .collect();
        let _ = tx.send(Event::Broadcasts(choices));
        let selected = choose_broadcast(v, &self.broadcast);
        if selected.is_empty() {
            self.broadcast.clear();
            self.chat_id.clear();
            let _ = tx.send(Event::Selected(String::new()));
            let _=tx.send(Event::Status("Connected — waiting for your YouTube broadcast. Multiple active broadcasts can be chosen in Broadcast.".into()));
        } else if selected != self.broadcast || self.chat_id.is_empty() {
            self.select(selected, client, token, tx)?;
        } else if let Some(item) = v["items"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|v| v["id"].as_str() == Some(selected.as_str()))
        {
            self.chat_id = item["snippet"]["liveChatId"].as_str().unwrap_or("").into();
            if item["contentDetails"]["boundStreamId"]
                .as_str()
                .unwrap_or("")
                != self.bound_stream
            {
                self.select(selected, client, token, tx)?;
            }
        } else {
            // Explicit selections outside the first page must still be checked for completion.
            let state = json(
                self.yt(client, token, reqwest::Method::GET, "liveBroadcasts")
                    .query(&[("part", "status,snippet"), ("id", selected.as_str())]),
            )?;
            if state["items"].as_array().is_none_or(|a| a.is_empty())
                || matches!(
                    state["items"][0]["status"]["lifeCycleStatus"].as_str(),
                    Some("complete" | "revoked")
                )
            {
                self.broadcast.clear();
                self.chat_id.clear();
                let _ = tx.send(Event::Selected(String::new()));
            }
        }
        Ok(())
    }
    fn send_chat(
        &self,
        text: &str,
        expected: &str,
        client: &Client,
        config: &ClientConfig,
        token: &Token,
    ) -> Result<()> {
        ensure!(
            !text.trim().is_empty()
                && text.chars().count()
                    <= if self.platform == Platform::YouTube {
                        200
                    } else {
                        500
                    },
            "Message is empty or too long"
        );
        if self.platform == Platform::Twitch {
            let v = json(
                self.twitch(
                    client,
                    config,
                    token,
                    reqwest::Method::POST,
                    "chat/messages",
                )
                .json(&j!({"broadcaster_id":self.id,"sender_id":self.id,"message":text})),
            )?;
            ensure!(
                v["data"][0]["is_sent"] == true,
                "Twitch did not send this message. Check chat restrictions or reconnect the account in Settings"
            );
        } else {
            ensure!(
                !expected.is_empty() && expected == self.broadcast,
                "The active broadcast changed. Review it before sending"
            );
            ensure!(
                !self.chat_id.is_empty(),
                "YouTube chat is not available yet"
            );
            let v=json(self.yt(client,token,reqwest::Method::POST,"liveChat/messages").query(&[("part","snippet")]).json(&j!({"snippet":{"liveChatId":self.chat_id,"type":"textMessageEvent","textMessageDetails":{"messageText":text}}})))?;
            ensure!(
                v["id"].as_str().is_some_and(|id| !id.is_empty()),
                "YouTube did not confirm message delivery"
            );
        }
        Ok(())
    }
    pub fn command(
        &mut self,
        command: Command,
        client: &Client,
        config: &ClientConfig,
        token: &Token,
        tx: &mpsc::SyncSender<Event>,
    ) -> Result<()> {
        use reqwest::Method as M;
        match command {
            Command::Rewards(_)=>anyhow::bail!("Reward request must use the account worker"),
            Command::Moderate(_) => anyhow::bail!("Moderation must use the account worker"),
            Command::Live(_) => anyhow::bail!("Going live is handled by the account worker"),
            Command::Discover => {
                self.discovery_burst = 6;
                self.next_discovery = Instant::now();
            }
            Command::Post {
                request,
                text,
                broadcast,
            } => {
                self.send_chat(&text, &broadcast, client, config, token)?;
                let _=tx.send(Event::PostResult(j!({"type":"post_result","platform":self.platform.name(),"request":request,"ok":true,"text":"Sent"})));
            }
            Command::Reconnect => {
                self.reset_chat();
                let _ = tx.try_send(Event::Status("Reconnecting Twitch activity…".into()));
            }
            Command::Create(draft) => self.create_broadcast(&draft, client, token, tx)?,
            Command::Thumbnail(path) => {
                ensure!(!self.broadcast.is_empty(), "Select a broadcast first");
                self.upload_thumbnail(&self.broadcast, &path, client, token)?;
                let _ = tx.try_send(Event::Status("Thumbnail uploaded".into()));
            }
            Command::Bind(id) => {
                ensure!(
                    !self.broadcast.is_empty() && !id.is_empty(),
                    "Select a broadcast and stream key first"
                );
                self.bind_broadcast(&self.broadcast, &id, client, token)?;
                self.select(self.broadcast.clone(), client, token, tx)?;
                let _ = tx.try_send(Event::Status(
                    "Broadcast linked. Its stream key is supplied to Streaming automatically."
                        .into(),
                ));
            }
            Command::Refresh => {
                if self.platform == Platform::Twitch {
                    let v = json(
                        self.twitch(client, config, token, M::GET, "channels")
                            .query(&[("broadcaster_id", &self.id)]),
                    )?;
                    let item = &v["data"][0];
                    let _ = tx.send(Event::Metadata(
                        field(item, "title").unwrap_or("").into(),
                        item["game_id"].as_str().unwrap_or("").into(),
                    ));
                    let _ = tx.send(Event::Categories(vec![Choice {
                        id: item["game_id"].as_str().unwrap_or("").into(),
                        name: item["game_name"].as_str().unwrap_or("No category").into(),
                    }]));
                    let v = json(
                        self.twitch(client, config, token, M::GET, "streams/key")
                            .query(&[("broadcaster_id", self.id.as_str())]),
                    )?;
                    if let Some(key) = v["data"][0]["stream_key"].as_str() {
                        let _ = tx.send(Event::Ingest(
                            "rtmp://ingest.global-contribute.live-video.net/app".into(),
                            key.into(),
                        ));
                    }
                } else {
                    self.discover(client, token, tx)?;
                    self.next_discovery = Instant::now() + Duration::from_secs(120);
                    let streams = json(self.yt(client, token, M::GET, "liveStreams").query(&[
                        ("part", "id,snippet"),
                        ("mine", "true"),
                        ("maxResults", "50"),
                    ]))?;
                    let choices = streams["items"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|v| {
                            Some(Choice {
                                id: v["id"].as_str()?.into(),
                                name: v["snippet"]["title"].as_str()?.into(),
                            })
                        })
                        .collect();
                    let _ = tx.try_send(Event::Streams(choices));
                    let v = json(
                        self.yt(client, token, M::GET, "videoCategories")
                            .query(&[("part", "snippet"), ("regionCode", "US")]),
                    )?;
                    let choices = v["items"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|v| v["snippet"]["assignable"].as_bool() == Some(true))
                        .filter_map(|v| {
                            Some(Choice {
                                id: v["id"].as_str()?.into(),
                                name: v["snippet"]["title"].as_str()?.into(),
                            })
                        })
                        .collect();
                    let _ = tx.send(Event::Categories(choices));
                }
            }
            Command::Select(id) => self.select(id, client, token, tx)?,
            Command::Search(query) => {
                ensure!(
                    self.platform == Platform::Twitch,
                    "Choose a YouTube category from the list"
                );
                let v = json(
                    self.twitch(client, config, token, M::GET, "search/categories")
                        .query(&[("query", query.as_str()), ("first", "30")]),
                )?;
                let choices = v["data"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| {
                        Some(Choice {
                            id: v["id"].as_str()?.into(),
                            name: v["name"].as_str()?.into(),
                        })
                    })
                    .collect();
                let _ = tx.send(Event::Categories(choices));
            }
            Command::Update { title, category } => {
                ensure!(
                    !title.trim().is_empty()
                        && title.chars().count()
                            <= if self.platform == Platform::Twitch {
                                140
                            } else {
                                100
                            },
                    "Title is empty or too long"
                );
                if self.platform == Platform::Twitch {
                    json(
                        self.twitch(client, config, token, M::PATCH, "channels")
                            .query(&[("broadcaster_id", &self.id)])
                            .json(&j!({"title":title,"game_id":category})),
                    )?;
                } else {
                    ensure!(
                        !self.broadcast.is_empty(),
                        "Choose a YouTube broadcast first"
                    );
                    ensure!(!category.is_empty(), "Choose a YouTube category");
                    let v = json(
                        self.yt(client, token, M::GET, "videos")
                            .query(&[("part", "snippet"), ("id", self.broadcast.as_str())]),
                    )?;
                    let body = updated_video(&v["items"][0], &title, &category)?;
                    json(
                        self.yt(client, token, M::PUT, "videos")
                            .query(&[("part", "snippet")])
                            .json(&body),
                    )?;
                }
                let _ = tx.send(Event::Status("Stream title and category updated".into()));
            }
        }
        Ok(())
    }
    fn select(
        &mut self,
        id: String,
        client: &Client,
        token: &Token,
        tx: &mpsc::SyncSender<Event>,
    ) -> Result<()> {
        ensure!(
            self.platform == Platform::YouTube,
            "Broadcast selection is for YouTube"
        );
        let v = json(
            self.yt(client, token, reqwest::Method::GET, "liveBroadcasts")
                .query(&[
                    ("part", "snippet,contentDetails,status"),
                    ("id", id.as_str()),
                ]),
        )?;
        let item = &v["items"][0];
        ensure!(
            item["id"].as_str() == Some(id.as_str()),
            "Broadcast is unavailable"
        );
        ensure!(
            !matches!(
                item["status"]["lifeCycleStatus"].as_str(),
                Some("complete" | "revoked")
            ),
            "This broadcast has ended"
        );
        if let Some(bound) = item["contentDetails"]["boundStreamId"]
            .as_str()
            .filter(|v| !v.is_empty())
        {
            let stream = json(
                self.yt(client, token, reqwest::Method::GET, "liveStreams")
                    .query(&[("part", "cdn"), ("id", bound)]),
            )?;
            let info = &stream["items"][0]["cdn"]["ingestionInfo"];
            if let Some(key) = info["streamName"].as_str() {
                let server = info["rtmpsIngestionAddress"]
                    .as_str()
                    .or(info["ingestionAddress"].as_str())
                    .unwrap_or("rtmps://a.rtmps.youtube.com:443/live2");
                let _ = tx.send(Event::Ingest(server.into(), key.into()));
            }
        }
        self.bound_stream = item["contentDetails"]["boundStreamId"]
            .as_str()
            .unwrap_or("")
            .into();
        self.broadcast = id;
        let _ = tx.try_send(Event::Selected(self.broadcast.clone()));
        self.chat_id = item["snippet"]["liveChatId"].as_str().unwrap_or("").into();
        self.page.clear();
        self.next_poll = Instant::now();
        let v = json(
            self.yt(client, token, reqwest::Method::GET, "videos")
                .query(&[("part", "snippet"), ("id", self.broadcast.as_str())]),
        )?;
        let snippet = &v["items"][0]["snippet"];
        let _ = tx.send(Event::Metadata(
            snippet["title"].as_str().unwrap_or("").into(),
            snippet["categoryId"].as_str().unwrap_or("").into(),
        ));
        let _ = tx.send(Event::Status(
            if self.chat_id.is_empty() {
                "Broadcast selected; live chat is unavailable"
            } else {
                "Broadcast selected; chat follows automatically"
            }
            .into(),
        ));
        Ok(())
    }
}
fn discovery_retry(error: &str, failures: u32) -> u64 {
    if error.contains("quotaExceeded") || error.contains("dailyLimitExceeded") {
        3600
    } else {
        (30_u64 << failures.min(5)).min(900)
    }
}
pub(super) fn updated_video(item: &Value, title: &str, category: &str) -> Result<Value> {
    let id = field(item, "id")?;
    let old = item["snippet"]
        .as_object()
        .context("Video snippet missing")?;
    let mut snippet = serde_json::Map::new();
    for key in [
        "description",
        "tags",
        "defaultLanguage",
        "defaultAudioLanguage",
    ] {
        if let Some(v) = old.get(key) {
            snippet.insert(key.into(), v.clone());
        }
    }
    snippet.insert("title".into(), j!(title));
    snippet.insert("categoryId".into(), j!(category));
    Ok(j!({"id":id,"snippet":snippet}))
}
#[cfg(test)]
fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(c) = chars.next() {
                out.push(match c {
                    's' => ' ',
                    ':' => ';',
                    'r' => '\r',
                    'n' => '\n',
                    other => other,
                });
            }
        } else {
            out.push(c);
        }
    }
    out
}
#[cfg(test)]
fn parse_irc(line: &str) -> Option<Event> {
    let (tag_string, rest) = line.strip_prefix('@')?.split_once(' ')?;
    let tags: HashMap<_, _> = tag_string
        .split(';')
        .filter_map(|s| s.split_once('='))
        .collect();
    let (_, command) = rest.split_once(' ')?;
    if command.starts_with("CLEARMSG ") {
        return Some(Event::Delete(tags.get("target-msg-id")?.to_string()));
    }
    if command.starts_with("CLEARCHAT ") {
        return Some(Event::Clear(
            command
                .split_once(" :")
                .map(|(_, s)| s)
                .unwrap_or("")
                .into(),
        ));
    }
    if !command.starts_with("PRIVMSG ") {
        return None;
    }
    Some(Event::Chat(ChatMessage {
        metadata: auroracast_core::alert_filters::ChatMetadata {
            moderator: tags.get("mod").copied() == Some("1")
                || tags
                    .get("badges")
                    .is_some_and(|s| s.contains("broadcaster/")),
            subscriber: tags.get("subscriber").copied() == Some("1"),
            user_id: tags.get("user-id").copied().unwrap_or("").into(),
            first_ever: tags.get("first-msg").copied() == Some("1"),
            historical: false,
        },
        platform: Platform::Twitch,
        id: tags.get("id")?.to_string(),
        author: unescape(tags.get("display-name").copied().unwrap_or("Twitch")),
        text: command.split_once(" :")?.1.into(),
        received_ms: tags
            .get("tmi-sent-ts")
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(now),
        demo: false,
    }))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_chat_and_moderation() {
        let Event::Chat(m)=parse_irc("@id=123;display-name=Jane\\sDoe;tmi-sent-ts=42 :jane!jane@host PRIVMSG #channel :hello : world").unwrap() else{panic!()};
        assert_eq!(
            (
                m.id.as_str(),
                m.author.as_str(),
                m.text.as_str(),
                m.received_ms
            ),
            ("123", "Jane Doe", "hello : world", 42)
        );
        assert!(
            matches!(parse_irc("@target-msg-id=123 :tmi.twitch.tv CLEARMSG #channel :hello"),Some(Event::Delete(id)) if id=="123")
        );
        assert!(parse_irc("PING :tmi.twitch.tv").is_none());
    }
    #[test]
    fn title_update_preserves_existing_editable_snippet() {
        let item = j!({"id":"video","snippet":{"title":"old","categoryId":"1","description":"keep","tags":["a"],"defaultLanguage":"en","thumbnails":{"default":{}},"channelId":"owner"}});
        let updated = updated_video(&item, "new", "20").unwrap();
        assert_eq!(updated["snippet"]["description"], "keep");
        assert_eq!(updated["snippet"]["tags"], j!(["a"]));
        assert_eq!(updated["snippet"]["title"], "new");
        assert!(updated["snippet"].get("channelId").is_none());
    }
}

#[cfg(test)]
mod discovery_tests {
    use super::*;
    fn list(rows: &[(&str, &str)]) -> Value {
        j!({"items":rows.iter().map(|(id,status)| j!({"id":id,"status":{"lifeCycleStatus":status}})).collect::<Vec<_>>()})
    }
    #[test]
    fn prefers_only_live_broadcast_over_scheduled_choices() {
        let v = list(&[("later", "ready"), ("now", "live"), ("old", "complete")]);
        assert_eq!(choose_broadcast(&v, ""), "now");
    }
    #[test]
    fn active_broadcast_takes_over_from_stale_scheduled_selection() {
        let v = list(&[("later", "ready"), ("now", "live")]);
        assert_eq!(choose_broadcast(&v, "later"), "now");
        assert_eq!(choose_broadcast(&v, "new"), "now");
    }
    #[test]
    fn ambiguous_and_empty_results_require_selection_by_title() {
        assert_eq!(
            choose_broadcast(&list(&[("a", "ready"), ("b", "ready")]), ""),
            ""
        );
        assert_eq!(
            choose_broadcast(&list(&[("a", "live"), ("b", "live")]), ""),
            ""
        );
        assert_eq!(choose_broadcast(&list(&[]), ""), "");
    }
    #[test]
    fn simultaneous_live_streams_keep_the_valid_choice() {
        let v = list(&[("a", "live"), ("b", "live"), ("later", "ready")]);
        assert_eq!(choose_broadcast(&v, "b"), "b");
        assert_eq!(choose_broadcast(&v, "later"), "");
        assert_eq!(discovery_retry("quotaExceeded", 1), 3600);
        assert_eq!(discovery_retry("dailyLimitExceeded", 10), 3600);
        assert!(discovery_retry("unavailable", 5) > discovery_retry("unavailable", 1));
        assert!(discovery_retry("unavailable", 20) <= 900);
    }
    #[test]
    fn selects_only_available_broadcast_and_replaces_ended_selection() {
        let v = list(&[("old", "complete"), ("bad", "revoked"), ("next", "ready")]);
        assert_eq!(choose_broadcast(&v, ""), "next");
        assert_eq!(choose_broadcast(&v, "old"), "next");
        assert_eq!(choose_broadcast(&list(&[("old", "complete")]), "old"), "");
    }
}
