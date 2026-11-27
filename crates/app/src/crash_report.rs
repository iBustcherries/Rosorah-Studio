//! Crash, disconnect and bug reports. Each session keeps a log on disk; when the last session ended
//! without closing (a crash, a freeze or the PC losing power) or a stream disconnects, a report is
//! put aside, and sent to the developer through the report relay, which files it as an issue in a
//! private GitHub repository. Nothing is sent unless the person sends it or chose to send
//! automatically. Reports carry no stream keys, passwords, tokens or chat messages.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// The report relay (relay/ in the source). Builds may point elsewhere with ROSORAH_REPORT_URL.
const RELAY: &str = "https://rosorah-reports.justmoeislive.workers.dev/report";
/// Lines of the session log a report carries, newest last.
const LOG_LINES: usize = 400;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Crash,
    Disconnect,
    Bug,
}
impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::Crash => "crash",
            Kind::Disconnect => "disconnect",
            Kind::Bug => "bug",
        }
    }
}
/// A report waiting to be sent, kept as a file until it's sent or dismissed.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub id: String,
    pub kind: Kind,
    pub title: String,
    /// What happened, in plain words, for the prompt and the top of the issue.
    pub summary: String,
    /// Technical details: system, Windows' crash records, the session log.
    pub details: String,
    pub created: u64,
}

fn dir() -> PathBuf {
    auroracast_core::platform::config_root().join("rosorah-studio").join("logs")
}
fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}
/// "2026-11-25 19:22:05 UTC", without GStreamer or a time zone library, so it also works in the
/// panic hook and before the media runtime is found.
fn utc(secs: u64) -> String {
    let (days, rest) = (secs / 86_400, secs % 86_400);
    // Howard Hinnant's civil-from-days.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC", rest / 3600, rest % 3600 / 60, rest % 60)
}

struct Session {
    file: Option<std::fs::File>,
    started: u64,
}
static SESSION: OnceLock<Mutex<Session>> = OnceLock::new();
/// Panics in background threads that kept running; the studio offers a report for each.
static PANICS: Mutex<Vec<Report>> = Mutex::new(Vec::new());

/// First thing in main: rotates the log, notices an unfinished last session, marks this one as
/// running and catches panics.
pub fn start() {
    let dir = dir();
    let _ = std::fs::create_dir_all(dir.join("reports"));
    let log = dir.join("session.log");
    let previous_log = dir.join("previous-session.log");
    let marker = dir.join("running.json");
    let panic_file = dir.join("panic.txt");
    let unfinished: Option<Marker> = std::fs::read(&marker).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let _ = std::fs::rename(&log, &previous_log);
    let started = now();
    let file = std::fs::OpenOptions::new().create(true).append(true).open(&log).ok();
    let _ = SESSION.set(Mutex::new(Session { file, started }));
    log_line(&format!("Rosorah Studio {} started {} ({} {})", env!("CARGO_PKG_VERSION"), utc(started), std::env::consts::OS, std::env::consts::ARCH));
    let _ = std::fs::write(&marker, serde_json::to_vec(&Marker { started, version: env!("CARGO_PKG_VERSION").into(), pid: std::process::id() }).unwrap_or_default());
    if let Some(last) = unfinished {
        let panic = std::fs::read_to_string(&panic_file).ok();
        let _ = std::fs::remove_file(&panic_file);
        // Windows' event logs are slow to read; the studio opens meanwhile.
        std::thread::spawn(move || {
            if let Some(report) = unfinished_session(&last, panic, &previous_log) {
                let _ = save(&report);
            }
        });
    }
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current().name().unwrap_or("unnamed").to_owned();
        let text = format!("Panic in thread '{thread}': {info}\n\n{}", std::backtrace::Backtrace::force_capture());
        log_line(&text);
        let _ = std::fs::write(dir.join("panic.txt"), &text);
        if let Ok(mut panics) = PANICS.lock() {
            panics.push(new_report(Kind::Crash, format!("Panic in {thread}: {}", first_line(&info.to_string())), "Part of Rosorah stopped with an internal error.".into(), text.clone()));
        }
        default_hook(info);
    }));
}
#[derive(Serialize, Deserialize)]
struct Marker {
    started: u64,
    version: String,
    pid: u32,
}
/// Rosorah closed normally: no report next time.
pub fn clean_exit() {
    log_line("Closed normally");
    let _ = std::fs::remove_file(dir().join("running.json"));
    let _ = std::fs::remove_file(dir().join("panic.txt"));
}
/// Adds a line to this session's log (see `capture_log::event`).
pub fn log_line(text: &str) {
    let Some(session) = SESSION.get() else { return };
    let Ok(mut session) = session.lock() else { return };
    let elapsed = now().saturating_sub(session.started);
    let stamp = format!("[{:02}:{:02}:{:02}]", elapsed / 3600, elapsed % 3600 / 60, elapsed % 60);
    if let Some(file) = &mut session.file {
        let _ = writeln!(file, "{stamp} {}", scrub(text).replace('\n', "\n           "));
    }
}
fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or_default().chars().take(120).collect()
}
fn tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}
fn session_log() -> String {
    tail(&std::fs::read_to_string(dir().join("session.log")).unwrap_or_default(), LOG_LINES)
}
fn new_report(kind: Kind, title: String, summary: String, details: String) -> Report {
    let created = now();
    let mut random = [0u8; 4];
    let _ = getrandom::fill(&mut random);
    Report { id: format!("{created}-{:08x}", u32::from_le_bytes(random)), kind, title, summary, details, created }
}

