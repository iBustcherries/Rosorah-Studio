//! Event-driven game renderer receiver. A home-directory socket passes an anonymous
//! memfd; pixels never go through regular files or Steam launch-option mounts.
use anyhow::{Context, Result, ensure};
use rosorah_capture::{Backend, Frame, GameProcess, Options, Producer};
use std::{
    fs::{self, File, OpenOptions},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
        unix::{
            ffi::OsStrExt,
            fs::{DirBuilderExt, FileExt, FileTypeExt, MetadataExt, OpenOptionsExt},
        },
    },
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::watch;
const HEADER: usize = 112;
const CAPACITY: u64 = HEADER as u64 + 4096 * 4096 * 4;
const MAGIC: u32 = 0x52535246;
const VERSION: u32 = 3;
fn monotonic_ns() -> u64 {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    unsafe {
        libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time);
    }
    time.tv_sec as u64 * 1_000_000_000 + time.tv_nsec as u64
}
fn lock(file: &File, exclusive: bool) -> bool {
    unsafe {
        libc::flock(
            file.as_raw_fd(),
            (if exclusive {
                libc::LOCK_EX
            } else {
                libc::LOCK_SH
            }) | libc::LOCK_NB,
        ) == 0
    }
}
struct ReadLock<'a>(&'a File);
impl Drop for ReadLock<'_> {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}
fn socket() -> Result<OwnedFd> {
    let fd = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_SEQPACKET | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            0,
        )
    };
    ensure!(
        fd >= 0,
        "Cannot create renderer connection: {}",
        std::io::Error::last_os_error()
    );
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}
fn peer_pid(fd: RawFd) -> Option<u32> {
    let mut peer: libc::ucred = unsafe { std::mem::zeroed() };
    let mut size = std::mem::size_of_val(&peer) as libc::socklen_t;
    let valid = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut peer as *mut libc::ucred).cast(),
            &mut size,
        ) == 0
            && peer.uid == libc::geteuid()
            && peer.pid > 0
    };
    valid.then_some(peer.pid as u32)
}
fn message(kind: u32, value: u32) -> [u8; 16] {
    let mut bytes = [0; 16];
    for (i, v) in [MAGIC, VERSION, kind, value].iter().enumerate() {
        bytes[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    bytes
}
fn send_control(fd: RawFd, kind: u32, value: u32) -> bool {
    let bytes = message(kind, value);
    unsafe {
        libc::send(
            fd,
            bytes.as_ptr().cast(),
            bytes.len(),
            libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
        ) == bytes.len() as isize
    }
}
struct Received {
    bytes: Vec<u8>,
    files: Vec<OwnedFd>,
}
fn receive(fd: RawFd, capacity: usize) -> Result<Option<Received>> {
    let mut bytes = vec![0; capacity];
    // usize ensures cmsghdr alignment. Collect/close every passed descriptor,
    // including malformed messages; MSG_CMSG_CLOEXEC prevents exec leakage.
    let mut control = [0usize; 32];
    let mut iov = libc::iovec {
        iov_base: bytes.as_mut_ptr().cast(),
        iov_len: bytes.len(),
    };
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = control.as_mut_ptr().cast();
    msg.msg_controllen = std::mem::size_of_val(&control);
    let count = unsafe { libc::recvmsg(fd, &mut msg, libc::MSG_DONTWAIT | libc::MSG_CMSG_CLOEXEC) };
    if count < 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::WouldBlock
            || error.kind() == std::io::ErrorKind::Interrupted
        {
            return Ok(None);
        }
        return Err(error.into());
    }
    let mut files = Vec::new();
    unsafe {
        let mut c = libc::CMSG_FIRSTHDR(&msg);
        while !c.is_null() {
            if (*c).cmsg_level == libc::SOL_SOCKET && (*c).cmsg_type == libc::SCM_RIGHTS {
                let len = (*c).cmsg_len.saturating_sub(libc::CMSG_LEN(0) as usize);
                for i in 0..len / std::mem::size_of::<RawFd>() {
                    let raw = std::ptr::read_unaligned(libc::CMSG_DATA(c).cast::<RawFd>().add(i));
                    files.push(OwnedFd::from_raw_fd(raw));
                }
            }
            c = libc::CMSG_NXTHDR(&msg, c);
        }
    }
    ensure!(
        count > 0 && msg.msg_flags & (libc::MSG_TRUNC | libc::MSG_CTRUNC) == 0,
        "Renderer connection closed or invalid packet"
    );
    bytes.truncate(count as usize);
    Ok(Some(Received { bytes, files }))
}
#[derive(Clone, Debug)]
struct Header {
    width: u32,
    height: u32,
    bgra: bool,
    pid: u32,
    steam: bool,
    gl: bool,
    hdr: bool,
    bits32: bool,
    sequence: u64,
    /// When the game captured the frame (monotonic nanoseconds).
    timestamp: u64,
    name: String,
}
fn parse_header(bytes: &[u8], now: u64) -> Option<Header> {
    if bytes.len() != HEADER {
        return None;
    }
    let u32at = |i| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
    let u64at = |i| u64::from_le_bytes(bytes[i..i + 8].try_into().unwrap());
    let (width, height, timestamp) = (u32at(8), u32at(12), u64at(40));
    if u32at(0) != MAGIC
        || u32at(4) != VERSION
        || !matches!(u32at(16), 1 | 2)
        || width == 0
        || height == 0
        || width > 4096
        || height > 4096
        || timestamp > now
        || now - timestamp > 2_000_000_000
        || u32at(20) == std::process::id()
    {
        return None;
    }
    let end = bytes[48..].iter().position(|b| *b == 0).unwrap_or(64);
    Some(Header {
        width,
        height,
        bgra: u32at(16) == 2,
        pid: u32at(20),
        steam: u32at(24) & 1 != 0,
        gl: u32at(24) & 2 != 0,
        hdr: u32at(24) & 4 != 0,
        bits32: u32at(24) & 8 != 0,
        sequence: u64at(32),
        timestamp,
        name: String::from_utf8_lossy(&bytes[48..48 + end])
            .chars()
            .filter(|c| !c.is_control())
            .collect(),
    })
}
fn open_memory(fd: &OwnedFd) -> Result<File> {
    // SCM_RIGHTS shares an open-file description (and its flock owner). Reopening
    // creates an independent owner, so a reader cannot unlock the writer's lock.
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC)
        .open(format!("/proc/self/fd/{}", fd.as_raw_fd()))?;
    let m = file.metadata()?;
    let seals = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GET_SEALS) };
    let required = libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL;
    ensure!(
        m.is_file()
            && m.uid() == unsafe { libc::geteuid() }
            && m.mode() & 0o077 == 0
            && m.nlink() == 0
            && m.len() == CAPACITY
            && seals >= 0
            && seals & required == required,
        "Renderer memory has invalid ownership, size, or seals"
    );
    Ok(file)
}
fn read_frame(file: &File, previous: u64) -> Result<Option<(Header, Frame)>> {
    if !lock(file, false) {
        return Ok(None);
    }
    let _guard = ReadLock(file);
    let mut bytes = [0u8; HEADER];
    file.read_exact_at(&mut bytes, 0)?;
    let Some(header) = parse_header(&bytes, monotonic_ns()) else {
        return Ok(None);
    };
    if header.sequence == previous {
        return Ok(None);
    }
    let mut rgba = vec![0; header.width as usize * header.height as usize * 4];
    file.read_exact_at(&mut rgba, HEADER as u64)?;
    // Release the mailbox before conversion, minimizing producer contention.
    drop(_guard);
    // Whole pixels at a time; a byte-wise swap took several milliseconds per 1080p frame.
    for pixel in rgba.as_chunks_mut::<4>().0 {
        let v = u32::from_le_bytes(*pixel);
        let v = if header.bgra { (v & 0x0000_ff00) | ((v >> 16) & 0xff) | ((v & 0xff) << 16) } else { v & 0x00ff_ffff };
        *pixel = (v | 0xff00_0000).to_le_bytes();
    }
    let frame = Frame {
        width: header.width,
        height: header.height,
        rgba,
    };
    Ok(Some((header, frame)))
}
struct Client {
    socket: OwnedFd,
    process: Option<GameProcess>,
    memory: Option<File>,
    header: Option<Header>,
    sequence: u64,
    accepted: Instant,
    alive: bool,
    notified: bool,
    rate: Option<u32>,
    resolution: Option<u32>,
}
impl Client {
    fn drain(&mut self) -> Result<()> {
        for _ in 0..64 {
            let Some(mut packet) = receive(self.socket.as_raw_fd(), HEADER)? else {
                break;
            };
            if self.memory.is_none() {
                ensure!(
                    packet.files.len() == 1,
                    "Missing renderer memory descriptor"
                );
                let header = parse_header(&packet.bytes, monotonic_ns())
                    .context("Invalid renderer handshake")?;
                self.memory = Some(open_memory(&packet.files.remove(0))?);
                self.header = Some(header);
            } else {
                ensure!(
                    packet.files.is_empty() && packet.bytes == message(2, 0),
                    "Invalid frame notification"
                );
                self.notified = true;
            }
        }
        Ok(())
    }
}
fn select_client(clients: &[Client], previous: Option<RawFd>) -> Option<usize> {
    let current = clients
        .iter()
        .position(|c| c.header.is_some() && Some(c.socket.as_raw_fd()) == previous);
    // Pin an active game; allow Steam to replace an initially selected
    // non-game graphics application. Other processes receive a zero rate.
    let best = clients
        .iter()
        .enumerate()
        .filter(|(_, c)| c.header.is_some())
        .max_by_key(|(_, c)| {
            let h = c.header.as_ref().unwrap();
            (h.steam, u64::from(h.width) * u64::from(h.height))
        })
        .map(|(i, _)| i);
    match (current, best) {
        (Some(i), Some(j))
            if !clients[i].header.as_ref().unwrap().steam
                && clients[j].header.as_ref().unwrap().steam =>
        {
            Some(j)
        }
        (Some(i), _) => Some(i),
        (_, best) => best,
    }
}
struct Server {
    listener: OwnedFd,
    clients: Vec<Client>,
    selected: Option<RawFd>,
    latest: Option<Frame>,
    game: Option<GameProcess>,
    generation: u64,
    disconnected: Option<Instant>,
    /// `ROSORAH_CAPTURE_TRACE=1`: frame timing for game capture.
    trace: Option<crate::capture::Trace>,
}
impl Server {
    /// The sockets to wait on before the next `pump`.
    fn poll_fds(&self) -> Vec<libc::pollfd> {
        std::iter::once(self.listener.as_raw_fd())
            .chain(self.clients.iter().map(|c| c.socket.as_raw_fd()))
            .map(|fd| libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            })
            .collect()
    }
    fn pump(&mut self, fps: u32, resolution: u32) {
        for _ in 0..16 {
            let raw = unsafe {
                libc::accept4(
                    self.listener.as_raw_fd(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
                )
            };
            if raw < 0 {
                break;
            }
            let fd = unsafe { OwnedFd::from_raw_fd(raw) };
            if self.clients.len() < 32
                && let Some(pid) = peer_pid(raw)
                && crate::desktop::renderer_process_allowed(pid)
            {
                self.clients.push(Client {
                    socket: fd,
                    process: crate::game_audio::identity(pid, String::new()),
                    memory: None,
                    header: None,
                    sequence: 0,
                    accepted: Instant::now(),
                    alive: true,
                    notified: false,
                    rate: None,
                    resolution: None,
                });
            }
        }
        for client in &mut self.clients {
            if client.drain().is_err()
                || (client.memory.is_none() && client.accepted.elapsed() > Duration::from_secs(2))
            {
                client.alive = false;
            }
        }
        self.clients.retain(|c| c.alive);
        let selected = select_client(&self.clients, self.selected);
        let next_selected = selected.map(|i| self.clients[i].socket.as_raw_fd());
        if next_selected != self.selected || next_selected.is_none() {
            self.game = None;
        }
        if self.trace.is_some() && next_selected != self.selected {
            eprintln!("capture trace: game capture switched source ({} connected)", self.clients.len());
        }
        self.selected = next_selected;
        for (i, client) in self.clients.iter_mut().enumerate() {
            if client.memory.is_none() {
                continue;
            }
            let rate = if Some(i) == selected { fps } else { 0 };
            if client.resolution != Some(resolution) {
                if send_control(client.socket.as_raw_fd(), 3, resolution) {
                    client.resolution = Some(resolution);
                } else {
                    continue;
                }
            }
            if client.rate != Some(rate) && send_control(client.socket.as_raw_fd(), 1, rate) {
                client.rate = Some(rate);
            }
            if Some(i) != selected || !client.notified {
                continue;
            }
            let work = Instant::now();
            match read_frame(client.memory.as_ref().unwrap(), client.sequence) {
                Ok(Some((header, frame))) => {
                    if let Some(trace) = &mut self.trace {
                        trace.frame(Some(gst::ClockTime::from_nseconds(header.timestamp)), work.elapsed(), Some(crate::capture::fingerprint(&frame.rgba)));
                    }
                    if client.sequence == 0 {
                        crate::capture_log::event(format!(
                            "{} connected: {} (PID {}) {}x{} · {}-bit{} · shared memory",
                            if header.gl { "OpenGL" } else { "Vulkan" },
                            header.name,
                            header.pid,
                            header.width,
                            header.height,
                            if header.bits32 { 32 } else { 64 },
                            if header.hdr { " · HDR to SDR" } else { "" }
                        ));
                    }
                    client.sequence = header.sequence;
                    client.notified = false;
                    self.game = client.process.clone().map(|mut p| {
                        p.name = header.name.clone();
                        p
                    });
                    self.latest = Some(frame);
                    self.generation += 1;
                    self.disconnected = None;
                }
                Ok(None) => {} // A contended or repeated frame keeps the last image.
                Err(_) => client.alive = false,
            }
        }
        if self.selected.is_none() && self.latest.is_some() {
            let since = self.disconnected.get_or_insert_with(Instant::now);
            if since.elapsed() >= Duration::from_secs(2) {
                self.latest = None;
                self.generation += 1;
            }
        }
    }
}
struct Lease {
    path: PathBuf,
    server: Mutex<Server>,
    /// Signalled after every pump, so each game capture source publishes a new frame at once.
    pumped: Condvar,
    /// One source's thread waits for the game at a time, without holding `server`; while it
    /// held the lock through its wait, three game layers in a scene took turns and each got
    /// its frames late and in bursts.
    pumping: AtomicBool,
    _owner: File,
}
impl Drop for Lease {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
static LEASE: OnceLock<Mutex<Weak<Lease>>> = OnceLock::new();
impl Lease {
    fn acquire() -> Result<Arc<Self>> {
        let mut global = LEASE.get_or_init(Default::default).lock().unwrap();
        if let Some(lease) = global.upgrade() {
            return Ok(lease);
        }
        let home = std::env::var_os("HOME").context("HOME is missing")?;
        let directory = PathBuf::from(home).join(".rosorah-capture");
        let lease = Arc::new(Self::bind(directory)?);
        *global = Arc::downgrade(&lease);
        Ok(lease)
    }
    fn bind(directory: PathBuf) -> Result<Self> {
        ensure!(directory.is_absolute(), "Home directory must be absolute");
        if !directory.exists() {
            fs::DirBuilder::new().mode(0o700).create(&directory)?;
        }
        let meta = fs::symlink_metadata(&directory)?;
        ensure!(
            meta.is_dir() && meta.uid() == unsafe { libc::geteuid() } && meta.mode() & 0o077 == 0,
            "Renderer connection directory must belong to you and be private"
        );
        let owner = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(directory.join("owner"))?;
        let meta = owner.metadata()?;
        ensure!(
            meta.is_file()
                && meta.uid() == unsafe { libc::geteuid() }
                && meta.mode() & 0o077 == 0
                && meta.nlink() == 1
                && lock(&owner, true),
            "Another Rosorah instance owns automatic capture"
        );
        let path = directory.join("receiver.sock");
        if let Ok(meta) = fs::symlink_metadata(&path) {
            ensure!(
                meta.file_type().is_socket() && meta.uid() == unsafe { libc::geteuid() },
                "Renderer socket path is occupied"
            );
            fs::remove_file(&path)?;
        }
        let listener = socket()?;
        let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        address.sun_family = libc::AF_UNIX as libc::sa_family_t;
        let bytes = path.as_os_str().as_bytes();
        ensure!(
            bytes.len() < address.sun_path.len(),
            "Home path is too long for a renderer socket"
        );
        for (a, b) in address.sun_path.iter_mut().zip(bytes) {
            *a = *b as libc::c_char;
        }
        let bound = unsafe {
            libc::bind(
                listener.as_raw_fd(),
                (&address as *const libc::sockaddr_un).cast(),
                std::mem::size_of_val(&address) as libc::socklen_t,
            )
        };
        ensure!(
            bound == 0,
            "Cannot bind renderer connection: {}",
            std::io::Error::last_os_error()
        );
        if unsafe { libc::listen(listener.as_raw_fd(), 16) } != 0 {
            let _ = fs::remove_file(&path);
            anyhow::bail!("Cannot listen for renderer connections");
        }
        Ok(Self {
            path,
            pumped: Condvar::new(),
            pumping: AtomicBool::new(false),
            _owner: owner,
            server: Mutex::new(Server {
                listener,
                clients: Vec::new(),
                selected: None,
                latest: None,
                game: None,
                generation: 0,
                disconnected: None,
                trace: std::env::var_os("ROSORAH_CAPTURE_TRACE").map(|_| Default::default()),
            }),
        })
    }
}
pub fn installation_status() -> &'static str {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share")));
    if base.is_some_and(|p| {
        p.join("vulkan/implicit_layer.d/rosorah-capture.json")
            .is_file()
    }) {
        if std::env::var("ROSORAH_GL_HOOK_VERSION").as_deref() == Ok(env!("CARGO_PKG_VERSION")) {
            "Waiting for a game. Launch normally; Vulkan and OpenGL capture are ready."
        } else {
            "Vulkan is ready. OpenGL launch activation status is in Capture details."
        }
    } else {
        "Install automatic capture by launching run-studio.sh from the complete package."
    }
}
pub struct RendererBackend;
impl Backend for RendererBackend {
    fn run(
        self: Box<Self>,
        output: Producer,
        cancel: watch::Receiver<bool>,
        options: Options,
    ) -> Result<()> {
        let lease = Lease::acquire()?;
        let mut generation = 0;
        output.waiting(installation_status());
        while !*cancel.borrow() {
            if !lease.pumping.swap(true, Ordering::AcqRel) {
                let mut poll = lease.server.lock().unwrap().poll_fds();
                // Notifications wake immediately; the short timeout serves cancellation
                // and a retry if the producer owned the mailbox at the notification.
                unsafe {
                    libc::poll(poll.as_mut_ptr(), poll.len() as libc::nfds_t, 10);
                }
                lease.server.lock().unwrap().pump(options.fps, options.resolution);
                lease.pumping.store(false, Ordering::Release);
                lease.pumped.notify_all();
            } else {
                let server = lease.server.lock().unwrap();
                if server.generation == generation {
                    let _ = lease.pumped.wait_timeout(server, Duration::from_millis(20)).unwrap();
                }
            }
            let update = {
                let server = lease.server.lock().unwrap();
                output.game(server.game.clone());
                if server.generation == generation {
                    None
                } else {
                    generation = server.generation;
                    Some(server.latest.clone())
                }
            };
            if let Some(frame) = update {
                match frame {
                    Some(frame) => output.publish(frame)?,
                    None => {
                        output.publish(Frame {
                            width: 1,
                            height: 1,
                            rgba: vec![0; 4],
                        })?;
                        output.waiting("Waiting for a game");
                    }
                }
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn header(now: u64) -> [u8; HEADER] {
        let mut b = [0; HEADER];
        for (i, v) in [
            (0, MAGIC),
            (4, VERSION),
            (8, 2),
            (12, 1),
            (16, 2),
            (20, 999999),
        ] {
            b[i..i + 4].copy_from_slice(&v.to_le_bytes());
        }
        b[32..40].copy_from_slice(&1u64.to_le_bytes());
        b[40..48].copy_from_slice(&now.to_le_bytes());
        b
    }
    #[test]
    fn malformed_stale_oversized_and_old_protocol_are_rejected() {
        let mut b = header(10_000_000_000);
        assert!(parse_header(&b, 10_000_000_000).is_some());
        assert!(parse_header(&b[..111], 10_000_000_000).is_none());
        assert!(parse_header(&b, 13_000_000_000).is_none());
        assert!(parse_header(&b, 9_000_000_000).is_none());
        b[4..8].copy_from_slice(&1u32.to_le_bytes());
        assert!(parse_header(&b, 10_000_000_000).is_none());
        b[4..8].copy_from_slice(&VERSION.to_le_bytes());
        b[8..12].copy_from_slice(&4097u32.to_le_bytes());
        assert!(parse_header(&b, 10_000_000_000).is_none());
    }
    #[test]
    fn renderer_metadata_accepts_both_architectures_and_preserves_protocol() {
        let now = 10_000_000_000;
        let mut bytes = header(now);
        bytes[24..28].copy_from_slice(&15u32.to_le_bytes());
        let h = parse_header(&bytes, now).unwrap();
        assert!(h.steam && h.gl && h.hdr && h.bits32);
        bytes[24..28].copy_from_slice(&0u32.to_le_bytes());
        let h = parse_header(&bytes, now).unwrap();
        assert!(!h.steam && !h.gl && !h.hdr && !h.bits32);
    }
    #[test]
    fn shared_memory_has_independent_locks_and_holds_on_contention() {
        let raw = unsafe {
            libc::memfd_create(
                c"rosorah-test".as_ptr(),
                libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
            )
        };
        assert!(raw >= 0);
        let owned = unsafe { OwnedFd::from_raw_fd(raw) };
        let writer = File::from(owned.try_clone().unwrap());
        writer.set_len(CAPACITY).unwrap();
        unsafe {
            assert_eq!(libc::fchmod(raw, 0o600), 0);
            assert_eq!(
                libc::fcntl(
                    raw,
                    libc::F_ADD_SEALS,
                    libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL
                ),
                0
            );
        }
        writer.write_all_at(&header(monotonic_ns()), 0).unwrap();
        writer
            .write_all_at(&[1, 2, 3, 0, 4, 5, 6, 0], HEADER as u64)
            .unwrap();
        let reader = open_memory(&owned).unwrap();
        assert!(lock(&writer, true));
        for _ in 0..100 {
            assert!(read_frame(&reader, 0).unwrap().is_none());
        }
        unsafe {
            libc::flock(raw, libc::LOCK_UN);
        }
        let (h, f) = read_frame(&reader, 0).unwrap().unwrap();
        assert_eq!(f.rgba, vec![3, 2, 1, 255, 6, 5, 4, 255]);
        assert!(read_frame(&reader, h.sequence).unwrap().is_none());
        // In-flight readers cannot be invalidated by a producer resizing storage.
        assert!(writer.set_len(1).is_err());
    }
    fn client(steam: bool, width: u32) -> Client {
        let fd = File::open("/dev/null").unwrap();
        Client {
            socket: fd.into(),
            process: None,
            memory: None,
            header: Some(Header {
                width,
                height: 720,
                bgra: false,
                pid: 99999,
                steam,
                gl: false,
                hdr: false,
                bits32: false,
                sequence: 0,
                timestamp: 0,
                name: "test".into(),
            }),
            sequence: 0,
            accepted: Instant::now(),
            alive: true,
            notified: false,
            rate: None,
            resolution: None,
        }
    }
    #[test]
    fn selection_prefers_steam_then_pins_it_and_ignores_pending_handshakes() {
        let mut clients = vec![client(false, 1280), client(true, 640), client(true, 1280)];
        assert_eq!(select_client(&clients, None), Some(2));
        assert_eq!(
            select_client(&clients, Some(clients[0].socket.as_raw_fd())),
            Some(2)
        );
        assert_eq!(
            select_client(&clients, Some(clients[1].socket.as_raw_fd())),
            Some(1)
        );
        clients[1].header = None;
        assert_eq!(
            select_client(&clients, Some(clients[1].socket.as_raw_fd())),
            Some(2)
        );
    }
    #[test]
    #[ignore = "Requires Unix sockets and ROSORAH_TEST_PRODUCER pointing to native/build/test-transport"]
    fn native_producer_connects_and_delivers_to_rust_receiver() {
        let directory =
            std::env::temp_dir().join(format!("rs-ipc-{}-{}", std::process::id(), monotonic_ns()));
        let lease = Lease::bind(directory.clone()).unwrap();
        let program =
            std::env::var_os("ROSORAH_TEST_PRODUCER").expect("Build native/test.sh first");
        let mut child = std::process::Command::new(program)
            .arg("--produce")
            .arg(&lease.path)
            .spawn()
            .unwrap();
        let until = Instant::now() + Duration::from_secs(3);
        let mut received = false;
        while Instant::now() < until {
            let mut server = lease.server.lock().unwrap();
            server.pump(30, 1);
            if let Some(frame) = &server.latest {
                assert_eq!(frame.rgba, vec![1, 2, 3, 255, 4, 5, 6, 255]);
                received = true;
            }
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
        }
        if child.try_wait().unwrap().is_none() {
            let _ = child.kill();
            let _ = child.wait();
            panic!("Native producer timed out");
        }
        assert!(received);
        drop(lease);
        fs::remove_dir_all(directory).unwrap();
    }
}
