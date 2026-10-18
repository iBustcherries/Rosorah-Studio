// Remote control: carrying out controllers' commands, sending them events, and its Settings page.
// Included into studio.rs.

/// What controllers are told about when it changes.
#[derive(Clone, Default, PartialEq)]
struct RemoteState {
    scene: Option<Id>,
    preview: Option<Id>,
    studio: bool,
    live: bool,
    recording: bool,
    replay: bool,
    camera: bool,
    muted: std::collections::BTreeMap<Id, bool>,
}

impl Studio {
    /// A scene by name or id, from a controller.
    fn remote_scene(&self, value: &serde_json::Value) -> Option<Id> {
        let main = self.project.canvases.first()?;
        let scenes = self.project.canvas_scenes(main.id);
        let found = match value {
            serde_json::Value::Number(n) => n.as_u64().filter(|id| scenes.contains(id)),
            serde_json::Value::String(s) => s.parse::<u64>().ok().filter(|id| scenes.contains(id)).or_else(|| scenes.iter().copied().find(|id| self.project.scene(*id).is_some_and(|sc| sc.name.eq_ignore_ascii_case(s)))),
            _ => None,
        };
        found.or_else(|| value.is_null().then_some(main.scene))
    }
    /// A sound strip by name or id.
    fn remote_sound(&self, value: &serde_json::Value) -> Option<Id> {
        let name = |c: &auroracast_core::AudioChannel| self.project.preferences.workspace.sound_names.get(&c.id).cloned().unwrap_or_else(|| c.name.clone());
        self.project.audio.iter().find(|c| match value {
            serde_json::Value::Number(n) => n.as_u64() == Some(c.id),
            serde_json::Value::String(s) => s.parse::<u64>().ok() == Some(c.id) || name(c).eq_ignore_ascii_case(s),
            _ => false,
        }).map(|c| c.id)
    }
    fn remote_status(&self) -> RemoteState {
        let scene = self.project.canvases.first().map(|c| c.scene);
        let studio = self.project.preferences.workspace.studio_mode;
        let muted = scene
            .map(|s| self.project.mixer_channels(s).iter().filter(|c| c.media_source.is_none()).map(|c| (c.id, c.muted)).collect())
            .unwrap_or_default();
        RemoteState {
            scene,
            preview: if studio { self.studio_preview.or(scene) } else { None },
            studio,
            live: self.live_stream.is_some(),
            recording: self.live_recording.is_some(),
            replay: self.replay_on,
            camera: self.virtual_camera.is_some(),
            muted,
        }
    }
    /// Carries out one controller command; the answer goes back to that controller.
    fn remote_command(&mut self, command: &str, d: &serde_json::Value) -> Result<serde_json::Value, String> {
        use serde_json::json;
        let on = d["on"].as_bool();
        let scene_json = |s: &Studio, id: Option<Id>| id.and_then(|id| s.project.scene(id)).map_or(serde_json::Value::Null, |sc| json!({"id": sc.id, "name": sc.name}));
        Ok(match command {
            "status" => {
                let s = self.remote_status();
                let platforms: Vec<serde_json::Value> = self.live_stream.as_ref().map(|st| st.destinations().iter().map(|v| json!({"name": v.name, "phase": v.phase.label()})).collect()).unwrap_or_default();
                json!({"scene": scene_json(self, s.scene), "preview": scene_json(self, s.preview), "studio_mode": s.studio, "live": s.live, "recording": s.recording, "replay": s.replay, "virtual_camera": s.camera, "platforms": platforms})
            }
            "scenes" => {
                let main = self.project.canvases.first().cloned().ok_or("No video")?;
                let s = self.remote_status();
                let scenes: Vec<serde_json::Value> = self.project.canvas_scenes(main.id).iter().filter_map(|id| self.project.scene(*id)).map(|sc| json!({"id": sc.id, "name": sc.name, "live": Some(sc.id) == s.scene, "preview": Some(sc.id) == s.preview})).collect();
                json!({"scenes": scenes, "live": scene_json(self, s.scene), "preview": scene_json(self, s.preview)})
            }
            "switch_scene" | "preview_scene" => {
                let scene = self.remote_scene(&d["scene"]).ok_or("No scene by that name")?;
                let canvas = self.project.canvases.first().map(|c| c.id).ok_or("No video")?;
                if command == "preview_scene" {
                    if !self.project.preferences.workspace.studio_mode {
                        return Err("Studio Mode isn't on".into());
                    }
                    self.studio_preview = Some(scene);
                    self.revision = true;
                } else {
                    // "live": true goes straight to air even in Studio Mode (as OBS's program scene does).
                    self.studio_taking = d["live"].as_bool().unwrap_or(false);
                    self.request_scene(canvas, scene);
                    self.studio_taking = false;
                }
                json!({})
            }
            "transition" => {
                self.studio_transition();
                json!({})
            }
            "studio_mode" => {
                let want = on.unwrap_or(!self.project.preferences.workspace.studio_mode);
                if want != self.project.preferences.workspace.studio_mode {
                    self.run_hotkey(auroracast_core::HotkeyAction::StudioMode, true);
                }
                json!({"studio_mode": want})
            }
            "go_live" => {
                if self.live_stream.is_some() {
                    return Err("Already live".into());
                }
                // A list of platforms switches exactly those on for this stream.
                if let Some(names) = d["platforms"].as_array() {
                    let wanted: Vec<String> = names.iter().filter_map(|n| n.as_str().map(str::to_lowercase)).collect();
                    let slots = 3 + self.stream_config.services.len();
                    for slot in 0..slots {
                        let name = self.stream_config.connection_name(slot).to_lowercase();
                        if let Some(dest) = self.stream_config.destination_mut(slot) {
                            dest.enabled = wanted.iter().any(|w| name.contains(w.as_str()));
                        }
                    }
                }
                self.start_streaming();
                if !self.stream_error.is_empty() {
                    return Err(self.stream_error.clone());
                }
                json!({"live": true})
            }
            "stop_live" => {
                if let Some(stream) = &self.live_stream {
                    stream.stop();
                }
                json!({"live": false})
            }
            "record" => {
                let want = on.unwrap_or(self.live_recording.is_none());
                match (&self.live_recording, want) {
                    (Some(r), false) => r.stop(),
                    (None, true) => self.start_recording(),
                    _ => {}
                }
                json!({"recording": want})
            }
            "replay" => {
                let want = on.unwrap_or(!self.replay_on);
                if want != self.replay_on {
                    self.toggle_replay_buffer();
                }
                json!({"replay": want})
            }
            "save_replay" => {
                self.save_replay();
                json!({})
            }
            "virtual_camera" => {
                let want = on.unwrap_or(self.virtual_camera.is_none());
                if want != self.virtual_camera.is_some() {
                    self.toggle_virtual_camera();
                }
                json!({"virtual_camera": self.virtual_camera.is_some()})
            }
            "sounds" | "sound" => {
                let scene = self.project.canvases.first().map(|c| c.scene).ok_or("No video")?;
                let channels = self.project.mixer_channels(scene);
                let describe = |c: &auroracast_core::AudioChannel| json!({"id": c.id, "name": self.project.preferences.workspace.sound_names.get(&c.id).cloned().unwrap_or_else(|| c.name.clone()), "muted": c.muted, "volume_db": c.gain_db.max(-100.)});
                if command == "sound" {
                    let id = self.remote_sound(&d["sound"]).ok_or("No sound by that name")?;
                    channels.iter().find(|c| c.id == id).map(describe).ok_or("No sound by that name")?
                } else {
                    json!({"sounds": channels.iter().filter(|c| c.media_source.is_none()).map(describe).collect::<Vec<_>>()})
                }
            }
            "mute" => {
                let id = self.remote_sound(&d["sound"]).ok_or("No sound by that name")?;
                self.set_muted(id, on);
                let scene = self.project.canvases.first().map(|c| c.scene).unwrap_or_default();
                json!({"muted": self.project.mixer_channels(scene).iter().find(|c| c.id == id).is_some_and(|c| c.muted)})
            }
            "volume" => {
                let id = self.remote_sound(&d["sound"]).ok_or("No sound by that name")?;
                let db = d["db"].as_f64().ok_or("Give the volume as db")?.clamp(-100., 6.) as f32;
                let scene = self.project.canvases.first().map(|c| c.scene).ok_or("No video")?;
                let muted = self.project.mixer_channels(scene).iter().find(|c| c.id == id).is_some_and(|c| c.muted);
                self.project.preferences.scene_audio.entry(scene).or_default().insert(id, auroracast_core::SceneVolume { gain_db: db, muted });
                self.dirty = true;
                json!({"volume_db": db})
            }
            "layers" => {
                let scene = self.remote_scene(&d["scene"]).ok_or("No scene by that name")?;
                let sc = self.project.scene(scene).ok_or("No scene by that name")?;
                let layers: Vec<serde_json::Value> = sc.layers.iter().enumerate().rev().map(|(i, l)| json!({"index": i, "name": self.project.source(l.source).map(|s| s.name.clone()).unwrap_or_default(), "visible": l.visible, "source": l.source})).collect();
                json!({"layers": layers})
            }
            "layer" => {
                let scene = self.remote_scene(&d["scene"]).ok_or("No scene by that name")?;
                let si = self.project.scenes.iter().position(|s| s.id == scene).ok_or("No scene by that name")?;
                let index = match &d["layer"] {
                    serde_json::Value::Number(n) => n.as_u64().map(|n| n as usize),
                    serde_json::Value::String(name) => self.project.scenes[si].layers.iter().position(|l| self.project.source(l.source).is_some_and(|s| s.name.eq_ignore_ascii_case(name))),
                    _ => None,
                }
                .filter(|i| *i < self.project.scenes[si].layers.len())
                .ok_or("No layer by that name")?;
                let layer = &mut self.project.scenes[si].layers[index];
                layer.visible = d["visible"].as_bool().unwrap_or(!layer.visible);
                let visible = layer.visible;
                self.changed();
                json!({"visible": visible})
            }
            "automations" => json!({"automations": self.project.rules.iter().map(|r| json!({"name": r.name, "enabled": r.enabled})).collect::<Vec<_>>()}),
            "run_automation" => {
                let name = d["name"].as_str().ok_or("Give the automation's name")?;
                let actions = self.project.rules.iter().find(|r| r.name.eq_ignore_ascii_case(name)).map(|r| r.actions.clone()).ok_or("No automation by that name")?;
                self.run_actions(actions);
                json!({})
            }
            "alert" => {
                let text = d["text"].as_str().ok_or("Give the alert text")?;
                self.shared_alert(text, d["seconds"].as_u64().unwrap_or(5).clamp(1, 60) as u32, None);
                json!({})
            }
            "chat" => {
                let text = d["text"].as_str().ok_or("Give the message text")?.trim();
                if text.is_empty() {
                    return Err("The message is empty".into());
                }
                let targets: Vec<String> = match d["platforms"].as_array() {
                    Some(list) => list.iter().filter_map(|p| p.as_str().map(str::to_owned)).collect(),
                    None => self.accounts.iter().chain(std::iter::once(&self.kick_account)).filter(|a| a.connected).map(|a| a.platform.name().to_owned()).collect(),
                };
                if targets.is_empty() {
                    return Err("No chat is connected".into());
                }
                self.browser_chat.draft = text.chars().take(500).collect();
                self.browser_chat.post_to(&targets);
                json!({"sent_to": targets})
            }
            _ => return Err(format!("Unknown command “{command}”")),
        })
    }
    /// Runs the server when it's switched on, carries out commands, and tells controllers what changed.
    fn remote_tick(&mut self) {
        let settings = &self.stream_config.remote;
        let target = (settings.enabled && self.screenshot.is_none()).then(|| RemoteRunning { port: settings.port, network: settings.network, password: self.stream_config.remote_password.clone() });
        let current = self.remote.is_some().then(|| self.remote_settings.clone());
        // (Re)start when the settings change; a failed start (port busy) is retried every 10 s.
        let retry = self.remote.is_none() && target.is_some() && self.remote_attempt.elapsed() > Duration::from_secs(10);
        if target != current && (target != Some(self.remote_settings.clone()) || retry || target.is_none()) {
            self.remote = None;
            self.remote_error.clear();
            self.remote_attempt = Instant::now();
            if let Some(t) = &target {
                match crate::remote::Server::start(t.port, t.network, t.password.clone()) {
                    Ok(server) => self.remote = Some(server),
                    Err(e) => self.remote_error = format!("{e:#}"),
                }
            }
            self.remote_settings = target.unwrap_or_default();
        }
        let Some(server) = &self.remote else { return };
        let mut calls = Vec::new();
        while let Ok(call) = server.calls.try_recv() {
            calls.push(call);
        }
        for call in calls {
            let answer = self.remote_command(&call.command, &call.data);
            let _ = call.reply.send(answer);
        }
        // Events: what changed since last time, then new activity and chat.
        let now = self.remote_status();
        let before = std::mem::replace(&mut self.remote_seen, now.clone());
        let Some(server) = &self.remote else { return };
        let scene = |id: Option<Id>| id.and_then(|id| self.project.scene(id)).map_or(serde_json::Value::Null, |s| serde_json::json!(s.name));
        if now.scene != before.scene {
            server.broadcast(serde_json::json!({"event": "scene", "id": now.scene, "name": scene(now.scene)}));
        }
        if now.preview != before.preview && now.preview.is_some() {
            server.broadcast(serde_json::json!({"event": "preview", "id": now.preview, "name": scene(now.preview)}));
        }
        for (key, a, b) in [("studio_mode", before.studio, now.studio), ("live", before.live, now.live), ("recording", before.recording, now.recording), ("replay", before.replay, now.replay), ("virtual_camera", before.camera, now.camera)] {
            if a != b {
                server.broadcast(serde_json::json!({"event": key, "on": b}));
            }
        }
        for (id, muted) in &now.muted {
            if before.muted.get(id).is_some_and(|m| m != muted) {
                let name = self.project.preferences.workspace.sound_names.get(id).cloned().or_else(|| self.project.audio.iter().find(|c| c.id == *id).map(|c| c.name.clone()));
                server.broadcast(serde_json::json!({"event": "mute", "id": id, "name": name, "muted": muted}));
            }
        }
        let newest = self.chat.messages().map(|m| m.received_ms).max().unwrap_or(0);
        if newest > self.remote_chat_seen {
            for m in self.chat.messages().filter(|m| m.received_ms > self.remote_chat_seen) {
                server.broadcast(serde_json::json!({"event": "chat", "platform": m.platform.name(), "author": m.author, "text": m.text}));
            }
            self.remote_chat_seen = newest;
        }
    }
    /// Settings > Remote control.
    fn remote_settings_page(&mut self, ui: &mut egui::Ui) {
        ui.heading("Remote control");
        note(ui, "Lets a Stream Deck, Touch Portal, a phone app or your own tools control Rosorah. Tools made for OBS work too: Rosorah answers in OBS's language when they ask for it.");
        ui.add_space(12.);
        let before = (self.stream_config.remote.clone(), self.stream_config.remote_password.clone());
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            let r = &mut self.stream_config.remote;
            ui.checkbox(&mut r.enabled, "Allow remote control");
            ui.horizontal(|ui| {
                ui.label("Port");
                ui.add(egui::DragValue::new(&mut r.port).range(1024..=65535));
                note(ui, "4455 is OBS's usual port, so OBS tools find Rosorah without changes (OBS's own server can't run at the same time).");
            });
            ui.checkbox(&mut r.network, "Allow other devices on my network (a phone or a second computer)");
            if !r.network {
                note(ui, "Only programs on this computer can connect.");
            }
            ui.horizontal(|ui| {
                ui.label("Password");
                ui.add(egui::TextEdit::singleline(&mut self.stream_config.remote_password).password(true).desired_width(180.));
                if ui.small_button("New").on_hover_text("Make a new random password").clicked() {
                    self.stream_config.remote_password = crate::remote::new_password();
                }
                if ui.small_button("Copy").clicked() {
                    ui.ctx().copy_text(self.stream_config.remote_password.clone());
                }
            });
            if self.stream_config.remote_password.is_empty() {
                ui.colored_label(ui.visuals().warn_fg_color, "Without a password anything that can reach the port can control Rosorah.");
            }
            match (&self.remote, self.remote_error.is_empty()) {
                (Some(server), _) => note(ui, format!("Running on port {} · {} connected", server.port, server.connected.load(std::sync::atomic::Ordering::Relaxed))),
                (None, false) => {
                    ui.colored_label(ui.visuals().warn_fg_color, &self.remote_error);
                }
                _ => {}
            }
        });
        if self.stream_config.remote.enabled && self.stream_config.remote_password.is_empty() && before.0.enabled != self.stream_config.remote.enabled {
            // Switching it on for the first time makes a password straight away.
            self.stream_config.remote_password = crate::remote::new_password();
        }
        ui.add_space(12.);
        ui.collapsing("For developers: Rosorah's own protocol", |ui| {
            note(ui, "WebSocket, JSON text messages. Connect (optionally asking for subprotocol \"rosorah.v1\"), send {\"do\": \"hello\", \"password\": \"…\"}, then commands like {\"id\": 1, \"do\": \"switch_scene\", \"scene\": \"Gaming\"}; answers echo the id with \"ok\". Events arrive as {\"event\": …}.");
            note(ui, "Commands: status, scenes, switch_scene, preview_scene, transition, studio_mode, go_live (with \"platforms\": [\"kick\"] for some), stop_live, record, replay, save_replay, virtual_camera, sounds, sound, mute, volume, layers, layer, automations, run_automation, alert, chat.");
            note(ui, "Events: scene, preview, studio_mode, live, recording, replay, virtual_camera, mute, chat.");
        });
        if before != (self.stream_config.remote.clone(), self.stream_config.remote_password.clone()) {
            self.save_stream_settings();
        }
    }
}
/// The settings the running server was started with.
#[derive(Clone, Default, PartialEq)]
struct RemoteRunning {
    port: u16,
    network: bool,
    password: String,
}
