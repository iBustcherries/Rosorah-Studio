// Layer Properties: everything about one layer in one window, opened by double-clicking it in Layers.
/// One part of Layer Properties, drawn when its tab is open; `title` heads a part that shares a tab.
fn section<R>(ui: &mut egui::Ui, show: bool, title: Option<&str>, add: impl FnOnce(&mut egui::Ui) -> R) {
    if !show {
        return;
    }
    if let Some(title) = title {
        ui.add_space(6.);
        ui.strong(title);
    }
    add(ui);
}
impl Studio {
    fn layer_properties_window(&mut self, ctx: &egui::Context) {
        let Some((scene, index)) = self.layer_properties else { return };
        let Some(si) = self.project.scenes.iter().position(|s| s.id == scene) else {
            self.layer_properties = None;
            return;
        };
        let Some(source) = self.project.scenes[si].layers.get(index).and_then(|l| self.project.source(l.source)) else {
            self.layer_properties = None;
            return;
        };
        let title = source.name.clone();
        let scene_name = self.project.scenes[si].name.clone();
        let mut open = true;
        // Front and center, standing out from the studio; it can still be moved.
        let shown = egui::Window::new(title)
            .id(egui::Id::new("layer_properties"))
            .open(&mut open)
            .frame(prominent_window(ctx, self.accent()))
            .collapsible(false)
            .default_width(460.)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                note(ui, format!("Layer in {scene_name} · changes apply right away · Esc closes"));
                ui.add_space(4.);
                ui.horizontal(|ui| {
                    for (tab, label) in [(PropertiesTab::Source, "Source"), (PropertiesTab::Position, "Position & size"), (PropertiesTab::Appearance, "Appearance"), (PropertiesTab::Effects, "Effects")] {
                        ui.selectable_value(&mut self.properties_tab, tab, label);
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(ctx.content_rect().height() * 0.7)
                    .show(ui, |ui| self.layer_properties_body(ui, si, index));
            });
        if let Some(shown) = shown {
            self.keep_selection.push(shown.response.rect);
        }
        if !open || (!ctx.wants_keyboard_input() && ctx.input(|i| i.key_pressed(egui::Key::Escape))) {
            self.layer_properties = None;
        }
    }
    /// The one layer menu, shared by right-click and the row's "..." button.
    fn layer_menu(&mut self, ui: &mut egui::Ui, si: usize, i: usize, name: &str, dropped: &mut Option<(usize, usize)>) {
        // A menu keeps the width it first opened with (on the canvas that can be the small Paste
        // menu); unwrapped labels make it grow to fit instead of wrapping every word.
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        let active = self.project.scenes[si].id;
        let count = self.project.scenes[si].layers.len();
        let source_id = self.project.scenes[si].layers[i].source;
        let locked = self.project.scenes[si].layers[i].locked;
        if !self.selections.get(&active).is_some_and(|v| v.contains(&i)) {
            self.select_layer(active, i, false);
        }
        if ui.button("Properties…").clicked() {
            self.layer_properties = Some((active, i));
            ui.close();
        }
        if ui.button("Rename…").clicked() {
            self.layer_rename = Some((source_id, name.to_owned()));
            ui.close();
        }
        if self.project.source(source_id).is_some_and(|s| matches!(s.kind, SourceKind::Widget(_)))
            && ui.button("Edit widget…").clicked()
        {
            self.widget_editor.id = Some(source_id);
            self.widget_editor.sample = false;
            ui.close();
        }
        ui.separator();
        ui.add_enabled_ui(!locked, |ui| {
            if ui.button("Edit crop…").clicked() {
                self.crop_layer = Some((active, i));
                self.selections.insert(active, vec![i]);
                self.layer = Some(i);
                ui.close();
            }
            if ui.button("Reset crop").clicked() {
                auroracast_core::layout::set_crop(&mut self.project.scenes[si].layers[i], [0.; 4]);
                self.changed();
                ui.close();
            }
            ui.menu_button("Align to canvas", |ui| {
                for (label, axis, edge) in [
                    ("Left", 0, 0.),
                    ("Center horizontally", 0, 0.5),
                    ("Right", 0, 1.),
                    ("Top", 1, 0.),
                    ("Center vertically", 1, 0.5),
                    ("Bottom", 1, 1.),
                ] {
                    if ui.button(label).clicked() {
                        let layer = &mut self.project.scenes[si].layers[i];
                        layer.rect[axis] = (1. - layer.rect[axis + 2]) * edge;
                        self.changed();
                        ui.close();
                    }
                }
            });
            if ui.button("Restore source proportions").clicked() {
                self.restore_layer_proportions(si, i);
                ui.close();
            }
        });
        ui.add_enabled_ui(!locked, |ui| {
            ui.menu_button("Rotate / Flip", |ui| {
                let look = &mut self.project.scenes[si].layers[i].look;
                let mut edited = true;
                if ui.button("Rotate 90° right").clicked() {
                    look.rotation = (look.rotation + 90.).rem_euclid(360.);
                } else if ui.button("Rotate 90° left").clicked() {
                    look.rotation = (look.rotation - 90.).rem_euclid(360.);
                } else if ui.button("Flip horizontally").clicked() {
                    look.flip_horizontal = !look.flip_horizontal;
                } else if ui.button("Flip vertically").clicked() {
                    look.flip_vertical = !look.flip_vertical;
                } else if ui.add_enabled(look.rotation != 0. || look.flip_horizontal || look.flip_vertical, egui::Button::new("Reset")).clicked() {
                    look.rotation = 0.;
                    look.flip_horizontal = false;
                    look.flip_vertical = false;
                } else {
                    edited = false;
                }
                if edited {
                    self.changed();
                    ui.close();
                }
            });
        });
        ui.menu_button("Order", |ui| {
            for (label, to) in [
                ("Bring to front", count - 1),
                ("Move forward", (i + 1).min(count - 1)),
                ("Move backward", i.saturating_sub(1)),
                ("Send to back", 0),
            ] {
                if ui.add_enabled(to != i && !locked, egui::Button::new(label)).clicked() {
                    *dropped = Some((i, to));
                    ui.close();
                }
            }
        });
        ui.separator();
        if ui.button("Copy").on_hover_text("Paste it into any scene, on either canvas (Ctrl+C / Ctrl+V)").clicked() {
            self.copy_layers(si, i);
            ui.close();
        }
        if !self.layer_clipboard.is_empty() && ui.button("Paste here").clicked() {
            self.paste_layers(si);
            ui.close();
        }
        if ui.button(if locked { "Unlock" } else { "Lock" }).clicked() {
            self.project.scenes[si].layers[i].locked = !locked;
            self.changed();
            ui.close();
        }
        ui.separator();
        if ui.add_enabled(!locked, egui::Button::new("Delete")).on_hover_text("Remove this layer from this scene").clicked() {
            self.pending_layer_delete = Some((active, i));
            ui.close();
        }
    }
    fn layer_properties_body(&mut self, ui: &mut egui::Ui, si: usize, i: usize) {
        let mut changed = false;
        let tab = self.properties_tab;
        let id = self.project.scenes[si].layers[i].source;
        let locked = self.project.scenes[si].layers[i].locked;
        if locked {
            note(ui, "This layer is locked. Unlock it from its right-click menu to move or resize it.");
        }
        section(ui, tab == PropertiesTab::Source, None, |ui| {
            if let Some(source) = self.project.sources.iter_mut().find(|s| s.id == id) {
                ui.horizontal(|ui| {
                    ui.label("Name");
                    changed |= ui.text_edit_singleline(&mut source.name).changed();
                });
                if let SourceKind::Color(ref mut color) = source.kind {
                    let mut c = Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]);
                    ui.horizontal(|ui| {
                        ui.label("Color");
                        if ui.color_edit_button_srgba(&mut c).changed() {
                            *color = c.to_srgba_unmultiplied();
                            changed = true;
                        }
                    });
                }
                if let SourceKind::Image { path } = &source.kind {
                    ui.small(path.as_str());
                }
            }
            match self.project.source(id).map(|s| &s.kind) {
                Some(SourceKind::ScreenCapture) => self.capture_properties(ui, id),
                Some(SourceKind::Widget(_)) => {
                    if ui.button("Edit widget…").clicked() {
                        self.widget_editor.id = Some(id);
                        self.widget_editor.sample = false;
                    }
                }
                _ => {}
            }
            self.camera_controls(ui, id);
            self.video_controls(ui, id);
            self.browser_source_ui(ui, id);
            self.folder_source_controls(ui, id);
        });
        section(ui, tab == PropertiesTab::Position, None, |ui| {
            ui.add_enabled_ui(!locked, |ui| {
                let layer = &mut self.project.scenes[si].layers[i];
                ui.checkbox(&mut self.keep_aspect, "Keep aspect ratio");
                let canvas = &self.project.canvases[self.canvas];
                let axes = [canvas.width as f32, canvas.height as f32, canvas.width as f32, canvas.height as f32];
                egui::Grid::new("layer-geometry").num_columns(4).spacing([8., 6.]).show(ui, |ui| {
                    for (j, label) in ["X", "Y", "Width", "Height"].iter().enumerate() {
                        let mut pixels = layer.rect[j] * axes[j];
                        ui.label(*label);
                        let range = if j < 2 { -16384.0..=16384.0 } else { 1.0..=16384.0 };
                        if ui.add(egui::DragValue::new(&mut pixels).range(range).speed(1.).suffix(" px")).changed() {
                            let next = pixels / axes[j];
                            if self.keep_aspect && j >= 2 {
                                let other = if j == 2 { 3 } else { 2 };
                                layer.rect[other] *= next / layer.rect[j].max(0.0001);
                            }
                            layer.rect[j] = next;
                            changed = true;
                        }
                        if j % 2 == 1 {
                            ui.end_row();
                        }
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Fit to canvas").on_hover_text("Largest size that fits, keeping proportions").clicked() {
                        let ratio = layer.rect[2] / layer.rect[3].max(0.0001);
                        let (w, h) = if ratio >= 1. { (1., 1. / ratio) } else { (ratio, 1.) };
                        layer.rect = [(1. - w) / 2., (1. - h) / 2., w, h];
                        changed = true;
                    }
                    if ui.button("Stretch to fill").on_hover_text("Cover the whole canvas; may distort").clicked() {
                        layer.rect = [0., 0., 1., 1.];
                        changed = true;
                    }
                    if ui.button("Center").clicked() {
                        layer.rect[0] = (1. - layer.rect[2]) / 2.;
                        layer.rect[1] = (1. - layer.rect[3]) / 2.;
                        changed = true;
                    }
                });
                ui.label("Snap to");
                egui::Grid::new("layer-anchor").spacing([4., 4.]).show(ui, |ui| {
                    for row in 0..3 {
                        for column in 0..3 {
                            let label = ["Top left", "Top", "Top right", "Left", "Center", "Right", "Bottom left", "Bottom", "Bottom right"][row * 3 + column];
                            if ui.add_sized([86., 22.], egui::Button::new(RichText::new(label).small())).clicked() {
                                layer.rect[0] = (1. - layer.rect[2]) * column as f32 / 2.;
                                layer.rect[1] = (1. - layer.rect[3]) * row as f32 / 2.;
                                changed = true;
                            }
                        }
                        ui.end_row();
                    }
                });
            });
            if ui.add_enabled(!locked, egui::Button::new("Restore source proportions")).clicked() {
                self.restore_layer_proportions(si, i);
            }
        });
        section(ui, tab == PropertiesTab::Appearance, None, |ui| {
            ui.add_enabled_ui(!locked, |ui| {
                changed |= ui.add(egui::Slider::new(&mut self.project.scenes[si].layers[i].opacity, 0.0..=1.0).text("Opacity")).changed();
                // The frame this layer is drawn in: square, rounded or a circle, with edges that can fade out.
                let frame = &mut self.project.scenes[si].layers[i].frame;
                ui.horizontal(|ui| {
                    ui.label("Frame");
                    for (shape, label) in [(FrameShape::Square, "Square"), (FrameShape::Rounded, "Rounded"), (FrameShape::Circle, "Circle")] {
                        if ui.selectable_value(&mut frame.shape, shape, label).changed() {
                            if shape == FrameShape::Rounded && frame.radius <= 0. {
                                frame.radius = 0.15;
                            }
                            changed = true;
                        }
                    }
                });
                if frame.shape == FrameShape::Rounded {
                    changed |= ui.add(egui::Slider::new(&mut frame.radius, 0.0..=0.5).text("Corner roundness").show_value(false)).changed();
                }
                changed |= ui.add(egui::Slider::new(&mut frame.feather, 0.0..=0.5).text("Fade edges").show_value(false)).on_hover_text("Blends the edges into what's behind this layer").changed();
            });
            let key = self.project.preferences.chroma_keys.entry(id).or_default();
            ui.horizontal(|ui| {
                changed |= ui.checkbox(&mut key.enabled, "Green screen · remove a background colour").changed();
                ui.label(RichText::new("EXPERIMENTAL").small().strong().color(Color32::from_rgb(230, 190, 90)));
            });
            if key.enabled {
                ui.horizontal(|ui| {
                    changed |= ui.color_edit_button_srgb(&mut key.color).changed();
                    if ui.button("Green").clicked() { key.color = [0, 255, 0]; changed = true; }
                    if ui.button("Blue").clicked() { key.color = [0, 0, 255]; changed = true; }
                });
                changed |= ui.add(egui::Slider::new(&mut key.similarity, 0.0..=1.0).text("Remove").show_value(false)).on_hover_text("More removes shades further from the screen colour").changed();
                changed |= ui.add(egui::Slider::new(&mut key.smoothness, 0.001..=0.5).text("Edge softness").show_value(false)).changed();
                changed |= ui.add(egui::Slider::new(&mut key.spill, 0.0..=1.0).text("Fringe removal").show_value(false)).on_hover_text("Takes the green tint out of hair and edges").changed();
                ui.small("Light the screen evenly and keep some distance from it. Applies to every layer showing this source. Still experimental: please report how it works for you.");
            }
            ui.horizontal(|ui| {
                let scene = self.project.scenes[si].id;
                if ui.add_enabled(!locked, egui::Button::new("Edit crop…")).clicked() {
                    self.crop_layer = Some((scene, i));
                    self.selections.insert(scene, vec![i]);
                    self.layer = Some(i);
                }
                if ui.add_enabled(!locked, egui::Button::new("Reset crop")).clicked() {
                    auroracast_core::layout::set_crop(&mut self.project.scenes[si].layers[i], [0.; 4]);
                    changed = true;
                }
            });
        });
        ui.add_enabled_ui(!locked, |ui| {
            let scene_id = self.project.scenes[si].id;
            let picking = self.look_picker.is_some();
            let mut pick = None;
            let look = &mut self.project.scenes[si].layers[i].look;
            let before = look.clone();
            section(ui, tab == PropertiesTab::Effects, Some("Transform"), |ui| {
                ui.horizontal(|ui| {
                    ui.label("Rotate");
                    ui.add(egui::DragValue::new(&mut look.rotation).range(-360.0..=360.0).suffix("°").speed(0.5));
                    for (label, step) in [("↺ 90°", -90.), ("↻ 90°", 90.)] {
                        if ui.small_button(label).clicked() {
                            look.rotation = ((look.rotation + step) % 360. + 360.) % 360.;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut look.flip_horizontal, "Flip horizontally");
                    ui.checkbox(&mut look.flip_vertical, "Flip vertically");
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label("Blend");
                    for mode in auroracast_core::BlendMode::ALL {
                        ui.selectable_value(&mut look.blend, mode, mode.label());
                    }
                });
            });
            section(ui, tab == PropertiesTab::Effects, Some("Filters"), |ui| {
                // Each filter shows its settings only when it's in use.
                let mut colour = look.colour_adjusted();
                if ui.checkbox(&mut colour, "Colour correction").changed() && !colour {
                    look.brightness = 0.;
                    look.contrast = 0.;
                    look.saturation = 1.;
                    look.hue = 0.;
                    look.gamma = 1.;
                }
                if colour || look.colour_adjusted() {
                    ui.indent("colour", |ui| {
                        ui.add(egui::Slider::new(&mut look.brightness, -1.0..=1.0).text("Brightness"));
                        ui.add(egui::Slider::new(&mut look.contrast, -1.0..=1.0).text("Contrast"));
                        ui.add(egui::Slider::new(&mut look.saturation, 0.0..=2.0).text("Saturation"));
                        ui.add(egui::Slider::new(&mut look.hue, -180.0..=180.0).text("Hue").suffix("°"));
                        ui.add(egui::Slider::new(&mut look.gamma, 0.2..=5.0).text("Gamma").logarithmic(true));
                        if colour && !look.colour_adjusted() {
                            note(ui, "Move a slider to start.");
                        }
                    });
                }
                ui.horizontal(|ui| {
                    ui.label("Colour grade (LUT)");
                    if look.lut.is_empty() {
                        if ui.add_enabled(!picking, egui::Button::new("Choose .cube file…")).clicked() {
                            pick = Some(true);
                        }
                    } else {
                        note(ui, std::path::Path::new(&look.lut).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                        if ui.small_button("Remove").clicked() {
                            look.lut.clear();
                        }
                    }
                });
                if !look.lut.is_empty() {
                    ui.add(egui::Slider::new(&mut look.lut_amount, 0.0..=1.0).text("Strength"));
                }
                ui.add(egui::Slider::new(&mut look.sharpen, 0.0..=1.0).text("Sharpen"));
                ui.horizontal(|ui| {
                    ui.label("Scroll");
                    ui.add(egui::DragValue::new(&mut look.scroll[0]).range(-5.0..=5.0).speed(0.01).prefix("→ ").suffix("/s"));
                    ui.add(egui::DragValue::new(&mut look.scroll[1]).range(-5.0..=5.0).speed(0.01).prefix("↓ ").suffix("/s"));
                }).response.on_hover_text("Moves the picture continuously, wrapping around; in layer widths/heights per second");
                ui.horizontal(|ui| {
                    ui.label("Image mask");
                    if look.mask.is_empty() {
                        if ui.add_enabled(!picking, egui::Button::new("Choose picture…")).on_hover_text("White shows the layer, black hides it").clicked() {
                            pick = Some(false);
                        }
                    } else {
                        note(ui, std::path::Path::new(&look.mask).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                        ui.checkbox(&mut look.mask_invert, "Invert");
                        if ui.small_button("Remove").clicked() {
                            look.mask.clear();
                        }
                    }
                });
                ui.checkbox(&mut look.luma_key, "Luma key · remove dark or light areas");
                if look.luma_key {
                    ui.indent("luma", |ui| {
                        ui.add(egui::Slider::new(&mut look.luma_min, 0.0..=1.0).text("Remove darker than"));
                        ui.add(egui::Slider::new(&mut look.luma_max, 0.0..=1.0).text("Remove lighter than"));
                        ui.add(egui::Slider::new(&mut look.luma_softness, 0.0..=0.5).text("Softness"));
                    });
                }
                ui.horizontal(|ui| {
                    ui.label("Delay");
                    ui.add(egui::DragValue::new(&mut look.delay_ms).range(0..=2000).speed(5).suffix(" ms"));
                    if look.delay_ms > 0 {
                        // About 8 MB per 1080p frame, at the source's frame rate.
                        note(ui, format!("Uses about {} MB of graphics memory at 1080p 30 fps", (look.delay_ms as u64 * 30 / 1000 + 1) * 8));
                    }
                }).response.on_hover_text("Shows this source late, e.g. to line a camera up with your microphone");
            });
            section(ui, tab == PropertiesTab::Effects, Some("Show and hide"), |ui| {
                for (label, value) in [("When shown", &mut look.show), ("When hidden", &mut look.hide)] {
                    ui.horizontal(|ui| {
                        ui.label(label);
                        egui::ComboBox::from_id_salt(("layer-transition", label)).selected_text(value.label()).show_ui(ui, |ui| {
                            for t in auroracast_core::LayerTransition::ALL {
                                ui.selectable_value(value, t, t.label());
                            }
                        });
                    });
                }
                ui.add(egui::DragValue::new(&mut look.transition_ms).range(50..=3000).speed(10).prefix("Takes ").suffix(" ms"));
            });
            if *look != before {
                changed = true;
            }
            if let Some(lut) = pick {
                let (tx, rx) = std::sync::mpsc::channel();
                self.look_picker = Some((scene_id, i, lut, rx));
                std::thread::spawn(move || {
                    let title = if lut { "Choose a .cube colour look-up table" } else { "Choose a mask picture (PNG or JPEG)" };
                    let _ = tx.send(crate::desktop::pick(crate::desktop::Pick::Open, title, "").ok().flatten());
                });
            }
        });
        ui.separator();
        if ui.add_enabled(!locked, egui::Button::new("Remove from scene")).clicked() {
            self.pending_layer_delete = Some((self.project.scenes[si].id, i));
            self.layer_properties = None;
        }
        if changed {
            self.changed();
        }
    }
    /// Window, monitor or automatic game capture for one capture source, plus shared capture options.
    fn capture_properties(&mut self, ui: &mut egui::Ui, source: Id) {
        let capture_id = Some(source);
        let mut mode = capture_id.map_or(self.captures.options.target, |id| self.captures.mode(id));
        let before = mode;
        #[cfg(windows)]
        {
            egui::ComboBox::from_id_salt("windows-capture-mode")
                .selected_text(match mode {rosorah_capture::Target::AutoFullscreen=>"Automatic fullscreen application",rosorah_capture::Target::Window=>"Specific window",_=>"Entire monitor"})
                .show_ui(ui,|ui|{
                    ui.selectable_value(&mut mode,rosorah_capture::Target::AutoFullscreen,"Automatic fullscreen application");
                    ui.selectable_value(&mut mode,rosorah_capture::Target::Window,"Specific window");
                    ui.selectable_value(&mut mode,rosorah_capture::Target::Screen,"Entire monitor");
                });
            if let Some(id)=capture_id {
                if mode==rosorah_capture::Target::Window {
                    let windows=crate::desktop::windows();
                    let mut selected=self.captures.windows.get(&id).map(|w|w.handle).unwrap_or(0);
                    let previous=selected;
                    let label=self.captures.windows.get(&id).map(|w|w.title.as_str()).unwrap_or("Select a window");
                    egui::ComboBox::from_id_salt(("windows-window",id)).selected_text(label).show_ui(ui,|ui|{
                        for w in &windows {ui.selectable_value(&mut selected,w.handle,format!("{} · PID {}",w.title,w.process.pid));}
                    });
                    if selected!=previous {if let Some(w)=windows.into_iter().find(|w|w.handle==selected){self.captures.stop(id);self.captures.windows.insert(id,w);}}
                } else if mode==rosorah_capture::Target::Screen {
                    let mut selected=self.captures.monitors.get(&id).cloned().unwrap_or_default();let previous=selected.clone();
                    let (monitors,error)=crate::desktop::monitors();
                    egui::ComboBox::from_id_salt(("windows-monitor",id)).selected_text(if selected.is_empty(){"Select a monitor"}else{&selected}).show_ui(ui,|ui|{
                        for monitor in monitors {ui.selectable_value(&mut selected,monitor.id.clone(),format!("{} · {} × {}",monitor.id,monitor.width,monitor.height));}
                    });
                    if !error.is_empty(){ui.label(error);}
                    if selected!=previous{self.captures.stop(id);self.captures.monitors.insert(id,selected.clone());self.project.preferences.windows_capture_monitors.insert(id,selected);self.changed();}
                }
            }
            if mode!=rosorah_capture::Target::AutoFullscreen && ui.button("Refresh windows / monitors").clicked(){crate::desktop::refresh();}
            if crate::desktop::probing(){ui.spinner();ui.ctx().request_repaint_after(Duration::from_millis(100));}
            ui.small(if mode==rosorah_capture::Target::AutoFullscreen {"Automatically follows eligible fullscreen graphics applications. Uses Windows window capture; no renderer injection or desktop fallback."} else if mode==rosorah_capture::Target::Window {"Captures only the selected window. Protected or minimized windows may not supply frames."} else {"Captures the entire selected monitor."});
        }
        #[cfg(not(windows))]
        egui::ComboBox::from_id_salt("capture-target")
            .width(ui.available_width().min(250.))
            .selected_text(if mode == rosorah_capture::Target::AutoFullscreen {
                "Automatic renderer capture"
            } else {
                "Choose window / screen"
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut mode,
                    rosorah_capture::Target::AutoFullscreen,
                    "Automatic game capture",
                );
                ui.selectable_value(
                    &mut mode,
                    rosorah_capture::Target::WindowOrScreen,
                    "Choose window / screen",
                );
            });
        if mode != before {
            if let Some(id) = capture_id {
                self.captures.set_mode(id, mode);
                self.remember_capture_mode(id);
            } else {
                self.captures.options.target = mode;
            }
        }
        if let Some(source_id) = capture_id {
            ui.horizontal_wrapped(|ui| {
                if self.captures.enabled(source_id) {
                    if ui.button("Pause capture").clicked() {
                        self.captures.stop(source_id);
                    }
                } else if mode == rosorah_capture::Target::AutoFullscreen
                    && ui.button("Resume automatic capture").clicked()
                {
                    self.captures.start(source_id);
                }
                if mode != rosorah_capture::Target::AutoFullscreen && ui.button(if cfg!(windows) { "Start selected source" } else { "Choose window…" }).clicked() {
                    self.captures.choose_manual(source_id);
                    self.remember_capture_mode(source_id);
                }
            });
            let status = self.captures.status(source_id);
            ui.label(RichText::new(status.chars().take(160).collect::<String>()).small())
                .on_hover_text(&status);
        }
        if ui
            .checkbox(
                &mut self.project.preferences.auto_game_audio,
                "Include game audio automatically",
            )
            .changed()
        {
            self.changed();
        }
        if self.project.preferences.auto_game_audio {
            let audio = self.audio.view();
            ui.small(
                audio
                    .status
                    .get(&crate::game_audio::CHANNEL)
                    .map_or("Waiting for a captured game", String::as_str),
            );
        }
        if !self.advanced() {
            if crate::launch_hook::needs_permission() {
                note(ui, "Some games need a one-time permission before Rosorah can show them.");
                if ui.button("Allow game capture").clicked() {
                    crate::launch_hook::authorize();
                }
            }
            return;
        }
        ui.collapsing("Capture details", |ui| {
            ui.label(crate::renderer_capture::installation_status());
            ui.label(crate::launch_hook::status());
            #[cfg(not(windows))]
            if !crate::launch_hook::active()
                && ui
                    .button("Turn on OpenGL launch activation (experimental)")
                    .on_hover_text("Only for OpenGL games. It attaches to games as Steam starts them, which can stop some from launching.")
                    .clicked()
            {
                crate::launch_hook::enable();
            }
            if crate::launch_hook::needs_permission()
                && ui.button("Allow OpenGL activation without restarting Steam").clicked()
            {
                crate::launch_hook::authorize();
            }
            #[cfg(not(windows))]
            ui.small("Experimental launch activation applies to games started after Rosorah. An already-running OpenGL game may need restarting; Steam and your desktop session can stay open.");
            if ui.button("Copy diagnostics").clicked() {
                ui.ctx().copy_text(format!(
                    "{}\n{}\nGame audio: {}\nAudio error: {}\nAudio connection: {}\nCaptured process: {:?}",
                    crate::capture_log::details(),
                    capture_id
                        .map(|id| self.captures.status(id))
                        .unwrap_or_default(),
                    self.audio.view().status.get(&crate::game_audio::CHANNEL).cloned().unwrap_or_default(),
                    self.audio.view().error,
                    self.audio.view().capture_error,
                    self.captures.game(),
                ));
            }
        });
        let before_resolution = self.project.preferences.capture_resolution;
        ui.label("Game capture resolution");
        egui::ComboBox::from_id_salt("capture-resolution")
            .selected_text(before_resolution.label())
            .show_ui(ui, |ui| {
                for value in auroracast_core::CaptureResolution::ALL {
                    ui.selectable_value(
                        &mut self.project.preferences.capture_resolution,
                        value,
                        value.label(),
                    );
                }
            });
        if self.project.preferences.capture_resolution != before_resolution {
            self.captures.options.resolution = self.project.preferences.capture_resolution.wire();
            self.captures.apply_game_options();
            self.changed();
        }
        ui.small("Keeps the game's proportions; smaller games are not upscaled. Recording size is set in Canvases & recording.");
        ui.collapsing("Advanced capture", |ui| {
            ui.label("Applies when capture starts");
            ui.horizontal(|ui| {
                ui.label("Frame rate");
                ui.selectable_value(&mut self.captures.options.fps, 30, "30");
                ui.selectable_value(&mut self.captures.options.fps, 60, "60");
            });
            if ui.checkbox(&mut self.captures.options.cursor, "Show cursor").changed() {
                #[cfg(windows)] {self.project.preferences.windows_capture_cursor=self.captures.options.cursor;self.changed();}
            }
        });
    }
}
impl Studio {
    /// Adds a new source with one layer in scene `si`, selects it, and returns the source id.
    fn add_source_layer(&mut self, si: usize, name: &str, kind: SourceKind, rect: [f32; 4]) -> Id {
        let id = self.project.id();
        self.project.sources.push(Source { id, name: name.into(), kind });
        self.project.scenes[si].layers.push(Layer { crop: [0.; 4], group: 0, frame: Default::default(), look: Default::default(), source: id, rect, visible: true, locked: false, opacity: 1. });
        let index = self.project.scenes[si].layers.len() - 1;
        self.layer = Some(index);
        self.selections.insert(self.project.scenes[si].id, vec![index]);
        self.changed();
        id
    }
}
#[cfg(test)]
mod tests {
    use eframe::egui;
    /// The canvas menu first opened as the small Paste menu used to keep that width, wrapping the
    /// layer menu one word per line.
    fn menu_width(extend: bool) -> f32 {
        let ctx = egui::Context::default();
        let pos = egui::pos2(100., 100.);
        let mut width = 0f32;
        let mut frame = |events: Vec<egui::Event>, long: bool, width: &mut f32| {
            let input = egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 600.))), ..Default::default() };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let response = ui.allocate_rect(egui::Rect::from_min_size(egui::pos2(50., 50.), egui::vec2(200., 200.)), egui::Sense::click());
                    response.context_menu(|ui| {
                        if long {
                            if extend {
                                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                            }
                            let _ = ui.button("Restore source proportions");
                            *width = width.max(ui.min_rect().width());
                        } else {
                            let _ = ui.button("Paste");
                        }
                    });
                });
            });
        };
        let click = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() };
        let escape = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        for long in [false, true] {
            frame(vec![egui::Event::PointerMoved(pos)], long, &mut width);
            frame(vec![click(true)], long, &mut width);
            frame(vec![click(false)], long, &mut width);
            for _ in 0..4 {
                frame(vec![], long, &mut width);
            }
            if !long {
                frame(vec![escape.clone()], long, &mut width);
                frame(vec![], long, &mut width);
            }
        }
        width
    }
    #[test]
    fn layer_menu_grows_past_an_earlier_narrow_menu() {
        assert!(menu_width(false) < 120., "the wrapping this guards against still happens without the fix");
        assert!(menu_width(true) > 150., "the menu fits its longest item");
    }
}
