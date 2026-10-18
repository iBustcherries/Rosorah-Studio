//! Kick delivers activity only as webhooks to the publisher's sign-in broker. The broker
//! verifies Kick's signature and relays the channel's events over one WebSocket.
use super::*;
use std::{
    collections::VecDeque,
    net::{TcpStream, ToSocketAddrs},
};
use tungstenite::{Message, WebSocket, client::IntoClientRequest, stream::MaybeTlsStream};
type Socket = WebSocket<MaybeTlsStream<TcpStream>>;

const API: &str = "https://api.kick.com/public/v1";
/// Events Rosorah turns into activity, alerts and automation triggers. chat.message.sent
/// only supplies message IDs for Delete message; chat itself comes from the chat window.
const EVENTS: [&str; 7] = [
    "channel.followed",
    "channel.subscription.new",
    "channel.subscription.renewal",
    "channel.subscription.gifts",
    "kicks.gifted",
    "channel.reward.redemption.updated",
    "chat.message.sent",
];

#[derive(Default)]
pub(super) struct Events {
    socket: Option<Socket>,
    next: Option<Instant>,
    last: Option<Instant>,
    ping: Option<Instant>,
    attempts: u32,
    subscribed: bool,
    seen: VecDeque<String>,
    /// Recent chat messages as (sender, normalized text, Kick message id), newest last.
    messages: VecDeque<(String, String, String)>,
}

pub(super) fn endpoint(config: &ClientConfig) -> Result<reqwest::Url> {
    let mut url = kick::broker(config)?;
    let path = url.path().trim_end_matches("/token").to_string();
    url.set_path(&format!("{path}/events"));
    url.set_scheme("wss").map_err(|_| anyhow::anyhow!("Invalid Kick sign-in service address"))?;
    Ok(url)
}

fn connect(url: &reqwest::Url, token: &Token) -> Result<Socket> {
    let host = url.host_str().context("Invalid Kick sign-in service address")?.to_string();
    let port = url.port_or_known_default().unwrap_or(443);
    let tcp = (host.as_str(), port)
        .to_socket_addrs()?
        .take(4)
        .find_map(|a| TcpStream::connect_timeout(&a, Duration::from_secs(5)).ok())
        .context("Cannot reach the Kick sign-in service")?;
    tcp.set_read_timeout(Some(Duration::from_secs(10)))?;
    tcp.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut request = url.as_str().into_client_request()?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {}", token.access).parse().context("Invalid Kick token")?,
    );
    let (mut socket, _) = tungstenite::client_tls_with_config(request, tcp, None, None).map_err(|e| match e {
        tungstenite::HandshakeError::Failure(tungstenite::Error::Http(r)) if r.status() == 404 => {
            anyhow::anyhow!("the sign-in service needs updating; redeploy auth-broker")
        }
        tungstenite::HandshakeError::Failure(tungstenite::Error::Http(r)) => {
            anyhow::anyhow!("the sign-in service refused the connection (HTTP {})", r.status().as_u16())
        }
        _ => anyhow::anyhow!("Kick activity TLS connection failed"),
    })?;
    let tcp = match socket.get_mut() {
        MaybeTlsStream::Rustls(t) => &mut t.sock,
        MaybeTlsStream::Plain(t) => t,
        _ => anyhow::bail!("Unsupported transport"),
    };
    tcp.set_read_timeout(Some(Duration::from_millis(50)))?;
    Ok(socket)
}