/// What ended the last session, from the panic it left and Windows' records.
fn unfinished_session(last: &Marker, panic: Option<String>, log: &std::path::Path) -> Option<Report> {
    let log = tail(&std::fs::read_to_string(log).unwrap_or_default(), LOG_LINES);
    let since = Duration::from_secs(now().saturating_sub(last.started) + 60);
    let records = windows_records(since);
    let (title, summary) = if let Some(panic) = &panic {
        (format!("Crash: {}", first_line(panic)), "Rosorah stopped with an internal error last time.".to_owned())
    } else if records.app_crash {
        ("Crash: Rosorah closed unexpectedly".to_owned(), "Rosorah crashed last time (Windows recorded the crash).".to_owned())
    } else if records.power_loss {
        ("PC froze or restarted while Rosorah was open".to_owned(), "Your PC restarted without shutting down while Rosorah was open (a freeze, a power cut or holding the power button).".to_owned())
    } else if records.normal_shutdown {
        // Windows shut down or restarted with Rosorah open: not a problem.
        return None;
    } else {
        ("Rosorah closed unexpectedly".to_owned(), "Rosorah didn't close normally last time.".to_owned())
    };
    let details = format!(
        "Session started {} (version {})\n{}\n\n### Windows records since then\n{}\n\n### Panic\n{}\n\n### Log of that session\n{}",
        utc(last.started),
        last.version,
        system(),
        if records.text.is_empty() { "none" } else { &records.text },
        panic.as_deref().unwrap_or("none"),
        log
    );
    Some(new_report(Kind::Crash, title, summary, details))
}
#[derive(Default)]
struct Records {
    app_crash: bool,
    power_loss: bool,
    normal_shutdown: bool,
    text: String,
}
/// Crashes of Rosorah and its helpers, unexpected restarts, graphics driver resets and hardware
/// errors that Windows logged in the last `since`.
#[cfg(windows)]
fn windows_records(since: Duration) -> Records {
    let ms = since.as_millis();
    let query = |log: &str, filter: &str| -> String {
        let mut command = std::process::Command::new("wevtutil");
        command.args(["qe", log, &format!("/q:*[System[{filter} and TimeCreated[timediff(@SystemTime) <= {ms}]]]"), "/f:text", "/c:6", "/rd:true"]);
        std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000);
        command.output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
    };
    // Each event as "Date / Source / Event ID / Description", trimmed.
    let events = |text: String| -> Vec<String> {
        text.split("Event[")
            .skip(1)
            .map(|e| {
                let keep: Vec<&str> = e
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty() && !["Log Name:", "Task:", "Level:", "Opcode:", "Keyword:", "User:", "User Name:", "Computer:"].iter().any(|k| l.starts_with(k)))
                    .take(14)
                    .collect();
                keep.join("\n").chars().take(1200).collect()
            })
            .collect()
    };
    let mut records = Records::default();
    let crashes: Vec<String> = events(query("Application", "(EventID=1000 or EventID=1002)"))
        .into_iter()
        .filter(|e| {
            let e = e.to_lowercase();
            ["rosorah", "ffmpeg", "python", "gst-"].iter().any(|n| e.contains(n))
        })
        .collect();
    records.app_crash = crashes.iter().any(|e| e.to_lowercase().contains("rosorah-studio.exe"));
    let power = events(query("System", "Provider[@Name='Microsoft-Windows-Kernel-Power'] and EventID=41"));
    records.power_loss = !power.is_empty();
    let drivers = events(query("System", "Provider[@Name='Display'] and EventID=4101"));
    let hardware = events(query("System", "Provider[@Name='Microsoft-Windows-WHEA-Logger']"));
    records.normal_shutdown = !events(query("System", "Provider[@Name='EventLog'] and EventID=6006")).is_empty();
    for (name, list) in [("Program crashes", &crashes), ("Unexpected restarts", &power), ("Graphics driver resets", &drivers), ("Hardware errors", &hardware)] {
        if !list.is_empty() {
            records.text.push_str(&format!("**{name}**\n```\n{}\n```\n", list.join("\n---\n")));
        }
    }
    records
}
#[cfg(not(windows))]
fn windows_records(_: Duration) -> Records {
    Records::default()
}

