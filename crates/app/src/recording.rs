//! Live canvas + Track 1 recording. Capture, mixing and UI never write to an
//! encoder pipe. Fixed-rate writers share a monotonic clock; missed video is
//! repeated and missing audio is silence, so a stall never shortens one stream.
mod encoding;
pub(crate) mod renderer;
use anyhow::{Context, Result, bail, ensure};
use auroracast_core::{Canvas, Id, Project};
use auroracast_gpu::Compositor;
#[cfg(test)]
use encoding::launch;
use encoding::select;
use rosorah_capture::{Frame, FrameReader};
use std::{
    collections::{BTreeMap, VecDeque},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const RATE: u64 = 48_000;
const BLOCK: u64 = 480;
const LEAD: Duration = Duration::from_millis(60);
const MAX_LAG: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Starting,
    Recording,
    Finishing,
    Finished,
}
#[derive(Clone)]
pub struct View {
    pub render_ms:f32,
    pub missed_frames:u64,
    pub queue_drops:u64,
    pub encoder_lag_ms:u64,
    pub phase: Phase,
    pub elapsed: Duration,
    pub encoder: String,
    pub details: String,
    pub path: PathBuf,
    pub result: Option<Result<PathBuf, String>>,
}
struct Video {
    at: Duration,
    /// The frame as the encoder takes it (NV12, or RGBA for sizes the graphics card can't convert).
    data: Arc<Vec<u8>>,
}
struct Pcm {
    start: u64,
    samples: Vec<f32>,
}
#[derive(Default)]
struct AudioQueue {
    blocks: VecDeque<Pcm>,
    next: Option<u64>,
}
struct Shared {
    clock: Mutex<Option<Instant>>,
    end: Mutex<Option<Duration>>,
    abort: AtomicBool,
    error: Mutex<Option<String>>,
    video: Mutex<VecDeque<Video>>,
    audio: Mutex<AudioQueue>,
    video_progress: AtomicU64,
    audio_progress: AtomicU64,
    view: Mutex<View>,
    render_ns:AtomicU64,missed_frames:AtomicU64,queue_drops:AtomicU64,
    fps: u32,
    audio_channels: usize,
    /// The mixer tracks this output carries, in order (two channels each): Track 1 alone, all
    /// six for a multitrack recording, or Track 1 and Twitch's VOD track for a stream.
    track_map: Vec<usize>,
    /// Each carried track's name, for the file's track titles.
    track_titles: Vec<String>,
    video_capacity: usize,
}
impl Shared {
    fn origin(&self) -> Option<Instant> {
        *self.clock.lock().unwrap()
    }
    fn stop(&self) {
        let mut end = self.end.lock().unwrap();
        if end.is_none() {
            *end = Some(self.origin().map_or(Duration::ZERO, |t| t.elapsed()));
            self.view.lock().unwrap().phase = Phase::Finishing;
        }
    }
    fn fail(&self, error: impl ToString) {
        self.error
            .lock()
            .unwrap()
            .get_or_insert_with(|| error.to_string());
        self.stop();
    }
    fn video_limit(&self) -> Option<u64> {
        self.end
            .lock()
            .unwrap()
            .map(|end| units_up(end, self.fps as u64).max(1))
    }
    fn audio_limit(&self) -> Option<u64> {
        self.video_limit().map(|n| n * RATE / self.fps as u64)
    }
    fn wait_for(&self, at: Duration) -> bool {
        let target = self.origin().unwrap() + at + LEAD;
        while Instant::now() < target {
            if self.abort.load(Ordering::Relaxed) {
                return false;
            }
            // Finishing can drain immediately: no producer will supply new video.
            if self.end.lock().unwrap().is_some() {
                break;
            }
            thread::sleep((target - Instant::now().min(target)).min(Duration::from_millis(4)));
        }
        !self.abort.load(Ordering::Relaxed)
    }
}
/// Pictures the writer queue holds. Every picture waits `LEAD` before it's written, so the queue
/// must hold that many frames and a few spare; sized by memory alone it held 3 at 1080p60, and
/// the oldest were thrown away before their turn, repeating a frame and skipping the next.
fn video_capacity(canvas: &Canvas) -> usize {
    let in_lead = units_up(LEAD, u64::from(canvas.fps.max(1))) as usize;
    let by_memory = 32 * 1024 * 1024 / (canvas.width as usize * canvas.height as usize * 4).max(1);
    by_memory.max(in_lead + 3).clamp(2, 16)
}
fn units(at: Duration, rate: u64) -> u64 {
    (at.as_nanos() * rate as u128 / 1_000_000_000) as u64
}
fn units_up(at: Duration, rate: u64) -> u64 {
    (at.as_nanos() * rate as u128).div_ceil(1_000_000_000) as u64
}
fn time_of(index: u64, rate: u64) -> Duration {
    Duration::from_nanos((index as u128 * 1_000_000_000 / rate as u128) as u64)
}

/// A cheap, bounded outlet from the existing mixer. It never waits for FFmpeg.
#[derive(Default)]
struct SceneMix {
    revision: Option<u64>,
    channels: Arc<[auroracast_core::AudioChannel]>,
}
impl SceneMix {
    fn channels(
        &mut self,
        revision: u64,
        project: &Project,
        canvas: Id,
    ) -> Arc<[auroracast_core::AudioChannel]> {
        if self.revision != Some(revision) {
            self.channels = project
                .canvases
                .iter()
                .find(|c| c.id == canvas)
                .map(|c| project.audio_for_scene(c.scene))
                .unwrap_or_default()
                .into();
            self.revision = Some(revision);
        }
        self.channels.clone()
    }
}
#[derive(Clone)]
pub struct AudioOutlet(
    Arc<Shared>,
    Arc<Mutex<(u64, Project)>>,
    Id,
    Arc<Mutex<SceneMix>>,
);
impl AudioOutlet {
    pub fn channels(&self) -> Arc<[auroracast_core::AudioChannel]> {
        let p = self.1.lock().unwrap();
        self.3.lock().unwrap().channels(p.0, &p.1, self.2)
    }

    pub fn tracks(&self) -> usize {
        self.0.audio_channels / 2
    }
    /// The mixer track behind each of this output's tracks.
    pub fn track_map(&self) -> &[usize] {
        &self.0.track_map
    }
    pub fn push_tracks(&self, at: Instant, tracks: &[Vec<f32>]) {
        let count = self.tracks();
        let frames = tracks.first().map_or(0, |v| v.len() / 2);
        let mut pcm = Vec::with_capacity(frames * count * 2);
        for i in 0..frames {
            for t in 0..count {
                pcm.push(
                    tracks
                        .get(t)
                        .and_then(|p| p.get(i * 2))
                        .copied()
                        .unwrap_or(0.),
                );
                pcm.push(
                    tracks
                        .get(t)
                        .and_then(|p| p.get(i * 2 + 1))
                        .copied()
                        .unwrap_or(0.),
                );
            }
        }
        self.push_interleaved(at, pcm);
    }
    pub fn push(&self, at: Instant, pcm: Vec<f32>) {
        if self.tracks() == 1 {
            self.push_interleaved(at, pcm);
        } else {
            self.push_tracks(at, &[pcm]);
        }
    }
    fn push_interleaved(&self, at: Instant, mut pcm: Vec<f32>) {
        let s = &self.0;
        let Some(origin) = s.origin() else {
            return;
        };
        if s.abort.load(Ordering::Relaxed) || s.end.lock().unwrap().is_some() {
            return;
        }
        let observed = if at >= origin {
            units(at - origin, RATE)
        } else {
            let trim = units_up(origin - at, RATE) as usize * s.audio_channels;
            if trim >= pcm.len() {
                return;
            }
            pcm.drain(..trim);
            0
        };
        let mut audio = s.audio.lock().unwrap();
        // Keep contiguous samples through ordinary scheduler jitter. A real
        // mixer interruption gets a timestamped gap instead of cumulative drift.
        let start = audio
            .next
            .filter(|n| n.abs_diff(observed) <= BLOCK)
            .unwrap_or(observed);
        audio.next = Some(start + pcm.len() as u64 / s.audio_channels as u64);
        audio.blocks.push_back(Pcm {
            start,
            samples: pcm,
        });
        while audio.blocks.len() > 100 {
            audio.blocks.pop_front();
        }
    }
}
impl AudioQueue {
    #[cfg(test)]
    fn read(&mut self, start: u64, frames: u64) -> Vec<f32> {
        self.read_channels(start, frames, 2)
    }
    fn read_channels(&mut self, start: u64, frames: u64, channels: usize) -> Vec<f32> {
        let end = start + frames;
        let mut out = vec![0.; frames as usize * channels];
        for b in &self.blocks {
            let a = start.max(b.start);
            let z = end.min(b.start + b.samples.len() as u64 / channels as u64);
            if z > a {
                let src = (a - b.start) as usize * channels;
                let dst = (a - start) as usize * channels;
                let len = (z - a) as usize * channels;
                out[dst..dst + len].copy_from_slice(&b.samples[src..src + len]);
            }
        }
        while self
            .blocks
            .front()
            .is_some_and(|b| b.start + b.samples.len() as u64 / channels as u64 <= end)
        {
            self.blocks.pop_front();
        }
        out
    }
}

