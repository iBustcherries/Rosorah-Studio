//! One renderer for every output. Recording, streaming, the replay buffer and the virtual camera
//! each subscribe to a canvas; Rosorah draws that canvas once per frame on one graphics device and
//! hands each of them the same picture, converted on the graphics card, instead of every output
//! drawing (and uploading every capture to) its own copy. Frames are copied out while the next
//! one is drawn, so the renderer never sits waiting on the graphics card.
use super::{live_sources, selected_project, upload_frames};
use anyhow::{Result, bail};
use auroracast_core::{Canvas, Id, Project};
use auroracast_gpu::{Compositor, Nv12};
use rosorah_capture::FrameReader;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    sync::{Arc, Condvar, LazyLock, Mutex},
    thread,
    time::{Duration, Instant},
};

/// The picture an output wants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    Nv12(Nv12),
    /// For canvas sizes the graphics card conversion can't make (width not a multiple of 4).
    Rgba,
}
impl Format {
    /// What an encoder gets for a canvas: NV12 at the canvas's size when possible.
    pub fn for_encoder(canvas: &Canvas) -> Self {
        Nv12::same(canvas.width, canvas.height).map_or(Self::Rgba, Self::Nv12)
    }
}
pub struct Picture {
    /// When the frame was drawn.
    pub at: Instant,
    pub data: Arc<Vec<u8>>,
    /// How long drawing it took.
    pub render: Duration,
}
#[derive(Default)]
struct Mailbox {
    pictures: VecDeque<Picture>,
    error: Option<String>,
}
type Inbox = Arc<(Mutex<Mailbox>, Condvar)>;
fn deliver(inbox: &Inbox, picture: Picture) {
    let mut mailbox = inbox.0.lock().unwrap();
    mailbox.pictures.push_back(picture);
    // An output that stopped reading doesn't hold on to old frames.
    while mailbox.pictures.len() > 8 {
        mailbox.pictures.pop_front();
    }
    inbox.1.notify_all();
}
fn fail(inbox: &Inbox, error: &str) {
    inbox.0.lock().unwrap().error.get_or_insert_with(|| error.to_owned());
    inbox.1.notify_all();
}

struct Subscriber {
    id: u64,
    canvas: Canvas,
    format: Format,
    period: Duration,
    next: Instant,
    inbox: Inbox,
}
#[derive(Default)]
struct Hub {
    subscribers: Vec<Subscriber>,
    /// The project every output is showing, numbered so the renderer notices changes.
    project: Option<(u64, Project)>,
    frames: Option<FrameReader>,
    next_id: u64,
    running: bool,
}
static HUB: LazyLock<(Mutex<Hub>, Condvar)> = LazyLock::new(Default::default);

