//! Live canvas + Track 1 recording. Capture, mixing and UI never write to an
//! encoder pipe. Fixed-rate writers share a monotonic clock; missed video is
//! repeated and missing audio is silence, so a stall never shortens one stream.
mod encoding;
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
    rgba: Arc<Vec<u8>>,
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
}
impl Output {
    fn path(&self) -> PathBuf {
        match self {
            Self::Recording(path) => path.clone(),
            Self::Streaming(setup) => setup.recording.clone().unwrap_or_default(),
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
            audio_channels: if matches!(&output, Output::Recording(_))
                && project.preferences.multitrack_recording
            {
                12
            } else {
                2
            },
            video_capacity: (32 * 1024 * 1024
                / (canvas.width as usize * canvas.height as usize * 4))
                .clamp(2, 8),
        });
        let project = Arc::new(Mutex::new((0, project.clone())));
        let (s, p, c) = (shared.clone(), project.clone(), canvas.clone());
        let worker = thread::Builder::new()
            .name("rosorah-recording".into())
            .spawn(move || {
                let recovery=match &output{Output::Recording(path)=>Some(path.clone()),Output::Streaming(setup)=>setup.recording.clone()};if let Some(path)=&recovery{crate::utilities::mark_recording(path);}
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
        {
            let mut queue = s.video.lock().unwrap();
            while queue.front().is_some_and(|v| v.at <= at) {
                last = queue.pop_front().unwrap().rgba;
            }
        }
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
    for (id, current) in frames.into_iter().filter(|(id, _)| used.contains(id)) {
        let same = match (previous.get(&id), &current) {
            (Some(Some(a)), Some(b)) => Arc::ptr_eq(a, b),
            (Some(None), None) => true,
            _ => false,
        };
        if same {
            continue;
        }
        match &current {
            Some(f) => gpu.upload(id, f.width, f.height, &f.rgba)?,
            None => gpu.upload(id, 1, 1, &[0, 0, 0, 0])?,
        }
        previous.insert(id, current);
    }
    Ok(())
}
fn run(
    s: &Arc<Shared>,
    project: Arc<Mutex<(u64, Project)>>,
    canvas: Canvas,
    frames: FrameReader,
    output: &Output,
) -> Result<()> {
    let path = output.path();
    let mode = project.lock().unwrap().1.preferences.recording_encoder;
    let cancelled = || s.end.lock().unwrap().is_some();
    let report = |status: &str| s.view.lock().unwrap().encoder = status.into();
    let selection = match output {
        Output::Recording(_) => select(&canvas, mode, cancelled, report),
        Output::Streaming(setup) => {
            encoding::select_stream(&canvas, mode, setup.bitrate, cancelled, report)
        }
    }?;
    let codec = selection.codecs;
    s.view.lock().unwrap().details = selection.details.clone();
    crate::capture_log::event(format!("Recording encoder: {}", selection.details));
    ensure!(
        s.end.lock().unwrap().is_none(),
        "Recording cancelled before it started"
    );
    let mut gpu =
        pollster::block_on(Compositor::new()).context("Initialize recording compositor")?;
    let (mut revision, initial) = project.lock().unwrap().clone();
    let mut slideshow_check = Instant::now();
    let (mut current, mut active_canvas) = selected_project(&initial, &canvas)?;
    let mut source_uploads = crate::media::Sources::default();
    source_uploads.upload(&mut gpu, &current, 0, false)?;
    let mut previous = BTreeMap::new();
    let mut used = live_sources(&current);
    upload_frames(&mut gpu, frames.latest(), &used, &mut previous)?;
    let first = Arc::new(gpu.render(&current, &active_canvas, canvas.width, canvas.height)?);
    // Cancel during setup does not create an empty recording.
    if s.end.lock().unwrap().is_some() {
        bail!("Recording cancelled before it started");
    }
    let (mut encoder, video_pipe, audio_pipe) = match output {
        Output::Recording(path) => {
            encoding::launch_tracks(&canvas, &codec, path, s.audio_channels / 2)
        }
        Output::Streaming(_) => encoding::launch_stream(&canvas, &codec),
    }?;
    let hub = match output {
        Output::Streaming(setup) => Some(crate::streaming::Hub::start(
            encoder.0.stdout.take().unwrap(),
            setup,
        )),
        Output::Recording(_) => None,
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
    let mut next_frame = time_of(1, canvas.fps as u64);
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
            let now = start.elapsed();
            if now >= next_frame {
                s.missed_frames.fetch_add(units(now,canvas.fps as u64).saturating_sub(units(next_frame,canvas.fps as u64)),Ordering::Relaxed);
                let render_started=Instant::now();
                let result = (|| -> Result<()> {
                    {
                        let p = project.lock().unwrap();
                        if p.0 != revision {
                            (current, active_canvas) = selected_project(&p.1, &canvas)?;
                            source_uploads.upload(&mut gpu, &current, 0, false)?;
                            used = live_sources(&current);
                            revision = p.0;
                            slideshow_check = Instant::now();
                        }
                    }
                    // Slideshows change pictures on a timer, not when the project changes.
                    if slideshow_check.elapsed() >= Duration::from_millis(500) {
                        slideshow_check = Instant::now();
                        if current.sources.iter().any(|s| matches!(s.kind, auroracast_core::SourceKind::Slideshow(_))) {
                            source_uploads.upload(&mut gpu, &current, 0, false)?;
                        }
                    }
                    upload_frames(&mut gpu, frames.latest(), &used, &mut previous)?;
                    let rgba = gpu.render(&current, &active_canvas, canvas.width, canvas.height)?;
                    let mut queue = s.video.lock().unwrap();
                    queue.push_back(Video {
                        at: time_of(units(now, canvas.fps as u64), canvas.fps as u64),
                        rgba: Arc::new(rgba),
                    });
                    while queue.len() > s.video_capacity {
                        queue.pop_front();s.queue_drops.fetch_add(1,Ordering::Relaxed);
                    }
                    Ok(())
                })();
                s.render_ns.store(render_started.elapsed().as_nanos()as u64,Ordering::Relaxed);
                s.missed_frames.fetch_add(units(start.elapsed(),canvas.fps as u64).saturating_sub(units(now,canvas.fps as u64)),Ordering::Relaxed);
                if let Err(e) = result {
                    s.fail(format!("Recording compositor: {e:#}"));
                }
                next_frame = time_of(
                    units(start.elapsed(), canvas.fps as u64) + 1,
                    canvas.fps as u64,
                );
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

#[cfg(test)]
mod tests {
    use super::*;
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
        assert_eq!(g.render(&p, &p.canvases[0], 64, 36)?[0], 16);
        // Replacing a live layer with an image/color must not be overwritten by
        // a frame still present in the asynchronous capture registry.
        p.sources[1].kind = auroracast_core::SourceKind::Color([0, 255, 0, 255]);
        upload_frames(&mut g, frames(), &live_sources(&p), &mut previous)?;
        assert!(!previous.contains_key(&2));
        Ok(())
    }
}
