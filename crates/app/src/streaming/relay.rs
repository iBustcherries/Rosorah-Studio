#[cfg(unix)]
use std::os::fd::AsRawFd;
use super::{Location, Phase, Setup, State, Target};
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::VecDeque,
    io::{self, BufRead, BufReader, Read, Write},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[cfg(unix)] type RelayPipe = std::process::ChildStdin;
#[cfg(windows)] type RelayPipe = rosorah_windows::pipe::Writer;
const MAX_QUEUE: usize = 8 * 1024 * 1024;
const MAX_TAG: usize = 4 * 1024 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(10);
#[derive(Clone)]
struct Tag {
    data: Arc<Vec<u8>>,
    timestamp: u32,
    keyframe: bool,
}
#[derive(Default)]
struct Headers {
    file: Vec<u8>,
    metadata: Option<Tag>,
    video: Option<Tag>,
    audio: Option<Tag>,
}
impl Headers {
    fn bytes(&self) -> Result<Vec<u8>> {
        ensure!(
            !self.file.is_empty() && self.video.is_some() && self.audio.is_some(),
            "Encoder did not provide H.264/AAC headers"
        );
        let mut bytes = self.file.clone();
        for tag in [&self.metadata, &self.video, &self.audio]
            .into_iter()
            .flatten()
        {
            let mut data = (*tag.data).clone();
            data[4..8].fill(0);
            bytes.extend_from_slice(&data);
        }
        Ok(bytes)
    }
}
#[derive(Default)]
struct Queue {
    tags: VecDeque<Tag>,
    bytes: usize,
    overflow: bool,
    closed: bool,
    inactive: bool,
}
impl Queue {
    fn push(&mut self, tag: Tag) {
        if self.inactive {
            return;
        }
        if self.bytes + tag.data.len() > MAX_QUEUE || self.tags.len() >= 512 {
            self.tags.clear();
            self.bytes = 0;
            self.overflow = true;
        }
        self.bytes += tag.data.len();
        self.tags.push_back(tag);
    }
    fn pop(&mut self) -> Option<Tag> {
        let tag = self.tags.pop_front()?;
        self.bytes -= tag.data.len();
        Some(tag)
    }
    fn restart(&mut self) {
        self.tags.clear();
        self.bytes = 0;
        self.overflow = false;
    }
}
fn u24(b: &[u8]) -> u32 {
    ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32
}
fn read_tag(input: &mut impl Read) -> Result<Option<Tag>> {
    let mut head = [0u8; 11];
    let first = input.read(&mut head[..1])?;
    if first == 0 {
        return Ok(None);
    }
    input
        .read_exact(&mut head[1..])
        .context("Truncated FLV tag")?;
    let size = u24(&head[1..4]) as usize;
    ensure!(
        matches!(head[0], 8 | 9 | 18) && size <= MAX_TAG,
        "Invalid or oversized FLV tag"
    );
    let mut data = Vec::with_capacity(15 + size);
    data.extend_from_slice(&head);
    data.resize(15 + size, 0);
    input
        .read_exact(&mut data[11..])
        .context("Truncated encoded packet")?;
    ensure!(
        u32::from_be_bytes(data[11 + size..].try_into().unwrap()) as usize == size + 11,
        "Invalid FLV packet size"
    );
    let keyframe = head[0] == 9 && size >= 5 && data[11] == 0x17 && data[12] == 1;
    Ok(Some(Tag {
        timestamp: u24(&head[4..7]) | (u32::from(head[7]) << 24),
        data: Arc::new(data),
        keyframe,
    }))
}
/// The replay buffer: the last few seconds of the main video's encoded stream, kept in memory
/// (no extra encoding) so they can be saved to a file at any moment.
pub struct Replay {
    pub enabled: bool,
    pub seconds: u32,
    file: Vec<u8>,
    headers: Vec<Tag>,
    tags: VecDeque<Tag>,
    bytes: usize,
}
const REPLAY_MAX_BYTES: usize = 256 * 1024 * 1024;
pub static REPLAY: Mutex<Replay> = Mutex::new(Replay { enabled: false, seconds: 30, file: Vec::new(), headers: Vec::new(), tags: VecDeque::new(), bytes: 0 });
impl Replay {
    fn clear(&mut self) {
        self.file.clear();
        self.headers.clear();
        self.tags.clear();
        self.bytes = 0;
    }
    fn push(&mut self, tag: &Tag) {
        self.bytes += tag.data.len();
        self.tags.push_back(tag.clone());
        // Keep at least `seconds`, from the last keyframe that's that old, so a saved clip starts
        // cleanly; over the memory cap, drop whole keyframe groups from the front.
        let newest = tag.timestamp;
        let window = self.seconds.clamp(5, 120) * 1000;
        let start = self
            .tags
            .iter()
            .rposition(|t| t.keyframe && newest.saturating_sub(t.timestamp) >= window)
            .or_else(|| self.tags.iter().position(|t| t.keyframe))
            .unwrap_or(0);
        for _ in 0..start {
            let gone = self.tags.pop_front().unwrap();
            self.bytes -= gone.data.len();
        }
        while self.bytes > REPLAY_MAX_BYTES && self.tags.len() > 1 {
            let gone = self.tags.pop_front().unwrap();
            self.bytes -= gone.data.len();
            while self.tags.front().is_some_and(|t| !t.keyframe) && self.tags.len() > 1 {
                let gone = self.tags.pop_front().unwrap();
                self.bytes -= gone.data.len();
            }
        }
    }
    /// Seconds currently held.
    pub fn held(&self) -> u32 {
        match (self.tags.front(), self.tags.back()) {
            (Some(a), Some(b)) => b.timestamp.saturating_sub(a.timestamp) / 1000,
            _ => 0,
        }
    }
    /// The held seconds as a playable FLV file (timestamps starting at zero).
    pub fn flv(&self) -> Result<Vec<u8>> {
        ensure!(!self.file.is_empty() && !self.tags.is_empty(), "The replay buffer is empty; it fills while it's on");
        let start = self.tags.front().map_or(0, |t| t.timestamp);
        let mut out = self.file.clone();
        out.reserve(self.bytes + 1024);
        let stamped = |data: &[u8], ts: u32, out: &mut Vec<u8>| {
            let mut data = data.to_vec();
            data[4] = (ts >> 16) as u8;
            data[5] = (ts >> 8) as u8;
            data[6] = ts as u8;
            data[7] = (ts >> 24) as u8;
            out.extend_from_slice(&data);
        };
        for tag in &self.headers {
            stamped(&tag.data, 0, &mut out);
        }
        for tag in &self.tags {
            stamped(&tag.data, tag.timestamp - start, &mut out);
        }
        Ok(out)
    }
}
/// Writes the replay buffer to `path` (.mp4), repackaging without re-encoding.
pub fn save_replay(path: &std::path::Path) -> Result<()> {
    let flv = REPLAY.lock().unwrap().flv()?;
    let temporary = path.with_extension("flv");
    std::fs::write(&temporary, &flv).context("Couldn't write the replay")?;
    let status = Command::new("ffmpeg")
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(&temporary)
        .args(["-c", "copy", "-movflags", "+faststart"])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("Saving the replay needs FFmpeg")?;
    if status.success() {
        let _ = std::fs::remove_file(&temporary);
        Ok(())
    } else {
        // The FLV plays in most players, so it's kept when the repackaging fails.
        bail!("Saved as {} (couldn't repackage it as MP4)", temporary.display())
    }
}
fn dispatch(
    mut input: impl Read,
    headers: &Mutex<Headers>,
    queues: &[Arc<Mutex<Queue>>],
    replay: bool,
) -> Result<()> {
    let mut header = [0; 13];
    input.read_exact(&mut header)?;
    ensure!(
        &header[..3] == b"FLV" && header[3] == 1 && header[8] == 9 && header[5..8] == [0, 0, 0],
        "Invalid FLV encoder header"
    );
    headers.lock().unwrap().file = header.to_vec();
    if replay {
        let mut r = REPLAY.lock().unwrap();
        r.clear();
        r.file = header.to_vec();
    }
    while let Some(tag) = read_tag(&mut input)? {
        let d = &tag.data;
        let mut h = headers.lock().unwrap();
        let is_header = d[0] == 18 || (d[0] == 9 && d.len() >= 17 && d[11] & 15 == 7 && d[12] == 0) || (d[0] == 8 && d.len() >= 17 && d[11] >> 4 == 10 && d[12] == 0);
        if d[0] == 18 {
            h.metadata = Some(tag.clone());
        } else if d[0] == 9 && d.len() >= 17 && d[11] & 15 == 7 && d[12] == 0 {
            h.video = Some(tag.clone());
        } else if d[0] == 8 && d.len() >= 17 && d[11] >> 4 == 10 && d[12] == 0 {
            h.audio = Some(tag.clone());
        } else {
            drop(h);
            for q in queues {
                q.lock().unwrap().push(tag.clone());
            }
        }
        if replay {
            let mut r = REPLAY.lock().unwrap();
            if !r.enabled {
                r.tags.clear();
                r.bytes = 0;
            } else if is_header {
                r.headers.retain(|t| t.data[0] != d[0]);
                r.headers.push(tag.clone());
            } else {
                r.push(&tag);
            }
        }
    }
    Ok(())
}

