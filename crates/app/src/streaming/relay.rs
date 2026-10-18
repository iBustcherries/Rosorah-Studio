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
fn dispatch(
    mut input: impl Read,
    headers: &Mutex<Headers>,
    queues: &[Arc<Mutex<Queue>>],
) -> Result<()> {
    let mut header = [0; 13];
    input.read_exact(&mut header)?;
    ensure!(
        &header[..3] == b"FLV" && header[3] == 1 && header[8] == 9 && header[5..8] == [0, 0, 0],
        "Invalid FLV encoder header"
    );
    headers.lock().unwrap().file = header.to_vec();
    while let Some(tag) = read_tag(&mut input)? {
        let d = &tag.data;
        let mut h = headers.lock().unwrap();
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
            if let Err(e) = dispatch(input, &headers, &queues) {
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
