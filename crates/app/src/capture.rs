//! Desktop adapter for the independent Rosorah capture engine.
use anyhow::{Context, Result, bail};
#[cfg(not(windows))]
use ashpd::desktop::screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType};
use auroracast_core::{Id, Project, SourceKind};
#[cfg(not(windows))]
use futures_util::StreamExt;
use gst::prelude::*;
use gst_video::prelude::*;
use rosorah_capture::{Backend, Engine, Frame, Options, Producer, Target};
use std::time::Duration;
#[cfg(not(windows))]
use std::os::fd::AsRawFd;
use tokio::sync::watch;

#[cfg(not(windows))]
struct DesktopBackend;
#[cfg(not(windows))]
impl Backend for DesktopBackend {
    fn run(
        self: Box<Self>,
        output: Producer,
        mut cancel: watch::Receiver<bool>,
        options: Options,
    ) -> Result<()> {
        gst::init()?;
        for name in ["videoconvert", "videoscale", "appsink"] {
            anyhow::ensure!(
                gst::ElementFactory::find(name).is_some(),
                "Missing GStreamer element {name}. Install the GStreamer base plugins for your distribution."
            );
        }
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(run(&output, &mut cancel, options))
    }
}
#[derive(Default)]
pub struct Captures {
    pub videos: crate::video::Playback,
    pub widgets: crate::widgets::Hub,
    widget_running: std::collections::BTreeSet<Id>,
    engine: Engine,
    preview_active: std::sync::Arc<std::sync::atomic::AtomicBool>,
    preview_timing: std::collections::BTreeMap<Id, (std::time::Instant, f64, f64, Option<f64>)>,
    #[cfg(windows)]
    pub monitors: std::collections::BTreeMap<Id, String>,
    #[cfg(windows)]
    pub windows: std::collections::BTreeMap<Id, rosorah_windows::window::Window>,
    pub options: Options,
    error: Option<String>,
    pending: std::collections::BTreeMap<Id, Options>,
    browsers: std::collections::BTreeMap<Id, (String, auroracast_core::browser::Settings)>,
    browser_commands:
        std::collections::BTreeMap<Id, std::sync::mpsc::SyncSender<serde_json::Value>>,
    browser_attempts: std::collections::BTreeMap<Id, std::time::Instant>,
    cameras: std::collections::BTreeMap<Id, auroracast_core::CameraSettings>,
    camera_errors: std::collections::BTreeMap<Id, String>,
    modes: std::collections::BTreeMap<Id, Target>,
    armed: std::collections::BTreeSet<Id>,
    attempts: std::collections::BTreeMap<Id, std::time::Instant>,
}
impl Captures {
    pub fn stop_alert_source(&mut self, id: Id) {
        self.engine.stop(id);
        self.videos.queued.remove(&id);
        self.videos.bus.clear(id);
    }

