//! Spout on Linux. Spout is Windows apps sharing DirectX pictures, so on Linux it only exists
//! between Windows apps running in Wine or Proton. Rosorah finds those sessions in /proc and runs
//! its small Windows helper (rosorah-spout-bridge.exe) inside one, with the app's own Wine and
//! environment, so it joins the same Wine session; the helper sends the pictures out on stdout.
use anyhow::{Context, Result, bail};
use std::{
    ffi::OsString,
    io::Read,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};

/// One Wine prefix with a Windows app running in it.
#[derive(Clone)]
struct Session {
    prefix: String,
    loader: PathBuf,
    env: Vec<(OsString, OsString)>,
}
/// Wine's own background programs, never the app someone wants pictures from.
const WINE_SERVICES: [&str; 15] = [
    "services.exe", "winedevice.exe", "plugplay.exe", "explorer.exe", "rpcss.exe", "svchost.exe", "tabtip.exe", "steam.exe", "conhost.exe", "start.exe",
    "rundll32.exe", "wineboot.exe", "xalia.exe", "winemenubuilder.exe", "steamwebhelper.exe",
];
/// Running Wine/Proton sessions that have an app open, one per prefix.
fn sessions() -> Vec<Session> {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut found: Vec<Session> = Vec::new();
    for entry in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
        let dir = entry.path();
        let Ok(exe) = std::fs::read_link(dir.join("exe")) else { continue };
        let name = exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if !name.starts_with("wine") || name.contains("server") {
            continue;
        }
        let Ok(cmdline) = std::fs::read(dir.join("cmdline")) else { continue };
        let program = String::from_utf8_lossy(cmdline.split(|b| *b == 0).next().unwrap_or_default()).to_lowercase();
        let program = program.rsplit(['\\', '/']).next().unwrap_or_default().to_owned();
        if !program.ends_with(".exe") || WINE_SERVICES.contains(&program.as_str()) {
            continue;
        }
        let Ok(environ) = std::fs::read(dir.join("environ")) else { continue };
        let mut env: Vec<(OsString, OsString)> = Vec::new();
        for pair in environ.split(|b| *b == 0) {
            let pair = String::from_utf8_lossy(pair);
            // The server socket is an inherited handle that only works in Wine's own children.
            if let Some((k, v)) = pair.split_once('=')
                && !matches!(k, "WINESERVERSOCKET" | "WINELOADERNOEXEC")
            {
                env.push((k.into(), v.into()));
            }
        }
        let var = |key: &str| env.iter().find(|(k, _)| k == key).map(|(_, v)| v.to_string_lossy().into_owned());
        let prefix = var("WINEPREFIX").unwrap_or_else(|| format!("{home}/.wine"));
        let prefix = prefix.trim_end_matches('/').to_owned();
        if found.iter().any(|s| s.prefix == prefix) {
            continue;
        }
        // The same Wine build as the app's: a different version can't join its session.
        let beside = exe.parent().map(|d| d.join("wine")).filter(|p| p.is_file());
        let Some(loader) = var("WINELOADER").map(PathBuf::from).filter(|p| p.is_file()).or(beside) else { continue };
        found.push(Session { prefix, loader, env });
    }
    found
}
/// The helper shipped beside Rosorah (ROSORAH_SPOUT_BRIDGE overrides it, for testing).
fn helper() -> Result<PathBuf> {
    let path = std::env::var_os("ROSORAH_SPOUT_BRIDGE").map(PathBuf::from).or_else(|| std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("rosorah-spout-bridge.exe"))));
    path.filter(|p| p.is_file()).context("This Rosorah build has no Spout bridge (rosorah-spout-bridge.exe)")
}
fn command(session: &Session, args: &[&str]) -> Result<Command> {
    let mut command = Command::new(&session.loader);
    command.env_clear().envs(session.env.iter().map(|(k, v)| (k, v))).arg(helper()?).args(args);
    Ok(command)
}
/// Kills the helper when the source stops.
struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
/// Runs `--list` in one session; a Wine that hangs is given up on.
fn list(session: &Session) -> Vec<String> {
    let Ok(mut command) = command(session, &["--list"]) else { return Vec::new() };
    let Ok(child) = command.stdout(Stdio::piped()).stderr(Stdio::null()).spawn() else { return Vec::new() };
    let mut child = Running(child);
    let mut out = String::new();
    let mut stdout = child.0.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = stdout.read_to_string(&mut out);
        let _ = tx.send(out);
    });
    rx.recv_timeout(Duration::from_secs(20)).unwrap_or_default().lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect()
}
/// Spout senders in every running Wine/Proton session, with the session each is in.
fn find() -> Vec<(String, Session)> {
    sessions().into_iter().flat_map(|s| list(&s).into_iter().map(move |name| (name, s.clone()))).collect()
}