pub(crate) fn output_path(folder: &Path) -> Result<PathBuf> {
    ensure!(
        !folder.as_os_str().is_empty(),
        "Choose a recording folder in Settings"
    );
    let folder = std::path::absolute(folder)?;
    std::fs::create_dir_all(&folder).context("Cannot create recording folder")?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    Ok(folder.join(format!("Rosorah-{stamp}.mkv")))
}
enum Output {
    Recording(PathBuf),
    Streaming(crate::streaming::Setup),
    /// A virtual camera device (Linux).
    Camera(String),
    /// The Windows camera: Rosorah converts frames itself and hands them straight over.
    #[cfg(windows)]
    WindowsCamera(rosorah_windows::virtual_camera::Sink),
    /// An NDI sender on the network, with the stream mix as its sound.
    Ndi(String),
    /// A Spout sender for apps on this PC (on Linux, Windows apps in Wine/Proton).
    Spout(String),
}
impl Output {
    fn path(&self) -> PathBuf {
        match self {
            Self::Recording(path) => path.clone(),
            Self::Streaming(setup) => setup.recording.clone().unwrap_or_default(),
            Self::Camera(_) => PathBuf::new(),
            #[cfg(windows)]
            Self::WindowsCamera(_) => PathBuf::new(),
            Self::Ndi(_) | Self::Spout(_) => PathBuf::new(),
        }
    }
}