    pub fn sync_widgets(&mut self, project: &Project) {
        let mut states = self.widgets.0.lock().unwrap();
        states.retain(|id, _| {
            project
                .source(*id)
                .is_some_and(|s| matches!(s.kind, SourceKind::Widget(_)))
        });
        let mut visible = std::collections::BTreeSet::new();
        for source in &project.sources {
            if let SourceKind::Widget(settings) = &source.kind {
                let state = states.entry(source.id).or_insert_with(|| {
                    let mut state = crate::widgets::State::new(settings.clone());
                    if project
                        .preferences
                        .widget_periods
                        .get(&source.id)
                        .is_none_or(|key| *key == state.period_key)
                    {
                        if let Some(value) = project.preferences.widget_values.get(&source.id) {
                            state.value = *value;
                        }
                    }
                    std::sync::Arc::new(std::sync::Mutex::new(state))
                });
                {
                    let mut saved = state.lock().unwrap();
                    if saved.settings != *settings {
                        if saved.settings.initial != settings.initial {
                            saved.value = settings.initial;
                        }
                        saved.settings = settings.clone();
                        saved.update("", 0, false);
                    }
                }
                if project
                    .canvases
                    .iter()
                    .filter_map(|c| project.scene(c.scene))
                    .any(|s| s.layers.iter().any(|l| l.visible && l.source == source.id))
                {
                    visible.insert(source.id);
                    if !self.engine.running(source.id) {
                        match self.engine.start(
                            source.id,
                            Box::new(crate::widgets::Renderer(
                                state.clone(),
                                self.engine.reader(),
                            )),
                            self.options,
                        ) {
                            Ok(_) => {
                                self.widget_running.insert(source.id);
                            }
                            Err(e) => {
                                self.error = Some(e.to_string());
                            }
                        }
                    }
                }
            }
        }
        for id in self
            .widget_running
            .difference(&visible)
            .copied()
            .collect::<Vec<_>>()
        {
            self.engine.stop(id);
            self.widget_running.remove(&id);
        }
    }
    pub fn browser_command(&self, id: Id, kind: &str) {
        if let Some(tx) = self.browser_commands.get(&id) {
            let _ = tx.try_send(serde_json::json!({"type":kind}));
        }
    }
    pub fn browser_status(&self, id: Id) -> String {
        self.engine
            .snapshot(id)
            .map(|s| s.detail)
            .unwrap_or_else(|| "Waiting for an enabled browser source in an active scene".into())
    }
    pub fn sync_browsers(&mut self, project: &Project) {
        let desired: std::collections::BTreeMap<_, _> = project
            .sources
            .iter()
            .filter_map(|s| {
                if let SourceKind::Browser { url } = &s.kind {
                    let settings = project
                        .preferences
                        .browser_sources
                        .get(&s.id)
                        .cloned()
                        .unwrap_or_default();
                    if settings.enabled
                        && crate::browser_source::valid_url(url)
                        && project
                            .canvases
                            .iter()
                            .filter_map(|c| project.scene(c.scene))
                            .any(|scene| scene.layers.iter().any(|l| l.visible && l.source == s.id))
                    {
                        return Some((s.id, (url.clone(), settings)));
                    }
                }
                None
            })
            .collect();
        for id in self
            .browsers
            .keys()
            .filter(|id| !desired.contains_key(id))
            .copied()
            .collect::<Vec<_>>()
        {
            self.engine.stop(id);
            self.browsers.remove(&id);
            self.browser_commands.remove(&id);
            self.browser_attempts.remove(&id);
            self.videos.bus.clear(id);
        }
        for (id, (url, settings)) in desired {
            if self.browsers.get(&id) != Some(&(url.clone(), settings.clone())) {
                self.engine.stop(id);
                self.browser_commands.remove(&id);
                self.browsers.insert(id, (url.clone(), settings.clone()));
                self.browser_attempts.remove(&id);
                self.videos.bus.clear(id);
            }
            if !crate::browser_source::ready()
                || self.engine.running(id)
                || self
                    .browser_attempts
                    .get(&id)
                    .is_some_and(|at| at.elapsed() < Duration::from_secs(5))
            {
                continue;
            }
            let (tx, rx) = std::sync::mpsc::sync_channel(4);
            self.browser_commands.insert(id, tx);
            self.browser_attempts.insert(id, std::time::Instant::now());
            if let Err(e) = self.engine.start(
                id,
                Box::new(crate::browser_source::BrowserBackend {
                    id,
                    url,
                    settings,
                    audio: self.videos.bus.clone(),
                    commands: rx,
                }),
                Options::default(),
            ) {
                self.error = Some(e.to_string());
            }
        }
    }
    pub fn sync_videos(&mut self, project: &Project) {
        for id in self.widgets.tick_alerts(project) {
            if self.videos.queued.get(&id).copied().unwrap_or(0) == 0 {
                self.videos.trigger(id);
            }
        }
        let alert_media = self.widgets.active_media();
        let desired: std::collections::BTreeMap<_, _> = project
            .sources
            .iter()
            .filter_map(|s| {
                if let SourceKind::VideoFile(v) = &s.kind {
                    (v.enabled
                        && (alert_media.contains(&s.id)
                            || project.preferences.overlays.values().any(|o| o.source == s.id)
                            || project
                                .canvases
                                .iter()
                                .filter_map(|c| project.scene(c.scene))
                                .any(|scene| {
                                    scene.layers.iter().any(|l| l.source == s.id && l.visible)
                                })))
                    .then_some((s.id, v))
                } else {
                    None
                }
            })
            .collect();
        self.videos.queued.retain(|id, _| desired.contains_key(id));
        let removed: Vec<_> = self
            .videos
            .settings
            .keys()
            .filter(|id| !desired.contains_key(id))
            .copied()
            .collect();
        for id in removed {
            self.engine.stop(id);
            self.videos.settings.remove(&id);
            self.videos.queued.remove(&id);
            self.videos.bus.clear(id);
        }
        for (id, settings) in desired {
            let changed = self.videos.settings.get(&id) != Some(settings);
            if changed || self.videos.restart.contains(&id) {
                if self.engine.running(id) {
                    self.engine.stop(id);
                    continue;
                }
                self.videos.settings.insert(id, settings.clone());
                self.videos.restart.remove(&id);
                if settings.trigger.is_none() && !settings.automation_only {
                    self.videos.trigger(id);
                }
            }
            let queued = self.videos.queued.entry(id).or_default();
            if *queued > 0 && !self.engine.running(id) {
                *queued -= 1;
                if let Err(e) = self.engine.start(
                    id,
                    Box::new(crate::video::VideoBackend {
                        id,
                        settings: settings.clone(),
                        audio: self.videos.bus.clone(),
                    }),
                    Options::default(),
                ) {
                    self.error = Some(e.to_string());
                }
            }
        }
    }
    pub fn video_failure(&self, id: Id) -> Option<String> {
        self.engine
            .snapshot(id)
            .filter(|s| s.phase == rosorah_capture::Phase::Failed)
            .map(|s| s.detail)
    }
    pub fn video_status(&self, id: Id) -> String {
        self.engine
            .snapshot(id)
            .map(|s| s.detail)
            .unwrap_or_else(|| "Ready".into())
    }