static CACHE: Mutex<Option<(Instant, Vec<String>)>> = Mutex::new(None);
static LOOKING: Mutex<bool> = Mutex::new(false);
/// Senders last found; looking again happens in the background (starting Wine takes a moment).
pub fn senders() -> Result<Vec<String>> {
    helper()?;
    let fresh = CACHE.lock().unwrap().as_ref().is_some_and(|(at, _)| at.elapsed() < Duration::from_secs(3));
    let mut looking = LOOKING.lock().unwrap();
    if !fresh && !*looking {
        *looking = true;
        std::thread::spawn(|| {
            let names = find().into_iter().map(|(n, _)| n).collect();
            *CACHE.lock().unwrap() = Some((Instant::now(), names));
            *LOOKING.lock().unwrap() = false;
        });
    }
    Ok(CACHE.lock().unwrap().as_ref().map(|(_, n)| n.clone()).unwrap_or_default())
}

/// Rosorah's picture as a Spout sender in every running Wine/Proton session, for Windows apps there.
pub struct Sender(Vec<(Running, std::process::ChildStdin)>);
impl Sender {
    pub fn new(name: &str) -> Result<Self> {
        let sessions = sessions();
        anyhow::ensure!(!sessions.is_empty(), "No Windows app is running in Wine or Proton to share with. Start the app first, then turn on Spout output");
        let mut bridges = Vec::new();
        for session in &sessions {
            let mut child = Running(command(session, &["--serve", name])?.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().context("Couldn't start the Spout bridge in Wine")?);
            let input = child.0.stdin.take().unwrap();
            bridges.push((child, input));
        }
        Ok(Self(bridges))
    }
    /// Sends one RGBA picture, tightly packed, to every session still listening.
    pub fn send(&mut self, rgba: &[u8], width: u32, height: u32) -> Result<()> {
        use std::io::Write;
        self.0.retain_mut(|(_, input)| {
            input.write_all(b"RSPF").and_then(|_| input.write_all(&width.to_le_bytes())).and_then(|_| input.write_all(&height.to_le_bytes())).and_then(|_| input.write_all(rgba)).is_ok()
        });
        anyhow::ensure!(!self.0.is_empty(), "The Wine/Proton apps Spout output was shared with have closed");
        Ok(())
    }
}

