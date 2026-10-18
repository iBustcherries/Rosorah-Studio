// Settings > Connections: one card per platform with its account sign-in and live destinations.
impl Studio {
    fn connections_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Connections");
        note(ui, "Sign in once for chat, stream details and automatic stream keys. Switch on the destinations to include when you go live.");
        ui.add_space(12.);
        let live = self.live_stream.is_some();
        let mut save = false;
        for platform in [Platform::Twitch, Platform::YouTube, Platform::Kick] {
            card().show(ui, |ui| {
                ui.push_id(platform.name(), |ui| {
                    ui.set_width(ui.available_width());
                    self.account_row(ui, platform);
                    let slots: Vec<usize> = match platform {
                        Platform::Twitch => vec![0],
                        Platform::YouTube => vec![1, 2],
                        _ => self
                            .stream_config
                            .services
                            .iter()
                            .enumerate()
                            .filter(|(_, s)| s.provider == crate::streaming::Provider::Kick)
                            .map(|(i, _)| i + 3)
                            .collect(),
                    };
                    ui.add_space(4.);
                    ui.add_enabled_ui(!live, |ui| {
                        for slot in &slots {
                            save |= self.destination_row(ui, *slot, platform == Platform::YouTube && *slot == 2);
                        }
                        if platform == Platform::Kick && slots.is_empty() && ui.button("Add Kick as a live destination").clicked()
                            && let Some(slot) = self.stream_config.add_service(crate::streaming::Provider::Kick)
                            && let Some(dest) = self.stream_config.destination_mut(slot)
                        {
                            dest.managed_by_account = true;
                            dest.enabled = true;
                            if self.kick_account.connected {
                                self.kick_account.send(crate::accounts::Command::Refresh);
                            }
                            save = true;
                        }
                    });
                });
            });
            ui.add_space(10.);
        }
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            save |= self.tiktok_card(ui, live);
        });
        ui.add_space(10.);
        let others: Vec<usize> = self
            .stream_config
            .services
            .iter()
            .enumerate()
            .filter(|(_, s)| !matches!(s.provider, crate::streaming::Provider::Kick | crate::streaming::Provider::TikTok))
            .map(|(i, _)| i + 3)
            .collect();
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.strong("Other destinations");
            ui.add_enabled_ui(!live, |ui| {
                for slot in others {
                    save |= self.destination_row(ui, slot, false);
                }
                if ui.button("+ Add destination").on_hover_text("Any RTMP server").clicked() {
                    self.broadcast_window = 3;
                }
            });
        });
        ui.add_space(10.);
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.strong("Output");
            ui.add_enabled_ui(!live, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Video bitrate");
                    save |= ui
                        .add(egui::DragValue::new(&mut self.stream_config.bitrate).range(1000..=20_000).speed(100).suffix(" kbps"))
                        .lost_focus();
                });
                save |= ui.checkbox(&mut self.stream_config.record, "Record while streaming (same encoder)").changed();
            });
            note(ui, "Destinations on one canvas share an encoder unless a connection sets its own bitrate. Twitch accepts up to 6,000 kbps. Recordings finish when streaming stops.");
        });
        if live {
            note(ui, "Stop streaming to change destinations.");
        }
        if save {
            self.stream_error.clear();
            self.save_stream_settings();
        }
        if !self.stream_error.is_empty() {
            ui.colored_label(Color32::LIGHT_RED, &self.stream_error);
        }
        ui.add_space(10.);
        ui.collapsing("Chat troubleshooting", |ui| {
            ui.small(&self.browser_chat.status);
            if ui.button("Open browser chat window").clicked() && let Err(e) = self.browser_chat.start() {
                self.browser_chat.status = e.to_string();
            }
            if ui.button("Restart chat connection").clicked() {
                self.browser_chat.send(serde_json::json!({"type":"reload"}));
            }
            if ui.add_enabled(!self.browser_chat.installing(), egui::Button::new("Repair chat components")).clicked() {
                self.browser_chat.setup();
            }
            if ui.button("Reconnect Twitch activity").clicked() {
                self.accounts[0].send(crate::accounts::Command::Reconnect);
            }
            ui.small("Chat is read from the platform websites. Sending a YouTube message uses 20 units of API quota.");
        });
    }
    /// TikTok has no sign-in API for chat: Rosorah reads the user's LIVE page in the chat browser.
    fn tiktok_card(&mut self, ui: &mut egui::Ui, live: bool) -> bool {
        let mut save = false;
        let name = self.stream_config.tiktok_username.trim().trim_start_matches('@').to_owned();
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("TikTok").heading().color(platform_color(Platform::TikTok)));
            let (badge, color) = if name.is_empty() {
                ("● Not set up".to_owned(), Color32::GRAY)
            } else {
                (format!("● Reading @{name}'s LIVE chat"), Color32::from_rgb(110, 210, 140))
            };
            ui.label(RichText::new(badge).small().color(color));
        });
        ui.horizontal(|ui| {
            ui.label("TikTok username");
            let field = ui.add(egui::TextEdit::singleline(&mut self.stream_config.tiktok_username).hint_text("@yourname").desired_width(180.));
            save |= field.lost_focus();
            if !name.is_empty() && ui.button("Open TikTok page").on_hover_text("Sign in to TikTok here once so Rosorah can read your LIVE chat").clicked() {
                if !self.browser_chat.running() {
                    let _ = self.browser_chat.start();
                }
                self.browser_chat.send(serde_json::json!({"type":"open_platform","platform":"TikTok"}));
            }
        });
        note(ui, "Chat, gifts, follows, subscriptions and shares are read from your TikTok LIVE page while you're live. Sign in to TikTok once with Open TikTok page; TikTok doesn't show LIVE chat to signed-out visitors.");
        save |= ui
            .checkbox(&mut self.stream_config.tiktok_send, "Send chat messages to TikTok from Rosorah")
            .on_hover_text("Off by default")
            .changed();
        if self.stream_config.tiktok_send {
            ui.small("Rosorah types your message into TikTok's chat box on your LIVE page. TikTok has no chat API for this, and it may flag automated typing.");
        }
        let slots: Vec<usize> = self
            .stream_config
            .services
            .iter()
            .enumerate()
            .filter(|(_, s)| s.provider == crate::streaming::Provider::TikTok)
            .map(|(i, _)| i + 3)
            .collect();
        ui.add_enabled_ui(!live, |ui| {
            for slot in &slots {
                save |= self.destination_row(ui, *slot, false);
            }
            if slots.is_empty()
                && ui.button("Add TikTok as a live destination").on_hover_text("Needs a stream key from TikTok LIVE Studio").clicked()
                && let Some(slot) = self.stream_config.add_service(crate::streaming::Provider::TikTok)
            {
                self.broadcast_window = 100 + slot as u8;
                save = true;
            }
        });
        if !name.is_empty() && ui.small_button("Copy TikTok page structure for support").on_hover_text("Copies tags and hook names only, no messages or names. Paste it to whoever is helping if TikTok chat stops appearing.").clicked() {
            self.browser_chat.diagnostic = None;
            self.browser_chat.send(serde_json::json!({"type":"diagnose","platform":"TikTok"}));
            self.tiktok_diagnostic_wanted = true;
        }
        if self.tiktok_diagnostic_wanted && let Some(report) = self.browser_chat.diagnostic.take() {
            ui.ctx().copy_text(report);
            self.tiktok_diagnostic_wanted = false;
            self.message = "TikTok page structure copied".into();
        }
        save
    }
    /// Platform name, sign-in state and Connect / Sign out for one account.
    fn account_row(&mut self, ui: &mut egui::Ui, platform: Platform) {
        let (account, config) = match platform {
            Platform::Twitch => (&mut self.accounts[0], &self.account_config.twitch),
            Platform::YouTube => (&mut self.accounts[1], &self.account_config.youtube),
            _ => (&mut self.kick_account, &self.account_config.kick),
        };
        let configured = !config.id.trim().is_empty() && (platform != Platform::Kick || !config.broker_url.trim().is_empty());
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(platform.name()).heading().color(platform_color(platform)));
            let (badge, color) = if account.connected {
                (format!("● Connected as {}", account.name), Color32::from_rgb(110, 210, 140))
            } else if account.busy {
                ("● Connecting…".into(), Color32::from_rgb(230, 190, 90))
            } else {
                ("● Not connected".into(), Color32::GRAY)
            };
            ui.label(RichText::new(badge).small().color(color));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if account.can_disconnect() {
                    if ui.button(if account.busy { "Cancel" } else { "Sign out" }).clicked() {
                        account.disconnect();
                    }
                } else if ui.add_enabled(configured, egui::Button::new("Connect")).clicked() {
                    account.connect(config.clone());
                }
                if !account.connected && !account.busy && account.can_disconnect() && ui.button("Reconnect").clicked() {
                    account.connect(config.clone());
                }
            });
        });
        // Status adds detail beyond the badge (errors, permission hints); skip the plain restatements.
        let routine = ["Disconnected", "Reconnecting saved account…", "Opening account sign-in…"];
        if !account.connected && !account.status.is_empty() && !routine.contains(&account.status.as_str()) {
            ui.small(&account.status);
        } else if account.connected && !account.status.starts_with("Connected as") {
            ui.small(&account.status);
        }
        if !account.remembered_status.is_empty() {
            ui.small(&account.remembered_status);
        }
        if !configured {
            ui.small("This build has no sign-in registration for this platform.");
        }
        if !account.auth_url.is_empty() {
            ui.hyperlink_to("Open browser to authorize", &account.auth_url);
        }
        if platform == Platform::Kick && account.connected && ui.small_button("Refresh stream key").clicked() {
            account.send(crate::accounts::Command::Refresh);
        }
    }
    /// One live destination: include switch, where its key comes from, and its editor. Returns true when changed.
    fn destination_row(&mut self, ui: &mut egui::Ui, slot: usize, vertical: bool) -> bool {
        let name = self.stream_config.connection_name(slot);
        let canvases = self.project.canvases.clone();
        let Some(dest) = self.stream_config.destination_mut(slot) else { return false };
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            let label = if vertical && !name.to_lowercase().contains("vertical") {
                format!("Go live on {name} (vertical)")
            } else {
                format!("Go live on {name}")
            };
            changed |= ui.checkbox(&mut dest.enabled, label).changed();
            let canvas = dest
                .canvas
                .and_then(|id| canvases.iter().find(|c| c.id == id))
                .or(canvases.first())
                .map(|c| c.name.clone())
                .unwrap_or_default();
            let key = if dest.managed_by_account {
                "key from account"
            } else if dest.key.is_empty() {
                "no stream key yet"
            } else {
                "manual key"
            };
            ui.small(format!("{canvas} · {key}"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Stream key & server…").clicked() {
                    self.broadcast_window = 100 + slot as u8;
                }
            });
        });
        changed
    }
}
