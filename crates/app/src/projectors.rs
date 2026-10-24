// Projectors: separate windows showing the program, one scene, or Multiview (every scene at once),
// drawn at the window's own size. Double-click toggles fullscreen; Esc leaves it. They cost
// drawing time only while open. Included into studio.rs.

/// What a projector window shows.
#[derive(Clone, Copy, PartialEq)]
enum Projection {
    /// A canvas as it goes out (the main or vertical video).
    Canvas(Id),
    Scene(Id),
    Multiview,
}
struct Projector {
    id: u64,
    show: Projection,
    /// One picture per thing drawn (Multiview has many), by the key it was drawn under.
    textures: std::collections::HashMap<Id, egui::TextureId>,
    drawn: Instant,
    fullscreen: bool,
}
/// Keys that tell projector pictures apart from the studio's own.
const PROJECTOR_KEY: Id = 0x5052_4f4a_0000_0000;
const MULTIVIEW_KEY: Id = 0x4d55_4c54_0000_0000;

impl Studio {
    fn open_projector(&mut self, show: Projection) {
        let id = self.projectors.iter().map(|p| p.id).max().unwrap_or(0) + 1;
        self.projectors.push(Projector { id, show, textures: Default::default(), drawn: Instant::now() - Duration::from_secs(1), fullscreen: false });
    }
    /// Projector entries for a menu.
    fn projector_menu(&mut self, ui: &mut egui::Ui) {
        let canvases: Vec<(Id, String)> = self.project.canvases.iter().map(|c| (c.id, c.name.clone())).collect();
        for (id, name) in canvases {
            if ui.button(format!("{name} (as it goes out)")).clicked() {
                self.open_projector(Projection::Canvas(id));
                ui.close();
            }
        }
        if ui.button("Multiview · every scene").clicked() {
            self.open_projector(Projection::Multiview);
            ui.close();
        }
        ui.menu_button("A scene", |ui| {
            let scenes: Vec<(Id, String)> = self.project.scenes.iter().map(|s| (s.id, s.name.clone())).collect();
            for (id, name) in scenes {
                if ui.button(name).clicked() {
                    self.open_projector(Projection::Scene(id));
                    ui.close();
                }
            }
        });
        note(ui, "Double-click a projector for fullscreen on its monitor; Esc to leave.");
    }
    /// Draws `scene` (or a canvas when `canvas` is set) under `key` at `size` and returns its picture.
    fn projector_picture(&mut self, textures: &mut std::collections::HashMap<Id, egui::TextureId>, key: Id, canvas: Option<Id>, scene: Id, size: [u32; 2]) -> Option<egui::TextureId> {
        let (Some(gpu), Some(state)) = (&mut self.gpu, &self.render_state) else { return None };
        // Drawn under its own key, so it never disturbs the studio's view of the same canvas.
        let view = match canvas.and_then(|id| self.project.canvases.iter().find(|c| c.id == id)) {
            Some(c) => {
                let copy = auroracast_core::Canvas { id: key, ..c.clone() };
                gpu.render_gpu(&self.project, &copy, size[0], size[1]).ok()?
            }
            None => gpu.render_scene(&self.project, key, scene, size[0], size[1]).ok()?,
        };
        let mut renderer = state.renderer.write();
        let texture = match textures.get(&key) {
            Some(id) => {
                renderer.update_egui_texture_from_wgpu_texture(&state.device, &view, eframe::wgpu::FilterMode::Linear, *id);
                *id
            }
            None => {
                let id = renderer.register_native_texture(&state.device, &view, eframe::wgpu::FilterMode::Linear);
                textures.insert(key, id);
                id
            }
        };
        Some(texture)
    }
    fn projector_windows(&mut self, ctx: &egui::Context) {
        let mut closed = Vec::new();
        for index in 0..self.projectors.len() {
            let (pid, show, fullscreen) = (self.projectors[index].id, self.projectors[index].show, self.projectors[index].fullscreen);
            let title = match show {
                Projection::Canvas(id) => self.project.canvases.iter().find(|c| c.id == id).map_or("Video".into(), |c| c.name.clone()),
                Projection::Scene(id) => self.project.scene(id).map_or("Scene".into(), |s| s.name.clone()),
                Projection::Multiview => "Multiview".into(),
            };
            let builder = egui::ViewportBuilder::default()
                .with_title(format!("{title} · Projector · Rosorah Studio"))
                .with_app_id("io.rosorah.Studio")
                .with_inner_size([960., 540.])
                .with_min_inner_size([240., 135.]);
            let mut projector = std::mem::replace(&mut self.projectors[index], Projector { id: pid, show, textures: Default::default(), drawn: Instant::now(), fullscreen });
            let mut close = false;
            ctx.show_viewport_immediate(egui::ViewportId::from_hash_of(("projector", pid)), builder, |ctx, _| {
                close |= ctx.input(|i| i.viewport().close_requested());
                egui::CentralPanel::default().frame(egui::Frame::new().fill(Color32::BLACK)).show(ctx, |ui| {
                    let area = ui.max_rect();
                    let response = ui.interact(area, ui.id().with("projector"), egui::Sense::click());
                    if response.double_clicked() {
                        projector.fullscreen = !projector.fullscreen;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(projector.fullscreen));
                    }
                    if projector.fullscreen && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                        projector.fullscreen = false;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                    }
                    response.context_menu(|ui| {
                        if ui.button(if projector.fullscreen { "Leave fullscreen" } else { "Fullscreen" }).clicked() {
                            projector.fullscreen = !projector.fullscreen;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(projector.fullscreen));
                            ui.close();
                        }
                        if ui.button("Close projector").clicked() {
                            close = true;
                            ui.close();
                        }
                    });
                    // Multiview refreshes 10 times a second, single views about 30.
                    let interval = if show == Projection::Multiview { 100 } else { 33 };
                    let due = projector.drawn.elapsed() >= Duration::from_millis(interval);
                    let main = self.project.canvases.first().cloned();
                    let (cw, ch) = main.as_ref().map_or((16., 9.), |c| (c.width as f32, c.height as f32));
                    let fit = |rect: egui::Rect, w: f32, h: f32| {
                        let scale = (rect.width() / w).min(rect.height() / h);
                        egui::Rect::from_center_size(rect.center(), egui::vec2(w * scale, h * scale))
                    };
                    let uv = egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.));
                    match show {
                        Projection::Canvas(id) | Projection::Scene(id) => {
                            let (w, h, canvas, scene) = match show {
                                Projection::Canvas(_) => match self.project.canvases.iter().find(|c| c.id == id) {
                                    Some(c) => (c.width as f32, c.height as f32, Some(id), c.scene),
                                    None => return,
                                },
                                _ => (cw, ch, None, id),
                            };
                            let target = fit(area, w, h);
                            let pixels = ctx.pixels_per_point();
                            let size = [((target.width() * pixels) as u32).clamp(16, 4096), ((target.height() * pixels) as u32).clamp(16, 4096)];
                            let key = id ^ PROJECTOR_KEY ^ if canvas.is_some() { 1 } else { 0 };
                            if due || !projector.textures.contains_key(&key) {
                                self.projector_picture(&mut projector.textures, key, canvas, scene, size);
                            }
                            if let Some(texture) = projector.textures.get(&key) {
                                ui.painter().image(*texture, target, uv, Color32::WHITE);
                            }
                        }
                        Projection::Multiview => {
                            // Program and (in Studio Mode) Preview large on top, every scene below.
                            let live = main.as_ref().map(|c| c.scene);
                            let preview = self.studio_preview.filter(|_| self.project.preferences.workspace.studio_mode);
                            let scenes: Vec<(Id, String)> = self.project.scenes.iter().filter(|s| self.project.canvases.first().is_some_and(|c| self.project.canvas_scenes(c.id).contains(&s.id))).map(|s| (s.id, s.name.clone())).collect();
                            let columns = 4usize;
                            let rows = scenes.len().div_ceil(columns).max(1);
                            let top = egui::Rect::from_min_max(area.min, egui::pos2(area.max.x, area.min.y + area.height() * 0.5));
                            let halves = [egui::Rect::from_min_max(top.min, egui::pos2(top.center().x, top.max.y)), egui::Rect::from_min_max(egui::pos2(top.center().x, top.min.y), top.max)];
                            let small = [((cw / 4.) as u32).clamp(16, 640), ((ch / 4.) as u32).clamp(16, 360)];
                            let big = [((cw / 2.) as u32).clamp(16, 1280), ((ch / 2.) as u32).clamp(16, 720)];
                            let mut tiles: Vec<(egui::Rect, Id, Option<Id>, Id, String, Color32)> = Vec::new();
                            if let (Some(main), Some(live)) = (&main, live) {
                                tiles.push((halves[1].shrink(4.), main.id ^ MULTIVIEW_KEY ^ 2, Some(main.id), live, "PROGRAM".into(), crate::appearance::current().live));
                                let shown = preview.unwrap_or(live);
                                tiles.push((halves[0].shrink(4.), shown ^ MULTIVIEW_KEY ^ 1, None, shown, if preview.is_some() { "PREVIEW".into() } else { "PROGRAM (no Studio Mode)".into() }, self.accent()));
                            }
                            let cell_w = area.width() / columns as f32;
                            let cell_h = (area.height() * 0.5) / rows as f32;
                            for (n, (scene, name)) in scenes.iter().enumerate() {
                                let (c, r) = (n % columns, n / columns);
                                let rect = egui::Rect::from_min_size(egui::pos2(area.min.x + c as f32 * cell_w, top.max.y + r as f32 * cell_h), egui::vec2(cell_w, cell_h)).shrink(4.);
                                let colour = if Some(*scene) == live { crate::appearance::current().live } else if Some(*scene) == preview { self.accent() } else { Color32::from_gray(90) };
                                tiles.push((rect, scene ^ MULTIVIEW_KEY, None, *scene, name.clone(), colour));
                            }
                            for (i, (rect, key, canvas, scene, label, colour)) in tiles.into_iter().enumerate() {
                                let size = if i < 2 { big } else { small };
                                if due || !projector.textures.contains_key(&key) {
                                    self.projector_picture(&mut projector.textures, key, canvas, scene, size);
                                }
                                let target = fit(rect, cw, ch);
                                if let Some(texture) = projector.textures.get(&key) {
                                    ui.painter().image(*texture, target, uv, Color32::WHITE);
                                }
                                ui.painter().rect_stroke(target, 2., egui::Stroke::new(2.0_f32, colour), egui::StrokeKind::Outside);
                                ui.painter().text(target.left_bottom() + egui::vec2(6., -6.), egui::Align2::LEFT_BOTTOM, label, egui::FontId::proportional(13.), Color32::WHITE);
                            }
                            // Clicking a scene tile prepares it (Studio Mode) or switches to it.
                            if response.clicked()
                                && let Some(pos) = response.interact_pointer_pos()
                                && pos.y > top.max.y
                            {
                                let (c, r) = (((pos.x - area.min.x) / cell_w) as usize, ((pos.y - top.max.y) / cell_h) as usize);
                                if let (Some((scene, _)), Some(main)) = (scenes.get(r * columns + c), &main) {
                                    self.request_scene(main.id, *scene);
                                }
                            }
                        }
                    }
                    if due {
                        projector.drawn = Instant::now();
                    }
                });
                ctx.request_repaint_after(Duration::from_millis(33));
            });
            // Pictures of a closed projector are freed with it.
            if close {
                if let Some(state) = &self.render_state {
                    let mut renderer = state.renderer.write();
                    for texture in projector.textures.values() {
                        renderer.free_texture(texture);
                    }
                }
                closed.push(pid);
            }
            self.projectors[index] = projector;
        }
        self.projectors.retain(|p| !closed.contains(&p.id));
    }
}