/// An output's place in the renderer; dropping it stops its frames.
pub struct Subscription {
    id: u64,
    inbox: Inbox,
}
impl Subscription {
    /// Starts receiving `canvas` as `format`, `fps` times a second.
    pub fn new(project: &Project, frames: FrameReader, canvas: &Canvas, format: Format, fps: u32) -> Self {
        let (lock, wake) = &*HUB;
        let mut hub = lock.lock().unwrap();
        hub.next_id += 1;
        let id = hub.next_id;
        if hub.project.is_none() {
            hub.project = Some((0, project.clone()));
        }
        hub.frames.get_or_insert(frames);
        let period = Duration::from_secs(1) / fps.max(1);
        // In step with outputs already taking this canvas, so one drawing serves them all.
        let next = hub
            .subscribers
            .iter()
            .filter(|s| s.canvas.id == canvas.id && s.period <= period)
            .map(|s| s.next)
            .min()
            .unwrap_or_else(Instant::now);
        let inbox = Inbox::default();
        hub.subscribers.push(Subscriber { id, canvas: canvas.clone(), format, period, next, inbox: inbox.clone() });
        if !hub.running {
            hub.running = true;
            if let Err(e) = thread::Builder::new().name("rosorah-renderer".into()).spawn(run) {
                hub.running = false;
                fail(&inbox, &format!("Couldn't start the renderer: {e}"));
            }
        }
        wake.notify_all();
        Self { id, inbox }
    }
    /// The pictures that arrived since the last call, oldest first, waiting up to `timeout` for one.
    pub fn take(&self, timeout: Duration) -> Result<Vec<Picture>> {
        let (lock, arrived) = &*self.inbox;
        let mut mailbox = lock.lock().unwrap();
        if mailbox.pictures.is_empty() && mailbox.error.is_none() && !timeout.is_zero() {
            mailbox = arrived.wait_timeout(mailbox, timeout).unwrap().0;
        }
        if let Some(e) = &mailbox.error {
            bail!("{e}");
        }
        Ok(mailbox.pictures.drain(..).collect())
    }
    /// Waits for the first picture (the graphics device may still be starting).
    pub fn first(&self, timeout: Duration) -> Result<Picture> {
        let until = Instant::now() + timeout;
        loop {
            if let Some(picture) = self.take(Duration::from_millis(50))?.pop() {
                return Ok(picture);
            }
            if Instant::now() > until {
                bail!("The renderer didn't draw a frame in time");
            }
        }
    }
}
impl Drop for Subscription {
    fn drop(&mut self) {
        let (lock, wake) = &*HUB;
        let mut hub = lock.lock().unwrap();
        hub.subscribers.retain(|s| s.id != self.id);
        if hub.subscribers.is_empty() {
            hub.project = None;
            hub.frames = None;
        }
        wake.notify_all();
    }
}
/// Gives the renderer the project as it is now (called whenever outputs are updated).
pub fn update_project(project: &Project) {
    let mut hub = HUB.0.lock().unwrap();
    if let Some((revision, current)) = &mut hub.project {
        *revision += 1;
        *current = project.clone();
    }
}

/// One canvas drawn this tick, and who gets which of its pictures.
struct Job {
    /// The tick this drawing is for. Pictures carry it rather than when drawing began: under a
    /// busy graphics card a late tick otherwise landed in the next frame slot, so the recording
    /// repeated one frame and dropped the next.
    due: Instant,
    canvas: Canvas,
    formats: Vec<Format>,
    deliveries: Vec<(Inbox, usize)>,
}
/// A frame on its way out of the graphics card.
struct Pending {
    at: Instant,
    render: Duration,
    deliveries: Vec<(Inbox, usize)>,
}
/// The renderer's own copy of what it draws.
#[derive(Default)]
struct Scene {
    revision: Option<u64>,
    canvases: HashMap<(Id, u32, u32), (Project, Canvas)>,
    sources: crate::media::Sources,
    previous: BTreeMap<Id, Option<Arc<rosorah_capture::Frame>>>,
    slideshow_check: Option<Instant>,
}