    pub fn sync_cameras(&mut self, project: &Project) {
        let desired: std::collections::BTreeMap<_, _> = project
            .sources
            .iter()
            .filter_map(|s| {
                if let SourceKind::VideoCamera(settings) = &s.kind {
                    (settings.enabled
                        && project
                            .scenes
                            .iter()
                            .any(|scene| scene.layers.iter().any(|l| l.source == s.id)))
                    .then_some((s.id, settings))
                } else {
                    None
                }
            })
            .collect();
        let removed: Vec<_> = self
            .cameras
            .keys()
            .filter(|id| !desired.contains_key(id))
            .copied()
            .collect();
        for id in removed {
            self.engine.stop(id);
            self.cameras.remove(&id);
            self.camera_errors.remove(&id);
        }
        for (id, settings) in desired {
            if self.cameras.get(&id) != Some(settings) {
                self.engine.stop(id);
                if self.engine.running(id) {
                    continue;
                }
                self.cameras.insert(id, settings.clone());
                self.attempts.remove(&id);
            }
            if !self.engine.running(id)
                && self
                    .attempts
                    .get(&id)
                    .is_none_or(|t| t.elapsed() >= Duration::from_secs(5))
            {
                self.attempts.insert(id, std::time::Instant::now());
                if let Err(e) = self.engine.start(
                    id,
                    Box::new(crate::camera::CameraBackend(settings.clone())),
                    Options::default(),
                ) {
                    self.camera_errors.insert(id, e.to_string());
                } else {
                    self.camera_errors.remove(&id);
                }
            }
        }
    }
    pub fn camera_dimensions(&self, id: Id) -> Option<[u32; 2]> {
        self.engine.snapshot(id).and_then(|s| s.dimensions)
    }
    pub fn camera_status(&self, id: Id) -> String {
        self.camera_errors
            .get(&id)
            .cloned()
            .or_else(|| self.engine.snapshot(id).map(|s| s.detail))
            .unwrap_or_else(|| "Camera disconnected".into())
    }
    pub fn reader(&self) -> rosorah_capture::FrameReader {
        self.engine.reader()
    }
    pub fn set_repaint(&mut self, ctx: eframe::egui::Context) {
        let active = self.preview_active.clone();
        active.store(true, std::sync::atomic::Ordering::Relaxed);
        self.engine.set_wake(std::sync::Arc::new(move || {
            if active.load(std::sync::atomic::Ordering::Relaxed) { ctx.request_repaint(); }
        }));
    }
    pub fn set_preview_active(&self, active: bool) {
        self.preview_active.store(active, std::sync::atomic::Ordering::Relaxed);
    }
    pub fn restore(&mut self, project: &Project) {
        self.options.resolution = project.preferences.capture_resolution.wire();
        #[cfg(windows)] {
            self.options.target = Target::Screen;
            self.options.cursor = project.preferences.windows_capture_cursor;
            self.monitors = project.preferences.windows_capture_monitors.clone();
        }
        for source in &project.sources {
            if !matches!(source.kind, SourceKind::ScreenCapture) {
                continue;
            }
            #[cfg(windows)] {
                if project.preferences.windows_auto_sources.contains(&source.id) { self.set_mode(source.id,Target::AutoFullscreen); }
                else { self.modes.insert(source.id,Target::Screen); }
                // Explicit monitor/window capture never starts merely by importing a project.
                continue;
            }
            #[cfg(not(windows))]
            if project
                .preferences
                .manual_capture_sources
                .contains(&source.id)
            {
                self.modes.insert(source.id, Target::WindowOrScreen);
            } else if project.preferences.auto_game_capture
                && project
                    .scenes
                    .iter()
                    .any(|s| s.layers.iter().any(|l| l.source == source.id))
            {
                self.set_mode(source.id, Target::AutoFullscreen);
            }
        }
    }
    pub fn apply_game_options(&mut self) {
        {
        let ids: Vec<_> = self.armed.iter().copied().collect();
        for id in ids {
            self.set_mode(id, Target::AutoFullscreen);
        }
        }
    }
    pub fn enabled(&self, id: Id) -> bool {
        self.armed.contains(&id) || self.running(id) || self.pending.contains_key(&id)
    }

    pub fn any_running(&self) -> bool {
        self.engine.any_running() || !self.pending.is_empty() || !self.armed.is_empty()
    }
    pub fn game(&self) -> Option<rosorah_capture::GameProcess> { self.engine.reader().game() }
    pub fn running(&self, id: Id) -> bool {
        self.engine.running(id)
    }
    pub fn mode(&self, id: Id) -> Target {
        self.modes.get(&id).copied().unwrap_or(self.options.target)
    }
    pub fn set_mode(&mut self, id: Id, mode: Target) {
        self.modes.insert(id, mode);
        self.stop(id);
        if mode == Target::AutoFullscreen {
            self.armed.insert(id);
            let mut options = self.options;
            options.target = mode;
            self.pending.insert(id, options);
        }
    }
    fn desktop_backend(&self, id: Id) -> Box<dyn Backend> {
        #[cfg(not(windows))]
        { Box::new(DesktopBackend) }
        #[cfg(windows)]
        { if self.mode(id)==Target::Window { Box::new(rosorah_windows::window::WindowBackend{window:self.windows.get(&id).cloned()}) }
          else if self.options.cursor { Box::new(rosorah_windows::window::MonitorBackend{id:self.monitors.get(&id).cloned().unwrap_or_default()}) }
          else { Box::new(rosorah_windows::capture::ScreenBackend{monitor_id:self.monitors.get(&id).cloned().unwrap_or_default()}) } }
    }
    pub fn start(&mut self, id: Id) {
        let mut options = self.options;
        options.target = self.mode(id);
        self.modes.insert(id, options.target);
        if options.target == Target::AutoFullscreen {
            self.armed.insert(id);
        }
        self.attempts.insert(id, std::time::Instant::now());
        self.error = self
            .engine
            .start(
                id,
                if options.target == Target::AutoFullscreen {
                    Box::new(crate::renderer_capture::RendererBackend)
                } else {
                    self.desktop_backend(id)
                },
                options,
            )
            .err()
            .map(|e| e.to_string());
    }
    pub fn choose_manual(&mut self, id: Id) {
        self.stop(id);
        let mut options = self.options;
        options.target = if cfg!(windows) { self.mode(id) } else { Target::WindowOrScreen };
        self.modes.insert(id, options.target);
        self.pending.insert(id, options);
    }
    pub fn stop(&mut self, id: Id) {
        self.armed.remove(&id);
        self.pending.remove(&id);
        self.engine.stop(id);
    }
    pub fn status(&self, id: Id) -> String {
        if let Some(e) = &self.error {
            return e.clone();
        }
        self.engine
            .snapshot(id)
            .map(|s| {
                if s.phase == rosorah_capture::Phase::Failed && self.armed.contains(&id) {
                    format!("{} — retrying automatically", s.detail)
                } else {
                    let mut detail = s.detail;
                    if let Some((_, mailbox, upload, submit)) = self.preview_timing.get(&id) {
                        detail.push_str(&format!("\nPreview: mailbox {:.1} ms · upload CPU {:.1} ms", mailbox, upload));
                        if let Some(ms) = submit { detail.push_str(&format!(" · upload-to-submit {:.1} ms", ms)); }
                        detail.push_str("\nTimings end at GPU submission; display scanout is not measured.");
                    }
                    detail
                }
            })
            .unwrap_or_else(|| "Not connected — choose a source".into())
    }
    pub fn tick(&mut self) {
        let retries: Vec<_> = self
            .armed
            .iter()
            .copied()
            .filter(|id| {
                !self.engine.running(*id)
                    && !self.pending.contains_key(id)
                    && self
                        .attempts
                        .get(id)
                        .is_none_or(|t| t.elapsed() >= Duration::from_secs(10))
            })
            .collect();
        for id in retries {
            self.start(id);
        }

        let ready: Vec<_> = self
            .pending
            .keys()
            .copied()
            .filter(|id| !self.engine.running(*id))
            .collect();
        for id in ready {
            if let Some(options) = self.pending.remove(&id) {
                self.attempts.insert(id, std::time::Instant::now());
                self.error = self
                    .engine
                    .start(
                        id,
                        if options.target == Target::AutoFullscreen {
                            Box::new(crate::renderer_capture::RendererBackend)
                        } else {
                            self.desktop_backend(id)
                        },
                        options,
                    )
                    .err()
                    .map(|e| e.to_string());
            }
        }
    }
    pub fn preview_submitted(&mut self, sources: impl Iterator<Item = Id>) {
        for id in sources {
            let Some((start, _, _, submitted)) = self.preview_timing.get_mut(&id) else { continue; };
            if submitted.is_none() { *submitted = Some(start.elapsed().as_secs_f64()*1000.); }
        }
    }
    pub fn upload(&mut self, gpu: &mut auroracast_gpu::Compositor) -> Result<bool> {
        let updates = self.engine.drain();
        let changed = !updates.is_empty();
        for (id, frame) in updates {
            let start = std::time::Instant::now();
            match frame {
                Some(f) => {
                    gpu.upload(id, f.width, f.height, &f.rgba)?;
                    let mailbox = self.engine.snapshot(id).and_then(|s| s.mailbox_ms).unwrap_or(0.);
                    self.preview_timing.insert(id, (start, mailbox, start.elapsed().as_secs_f64()*1000., None));
                },
                None => { gpu.upload(id, 1, 1, &[0, 0, 0, 0])?; self.preview_timing.remove(&id); },
            }
        }
        Ok(changed)
    }
}