pub struct Recording {
    shared: Arc<Shared>,
    project: Arc<Mutex<(u64, Project)>>,
    worker: Option<JoinHandle<()>>,
    pub canvas: Canvas,
}
impl Recording {
    pub fn start(
        project: &Project,
        canvas: usize,
        frames: FrameReader,
        folder: &Path,
    ) -> Result<Self> {
        Self::start_output(
            project,
            canvas,
            frames,
            Output::Recording(output_path(folder)?),
        )
    }
    pub fn start_stream(
        project: &Project,
        canvas: usize,
        frames: FrameReader,
        setup: crate::streaming::Setup,
    ) -> Result<Self> {
        Self::start_output(project, canvas, frames, Output::Streaming(setup))
    }
    /// Sends the canvas to a virtual camera until stopped.
    #[cfg_attr(windows, allow(dead_code))]
    pub fn start_camera(project: &Project, canvas: usize, frames: FrameReader, device: String) -> Result<Self> {
        Self::start_output(project, canvas, frames, Output::Camera(device))
    }
    #[cfg(windows)]
    pub fn start_windows_camera(project: &Project, canvas: usize, frames: FrameReader, sink: rosorah_windows::virtual_camera::Sink) -> Result<Self> {
        Self::start_output(project, canvas, frames, Output::WindowsCamera(sink))
    }
    /// Sends the canvas to the network as an NDI sender until stopped.
    pub fn start_ndi(project: &Project, canvas: usize, frames: FrameReader, name: String) -> Result<Self> {
        Self::start_output(project, canvas, frames, Output::Ndi(name))
    }
    /// Shares the canvas with apps on this PC as a Spout sender until stopped.
    pub fn start_spout(project: &Project, canvas: usize, frames: FrameReader, name: String) -> Result<Self> {
        Self::start_output(project, canvas, frames, Output::Spout(name))
    }
    fn start_output(
        project: &Project,
        canvas: usize,
        frames: FrameReader,
        output: Output,
    ) -> Result<Self> {
        project.validate()?;
        let canvas = project
            .canvases
            .get(canvas)
            .context("Choose a canvas")?
            .clone();
        ensure!(
            canvas.width.is_multiple_of(2) && canvas.height.is_multiple_of(2),
            "Output requires even canvas width and height (Settings > Video & recording)"
        );
        let path = output.path();
        let track_map: Vec<usize> = match &output {
            Output::Recording(_) if project.preferences.multitrack_recording => (0..6).collect(),
            Output::Streaming(setup) => setup.tracks.clone(),
            _ => vec![0],
        };
        let shared = Arc::new(Shared {
            clock: Mutex::new(None),
            end: Mutex::new(None),
            abort: AtomicBool::new(false),
            error: Mutex::new(None),
            video: Mutex::new(VecDeque::new()),
            audio: Mutex::default(),
            video_progress: AtomicU64::new(0),
            audio_progress: AtomicU64::new(0),
            view: Mutex::new(View {render_ms:0.,missed_frames:0,queue_drops:0,encoder_lag_ms:0,
                phase: Phase::Starting,
                elapsed: Duration::ZERO,
                encoder: String::new(),
                details: String::new(),
                path: path.clone(),
                result: None,
            }),
            render_ns:AtomicU64::new(0),missed_frames:AtomicU64::new(0),queue_drops:AtomicU64::new(0),
            fps: canvas.fps,
            audio_channels: track_map.len() * 2,
            track_titles: track_map.iter().map(|&t| project.preferences.track_name(t)).collect(),
            track_map,
            video_capacity: video_capacity(&canvas),
        });
        let project = Arc::new(Mutex::new((0, project.clone())));
        let (s, p, c) = (shared.clone(), project.clone(), canvas.clone());
        let worker = thread::Builder::new()
            .name("rosorah-recording".into())
            .spawn(move || {
                let recovery=match &output{Output::Recording(path)=>Some(path.clone()),Output::Streaming(setup)=>setup.recording.clone(),_=>None};if let Some(path)=&recovery{crate::utilities::mark_recording(path);}
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run(&s, p, c, frames, &output)
                }))
                .unwrap_or_else(|_| Err(anyhow::anyhow!("Recording worker stopped unexpectedly")))
                .map(|_| path.clone())
                .map_err(|e| format!("{e:#}"));
                if let Some(path)=&recovery{crate::utilities::finish_recording(path,result.is_ok());}
                let mut v = s.view.lock().unwrap();
                v.phase = Phase::Finished;
                v.result = Some(result);
            })?;
        Ok(Self {
            shared,
            project,
            worker: Some(worker),
            canvas,
        })
    }
    pub fn outlet(&self) -> AudioOutlet {
        AudioOutlet(
            self.shared.clone(),
            self.project.clone(),
            self.canvas.id,
            Arc::default(),
        )
    }
    pub fn update_project(&self, project: &Project) {
        renderer::update_project(project);
        let mut p = self.project.lock().unwrap();
        p.0 += 1;
        p.1 = project.clone();
    }
    pub fn stop(&self) {
        self.shared.stop();
    }
    pub fn view(&self) -> View {
        let elapsed = self
            .shared
            .end
            .lock()
            .unwrap()
            .unwrap_or_else(|| self.shared.origin().map_or(Duration::ZERO, |t| t.elapsed()));
        let mut view = self.shared.view.lock().unwrap().clone();
        view.elapsed = elapsed;
        view.render_ms=self.shared.render_ns.load(Ordering::Relaxed)as f32/1e6;
        view.missed_frames=self.shared.missed_frames.load(Ordering::Relaxed);
        view.queue_drops=self.shared.queue_drops.load(Ordering::Relaxed);
        view.encoder_lag_ms=(elapsed.as_millis()as u64).saturating_sub(self.shared.video_progress.load(Ordering::Relaxed)*1000/self.shared.fps as u64);
        view
    }
}
impl Drop for Recording {
    fn drop(&mut self) {
        self.stop();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn video_writer(s: Arc<Shared>, mut pipe: impl Write, first: Arc<Vec<u8>>) -> Result<()> {
    let mut index = 0;
    let mut last = first;
    loop {
        if s.video_limit().is_some_and(|n| index >= n) {
            break;
        }
        let at = time_of(index, s.fps as u64);
        if !s.wait_for(at) {
            break;
        }
        // Stop may have arrived while waiting for the next frame.
        if s.video_limit().is_some_and(|n| index >= n) {
            break;
        }
        let mut taken = 0;
        {
            let mut queue = s.video.lock().unwrap();
            while queue.front().is_some_and(|v| v.at <= at) {
                last = queue.pop_front().unwrap().data;
                taken += 1;
            }
        }
        trace_writes(taken);
        pipe.write_all(&last)
            .context("Video encoder stopped accepting frames")?;
        index += 1;
        s.video_progress.store(index, Ordering::Relaxed);
    }
    Ok(())
}
fn audio_writer(s: Arc<Shared>, mut pipe: impl Write) -> Result<()> {
    let mut index = 0;
    loop {
        let frames = s
            .audio_limit()
            .map_or(BLOCK, |n| (n.saturating_sub(index)).min(BLOCK));
        if frames == 0 {
            break;
        }
        if !s.wait_for(time_of(index + frames, RATE)) {
            break;
        }
        let frames = s
            .audio_limit()
            .map_or(frames, |n| (n.saturating_sub(index)).min(frames));
        if frames == 0 {
            break;
        }
        let pcm = s
            .audio
            .lock()
            .unwrap()
            .read_channels(index, frames, s.audio_channels);
        let bytes: Vec<_> = pcm.iter().flat_map(|v| v.to_le_bytes()).collect();
        pipe.write_all(&bytes)
            .context("Audio encoder stopped accepting samples")?;
        index += frames;
        s.audio_progress.store(index, Ordering::Relaxed);
    }
    Ok(())
}
fn writer(s: Arc<Shared>, f: impl FnOnce() -> Result<()> + Send + 'static) -> JoinHandle<()> {
    thread::spawn(move || {
        if let Err(e) = f() {
            s.fail(e);
            s.abort.store(true, Ordering::Relaxed);
        }
    })
}
fn selected_project(project: &Project, canvas: &Canvas) -> Result<(Project, Canvas)> {
    let mut c = project
        .canvases
        .iter()
        .find(|c| c.id == canvas.id)
        .context("Recorded canvas was removed")?
        .clone();
    // The selected canvas identity and output format stay fixed until Stop.
    c.width = canvas.width;
    c.height = canvas.height;
    c.fps = canvas.fps;
    let mut project = project.clone();
    project.canvases = vec![c.clone()];
    Ok((project, c))
}
fn live_sources(project: &Project) -> std::collections::BTreeSet<Id> {
    project
        .canvases
        .iter()
        .filter_map(|c| project.scene(c.scene))
        .flat_map(|s| &s.layers)
        .filter(|l| l.visible)
        .filter(|l| {
            project.source(l.source).is_some_and(|s| {
                matches!(
                    s.kind,
                    auroracast_core::SourceKind::Browser { .. }
                        | auroracast_core::SourceKind::ScreenCapture
                        | auroracast_core::SourceKind::VideoCamera(_)
                        | auroracast_core::SourceKind::VideoFile(_)
                        | auroracast_core::SourceKind::Widget(_)
                        | auroracast_core::SourceKind::Ndi(_)
                        | auroracast_core::SourceKind::Spout(_)
                )
            })
        })
        .map(|l| l.source)
        // Stinger/screen overlays are drawn on top of the canvas while a transition runs.
        .chain(project.preferences.overlays.values().map(|o| o.source))
        .collect()
}
fn upload_frames(
    gpu: &mut Compositor,
    frames: impl IntoIterator<Item = (Id, Option<Arc<Frame>>)>,
    used: &std::collections::BTreeSet<Id>,
    previous: &mut BTreeMap<Id, Option<Arc<Frame>>>,
) -> Result<()> {
    previous.retain(|id, _| used.contains(id));
    let mut fresh = 0;
    let mut stale = 0;
    for (id, current) in frames.into_iter().filter(|(id, _)| used.contains(id)) {
        let same = match (previous.get(&id), &current) {
            (Some(Some(a)), Some(b)) => Arc::ptr_eq(a, b),
            (Some(None), None) => true,
            _ => false,
        };
        if same {
            stale += u32::from(id == TRACED.load(Ordering::Relaxed));
            continue;
        }
        match &current {
            Some(f) => gpu.upload(id, f.width, f.height, &f.rgba)?,
            None => gpu.upload(id, 1, 1, &[0, 0, 0, 0])?,
        }
        previous.insert(id, current);
        fresh += 1;
    }
    trace_ticks(fresh, stale);
    Ok(())
}
/// The capture the trace follows: the game capture (set when it publishes a frame).
pub(crate) static TRACED: AtomicU64 = AtomicU64::new(0);
/// `ROSORAH_CAPTURE_TRACE=1`: once a second, frames written again for want of a new picture,
/// and pictures dropped because two arrived for one frame.
fn trace_writes(taken: u32) {
    static TRACE: std::sync::Mutex<Option<(Instant, u32, u32, u32)>> = std::sync::Mutex::new(None);
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*ON.get_or_init(|| std::env::var_os("ROSORAH_CAPTURE_TRACE").is_some()) {
        return;
    }
    let mut trace = TRACE.lock().unwrap();
    let (since, written, repeated, dropped) = trace.get_or_insert((Instant::now(), 0, 0, 0));
    *written += 1;
    *repeated += u32::from(taken == 0);
    *dropped += taken.saturating_sub(1);
    if since.elapsed() >= Duration::from_secs(1) {
        eprintln!("writer trace: {written} frames written, {repeated} repeated, {dropped} pictures dropped");
        *trace = None;
    }
}
/// `ROSORAH_CAPTURE_TRACE=1`: once a second, how many output ticks had a new capture frame.
fn trace_ticks(fresh: u32, stale: u32) {
    static TRACE: std::sync::Mutex<Option<(Instant, u32, u32, u32)>> = std::sync::Mutex::new(None);
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*ON.get_or_init(|| std::env::var_os("ROSORAH_CAPTURE_TRACE").is_some()) {
        return;
    }
    let mut trace = TRACE.lock().unwrap();
    let (since, ticks, with_new, game_stale) = trace.get_or_insert((Instant::now(), 0, 0, 0));
    *ticks += 1;
    *with_new += u32::from(fresh > 0);
    *game_stale += stale;
    if since.elapsed() >= Duration::from_secs(1) {
        eprintln!("output trace: {ticks} ticks/s, {with_new} with a new capture frame, {game_stale} reusing the game's previous frame");
        *trace = None;
    }
}
fn run(
    s: &Arc<Shared>,
    project: Arc<Mutex<(u64, Project)>>,
    canvas: Canvas,
    frames: FrameReader,
    output: &Output,
) -> Result<()> {
    #[cfg(windows)]
    if let Output::WindowsCamera(sink) = output {
        return run_windows_camera(s, project, canvas, frames, sink);
    }
    if matches!(output, Output::Ndi(_) | Output::Spout(_)) {
        return run_shared(s, project, canvas, frames, output);
    }
    let path = output.path();
    let mode = project.lock().unwrap().1.preferences.recording_encoder;
    let cancelled = || s.end.lock().unwrap().is_some();
    let report = |status: &str| s.view.lock().unwrap().encoder = status.into();
    let selection = match output {
        Output::Recording(_) => select(&canvas, mode, cancelled, report),
        Output::Streaming(setup) => {
            encoding::select_stream(&canvas, mode, setup.bitrate, cancelled, report)
        }
        // A camera takes plain frames: nothing to choose.
        #[cfg(windows)]
        Output::WindowsCamera(_) => unreachable!(),
        Output::Ndi(_) | Output::Spout(_) => unreachable!(),
        Output::Camera(_) => Ok(encoding::Selection { codecs: encoding::Codecs::software("rawvideo", "pcm_f32le"), details: "Virtual camera (no encoding)".into() }),
    }?;
    let codec = selection.codecs;
    s.view.lock().unwrap().details = selection.details.clone();
    crate::capture_log::event(format!("Recording encoder: {}", selection.details));
    ensure!(
        s.end.lock().unwrap().is_none(),
        "Recording cancelled before it started"
    );
    // Frames come from the shared renderer, already in the encoder's picture format.
    let format = renderer::Format::for_encoder(&canvas);
    let initial = project.lock().unwrap().1.clone();
    selected_project(&initial, &canvas)?;
    let subscription = renderer::Subscription::new(&initial, frames, &canvas, format, canvas.fps);
    let first = subscription.first(Duration::from_secs(20))?.data;
    let nv12 = matches!(format, renderer::Format::Nv12(_));
    // Cancel during setup does not create an empty recording.
    if s.end.lock().unwrap().is_some() {
        bail!("Recording cancelled before it started");
    }
    let (mut encoder, video_pipe, audio_pipe) = match output {
        Output::Recording(path) => {
            encoding::launch_tracks(&canvas, &codec, path, &s.track_titles, nv12)
        }
        Output::Streaming(_) => encoding::launch_stream(&canvas, &codec, &s.track_titles, nv12),
        Output::Camera(device) => encoding::launch_camera(&canvas, device, nv12),
        #[cfg(windows)]
        Output::WindowsCamera(_) => unreachable!(),
        Output::Ndi(_) | Output::Spout(_) => unreachable!(),
    }?;
    let hub = match output {
        Output::Streaming(setup) => Some(crate::streaming::Hub::start(
            encoder.0.stdout.take().unwrap(),
            setup,
        )),
        _ => None,
    };
    let mut stderr = encoder.0.stderr.take().unwrap();
    let log = Arc::new(Mutex::new(VecDeque::new()));
    let logs = log.clone();
    let logger = thread::spawn(move || {
        let mut bytes = [0; 4096];
        while let Ok(n) = stderr.read(&mut bytes) {
            if n == 0 {
                break;
            }
            let mut log = logs.lock().unwrap();
            log.extend(&bytes[..n]);
            while log.len() > 16_384 {
                log.pop_front();
            }
        }
    });
    let start = Instant::now();
    *s.clock.lock().unwrap() = Some(start);
    {
        let mut view = s.view.lock().unwrap();
        view.phase = Phase::Recording;
        view.encoder = codec.label();
    }
    let sv = s.clone();
    let sa = s.clone();
    let video = writer(s.clone(), move || video_writer(sv, video_pipe, first));
    let audio = writer(s.clone(), move || audio_writer(sa, audio_pipe));
    let mut last_picture: Option<Instant> = None;
    let mut progress = (0, 0);
    let mut progress_at = [Instant::now(); 2];
    let mut finishing = None;
    let status = loop {
        if let Some(error) = hub.as_ref().and_then(|h| h.error()) {
            s.fail(error);
            s.abort.store(true, Ordering::Relaxed);
        }
        if let Some(status) = encoder.0.try_wait()? {
            break status;
        }
        if s.abort.load(Ordering::Relaxed) {
            let _ = encoder.0.kill();
            break encoder.0.wait()?;
        }
        if s.end.lock().unwrap().is_some() {
            let since = finishing.get_or_insert_with(Instant::now);
            if since.elapsed() > Duration::from_secs(8) {
                s.fail("Encoder did not finish within 8 seconds; the partial MKV has been kept");
                s.abort.store(true, Ordering::Relaxed);
            }
        } else {
            let period = Duration::from_secs(1) / canvas.fps;
            match subscription.take(Duration::ZERO) {
                Ok(pictures) => {
                    for picture in pictures {
                        // On the output's frame grid, rounded to the nearest frame.
                        let at = picture.at.saturating_duration_since(start) + period / 2;
                        if let Some(last) = last_picture
                            && picture.at.saturating_duration_since(last) > period * 3 / 2
                        {
                            s.missed_frames.fetch_add((picture.at - last).as_nanos() as u64 / period.as_nanos() as u64 - 1, Ordering::Relaxed);
                        }
                        last_picture = Some(picture.at);
                        s.render_ns.store(picture.render.as_nanos() as u64, Ordering::Relaxed);
                        let mut queue = s.video.lock().unwrap();
                        queue.push_back(Video { at: time_of(units(at, canvas.fps as u64), canvas.fps as u64), data: picture.data });
                        while queue.len() > s.video_capacity {
                            queue.pop_front();
                            s.queue_drops.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
                Err(e) => s.fail(format!("{e:#}")),
            }
        }
        let current_progress = (
            s.video_progress.load(Ordering::Relaxed),
            s.audio_progress.load(Ordering::Relaxed),
        );
        if current_progress.0 != progress.0 {
            progress_at[0] = Instant::now();
        }
        if current_progress.1 != progress.1 {
            progress_at[1] = Instant::now();
        }
        progress = current_progress;
        if finishing.is_none()
            && (progress_at.iter().any(|at| at.elapsed() > MAX_LAG)
                || start
                    .elapsed()
                    .saturating_sub(time_of(progress.0, canvas.fps as u64))
                    > MAX_LAG)
        {
            s.fail("Encoder cannot keep up. Lower the canvas resolution or FPS; the partial MKV has been kept");
            s.abort.store(true, Ordering::Relaxed);
        }
        thread::sleep(Duration::from_millis(3));
    };
    s.abort.store(true, Ordering::Relaxed);
    let _ = video.join();
    let _ = audio.join();
    let _ = logger.join();
    let relay_result = hub.map(|h| h.finish()).transpose();
    let log: Vec<_> = log.lock().unwrap().iter().copied().collect();
    if let Some(e) = s.error.lock().unwrap().clone() {
        bail!(
            "{e}\n{}\nFile: {}",
            String::from_utf8_lossy(&log),
            path.display()
        );
    }
    ensure!(
        status.success() && log.is_empty(),
        "FFmpeg failed ({status}): {}\nFile: {}",
        String::from_utf8_lossy(&log),
        path.display()
    );
    ensure!(
        s.end.lock().unwrap().is_some(),
        "Encoder ended before Stop was requested"
    );
    relay_result?;
    if matches!(output, Output::Recording(_)) {
        ensure!(
            path.metadata().is_ok_and(|m| m.len() > 512),
            "Encoder did not save a recording"
        );
    }
    Ok(())
}

/// The Windows camera's loop: the renderer draws and converts at the camera's rate and the newest
/// picture is handed straight over. No encoder and no queue, so each picture reaches apps about one
/// frame after it's drawn.
#[cfg(windows)]
fn run_windows_camera(s: &Arc<Shared>, project: Arc<Mutex<(u64, Project)>>, canvas: Canvas, frames: FrameReader, sink: &rosorah_windows::virtual_camera::Sink) -> Result<()> {
    use rosorah_windows::virtual_camera::{FPS, HEIGHT, WIDTH};
    let picture = auroracast_gpu::Nv12::fitted(canvas.width, canvas.height, WIDTH, HEIGHT).context("The camera's picture size")?;
    let initial = project.lock().unwrap().1.clone();
    selected_project(&initial, &canvas)?;
    let subscription = renderer::Subscription::new(&initial, frames, &canvas, renderer::Format::Nv12(picture), FPS);
    let start = Instant::now();
    *s.clock.lock().unwrap() = Some(start);
    {
        let mut view = s.view.lock().unwrap();
        view.phase = Phase::Recording;
        view.encoder = "Virtual camera".into();
        view.details = "Virtual camera (no encoding)".into();
    }
    while s.end.lock().unwrap().is_none() && !s.abort.load(Ordering::Relaxed) {
        // Only the newest picture matters to a camera.
        if let Some(picture) = subscription.take(Duration::from_millis(100))?.pop() {
            sink.send(&picture.data);
            s.render_ns.store(picture.render.as_nanos() as u64, Ordering::Relaxed);
            s.video_progress.fetch_add(1, Ordering::Relaxed);
        }
    }
    Ok(())
}

/// NDI and Spout outputs: the renderer's RGBA pictures handed straight over, newest only, with no
/// encoder. NDI also sends the stream mix, read from the audio queue once each block is due.
fn run_shared(s: &Arc<Shared>, project: Arc<Mutex<(u64, Project)>>, canvas: Canvas, frames: FrameReader, output: &Output) -> Result<()> {
    let initial = project.lock().unwrap().1.clone();
    selected_project(&initial, &canvas)?;
    let mut ndi = match output {
        Output::Ndi(name) => Some(crate::ndi::Sender::new(name, canvas.width, canvas.height, canvas.fps)?),
        _ => None,
    };
    let mut spout = match output {
        #[cfg(windows)]
        Output::Spout(name) => Some(rosorah_windows::spout::Sender::new(name)?),
        #[cfg(not(windows))]
        Output::Spout(name) => Some(crate::spout_bridge::Sender::new(name)?),
        _ => None,
    };
    let subscription = renderer::Subscription::new(&initial, frames, &canvas, renderer::Format::Rgba, canvas.fps);
    let start = Instant::now();
    *s.clock.lock().unwrap() = Some(start);
    {
        let mut view = s.view.lock().unwrap();
        view.phase = Phase::Recording;
        view.encoder = if ndi.is_some() { "NDI output" } else { "Spout output" }.into();
        view.details = format!("{} (no encoding)", view.encoder);
    }
    let mut sound = 0u64;
    while s.end.lock().unwrap().is_none() && !s.abort.load(Ordering::Relaxed) {
        if let Some(picture) = subscription.take(Duration::from_millis(5))?.pop() {
            if let Some(ndi) = &ndi {
                ndi.picture(&picture.data, picture.at.saturating_duration_since(start))?;
            }
            if let Some(spout) = &mut spout {
                spout.send(&picture.data, canvas.width, canvas.height)?;
            }
            s.render_ns.store(picture.render.as_nanos() as u64, Ordering::Relaxed);
            s.video_progress.fetch_add(1, Ordering::Relaxed);
        }
        if let Some(ndi) = &mut ndi {
            while start.elapsed() >= time_of(sound + BLOCK, RATE) + LEAD {
                let pcm = s.audio.lock().unwrap().read_channels(sound, BLOCK, s.audio_channels);
                ndi.sound(&pcm)?;
                sound += BLOCK;
                s.audio_progress.store(sound, Ordering::Relaxed);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn writer_queue_holds_every_picture_waiting_out_the_lead() {
        for (width, height, fps) in [(1920, 1080, 60), (2560, 1440, 60), (1080, 1920, 60), (3840, 2160, 120), (1280, 720, 30)] {
            let canvas = Canvas { id: 1, name: String::new(), width, height, fps, scene: 1 };
            let waiting = units_up(LEAD, u64::from(fps)) as usize;
            assert!(video_capacity(&canvas) > waiting, "{width}x{height}@{fps}");
        }
    }
    use encoding::Codecs;
    use std::process::Command;
    #[test]
    fn clock_rounds_stop_to_one_shared_duration() {
        for fps in [1, 24, 30, 60, 120] {
            for ms in [0, 1, 17, 999, 1001, 60_123, 3_600_001] {
                let end = Duration::from_millis(ms);
                let video = units_up(end, fps).max(1);
                let audio = video * RATE / fps;
                assert!(time_of(video, fps) >= end);
                assert!(
                    time_of(video, fps).abs_diff(time_of(audio, RATE)) < Duration::from_micros(22)
                );
            }
        }
    }
    #[test]
    fn timestamped_audio_keeps_gaps_and_trims_overlaps() {
        let mut q = AudioQueue::default();
        q.blocks.push_back(Pcm {
            start: 2,
            samples: vec![0.25; 8],
        });
        q.blocks.push_back(Pcm {
            start: 5,
            samples: vec![0.75; 4],
        });
        assert_eq!(q.read(0, 4), vec![0., 0., 0., 0., 0.25, 0.25, 0.25, 0.25]);
        assert_eq!(
            q.read(4, 4),
            vec![0.25, 0.25, 0.75, 0.75, 0.75, 0.75, 0., 0.]
        );
        assert!(q.blocks.is_empty());
    }
    struct Controlled(std::sync::mpsc::Sender<rosorah_capture::Producer>);
    impl rosorah_capture::Backend for Controlled {
        fn run(
            self: Box<Self>,
            p: rosorah_capture::Producer,
            mut stop: tokio::sync::watch::Receiver<bool>,
            _: rosorah_capture::Options,
        ) -> Result<()> {
            self.0.send(p)?;
            while !*stop.borrow_and_update() {
                thread::sleep(Duration::from_millis(2));
            }
            Ok(())
        }
    }
    fn wait_ready(recording: &Recording) -> Instant {
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            let v = recording.view();
            assert!(v.result.is_none(), "{:?}", v.result);
            if let Some(origin) = recording.shared.origin() {
                return origin;
            }
            assert!(Instant::now() < deadline, "Recorder startup timed out");
            thread::sleep(Duration::from_millis(5));
        }
    }
    fn finish(recording: &Recording) -> PathBuf {
        recording.stop();
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            if let Some(result) = recording.view().result {
                return result.unwrap();
            }
            assert!(Instant::now() < deadline, "Recorder finalization timed out");
            thread::sleep(Duration::from_millis(5));
        }
    }
    fn output_dir() -> PathBuf {
        let root = std::env::var_os("ROSORAH_RECORDING_TEST_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        root.join(format!(
            "rosorah-recording-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
    fn ffmpeg_decode(path: &Path, args: &[&str]) -> Vec<u8> {
        let output = Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(path)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    }
    #[test]
    #[ignore = "Requires FFmpeg 8+ and Vulkan; streams to local RTMP receivers"]
    fn each_destination_hears_its_own_track_and_twitch_its_vod_track() -> Result<()> {
        use crate::streaming::{Config, Destination, Stream};
        use std::process::Stdio;
        struct Listener(std::process::Child);
        impl Drop for Listener {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let receiver = |url: &str, path: &Path| -> Result<Listener> {
            Ok(Listener(
                Command::new("ffmpeg")
                    .args(["-nostdin", "-loglevel", "error", "-listen", "1", "-i", url, "-map", "0:a", "-c", "copy", "-f", "matroska", "-y"])
                    .arg(path)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()?,
            ))
        };
        let port = || -> Result<u16> { Ok(std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port()) };
        while crate::streaming::multitrack_flv().is_none() {
            thread::sleep(Duration::from_millis(50));
        }
        ensure!(crate::streaming::multitrack_flv() == Some(true), "this FFmpeg can't write multitrack FLV");
        let folder = output_dir();
        std::fs::create_dir_all(&folder)?;
        let (twitch, youtube) = (format!("rtmp://127.0.0.1:{}/live", port()?), format!("rtmp://127.0.0.1:{}/live", port()?));
        let (twitch_file, youtube_file) = (folder.join("vod-twitch.mkv"), folder.join("vod-youtube.mkv"));
        let a = receiver(&format!("{twitch}/vod-test-a"), &twitch_file)?;
        let b = receiver(&format!("{youtube}/vod-test-b"), &youtube_file)?;
        thread::sleep(Duration::from_millis(150));
        let config = Config {
            twitch: Destination { enabled: true, server: twitch, key: "vod-test-a".into(), ..Default::default() },
            // Twitch: Track 1 live and Track 3 as VOD audio; YouTube: Track 2 only.
            twitch_vod_track: Some(2),
            youtube: Destination { enabled: true, server: youtube, key: "vod-test-b".into(), audio_track: Some(1), ..Default::default() },
            bitrate: 1000,
            ..Default::default()
        };
        let mut project = Project::default();
        project.preferences.recording_encoder = auroracast_core::RecordingEncoder::Software;
        project.sources[0].kind = auroracast_core::SourceKind::ScreenCapture;
        project.scenes[0].layers.retain(|l| l.source == 1);
        (project.canvases[0].width, project.canvases[0].height) = (320, 180);
        let mut engine = rosorah_capture::Engine::default();
        let (tx, rx) = std::sync::mpsc::channel();
        engine.start(1, Box::new(Controlled(tx)), rosorah_capture::Options::default())?;
        let source = rx.recv_timeout(Duration::from_secs(2))?;
        source.publish(Frame { width: 1, height: 1, rgba: vec![0, 0, 180, 255] })?;
        let stream = Stream::start(&project, 0, engine.reader(), &config, &folder)?;
        let start = wait_ready(&stream.engine);
        let outlet = stream.engine.outlet();
        assert_eq!(outlet.track_map(), [0, 1, 2]);
        let tone = |hz: f32, block: u64| -> Vec<f32> { (0..BLOCK).flat_map(|n| { let v = (std::f32::consts::TAU * hz * (block * BLOCK + n) as f32 / RATE as f32).sin() * 0.5; [v, v] }).collect() };
        for block in 0..500u64 {
            let at = start + Duration::from_millis(block * 10);
            if let Some(wait) = at.checked_duration_since(Instant::now()) {
                thread::sleep(wait);
            }
            outlet.push_tracks(at, &[tone(440., block), tone(660., block), tone(880., block)]);
            let _ = engine.drain();
        }
        stream.stop();
        finish(&stream.engine);
        thread::sleep(Duration::from_millis(500));
        drop((a, b));
        let audio_streams = |path: &Path| -> usize {
            let out = Command::new("ffprobe").args(["-v", "error", "-select_streams", "a", "-show_entries", "stream=index", "-of", "csv=p=0"]).arg(path).output().unwrap();
            String::from_utf8_lossy(&out.stdout).lines().filter(|l| !l.trim().is_empty()).count()
        };
        // The tone's pitch from its zero crossings, over the second second.
        let pitch = |path: &Path, track: usize| -> f32 {
            let pcm = ffmpeg_decode(path, &["-map", &format!("0:a:{track}"), "-ss", "1", "-t", "1", "-ac", "1", "-ar", "48000", "-f", "f32le", "pipe:1"]);
            let samples: Vec<f32> = pcm.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
            samples.windows(2).filter(|w| w[0] <= 0. && w[1] > 0.).count() as f32 * 48000. / samples.len().max(1) as f32
        };
        assert_eq!(audio_streams(&twitch_file), 2, "Twitch gets its track and the VOD track");
        assert_eq!(audio_streams(&youtube_file), 1, "YouTube gets its own track only");
        assert!((pitch(&twitch_file, 0) - 440.).abs() < 15., "Twitch's Track 1 {}", pitch(&twitch_file, 0));
        assert!((pitch(&twitch_file, 1) - 880.).abs() < 15., "Twitch's VOD track {}", pitch(&twitch_file, 1));
        assert!((pitch(&youtube_file, 0) - 660.).abs() < 15., "YouTube's Track 2 {}", pitch(&youtube_file, 0));
        Ok(())
    }
    #[test]
    #[ignore = "Requires FFmpeg/ffprobe and Vulkan; exercises the real background compositor and muxer"]
    fn a_180_fps_capture_records_a_new_picture_in_every_1080p60_frame() -> Result<()> {
        let folder = output_dir();
        let mut project = Project::default();
        project.sources[0].kind = auroracast_core::SourceKind::ScreenCapture;
        project.scenes[0].layers.retain(|l| l.source == 1);
        (project.canvases[0].width, project.canvases[0].height, project.canvases[0].fps) = (1920, 1080, 60);
        let mut engine = rosorah_capture::Engine::default();
        let (tx, rx) = std::sync::mpsc::channel();
        engine.start(1, Box::new(Controlled(tx)), rosorah_capture::Options::default())?;
        let source = rx.recv_timeout(Duration::from_secs(2))?;
        // Eight grey levels far apart, stepping one level per captured frame like a 180 fps game.
        let level = |n: u64| (16 + (n % 8) * 30) as u8;
        source.publish(Frame { width: 1, height: 1, rgba: vec![level(0), level(0), level(0), 255] })?;
        let recording = Recording::start(&project, 0, engine.reader(), &folder)?;
        let start = wait_ready(&recording);
        let mut n = 0;
        while start.elapsed() < Duration::from_secs(4) {
            n += 1;
            let v = level(n);
            // Full-size frames, as a game capture delivers them.
            source.publish(Frame { width: 1920, height: 1080, rgba: [v, v, v, 255].repeat(1920 * 1080) })?;
            let _ = engine.drain();
            thread::sleep(Duration::from_micros(5555));
        }
        let path = finish(&recording);
        engine.stop(1);
        let pixels = ffmpeg_decode(&path, &["-map", "0:v:0", "-vf", "scale=1:1", "-fps_mode", "passthrough", "-pix_fmt", "gray", "-f", "rawvideo", "pipe:1"]);
        // Skip the first half second while the pipeline settles.
        let levels: Vec<u64> = pixels[30..].iter().map(|&p| (u64::from(p).saturating_sub(1) / 30).min(7)).collect();
        let repeats = levels.windows(2).filter(|w| w[0] == w[1]).count();
        assert!(repeats * 100 <= levels.len(), "{repeats} of {} recorded frames repeated the one before", levels.len());
        Ok(())
    }
    #[test]
    #[ignore = "Requires FFmpeg/ffprobe and Vulkan; exercises the real background compositor and muxer"]
    fn live_recording_keeps_audio_video_timing_without_ui() -> Result<()> {
        let folder = output_dir();
        let sizes = if std::env::var_os("ROSORAH_RECORDING_TEST_LARGE").is_some() {
            [(1920, 1080, 30), (1080, 1920, 30)]
        } else {
            [(160, 90, 30), (90, 160, 60)]
        };
        for (width, height, fps) in sizes {
            let mut project = Project::default();
            project.sources[0].kind = auroracast_core::SourceKind::ScreenCapture;
            project.scenes[0].layers.retain(|l| l.source == 1);
            project.canvases[0].width = width;
            project.canvases[0].height = height;
            project.canvases[0].fps = fps;
            let mut engine = rosorah_capture::Engine::default();
            let (tx, rx) = std::sync::mpsc::channel();
            engine.start(
                1,
                Box::new(Controlled(tx)),
                rosorah_capture::Options::default(),
            )?;
            let source = rx.recv_timeout(Duration::from_secs(2))?;
            let set_color = |color: [u8; 4]| {
                source.publish(Frame {
                    width: 1,
                    height: 1,
                    rgba: color.to_vec(),
                })
            };
            set_color([0, 0, 180, 255])?;
            let recording = Recording::start(&project, 0, engine.reader(), &folder)?;
            let start = wait_ready(&recording);
            let outlet = recording.outlet();
            let duration = std::env::var("ROSORAH_RECORDING_TEST_SECONDS")
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(3)
                .max(3);
            for block in 0..duration * 100 {
                let at = start + Duration::from_millis(block * 10);
                if let Some(wait) = at.checked_duration_since(Instant::now()) {
                    thread::sleep(wait);
                }
                // A paired flash/tone at one second, followed by a real input gap.
                if block == 100 {
                    set_color([255, 255, 255, 255])?;
                }
                if block == 120 {
                    set_color([0, 0, 180, 255])?;
                }
                if !(150..180).contains(&block) {
                    let tone = (100..120).contains(&block);
                    let pcm = (0..BLOCK)
                        .flat_map(|n| {
                            let value = if tone {
                                (std::f32::consts::TAU * 660. * (block * BLOCK + n) as f32
                                    / RATE as f32)
                                    .sin()
                                    * 0.5
                            } else {
                                0.
                            };
                            [value, value]
                        })
                        .collect();
                    outlet.push(at, pcm);
                }
                if block == 200 {
                    // Editing another canvas must not change the recorded format.
                    project.canvases[1].width = 400;
                    recording.update_project(&project);
                }
                // The preview consumes its copy while the recorder runs separately.
                let _ = engine.drain();
            }
            if let Some(wait) =
                (start + Duration::from_secs(duration)).checked_duration_since(Instant::now())
            {
                thread::sleep(wait);
            }
            let path = finish(&recording);
            engine.stop(1);
            let probe = Command::new("ffprobe")
                .args([
                    "-v",
                    "error",
                    "-show_streams",
                    "-show_format",
                    "-of",
                    "json",
                ])
                .arg(&path)
                .output()?;
            ensure!(probe.status.success(), "ffprobe failed");
            let probe: serde_json::Value = serde_json::from_slice(&probe.stdout)?;
            let streams = probe["streams"].as_array().unwrap();
            assert_eq!(streams.len(), 2);
            assert_eq!(streams[0]["width"], width);
            assert_eq!(streams[0]["height"], height);
            assert_eq!(streams[1]["sample_rate"], "48000");
            assert_eq!(streams[1]["channels"], 2);
            let seconds: f64 = probe["format"]["duration"].as_str().unwrap().parse()?;
            assert!(
                (seconds - duration as f64).abs() < 0.12,
                "duration {seconds}"
            );
            let pixels = ffmpeg_decode(
                &path,
                &[
                    "-map",
                    "0:v:0",
                    "-vf",
                    "scale=1:1",
                    "-fps_mode",
                    "passthrough",
                    "-pix_fmt",
                    "rgb24",
                    "-f",
                    "rawvideo",
                    "pipe:1",
                ],
            );
            let white = pixels
                .as_chunks::<3>()
                .0
                .iter()
                .position(|p| p[0] > 220 && p[1] > 220)
                .context("No recorded flash")?;
            let video_start: f64 = streams[0]["start_time"].as_str().unwrap().parse()?;
            let flash_time = video_start + white as f64 / fps as f64;
            let bytes = ffmpeg_decode(
                &path,
                &[
                    "-map", "0:a:0", "-f", "f32le", "-ac", "2", "-ar", "48000", "pipe:1",
                ],
            );
            let samples: Vec<f32> = bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|v| f32::from_le_bytes(*v))
                .collect();
            let onset = samples
                .as_chunks::<2>()
                .0
                .iter()
                .position(|v| v[0].abs() > 0.2)
                .context("No recorded tone")?;
            let audio_start: f64 = streams[1]["start_time"].as_str().unwrap().parse()?;
            let tone_time = audio_start + onset as f64 / RATE as f64;
            assert!(
                (flash_time - tone_time).abs() < 0.10,
                "flash={flash_time}, tone={tone_time}"
            );
            assert!((tone_time - 1.).abs() < 0.06, "tone onset {tone_time}");
            assert!(
                samples[(RATE as usize * 16 / 10) * 2..(RATE as usize * 17 / 10) * 2]
                    .iter()
                    .all(|v| v.abs() < 0.001)
            );
            let video_seconds = pixels.len() as f64 / 3. / fps as f64;
            let audio_seconds = samples.len() as f64 / 2. / RATE as f64;
            assert!(
                (video_seconds - audio_seconds).abs() < 0.05,
                "A/V endpoints: {video_seconds}, {audio_seconds}"
            );
            // An outlet retained by the mixer after stop cannot accumulate samples.
            let len = recording.shared.audio.lock().unwrap().blocks.len();
            outlet.push(Instant::now(), vec![0.5; 960]);
            assert_eq!(recording.shared.audio.lock().unwrap().blocks.len(), len);
            println!(
                "PASS {}: {width}x{height}@{fps}, duration={seconds:.3}s, flash={flash_time:.3}s, tone={tone_time:.3}s",
                path.display()
            );
        }
        Ok(())
    }
    #[test]
    #[ignore = "Requires FFmpeg/ffprobe and Vulkan"]
    fn short_silent_recording_and_drop_finalize() -> Result<()> {
        let folder = output_dir();
        let mut project = Project::default();
        project.canvases[0].width = 160;
        project.canvases[0].height = 90;
        let recording = Recording::start(&project, 0, FrameReader::default(), &folder)?;
        wait_ready(&recording);
        let path = recording.view().path;
        thread::sleep(Duration::from_millis(70));
        drop(recording); // The application-exit path must produce a playable file.
        let probe = Command::new("ffprobe")
            .args(["-v", "error", "-show_streams", "-of", "json"])
            .arg(&path)
            .output()?;
        assert!(probe.status.success());
        let p: serde_json::Value = serde_json::from_slice(&probe.stdout)?;
        assert_eq!(p["streams"].as_array().unwrap().len(), 2);
        let bytes = ffmpeg_decode(&path, &["-map", "0:a:0", "-f", "f32le", "pipe:1"]);
        assert!(!bytes.is_empty());
        assert!(bytes.iter().all(|b| *b == 0));
        println!("PASS short silent recording: {}", path.display());
        Ok(())
    }
    #[test]
    #[ignore = "Requires FFmpeg"]
    fn mpeg4_fallback_does_not_overwrite_existing_files() -> Result<()> {
        let folder = output_dir();
        std::fs::create_dir_all(&folder)?;
        let path = folder.join("keep.mkv");
        std::fs::write(&path, b"keep this file")?;
        let canvas = Canvas {
            width: 16,
            height: 16,
            ..Project::default().canvases[0].clone()
        };
        assert!(launch(&canvas, &Codecs::software("mpeg4", "aac"), &path).is_err());
        assert_eq!(std::fs::read(&path)?, b"keep this file");
        let path = folder.join("fallback.mkv");
        let (mut encoder, mut video, mut audio) =
            launch(&canvas, &Codecs::software("mpeg4", "aac"), &path)?;
        video.write_all(&vec![0; 16 * 16 * 4])?;
        audio.write_all(&vec![0; 960 * 4])?;
        drop(video);
        drop(audio);
        assert!(encoder.0.wait()?.success());
        assert!(path.metadata()?.len() > 100);
        let path = folder.join("vp8-flac.mkv");
        let (mut encoder, mut video, mut audio) =
            launch(&canvas, &Codecs::software("libvpx", "flac"), &path)?;
        video.write_all(&vec![0; 16 * 16 * 4])?;
        audio.write_all(&vec![0; 960 * 4])?;
        drop(video);
        drop(audio);
        assert!(encoder.0.wait()?.success());
        let output = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_name",
                "-of",
                "json",
            ])
            .arg(&path)
            .output()?;
        let p: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(p["streams"][0]["codec_name"], "vp8");
        assert_eq!(p["streams"][1]["codec_name"], "flac");
        Ok(())
    }
    /// Sends a red canvas to a loopback camera and reads a frame back:
    /// ROSORAH_TEST_CAMERA=/dev/videoN cargo test -- --ignored virtual_camera_delivers_frames
    #[test]
    #[ignore = "Requires a v4l2loopback device, FFmpeg and Vulkan"]
    fn virtual_camera_delivers_frames_without_audio() -> Result<()> {
        let device = std::env::var("ROSORAH_TEST_CAMERA").context("Set ROSORAH_TEST_CAMERA")?;
        let mut project = Project::default();
        project.sources[0].kind = auroracast_core::SourceKind::ScreenCapture;
        project.scenes[0].layers.retain(|l| l.source == 1);
        project.canvases[0].width = 320;
        project.canvases[0].height = 180;
        let mut engine = rosorah_capture::Engine::default();
        let (tx, rx) = std::sync::mpsc::channel();
        engine.start(1, Box::new(Controlled(tx)), rosorah_capture::Options::default())?;
        let source = rx.recv_timeout(Duration::from_secs(2))?;
        source.publish(Frame { width: 1, height: 1, rgba: vec![255, 0, 0, 255] })?;
        // No audio is pushed at all: video must still flow.
        let camera = Recording::start_camera(&project, 0, engine.reader(), device.clone())?;
        wait_ready(&camera);
        thread::sleep(Duration::from_secs(2));
        let view = camera.view();
        eprintln!("camera phase {:?} result {:?} details {:?}", view.phase, view.result, view.details);
        let grab = Command::new("ffmpeg")
            .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-f", "v4l2", "-i", &device, "-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", "4x4", "-"])
            .output()?;
        camera.stop();
        assert!(grab.stdout.len() >= 48, "no frame read: {}", String::from_utf8_lossy(&grab.stderr));
        let (r, g, b) = (grab.stdout[0], grab.stdout[1], grab.stdout[2]);
        assert!(r > 180 && g < 80 && b < 80, "red canvas came through as {r},{g},{b}");
        Ok(())
    }
    #[test]
    #[ignore = "Requires FFmpeg, ffprobe and Vulkan"]
    fn replay_buffer_saves_a_playable_clip_without_streaming() -> Result<()> {
        use crate::streaming::{REPLAY, Stream, save_replay};
        let folder = output_dir();
        std::fs::create_dir_all(&folder)?;
        let mut project = Project::default();
        project.preferences.recording_encoder = auroracast_core::RecordingEncoder::Software;
        project.sources[0].kind = auroracast_core::SourceKind::ScreenCapture;
        project.scenes[0].layers.retain(|l| l.source == 1);
        project.canvases[0].width = 320;
        project.canvases[0].height = 180;
        let mut engine = rosorah_capture::Engine::default();
        let (tx, rx) = std::sync::mpsc::channel();
        engine.start(1, Box::new(Controlled(tx)), rosorah_capture::Options::default())?;
        let source = rx.recv_timeout(Duration::from_secs(2))?;
        source.publish(Frame { width: 1, height: 1, rgba: vec![0, 0, 180, 255] })?;
        {
            let mut r = REPLAY.lock().unwrap();
            r.enabled = true;
            r.seconds = 5;
        }
        let stream = Stream::start_replay_only(&project, 0, engine.reader(), 1000)?;
        let start = wait_ready(&stream.engine);
        let outlet = stream.engine.outlet();
        // Eight seconds of picture and silence.
        for block in 0..800u64 {
            let at = start + Duration::from_millis(block * 10);
            if let Some(wait) = at.checked_duration_since(Instant::now()) {
                thread::sleep(wait);
            }
            outlet.push(at, vec![0.; BLOCK as usize * 2]);
        }
        let held = REPLAY.lock().unwrap().held();
        assert!((5..=8).contains(&held), "held {held} s");
        let path = folder.join("replay-test.mp4");
        let _ = std::fs::remove_file(&path);
        save_replay(&path)?;
        stream.stop();
        let probe = Command::new("ffprobe").args(["-v", "error", "-show_entries", "format=duration:stream=codec_type", "-of", "json"]).arg(&path).output()?;
        let info: serde_json::Value = serde_json::from_slice(&probe.stdout)?;
        let duration: f64 = info["format"]["duration"].as_str().unwrap_or("0").parse()?;
        assert!((4.5..=8.5).contains(&duration), "clip lasts {duration} s");
        let kinds: Vec<_> = info["streams"].as_array().unwrap().iter().map(|s| s["codec_type"].as_str().unwrap_or("").to_owned()).collect();
        assert!(kinds.contains(&"video".to_owned()) && kinds.contains(&"audio".to_owned()), "{kinds:?}");
        REPLAY.lock().unwrap().enabled = false;
        Ok(())
    }
    #[test]
    #[ignore = "Requires FFmpeg RTMP listener, ffprobe and Vulkan; never contacts a platform"]
    fn live_streaming_two_destinations_reconnect_and_shared_recording() -> Result<()> {
        use crate::streaming::{Config, Destination, Phase as StreamPhase, Stream};
        use std::process::Stdio;
        struct Listener(std::process::Child);
        impl Drop for Listener {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        fn receiver(url: &str, path: &Path) -> Result<Listener> {
            let log = std::fs::File::create(path.with_extension("log"))?;
            Ok(Listener(
                Command::new("ffmpeg")
                    .args([
                        "-nostdin",
                        "-hide_banner",
                        "-loglevel",
                        "error",
                        "-listen",
                        "1",
                        "-i",
                        url,
                        "-map",
                        "0:v:0",
                        "-map",
                        "0:a:0",
                        "-c",
                        "copy",
                        "-f",
                        "flv",
                        "-flvflags",
                        "no_duration_filesize",
                        "-y",
                    ])
                    .arg(path)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::from(log))
                    .spawn()?,
            ))
        }
        fn port() -> Result<u16> {
            Ok(std::net::TcpListener::bind("127.0.0.1:0")?
                .local_addr()?
                .port())
        }
        let folder = output_dir();
        std::fs::create_dir_all(&folder)?;
        let a = format!("rtmp://127.0.0.1:{}/live", port()?);
        let b = format!("rtmp://127.0.0.1:{}/live", port()?);
        let mut receiver_a =
            receiver(&format!("{a}/local-test-a"), &folder.join("receiver-a.flv"))?;
        let mut receiver_b = Some(receiver(
            &format!("{b}/local-test-b"),
            &folder.join("receiver-b-before.flv"),
        )?);
        thread::sleep(Duration::from_millis(150));
        let config = Config {
            twitch: Destination {
                enabled: true,
                server: a,
                key: "local-test-a".into(),
                ..Default::default()
            },
            youtube: Destination {
                enabled: true,
                server: b.clone(),
                key: "local-test-b".into(),
                ..Default::default()
            },
            bitrate: 1000,
            record: true,
            ..Default::default()
        };
        let mut project = Project::default();
        project.preferences.recording_encoder = auroracast_core::RecordingEncoder::Software;
        project.sources[0].kind = auroracast_core::SourceKind::ScreenCapture;
        project.scenes[0].layers.retain(|l| l.source == 1);
        project.canvases[0].width = 320;
        project.canvases[0].height = 180;
        assert_eq!(project.canvases[0].fps, 60);
        assert_eq!(rosorah_capture::Options::default().fps, 60);
        let mut engine = rosorah_capture::Engine::default();
        let (tx, rx) = std::sync::mpsc::channel();
        engine.start(
            1,
            Box::new(Controlled(tx)),
            rosorah_capture::Options::default(),
        )?;
        let source = rx.recv_timeout(Duration::from_secs(2))?;
        let color = |rgba: [u8; 4]| {
            source.publish(Frame {
                width: 1,
                height: 1,
                rgba: rgba.to_vec(),
            })
        };
        color([0, 0, 180, 255])?;
        let stream = Stream::start(&project, 0, engine.reader(), &config, &folder)?;
        let start = wait_ready(&stream.engine);
        let outlet = stream.engine.outlet();
        let mut initial_sending = false;
        let mut reconnecting = false;
        let mut recovered = false;
        for block in 0..1600u64 {
            let at = start + Duration::from_millis(block * 10);
            if let Some(wait) = at.checked_duration_since(Instant::now()) {
                thread::sleep(wait);
            }
            if block == 100 {
                color([255, 255, 255, 255])?;
            }
            if block == 120 {
                color([0, 0, 180, 255])?;
            }
            let tone = (100..120).contains(&block);
            outlet.push(
                at,
                (0..BLOCK)
                    .flat_map(|n| {
                        let v = if tone {
                            (std::f32::consts::TAU * 660. * (block * BLOCK + n) as f32
                                / RATE as f32)
                                .sin()
                                * 0.5
                        } else {
                            0.
                        };
                        [v, v]
                    })
                    .collect(),
            );
            if block == 400 {
                assert!(
                    initial_sending,
                    "Both destinations did not connect: {:?}",
                    stream
                        .state
                        .view()
                        .iter()
                        .map(|v| (&v.name, v.phase, &v.detail))
                        .collect::<Vec<_>>()
                );
                drop(receiver_b.take());
                receiver_b = Some(receiver(
                    &format!("{b}/local-test-b"),
                    &folder.join("receiver-b-after.flv"),
                )?);
            }
            let views = stream.state.view();
            if block < 400 && views[..2].iter().all(|v| v.phase == StreamPhase::Sending) {
                initial_sending = true;
            }
            if block > 400 && views[1].phase == StreamPhase::Reconnecting {
                reconnecting = true;
            }
            if block > 400 && views[1].phase == StreamPhase::Sending && views[1].attempts >= 2 {
                recovered = true;
            }
            assert!(
                stream.engine.view().result.is_none(),
                "Stream exited: {:?}",
                stream.engine.view().result
            );
            if block > 200 {
                assert_eq!(
                    views[0].phase,
                    StreamPhase::Sending,
                    "Healthy destination was interrupted: {}",
                    views[0].detail
                );
            }
            let _ = engine.drain();
        }
        stream.stop();
        let path = finish(&stream.engine);
        let views = stream.state.view();
        assert!(
            reconnecting && recovered,
            "reconnecting={reconnecting}, recovered={recovered}: {:?}",
            views
                .iter()
                .map(|v| (&v.name, v.phase, &v.detail))
                .collect::<Vec<_>>()
        );
        assert_eq!(views[2].detail, "Recording saved");
        assert_eq!(views[2].path.as_ref(), Some(&path));
        let deadline = Instant::now() + Duration::from_secs(2);
        while receiver_a.0.try_wait()?.is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        drop(receiver_a);
        drop(receiver_b);
        engine.stop(1);
        for file in [
            &path,
            &folder.join("receiver-a.flv"),
            &folder.join("receiver-b-after.flv"),
        ] {
            let probe = Command::new("ffprobe")
                .args([
                    "-v",
                    "error",
                    "-show_streams",
                    "-show_format",
                    "-of",
                    "json",
                ])
                .arg(file)
                .output()?;
            ensure!(
                probe.status.success(),
                "ffprobe failed for {}",
                file.display()
            );
            let probe: serde_json::Value = serde_json::from_slice(&probe.stdout)?;
            assert_eq!(probe["streams"][0]["codec_name"], "h264");
            assert_eq!(probe["streams"][1]["codec_name"], "aac");
            assert_eq!(probe["streams"][0]["width"], 320);
            assert_eq!(probe["streams"][0]["r_frame_rate"], "60/1");
            let pixels = ffmpeg_decode(
                file,
                &[
                    "-map",
                    "0:v:0",
                    "-vf",
                    "scale=1:1",
                    "-fps_mode",
                    "passthrough",
                    "-pix_fmt",
                    "rgb24",
                    "-f",
                    "rawvideo",
                    "pipe:1",
                ],
            );
            if file == &path || file.ends_with("receiver-a.flv") {
                assert!(
                    pixels.len() / 3 > 900,
                    "Lost frames in {}: {}",
                    file.display(),
                    pixels.len() / 3
                );
                let white = pixels
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .position(|p| p[0] > 220 && p[1] > 220)
                    .context("No streamed flash")?;
                let video_start: f64 = probe["streams"][0]["start_time"]
                    .as_str()
                    .unwrap()
                    .parse()?;
                let flash = video_start + white as f64 / 60.;
                let audio = ffmpeg_decode(
                    file,
                    &[
                        "-map", "0:a:0", "-f", "f32le", "-ac", "2", "-ar", "48000", "pipe:1",
                    ],
                );
                let onset = audio
                    .as_chunks::<8>()
                    .0
                    .iter()
                    .position(|p| f32::from_le_bytes(p[..4].try_into().unwrap()).abs() > 0.2)
                    .context("No streamed tone")?;
                let audio_start: f64 = probe["streams"][1]["start_time"]
                    .as_str()
                    .unwrap()
                    .parse()?;
                let tone = audio_start + onset as f64 / 48000.;
                assert!(
                    (flash - tone).abs() < 0.075,
                    "Stream sync: flash {flash}, tone {tone}"
                );
                assert!((tone - 1.).abs() < 0.075, "Tone time {tone}");
                println!(
                    "PASS {}: 60 FPS, {} frames, flash={flash:.3}, tone={tone:.3}",
                    file.display(),
                    pixels.len() / 3
                );
            } else {
                assert!(
                    pixels.len() / 3 > 240,
                    "Reconnect did not deliver decodable video"
                );
                println!(
                    "PASS {}: reconnected with {} decodable frames",
                    file.display(),
                    pixels.len() / 3
                );
            }
        }
        Ok(())
    }
    #[test]
    #[ignore = "Requires local TCP, FFmpeg and Vulkan; verifies cancellation during a stuck handshake"]
    fn stream_cancel_during_network_handshake_is_bounded() -> Result<()> {
        use crate::streaming::{Config, Destination, Phase as StreamPhase, Stream};
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;
        let done = Arc::new(AtomicBool::new(false));
        let shutdown = done.clone();
        let server = thread::spawn(move || {
            let mut sockets = Vec::new();
            while !shutdown.load(Ordering::Relaxed) {
                if let Ok((socket, _)) = listener.accept() {
                    sockets.push(socket);
                }
                thread::sleep(Duration::from_millis(5));
            }
        });
        let result = (|| -> Result<()> {
            let mut project = Project::default();
            project.canvases[0].width = 160;
            project.canvases[0].height = 90;
            project.preferences.recording_encoder = auroracast_core::RecordingEncoder::Software;
            let config = Config {
                twitch: Destination {
                    enabled: true,
                    server: format!("rtmp://127.0.0.1:{port}/live"),
                    key: "handshake-test".into(),
                    ..Default::default()
                },
                bitrate: 1000,
                ..Default::default()
            };
            let engine = rosorah_capture::Engine::default();
            let stream = Stream::start(&project, 0, engine.reader(), &config, &output_dir())?;
            wait_ready(&stream.engine);
            thread::sleep(Duration::from_millis(800));
            assert_ne!(stream.state.view()[0].phase, StreamPhase::Sending);
            let before = Instant::now();
            stream.stop();
            finish(&stream.engine);
            assert!(
                before.elapsed() < Duration::from_secs(3),
                "Stop blocked on a network handshake"
            );
            assert_eq!(stream.state.view()[0].phase, StreamPhase::Stopped);
            assert!(stream.state.view()[0].path.is_none());
            Ok(())
        })();
        done.store(true, Ordering::Relaxed);
        let _ = server.join();
        result
    }
}

#[cfg(test)]
mod media_output_tests {
    use super::*;
    use auroracast_core::{SceneVolume, SourceKind, Transition, VideoSettings, mix_audio};
    use std::process::Command;
    #[test]
    #[ignore = "requires FFmpeg, media codecs and Vulkan; exercised in release validation"]
    fn video_file_sound_scene_mix_and_transition_reach_recording() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("rosorah-media-output-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let clip = dir.join("clip.webm");
        let status = Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=red:s=160x90:r=30:d=0.5",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000:duration=0.5",
                "-c:v",
                "libvpx",
                "-c:a",
                "libvorbis",
                "-shortest",
            ])
            .arg(&clip)
            .status()?;
        ensure!(status.success(), "Create media clip");
        let mut p = Project::default();
        let settings = VideoSettings {
            path: clip.display().to_string(),
            audio: true,
            ..Default::default()
        };
        p.sources[0].kind = SourceKind::VideoFile(settings.clone());
        p.sources[1].kind = SourceKind::Color([0, 255, 0, 255]);
        p.scenes[0].layers.truncate(1);
        let mut layer = p.scenes[0].layers[0].clone();
        layer.source = 2;
        p.scenes[1].layers = vec![layer];
        p.canvases[0].width = 160;
        p.canvases[0].height = 90;
        p.canvases[0].fps = 30;
        p.preferences.transition = Transition::Fade;
        p.preferences.transition_ms = 200;
        p.preferences.recording_encoder = auroracast_core::RecordingEncoder::Software;
        p.audio[1].media_source = Some(1);
        p.preferences
            .scene_audio
            .entry(p.scenes[1].id)
            .or_default()
            .insert(
                11,
                SceneVolume {
                    gain_db: 0.,
                    muted: true,
                },
            );
        let bus = crate::video::AudioBus::default();
        let mut engine = rosorah_capture::Engine::default();
        engine.start(
            1,
            Box::new(crate::video::VideoBackend {
                id: 1,
                settings,
                audio: bus.clone(),
            }),
            rosorah_capture::Options::default(),
        )?;
        let recording = Recording::start(&p, 0, engine.reader(), &dir)?;
        let deadline = Instant::now() + Duration::from_secs(15);
        while recording.view().phase != Phase::Recording {
            ensure!(
                Instant::now() < deadline && recording.view().result.is_none(),
                "Recorder did not start: {:?}",
                recording.view().result
            );
            thread::sleep(Duration::from_millis(10));
        }
        let outlet = recording.outlet();
        for i in 0..200 {
            if i == 100 {
                p.canvases[0].scene = p.scenes[1].id;
                recording.update_project(&p);
            }
            let media = bus.read();
            let raw = media.get(&1).cloned().unwrap_or_else(|| vec![0.; 960]);
            let pcm = mix_audio(&outlet.channels(), &[(11, raw.as_slice())], 0, 960)?;
            outlet.push(Instant::now(), pcm);
            thread::sleep(Duration::from_millis(10));
        }
        recording.stop();
        let deadline = Instant::now() + Duration::from_secs(15);
        let path = loop {
            if let Some(result) = recording.view().result {
                break result.map_err(anyhow::Error::msg)?;
            }
            ensure!(Instant::now() < deadline, "Recording did not finish");
            thread::sleep(Duration::from_millis(20));
        };
        engine.stop(1);
        let pixels = Command::new("ffmpeg")
            .args(["-v", "error", "-ss", "1.6", "-i"])
            .arg(&path)
            .args([
                "-frames:v",
                "1",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgb24",
                "pipe:1",
            ])
            .output()?;
        ensure!(
            pixels.status.success() && pixels.stdout.len() >= 3,
            "Decode recording"
        );
        ensure!(
            pixels.stdout[1] > 200 && pixels.stdout[0] < 25,
            "Scene did not reach output: {:?}",
            &pixels.stdout[..3]
        );
        let audio = Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&path)
            .args(["-vn", "-ar", "48000", "-ac", "2", "-f", "f32le", "pipe:1"])
            .output()?;
        ensure!(audio.status.success(), "Decode recorded audio");
        let pcm: Vec<_> = audio
            .stdout
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        let rms = |range: std::ops::Range<usize>| {
            let s = &pcm[range];
            (s.iter().map(|v| v * v).sum::<f32>() / s.len() as f32).sqrt()
        };
        ensure!(pcm.len() > 172800, "Audio too short");
        let first = rms(28800..67200);
        let last = rms(134400..172800);
        ensure!(
            first > 0.02 && last < 0.005,
            "Scene audio did not follow output: {first} -> {last}"
        );
        drop(recording);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }
}

#[cfg(test)]
mod scene_mix_cache_tests {
    use super::*;
    #[test]
    fn cached_mix_preserves_independent_canvases_and_live_edits() {
        let mut p = Project::default();
        let mut main = SceneMix::default();
        let mut vertical = SceneMix::default();
        let canvas = p.canvases[0].id;
        let initial = main.channels(0, &p, canvas);
        for _ in 0..1000 {
            assert!(Arc::ptr_eq(&initial, &main.channels(0, &p, canvas)));
        }
        let scene = p.canvases[0].scene;
        p.canvases[1].scene = p.scenes[1].id;
        let channel = p.audio[0].id;
        p.preferences.scene_audio.entry(scene).or_default().insert(
            channel,
            auroracast_core::SceneVolume {
                gain_db: -12.,
                muted: true,
            },
        );
        let changed = main.channels(1, &p, canvas);
        assert!(!Arc::ptr_eq(&initial, &changed));
        assert!(changed[0].muted);
        let separate = vertical.channels(1, &p, p.canvases[1].id);
        assert_eq!(separate[0].muted, p.audio[0].muted);
        p.canvases[0].scene = p.canvases[1].scene;
        assert_eq!(main.channels(2, &p, canvas)[0].muted, separate[0].muted);
    }
}

#[cfg(test)]
mod selective_upload_tests {
    use super::*;
    #[test]
    #[ignore = "requires Vulkan; exercised in release validation"]
    fn uploads_only_visible_sources_and_reveals_latest_frame() -> Result<()> {
        let mut p = Project::default();
        p.sources[0].kind = auroracast_core::SourceKind::ScreenCapture;
        p.sources[1].kind = auroracast_core::SourceKind::ScreenCapture;
        p.canvases.truncate(1);
        p.scenes[0].layers.truncate(2);
        p.scenes[0].layers[1].visible = false;
        let frame = Arc::new(Frame {
            width: 1,
            height: 1,
            rgba: vec![255, 0, 0, 255],
        });
        let frames = || vec![(1, Some(frame.clone())), (2, Some(frame.clone()))];
        let mut g = pollster::block_on(Compositor::new())?;
        let mut previous = BTreeMap::new();
        let used = live_sources(&p);
        assert_eq!(used, [1].into());
        upload_frames(&mut g, frames(), &used, &mut previous)?;
        assert_eq!(g.uploads, 1);
        upload_frames(&mut g, frames(), &used, &mut previous)?;
        assert_eq!(g.uploads, 1);
        p.scenes[0].layers[1].visible = true;
        upload_frames(&mut g, frames(), &live_sources(&p), &mut previous)?;
        assert_eq!(
            g.uploads, 2,
            "newly revealed source must upload its retained frame"
        );
        upload_frames(
            &mut g,
            vec![(1, None), (2, Some(frame.clone()))],
            &live_sources(&p),
            &mut previous,
        )?;
        assert_eq!(g.uploads, 3, "stopped source must clear immediately");
        // The canvas background, not the red frame. Its red is 15.56 after sRGB encoding, and
        // drivers may round that either way (Mesa RADV gives 15, others 16).
        let red = g.render(&p, &p.canvases[0], 64, 36)?[0];
        assert!((15..=16).contains(&red), "stopped source still shown: red = {red}");
        // Replacing a live layer with an image/color must not be overwritten by
        // a frame still present in the asynchronous capture registry.
        p.sources[1].kind = auroracast_core::SourceKind::Color([0, 255, 0, 255]);
        upload_frames(&mut g, frames(), &live_sources(&p), &mut previous)?;
        assert!(!previous.contains_key(&2));
        Ok(())
    }
}
