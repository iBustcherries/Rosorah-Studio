//! User-only, local named pipes for FFmpeg. No TCP listener, inherited descriptor
//! number, unbounded queue, or detached I/O worker. Every wait can be cancelled.
use std::{
    io::{self, Write},
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::*,
        Security::{Authorization::*, *},
        Storage::FileSystem::*,
        System::{Com::CoCreateGuid, IO::*, Pipes::*, Threading::*},
    },
    core::{PCWSTR, PWSTR},
};
fn raw(h: &OwnedHandle) -> HANDLE {
    HANDLE(h.as_raw_handle())
}
fn owned(h: HANDLE) -> OwnedHandle {
    unsafe { OwnedHandle::from_raw_handle(h.0) }
}
fn win(e: windows::core::Error) -> io::Error {
    io::Error::from_raw_os_error((e.code().0 & 0xffff) as i32)
}
struct Local(*mut std::ffi::c_void);
impl Drop for Local {
    fn drop(&mut self) {
        unsafe {
            let _ = LocalFree(HLOCAL(self.0));
        }
    }
}
fn descriptor() -> io::Result<Local> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).map_err(win)?;
        let token = owned(token);
        let mut count = 0;
        let _ = GetTokenInformation(raw(&token), TokenUser, None, 0, &mut count);
        if count == 0 || count > 65536 {
            return Err(io::Error::other("Invalid Windows user token"));
        }
        let mut buffer = vec![0usize; (count as usize).div_ceil(std::mem::size_of::<usize>())];
        GetTokenInformation(
            raw(&token),
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            count,
            &mut count,
        )
        .map_err(win)?;
        let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
        let mut sid = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut sid).map_err(win)?;
        let _sid = Local(sid.0.cast());
        let text = format!(
            "D:P(A;;GA;;;{})",
            sid.to_string().map_err(io::Error::other)?
        );
        let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        let mut sd = PSECURITY_DESCRIPTOR::default();
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(wide.as_ptr()),
            SDDL_REVISION_1,
            &mut sd,
            None,
        )
        .map_err(win)?;
        Ok(Local(sd.0))
    }
}
pub struct Writer {
    handle: OwnedHandle,
    name: String,
    peer: u32,
    connected: bool,
    cancel: Arc<AtomicBool>,
}
impl Writer {
    pub fn new(cancel: Arc<AtomicBool>) -> io::Result<Self> {
        unsafe {
            let name = format!(
                r"\\.\pipe\Rosorah-{}-{:?}",
                std::process::id(),
                CoCreateGuid().map_err(win)?
            );
            let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
            let sd = descriptor()?;
            let security = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: sd.0,
                bInheritHandle: false.into(),
            };
            let handle = CreateNamedPipeW(
                PCWSTR(wide.as_ptr()),
                PIPE_ACCESS_OUTBOUND | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                65536,
                0,
                0,
                Some(&security),
            );
            if handle == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                handle: owned(handle),
                name,
                peer: 0,
                connected: false,
                cancel,
            })
        }
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn set_peer(&mut self, pid: u32) {
        self.peer = pid;
    }
    fn operation(
        &self,
        deadline: Instant,
        cancel: &impl Fn() -> bool,
        start: impl FnOnce(*mut OVERLAPPED) -> windows::core::Result<()>,
    ) -> io::Result<u32> {
        unsafe {
            let event = owned(CreateEventW(None, true, false, None).map_err(win)?);
            let mut overlapped = OVERLAPPED {
                hEvent: raw(&event),
                ..Default::default()
            };
            // This guard drains cancelled I/O before the stack OVERLAPPED and
            // caller's buffer can disappear, including error/unwind paths.
            struct Pending {
                handle: HANDLE,
                op: *mut OVERLAPPED,
                active: bool,
            }
            impl Drop for Pending {
                fn drop(&mut self) {
                    if self.active {
                        unsafe {
                            let _ = CancelIoEx(self.handle, Some(self.op));
                            let mut n = 0;
                            let _ = GetOverlappedResult(self.handle, self.op, &mut n, true);
                        }
                    }
                }
            }
            let mut pending = Pending {
                handle: raw(&self.handle),
                op: &mut overlapped,
                active: false,
            };
            match start(&mut overlapped) {
                Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => return Ok(0),
                Err(e) if e.code() != ERROR_IO_PENDING.to_hresult() => return Err(win(e)),
                _ => pending.active = true,
            }
            loop {
                if cancel() || self.cancel.load(Ordering::Relaxed) {
                    return Err(io::Error::new(
                        io::ErrorKind::ConnectionAborted,
                        "Windows pipe cancelled",
                    ));
                }
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "Windows pipe stalled",
                    ));
                }
                match WaitForSingleObject(raw(&event), 20) {
                    WAIT_OBJECT_0 => break,
                    WAIT_TIMEOUT => (),
                    _ => return Err(io::Error::last_os_error()),
                }
            }
            let mut n = 0;
            let result = GetOverlappedResult(raw(&self.handle), &overlapped, &mut n, false);
            pending.active = false;
            result.map_err(win)?;
            Ok(n)
        }
    }
    pub fn write_cancelled(
        &mut self,
        bytes: &[u8],
        cancel: impl Fn() -> bool,
    ) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        if self.peer == 0 {
            return Err(io::Error::other("FFmpeg process has not been attached"));
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        if !self.connected {
            self.operation(deadline, &cancel, |op| unsafe {
                ConnectNamedPipe(raw(&self.handle), Some(op))
            })?;
            let mut pid = 0;
            unsafe {
                GetNamedPipeClientProcessId(raw(&self.handle), &mut pid).map_err(win)?;
            }
            if pid != self.peer {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Unexpected named-pipe client",
                ));
            }
            self.connected = true;
        }
        let bytes = &bytes[..bytes.len().min(65536)];
        self.operation(deadline, &cancel, |op| unsafe {
            WriteFile(raw(&self.handle), Some(bytes), None, Some(op))
        })
        .map(|n| n as usize)
    }
}
impl Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.write_cancelled(bytes, || false)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs::File,
        io::Read,
        process::{Child, Command, Stdio},
    };
    struct ChildGuard(Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    fn client(pipe: &Writer, mode: &str) -> ChildGuard {
        ChildGuard(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "pipe::tests::child_client", "--ignored"])
                .env("ROSORAH_TEST_PIPE", pipe.name())
                .env("ROSORAH_TEST_PIPE_MODE", mode)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        )
    }
    #[test]
    #[ignore = "Only invoked as a subprocess by pipe tests"]
    fn child_client() {
        let Ok(name) = std::env::var("ROSORAH_TEST_PIPE") else {
            return;
        };
        let mut file = File::open(name).unwrap();
        if std::env::var("ROSORAH_TEST_PIPE_MODE").unwrap() == "stall" {
            std::thread::sleep(Duration::from_secs(20));
            return;
        }
        let mut data = Vec::new();
        file.read_to_end(&mut data).unwrap();
        assert_eq!(data.len(), 2 * 1024 * 1024);
        assert!(data.iter().all(|b| *b == 0x5a));
    }
    #[test]
    fn sends_payload_larger_than_pipe_buffer_and_closes_cleanly() {
        let mut pipe = Writer::new(Arc::new(AtomicBool::new(false))).unwrap();
        let mut child = client(&pipe, "read");
        pipe.set_peer(child.0.id());
        pipe.write_all(&vec![0x5a; 2 * 1024 * 1024]).unwrap();
        drop(pipe);
        let start = Instant::now();
        loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    #[test]
    fn unconnected_pipe_cancels_without_waiting_for_timeout() {
        let mut pipe = Writer::new(Arc::new(AtomicBool::new(true))).unwrap();
        pipe.set_peer(std::process::id());
        let start = Instant::now();
        assert_eq!(
            pipe.write(b"test").unwrap_err().kind(),
            io::ErrorKind::ConnectionAborted
        );
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn stalled_reader_is_cancellable_and_other_process_is_rejected() {
        let mut pipe = Writer::new(Arc::new(AtomicBool::new(false))).unwrap();
        let child = client(&pipe, "stall");
        pipe.set_peer(child.0.id());
        let start = Instant::now();
        loop {
            match pipe.write_cancelled(&[0x5a; 65536], || {
                start.elapsed() > Duration::from_millis(200)
            }) {
                Ok(_) => (),
                Err(e) => {
                    assert_eq!(e.kind(), io::ErrorKind::ConnectionAborted);
                    break;
                }
            }
        }
        assert!(start.elapsed() < Duration::from_secs(2));
        drop(pipe);
        drop(child);
        let mut pipe = Writer::new(Arc::new(AtomicBool::new(false))).unwrap();
        let _child = client(&pipe, "stall");
        pipe.set_peer(std::process::id());
        assert_eq!(
            pipe.write(b"private audio").unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
    }
}
