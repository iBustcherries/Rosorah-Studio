// Settings > Look & feel: theme, background and panel style.
impl Studio {
    fn look_settings(&mut self, ui: &mut egui::Ui) -> bool {
        use auroracast_core::workspace::{Backdrop, BackdropFit, PanelStyle, ThemeMode};
        let mut changed = false;
        ui.add_space(12.);
        card().show(ui, |ui| {
            let look = &mut self.project.preferences.workspace.appearance;
            ui.strong("Theme");
            ui.horizontal(|ui| {
                for (mode, label) in [(ThemeMode::Dark, "Dark"), (ThemeMode::Light, "Light")] {
                    changed |= ui.selectable_value(&mut look.theme, mode, label).changed();
                }
            });
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
                        ui.colored_label(Color32::LIGHT_RED, &self.appearance.error);
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
