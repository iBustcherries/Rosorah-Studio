//! Web pages shown as studio panels. Each page runs in the browser runtime off screen
//! (browser/dock_browser.py); its frames are painted here and the panel's mouse and
//! keyboard input is sent back to it, so the page behaves as if it were in the panel.
use eframe::egui;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{Read, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Default)]
struct Shared {
    frame: Option<(usize, usize, Vec<u8>)>,
    title: String,
    note: String,
    url: String,
    ended: bool,
}
struct Page {
    child: Option<Child>,
    stdin: ChildStdin,
    shared: Arc<Mutex<Shared>>,
    texture: Option<egui::TextureHandle>,
    url: String,
    /// Page size in pixels and zoom last sent, and when.
    sent: ([u32; 2], f32, Instant),
    drawn: Instant,
    paused: bool,
}
impl Page {
    fn start(ctx: &egui::Context, url: &str, size: [u32; 2], zoom: f32) -> anyhow::Result<Self> {
        let storage = crate::browser_chat::venv().with_file_name("browser-docks");
        std::fs::create_dir_all(&storage)?;
        let mut child = Command::new(crate::browser_chat::python())
            .arg(crate::browser_chat::base().join("dock_browser.py"))
            .env_remove("LD_LIBRARY_PATH")
            .env_remove("QT_PLUGIN_PATH")
            .env_remove("QT_QPA_PLATFORM_PLUGIN_PATH")
            .env("QT_SCALE_FACTOR", "1")
            .env("QT_AUTO_SCREEN_SCALE_FACTOR", "0")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let mut stdin = child.stdin.take().unwrap();
        let config = json!({"url": url, "width": size[0], "height": size[1], "zoom": zoom, "storage": storage});
        writeln!(stdin, "{config}")?;
        let shared = Arc::new(Mutex::new(Shared::default()));
        let mut stdout = child.stdout.take().unwrap();
        let (into, ctx) = (shared.clone(), ctx.clone());
        std::thread::spawn(move || {
            let mut header = [0u8; 20];
            while stdout.read_exact(&mut header).is_ok() && &header[..4] == b"RSDK" {
                let field = |i: usize| u32::from_le_bytes(header[4 + i * 4..8 + i * 4].try_into().unwrap()) as usize;
                let (kind, w, h, n) = (field(0), field(1), field(2), field(3));
                if n > 3840 * 2160 * 4 || (kind == 1 && n != w * h * 4) {
                    break;
                }
                let mut data = vec![0; n];
                if stdout.read_exact(&mut data).is_err() {
                    break;
                }
                let mut s = into.lock().unwrap();
                match kind {
                    1 => s.frame = Some((w, h, data)),
                    2 => s.title = String::from_utf8_lossy(&data).into(),
                    3 => s.note = String::from_utf8_lossy(&data).into(),
                    4 => s.url = String::from_utf8_lossy(&data).into(),
                    _ => {}
                }
                drop(s);
                ctx.request_repaint();
            }
            into.lock().unwrap().ended = true;
            ctx.request_repaint();
        });
        Ok(Self { child: Some(child), stdin, shared, texture: None, url: url.into(), sent: (size, zoom, Instant::now()), drawn: Instant::now(), paused: false })
    }
    fn send(&mut self, command: Value) {
        let _ = writeln!(self.stdin, "{command}");
    }
}
impl Drop for Page {
    fn drop(&mut self) {
        self.send(json!({"type": "quit"}));
        let Some(mut child) = self.child.take() else { return };
        std::thread::spawn(move || {
            for _ in 0..40 {
                if child.try_wait().ok().flatten().is_some() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            let _ = child.kill();
            let _ = child.wait();
        });
    }
}

#[derive(Default)]
pub struct Docks {
    pages: HashMap<u64, Page>,
    errors: HashMap<u64, String>,
}
impl Docks {
    /// Closes the page of a removed web panel.
    pub fn remove(&mut self, id: u64) {
        self.pages.remove(&id);
        self.errors.remove(&id);
    }
    /// Pages whose panel isn't on screen stop sending frames until it is again.
    pub fn tick(&mut self) {
        for page in self.pages.values_mut() {
            if !page.paused && page.drawn.elapsed() > Duration::from_secs(1) {
                page.paused = true;
                page.send(json!({"type": "pause"}));
            }
        }
    }
    pub fn ui(&mut self, ui: &mut egui::Ui, id: u64, url: &str) {
        if !crate::browser_source::ready() {
            ui.label("Web pages use the browser runtime. Prepare it from a browser source's settings, then come back here.");
            return;
        }
        let ppp = ui.ctx().pixels_per_point();
        if self.pages.get(&id).is_some_and(|p| p.url != url) {
            self.pages.remove(&id);
        }
        // A page that closed waits for Reload instead of restarting over and over.
        if self.pages.get(&id).is_some_and(|p| p.shared.lock().unwrap().ended) {
            self.pages.remove(&id);
            self.errors.insert(id, "The page closed. Press ⟳ to open it again.".into());
        }
        let mut go = None;
        ui.horizontal(|ui| {
            let page = self.pages.get(&id);
            if ui.small_button("←").on_hover_text("Back").clicked() {
                go = Some("back");
            }
            if ui.small_button("→").on_hover_text("Forward").clicked() {
                go = Some("forward");
            }
            if ui.small_button("⟳").on_hover_text("Reload").clicked() {
                go = Some("reload");
            }
            let shown = page.map(|p| p.shared.lock().unwrap().url.clone()).filter(|u| !u.is_empty()).unwrap_or_else(|| url.to_owned());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("↗").on_hover_text("Open in your web browser").clicked() {
                    let _ = crate::desktop::open(&shown);
                }
                ui.add(egui::Label::new(egui::RichText::new(&shown).small().weak()).truncate());
            });
        });
        if go == Some("reload") && self.errors.remove(&id).is_some() {
            go = None;
        }
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        let size = [(rect.width() * ppp).round().max(16.) as u32, (rect.height() * ppp).round().max(16.) as u32];
        if !self.pages.contains_key(&id) && !self.errors.contains_key(&id) {
            match Page::start(ui.ctx(), url, size, ppp) {
                Ok(page) => {
                    self.pages.insert(id, page);
                }
                Err(e) => {
                    self.errors.insert(id, format!("The page couldn't start: {e}"));
                }
            }
        }
        let Some(page) = self.pages.get_mut(&id) else {
            if let Some(error) = self.errors.get(&id) {
                ui.painter_at(rect).text(rect.center(), egui::Align2::CENTER_CENTER, error, egui::FontId::proportional(14.), ui.visuals().warn_fg_color);
            }
            return;
        };
        page.drawn = Instant::now();
        if page.paused {
            page.paused = false;
            page.send(json!({"type": "resume"}));
        }
        if let Some(command) = go {
            page.send(json!({"type": command}));
        }
        // Follow the panel's size, without flooding the page while a divider is being dragged.
        if (page.sent.0 != size || page.sent.1 != ppp) && page.sent.2.elapsed() > Duration::from_millis(120) {
            page.sent = (size, ppp, Instant::now());
            page.send(json!({"type": "resize", "width": size[0], "height": size[1], "zoom": ppp}));
        }
        if let Some((w, h, data)) = page.shared.lock().unwrap().frame.take() {
            let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &data);
            match &mut page.texture {
                Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
                None => page.texture = Some(ui.ctx().load_texture(format!("web-dock-{id}"), image, egui::TextureOptions::LINEAR)),
            }
        }
        let painter = ui.painter_at(rect);
        match &page.texture {
            Some(texture) => {
                // Drawn at the page's own size while a resize catches up, so it never stretches.
                let shown = egui::Rect::from_min_size(rect.min, texture.size_vec2() / ppp);
                painter.image(texture.id(), shown, egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.)), egui::Color32::WHITE);
            }
            None => {
                painter.text(rect.center(), egui::Align2::CENTER_CENTER, "Loading…", egui::FontId::proportional(14.), ui.visuals().weak_text_color());
            }
        }
        // Once the page draws, it shows its own errors; the note covers the time before that.
        let note = page.shared.lock().unwrap().note.clone();
        if !note.is_empty() && page.texture.is_none() {
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, note, egui::FontId::proportional(14.), ui.visuals().warn_fg_color);
        }
        forward_input(ui, &response, rect, ppp, page);
    }
}

