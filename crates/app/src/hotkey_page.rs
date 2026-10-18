// Hotkeys: running actions, in-app keys, system-wide registration and the Settings page.
// Included into studio.rs.
impl Studio {
    /// What a hotkey does, in words.
    fn hotkey_label(&self, action: auroracast_core::HotkeyAction) -> String {
        use auroracast_core::HotkeyAction as A;
        let sound = |id: Id| self.project.preferences.workspace.sound_names.get(&id).cloned().or_else(|| self.project.audio.iter().find(|c| c.id == id).map(|c| c.name.clone())).unwrap_or_else(|| "Sound".into());
        match action {
            A::Scene(id) => format!("Switch to “{}”", self.project.scene(id).map_or("?", |s| s.name.as_str())),
            A::Transition => "Studio Mode: Transition".into(),
            A::StudioMode => "Studio Mode on/off".into(),
            A::Streaming => "Start / stop streaming".into(),
            A::Recording => "Start / stop recording".into(),
            A::SaveReplay => "Save replay".into(),
            A::ReplayBuffer => "Replay buffer on/off".into(),
            A::VirtualCamera => "Virtual camera on/off".into(),
            A::Mute(id) => format!("Mute / unmute {}", sound(id)),
            A::PushToTalk(id) => format!("Push to talk: {}", sound(id)),
            A::PushToMute(id) => format!("Push to mute: {}", sound(id)),
        }
    }
    /// Mutes or unmutes a strip in the scene that's live (volumes are kept per scene).
    fn set_muted(&mut self, channel: Id, muted: Option<bool>) {
        let Some(scene) = self.project.canvases.first().map(|c| c.scene) else { return };
        let current = self.project.mixer_channels(scene).iter().find(|c| c.id == channel).map(|c| (c.gain_db, c.muted));
        let Some((gain_db, was)) = current else { return };
        let muted = muted.unwrap_or(!was);
        self.project.preferences.scene_audio.entry(scene).or_default().insert(channel, auroracast_core::SceneVolume { gain_db, muted });
        self.dirty = true;
    }
    fn run_hotkey(&mut self, action: auroracast_core::HotkeyAction, down: bool) {
        use auroracast_core::HotkeyAction as A;
        match (action, down) {
            (A::PushToTalk(id), down) => self.set_muted(id, Some(!down)),
            (A::PushToMute(id), down) => self.set_muted(id, Some(down)),
            (_, false) => {}
            (A::Scene(scene), true) => {
                if let Some(canvas) = self.project.canvases.first().map(|c| c.id) {
                    self.request_scene(canvas, scene);
                }
            }
            (A::Transition, true) => self.studio_transition(),
            (A::StudioMode, true) => {
                let on = !self.project.preferences.workspace.studio_mode;
                self.project.preferences.workspace.studio_mode = on;
                self.studio_preview = on.then(|| self.project.canvases[0].scene);
                self.revision = true;
                self.dirty = true;
            }
            (A::Streaming, true) => match &self.live_stream {
                Some(stream) => stream.stop(),
                None => self.start_streaming(),
            },
            (A::Recording, true) => match &self.live_recording {
                Some(recording) => recording.stop(),
                None => self.start_recording(),
            },
            (A::SaveReplay, true) => self.save_replay(),
            (A::ReplayBuffer, true) => self.toggle_replay_buffer(),
            (A::VirtualCamera, true) => self.toggle_virtual_camera(),
            (A::Mute(id), true) => self.set_muted(id, None),
        }
    }
    /// Runs hotkeys pressed in Rosorah or anywhere (when registered with the system), and keeps
    /// the system registration in step with the list, a couple of seconds after it last changed.
    fn hotkeys_tick(&mut self, ctx: &egui::Context) {
        let wanted = self.project.preferences.hotkeys.clone();
        if wanted != self.hotkeys_bound {
            let settled = *self.hotkeys_changed.get_or_insert_with(Instant::now);
            if settled.elapsed() > Duration::from_secs(2) && self.hotkey_capture.is_none() {
                self.hotkeys_changed = None;
                self.global_hotkeys = None;
                let combos: Vec<_> = wanted.iter().filter_map(|h| crate::hotkeys::Combo::parse(&h.keys)).collect();
                let descriptions: Vec<_> = wanted.iter().filter(|h| crate::hotkeys::Combo::parse(&h.keys).is_some()).map(|h| self.hotkey_label(h.action)).collect();
                self.global_hotkeys = (!combos.is_empty() && self.screenshot.is_none()).then(|| crate::hotkeys::Global::start(combos, descriptions));
                self.hotkeys_bound = wanted.clone();
            }
        }
        let bound: Vec<(auroracast_core::HotkeyAction, crate::hotkeys::Combo)> = self.hotkeys_bound.iter().filter_map(|h| Some((h.action, crate::hotkeys::Combo::parse(&h.keys)?))).collect();
        let mut happened = Vec::new();
        if let Some(global) = &self.global_hotkeys {
            while let Ok(event) = global.events.try_recv() {
                match event {
                    crate::hotkeys::Event::Pressed(i) => happened.extend(bound.get(i).map(|b| (b.0, true))),
                    crate::hotkeys::Event::Released(i) => happened.extend(bound.get(i).map(|b| (b.0, false))),
                }
            }
        }
        // In-app keys, for combinations the system didn't take (and never while typing).
        if self.hotkey_capture.is_none() && ctx.memory(|m| m.focused().is_none()) {
            let registered = self.global_hotkeys.as_ref().map(|g| g.registered.clone());
            ctx.input(|input| {
                for (i, (action, combo)) in bound.iter().enumerate() {
                    if registered.as_ref().is_some_and(|r| r.get(i).is_some_and(|f| f.load(std::sync::atomic::Ordering::Relaxed))) {
                        continue;
                    }
                    if combo.pressed(input) {
                        happened.push((*action, true));
                    } else if combo.released(input) {
                        happened.push((*action, false));
                    }
                }
            });
        }
        for (action, down) in happened {
            self.run_hotkey(action, down);
        }
    }
    /// Settings > Hotkeys.
    fn hotkey_settings(&mut self, ui: &mut egui::Ui) {
        use auroracast_core::HotkeyAction as A;
        ui.heading("Hotkeys");
        note(ui, "Key combinations for common actions. They work in other apps too when your system allows it (Windows; KDE and other desktops with global shortcuts confirm them once).");
        if let Some(global) = &self.global_hotkeys {
            let status = global.status.lock().unwrap().clone();
            if !status.is_empty() {
                ui.colored_label(ui.visuals().warn_fg_color, status);
            }
        }
        ui.add_space(10.);
        // Capturing a combination: the next key pressed (with its modifiers) is taken.
        if let Some(action) = self.hotkey_capture {
            let captured = ui.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Key { key, pressed: true, modifiers, .. } if !matches!(key, egui::Key::Escape) => Some(crate::hotkeys::Combo { ctrl: modifiers.ctrl || modifiers.command, alt: modifiers.alt, shift: modifiers.shift, key: *key }),
                    _ => None,
                })
            });
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.hotkey_capture = None;
            } else if let Some(combo) = captured {
                let keys = combo.text();
                let hotkeys = &mut self.project.preferences.hotkeys;
                // One action per combination: taking it from another action moves it here.
                hotkeys.retain(|h| h.keys != keys && h.action != action);
                hotkeys.push(auroracast_core::Hotkey { action, keys });
                self.hotkey_capture = None;
                self.dirty = true;
            }
        }
        let main_scenes: Vec<Id> = self.project.canvases.first().map(|c| self.project.canvas_scenes(c.id)).unwrap_or_default();
        let sounds: Vec<Id> = self.project.audio.iter().filter(|c| c.media_source.is_none()).map(|c| c.id).collect();
        let groups: Vec<(&str, Vec<A>)> = vec![
            ("Going live", vec![A::Streaming, A::Recording, A::ReplayBuffer, A::SaveReplay, A::VirtualCamera]),
            ("Scenes", std::iter::once(A::StudioMode).chain(std::iter::once(A::Transition)).chain(main_scenes.into_iter().map(A::Scene)).collect()),
            ("Sound", sounds.iter().flat_map(|id| [A::Mute(*id), A::PushToTalk(*id), A::PushToMute(*id)]).collect()),
        ];
        for (heading, actions) in groups {
            card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.strong(heading);
                egui::Grid::new(("hotkeys", heading)).num_columns(3).spacing([12., 6.]).show(ui, |ui| {
                    for action in actions {
                        ui.label(self.hotkey_label(action));
                        let current = self.project.preferences.hotkeys.iter().find(|h| h.action == action).map(|h| h.keys.clone());
                        let listening = self.hotkey_capture == Some(action);
                        let text = if listening { "Press keys… (Esc to cancel)".to_owned() } else { current.clone().unwrap_or_else(|| "—".into()) };
                        if ui.add(egui::Button::new(RichText::new(text).monospace()).selected(listening)).on_hover_text("Click, then press the combination").clicked() {
                            self.hotkey_capture = Some(action);
                        }
                        if current.is_some() && ui.small_button("✕").on_hover_text("Remove").clicked() {
                            self.project.preferences.hotkeys.retain(|h| h.action != action);
                            self.dirty = true;
                        }
                        ui.end_row();
                    }
                });
            });
            ui.add_space(10.);
        }
        note(ui, "Automations have their own shortcuts (Ctrl+Alt+F1–F12) in Alerts & automation.");
    }
}