fn run() {
    let mut gpu: Option<Compositor> = None;
    let mut scene = Scene::default();
    let mut pending: HashMap<u64, Pending> = HashMap::new();
    let mut tag = 0u64;
    loop {
        // Who's due now, and the project to draw.
        let (jobs, project, frames, wake_at) = {
            let (lock, _) = &*HUB;
            let mut hub = lock.lock().unwrap();
            if hub.subscribers.is_empty() {
                // Nothing to draw: let the graphics device go (frees its memory) and stop.
                hub.running = false;
                return;
            }
            let now = Instant::now();
            let mut jobs: Vec<Job> = Vec::new();
            for s in hub.subscribers.iter_mut() {
                // A millisecond early is on time: the sleep below wakes about then.
                if s.next > now + Duration::from_millis(1) {
                    continue;
                }
                let due = s.next;
                s.next += s.period;
                if s.next < now {
                    s.next = now + s.period;
                }
                let job = match jobs.iter_mut().position(|j| j.canvas.id == s.canvas.id && (j.canvas.width, j.canvas.height) == (s.canvas.width, s.canvas.height)) {
                    Some(i) => &mut jobs[i],
                    None => {
                        jobs.push(Job { due, canvas: s.canvas.clone(), formats: Vec::new(), deliveries: Vec::new() });
                        jobs.last_mut().unwrap()
                    }
                };
                job.due = job.due.min(due);
                let index = job.formats.iter().position(|f| *f == s.format).unwrap_or_else(|| {
                    job.formats.push(s.format);
                    job.formats.len() - 1
                });
                job.deliveries.push((s.inbox.clone(), index));
            }
            let wake_at = hub.subscribers.iter().map(|s| s.next).min().unwrap();
            (jobs, hub.project.clone(), hub.frames.clone(), wake_at)
        };
        if !jobs.is_empty() {
            if gpu.is_none() {
                match pollster::block_on(Compositor::new()) {
                    Ok(g) => gpu = Some(g),
                    Err(e) => {
                        let error = format!("Couldn't start the renderer's graphics device: {e:#}");
                        for (inbox, _) in jobs.iter().flat_map(|j| &j.deliveries) {
                            fail(inbox, &error);
                        }
                        idle(Instant::now() + Duration::from_secs(1));
                        continue;
                    }
                }
            }
            let g = gpu.as_mut().unwrap();
            if let (Some((revision, project)), Some(frames)) = (&project, &frames)
                && let Err(e) = prepare(g, &mut scene, *revision, project, frames, &jobs)
            {
                let error = format!("Recording compositor: {e:#}");
                for (inbox, _) in jobs.iter().flat_map(|j| &j.deliveries) {
                    fail(inbox, &error);
                }
            }
            for job in &jobs {
                let key = (job.canvas.id, job.canvas.width, job.canvas.height);
                let Some((current, active)) = scene.canvases.get(&key) else { continue };
                if let Err(e) = draw(g, current, active, job, &mut tag, &mut pending) {
                    let error = format!("Recording compositor: {e:#}");
                    for (inbox, _) in &job.deliveries {
                        fail(inbox, &error);
                    }
                }
            }
        }
        // Hand over whatever finished copying out.
        if let Some(g) = gpu.as_mut() {
            match g.finished_nv12(false) {
                Ok(done) => {
                    for (done_tag, pictures) in done {
                        hand_over(&mut pending, done_tag, pictures);
                    }
                }
                Err(e) => {
                    let error = format!("Recording compositor: {e:#}");
                    for (inbox, _) in pending.drain().flat_map(|(_, p)| p.deliveries) {
                        fail(&inbox, &error);
                    }
                }
            }
        }
        // Sleep until the next frame is due; check on frames in flight every couple of milliseconds.
        let in_flight = gpu.as_ref().is_some_and(|g| g.nv12_in_flight() > 0);
        let until = if in_flight { wake_at.min(Instant::now() + Duration::from_millis(2)) } else { wake_at };
        idle(until);
    }
}
/// Waits until `until`, or until an output joins or leaves.
fn idle(until: Instant) {
    let (lock, wake) = &*HUB;
    let hub = lock.lock().unwrap();
    let now = Instant::now();
    if until > now {
        let _ = wake.wait_timeout(hub, until - now).unwrap();
    }
}
/// Brings the renderer's copy of the project and the capture pictures up to date.
fn prepare(g: &mut Compositor, scene: &mut Scene, revision: u64, project: &Project, frames: &FrameReader, jobs: &[Job]) -> Result<()> {
    if scene.revision != Some(revision) {
        scene.revision = Some(revision);
        scene.canvases.clear();
        scene.slideshow_check = Some(Instant::now());
    }
    for job in jobs {
        let key = (job.canvas.id, job.canvas.width, job.canvas.height);
        if !scene.canvases.contains_key(&key) {
            let (current, active) = selected_project(project, &job.canvas)?;
            scene.sources.upload(g, &current, 0, false)?;
            scene.canvases.insert(key, (current, active));
        }
    }
    // Slideshows change pictures on a timer, not when the project changes.
    if scene.slideshow_check.is_none_or(|t| t.elapsed() >= Duration::from_millis(500)) {
        scene.slideshow_check = Some(Instant::now());
        for (current, _) in scene.canvases.values() {
            if current.sources.iter().any(|s| matches!(s.kind, auroracast_core::SourceKind::Slideshow(_))) {
                scene.sources.upload(g, current, 0, false)?;
            }
        }
    }
    // Each capture is uploaded once, however many canvases and outputs show it.
    let used: BTreeSet<Id> = scene.canvases.values().flat_map(|(current, _)| live_sources(current)).collect();
    upload_frames(g, frames.latest(), &used, &mut scene.previous)
}
fn draw(g: &mut Compositor, current: &Project, active: &Canvas, job: &Job, tag: &mut u64, pending: &mut HashMap<u64, Pending>) -> Result<()> {
    let (width, height) = (job.canvas.width, job.canvas.height);
    let (at, started) = (job.due, Instant::now());
    let mut waited = Duration::ZERO;
    let pictures: Vec<Nv12> = job.formats.iter().filter_map(|f| if let Format::Nv12(n) = f { Some(*n) } else { None }).collect();
    if !pictures.is_empty() {
        // Two frames out at once at most; beyond that the graphics card is behind, so wait.
        let wait = Instant::now();
        while g.nv12_in_flight() >= 2 {
            for (done_tag, done) in g.finished_nv12(true)? {
                hand_over(pending, done_tag, done);
            }
        }
        waited = wait.elapsed();
        *tag += 1;
        g.begin_nv12(current, active, width, height, &pictures, *tag)?;
        // Which picture each output gets, by its place among the NV12 ones.
        let deliveries = job
            .deliveries
            .iter()
            .filter_map(|(inbox, index)| {
                let Format::Nv12(n) = job.formats[*index] else { return None };
                Some((inbox.clone(), pictures.iter().position(|p| *p == n)?))
            })
            .collect();
        pending.insert(*tag, Pending { at, render: started.elapsed(), deliveries });
        trace_draw(started.saturating_duration_since(at), waited, started.elapsed());
    }
    if let Some(rgba) = job.formats.iter().position(|f| *f == Format::Rgba) {
        let data = Arc::new(g.render(current, active, width, height)?);
        for (inbox, index) in &job.deliveries {
            if *index == rgba {
                deliver(inbox, Picture { at, data: data.clone(), render: started.elapsed() });
            }
        }
    }
    Ok(())
}
/// `ROSORAH_CAPTURE_TRACE=1`: once a second, how late drawing started after its tick, how long it
/// waited for the graphics card, and how long the whole draw took (the worst of each).
fn trace_draw(late: Duration, waited: Duration, took: Duration) {
    static TRACE: Mutex<Option<(Instant, Duration, Duration, Duration, u32)>> = Mutex::new(None);
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*ON.get_or_init(|| std::env::var_os("ROSORAH_CAPTURE_TRACE").is_some()) {
        return;
    }
    let mut trace = TRACE.lock().unwrap();
    let (since, worst_late, worst_wait, worst_draw, late_half) = trace.get_or_insert((Instant::now(), Duration::ZERO, Duration::ZERO, Duration::ZERO, 0));
    *worst_late = (*worst_late).max(late);
    *worst_wait = (*worst_wait).max(waited);
    *worst_draw = (*worst_draw).max(took);
    *late_half += u32::from(late > Duration::from_millis(8));
    if since.elapsed() >= Duration::from_secs(1) {
        let ms = |d: Duration| d.as_secs_f64() * 1000.;
        eprintln!("draw trace: worst start {:.1} ms late, {late_half} ticks over 8 ms late, worst GPU wait {:.1} ms, worst draw {:.1} ms", ms(*worst_late), ms(*worst_wait), ms(*worst_draw));
        *trace = None;
    }
}
fn hand_over(pending: &mut HashMap<u64, Pending>, tag: u64, pictures: Vec<Vec<u8>>) {
    let Some(frame) = pending.remove(&tag) else { return };
    let pictures: Vec<Arc<Vec<u8>>> = pictures.into_iter().map(Arc::new).collect();
    for (inbox, index) in frame.deliveries {
        if let Some(data) = pictures.get(index) {
            deliver(&inbox, Picture { at: frame.at, data: data.clone(), render: frame.render });
        }
    }
}