/// Sends the panel's pointer and (while it has focus) keyboard input to the page.
fn forward_input(ui: &egui::Ui, response: &egui::Response, rect: egui::Rect, ppp: f32, page: &mut Page) {
    if response.is_pointer_button_down_on() || response.clicked() {
        response.request_focus();
    }
    let focused = response.has_focus();
    if focused {
        // Tab, arrows and Esc belong to the page while it has focus.
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(response.id, egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true })
        });
    }
    let at = |p: egui::Pos2| ((p.x - rect.min.x) * ppp, (p.y - rect.min.y) * ppp);
    let mods = |m: &egui::Modifiers| json!({"shift": m.shift, "ctrl": m.command || m.ctrl, "alt": m.alt});
    let hovered = response.hovered() || response.is_pointer_button_down_on();
    let events = ui.input(|i| i.events.clone());
    let mut out = Vec::new();
    for event in events {
        match event {
            egui::Event::PointerMoved(p) if hovered => {
                let (x, y) = at(p);
                out.push(json!({"type": "move", "x": x, "y": y}));
            }
            egui::Event::PointerButton { pos, button, pressed, modifiers } if hovered || !pressed => {
                let (x, y) = at(pos);
                let button = match button {
                    egui::PointerButton::Primary => "left",
                    egui::PointerButton::Secondary => "right",
                    egui::PointerButton::Middle => "middle",
                    egui::PointerButton::Extra1 => "back",
                    egui::PointerButton::Extra2 => "forward",
                };
                let mut c = mods(&modifiers);
                c["type"] = (if pressed { "press" } else { "release" }).into();
                c["x"] = x.into();
                c["y"] = y.into();
                c["button"] = button.into();
                out.push(c);
            }
            egui::Event::MouseWheel { unit, delta, modifiers, .. } if response.hovered() => {
                let scale = match unit {
                    egui::MouseWheelUnit::Point => ppp,
                    egui::MouseWheelUnit::Line => 40. * ppp,
                    egui::MouseWheelUnit::Page => rect.height() * ppp,
                };
                let (x, y) = ui.input(|i| i.pointer.latest_pos()).map(at).unwrap_or_default();
                let mut c = mods(&modifiers);
                c["type"] = "wheel".into();
                c["x"] = x.into();
                c["y"] = y.into();
                c["dx"] = ((delta.x * scale) as i32).into();
                c["dy"] = ((delta.y * scale) as i32).into();
                out.push(c);
            }
            egui::Event::Text(text) if focused => out.push(json!({"type": "text", "text": text})),
            egui::Event::Paste(text) if focused => out.push(json!({"type": "text", "text": text})),
            egui::Event::Copy if focused => out.push(json!({"type": "key", "key": "C", "ctrl": true, "down": true})),
            egui::Event::Cut if focused => out.push(json!({"type": "key", "key": "X", "ctrl": true, "down": true})),
            egui::Event::Key { key, pressed, modifiers, .. } if focused => {
                use egui::Key as K;
                let name = match key {
                    K::Enter => "Enter",
                    K::Escape => "Escape",
                    K::Tab => "Tab",
                    K::Backspace => "Backspace",
                    K::Delete => "Delete",
                    K::Insert => "Insert",
                    K::Home => "Home",
                    K::End => "End",
                    K::PageUp => "PageUp",
                    K::PageDown => "PageDown",
                    K::ArrowLeft => "Left",
                    K::ArrowRight => "Right",
                    K::ArrowUp => "Up",
                    K::ArrowDown => "Down",
                    // Plain letters arrive as Text; only shortcuts (Ctrl+A, Ctrl+Z…) go as keys.
                    _ if modifiers.command || modifiers.ctrl => {
                        // Copy, cut and paste arrive as their own events below.
                        let n = key.name();
                        if n.len() == 1 && !matches!(n, "C" | "V" | "X") { n } else { continue }
                    }
                    _ => continue,
                };
                let mut c = mods(&modifiers);
                c["type"] = "key".into();
                c["key"] = name.into();
                c["down"] = pressed.into();
                out.push(c);
            }
            _ => {}
        }
    }
    for command in out {
        page.send(command);
    }
}