pub struct Hub {
    reader: Option<JoinHandle<()>>,
    workers: Vec<JoinHandle<()>>,
    error: Arc<Mutex<Option<String>>>,
    state: Arc<State>,
}
impl Hub {
    pub fn start(input: impl Read + Send + 'static, setup: &Setup) -> Self {
        let replay = setup.replay;
        let headers = Arc::new(Mutex::new(Headers::default()));
        let queues: Vec<_> = setup
            .targets
            .iter()
            .map(|_| Arc::new(Mutex::new(Queue::default())))
            .collect();
        let mut workers = Vec::new();
        for (i, target) in setup.targets.iter().cloned().enumerate() {
            let (q, h, state) = (queues[i].clone(), headers.clone(), setup.state.clone());
            workers.push(thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_target(&target, i, &q, &h, &state)
                }));
                if result.is_err() {
                    set(
                        &state,
                        i,
                        Phase::Failed,
                        "Output worker stopped unexpectedly",
                    );
                }
                let mut q = q.lock().unwrap();
                q.inactive = true;
                q.restart();
            }));
        }
        let error = Arc::new(Mutex::new(None));
        let err = error.clone();
        let reader = thread::spawn(move || {
            if let Err(e) = dispatch(input, &headers, &queues, replay) {
                *err.lock().unwrap() = Some(format!("{e:#}"));
            }
            for q in &queues {
                q.lock().unwrap().closed = true;
            }
        });
        Self {
            reader: Some(reader),
            workers,
            error,
            state: setup.state.clone(),
        }
    }
    pub fn error(&self) -> Option<String> {
        self.error.lock().unwrap().clone()
    }
    pub fn finish(mut self) -> Result<()> {
        self.join();
        if let Some(e) = self.error() {
            bail!(e);
        }
        Ok(())
    }
    fn join(&mut self) {
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}
impl Drop for Hub {
    // On an early producer error its Encoder guard closes the pipe after us.
    // Do not join a reader that is still waiting for that EOF during unwinding.
    fn drop(&mut self) {
        self.state.stop();
    }
}
fn set(state: &State, index: usize, phase: Phase, detail: impl Into<String>) {
    let mut views = state.destinations.lock().unwrap();
    views[index].phase = phase;
    views[index].detail = detail.into();
}
struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
struct Progress {
    packets: AtomicU64,
    last: Mutex<Instant>,
}
fn run_target(
    target: &Target,
    index: usize,
    queue: &Mutex<Queue>,
    headers: &Mutex<Headers>,
    state: &Arc<State>,
) {
    let network = matches!(target.destination, Location::Network(_));
    let mut attempt = 0u32;
    loop {
        let ended = {
            let q = queue.lock().unwrap();
            q.closed && q.tags.is_empty()
        };
        if (network && state.stopping.load(Ordering::Relaxed)) || ended {
            break;
        }
        attempt += 1;
        state.destinations.lock().unwrap()[index].attempts = attempt;
        set(
            state,
            index,
            if attempt == 1 {
                Phase::Connecting
            } else {
                Phase::Reconnecting
            },
            "Waiting for a video keyframe",
        );
        let first = loop {
            let mut q = queue.lock().unwrap();
            if network && state.stopping.load(Ordering::Relaxed) {
                break None;
            }
            if q.overflow {
                q.restart();
            }
            if let Some(tag) = q.pop() {
                if tag.keyframe {
                    break Some(tag);
                }
            } else if q.closed {
                break None;
            }
            drop(q);
            thread::sleep(Duration::from_millis(3));
        };
        let Some(first) = first else {
            break;
        };
        let connected_at=Instant::now();
        let result = connection(target, index, queue, headers, state, first);
        if connected_at.elapsed()>Duration::from_secs(30){attempt=1;}
        match result {
            Ok(()) => {
                set(
                    state,
                    index,
                    Phase::Stopped,
                    if network {
                        "Stream stopped"
                    } else {
                        "Recording saved"
                    },
                );
                return;
            }
            Err(e) if !network => {
                set(
                    state,
                    index,
                    Phase::Failed,
                    format!("{e:#}; any partial file has been kept"),
                );
                return;
            }
            Err(e) => {
                if state.stopping.load(Ordering::Relaxed) || queue.lock().unwrap().closed {
                    break;
                }
                let seconds = (1u64 << attempt.min(4)).min(15);
                set(
                    state,
                    index,
                    Phase::Reconnecting,
                    format!("{} · retry in {seconds}s", target.redact(&format!("{e:#}"))),
                );
                let deadline = Instant::now() + Duration::from_secs(seconds);
                while Instant::now() < deadline {
                    if state.stopping.load(Ordering::Relaxed) || queue.lock().unwrap().closed || state.retry_now.lock().unwrap().remove(&index) {
                        break;
                    }
                    thread::sleep(Duration::from_millis(20));
                }
                queue.lock().unwrap().restart();
            }
        }
    }
    set(
        state,
        index,
        Phase::Stopped,
        if network {
            "Stream stopped"
        } else {
            "No recording frames received"
        },
    );
}
fn connection(
    target: &Target,
    index: usize,
    queue: &Mutex<Queue>,
    headers: &Mutex<Headers>,
    state: &Arc<State>,
    first: Tag,
) -> Result<()> {
    #[cfg(windows)] let mut pipe=RelayPipe::new(Arc::new(std::sync::atomic::AtomicBool::new(false)))?;
    #[cfg(windows)] let input=pipe.name().to_owned();
    #[cfg(unix)] let input=String::from("pipe:0");
    let mut cmd = Command::new("ffmpeg");
    cmd.args([
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-stats_period",
        "0.25",
        "-probesize",
        "32768",
        "-analyzeduration",
        "1",
        "-f",
        "flv",
        "-i",
        &input,
        "-map",
        "0:v:0",
        "-map",
        "0:a:0",
        "-c",
        "copy",
        "-progress",
        "pipe:1",
    ]);
    let network = match &target.destination {
        Location::Network(url) => {
            cmd.args([
                "-rw_timeout",
                "5000000",
                "-f",
                "flv",
                "-flvflags",
                "no_duration_filesize",
                "-flush_packets",
                "1",
            ]);
            if url.starts_with("rtmps://") {
                cmd.args(["-tls_verify", "1"]);
            }
            cmd.arg(url);
            true
        }
        Location::File(path) => {
            cmd.args(["-f", "matroska", "-n"]).arg(path);
            false
        }
    };
    let mut child = ChildGuard(
        cmd.stdin(if cfg!(windows) { Stdio::null() } else { Stdio::piped() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Start FFmpeg output")?,
    );
    #[cfg(unix)] let mut pipe = child.0.stdin.take().unwrap();
    #[cfg(windows)] pipe.set_peer(child.0.id());
    #[cfg(unix)]
    unsafe {
        let fd = pipe.as_raw_fd();
        let flags = libc::fcntl(fd, libc::F_GETFL);
        ensure!(
            flags >= 0 && libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) >= 0,
            "Cannot configure streaming pipe"
        );
    }
    let stdout = child.0.stdout.take().unwrap();
    let stderr = child.0.stderr.take().unwrap();
    let progress = Arc::new(Progress {
        packets: AtomicU64::new(0),
        last: Mutex::new(Instant::now()),
    });
    let p = progress.clone();
    let states = state.clone();
    let read_progress = thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(micros) = line
                .strip_prefix("out_time_us=")
                .and_then(|n| n.parse::<u64>().ok())
                && micros > p.packets.load(Ordering::Relaxed)
            {
                p.packets.store(micros, Ordering::Relaxed);
                *p.last.lock().unwrap() = Instant::now();
                let mut views = states.destinations.lock().unwrap();
                views[index].phase = Phase::Sending;
                views[index].detail.clear();
                views[index].elapsed = Duration::from_micros(micros);
            }
        }
    });
    let secrets = target.clone();
    let read_errors = thread::spawn(move || error_log(stderr, &secrets));
    let result = (|| -> Result<()> {
        let header_bytes = headers.lock().unwrap().bytes()?;
        write_packet(&mut pipe, &header_bytes, state, network, &progress)?;
        let base = first.timestamp;
        let mut next = Some(first);
        let mut eof_at = None;
        loop {
            if network && state.stopping.load(Ordering::Relaxed) {
                break;
            }
            ensure!(child.0.try_wait()?.is_none(), "Output process exited");
            ensure!(
                progress.last.lock().unwrap().elapsed() < IO_TIMEOUT,
                "Server or output stopped accepting video"
            );
            let tag = if next.is_some() {
                next.take()
            } else {
                let mut q = queue.lock().unwrap();
                ensure!(
                    !q.overflow,
                    "Output fell behind; discarded its buffered video"
                );
                if q.closed {
                    let at = eof_at.get_or_insert_with(Instant::now);
                    ensure!(
                        at.elapsed() < Duration::from_secs(6),
                        "Output did not drain in time"
                    );
                    if q.tags.is_empty() {
                        break;
                    }
                }
                q.pop()
            };
            if let Some(tag) = tag {
                // After reconnect the new FLV starts at a keyframe with a fresh
                // zero-based clock. Preserve all subsequent A/V timestamp gaps.
                if tag.timestamp < base {
                    continue;
                }
                let mut packet = (*tag.data).clone();
                let timestamp = tag.timestamp - base;
                packet[4] = (timestamp >> 16) as u8;
                packet[5] = (timestamp >> 8) as u8;
                packet[6] = timestamp as u8;
                packet[7] = (timestamp >> 24) as u8;
                write_packet(&mut pipe, &packet, state, network, &progress)?;
            } else {
                thread::sleep(Duration::from_millis(3));
            }
        }
        Ok(())
    })();
    drop(pipe);
    let deadline = Instant::now() + Duration::from_secs(if network { 1 } else { 6 });
    let status = loop {
        if let Some(status) = child.0.try_wait()? {
            break status;
        }
        if result.is_err() || Instant::now() >= deadline {
            let _ = child.0.kill();
            break child.0.wait()?;
        }
        thread::sleep(Duration::from_millis(10));
    };
    drop(child);
    let _ = read_progress.join();
    let errors = read_errors
        .join()
        .unwrap_or_else(|_| "Output diagnostics unavailable".into());
    if network && state.stopping.load(Ordering::Relaxed) {
        return Ok(());
    }
    if !errors.is_empty() {
        bail!(errors);
    }
    result?;
    ensure!(status.success(), "FFmpeg output stopped ({status})");
    ensure!(
        progress.packets.load(Ordering::Relaxed) > 0,
        "No encoded video reached the output"
    );
    if let Location::File(path) = &target.destination {
        ensure!(
            path.metadata().is_ok_and(|m| m.len() > 512),
            "Recording was not saved"
        );
    }
    Ok(())
}
fn write_packet(
    pipe: &mut RelayPipe,
    mut bytes: &[u8],
    state: &State,
    network: bool,
    progress: &Progress,
) -> Result<()> {
    let start = Instant::now();
    while !bytes.is_empty() {
        if network && state.stopping.load(Ordering::Relaxed) {
            return Ok(());
        }
        ensure!(
            start.elapsed() < IO_TIMEOUT && progress.last.lock().unwrap().elapsed() < IO_TIMEOUT,
            "Output write timed out"
        );
        #[cfg(unix)] let written=pipe.write(bytes);
        #[cfg(windows)] let written=pipe.write_cancelled(bytes,|| (network && state.stopping.load(Ordering::Relaxed)) || progress.last.lock().unwrap().elapsed()>=IO_TIMEOUT);
        match written {
            Ok(0) => bail!("Output pipe closed"),
            Ok(n) => bytes = &bytes[n..],
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(3))
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => bail!("Output pipe closed"),
        }
    }
    Ok(())
}
fn error_log(mut input: impl Read, target: &Target) -> String {
    let mut buf = [0; 4096];
    let mut line = Vec::new();
    let mut too_long = false;
    let mut log = VecDeque::new();
    let emit = |line: &mut Vec<u8>, long: bool, log: &mut VecDeque<String>| {
        let text = if long {
            "Output message omitted (too long)".into()
        } else {
            target.redact(&String::from_utf8_lossy(line))
        };
        log.push_back(text.chars().take(1200).collect());
        while log.len() > 4 {
            log.pop_front();
        }
        line.clear();
    };
    while let Ok(n) = input.read(&mut buf) {
        if n == 0 {
            break;
        }
        for b in &buf[..n] {
            if *b == b'\n' {
                emit(&mut line, too_long, &mut log);
                too_long = false;
            } else if line.len() < 8192 {
                line.push(*b);
            } else {
                too_long = true;
            }
        }
    }
    if !line.is_empty() {
        emit(&mut line, too_long, &mut log);
    }
    log.into_iter().collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_queue_marks_loss_instead_of_blocking_other_destinations() {
        let mut slow = Queue::default();
        let mut healthy = Queue::default();
        let tag = Tag {
            data: Arc::new(vec![0; 65536]),
            timestamp: 0,
            keyframe: false,
        };
        for _ in 0..200 {
            slow.push(tag.clone());
            healthy.push(tag.clone());
            assert!(healthy.pop().is_some());
        }
        assert!(slow.overflow);
        assert!(slow.bytes <= MAX_QUEUE);
        assert!(!healthy.overflow);
    }
    #[test]
    fn malformed_or_oversized_packets_are_rejected_before_allocating() {
        assert!(read_tag(&mut &b"\x09\xff\xff\xff\0\0\0\0\0\0\0"[..]).is_err());
        assert!(read_tag(&mut &b"\x09\0\0\x08"[..]).is_err());
        assert!(read_tag(&mut &b""[..]).unwrap().is_none());
    }
    #[test]
    fn diagnostics_redact_before_truncation_and_drop_oversized_lines() {
        let t = Target {
            name: "Test".into(),
            destination: Location::Network("rtmps://example/app/key-secret".into()),
            secret: "key-secret".into(),
        };
        let text = format!(
            "{}rtmps://example/app/key-secret key-secret\n{}key-secret\n",
            "x".repeat(1195),
            "y".repeat(9000)
        );
        let log = error_log(text.as_bytes(), &t);
        assert!(!log.contains("key-secret"));
        assert!(!log.contains("rtmps://"));
        assert!(log.contains("omitted"));
    }
}
#[cfg(test)]
mod replay_tests {
    use super::*;
    /// A minimal FLV tag of `kind` at `ts` ms; video keyframes start with 0x17 0x01.
    fn tag(kind: u8, ts: u32, key: bool) -> Tag {
        let body: Vec<u8> = if kind == 9 { vec![if key { 0x17 } else { 0x27 }, 1, 0, 0, 0] } else { vec![0xAF, 1, 0, 0, 0] };
        let size = body.len() as u32;
        let mut data = vec![kind, (size >> 16) as u8, (size >> 8) as u8, size as u8, (ts >> 16) as u8, (ts >> 8) as u8, ts as u8, (ts >> 24) as u8, 0, 0, 0];
        data.extend(body);
        data.extend((size + 11).to_be_bytes());
        Tag { data: Arc::new(data), timestamp: ts, keyframe: kind == 9 && key }
    }
    #[test]
    fn keeps_the_window_from_a_keyframe_with_timestamps_from_zero() {
        let mut r = Replay { enabled: true, seconds: 5, file: b"FLV\x01\x05\0\0\0\x09\0\0\0\0".to_vec(), headers: vec![tag(9, 0, true)], tags: VecDeque::new(), bytes: 0 };
        // 12 seconds at 10 fps, a keyframe every 2 s, audio in between.
        for i in 0..120u32 {
            let ts = i * 100;
            r.push(&tag(9, ts, i % 20 == 0));
            r.push(&tag(8, ts + 50, false));
        }
        let first = r.tags.front().unwrap();
        assert!(first.keyframe, "starts on a keyframe");
        assert!(r.held() >= 5 && r.held() <= 8, "at least 5 s held: {}", r.held());
        let flv = r.flv().unwrap();
        assert_eq!(&flv[..3], b"FLV");
        // The first packet after the header tag is stamped 0.
        let after_header = 13 + r.headers[0].data.len();
        assert_eq!(&flv[after_header + 4..after_header + 8], &[0, 0, 0, 0]);
    }
    #[test]
    fn empty_buffer_says_so() {
        let r = Replay { enabled: true, seconds: 30, file: Vec::new(), headers: Vec::new(), tags: VecDeque::new(), bytes: 0 };
        assert!(r.flv().is_err());
    }
}
