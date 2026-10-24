// Settings > Look & feel: theme, background and panel style.
impl Studio {
    /// Dark, Light and the custom themes in the themes folder, with the tools for making them.
    fn theme_choice(&mut self, ui: &mut egui::Ui) -> bool {
        use auroracast_core::workspace::ThemeMode;
        let mut changed = false;
        ui.strong("Theme");
        let themes = self.appearance.themes();
        let look = &mut self.project.preferences.workspace.appearance;
        ui.horizontal_wrapped(|ui| {
            for (mode, label) in [(ThemeMode::Dark, "Dark"), (ThemeMode::Light, "Light")] {
                if ui.selectable_label(look.custom_theme.is_empty() && look.theme == mode, label).clicked() {
                    look.theme = mode;
                    look.custom_theme.clear();
                    changed = true;
                }
            }
            for (id, name, theme) in &themes {
                let tip = format!("{}{}", theme.as_ref().map_or(String::new(), |t| if t.author.is_empty() { String::new() } else { format!("By {}\n", t.author) }), id);
                if ui.selectable_label(look.custom_theme == *id, name).on_hover_text(tip).clicked() && look.custom_theme != *id {
                    look.custom_theme = id.clone();
                    // A theme's background and panel style come with it; change them below after.
                    if let Some(t) = theme {
                        t.apply_backdrop(look);
                    }
                    changed = true;
                }
            }
        });
        if !look.custom_theme.is_empty() {
            if !self.appearance.theme_error.is_empty() {
                ui.colored_label(crate::appearance::current().error, format!("This theme has a mistake, so its last working version is shown: {}", self.appearance.theme_error));
            }
            if self.appearance.theme().is_some_and(|t| t.sets_accent()) {
                note(ui, "This theme sets its own accent colour.");
            }
        }
        ui.horizontal_wrapped(|ui| {
            if ui.button("Open themes folder").on_hover_text(crate::themes::folder().display().to_string()).clicked() {
                let folder = crate::themes::folder();
                match std::fs::create_dir_all(&folder).and_then(|_| crate::desktop::open(&folder)) {
                    Ok(mut child) => {
                        std::thread::spawn(move || {
                            let _ = child.wait();
                        });
                    }
                    Err(e) => self.tell(format!("Couldn't open the themes folder: {e}")),
                }
            }
            if ui.button("New theme from this look").on_hover_text("Writes a theme file with every colour filled in, ready to edit").clicked() {
                changed |= self.new_theme();
            }
            if ui.add_enabled(self.theme_import.is_none(), egui::Button::new("Import theme…")).on_hover_text("Copies a theme file (.toml) into your themes folder").clicked() {
                let (tx, rx) = mpsc::channel();
                self.theme_import = Some(rx);
                std::thread::spawn(move || {
                    let path = crate::desktop::pick(crate::desktop::Pick::Open, "Choose a Rosorah theme (.toml)", "").unwrap_or_else(|e| {
                        crate::capture_log::event(e.to_string());
                        None
                    });
                    let _ = tx.send(path);
                });
            }
        });
        note(ui, "Themes are small text files anyone can make and share. Edit one in any text editor; Rosorah updates as you save.");
        changed
    }
    /// Saves the current look as a new theme file, chooses it and opens it for editing.
    fn new_theme(&mut self) -> bool {
        let look = &self.project.preferences.workspace.appearance;
        let palette = crate::appearance::solid(look, self.appearance.theme());
        let base = self.appearance.theme().map_or(look.theme, |t| t.base());
        let accent = self.accent();
        let text = crate::themes::template("My theme", base, &palette, accent, on_accent(accent));
        let name = crate::themes::free_name("my-theme");
        let path = crate::themes::folder().join(&name);
        match std::fs::create_dir_all(crate::themes::folder()).and_then(|_| std::fs::write(&path, text)) {
            Ok(()) => {
                self.project.preferences.workspace.appearance.custom_theme = name.clone();
                if let Ok(mut child) = crate::desktop::open(&path) {
                    std::thread::spawn(move || {
                        let _ = child.wait();
                    });
                }
                self.tell(format!("Made {name} in your themes folder. Edit it and save to see changes."));
                true
            }
            Err(e) => {
                self.tell(format!("Couldn't write the theme: {e}"));
                false
            }
        }
    }
    /// A theme file chosen with Import theme…: checked, then copied into the themes folder.
    pub(crate) fn import_theme(&mut self, path: &std::path::Path) {
        if let Err(e) = crate::themes::Theme::read(path) {
            self.tell(format!("That file isn't a Rosorah theme: {e}"));
            return;
        }
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let name = crate::themes::free_name(&stem);
        match std::fs::create_dir_all(crate::themes::folder()).and_then(|_| std::fs::copy(path, crate::themes::folder().join(&name))) {
            Ok(_) => {
                let look = &mut self.project.preferences.workspace.appearance;
                look.custom_theme = name.clone();
                if let Ok(t) = crate::themes::Theme::read(&crate::themes::folder().join(&name)) {
                    t.apply_backdrop(look);
                }
                self.dirty = true;
                self.tell(format!("Imported {name}. A theme with a picture needs its folder copied into the themes folder instead."));
            }
            Err(e) => self.tell(format!("Couldn't import the theme: {e}")),
        }
    }
    fn look_settings(&mut self, ui: &mut egui::Ui) -> bool {
        use auroracast_core::workspace::{Backdrop, BackdropFit, PanelStyle};
        let mut changed = false;
        ui.add_space(12.);
        card().show(ui, |ui| {
            changed |= self.theme_choice(ui);
            let look = &mut self.project.preferences.workspace.appearance;
            #[cfg(not(windows))]
            {
                use auroracast_core::workspace::TitleBar;
                ui.add_space(8.);
                ui.strong("Title bar");
                ui.horizontal(|ui| {
                    for (choice, label) in [(TitleBar::Automatic, "Automatic"), (TitleBar::Always, "GNOME style"), (TitleBar::Never, "Desktop's own")] {
                        changed |= ui.selectable_value(&mut look.title_bar, choice, label).changed();
                    }
                });
                note(
                    ui,
                    if crate::title_bar::gnome() {
                        "Automatic: you're on GNOME, so Rosorah's header is also the title bar, with GNOME's window buttons."
                    } else {
                        "Automatic: your desktop draws the title bar, matching its theme. GNOME style is only used on GNOME."
                    },
                );
            }
            ui.add_space(8.);
            ui.strong("Background");
            ui.horizontal(|ui| {
                for (backdrop, label) in [(Backdrop::None, "Theme color"), (Backdrop::Color, "Custom color"), (Backdrop::Picture, "Picture")] {
                    changed |= ui.selectable_value(&mut look.backdrop, backdrop, label).changed();
                }
            });
            match look.backdrop {
                Backdrop::Color => {
                    ui.horizontal(|ui| {
                        ui.label("Color behind the panels");
                        changed |= ui.color_edit_button_srgb(&mut look.backdrop_color).changed();
                    });
                }
                Backdrop::Picture => {
                    ui.horizontal_wrapped(|ui| {
                        let name = std::path::Path::new(&look.backdrop_image).file_name().map(|n| n.to_string_lossy().into_owned());
                        ui.label(name.unwrap_or_else(|| "No picture chosen".into()));
                        if ui.add_enabled(self.appearance_picker.is_none(), egui::Button::new("Choose picture…")).clicked() {
                            let (tx, rx) = mpsc::channel();
                            self.appearance_picker = Some(rx);
                            std::thread::spawn(move || {
                                let path = crate::desktop::pick(crate::desktop::Pick::Open, "Choose a background picture (PNG or JPEG)", "").unwrap_or_else(|e| {
                                    crate::capture_log::event(e.to_string());
                                    None
                                });
                                let _ = tx.send(path);
                            });
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Fit");
                        for (fit, label, tip) in [
                            (BackdropFit::Fill, "Fill", "Cover the window; edges may be cropped"),
                            (BackdropFit::Fit, "Fit", "Show the whole picture"),
                            (BackdropFit::Stretch, "Stretch", "Fill exactly; may distort"),
                        ] {
                            changed |= ui.selectable_value(&mut look.backdrop_fit, fit, label).on_hover_text(tip).changed();
                        }
                    });
                    if !self.appearance.error.is_empty() {
                        ui.colored_label(crate::appearance::current().error, &self.appearance.error);
                    }
                }
                Backdrop::None => {}
            }
            ui.add_space(8.);
            ui.strong("Panels");
            ui.horizontal_wrapped(|ui| {
                for (style, label, tip) in [
                    (PanelStyle::Solid, "Solid", "No transparency"),
                    (PanelStyle::Transparent, "Transparent", "See the background through tinted panels"),
                    (PanelStyle::Frosted, "Frosted glass", "Blurred but see-through, slightly bright, with a crisp light edge"),
                    (PanelStyle::Matte, "Matte glass", "Heavily diffused: soft color, fine grain, no shine"),
                    (PanelStyle::Liquid, "Liquid glass", "Clear-ish glass with a sheen and bright edges"),
                ] {
                    changed |= ui.selectable_value(&mut look.panels, style, label).on_hover_text(tip).changed();
                }
            });
            if look.panels != PanelStyle::Solid {
                if look.backdrop == Backdrop::None {
                    note(ui, "Choose a custom color or picture to see through the panels.");
                }
                let mut through = (look.transparency * 100.).round();
                if ui.add(egui::Slider::new(&mut through, 0.0..=95.0).suffix("%").text("transparency")).changed() {
                    look.transparency = through / 100.;
                    changed = true;
                }
                if matches!(look.panels, PanelStyle::Frosted | PanelStyle::Matte | PanelStyle::Liquid) {
                    let mut clarity = (look.clarity * 100.).round();
                    if ui.add(egui::Slider::new(&mut clarity, 0.0..=100.0).suffix("%").text("clarity")).on_hover_text("How sharp the background looks through the glass").changed() {
                        look.clarity = clarity / 100.;
                        changed = true;
                    }
                }
                changed |= ui.checkbox(&mut look.contrast_boost, "Contrast boost").on_hover_text("A stronger tint behind text over busy pictures").changed();
            }
            ui.small("Dialogs stay nearly solid so they're always easy to read.");
        });
        changed
    }
}