struct Playing(gst::Pipeline);
impl Drop for Playing {
    fn drop(&mut self) {
        let _ = self.0.set_state(gst::State::Null);
    }
}

#[cfg(not(windows))]
async fn run(
    output: &Producer,
    cancel: &mut watch::Receiver<bool>,
    options: Options,
) -> Result<()> {
    let portal = tokio::select! { r = tokio::time::timeout(Duration::from_secs(8),async{let bus=zbus::Connection::session().await.map_err(ashpd::Error::from)?;Screencast::with_connection(bus).await}) => r, _ = cancel.changed() => return Ok(()) };
    let proxy=match portal{Ok(Ok(proxy))=>proxy,_ if crate::desktop::session()==crate::desktop::Session::X11=>return run_x11(output,cancel,options).await,_=>bail!("Screen sharing is unavailable. Enable xdg-desktop-portal and the backend for your current desktop. See Edit > Desktop compatibility.")};
    let capabilities=tokio::select!{ r=tokio::time::timeout(Duration::from_secs(5),async{Ok::<_,ashpd::Error>((proxy.available_source_types().await?.bits(),proxy.available_cursor_modes().await?.bits()))})=>r,_=cancel.changed()=>return Ok(())};
    let capabilities=match capabilities{Ok(Ok(c))=>c,_ if crate::desktop::session()==crate::desktop::Session::X11=>return run_x11(output,cancel,options).await,_=>bail!("This desktop does not provide working screen-sharing capabilities. See Edit > Desktop compatibility.")};
    let modes=crate::desktop::capture_modes(options.target,capabilities.0,capabilities.1,options.cursor);
    let (sources,cursor,notes)=match modes{Ok(v)=>v,Err(_) if crate::desktop::session()==crate::desktop::Session::X11=>return run_x11(output,cancel,options).await,Err(e)=>return Err(e)};
    for note in notes{output.note(note);crate::capture_log::event(note);}
    anyhow::ensure!(gst::ElementFactory::find("pipewiresrc").is_some(),"The desktop portal is available, but GStreamer's PipeWire source is missing. Install your distribution's GStreamer PipeWire plugin.");
    // Session creation is bounded; once created, every exit explicitly closes it.
    let session = tokio::time::timeout(
        Duration::from_secs(10),
        proxy.create_session(Default::default()),
    )
    .await??;
    let result = async {
        let mut closed = session.receive_closed().await?;
        let work = async {
            proxy
                .select_sources(
                    &session,
                    SelectSourcesOptions::default()
                        .set_cursor_mode(if cursor==2{CursorMode::Embedded}else{CursorMode::Hidden})
                        .set_sources(match sources{1=>SourceType::Monitor.into(),2=>SourceType::Window.into(),_=>SourceType::Monitor | SourceType::Window})
                        .set_multiple(false)
                        .set_persist_mode(ashpd::desktop::PersistMode::DoNot),
                )
                .await?
                .response()?;
            let response = proxy
                .start(&session, None, Default::default())
                .await?
                .response()?;
            let stream = response
                .streams()
                .first()
                .context("No window or screen selected")?;
            let fd = proxy
                .open_pipe_wire_remote(&session, Default::default())
                .await?;
            let source = gst::ElementFactory::make("pipewiresrc")
                .property("fd", fd.as_raw_fd())
                .property("path", stream.pipe_wire_node_id().to_string())
                .property("do-timestamp", true)
                .build()?;
            let (pipeline, sink) = pipeline(source)?;
            let _playing = Playing(pipeline.clone());
            pipeline.set_state(gst::State::Playing)?;
            output.connecting();
            let bus = pipeline.bus().context("Missing capture message bus")?;
            let started = std::time::Instant::now();
            let mut received = false;
            loop {
                while let Some(msg) = bus.pop() {
                    match msg.view() {
                        gst::MessageView::Error(e) => bail!("{}", e.error()),
                        gst::MessageView::Eos(_) => {
                            bail!("Selected window closed or capture ended")
                        }
                        _ => (),
                    }
                }
                if let Some(sample) = sink.try_pull_sample(gst::ClockTime::ZERO) {
                    let frame = decode(&sample)?;
                    output.publish(frame)?;
                    received = true;
                }
                if !received && started.elapsed() > Duration::from_secs(15) {
                    bail!("No frames received from PipeWire; stop and choose the source again");
                }
                tokio::time::sleep(Duration::from_millis(1000 / u64::from(options.fps))).await;
            }
        };
        tokio::select! {
            r = work => r,
            _ = cancel.changed() => Ok(()),
            _ = closed.next() => bail!("Desktop sharing permission ended"),
        }
    }
    .await;
    let _ = tokio::time::timeout(Duration::from_secs(3), session.close()).await;
    result
}

