//! One bounded, push-only donation connection. No payment credentials in projects.
use anyhow::{Context, Result, ensure};
use auroracast_core::{
    EventKind, Platform,
    donations::{Provider, Setup},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    net::{TcpStream, ToSocketAddrs},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use tungstenite::{Message, WebSocket, stream::MaybeTlsStream};

#[derive(Clone, Serialize, Deserialize)]
pub struct Connection {
    pub provider: Provider,
    pub room: String,
    pub token: String,
}
impl Connection {
    fn validate(&self) -> Result<()> {
        ensure!(
            matches!(
                self.provider,
                Provider::StreamElements | Provider::Streamlabs
            ),
            "Choose StreamElements or Streamlabs for a direct connection."
        );
        ensure!(
            (8..=8192).contains(&self.token.len()) && !self.token.chars().any(char::is_whitespace),
            "Paste the complete event token from your donation provider."
        );
        if self.provider == Provider::StreamElements {
            ensure!(
                self.room.len() == 24 && self.room.bytes().all(|c| c.is_ascii_hexdigit()),
                "Copy your 24-character channel ID from StreamElements."
            );
        }
        Ok(())
    }
    fn owner(&self) -> String {
        if self.provider == Provider::StreamElements {
            format!("StreamElements:{}", self.room)
        } else {
            "Streamlabs".into()
        }
    }
}
#[derive(Clone, Default)]
pub struct Status {
    pub text: String,
    pub connected: bool,
    pub owner: String,
}
pub struct Donation {
    pub historical: bool,
    pub at_ms: Option<u64>,
    pub id: String,
    pub owner: String,
    pub provider: Provider,
    pub name: String,
    pub message: String,
    pub hundredths: u64,
    pub currency: String,
}
impl Donation {
    pub fn activity(&self, setup: &Setup) -> crate::activity::Item {
        let author = if setup.show_name {
            self.name.clone()
        } else {
            "Anonymous".into()
        };
        let text = setup
            .template
            .replace("{user}", &author)
            .replace(
                "{amount}",
                &if setup.show_amount {
                    format!("{}.{:02}", self.hundredths / 100, self.hundredths % 100)
                } else {
                    String::new()
                },
            )
            .replace(
                "{currency}",
                if setup.show_amount {
                    &self.currency
                } else {
                    ""
                },
            )
            .replace(
                "{message}",
                if setup.show_message {
                    &self.message
                } else {
                    ""
                },
            );
        crate::activity::Item {
            facts: auroracast_core::automation::Facts {
                amount_hidden: !setup.show_amount,
                amount: self.hundredths,
                currency: self.currency.clone(),
                ..Default::default()
            },
            reward_id: String::new(),
            goal_count: 0,
            platform: Platform::Donations,
            owner: self.owner.clone(),
            id: self.id.clone(),
            kind: EventKind::Donation,
            author,
            text: format!("{} · {text}", self.provider.name()),
            at_ms: self.at_ms.unwrap_or_else(crate::activity::now),
            historical: self.historical,
        }
    }
}
// Decimal parsing avoids rounding/float errors and rejects malformed/negative money.
fn amount(v: &Value) -> Option<u64> {
    let raw = match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    let (whole, fraction) = raw.split_once('.').unwrap_or((&raw, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > 2
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let cents = match fraction.len() {
        0 => 0,
        1 => fraction.parse::<u64>().ok()?.checked_mul(10)?,
        _ => fraction.parse().ok()?,
    };
    let value = whole
        .parse::<u64>()
        .ok()?
        .checked_mul(100)?
        .checked_add(cents)?;
    (value > 0 && value <= 100_000_000_000).then_some(value)
}
fn donation(
    c: &Connection,
    id: &Value,
    name: &Value,
    message: &Value,
    money: &Value,
    currency: &Value,
) -> Option<Donation> {
    let id = match id {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    let currency = currency.as_str()?.to_ascii_uppercase();
    if id.is_empty()
        || id.len() > 256
        || currency.len() != 3
        || !currency.bytes().all(|b| b.is_ascii_uppercase())
    {
        return None;
    }
    Some(Donation {
        historical: false,
        at_ms: None,
        id,
        owner: c.owner(),
        provider: c.provider,
        name: name
            .as_str()
            .unwrap_or("Anonymous")
            .chars()
            .take(100)
            .collect(),
        message: message.as_str().unwrap_or("").chars().take(1000).collect(),
        hundredths: amount(money)?,
        currency,
    })
}
fn is_test(v: &Value) -> bool {
    ["isTest", "is_test", "test", "isMock"]
        .iter()
        .any(|k| v[*k] == true || v[*k] == "true" || v[*k] == 1)
}
pub fn parse(c: &Connection, v: &Value) -> Vec<Donation> {
    if is_test(v) {
        return Vec::new();
    }
    match c.provider {
        Provider::StreamElements => {
            let d = &v["data"];
            if v["type"] != "message"
                || v["topic"] != "channel.tips"
                || v["room"].as_str() != Some(c.room.as_str())
                || d["status"] != "success"
                || d["approved"] != "allowed"
                || is_test(d)
            {
                return Vec::new();
            }
            let p = &d["donation"];
            if is_test(p) {
                return Vec::new();
            }
            donation(
                c,
                &d["_id"],
                &p["user"]["username"],
                &p["message"],
                &p["amount"],
                &p["currency"],
            )
            .into_iter()
            .collect()
        }
        Provider::Streamlabs => {
            if v["type"] != "donation" {
                return Vec::new();
            }
            v["message"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|d| !is_test(d))
                .filter_map(|d| {
                    donation(
                        c,
                        &d["id"],
                        &d["name"],
                        &d["message"],
                        &d["amount"],
                        &d["currency"],
                    )
                })
                .collect()
        }
        _ => Vec::new(),
    }
}
enum Command {
    Connect(Connection, bool),
    Disconnect,
}
pub struct Hub {
    tx: mpsc::Sender<Command>,
    rx: mpsc::Receiver<Donation>,
    state: Arc<Mutex<Status>>,
    stop: Arc<AtomicBool>,
    pub token: String,
    pub room: String,
    pub remember: bool,
}
fn marker() -> std::path::PathBuf {
    crate::streaming::Config::path().with_file_name("donation-remembered")
}
impl Hub {
    pub fn new() -> Self {
        let (tx, commands) = mpsc::channel();
        let (events, rx) = mpsc::sync_channel(256);
        let state = Arc::new(Mutex::new(Status {
            text: "Not connected".into(),
            ..Default::default()
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let (s, c) = (state.clone(), stop.clone());
        std::thread::spawn(move || worker(commands, events, s, c));
        Self {
            tx,
            rx,
            state,
            stop,
            token: String::new(),
            room: String::new(),
            remember: true,
        }
    }
    pub fn has_saved_connection(&self) -> bool {
        marker().exists()
    }
    pub fn status(&self) -> Status {
        self.state.lock().unwrap().clone()
    }
    pub fn drain(&self) -> Vec<Donation> {
        self.rx.try_iter().take(256).collect()
    }
    pub fn connect(&mut self, provider: Provider) {
        let c = Connection {
            provider,
            room: self.room.trim().to_ascii_lowercase(),
            token: self.token.trim().into(),
        };
        if let Err(e) = c.validate() {
            self.state.lock().unwrap().text = e.to_string();
            return;
        }
        let _ = self.tx.send(Command::Connect(c, self.remember));
        self.token.clear();
    }
    pub fn disconnect(&self) {
        let _ = self.tx.send(Command::Disconnect);
    }
}
impl Drop for Hub {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
type Socket = WebSocket<MaybeTlsStream<TcpStream>>;
fn connect(c: &Connection) -> Result<Socket> {
    let (host, mut url) = if c.provider == Provider::StreamElements {
        (
            "astro.streamelements.com",
            reqwest::Url::parse("wss://astro.streamelements.com/")?,
        )
    } else {
        (
            "sockets.streamlabs.com",
            reqwest::Url::parse(
                "wss://sockets.streamlabs.com/socket.io/?EIO=3&transport=websocket",
            )?,
        )
    };
    if c.provider == Provider::Streamlabs {
        url.query_pairs_mut().append_pair("token", &c.token);
    }
    let tcp = (host, 443)
        .to_socket_addrs()?
        .take(3)
        .find_map(|a| TcpStream::connect_timeout(&a, Duration::from_secs(4)).ok())
        .context("Donation server is unreachable")?;
    tcp.set_read_timeout(Some(Duration::from_secs(5)))?;
    tcp.set_write_timeout(Some(Duration::from_secs(3)))?;
    let config = tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(512 * 1024))
        .max_frame_size(Some(512 * 1024));
    let (mut socket, _) =
        tungstenite::client_tls_with_config(url.as_str(), tcp, Some(config), None).map_err(
            |_| {
                anyhow::anyhow!(
                    "Could not connect securely. Check your event token and internet connection."
                )
            },
        )?;
    match socket.get_mut() {
        MaybeTlsStream::Rustls(t) => t.sock.set_read_timeout(Some(Duration::from_millis(100)))?,
        MaybeTlsStream::Plain(t) => t.set_read_timeout(Some(Duration::from_millis(100)))?,
        _ => anyhow::bail!("Unsupported secure connection"),
    }
    Ok(socket)
}
fn status(s: &Mutex<Status>, text: &str, connected: bool, owner: &str) {
    *s.lock().unwrap() = Status {
        text: text.into(),
        connected,
        owner: owner.into(),
    };
}
fn worker(
    commands: mpsc::Receiver<Command>,
    events: mpsc::SyncSender<Donation>,
    state: Arc<Mutex<Status>>,
    stop: Arc<AtomicBool>,
) {
    let mut active: Option<Connection> = None;
    if marker().exists() {
        status(
            &state,
            "Restoring donation connection from desktop wallet…",
            false,
            "",
        );
        if let Ok(Some(secret)) = crate::accounts::donation_secret(None, false, &stop) {
            active = serde_json::from_str::<Connection>(&secret)
                .ok()
                .filter(|c| c.validate().is_ok());
        }
        if active.is_none() {
            status(
                &state,
                "Unlock your desktop wallet or reconnect your donation service.",
                false,
                "",
            );
        }
    }
    let mut socket: Option<Socket> = None;
    let mut next = Instant::now();
    let mut attempts = 0u32;
    let mut last = Instant::now();
    let mut ping = Instant::now();
    let mut interval = Duration::from_secs(25);
    let mut timeout = Duration::from_secs(70);
    let mut ready = false;
    while !stop.load(Ordering::Relaxed) {
        let command = if socket.is_none() {
            commands.recv_timeout(Duration::from_millis(100)).ok()
        } else {
            commands.try_recv().ok()
        };
        if let Some(command) = command {
            socket = None;
            ready = false;
            match command {
                Command::Disconnect => {
                    active = None;
                    let _ = std::fs::remove_file(marker());
                    let result = crate::accounts::donation_secret(None, true, &stop);
                    status(
                        &state,
                        if result.is_ok() {
                            "Disconnected"
                        } else {
                            "Disconnected. The desktop wallet could not remove the saved token; unlock it and click Disconnect again."
                        },
                        false,
                        "",
                    );
                }
                Command::Connect(c, remember) => {
                    if c.validate().is_err() {
                        continue;
                    }
                    let _ = std::fs::remove_file(marker());
                    if remember {
                        status(&state, "Saving event token in desktop wallet…", false, "");
                        let saved = serde_json::to_string(&c).ok().and_then(|v| {
                            crate::accounts::donation_secret(Some(&v), false, &stop).ok()
                        });
                        if saved.is_none() {
                            active = None;
                            status(
                                &state,
                                "Could not save to desktop wallet. Unlock it or turn off Remember on this computer and reconnect.",
                                false,
                                "",
                            );
                            continue;
                        }
                        if let Some(parent) = marker().parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        if std::fs::write(marker(), b"1").is_err() {
                            active = None;
                            status(&state, "Could not save connection settings.", false, "");
                            continue;
                        }
                    }
                    active = Some(c);
                    next = Instant::now();
                    attempts = 0;
                }
            }
        }
        let Some(c) = active.as_ref() else {
            continue;
        };
        if socket.is_none() {
            if Instant::now() < next {
                continue;
            }
            status(
                &state,
                &format!("Connecting to {}…", c.provider.name()),
                false,
                &c.owner(),
            );
            match connect(c) {
                Ok(s) => {
                    socket = Some(s);
                    last = Instant::now();
                    ping = Instant::now();
                    ready = false;
                    timeout = Duration::from_secs(15);
                }
                Err(_) => {
                    attempts = attempts.saturating_add(1);
                    let delay = (2u64.pow(attempts.min(5))).min(30);
                    next = Instant::now() + Duration::from_secs(delay);
                    status(
                        &state,
                        &format!(
                            "Connection unavailable. Check the event token and internet connection. Retrying in {delay}s."
                        ),
                        false,
                        &c.owner(),
                    );
                    continue;
                }
            }
        }
        let result = (|| -> Result<()> {
            ensure!(last.elapsed() < timeout, "Donation connection timed out");
            let s = socket.as_mut().unwrap();
            if c.provider == Provider::Streamlabs && ready && ping.elapsed() >= interval {
                s.send(Message::Text("2".into()))?;
                ping = Instant::now();
            }
            let msg = match s.read() {
                Ok(m) => m,
                Err(tungstenite::Error::Io(e))
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) =>
                {
                    return Ok(());
                }
                Err(_) => anyhow::bail!("Donation connection interrupted"),
            };
            last = Instant::now();
            let mut parsed = None;
            match msg {
                Message::Text(text) => {
                    if c.provider == Provider::StreamElements {
                        let v: Value = serde_json::from_str(&text)?;
                        match v["type"].as_str(){
                            Some("welcome")=>s.send(Message::Text(json!({"type":"subscribe","nonce":"donations","data":{"topic":"channel.tips","room":c.room,"token":c.token,"token_type":"jwt"}}).to_string().into()))?,
                            Some("response")=>{ensure!(v.get("error").is_none(),"Event token rejected or subscription unavailable");if v["nonce"]=="donations" && v["data"]["topic"]=="channel.tips" {ready=true;timeout=Duration::from_secs(75);}},
                            Some("reconnect")=>anyhow::bail!("Donation service is reconnecting"),
                            Some("message") if ready=>parsed=Some(v),_=>()
                        }
                    } else if let Some(data) = text.strip_prefix('0') {
                        let v: Value = serde_json::from_str(data)?;
                        interval = Duration::from_millis(
                            v["pingInterval"]
                                .as_u64()
                                .unwrap_or(25000)
                                .clamp(1000, 60000),
                        );
                        timeout = interval
                            + Duration::from_millis(
                                v["pingTimeout"]
                                    .as_u64()
                                    .unwrap_or(60000)
                                    .clamp(1000, 120000),
                            );
                    } else if text == "40" {
                        ready = true;
                    } else if text.starts_with("44") || text.starts_with("41") {
                        anyhow::bail!("Event token rejected or server disconnected");
                    } else if let Some(data) = text.strip_prefix("42") {
                        let v: Value = serde_json::from_str(data)?;
                        if ready && v[0] == "event" {
                            parsed = Some(v[1].clone());
                        }
                    }
                }
                Message::Ping(_) => {
                    s.flush()?;
                }
                Message::Close(_) => anyhow::bail!("Donation connection closed"),
                _ => (),
            }
            if ready {
                attempts = 0;
                status(
                    &state,
                    &format!(
                        "Connected to {} · Waiting for live donations",
                        c.provider.name()
                    ),
                    true,
                    &c.owner(),
                );
            }
            if let Some(v) = parsed {
                for donation in parse(c, &v) {
                    events.try_send(donation).map_err(|_|anyhow::anyhow!("Donation queue is full. Some events may be missing; check your provider history."))?;
                }
            }
            Ok(())
        })();
        if let Err(e) = result {
            socket = None;
            ready = false;
            attempts = attempts.saturating_add(1);
            let delay = (2u64.pow(attempts.min(5))).min(30);
            next = Instant::now() + Duration::from_secs(delay);
            // Only fixed error descriptions: never expose URLs, credentials, or received payloads.
            let description = if e.to_string().contains("queue is full") {
                "Donation queue is full; check provider history for missing events."
            } else {
                "Connection interrupted or token rejected."
            };
            status(
                &state,
                &format!("{description} Reconnecting in {delay}s."),
                false,
                &c.owner(),
            );
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_money() {
        assert_eq!(amount(&json!("13.37")), Some(1337));
        assert_eq!(amount(&json!(4.2)), Some(420));
        for bad in [
            json!("-1"),
            json!("NaN"),
            json!("1.001"),
            json!(0),
            json!("18446744073709551615"),
        ] {
            assert_eq!(amount(&bad), None);
        }
    }
    #[test]
    fn completed_approved_events_only() {
        let c = Connection {
            provider: Provider::StreamElements,
            room: "123".into(),
            token: String::new(),
        };
        let mut v = json!({"type":"message","topic":"channel.tips","room":"123","data":{"_id":"tip1","status":"success","approved":"allowed","donation":{"amount":5,"currency":"USD","user":{"username":"Viewer","email":"private@example.com"},"message":"hello"}}});
        assert_eq!(parse(&c, &v)[0].hundredths, 500);
        v["data"]["status"] = "pending".into();
        assert!(parse(&c, &v).is_empty());
        v["data"]["status"] = "success".into();
        v["data"]["approved"] = "blocked".into();
        assert!(parse(&c, &v).is_empty());
    }
    #[test]
    fn streamlabs_batch_privacy_and_tests() {
        let c = Connection {
            provider: Provider::Streamlabs,
            room: String::new(),
            token: String::new(),
        };
        let v = json!({"type":"donation","message":[{"id":1,"name":"Viewer","amount":"13.37","currency":"USD","message":"private"},{"id":2,"amount":"2","currency":"EUR","isTest":true}]});
        let d = parse(&c, &v);
        assert_eq!(d.len(), 1);
        let mut setup = Setup::default();
        setup.show_name = false;
        setup.show_amount = false;
        setup.template = "{user} {amount} {currency} {message}".into();
        let a = d[0].activity(&setup);
        assert!(!a.text.contains("private"));
        assert!(!a.text.contains("13.37"));
        assert_eq!(a.author, "Anonymous");
    }
}

#[cfg(test)]
mod delivery_tests {
    use super::*;
    use auroracast_core::{
        Id,
        widgets::{Kind, Settings},
    };
    #[test]
    fn persisted_history_deduplicates_before_currency_goal_updates() {
        let path = std::env::temp_dir().join(format!(
            "rosorah-donation-dedup-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let hub = crate::widgets::Hub::default();
        for (id, currency) in [(1 as Id, "USD"), (2 as Id, "EUR")] {
            hub.0.lock().unwrap().insert(
                id,
                Arc::new(Mutex::new(crate::widgets::State::new(Settings {
                    kind: Kind::DonationGoal,
                    currency: currency.into(),
                    ..Default::default()
                }))),
            );
        }
        let c = Connection {
            provider: Provider::Streamlabs,
            room: String::new(),
            token: String::new(),
        };
        let d=parse(&c,&json!({"type":"donation","message":[{"id":555,"name":"Viewer","amount":"12.34","currency":"USD"}]})).remove(0);
        let item = d.activity(&Setup::default());
        {
            let mut history = crate::activity::History::load(path.clone());
            for _ in 0..2 {
                if history.push(item.clone()) {
                    hub.donation(&d.currency, d.hundredths);
                }
            }
            assert_eq!(hub.0.lock().unwrap()[&1].lock().unwrap().value, 1234);
            assert_eq!(hub.0.lock().unwrap()[&2].lock().unwrap().value, 0);
        }
        let mut reopened = crate::activity::History::load(path.clone());
        assert!(!reopened.push(item));
        drop(reopened);
        let _ = std::fs::remove_file(path);
    }
}