/// A Spout sender's picture from a Windows app in Wine/Proton, as a source.
pub struct BridgeBackend(pub String);
impl rosorah_capture::Backend for BridgeBackend {
    fn run(self: Box<Self>, output: rosorah_capture::Producer, cancel: tokio::sync::watch::Receiver<bool>, _: rosorah_capture::Options) -> Result<()> {
        anyhow::ensure!(!self.0.is_empty(), "Choose a Spout sender in Layer Properties");
        output.waiting("Looking for Windows apps in Wine or Proton…");
        let Some((_, session)) = find().into_iter().find(|(n, _)| *n == self.0) else {
            bail!("“{}” isn't sending. Start the Windows app (in Wine or Proton) and turn on its Spout output", self.0)
        };
        let child = command(&session, &[&self.0])?.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().context("Couldn't start the Spout bridge in Wine")?;
        let mut child = Running(child);
        let mut stdout = child.0.stdout.take().unwrap();
        let mut stderr = child.0.stderr.take().unwrap();
        let problem = std::thread::spawn(move || {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text);
            text
        });
        let detail = format!("Spout (Wine) · {}", self.0);
        let mut header = [0u8; 12];
        while !*cancel.borrow() {
            if stdout.read_exact(&mut header).is_err() {
                let _ = child.0.kill();
                let text = problem.join().unwrap_or_default();
                let hint = if text.contains("Couldn't open") { " Wine needs DXVK in that prefix to share pictures (Proton, Lutris and Bottles include it)." } else { "" };
                bail!("The Spout bridge stopped: {}{hint}", text.trim());
            }
            anyhow::ensure!(&header[..4] == b"RSPF", "The Spout bridge sent something unexpected");
            let width = u32::from_le_bytes(header[4..8].try_into().unwrap());
            let height = u32::from_le_bytes(header[8..12].try_into().unwrap());
            if width == 0 || height == 0 {
                output.waiting(&format!("Waiting for “{}” to start sending", self.0));
                continue;
            }
            anyhow::ensure!(width <= 8192 && height <= 8192, "The Spout picture is too large");
            let mut rgba = vec![0u8; width as usize * height as usize * 4];
            stdout.read_exact(&mut rgba).context("The Spout bridge stopped mid-picture")?;
            output.publish_with_detail(rosorah_capture::Frame { width, height, rgba }, Some(&detail))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Needs a Windows app sending ROSORAH_SPOUT_TEST_SENDER in Wine/Proton (the bridge's own
    /// `--send` works) and ROSORAH_SPOUT_BRIDGE pointing at rosorah-spout-bridge.exe.
    #[test]
    #[ignore]
    fn real_spout_sender_in_wine_is_found_and_shown() -> Result<()> {
        let name = std::env::var("ROSORAH_SPOUT_TEST_SENDER").context("Set ROSORAH_SPOUT_TEST_SENDER")?;
        assert!(find().iter().any(|(n, _)| *n == name), "{name} not found in any Wine session");
        let mut engine = rosorah_capture::Engine::default();
        engine.start(1, Box::new(BridgeBackend(name)), rosorah_capture::Options::default())?;
        let deadline = Instant::now() + Duration::from_secs(40);
        let frame = loop {
            if let Some((_, Some(frame))) = engine.reader().latest().into_iter().find(|(id, _)| *id == 1) {
                break frame;
            }
            anyhow::ensure!(Instant::now() < deadline, "No Spout picture arrived: {:?}", engine.snapshot(1).map(|s| s.detail));
            std::thread::sleep(Duration::from_millis(100));
        };
        engine.stop(1);
        assert!(frame.width > 0 && frame.height > 0);
        assert_eq!(frame.rgba.len(), (frame.width * frame.height * 4) as usize);
        Ok(())
    }
}
#[cfg(test)]
mod output_tests {
    use super::*;
    /// Needs a running Wine/Proton app and ROSORAH_SPOUT_BRIDGE, like the test above.
    #[test]
    #[ignore]
    fn real_spout_output_reaches_wine_apps() -> Result<()> {
        let mut sender = Sender::new("Rosorah output test")?;
        let picture: Vec<u8> = [0, 200, 0, 255].repeat(320 * 180);
        let mut engine = rosorah_capture::Engine::default();
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut started = false;
        loop {
            sender.send(&picture, 320, 180)?;
            if !started && find().iter().any(|(n, _)| n == "Rosorah output test") {
                engine.start(1, Box::new(BridgeBackend("Rosorah output test".into())), rosorah_capture::Options::default())?;
                started = true;
            }
            if let Some((_, Some(frame))) = engine.reader().latest().into_iter().find(|(id, _)| *id == 1) {
                assert_eq!((frame.width, frame.height), (320, 180));
                assert_eq!(&frame.rgba[..4], &[0, 200, 0, 255]);
                break;
            }
            anyhow::ensure!(Instant::now() < deadline, "Rosorah's Spout output never came back from Wine (listed: {started})");
            std::thread::sleep(Duration::from_millis(33));
        }
        engine.stop(1);
        Ok(())
    }
}