pub(crate) fn pipeline(source: gst::Element) -> Result<(gst::Pipeline, gst_app::AppSink)> {
    let convert = gst::ElementFactory::make("videoconvert").build()?;
    let scale = gst::ElementFactory::make("videoscale").build()?;
    let sink = gst_app::AppSink::builder()
        .caps(
            &gst::Caps::builder("video/x-raw")
                .field("format", "RGBA")
                .field("width", gst::IntRange::<i32>::new(1, 4096))
                .field("height", gst::IntRange::<i32>::new(1, 4096))
                .build(),
        )
        .max_buffers(1)
        .drop(true)
        .sync(false)
        .build();
    let pipeline = gst::Pipeline::default();
    pipeline.add_many([&source, &convert, &scale, sink.upcast_ref()])?;
    gst::Element::link_many([&source, &convert, &scale, sink.upcast_ref()])?;
    Ok((pipeline, sink))
}
pub(crate) fn decode(sample: &gst::Sample) -> Result<Frame> {
    let info = gst_video::VideoInfo::from_caps(sample.caps().context("Frame has no caps")?)?;
    let buffer = sample.buffer().context("Frame has no buffer")?;
    let mapped = gst_video::VideoFrameRef::from_buffer_ref_readable(buffer, &info)?;
    let width = info.width();
    let height = info.height();
    anyhow::ensure!(
        width > 0 && height > 0 && width <= 4096 && height <= 4096,
        "Capture dimensions out of bounds"
    );
    let stride = usize::try_from(mapped.plane_stride()[0]).context("Negative capture stride")?;
    let data = mapped.plane_data(0)?;
    Ok(Frame {
        width,
        height,
        rgba: copy_rows(data, width as usize * 4, height as usize, stride)?,
    })
}
fn copy_rows(data: &[u8], row: usize, height: usize, stride: usize) -> Result<Vec<u8>> {
    anyhow::ensure!(
        stride >= row && data.len() >= (height.saturating_sub(1)) * stride + row,
        "Truncated capture frame"
    );
    let mut rgba = Vec::with_capacity(row * height);
    for y in 0..height {
        rgba.extend_from_slice(&data[y * stride..y * stride + row]);
    }
    Ok(rgba)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(not(windows))]
    fn restore_arms_automatic_but_never_opens_saved_manual_picker() {
        let mut project = Project::default();
        let id = project.id();
        project.sources.push(auroracast_core::Source {
            id,
            name: "test".into(),
            kind: SourceKind::ScreenCapture,
        });
        project.scenes[0].layers.push(auroracast_core::Layer { crop: [0.;4], group: 0,
            source: id,
            rect: [0., 0., 1., 1.],
            visible: true,
            locked: false,
            opacity: 1.,
        });
        let mut captures = Captures::default();
        captures.restore(&project);
        assert!(captures.armed.contains(&id));
        assert!(captures.pending.contains_key(&id));
        captures.stop(id);
        assert!(!captures.enabled(id));
        captures.tick();
        assert!(!captures.running(id));
        project.preferences.manual_capture_sources.push(id);
        captures.restore(&project);
        assert_eq!(captures.mode(id), Target::WindowOrScreen);
        assert!(!captures.enabled(id));
    }

    #[test]
    fn padded_rows_and_truncation() {
        assert_eq!(
            copy_rows(&[1, 2, 3, 4, 99, 99, 5, 6, 7, 8], 4, 2, 6).unwrap(),
            [1, 2, 3, 4, 5, 6, 7, 8]
        );
        assert!(copy_rows(&[0; 8], 4, 2, 6).is_err());
        assert!(copy_rows(&[0; 8], 4, 2, 3).is_err());
    }
    #[test]
    fn real_gstreamer_frames_and_shutdown() -> Result<()> {
        gst::init()?;
        let source = gst::ElementFactory::make("videotestsrc")
            .property("num-buffers", 3i32)
            .build()?;
        let (pipeline, sink) = pipeline(source)?;
        let guard = Playing(pipeline.clone());
        pipeline.set_state(gst::State::Playing)?;
        let sample = sink
            .try_pull_sample(gst::ClockTime::from_seconds(5))
            .context("No test frame")?;
        let f = decode(&sample)?;
        assert_eq!(f.rgba.len(), (f.width * f.height * 4) as usize);
        assert!(f.rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
        drop(guard);
        assert_eq!(pipeline.current_state(), gst::State::Null);
        Ok(())
    }
}

