// The Go live checklist, Check my setup, and one-click fixes for common errors.
#[derive(Clone, Copy, PartialEq)]
enum Level {
    Good,
    Warn,
    /// Going live would fail; Start stays disabled.
    Block,
}
#[derive(Clone, Copy)]
enum Fix {
    Accounts,
    Unmute(Id),
    Studio,
    Media,
    Video,
    SoftwareEncoder,
    Bitrate,
    EvenSize,
}
struct Check {
    level: Level,
    text: String,
    fix: Option<(&'static str, Fix)>,
}
impl Studio {
    fn checks(&mut self) -> Vec<Check> {
        let mut checks = Vec::new();
        let mut add = |level, text: String, fix| checks.push(Check { level, text, fix });
        // Where the stream goes.
        let config = &self.stream_config;
        let mut destinations = vec![
            (config.connection_name(0), &config.twitch),
            (config.connection_name(1), &config.youtube),
            (config.connection_name(2), &config.youtube_vertical),
        ];
        destinations.extend(config.services.iter().map(|s| (s.name.clone(), &s.destination)));
        let enabled: Vec<_> = destinations.into_iter().filter(|(_, d)| d.enabled).collect();
        if enabled.is_empty() {
            add(Level::Block, "Choose where to stream (Twitch, YouTube, Kick…)".into(), Some(("Choose", Fix::Accounts)));
        }
        for (name, dest) in enabled {
            if dest.key.trim().is_empty() || dest.server.trim().is_empty() {
                add(Level::Block, format!("{name}: sign in, or add its stream key"), Some(("Set up", Fix::Accounts)));
            } else if dest.canvas.is_some_and(|id| !self.project.canvases.iter().any(|c| c.id == id)) {
                add(Level::Block, format!("{name}: choose which video to send"), Some(("Set up", Fix::Accounts)));
            } else {
                add(Level::Good, format!("{name} is ready"), None);
            }
        }
        if !self.accounts.iter().any(|a| a.connected) && !self.kick_account.connected {
            add(Level::Warn, "You're not signed in, so chat and alerts won't appear".into(), Some(("Sign in", Fix::Accounts)));
        }
        // Something to see.
        let scene = self.project.canvases[0].scene;
        let visible = self.project.scene(scene).is_some_and(|s| s.layers.iter().any(|l| l.visible));
        if visible {
            add(Level::Good, "Your main video has something on screen".into(), None);
        } else {
            add(Level::Warn, "Your main video is empty, so viewers would see a black screen".into(), Some(("Add something", Fix::Studio)));
        }
        // Something to hear.
        let view = self.audio.view();
        let channels = self.project.mixer_channels(scene);
        let mic = channels.iter().find(|c| {
            c.media_source.is_none()
                && (c.name.to_lowercase().contains("mic")
                    || self.audio.selected.get(&c.id).is_some_and(|serial| view.devices.iter().any(|d| &d.serial == serial && d.kind == "Audio/Source")))
        });
        match mic {
            None => add(Level::Warn, "No microphone is set up".into(), Some(("Set up", Fix::Studio))),
            Some(c) if c.muted || c.gain_db <= -60. => add(Level::Warn, format!("{} is muted", c.name), Some(("Unmute", Fix::Unmute(c.id)))),
            Some(c) => {
                let level = view.stereo_peaks.get(&c.id).map_or(0., |p| p[0].max(p[1]));
                if level > 0.03 {
                    self.mic_heard = Some(Instant::now());
                }
                if self.mic_heard.is_some_and(|t| t.elapsed() < Duration::from_secs(20)) {
                    add(Level::Good, format!("{} is working", c.name), None);
                } else {
                    add(Level::Warn, format!("Say something to test {}", c.name), None);
                }
            }
        }
        for c in channels.iter().filter(|c| Some(c.id) != mic.map(|m| m.id) && c.media_source.is_none() && !self.project.is_alert_channel(c)) {
            if c.muted || c.gain_db <= -60. {
                add(Level::Warn, format!("{} is muted, so viewers won't hear it", c.name), Some(("Unmute", Fix::Unmute(c.id))));
            }
        }
        let missing = self
            .project
            .sources
            .iter()
            .filter(|s| match &s.kind {
                SourceKind::Image { path } => !std::path::Path::new(path).is_file(),
                SourceKind::VideoFile(v) => !std::path::Path::new(&v.path).is_file(),
                _ => false,
            })
            .count();
        if missing > 0 {
            add(Level::Warn, format!("{missing} picture or video file(s) can't be found"), Some(("Find them", Fix::Media)));
        }
        checks
    }
    fn check_list(&mut self, ui: &mut egui::Ui) -> bool {
        let checks = self.checks();
        for check in &checks {
            ui.horizontal(|ui| {
                let (icon, color) = match check.level {
                    Level::Good => ("✔", crate::appearance::current().good),
                    Level::Warn => ("⚠", crate::appearance::current().warning),
                    Level::Block => ("✖", Color32::from_rgb(255, 110, 110)),
                };
                ui.label(RichText::new(icon).strong().color(color));
                ui.label(&check.text);
                if let Some((label, fix)) = check.fix
                    && ui.small_button(label).clicked()
                {
                    self.apply_fix(fix);
                }
            });
        }
        ui.ctx().request_repaint_after(Duration::from_millis(100));
        !checks.iter().any(|c| c.level == Level::Block)
    }
    fn go_live_window(&mut self, ctx: &egui::Context) {
        if !self.go_live_open {
            return;
        }
        if self.live_stream.is_some() {
            self.go_live_open = false;
            return;
        }
        let mut open = true;
        let mut start = false;
        pop_modal(ctx, egui::Id::new("go-live"))
            .show(ctx, |ui| {
                ui.set_width(460.);
                ui.heading("Ready to go live?");
                ui.add_space(6.);
                let ready = self.check_list(ui);
                if !self.stream_error.is_empty() {
                    ui.separator();
                    self.error_line(ui, self.stream_error.clone());
                }
                let announce = &self.stream_config.announce;
                let places = [(announce.discord_ready(), "Discord"), (announce.bluesky_ready(), "Bluesky"), (announce.threads_ready(), "Threads"), (announce.x_ready(), "X")];
                if places.iter().any(|(ready, _)| *ready) {
                    ui.separator();
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Announce on");
                        for (i, (ready, name)) in places.into_iter().enumerate() {
                            let mut on = !self.announce_skip[i];
                            if ready && ui.checkbox(&mut on, name).on_hover_text("Untick to skip it for this stream only").changed() {
                                self.announce_skip[i] = !on;
                            }
                        }
                    });
                }
                // Whether this stream's YouTube broadcast turns public as it starts (kept as a setting).
                let youtube = self.stream_config.youtube.enabled && self.accounts[1].connected;
                if youtube {
                    ui.separator();
                    let mut public = self.stream_config.youtube_public;
                    if ui.checkbox(&mut public, "Make my YouTube stream public when it starts").on_hover_text("Rosorah switches the YouTube broadcast to public as your stream reaches YouTube, so its recording stays public too. Remembered for next time.").changed() {
                        self.stream_config.youtube_public = public;
                        self.save_stream_settings();
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(ready, self.primary_button("Start streaming"))
                        .on_disabled_hover_text("Fix the items marked ✖ first")
                        .clicked()
                    {
                        start = true;
                    }
                    if ui.button("Not yet").clicked() {
                        self.go_live_open = false;
                    }
                });
                note(ui, "⚠ items won't stop the stream, but are worth a look.");
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            open = false;
        }
        if start {
            self.start_streaming();
            if self.stream_error.is_empty() {
                self.go_live_open = false;
            }
        }
        if !open {
            self.go_live_open = false;
        }
    }
    fn readiness_window(&mut self, ctx: &egui::Context) {
        if !self.readiness_open {
            return;
        }
        let mut open = true;
        pop_window(ctx, "Check my setup")
            .frame(solid_window(ctx))
            .open(&mut open)
            .default_width(460.)
            .show(ctx, |ui| {
                self.check_list(ui);
                ui.separator();
                note(ui, "This checks this computer only. It doesn't start a stream, and it can't see what viewers receive.");
            });
        self.readiness_open = open;
    }
    fn apply_fix(&mut self, fix: Fix) {
        match fix {
            Fix::Accounts => {
                self.page = Page::Settings;
                self.settings = Settings::Accounts;
                self.go_live_open = false;
                self.readiness_open = false;
            }
            Fix::Studio => {
                self.page = Page::Studio;
                self.go_live_open = false;
                self.readiness_open = false;
            }
            Fix::Video => {
                self.page = Page::Settings;
                self.settings = Settings::Video;
            }
            Fix::Media => self.tools.open = true,
            Fix::Unmute(id) => {
                let scene = self.project.canvases[self.canvas].scene;
                if let Some(c) = self.project.mixer_channels(scene).into_iter().find(|c| c.id == id) {
                    self.project.preferences.scene_audio.entry(scene).or_default().insert(
                        id,
                        auroracast_core::SceneVolume { gain_db: if c.gain_db <= -60. { 0. } else { c.gain_db }, muted: false },
                    );
                    self.changed();
                }
            }
            Fix::SoftwareEncoder => {
                self.project.preferences.recording_encoder = auroracast_core::RecordingEncoder::Software;
                self.stream_error.clear();
                self.recording_error.clear();
                self.changed();
                self.tell("Switched to the software encoder. Try again.".into());
            }
            Fix::Bitrate => {
                self.stream_config.bitrate = 6000;
                for slot in 0..3 + self.stream_config.services.len() {
                    if let Some(dest) = self.stream_config.destination_mut(slot) {
                        dest.bitrate = None;
                    }
                }
                self.stream_error.clear();
                self.save_stream_settings();
                self.tell("Quality set to the recommended 6,000 kbps. Try again.".into());
            }
            Fix::EvenSize => {
                for canvas in &mut self.project.canvases {
                    canvas.width += canvas.width % 2;
                    canvas.height += canvas.height % 2;
                }
                self.recording_error.clear();
                self.changed();
            }
        }
    }
    /// The one-click fix for a known error message, if there is one.
    fn error_fix(error: &str) -> Option<(&'static str, Fix)> {
        let e = error.to_lowercase();
        if e.contains("even canvas") {
            Some(("Fix the video size", Fix::EvenSize))
        } else if e.contains("encoder") {
            Some(("Use the software encoder", Fix::SoftwareEncoder))
        } else if e.contains("bitrate") || e.contains("kbps") {
            Some(("Use recommended quality", Fix::Bitrate))
        } else if e.contains("folder") {
            Some(("Choose another folder", Fix::Video))
        } else if e.contains("stream key") || e.contains("server") || e.contains("where to stream") || e.contains("canvas") || e.contains("video again") || e.contains("sign in") {
            Some(("Open Accounts", Fix::Accounts))
        } else {
            None
        }
    }
    /// An error in red, with its Fix it button when one is known.
    fn error_line(&mut self, ui: &mut egui::Ui, error: String) {
        ui.colored_label(crate::appearance::current().error, &error);
        if let Some((label, fix)) = Self::error_fix(&error)
            && ui.add(self.primary_button(label)).clicked()
        {
            self.apply_fix(fix);
        }
    }
}
impl Studio {
    /// Help > Report a problem: technical details to pass to whoever is helping.
    fn report_window(&mut self, ctx: &egui::Context) {
        if !self.report_open {
            return;
        }
        let mut open = true;
        pop_window(ctx, "Report a problem")
            .frame(solid_window(ctx))
            .open(&mut open)
            .default_width(460.)
            .show(ctx, |ui| {
                ui.label("Copy a report, then paste it to whoever is helping you.");
                note(ui, "It has technical details only: no chat messages, passwords or stream keys.");
                if ui.add(self.primary_button("Copy problem report")).clicked() {
                    let audio = self.audio.view();
                    let report = format!(
                        "Rosorah Studio {}\n{}\nStream: {}\nRecording: {}\nGame audio: {}\nAudio error: {}\nAudio connection: {}\nCaptured process: {:?}\nChat: {}",
                        env!("CARGO_PKG_VERSION"),
                        crate::capture_log::details(),
                        self.stream_error,
                        self.recording_error,
                        audio.status.get(&crate::game_audio::CHANNEL).cloned().unwrap_or_default(),
                        audio.error,
                        audio.capture_error,
                        self.captures.game(),
                        self.browser_chat.status,
                    );
                    ui.ctx().copy_text(report);
                    self.tell("Problem report copied".into());
                }
                if !self.stream_config.tiktok_username.trim().is_empty()
                    && ui
                        .button("Copy TikTok page structure")
                        .on_hover_text("Copies tags and hook names only, no messages or names. Use it if TikTok chat stops appearing.")
                        .clicked()
                {
                    self.browser_chat.diagnostic = None;
                    self.browser_chat.send(serde_json::json!({"type":"diagnose","platform":"TikTok"}));
                    self.tiktok_diagnostic_wanted = true;
                }
            });
        if self.tiktok_diagnostic_wanted
            && let Some(report) = self.browser_chat.diagnostic.take()
        {
            ctx.copy_text(report);
            self.tiktok_diagnostic_wanted = false;
            self.tell("TikTok page structure copied".into());
        }
        self.report_open = open;
    }
}
#[cfg(test)]
mod go_live_tests {
    use super::*;
    #[test]
    fn known_errors_offer_a_fix() {
        let label = |e: &str| Studio::error_fix(e).map(|(label, _)| label);
        assert_eq!(label("Choose where to stream: switch on a platform in Settings > Accounts"), Some("Open Accounts"));
        assert_eq!(label("Enter your Twitch stream key in Streaming settings"), Some("Open Accounts"));
        assert_eq!(label("Encoders failed to start"), Some("Use the software encoder"));
        assert_eq!(label("Use 6,000 kbps or less for Twitch"), Some("Use recommended quality"));
        assert_eq!(label("Cannot create recording folder"), Some("Choose another folder"));
        assert_eq!(label("Output requires even canvas width and height (Settings > Video & recording)"), Some("Fix the video size"));
        assert_eq!(label("Something unexpected"), None);
    }
}
