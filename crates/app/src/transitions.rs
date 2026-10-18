// Stinger and transition-screen scene switches. The media plays as an overlay on every
// canvas that changes scene; the switch itself happens underneath, at the cut point.
struct ActiveTransition {
    kind: auroracast_core::Transition,
    source: Id,
    canvases: Vec<Id>,
    /// Canvas and scene to switch to; taken when the switch happens.
    switch: Option<(Id, Id)>,
    requested: Instant,
    /// Starts when the media shows its first frame, so slow starts don't cut early.
    started: Option<Instant>,
}
impl Studio {
    /// Switches scene, using the stinger or transition screen when one is set up.
    fn request_scene(&mut self, canvas: Id, scene: Id) {
        // In Studio Mode, picking a scene for the main video prepares it in the preview instead.
        if self.project.preferences.workspace.studio_mode
            && !self.studio_taking
            && self.project.canvases.first().is_some_and(|c| c.id == canvas)
        {
            self.studio_preview = Some(scene);
            self.revision = true;
            return;
        }
        let media = &self.project.preferences.transition_media;
        let source = match self.project.preferences.transition {
            auroracast_core::Transition::Stinger => media.stinger,
            auroracast_core::Transition::Screen => media.screen,
            _ => None,
        }
        .filter(|id| {
            self.project.source(*id).is_some_and(|s| matches!(&s.kind, SourceKind::VideoFile(v) if std::path::Path::new(&v.path).is_file()))
        });
        // Every canvas whose scene would change gets the overlay (linked scenes included).
        let mut after = self.project.clone();
        after.switch_scene(canvas, scene);
        let canvases: Vec<Id> = self
            .project
            .canvases
            .iter()
            .zip(&after.canvases)
            .filter(|(a, b)| a.scene != b.scene)
            .map(|(a, _)| a.id)
            .collect();
        match source {
            Some(source) if self.scene_transition.is_none() && !canvases.is_empty() => {
                let kind = self.project.preferences.transition;
                if kind == auroracast_core::Transition::Screen
                    && let Some(Source { kind: SourceKind::VideoFile(v), .. }) = self.project.sources.iter_mut().find(|s| s.id == source)
                {
                    // A picture plays as a still for this long.
                    let total = 2 * media_ms(media.fade_ms) + media.hold_ms;
                    v.duration_secs = total.div_ceil(1000).max(1);
                }
                for id in &canvases {
                    self.project.preferences.overlays.insert(*id, auroracast_core::Overlay { source, opacity: 0. });
                }
                self.captures.sync_videos(&self.project);
                self.captures.videos.trigger(source);
                self.scene_transition = Some(ActiveTransition { kind, source, canvases, switch: Some((canvas, scene)), requested: Instant::now(), started: None });
                self.push_outputs();
            }
            _ => {
                self.project.switch_scene(canvas, scene);
                self.layer = None;
                self.changed();
            }
        }
    }
    /// Advances a running stinger or screen transition. Call once per frame.
    fn tick_transition(&mut self, ctx: &egui::Context) {
        let Some(t) = &mut self.scene_transition else { return };
        let media = self.project.preferences.transition_media.clone();
        let has_frame = self.captures.reader().latest().into_iter().any(|(id, f)| id == t.source && f.is_some_and(|f| f.width > 1));
        if t.started.is_none() && (has_frame || t.requested.elapsed() > Duration::from_millis(1500)) {
            t.started = Some(Instant::now());
        }
        let Some(start) = t.started else {
            ctx.request_repaint_after(Duration::from_millis(16));
            return;
        };
        let elapsed = start.elapsed().as_millis() as u32;
        let stinger = t.kind == auroracast_core::Transition::Stinger;
        let fade = media_ms(media.fade_ms);
        let cut = if stinger { media.cut_ms } else { fade + media.hold_ms / 2 };
        let mut switch = None;
        if elapsed >= cut {
            switch = t.switch.take();
        }
        let opacity = if stinger {
            1.
        } else if elapsed < fade {
            elapsed as f32 / fade as f32
        } else if elapsed < fade + media.hold_ms {
            1.
        } else {
            1. - (elapsed - fade - media.hold_ms) as f32 / fade as f32
        };
        let done = if stinger {
            // The video ends on its own; never let a stuck video cover the stream for long.
            t.switch.is_none() && ((elapsed > cut + 300 && !self.captures.running(t.source)) || elapsed > 30_000)
        } else {
            elapsed >= 2 * fade + media.hold_ms
        };
        let (source, canvases) = (t.source, t.canvases.clone());
        if let Some((canvas, scene)) = switch {
            self.project.switch_scene(canvas, scene);
            self.layer = None;
            self.changed();
        }
        if done {
            for id in &canvases {
                self.project.preferences.overlays.remove(id);
            }
            self.captures.stop_alert_source(source);
            self.scene_transition = None;
        } else {
            for id in &canvases {
                self.project.preferences.overlays.insert(*id, auroracast_core::Overlay { source, opacity: opacity.clamp(0., 1.) });
            }
            ctx.request_repaint_after(Duration::from_millis(16));
        }
        self.push_outputs();
    }
    /// Sends the current state to live outputs without marking the project unsaved.
    fn push_outputs(&mut self) {
        self.audio_scene = None;
        self.revision = true;
        self.vertical_revision = true;
        if let Some(r) = &self.live_recording {
            r.update_project(&self.project);
        }
        if let Some(stream) = &self.live_stream {
            stream.update_project(&self.project);
        }
    }
    /// The Transition menu in the Scenes panel.
    fn transition_menu(&mut self, ui: &mut egui::Ui) {
        use auroracast_core::Transition;
        if let Some((stinger, rx)) = &self.transition_picker
            && let Ok(path) = rx.try_recv()
        {
            let stinger = *stinger;
            self.transition_picker = None;
            if let Some(path) = path {
                let id = self.transition_source(&path, stinger);
                let media = &mut self.project.preferences.transition_media;
                if stinger { media.stinger = Some(id) } else { media.screen = Some(id) }
                self.project.preferences.transition = if stinger { Transition::Stinger } else { Transition::Screen };
                self.changed();
            }
        }
        let before = (self.project.preferences.transition, self.project.preferences.transition_ms, self.project.preferences.transition_media.clone(), self.project.preferences.transition_style.clone());
        for kind in [Transition::Cut, Transition::Fade, Transition::Wipe, Transition::Slide, Transition::Swipe, Transition::PatternWipe, Transition::FadeToColor, Transition::Move, Transition::Stinger, Transition::Screen] {
            ui.selectable_value(&mut self.project.preferences.transition, kind, kind.label());
        }
        ui.separator();
        match self.project.preferences.transition {
            Transition::Fade | Transition::Wipe => {
                ui.add(egui::DragValue::new(&mut self.project.preferences.transition_ms).range(0..=2000).suffix(" ms"));
            }
            Transition::Move => {
                ui.add(egui::DragValue::new(&mut self.project.preferences.transition_ms).range(100..=2000).suffix(" ms"));
                note(ui, "Layers showing the same thing in both scenes (your camera, a game, an image) glide to their new place and size. Everything else fades.");
            }
            Transition::Slide | Transition::Swipe => {
                ui.add(egui::DragValue::new(&mut self.project.preferences.transition_ms).range(100..=2000).suffix(" ms"));
            }
            Transition::PatternWipe => {
                ui.add(egui::DragValue::new(&mut self.project.preferences.transition_ms).range(100..=3000).suffix(" ms"));
                let style = &mut self.project.preferences.transition_style;
                ui.horizontal_wrapped(|ui| {
                    for (id, name) in auroracast_core::WIPE_PATTERNS {
                        ui.selectable_value(&mut style.pattern, id, name);
                    }
                });
                ui.add(egui::Slider::new(&mut style.softness, 0.0..=0.5).text("Soft edge").show_value(false));
            }
            Transition::FadeToColor => {
                ui.add(egui::DragValue::new(&mut self.project.preferences.transition_ms).range(100..=3000).suffix(" ms"));
                ui.horizontal(|ui| {
                    ui.label("Colour");
                    ui.color_edit_button_srgb(&mut self.project.preferences.transition_style.color);
                });
            }
            Transition::Stinger => {
                self.transition_file_row(ui, true);
                ui.horizontal(|ui| {
                    ui.label("Switch scene at");
                    ui.add(egui::DragValue::new(&mut self.project.preferences.transition_media.cut_ms).range(0..=10_000).speed(10).suffix(" ms"));
                })
                .response
                .on_hover_text("When the stinger fully covers the screen, measured from its start");
                note(ui, "Use a video with transparency (WebM VP9 with alpha, or MOV ProRes 4444) so the scenes show around it.");
            }
            Transition::Screen => {
                self.transition_file_row(ui, false);
                ui.horizontal(|ui| {
                    ui.label("Fade");
                    ui.add(egui::DragValue::new(&mut self.project.preferences.transition_media.fade_ms).range(50..=3000).speed(10).suffix(" ms"));
                    ui.label("Hold");
                    ui.add(egui::DragValue::new(&mut self.project.preferences.transition_media.hold_ms).range(0..=10_000).speed(10).suffix(" ms"));
                });
                note(ui, "Fades to the picture, switches scene while it's showing, then fades out. A short video also works.");
            }
            Transition::Cut => {}
        }
        if before != (self.project.preferences.transition, self.project.preferences.transition_ms, self.project.preferences.transition_media.clone(), self.project.preferences.transition_style.clone()) {
            self.changed();
        }
    }
    fn transition_file_row(&mut self, ui: &mut egui::Ui, stinger: bool) {
        let media = &self.project.preferences.transition_media;
        let current = if stinger { media.stinger } else { media.screen };
        let name = current.and_then(|id| self.project.source(id)).and_then(|s| match &s.kind {
            SourceKind::VideoFile(v) => std::path::Path::new(&v.path).file_name().map(|n| n.to_string_lossy().into_owned()),
            _ => None,
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(name.unwrap_or_else(|| if stinger { "No video chosen".into() } else { "No picture chosen".into() }));
            if ui.add_enabled(self.transition_picker.is_none(), egui::Button::new(if stinger { "Choose video…" } else { "Choose picture…" })).clicked() {
                let (tx, rx) = mpsc::channel();
                self.transition_picker = Some((stinger, rx));
                std::thread::spawn(move || {
                    let path = crate::desktop::pick(crate::desktop::Pick::Open, if stinger { "Choose a stinger video" } else { "Choose a transition picture" }, "").unwrap_or_else(|e| {
                        crate::capture_log::event(e.to_string());
                        None
                    });
                    let _ = tx.send(path);
                });
            }
        });
        if let Some(id) = current
            && let Some(Source { kind: SourceKind::VideoFile(v), .. }) = self.project.sources.iter_mut().find(|s| s.id == id)
        {
            let mut changed = false;
            ui.horizontal(|ui| {
                changed |= ui.checkbox(&mut v.audio, "Play its sound").changed();
                let mut percent = (v.gain * 100.).round();
                if ui.add(egui::Slider::new(&mut percent, 0.0..=200.0).suffix("%")).changed() {
                    v.gain = percent / 100.;
                    changed = true;
                }
            });
            if changed {
                self.changed();
            }
        }
    }
    /// Reuses or creates the hidden media source (with a mixer channel) for a transition file.
    fn transition_source(&mut self, path: &std::path::Path, stinger: bool) -> Id {
        let text = path.to_string_lossy().into_owned();
        if let Some(existing) = self.project.sources.iter().find(|s| matches!(&s.kind, SourceKind::VideoFile(v) if v.path == text && v.automation_only)) {
            return existing.id;
        }
        let id = self.project.id();
        let label = if stinger { "Stinger" } else { "Transition screen" };
        self.project.sources.push(Source {
            id,
            name: format!("{label} · {}", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
            kind: SourceKind::VideoFile(auroracast_core::VideoSettings { path: text, audio: true, looped: false, automation_only: true, ..Default::default() }),
        });
        let audio = self.project.id();
        self.project.audio.push(auroracast_core::AudioChannel {
            id: audio,
            name: label.into(),
            gain_db: 0.,
            muted: false,
            tracks: [true, false, false, false, false, false],
            delay_ms: 0,
            monitor: false,
            monitor_only: false,
            noise_suppression: Default::default(),
            media_source: Some(id),
        });
        id
    }
}
/// Fades need at least 50 ms to read as a fade.
fn media_ms(ms: u32) -> u32 {
    ms.max(50)
}