#[cfg(test)]
mod widget_tests {
    use super::*;
    #[test]
    fn widget_frames_are_shared_updated_and_stop_when_hidden() -> anyhow::Result<()> {
        let mut p = Project::default();
        let id = p.sources[0].id;
        p.sources[0].kind = SourceKind::Widget(Default::default());
        // Both canvases show the same source; only one renderer is created.
        let scene = p.canvases[0].scene;
        for c in &mut p.canvases {
            c.scene = scene;
        }
        let mut captures = Captures::default();
        captures.sync_widgets(&p);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let first = loop {
            if let Some((_, Some(frame))) = captures
                .reader()
                .latest()
                .into_iter()
                .find(|(source, _)| *source == id)
            {
                break frame;
            }
            anyhow::ensure!(std::time::Instant::now() < deadline, "No widget frame");
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        assert_eq!(captures.widgets.0.lock().unwrap().len(), 1);
        captures.widgets.update(id, "", 1, false);
        loop {
            if captures
                .reader()
                .latest()
                .into_iter()
                .find(|(source, _)| *source == id)
                .and_then(|(_, f)| f)
                .is_some_and(|f| f.rgba != first.rgba)
            {
                break;
            }
            anyhow::ensure!(
                std::time::Instant::now() < deadline,
                "Counter frame did not change"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        for s in &mut p.scenes {
            for l in &mut s.layers {
                if l.source == id {
                    l.visible = false;
                }
            }
        }
        captures.sync_widgets(&p);
        assert!(captures.widget_running.is_empty());
        Ok(())
    }
}

#[cfg(not(windows))]
fn parse_xid(text:&str)->Result<u64>{let line=text.lines().find(|l|l.contains("Window id:")).context("No X11 window selected")?;let value=line.split("Window id:").nth(1).unwrap_or("").split_whitespace().next().context("Missing X11 window ID")?;let id=value.strip_prefix("0x").map_or_else(||value.parse(),|v|u64::from_str_radix(v,16))?;anyhow::ensure!(id!=0,"Invalid X11 window ID");Ok(id)}
#[cfg(not(windows))]
async fn x11_info(args:&[&str],cancel:&mut watch::Receiver<bool>,seconds:u64)->Result<String>{
 use std::process::{Command,Stdio};struct Child(std::process::Child);impl Drop for Child{fn drop(&mut self){let _=self.0.kill();let _=self.0.wait();}}
 let mut child=Child(Command::new("xwininfo").args(args).env("LC_ALL","C").stdout(Stdio::piped()).stderr(Stdio::null()).spawn().context("X11 window selection needs xwininfo (xorg-x11-utils on Fedora, x11-utils on Debian/Ubuntu)")?);let start=std::time::Instant::now();
 loop{if *cancel.borrow(){return Ok(String::new())}if let Some(status)=child.0.try_wait()?{anyhow::ensure!(status.success(),"X11 window selection cancelled or unavailable");let mut text=String::new();use std::io::Read;child.0.stdout.take().unwrap().read_to_string(&mut text)?;return Ok(text)}anyhow::ensure!(start.elapsed()<Duration::from_secs(seconds),"X11 window selection timed out");tokio::time::sleep(Duration::from_millis(30)).await;}
}
#[cfg(not(windows))]
async fn run_x11(output:&Producer,cancel:&mut watch::Receiver<bool>,options:Options)->Result<()>{
 anyhow::ensure!(crate::desktop::session()==crate::desktop::Session::X11,"X11 fallback is unavailable in a Wayland session");anyhow::ensure!(gst::ElementFactory::find("ximagesrc").is_some(),"X11 capture requires GStreamer's ximagesrc plugin (GStreamer good plugins)");
 let xid=if options.target==Target::Screen{0}else{
  output.note("Desktop portal unavailable for this request. Click the X11 window to capture; Stop cancels selection.");
  let selected=x11_info(&["-int"],cancel,120).await?;if *cancel.borrow(){return Ok(())}let id=parse_xid(&selected)?;
  if options.target==Target::Window{let root=x11_info(&["-root","-int"],cancel,5).await?;if *cancel.borrow(){return Ok(())}anyhow::ensure!(id!=parse_xid(&root)?,"The desktop was selected. Choose Screen capture to share the entire desktop, or select a window.");}id
 };
 let source=gst::ElementFactory::make("ximagesrc").property("xid",xid).property("show-pointer",options.cursor).property("use-damage",false).property("do-timestamp",true).build()?;
 let (pipeline,sink)=pipeline(source)?;sink.set_caps(Some(&gst::Caps::builder("video/x-raw").field("format","RGBA").field("width",gst::IntRange::<i32>::new(1,4096)).field("height",gst::IntRange::<i32>::new(1,4096)).field("framerate",gst::Fraction::new(options.fps.clamp(1,240)as i32,1)).build()));let _playing=Playing(pipeline.clone());pipeline.set_state(gst::State::Playing)?;output.connecting();crate::capture_log::event("Using explicit X11 capture fallback");let bus=pipeline.bus().context("Missing X11 capture message bus")?;let started=std::time::Instant::now();let mut received=false;
 loop{if *cancel.borrow(){break}while let Some(msg)=bus.pop(){match msg.view(){gst::MessageView::Error(e)=>bail!("X11 capture: {}",e.error()),gst::MessageView::Eos(_)=>bail!("Selected X11 window closed"),_=>()}}
 if let Some(sample)=sink.try_pull_sample(gst::ClockTime::ZERO){output.publish(decode(&sample)?)?;received=true;}anyhow::ensure!(received||started.elapsed()<Duration::from_secs(15),"No frames received from the selected X11 source");tokio::time::sleep(Duration::from_millis(1000/u64::from(options.fps.max(1)))).await;}Ok(())
}
#[cfg(all(test, not(windows)))]mod x11_tests{use super::*;#[test]fn accepts_decimal_and_hex_window_ids(){assert_eq!(parse_xid("xwininfo: Window id: 12345 title").unwrap(),12345);assert_eq!(parse_xid("xwininfo: Window id: 0x20 title").unwrap(),32);assert!(parse_xid("bad output").is_err());assert!(parse_xid("Window id: 0").is_err());}}

#[cfg(all(test, not(windows)))]mod desktop_integration_tests{
 use super::*;
 #[test]#[ignore="Requires an isolated X11 display and GStreamer good plugins"]
 fn x11_fallback_delivers_frames_and_stops()->Result<()>{
  anyhow::ensure!(crate::desktop::session()==crate::desktop::Session::X11,"Run inside the X11 test display");let mut engine=Engine::default();let reader=engine.reader();engine.start(991,Box::new(DesktopBackend),Options{target:Target::Screen,fps:30,..Default::default()})?;
  let start=std::time::Instant::now();let frame=loop{if let Some(frame)=reader.latest().into_iter().find(|(id,_)|*id==991).and_then(|(_,frame)|frame){break frame}anyhow::ensure!(start.elapsed()<Duration::from_secs(15),"No X11 frame: {:?}",engine.snapshot(991));std::thread::sleep(Duration::from_millis(20));};anyhow::ensure!(frame.width>0&&frame.height>0&&frame.rgba.len()==(frame.width*frame.height*4)as usize,"Bad X11 image");engine.stop(991);let start=std::time::Instant::now();while engine.snapshot(991).is_some_and(|s|s.phase.running()){anyhow::ensure!(start.elapsed()<Duration::from_secs(3),"X11 capture did not stop");std::thread::sleep(Duration::from_millis(20));}Ok(())
 }
}

#[cfg(all(test,windows))]
mod windows_selection_tests {
    use super::*;
    #[test]
    fn explicit_windows_auto_opt_in_is_restored_without_monitor_fallback() {
        let mut project=Project::default();let id=project.sources[0].id;
        project.sources[0].kind=SourceKind::ScreenCapture;
        project.preferences.windows_auto_sources.push(id);
        project.preferences.windows_capture_cursor=true;
        let mut captures=Captures::default();captures.restore(&project);
        assert!(captures.enabled(id));assert_eq!(captures.mode(id),Target::AutoFullscreen);
        assert!(captures.options.cursor);assert!(captures.monitors.is_empty());
    }
    #[test]
    fn imported_project_never_auto_captures_a_monitor() {
        let mut project=Project::default();
        let id=project.sources[0].id;
        project.sources[0].kind=SourceKind::ScreenCapture;
        project.preferences.auto_game_capture=true;
        project.preferences.windows_capture_monitors.insert(id,r"\\.\DISPLAY2".into());
        let mut captures=Captures::default();
        captures.restore(&project);
        assert_eq!(captures.mode(id),Target::Screen);
        assert!(!captures.enabled(id));
        assert!(!captures.options.cursor);
        assert_eq!(captures.monitors.get(&id),project.preferences.windows_capture_monitors.get(&id));
        captures.set_mode(id,Target::AutoFullscreen);
        assert!(captures.enabled(id));
        assert_eq!(captures.mode(id),Target::AutoFullscreen);
    }
}
