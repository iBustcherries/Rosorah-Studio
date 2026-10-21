//! Experimental Steam launch observer; the capture receiver remains independent.
//! Off until turned on in Capture details (or `ROSORAH_LAUNCH_WATCH=1`): attaching to Steam's
//! launches stalled Proton games. Permission prompts come only from the explicit in-app button.
use std::{
    fs,
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, OnceLock, mpsc},
    thread,
    time::{Duration, Instant},
};

#[derive(Default)]
struct State {
    message: String,
    blocked: bool,
    elevated: bool,
    requesting: bool,
    generation: u64,
    /// The observer has been turned on this session.
    on: bool,
}
enum Request {
    Start,
    Elevate,
    Stop,
}
struct Control {
    state: Arc<Mutex<State>>,
    sender: mpsc::Sender<Request>,
}
static CONTROL: OnceLock<Control> = OnceLock::new();

pub fn status() -> String {
    CONTROL
        .get()
        .map(|c| c.state.lock().unwrap().message.clone())
        .unwrap_or_else(|| "OpenGL launch activation has not started.".into())
}
/// Whether the observer was turned on this session.
pub fn active() -> bool {
    CONTROL.get().is_some_and(|c| c.state.lock().unwrap().on)
}
/// Turns the observer on for this session.
pub fn enable() {
    if let Some(c) = CONTROL.get() {
        let _ = c.sender.send(Request::Start);
    }
}
pub fn needs_permission() -> bool {
    CONTROL.get().is_some_and(|c| {
        let state = c.state.lock().unwrap();
        state.blocked && !state.elevated && !state.requesting
    })
}
pub fn authorize() {
    if let Some(c) = CONTROL.get() {
        let mut state = c.state.lock().unwrap();
        if state.requesting || state.elevated {
            return;
        }
        state.requesting = true;
        state.message = "Waiting for permission to watch Steam launches…".into();
        let _ = c.sender.send(Request::Elevate);
    }
}
fn start_time(stat: &str) -> Option<u64> {
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(19)?
        .parse()
        .ok()
}
fn hook_path() -> Option<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(data.join("rosorah-studio/renderer-0.8.0/$LIB/librosorah-gl.so"))
}
fn message(state: &Arc<Mutex<State>>, text: &str) {
    state.lock().unwrap().message = text.into();
    crate::capture_log::event(text.to_owned());
}
fn receive(line: &str, state: &Arc<Mutex<State>>) {
    let fields: Vec<_> = line.split_whitespace().collect();
    if fields.len() != 3 || fields[1].parse::<u32>().is_err() || fields[2].parse::<u32>().is_err() {
        return;
    }
    let text = match fields[0] {
        "STARTED" => "OpenGL activation is waiting for Steam (experimental).",
        "WATCHING" if state.lock().unwrap().blocked => {
            "Some Steam launches are blocked by permissions. Vulkan capture remains available."
        }
        "WATCHING" => {
            "Watching new Steam games for OpenGL capture. Steam can stay open (experimental)."
        }
        "BLOCKED" => {
            let mut s = state.lock().unwrap();
            s.blocked = true;
            if s.elevated {
                "Fedora still blocks launch attachment. Vulkan capture remains available."
            } else {
                "OpenGL launch activation needs permission. Vulkan capture remains available."
            }
        }
        "INJECTED" => "OpenGL hook prepared for a new Steam process; waiting for rendered frames.",
        "HOOK_UNAVAILABLE" => {
            "The game's runtime cannot access the OpenGL hook. Vulkan capture remains available."
        }
        "SKIPPED" => "Skipped an unsupported game launch; the original launch was preserved.",
        "DETACH_RETRY" => "Finishing launch attachment cleanup…",
        "FAILED" => "OpenGL launch activation failed. Vulkan capture remains available.",
        "STOPPED" => return,
        _ => return,
    };
    message(state, text);
    crate::capture_log::event(format!(
        "Launch observer: {} PID={} detail={}",
        fields[0], fields[1], fields[2]
    ));
}
fn spawn(elevated: bool, state: &Arc<Mutex<State>>) -> Option<Child> {
    let generation = {
        let mut s = state.lock().unwrap();
        s.generation += 1;
        s.on = true;
        s.elevated = elevated;
        s.blocked = false;
        s.requesting = false;
        s.generation
    };
    let result = (|| -> anyhow::Result<Child> {
        let executable = std::env::current_exe()?;
        let helper = executable
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Missing executable directory"))?
            .join("rosorah-launch-watch");
        anyhow::ensure!(
            helper.is_file(),
            "Extract the complete Rosorah package; the launch helper is missing"
        );
        let stamp = start_time(&fs::read_to_string("/proc/self/stat")?)
            .ok_or_else(|| anyhow::anyhow!("Cannot identify the studio process"))?;
        let hook = hook_path()
            .ok_or_else(|| anyhow::anyhow!("Cannot locate the installed capture hook"))?;
        let mut command = if elevated {
            let mut command = Command::new("pkexec");
            command.arg(&helper);
            command
        } else {
            Command::new(&helper)
        };
        command
            .args([
                "--owner-pid",
                &std::process::id().to_string(),
                "--owner-start",
                &stamp.to_string(),
                "--hook",
            ])
            .arg(hook)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("Missing helper output"))?;
        let state = state.clone();
        thread::spawn(move || {
            // The helper emits only fixed event codes and numeric process ids.
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if line.len() <= 256 && state.lock().unwrap().generation == generation {
                    receive(&line, &state);
                }
            }
        });
        Ok(child)
    })();
    match result {
        Ok(child) => Some(child),
        Err(error) => {
            let mut s = state.lock().unwrap();
            s.requesting = false;
            s.elevated = false;
            s.message = format!(
                "OpenGL activation unavailable: {error}. Vulkan capture remains available."
            );
            crate::capture_log::event(s.message.clone());
            None
        }
    }
}
fn stop(child: &mut Option<Child>) -> bool {
    if let Some(mut running) = child.take() {
        drop(running.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            match running.try_wait() {
                Ok(Some(_)) => return true,
                Err(_) => return false,
                _ if Instant::now() >= deadline => return false,
                _ => thread::sleep(Duration::from_millis(20)),
            }
        }
    }
    true
}
pub struct Guard {
    thread: Option<thread::JoinHandle<()>>,
}
impl Guard {
    pub fn start() -> Self {
        let (sender, receiver) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            message: "OpenGL launch activation is off. Vulkan games, including Proton games, are captured without it.".into(),
            ..State::default()
        }));
        if CONTROL
            .set(Control {
                state: state.clone(),
                sender,
            })
            .is_err()
        {
            return Self { thread: None };
        }
        if std::env::var_os("ROSORAH_DISABLE_LAUNCH_WATCH").is_some() {
            message(
                &state,
                "OpenGL launch observer disabled. Existing capture hooks remain available.",
            );
            return Self { thread: None };
        }
        let handle = thread::spawn(move || {
            let mut child = std::env::var_os("ROSORAH_LAUNCH_WATCH").and_then(|_| spawn(false, &state));
            loop {
                match receiver.recv_timeout(Duration::from_millis(250)) {
                    Ok(Request::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                        stop(&mut child);
                        break;
                    }
                    Ok(Request::Start) => {
                        if child.is_none() {
                            child = spawn(false, &state);
                        }
                    }
                    Ok(Request::Elevate) => {
                        if stop(&mut child) {
                            child = spawn(true, &state);
                        } else {
                            message(
                                &state,
                                "The previous launch observer has not stopped. Close and reopen Rosorah to retry.",
                            );
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
                if child
                    .as_mut()
                    .is_some_and(|c| matches!(c.try_wait(), Ok(Some(_))))
                {
                    child = None;
                    let mut s = state.lock().unwrap();
                    s.generation += 1;
                    s.requesting = false;
                    s.elevated = false;
                    s.blocked = true;
                    s.message = "OpenGL activation stopped or permission was declined. Vulkan capture remains available.".into();
                }
            }
        });
        Self {
            thread: Some(handle),
        }
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(c) = CONTROL.get() {
            let _ = c.sender.send(Request::Stop);
        }
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn process_identity_handles_spaces_and_parentheses() {
        let mut fields = vec!["S".to_owned()];
        fields.extend((4..=21).map(|_| "0".to_owned()));
        fields.push("987654321".to_owned());
        assert_eq!(
            start_time(&format!("123 (studio (test)) {}", fields.join(" "))),
            Some(987654321)
        );
        assert_eq!(start_time("bad stat"), None);
    }
    #[test]
    fn only_fixed_numeric_helper_events_are_accepted() {
        let state = Arc::new(Mutex::new(State::default()));
        receive("INJECTED 123 64", &state);
        assert!(state.lock().unwrap().message.contains("prepared"));
        receive("BLOCKED 123 1", &state);
        assert!(state.lock().unwrap().blocked);
        receive("WATCHING 234 0", &state);
        assert!(state.lock().unwrap().message.contains("blocked"));
        let before = state.lock().unwrap().message.clone();
        receive("INJECTED bad 64", &state);
        receive("UNKNOWN 123 0", &state);
        assert_eq!(state.lock().unwrap().message, before);
    }
}