/// System facts for a report: Rosorah, the OS, the processor and the graphics card (logged at start).
fn system() -> String {
    let mut text = format!("Rosorah Studio {} on {} {}", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH);
    #[cfg(windows)]
    {
        let run = |program: &str, args: &[&str]| -> String {
            let mut command = std::process::Command::new(program);
            command.args(args);
            std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000);
            command.output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
        };
        let value = |out: String| out.lines().find_map(|l| l.split_once("REG_SZ").map(|(_, v)| v.trim().to_owned())).unwrap_or_default();
        text.push_str(&format!("\nWindows: {}", run("cmd", &["/c", "ver"]).trim()));
        text.push_str(&format!("\nProcessor: {}", value(run("reg", &["query", r"HKLM\HARDWARE\DESCRIPTION\System\CentralProcessor\0", "/v", "ProcessorNameString"]))));
    }
    #[cfg(not(windows))]
    {
        let release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
        if let Some(name) = release.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=")) {
            text.push_str(&format!("\nSystem: {}", name.trim_matches('"')));
        }
        text.push_str(&format!("\nDesktop: {} ({})", std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(), std::env::var("XDG_SESSION_TYPE").unwrap_or_default()));
    }
    text
}

/// Takes out anything secret-looking: stream keys and addresses, tokens, passwords and the
/// account name in file paths.
pub fn scrub(text: &str) -> String {
    let mut clean = String::with_capacity(text.len());
    for word in text.split_inclusive(|c: char| c.is_whitespace() || c == '"' || c == '\'') {
        let (body, end) = word.split_at(word.trim_end_matches(|c: char| c.is_whitespace() || c == '"' || c == '\'').len());
        let lower = body.to_lowercase();
        let hidden = if lower.starts_with("rtmp://") || lower.starts_with("rtmps://") || lower.starts_with("srt://") {
            // Keep the server, never the path (it can hold the key).
            let host = body.splitn(4, '/').take(3).collect::<Vec<_>>().join("/");
            Some(format!("{host}/[hidden]"))
        } else if ["key=", "token=", "password=", "secret=", "access_token=", "refresh_token=", "authorization:", "bearer"].iter().any(|k| lower.contains(k)) {
            Some(match body.find(['=', ':']) {
                Some(at) => format!("{}[hidden]", &body[..=at]),
                None => "[hidden]".into(),
            })
        } else if lower.starts_with("live_") || lower.starts_with("sk_") || lower.starts_with("sk-") || looks_like_token(body) {
            Some("[hidden]".into())
        } else {
            None
        };
        clean.push_str(hidden.as_deref().unwrap_or(body));
        clean.push_str(end);
    }
    // The Windows/Linux account name in paths such as C:\Users\name\ or /home/name/.
    for (prefix, separator) in [(r"\Users\", '\\'), ("/Users/", '/'), ("/home/", '/')] {
        let mut out = String::with_capacity(clean.len());
        let mut rest = clean.as_str();
        while let Some(at) = rest.find(prefix) {
            out.push_str(&rest[..at + prefix.len()]);
            rest = &rest[at + prefix.len()..];
            let end = rest.find(separator).unwrap_or(rest.len()).min(rest.find(char::is_whitespace).unwrap_or(rest.len()));
            if end > 0 {
                out.push_str("[user]");
            }
            rest = &rest[end..];
        }
        out.push_str(rest);
        clean = out;
    }
    clean
}
/// A long run of letters and digits with both mixed in, like an access token or stream key.
fn looks_like_token(word: &str) -> bool {
    let word = word.trim_matches(|c: char| !c.is_ascii_alphanumeric());
    word.len() >= 28
        && word.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
        && word.chars().any(|c| c.is_ascii_digit())
        && word.chars().any(|c| c.is_ascii_alphabetic())
        && !word.contains("..")
}

fn save(report: &Report) -> Result<()> {
    let path = dir().join("reports").join(format!("{}.json", report.id));
    std::fs::write(path, serde_json::to_vec_pretty(report)?)?;
    Ok(())
}
fn waiting() -> Vec<Report> {
    let mut list: Vec<Report> = std::fs::read_dir(dir().join("reports"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok())
        .collect();
    list.sort_by_key(|r| r.created);
    list
}
fn forget(id: &str) {
    let _ = std::fs::remove_file(dir().join("reports").join(format!("{id}.json")));
}

/// A stream destination stopped sending: put a report aside with this session's log.
pub fn disconnect(destination: &str, detail: &str) {
    let (destination, detail) = (destination.to_owned(), detail.to_owned());
    // Off the studio's thread: looking up the system takes a moment on Windows.
    std::thread::spawn(move || {
        let report = new_report(
            Kind::Disconnect,
            format!("Stream to {destination} disconnected: {}", first_line(&detail)),
            format!("Your stream to {destination} disconnected."),
            format!("{}\n\n### What Rosorah said\n{}\n\n### Log of this session\n{}", system(), detail, session_log()),
        );
        let _ = save(&report);
    });
}
/// The text the developer gets, as GitHub Markdown.
fn issue_body(report: &Report, note: &str) -> String {
    let mut body = format!("**What happened:** {}\n\n", report.summary);
    if !note.trim().is_empty() {
        body.push_str(&format!("**From the person reporting:**\n> {}\n\n", note.trim().replace('\n', "\n> ")));
    }
    body.push_str(&format!("**Reported:** {} · Rosorah {}\n\n<details><summary>Technical details</summary>\n\n```\n{}\n```\n</details>\n", utc(report.created), env!("CARGO_PKG_VERSION"), report.details.replace("```", "'''")));
    // GitHub takes up to 65,536 characters; keep the end of the log, which matters most.
    let body = scrub(&body);
    if body.chars().count() > 60_000 {
        let skip = body.chars().count() - 60_000;
        format!("(start of the log cut)\n{}", body.chars().skip(skip).collect::<String>())
    } else {
        body
    }
}
/// Sends one report to the relay; returns the issue number it was filed as.
fn send(report: &Report, note: &str) -> Result<u64> {
    let url = std::env::var("ROSORAH_REPORT_URL").unwrap_or_else(|_| RELAY.into());
    let client = reqwest::blocking::Client::builder().connect_timeout(Duration::from_secs(8)).timeout(Duration::from_secs(30)).build()?;
    let response = client
        .post(url)
        .header("X-Rosorah-Version", env!("CARGO_PKG_VERSION"))
        .json(&serde_json::json!({
            "kind": report.kind.label(),
            "title": scrub(&report.title).chars().take(150).collect::<String>(),
            "body": issue_body(report, note),
            "version": env!("CARGO_PKG_VERSION"),
            "os": std::env::consts::OS,
        }))
        .send()
        .context("Couldn't reach the report service")?;
    let status = response.status();
    let value: serde_json::Value = response.json().unwrap_or_default();
    ensure!(status.is_success(), "The report service turned it down (HTTP {}{})", status.as_u16(), value["error"].as_str().map(|e| format!(", {e}")).unwrap_or_default());
    Ok(value["number"].as_u64().unwrap_or_default())
}

/// What the studio shows and does with reports.
#[derive(Clone, Debug, Default)]
pub enum Sending {
    #[default]
    Idle,
    Busy,
    Sent(u64),
    Failed(String),
}
#[derive(Default)]
pub struct Outbox {
    /// Reports waiting for a yes or no, oldest first.
    pub waiting: Vec<Report>,
    checked: Option<Instant>,
    /// The latest send from the bug report window or the prompt.
    pub sending: Arc<Mutex<Sending>>,
    /// Sent automatically in the background: how many went, for one quiet toast.
    pub sent_quietly: Arc<Mutex<Vec<Result<u64, String>>>>,
    /// After a failed automatic send (offline, service down), the next try waits.
    paused_until: Arc<Mutex<Option<Instant>>>,
}
impl Outbox {
    /// Looks for new reports every few seconds. With `automatic`, sends them straight away.
    pub fn tick(&mut self, automatic: bool) {
        for report in PANICS.lock().map(|mut p| std::mem::take(&mut *p)).unwrap_or_default() {
            let _ = save(&report);
        }
        if self.checked.is_some_and(|t| t.elapsed() < Duration::from_secs(3)) {
            return;
        }
        self.checked = Some(Instant::now());
        let mut found = waiting();
        // A burst (several destinations dropping at once) makes one prompt, not many.
        found.dedup_by(|b, a| a.kind == b.kind && b.created.saturating_sub(a.created) < 120 && {
            forget(&b.id);
            true
        });
        if automatic {
            if self.paused_until.lock().unwrap().is_some_and(|t| Instant::now() < t) {
                return;
            }
            for report in found {
                forget(&report.id);
                let (quiet, paused) = (self.sent_quietly.clone(), self.paused_until.clone());
                std::thread::spawn(move || {
                    let result = send(&report, "").map_err(|e| format!("{e:#}"));
                    if result.is_err() {
                        // Kept to try again in 15 minutes.
                        let _ = save(&report);
                        *paused.lock().unwrap() = Some(Instant::now() + Duration::from_secs(900));
                    }
                    if let Ok(mut q) = quiet.lock() {
                        q.push(result);
                    }
                });
            }
            self.waiting.clear();
        } else {
            self.waiting = found;
        }
    }
    /// Sends a report the person chose to send: from the prompt (`toast`: the result shows as a
    /// toast) or the bug report window (which shows `state`).
    pub fn send(&mut self, report: Report, note: String, toast: bool) {
        forget(&report.id);
        self.waiting.retain(|r| r.id != report.id);
        *self.sending.lock().unwrap() = Sending::Busy;
        let (sending, quiet) = (self.sending.clone(), self.sent_quietly.clone());
        std::thread::spawn(move || {
            let result = send(&report, &note).map_err(|e| format!("{e:#}"));
            if toast && let Ok(mut q) = quiet.lock() {
                q.push(result.clone());
            }
            *sending.lock().unwrap() = match result {
                Ok(number) => Sending::Sent(number),
                Err(e) => Sending::Failed(e),
            };
        });
    }
    pub fn dismiss(&mut self, id: &str) {
        forget(id);
        self.waiting.retain(|r| r.id != id);
    }
    pub fn state(&self) -> Sending {
        self.sending.lock().unwrap().clone()
    }
}
/// A bug report from Help > Report a bug, with this session's log when `details` is on.
pub fn bug(what: &str, contact: &str, extra: &str, details: bool) -> Report {
    let title = format!("Bug: {}", first_line(what));
    let mut summary = what.trim().to_owned();
    if !contact.trim().is_empty() {
        summary.push_str(&format!("\n\nContact: {}", contact.trim()));
    }
    let details = if details {
        format!("{}\n\n{}\n\n### Log of this session\n{}", system(), extra, session_log())
    } else {
        "Not included".into()
    };
    new_report(Kind::Bug, title, summary, details)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dates_are_utc() {
        assert_eq!(utc(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(utc(1_795_637_000), "2026-11-25 20:03:20 UTC");
        assert_eq!(utc(951_782_400), "2000-02-29 00:00:00 UTC");
    }
    #[test]
    fn secrets_are_scrubbed() {
        let text = r#"ffmpeg -i x rtmp://live.twitch.tv/app/live_123456_abcdefABCDEF stream_key=abc123 "access_token":"xyz" C:\Users\mdhal\AppData\Roaming /home/moe/.config Bearer abcdefghijklmnopqrstuvwxyz0123456789"#;
        let clean = scrub(text);
        for secret in ["live_123456", "abc123", "mdhal", "moe", "abcdefghijklmnopqrstuvwxyz0123456789"] {
            assert!(!clean.contains(secret), "{secret} left in {clean}");
        }
        assert!(clean.contains("rtmp://live.twitch.tv/[hidden]"), "{clean}");
        assert!(clean.contains(r"C:\Users\[user]\AppData"), "{clean}");
        // Ordinary words, versions and short numbers stay.
        assert_eq!(scrub("Capture 102 Live: League of Legends (TM) Client 0.57.0"), "Capture 102 Live: League of Legends (TM) Client 0.57.0");
    }
}
