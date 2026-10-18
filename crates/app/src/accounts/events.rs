//! EventSub is push-based: REST is used only to create subscriptions after a new session.
use super::*;
use std::{
    collections::VecDeque,
    net::{TcpStream, ToSocketAddrs},
};
use tungstenite::{Message, WebSocket, stream::MaybeTlsStream};
type Socket = WebSocket<MaybeTlsStream<TcpStream>>;
#[derive(Default)]
pub(super) struct Events {
    socket: Option<Socket>,
    next: Option<Instant>,
    last: Option<Instant>,
    timeout: u64,
    resume: Option<String>,
    migrating: bool,
    attempts: u32,
    seen: VecDeque<String>,
}
fn connect(url: &str) -> Result<Socket> {
    let parsed = reqwest::Url::parse(url)?;
    ensure!(
        parsed.scheme() == "wss"
            && parsed.host_str() == Some("eventsub.wss.twitch.tv")
            && parsed.port_or_known_default() == Some(443),
        "Unexpected Twitch event server"
    );
    let tcp = ("eventsub.wss.twitch.tv", 443)
        .to_socket_addrs()?
        .take(4)
        .find_map(|a| TcpStream::connect_timeout(&a, Duration::from_secs(5)).ok())
        .context("Cannot reach Twitch event server")?;
    tcp.set_read_timeout(Some(Duration::from_secs(5)))?;
    tcp.set_write_timeout(Some(Duration::from_secs(5)))?;
    let (mut socket, _) = tungstenite::client_tls_with_config(url, tcp, None, None)
        .map_err(|_| anyhow::anyhow!("Twitch event TLS connection failed"))?;
    let tcp = match socket.get_mut() {
        MaybeTlsStream::Rustls(t) => &mut t.sock,
        MaybeTlsStream::Plain(t) => t,
        _ => anyhow::bail!("Unsupported transport"),
    };
    tcp.set_read_timeout(Some(Duration::from_millis(50)))?;
    Ok(socket)
}
impl Events {
    pub fn reset(&mut self) {
        self.socket = None;
        self.next = None;
        self.resume = None;
        self.migrating = false;
        self.attempts = 0;
    }
    pub fn poll(
        &mut self,
        client: &Client,
        config: &ClientConfig,
        token: &Token,
        id: &str,
        tx: &mpsc::SyncSender<Event>,
    ) {
        if self.next.is_some_and(|t| Instant::now() < t) {
            return;
        }
        if let Err(e) = self.step(client, config, token, id, tx) {
            self.socket = None;
            self.resume = None;
            self.migrating = false;
            self.attempts = self.attempts.saturating_add(1);
            let delay = (2u64.saturating_pow(self.attempts.min(5))).min(30);
            self.next = Some(Instant::now() + Duration::from_secs(delay));
            let _ = tx.try_send(Event::Status(format!(
                "Twitch activity disconnected: {e}. Retrying in {delay}s (attempt {}).",
                self.attempts
            )));
        }
    }
    fn step(
        &mut self,
        client: &Client,
        config: &ClientConfig,
        token: &Token,
        id: &str,
        tx: &mpsc::SyncSender<Event>,
    ) -> Result<()> {
        if self.socket.is_none() {
            let _ = tx.try_send(Event::Status("Connecting Twitch activity…".into()));
            let url = self.resume.take().unwrap_or_else(|| {
                "wss://eventsub.wss.twitch.tv/ws?keepalive_timeout_seconds=30".into()
            });
            self.socket = Some(connect(&url)?);
            self.last = Some(Instant::now());
            self.timeout = 15;
        }
        ensure!(
            self.last
                .is_none_or(|t| t.elapsed() <= Duration::from_secs(self.timeout + 5)),
            "Twitch event keepalive timed out"
        );
        let message = match self.socket.as_mut().unwrap().read() {
            Ok(v) => v,
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                return Ok(());
            }
            Err(_) => anyhow::bail!("Connection closed unexpectedly"),
        };
        match message {
            Message::Text(text) => {
                ensure!(text.len() < 1024 * 1024, "Event too large");
                let v: Value = serde_json::from_str(&text)?;
                self.last = Some(Instant::now());
                match v["metadata"]["message_type"].as_str().unwrap_or("") {
                    "session_welcome" => {
                        self.timeout = v["payload"]["session"]["keepalive_timeout_seconds"]
                            .as_u64()
                            .unwrap_or(30)
                            .clamp(10, 600);
                        let session = field(&v["payload"]["session"], "id")?;
                        let failures = if self.migrating {
                            vec![]
                        } else {
                            subscribe(client, config, token, id, session)
                        };
                        self.migrating = false;
                        self.attempts = 0;
                        let text = if failures.is_empty() {
                            "Twitch activity connected".into()
                        } else {
                            format!(
                                "Twitch activity connected; unavailable: {}. Sign in again to grant event permissions.",
                                failures.join(", ")
                            )
                        };
                        let _ = tx.try_send(Event::Status(text));
                    }
                    "session_keepalive" => (),
                    "session_reconnect" => {
                        let url = field(&v["payload"]["session"], "reconnect_url")?;
                        // Connect to the prescribed server; transferred session keeps its subscriptions.
                        let socket = connect(url)?;
                        self.socket = Some(socket);
                        self.migrating = true;
                        self.last = Some(Instant::now());
                    }
                    "notification" => {
                        let mid = field(&v["metadata"], "message_id")?.to_string();
                        if !self.seen.contains(&mid) {
                            self.seen.push_back(mid.clone());
                            while self.seen.len() > 2048 {
                                self.seen.pop_front();
                            }
                            if let Some(text) = describe(&v["payload"]) {
                                if let Some(kind) = crate::activity::twitch_kind(&v["payload"]) {
                                    let author = v["payload"]["event"]["user_name"]
                                        .as_str()
                                        .or_else(|| {
                                            v["payload"]["event"]["from_broadcaster_user_name"]
                                                .as_str()
                                        })
                                        .unwrap_or("Viewer");
                                    let _ = tx.send(Event::Activity(crate::activity::Item {
                                        facts: auroracast_core::automation::Facts {
                                            count: v["payload"]["event"]["bits"]
                                                .as_u64()
                                                .or_else(|| {
                                                    v["payload"]["event"]["viewers"].as_u64()
                                                })
                                                .or_else(|| v["payload"]["event"]["total"].as_u64())
                                                .unwrap_or(0),
                                            ..Default::default()
                                        },
                                        goal_count: match v["payload"]["subscription"]["type"]
                                            .as_str()
                                        {
                                            Some("channel.subscribe")
                                                if v["payload"]["event"]["is_gift"] == false =>
                                            {
                                                1
                                            }
                                            Some("channel.subscription.gift") => {
                                                v["payload"]["event"]["total"].as_u64().unwrap_or(0)
                                            }
                                            _ => 0,
                                        },
                                        reward_id: v["payload"]["event"]["reward"]["id"]
                                            .as_str()
                                            .unwrap_or("")
                                            .into(),
                                        platform: Platform::Twitch,
                                        owner: id.into(),
                                        id: format!("event-{mid}"),
                                        kind,
                                        author: author.into(),
                                        text: text.clone(),
                                        at_ms: now(),
                                        historical: false,
                                    }));
                                }
                                let _ = tx.try_send(Event::Chat(ChatMessage {
                                    metadata: Default::default(),
                                    platform: Platform::Twitch,
                                    id: format!("event-{mid}"),
                                    author: "Activity".into(),
                                    text,
                                    received_ms: now(),
                                    demo: false,
                                }));
                            }
                        }
                    }
                    "revocation" => {
                        let _ = tx.try_send(Event::Status(format!(
                            "Twitch revoked {}. Sign in again to restore access.",
                            v["payload"]["subscription"]["type"]
                                .as_str()
                                .unwrap_or("an event subscription")
                        )));
                    }
                    _ => (),
                }
            }
            Message::Ping(_) => {
                self.socket.as_mut().unwrap().flush()?;
            }
            Message::Close(_) => anyhow::bail!("Twitch closed activity connection"),
            _ => (),
        }
        Ok(())
    }
}
fn subscribe(
    client: &Client,
    config: &ClientConfig,
    token: &Token,
    id: &str,
    session: &str,
) -> Vec<String> {
    let mut failures = vec![];
    for kind in [
        "channel.follow",
        "channel.subscribe",
        "channel.subscription.message",
        "channel.subscription.gift",
        "channel.cheer",
        "channel.raid",
        "channel.channel_points_custom_reward_redemption.add",
        "channel.hype_train.begin",
        "channel.hype_train.progress",
        "channel.hype_train.end",
        "channel.poll.begin",
        "channel.poll.end",
        "channel.prediction.begin",
        "channel.prediction.end",
    ] {
        let mut condition = j!({"broadcaster_user_id":id});
        if kind == "channel.follow" {
            condition["moderator_user_id"] = id.into();
        }
        if kind == "channel.raid" {
            condition = j!({"to_broadcaster_user_id":id});
        }
        let version = if kind == "channel.follow" || kind.starts_with("channel.hype_train") {
            "2"
        } else {
            "1"
        };
        let body = j!({"type":kind,"version":version,"condition":condition,"transport":{"method":"websocket","session_id":session}});
        if json(
            client
                .post("https://api.twitch.tv/helix/eventsub/subscriptions")
                .header("Client-Id", &config.id)
                .bearer_auth(&token.access)
                .json(&body),
        )
        .is_err()
        {
            failures.push(kind.trim_start_matches("channel.").to_string());
        }
    }
    failures
}
fn describe(payload: &Value) -> Option<String> {
    let kind = payload["subscription"]["type"].as_str()?;
    let e = &payload["event"];
    let user = e["user_name"].as_str().unwrap_or("Anonymous");
    Some(match kind {
        "channel.follow" => format!("{user} followed"),
        "channel.subscribe" => format!(
            "{user} subscribed · tier {}{}",
            e["tier"].as_str().unwrap_or(""),
            if e["is_gift"].as_bool() == Some(true) {
                " · gifted"
            } else {
                ""
            }
        ),
        "channel.subscription.message" => format!(
            "{user} resubscribed · {} months · {}",
            e["cumulative_months"],
            e["message"]["text"].as_str().unwrap_or("")
        ),
        "channel.subscription.gift" => format!("{user} gifted {} subscriptions", e["total"]),
        "channel.cheer" => format!(
            "{user} cheered {} Bits · {}",
            e["bits"],
            e["message"].as_str().unwrap_or("")
        ),
        "channel.raid" => format!(
            "{} raided with {} viewers",
            e["from_broadcaster_user_name"]
                .as_str()
                .unwrap_or("A channel"),
            e["viewers"]
        ),
        "channel.channel_points_custom_reward_redemption.add" => format!(
            "{user} redeemed {} · {} points · {}",
            e["reward"]["title"].as_str().unwrap_or("reward"),
            e["reward"]["cost"],
            e["user_input"].as_str().unwrap_or("")
        ),
        k if k.starts_with("channel.hype_train") => format!(
            "Hype Train {} · level {}",
            k.rsplit('.').next().unwrap_or(""),
            e["level"]
        ),
        k if k.starts_with("channel.poll") => format!(
            "Poll {} · {}",
            k.rsplit('.').next().unwrap_or(""),
            e["title"].as_str().unwrap_or("")
        ),
        k if k.starts_with("channel.prediction") => format!(
            "Prediction {} · {}",
            k.rsplit('.').next().unwrap_or(""),
            e["title"].as_str().unwrap_or("")
        ),
        _ => return None,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raid_and_redemption_are_not_chat_text() {
        assert_eq!(describe(&j!({"subscription":{"type":"channel.raid"},"event":{"from_broadcaster_user_name":"Rose","viewers":25}})).unwrap(),"Rose raided with 25 viewers");
        assert!(describe(&j!({"subscription":{"type":"unknown"}})).is_none());
    }
}
