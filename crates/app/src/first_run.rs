// The setup guide, opened automatically on first launch and from Help > Setup guide.
impl Studio {
    fn setup_window(&mut self, ctx: &egui::Context) {
        let Some(mut step) = self.tools.guide else { return };
        let mut open = true;
        let titles = ["What will you use Rosorah for?", "Sign in to your accounts", "Check your microphone", "Add a camera (optional)", "Make a test recording"];
        pop_window(ctx, "Welcome to Rosorah")
            .frame(solid_window(ctx))
            .fade_in(false)
            .open(&mut open)
            .collapsible(false)
            .default_width(540.)
            .show(ctx, |ui| {
                ui.heading(format!("{} of 5 · {}", step + 1, titles[step]));
                ui.add_space(6.);
                match step {
                    0 => self.guide_start(ui, &mut step),
                    1 => {
                        ui.label("Signing in lets Rosorah show your chat and alerts, and fills in your stream settings for you.");
                        for (name, connected) in self
                            .accounts
                            .iter()
                            .map(|a| (a.platform.name(), a.connected))
                            .chain(std::iter::once(("Kick", self.kick_account.connected)))
                        {
                            ui.label(format!("{} {name}", if connected { "✔" } else { "○" }));
                        }
                        if ui.add(self.primary_button("Sign in…")).clicked() {
                            self.page = Page::Settings;
                            self.settings = Settings::Accounts;
                        }
                        note(ui, "You can skip this and sign in later from Settings > Accounts.");
                    }
                    2 => self.guide_microphone(ui),
                    3 => {
                        ui.label("A webcam, USB camera or phone (with DroidCam) can appear in your video. You can skip this.");
                        let scene = self.project.canvases[0].scene;
                        let camera = self.project.scene(scene).and_then(|s| {
                            s.layers.iter().position(|l| self.project.source(l.source).is_some_and(|s| matches!(s.kind, SourceKind::VideoCamera(_))))
                        });
                        let label = if camera.is_some() { "Choose my camera…" } else { "Add a camera to this scene" };
                        if ui.add(self.primary_button(label)).clicked() {
                            self.page = Page::Studio;
                            self.canvas = 0;
                            if let Some(si) = self.project.scenes.iter().position(|s| s.id == scene) {
                                let index = camera.unwrap_or_else(|| {
                                    let c = &self.project.canvases[0];
                                    let w = 0.28;
                                    let rect = [0.68, 0.05, w, w * c.width as f32 / c.height as f32 * 9. / 16.];
                                    self.add_source_layer(si, "Camera", SourceKind::VideoCamera(Default::default()), rect);
                                    self.project.scenes[si].layers.len() - 1
                                });
                                self.layer_properties = Some((scene, index));
                            }
                        }
                    }
                    _ => {
                        ui.label("Record ten seconds of your main video and sound. It stays on your computer and doesn't go live.");
                        if ui.add_enabled(self.live_recording.is_none() && self.live_stream.is_none(), self.primary_button("Record a 10-second test")).clicked() {
                            self.start_recording();
                            self.tools.test_pending = self.live_recording.is_some();
                        }
                        if self.tools.test_pending || self.tools.test_until.is_some() {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label("Recording your test…");
                            });
                        }
                        if !self.tools.recording_file.is_empty()
                            && std::path::Path::new(&self.tools.recording_file).is_file()
                            && ui.button("Play my test recording").clicked()
                            && let Ok(mut child) = crate::desktop::open(&self.tools.recording_file)
                        {
                            std::thread::spawn(move || {
                                let _ = child.wait();
                            });
                        }
                        ui.label(&self.tools.status);
                        if !self.recording_error.is_empty() {
                            self.error_line(ui, self.recording_error.clone());
                        }
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.add_enabled(step > 0, egui::Button::new("Back")).clicked() {
                        step -= 1;
                    }
                    if step == 0 {
                        // The first step moves on by choosing an option.
                    } else if step < 4 {
                        if ui.button(if step == 3 { "Next (or skip)" } else { "Next" }).clicked() {
                            step += 1;
                        }
                    } else if ui.add(self.primary_button("Finish")).clicked() {
                        self.tools.guide = None;
                        self.page = Page::Studio;
                        self.tell("You're set up. Press Go live when you're ready.".into());
                    }
                });
            });
        if !open {
            self.tools.guide = None;
        } else if self.tools.guide.is_some() {
            self.tools.guide = Some(step);
        }
    }
    fn guide_start(&mut self, ui: &mut egui::Ui, step: &mut usize) {
        ui.label(if self.first_run {
            "Pick one and Rosorah sets up your scenes. You can change everything later."
        } else {
            "Pick one to add a ready-made set of scenes. Your existing scenes are kept."
        });
        ui.add_space(6.);
        for (kind, title, detail) in [
            (Starter::Games, "Stream games", "Your game full screen, with Starting soon and Be right back screens"),
            (Starter::Chatting, "Just chatting", "Your camera full screen, with Starting soon and Be right back screens"),
            (Starter::Recording, "Record videos", "Your screen, recorded to your computer"),
        ] {
            let button = egui::Button::new(RichText::new(format!("{title}\n{detail}")).size(15.)).min_size(Vec2::new(ui.available_width(), 52.));
            if ui.add(button).clicked() {
                self.apply_starter(kind);
                *step = 1;
            }
        }
        ui.add_space(6.);
        if self.project.canvases.len() > 1 {
            let before = self.project.preferences.workspace.vertical_video;
            ui.checkbox(&mut self.project.preferences.workspace.vertical_video, "Also make a vertical video for TikTok, YouTube Shorts or Reels");
            if before != self.project.preferences.workspace.vertical_video {
                self.sync_vertical_panel();
                self.changed();
            }
        }
        if ui.button("Skip, I'll set it up myself").clicked() {
            *step = 1;
        }
        ui.collapsing("Coming from OBS? Import your scenes", |ui| self.external_import_ui(ui));
    }
    /// Pick the microphone, then watch it react to your voice.
    fn guide_microphone(&mut self, ui: &mut egui::Ui) {
        let view = self.audio.view();
        let scene = self.project.canvases[0].scene;
        let Some(mic) = self.project.mixer_channels(scene).into_iter().find(|c| c.media_source.is_none() && c.name.to_lowercase().contains("mic")) else {
            ui.label("Add your microphone in the Sound panel: open Options, then Refresh inputs.");
            return;
        };
        ui.label("Choose your microphone, then say something. The bar should move.");
        let selected = self.audio.selected.entry(mic.id).or_default();
        let before = selected.clone();
        egui::ComboBox::from_id_salt("guide-mic")
            .width(ui.available_width().min(360.))
            .selected_text(view.devices.iter().find(|d| &d.serial == selected).map_or("Choose your microphone…", |d| d.label.as_str()))
            .show_ui(ui, |ui| {
                for d in view.devices.iter().filter(|d| d.kind == "Audio/Source") {
                    ui.selectable_value(selected, d.serial.clone(), &d.label);
                }
            });
        if *selected != before {
            let on = !selected.is_empty();
            self.audio.enabled.insert(mic.id, on);
        }
        crate::material::meter_plain(ui, view.stereo_peaks.get(&mic.id).copied().unwrap_or([0.; 2]));
        if mic.muted {
            ui.horizontal(|ui| {
                ui.label("Your microphone is muted.");
                if ui.button("Unmute").clicked() {
                    self.apply_fix(Fix::Unmute(mic.id));
                }
            });
        }
        if ui.button("Hear yourself in headphones…").clicked() {
            self.tools.audio = true;
        }
        ui.ctx().request_repaint_after(Duration::from_millis(50));
    }
    /// Builds ready-made scenes. On first run they replace the demo scenes; otherwise they are added.
    fn apply_starter(&mut self, kind: Starter) {
        let replace = self.first_run;
        self.first_run = false;
        let main = self.project.canvases[0].id;
        if replace {
            // The demo pattern and placeholders mean nothing to a new user.
            let demo: Vec<Id> = self
                .project
                .sources
                .iter()
                .filter(|s| matches!(s.kind, SourceKind::TestPattern) || (matches!(s.kind, SourceKind::Color(_)) && matches!(s.name.as_str(), "Camera placeholder" | "Lower third")))
                .map(|s| s.id)
                .collect();
            self.project.sources.retain(|s| !demo.contains(&s.id));
            for scene in &mut self.project.scenes {
                scene.layers.retain(|l| !demo.contains(&l.source));
            }
            // An empty Music strip with no input is confusing; it can be added again from Sound.
            self.project.audio.retain(|c| !(c.name == "Music" && c.media_source.is_none()));
        }
        let capture = match self.project.sources.iter_mut().find(|s| matches!(s.kind, SourceKind::ScreenCapture)) {
            Some(s) => {
                if replace {
                    s.name = "Game or screen".into();
                }
                s.id
            }
            None => {
                let id = self.project.id();
                self.project.sources.push(Source { id, name: "Game or screen".into(), kind: SourceKind::ScreenCapture });
                #[cfg(not(windows))]
                self.captures.start(id);
                id
            }
        };
        let source = |project: &mut Project, name: &str, kind: SourceKind| {
            let id = project.id();
            project.sources.push(Source { id, name: name.into(), kind });
            id
        };
        let text = |content: &str| {
            let mut settings = widget_settings(widgets::Kind::Text);
            settings.content = content.into();
            SourceKind::Widget(settings)
        };
        let alerts = self
            .project
            .sources
            .iter()
            .find(|s| matches!(&s.kind, SourceKind::Widget(w) if w.kind == widgets::Kind::AlertBox))
            .map(|s| s.id);
        let alerts = match (kind, alerts) {
            (Starter::Recording, _) => None,
            (_, Some(id)) => Some(id),
            (_, None) => Some(source(&mut self.project, "Alerts", SourceKind::Widget(widget_settings(widgets::Kind::AlertBox)))),
        };
        if let Some(id) = alerts {
            self.route_visual_alerts(id);
        }
        let layer = |source: Id, rect: [f32; 4]| Layer { crop: [0.; 4], group: 0, frame: Default::default(), look: Default::default(), source, rect, visible: true, locked: false, opacity: 1. };
        let mut scenes: Vec<Scene> = Vec::new();
        let mut add_scene = |project: &mut Project, name: &str, layers: Vec<Layer>| {
            let id = project.id();
            scenes.push(Scene { id, name: name.into(), layers });
            id
        };
        let alert_layer = alerts.map(|id| layer(id, [0.15, 0.1, 0.7, 0.35]));
        let main_content = match kind {
            Starter::Chatting => source(&mut self.project, "Camera", SourceKind::VideoCamera(Default::default())),
            _ => capture,
        };
        let first = match kind {
            Starter::Games => add_scene(&mut self.project, "Gameplay", std::iter::once(layer(main_content, [0., 0., 1., 1.])).chain(alert_layer.clone()).collect()),
            Starter::Chatting => add_scene(&mut self.project, "Just chatting", std::iter::once(layer(main_content, [0., 0., 1., 1.])).chain(alert_layer.clone()).collect()),
            Starter::Recording => add_scene(&mut self.project, "Recording", vec![layer(main_content, [0., 0., 1., 1.])]),
        };
        if kind != Starter::Recording {
            let background = source(&mut self.project, "Background", SourceKind::Color([22, 18, 36, 255]));
            for title in ["Starting soon", "Be right back"] {
                let words = source(&mut self.project, title, text(title));
                add_scene(&mut self.project, title, vec![layer(background, [0., 0., 1., 1.]), layer(words, [0.1, 0.3, 0.8, 0.4])]);
            }
        }
        let ids: Vec<Id> = scenes.iter().map(|s| s.id).collect();
        let old_main = self.project.canvas_scenes(main);
        self.project.scenes.extend(scenes);
        let lists = &mut self.project.preferences.workspace.scenes;
        if replace {
            lists.insert(main, ids.clone());
        } else {
            lists.entry(main).or_default().extend(ids.iter().copied());
        }
        // The vertical video follows the main one with its own copy of the first scene.
        if let Some(vertical) = self.project.canvases.get(1).map(|c| c.id) {
            let id = self.project.id();
            let layers = vec![layer(main_content, [0., 0., 1., 1.])];
            self.project.scenes.push(Scene { id, name: format!("{} (vertical)", self.project.scene(first).map(|s| s.name.clone()).unwrap_or_default()), layers });
            let old_vertical = self.project.canvas_scenes(vertical);
            if replace {
                self.project.preferences.workspace.scenes.insert(vertical, vec![id]);
                self.project.scenes.retain(|s| !old_vertical.contains(&s.id));
            } else {
                self.project.preferences.workspace.scenes.entry(vertical).or_default().push(id);
            }
            self.project.preferences.workspace.scene_links.insert(first, id);
            self.project.switch_scene(vertical, id);
        }
        if replace {
            self.project.scenes.retain(|s| !old_main.contains(&s.id));
            let remaining: Vec<Id> = self.project.scenes.iter().map(|s| s.id).collect();
            self.project.preferences.workspace.scene_links.retain(|a, b| remaining.contains(a) && remaining.contains(b));
        }
        self.project.switch_scene(main, first);
        self.canvas = 0;
        self.layer = None;
        self.selections.clear();
        self.changed();
        self.save();
    }
}