/// Subscribes this channel to any missing events. Subscriptions persist on Kick's side,
/// so this only creates them the first time (or after Kick drops them).
fn subscribe(client: &Client, token: &Token) -> Result<Vec<String>> {
    let existing = json(client.get(format!("{API}/events/subscriptions")).bearer_auth(&token.access))?;
    let have: Vec<&str> = existing["data"].as_array().into_iter().flatten()
        .filter(|s| s["method"] == "webhook").filter_map(|s| s["event"].as_str()).collect();
    let missing: Vec<_> = EVENTS.iter().filter(|e| !have.contains(e)).map(|e| j!({"name": e, "version": 1})).collect();
    if missing.is_empty() {
        return Ok(vec![]);
    }
    let value = json(client.post(format!("{API}/events/subscriptions")).bearer_auth(&token.access)
        .json(&j!({"events": missing, "method": "webhook"})))?;
    Ok(value["data"].as_array().into_iter().flatten()
        .filter(|s| s["error"].as_str().is_some_and(|e| !e.is_empty()))
        .filter_map(|s| s["name"].as_str().map(String::from)).collect())
}

/// Kick writes emotes as [emote:123:Name]; the chat window shows the emote's name.
fn normalize(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("[emote:") {
        out.push_str(&rest[..start]);
        let tail = &rest[start + 7..];
        match tail.find(']') {
            Some(end) => {
                out.push_str(tail[..end].rsplit(':').next().unwrap_or_default());
                rest = &tail[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn user(v: &Value) -> &str {
    if v["is_anonymous"] == true {
        return "Anonymous";
    }
    v["username"].as_str().filter(|s| !s.is_empty()).unwrap_or("Viewer")
}

/// Activity line, alert kind, count and goal contribution for one Kick event.
fn describe(kind: &str, e: &Value) -> Option<(String, String, auroracast_core::EventKind, u64, u64)> {
    use auroracast_core::EventKind::*;
    Some(match kind {
        "channel.followed" => {
            let u = user(&e["follower"]);
            (u.into(), format!("{u} followed"), Follow, 0, 0)
        }
        "channel.subscription.new" => {
            let u = user(&e["subscriber"]);
            (u.into(), format!("{u} subscribed"), Subscription, 0, 1)
        }
        "channel.subscription.renewal" => {
            let u = user(&e["subscriber"]);
            let months = e["duration"].as_u64().unwrap_or(0);
            let text = if months > 1 { format!("{u} resubscribed for {months} months") } else { format!("{u} resubscribed") };
            (u.into(), text, Subscription, months, 0)
        }
        "channel.subscription.gifts" => {
            let u = user(&e["gifter"]);
            let n = e["giftees"].as_array().map_or(0, |g| g.len() as u64).max(1);
            let text = if n == 1 { format!("{u} gifted a subscription") } else { format!("{u} gifted {n} subscriptions") };
            (u.into(), text, Gift, n, n)
        }
        "kicks.gifted" => {
            let u = user(&e["sender"]);
            let amount = e["gift"]["amount"].as_u64().unwrap_or(0);
            let message = e["gift"]["message"].as_str().unwrap_or_default().trim();
            let mut text = format!("{u} sent {amount} Kicks");
            if !message.is_empty() {
                text = format!("{text}: {message}");
            }
            (u.into(), text, Bits, amount, 0)
        }
        // Kick sends an update for each status change; only the redemption itself is activity.
        "channel.reward.redemption.updated" if e["status"] == "pending" || e["status"] == "accepted" => {
            let u = user(&e["redeemer"]);
            let title = e["reward"]["title"].as_str().unwrap_or("a reward");
            let input = e["user_input"].as_str().unwrap_or_default().trim();
            let mut text = format!("{u} redeemed {title}");
            if !input.is_empty() {
                text = format!("{text}: {input}");
            }
            (u.into(), text, Redemption, e["reward"]["cost"].as_u64().unwrap_or(0), 0)
        }
        _ => return None,
    })
}

impl Events {
    pub fn reset(&mut self) {
        self.socket = None;
        self.next = None;
        self.attempts = 0;
    }
    /// The Kick message id for a message shown in the chat window, matched by sender and text.
    pub fn message_id(&self, author: &str, text: &str) -> Option<String> {
        let (author, text) = (author.trim().to_lowercase(), normalize(text));
        self.messages.iter().rev().find(|(a, t, _)| *a == author && *t == text).map(|m| m.2.clone())
    }
    pub fn poll(&mut self, client: &Client, config: &ClientConfig, token: &Token, channel: u64, tx: &mpsc::SyncSender<Event>) {
        if self.next.is_some_and(|t| Instant::now() < t) {
            return;
        }
        if let Err(e) = self.step(client, config, token, channel, tx) {
            self.socket = None;
            self.attempts = self.attempts.saturating_add(1);
            let delay = (2u64.saturating_pow(self.attempts.min(5))).min(30);
            self.next = Some(Instant::now() + Duration::from_secs(delay));
            let _ = tx.try_send(Event::Status(format!(
                "Kick activity disconnected: {e}. Retrying in {delay}s (attempt {}).",
                self.attempts
            )));
        }
    }
    fn step(&mut self, client: &Client, config: &ClientConfig, token: &Token, channel: u64, tx: &mpsc::SyncSender<Event>) -> Result<()> {
        if self.socket.is_none() {
            let _ = tx.try_send(Event::Status("Connecting Kick activity…".into()));
            self.socket = Some(connect(&endpoint(config)?, token)?);
            self.last = Some(Instant::now());
            self.ping = Some(Instant::now());
            if !self.subscribed {
                match subscribe(client, token) {
                    Ok(failed) if failed.is_empty() => {
                        self.subscribed = true;
                        let _ = tx.try_send(Event::Status("Kick activity connected".into()));
                    }
                    Ok(failed) => {
                        self.subscribed = true;
                        let _ = tx.try_send(Event::Status(format!("Kick activity connected; unavailable: {}", failed.join(", "))));
                    }
                    Err(e) => {
                        let _ = tx.try_send(Event::Status(format!(
                            "Kick activity connected, but events could not be enabled: {e} Turn on webhooks for your Kick app, then sign out of Kick and connect again."
                        )));
                    }
                }
            } else {
                let _ = tx.try_send(Event::Status("Kick activity connected".into()));
            }
            self.attempts = 0;
        }
        // The broker answers "ping" with "pong"; silence for 75 seconds means a dead connection.
        if self.ping.is_some_and(|t| t.elapsed() >= Duration::from_secs(30)) {
            self.socket.as_mut().unwrap().send(Message::text("ping"))?;
            self.ping = Some(Instant::now());
        }
        ensure!(self.last.is_none_or(|t| t.elapsed() <= Duration::from_secs(75)), "Kick activity keepalive timed out");
        let message = match self.socket.as_mut().unwrap().read() {
            Ok(v) => v,
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => return Ok(()),
            Err(_) => anyhow::bail!("Connection closed unexpectedly"),
        };
        self.last = Some(Instant::now());
        match message {
            Message::Text(text) if text.as_str() == "pong" => (),
            Message::Text(text) => {
                ensure!(text.len() < 256 * 1024, "Event too large");
                let v: Value = serde_json::from_str(&text)?;
                self.handle(&v, channel, tx);
            }
            Message::Close(_) => anyhow::bail!("the sign-in service closed the connection"),
            _ => (),
        }
        Ok(())
    }
    fn handle(&mut self, v: &Value, channel: u64, tx: &mpsc::SyncSender<Event>) {
        let (Some(kind), Some(mid)) = (v["type"].as_str(), v["id"].as_str()) else { return };
        let e = &v["payload"];
        if e["broadcaster"]["user_id"].as_u64().is_some_and(|id| id != channel) || self.seen.iter().any(|s| s == mid) {
            return;
        }
        self.seen.push_back(mid.into());
        while self.seen.len() > 2048 {
            self.seen.pop_front();
        }
        if kind == "chat.message.sent" {
            if let (Some(id), Some(content)) = (e["message_id"].as_str(), e["content"].as_str()) {
                self.messages.push_back((e["sender"]["username"].as_str().unwrap_or_default().to_lowercase(), normalize(content), id.into()));
                while self.messages.len() > 500 {
                    self.messages.pop_front();
                }
            }
            return;
        }
        // A redemption's later status changes share its id; alert on the first one only.
        let event_id = match kind {
            "channel.reward.redemption.updated" => format!("event-kick-{}", e["id"].as_str().unwrap_or(mid)),
            _ => format!("event-kick-{mid}"),
        };
        if kind == "channel.reward.redemption.updated" {
            if self.seen.iter().any(|s| *s == event_id) {
                return;
            }
            self.seen.push_back(event_id.clone());
        }
        let Some((author, text, kind, count, goal)) = describe(kind, e) else { return };
        let _ = tx.send(Event::Activity(crate::activity::Item {
            facts: auroracast_core::automation::Facts { count, ..Default::default() },
            reward_id: if kind == auroracast_core::EventKind::Redemption { e["reward"]["id"].as_str().unwrap_or_default().into() } else { String::new() },
            goal_count: goal,
            platform: Platform::Kick,
            owner: channel.to_string(),
            id: event_id.clone(),
            kind,
            author,
            text: text.clone(),
            at_ms: now(),
            historical: false,
        }));
        let _ = tx.try_send(Event::Chat(ChatMessage {
            metadata: Default::default(),
            platform: Platform::Kick,
            id: event_id,
            author: "Activity".into(),
            text,
            received_ms: now(),
            demo: false,
        }));
    }
}

#[cfg(test)] mod tests {
    use super::*;
    fn relay(events: &mut Events, kind: &str, id: &str, payload: Value) -> Vec<Event> {
        let (tx, rx) = mpsc::sync_channel(16);
        events.handle(&j!({"type": kind, "id": id, "payload": payload}), 7, &tx);
        rx.try_iter().collect()
    }
    #[test] fn endpoint_follows_broker_path() {
        let config = ClientConfig {broker_url: "https://auth.example.com/kick/".into(), ..Default::default()};
        assert_eq!(endpoint(&config).unwrap().as_str(), "wss://auth.example.com/kick/events");
    }
    #[test] fn gifts_become_activity_once_and_other_channels_are_ignored() {
        let mut events = Events::default();
        let gift = j!({"broadcaster": {"user_id": 7}, "gifter": {"username": "Rose"}, "giftees": [{}, {}, {}]});
        let out = relay(&mut events, "channel.subscription.gifts", "a", gift.clone());
        let Some(Event::Activity(item)) = out.first() else { panic!("no activity") };
        assert_eq!((item.text.as_str(), item.kind, item.goal_count), ("Rose gifted 3 subscriptions", auroracast_core::EventKind::Gift, 3));
        assert!(relay(&mut events, "channel.subscription.gifts", "a", gift).is_empty());
        assert!(relay(&mut events, "channel.followed", "b", j!({"broadcaster": {"user_id": 8}, "follower": {"username": "X"}})).is_empty());
    }
    #[test] fn redemption_alerts_once_across_status_updates() {
        let mut events = Events::default();
        let r = |status| j!({"id": "r1", "status": status, "redeemer": {"username": "Rose"}, "reward": {"id": "w", "title": "Hydrate"}});
        assert_eq!(relay(&mut events, "channel.reward.redemption.updated", "m1", r("pending")).len(), 2);
        assert!(relay(&mut events, "channel.reward.redemption.updated", "m2", r("accepted")).is_empty());
    }
    #[test] fn chat_ids_match_window_text_including_emotes() {
        let mut events = Events::default();
        assert!(relay(&mut events, "chat.message.sent", "c", j!({"message_id": "uuid-1", "sender": {"username": "Rose"}, "content": "hi [emote:37226:KEKW]  there"})).is_empty());
        assert_eq!(events.message_id("rose", "hi KEKW there").as_deref(), Some("uuid-1"));
        assert!(events.message_id("rose", "something else").is_none());
    }
}
