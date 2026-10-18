//! Independent Ko-fi relay connection; StreamElements/Streamlabs remain untouched.
use crate::donations_live::Donation;
use anyhow::{Result, ensure};
use auroracast_core::donations::Provider;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};
#[derive(Clone, Serialize, Deserialize)]
struct Connection {
    version: u32,
    url: String,
    key: String,
}
impl Connection {
    fn decode(code: &str) -> Result<Self> {
        ensure!(code.len() <= 8192, "Connection code is too long");
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(code.trim().trim_end_matches('='))?;
        let c: Self = serde_json::from_slice(&bytes)?;
        c.validate()?;
        Ok(c)
    }
    fn validate(&self) -> Result<()> {
        let url = reqwest::Url::parse(&self.url)?;
        ensure!(
            (self.version == 1 || self.version == 2)
                && url.scheme() == "https"
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
                && url.path() == "/"
                && !self.url.ends_with('/'),
            "Use the HTTPS connection code supplied for Rosorah"
        );
        ensure!(
            (32..=128).contains(&self.key.len())
                && self
                    .key
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c)),
            "Invalid connection key"
        );
        Ok(())
    }
}
#[derive(Clone, Default)]
pub struct Status {
    pub text: String,
    pub owner: String,
    pub webhook: String,
    pub verified: bool,
    pub live: bool,
    pub connected: bool,
}
struct Batch {
    donations: Vec<Donation>,
    seq: u64,
}
pub struct Hub {
    pub code: String,
    pub service_url: String,
    pub verification: String,
    pub remember: bool,
    state: Arc<Mutex<Status>>,
    connection: Arc<Mutex<Option<Connection>>>,
    stop: Arc<AtomicBool>,
    rx: mpsc::Receiver<Batch>,
    ack: mpsc::Sender<u64>,
    delivered: u64,
}
fn marker() -> std::path::PathBuf {
    crate::streaming::Config::path().with_file_name("kofi-remembered")
}
fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}
fn request(
    client: &reqwest::blocking::Client,
    c: &Connection,
    path: &str,
    body: Option<Value>,
) -> Result<Value> {
    let url = format!("{}{path}", c.url);
    let req = if let Some(body) = body {
        client.post(url).json(&body)
    } else {
        client.get(url)
    };
    let response = req
        .bearer_auth(&c.key)
        .send()
        .map_err(|_| anyhow::anyhow!("Cannot reach the Ko-fi receiver securely"))?;
    ensure!(
        response.status().is_success(),
        "Receiver declined the request. Check the connection code or send a Ko-fi setup test first."
    );
    use std::io::Read;
    let mut bytes = Vec::new();
    response.take(512 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 512 * 1024, "Receiver response is too large");
    Ok(serde_json::from_slice(&bytes)?)
}
fn apply(state: &Mutex<Status>, v: &Value) -> Result<()> {
    let owner = v["owner"].as_str().unwrap_or("");
    ensure!(
        owner.starts_with("Ko-fi:") && owner.len() <= 100,
        "Invalid receiver account"
    );
    let verified = v["verified"] == true;
    let live = v["live"] == true;
    *state.lock().unwrap() = Status {
        owner: owner.into(),
        webhook: v["webhook_url"]
            .as_str()
            .unwrap_or("")
            .chars()
            .take(2048)
            .collect(),
        verified,
        live,
        connected: true,
        text: if live {
            "Ko-fi connected · Live donations enabled"
        } else if verified {
            "Ko-fi test received · Ready to enable live donations"
        } else {
            "Ko-fi receiver connected · Waiting for your setup test"
        }
        .into(),
    };
    Ok(())
}
fn parse_batch(v: &Value) -> Result<Batch> {
    let owner = v["owner"].as_str().unwrap_or("");
    ensure!(owner.starts_with("Ko-fi:"), "Invalid Ko-fi account");
    let rows = v["events"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Missing event list"))?;
    ensure!(rows.len() <= 64, "Too many events");
    let mut donations = Vec::new();
    let mut seq = 0;
    for e in rows {
        let n = e["seq"].as_u64().unwrap_or(0);
        ensure!(n > seq, "Invalid event sequence");
        seq = n;
        let amount = e["hundredths"].as_u64().unwrap_or(0);
        let currency = e["currency"].as_str().unwrap_or("");
        let id = e["id"].as_str().unwrap_or("");
        ensure!(
            amount > 0
                && amount <= 100_000_000_000
                && currency.len() == 3
                && currency.bytes().all(|c| c.is_ascii_uppercase())
                && !id.is_empty()
                && id.len() <= 128,
            "Invalid donation payload"
        );
        donations.push(Donation {
            historical: v["historical"] == true,
            at_ms: e["at_ms"].as_u64(),
            id: id.into(),
            owner: owner.into(),
            provider: Provider::Kofi,
            name: e["name"]
                .as_str()
                .unwrap_or("Anonymous")
                .chars()
                .take(100)
                .collect(),
            message: e["message"]
                .as_str()
                .unwrap_or("")
                .chars()
                .take(1000)
                .collect(),
            hundredths: amount,
            currency: currency.into(),
        });
    }
    Ok(Batch { donations, seq })
}
impl Hub {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::sync_channel(1);
        let (ack, acks) = mpsc::channel();
        let state = Arc::new(Mutex::new(Status {
            text: "Ko-fi not connected".into(),
            ..Default::default()
        }));
        let connection = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        if marker().exists() {
            let (s, c, cancel) = (state.clone(), connection.clone(), stop.clone());
            std::thread::spawn(move || {
                if let Ok(Some(secret)) = crate::accounts::kofi_secret(None, false, &cancel)
                    && let Ok(saved) = serde_json::from_str::<Connection>(&secret)
                    && saved.validate().is_ok()
                {
                    if cancel.load(Ordering::Relaxed) {
                        return;
                    }
                    *c.lock().unwrap() = Some(saved.clone());
                    run(saved, String::new(), false, tx, acks, s, cancel);
                } else {
                    s.lock().unwrap().text =
                        "Unlock your desktop wallet or reconnect Ko-fi.".into();
                }
            });
        }
        Self {
            code: String::new(),
            service_url: service_url(),
            verification: String::new(),
            remember: true,
            state,
            connection,
            stop,
            rx,
            ack,
            delivered: 0,
        }
    }
    pub fn status(&self) -> Status {
        self.state.lock().unwrap().clone()
    }
    pub fn connect(&mut self) {
        let decoded = if self.code.trim().is_empty() {
            simple_connection(&self.service_url, &self.verification)
        } else {
            Connection::decode(&self.code)
        };
        let c = match decoded {
            Ok(c) => c,
            Err(_) => {
                self.state.lock().unwrap().text="Check your Ko-fi verification token and the configured HTTPS receiver address.".into();
                return;
            }
        };
        self.stop.store(true, Ordering::Relaxed);
        self.stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(1);
        let (ack, acks) = mpsc::channel();
        self.rx = rx;
        self.ack = ack;
        self.delivered = 0;
        self.state = Arc::new(Mutex::new(Status {
            text: "Connecting to Ko-fi receiver…".into(),
            ..Default::default()
        }));
        self.connection = Arc::new(Mutex::new(Some(c.clone())));
        let _ = std::fs::remove_file(marker());
        let (s, stop) = (self.state.clone(), self.stop.clone());
        let verification = std::mem::take(&mut self.verification);
        let remember = self.remember;
        self.code.clear();
        std::thread::spawn(move || run(c, verification, remember, tx, acks, s, stop));
    }
    pub fn live(&self, live: bool) {
        let Some(c) = self.connection.lock().unwrap().clone() else {
            return;
        };
        let (s, stop) = (self.state.clone(), self.stop.clone());
        std::thread::spawn(move || {
            let result = client()
                .and_then(|client| request(&client, &c, "/v1/live", Some(json!({"live":live}))));
            if !stop.load(Ordering::Relaxed) {
                match result{Ok(v)=>{let _=apply(&s,&v);},Err(_)=>s.lock().unwrap().text="Could not change Ko-fi live mode. Check the receiver connection and setup test.".into()}
            }
        });
    }
    pub fn disconnect(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = std::fs::remove_file(marker());
        if let Some(c) = self.connection.lock().unwrap().take() {
            std::thread::spawn(move || {
                if let Ok(client) = client() {
                    let _ = request(&client, &c, "/v1/live", Some(json!({"live":false})));
                }
            });
        }
        std::thread::spawn(|| {
            let _ = crate::accounts::kofi_secret(None, true, &AtomicBool::new(false));
        });
        self.state=Arc::new(Mutex::new(Status{text:"Ko-fi disconnected. Remove the webhook in Ko-fi if you no longer want it forwarded.".into(),..Default::default()}));
        while self.rx.try_recv().is_ok() {}
        self.delivered = 0;
    }
    pub fn drain(&mut self) -> Vec<Donation> {
        if self.stop.load(Ordering::Relaxed) {
            return Vec::new();
        }
        if let Ok(batch) = self.rx.try_recv() {
            self.delivered = if self
                .connection
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|c| c.version == 2)
            {
                0
            } else {
                batch.seq
            };
            batch.donations
        } else {
            Vec::new()
        }
    }
    pub fn acknowledge(&mut self) {
        if self.delivered > 0 {
            let _ = self.ack.send(self.delivered);
            self.delivered = 0;
        }
    }
}
impl Drop for Hub {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
fn run(
    c: Connection,
    verification: String,
    remember: bool,
    tx: mpsc::SyncSender<Batch>,
    acks: mpsc::Receiver<u64>,
    state: Arc<Mutex<Status>>,
    stop: Arc<AtomicBool>,
) {
    let init = (|| -> Result<reqwest::blocking::Client> {
        let client = client()?;
        let value = request(
            &client,
            &c,
            if verification.trim().is_empty() {
                "/v1/status"
            } else {
                "/v1/configure"
            },
            if verification.trim().is_empty() {
                None
            } else {
                Some(json!({"verification_token":verification.trim()}))
            },
        )?;
        if stop.load(Ordering::Relaxed) {
            return Err(anyhow::anyhow!("Cancelled"));
        }
        if remember {
            crate::accounts::kofi_secret(Some(&serde_json::to_string(&c)?), false, &stop)?;
            if let Some(p) = marker().parent() {
                std::fs::create_dir_all(p)?;
            }
            std::fs::write(marker(), b"1")?;
        }
        apply(&state, &value)?;
        Ok(client)
    })();
    let client = match init {
        Ok(client) => client,
        Err(_) => {
            state.lock().unwrap().text="Ko-fi setup did not finish. Check the service connection, verification token, and desktop wallet; you can turn Remember off for this session.".into();
            return;
        }
    };
    if c.version == 2 {
        run_socket(&c, tx, state, stop);
        return;
    }
    let mut attempts = 0u32;
    while !stop.load(Ordering::Relaxed) {
        let result = (|| -> Result<()> {
            let value = request(&client, &c, "/v1/events", None)?;
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            apply(&state, &value)?;
            let batch = parse_batch(&value)?;
            let seq = batch.seq;
            if seq > 0 {
                tx.send(batch).map_err(|_| anyhow::anyhow!("App closed"))?;
                loop {
                    if stop.load(Ordering::Relaxed) {
                        return Ok(());
                    }
                    match acks.recv_timeout(Duration::from_millis(100)) {
                        Ok(ack) if ack >= seq => {
                            request(&client, &c, "/v1/ack", Some(json!({"seq":seq})))?;
                            break;
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            return Err(anyhow::anyhow!("App closed"));
                        }
                        _ => (),
                    }
                }
            }
            Ok(())
        })();
        if result.is_err() {
            attempts = attempts.saturating_add(1);
            let seconds = 2u64.pow(attempts.min(5)).min(30);
            if stop.load(Ordering::Relaxed) {
                break;
            }
            {
                let mut s = state.lock().unwrap();
                s.connected = false;
                s.text = format!("Ko-fi receiver disconnected. Retrying in {seconds}s.");
            }
            for _ in 0..seconds * 10 {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        } else {
            attempts = 0;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unsafe_connection_codes() {
        for url in [
            "http://relay.example",
            "https://name:secret@relay.example",
            "https://relay.example/path",
            "https://relay.example?key=x",
        ] {
            assert!(
                Connection {
                    version: 1,
                    url: url.into(),
                    key: "a".repeat(43)
                }
                .validate()
                .is_err()
            );
        }
    }
    #[test]
    fn simple_token_setup_and_missed_metadata() {
        let c = simple_connection("https://relay.example", "example-private-kofi-token").unwrap();
        assert_eq!(c.version, 2);
        assert_eq!(c.key.len(), 64);
        assert!(simple_connection("http://relay.example", "example-private-kofi-token").is_err());
        let v = json!({"owner":"Ko-fi:one","historical":true,"events":[{"seq":4,"id":"missed","hundredths":1200,"currency":"USD","at_ms":1234}]});
        let batch = parse_batch(&v).unwrap();
        assert!(batch.donations[0].historical);
        assert_eq!(batch.donations[0].at_ms, Some(1234));
    }
    #[test]
    fn normalizes_relay_events_without_losing_currency() {
        let v = json!({"owner":"Ko-fi:abc","events":[{"seq":8,"id":"payment1","name":"Viewer","message":"hi","hundredths":1200,"currency":"USD"}]});
        let b = parse_batch(&v).unwrap();
        assert_eq!(b.seq, 8);
        assert_eq!(b.donations[0].hundredths, 1200);
        assert_eq!(b.donations[0].provider, Provider::Kofi);
        let mut bad = v;
        bad["events"][0]["hundredths"] = (-1).into();
        assert!(parse_batch(&bad).is_err());
    }
}

fn service_url() -> String {
    let packaged = std::env::current_exe()
        .ok()
        .and_then(|p| std::fs::read(p.with_file_name("kofi-service.json")).ok())
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v["url"].as_str().map(str::to_owned));
    packaged
        .or_else(|| option_env!("ROSORAH_KOFI_SERVICE_URL").map(str::to_owned))
        .unwrap_or_default()
}
fn simple_connection(url: &str, token: &str) -> Result<Connection> {
    use sha2::{Digest, Sha256};
    let token = token.trim();
    ensure!(
        (16..=256).contains(&token.len()) && !token.chars().any(char::is_whitespace),
        "Invalid verification token"
    );
    let key = format!(
        "{:x}",
        Sha256::digest(format!("rosorah-kofi-v2:{token}").as_bytes())
    );
    let c = Connection {
        version: 2,
        url: url.trim().trim_end_matches('/').into(),
        key,
    };
    c.validate()?;
    Ok(c)
}
fn run_socket(
    c: &Connection,
    tx: mpsc::SyncSender<Batch>,
    state: Arc<Mutex<Status>>,
    stop: Arc<AtomicBool>,
) {
    use std::{
        net::{TcpStream, ToSocketAddrs},
        time::Instant,
    };
    use tungstenite::{Message, client::IntoClientRequest, stream::MaybeTlsStream};
    let mut attempts = 0u32;
    while !stop.load(Ordering::Relaxed) {
        let result = (|| -> Result<()> {
            let mut url = reqwest::Url::parse(&format!("{}/v2/socket", c.url))?;
            url.set_scheme("wss").map_err(|_| anyhow::anyhow!("URL"))?;
            let host = url.host_str().ok_or_else(|| anyhow::anyhow!("Host"))?;
            let tcp = (host, url.port_or_known_default().unwrap_or(443))
                .to_socket_addrs()?
                .take(3)
                .find_map(|a| TcpStream::connect_timeout(&a, Duration::from_secs(4)).ok())
                .ok_or_else(|| anyhow::anyhow!("Unavailable"))?;
            tcp.set_read_timeout(Some(Duration::from_secs(5)))?;
            tcp.set_write_timeout(Some(Duration::from_secs(3)))?;
            let mut req = url.as_str().into_client_request()?;
            req.headers_mut()
                .insert("Authorization", format!("Bearer {}", c.key).parse()?);
            let config = tungstenite::protocol::WebSocketConfig::default()
                .max_message_size(Some(512 * 1024))
                .max_frame_size(Some(512 * 1024));
            let (mut socket, _) =
                tungstenite::client_tls_with_config(req, tcp, Some(config), None)?;
            match socket.get_mut() {
                MaybeTlsStream::Rustls(s) => {
                    s.sock.set_read_timeout(Some(Duration::from_millis(100)))?
                }
                MaybeTlsStream::Plain(s) => s.set_read_timeout(Some(Duration::from_millis(100)))?,
                _ => anyhow::bail!("TLS"),
            }
            let mut ping = Instant::now();
            let mut last = Instant::now();
            attempts = 0;
            while !stop.load(Ordering::Relaxed) {
                if ping.elapsed() > Duration::from_secs(25) {
                    socket.send(Message::Ping(Vec::new().into()))?;
                    ping = Instant::now();
                }
                ensure!(last.elapsed() < Duration::from_secs(75), "Timed out");
                match socket.read() {
                    Ok(Message::Text(text)) => {
                        last = Instant::now();
                        let v: Value = serde_json::from_str(&text)?;
                        apply(&state, &v)?;
                        let batch = parse_batch(&v)?;
                        if !batch.donations.is_empty() {
                            tx.send(batch).map_err(|_| anyhow::anyhow!("Closed"))?;
                        }
                    }
                    Ok(Message::Pong(_)) => last = Instant::now(),
                    Ok(Message::Ping(_)) => {
                        last = Instant::now();
                        socket.flush()?;
                    }
                    Ok(Message::Close(_)) => anyhow::bail!("Closed"),
                    Err(tungstenite::Error::Io(e))
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                    {
                        ()
                    }
                    Err(_) => anyhow::bail!("Disconnected"),
                    _ => (),
                }
            }
            let _ = socket.close(None);
            Ok(())
        })();
        if stop.load(Ordering::Relaxed) {
            break;
        }
        if result.is_err() {
            attempts = attempts.saturating_add(1);
            let seconds = 2u64.pow(attempts.min(5)).min(30);
            {
                let mut s = state.lock().unwrap();
                s.connected = false;
                s.text = format!(
                    "Ko-fi disconnected · retrying in {seconds}s. Check that another Rosorah instance is not connected."
                );
            }
            for _ in 0..seconds * 10 {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}
