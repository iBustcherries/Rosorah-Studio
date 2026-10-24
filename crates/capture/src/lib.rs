//! Rosorah's capture lifecycle and bounded frame transport, independent of any
//! desktop, graphics API, UI toolkit or compositor implementation.
use anyhow::{Result, ensure};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::sync::watch;

pub type SourceId = u64;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Choosing,
    Connecting,
    Live,
    Stopping,
    Stopped,
    Failed,
}
impl Phase {
    pub fn running(self) -> bool {
        matches!(
            self,
            Self::Choosing | Self::Connecting | Self::Live | Self::Stopping
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Target {
    #[default]
    AutoFullscreen,
    WindowOrScreen,
    Window,
    Screen,
}
#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub target: Target,
    pub fps: u32,
    pub resolution: u32, // 0:720p, 1:1080p, 2:1440p, 3:2160p, 4:original
    pub cursor: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            target: Target::AutoFullscreen,
            fps: 60,
            resolution: 1,
            cursor: true,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
/// Host process identity. The start time prevents a recycled PID from becoming
/// an audio target after the captured application exits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameProcess {
    pub pid: u32,
    pub start_time: u64,
    pub name: String,
}
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub phase: Phase,
    pub detail: String,
    pub dimensions: Option<[u32; 2]>,
    pub received: u64,
    /// Frames replaced in Rosorah's mailbox before the compositor consumed them.
    pub replaced: u64,
    pub frame_age_ms: Option<u128>,
    /// Time the consumed frame waited in the latest-frame mailbox.
    pub mailbox_ms: Option<f64>,
    pub game: Option<GameProcess>,
}
struct State {
    snapshot: Snapshot,
    pending: Option<Option<Arc<Frame>>>,
    latest: Option<Arc<Frame>>,
    last_frame: Option<Instant>,
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
}
#[derive(Clone)]
pub struct Producer(Arc<Mutex<State>>);
impl Producer {
    pub fn note(&self, detail: &str) {
        let mut s = self.0.lock().unwrap();
        if s.snapshot.phase.running() {
            s.snapshot.detail = detail.into();
            let wake = s.wake.clone();
            drop(s);
            if let Some(wake) = wake {
                wake();
            }
        }
    }

    pub fn game(&self, game: Option<GameProcess>) {
        let mut s = self.0.lock().unwrap();
        if s.snapshot.phase.running() && s.snapshot.phase != Phase::Stopping {
            s.snapshot.game = game;
        }
    }
    pub fn waiting(&self, detail: &str) {
        let mut s = self.0.lock().unwrap();
        if s.snapshot.phase.running() && s.snapshot.phase != Phase::Stopping {
            s.snapshot.phase = Phase::Connecting;
            s.snapshot.detail = detail.into();
        }
    }

    /// Clear a departed target so a stale private frame is not retained while waiting.
    pub fn clear(&self, detail: &str) {
        let mut s=self.0.lock().unwrap();
        if !s.snapshot.phase.running() || s.snapshot.phase==Phase::Stopping { return; }
        s.latest=None;s.pending=Some(None);s.last_frame=None;s.snapshot.game=None;
        s.snapshot.phase=Phase::Connecting;s.snapshot.detail=detail.into();
        let wake=s.wake.clone();drop(s);if let Some(wake)=wake{wake();}
    }
    pub fn connecting(&self) {
        let mut s = self.0.lock().unwrap();
        if s.snapshot.phase == Phase::Choosing {
            s.snapshot.phase = Phase::Connecting;
            s.snapshot.detail = "Connecting to selected source…".into();
        }
    }
    pub fn publish(&self, frame: Frame) -> Result<()> {
        self.publish_with_detail(frame, None)
    }
    /// Publish the frame and its negotiated input description atomically.
    pub fn publish_with_detail(&self, frame: Frame, detail: Option<&str>) -> Result<()> {
        ensure!(
            frame.width > 0 && frame.height > 0 && frame.width <= 4096 && frame.height <= 4096,
            "Capture dimensions out of bounds"
        );
        ensure!(
            frame.rgba.len() == frame.width as usize * frame.height as usize * 4,
            "Invalid RGBA frame length"
        );
        let mut s = self.0.lock().unwrap();
        // Stopping is terminal for this generation. Late backend frames cannot
        // replace the clear notification or revive a stopped source.
        if matches!(
            s.snapshot.phase,
            Phase::Stopping | Phase::Stopped | Phase::Failed
        ) {
            return Ok(());
        }
        if matches!(s.pending, Some(Some(_))) {
            s.snapshot.replaced += 1;
        }
        s.snapshot.phase = Phase::Live;
        s.snapshot.dimensions = Some([frame.width, frame.height]);
        s.snapshot.received += 1;
        s.snapshot.detail = detail
            .map(str::to_owned)
            .unwrap_or_else(|| format!("Live · {} × {}", frame.width, frame.height));
        s.last_frame = Some(Instant::now());
        let frame = Arc::new(frame);
        s.latest = Some(frame.clone());
        s.pending = Some(Some(frame));
        let wake = s.wake.clone();
        drop(s);
        if let Some(wake) = wake {
            wake();
        }
        Ok(())
    }
}
/// A backend supplies authorized frames. It must respond to cancellation and
/// release its session/resources before returning. The engine owns final status.
pub trait Backend: Send + 'static {
    fn run(
        self: Box<Self>,
        output: Producer,
        cancel: watch::Receiver<bool>,
        options: Options,
    ) -> Result<()>;
}
struct Worker {
    output: Producer,
    cancel: watch::Sender<bool>,
}
/// Independent, bounded reader. Recording never consumes the preview's frame.
/// The registry follows backend replacement, so reconnects need no resubscribe.
#[derive(Clone, Default)]
pub struct FrameReader(Arc<Mutex<BTreeMap<SourceId, Producer>>>);
impl FrameReader {
    pub fn latest(&self) -> Vec<(SourceId, Option<Arc<Frame>>)> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|(id, p)| (*id, p.0.lock().unwrap().latest.clone()))
            .collect()
    }
    pub fn game(&self) -> Option<GameProcess> {
        self.0.lock().unwrap().values().find_map(|p| {
            let s = p.0.lock().unwrap();
            (s.snapshot.phase == Phase::Live)
                .then(|| s.snapshot.game.clone())
                .flatten()
        })
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.cancel.send(true);
    }
}
#[derive(Default)]
pub struct Engine {
    workers: BTreeMap<SourceId, Worker>,
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
    reader: FrameReader,
}
impl Engine {
    pub fn reader(&self) -> FrameReader {
        self.reader.clone()
    }
    pub fn set_wake(&mut self, wake: Arc<dyn Fn() + Send + Sync>) {
        for worker in self.workers.values() {
            worker.output.0.lock().unwrap().wake = Some(wake.clone());
        }
        self.wake = Some(wake);
    }
    pub fn start(
        &mut self,
        id: SourceId,
        backend: Box<dyn Backend>,
        options: Options,
    ) -> Result<bool> {
        ensure!(
            matches!(options.fps, 30 | 60),
            "Capture rate must be 30 or 60 FPS"
        );
        ensure!(options.resolution <= 4, "Invalid capture resolution");
        if self.running(id) {
            return Ok(false);
        }
        let output = Producer(Arc::new(Mutex::new(State {
            snapshot: Snapshot {
                phase: Phase::Choosing,
                detail: "Choose a source in the system picker…".into(),
                dimensions: None,
                received: 0,
                replaced: 0,
                frame_age_ms: None,
                mailbox_ms: None,
                game: None,
            },
            pending: Some(None),
            latest: None,
            last_frame: None,
            wake: self.wake.clone(),
        })));
        let (cancel, receiver) = watch::channel(false);
        let child = output.clone();
        std::thread::Builder::new()
            .name(format!("rosorah-capture-{id}"))
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    backend.run(child.clone(), receiver, options)
                }));
                let mut s = child.0.lock().unwrap();
                let stopped = s.snapshot.phase == Phase::Stopping;
                let error = match result {
                    Ok(Ok(())) => None,
                    Ok(Err(e)) => Some(format!("{e:#}")),
                    Err(_) => Some("Capture backend unexpectedly stopped".into()),
                };
                s.snapshot.phase = if stopped || error.is_none() {
                    Phase::Stopped
                } else {
                    Phase::Failed
                };
                s.snapshot.detail = if stopped {
                    "Capture stopped".into()
                } else {
                    error.unwrap_or_else(|| "Capture ended".into())
                };
                s.snapshot.dimensions = None;
                s.snapshot.game = None;
                s.pending = Some(None);
                s.latest = None;
                s.last_frame = None;
            })?;
        self.reader.0.lock().unwrap().insert(id, output.clone());
        self.workers.insert(id, Worker { output, cancel });
        Ok(true)
    }
    pub fn stop(&self, id: SourceId) {
        if let Some(w) = self.workers.get(&id) {
            let mut s = w.output.0.lock().unwrap();
            if s.snapshot.phase.running() {
                s.snapshot.phase = Phase::Stopping;
                s.snapshot.detail = "Stopping capture…".into();
                s.pending = Some(None);
                s.latest = None;
                s.snapshot.dimensions = None;
                s.snapshot.game = None;
                s.last_frame = None;
                let _ = w.cancel.send(true);
            }
        }
    }
    pub fn snapshot(&self, id: SourceId) -> Option<Snapshot> {
        self.workers.get(&id).map(|w| {
            let s = w.output.0.lock().unwrap();
            let mut v = s.snapshot.clone();
            v.frame_age_ms = s.last_frame.map(|t| t.elapsed().as_millis());
            v
        })
    }
    pub fn running(&self, id: SourceId) -> bool {
        self.snapshot(id).is_some_and(|s| s.phase.running())
    }
    /// Drains at most one frame/clear per source. Canvas count never creates
    /// additional backend sessions or duplicate uploads.
    pub fn drain(&self) -> Vec<(SourceId, Option<Arc<Frame>>)> {
        self.workers
            .iter()
            .filter_map(|(id, w)| {
                let mut s = w.output.0.lock().unwrap();
                let frame = s.pending.take()?;
                s.snapshot.mailbox_ms = if frame.is_some() {
                    s.last_frame.map(|at| at.elapsed().as_secs_f64() * 1000.)
                } else { None };
                Some((*id, frame))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    struct Controlled {
        started: mpsc::Sender<Producer>,
    }
    impl Backend for Controlled {
        fn run(
            self: Box<Self>,
            p: Producer,
            mut stop: watch::Receiver<bool>,
            _: Options,
        ) -> Result<()> {
            self.started.send(p)?;
            while !*stop.borrow_and_update() {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Ok(())
        }
    }
    fn begin(e: &mut Engine) -> Producer {
        let (tx, rx) = mpsc::channel();
        assert!(
            e.start(1, Box::new(Controlled { started: tx }), Options::default())
                .unwrap()
        );
        rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap()
    }
    fn frame(value: u8) -> Frame {
        Frame {
            width: 1,
            height: 1,
            rgba: vec![value, 0, 0, 255],
        }
    }
    #[test]
    fn latest_frame_is_bounded_and_consumed_once() {
        let mut e = Engine::default();
        let p = begin(&mut e);
        for n in 0..100 {
            p.publish(frame(n)).unwrap();
        }
        assert_eq!(e.snapshot(1).unwrap().replaced, 99);
        assert_eq!(e.drain().pop().unwrap().1.unwrap().rgba[0], 99);
        assert!(e.snapshot(1).unwrap().mailbox_ms.is_some());
        assert!(e.drain().is_empty());
        e.stop(1);
    }
    #[test]
    fn clearing_departed_target_clears_both_preview_and_recording() {
        let mut e=Engine::default();let p=begin(&mut e);
        p.publish(frame(7)).unwrap();p.game(Some(GameProcess{pid:77,start_time:1,name:"game".into()}));
        let reader=e.reader();assert!(reader.game().is_some());
        p.clear("Waiting for a new game");
        assert!(reader.game().is_none());assert!(reader.latest()[0].1.is_none());
        assert!(e.drain().pop().unwrap().1.is_none());
        p.publish(frame(8)).unwrap();assert_eq!(reader.latest()[0].1.as_ref().unwrap().rgba[0],8);
        e.stop(1);
    }
    #[test]
    fn stop_rejects_late_frames_and_reconnect_is_isolated() {
        let mut e = Engine::default();
        let old = begin(&mut e);
        let game = GameProcess {
            pid: 42,
            start_time: 100,
            name: "test game".into(),
        };
        old.game(Some(game.clone()));
        old.publish(frame(1)).unwrap();
        assert_eq!(e.snapshot(1).unwrap().game, Some(game.clone()));
        e.stop(1);
        old.game(Some(game));
        old.publish(frame(2)).unwrap();
        assert!(e.snapshot(1).unwrap().game.is_none());
        assert!(e.drain().pop().unwrap().1.is_none());
        let deadline = Instant::now() + std::time::Duration::from_secs(2);
        while e.running(1) && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(!e.running(1));
        let new = begin(&mut e);
        assert!(e.snapshot(1).unwrap().game.is_none());
        new.publish(frame(3)).unwrap();
        old.publish(frame(4)).unwrap();
        assert_eq!(e.drain().pop().unwrap().1.unwrap().rgba[0], 3);
        e.stop(1);
    }
    #[test]
    fn recording_reader_is_independent_and_follows_reconnect() {
        let mut e = Engine::default();
        let reader = e.reader();
        let old = begin(&mut e);
        old.publish(frame(7)).unwrap();
        let preview = e.drain().pop().unwrap().1.unwrap();
        let recording = reader.latest().pop().unwrap().1.unwrap();
        assert!(Arc::ptr_eq(&preview, &recording));
        assert!(e.drain().is_empty());
        assert_eq!(reader.latest().pop().unwrap().1.unwrap().rgba[0], 7);
        e.stop(1);
        assert!(reader.latest().pop().unwrap().1.is_none());
        assert!(reader.game().is_none());
        let deadline = Instant::now() + std::time::Duration::from_secs(2);
        while e.running(1) && Instant::now() < deadline {
            std::thread::yield_now();
        }
        let new = begin(&mut e);
        new.publish(frame(9)).unwrap();
        old.publish(frame(8)).unwrap();
        assert_eq!(reader.latest().pop().unwrap().1.unwrap().rgba[0], 9);
        e.stop(1);
    }
    #[test]
    fn duplicate_start_and_invalid_frames_are_rejected() {
        let mut e = Engine::default();
        let p = begin(&mut e);
        let (tx, _) = mpsc::channel();
        assert!(
            !e.start(1, Box::new(Controlled { started: tx }), Options::default())
                .unwrap()
        );
        assert!(
            p.publish(Frame {
                width: 2,
                height: 2,
                rgba: vec![0; 4]
            })
            .is_err()
        );
        assert_eq!(e.snapshot(1).unwrap().received, 0);
        e.stop(1);
    }
}
