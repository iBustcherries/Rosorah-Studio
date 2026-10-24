use crate::workspace::{Panel, Workspace};
use auroracast_core::*;
use auroracast_gpu::Compositor;
use eframe::egui::{self, Color32, RichText, Vec2};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Colors follow Settings > Look & feel (theme and see-through panels).
fn bg() -> Color32 {
    crate::appearance::current().bg
}
fn panel() -> Color32 {
    crate::appearance::current().panel
}
fn muted() -> Color32 {
    crate::appearance::current().muted
}
pub fn default_project_path() -> PathBuf {
    let base = auroracast_core::platform::config_root();
    migrate_project(&base)
}
fn migrate_project(base: &std::path::Path) -> PathBuf {
    let new = base.join("rosorah-studio/project.json");
    let old = base.join("auroracast/project.json");
    if !new.exists() && old.exists() {
        // Preserve the original project and layout. Fall back if migration fails.
        if std::fs::create_dir_all(new.parent().unwrap()).is_err()
            || std::fs::copy(&old, &new).is_err()
        {
            return old;
        }
        let _ = std::fs::copy(
            old.with_extension("layout.json"),
            new.with_extension("layout.json"),
        );
    }
    new
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn platform_color(p: Platform) -> Color32 {
    let [r, g, b] = p.color();
    Color32::from_rgb(r, g, b)
}
enum ChatAction {
    Moderate(Platform, crate::accounts::Moderation),
    Hide(Platform, String),
    /// Leave this chatter out of every chat widget.
    HideFromWidgets(String),
    Clear(Option<Platform>),
}
const TIMEOUTS: [u32; 5] = [60, 300, 600, 3600, 86400];
/// Mixer-only id for the combined Alerts strip; never stored as a project channel.
const ALERTS_STRIP: Id = u64::MAX - 1;
/// Right-click menu for one chat message: moderation where the platform allows it, then local actions.
fn message_menu(
    ui: &mut egui::Ui,
    m: &auroracast_core::ChatMessage,
    connected: bool,
    action: &mut Option<ChatAction>,
) {
    use crate::accounts::{ModAction, Moderation};
    ui.label(RichText::new(format!("{} · {}", m.author, m.platform.name())).strong());
    let message = m.id.strip_prefix("web-").unwrap_or(&m.id).to_string();
    let user = match m.platform {
        Platform::Twitch => m.metadata.user_id.clone(),
        Platform::Kick => m.author.clone(),
        _ => String::new(),
    };
    let moderate = |kind: ModAction| {
        ChatAction::Moderate(m.platform, Moderation { action: kind, user: user.clone(), author: m.author.clone(), message: message.clone(), text: m.text.clone() })
    };
    let real = !m.demo && !m.id.starts_with("event-");
    let identified = match m.platform {
        Platform::YouTube => !message.is_empty(),
        _ => !user.is_empty(),
    };
    if !real {
    } else if !connected {
        note(ui, format!("Connect {} to moderate.", m.platform.name()));
    } else if !identified {
        note(ui, format!("This message can't be moderated from Rosorah; use {}.", m.platform.name()));
    } else {
        if matches!(m.platform, Platform::YouTube | Platform::Kick) && ui.button("Delete message").clicked() {
            *action = Some(moderate(ModAction::Delete));
        }
        ui.menu_button("Timeout", |ui| {
            for seconds in TIMEOUTS {
                if ui.button(crate::accounts::duration_label(seconds)).clicked() {
                    *action = Some(moderate(ModAction::Timeout(seconds)));
                }
            }
        });
        ui.menu_button("Ban", |ui| {
            if ui.button(format!("Yes, ban {}", m.author)).clicked() {
                *action = Some(moderate(ModAction::Ban));
            }
        });
        if m.platform != Platform::YouTube && ui.button("Unban").clicked() {
            *action = Some(moderate(ModAction::Unban));
        }
    }
    if real {
        ui.separator();
    }
    if ui.button("Hide from panel").clicked() {
        *action = Some(ChatAction::Hide(m.platform, m.id.clone()));
    }
    if ui.button("Hide from chat widgets").on_hover_text(format!("{}'s messages won't show in your on-screen chat", m.author)).clicked() {
        *action = Some(ChatAction::HideFromWidgets(m.author.clone()));
    }
}
/// A yes/no question in the middle of the screen. Nothing else can be clicked until it's answered;
/// Esc counts as Cancel. Returns Some(true) for yes, Some(false) for cancel.
fn confirm(ctx: &egui::Context, id: &str, title: &str, body: &str, yes: &str) -> Option<bool> {
    let mut answer = None;
    pop_modal(ctx, egui::Id::new(("confirm", id))).show(ctx, |ui| {
        ui.set_width(360.);
        ui.heading(title);
        ui.add_space(6.);
        ui.label(body);
        ui.add_space(10.);
        ui.horizontal(|ui| {
            if ui.button(yes).clicked() {
                answer = Some(true);
            }
            if ui.button("Cancel").clicked() {
                answer = Some(false);
            }
        });
    });
    if answer.is_none() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        answer = Some(false);
    }
    answer
}
/// Pages of the Add dialog.
#[derive(Clone, Copy, PartialEq)]
enum AddView {
    Main,
    Devices,
    Widgets,
    Scenes,
}
/// What the Add dialog adds.
#[derive(Clone, Copy)]
enum AddChoice {
    Capture,
    Camera,
    Ndi,
    Spout,
    Media,
    Image,
    Slideshow,
    Playlist,
    Scene(Id),
    Browser,
    Color,
    Widget(widgets::Kind),
    Reuse(Id),
}
/// An icon and a short name for a kind of layer.
fn source_kind_label(kind: &SourceKind) -> (&'static str, &'static str) {
    match kind {
        SourceKind::ScreenCapture | SourceKind::Game => ("🖥", "Capture"),
        SourceKind::VideoCamera(_) | SourceKind::Camera => ("📷", "Camera"),
        SourceKind::VideoFile(_) => ("🎞", "Media"),
        SourceKind::Ndi(_) => ("📡", "NDI"),
        SourceKind::Spout(_) => ("🔀", "Spout"),
        SourceKind::Image { .. } => ("🖼", "Image"),
        SourceKind::Slideshow(_) => ("🗐", "Slideshow"),
        SourceKind::Browser { .. } => ("🌐", "Browser source"),
        SourceKind::Color(_) => ("🎨", "Colour"),
        SourceKind::Widget(w) if w.kind == widgets::Kind::AlertBox => ("🔔", "Alerts"),
        SourceKind::Widget(w) if w.kind == widgets::Kind::Text => ("✏", "Text"),
        SourceKind::Widget(w) => ("⚙", w.kind.label()),
        SourceKind::Scene(_) => ("🗖", "Scene"),
        SourceKind::ChatOverlay => ("💬", "Chat"),
        SourceKind::TestPattern => ("▦", "Test pattern"),
    }
}
/// Tiles and cards in the Add dialog stand out a little from the dialog behind them.
fn tile_frame(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::new().fill(ui.visuals().widgets.inactive.weak_bg_fill).corner_radius(12).inner_margin(12)
}
/// Rows of Add dialog tiles, wrapping, with their tops lined up.
fn tile_rows<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.with_layout(egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true), |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(8., 8.);
        add(ui)
    })
    .inner
}
/// One tile in the Add dialog. Returns true when clicked.
fn add_tile(ui: &mut egui::Ui, icon: &str, title: &str, hint: &str, enabled: bool) -> bool {
    let size = egui::vec2(160., if hint.is_empty() { 74. } else { 104. });
    let response = ui
        .allocate_ui_with_layout(size, egui::Layout::top_down(egui::Align::Min), |ui| {
            tile_frame(ui).show(ui, |ui| {
                // The frame's margins are inside the tile's size.
                ui.set_width(size.x - 24.);
                ui.set_height(size.y - 24.);
                if !enabled {
                    ui.multiply_opacity(0.4);
                }
                ui.allocate_ui(egui::vec2(size.x - 24., 28.), |ui| ui.label(RichText::new(icon).size(22.)));
                ui.add(egui::Label::new(RichText::new(title).strong()).truncate());
                if !hint.is_empty() {
                    ui.add(egui::Label::new(RichText::new(hint).small().color(muted())).wrap());
                }
            })
        })
        .inner
        .response;
    if !enabled {
        return false;
    }
    let response = response.interact(egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.hovered() {
        ui.painter().rect_stroke(response.rect, 12, egui::Stroke::new(2.0_f32, ui.visuals().selection.bg_fill), egui::StrokeKind::Inside);
    }
    response.clicked()
}
/// A small copy of a layer's latest frame that fits in `w`×`h`, for previews.
fn preview_picture(frame: &rosorah_capture::Frame, w: u32, h: u32) -> Option<egui::ColorImage> {
    let (fw, fh) = (frame.width as usize, frame.height as usize);
    if fw == 0 || fh == 0 || frame.rgba.len() < fw * fh * 4 {
        return None;
    }
    let scale = (w as f32 / fw as f32).min(h as f32 / fh as f32).min(1.);
    let (tw, th) = (((fw as f32 * scale) as usize).max(1), ((fh as f32 * scale) as usize).max(1));
    let mut pixels = Vec::with_capacity(tw * th * 4);
    for y in 0..th {
        let row = (y * fh / th) * fw;
        for x in 0..tw {
            let i = (row + x * fw / tw) * 4;
            pixels.extend_from_slice(&frame.rgba[i..i + 4]);
        }
    }
    Some(egui::ColorImage::from_rgba_unmultiplied([tw, th], &pixels))
}
/// Guide and checklist windows stay solid even with see-through panels, so they're easy to read.
fn solid_window(ctx: &egui::Context) -> egui::Frame {
    let [r, g, b, _] = panel().to_srgba_unmultiplied();
    egui::Frame::window(&ctx.style()).fill(Color32::from_rgb(r, g, b))
}
/// A window that should stand out over the studio: the panel colour, a deep shadow and an
/// accent edge.
fn prominent_window(ctx: &egui::Context, accent: Color32) -> egui::Frame {
    solid_window(ctx)
        .stroke(egui::Stroke::new(1.5_f32, accent.gamma_multiply(0.75)))
        .shadow(egui::Shadow { offset: [0, 12], blur: 48, spread: 6, color: Color32::from_black_alpha(150) })
        .inner_margin(egui::Margin::same(14))
}
/// Every pop-out window, in the Layer Properties style: the prominent frame in the accent colour,
/// opened in the middle of the screen and free to move.
pub(crate) fn pop_window<'a>(ctx: &egui::Context, title: impl Into<egui::WidgetText>) -> egui::Window<'a> {
    egui::Window::new(title)
        .frame(prominent_window(ctx, ctx.style().visuals.selection.bg_fill))
        .collapsible(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
}
/// Every dialog: the same frame, in front of a greyed-out studio.
pub(crate) fn pop_modal(ctx: &egui::Context, id: egui::Id) -> egui::Modal {
    egui::Modal::new(id).frame(prominent_window(ctx, ctx.style().visuals.selection.bg_fill)).backdrop_color(DIM)
}
/// Behind a question that needs an answer, the studio is greyed out this much.
const DIM: Color32 = Color32::from_black_alpha(175);
/// Selects all of a text field's text, so the next keystroke replaces it.
fn select_all(ctx: &egui::Context, id: egui::Id, text: &str) {
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(text.chars().count()));
    state.cursor.set_char_range(Some(all));
    state.store(ctx, id);
}
/// Which list Delete and F2 act on: the last one clicked (the canvas counts as Layers).
#[derive(Clone, Copy, Default, PartialEq)]
enum ListFocus {
    #[default]
    Layers,
    Scenes,
}
/// The tabs of Layer Properties.
#[derive(Clone, Copy, Default, PartialEq)]
enum PropertiesTab {
    #[default]
    Source,
    Position,
    Appearance,
    Effects,
}
/// Tells Studio Mode's preview drawing apart from the live canvas.
const STUDIO_PREVIEW: Id = 0x5747_0000_0000_0000;
/// What a file chooser's pick becomes.
#[derive(Clone, Copy, PartialEq)]
enum MediaPick {
    File,
    Slideshow,
    Playlist,
}
/// What a sound strip's right-click menu asked for.
enum SoundAction {
    Rename(String),
    Hide,
    Delete(String),
}
/// Text that reads on the accent colour: black, or white when the accent is (nearly) black.
fn on_accent(accent: Color32) -> Color32 {
    if let Some(ink) = crate::appearance::current().on_accent {
        return ink;
    }
    let brightness = (0.2126 * accent.r() as f32 + 0.7152 * accent.g() as f32 + 0.0722 * accent.b() as f32) / 255.;
    if brightness < 0.18 { Color32::WHITE } else { Color32::BLACK }
}
/// The cursor over something that can be dragged. Windows has no open-hand cursor (it would show
/// the four-arrow "move" cross), so it gets the pointing hand there.
pub(crate) fn grab_cursor() -> egui::CursorIcon {
    if cfg!(windows) { egui::CursorIcon::PointingHand } else { egui::CursorIcon::Grab }
}
/// The cursor while dragging; the pointing hand on Windows for the same reason.
pub(crate) fn grabbing_cursor() -> egui::CursorIcon {
    if cfg!(windows) { egui::CursorIcon::PointingHand } else { egui::CursorIcon::Grabbing }
}
/// Drag-to-reorder for lists laid out normally by egui, with items of any size, down or across.
/// While an item is held the others slide aside to show where it will land and the held one
/// follows the pointer. Only painting moves (the layout stays put), so it costs almost nothing.
pub(crate) struct ShiftDrag {
    key: egui::Id,
    horizontal: bool,
    animate: bool,
    /// The held item and where the pointer was when it was picked up.
    held: Option<(usize, egui::Pos2)>,
    target: Option<usize>,
    previous: Vec<egui::Rect>,
    rects: Vec<egui::Rect>,
    next: Option<(usize, egui::Pos2)>,
}
impl ShiftDrag {
    pub(crate) fn begin(ui: &egui::Ui, key: egui::Id, horizontal: bool, animate: bool, count: usize) -> Self {
        let held = ui.data(|d| d.get_temp::<(usize, egui::Pos2)>(key)).filter(|(from, _)| *from < count);
        let previous: Vec<egui::Rect> = ui.data(|d| d.get_temp(key.with("rects"))).unwrap_or_default();
        let along = |p: egui::Pos2| if horizontal { p.x } else { p.y };
        // Where the held item would land: past every other item whose middle the pointer has passed.
        let target = held.zip(ui.ctx().pointer_interact_pos()).filter(|_| previous.len() == count).map(|((from, start), pointer)| {
            let centre = along(previous[from].center()) + along(pointer) - along(start);
            let mut to = 0;
            for (i, r) in previous.iter().enumerate() {
                if i != from && centre > along(r.center()) {
                    to += 1;
                }
            }
            to
        });
        Self { key, horizontal, animate, held, target, previous, rects: Vec::with_capacity(count), next: held }
    }
    /// How far item `p` is drawn from its place in the layout.
    fn offset(&self, ui: &egui::Ui, p: usize) -> egui::Vec2 {
        let axis = |v: f32| if self.horizontal { egui::vec2(v, 0.) } else { egui::vec2(0., v) };
        if let Some((from, start)) = self.held
            && from == p
            && let Some(pointer) = ui.ctx().pointer_interact_pos()
        {
            return axis(if self.horizontal { pointer.x - start.x } else { pointer.y - start.y });
        }
        let shift = match (self.held, self.target) {
            (Some((from, _)), Some(to)) if self.previous.len() > from.max(p) => {
                // The gap the held item leaves: its size plus the space to its neighbour.
                let size = |r: egui::Rect| if self.horizontal { r.width() } else { r.height() };
                let gap = self.previous.get(from + 1).or(from.checked_sub(1).and_then(|i| self.previous.get(i))).map_or(0., |n| {
                    let r = self.previous[from];
                    if self.horizontal { (n.left() - r.right()).max(r.left() - n.right()).max(0.) } else { (n.top() - r.bottom()).max(r.top() - n.bottom()).max(0.) }
                });
                let step = size(self.previous[from]) + gap;
                if from < p && p <= to { -step } else if to <= p && p < from { step } else { 0. }
            }
            _ => 0.,
        };
        let duration = if self.animate { 0.12 } else { 0. };
        axis(ui.ctx().animate_value_with_time(self.key.with(("shift", p)), shift, duration))
    }
    /// Draws item `p`; `add` returns the response that picks the item up when dragged.
    pub(crate) fn item(&mut self, ui: &mut egui::Ui, p: usize, add: impl FnOnce(&mut egui::Ui) -> egui::Response) {
        let offset = self.offset(ui, p);
        let held = self.held.is_some_and(|(from, _)| from == p);
        let inner = ui.with_visual_transform(egui::emath::TSTransform::from_translation(offset), |ui| {
            if held {
                ui.multiply_opacity(0.85);
            }
            add(ui)
        });
        self.rects.push(inner.response.rect);
        let handle = inner.inner;
        if handle.drag_started()
            && let Some(pointer) = ui.ctx().pointer_interact_pos()
        {
            self.next = Some((p, pointer));
        }
        if held {
            ui.ctx().set_cursor_icon(grabbing_cursor());
        }
    }
    /// Returns (from, to) when the held item is let go somewhere new.
    pub(crate) fn finish(self, ui: &egui::Ui) -> Option<(usize, usize)> {
        let mut next = self.next;
        let mut dropped = None;
        if self.held.is_some() && !ui.input(|i| i.pointer.any_down()) {
            dropped = self.held.zip(self.target).map(|((from, _), to)| (from, to)).filter(|(from, to)| from != to);
            next = None;
        }
        let key = self.key;
        ui.data_mut(|d| {
            match next {
                Some(h) => d.insert_temp(key, h),
                None => d.remove::<(usize, egui::Pos2)>(key),
            }
            d.insert_temp(key.with("rects"), self.rects);
        });
        if next.is_some() {
            ui.ctx().request_repaint();
        }
        if dropped.is_some() {
            // The list is renumbered, so nothing should slide back from an old shift.
            for p in 0..self.previous.len() {
                ui.ctx().animate_value_with_time(key.with(("shift", p)), 0., 0.);
            }
        }
        dropped
    }
}
/// A list whose rows can be picked up and moved: the held row follows the pointer and the
/// others slide out of its way. `row` draws one row (by position, top first) and returns the
/// response that starts a drag. Returns `(from, to)` positions when a row is dropped somewhere new.
fn sortable_rows(
    ui: &mut egui::Ui,
    key: egui::Id,
    count: usize,
    stride: f32,
    animate: bool,
    mut row: impl FnMut(&mut egui::Ui, usize) -> egui::Response,
) -> Option<(usize, usize)> {
    let start = ui.cursor().min;
    let width = ui.available_width();
    let held: Option<(usize, f32)> = ui.data(|d| d.get_temp(key)).filter(|(from, _): &(usize, f32)| *from < count);
    let snap = ui.data(|d| d.get_temp::<bool>(key.with("snap"))).unwrap_or(false);
    let pointer = ui.ctx().pointer_interact_pos();
    let last = count.saturating_sub(1) as f32;
    let target = held.zip(pointer).map(|((from, grab), p)| (from, ((p.y - grab - start.y) / stride).round().clamp(0., last) as usize));
    let mut next = held;
    let mut dropped = None;
    // The held row is drawn last so it stays on top of the rows sliding under it.
    let order: Vec<usize> = (0..count).filter(|p| Some(*p) != held.map(|h| h.0)).chain(held.map(|h| h.0)).collect();
    for p in order {
        let y = match (held, target, pointer) {
            (Some((from, grab)), _, Some(pointer)) if from == p => (pointer.y - grab).clamp(start.y, start.y + last * stride),
            _ => {
                let slot = match target {
                    Some((from, to)) if from < to && p > from && p <= to => p - 1,
                    Some((from, to)) if from > to && p >= to && p < from => p + 1,
                    _ => p,
                };
                // Only the place in the list animates; the panel's own position is added afterwards,
                // so rows move exactly with the panel when it's resized or moved.
                let duration = if animate && !snap { 0.12 } else { 0. };
                start.y + ui.ctx().animate_value_with_time(key.with(("slot", p)), slot as f32, duration) * stride
            }
        };
        let rect = egui::Rect::from_min_size(egui::pos2(start.x, y), egui::vec2(width, stride - 3.));
        let child = &mut ui.new_child(egui::UiBuilder::new().max_rect(rect).id_salt(key.with(p)));
        if held.is_some_and(|h| h.0 == p) {
            child.painter().rect_filled(rect.expand(2.), 8., child.visuals().extreme_bg_color);
        }
        let response = row(child, p);
        if response.drag_started()
            && let Some(pointer) = pointer
        {
            next = Some((p, pointer.y - rect.top()));
        }
        if held.is_some_and(|h| h.0 == p) {
            ui.ctx().set_cursor_icon(grabbing_cursor());
        }
    }
    if held.is_some() && !ui.input(|i| i.pointer.any_down()) {
        if let Some((from, to)) = target.filter(|(from, to)| from != to) {
            dropped = Some((from, to));
        }
        next = None;
    }
    ui.allocate_space(egui::vec2(width, count as f32 * stride));
    ui.data_mut(|d| {
        match next {
            Some(h) => d.insert_temp(key, h),
            None => d.remove::<(usize, f32)>(key),
        }
        // After a drop the rows are renumbered, so they jump straight to their new places once.
        d.insert_temp(key.with("snap"), dropped.is_some());
    });
    if next.is_some() {
        ui.ctx().request_repaint();
    }
    dropped
}
fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(panel())
        .corner_radius(crate::appearance::current().panel_corners)
        .inner_margin(12)
}
fn note(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(RichText::new(text).small().color(muted()));
}
#[derive(Clone, Copy, PartialEq)]
enum Page {
    Studio,
    Alerts,
    Settings,
}
#[derive(Clone, Copy, PartialEq)]
enum Settings {
    Appearance,
    /// Accounts has two tabs: streaming platforms (Accounts) and Donations.
    Accounts,
    Donations,
    /// "I'm live" posts to Discord and Bluesky; the third Accounts tab.
    Announcements,
    Video,
    Audio,
    Hotkeys,
    Remote,
    Scripts,
    /// The advanced-options switch, backups, profiles and imports.
    Advanced,
}
/// What a first-time user plans to do; picks the starter scenes.
#[derive(Clone, Copy, PartialEq)]
enum Starter {
    Games,
    Chatting,
    Recording,
}
/// A short notice above the footer, optionally offering Undo.
struct Toast {
    text: String,
    undo: bool,
    until: Instant,
}
use crate::system_monitor::Resources;
pub struct Studio {
    widget_editor: WidgetEditor,
    tools:crate::utilities::Tools,
    selections: std::collections::BTreeMap<Id,Vec<usize>>,
    crop_layer:Option<(Id,usize)>,
    pending_layer_delete:Option<(Id,usize)>,
    layer_rename:Option<(Id,String)>,
    layer_properties:Option<(Id,usize)>,
    /// TikTok is selected in the chat composer's "Send to" row (when sending is enabled).
    tiktok_chat: bool,
    /// Co-streaming: the shared chat last announced, and the invite under Chat (kept while it slides away).
    shared_chat_seen: Vec<(String, String)>,
    invite_shown: Option<crate::browser_chat::Invite>,
    /// When the invite card finished sliding in, for its one yellow flash.
    invite_flash: Option<f64>,
    signin_hinted: bool,
    invite_told: String,
    /// Chat panel message heights for the layout (width, text size) they were measured at.
    chat_heights: ((i32, u32), std::collections::HashMap<(Platform, String), f32>),
    tiktok_diagnostic_wanted: bool,
    appearance: crate::appearance::Renderer,
    appearance_picker: Option<mpsc::Receiver<Option<PathBuf>>>,
    /// The file chosen with Settings > Look & feel > Import theme…
    theme_import: Option<mpsc::Receiver<Option<PathBuf>>>,
    scene_transition: Option<ActiveTransition>,
    transition_picker: Option<(bool, mpsc::Receiver<Option<PathBuf>>)>,
    edit_history: crate::edit_history::History,
    continuous_edit: bool,
    readiness_open: bool,
    /// The Go live checklist is open; starting the stream happens from there.
    go_live_open: bool,
    /// When the microphone was last heard, for the Go live checklist.
    mic_heard: Option<Instant>,
    report_open: bool,
    /// The stream title being typed in the Stream panel.
    title_draft: String,
    /// No saved project existed at start, so the setup guide may replace the demo scenes.
    first_run: bool,
    toast: Option<Toast>,
    /// Incomplete recordings being repaired automatically at start.
    recovering: Option<mpsc::Receiver<Vec<Result<PathBuf, String>>>>,
    recovery_candidate: Option<Project>,
    recovery_write: Option<mpsc::Receiver<Result<(), String>>>,
    last_recovery: Instant,
    audio: crate::audio::Audio,
    captures: crate::capture::Captures,
    camera_devices: Vec<crate::camera::Device>,
    browser_drafts: std::collections::BTreeMap<Id, (String, auroracast_core::browser::Settings)>,
    camera_drafts: std::collections::BTreeMap<Id, auroracast_core::CameraSettings>,
    workspace: Workspace,
    scene_dialog: Option<(Id, String, bool)>,
    /// The sound whose settings window is open.
    sound_settings: Option<Id>,
    /// The layer whose menu was opened by right-clicking it on the canvas.
    canvas_menu: Option<(Id, usize)>,
    /// Open projector windows.
    projectors: Vec<Projector>,
    /// The remote-control server while it runs, what it was started with, and what controllers last heard.
    remote: Option<crate::remote::Server>,
    remote_settings: RemoteRunning,
    remote_error: String,
    remote_attempt: Instant,
    remote_seen: RemoteState,
    remote_chat_seen: u64,
    /// Lua scripts running, the list they were loaded from, and events waiting for them.
    scripts: crate::scripts::Scripts,
    scripts_loaded: Vec<String>,
    pending_events: Vec<serde_json::Value>,
    /// The virtual camera's output while it's on, and ones still finishing.
    virtual_camera: Option<crate::recording::Recording>,
    /// Rosorah's main video sent out as NDI (network) and Spout (this PC, Windows).
    ndi_output: Option<crate::recording::Recording>,
    spout_output: Option<crate::recording::Recording>,
    /// NDI (true) and Spout (false) senders last seen, refreshed every 2 s while picking one.
    shared_senders: std::collections::BTreeMap<bool, (Instant, Result<Vec<String>, String>)>,
    /// Windows: the camera as Windows shows it to apps, while it's on.
    #[cfg(windows)]
    windows_camera: Option<rosorah_windows::virtual_camera::Camera>,
    camera_stopping: Vec<crate::recording::Recording>,
    /// Replay buffer: on or off, its own encoder when not live, and a save in progress.
    replay_on: bool,
    replay_stream: Option<crate::streaming::Stream>,
    replay_stopping: Vec<crate::streaming::Stream>,
    replay_saving: Option<mpsc::Receiver<anyhow::Result<PathBuf>>>,
    replay_retry: Instant,
    /// Hotkeys registered with the system, the list they were made from, and when it last changed.
    global_hotkeys: Option<crate::hotkeys::Global>,
    hotkeys_bound: Vec<auroracast_core::Hotkey>,
    hotkeys_changed: Option<Instant>,
    /// The action waiting for its key combination on the Hotkeys page.
    hotkey_capture: Option<auroracast_core::HotkeyAction>,
    /// Studio Mode: the scene being prepared, and the Program view's picture.
    studio_preview: Option<Id>,
    studio_taking: bool,
    program_texture: Option<egui::TextureId>,
    program_drawn: Instant,
    /// Layers copied for pasting into any scene.
    layer_clipboard: Vec<CopiedLayer>,
    /// The scene Ctrl+C copied in the Scenes panel.
    scene_clipboard: Option<Id>,
    /// A folder chooser re-pointing a slideshow.
    slideshow_folder: Option<(Id, mpsc::Receiver<Option<PathBuf>>)>,
    /// A file chooser for a layer's look-up table (true) or mask (false).
    look_picker: Option<(Id, usize, bool, mpsc::Receiver<Option<PathBuf>>)>,
    /// A local overlay file being chosen for a browser source.
    browser_picker: Option<(Id, mpsc::Receiver<Option<PathBuf>>)>,
    /// A sound strip being renamed, and the name typed so far.
    sound_rename: Option<(Id, String)>,
    web_docks: crate::browser_dock::Docks,
    announcer: crate::announce::Announcer,
    /// A file chooser open for the announcement image.
    announce_picker: Option<mpsc::Receiver<Option<PathBuf>>>,
    /// The announcement image's thumbnail, and the file it was made from.
    announce_thumb: Option<(String, Option<egui::TextureHandle>)>,
    /// Whether this stream has been announced yet.
    announced: bool,
    /// This stream's YouTube broadcast has been made public (or didn't need to be).
    made_public: bool,
    /// Announcements switched off in the Go live checklist, for this stream only ([Discord, Bluesky]).
    announce_skip: [bool; 4],
    updates: crate::updates::Updates,
    update_window: bool,
    update_checked: bool,
    /// When updates were last looked for; long sessions look again every few hours.
    last_update_check: Instant,
    /// Help > Check for updates: says so when there's nothing new.
    update_asked: bool,
    /// Start the new version once this one has closed (after an update installed).
    restart_after_update: bool,
    /// The scene being named inside its own pill: its id, the text so far, and whether the
    /// box has been focused yet.
    naming_scene: Option<(Id, String, bool)>,
    /// Which list Delete and F2 act on.
    list_focus: ListFocus,
    /// Layers waiting for "Delete?" to be answered: the scene and the layers' places in it.
    delete_confirm: Option<(Id, Vec<usize>)>,
    /// How each "Same in every scene" layer was left after the last edit, per video and source.
    every_scene_last: std::collections::BTreeMap<(Id, Id), Layer>,
    /// Where each canvas was drawn last frame (its scene), so a click elsewhere unselects it.
    canvas_hits: Vec<(Id, egui::Rect)>,
    /// Places a click doesn't unselect from: Layers, Layer Properties, the widget studio.
    keep_selection: Vec<egui::Rect>,
    /// Where the Scenes panel was drawn last frame.
    scenes_area: Option<egui::Rect>,
    properties_tab: PropertiesTab,
    /// Name and address being entered for a new web page panel.
    web_dock_dialog: Option<(String, String)>,
    /// The scene the Add dialog adds to, and which page of it shows.
    add_dialog: Option<(Id, AddView)>,
    add_search: String,
    /// Small pictures for the Add dialog's Use again cards, and when each was made.
    reuse_previews: std::collections::BTreeMap<Id, (egui::TextureHandle, std::time::Instant)>,
    broadcast_window: u8,
    panels_animating: bool,
    toggle_panel: Option<Panel>,
    project: Project,
    project_path: PathBuf,
    page: Page,
    settings: Settings,
    canvas: usize,
    layer: Option<usize>,
    canvas_drag: Option<crate::canvas_edit::Drag>,
    keep_aspect: bool,
    camera_aspect: crate::camera_layout::AutoAspect,
    dirty: bool,
    revision: bool,
    source_uploads: crate::media::Sources,
    gpu: Option<Compositor>,
    gpu_error: Option<String>,
    preview: Option<egui::TextureId>,
    render_state: Option<eframe::egui_wgpu::RenderState>,
    preview_size: [u32; 2],
    animate: bool,
    frame: u32,
    last_preview: Instant,
    resources: Resources,
    last_stats: Instant,
    chat: ChatFeed,
    event_history: crate::activity::History,
    accounts: [crate::accounts::Account; 2],
    kick_account: crate::accounts::Account,
    /// Bot accounts: Twitch, then Kick.
    bots: [crate::accounts::Account; 2],
    /// Timers: when each last ran, and the chat message count then.
    timer_runs: std::collections::HashMap<Id, (Instant, u64)>,
    /// Chat messages seen (for timers that wait for chat activity).
    chat_messages_seen: u64,
    account_config: crate::accounts::Config,
    browser_chat: crate::browser_chat::BrowserChat,
    emote_picker: crate::emote_picker::Picker,
    /// Rosorah draws the title bar itself (GNOME); the window frame is off.
    custom_title_bar: bool,
    audio_scene: Option<Id>,
    events_only: bool,
    thumbnail_picker: Option<mpsc::Receiver<Option<PathBuf>>>,
    thumbnail_preview: Option<egui::TextureHandle>,
    thumbnail_preview_path: String,
    vertical_texture: Option<egui::TextureId>,
    vertical_size: [u32; 2],
    vertical_last: Instant,
    vertical_revision: bool,
    preview_canvases: [Option<Id>; 2],
    filter: Option<Platform>,
    alerts: AlertEngine,
    automation_editor: crate::automations::Editor,
    speech: crate::speech::Reader,
    rewards_ui:crate::rewards_ui::RewardsUi,
    automation_queue: crate::automations::Scheduler,
    chat_session: alert_filters::ChatSession,
    was_streaming: bool,
    donation_save_due: Option<Instant>,
    donations: crate::donations_live::Hub,
    kofi: crate::kofi::Hub,
    event_user: String,
    event_text: String,
    active_alert: Option<(String, Instant)>,
    activity: VecDeque<String>,
    message: String,
    /// A file or folder chooser that is open, the scene the pick is added to, and what it becomes.
    media_picker: Option<(Id, MediaPick, mpsc::Receiver<Option<PathBuf>>)>,
    video_aspects: std::collections::BTreeMap<Id, [u32; 2]>,
    recorder: Option<mpsc::Receiver<Result<PathBuf, String>>>,
    live_recording: Option<crate::recording::Recording>,
    live_stream: Option<crate::streaming::Stream>,
    stream_config: crate::streaming::Config,
    stream_error: String,
    stream_details: String,
    stream_saved: String,
    show_stream_keys: bool,
    /// Profile to start after this window closes.
    restart_profile: Option<String>,
    profile_name: String,
    profile_copy: bool,
    profile_rename: Option<(String, String)>,
    profile_delete: Option<String>,
    profile_status: String,
    last_backup: Instant,
    /// The project changed since the last hourly backup.
    backup_due: bool,
    restore_confirm: Option<PathBuf>,
    backup_status: String,
    pop_out_panel: Option<Panel>,
    /// The tab being dragged, so a drop outside the main window can give it its own window.
    dragging_panel: Option<Panel>,
    /// A floating panel window being dragged, with where it sat when the drag began.
    dragging_window: Option<(Vec<Panel>, egui::Rect)>,
    last_recording: Option<PathBuf>,
    recording_error: String,
    recording_details: String,
    closing: bool,
    record_folder: String,
    screenshot: Option<PathBuf>,
    screenshot_requested: bool,
    /// When the studio opened; screenshots wait a moment so pages and fades can settle.
    started: Instant,
    last_slideshow_check: Instant,
    /// Where the window is and how big, saved shortly after it changes and on exit.
    window_placement: crate::window_memory::Placement,
    window_changed: Option<Instant>,
    loading: Option<Loading>,
    ui_frames: u32,
    /// Sample content for screenshots: the sample alert box.
    demo: Option<Id>,
    /// `--page demo-add`: opens the Add dialog for a new scene.
    demo_add: bool,
    /// `--page demo-drag`: drags the Sources tab partway across the window for screenshots,
    /// with where its tab sits once drawn.
    demo_drag: Option<Option<egui::Rect>>,
    /// `--page press-delete`, `press-f2` or `click-away`: that input once the studio is drawn.
    demo_input: Option<&'static str>,
}
impl Studio {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        mut project: Project,
        project_path: PathBuf,
        screenshot: Option<PathBuf>,
        page: String,
    ) -> Self {
        // `--page demo` (or demo-alerts, demo-activity…) fills the studio with sample content.
        let (demo, page) = match page.strip_prefix("demo") {
            Some(rest) => (true, Some(rest.trim_start_matches('-')).filter(|p| !p.is_empty()).unwrap_or("studio").to_string()),
            None => (false, page),
        };
        let demo_add = demo && page == "add";
        let demo_drag = (demo && page == "drag").then_some(None);
        let demo = demo.then(|| crate::demo::project(&mut project));
        project.prepare_workspace();
        project.merge_alert_boxes();
        project.drop_hidden_alert_layers();
        let mut every_scene_last = Default::default();
        project.sync_every_scene(&mut every_scene_last);
        if project.preferences.accent == [141, 101, 255] {
            project.preferences.accent = [208, 188, 255];
        }
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "Roboto".into(),
            egui::FontData::from_static(include_bytes!("../assets/fonts/Roboto.ttf")).into(),
        );
        fonts
            .families
            .get_mut(&egui::FontFamily::Proportional)
            .unwrap()
            .insert(0, "Roboto".into());
        // Fallbacks for symbols the other fonts lack (⋯ ◌ ⋮ ⬤ and more).
        for (name, data) in [
            ("Noto Sans Symbols 2", &include_bytes!("../assets/fonts/NotoSansSymbols2-Regular.ttf")[..]),
            ("Noto Sans Math", &include_bytes!("../assets/fonts/NotoSansMath-Regular.ttf")[..]),
        ] {
            fonts.font_data.insert(name.into(), egui::FontData::from_static(data).into());
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts.families.get_mut(&family).unwrap().push(name.into());
            }
        }
        cc.egui_ctx.set_fonts(fonts);
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = bg();
        style.visuals.window_fill = panel();
        style.visuals.override_text_color = None;
        style.visuals.extreme_bg_color = bg();
        style.visuals.window_corner_radius = egui::CornerRadius::same(16);
        style.visuals.window_stroke = egui::Stroke::NONE;
        style.visuals.selection.bg_fill = Color32::from_rgb(208, 188, 255);
        style.visuals.selection.stroke = egui::Stroke::new(1.0_f32, Color32::from_rgb(56, 30, 114));
        style.spacing.item_spacing = Vec2::new(8., 8.);
        style.spacing.button_padding = Vec2::new(11., 7.);
        for widget in [
            &mut style.visuals.widgets.inactive,
            &mut style.visuals.widgets.hovered,
            &mut style.visuals.widgets.active,
        ] {
            widget.bg_fill = Color32::from_rgb(44, 43, 53);
            widget.weak_bg_fill = widget.bg_fill;
            widget.bg_stroke = egui::Stroke::NONE;
            widget.fg_stroke = egui::Stroke::new(1.0_f32, Color32::from_rgb(238, 235, 245));
            widget.corner_radius = egui::CornerRadius::same(20);
        }
        style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(61, 57, 73);
        style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(61, 57, 73);
        style
            .text_styles
            .insert(egui::TextStyle::Small, egui::FontId::proportional(12.));
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(14.));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(14.));
        style
            .text_styles
            .insert(egui::TextStyle::Heading, egui::FontId::proportional(20.));
        cc.egui_ctx.set_style(style);
        let render_state = cc.wgpu_render_state.clone();
        let (gpu, gpu_error) = match &render_state {
            Some(state) => (
                Some(Compositor::from_device(
                    state.device.clone(),
                    state.queue.clone(),
                    state.adapter.get_info(),
                )),
                None,
            ),
            None => (None, Some("GPU canvas is unavailable".into())),
        };
        let default_record_folder = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Videos/Rosorah Studio")
            .display()
            .to_string();
        let record_folder = if project.preferences.recording_directory.is_empty() {
            default_record_folder
        } else {
            project.preferences.recording_directory.clone()
        };
        let mut workspace = Workspace::load(
            project_path.with_extension("layout.json"),
            egui::Frame::window(&cc.egui_ctx.style())
                .total_margin()
                .left_top(),
        );
        workspace.retain_web(&project.preferences.workspace.browser_docks.iter().map(|d| d.id).collect::<Vec<_>>());
        // `--page web-dock` shows the first web page panel for screenshots.
        if page == "web-dock" && let Some(dock) = project.preferences.workspace.browser_docks.first() {
            workspace.set_visible(Panel::Web(dock.id), true);
        }
        if page == "activity" {
            workspace.set_visible(Panel::Activity, true);
            if let Some((surface, node, tab)) = workspace.dock.find_tab(&Panel::Activity) {
                workspace.dock[surface][node]
                    .get_leaf_mut()
                    .unwrap()
                    .set_active_tab(tab);
            }
        }
        let (mut stream_config, stream_error) =
            match crate::streaming::Config::load_with_keyring(&crate::streaming::Config::path()) {
                Ok(config) => (config, String::new()),
                Err(e) => (crate::streaming::Config::default(), format!("{e:#}")),
            };
        // `--page twitch-vod`: the Twitch connection with a VOD track chosen, for screenshots.
        if page == "twitch-vod" {
            stream_config.twitch_vod_track = Some(1);
        }
        let added_capture = ensure_automatic_source(&mut project);
        let mut captures = crate::capture::Captures::default();
        captures.set_repaint(cc.egui_ctx.clone());
        captures.restore(&project);
        let mut audio = crate::audio::Audio::default();
        audio.follow_capture(captures.reader());
        for (id, input) in &project.preferences.audio_inputs {
            audio.selected.insert(*id, input.clone());
            audio.enabled.insert(*id, true);
        }
        let selected = project
            .canvases
            .first()
            .and_then(|c| project.scenes.iter().find(|s| s.id == c.scene))
            .and_then(|s| {
                s.layers.iter().position(|l| {
                    project.source(l.source).is_some_and(|s| {
                        matches!(
                            s.kind,
                            SourceKind::ScreenCapture | SourceKind::VideoCamera(_)
                        )
                    })
                })
            });
        crate::desktop::refresh();
        let account_config = crate::accounts::Config::load();
        let mut accounts = [
            crate::accounts::Account::new(Platform::Twitch),
            crate::accounts::Account::new(Platform::YouTube),
        ];
        accounts[0].restore(account_config.twitch.clone());
        accounts[1].restore(account_config.youtube.clone());
        let mut kick_account = crate::accounts::Account::new(Platform::Kick);
        kick_account.restore(account_config.kick.clone());
        let mut bots = [crate::accounts::Account::new_bot(Platform::Twitch), crate::accounts::Account::new_bot(Platform::Kick)];
        bots[0].restore(account_config.twitch.clone());
        bots[1].restore(account_config.kick.clone());
        let first_run = !project_path.exists() && demo.is_none();
        let tools=crate::utilities::Tools{guide:first_run.then_some(0),..Default::default()};
        // Recordings cut short by a crash are repaired in the background; the original files are kept.
        let unfinished = crate::utilities::incomplete(std::path::Path::new(&record_folder));
        let recovering = (!unfinished.is_empty()).then(|| {
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let results = unfinished
                    .into_iter()
                    .map(|path| match crate::utilities::remux(path, false) {
                        Ok(crate::utilities::Finished::Saved(saved)) => Ok(saved),
                        Ok(_) => Err("Unexpected recovery result".into()),
                        Err(e) => Err(format!("{e:#}")),
                    })
                    .collect();
                let _ = tx.send(results);
            });
            rx
        });
        // Older projects named their videos "Canvas 1" and "Canvas 2".
        for (canvas, name) in project.canvases.iter_mut().zip(["Main video", "Vertical video"]) {
            if canvas.name == "Canvas 1" || canvas.name == "Canvas 2" {
                canvas.name = name.into();
            }
        }
        let edit_history = crate::edit_history::History::new(&project);
        let recovery_path = crate::edit_history::recovery_path(&project_path);
        let newer = std::fs::metadata(&recovery_path)
            .and_then(|m| m.modified())
            .ok()
            .zip(
                std::fs::metadata(&project_path)
                    .and_then(|m| m.modified())
                    .ok(),
            )
            .is_some_and(|(a, b)| a > b);
        let recovery_candidate = if newer {
            Project::load(&recovery_path).ok()
        } else {
            None
        };
        // `--page delete-scene` opens the delete question for screenshots.
        let scene_dialog = (page == "delete-scene")
            .then(|| project.canvases.first().and_then(|c| project.scene(c.scene)).map(|s| (s.id, s.name.clone(), true)))
            .flatten();
        let scene_for_naming = project.canvases.first().map(|c| c.scene).unwrap_or_default();
        let name_for_renaming = project.scene(scene_for_naming).map(|s| s.name.clone()).unwrap_or_default();
        // `--page sound-settings` opens the microphone's settings for screenshots.
        // (`--page music-settings` opens the Music strip's instead.)
        let wanted = match page.as_str() { "sound-settings" => Some("mic"), "music-settings" => Some("music"), _ => None };
        let sound_settings = wanted.and_then(|w| project.audio.iter().find(|c| c.name.to_lowercase().contains(w)).map(|c| c.id));
        let mut widget_editor = WidgetEditor::default();
        if page == "widgets" {
            widget_editor.id = project
                .sources
                .iter()
                .find(|s| matches!(s.kind, SourceKind::Widget(_)))
                .map(|s| s.id);
            widget_editor.sample = true;
        }
        Self {
            tools,
            selections:project.canvases.first().and_then(|c|selected.map(|i|(c.scene,vec![i]))).into_iter().collect(),crop_layer:None,pending_layer_delete:None,layer_rename:None,layer_properties:(page=="properties").then_some((scene_for_naming,0)),tiktok_chat:true,shared_chat_seen:Vec::new(),invite_shown:None,invite_flash:None,signin_hinted:false,invite_told:String::new(),chat_heights:Default::default(),tiktok_diagnostic_wanted:false,appearance:Default::default(),appearance_picker:None,theme_import:None,scene_transition:None,transition_picker:None,
            widget_editor,
            edit_history,
            continuous_edit: false,
            readiness_open: false,
            go_live_open: false,
            mic_heard: None,
            report_open: false,
            title_draft: String::new(),
            first_run,
            toast: None,
            recovering,
            recovery_candidate,
            recovery_write: None,
            last_recovery: Instant::now(),
            captures,
            camera_devices: crate::camera::devices(),
            browser_drafts: Default::default(),
            camera_drafts: Default::default(),
            audio,
            workspace,
            toggle_panel: None,
            custom_title_bar: crate::title_bar::wanted(project.preferences.workspace.appearance.title_bar),
            project,
            project_path,
            page: match page.as_str() {
                "settings" | "streaming" | "accounts" | "donations" | "announcements" | "hotkeys" | "advanced" | "audio-tracks" | "twitch-vod" => Page::Settings,
                "automation-preview" | "alerts" | "alert-guide" | "alert-guide-media" | "alert-guide-finish" => Page::Alerts,
                _ => Page::Studio,
            },
            settings: if page == "donations" {
                Settings::Donations
            } else if page == "announcements" {
                Settings::Announcements
            } else if page == "hotkeys" {
                Settings::Hotkeys
            } else if page == "accounts" || page == "streaming" {
                Settings::Accounts
            } else if page == "advanced" {
                Settings::Advanced
            } else if page == "audio-tracks" {
                Settings::Audio
            } else if page == "twitch-vod" {
                Settings::Accounts
            } else {
                Settings::Appearance
            },
            canvas: 0,
            layer: selected,
            canvas_drag: None,
            keep_aspect: true,
            camera_aspect: Default::default(),
            dirty: added_capture,
            revision: true,
            source_uploads: Default::default(),
            gpu,
            gpu_error,
            preview: None,
            render_state,
            preview_size: [0, 0],
            animate: false,
            scene_dialog,
            sound_settings,
            sound_rename: None,
            canvas_menu: None,
            projectors: Vec::new(),
            remote: None,
            remote_settings: Default::default(),
            remote_error: String::new(),
            remote_attempt: Instant::now() - Duration::from_secs(60),
            remote_seen: Default::default(),
            remote_chat_seen: 0,
            scripts: Default::default(),
            scripts_loaded: Vec::new(),
            pending_events: Vec::new(),
            virtual_camera: None,
            ndi_output: None,
            spout_output: None,
            shared_senders: Default::default(),
            #[cfg(windows)]
            windows_camera: None,
            camera_stopping: Vec::new(),
            replay_on: false,
            replay_stream: None,
            replay_stopping: Vec::new(),
            replay_saving: None,
            replay_retry: Instant::now() - Duration::from_secs(60),
            global_hotkeys: None,
            hotkeys_bound: Vec::new(),
            hotkeys_changed: None,
            hotkey_capture: None,
            studio_preview: None,
            studio_taking: false,
            program_texture: None,
            program_drawn: Instant::now(),
            layer_clipboard: Vec::new(),
            scene_clipboard: None,
            look_picker: None,
            browser_picker: None,
            slideshow_folder: None,
            web_docks: Default::default(),
            announcer: Default::default(),
            announce_picker: None,
            announce_thumb: None,
            announced: false,
            made_public: false,
            announce_skip: [false; 4],
            updates: Default::default(),
            // `--page update` (with --screenshot) shows the update window, checking as an old version.
            update_window: page == "update",
            update_checked: false,
            last_update_check: Instant::now(),
            update_asked: false,
            restart_after_update: false,
            // `--page name-scene` opens the name box in the current scene's pill for screenshots.
            // `--page rename-scene`: renaming as F2 starts it, the old name selected.
            naming_scene: match page.as_str() {
                "name-scene" => Some((scene_for_naming, String::new(), false)),
                "rename-scene" => Some((scene_for_naming, name_for_renaming, false)),
                _ => None,
            },
            list_focus: if page == "copy-scene" { ListFocus::Scenes } else { ListFocus::default() },
            // `--page delete-layers` asks about the first two layers, for screenshots.
            delete_confirm: (page == "delete-layers").then(|| (scene_for_naming, vec![0, 1])),
            every_scene_last,
            canvas_hits: Vec::new(),
            keep_selection: Vec::new(),
            scenes_area: None,
            properties_tab: PropertiesTab::default(),
            web_dock_dialog: None,
            add_dialog: None,
            add_search: String::new(),
            reuse_previews: Default::default(),
            broadcast_window: match page.as_str() { "add-service" => 3, "twitch-vod" => 100, _ => 0 },
            panels_animating: false,
            frame: 0,
            last_preview: Instant::now(),
            resources: Resources::default(),
            last_stats: Instant::now() - Duration::from_secs(2),
            chat: ChatFeed::new(500),
            event_history: crate::activity::History::load(
                crate::streaming::Config::path().with_file_name("activity.json"),
            ),
            accounts,
            kick_account,
            bots,
            timer_runs: Default::default(),
            chat_messages_seen: 0,
            account_config,
            browser_chat: crate::browser_chat::BrowserChat::default(),
            emote_picker: Default::default(),
            audio_scene: None,
            events_only: false,
            thumbnail_picker: None,
            thumbnail_preview: None,
            thumbnail_preview_path: String::new(),
            vertical_texture: None,
            vertical_size: [0, 0],
            vertical_last: Instant::now(),
            vertical_revision: true,
            preview_canvases: [None, None],
            filter: None,
            alerts: AlertEngine::default(),
            automation_editor: crate::automations::Editor::for_page(&page),
            speech: Default::default(),
            rewards_ui:Default::default(),
            automation_queue: Default::default(),
            chat_session: {
                let mut session = alert_filters::ChatSession::default();
                session.start(now_ms());
                session
            },
            was_streaming: false,
            donation_save_due: None,
            donations: crate::donations_live::Hub::new(),
            kofi: crate::kofi::Hub::new(),
            event_user: "Moe".into(),
            event_text: "!hello".into(),
            active_alert: None,
            activity: VecDeque::new(),
            message: "Ready".into(),
            media_picker: None,
            video_aspects: Default::default(),
            recorder: None,
            live_recording: None,
            live_stream: None,
            stream_config,
            stream_error,
            stream_details: String::new(),
            stream_saved: String::new(),
            show_stream_keys: false,
            restart_profile: None,
            profile_name: String::new(),
            profile_copy: true,
            profile_rename: None,
            profile_delete: None,
            profile_status: String::new(),
            last_backup: Instant::now(),
            backup_due: false,
            restore_confirm: None,
            backup_status: String::new(),
            pop_out_panel: None,
            dragging_panel: None,
            dragging_window: None,
            last_recording: None,
            recording_error: String::new(),
            recording_details: String::new(),
            closing: false,
            record_folder,
            loading: screenshot.is_none().then(Loading::start),
            screenshot,
            screenshot_requested: false,
            started: Instant::now(),
            last_slideshow_check: Instant::now(),
            window_placement: crate::window_memory::load(),
            window_changed: None,
            ui_frames: 0,
            demo,
            demo_add,
            demo_drag,
            demo_input: ["press-delete", "press-f2", "click-away", "copy-scene", "costream"].into_iter().find(|p| *p == page),
        }
    }
    fn remember_capture_mode(&mut self, id: Id) {
        #[cfg(windows)] {
            self.project.preferences.windows_auto_sources.retain(|v|*v!=id);
            if self.captures.mode(id)==rosorah_capture::Target::AutoFullscreen {self.project.preferences.windows_auto_sources.push(id);}
        }
        self.project
            .preferences
            .manual_capture_sources
            .retain(|v| *v != id);
        if self.captures.mode(id) != rosorah_capture::Target::AutoFullscreen {
            self.project.preferences.manual_capture_sources.push(id);
        }
        self.changed();
    }
    fn changed(&mut self) {
        self.project.sync_every_scene(&mut self.every_scene_last);
        self.update_widget_geometry();
        self.project.ensure_alert_channels();
        self.edit_history
            .record(&self.project, self.continuous_edit);
        self.notify_changed();
    }
    fn notify_changed(&mut self) {
        self.audio_scene = None;
        self.vertical_revision = true;
        self.dirty = true;
        self.backup_due = true;
        self.revision = true;
        if let Some(r) = &self.live_recording {
            r.update_project(&self.project);
        }
        if let Some(stream) = &self.live_stream {
            stream.update_project(&self.project);
        }
        if let Some(camera) = &self.virtual_camera {
            camera.update_project(&self.project);
        }
        if let Some(output) = &self.ndi_output {
            output.update_project(&self.project);
        }
        if let Some(output) = &self.spout_output {
            output.update_project(&self.project);
        }
    }
    /// The accent colour: the theme's, if it sets one, otherwise the one chosen in Settings.
    fn accent(&self) -> Color32 {
        let [r, g, b] = self.project.preferences.accent;
        crate::appearance::current().accent.unwrap_or(Color32::from_rgb(r, g, b))
    }
    fn checkpoint_widgets(&mut self) {
        let periods = self
            .captures
            .widgets
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|(id, s)| (*id, s.lock().unwrap().period_key.clone()))
            .collect();
        if self.project.preferences.widget_periods != periods {
            self.project.preferences.widget_periods = periods;
            self.dirty = true;
        }

        let values = self
            .captures
            .widgets
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|(id, s)| (*id, s.lock().unwrap().value))
            .collect();
        if self.project.preferences.widget_values != values {
            self.project.preferences.widget_values = values;
            self.dirty = true;
        }
    }
    fn save(&mut self) {
        if self.recovery_candidate.is_some() {
            self.message = "Choose whether to recover the unsaved project first".into();
            return;
        }
        if let Some(rx) = self.recovery_write.take()
            && matches!(
                rx.recv_timeout(Duration::from_secs(3)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ) {
                self.recovery_write = Some(rx);
                self.message =
                    "Recovery save is still finishing. Please save again in a moment.".into();
                return;
            }
        self.checkpoint_widgets();
        match crate::edit_history::save_with_backups(&self.project, &self.project_path) {
            Ok(()) => {
                self.dirty = false;
                self.message = "Project saved".into();
            }
            Err(e) => self.message = format!("Could not save project: {e}"),
        }
    }
    pub fn notice(&mut self, text: String) {
        self.message = text;
    }
    fn profiles_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Profiles");
        note(ui, "Each profile has its own scenes, sources, mixer, alerts, automations, layout and backups. Accounts and stream keys are shared.");
        ui.add_space(12.);
        let open = self.project_path.clone();
        let current = crate::profiles::list().into_iter().find(|n| crate::profiles::path(n) == open);
        let busy = self.live_stream.is_some() || self.live_recording.is_some();
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            if current.is_none() {
                ui.small(format!("Open project: {} (opened from a file, not a profile)", open.display()));
            }
            for name in crate::profiles::list() {
                ui.horizontal(|ui| {
                    let here = current.as_deref() == Some(name.as_str());
                    ui.label(RichText::new(&name).strong());
                    if here {
                        ui.small("· open now");
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if !here && name != crate::profiles::DEFAULT && ui.small_button("Delete…").clicked() {
                            self.profile_delete = Some(name.clone());
                        }
                        if !here && name != crate::profiles::DEFAULT && ui.small_button("Rename…").clicked() {
                            self.profile_rename = Some((name.clone(), name.clone()));
                        }
                        if !here && ui.add_enabled(!busy, egui::Button::new("Switch")).on_disabled_hover_text("Stop streaming and recording first").clicked() {
                            match crate::profiles::set_current(&name) {
                                Ok(()) => {
                                    self.restart_profile = Some(name.clone());
                                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                                }
                                Err(e) => self.profile_status = format!("{e:#}"),
                            }
                        }
                    });
                });
            }
            if let Some((old, mut new)) = self.profile_rename.clone() {
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(format!("Rename {old} to"));
                    ui.text_edit_singleline(&mut new);
                    if ui.button("Rename").clicked() {
                        self.profile_status = match crate::profiles::rename(&old, &new) {
                            Ok(()) => {
                                self.profile_rename = None;
                                "Profile renamed".into()
                            }
                            Err(e) => format!("{e:#}"),
                        };
                        return;
                    }
                    if ui.button("Cancel").clicked() {
                        self.profile_rename = None;
                        return;
                    }
                    self.profile_rename = Some((old, new));
                });
            }
            if let Some(name) = self.profile_delete.clone() {
                match confirm(ui.ctx(), "profile-delete", "Delete profile", &format!("Delete the {name} profile with its layout and backups? This can't be undone."), "Yes, delete it") {
                    Some(true) => {
                        self.profile_status = match crate::profiles::delete(&name, current.as_deref().unwrap_or("")) {
                            Ok(()) => format!("Deleted {name}"),
                            Err(e) => format!("{e:#}"),
                        };
                        self.profile_delete = None;
                    }
                    Some(false) => self.profile_delete = None,
                    None => {}
                }
            }
        });
        ui.add_space(12.);
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.strong("New profile");
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.profile_name).hint_text("Profile name"));
                ui.checkbox(&mut self.profile_copy, "Start from a copy of this profile");
                if ui.button("Create").clicked() {
                    let project = if self.profile_copy { self.project.clone() } else { Project::default() };
                    self.profile_status = match crate::profiles::create(&self.profile_name, &project) {
                        Ok(()) => {
                            if self.profile_copy {
                                let _ = std::fs::copy(self.project_path.with_extension("layout.json"), crate::profiles::path(self.profile_name.trim()).with_extension("layout.json"));
                            }
                            let name = std::mem::take(&mut self.profile_name);
                            format!("Created {}. Press Switch to open it; Rosorah restarts into that profile.", name.trim())
                        }
                        Err(e) => format!("{e:#}"),
                    };
                }
            });
        });
        if !self.profile_status.is_empty() {
            ui.small(&self.profile_status);
        }
    }
    /// Switches to another project (imported, restored or recovered), preparing its scene lists.
    fn adopt_project(&mut self, mut project: Project) {
        project.prepare_workspace();
        project.ensure_alert_channels();
        project.merge_alert_boxes();
        project.drop_hidden_alert_layers();
        self.every_scene_last.clear();
        project.sync_every_scene(&mut self.every_scene_last);
        self.project = project;
        self.selections.clear();
        self.layer = None;
        self.crop_layer = None;
        self.canvas = 0;
        self.edit_history = crate::edit_history::History::new(&self.project);
        self.notify_changed();
    }
    /// Saves the project, then keeps a timestamped copy of it in the backups folder.
    fn back_up(&mut self, reason: &str) -> Result<PathBuf, String> {
        self.save();
        if self.dirty {
            return Err(self.message.clone());
        }
        let result = crate::backups::create(&self.project, &self.project_path, reason, self.project.preferences.backups.keep)
            .map_err(|e| format!("Backup failed: {e:#}"));
        if result.is_ok() {
            self.last_backup = Instant::now();
            self.backup_due = false;
        }
        result
    }
    fn backups_settings(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        ui.heading("Backups");
        note(ui, "Back up saves your project and keeps a dated copy you can restore later. Ctrl+S saves without making a copy.");
        ui.add_space(12.);
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                if ui.button(if self.dirty { "Back up *" } else { "Back up" }).clicked() {
                    self.backup_status = match self.back_up("manual") {
                        Ok(path) => format!("Backed up to {}", path.display()),
                        Err(e) => e,
                    };
                }
                if self.dirty {
                    ui.small("Unsaved changes");
                }
            });
            if !self.backup_status.is_empty() {
                ui.small(&self.backup_status);
            }
            ui.separator();
            ui.strong("Automatic backups");
            let policy = &mut self.project.preferences.backups;
            changed |= ui.checkbox(&mut policy.hourly, "Every hour (only when something changed)").changed();
            changed |= ui.checkbox(&mut policy.on_close, "Every time Rosorah closes").changed();
            changed |= ui.checkbox(&mut policy.restore_on_start, "Open the newest backup at start if it's newer than the saved project")
                .on_hover_text("Also opens the newest backup if the saved project can't be read")
                .changed();
            ui.horizontal(|ui| {
                ui.label("Keep the newest");
                changed |= ui.add(egui::DragValue::new(&mut policy.keep).range(1..=500)).changed();
                ui.label("backups");
            });
        });
        ui.add_space(12.);
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.strong("Restore a backup");
            let entries = crate::backups::list(&self.project_path);
            if entries.is_empty() {
                ui.small("No backups yet.");
            }
            egui::ScrollArea::vertical().id_salt("backup-list").max_height(260.).show(ui, |ui| {
                for entry in entries.iter().take(200) {
                    ui.horizontal(|ui| {
                        ui.label(&entry.label);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let busy = self.live_stream.is_some() || self.live_recording.is_some();
                            if ui.add_enabled(!busy, egui::Button::new("Restore…")).on_disabled_hover_text("Stop streaming and recording first").clicked() {
                                self.restore_confirm = Some(entry.path.clone());
                            }
                        });
                    });
                }
            });
            if let Some(path) = self.restore_confirm.clone() {
                let body = format!("Restore {}? Your current project is backed up first, so you can undo this by restoring that copy.",
                    path.file_stem().unwrap_or_default().to_string_lossy());
                match confirm(ui.ctx(), "restore-backup", "Restore backup", &body, "Yes, restore") {
                    Some(true) => {
                        self.restore_confirm = None;
                        self.backup_status = match crate::backups::load(&path) {
                            Err(e) => format!("{e:#}"),
                            Ok(project) => match self.back_up("before restore") {
                                Err(e) => format!("Not restored: {e}"),
                                Ok(_) => {
                                    self.adopt_project(project);
                                    self.save();
                                    "Backup restored".into()
                                }
                            },
                        };
                    }
                    Some(false) => self.restore_confirm = None,
                    None => {}
                }
            }
        });
        ui.add_space(12.);
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.strong("Import from OBS or Firebot");
            self.external_import_ui(ui);
        });
        changed
    }
    fn log(&mut self, text: String) {
        self.activity.push_back(text);
        while self.activity.len() > 64 {
            self.activity.pop_front();
        }
    }
    fn header(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("header")
            .exact_height(56.)
            .show(ctx, |ui| {
                crate::appearance::glass(ui, egui::CornerRadius::ZERO);
                // On GNOME the header is also the title bar: drag it to move the window.
                let custom = self.custom_title_bar;
                if custom {
                    crate::title_bar::drag_area(ui);
                }
                ui.horizontal_centered(|ui| {
                    if custom && !crate::title_bar::buttons().left.is_empty() {
                        crate::title_bar::show_buttons(ui, &crate::title_bar::buttons().left);
                        ui.add_space(8.);
                    }
                    ui.label(RichText::new("Rosorah Studio").size(22.).strong());
                    ui.add_space(24.);
                    for (p, label) in [
                        (Page::Studio, "Studio"),
                        (Page::Alerts, "Alerts & automation"),
                        (Page::Settings, "Settings"),
                    ] {
                        if ui.selectable_label(self.page == p, label).clicked() {
                            self.page = p;
                        }
                    }
                    if self.page == Page::Studio {
                        let mut studio = self.project.preferences.workspace.studio_mode;
                        if ui.toggle_value(&mut studio, "Studio Mode").on_hover_text("Prepare scenes in a preview, then send them live with Transition").changed() {
                            self.project.preferences.workspace.studio_mode = studio;
                            // The preview starts on what's live.
                            self.studio_preview = studio.then(|| self.project.canvases[0].scene);
                            self.revision = true;
                            self.dirty = true;
                        }
                        ui.menu_button("Edit", |ui| {
                            if ui.add_enabled(self.edit_history.can_undo(), egui::Button::new("Undo · Ctrl+Z")).clicked() {
                                self.undo_edit(false);
                                ui.close();
                            }
                            if ui.add_enabled(self.edit_history.can_redo(), egui::Button::new("Redo · Ctrl+Shift+Z")).clicked() {
                                self.undo_edit(true);
                                ui.close();
                            }
                        });
                        // Panels can be moved, hidden and brought back in both modes.
                        ui.menu_button("Panels", |ui| {
                            note(ui, "Drag to an edge to snap; center to group tabs");
                            note(ui, "Double-click a tab to float or dock it");
                            note(ui, "Drag a tab out of the window, or right-click it, to give it its own window");
                            for panel in Panel::ALL {
                                let mut visible = self.workspace.contains(panel);
                                if ui
                                    .add_enabled(panel != Panel::Preview, egui::Checkbox::new(&mut visible, panel.title()))
                                    .changed()
                                {
                                    self.workspace.set_visible(panel, visible);
                                }
                            }
                            ui.separator();
                            ui.strong("Web pages");
                            let mut remove = None;
                            for dock in &self.project.preferences.workspace.browser_docks {
                                ui.horizontal(|ui| {
                                    let panel = Panel::Web(dock.id);
                                    let mut visible = self.workspace.contains(panel);
                                    if ui.checkbox(&mut visible, &dock.name).on_hover_text(&dock.url).changed() {
                                        self.workspace.set_visible(panel, visible);
                                    }
                                    if ui.small_button("✕").on_hover_text("Remove this web page").clicked() {
                                        remove = Some(dock.id);
                                    }
                                });
                            }
                            if let Some(id) = remove {
                                self.project.preferences.workspace.browser_docks.retain(|d| d.id != id);
                                self.workspace.retain_web(&self.project.preferences.workspace.browser_docks.iter().map(|d| d.id).collect::<Vec<_>>());
                                self.web_docks.remove(id);
                                self.dirty = true;
                            }
                            if ui.button("+ Add a web page…").clicked() {
                                self.web_dock_dialog = Some((String::new(), "https://".into()));
                                ui.close();
                            }
                            ui.separator();
                            ui.menu_button("Open a projector", |ui| self.projector_menu(ui));
                            if ui.button("Reset layout").clicked() {
                                self.reset_layout();
                                ui.close();
                            }
                        });
                    }
                    ui.menu_button("Help", |ui| self.help_menu(ui));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if custom && !crate::title_bar::buttons().right.is_empty() {
                            let mut right = crate::title_bar::buttons().right.clone();
                            right.reverse();
                            crate::title_bar::show_buttons(ui, &right);
                            ui.add_space(8.);
                        }
                        self.live_controls(ui)
                    });
                });
            });
    }
    /// Go live, Record and what is happening now, together at the right of the header.
    fn live_controls(&mut self, ui: &mut egui::Ui) {
        use crate::recording::Phase;
        let clock = |d: Duration| format!("{}:{:02}", d.as_secs() / 60, d.as_secs() % 60);
        if let Some(stream) = &self.live_stream {
            let view = stream.view();
            ui.add_enabled_ui(view.phase != Phase::Finishing, |ui| {
                ui.menu_button("Stop streaming", |ui| {
                    note(ui, "Your viewers will see the stream end.");
                    if ui.button("Yes, end my stream").clicked() {
                        stream.stop();
                        ui.close();
                    }
                });
            });
            let destinations: Vec<_> = stream
                .destinations()
                .into_iter()
                .filter(|d| d.path.is_none())
                .map(|d| {
                    let mark = match d.phase {
                        crate::streaming::Phase::Sending => "✓",
                        crate::streaming::Phase::Failed => "✕",
                        _ => "…",
                    };
                    format!("{} {mark}", d.name)
                })
                .collect();
            // Just the timer up here; where it's going is in the hover (and the Stream panel).
            let failed = destinations.iter().filter(|d| d.ends_with('✕')).count();
            let (text, color) = if view.phase == Phase::Starting {
                ("● STARTING…".to_owned(), crate::appearance::current().warning)
            } else if failed > 0 {
                (format!("● LIVE {} · {failed} {} attention", clock(view.elapsed), if failed == 1 { "needs" } else { "need" }), crate::appearance::current().live)
            } else {
                (format!("● LIVE {}", clock(view.elapsed)), crate::appearance::current().live)
            };
            ui.label(RichText::new(text).strong().color(color)).on_hover_text(format!(
                "{}\n\n✓ sending · … connecting · ✕ needs attention. Check your platform's dashboard to see what viewers get.",
                destinations.join("\n")
            ));
        } else if ui
            .add(self.primary_button("Go live"))
            .on_hover_text("Checks your setup, then starts the stream")
            .clicked()
        {
            self.go_live_open = true;
        }
        let shared = self.live_stream.as_ref().is_some_and(|s| s.destinations().iter().any(|v| v.path.is_some()));
        if let Some(r) = &self.live_recording {
            let view = r.view();
            if ui
                .add_enabled(view.phase != Phase::Finishing, egui::Button::new(format!("Stop recording · {}", clock(view.elapsed))))
                .clicked()
            {
                r.stop();
            }
            ui.label(RichText::new("● REC").strong().color(crate::appearance::current().live));
        } else if shared {
            ui.label(RichText::new("● Recording the stream").small().color(muted()));
        } else if ui.add(self.primary_button("Record")).on_hover_text("Records the main video to your computer").clicked() {
            self.start_recording();
        }
        if self.live_stream.is_none() && self.live_recording.is_none() {
            ui.label(RichText::new("OFFLINE").size(11.).color(muted()));
        }
    }
    fn help_menu(&mut self, ui: &mut egui::Ui) {
        if ui.button("Setup guide…").clicked() {
            self.tools.guide = Some(0);
            ui.close();
        }
        if ui.button("Check my setup…").clicked() {
            self.readiness_open = true;
            ui.close();
        }
        if ui.button("Reset layout").on_hover_text("Put every panel back where it started").clicked() {
            self.reset_layout();
            self.page = Page::Studio;
            ui.close();
        }
        if ui.button("Report a problem…").clicked() {
            self.report_open = true;
            ui.close();
        }
        if ui.button("Check for updates…").clicked() {
            self.check_for_updates(ui.ctx());
            ui.close();
        }
        if self.advanced() {
            ui.separator();
            if ui.button("Monitoring & sync…").clicked() {
                self.tools.audio = true;
                ui.close();
            }
            if ui.button("Project & recording tools…").clicked() {
                self.tools.open = true;
                ui.close();
            }
            if ui.button("Desktop compatibility…").clicked() {
                self.tools.compatibility = true;
                crate::desktop::refresh();
                ui.close();
            }
            if ui.button("Performance details…").clicked() {
                self.tools.performance = true;
                ui.close();
            }
        }
    }
    /// Looks again during long sessions, opens the window once for a new version, and answers
    /// Help > Check for updates.
    fn update_tick(&mut self, ctx: &egui::Context) {
        use crate::updates::State;
        if self.screenshot.is_some() {
            return;
        }
        let state = self.updates.state();
        if self.project.preferences.workspace.check_updates
            && self.last_update_check.elapsed() >= Duration::from_secs(6 * 3600)
            && matches!(state, State::Idle | State::UpToDate | State::Failed(_))
        {
            self.updates.check(ctx);
            self.last_update_check = Instant::now();
        }
        if self.update_asked {
            match &state {
                State::UpToDate => self.tell(concat!("You have the latest version (", env!("CARGO_PKG_VERSION"), ").").into()),
                State::Failed(e) => self.tell(e.clone()),
                _ => {}
            }
            if !matches!(state, State::Idle | State::Checking) {
                self.update_asked = false;
            }
        }
        let live = self.live_stream.is_some() || self.live_recording.is_some();
        let workspace = &self.project.preferences.workspace;
        if let State::Available(release) = &state
            && workspace.update_popup
            && !live
            && release.version != workspace.skipped_update
            && release.version != workspace.update_offered
        {
            self.project.preferences.workspace.update_offered = release.version.clone();
            self.update_window = true;
            self.dirty = true;
        }
        ctx.request_repaint_after(Duration::from_secs(60));
    }
    /// Help > Check for updates: looks now and shows the result, even for a skipped version.
    fn check_for_updates(&mut self, ctx: &egui::Context) {
        use crate::updates::State;
        // A found or downloaded update is kept rather than looked up again.
        if !matches!(self.updates.state(), State::Available(_) | State::Downloaded(..) | State::Downloading(..) | State::Installing(..) | State::Installed(_) | State::InstallFailed(..)) {
            self.updates.check(ctx);
            self.last_update_check = Instant::now();
        }
        self.update_asked = true;
        self.update_window = true;
    }
    /// Clicks and keys for selected layers and scenes, from where things were drawn last frame:
    /// a click away from a canvas unselects its layers, Delete asks before deleting, F2 renames.
    fn selection_input(&mut self, ctx: &egui::Context) {
        let hits = std::mem::take(&mut self.canvas_hits);
        let keep = std::mem::take(&mut self.keep_selection);
        let scenes_area = self.scenes_area.take();
        let busy = egui::Popup::is_any_open(ctx) || ctx.memory(|m| m.top_modal_layer()).is_some() || self.delete_confirm.is_some();
        if self.page != Page::Studio || busy {
            return;
        }
        if let Some(pos) = ctx.input(|i| i.pointer.primary_pressed().then(|| i.pointer.press_origin()).flatten()) {
            if scenes_area.is_some_and(|r| r.contains(pos)) {
                self.list_focus = ListFocus::Scenes;
            } else if keep.iter().chain(hits.iter().map(|(_, r)| r)).any(|r| r.contains(pos)) {
                self.list_focus = ListFocus::Layers;
            }
            // Cropping keeps its layer selected; anywhere else that isn't Layers or a properties window lets go.
            if self.crop_layer.is_none() && !keep.iter().any(|r| r.contains(pos)) {
                for (scene, rect) in &hits {
                    if !rect.contains(pos) && self.selections.remove(scene).is_some() && self.project.canvases.get(self.canvas).is_some_and(|c| c.scene == *scene) {
                        self.layer = None;
                    }
                }
            }
        }
        if ctx.wants_keyboard_input() {
            return;
        }
        let (delete, rename) = ctx.input(|i| (i.key_pressed(egui::Key::Delete), i.key_pressed(egui::Key::F2)));
        if !delete && !rename {
            return;
        }
        let canvas = self.project.canvases[self.canvas].id;
        let scene = self.project.canvases[self.canvas].scene;
        match self.list_focus {
            ListFocus::Layers => {
                let Some(layers) = self.project.scene(scene).map(|s| s.layers.clone()) else { return };
                let selected: Vec<usize> = self.selections.get(&scene).cloned().unwrap_or_default().into_iter().filter(|&i| i < layers.len()).collect();
                if delete {
                    let unlocked: Vec<usize> = selected.iter().copied().filter(|&i| !layers[i].locked).collect();
                    if !unlocked.is_empty() {
                        self.delete_confirm = Some((scene, unlocked));
                    } else if !selected.is_empty() {
                        self.tell("Locked layers can't be deleted. Unlock them from their right-click menu first.".into());
                    }
                } else if let Some(&i) = selected.last()
                    && let Some(source) = self.project.source(layers[i].source)
                {
                    self.layer_rename = Some((source.id, source.name.clone()));
                }
            }
            ListFocus::Scenes => {
                let name = self.project.scene(scene).map(|s| s.name.clone()).unwrap_or_default();
                if rename {
                    self.naming_scene = Some((scene, name, false));
                } else if self.project.canvas_scenes(canvas).len() < 2 {
                    self.tell("A video needs at least one scene.".into());
                } else if self.live_stream.is_some() || self.live_recording.is_some() {
                    self.tell("This scene is on air. Switch to another scene first.".into());
                } else {
                    self.scene_dialog = Some((scene, name, true));
                }
            }
        }
    }
    /// Pastes a copy of a scene just after the current one, into whichever video is selected.
    fn paste_scene(&mut self, copied: Id) {
        let canvas = self.project.canvases[self.canvas].id;
        let current = self.project.canvases[self.canvas].scene;
        let pasted = if self.project.canvas_scenes(canvas).contains(&copied) {
            self.project.duplicate_scene(canvas, copied)
        } else if let Some(mut scene) = self.project.scene(copied).cloned() {
            // From the other video: same layers, added to this one's list.
            scene.id = self.project.id();
            let base = format!("{} copy", scene.name);
            scene.name = base.clone();
            let mut n = 2;
            while self.project.scenes.iter().any(|s| s.name == scene.name) {
                scene.name = format!("{base} {n}");
                n += 1;
            }
            let id = scene.id;
            self.project.scenes.push(scene);
            let ids = self.project.canvas_scenes(canvas);
            let list = self.project.preferences.workspace.scenes.entry(canvas).or_insert(ids);
            let at = list.iter().position(|i| *i == current).map_or(list.len(), |i| i + 1);
            list.insert(at, id);
            if let Some(audio) = self.project.preferences.scene_audio.get(&copied).cloned() {
                self.project.preferences.scene_audio.insert(id, audio);
            }
            Some(id)
        } else {
            self.scene_clipboard = None;
            None
        };
        match pasted.and_then(|id| self.project.scene(id).map(|s| s.name.clone())) {
            Some(name) => {
                self.changed();
                self.tell(format!("Pasted \"{name}\"."));
            }
            None => self.tell("That scene was deleted, so there's nothing to paste.".into()),
        }
    }
    /// "Delete these layers?", front and center with the studio dimmed behind it.
    fn delete_confirm_ui(&mut self, ctx: &egui::Context) {
        let Some((scene, layers)) = self.delete_confirm.clone() else { return };
        let Some(si) = self.project.scenes.iter().position(|s| s.id == scene) else {
            self.delete_confirm = None;
            return;
        };
        let names: Vec<String> = layers
            .iter()
            .filter_map(|&i| self.project.scenes[si].layers.get(i))
            .map(|l| self.project.source(l.source).map(|s| s.name.clone()).unwrap_or_default())
            .collect();
        let (mut confirm, mut cancel) = (false, false);
        pop_modal(ctx, egui::Id::new("delete-layers")).show(ctx, |ui| {
            ui.set_width(360.);
            ui.heading(if names.len() == 1 { "Delete layer".to_owned() } else { format!("Delete {} layers", names.len()) });
            ui.add_space(6.);
            ui.label(if names.len() == 1 { format!("Remove “{}” from {}?", names[0], self.project.scenes[si].name) } else { format!("Remove these from {}?", self.project.scenes[si].name) });
            if names.len() > 1 {
                for name in names.iter().take(8) {
                    ui.label(format!("• {name}"));
                }
                if names.len() > 8 {
                    note(ui, format!("and {} more", names.len() - 8));
                }
            }
            let everywhere: Vec<&String> = layers.iter().zip(&names).filter(|&(&i, _)| self.project.scenes[si].layers.get(i).is_some_and(|l| self.project.every_scene(l.source))).map(|(_, n)| n).collect();
            match everywhere.as_slice() {
                [] => note(ui, "Their sources stay available to your other scenes."),
                [name] => note(ui, format!("“{name}” is the same in every scene, so it's removed from all of them.")),
                _ => note(ui, "Layers set to be the same in every scene are removed from all of them."),
            }
            ui.add_space(8.);
            ui.horizontal(|ui| {
                confirm = ui.add(egui::Button::new(RichText::new("Delete").strong().color(Color32::WHITE)).fill(Color32::from_rgb(190, 50, 60))).clicked();
                cancel = ui.button("Cancel").clicked();
            });
            note(ui, "Enter deletes · Esc cancels");
        });
        let (enter, escape) = ctx.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
        if confirm || enter {
            let layers_in_scene = &mut self.project.scenes[si].layers;
            let mut doomed: Vec<usize> = layers.into_iter().filter(|&i| layers_in_scene.get(i).is_some_and(|l| !l.locked)).collect();
            doomed.sort_unstable();
            doomed.dedup();
            let mut everywhere = Vec::new();
            for &i in doomed.iter().rev() {
                everywhere.push(layers_in_scene.remove(i).source);
            }
            everywhere.retain(|&source| self.project.every_scene(source));
            for source in everywhere {
                self.project.remove_from_every_scene(scene, source);
            }
            self.selections.remove(&scene);
            self.layer = None;
            self.crop_layer = None;
            self.layer_properties = None;
            self.changed();
            self.deleted(if names.len() == 1 { format!("Removed {} from this scene", names[0]) } else { format!("Removed {} layers from this scene", names.len()) });
            self.delete_confirm = None;
        } else if cancel || escape {
            self.delete_confirm = None;
        }
    }
    /// Confirms a removal with an Undo button, so a slip is one click to reverse.
    fn deleted(&mut self, text: String) {
        self.toast = Some(Toast { text, undo: true, until: Instant::now() + Duration::from_secs(10) });
    }
    fn tell(&mut self, text: String) {
        self.toast = Some(Toast { text, undo: false, until: Instant::now() + Duration::from_secs(8) });
    }
    fn toast_ui(&mut self, ctx: &egui::Context) {
        let Some(toast) = &self.toast else { return };
        if toast.until <= Instant::now() {
            self.toast = None;
            return;
        }
        let (text, undo) = (toast.text.clone(), toast.undo);
        let mut action = 0;
        egui::Area::new(egui::Id::new("toast"))
            .anchor(egui::Align2::CENTER_BOTTOM, [0., -44.])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new().fill(crate::appearance::current().card).corner_radius(12).inner_margin(egui::Margin::symmetric(14, 8)).shadow(ui.visuals().popup_shadow).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(&text);
                        if undo && ui.add(self.primary_button("Undo")).clicked() {
                            action = 1;
                        }
                        if ui.small_button("✕").on_hover_text("Dismiss").clicked() {
                            action = 2;
                        }
                    });
                });
            });
        if action == 1 {
            self.undo_edit(false);
        }
        if action > 0 {
            self.toast = None;
        }
        ctx.request_repaint_after(Duration::from_millis(500));
    }
    /// A mixer input with no device chosen yet (game audio follows the captured game on its own).
    fn unconnected(&self, channel: &auroracast_core::AudioChannel) -> bool {
        channel.media_source.is_none()
            && !(channel.id == crate::game_audio::CHANNEL && self.project.preferences.auto_game_audio)
            && self.audio.selected.get(&channel.id).is_none_or(|s| s.is_empty())
    }
    /// Simple mode's way to bring in a sound: pick the device for an input that has none yet.
    fn add_sound_menu(&mut self, ui: &mut egui::Ui, view: &crate::audio::View) {
        egui::containers::menu::MenuButton::from_button(self.primary_button("+ Add sound")).ui(ui, |ui| {
            let waiting: Vec<_> = self.project.audio.iter().filter(|c| self.unconnected(c)).map(|c| (c.id, c.name.clone())).collect();
            for (id, name) in waiting {
                let output = name == "Desktop audio";
                ui.menu_button(if output { "Computer sound".to_owned() } else { name }, |ui| {
                    note(ui, if output { "Choose the speakers or headphones whose sound to include" } else { "Choose the device" });
                    for d in view.devices.iter().filter(|d| (d.kind == "Audio/Sink") == output) {
                        if ui.button(&d.label).clicked() {
                            self.audio.selected.insert(id, d.serial.clone());
                            self.audio.enabled.insert(id, true);
                            ui.close();
                        }
                    }
                });
            }
            if !self.project.audio.iter().any(|c| c.name == "Desktop audio") {
                ui.menu_button("Computer sound", |ui| {
                    note(ui, "Everything playing on this computer. Choose the speakers or headphones it plays through.");
                    for d in view.devices.iter().filter(|d| d.kind == "Audio/Sink") {
                        if ui.button(&d.label).clicked() {
                            let id = self.project.id();
                            self.project.audio.push(auroracast_core::AudioChannel { monitor_only: false, id, name: "Desktop audio".into(), gain_db: 0., muted: false, tracks: [true, false, false, false, false, false], delay_ms: 0, monitor: false, noise_suppression: Default::default(), media_source: None });
                            self.audio.selected.insert(id, d.serial.clone());
                            self.audio.enabled.insert(id, true);
                            self.changed();
                            ui.close();
                        }
                    }
                });
            }
            // Its own strip for one app's sound, followed by name so it's back after a restart.
            ui.menu_button("An application…", |ui| {
                let followed: Vec<String> = self.audio.selected.values().filter_map(|t| crate::app_audio::followed_app(t)).map(str::to_owned).collect();
                let mut apps: Vec<(String, String)> = view
                    .devices
                    .iter()
                    .filter(|d| d.kind == "Stream/Output/Audio" && !d.app.is_empty() && !followed.contains(&d.app))
                    .map(|d| (d.app.clone(), crate::app_audio::app_name(d)))
                    .collect();
                apps.sort_by_key(|a| a.1.to_lowercase());
                apps.dedup_by(|a, b| a.0 == b.0);
                if apps.is_empty() {
                    note(ui, "No app is playing sound right now. Start it playing, then look again.");
                } else {
                    note(ui, "Apps playing sound now");
                }
                for (app, name) in apps {
                    if ui.button(&name).clicked() {
                        let id = self.project.id();
                        self.project.audio.push(auroracast_core::AudioChannel { monitor_only: false, id, name, gain_db: 0., muted: false, tracks: [true, false, false, false, false, false], delay_ms: 0, monitor: false, noise_suppression: Default::default(), media_source: None });
                        self.audio.selected.insert(id, crate::app_audio::app_target(&app));
                        self.audio.enabled.insert(id, true);
                        self.changed();
                        ui.close();
                    }
                }
            });
            ui.separator();
            if ui.button("Look for new devices").clicked() {
                self.audio.refresh();
            }
        });
    }
    fn advanced(&self) -> bool {
        self.project.preferences.workspace.advanced
    }
    /// The accent-colored button used only for main actions (Go live, Record, Add).
    fn primary_button(&self, text: &str) -> egui::Button<'static> {
        egui::Button::new(RichText::new(text.to_owned()).strong().color(Color32::from_rgb(40, 22, 84))).fill(self.accent())
    }
    fn reset_layout(&mut self) {
        self.workspace.reset();
        self.sync_vertical_panel();
    }
    /// Shows the vertical video panel only when vertical video is switched on.
    fn sync_vertical_panel(&mut self) {
        let wanted = self.project.preferences.workspace.vertical_video && self.project.canvases.len() > 1;
        if self.workspace.contains(Panel::Vertical) != wanted {
            self.workspace.set_visible(Panel::Vertical, wanted);
        }
        if !wanted && self.canvas > 0 {
            self.canvas = 0;
            self.layer = None;
        }
    }
    fn footer(&mut self, ctx: &egui::Context) {
        ctx.request_repaint_after(Duration::from_secs(1));
        if self.last_stats.elapsed() >= Duration::from_secs(1) {
            self.resources.sample();
            self.last_stats = Instant::now();
        }
        let fmt = |v: Option<f32>| v.map(|v| format!("{v:.1}%")).unwrap_or_else(|| "—".into());
        egui::TopBottomPanel::bottom("footer").exact_height(32.).show(ctx,|ui|{ui.horizontal_centered(|ui|{
            note(ui,concat!("Rosorah Studio ",env!("CARGO_PKG_VERSION")));
            // A quiet reminder in the footer; the window with the details opens from it.
            let offer = match self.updates.state() {
                crate::updates::State::Available(r) if r.version != self.project.preferences.workspace.skipped_update => Some(format!("⬆ Update {} available", r.version)),
                crate::updates::State::Downloading(..) => Some("⬆ Downloading update…".into()),
                crate::updates::State::Downloaded(r, _) | crate::updates::State::InstallFailed(r, ..) => Some(format!("⬆ Install {}", r.version)),
                crate::updates::State::Installing(..) => Some("⬆ Installing update…".into()),
                crate::updates::State::Installed(r) => Some(format!("⬆ Restart for {}", r.version)),
                _ => None,
            };
            if let Some(text) = offer && ui.add(egui::Button::new(RichText::new(text).small().strong().color(Color32::from_rgb(40, 22, 84))).fill(self.accent())).clicked() {
                self.update_window = true;
            }
            ui.separator();ui.label(RichText::new(&self.message).small());
            // Laid out right to left: RAM, GPU temperature, GPU, CPU temperature, CPU.
            let temperature = |ui: &mut egui::Ui, t: Option<crate::system_monitor::Temperature>, what: &str| {
                if let Some(t) = t {
                    let text = RichText::new(format!("{:.0}°C", t.celsius)).small();
                    let text = if t.too_hot() { text.strong().color(crate::appearance::current().live) } else { text.weak() };
                    ui.label(text).on_hover_text(format!("{what} temperature. Red from {:.0}°C, close to its limit.", t.hot));
                }
            };
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{note(ui,format!("RAM {}",fmt(self.resources.ram)));temperature(ui,self.resources.gpu_temp,"GPU");note(ui,format!("GPU {}",self.resources.gpu.iter().map(|(_,v)|*v).max().map_or("—".into(),|v|format!("{v}%"))));temperature(ui,self.resources.cpu_temp,"CPU");note(ui,format!("CPU {}",fmt(self.resources.cpu)));}).response.on_hover_text("System usage (all applications), updated once per second. GPU shows the busiest reported device/engine. Unsupported counters show —, never a fabricated zero.");
        });});
    }
    fn scene_context(&mut self, ui: &mut egui::Ui, canvas: Id, id: Id) {
        // Switching to this scene can use its own transition instead of the usual one.
        let usual = self.project.preferences.transition;
        let current = self.project.preferences.scene_transitions.get(&id).copied();
        ui.menu_button(format!("Transition into this scene · {}", current.map_or("Usual".to_owned(), |t| t.kind.label().to_owned())), |ui| {
            if ui.selectable_label(current.is_none(), format!("Usual ({})", usual.label())).clicked() {
                self.project.preferences.scene_transitions.remove(&id);
                self.changed();
                ui.close();
            }
            for kind in [Transition::Cut, Transition::Fade, Transition::Wipe, Transition::Slide, Transition::Swipe, Transition::PatternWipe, Transition::FadeToColor, Transition::Move] {
                if ui.selectable_label(current.is_some_and(|t| t.kind == kind), kind.label()).clicked() {
                    let ms = current.map_or(self.project.preferences.transition_ms, |t| t.ms);
                    self.project.preferences.scene_transitions.insert(id, auroracast_core::SceneTransition { kind, ms });
                    self.changed();
                    ui.close();
                }
            }
            if let Some(mut t) = current.filter(|t| t.kind != Transition::Cut) {
                ui.separator();
                if ui.add(egui::DragValue::new(&mut t.ms).range(100..=3000).speed(10).prefix("Takes ").suffix(" ms")).changed() {
                    self.project.preferences.scene_transitions.insert(id, t);
                    self.changed();
                }
            }
        });
        ui.separator();
        if ui.button("Duplicate").clicked() {
            self.project.duplicate_scene(canvas, id);
            self.changed();
            ui.close();
        }
        if ui.button("Rename").clicked() {
            self.naming_scene = Some((id, self.project.scene(id).unwrap().name.clone(), false));
            ui.close();
        }
        let vertical_in_use = self.project.canvases.len() > 1 && (self.advanced() || self.project.preferences.workspace.vertical_video);
        if vertical_in_use { ui.menu_button(if canvas == self.project.canvases[0].id { "Show on the vertical video" } else { "Show with main video scene" }, |ui| {
            let main = self.project.canvases[0].id;
            let other = if canvas == main {
                self.project.canvases.get(1).map(|c| c.id)
            } else {
                Some(main)
            };
            if let Some(other) = other {
                ui.small(if canvas == main {
                    "Pick the vertical scene to switch to with this one"
                } else {
                    "Pick the main video scene that switches to this one"
                });
                for target in self.project.canvas_scenes(other) {
                    let (from, to) = if canvas == main {
                        (id, target)
                    } else {
                        (target, id)
                    };
                    let linked =
                        self.project.preferences.workspace.scene_links.get(&from) == Some(&to);
                    let name = self
                        .project
                        .scene(target)
                        .map(|s| s.name.clone())
                        .unwrap_or_default();
                    if ui.selectable_label(linked, name).clicked() {
                        self.project
                            .preferences
                            .workspace
                            .scene_links
                            .insert(from, to);
                        self.changed();
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button("Don't switch together").clicked() {
                    self.project
                        .preferences
                        .workspace
                        .scene_links
                        .retain(|a, b| *a != id && *b != id);
                    self.changed();
                    ui.close();
                }
            } else {
                ui.label("Add another canvas first.");
            }
        }); }
        let on_air = self.project.canvases.iter().any(|c| c.scene == id) && (self.live_stream.is_some() || self.live_recording.is_some());
        if ui
            .add_enabled(
                self.project.canvas_scenes(canvas).len() > 1 && !on_air,
                egui::Button::new("Delete"),
            )
            .on_disabled_hover_text(if on_air { "This scene is on air. Switch to another scene first." } else { "A video needs at least one scene" })
            .clicked()
        {
            self.scene_dialog = Some((id, self.project.scene(id).unwrap().name.clone(), true));
            ui.close();
        }
    }
    /// Main video / Vertical video tabs over Scenes and Layers. Clicking a video's preview
    /// switches them too.
    fn canvas_tabs(&mut self, ui: &mut egui::Ui) {
        if !(self.project.canvases.len() > 1 && (self.advanced() || self.project.preferences.workspace.vertical_video)) {
            return;
        }
        let canvases: Vec<(String, Id)> = self.project.canvases.iter().map(|c| (c.name.clone(), c.scene)).collect();
        ui.horizontal_wrapped(|ui| {
            for (i, (name, scene)) in canvases.iter().enumerate() {
                if ui.selectable_label(self.canvas == i, name).clicked() && self.canvas != i {
                    self.canvas = i;
                    self.layer = self.selections.get(scene).and_then(|v| v.last().copied());
                }
            }
        });
        ui.add_space(4.);
    }
    fn scenes_panel(&mut self, ui: &mut egui::Ui) {
        self.scenes_area = Some(ui.max_rect());
        self.canvas_tabs(ui);
        let canvas = self.project.canvases[self.canvas].id;
        let active = self.project.canvases[self.canvas].scene;
        let vertical_in_use = self.project.canvases.len() > 1 && (self.advanced() || self.project.preferences.workspace.vertical_video);
        let ids = self.project.canvas_scenes(canvas);
        let live = self.live_stream.is_some() || self.live_recording.is_some();
        let animate = self.project.preferences.workspace.animations;
        let moved = sortable_rows(ui, egui::Id::new(("scene-rows", canvas)), ids.len(), 36., animate, |ui, p| {
            let id = ids[p];
            let name = self.project.scene(id).map(|s| s.name.clone()).unwrap_or_default();
            let on_air = live && active == id;
            if let Some((naming, text, focused)) = &mut self.naming_scene
                && *naming == id
            {
                // The pill becomes a name box; Enter or clicking away keeps the name, Esc leaves it as it was.
                let response = ui
                    .horizontal(|ui| {
                        egui::Frame::new()
                            .fill(ui.visuals().selection.bg_fill)
                            .corner_radius(ui.visuals().widgets.inactive.corner_radius)
                            .inner_margin(egui::Margin::symmetric(8, 0))
                            .show(ui, |ui| {
                                let ink = ui.visuals().selection.stroke.color;
                                // The pill is the selection colour, so selected text gets a darker shade to show.
                                ui.visuals_mut().selection.bg_fill = ink.gamma_multiply(0.3);
                                ui.add_sized(
                                    [(ui.available_width() - 34.).max(30.), 30.],
                                    egui::TextEdit::singleline(text)
                                        .hint_text(RichText::new("New Scene").color(ink.gamma_multiply(0.55)))
                                        .text_color(ink)
                                        .frame(false)
                                        .vertical_align(egui::Align::Center),
                                )
                            })
                            .inner
                    })
                    .inner;
                if !*focused {
                    response.request_focus();
                    // The whole name is selected, so typing replaces it.
                    select_all(ui.ctx(), response.id, text);
                    *focused = true;
                } else if response.lost_focus() {
                    let typed = text.trim().chars().take(80).collect::<String>();
                    let cancelled = ui.input(|i| i.key_pressed(egui::Key::Escape));
                    self.naming_scene = None;
                    if !cancelled && !typed.is_empty() && let Some(scene) = self.project.scenes.iter_mut().find(|s| s.id == id) {
                        scene.name = typed;
                        self.changed();
                    }
                }
                return response;
            }
            ui.horizontal(|ui| {
                // In Studio Mode the scene being prepared is marked too.
                let preparing = self.project.preferences.workspace.studio_mode && self.studio_preview == Some(id) && active != id;
                let text = if on_air {
                    RichText::new(format!("● LIVE  {name}")).color(crate::appearance::current().live).strong()
                } else if preparing {
                    RichText::new(format!("◐ {name}")).strong()
                } else {
                    RichText::new(name)
                };
                let response = ui
                    .add_sized([ui.available_width().max(30.), 30.], egui::Button::new(text).selected(active == id).truncate().sense(egui::Sense::click_and_drag()))
                    .on_hover_text("Click to switch · drag to reorder · right-click for more")
                    .on_hover_cursor(grab_cursor());
                if response.clicked() {
                    self.request_scene(canvas, id);
                }
                response.context_menu(|ui| self.scene_context(ui, canvas, id));
                response
            })
            .inner
        });
        if let Some((from, to)) = moved {
            self.project.move_scene(canvas, ids[from], to);
            self.changed();
        }
        ui.add_space(4.);
        ui.horizontal_wrapped(|ui| {
            if ui.add(self.primary_button("+ Scene")).clicked() {let id=self.project.id();
                // Named in its pill right away; until then it's "New Scene" (numbered if that's taken).
                let name=(1..).map(|n|if n==1{"New Scene".to_owned()}else{format!("New Scene {n}")}).find(|n|!self.project.scenes.iter().any(|s|&s.name==n)).unwrap();
                self.naming_scene=Some((id,String::new(),false));
                self.project.scenes.push(Scene{id,name,layers:vec![]});self.project.preferences.workspace.scenes.entry(canvas).or_default().push(id);self.project.switch_scene(canvas,id);self.layer=None;self.changed();}
            ui.menu_button("⚙", |ui| {
                ui.menu_button(format!("Transition · {}", self.project.preferences.transition.label()), |ui| self.transition_menu(ui));
                if vertical_in_use {
                    if ui.checkbox(&mut self.project.preferences.workspace.link_scenes, "Switch the vertical video too").on_hover_text("Switching a main video scene also switches the vertical video to its matching scene. Right-click a scene to choose which one.").changed() {
                        self.changed();
                    }
                }
            })
            .response
            .on_hover_text("Transition and scene options");
        });
        if let Some(link) = self
            .project
            .preferences
            .workspace
            .scene_links
            .get(&active)
            .and_then(|id| self.project.scene(*id))
            .filter(|_| vertical_in_use && self.project.preferences.workspace.link_scenes)
        {
            note(ui, format!("Vertical video switches to: {}", link.name));
        }
    }
    /// What's new in the update, and getting it installed.
    fn update_window_ui(&mut self, ctx: &egui::Context) {
        use crate::updates::State;
        if !self.update_window {
            return;
        }
        let state = self.updates.state();
        let release = match &state {
            State::Available(r) | State::Downloading(r, ..) | State::Downloaded(r, _) | State::Installing(r) | State::Installed(r) | State::InstallFailed(r, ..) => r.clone(),
            // Still finding out; the window opens once there's something to show.
            State::Idle | State::Checking => return,
            _ => {
                self.update_window = false;
                return;
            }
        };
        let mut close = false;
        let modal = pop_modal(ctx, egui::Id::new("update-window")).show(ctx, |ui| {
            ui.set_width(460.);
            ui.heading(format!("Rosorah Studio {}", release.version));
            note(ui, concat!("You have ", env!("CARGO_PKG_VERSION"), "."));
            ui.add_space(8.);
            egui::ScrollArea::vertical().max_height(240.).show(ui, |ui| {
                // Plain text from the release notes: headings without their #s, up to the downloads table.
                let notes: Vec<&str> = release.notes.lines().take_while(|l| !l.starts_with("## Downloads")).filter(|l| !l.starts_with('|')).map(|l| l.trim_start_matches('#').trim_start()).collect();
                ui.label(notes.join("\n").trim().replace("**", ""));
            });
            ui.add_space(10.);
            match &state {
                State::Downloading(_, done, total) => {
                    let progress = if *total > 0 { *done as f32 / *total as f32 } else { 0. };
                    ui.add(egui::ProgressBar::new(progress).text(format!("{:.0} of {:.0} MB", *done as f32 / 1e6, *total as f32 / 1e6)));
                }
                State::Downloaded(_, path) => {
                    note(ui, format!("Saved to {}", path.display()));
                    if cfg!(windows) {
                        // The installer can't replace Rosorah's files while it runs, so Rosorah closes
                        // (saving as usual) once the installer has started.
                        let busy = self.live_stream.is_some() || self.live_recording.is_some();
                        if ui.add_enabled(!busy, self.primary_button("Install now")).on_hover_text("Closes Rosorah and opens the installer").on_disabled_hover_text("Stop streaming and recording first").clicked() {
                            match std::process::Command::new(path).spawn() {
                                Ok(_) => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close),
                                Err(e) => self.tell(format!("Couldn't open the installer: {e}")),
                            }
                        }
                    } else {
                        self.linux_install_buttons(ui, &release, path);
                    }
                }
                State::InstallFailed(_, path, error) => {
                    note(ui, error);
                    self.linux_install_buttons(ui, &release, path);
                }
                State::Installing(..) => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        note(ui, "Installing… approve the password prompt if one appears.");
                    });
                }
                State::Installed(_) => {
                    note(ui, format!("Rosorah {} is installed. Restart to start using it.", release.version));
                    let busy = self.live_stream.is_some() || self.live_recording.is_some();
                    if ui.add_enabled(!busy, self.primary_button("Restart now")).on_disabled_hover_text("Stop streaming and recording first").clicked() {
                        self.restart_after_update = true;
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
                _ => {
                    ui.horizontal_wrapped(|ui| {
                        if release.asset.is_some() {
                            if ui.add(self.primary_button("Download and install")).clicked() {
                                self.updates.download(ctx, release.clone());
                            }
                        } else {
                            note(ui, "There's no installer for this system here; get it from the release page.");
                        }
                    });
                }
            }
            ui.add_space(6.);
            ui.horizontal_wrapped(|ui| {
                if ui.button("View on GitHub").clicked() {
                    let _ = crate::desktop::open(&release.page);
                }
                if matches!(state, State::Available(_) | State::Downloaded(..) | State::InstallFailed(..)) && ui.button("Skip this version").clicked() {
                    self.project.preferences.workspace.skipped_update = release.version.clone();
                    self.dirty = true;
                    close = true;
                }
                if ui.button("Later").clicked() {
                    close = true;
                }
            });
        });
        if close || modal.should_close() {
            self.update_window = false;
        }
    }
    /// Linux: install the downloaded package here (system password prompt), or by hand.
    fn linux_install_buttons(&mut self, ui: &mut egui::Ui, release: &crate::updates::Release, path: &std::path::Path) {
        let command = if path.extension().is_some_and(|e| e == "deb") { "sudo apt install" } else { "sudo dnf install" };
        let command = format!("{command} \"{}\"", path.display());
        let busy = self.live_stream.is_some() || self.live_recording.is_some();
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(!busy, self.primary_button("Install now")).on_hover_text("Installs it with your system's password prompt").on_disabled_hover_text("Stop streaming and recording first").clicked() {
                self.updates.install(ui.ctx(), release.clone(), path.to_path_buf());
            }
            if ui.button("Open in software app").on_hover_text("Discover or Software").clicked() {
                let _ = crate::desktop::open(path);
            }
            if ui.button("Copy install command").on_hover_text(&command).clicked() {
                ui.ctx().copy_text(command.clone());
                self.tell("Copied. Paste it into a terminal, then restart Rosorah.".into());
            }
        });
    }
    /// Title, category and where to watch, from the accounts and the destinations switched on.
    fn announce_stream(&self) -> crate::announce::Stream {
        let config = &self.stream_config;
        let [twitch, youtube] = &self.accounts;
        let kick = &self.kick_account;
        let mut links = Vec::new();
        if config.twitch.enabled && twitch.connected && !twitch.name.is_empty() {
            links.push(("Twitch".to_owned(), format!("https://twitch.tv/{}", twitch.name.to_lowercase())));
        }
        if (config.youtube.enabled || config.youtube_vertical.enabled) && youtube.connected {
            // The broadcast itself when there is one; otherwise the channel's live page.
            if !youtube.broadcast.is_empty() {
                links.push(("YouTube".to_owned(), format!("https://youtu.be/{}", youtube.broadcast)));
            } else if !youtube.user_id.is_empty() {
                links.push(("YouTube".to_owned(), format!("https://www.youtube.com/channel/{}/live", youtube.user_id)));
            }
        }
        for service in config.services.iter().filter(|s| s.destination.enabled) {
            match service.provider {
                crate::streaming::Provider::Kick if kick.connected && !kick.name.is_empty() => {
                    links.push(("Kick".to_owned(), format!("https://kick.com/{}", kick.name.to_lowercase())));
                }
                crate::streaming::Provider::TikTok if !config.tiktok_username.trim().is_empty() => {
                    links.push(("TikTok".to_owned(), format!("https://www.tiktok.com/@{}/live", config.tiktok_username.trim().trim_start_matches('@'))));
                }
                _ => {}
            }
        }
        links.dedup_by(|a, b| a.0 == b.0);
        let live = [twitch, youtube, kick].into_iter().filter(|a| a.connected);
        let title = live.clone().map(|a| a.title.trim()).find(|t| !t.is_empty()).unwrap_or_default().to_owned();
        let category = live
            .filter_map(|a| a.categories.iter().find(|c| c.id == a.category).map(|c| c.name.clone()))
            .find(|n| !n.is_empty())
            .unwrap_or_default();
        crate::announce::Stream { title, category, links }
    }
    /// Settings > Accounts > Announcements.
    fn announce_settings(&mut self, ui: &mut egui::Ui) {
        let before = self.stream_config.announce.clone();
        let preview_stream = {
            let mut s = self.announce_stream();
            if s.title.is_empty() {
                s.title = "Your stream title".into();
            }
            if s.links.is_empty() {
                s.links.push(("Twitch".into(), "https://twitch.tv/yourname".into()));
            }
            s
        };
        let mut test = None;
        ui.heading("Go-live announcements");
        note(ui, "When your stream starts, Rosorah posts this once to the places switched on below. You can skip any of them for one stream in the Go live checklist.");
        ui.add_space(14.);
        let a = &mut self.stream_config.announce;
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.strong("Message");
            ui.add(egui::TextEdit::multiline(&mut a.message).desired_rows(3).desired_width(f32::INFINITY));
            note(ui, "{title} your stream title · {category} the game or category · {links} where to watch (one per line) · {twitch} {youtube} {kick} {tiktok} a single link");
            ui.horizontal_wrapped(|ui| {
                if ui.small_button("Use the default message").clicked() {
                    a.message = crate::announce::DEFAULT_MESSAGE.into();
                }
            });
            ui.add_space(6.);
            ui.label(RichText::new("Image (optional)").small().strong());
            ui.horizontal(|ui| {
                if let Some((_, Some(texture))) = &self.announce_thumb {
                    let size = texture.size_vec2();
                    let scale = (120. / size.x).min(68. / size.y);
                    ui.add(egui::Image::new((texture.id(), size * scale)).corner_radius(6));
                }
                ui.vertical(|ui| {
                    if a.image.is_empty() {
                        note(ui, "A picture posted with the announcement: attached on Discord, the link card's thumbnail on Bluesky, and copied for pasting on X.");
                    } else {
                        note(ui, std::path::Path::new(&a.image).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                    }
                    ui.horizontal(|ui| {
                        if ui.add_enabled(self.announce_picker.is_none(), egui::Button::new("Choose image…")).clicked() {
                            let (tx, rx) = mpsc::channel();
                            self.announce_picker = Some(rx);
                            std::thread::spawn(move || {
                                let path = crate::desktop::pick(crate::desktop::Pick::Open, "Choose an announcement image (PNG or JPEG)", "").unwrap_or_else(|e| {
                                    crate::capture_log::event(e.to_string());
                                    None
                                });
                                let _ = tx.send(path);
                            });
                        }
                        if !a.image.is_empty() && ui.button("Remove").clicked() {
                            a.image.clear();
                        }
                    });
                });
            });
            ui.add_space(6.);
            ui.label(RichText::new("Preview").small().strong());
            let text = crate::announce::fill(&a.message, &preview_stream);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(&text);
            });
            let count = text.chars().count();
            if count > crate::announce::BLUESKY_LIMIT {
                ui.colored_label(ui.visuals().warn_fg_color, format!("{count} characters: Bluesky allows {}, so shorten it if you announce there.", crate::announce::BLUESKY_LIMIT));
            }
            let threads = crate::announce::threads_length(&text);
            if threads > crate::announce::THREADS_LIMIT {
                ui.colored_label(ui.visuals().warn_fg_color, format!("{threads} characters as Threads counts them (emoji count as several): Threads allows {}.", crate::announce::THREADS_LIMIT));
            }
            let x = crate::announce::x_length(&text);
            if a.x && x > crate::announce::X_LIMIT {
                ui.colored_label(ui.visuals().warn_fg_color, format!("{x} characters as X counts them (links count as 23): X allows {}, so you'd need to shorten it there before posting.", crate::announce::X_LIMIT));
            }
        });
        ui.add_space(12.);
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Discord").heading().color(Color32::from_rgb(114, 137, 218)));
            ui.checkbox(&mut a.discord, "Announce on Discord");
            ui.label("Webhook address");
            ui.add(egui::TextEdit::singleline(&mut a.discord_webhook).password(true).hint_text("https://discord.com/api/webhooks/…").desired_width(f32::INFINITY));
            note(ui, "In Discord: Server Settings → Integrations → Webhooks → New Webhook, choose the channel, then Copy Webhook URL. Add @everyone to the message to ping the server.");
            if !a.discord_webhook.trim().is_empty() && !crate::announce::valid_webhook(&a.discord_webhook) {
                ui.colored_label(ui.visuals().warn_fg_color, "That doesn't look like a Discord webhook address.");
            }
            if ui.add_enabled(crate::announce::valid_webhook(&a.discord_webhook), egui::Button::new("Send test")).on_hover_text("Posts the message to Discord now").clicked() {
                test = Some([true, false, false, false]);
            }
        });
        ui.add_space(12.);
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Bluesky").heading().color(Color32::from_rgb(32, 139, 254)));
            ui.checkbox(&mut a.bluesky, "Announce on Bluesky");
            ui.label("Handle");
            ui.add(egui::TextEdit::singleline(&mut a.bluesky_handle).hint_text("yourname.bsky.social").desired_width(f32::INFINITY));
            ui.label("App password");
            ui.add(egui::TextEdit::singleline(&mut a.bluesky_password).password(true).hint_text("xxxx-xxxx-xxxx-xxxx").desired_width(f32::INFINITY));
            ui.horizontal_wrapped(|ui| {
                note(ui, "Use an app password, not your main password: Bluesky → Settings → Privacy and security → App passwords.");
                if ui.small_button("Open").clicked() {
                    let _ = crate::desktop::open("https://bsky.app/settings/app-passwords");
                }
            });
            let ready = !a.bluesky_handle.trim().is_empty() && !a.bluesky_password.trim().is_empty();
            if ui.add_enabled(ready, egui::Button::new("Send test")).on_hover_text("Posts the message to Bluesky now").clicked() {
                test = Some([false, true, false, false]);
            }
        });
        ui.add_space(12.);
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Threads").heading());
            ui.checkbox(&mut a.threads, "Announce on Threads");
            ui.label("Access token");
            let token = a.threads_token.clone();
            ui.add(egui::TextEdit::singleline(&mut a.threads_token).password(true).hint_text("Paste the token from your Meta app").desired_width(f32::INFINITY));
            if a.threads_token != token {
                // A different token may be a different account, and its age is unknown.
                a.threads_expires = 0;
                a.threads_username.clear();
            }
            note(ui, "Threads posts need your own (free) Meta app: on developers.facebook.com, create an app with the \"Access the Threads API\" use case, add the threads_basic and threads_content_publish permissions, add your Threads account as a tester (accept the invite in Threads → Settings → Account → Website permissions), then generate an access token and paste it here.");
            ui.horizontal_wrapped(|ui| {
                if ui.small_button("Open Meta for Developers").clicked() {
                    let _ = crate::desktop::open("https://developers.facebook.com/apps/");
                }
            });
            note(ui, "Posts are text with a preview card for your stream link; the image isn't posted on Threads.");
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
            if !a.threads_username.is_empty() {
                note(ui, format!("Posting as @{}", a.threads_username));
            }
            if a.threads_expires > 0 {
                let day = gst::glib::DateTime::from_unix_local(a.threads_expires as i64).ok().and_then(|d| d.format("%e %B %Y").ok()).map(|d| d.trim().to_owned()).unwrap_or_default();
                if a.threads_expires <= now {
                    ui.colored_label(ui.visuals().error_fg_color, "This token has expired. Generate a new one and paste it here.");
                } else if a.threads_expires - now < 7 * 24 * 3600 {
                    ui.colored_label(ui.visuals().warn_fg_color, format!("This token runs out on {day}. Announce or send a test before then and Rosorah renews it."));
                } else {
                    note(ui, format!("Renewed automatically each time you announce; good until {day}."));
                }
            }
            if ui.add_enabled(!a.threads_token.trim().is_empty(), egui::Button::new("Send test")).on_hover_text("Posts the message to Threads now").clicked() {
                test = Some([false, false, true, false]);
            }
        });
        ui.add_space(12.);
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("X").heading());
            ui.checkbox(&mut a.x, "Open a post on X");
            note(ui, "X charges for posting through its API, so Rosorah opens the post in your browser instead, ready to send: you press Post. Sign in to X in your browser once. If you've chosen an image, it's copied too: press Ctrl+V in the post to add it.");
            if ui.button("Try it").on_hover_text("Opens the post on X in your browser now; nothing is posted until you press Post").clicked() {
                test = Some([false, false, false, true]);
            }
        });
        note(ui, "The webhook address, app password and Threads token are kept in your system's keyring with your stream keys.");
        if let Some(rx) = &self.announce_picker
            && let Ok(path) = rx.try_recv()
        {
            self.announce_picker = None;
            if let Some(path) = path {
                let path = path.display().to_string();
                // Checked now, so a file that can't be posted is caught here rather than at go-live.
                match crate::announce::load_picture(&path) {
                    Ok(_) => self.stream_config.announce.image = path,
                    Err(e) => self.tell(format!("{e:#}")),
                }
            }
        }
        // The thumbnail is made once per picture, small, and only while this page is open.
        let image = self.stream_config.announce.image.clone();
        if self.announce_thumb.as_ref().map(|(p, _)| p.as_str()) != Some(image.as_str()) {
            let texture = (!image.is_empty())
                .then(|| image::open(&image).ok())
                .flatten()
                .map(|picture| {
                    let small = picture.thumbnail(240, 240).to_rgba8();
                    let pixels = egui::ColorImage::from_rgba_unmultiplied([small.width() as usize, small.height() as usize], &small);
                    ui.ctx().load_texture("announcement-thumbnail", pixels, egui::TextureOptions::LINEAR)
                });
            self.announce_thumb = Some((image, texture));
        }
        if let Some(which) = test {
            self.announcer.send(&self.stream_config.announce, preview_stream, which, ui.ctx());
            self.tell("Sending a test announcement…".into());
        }
        if before != self.stream_config.announce {
            self.save_stream_settings();
        }
    }
    /// Moves a layer within its scene (front is last), keeping selections pointing at the same layers.
    fn move_layer(&mut self, si: usize, from: usize, to: usize) {
        let count = self.project.scenes[si].layers.len();
        if from == to || from >= count || to >= count || self.project.scenes[si].layers[from].locked {
            return;
        }
        let active = self.project.scenes[si].id;
        let mut origins: Vec<_> = (0..count).collect();
        let original = origins.remove(from);
        origins.insert(to, original);
        let moved = self.project.scenes[si].layers.remove(from);
        self.project.scenes[si].layers.insert(to, moved);
        let renumber = |index: usize| origins.iter().position(|old| *old == index);
        if let Some(selected) = self.selections.get_mut(&active) {
            *selected = selected.iter().filter_map(|i| renumber(*i)).collect();
        }
        if let Some((scene, index)) = self.crop_layer && scene == active {
            self.crop_layer = renumber(index).map(|i| (scene, i));
        }
        if let Some((scene, index)) = self.layer_properties && scene == active {
            self.layer_properties = renumber(index).map(|i| (scene, i));
        }
        if let Some((scene, index)) = self.canvas_menu && scene == active {
            self.canvas_menu = renumber(index).map(|i| (scene, i));
        }
        self.layer = self.layer.and_then(renumber);
        self.changed();
    }
    /// Saves the replay buffer to the recordings folder, in the background.
    fn save_replay(&mut self) {
        if !self.replay_on {
            self.tell("Turn the replay buffer on first; it saves the last seconds it has held.".into());
            return;
        }
        let folder = PathBuf::from(&self.record_folder);
        let stamp = gst::glib::DateTime::now_local().ok().and_then(|d| d.format("%Y-%m-%d %H-%M-%S").ok()).map(|d| d.to_string()).unwrap_or_else(|| "replay".into());
        let path = folder.join(format!("Replay {stamp}.mp4"));
        let (tx, rx) = mpsc::channel();
        self.replay_saving = Some(rx);
        std::thread::spawn(move || {
            let result = std::fs::create_dir_all(&folder).map_err(anyhow::Error::from).and_then(|_| crate::streaming::save_replay(&path));
            let _ = tx.send(result.map(|_| path));
        });
    }
    fn toggle_replay_buffer(&mut self) {
        self.replay_on = !self.replay_on;
        let mut replay = crate::streaming::REPLAY.lock().unwrap();
        replay.enabled = self.replay_on;
        replay.seconds = self.project.preferences.replay_seconds;
        drop(replay);
        if !self.replay_on
            && let Some(stream) = self.replay_stream.take() {
                stream.stop();
                self.replay_stopping.push(stream);
            }
    }
    /// Keeps the replay buffer fed: from the stream while live, otherwise from an encoder of its own.
    fn replay_tick(&mut self) {
        if let Some(rx) = &self.replay_saving
            && let Ok(result) = rx.try_recv()
        {
            self.replay_saving = None;
            match result {
                Ok(path) => {
                    self.tell(format!("Replay saved: {}", path.display()));
                    self.last_recording = Some(path);
                }
                Err(e) => self.tell(format!("{e:#}")),
            }
        }
        self.replay_stopping.retain(|s| s.view().result.is_none());
        crate::streaming::REPLAY.lock().unwrap().seconds = self.project.preferences.replay_seconds;
        if !self.replay_on {
            return;
        }
        if self.live_stream.is_some() {
            // The stream's encoder takes over; the replay-only one stops.
            if let Some(stream) = self.replay_stream.take() {
                stream.stop();
                self.replay_stopping.push(stream);
            }
        } else if self.replay_stream.is_none() && self.replay_stopping.is_empty() && self.replay_retry.elapsed() > Duration::from_secs(5) {
            self.replay_retry = Instant::now();
            match crate::streaming::Stream::start_replay_only(&self.project, 0, self.captures.reader(), self.stream_config.bitrate) {
                Ok(stream) => {
                    self.audio.streaming_outlets(stream.outlets());
                    self.replay_stream = Some(stream);
                }
                Err(e) => self.tell(format!("The replay buffer couldn't start: {e:#}")),
            }
        }
        if let Some(stream) = &self.replay_stream
            && let Some(Err(e)) = stream.view().result
        {
            self.tell(format!("The replay buffer stopped: {e}"));
            self.replay_stream = None;
        }
    }
    /// Replay buffer controls for the Stream panel.
    fn replay_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let mut on = self.replay_on;
            if ui.toggle_value(&mut on, "⏺ Replay").on_hover_text("Keeps the last seconds in memory so you can save a clip").changed() {
                self.toggle_replay_buffer();
            }
            if self.replay_on {
                if ui.add_enabled(self.replay_saving.is_none(), egui::Button::new("Save replay")).clicked() {
                    self.save_replay();
                }
                let held = crate::streaming::REPLAY.lock().unwrap().held();
                note(ui, format!("{held} s held"));
            }
            let mut seconds = self.project.preferences.replay_seconds;
            if ui.add(egui::DragValue::new(&mut seconds).range(5..=120).suffix(" s")).on_hover_text("How much the replay buffer keeps").changed() {
                self.project.preferences.replay_seconds = seconds;
                self.dirty = true;
            }
        });
        if self.replay_on && self.live_stream.is_none() {
            note(ui, "Not live, so the replay buffer runs its own encoder (about the cost of recording).");
        }
    }
    /// Starts or stops sending the main video to Rosorah's virtual camera.
    fn toggle_virtual_camera(&mut self) {
        if let Some(camera) = self.virtual_camera.take() {
            camera.stop();
            self.camera_stopping.push(camera);
            #[cfg(windows)]
            self.windows_camera.take();
            return;
        }
        let Some(device) = crate::virtual_camera::device() else {
            self.tell("Set up the virtual camera first (Stream panel).".into());
            return;
        };
        // Windows shows the camera to apps only while it's on; Rosorah hands it frames directly.
        #[cfg(windows)]
        let started = match rosorah_windows::virtual_camera::Camera::start() {
            Ok(camera) => {
                let sink = camera.sink();
                self.windows_camera = Some(camera);
                let _ = device;
                crate::recording::Recording::start_windows_camera(&self.project, 0, self.captures.reader(), sink)
            }
            Err(e) => {
                self.tell(format!("The virtual camera couldn't start: {e:#}"));
                return;
            }
        };
        #[cfg(not(windows))]
        let started = crate::recording::Recording::start_camera(&self.project, 0, self.captures.reader(), device);
        match started {
            Ok(camera) => {
                self.virtual_camera = Some(camera);
                self.tell("Virtual camera on: pick “Rosorah Virtual Camera” in Discord, Zoom or your browser.".into());
            }
            Err(e) => {
                #[cfg(windows)]
                self.windows_camera.take();
                self.tell(format!("The virtual camera couldn't start: {e:#}"));
            }
        }
    }
    /// NDI and Spout output switches for the Stream panel: the main video, as "Rosorah Studio".
    /// On Linux, Spout output reaches Windows apps already running in Wine or Proton.
    fn shared_output_controls(&mut self, ui: &mut egui::Ui) {
        let mut stopped = Vec::new();
        if let Some(Err(e)) = self.ndi_output.as_ref().and_then(|o| o.view().result) {
            stopped.push(format!("NDI output stopped: {e}"));
            self.ndi_output = None;
            self.audio.shared_outlet(None);
        }
        if let Some(Err(e)) = self.spout_output.as_ref().and_then(|o| o.view().result) {
            stopped.push(format!("Spout output stopped: {e}"));
            self.spout_output = None;
        }
        for message in stopped {
            self.tell(message);
        }
        ui.horizontal_wrapped(|ui| {
            let mut on = self.ndi_output.is_some();
            if ui.toggle_value(&mut on, "📡 NDI output").on_hover_text("Sends the main video and sound to your network as “Rosorah Studio”, for OBS, vMix or another PC").changed() {
                if let Some(output) = self.ndi_output.take() {
                    output.stop();
                    self.audio.shared_outlet(None);
                } else {
                    match crate::recording::Recording::start_ndi(&self.project, 0, self.captures.reader(), "Rosorah Studio".into()) {
                        Ok(output) => {
                            self.audio.shared_outlet(Some(output.outlet()));
                            self.ndi_output = Some(output);
                        }
                        Err(e) => self.tell(format!("{e:#}")),
                    }
                }
            }
            {
                let mut on = self.spout_output.is_some();
                let hint = if cfg!(windows) {
                    "Shares the main video with apps on this PC as the Spout sender “Rosorah Studio”"
                } else {
                    "Shares the main video as the Spout sender “Rosorah Studio” with Windows apps running in Wine or Proton (start them first)"
                };
                if ui.toggle_value(&mut on, "🔀 Spout output").on_hover_text(hint).changed() {
                    if let Some(output) = self.spout_output.take() {
                        output.stop();
                    } else {
                        match crate::recording::Recording::start_spout(&self.project, 0, self.captures.reader(), "Rosorah Studio".into()) {
                            Ok(output) => self.spout_output = Some(output),
                            Err(e) => self.tell(format!("{e:#}")),
                        }
                    }
                }
            }
        });
    }
    /// The sender picker for NDI and Spout layers, with what the source is doing now.
    fn shared_source_controls(&mut self, ui: &mut egui::Ui, id: Id) {
        let Some((ndi, current)) = self.project.source(id).and_then(|s| match &s.kind {
            SourceKind::Ndi(n) => Some((true, n.sender.clone())),
            SourceKind::Spout(n) => Some((false, n.sender.clone())),
            _ => None,
        }) else {
            return;
        };
        let stale = self.shared_senders.get(&ndi).is_none_or(|(at, _)| at.elapsed() > Duration::from_secs(2));
        if stale {
            let found = if ndi {
                crate::ndi::senders().map_err(|e| e.to_string())
            } else {
                #[cfg(windows)]
                let found = Ok(rosorah_windows::spout::senders());
                #[cfg(not(windows))]
                let found = crate::spout_bridge::senders().map_err(|e| e.to_string());
                found
            };
            self.shared_senders.insert(ndi, (Instant::now(), found));
            ui.ctx().request_repaint_after(Duration::from_secs(2));
        }
        let mut chosen = None;
        match &self.shared_senders[&ndi].1 {
            Err(e) => {
                ui.colored_label(crate::appearance::current().error, e);
                if ndi {
                    ui.hyperlink_to("Get NDI Tools (free)", crate::ndi::GET_NDI);
                }
            }
            Ok(list) => {
                ui.horizontal_wrapped(|ui| {
                    ui.label(if ndi { "NDI sender" } else { "Spout sender" });
                    let shown = if current.is_empty() { "Choose…".to_owned() } else { current.clone() };
                    egui::ComboBox::from_id_salt(("shared-sender", id)).selected_text(shown).show_ui(ui, |ui| {
                        if list.is_empty() {
                            ui.weak(if ndi { "Looking on your network…" } else if cfg!(windows) { "No app is sending with Spout" } else { "No Windows app in Wine or Proton is sending with Spout" });
                        }
                        for name in list {
                            if ui.selectable_label(*name == current, name).clicked() {
                                chosen = Some(name.clone());
                            }
                        }
                    });
                });
                note(ui, if ndi {
                    "Senders on your network appear here within a few seconds."
                } else if cfg!(windows) {
                    "Turn on Spout output in the other app (VTube Studio, Warudo, TouchDesigner…), and it appears here."
                } else {
                    "Start the Windows app in Wine or Proton (Steam, Lutris, Bottles…) and turn on its Spout output; it appears here within a few seconds."
                });
            }
        }
        if !current.is_empty() {
            note(ui, self.captures.camera_status(id));
        }
        if let Some(name) = chosen
            && let Some(source) = self.project.sources.iter_mut().find(|s| s.id == id)
        {
            if let SourceKind::Ndi(n) | SourceKind::Spout(n) = &mut source.kind {
                n.sender = name.clone();
            }
            if source.name == "NDI" || source.name == "Spout" {
                source.name = name;
            }
            self.changed();
        }
    }
    /// Virtual camera controls for the Stream panel.
    fn virtual_camera_controls(&mut self, ui: &mut egui::Ui) {
        self.camera_stopping.retain(|c| c.view().result.is_none());
        if let Some(camera) = &self.virtual_camera
            && let Some(Err(e)) = camera.view().result
        {
            self.tell(format!("The virtual camera stopped: {e}"));
            self.virtual_camera = None;
            #[cfg(windows)]
            self.windows_camera.take();
        }
        ui.horizontal_wrapped(|ui| {
            let ready = crate::virtual_camera::device().is_some();
            let mut on = self.virtual_camera.is_some();
            if ready && self.virtual_camera.is_none() && crate::virtual_camera::outdated() {
                if ui.button("📷 Update virtual camera").on_hover_text("This Rosorah has a newer camera component (Windows asks for administrator permission)").clicked() {
                    match crate::virtual_camera::set_up() {
                        Ok(()) => self.tell("Virtual camera updated.".into()),
                        Err(e) => self.tell(format!("{e:#}")),
                    }
                }
            } else if ready {
                #[allow(unused_mut)]
                let mut hint = "Rosorah's main video as a webcam for Discord, Zoom or a browser".to_owned();
                // What Windows is doing with it, to tell "no app reading" from "frames not arriving".
                #[cfg(windows)]
                if let Some(camera) = &self.windows_camera {
                    let (frames, readers) = camera.sink().stats();
                    hint += &format!("\n{frames} frames sent · {readers} Windows reader(s) connected");
                }
                if ui.toggle_value(&mut on, "📷 Virtual camera").on_hover_text(hint).changed() {
                    self.toggle_virtual_camera();
                }
            } else if crate::virtual_camera::supported() {
                let hint = if cfg!(windows) { "Adds “Rosorah Virtual Camera” (Windows asks for administrator permission once)" } else { "Adds “Rosorah Virtual Camera” (asks for your password once; lasts until restart)" };
                if ui.button("📷 Set up virtual camera").on_hover_text(hint).clicked() {
                    match crate::virtual_camera::set_up() {
                        Ok(()) => self.tell("Virtual camera added. Switch it on whenever you want.".into()),
                        Err(e) => self.tell(format!("{e:#}")),
                    }
                }
            } else {
                note(ui, if cfg!(windows) { "Virtual camera: needs Windows 11." } else { "Virtual camera: install v4l2loopback (Fedora: sudo dnf install v4l2loopback; Ubuntu: sudo apt install v4l2loopback-dkms)." });
            }
        });
    }
    /// Whether Studio Mode applies to `canvas` (the main, landscape video).
    fn studio_mode_for(&self, canvas: &auroracast_core::Canvas) -> bool {
        self.project.preferences.workspace.studio_mode && self.project.canvases.first().is_some_and(|c| c.id == canvas.id) && canvas.width >= canvas.height
    }
    /// Sends the prepared scene live with the usual transition.
    fn studio_transition(&mut self) {
        let Some(canvas) = self.project.canvases.first().map(|c| c.id) else { return };
        if let Some(scene) = self.studio_preview.filter(|s| self.project.scene(*s).is_some()) {
            self.studio_taking = true;
            self.request_scene(canvas, scene);
            self.studio_taking = false;
        }
    }
    /// Studio Mode's Program view: what viewers see, drawn up to 15 times a second.
    fn program_view(&mut self, ui: &mut egui::Ui) {
        let Some(canvas) = self.project.canvases.first().cloned() else { return };
        ui.horizontal(|ui| {
            ui.label(RichText::new("● PROGRAM").small().strong().color(crate::appearance::current().live));
            let ready = self.studio_preview.is_some_and(|s| s != canvas.scene);
            if ui.add_enabled(ready, self.primary_button("Transition ▶")).on_hover_text("Send the preview live").on_disabled_hover_text("Pick another scene to prepare it first").clicked() {
                self.studio_transition();
            }
        });
        let width = ui.available_width().max(1.);
        let height = (ui.available_height() - 8.).max(1.);
        let scale = (width / canvas.width as f32).min(height / canvas.height as f32);
        let size = Vec2::new(canvas.width as f32 * scale, canvas.height as f32 * scale);
        let dims = [(size.x as u32).clamp(16, 4096), (size.y as u32).clamp(16, 4096)];
        if self.program_drawn.elapsed() >= Duration::from_millis(66)
            && let (Some(gpu), Some(state)) = (&mut self.gpu, &self.render_state)
            && let Ok(view) = gpu.render_gpu(&self.project, &canvas, dims[0], dims[1])
        {
            self.program_drawn = Instant::now();
            let mut renderer = state.renderer.write();
            match self.program_texture {
                Some(id) => renderer.update_egui_texture_from_wgpu_texture(&state.device, &view, eframe::wgpu::FilterMode::Linear, id),
                None => self.program_texture = Some(renderer.register_native_texture(&state.device, &view, eframe::wgpu::FilterMode::Linear)),
            }
        }
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
        let target = egui::Rect::from_center_size(rect.center(), size);
        if let Some(texture) = self.program_texture {
            ui.painter().image(texture, target, egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.)), Color32::WHITE);
            ui.painter().rect_stroke(target, 2., egui::Stroke::new(2.0_f32, crate::appearance::current().live), egui::StrokeKind::Outside);
        }
        ui.ctx().request_repaint_after(Duration::from_millis(66));
    }
    fn panel_title(&self, panel: Panel) -> String {
        match panel {
            Panel::Web(id) => self.project.preferences.workspace.browser_docks.iter().find(|d| d.id == id).map_or_else(|| panel.title().into(), |d| d.name.clone()),
            _ => panel.title().into(),
        }
    }
    /// Renames a sound strip. Only the name shown changes; the sound keeps its own name underneath.
    fn sound_rename_ui(&mut self, ctx: &egui::Context) {
        let Some((id, mut name)) = self.sound_rename.take() else { return };
        let mut done = false;
        let mut save = false;
        pop_modal(ctx, egui::Id::new("sound-rename")).show(ctx, |ui| {
            ui.set_width(320.);
            ui.heading("Rename sound");
            ui.add_space(6.);
            let field = ui.add(egui::TextEdit::singleline(&mut name).desired_width(f32::INFINITY));
            if !field.has_focus() && !field.lost_focus() {
                field.request_focus();
            }
            save |= field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.add_space(8.);
            ui.horizontal(|ui| {
                save |= ui.button("Save").clicked();
                done |= ui.button("Cancel").clicked();
            });
        });
        if save {
            let original = self.project.audio.iter().find(|c| c.id == id).map(|c| c.name.clone());
            let trimmed: String = name.trim().chars().take(40).collect();
            let names = &mut self.project.preferences.workspace.sound_names;
            if trimmed.is_empty() || Some(&trimmed) == original.as_ref() {
                names.remove(&id);
            } else {
                names.insert(id, trimmed);
            }
            self.dirty = true;
            done = true;
        }
        if !done && !ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.sound_rename = Some((id, name));
        }
    }
    /// Asks for a name and address, then adds the page as a panel.
    fn web_dock_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some((mut name, mut url)) = self.web_dock_dialog.take() else {
            return;
        };
        let mut done = false;
        let valid = crate::browser_source::valid_url(url.trim());
        pop_modal(ctx, egui::Id::new("web-dock-dialog")).show(ctx, |ui| {
            ui.set_width(380.);
            ui.heading("Add a web page");
            ui.add_space(6.);
            ui.label("Name");
            ui.add(egui::TextEdit::singleline(&mut name).hint_text("e.g. Stream dashboard").desired_width(f32::INFINITY));
            ui.label("Address");
            ui.add(egui::TextEdit::singleline(&mut url).desired_width(f32::INFINITY));
            note(ui, "It opens as a panel you can move and group like the others. Sign-ins on these pages are remembered.");
            ui.add_space(10.);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    done = true;
                }
                if ui.add_enabled(valid, egui::Button::new("Add")).clicked() {
                    let docks = &mut self.project.preferences.workspace.browser_docks;
                    let id = docks.iter().map(|d| d.id).max().unwrap_or(0) + 1;
                    let url = url.trim().to_owned();
                    let name = match name.trim() {
                        "" => reqwest::Url::parse(&url).ok().and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_owned())).unwrap_or_else(|| "Web page".into()),
                        n => n.chars().take(40).collect(),
                    };
                    docks.push(auroracast_core::workspace::BrowserDock { id, name, url });
                    self.workspace.set_visible(Panel::Web(id), true);
                    self.dirty = true;
                    done = true;
                }
            });
        });
        if !done && !ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.web_dock_dialog = Some((name, url));
        }
    }
    fn scene_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some((id, mut name, delete)) = self.scene_dialog.clone() else {
            return;
        };
        let mut open = true;
        let mut done = false;
        // Front and center, with the studio dimmed behind it, so the question can't be missed.
        let modal = pop_modal(ctx, egui::Id::new("scene-dialog")).show(ctx, |ui| {
            ui.set_width(340.);
            ui.heading(if delete { "Delete scene" } else { "Rename scene" });
            ui.add_space(6.);
            if delete {
                ui.label(format!("Delete “{name}” and remove its scene links?"));
            } else {
                ui.text_edit_singleline(&mut name);
            }
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    done = true;
                }
                if ui
                    .add_enabled(
                        delete || !name.trim().is_empty(),
                        egui::Button::new(if delete { "Delete" } else { "Save" }),
                    )
                    .clicked()
                {
                    if delete {
                        let owner = self
                            .project
                            .canvases
                            .iter()
                            .find(|c| self.project.canvas_scenes(c.id).contains(&id))
                            .map(|c| c.id);
                        if let Some(owner) = owner {
                            self.project.delete_scene(owner, id);
                            self.layer = None;
                            self.deleted(format!("Deleted the scene “{name}”"));
                        }
                    } else if let Some(s) = self.project.scenes.iter_mut().find(|s| s.id == id) {
                        s.name = name.trim().chars().take(80).collect();
                    }
                    self.changed();
                    done = true;
                }
            });
        });
        let _ = modal;
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            open = false;
        }
        // Enter answers a delete question, as it does for layers.
        if delete && !done && open && ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
            let owner = self.project.canvases.iter().find(|c| self.project.canvas_scenes(c.id).contains(&id)).map(|c| c.id);
            if let Some(owner) = owner {
                self.project.delete_scene(owner, id);
                self.layer = None;
                self.deleted(format!("Deleted the scene “{name}”"));
                self.changed();
            }
            done = true;
        }
        self.scene_dialog = if open && !done {
            Some((id, name, delete))
        } else {
            None
        };
    }
    fn sources_panel(&mut self, ui: &mut egui::Ui) {
        self.keep_selection.push(ui.max_rect());
        self.canvas_tabs(ui);
        let active = self.project.canvases[self.canvas].scene;
        let si = self
            .project
            .scenes
            .iter()
            .position(|s| s.id == active)
            .unwrap();
        let count = self.project.scenes[si].layers.len();
        let mut dropped = None;
        let animate = self.project.preferences.workspace.animations;
        // Rows run top to bottom from the frontmost layer, which is last in the scene.
        let moved = sortable_rows(ui, egui::Id::new(("layer-rows", active)), count, 34., animate, |ui, p| {
            let i = count - 1 - p;
            let layer = &self.project.scenes[si].layers[i];
            let (visible, locked) = (layer.visible, layer.locked);
            let name = self.project.source(layer.source).map(|s| s.name.clone()).unwrap_or_default();
            ui.spacing_mut().button_padding = egui::vec2(5., 3.);
            ui.spacing_mut().item_spacing = egui::vec2(4., 2.);
            ui.horizontal(|ui| {
                // A large, bright eye; a hidden layer's is dim and struck through.
                let eye = egui::Button::new(if visible { RichText::new("👁").size(19.).color(ui.visuals().strong_text_color()) } else { RichText::new("👁").size(19.).color(muted().gamma_multiply(0.7)).strikethrough() }).frame(false);
                if ui.add_sized([30., 26.], eye).on_hover_text(if visible { "Showing · click to hide" } else { "Hidden · click to show" }).clicked() {
                    self.project.scenes[si].layers[i].visible = !visible;
                    self.changed();
                }
                let label = if locked { format!("🔒 {name}") } else { name.clone() };
                let selected = self.selections.get(&active).is_some_and(|v| v.contains(&i));
                // On the accent-coloured selection, text is black (white if the accent itself is black).
                let color = match (selected, visible) {
                    (true, true) => on_accent(self.accent()),
                    (true, false) => on_accent(self.accent()).gamma_multiply(0.6),
                    (false, true) => ui.visuals().text_color(),
                    (false, false) => muted(),
                };
                let response = ui
                    .add_sized(
                        [ui.available_width().max(30.), 26.],
                        egui::Button::new(RichText::new(label).color(color))
                            .sense(if locked { egui::Sense::click() } else { egui::Sense::click_and_drag() })
                            .selected(selected)
                            .truncate(),
                    )
                    .on_hover_text(if locked { "Locked · right-click to unlock" } else { "Drag to reorder (top appears in front) · double-click to change · right-click for more" });
                if !locked {
                    response.clone().on_hover_cursor(grab_cursor());
                }
                if response.clicked() {self.select_layer(active,i,ui.input(|i|i.modifiers.shift));}
                if response.double_clicked() {
                    self.select_layer(active, i, false);
                    self.layer_properties = Some((active, i));
                }
                response.context_menu(|ui| self.layer_menu(ui, si, i, &name, &mut dropped));
                response
            })
            .inner
        });
        if let Some((from, to)) = moved {
            dropped = Some((count - 1 - from, count - 1 - to));
        }
        if let Some((from, to)) = dropped {
            self.move_layer(si, from, to);
        }
        if count == 0 {
            card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label("This scene is empty. Start by adding what your viewers should see.");
                ui.horizontal_wrapped(|ui| {
                    if ui.add(self.primary_button("Add your game or screen")).clicked() {
                        let id = self.add_source_layer(si, "Game or screen", SourceKind::ScreenCapture, [0., 0., 1., 1.]);
                        #[cfg(windows)] self.captures.set_mode(id,rosorah_capture::Target::AutoFullscreen);
                        #[cfg(not(windows))] self.captures.start(id);
                        self.remember_capture_mode(id);
                    }
                    if ui.button("Add a camera").clicked() {
                        let c = &self.project.canvases[self.canvas];
                        let w = 0.28;
                        let rect = [0.68, 0.05, w, w * c.width as f32 / c.height as f32 * 9. / 16.];
                        self.add_source_layer(si, "Camera", SourceKind::VideoCamera(Default::default()), rect);
                        self.layer_properties = Some((self.project.scenes[si].id, self.project.scenes[si].layers.len() - 1));
                    }
                });
            });
            ui.add_space(6.);
        }
        if ui.add(self.primary_button("+ Add")).clicked() {
            self.add_dialog = Some((self.project.scenes[si].id, AddView::Main));
            self.add_search.clear();
        }
    }
    /// Layers from other scenes that can be shown here too, newest first, with what they are and
    /// where they're used. Nested scenes are left out; add those as a scene instead.
    fn reusable(&self, si: usize) -> Vec<(Id, String, &'static str, &'static str, String)> {
        let here = &self.project.scenes[si];
        let mut items: Vec<_> = self
            .project
            .sources
            .iter()
            .filter(|s| !matches!(s.kind, SourceKind::Scene(_)) && !here.layers.iter().any(|l| l.source == s.id))
            .filter_map(|s| {
                let scenes: Vec<_> = self.project.scenes.iter().filter(|sc| sc.layers.iter().any(|l| l.source == s.id)).map(|sc| sc.name.as_str()).collect();
                let (icon, kind) = source_kind_label(&s.kind);
                (!scenes.is_empty()).then(|| (s.id, s.name.clone(), icon, kind, format!("In {}", scenes.join(", "))))
            })
            .collect();
        // Ids only grow, so the newest additions come first.
        items.sort_by(|a, b| b.0.cmp(&a.0));
        items
    }
    /// Scenes that can be shown inside `scene`: not itself, and none that already contain it.
    fn nestable(&self, scene: Id) -> Vec<(Id, String)> {
        self.project.scenes.iter().filter(|s| s.id != scene && !self.project.scene_contains(s.id, scene)).map(|s| (s.id, s.name.clone())).collect()
    }
    /// Adds what was chosen in the Add dialog to scene `si`.
    fn add_choice(&mut self, si: usize, choice: AddChoice) {
        let scene = self.project.scenes[si].id;
        match choice {
            AddChoice::Capture => {
                let id = self.add_source_layer(si, "Window / Screen", SourceKind::ScreenCapture, [0., 0., 1., 1.]);
                #[cfg(windows)] self.captures.set_mode(id,rosorah_capture::Target::AutoFullscreen);
                #[cfg(not(windows))] self.captures.start(id);
                self.remember_capture_mode(id);
            }
            AddChoice::Camera => {
                let c = &self.project.canvases[self.canvas];
                let w = 0.28;
                let rect = [0.68, 0.05, w, w * c.width as f32 / c.height as f32 * 9. / 16.];
                self.add_source_layer(si, "Camera", SourceKind::VideoCamera(Default::default()), rect);
                self.layer_properties = Some((scene, self.project.scenes[si].layers.len() - 1));
            }
            AddChoice::Ndi | AddChoice::Spout => {
                let ndi = matches!(choice, AddChoice::Ndi);
                let (name, kind) = if ndi { ("NDI", SourceKind::Ndi(Default::default())) } else { ("Spout", SourceKind::Spout(Default::default())) };
                let id = self.add_source_layer(si, name, kind, [0.1, 0.1, 0.5, 0.5]);
                // NDI carries sound too; it gets a mixer strip like a media file.
                if ndi {
                    self.project.add_media_audio(id, name.into());
                }
                self.layer_properties = Some((scene, self.project.scenes[si].layers.len() - 1));
            }
            // Choosing a file adds it straight away; there's no separate Add step.
            AddChoice::Media => self.pick_media(scene, MediaPick::File, "Choose a video, GIF or sound"),
            AddChoice::Image => self.pick_media(scene, MediaPick::File, "Choose an image"),
            AddChoice::Slideshow => self.pick_media(scene, MediaPick::Slideshow, "Choose a folder of pictures"),
            AddChoice::Playlist => self.pick_media(scene, MediaPick::Playlist, "Choose a folder of videos"),
            AddChoice::Scene(other) => {
                if let Some(name) = self.project.scene(other).map(|s| s.name.clone()) {
                    self.add_source_layer(si, &name, SourceKind::Scene(other), [0., 0., 1., 1.]);
                }
            }
            AddChoice::Browser => {
                let id = self.add_source_layer(si, "Browser source", SourceKind::Browser { url: String::new() }, [0., 0., 1., 1.]);
                self.project.preferences.browser_sources.insert(id, Default::default());
                self.layer_properties = Some((scene, self.project.scenes[si].layers.len() - 1));
            }
            AddChoice::Color => {
                self.add_source_layer(si, "Color source", SourceKind::Color([70, 170, 220, 255]), [0.1, 0.1, 0.3, 0.3]);
            }
            AddChoice::Widget(kind) => self.add_widget(kind),
            AddChoice::Reuse(id) => {
                // Same place and look as where it's already used, so a corner camera stays in its corner.
                let mut layer = self.project.scenes.iter().flat_map(|s| &s.layers).find(|l| l.source == id).cloned().unwrap_or(Layer { crop: [0.; 4], group: 0, frame: Default::default(), look: Default::default(), source: id, rect: [0., 0., 1., 1.], visible: true, locked: false, opacity: 1. });
                layer.group = 0;
                layer.visible = true;
                layer.locked = false;
                self.project.scenes[si].layers.push(layer);
                let index = self.project.scenes[si].layers.len() - 1;
                self.layer = Some(index);
                self.selections.insert(scene, vec![index]);
                self.changed();
            }
        }
    }
    /// The Add dialog: tiles for new layers, then layers from other scenes with a live picture of each.
    fn add_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some((scene, view)) = self.add_dialog else {
            return;
        };
        let Some(si) = self.project.scenes.iter().position(|s| s.id == scene) else {
            self.add_dialog = None;
            self.reuse_previews.clear();
            return;
        };
        let all = self.reusable(si);
        let any = !all.is_empty();
        let search = self.add_search.trim().to_lowercase();
        let items: Vec<_> = all.into_iter().filter(|(_, name, ..)| search.is_empty() || name.to_lowercase().contains(&search)).take(24).collect();
        let others = self.nestable(scene);
        if view == AddView::Main {
            self.refresh_previews(ctx, &items);
        }
        let mut chosen = None;
        let mut next_view = None;
        let mut close = false;
        let modal = pop_modal(ctx, egui::Id::new("add-dialog")).show(ctx, |ui| {
            // Five tiles to a row on most screens.
            let width = ui.ctx().content_rect().width().min(888.) - 40.;
            ui.set_width(width);
            ui.set_max_width(width);
            match view {
                AddView::Main => {
                    ui.heading(format!("Add to {}", self.project.scenes[si].name));
                    ui.add_space(6.);
                    egui::ScrollArea::vertical().max_height(ui.ctx().content_rect().height() * 0.7).show(ui, |ui| {
                        ui.strong("New");
                        ui.add_space(4.);
                        tile_rows(ui, |ui| {
                            for (icon, title, hint, choice) in [
                                ("🖥", "Game, window or screen", "Capture what you're playing", Some(AddChoice::Capture)),
                                ("🎞", "Media file", "Video, GIF or sound", Some(AddChoice::Media)),
                                ("🖼", "Image", "A picture or logo", Some(AddChoice::Image)),
                                ("🗐", "Image slideshow", "Every picture in a folder", Some(AddChoice::Slideshow)),
                                ("▶", "Video playlist", "Every video in a folder", Some(AddChoice::Playlist)),
                                ("🌐", "Browser source", "A web page or overlay", Some(AddChoice::Browser)),
                                ("✏", "Text", "Words in any font", Some(AddChoice::Widget(widgets::Kind::Text))),
                                ("🎨", "Colour", "A solid colour block", Some(AddChoice::Color)),
                                ("🔔", "Alerts", "Follows, subs and donations", Some(AddChoice::Widget(widgets::Kind::AlertBox))),
                                ("⚙", "Widgets", "Chat, goals, timers and more", None),
                            ] {
                                if add_tile(ui, icon, title, hint, true) {
                                    match choice {
                                        Some(c) => chosen = Some(c),
                                        None => next_view = Some(AddView::Widgets),
                                    }
                                }
                                if title == "Game, window or screen" && add_tile(ui, "📷", "Capture devices", "Camera, NDI or Spout", true) {
                                    next_view = Some(AddView::Devices);
                                }
                            }
                            if add_tile(ui, "🗖", "Scene", "Another scene as one layer", !others.is_empty()) {
                                next_view = Some(AddView::Scenes);
                            }
                        });
                        if any {
                            ui.add_space(12.);
                            ui.horizontal(|ui| {
                                ui.strong("Use again");
                                note(ui, "from your other scenes, newest first");
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.add(egui::TextEdit::singleline(&mut self.add_search).hint_text("Search").desired_width(160.));
                                });
                            });
                            ui.add_space(4.);
                            if items.is_empty() {
                                note(ui, "Nothing matches that search.");
                            }
                            tile_rows(ui, |ui| {
                                for (id, name, icon, kind, place) in &items {
                                    if self.reuse_card(ui, *id, name, icon, kind, place) {
                                        chosen = Some(AddChoice::Reuse(*id));
                                    }
                                }
                            });
                            note(ui, "A reused layer is the same capture, camera or file: changing its settings changes it in every scene.");
                        }
                    });
                }
                AddView::Widgets => {
                    if ui.button("← Back").clicked() {
                        next_view = Some(AddView::Main);
                    }
                    ui.heading("Widgets");
                    ui.add_space(6.);
                    egui::ScrollArea::vertical().max_height(ui.ctx().content_rect().height() * 0.7).show(ui, |ui| {
                        tile_rows(ui, |ui| {
                            for kind in widgets::Kind::ALL.into_iter().filter(|k| !matches!(k, widgets::Kind::AlertBox | widgets::Kind::Text)) {
                                if add_tile(ui, "⚙", kind.label(), "", true) {
                                    chosen = Some(AddChoice::Widget(kind));
                                }
                            }
                        });
                    });
                }
                AddView::Devices => {
                    if ui.button("← Back").clicked() {
                        next_view = Some(AddView::Main);
                    }
                    ui.heading("Capture devices");
                    ui.add_space(6.);
                    tile_rows(ui, |ui| {
                        if add_tile(ui, "📷", "Camera", "Webcam, USB camera or DroidCam", true) {
                            chosen = Some(AddChoice::Camera);
                        }
                        if add_tile(ui, "📡", "NDI", "Video and sound from another app or PC on your network", true) {
                            chosen = Some(AddChoice::Ndi);
                        }
                        let spout = if cfg!(windows) { "Video from another app on this PC (VTube Studio, Warudo…)" } else { "Video from Windows apps running in Wine or Proton" };
                        if add_tile(ui, "🔀", "Spout", spout, true) {
                            chosen = Some(AddChoice::Spout);
                        }
                    });
                }
                AddView::Scenes => {
                    if ui.button("← Back").clicked() {
                        next_view = Some(AddView::Main);
                    }
                    ui.heading("Show another scene");
                    note(ui, "It appears here as one layer. Scenes that already contain this one are left out.");
                    ui.add_space(6.);
                    tile_rows(ui, |ui| {
                        for (id, name) in &others {
                            if add_tile(ui, "🗖", name, "", true) {
                                chosen = Some(AddChoice::Scene(*id));
                            }
                        }
                    });
                }
            }
            ui.add_space(8.);
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
        if modal.should_close() {
            close = true;
        }
        if let Some(view) = next_view {
            self.add_dialog = Some((scene, view));
        }
        if let Some(choice) = chosen {
            self.add_choice(si, choice);
            close = true;
        }
        if close {
            self.add_dialog = None;
            self.reuse_previews.clear();
        }
    }
    /// Updates the Use again pictures: once a second from running captures, cameras, pages and
    /// widgets; once for pictures on disk.
    fn refresh_previews(&mut self, ctx: &egui::Context, items: &[(Id, String, &'static str, &'static str, String)]) {
        let frames: std::collections::BTreeMap<_, _> = self.captures.reader().latest().into_iter().collect();
        for (id, ..) in items {
            let image = match self.project.source(*id).map(|s| &s.kind) {
                Some(SourceKind::Image { path }) => Some(path.clone()),
                _ => None,
            };
            if self.reuse_previews.get(id).is_some_and(|(_, at)| image.is_some() || at.elapsed() < Duration::from_secs(1)) {
                continue;
            }
            let picture = if let Some(path) = image {
                image::open(&path).ok().map(|p| {
                    let small = p.thumbnail(320, 180).to_rgba8();
                    egui::ColorImage::from_rgba_unmultiplied([small.width() as usize, small.height() as usize], &small)
                })
            } else if let Some(Some(frame)) = frames.get(id) {
                preview_picture(frame, 320, 180)
            } else {
                None
            };
            if let Some(picture) = picture {
                let texture = ctx.load_texture(format!("add-preview-{id}"), picture, egui::TextureOptions::LINEAR);
                self.reuse_previews.insert(*id, (texture, std::time::Instant::now()));
            }
        }
        ctx.request_repaint_after(Duration::from_secs(1));
    }
    /// One Use again card: its picture (or an icon), name, kind and the scenes it's in.
    fn reuse_card(&self, ui: &mut egui::Ui, id: Id, name: &str, icon: &str, kind: &str, place: &str) -> bool {
        let response = ui
            .allocate_ui_with_layout(egui::vec2(160., 150.), egui::Layout::top_down(egui::Align::Min), |ui| tile_frame(ui).show(ui, |ui| {
                ui.set_width(136.);
                let size = egui::vec2(136., 76.5);
                let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                if let Some((texture, _)) = self.reuse_previews.get(&id) {
                    let t = texture.size_vec2();
                    let scale = (size.x / t.x).min(size.y / t.y);
                    // A dark backdrop behind see-through widgets and pages.
                    ui.painter().rect_filled(rect, 8, Color32::from_gray(24));
                    let picture = egui::Rect::from_center_size(rect.center(), t * scale);
                    ui.painter().image(texture.id(), picture, egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.)), Color32::WHITE);
                } else {
                    ui.painter().rect_filled(rect, 8, Color32::from_black_alpha(90));
                    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(26.), muted());
                }
                ui.add_space(4.);
                ui.add(egui::Label::new(RichText::new(name).strong()).truncate());
                ui.add(egui::Label::new(RichText::new(format!("{kind} · {place}")).small().color(muted())).truncate());
            }))
            .inner
            .response
            .interact(egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.hovered() {
            ui.painter().rect_stroke(response.rect, 12, egui::Stroke::new(2.0_f32, ui.visuals().selection.bg_fill), egui::StrokeKind::Inside);
        }
        response.clicked()
    }
    /// Opens the file (or folder) chooser; whatever is picked is added to `scene` straight away.
    fn pick_media(&mut self, scene: Id, kind: MediaPick, title: &'static str) {
        if self.media_picker.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.media_picker = Some((scene, kind, rx));
        let mode = if kind == MediaPick::File { crate::desktop::Pick::Open } else { crate::desktop::Pick::Folder };
        std::thread::spawn(move || {
            let path = crate::desktop::pick(mode, title, "").unwrap_or_else(|e| {
                crate::capture_log::event(e.to_string());
                None
            });
            let _ = tx.send(path);
        });
    }
    /// Adds a picked file as a layer: pictures as images, anything else (video, GIF, sound) as media.
    fn add_media_file(&mut self, scene: Id, path: PathBuf) {
        let Some(si) = self.project.scenes.iter().position(|s| s.id == scene) else { return };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Media".into());
        let extension = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let (kind, rect) = if matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "webp" | "bmp") {
            match image::image_dimensions(&path) {
                Ok((w, h)) if w > 0 && h > 0 && w <= 4096 && h <= 4096 => (SourceKind::Image { path: path.display().to_string() }, [0.1, 0.1, 0.5, 0.5]),
                Ok(_) => return self.tell("Images can be at most 4096 × 4096".into()),
                Err(e) => return self.tell(format!("That image couldn't be opened: {e}")),
            }
        } else {
            (SourceKind::VideoFile(auroracast_core::VideoSettings { path: path.display().to_string(), ..Default::default() }), [0., 0., 1., 1.])
        };
        let id = self.project.id();
        self.project.sources.push(Source { id, name: name.clone(), kind });
        self.project.scenes[si].layers.push(Layer { crop: [0.; 4], group: 0, frame: Default::default(), look: Default::default(), source: id, rect, visible: true, locked: false, opacity: 1. });
        let index = self.project.scenes[si].layers.len() - 1;
        self.layer = Some(index);
        self.selections.insert(scene, vec![index]);
        self.changed();
        self.tell(format!("Added {name}"));
    }
    fn widget_controls(&mut self, ui: &mut egui::Ui, id: Id) {
        let Some(Source {
            kind: SourceKind::Widget(saved),
            ..
        }) = self.project.source(id)
        else {
            return;
        };
        let mut next = saved.clone();
        ui.separator();
        ui.strong(next.kind.label());
        ui.label("Heading");
        ui.text_edit_singleline(&mut next.label);
        ui.horizontal(|ui| {
            ui.label("Text");
            ui.color_edit_button_srgba_unmultiplied(&mut next.foreground);
            ui.label("Background");
            ui.color_edit_button_srgba_unmultiplied(&mut next.background);
        });
        crate::widgets::controls(ui, &mut next);
        if next.kind == widgets::Kind::AlertBox {
            ui.checkbox(
                &mut next.fit_media,
                "Fit media inside box (off = fill and crop)",
            );
            // Older alert boxes were a wide strip; this reshapes the box to 16:9 everywhere it's shown.
            if (next.layout_width, next.layout_height) != (960, 540)
                && ui.button("Make 16:9").on_hover_text("Reshape the alert box to 16:9 so videos fit without stretching").clicked()
            {
                next.layout_width = 960;
                next.layout_height = 540;
            }
            ui.small("Alerts use this source's canvas rectangle. Add media as a Media file source and enable Controlled by automation. Remove its ordinary canvas layer if it should only appear here. Set this box's background opacity to zero to reveal media.");
            let media: Vec<_> = self
                .project
                .sources
                .iter()
                .filter(|s| matches!(s.kind, SourceKind::VideoFile(_)))
                .map(|s| (s.id, s.name.clone()))
                .collect();
            let mut remove = None;
            for (index, rule) in next.alerts.iter_mut().enumerate() {
                ui.push_id(index, |ui| {
                    egui::ComboBox::from_id_salt("event")
                        .selected_text(rule.event.label())
                        .show_ui(ui, |ui| {
                            for event in EventKind::ALL {
                                ui.selectable_value(&mut rule.event, event, event.label());
                            }
                        });
                    egui::ComboBox::from_id_salt("media")
                        .selected_text(
                            media
                                .iter()
                                .find(|(id, _)| Some(*id) == rule.media)
                                .map(|(_, n)| n.as_str())
                                .unwrap_or("Text only"),
                        )
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut rule.media, None, "Text only");
                            for (id, name) in &media {
                                ui.selectable_value(&mut rule.media, Some(*id), name);
                            }
                        });
                    ui.text_edit_singleline(&mut rule.text);
                    ui.small("Use {user}, {event}, {message}");
                    ui.add(egui::Slider::new(&mut rule.seconds, 1..=60).text("Seconds"));
                    if ui.button("Remove override").clicked() {
                        remove = Some(index);
                    }
                    ui.separator();
                });
            }
            if let Some(index) = remove {
                next.alerts.remove(index);
            }
            if ui.button("Customize an event").clicked() {
                next.alerts.push(widgets::AlertStyle::default());
            }
            ui.horizontal(|ui| {
                if ui.button("Preview (does not count toward goals)").clicked() {
                    self.captures.widgets.preview_alert(id);
                }
                if ui.button("Clear alert boxes").clicked() {
                    self.captures.widgets.clear_alerts();
                }
            });
        }

        if matches!(
            next.kind,
            widgets::Kind::SubGoal
                | widgets::Kind::Chat
                | widgets::Kind::EventTicker
                | widgets::Kind::LatestSupporter
                | widgets::Kind::Credits
                | widgets::Kind::AlertBox
                | widgets::Kind::CommunityGoal
        ) {
            ui.label("Include these platforms (select several for a combined widget)");
            for p in [Platform::Twitch, Platform::YouTube, Platform::Kick, Platform::TikTok, Platform::Donations] {
                let mut on = next.platforms.contains(&p);
                if ui.checkbox(&mut on, p.name()).changed() {
                    next.platforms.retain(|v| *v != p);
                    if on {
                        next.platforms.push(p);
                    }
                }
            }
        }
        if matches!(
            next.kind,
            widgets::Kind::SubGoal
                | widgets::Kind::DonationGoal
                | widgets::Kind::Counter
                | widgets::Kind::CommunityGoal
        ) {
            ui.horizontal(|ui| {
                ui.label("Starting value");
                ui.add(egui::DragValue::new(&mut next.initial).range(0..=1_000_000_000));
            });
            if next.kind != widgets::Kind::Counter {
                ui.horizontal(|ui| {
                    ui.label("Goal");
                    ui.add(egui::DragValue::new(&mut next.target).range(1..=1_000_000_000));
                });
            }
            ui.horizontal(|ui| {
                if ui.button("−1").clicked() {
                    self.captures.widgets.update(id, "", -1, false);
                }
                if ui.button("+1").clicked() {
                    self.captures.widgets.update(id, "", 1, false);
                }
                if ui.button("Reset to starting value").clicked() {
                    self.captures.widgets.reset(id);
                }
            });
        }
        if next.kind == widgets::Kind::DonationGoal {
            ui.label("Currency");
            ui.text_edit_singleline(&mut next.currency);
            note(
                ui,
                "Values are in hundredths (500 = 5.00). Connected donations increase matching-currency goals while Rosorah is open; currencies are never converted automatically.",
            );
        }
        if next.kind == widgets::Kind::SubGoal {
            note(
                ui,
                "Counts verified Twitch new subscriptions and gifted totals while Rosorah is open, starting from your value. YouTube counts are manual until reliable membership quantities are available. This is not a fetched total of all subscribers.",
            );
        }
        if next.kind == widgets::Kind::Timer {
            ui.horizontal(|ui| {
                ui.label("Duration (seconds)");
                ui.add(egui::DragValue::new(&mut next.seconds).range(1..=86400));
            });
            ui.horizontal(|ui| {
                if ui.button("Start / restart").clicked() {
                    self.captures.widgets.update(id, "", 0, true);
                }
                if ui.button("Reset").clicked() {
                    self.captures.widgets.reset(id);
                }
            });
        }
        if next.kind == widgets::Kind::Chat {
            note(
                ui,
                "Shows the newest messages, with emoji and Twitch, YouTube and Kick emotes as pictures.",
            );
        }
        if next.kind == widgets::Kind::Shoutout {
            note(
                ui,
                "Use an Update widget action with text such as 'Go follow {user}!' This displays on the canvas; it does not post to chat.",
            );
        }
        if next != *saved {
            let reshaped = next.kind == widgets::Kind::AlertBox && (saved.layout_width, saved.layout_height) != (next.layout_width, next.layout_height);
            self.project
                .sources
                .iter_mut()
                .find(|s| s.id == id)
                .unwrap()
                .kind = SourceKind::Widget(next);
            if reshaped {
                self.fit_layers_to_16_9(id);
            }
            self.changed();
        }
    }
    /// Gives every layer showing `source` a 16:9 shape on its canvas, keeping its width and centre.
    fn fit_layers_to_16_9(&mut self, source: Id) {
        let ratios: Vec<(Id, f32)> = self
            .project
            .scenes
            .iter()
            .filter_map(|scene| {
                let canvas = self.project.canvases.iter().find(|c| c.scene == scene.id || self.project.canvas_scenes(c.id).contains(&scene.id))?;
                Some((scene.id, canvas.width.max(1) as f32 / canvas.height.max(1) as f32))
            })
            .collect();
        for (scene, ratio) in ratios {
            let Some(scene) = self.project.scenes.iter_mut().find(|s| s.id == scene) else { continue };
            for layer in scene.layers.iter_mut().filter(|l| l.source == source) {
                let [x, y, w, _] = layer.rect;
                let h = (w * ratio * 9. / 16.).min(1. - y);
                let fitted = h / (ratio * 9. / 16.);
                layer.rect = [x + (w - fitted) / 2., y, fitted, h];
            }
        }
    }
    /// Folder, timing and order for slideshows and video playlists.
    fn folder_source_controls(&mut self, ui: &mut egui::Ui, id: Id) {
        let mut changed = false;
        let mut pick = false;
        match self.project.sources.iter_mut().find(|s| s.id == id).map(|s| &mut s.kind) {
            Some(SourceKind::Slideshow(show)) => {
                let count = auroracast_core::folder_files(&show.folder, &auroracast_core::PICTURE_EXTENSIONS).len();
                note(ui, format!("{} · {count} pictures", show.folder));
                changed |= ui.add(egui::Slider::new(&mut show.seconds, 1.0..=120.0).text("Seconds each").logarithmic(true)).changed();
                changed |= ui.checkbox(&mut show.shuffle, "Shuffle").changed();
                pick = ui.button("Choose another folder…").clicked();
            }
            Some(SourceKind::VideoFile(video)) if !video.folder.is_empty() => {
                let count = auroracast_core::folder_files(&video.folder, &auroracast_core::VIDEO_EXTENSIONS).len();
                note(ui, format!("Playlist · {} · {count} videos", video.folder));
                changed |= ui.checkbox(&mut video.shuffle, "Shuffle").changed();
                changed |= ui.checkbox(&mut video.looped, "Start over after the last video").changed();
                note(ui, "Add or remove videos in the folder; the playlist picks them up the next time it starts.");
            }
            _ => {}
        }
        // Pointing a slideshow at another folder; the answer is applied when it comes back.
        if pick && self.slideshow_folder.is_none() {
            let (tx, rx) = mpsc::channel();
            self.slideshow_folder = Some((id, rx));
            std::thread::spawn(move || {
                let _ = tx.send(crate::desktop::pick(crate::desktop::Pick::Folder, "Choose a folder of pictures", "").ok().flatten());
            });
        }
        if changed {
            self.changed();
        }
    }
    fn video_controls(&mut self, ui: &mut egui::Ui, id: Id) {
        let Some(Source {
            kind: SourceKind::VideoFile(saved),
            ..
        }) = self.project.source(id)
        else {
            return;
        };
        let mut next = saved.clone();
        ui.separator();
        ui.strong("Video playback");
        ui.checkbox(&mut next.enabled, "Enabled");
        ui.checkbox(&mut next.audio, "Include video sound");
        ui.checkbox(&mut next.automation_only, "Controlled by automation rules");
        if next.automation_only {
            note(
                ui,
                "Add a Play media action in Alerts & automation. This source waits for a matching rule and plays once.",
            );
        }
        let mut alert = next.trigger.is_some();
        if ui.checkbox(&mut alert, "Play as an event alert").changed() {
            next.trigger = alert.then_some(EventKind::Follow);
        }
        if alert {
            egui::ComboBox::from_id_salt("video_event")
                .selected_text(next.trigger.unwrap().label())
                .show_ui(ui, |ui| {
                    for kind in EventKind::ALL {
                        if !matches!(
                            kind,
                            EventKind::ChatCommand
                                | EventKind::ChatMessage
                                | EventKind::FirstMessage
                                | EventKind::FirstTimeChatter
                        ) {
                            ui.selectable_value(&mut next.trigger, Some(kind), kind.label());
                        }
                    }
                });
            egui::ComboBox::from_id_salt("video_platform")
                .selected_text(
                    next.platform
                        .map_or("Any connected platform", Platform::name),
                )
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut next.platform, None, "Any connected platform");
                    for p in [Platform::Twitch, Platform::YouTube, Platform::Donations] {
                        ui.selectable_value(&mut next.platform, Some(p), p.name());
                    }
                });
            ui.small("Plays once for each new matching event while this scene is visible.");
        } else {
            ui.checkbox(&mut next.looped, "Loop video");
        }
        if ui
            .add_enabled(
                next.enabled,
                egui::Button::new(if alert || next.automation_only {
                    "Test alert"
                } else {
                    "Restart video"
                }),
            )
            .clicked()
        {
            if alert || next.automation_only {
                self.captures.videos.trigger(id);
            } else {
                self.captures.videos.restart.insert(id);
            }
        }
        ui.small(self.captures.video_status(id));
        if next != *saved {
            let audio = next.audio;
            let name = self.project.source(id).unwrap().name.clone();
            self.project
                .sources
                .iter_mut()
                .find(|s| s.id == id)
                .unwrap()
                .kind = SourceKind::VideoFile(next);
            if audio
                && !self
                    .project
                    .audio
                    .iter()
                    .any(|c| c.media_source == Some(id))
            {
                self.project.add_media_audio(id, name);
            }
            self.changed();
        }
    }
    fn browser_source_ui(&mut self, ui: &mut egui::Ui, id: Id) {
        let Some(Source {
            kind: SourceKind::Browser { url },
            ..
        }) = self.project.source(id)
        else {
            return;
        };
        let current = (
            url.clone(),
            self.project
                .preferences
                .browser_sources
                .get(&id)
                .cloned()
                .unwrap_or_default(),
        );
        let draft = self.browser_drafts.entry(id).or_insert(current);
        ui.separator();
        ui.strong("Browser source");
        let local = crate::browser_source::local_file(&draft.0).is_some();
        let mut browse = false;
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut draft.0)
                    .hint_text("https://… overlay URL, or a file on this PC")
                    .password(!local),
            );
            browse = ui.add_enabled(self.browser_picker.is_none(), egui::Button::new("Local file…")).clicked();
        });
        ui.small("Overlay URLs may contain private tokens. Web addresses (including local web servers) and HTML files on this PC both work.");
        ui.checkbox(&mut draft.1.enabled, "Enabled");
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::DragValue::new(&mut draft.1.width)
                    .range(16..=1920)
                    .prefix("Width "),
            );
            ui.add(
                egui::DragValue::new(&mut draft.1.height)
                    .range(16..=1920)
                    .prefix("Height "),
            );
            ui.add(
                egui::DragValue::new(&mut draft.1.fps)
                    .range(1..=60)
                    .suffix(" FPS"),
            );
        });
        ui.checkbox(
            &mut draft.1.audio,
            "Browser audio · independent mixer channel",
        );
        if cfg!(windows) && draft.1.audio {
            ui.small("On Windows you also hear the page through your speakers; Rosorah still gives it its own mixer strip.");
        }
        ui.checkbox(&mut draft.1.keep_running, "Keep running when not shown")
            .on_hover_text("The page keeps going while it's off screen, so alerts and timers don't restart. Without this, hidden pages stop to save power.");
        ui.checkbox(&mut draft.1.refresh_on_show, "Refresh the page each time it comes on screen");
        ui.label("Custom CSS");
        ui.add(
            egui::TextEdit::multiline(&mut draft.1.css)
                .code_editor()
                .desired_rows(3)
                .desired_width(f32::INFINITY),
        );
        ui.horizontal_wrapped(|ui| {
            ui.small("Added to the page, as in OBS.");
            if draft.1.css != auroracast_core::browser::DEFAULT_CSS && ui.small_button("Reset to default").clicked() {
                draft.1.css = auroracast_core::browser::DEFAULT_CSS.into();
            }
        });
        ui.small("Up to 1920×1080 or 1080×1920. Transparency follows the webpage's background.");
        let apply = ui.button("Apply browser settings").clicked();
        let mut next = draft.clone();
        if browse {
            let (tx, rx) = mpsc::channel();
            self.browser_picker = Some((id, rx));
            std::thread::spawn(move || {
                let _ = tx.send(crate::desktop::pick(crate::desktop::Pick::Open, "Choose an overlay (HTML file)", "").ok().flatten());
            });
        }
        if apply {
            if let Some(address) = crate::browser_source::address(&next.0) {
                next.0 = address;
            }
            if !crate::browser_source::valid_source(&next.0) {
                self.message = "Enter a web address (http:// or https://) or choose an HTML file".into();
            } else if crate::browser_source::local_file(&next.0).is_some_and(|f| !f.is_file()) {
                self.message = "That overlay file doesn't exist any more".into();
            } else if let Err(e) = next.1.validate() {
                self.message = e.to_string();
            } else {
                if let Some(source) = self.project.sources.iter_mut().find(|s| s.id == id) {
                    source.kind = SourceKind::Browser { url: next.0 };
                }
                self.project
                    .preferences
                    .browser_sources
                    .insert(id, next.1.clone());
                if !self
                    .project
                    .audio
                    .iter()
                    .any(|c| c.media_source == Some(id))
                {
                    let name = self.project.source(id).unwrap().name.clone();
                    self.project.add_media_audio(id, name);
                }
                if !crate::browser_source::ready() && !self.browser_chat.installing() {
                    self.browser_chat.setup();
                }
                self.changed();
            }
        }
        ui.horizontal_wrapped(|ui| {
            if ui.button("Refresh page").clicked() {
                self.captures.browser_command(id, "reload");
            }
            if ui.button("Interact").clicked() {
                self.captures.browser_command(id, "interact");
            }
            if ui.button("Forget sign-ins")
                .on_hover_text("Clears this source's cookies and saved sign-ins")
                .clicked()
            {
                self.captures.browser_command(id, "forget");
            }
        });
        if !crate::browser_source::ready() {
            ui.label(&self.browser_chat.status);
            if self.advanced()
                && ui
                    .add_enabled(!self.browser_chat.installing(), egui::Button::new("Prepare browser runtime"))
                    .clicked()
            {
                self.browser_chat.setup();
            }
        }
        ui.small(self.captures.browser_status(id));
    }
    fn save_stream_settings(&mut self) -> bool {
        match self.stream_config.save_with_keyring(&crate::streaming::Config::path()) {
            Ok(in_keyring) => {
                self.stream_saved = if in_keyring {
                    "Streaming settings saved; stream keys are in your system keyring".into()
                } else {
                    "Streaming settings saved on this computer (no system keyring found, so keys are in a private file)".into()
                };
                true
            }
            Err(e) => {
                self.stream_error = format!("Could not save streaming settings: {e:#}");
                false
            }
        }
    }
    fn destination_canvas(
        ui: &mut egui::Ui,
        destination: &mut crate::streaming::Destination,
        canvases: &[Canvas],
        vertical: bool,
    ) {
        let fallback = canvases.iter().find(|c| (c.height > c.width) == vertical);
        let label = destination
            .canvas
            .and_then(|id| canvases.iter().find(|c| c.id == id))
            .or_else(|| {
                if destination.canvas.is_none() {
                    fallback
                } else {
                    None
                }
            })
            .map(|c| format!("{} · {}×{}", c.name, c.width, c.height))
            .unwrap_or_else(|| "Choose canvas".into());
        egui::ComboBox::from_id_salt("destination_canvas")
            .selected_text(label)
            .show_ui(ui, |ui| {
                for c in canvases {
                    ui.selectable_value(
                        &mut destination.canvas,
                        Some(c.id),
                        format!("{} · {}×{}", c.name, c.width, c.height),
                    );
                }
            });
    }
    fn output_buttons(&mut self, ui: &mut egui::Ui) {
        use crate::recording::Phase;
        let stream_view = self.live_stream.as_ref().map(|s| s.view());
        let recording_view = self.live_recording.as_ref().map(|r| r.view());
        let shared_recording = self
            .live_stream
            .as_ref()
            .is_some_and(|s| s.destinations().iter().any(|v| v.path.is_some()));
        let text = |phase, stream| match phase {
            Some(Phase::Starting) => {
                if stream {
                    "Cancel streaming"
                } else {
                    "Cancel recording"
                }
            }
            Some(Phase::Recording) => {
                if stream {
                    "Stop streaming"
                } else {
                    "Stop recording"
                }
            }
            Some(_) => "Finishing…",
            None => {
                if stream {
                    "Go live"
                } else {
                    "Record"
                }
            }
        };
        let streaming_phase = stream_view.as_ref().map(|v| v.phase);
        let recording_phase = recording_view.as_ref().map(|v| v.phase);
        let active = |phase| !matches!(phase, Some(Phase::Finishing | Phase::Finished));
        let width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.;
        let mut stream_clicked = false;
        let mut record_clicked = false;
        ui.horizontal_wrapped(|ui| {
            stream_clicked = ui
                .add_enabled(
                    active(streaming_phase),
                    egui::Button::new(text(streaming_phase, true)).min_size(Vec2::new(width, 38.)),
                )
                .clicked();
            record_clicked = ui
                .add_enabled(
                    active(recording_phase) && !shared_recording,
                    egui::Button::new(if shared_recording {
                        "Recording stream"
                    } else {
                        text(recording_phase, false)
                    })
                    .min_size(Vec2::new(width, 38.)),
                )
                .clicked();
        });
        if stream_clicked {
            if let Some(stream) = &self.live_stream {
                stream.stop();
            } else {
                self.go_live_open = true;
            }
        }
        if record_clicked {
            if let Some(recording) = &self.live_recording {
                recording.stop();
            } else {
                self.start_recording();
            }
        }
        if let Some(view) = recording_view {
            note(
                ui,
                format!(
                    "Recording · {}:{:02}",
                    view.elapsed.as_secs() / 60,
                    view.elapsed.as_secs() % 60
                ),
            );
        }
    }
    fn streaming_controls(&mut self, ui: &mut egui::Ui) {
        let view = self.live_stream.as_ref().map(|s| s.view());
        if let Some(stream) = &self.live_stream {
            let v = view.as_ref().unwrap();
            note(
                ui,
                format!(
                    "{} · {}:{:02} · {}",
                    stream.canvas_names(),
                    v.elapsed.as_secs() / 60,
                    v.elapsed.as_secs() % 60,
                    stream.bitrates()
                ),
            );
            note(
                ui,
                if v.encoder.is_empty() {
                    "Preparing encoder…"
                } else {
                    &v.encoder
                },
            );
            for (destination_index,dest) in stream.destinations().into_iter().enumerate() {
                let label = if dest.path.is_some() && dest.phase == crate::streaming::Phase::Sending
                {
                    "Recording"
                } else {
                    dest.phase.label()
                };
                let color = match dest.phase {
                    crate::streaming::Phase::Sending => Color32::LIGHT_GREEN,
                    crate::streaming::Phase::Failed => crate::appearance::current().error,
                    _ => muted(),
                };
                ui.colored_label(
                    color,
                    format!(
                        "{} · {} · {}:{:02}",
                        dest.name,
                        label,
                        dest.elapsed.as_secs() / 60,
                        dest.elapsed.as_secs() % 60
                    ),
                );
                if !dest.detail.is_empty() {note(ui, dest.detail);}
                if dest.path.is_none()&&dest.phase==crate::streaming::Phase::Reconnecting&&ui.small_button("Retry this destination now").clicked(){stream.retry_destination(destination_index);}
                if dest.path.is_some(){ui.small("Local recording continues independently of network reconnection.");}
            }
        } else {
            note(ui, "Tick where to stream above, then press Go live.");
            if !self.accounts.iter().any(|a| a.connected) && ui.button("Sign in to your accounts").clicked()
            {
                self.page = Page::Settings;
                self.settings = Settings::Accounts;
            }
        }
        if !self.stream_error.is_empty() {
            self.error_line(ui, self.stream_error.clone());
        }
        ui.collapsing("Streaming details", |ui| {
            let detail = view.as_ref().map_or(self.stream_details.as_str(), |v| v.details.as_str());
            if !detail.is_empty() { ui.label(detail); }
            note(ui, "Each destination reconnects independently. Sending indicates encoder delivery, not confirmation that viewers can watch. Check the platform dashboard for stream visibility.");
        });
    }
    fn start_streaming(&mut self) {
        if self.accounts[1].connected {
            self.accounts[1].send(crate::accounts::Command::Discover);
        }
        self.stream_error.clear();
        self.stream_details.clear();
        if !self.save_stream_settings() {
            return;
        }
        let mut config = self.stream_config.clone();
        // An existing recording already owns its encoder and file; don't start a second copy.
        if self.live_recording.is_some() {
            config.record = false;
        }
        match crate::streaming::Stream::start(
            &self.project,
            self.canvas,
            self.captures.reader(),
            &config,
            std::path::Path::new(&self.record_folder),
        ) {
            Ok(stream) => {
                self.audio.sync(
                    &self.project.audio,
                    self.project.preferences.auto_game_audio,
                    self.captures.game(),
                );
                self.audio.streaming_outlets(stream.outlets());
                self.live_stream = Some(stream);
                self.message = "Preparing stream…".into();
                crate::capture_log::event("Streaming requested");
            }
            Err(e) => self.stream_error = format!("{e:#}"),
        }
    }
    fn streaming_tick(&mut self, ctx: &egui::Context) {
        for result in self.announcer.drain() {
            self.tell(result);
        }
        // A renewed Threads token replaces the old one, which stops working once renewed.
        if let Some(account) = self.announcer.threads_account() {
            let a = &mut self.stream_config.announce;
            if let Some((token, expires)) = account.renewed {
                a.threads_token = token;
                a.threads_expires = expires;
            }
            a.threads_username = account.username;
            self.save_stream_settings();
        }
        let Some(stream) = &self.live_stream else {
            self.announced = false;
            self.announce_skip = [false; 4];
            self.made_public = false;
            return;
        };
        // Once video is going to YouTube, that broadcast is made public (if chosen).
        let youtube = self.stream_config.connection_name(1);
        if !self.made_public && stream.destinations().iter().any(|d| d.path.is_none() && d.name == youtube && d.phase == crate::streaming::Phase::Sending) {
            self.made_public = true;
            if self.stream_config.youtube_public && self.accounts[1].connected {
                self.accounts[1].send(crate::accounts::Command::MakePublic);
            }
        }
        let Some(stream) = &self.live_stream else {
            return;
        };
        if !self.announced && stream.destinations().iter().any(|d| d.path.is_none() && d.phase == crate::streaming::Phase::Sending) {
            self.announced = true;
            let a = &self.stream_config.announce;
            let which = [a.discord_ready() && !self.announce_skip[0], a.bluesky_ready() && !self.announce_skip[1], a.threads_ready() && !self.announce_skip[2], a.x_ready() && !self.announce_skip[3]];
            if which.iter().any(|w| *w) {
                self.announcer.send(a, self.announce_stream(), which, ctx);
            }
        }
        let Some(stream) = &self.live_stream else {
            return;
        };
        let view = stream.view();
        if let Some(result) = view.result {
            self.stream_details = view.details;
            for dest in stream.destinations() {
                if let Some(path) = dest.path {
                    if dest.phase == crate::streaming::Phase::Stopped
                        && dest.detail == "Recording saved"
                    {
                        self.last_recording = Some(path);
                    } else if dest.phase == crate::streaming::Phase::Failed {
                        self.recording_error = dest.detail.clone();
                    }
                }
                self.stream_details.push_str(&format!(
                    "\n{}: {}. {}",
                    dest.name,
                    dest.phase.label(),
                    dest.detail
                ));
            }
            self.audio.streaming_outlets(Vec::new());
            self.live_stream = None;
            match result {
                Ok(_) => self.message = "Streaming stopped".into(),
                Err(e) => {
                    self.stream_error = e;
                    self.message = "Streaming stopped with an error".into();
                }
            }
            crate::capture_log::event(format!(
                "Streaming finished: {}\n{}",
                self.stream_error, self.stream_details
            ));
        }
        ctx.request_repaint_after(Duration::from_millis(100));
    }
    fn broadcast_routes(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        self.replay_controls(ui);
        self.virtual_camera_controls(ui);
        self.shared_output_controls(ui);
        ui.separator();
        ui.strong("Where to stream");
        let before = self.stream_config.clone();
        let main = self.project.canvases.first().map(|c| c.id);
        let vertical = self
            .project
            .canvases
            .iter()
            .find(|c| c.height > c.width)
            .map(|c| c.id)
            .or(main);
        let mut names = (0..3)
            .map(|slot| self.stream_config.connection_name(slot))
            .collect::<Vec<_>>();
        names.extend(self.stream_config.services.iter().map(|s| s.name.clone()));
        let several_videos = self.project.canvases.len() > 1 && (self.advanced() || self.project.preferences.workspace.vertical_video);
        let kick_connected = self.kick_account.connected;
        // The platform each row streams to, for its name color; None for custom servers.
        let platforms: Vec<Option<Platform>> = (0..names.len())
            .map(|slot| match slot {
                0 => Some(Platform::Twitch),
                1 | 2 => Some(Platform::YouTube),
                _ => match self.stream_config.services[slot - 3].provider {
                    crate::streaming::Provider::Kick => Some(Platform::Kick),
                    crate::streaming::Provider::TikTok => Some(Platform::TikTok),
                    crate::streaming::Provider::Custom => None,
                },
            })
            .collect();
        let mut hidden = Vec::new();
        ui.add_enabled_ui(self.live_stream.is_none(), |ui| {
            for (slot, name) in names.iter().enumerate() {
                let connected = match slot {
                    0 => self.accounts[0].connected,
                    1 => self.accounts[1].connected,
                    2 => false,
                    _ => platforms[slot] == Some(Platform::Kick) && kick_connected && self.stream_config.services[slot - 3].destination.managed_by_account,
                };
                let dest = self.stream_config.destination_mut(slot).unwrap();
                // Only platforms you can stream to (or have switched on) are listed; the rest wait in
                // "Set up another platform".
                if !connected && dest.key.is_empty() && !dest.enabled {
                    hidden.push(slot);
                    continue;
                }
                ui.push_id(("broadcast-service", slot), |ui| {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut dest.enabled, "").on_hover_text("Include when you go live");
                        let (dot, status, color) = if !dest.key.is_empty() {
                            ("●", "ready", crate::appearance::current().good)
                        } else if connected {
                            ("◌", "getting stream key…", crate::appearance::current().warning)
                        } else {
                            ("○", "sign in", muted())
                        };
                        let video = dest.canvas.or(if slot == 2 { vertical } else { main }).and_then(|id| self.project.canvases.iter().find(|c| c.id == id)).map(|c| c.name.clone()).unwrap_or_default();
                        ui.label(RichText::new(dot).color(color));
                        let label = ui.add(egui::Label::new(RichText::new(name).strong().color(platforms[slot].map_or(ui.visuals().strong_text_color(), platform_color))).truncate());
                        if several_videos {
                            label.on_hover_text(format!("Sends the {}", video.to_lowercase()));
                        }
                        ui.label(RichText::new(status).small().color(color));
                        if ui.small_button("Set up…").on_hover_text("Sign-in, stream key and which video to send").clicked() {
                            self.broadcast_window = 100 + slot as u8;
                        }
                    });
                });
            }
            ui.menu_button("+ Set up another platform", |ui| {
                for slot in &hidden {
                    if ui.button(&names[*slot]).clicked() {
                        self.broadcast_window = 100 + *slot as u8;
                        ui.close();
                    }
                }
                if !hidden.is_empty() {
                    ui.separator();
                }
                if ui.add_enabled(self.stream_config.services.len() < 16, egui::Button::new("A new platform…")).clicked() {
                    self.broadcast_window = 3;
                    ui.close();
                }
            });
        });
        if before != self.stream_config {
            self.save_stream_settings();
        }
    }
    fn service_window(&mut self, ctx: &egui::Context) {
        let which = self.broadcast_window;
        let mut open = true;
        if which == 3 {
            pop_window(ctx, "Add streaming service")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .default_width(380.)
                .show(ctx, |ui| {
                    ui.label("Choose where to send your broadcast.");
                    ui.add_enabled_ui(self.live_stream.is_none(), |ui| {
                        for (name, slot) in [
                            ("Twitch", 0),
                            (
                                "YouTube",
                                if !self.stream_config.youtube.key.is_empty()
                                    && self.stream_config.youtube_vertical.key.is_empty()
                                {
                                    2
                                } else {
                                    1
                                },
                            ),
                        ] {
                            if ui
                                .add_sized([ui.available_width(), 40.], egui::Button::new(name))
                                .clicked()
                            {
                                self.broadcast_window = 100 + slot;
                            }
                        }
                        for provider in [
                            crate::streaming::Provider::Kick,
                            crate::streaming::Provider::TikTok,
                            crate::streaming::Provider::Custom,
                        ] {
                            if ui
                                .add_sized(
                                    [ui.available_width(), 40.],
                                    egui::Button::new(provider.name()),
                                )
                                .clicked()
                                && let Some(slot) = self.stream_config.add_service(provider)
                            {
                                self.broadcast_window = 100 + slot as u8;
                                self.save_stream_settings();
                            }
                        }
                    });
                });
        } else if which >= 100 {
            let slot = (which - 100) as usize;
            let name = self.stream_config.connection_name(slot);
            let provider = self
                .stream_config
                .services
                .get(slot.wrapping_sub(3))
                .map(|s| s.provider);
            let mut save = false;
            let mut remove = false;
            let mut account = false;
            let advanced = self.advanced();
            let several_videos = self.project.canvases.len() > 1 && (advanced || self.project.preferences.workspace.vertical_video);
            pop_window(ctx, format!("{name} connection")).id(egui::Id::new(("service-connection",slot))).collapsible(false).open(&mut open).default_width(420.).show(ctx,|ui| {
                ui.add_enabled_ui(self.live_stream.is_none(),|ui| {
                    if let Some(name)=self.stream_config.connection_name_mut(slot) {
                        ui.label("Connection name");
                        ui.add(egui::TextEdit::singleline(name).char_limit(64));
                    }
                    if slot<3 {
                        if ui.add(self.primary_button(&format!("Sign in with {}",if slot==0{"Twitch"}else{"YouTube"}))).clicked(){account=true;}
                        note(ui,"Signing in fills in everything below for you.");
                    } else {
                        ui.horizontal_wrapped(|ui| {
                            ui.label("Connection method");
                            ui.label("Stream key");
                            if provider == Some(crate::streaming::Provider::Kick) && ui.button("Connect account").clicked() {account=true;}
                        });
                        match provider {
                            Some(crate::streaming::Provider::Kick)=>{ui.small("Connect Kick in Settings > Accounts to retrieve your streaming server and key automatically, or enter a key here.");},
                            Some(crate::streaming::Provider::TikTok)=>{ui.small("TikTok account login does not supply LIVE streaming access here. Use a server and key if TikTok has granted your account access.");},
                            _=>{ui.small("Custom services connect using a server address and stream key.");}
                        }
                    }
                    let dashboard=match slot {0=>"https://dashboard.twitch.tv/settings/stream",1|2=>"https://studio.youtube.com/channel/UC/livestreaming",_=>provider.map_or("",|p|p.dashboard())};
                    if !dashboard.is_empty(){ui.hyperlink_to("Open creator dashboard",dashboard);}
                    if slot==0 {
                        // Twitch keeps this track in VODs, so music heard live can stay out of them.
                        let names:Vec<String>=(0..6).map(|t|self.project.preferences.track_name(t)).collect();
                        let vod=&mut self.stream_config.twitch_vod_track;
                        ui.add_space(6.);
                        ui.horizontal(|ui|{
                            ui.label("VOD audio");
                            egui::ComboBox::from_id_salt("twitch-vod-track").selected_text(vod.map_or("Same as the stream".to_owned(),|t|names[t].clone())).show_ui(ui,|ui|{
                                ui.selectable_value(vod,None,"Same as the stream");
                                for (t,name) in names.iter().enumerate().skip(1){ui.selectable_value(vod,Some(t),name);}
                            });
                        });
                        if let Some(t)=*vod {
                            match crate::streaming::multitrack_flv() {
                                Some(false)=>note(ui,"Your FFmpeg can't send a second audio track (it needs FFmpeg 8 or newer), so VODs get the stream mix."),
                                None=>{note(ui,"Checking FFmpeg…");ui.ctx().request_repaint_after(Duration::from_millis(300));}
                                Some(true)=>{
                                    note(ui,format!("Viewers hear {} live; your VODs get {} instead. Leave music off {}.",names[0],names[t],names[t]));
                                    if ui.button(format!("Put everything except music on {}",names[t])).on_hover_text("Every sound goes on this track except the Music strip and music apps").clicked(){
                                        let inputs=self.project.preferences.audio_inputs.clone();
                                        for channel in &mut self.project.audio {
                                            let music=inputs.get(&channel.id).is_some_and(|i|i=="auto:music")||channel.name.to_lowercase().contains("music");
                                            channel.tracks[t]=!music;
                                        }
                                        self.changed();
                                    }
                                }
                            }
                        } else {
                            note(ui,"Choose a track to keep music out of your saved VODs.");
                        }
                    }
                    let shared_bitrate=self.stream_config.bitrate;
                    if let Some(dest)=self.stream_config.destination_mut(slot) {
                        if provider == Some(crate::streaming::Provider::Kick) && ui.checkbox(&mut dest.managed_by_account,"Use connected Kick account").changed() {
                            save = true;
                            if dest.managed_by_account {
                                if self.kick_account.connected {self.kick_account.send(crate::accounts::Command::Refresh);} else {account=true;}
                            }
                        }
                        let manual_open=!dest.managed_by_account&&!dest.key.is_empty();
                        let show=&mut self.show_stream_keys;
                        let mut manual=|ui:&mut egui::Ui|{
                            ui.label("Server address");ui.add(egui::TextEdit::singleline(&mut dest.server).desired_width(f32::INFINITY).hint_text("rtmps://…"));
                            ui.label("Stream key");
                            if ui.add(egui::TextEdit::singleline(&mut dest.key).password(!*show).desired_width(f32::INFINITY)).changed(){dest.managed_by_account=false;}
                            ui.checkbox(show,"Show stream key");
                        };
                        if slot<3||provider==Some(crate::streaming::Provider::Kick) {
                            egui::CollapsingHeader::new("Or connect manually with a stream key").default_open(manual_open).show(ui,|ui|manual(ui));
                        } else {
                            manual(ui);
                        }
                        if several_videos {Self::destination_canvas(ui,dest,&self.project.canvases,slot==2);}
                        if advanced {ui.horizontal(|ui|{
                            let mut own=dest.bitrate.is_some();
                            if ui.checkbox(&mut own,"Own video bitrate").changed(){dest.bitrate=own.then_some(if slot==0{shared_bitrate.min(6000)}else{shared_bitrate});}
                            if let Some(b)=&mut dest.bitrate{ui.add(egui::DragValue::new(b).range(1000..=if slot==0{6000}else{20_000}).speed(100).suffix(" kbps"));}
                        });
                        if dest.bitrate.is_some(){ui.small("A different bitrate uses its own encoder, which adds GPU and CPU load while live.");}}
                        ui.checkbox(&mut dest.enabled,"Include when going live");
                        ui.small("A stream-key connection sends video and audio; it does not connect chat or activity.");
                        ui.horizontal(|ui|{if ui.button("Save connection").clicked(){save=true;}
                        if slot>=3 && ui.button("Remove connection").clicked(){remove=true;}});
                    }
                });
            });
            if account {
                self.page = Page::Settings;
                self.settings = Settings::Accounts;
                open = false;
            }
            if remove && slot >= 3 && slot - 3 < self.stream_config.services.len() {
                self.stream_config.services.remove(slot - 3);
                save = true;
            }
            if save
                && !remove
                && (self.stream_config.connection_name(slot).trim().is_empty()
                    || self
                        .stream_config
                        .connection_name(slot)
                        .chars()
                        .any(char::is_control))
            {
                self.message = "Enter a connection name before saving.".into();
                save = false;
            }
            if save && self.save_stream_settings() {
                open = false;
            }
        }
        if !open {
            self.broadcast_window = 0;
        }
    }
    fn broadcast_panel(&mut self, ui: &mut egui::Ui) {
        // Go live and Record live in the header; Advanced repeats them here.
        if self.advanced() {
            self.output_buttons(ui);
        }
        self.broadcast_routes(ui);
        self.title_editor(ui);
        self.streaming_controls(ui);
        if ui.button("Category & thumbnail…").clicked() {
            self.broadcast_window = 1;
        }
        if self.advanced() && ui.button("Quality & recording").clicked() {
            self.broadcast_window = 2;
        }
        ui.checkbox(&mut self.stream_config.record, "Record while streaming");
        ui.separator();
        let mut commands = Vec::new();
        ui.push_id("broadcast_queue", |ui| crate::automations::queue_bar(ui, &mut self.automation_queue, &mut commands));
        for command in commands {
            self.automation_command(command);
        }
    }
    /// One stream title for every signed-in platform, right in the Stream panel.
    fn title_editor(&mut self, ui: &mut egui::Ui) {
        let mut accounts: Vec<&mut crate::accounts::Account> = self.accounts.iter_mut().chain(std::iter::once(&mut self.kick_account)).filter(|a| a.connected).collect();
        if accounts.is_empty() {
            return;
        }
        if self.title_draft.is_empty()
            && let Some(title) = accounts.iter().map(|a| a.title.clone()).find(|t| !t.is_empty())
        {
            self.title_draft = title;
        }
        ui.separator();
        ui.strong("Stream title");
        let field = ui.add(egui::TextEdit::singleline(&mut self.title_draft).hint_text("What are you streaming today?").desired_width(f32::INFINITY).char_limit(140));
        let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let busy = accounts.iter().any(|a| a.operation_busy);
        let names: Vec<_> = accounts.iter().map(|a| a.platform.name()).collect();
        if (ui.add_enabled(!self.title_draft.trim().is_empty() && !busy, egui::Button::new(format!("Update on {}", names.join(" and ")))).clicked() || enter)
            && !self.title_draft.trim().is_empty()
        {
            for account in &mut accounts {
                // YouTube titles belong to a broadcast; skip it until one exists.
                if account.platform == Platform::YouTube && account.broadcast.is_empty() {
                    continue;
                }
                account.title = self.title_draft.trim().to_owned();
                account.send(crate::accounts::Command::Update { title: account.title.clone(), category: account.category.clone() });
            }
            self.tell("Stream title sent".into());
        }
        if busy {
            ui.horizontal(|ui| {
                ui.spinner();
                note(ui, "Updating…");
            });
        }
    }
    fn broadcast_windows(&mut self, ctx: &egui::Context) {
        let which = self.broadcast_window;
        if which == 3 || which >= 100 {
            self.service_window(ctx);
            return;
        }
        if which == 0 {
            return;
        }
        let mut open = true;
        pop_window(ctx, if which == 1 {
            "Stream details"
        } else {
            "Quality & recording"
        })
        .open(&mut open)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .default_width(580.)
        .vscroll(true)
        .show(ctx, |ui| {
            if which == 1 {
                self.stream_details(ui);
            } else {
                if ui
                    .button("Encoder, resolution & recording folder")
                    .clicked()
                {
                    self.page = Page::Settings;
                    self.settings = Settings::Video;
                }
                if ui.button("Destinations & stream quality").clicked() {
                    self.page = Page::Settings;
                    self.settings = Settings::Accounts;
                }
                self.broadcast_advanced_panel(ui);
            }
        });
        if !open {
            self.broadcast_window = 0;
        }
    }
    fn broadcast_advanced_panel(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
        self.output_buttons(ui);
        self.streaming_controls(ui);
        ui.collapsing("Title, category & YouTube broadcast",|ui|self.stream_details(ui));
        ui.separator();
        ui.collapsing("Recording options & saved files",|ui| {
        let shared_recording = self.live_stream.as_ref().is_some_and(|s| s.destinations().iter().any(|v| v.path.is_some()));
        if shared_recording { note(ui, "Recording the stream with the shared encoder. Stop streaming to finish the file."); }
        let view = self.live_recording.as_ref().map(|r| r.view());
        if let (Some(r), Some(v)) = (&self.live_recording, &view) {
            note(
                ui,
                format!(
                    "{} · {}:{:02}",
                    r.canvas.name,
                    v.elapsed.as_secs() / 60,
                    v.elapsed.as_secs() % 60
                ),
            );
            note(
                ui,
                if v.encoder.is_empty() {
                    "Preparing recorder…"
                } else {
                    &v.encoder
                },
            );
            ui.label("Video + Track 1 audio")
                .on_hover_text(v.path.display().to_string());
        } else {
            note(
                ui,
                format!(
                    "{} · video + audio",
                    self.project.canvases[self.canvas].name
                ),
            );
            note(ui, if self.project.preferences.multitrack_recording{"Records the selected canvas with six separate audio tracks."}else{"Records the selected canvas and Track 1 mix."});
        }
        if !self.recording_error.is_empty() {
            self.error_line(ui, self.recording_error.clone());
        }
        if let Some(path) = self.last_recording.clone() {
            if ui
                .button("Open recording")
                .on_hover_text(path.display().to_string())
                .clicked()
            {
                match crate::desktop::open(&path) {
                    Ok(mut child) => {
                        std::thread::spawn(move || {
                            let _ = child.wait();
                        });
                    }
                    Err(e) => self.message = format!("Open {} manually: {e}", path.display()),
                }
            }
            ui.label("Saved recording")
                .on_hover_text(path.display().to_string());
        }
        ui.collapsing("Recording details", |ui| {
            ui.label(format!("Folder: {}", self.record_folder));
            note(ui, "Change encoder, folder, size and FPS in Settings > Video & recording. Missing audio is recorded as silence; the mixer shows what is connected.");
            let details = view.as_ref().map_or(self.recording_details.as_str(), |v| v.details.as_str());
            if !details.is_empty() { ui.separator(); ui.label(details); }
        });
        });
        ui.collapsing("Diagnostics", |ui| {
            let audio = self.audio.view();
            if ui.add_enabled(audio.capturing || audio.recording, egui::Button::new(if audio.recording { "Stop audio test" } else { "Record audio test" })).clicked() {
                self.audio.record(!audio.recording);
            }
            if !audio.saved.is_empty() { ui.label("Audio test folder").on_hover_text(&audio.saved); }
            if ui.add_enabled(self.live_recording.is_none() && self.live_stream.is_none() && self.recorder.is_none() && !self.project.sources.iter().any(|s| matches!(s.kind, SourceKind::ScreenCapture)), egui::Button::new("Record synthetic video test")).clicked() { self.record_demo(); }
            note(ui, "Audio tests save six WAV tracks in your Music folder. Synthetic video tests are three seconds long.");
        });
        });
    }
    fn start_recording(&mut self) {
        self.recording_error.clear();
        self.recording_details.clear();
        match crate::recording::Recording::start(
            &self.project,
            self.canvas,
            self.captures.reader(),
            std::path::Path::new(&self.record_folder),
        ) {
            Ok(recording) => {
                self.audio.sync(
                    &self.project.audio,
                    self.project.preferences.auto_game_audio,
                    self.captures.game(),
                );
                self.audio.recording_outlet(Some(recording.outlet()));
                self.live_recording = Some(recording);
                self.message = "Preparing recording…".into();
                crate::capture_log::event("Live recording requested");
            }
            Err(e) => self.recording_error = format!("{e:#}"),
        }
    }
    fn recording_tick(&mut self, ctx: &egui::Context) {
        if let Some(r) = &self.live_recording {
            let view = r.view();
            if let Some(result) = view.result {
                self.recording_details = view.details;
                self.audio.recording_outlet(None);
                self.live_recording = None;
                match result {
                    Ok(path) => {
                        self.message = format!("Recording saved: {}", path.display());
                        self.last_recording = Some(path);
                        crate::capture_log::event("Live recording saved successfully");
                    }
                    Err(e) => {
                        self.message = "Recording stopped with an error".into();
                        self.recording_error = e;
                        crate::capture_log::event(format!(
                            "Recording error: {}",
                            self.recording_error
                        ));
                    }
                }
            }
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
    fn camera_controls(&mut self, ui: &mut egui::Ui, id: Id) {
        let Some(Source {
            kind: SourceKind::VideoCamera(saved),
            ..
        }) = self.project.source(id)
        else {
            return;
        };
        let saved = saved.clone();
        let draft = self
            .camera_drafts
            .entry(id)
            .or_insert_with(|| saved.clone());
        let mut apply = false;
        let mut stop = false;
        egui::CollapsingHeader::new("Camera settings").default_open(true).show(ui, |ui| {
            ui.label(self.captures.camera_status(id));
            ui.label("Live preview appears on the canvas. Camera audio is not added to the mix.");
            // One list of cameras; DroidCam's own settings show only when DroidCam is the choice.
            let device_name = self.camera_devices.iter().find(|d| d.path == draft.device).map(|d| d.name.clone());
            let droidcam_device = device_name.as_deref().is_some_and(|n| n.to_lowercase().contains("droidcam"));
            ui.horizontal(|ui| {
                ui.label("Camera");
                egui::ComboBox::from_id_salt(("camera_device", id))
                    .selected_text(if draft.network { "DroidCam over Wi-Fi".to_owned() } else { device_name.clone().unwrap_or_else(|| "Select camera".into()) })
                    .show_ui(ui, |ui| {
                        for device in &self.camera_devices {
                            if ui.selectable_label(!draft.network && draft.device == device.path, format!("{} ({})", device.name, device.path)).clicked() {
                                draft.network = false;
                                draft.device = device.path.clone();
                            }
                        }
                        ui.separator();
                        if ui.selectable_label(draft.network, "DroidCam over Wi-Fi").on_hover_text("A phone running DroidCam on the same network").clicked() {
                            draft.network = true;
                        }
                    });
                if ui.small_button("⟳").on_hover_text("Look for cameras again").clicked() {
                    self.camera_devices = crate::camera::devices();
                }
            });
            if draft.network {
                ui.label("Phone Wi-Fi IP / hostname"); ui.text_edit_singleline(&mut draft.host);
                ui.horizontal(|ui| { ui.label("Port"); ui.add(egui::DragValue::new(&mut draft.port).range(1..=65535)); });
                ui.checkbox(&mut draft.legacy_url,"Use legacy DroidCam /video address");
                ui.small("Open DroidCam on your phone and use the IP and port shown there. Keep both devices on the same network.");
            } else {
                ui.collapsing("Device path",|ui| {ui.text_edit_singleline(&mut draft.device);});
                ui.checkbox(&mut draft.auto_mode,"Auto detect resolution and FPS");
                if draft.auto_mode && droidcam_device {
                    ui.small("For DroidCam, change quality in its desktop client; Rosorah follows the published mode.");
                } else if draft.auto_mode {
                    ui.small("Uses the camera's available output.");
                }
            }
            if draft.network || !draft.auto_mode {
            egui::ComboBox::from_id_salt(("camera_size",id)).selected_text(format!("{} × {}",draft.width,draft.height)).show_ui(ui, |ui| {
                for (w,h) in [(640,480),(1280,720),(1920,1080),(2560,1440),(3840,2160)] {
                    if ui.selectable_label((draft.width,draft.height)==(w,h),format!("{w} × {h}")).clicked() { draft.width=w; draft.height=h; }
                }
            });
            ui.horizontal(|ui| {ui.label("FPS"); for fps in [15,30,60] {ui.selectable_value(&mut draft.fps,fps,fps.to_string());}});
            }
            if draft.network {
                ui.small("Wi-Fi FPS is a maximum; the phone controls its source rate.");
            } else if !draft.auto_mode {
                ui.small("Manual mode must match a resolution and frame rate supported by the camera.");
            }
            ui.checkbox(&mut draft.auto_aspect,"Match camera aspect ratio automatically");
            ui.small("Fits to the actual video when it connects or changes resolution. Keeps the layer width and position; locked layers stay fixed.");
            ui.checkbox(&mut draft.flip,"Flip horizontally");
            ui.collapsing("Crop", |ui| {
                for (i,label) in ["Left","Top","Right","Bottom"].iter().enumerate() {
                    ui.horizontal(|ui| {ui.label(*label); ui.add(egui::DragValue::new(&mut draft.crop[i]).range(0..=4095).suffix(" px"));});
                }
                if ui.button("Reset crop").clicked() {draft.crop=[0;4];}
            });
            ui.horizontal(|ui| {
                apply=ui.button(if saved.enabled {"Apply camera settings"} else {"Connect camera"}).clicked();
                stop=ui.add_enabled(saved.enabled,egui::Button::new("Disconnect")).clicked();
            });
            ui.small("Connected cameras reopen with this project. Changes apply to every layer using this camera.");
        });
        let mut aspect_changed = false;
        if let Some([width, height]) = self.captures.camera_dimensions(id)
            && ui.button("Match camera aspect ratio").clicked()
        {
            let canvas = &self.project.canvases[self.canvas];
            let scene_id = canvas.scene;
            let ratio = width as f32 / height as f32 * canvas.height as f32 / canvas.width as f32;
            if let Some(layer) = self
                .project
                .scenes
                .iter_mut()
                .find(|s| s.id == scene_id)
                .and_then(|s| self.layer.and_then(|i| s.layers.get_mut(i)))
                && !layer.locked
            {
                layer.rect[3] = layer.rect[2] / ratio;
                aspect_changed = true;
            }
        }
        if apply || stop {
            draft.enabled = !stop;
            let next = draft.clone();
            let validation = next.validate().and_then(|_| {
                if next.network && next.enabled {
                    crate::camera::droidcam_url(&next).map(|_| ())
                } else {
                    Ok(())
                }
            });
            match validation {
                Ok(()) => {
                    if let Some(source) = self.project.sources.iter_mut().find(|s| s.id == id) {
                        source.kind = SourceKind::VideoCamera(next);
                    }
                    self.changed();
                }
                Err(e) => self.message = e.to_string(),
            }
        }
        if aspect_changed {
            self.changed();
        }
    }

    fn stream_details(&mut self, ui: &mut egui::Ui) {
        let mut youtube_public = self.stream_config.youtube_public;
        ui.strong("Broadcast information");
        if ui.button("Connections").clicked() {
            self.page = Page::Settings;
            self.settings = Settings::Accounts;
        }
        for (index, account) in self
            .accounts
            .iter_mut()
            .enumerate()
            .filter(|(_, a)| a.connected)
        {
            ui.push_id(("broadcast_account",index),|ui| {ui.collapsing(account.platform.name(),|ui|{
                ui.label(&account.status);
                ui.add_enabled_ui(account.connected && !account.operation_busy,|ui|{
                    ui.small("Connected broadcasts and chat update automatically.");
                    if index==1 {
                        let mut selected=None;
                        egui::ComboBox::from_id_salt("broadcast").selected_text(account.broadcasts.iter().find(|b|b.id==account.broadcast).map(|b|b.name.as_str()).unwrap_or(if account.broadcast.is_empty(){"Waiting for your broadcast…"}else{"Newly created broadcast"})).show_ui(ui,|ui|{for b in &account.broadcasts {if ui.selectable_label(b.id==account.broadcast,&b.name).clicked(){selected=Some(b.id.clone());}}});
                        if let Some(id)=selected{account.send(crate::accounts::Command::Select(id));}
                        ui.checkbox(&mut youtube_public,"Make it public when I go live").on_hover_text("As your stream starts going to YouTube, Rosorah switches the broadcast to public, so its recording stays public too");
                    }
                    ui.label("Title");ui.text_edit_singleline(&mut account.title);
                    if index==0 {ui.text_edit_singleline(&mut account.category_query);if ui.button("Find category").clicked(){account.send(crate::accounts::Command::Search(account.category_query.clone()));}}
                    egui::ComboBox::from_id_salt("category").selected_text(account.categories.iter().find(|c|c.id==account.category).map(|c|c.name.as_str()).unwrap_or("Category")).show_ui(ui,|ui|{for c in &account.categories{ui.selectable_value(&mut account.category,c.id.clone(),&c.name);}});
                    if ui.add_enabled(!account.title.trim().is_empty()&&(index==0||!account.broadcast.is_empty()),egui::Button::new("Update title & category")).clicked(){account.send(crate::accounts::Command::Update{title:account.title.clone(),category:account.category.clone()});}
                    if index==1 {
                        ui.separator();
                        egui::ComboBox::from_id_salt("stream_key").selected_text(account.streams.iter().find(|c|c.id==account.draft.stream).map(|c|c.name.as_str()).unwrap_or("Automatic stream key")).show_ui(ui,|ui|{ui.selectable_value(&mut account.draft.stream,String::new(),"Create and link a key automatically");for c in &account.streams{ui.selectable_value(&mut account.draft.stream,c.id.clone(),&c.name);}});
                        if ui.add_enabled(!account.broadcast.is_empty()&&!account.draft.stream.is_empty(),egui::Button::new("Link selected broadcast to this key")).clicked(){account.send(crate::accounts::Command::Bind(account.draft.stream.clone()));}
                        ui.small("Use this same stream key in Streaming services. Linking does not start your stream.");
                        ui.label("Thumbnail (JPEG/PNG, up to 2 MiB)");
                        ui.text_edit_singleline(&mut account.draft.thumbnail);
                        if ui.add_enabled(self.thumbnail_picker.is_none(),egui::Button::new("Choose thumbnail…")).clicked(){
                            let(tx,rx)=mpsc::channel();self.thumbnail_picker=Some(rx);
                            std::thread::spawn(move||{let path=crate::desktop::pick(crate::desktop::Pick::Open,"Choose YouTube thumbnail","").unwrap_or_else(|e|{crate::capture_log::event(e.to_string());None});let _=tx.send(path);});
                        }
                        if let Some(texture)=&self.thumbnail_preview {ui.image((texture.id(),egui::vec2(ui.available_width().min(240.),ui.available_width().min(240.)*texture.size()[1] as f32/texture.size()[0] as f32)));}
                        if ui.add_enabled(!account.broadcast.is_empty()&&!account.draft.thumbnail.is_empty(),egui::Button::new("Upload thumbnail to selected broadcast")).clicked(){account.send(crate::accounts::Command::Thumbnail(account.draft.thumbnail.clone()));}
                        ui.collapsing("Create new YouTube broadcast",|ui|{
                            ui.label("New broadcast title");ui.text_edit_singleline(&mut account.draft.title);
                            ui.label("Description");ui.add(egui::TextEdit::multiline(&mut account.draft.description).desired_rows(3));
                            egui::ComboBox::from_id_salt("privacy").selected_text(&account.draft.privacy).show_ui(ui,|ui|{for value in ["unlisted","private","public"]{ui.selectable_value(&mut account.draft.privacy,value.into(),value);}});
                            ui.label("Schedule · YYYY-MM-DD HH:MM · local time");ui.text_edit_singleline(&mut account.draft.schedule);ui.small("Leave blank to prepare for a stream starting shortly.");
                            egui::ComboBox::from_id_salt("new_category").selected_text(account.categories.iter().find(|c|c.id==account.draft.category).map(|c|c.name.as_str()).unwrap_or("Gaming")).show_ui(ui,|ui|{for c in &account.categories{ui.selectable_value(&mut account.draft.category,c.id.clone(),&c.name);}});
                            ui.label("Audience");for (value,label) in [(false,"Not made for kids"),(true,"Made for kids")]{if ui.radio(account.draft.audience_set&&account.draft.made_for_kids==value,label).clicked(){account.draft.audience_set=true;account.draft.made_for_kids=value;}}
                            ui.checkbox(&mut account.draft.auto_start,"Go live on YouTube when encoder starts");
                            ui.small("Uses the thumbnail and stream-key choice above. Creating a broadcast does not start Rosorah's encoder.");
                            if ui.add_enabled(account.draft.body().is_ok(),egui::Button::new("Create YouTube broadcast")).clicked(){account.send(crate::accounts::Command::Create(account.draft.clone()));}
                        });
                        if !account.broadcast.is_empty(){ui.hyperlink_to("Open YouTube broadcast",format!("https://www.youtube.com/watch?v={}",account.broadcast));}
                    }
                });
                if account.operation_busy{ui.spinner();ui.label("Applying broadcast changes…");}
            });});
        }
        if self.kick_account.connected {
            let account = &mut self.kick_account;
            ui.push_id("broadcast_kick",|ui| {ui.collapsing("Kick",|ui|{
                ui.label(&account.status);
                ui.small("Update your Kick title and category. Kick starts the broadcast when Rosorah streams to your Kick connection.");
                ui.label("Title");ui.text_edit_singleline(&mut account.title);
                ui.text_edit_singleline(&mut account.category_query);
                if ui.button("Find category").clicked(){account.send(crate::accounts::Command::Search(account.category_query.clone()));}
                egui::ComboBox::from_id_salt("kick_category").selected_text(account.categories.iter().find(|c|c.id==account.category).map(|c|c.name.as_str()).unwrap_or("Category")).show_ui(ui,|ui|{for c in &account.categories{ui.selectable_value(&mut account.category,c.id.clone(),&c.name);}});
                if ui.add_enabled(!account.title.trim().is_empty(),egui::Button::new("Update title & category")).clicked(){account.send(crate::accounts::Command::Update{title:account.title.clone(),category:account.category.clone()});}
            });});
        }
        if youtube_public != self.stream_config.youtube_public {
            self.stream_config.youtube_public = youtube_public;
            self.save_stream_settings();
        }
    }

    /// Tells the streamer when Twitch's shared chat starts, changes or ends, and when an invite
    /// arrives (the card under Chat may be out of sight).
    fn costream_notices(&mut self) {
        let shared = &self.accounts[0].shared_chat;
        if *shared != self.shared_chat_seen {
            let names: Vec<_> = shared.iter().map(|(_, name)| name.as_str()).collect();
            let text = if names.is_empty() {
                "Shared chat ended.".to_owned()
            } else {
                format!("You're now sharing chat with {}. Their chat shows in yours, marked ⇄.", names.join(", "))
            };
            self.shared_chat_seen = shared.clone();
            self.tell(text);
        }
        let invite = self.browser_chat.invite.as_ref().map_or("", |i| i.text.as_str());
        if invite != self.invite_told {
            self.invite_told = invite.to_owned();
            if !invite.is_empty() {
                self.tell(format!("Co-stream invite: {}. Accept or decline it under Chat.", invite.trim_end_matches('.')));
            }
        }
        if self.browser_chat.dashboard_signin && !self.signin_hinted {
            self.signin_hinted = true;
            self.tell("To see co-stream invites, sign in to Twitch once: Settings > Accounts > Twitch > Sign in for invites.".into());
        }
    }
    /// A co-stream invite, sliding in under the chat and away once it's answered.
    fn invite_card(&mut self, ui: &mut egui::Ui) {
        // The chat panel keeps room for it under this same id.
        let id = ui.make_persistent_id("costream-invite");
        // Twitch's pop-up went away unanswered: the card points to Twitch for a minute, then goes.
        if self.browser_chat.invite.as_ref().and_then(|i| i.gone).is_some_and(|t| t.elapsed().as_secs() >= 60) {
            self.browser_chat.forget_invite();
        }
        if let Some(invite) = &self.browser_chat.invite {
            if self.invite_shown.as_ref().is_none_or(|old| old.text != invite.text) {
                self.invite_flash = None;
            }
            self.invite_shown = Some(invite.clone());
        }
        let open = ui.ctx().animate_bool_with_time(id, self.browser_chat.invite.is_some(), 0.3);
        if open <= 0. {
            self.invite_shown = None;
            return;
        }
        let Some(invite) = self.invite_shown.clone() else { return };
        // Once it has slid all the way in, it flashes yellow one time.
        let now = ui.input(|i| i.time);
        if open >= 1. && self.invite_flash.is_none() {
            self.invite_flash = Some(now);
        }
        const FLASH: f64 = 0.7;
        let flash = self.invite_flash.map_or(0., |start| now - start);
        let glow = if flash < FLASH { (std::f64::consts::PI * flash / FLASH).sin() as f32 } else { 0. };
        if self.invite_flash.is_some() && flash < FLASH {
            ui.ctx().request_repaint();
        }
        let full = ui.ctx().data(|d| d.get_temp::<f32>(id)).unwrap_or(96.);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), full * open), egui::Sense::hover());
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), full.max(1.)))));
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        let accent = self.accent();
        let yellow = egui::Color32::from_rgb(255, 214, 10);
        let shown = egui::Frame::new()
            .fill(crate::appearance::current().card.lerp_to_gamma(yellow, glow * 0.6))
            .stroke(egui::Stroke::new(1.5_f32, accent.lerp_to_gamma(yellow, glow)))
            .corner_radius(crate::appearance::current().panel_corners / 2)
            .inner_margin(10)
            .show(&mut child, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Twitch co-stream invite").small().strong().color(accent));
                ui.label(&invite.text);
                ui.horizontal_wrapped(|ui| {
                    // Accepting opens Twitch's Stream Together page in your browser, where you join
                    // with camera and mic; that works even after Twitch's pop-up has closed.
                    if !invite.link.is_empty() {
                        if ui.add(egui::Button::new(RichText::new("Accept").strong().color(on_accent(accent))).fill(accent)).on_hover_text("Opens Stream Together on Twitch in your browser").clicked() {
                            let _ = crate::desktop::open(&invite.link);
                            self.browser_chat.forget_invite();
                        }
                    } else if invite.gone.is_some() {
                        note(ui, "Twitch's pop-up closed, so answer it on Twitch.");
                    } else if ui.add(egui::Button::new(RichText::new("Accept").strong().color(on_accent(accent))).fill(accent)).clicked() {
                        self.browser_chat.answer_invite(true);
                    }
                    let decline = if invite.close_only { "Not now" } else { "Decline" };
                    if invite.gone.is_none() && invite.can_decline && ui.button(decline).clicked() {
                        self.browser_chat.answer_invite(false);
                    }
                    if invite.link.is_empty() && ui.button("Open on Twitch").on_hover_text("Shows your Stream Manager, where the invite came in").clicked() {
                        self.browser_chat.send(serde_json::json!({"type":"open_platform","platform":"Twitch dashboard"}));
                    }
                    if invite.gone.is_some() && ui.button("Dismiss").clicked() {
                        self.browser_chat.forget_invite();
                    }
                });
            });
        let measured = shown.response.rect.height() + ui.spacing().item_spacing.y;
        if (measured - full).abs() > 0.5 {
            ui.ctx().data_mut(|d| d.insert_temp(id, measured));
        }
    }
    fn chat_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if self.filter.is_some() || self.events_only {
                note(ui, format!("Showing {}", if self.events_only { "events only".to_owned() } else { self.filter.map_or("all", |p| p.name()).to_owned() }));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.menu_button("⋯", |ui| {
                ui.checkbox(&mut self.events_only, "Events only");
                ui.selectable_value(&mut self.filter, None, "All platforms");
                for p in [Platform::Twitch, Platform::YouTube, Platform::Kick] {
                    ui.selectable_value(&mut self.filter, Some(p), p.name());
                }
                if ui.button("Connections & troubleshooting").clicked() {
                    self.page = Page::Settings;
                    self.settings = Settings::Accounts;
                    ui.close();
                }
                if ui.button("Open browser chat window").clicked() {
                    let _ = self.browser_chat.start();
                    ui.close();
                }
            }));
        });
        if self.browser_chat.installing() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.small("Preparing chat…");
            });
        }
        if !self.accounts[0].shared_chat.is_empty() {
            let names: Vec<_> = self.accounts[0].shared_chat.iter().map(|(_, name)| name.as_str()).collect();
            ui.label(RichText::new(format!("⇄ Sharing chat with {}", names.join(", "))).small().color(platform_color(Platform::Twitch)));
        }
        let mut connected: Vec<_> = self
            .accounts
            .iter()
            .filter(|a| a.connected)
            .map(|a| a.platform.name())
            .collect();
        if self.kick_account.connected {
            connected.push("Kick");
        }
        if !self.stream_config.tiktok_username.trim().is_empty() {
            connected.push("TikTok");
        }
        let mut chat_action: Option<ChatAction> = None;
        let composer_measure = ui.make_persistent_id("chat-composer-height");
        let reserve = ui
            .ctx()
            .data(|d| d.get_temp::<f32>(composer_measure))
            .unwrap_or(140.);
        // Without a composer, room is still kept for an invite card sliding in.
        let invite_id = ui.make_persistent_id("costream-invite");
        let invite = ui.ctx().animate_bool_with_time(invite_id, self.browser_chat.invite.is_some(), 0.3)
            * ui.ctx().data(|d| d.get_temp::<f32>(invite_id)).unwrap_or(96.);
        let height =
            (ui.available_height() - if connected.is_empty() { 8. + invite } else { reserve }).max(40.);
        egui::ScrollArea::vertical()
            .id_salt("messages")
            .auto_shrink([false, false])
            .max_height(height)
            .min_scrolled_height(height)
            .stick_to_bottom(true)
            .show_viewport(ui, |ui, viewport| {
                ui.set_min_height(height);
                let origin = ui.min_rect().top();
                // Registered before the messages so a message's own menu wins where they overlap.
                let background = ui.interact(ui.clip_rect(), ui.id().with("chat-background"), egui::Sense::click());
                if self.chat.messages().next().is_none() {
                    ui.add_space(16.);
                    note(
                        ui,
                        if self.accounts.iter().any(|a|a.connected) || self.kick_account.connected {"Chat is connected. Messages will appear here."}else{"Connect your accounts in Settings > Accounts. Chat starts automatically."},
                    );
                    if !self.accounts.iter().any(|a| a.connected || a.busy) && !self.kick_account.connected && !self.kick_account.busy && ui.button("Connect accounts").clicked() {
                        self.page = Page::Settings;
                        self.settings = Settings::Accounts;
                    }
                }
                // Only messages near the visible part are laid out; the rest are skipped by their
                // last measured height, so a long chat costs the same as a short one each frame.
                let layout = (ui.available_width().round() as i32, self.project.preferences.chat_font_size.to_bits());
                if self.chat_heights.0 != layout || self.chat_heights.1.len() > 2000 {
                    self.chat_heights = (layout, Default::default());
                }
                let mut y = ui.cursor().top() - origin;
                let mut skipped = 0.;
                let near = viewport.expand2(egui::vec2(0., 300.));
                for m in self
                    .chat
                    .messages()
                    .filter(|m| self.filter.is_none_or(|p| p == m.platform) && (!self.events_only || m.id.starts_with("event-") || m.text.starts_with("★ ")))
                {
                    let key = (m.platform, m.id.clone());
                    if let Some(&h) = self.chat_heights.1.get(&key)
                        && (y + h < near.min.y || y > near.max.y)
                    {
                        skipped += h;
                        y += h;
                        continue;
                    }
                    if skipped > 0. {
                        ui.add_space(skipped);
                        skipped = 0.;
                    }
                    let before = ui.cursor().top();
                    // Right-click anywhere on the message; it senses clicks underneath its contents,
                    // so their own hover text still shows.
                    ui.scope_builder(egui::UiBuilder::new().sense(egui::Sense::click()), |ui| egui::Frame::new()
                        .fill(crate::appearance::current().card)
                        .corner_radius(6)
                        .inner_margin(9)
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(
                                    RichText::new(m.platform.name())
                                        .small()
                                        .color(platform_color(m.platform)),
                                );
                                if !m.metadata.channel.is_empty() {
                                    ui.label(RichText::new(format!("⇄ {}", m.metadata.channel)).small().color(muted()))
                                        .on_hover_text(format!("Sent in {}'s chat", m.metadata.channel));
                                }
                                ui.strong(&m.author);
                                note(
                                    ui,
                                    format!(
                                        "| {}",
                                        format_time(
                                            m.received_ms,
                                            self.project.preferences.timestamps_24h
                                        )
                                    ),
                                );
                            });
                            self.browser_chat.rich.draw(ui,&format!("{}:{}",m.platform.name(),m.id),&m.text,self.project.preferences.chat_font_size);
                        }))
                        .response
                        .context_menu(|ui| message_menu(ui, m, connected.contains(&m.platform.name()), &mut chat_action));
                    ui.add_space(3.);
                    let h = ui.cursor().top() - before;
                    self.chat_heights.1.insert(key, h);
                    y += h;
                }
                if skipped > 0. {
                    ui.add_space(skipped);
                }
                background.context_menu(|ui| {
                    let mut any = false;
                    for platform in [Platform::Twitch, Platform::YouTube, Platform::Kick, Platform::TikTok] {
                        if connected.contains(&platform.name()) {
                            any = true;
                            if ui.button(format!("Clear {} chat", platform.name())).on_hover_text("Removes these messages from this panel only").clicked() {
                                chat_action = Some(ChatAction::Clear(Some(platform)));
                            }
                        }
                    }
                    if ui.button("Clear all chats").on_hover_text("Removes every message from this panel only").clicked() {
                        chat_action = Some(ChatAction::Clear(None));
                    }
                    if !any {
                        note(ui, "Connect an account to clear a single platform's chat.");
                    }
                    if connected.contains(&"Twitch") {
                        ui.separator();
                        ui.menu_button("Clear Twitch chat for everyone", |ui| {
                            note(ui, "Deletes every message in your Twitch chat for all viewers. This can't be undone.");
                            if ui.button("Yes, clear it for everyone").clicked() {
                                chat_action = Some(ChatAction::Moderate(Platform::Twitch, crate::accounts::Moderation {
                                    action: crate::accounts::ModAction::ClearChat,
                                    user: String::new(),
                                    author: String::new(),
                                    message: String::new(),
                                    text: String::new(),
                                }));
                            }
                        });
                    }
                });
            });
        match chat_action.take() {
            Some(ChatAction::Moderate(platform, action)) => {
                let account = match platform {
                    Platform::Twitch => &mut self.accounts[0],
                    Platform::YouTube => &mut self.accounts[1],
                    _ => &mut self.kick_account,
                };
                self.browser_chat.post_status = if account.moderate(action) {
                    format!("{}: working…", platform.name())
                } else {
                    format!("Connect {} in Settings > Accounts to moderate.", platform.name())
                };
            }
            Some(ChatAction::Hide(platform, id)) => self.chat.remove(platform, &id),
            Some(ChatAction::HideFromWidgets(author)) => {
                let mut any = false;
                for source in &mut self.project.sources {
                    if let SourceKind::Widget(w) = &mut source.kind
                        && w.kind == widgets::Kind::Chat
                    {
                        w.hide_chatter(&author);
                        any = true;
                    }
                }
                if any {
                    // The widgets take the new settings first, then drop what's already showing.
                    self.captures.sync_widgets(&self.project);
                    self.captures.widgets.drop_hidden();
                    self.changed();
                    self.tell(format!("{author} won't show in your chat widgets. Undo this in the chat widget's settings."));
                } else {
                    self.tell("Add a chat widget first; then hidden chatters stay out of it.".into());
                }
            }
            Some(ChatAction::Clear(Some(platform))) => self.chat.clear_author(platform, ""),
            Some(ChatAction::Clear(None)) => self.chat.clear(),
            None => {}
        }
        let composer_top = ui.cursor().top();
        self.invite_card(ui);
        ui.separator();
        if connected.is_empty() {
            return;
        }
        let mut destinations = Vec::new();
        // Colored chips choose where messages go; a hollow dot means the platform is still waiting.
        let youtube_waiting = self.accounts[1].connected && self.accounts[1].broadcast.is_empty();
        let chip = |ui: &mut egui::Ui, platform: Platform, on: bool, waiting: bool| {
            let color = if on { platform_color(platform) } else { muted() };
            let text = format!("{} {}", if waiting { "◌" } else if on { "●" } else { "○" }, platform.name());
            ui.add(egui::Button::new(RichText::new(text).small().color(color)).selected(false).frame(on))
                .on_hover_text(if waiting { format!("{} chat starts when your stream does", platform.name()) } else if on { "Sending here · click to stop".into() } else { "Not sending here · click to include".into() })
                .clicked()
        };
        ui.horizontal_wrapped(|ui| {
            for (index, (name, platform)) in [("Twitch", Platform::Twitch), ("YouTube", Platform::YouTube)].into_iter().enumerate() {
                if !connected.contains(&name) {
                    continue;
                }
                let selected = self.project.preferences.workspace.chat_destinations[index];
                if chip(ui, platform, selected, platform == Platform::YouTube && youtube_waiting) {
                    self.project.preferences.workspace.chat_destinations[index] = !selected;
                    self.dirty = true;
                }
            }
            if connected.contains(&"Kick") {
                let selected = self.project.preferences.workspace.kick_chat;
                if chip(ui, Platform::Kick, selected, false) {
                    self.project.preferences.workspace.kick_chat = !selected;
                    self.dirty = true;
                }
            }
            if connected.contains(&"TikTok") && self.stream_config.tiktok_send {
                let selected = self.tiktok_chat;
                if chip(ui, Platform::TikTok, selected, false) {
                    self.tiktok_chat = !selected;
                }
            }
        });
        for (index, name) in ["Twitch", "YouTube"].iter().enumerate() {
            if connected.contains(name)
                && self.project.preferences.workspace.chat_destinations[index]
            {
                destinations.push((*name).to_owned());
            }
        }
        if connected.contains(&"Kick") && self.project.preferences.workspace.kick_chat {
            destinations.push("Kick".to_owned());
        }
        if connected.contains(&"TikTok") && self.stream_config.tiktok_send && self.tiktok_chat {
            destinations.push("TikTok".to_owned());
        }
        let editor_id = ui.make_persistent_id("chat-composer");
        let enter = ui.memory(|m| m.has_focus(editor_id))
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        let row = ui.text_style_height(&egui::TextStyle::Body);
        let connected_names = connected.clone();
        ui.horizontal(|ui| {
        egui::ScrollArea::vertical()
            .id_salt("composer-scroll")
            .max_height(row * 5. + 8.)
            .max_width((ui.available_width() - 40.).max(60.))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut self.browser_chat.draft)
                        .id(editor_id)
                        .desired_rows(1)
                        .desired_width(f32::INFINITY)
                        .hint_text("Message your chat…")
                        .return_key(egui::KeyboardShortcut::new(
                            egui::Modifiers::SHIFT,
                            egui::Key::Enter,
                        )),
                );
            });
            let sources: Vec<crate::emote_picker::Source> = connected_names
                .iter()
                .filter(|name| ["Twitch", "YouTube", "Kick"].contains(name))
                .map(|name| {
                    let account = match *name {
                        "Twitch" => &self.accounts[0],
                        "YouTube" => &self.accounts[1],
                        _ => &self.kick_account,
                    };
                    let mut emotes = if *name == "YouTube" { self.browser_chat.youtube_emotes.clone() } else { account.emotes.clone() };
                    for (emote, src) in self.browser_chat.rich.recent(name) {
                        if !emotes.iter().any(|e| e.src == src || e.name == emote) {
                            emotes.push(crate::emote_picker::seen_emote(name, &emote, &src));
                        }
                    }
                    let note = if !account.emote_note.is_empty() {
                        account.emote_note.clone()
                    } else if *name == "YouTube" && self.browser_chat.youtube_emotes.is_empty() {
                        "YouTube's emotes appear once your chat page has loaded while you're signed in to YouTube in Rosorah's chat window. Emoji are on the Emoji tab.".into()
                    } else {
                        String::new()
                    };
                    crate::emote_picker::Source { label: name, emotes, note }
                })
                .collect();
            let button = match crate::twemoji::texture(ui.ctx(), "😀") {
                Some(t) => egui::Button::image(egui::Image::new((t.id(), egui::vec2(18., 18.)))),
                None => egui::Button::new("😀"),
            };
            let mut wanted = Vec::new();
            let mut picked = None;
            egui::containers::menu::MenuButton::from_button(button)
                .config(egui::containers::menu::MenuConfig::new().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside))
                .ui(ui, |ui| {
                    picked = self.emote_picker.show(ui, &self.browser_chat.rich, &sources, &mut wanted);
                })
                .0
                .on_hover_text("Emoji & emotes");
            if let Some(text) = picked {
                // Emotes need spaces around them, but not at the start of a message.
                if self.browser_chat.draft.is_empty() || self.browser_chat.draft.ends_with(' ') {
                    self.browser_chat.draft.push_str(text.trim_start());
                } else {
                    self.browser_chat.draft.push_str(&text);
                }
            }
            self.browser_chat.want_images(wanted);
        });
        let label = match destinations.as_slice() {
            [] => "Choose a platform".into(),
            [one] => format!("Send to {one}"),
            [_, _] => "Send to both".into(),
            _ => "Send to all".into(),
        };
        let enabled = !destinations.is_empty()
            && !self.browser_chat.posting()
            && !self.browser_chat.draft.trim().is_empty();
        if (ui.add_enabled(enabled, egui::Button::new(label)).clicked() || enter) && enabled {
            self.browser_chat.post_to(&destinations);
        }
        if !self.browser_chat.post_status.is_empty() {
            ui.small(&self.browser_chat.post_status);
        }
        let measured = (ui.cursor().top() - composer_top).max(40.);
        if (measured - reserve).abs() > 0.5 {
            ui.ctx()
                .data_mut(|d| d.insert_temp(composer_measure, measured));
            ui.ctx().request_repaint();
        }
    }
    fn record_demo(&mut self) {
        let folder = PathBuf::from(&self.record_folder).join(format!("test-{}", now_ms()));
        if let Err(e) = std::fs::create_dir_all(&folder).and_then(|_| {
            self.project
                .save(&folder.join("project.json"))
                .map_err(std::io::Error::other)
        }) {
            self.message = format!("Could not create recording: {e}");
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.recorder = Some(rx);
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<PathBuf> {
                let output = std::process::Command::new(std::env::current_exe()?)
                    .args(["--render-demo", "--seconds", "3", "--project"])
                    .arg(folder.join("project.json"))
                    .arg("--output")
                    .arg(&folder)
                    .output()?;
                if !output.status.success() {
                    anyhow::bail!("{}", String::from_utf8_lossy(&output.stderr));
                }
                Ok(folder)
            })()
            .map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
        self.message = "Rendering horizontal and vertical test recordings".into();
    }
    fn canvas_panel(&mut self, ui: &mut egui::Ui, vertical: bool) {
        if ui.ctx().input(|i| i.viewport().minimized.unwrap_or(false)) { return; }

        let candidates: Vec<_> = self
            .project
            .canvases
            .iter()
            .enumerate()
            .filter(|(_, c)| (c.height > c.width) == vertical)
            .map(|(i, c)| (i, c.id, c.name.clone()))
            .collect();
        if candidates.is_empty() {
            ui.label("Add a canvas in Settings > Video & recording.");
            return;
        }
        let slot = usize::from(vertical);
        let mut chosen = self.preview_canvases[slot]
            .filter(|id| candidates.iter().any(|(_, c, _)| c == id))
            .unwrap_or(candidates[0].1);
        if candidates.len() > 1 {
            egui::ComboBox::from_id_salt(("canvas_choice", slot))
                .selected_text(
                    candidates
                        .iter()
                        .find(|(_, id, _)| *id == chosen)
                        .map(|(_, _, n)| n.as_str())
                        .unwrap_or("Canvas"),
                )
                .show_ui(ui, |ui| {
                    for (_, id, name) in &candidates {
                        ui.selectable_value(&mut chosen, *id, name);
                    }
                });
        }
        if self.preview_canvases[slot] != Some(chosen) {
            self.preview_canvases[slot] = Some(chosen);
            if vertical {
                self.vertical_revision = true;
                self.vertical_size = [0, 0];
            } else {
                self.revision = true;
                self.preview_size = [0, 0];
            }
        }
        let index = candidates
            .iter()
            .find(|(_, id, _)| *id == chosen)
            .unwrap()
            .0;
        let old = self.canvas;
        let layer = self.layer;
        let old_revision = self.revision;
        self.canvas = index;
        if old != index {
            self.layer = self.selections.get(&self.project.canvases[index].scene).and_then(|v|v.last().copied());
        }
        if vertical {
            std::mem::swap(&mut self.preview, &mut self.vertical_texture);
            std::mem::swap(&mut self.preview_size, &mut self.vertical_size);
            std::mem::swap(&mut self.last_preview, &mut self.vertical_last);
            self.revision = self.vertical_revision;
        }
        let ctx = ui.ctx().clone();
        let studio = !vertical && self.studio_mode_for(&self.project.canvases[index].clone());
        if studio {
            // Preview on the left (click scenes and edit here), Program on the right (what viewers see).
            ui.columns(2, |columns| {
                columns[0].label(RichText::new("◐ PREVIEW").small().strong().color(self.accent()));
                self.preview(&mut columns[0], &ctx);
                self.program_view(&mut columns[1]);
            });
        } else {
            self.preview(ui, &ctx);
        }
        let selected =
            ui.rect_contains_pointer(ui.max_rect()) && ui.input(|i| i.pointer.any_pressed());
        if vertical {
            self.vertical_revision = self.revision;
            std::mem::swap(&mut self.preview, &mut self.vertical_texture);
            std::mem::swap(&mut self.preview_size, &mut self.vertical_size);
            std::mem::swap(&mut self.last_preview, &mut self.vertical_last);
            self.revision = old_revision || self.vertical_revision;
        }
        if !selected && old != index {
            self.canvas = old;
            self.layer = layer;
        } else if old != index {
            self.revision = true;
        }
    }
    fn preview(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        let mut canvas = self.project.canvases[self.canvas].clone();
        let live_id = canvas.id;
        // Studio Mode: this view shows and edits the scene being prepared, drawn separately from
        // what's live so preparing never starts a transition.
        if self.studio_mode_for(&canvas) {
            canvas.scene = self.studio_preview.filter(|s| self.project.scene(*s).is_some()).unwrap_or(canvas.scene);
            canvas.id = live_id ^ STUDIO_PREVIEW;
        }
        let width = ui.available_width().max(1.);
        let max_height = (ui.available_height() - if self.advanced() { 70. } else { 44. }).max(1.);
        let scale = (width / canvas.width as f32).min(max_height / canvas.height as f32);
        let size = Vec2::new(canvas.width as f32 * scale, canvas.height as f32 * scale);
        let dimensions = if self.panels_animating && self.preview_size[0] > 0 {
            self.preview_size
        } else {
            [
                (size.x as u32).clamp(16, 4096),
                (size.y as u32).clamp(16, 4096),
            ]
        };
        let due = self.animate && self.last_preview.elapsed() >= Duration::from_millis(66);
        if self.revision || dimensions != self.preview_size || due {
            if let Some(gpu) = &mut self.gpu {
                let result = (|| {
                    if self.revision {
                        self.source_uploads
                            .upload(gpu, &self.project, self.frame, false)?;
                    } else if due {
                        self.frame += 1;
                        self.source_uploads
                            .upload(gpu, &self.project, self.frame, true)?;
                    }
                    gpu.render_gpu(&self.project, &canvas, dimensions[0], dimensions[1])
                })();
                match result {
                    Ok(view) => {
                        if let Some(state) = &self.render_state {
                            let mut renderer = state.renderer.write();
                            if let Some(id) = self.preview {
                                renderer.update_egui_texture_from_wgpu_texture(
                                    &state.device,
                                    &view,
                                    eframe::wgpu::FilterMode::Linear,
                                    id,
                                );
                            } else {
                                self.preview = Some(renderer.register_native_texture(
                                    &state.device,
                                    &view,
                                    eframe::wgpu::FilterMode::Linear,
                                ));
                            }
                        }
                        self.gpu_error = None;
                        self.captures.preview_submitted(self.project.scene(canvas.scene).into_iter().flat_map(|s| s.layers.iter()).filter(|l| l.visible).map(|l| l.source));
                    }
                    Err(e) => {
                        if let (Some(id), Some(state)) = (self.preview.take(), &self.render_state) {
                            state.renderer.write().free_texture(&id);
                        }
                        self.gpu_error = Some(e.to_string());
                    }
                }
            }
            self.preview_size = dimensions;
            self.revision = false;
            self.last_preview = Instant::now();
        }
        if self.advanced() {
            note(
                ui,
                format!("{} × {} · {} fps", canvas.width, canvas.height, canvas.fps),
            );
        }
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, max_height), egui::Sense::hover());
        let target = egui::Rect::from_center_size(rect.center(), size);
        if let Some(texture) = &self.preview {
            ui.painter().image(
                *texture,
                target,
                egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.)),
                Color32::WHITE,
            );
            if let Some((text, until)) = &self.active_alert
                && *until > Instant::now()
            {
                let box_rect = egui::Rect::from_center_size(
                    target.center(),
                    Vec2::new((target.width() - 30.).clamp(120., 450.), 70.),
                );
                ui.painter().rect_filled(box_rect, 8., self.accent());
                ui.painter().text(
                    box_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    text,
                    egui::FontId::proportional(17.),
                    Color32::WHITE,
                );
            }
        } else {
            ui.painter().rect_filled(target, 8., panel());
            ui.painter().text(
                target.center(),
                egui::Align2::CENTER_CENTER,
                self.gpu_error.as_deref().unwrap_or("Preview unavailable"),
                egui::FontId::proportional(14.),
                muted(),
            );
        }
        if let Some(si) = self.project.scenes.iter().position(|s| s.id == canvas.scene) {
            let (edited, response) = ui
                .push_id(("preview_edit", canvas.id), |ui| {
                    crate::canvas_edit::edit_multi(
                        ui,
                        target,
                        &mut self.project.scenes[si],
                        &mut self.layer,
                        self.selections.entry(canvas.scene).or_default(),
                        &mut self.canvas_drag,
                        self.keep_aspect,
                        self.project.preferences.snapping,
                        self.crop_layer.is_some_and(|(scene,_)|scene==canvas.scene),
                        self.project.preferences.locked_canvases.contains(&live_id),
                    )
                })
                .inner;
            self.canvas_hits.push((canvas.scene, target));
            if edited {
                self.changed();
            }
            // Right-clicking a layer on the canvas opens the same menu as in the Layers panel.
            if response.secondary_clicked()
                && let Some(point) = response.interact_pointer_pos()
            {
                self.canvas_menu = crate::canvas_edit::layer_at(target, &self.project.scenes[si], point).map(|i| (canvas.scene, i));
                if let Some((scene, i)) = self.canvas_menu {
                    self.select_layer(scene, i, false);
                }
            }
            let mut dropped = None;
            response.context_menu(|ui| match self.canvas_menu.filter(|(scene, i)| *scene == canvas.scene && *i < self.project.scenes[si].layers.len()) {
                Some((_, i)) => {
                    let source = self.project.scenes[si].layers[i].source;
                    let name = self.project.source(source).map(|s| s.name.clone()).unwrap_or_default();
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    ui.label(RichText::new(&name).small().strong());
                    self.layer_menu(ui, si, i, &name, &mut dropped);
                }
                None => {
                    if ui.add_enabled(!self.layer_clipboard.is_empty(), egui::Button::new("Paste")).clicked() {
                        self.paste_layers(si);
                        ui.close();
                    }
                    if self.layer_clipboard.is_empty() {
                        note(ui, "Right-click a layer for its options, or copy one to paste here.");
                    }
                }
            });
            if let Some((from, to)) = dropped {
                self.move_layer(si, from, to);
            }
        }
        self.canvas_layout_tools(ui);
    }

    fn main_studio(&mut self, ctx: &egui::Context) {
        self.panels_animating = false;
        self.scene_dialog_ui(ctx);
        self.web_dock_dialog_ui(ctx);
        self.add_dialog_ui(ctx);
        if self.screenshot.is_none() {
            if crate::window_memory::track(ctx, &mut self.window_placement) {
                self.window_changed = Some(Instant::now());
            }
            // Saved once things settle, not on every step of a drag or resize.
            if self.window_changed.is_some_and(|at| at.elapsed() > Duration::from_secs(1)) {
                self.window_changed = None;
                crate::window_memory::save(&self.window_placement);
            }
        }
        self.sound_rename_ui(ctx);
        self.update_window_ui(ctx);
        // Ctrl+C / Ctrl+V copy and paste layers, or scenes when the Scenes panel was used last,
        // unless a text box has the keyboard.
        if ctx.memory(|m| m.focused().is_none()) && !egui::Popup::is_any_open(ctx) {
            let (copy, paste) = ctx.input(|i| {
                (
                    i.events.iter().any(|e| matches!(e, egui::Event::Copy)) || (i.modifiers.command && i.key_pressed(egui::Key::C)),
                    i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))) || (i.modifiers.command && i.key_pressed(egui::Key::V)),
                )
            });
            let scene = self.project.canvases.get(self.canvas).map(|c| c.scene);
            if self.list_focus == ListFocus::Scenes && let Some(scene) = scene {
                if copy {
                    self.scene_clipboard = Some(scene);
                    let name = self.project.scene(scene).map(|s| s.name.clone()).unwrap_or_default();
                    self.tell(format!("Copied the scene \"{name}\". Press Ctrl+V to paste a copy."));
                }
                if paste && let Some(copied) = self.scene_clipboard {
                    self.paste_scene(copied);
                }
            } else if let Some(si) = scene.and_then(|scene| self.project.scenes.iter().position(|s| s.id == scene)) {
                if copy && let Some(i) = self.layer.filter(|i| *i < self.project.scenes[si].layers.len()) {
                    self.copy_layers(si, i);
                }
                if paste && !self.layer_clipboard.is_empty() {
                    self.paste_layers(si);
                }
            }
        }
        if !self.update_checked && self.screenshot.is_some() && self.update_window {
            self.update_checked = true;
            self.updates.check_as(ctx, "0.0.1");
        }
        if !self.update_checked && self.ui_frames > 2 && self.screenshot.is_none() {
            self.update_checked = true;
            if self.project.preferences.workspace.check_updates {
                self.updates.check(ctx);
                self.last_update_check = Instant::now();
            }
        }
        self.update_tick(ctx);
        self.web_docks.tick();
        // The page itself stays clear so the backdrop shows in the gaps between docked panels.
        egui::CentralPanel::default().frame(egui::Frame::central_panel(&ctx.style()).fill(bg())).show(ctx, |ui| {
            self.workspace.remember_homes();
            if let Some(origin) = ctx.input(|i| i.pointer.primary_pressed().then(|| i.pointer.press_origin()).flatten()) {
                self.dragging_window = self.workspace.floating_at(origin);
            }
            let mut dock =
                std::mem::replace(&mut self.workspace.dock, egui_dock::DockState::new(vec![]));
            let mut style = egui_dock::Style::from_egui(ui.style());
            style.tab_bar.height = 36.;
            style.tab_bar.bg_fill = panel();
            let round = crate::appearance::current().panel_corners;
            style.tab_bar.corner_radius = egui::CornerRadius {
                nw: round,
                ne: round,
                sw: 0,
                se: 0,
            };
            style.tab_bar.hline_color = Color32::TRANSPARENT;
            // A panel on its own (not tabbed with another) gets a full-width title bar.
            style.tab_bar.lone_tab_title = true;
            style.tab.hline_below_active_tab_name = false;
            style.tab.tab_body.bg_fill = panel();
            style.tab.tab_body.stroke = egui::Stroke::NONE;
            style.tab.tab_body.corner_radius = egui::CornerRadius {
                nw: 0,
                ne: 0,
                sw: round,
                se: round,
            };
            style.tab.tab_body.inner_margin = egui::Margin::same(12);
            for tab in [
                &mut style.tab.active,
                &mut style.tab.inactive,
                &mut style.tab.focused,
                &mut style.tab.hovered,
            ] {
                tab.bg_fill = panel();
                tab.outline_color = Color32::TRANSPARENT;
                tab.text_color = crate::appearance::current().text;
                tab.corner_radius = egui::CornerRadius::same(round);
            }
            style.separator.width = 10.;
            style.separator.color_idle = bg();
            style.separator.extra_interact_width = 8.;
            style.separator.color_hovered = self.accent();
            style.separator.color_dragged = self.accent();
            style.overlay.overlay_type = egui_dock::OverlayType::HighlightedAreas;
            // Broad edge zones snap to halves. The center groups tabs; there
            // is no ambiguous floating band between those two destinations.
            style.overlay.feel.center_drop_coverage = 0.4;
            style.overlay.feel.window_drop_coverage = 0.4;
            style.overlay.feel.max_preference_time = 0.0;
            style.overlay.selection_color = self.accent().linear_multiply(0.38);
            style.overlay.selection_stroke_width = 2.;
            style.overlay.surface_fade_opacity = 0.85;
            dock.translations.tab_context_menu.eject_button = "Float panel".into();
            // Panels can be dragged, closed and floated in both Simple and Advanced mode.
            egui_dock::DockArea::new(&mut dock)
                .style(style)
                .draggable_tabs(true)
                .show_close_buttons(true)
                .tab_context_menus(true)
                .window_bounds(ui.max_rect())
                .show_leaf_close_all_buttons(false)
                .show_leaf_collapse_buttons(false)
                .show_inside(ui, &mut StudioTabs(self));
            self.workspace.dock = dock;
            // A tab dragged out past the edge of the main window becomes its own window.
            // The dock ignores drops outside the window, so the tab is still where it started.
            let (released, outside) = ctx.input(|i| {
                (i.pointer.any_released(), i.pointer.latest_pos().is_none_or(|p| !i.content_rect().shrink(4.).contains(p)))
            });
            if released && let Some(panel) = self.dragging_panel.take() && outside {
                self.workspace.pop_out(panel);
            } else if released && let Some((panels, start)) = self.dragging_window.take() && outside {
                // A floating panel stops at the window edge, so a drag that ends outside with the
                // window moved (not resized) takes its panels out into their own windows.
                let moved = panels.first().and_then(|p| self.workspace.floating_rect(*p))
                    .is_some_and(|rect| rect.min != start.min && (rect.size() - start.size()).length() < 1.);
                if moved {
                    for panel in panels {
                        self.workspace.pop_out(panel);
                    }
                }
            }
            if !ctx.input(|i| i.pointer.any_down()) {
                self.dragging_panel = None;
                self.dragging_window = None;
            }
            if let Some(panel) = self.toggle_panel.take() {
                self.workspace.toggle_floating(panel, ui.max_rect());
            }
            if let Some(panel) = self.pop_out_panel.take() {
                self.workspace.pop_out(panel);
            }
        });
    }
    /// Panels popped out of the dock, each drawn in its own native window. Closing one docks it again.
    fn panel_windows(&mut self, ctx: &egui::Context) {
        for mut tab in self.workspace.windows.clone() {
            let id = egui::ViewportId::from_hash_of(("panel-window", tab));
            let builder = egui::ViewportBuilder::default()
                .with_title(format!("{} · Rosorah Studio", self.panel_title(tab)))
                .with_app_id("io.rosorah.Studio")
                .with_inner_size([440., 600.])
                .with_min_inner_size([260., 200.]);
            let mut redock = false;
            ctx.show_viewport_immediate(id, builder, |ctx, _| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::central_panel(&ctx.style()).fill(panel()).inner_margin(12))
                    .show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            // Dragging this handle out of the window and letting go docks the panel again.
                            let handle = ui.add(egui::Label::new(RichText::new(format!("⠿ {}", self.panel_title(tab))).strong()).sense(egui::Sense::drag()))
                                .on_hover_cursor(grab_cursor())
                                .on_hover_text("Drag out of this window and let go to dock it back where it was");
                            if handle.dragged() {
                                ctx.set_cursor_icon(grabbing_cursor());
                            }
                            if handle.drag_stopped()
                                && ctx.input(|i| i.pointer.latest_pos().is_none_or(|p| !i.content_rect().contains(p)))
                            {
                                redock = true;
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                redock |= ui.small_button("Dock in workspace").clicked();
                            });
                        });
                        ui.separator();
                        egui_dock::TabViewer::ui(&mut StudioTabs(self), ui, &mut tab);
                    });
                redock |= ctx.input(|i| i.viewport().close_requested());
            });
            if redock {
                self.workspace.dock_window(tab);
            }
        }
    }
    fn mixer_panel(&mut self, ui: &mut egui::Ui) {
        let track_names: Vec<String> = (0..6).map(|t| self.project.preferences.track_name(t)).collect();
        let view = self.audio.view();
        let scene_id = self.project.canvases[self.canvas].scene;
        let advanced = self.advanced();
        ui.horizontal_wrapped(|ui| {
            if advanced && ui.button("Monitoring & sync…").clicked(){self.tools.audio=true;}
            note(ui, format!("Volumes for {}",self.project.scene(scene_id).map(|s|s.name.as_str()).unwrap_or("this scene")));
            self.add_sound_menu(ui, &view);
            if !advanced {
                return;
            }
            ui.menu_button("Options",|ui| {
                if ui.button("Refresh inputs").clicked(){self.audio.refresh();}
        if advanced { ui.collapsing("Advanced audio",|ui|{
            let old=self.project.preferences.monitor_device.clone();
            // A submenu, not a dropdown: a dropdown inside this menu would close the menu when clicked.
            let current=view.devices.iter().find(|d|d.target==old).map(|d|d.label.clone()).unwrap_or(if old==crate::audio::DEFAULT_MONITOR{"System default output".into()}else{"Monitoring off".into()});
            ui.menu_button(format!("Monitoring: {current}"),|ui|{ui.selectable_value(&mut self.project.preferences.monitor_device,String::new(),"Monitoring off");ui.selectable_value(&mut self.project.preferences.monitor_device,crate::audio::DEFAULT_MONITOR.into(),"System default output");for d in view.devices.iter().filter(|d|d.kind=="Audio/Sink"){ui.selectable_value(&mut self.project.preferences.monitor_device,d.target.clone(),&d.label);}});
            ui.small(&view.monitor_status);ui.small("Use headphones. Monitoring a desktop mix back to the same output can create feedback.");
            if ui.checkbox(&mut self.project.preferences.multitrack_recording,"Record six separate audio tracks in standalone MKV").changed()||old!=self.project.preferences.monitor_device{self.changed();}
            if ui.button("Separate game, microphone and music tracks").clicked(){for c in &mut self.project.audio{c.tracks=[false;6];c.tracks[0]=true;let t=if c.id==crate::game_audio::CHANNEL{1}else if c.name.to_lowercase().contains("mic"){2}else{3};c.tracks[t]=true;}self.project.preferences.multitrack_recording=true;self.changed();}
            ui.small("Track 1 is the stream mix. Route mic/game to other tracks below. The shared stream recording remains Track 1 only.");
        }); }
                ui.small("Volume and mute are saved separately for each scene.");
            });
        });
        if advanced { ui.horizontal(|ui| {
            let old = self.project.preferences.workspace.vertical_audio;
            ui.selectable_value(
                &mut self.project.preferences.workspace.vertical_audio,
                false,
                "Horizontal",
            );
            ui.selectable_value(
                &mut self.project.preferences.workspace.vertical_audio,
                true,
                "Vertical",
            );
            if old != self.project.preferences.workspace.vertical_audio {
                self.dirty = true;
            }
        }); }
        let vertical_audio = advanced && self.project.preferences.workspace.vertical_audio;
        self.audio
            .monitor_device(&self.project.preferences.monitor_device);
        // Alert sounds share one "Alerts" strip, shown where the first of them sits. In simple mode
        // the stinger and transition-screen sounds join it as "Effects", and inputs with nothing
        // chosen are left out until they're set up from + Add sound.
        let mut view = view;
        let mut scene_channels = Vec::new();
        let mut alert_members = Vec::new();
        let mut effect_members = Vec::new();
        let media = &self.project.preferences.transition_media;
        let transitions = [media.stinger, media.screen];
        let hidden = self.project.preferences.workspace.hidden_sounds.clone();
        let names = self.project.preferences.workspace.sound_names.clone();
        for channel in self.project.mixer_channels(scene_id) {
            if hidden.contains(&channel.id) {
                continue;
            }
            let effect = !advanced && channel.media_source.is_some() && transitions.contains(&channel.media_source);
            if !advanced && self.unconnected(&channel) {
                continue;
            }
            if self.project.is_alert_channel(&channel) || effect {
                if alert_members.is_empty() && effect_members.is_empty() && !hidden.contains(&ALERTS_STRIP) {
                    let bus = &self.project.preferences.alert_bus;
                    scene_channels.push(auroracast_core::AudioChannel {
                        id: ALERTS_STRIP,
                        name: if advanced { "Alerts".into() } else { "Effects".into() },
                        gain_db: bus.gain_db,
                        muted: bus.muted,
                        media_source: None,
                        ..channel.clone()
                    });
                } else if channel.monitor && let Some(strip) = scene_channels.iter_mut().find(|c| c.id == ALERTS_STRIP) {
                    strip.monitor = true;
                }
                let peak = view.stereo_peaks.get(&channel.id).copied().unwrap_or([0.; 2]);
                let combined = view.stereo_peaks.entry(ALERTS_STRIP).or_insert([0.; 2]);
                *combined = [combined[0].max(peak[0]), combined[1].max(peak[1])];
                if effect {
                    effect_members.push(channel.id);
                } else {
                    alert_members.push(channel.id);
                }
            } else {
                scene_channels.push(channel);
            }
        }
        if scene_channels.is_empty() {
            note(ui, "Nothing to hear yet. Use + Add sound to add your microphone or your computer's sound.");
        }
        let shown: Vec<Id> = scene_channels.iter().map(|c| c.id).collect();
        let alerts_before = scene_channels.iter().find(|c| c.id == ALERTS_STRIP).cloned();
        for error in [&view.error, &view.capture_error] {
            if !error.is_empty() {
                ui.colored_label(crate::appearance::current().error, error);
            }
        }
        let mut changed = false;
        // Vertical strips are sized to the panel so meters and buttons never fall
        // below it; the non-meter height of a strip is measured on the previous frame.
        let overhead_id = ui.id().with("mixer-strip-overhead");
        let row = ui.spacing().interact_size.y;
        let overhead = ui
            .data(|d| d.get_temp::<f32>(overhead_id))
            .unwrap_or(row * 3. + 32.);
        let strip_height = (ui.available_height() - ui.spacing().scroll.allocated_width() - 4.).max(0.);
        let meter_height = (strip_height - overhead).max(24.);
        let automatic = self.project.preferences.auto_game_audio;
        // Whether Desktop audio is capturing too (then music would be heard twice).
        let desktop_on = self.project.audio.iter().any(|c| c.name == "Desktop audio" && !c.muted && self.audio.enabled.get(&c.id) == Some(&true));
        let sound_settings = &mut self.sound_settings;
        // What a strip's right-click menu asked for; carried out after the strips are drawn.
        let mut strip_action: Option<(Id, SoundAction)> = None;
        // Apps added to the automatic music list; edited from the Music strip's settings.
        let music_apps = &mut self.project.preferences.music_apps;
        let mut music_apps_changed = false;
        // Each strip's filters, edited in its Sound settings window.
        let audio_filters = &mut self.project.preferences.audio_filters;
        let mut refresh = false;
        // Draws one strip; returns what picks it up for reordering (its grip and name).
        let mut draw_channel = |ui: &mut egui::Ui, channel: &mut auroracast_core::AudioChannel| -> egui::Response {
            let mut grab = None;
            let shown_name = names.get(&channel.id).cloned().unwrap_or_else(|| channel.name.clone());
            // Right-clicking anywhere on the strip that isn't a control opens its menu. The area sits
            // under the controls (placed first, at last frame's position) so they keep their clicks.
            let rect_key = egui::Id::new(("sound-strip-rect", channel.id));
            let menu_area = ui.data(|d| d.get_temp::<egui::Rect>(rect_key)).map(|r| ui.interact(r, rect_key.with("menu"), egui::Sense::click()));
            let strip = ui.push_id(channel.id, |ui| {
                egui::Frame::new().inner_margin(egui::Margin::symmetric(0,4)).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y=4.;
                    let levels = view.stereo_peaks.get(&channel.id).copied().unwrap_or([0.;2]);
                    let mut gain = channel.gain_db.max(-60.);
                    let level_text = if gain <= -60. { if advanced { "−∞ dB".into() } else { "Off".into() } } else if advanced {format!("{gain:.1} dB")} else {format!("{:.0}%", 10f32.powf(gain / 20.) * 100.)};
                    ui.horizontal(|ui| {
                        let (grip, handle) = ui.allocate_exact_size(egui::vec2(10., row), egui::Sense::drag());
                        let dot = if handle.hovered() || handle.dragged() { ui.visuals().strong_text_color() } else { ui.visuals().weak_text_color() };
                        for i in 0..6 {
                            let offset = egui::vec2(if i % 2 == 0 { -2.5 } else { 2.5 }, (i / 2) as f32 * 5. - 5.);
                            ui.painter().circle_filled(grip.center() + offset, 1.4, dot);
                        }
                        let name = ui.scope(|ui| {
                            // Side by side, the level sits by the mute button so the name gets the whole row.
                            ui.set_max_width(if vertical_audio { ui.available_width() } else { (ui.available_width() - 52.).max(24.) });
                            ui.add(egui::Label::new(egui::RichText::new(&shown_name).strong()).truncate().sense(egui::Sense::click_and_drag()))
                        }).inner;
                        let handle = handle.union(name);
                        if handle.hovered() && !handle.dragged() {
                            ui.ctx().set_cursor_icon(grab_cursor());
                        }
                        grab = Some(handle.on_hover_text("Drag to reorder"));
                        if !vertical_audio {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.small(&level_text);
                            });
                        }
                    });
                    if !vertical_audio {if advanced {crate::material::meter(ui,levels,false);} else {crate::material::meter_plain(ui,levels);}}
                    ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), if vertical_audio {meter_height+60.}else{30.}), if vertical_audio {egui::Layout::top_down(egui::Align::Center)} else {egui::Layout::left_to_right(egui::Align::Center)},|ui| {
                    ui.spacing_mut().slider_width=if vertical_audio {meter_height}else{(ui.available_width()-150.).max(40.)};
                    let slider=egui::Slider::new(&mut gain,-60.0..=6.0).show_value(false).handle_shape(egui::style::HandleShape::Rect{aspect_ratio:0.35});
                    let response=if vertical_audio {ui.horizontal(|ui| {crate::material::meter_sized(ui,levels,true,meter_height);ui.add(slider.vertical())}).inner}else{ui.add(slider)};
                    if response.changed() {channel.gain_db=if gain<=-60. {-100.}else{gain};changed=true;}
                    let mute = |ui: &mut egui::Ui| ui.selectable_label(channel.muted,if channel.muted {"🔇"}else{"🔊"}).on_hover_text(if channel.muted {"Muted · click to unmute"}else{"Click to mute"}).clicked();
                    let clicked = if vertical_audio { ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), row), egui::Layout::left_to_right(egui::Align::Center), |ui| { ui.small(&level_text); mute(ui) }).inner } else { mute(ui) };
                    if clicked {channel.muted = !channel.muted;changed=true;}
                    if ui.button("⚙").on_hover_text("Sound settings").clicked() {
                        *sound_settings = Some(channel.id);
                    }
                    });
                    // Settings open in a window in the middle of the screen rather than a popup menu,
                    // so choosing a device never closes them halfway through.
                    if *sound_settings == Some(channel.id) {
                    let ctx = ui.ctx().clone();
                    let mut done = false;
                    let modal = pop_modal(&ctx, egui::Id::new(("sound-settings", channel.id))).show(&ctx, |ui| {
                    ui.set_width(400.);
                    ui.spacing_mut().item_spacing.y = 6.;
                    ui.heading(&shown_name);
                    crate::material::meter_plain(ui, levels);
                    note(ui, "Speak or play something; the bar moves when this sound is coming through.");
                    ui.add_space(6.);
                    if automatic && channel.id == crate::game_audio::CHANNEL {
                        ui.label(RichText::new("Following the captured game").strong());
                        note(ui, view.status.get(&channel.id).map_or("Waiting for a captured game", String::as_str));
                    } else if channel.id == ALERTS_STRIP {
                        note(ui, "Every alert sound together. Each alert's own level is in its alert settings.");
                    } else if channel.media_source.is_some() {
                        note(ui, "Sound from a video. It plays while the video is in the visible scene.");
                    } else {
                        let output = channel.name == "Desktop audio";
                        ui.label(RichText::new(if output { "Speakers or headphones to capture" } else { "Device" }).strong());
                        if output {
                            note(ui, "Captures everything playing through this output. Mute game audio to avoid hearing it twice.");
                        }
                        let selected = self.audio.selected.entry(channel.id).or_default();
                        let enabled = self.audio.enabled.entry(channel.id).or_default();
                        let before = selected.clone();
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            egui::ScrollArea::vertical().max_height(220.).auto_shrink([false, true]).show(ui, |ui| {
                                ui.selectable_value(selected, String::new(), "None");
                                if !output {
                                    ui.add_space(4.);
                                    note(ui, "Follow automatically");
                                    ui.selectable_value(selected, crate::app_audio::MUSIC.to_owned(), "🎵  Music apps · Spotify, YouTube Music, TIDAL…")
                                        .on_hover_text("Captures music players as they start and stop. Video in a browser (YouTube, Netflix…) is never included.");
                                }
                                // Grouped under headings, so the rows don't each repeat what kind of device they are.
                                let groups: &[(&str, &str, fn(&str) -> bool)] = if output {
                                    &[("", "Desktop output: ", |k| k == "Audio/Sink")]
                                } else {
                                    &[
                                        ("Microphones and inputs", "Microphone / input: ", |k| k == "Audio/Source"),
                                        ("Sound playing on this computer", "Desktop output: ", |k| k == "Audio/Sink"),
                                    ]
                                };
                                for (heading, prefix, wanted) in groups {
                                    let devices: Vec<_> = view.devices.iter().filter(|d| wanted(&d.kind)).collect();
                                    if devices.is_empty() {
                                        continue;
                                    }
                                    if !heading.is_empty() {
                                        ui.add_space(4.);
                                        note(ui, *heading);
                                    }
                                    for d in devices {
                                        ui.selectable_value(selected, d.serial.clone(), d.label.strip_prefix(prefix).unwrap_or(&d.label));
                                    }
                                }
                                // One row per app, followed by name so it's found again after a restart.
                                let mut apps: Vec<(String, String)> = view.devices.iter().filter(|d| d.kind == "Stream/Output/Audio" && !d.app.is_empty()).map(|d| (d.app.clone(), crate::app_audio::app_name(d))).collect();
                                apps.sort();
                                apps.dedup_by(|a, b| a.0 == b.0);
                                if let Some(app) = crate::app_audio::followed_app(selected).filter(|a| !apps.iter().any(|(id, _)| id == a)) {
                                    apps.push((app.to_owned(), format!("{app} (not playing now)")));
                                }
                                if !output && !apps.is_empty() {
                                    ui.add_space(4.);
                                    note(ui, "Apps");
                                    for (app, name) in apps {
                                        ui.selectable_value(selected, crate::app_audio::app_target(&app), name);
                                    }
                                }
                            });
                        });
                        if *selected == crate::app_audio::MUSIC {
                            // What's being captured now, and other apps to count as music.
                            let playing: Vec<String> = crate::app_audio::resolve(crate::app_audio::MUSIC, &view.devices, music_apps, &[], &Default::default()).iter().map(|d| crate::app_audio::app_name(d)).collect();
                            note(ui, if playing.is_empty() { "No music app is playing right now.".to_owned() } else { format!("Capturing now: {}", playing.join(", ")) });
                            let mut others: Vec<(String, String)> = view.devices.iter().filter(|d| d.kind == "Stream/Output/Audio" && !d.app.is_empty() && !crate::app_audio::browser(d) && !crate::app_audio::is_music(d, &[])).map(|d| (d.app.clone(), crate::app_audio::app_name(d))).collect();
                            for app in music_apps.iter() {
                                if !others.iter().any(|(a, _)| a == app) {
                                    others.push((app.clone(), app.clone()));
                                }
                            }
                            others.sort();
                            others.dedup_by(|a, b| a.0 == b.0);
                            if !others.is_empty() {
                                ui.collapsing("Also include…", |ui| {
                                    for (app, name) in others {
                                        let mut on = music_apps.contains(&app);
                                        if ui.checkbox(&mut on, name).changed() {
                                            if on { music_apps.push(app); } else { music_apps.retain(|a| *a != app); }
                                            music_apps_changed = true;
                                        }
                                    }
                                    note(ui, "Browsers can't be added: they also play videos.");
                                });
                            }
                            if view.devices.iter().any(|d| d.kind == "Audio/Sink") && desktop_on {
                                ui.colored_label(ui.visuals().warn_fg_color, "Desktop audio also captures everything you hear, so music would be heard twice on stream. Mute Desktop audio, or remove it if you don't need it.");
                            }
                        }
                        if *selected != before {
                            *enabled = !selected.is_empty();
                        }
                        if !selected.is_empty() {
                            ui.horizontal_wrapped(|ui| {
                                if ui.button(if *enabled { "Stop" } else { "Start" }).clicked() {
                                    *enabled = !*enabled;
                                }
                                note(ui, if *enabled { view.status.get(&channel.id).map_or("Connecting…", String::as_str) } else { "Stopped" });
                            });
                        }
                        if ui.small_button("Look for new devices").clicked() {
                            refresh = true;
                        }
                    }
                    if channel.name.to_lowercase().contains("mic") || self.audio.selected.get(&channel.id).is_some_and(|serial| view.devices.iter().any(|d| &d.serial == serial && d.kind == "Audio/Source")) {
                        ui.add_space(6.);
                        ui.label(RichText::new("Noise suppression").strong());
                        ui.horizontal(|ui| {
                            for mode in auroracast_core::NoiseSuppression::ALL {
                                changed |= ui.selectable_value(&mut channel.noise_suppression, mode, mode.label()).changed();
                            }
                        });
                        note(ui, "Reduces steady background noise like fans or hum.");
                    }
                    if channel.id != ALERTS_STRIP {
                        ui.add_space(6.);
                        ui.label(RichText::new("Filters").strong());
                        let f = audio_filters.entry(channel.id).or_default();
                        let before = f.clone();
                        ui.horizontal_wrapped(|ui| {
                            ui.checkbox(&mut f.gate, "Noise gate");
                            ui.checkbox(&mut f.compressor, "Compressor");
                            ui.checkbox(&mut f.limiter, "Limiter");
                            ui.checkbox(&mut f.expander, "Expander");
                            ui.checkbox(&mut f.eq, "EQ");
                            ui.checkbox(&mut f.mono, "Mono");
                        });
                        if f.gate {
                            ui.add(egui::Slider::new(&mut f.gate_open, -80.0..=0.0).text("Gate opens above").suffix(" dB"));
                            ui.add(egui::Slider::new(&mut f.gate_release, 20.0..=1000.0).text("Closes over").suffix(" ms").logarithmic(true));
                        }
                        if f.expander {
                            ui.add(egui::Slider::new(&mut f.expand_below, -80.0..=0.0).text("Expand below").suffix(" dB"));
                            ui.add(egui::Slider::new(&mut f.expand_ratio, 1.0..=10.0).text("Ratio"));
                        }
                        if f.compressor {
                            ui.add(egui::Slider::new(&mut f.compress_above, -60.0..=0.0).text("Compress above").suffix(" dB"));
                            ui.add(egui::Slider::new(&mut f.compress_ratio, 1.0..=20.0).text("Ratio").logarithmic(true));
                            ui.add(egui::Slider::new(&mut f.compress_attack, 0.5..=100.0).text("Attack").suffix(" ms").logarithmic(true));
                            ui.add(egui::Slider::new(&mut f.compress_release, 10.0..=1000.0).text("Release").suffix(" ms").logarithmic(true));
                            ui.add(egui::Slider::new(&mut f.makeup, 0.0..=24.0).text("Make-up gain").suffix(" dB"));
                        }
                        if f.eq {
                            ui.add(egui::Slider::new(&mut f.bass, -15.0..=15.0).text("Bass").suffix(" dB"));
                            ui.add(egui::Slider::new(&mut f.middle, -15.0..=15.0).text("Middle").suffix(" dB"));
                            ui.add(egui::Slider::new(&mut f.treble, -15.0..=15.0).text("Treble").suffix(" dB"));
                        }
                        if f.limiter {
                            ui.add(egui::Slider::new(&mut f.limit_to, -30.0..=0.0).text("Never louder than").suffix(" dB"));
                        }
                        ui.add(egui::Slider::new(&mut f.balance, -1.0..=1.0).text("Balance (left ↔ right)").show_value(false));
                        if *f != before {
                            changed = true;
                        }
                        if !f.active() {
                            audio_filters.remove(&channel.id);
                        }
                    }
                    if advanced {
                        ui.add_space(6.);
                        ui.label(RichText::new("Timing and monitoring").strong());
                        changed |= ui.add(egui::DragValue::new(&mut channel.delay_ms).range(0..=2000).suffix(" ms audio delay")).changed();
                        note(ui, "Delay this audio to match a late camera, e.g. DroidCam. Applies to recording and streaming.");
                        if ui.checkbox(&mut channel.monitor, "Listen on monitoring output").changed() {
                            if !channel.monitor {
                                channel.monitor_only = false;
                            }
                            changed = true;
                        }
                        if channel.monitor {
                            changed |= ui.checkbox(&mut channel.monitor_only, "Monitor only (exclude from stream / recording)").changed();
                        }
                        ui.add_space(6.);
                        ui.label(RichText::new("Audio tracks").strong());
                        ui.horizontal_wrapped(|ui| {
                            for (i, route) in channel.tracks.iter_mut().enumerate() {
                                changed |= ui.checkbox(route, &track_names[i]).changed();
                            }
                        });
                        note(ui, "Rename tracks in Settings > Sound > Audio tracks.");
                    }
                    ui.add_space(10.);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new(RichText::new("Done").strong()).min_size(egui::vec2(80., 0.))).clicked() {
                            done = true;
                        }
                    });
                    });
                    if done || modal.should_close() {
                        *sound_settings = None;
                    }
                    }
                });
                ui.add_space(4.);
            }).response;
            ui.data_mut(|d| d.insert_temp(rect_key, strip.rect));
            // The same menu on the strip's name and on its empty space.
            let menu_items = |ui: &mut egui::Ui| -> Option<SoundAction> {
                let mut action = None;
                if ui.button("Rename…").clicked() {
                    action = Some(SoundAction::Rename(shown_name.clone()));
                }
                if ui.button("Hide").on_hover_text("Removes it from this panel; it's still heard on stream").clicked() {
                    action = Some(SoundAction::Hide);
                }
                // Game audio, the alerts strip and video sounds belong to captures and alerts.
                let removable = channel.id != crate::game_audio::CHANNEL && channel.id != ALERTS_STRIP && channel.media_source.is_none();
                if removable && ui.button("Delete").clicked() {
                    action = Some(SoundAction::Delete(shown_name.clone()));
                }
                if action.is_some() {
                    ui.close();
                }
                action
            };
            for response in menu_area.iter().chain(grab.iter()) {
                response.context_menu(|ui| {
                    if let Some(action) = menu_items(ui) {
                        strip_action = Some((channel.id, action));
                    }
                });
            }
            grab.unwrap_or(strip)
        };
        let mut reorder = None;
        let animate = self.project.preferences.workspace.animations;
        let shift_key = ui.id().with(("mixer-shift", vertical_audio));
        // The old index style counts the insertion point before the strip is taken out.
        let to_reorder = |(from, to): (usize, usize)| (shown[from], if to > from { to + 1 } else { to });
        // Empty space in the panel has its own right-click menu; strips drawn on top keep theirs.
        let empty_space = ui.interact(ui.available_rect_before_wrap(), ui.id().with("sound-empty-space"), egui::Sense::click());
        let mut rects = Vec::with_capacity(scene_channels.len());
        if vertical_audio {
            // One row of strips; extra channels scroll sideways instead of wrapping
            // onto rows below the visible panel.
            egui::ScrollArea::both()
                .id_salt("mixer-strips")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        let mut shift = ShiftDrag::begin(ui, shift_key, true, animate, scene_channels.len());
                        for (p, channel) in scene_channels.iter_mut().enumerate() {
                            shift.item(ui, p, |ui| {
                                ui.allocate_ui_with_layout(
                                    egui::vec2(108., strip_height),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        ui.set_max_width(108.);
                                        draw_channel(ui, channel)
                                    },
                                )
                                .inner
                            });
                        }
                        rects.extend_from_slice(&shift.rects);
                        reorder = shift.finish(ui).map(to_reorder);
                    });
                });
            if let Some(rect) = rects.first() {
                let measured = (rect.height() - meter_height).max(0.);
                if (measured - overhead).abs() > 0.5 {
                    ui.data_mut(|d| d.insert_temp(overhead_id, measured));
                    ui.ctx().request_repaint();
                }
            }
        } else {
            egui::ScrollArea::vertical()
                .id_salt("mixer-rows")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let mut shift = ShiftDrag::begin(ui, shift_key, false, animate, scene_channels.len());
                    for (p, channel) in scene_channels.iter_mut().enumerate() {
                        shift.item(ui, p, |ui| draw_channel(ui, channel));
                    }
                    reorder = shift.finish(ui).map(to_reorder);
                });
        }
        if refresh {
            self.audio.refresh();
        }
        if music_apps_changed {
            self.dirty = true;
        }
        match strip_action {
            Some((id, SoundAction::Rename(name))) => self.sound_rename = Some((id, name)),
            Some((id, SoundAction::Hide)) => {
                self.project.preferences.workspace.hidden_sounds.insert(id);
                self.dirty = true;
                self.tell("Hidden. It's still heard on stream; right-click empty space in Sound to show it again.".into());
            }
            Some((id, SoundAction::Delete(name))) if id != crate::game_audio::CHANNEL && id != ALERTS_STRIP => {
                self.project.audio.retain(|c| c.id != id);
                for volumes in self.project.preferences.scene_audio.values_mut() {
                    volumes.remove(&id);
                }
                self.project.preferences.workspace.sound_names.remove(&id);
                self.project.preferences.workspace.hidden_sounds.remove(&id);
                self.changed();
                self.deleted(format!("Deleted the sound “{name}”"));
            }
            _ => {}
        }
        empty_space.context_menu(|ui| {
            let hidden: Vec<(Id, String)> = self
                .project
                .preferences
                .workspace
                .hidden_sounds
                .iter()
                .map(|id| {
                    let name = self.project.preferences.workspace.sound_names.get(id).cloned().or_else(|| self.project.audio.iter().find(|c| c.id == *id).map(|c| c.name.clone()));
                    (*id, name.unwrap_or_else(|| if *id == ALERTS_STRIP { "Alerts".into() } else { "Sound".into() }))
                })
                .collect();
            ui.add_enabled_ui(!hidden.is_empty(), |ui| {
                ui.menu_button("Show hidden sounds", |ui| {
                    for (id, name) in &hidden {
                        if ui.button(name).clicked() {
                            self.project.preferences.workspace.hidden_sounds.remove(id);
                            self.dirty = true;
                            ui.close();
                        }
                    }
                    if hidden.len() > 1 {
                        ui.separator();
                        if ui.button("Show all").clicked() {
                            self.project.preferences.workspace.hidden_sounds.clear();
                            self.dirty = true;
                            ui.close();
                        }
                    }
                });
            });
            ui.separator();
            ui.label(RichText::new("Sound panel").small().strong());
            if advanced {
                let mut side = self.project.preferences.workspace.vertical_audio;
                if ui.radio_value(&mut side, true, "Strips side by side").clicked() || ui.radio_value(&mut side, false, "Strips stacked").clicked() {
                    self.project.preferences.workspace.vertical_audio = side;
                    self.dirty = true;
                }
            }
            if ui.button("Monitoring & sync…").clicked() {
                self.tools.audio = true;
                ui.close();
            }
            if ui.button("Look for new devices").clicked() {
                self.audio.refresh();
                ui.close();
            }
        });
        // Strip positions map to project channels; the Alerts strip stands for its first sound.
        let actual = |id: Id| if id == ALERTS_STRIP { alert_members.first().or(effect_members.first()).copied().unwrap_or(id) } else { id };
        if let Some((id, index)) = reorder
            && let Some(from) = self.project.audio.iter().position(|c| c.id == actual(id))
        {
            let index = shown
                .get(index)
                .and_then(|target| self.project.audio.iter().position(|c| c.id == actual(*target)))
                .unwrap_or(self.project.audio.len());
            let to = if index > from { index - 1 } else { index };
            if to != from {
                let channel = self.project.audio.remove(from);
                self.project.audio.insert(to.min(self.project.audio.len()), channel);
                self.changed();
            }
        }
        if changed {
            for channel in scene_channels {
                if channel.id == ALERTS_STRIP {
                    self.project.preferences.alert_bus = auroracast_core::AlertBus { gain_db: channel.gain_db, muted: channel.muted };
                    for id in &effect_members {
                        self.project.preferences.scene_audio.entry(scene_id).or_default().insert(*id, auroracast_core::SceneVolume { gain_db: channel.gain_db, muted: channel.muted });
                    }
                    // Only settings changed on the strip are copied to every alert sound.
                    let Some(before) = &alerts_before else { continue };
                    for base in self.project.audio.iter_mut().filter(|c| alert_members.contains(&c.id)) {
                        if channel.tracks != before.tracks { base.tracks = channel.tracks; }
                        if channel.delay_ms != before.delay_ms { base.delay_ms = channel.delay_ms; }
                        if channel.monitor != before.monitor { base.monitor = channel.monitor; }
                        if channel.monitor_only != before.monitor_only { base.monitor_only = channel.monitor_only; }
                    }
                    continue;
                }
                let volumes = self.project.preferences.scene_audio.entry(scene_id).or_default();
                volumes.insert(
                    channel.id,
                    auroracast_core::SceneVolume {
                        gain_db: channel.gain_db,
                        muted: channel.muted,
                    },
                );
                if let Some(base) = self.project.audio.iter_mut().find(|c| c.id == channel.id) {
                    base.tracks = channel.tracks;
                    base.delay_ms = channel.delay_ms;
                    base.monitor = channel.monitor;
                    base.monitor_only = channel.monitor_only;
                    base.noise_suppression = channel.noise_suppression;
                }
            }
            self.changed();
        }
    }
    fn settings(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("settings_nav")
            .exact_width(225.)
            .show(ctx, |ui| {
                crate::appearance::glass(ui, egui::CornerRadius::ZERO);
                ui.add_space(16.);
                ui.heading("Settings");
                ui.add_space(14.);
                for (p, label) in [
                    (Settings::Accounts, "Accounts"),
                    (Settings::Video, "Video & recording"),
                    (Settings::Audio, "Sound"),
                    (Settings::Hotkeys, "Hotkeys"),
                    (Settings::Remote, "Remote control"),
                    (Settings::Scripts, "Scripts"),
                    (Settings::Appearance, "Look & feel"),
                    (Settings::Advanced, "Advanced"),
                ] {
                    let selected = self.settings == p || (p == Settings::Accounts && matches!(self.settings, Settings::Donations | Settings::Announcements));
                    if ui.add_sized([ui.available_width(), 40.], egui::Button::new(label).selected(selected)).clicked() {
                        self.settings = p;
                    }
                }
            });
        let live = self.live_stream.is_some() || self.live_recording.is_some();
        egui::CentralPanel::default().show(ctx,|ui|{crate::appearance::glass(ui,egui::CornerRadius::ZERO);egui::ScrollArea::vertical().show(ui,|ui|{
            ui.add_space(16.);let mut changed=false;
            if matches!(self.settings, Settings::Accounts | Settings::Donations | Settings::Announcements) {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.settings, Settings::Accounts, "Streaming platforms");
                    ui.selectable_value(&mut self.settings, Settings::Donations, "Donations");
                    ui.selectable_value(&mut self.settings, Settings::Announcements, "Announcements");
                });
                ui.add_space(8.);
            }
            match self.settings{
                Settings::Appearance=>{ui.heading("Look & feel");note(ui,"Colors, background and chat text.");ui.add_space(18.);card().show(ui,|ui|{
                    changed|=ui.checkbox(&mut self.project.preferences.workspace.animations,"Smooth panel and layer movement").changed();ui.small("Short, on-demand animations; disable for reduced motion.");
                    ui.strong("Accent color");ui.horizontal(|ui|{for color in [[141,101,255],[66,148,255],[33,196,214],[238,99,157],[255,170,71]]{if ui.add(egui::Button::new("   ").fill(Color32::from_rgb(color[0],color[1],color[2]))).clicked(){self.project.preferences.accent=color;changed=true;}}changed|=ui.color_edit_button_srgb(&mut self.project.preferences.accent).changed();});ui.add_space(12.);ui.strong("Chat");changed|=ui.add(egui::Slider::new(&mut self.project.preferences.chat_font_size,10.0..=28.0).text("Text size")).changed();changed|=ui.checkbox(&mut self.project.preferences.timestamps_24h,"24-hour timestamps").changed();
                });changed|=self.look_settings(ui);},
                Settings::Accounts=>{ self.connections_settings(ui); },
                Settings::Advanced=>{ changed |= self.advanced_settings(ui); },
                Settings::Announcements=>self.announce_settings(ui),
                Settings::Hotkeys=>self.hotkey_settings(ui),
                Settings::Remote=>self.remote_settings_page(ui),
                Settings::Scripts=>self.scripts_page(ui),
                Settings::Donations=>{ let (edited, preview) = crate::donations::ui(ui, &mut self.project.preferences.donations, &mut self.donations, &mut self.kofi); changed |= edited; if edited {self.donation_save_due=Some(Instant::now());}
                if let Some(text) = preview { self.active_alert=Some((text,Instant::now()+Duration::from_secs(8))); self.page=Page::Studio; } },
                Settings::Video=>{
                    ui.heading("Video & recording");
                    if live { note(ui, "Stop streaming and recording to change these."); }
                    ui.add_space(16.);
                    ui.add_enabled_ui(!live, |ui| {
                        if self.project.canvases.len() > 1 {
                            card().show(ui,|ui|{
                                ui.set_width(ui.available_width());
                                if ui.checkbox(&mut self.project.preferences.workspace.vertical_video, "Also make a vertical video (TikTok, YouTube Shorts, Reels)").changed() {
                                    self.sync_vertical_panel();
                                    changed = true;
                                }
                                note(ui, "Shows a second, phone-shaped video beside your main one. Leave it off if you only stream to one place.");
                            });
                            ui.add_space(10.);
                        }
                        card().show(ui,|ui|{
                            ui.set_width(ui.available_width());
                            ui.strong("Where recordings are saved");
                            if ui.text_edit_singleline(&mut self.record_folder).changed() {
                                self.project.preferences.recording_directory = self.record_folder.clone(); changed = true;
                            }
                            if self.advanced() {
                                ui.add_space(10.);
                                ui.strong("Output encoder");
                                use auroracast_core::RecordingEncoder;
                                egui::ComboBox::from_id_salt("recording-encoder")
                                    .selected_text(match self.project.preferences.recording_encoder {
                                        RecordingEncoder::Auto => "Auto (prefer GPU)",
                                        RecordingEncoder::Software => "Software",
                                    })
                                    .show_ui(ui, |ui| {
                                        changed |= ui.selectable_value(&mut self.project.preferences.recording_encoder, RecordingEncoder::Auto, "Auto (prefer GPU)").changed();
                                        changed |= ui.selectable_value(&mut self.project.preferences.recording_encoder, RecordingEncoder::Software, "Software").changed();
                                    });
                                note(ui,"Auto checks available GPU encoders before recording or streaming and uses software if none work. The Stream panel shows the chosen encoder; Recording details explains any fallback. MKV video includes the Track 1 audio mix.");
                            }
                        });
                        if self.advanced() {
                            ui.add_space(16.);
                            ui.strong("Videos (canvases)");
                            note(ui,"Choose a video in the preview before starting a recording. Its size and FPS stay fixed until you stop.");
                            for canvas in &mut self.project.canvases {
                                ui.push_id(canvas.id,|ui|{card().show(ui,|ui|{
                                    changed|=ui.text_edit_singleline(&mut canvas.name).changed();
                                    ui.horizontal(|ui|{
                                        ui.label("Width"); changed|=ui.add(egui::DragValue::new(&mut canvas.width).range(16..=4096).speed(2.)).changed();
                                        ui.label("Height"); changed|=ui.add(egui::DragValue::new(&mut canvas.height).range(16..=4096).speed(2.)).changed();
                                        ui.label("FPS"); changed|=ui.add(egui::DragValue::new(&mut canvas.fps).range(1..=120)).changed();
                                    });
                                });}); ui.add_space(10.);
                            }
                            ui.horizontal_wrapped(|ui|{for vertical in [false,true]{if ui.button(if vertical{"Add portrait video"}else{"Add landscape video"}).clicked(){let id=self.project.id();let original=self.project.canvases[self.canvas].scene;let mut copy=self.project.scene(original).unwrap().clone();let scene=self.project.id();copy.id=scene;self.project.scenes.push(copy);self.project.preferences.workspace.scenes.insert(id,vec![scene]);self.project.canvases.push(Canvas{id,name:format!("Video {}",self.project.canvases.len()+1),width:if vertical{1080}else{1920},height:if vertical{1920}else{1080},fps:60,scene});self.preview_canvases[usize::from(vertical)]=Some(id);changed=true;}}});
                        }
                    });
                },
                Settings::Audio=>{
                    ui.heading("Sound");ui.add_space(18.);
                    let view=self.audio.view();
                    card().show(ui,|ui|{
                        ui.set_width(ui.available_width());
                        ui.strong("Hear yourself");
                        note(ui,"Pick your headphones to hear what goes into the stream. Use headphones, not speakers, or viewers will hear an echo.");
                        let before=self.project.preferences.monitor_device.clone();
                        egui::ComboBox::from_id_salt("settings-headphones").selected_text(view.devices.iter().find(|d|d.target==before).map_or("Off",|d|d.label.as_str())).show_ui(ui,|ui|{ui.selectable_value(&mut self.project.preferences.monitor_device,String::new(),"Off");for d in view.devices.iter().filter(|d|d.kind=="Audio/Sink"){ui.selectable_value(&mut self.project.preferences.monitor_device,d.target.clone(),&d.label);}});
                        if before!=self.project.preferences.monitor_device{self.audio.monitor_device(&self.project.preferences.monitor_device);changed=true;}
                        if ui.button("Headphones & audio sync…").on_hover_text("Choose what you hear, and delay sound that arrives before the picture").clicked(){self.tools.audio=true;}
                    });
                    if self.advanced() {
                        ui.add_space(16.);
                        card().show(ui,|ui|{ui.strong("Audio tracks");note(ui,"Assign each channel to up to six independent output tracks. Click a track's name to rename it. Track 1 is what your viewers hear.");egui::Grid::new("routing").striped(true).spacing([24.,16.]).show(ui,|ui|{ui.strong("Source");for (n,name) in self.project.preferences.track_names.iter_mut().enumerate(){changed|=ui.add_sized([92.,24.],egui::TextEdit::singleline(name).hint_text(format!("Track {}",n+1)).char_limit(32)).on_hover_text("Click to rename this track").changed();}ui.end_row();for channel in &mut self.project.audio{ui.label(&channel.name);for route in &mut channel.tracks{changed|=ui.checkbox(route,"").changed();}ui.end_row();}});ui.add_space(16.);note(ui,"Game audio follows automatic capture. Choose other inputs in Sound. Live video recordings use Track 1, including volume and mute. Enable separate tracks in Advanced audio to include Tracks 2–6 in standalone recordings. Monitoring is opt-in in the mixer.");});
                    }
                },
            }
if changed{self.changed();}
        });});
    }
    /// Settings > Advanced: the switch for advanced options, then backups, profiles and imports.
    fn advanced_settings(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        ui.heading("Advanced");
        ui.add_space(12.);
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            if ui.checkbox(&mut self.project.preferences.workspace.advanced, "Show advanced options").changed() {
                changed = true;
            }
            note(ui, "Adds audio tracks and decibels, encoder and video sizes, rearrangeable panels, capture details and troubleshooting tools. Leave it off to keep Rosorah simple.");
        });
        ui.add_space(20.);
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.strong("Updates");
            changed |= ui.checkbox(&mut self.project.preferences.workspace.check_updates, "Check for updates (when Rosorah starts and every few hours)").changed();
            ui.add_enabled_ui(self.project.preferences.workspace.check_updates, |ui| {
                changed |= ui.checkbox(&mut self.project.preferences.workspace.update_popup, "Show a window when a new version is found").on_hover_text("Never while you're streaming or recording. Off: only the button at the bottom of the window").changed();
            });
            ui.horizontal_wrapped(|ui| {
                let state = self.updates.state();
                let busy = matches!(state, crate::updates::State::Checking | crate::updates::State::Downloading(..));
                if ui.add_enabled(!busy, egui::Button::new("Check now")).clicked() {
                    self.project.preferences.workspace.skipped_update.clear();
                    self.updates.check(ui.ctx());
                }
                match state {
                    crate::updates::State::Checking => note(ui, "Checking…"),
                    crate::updates::State::UpToDate => note(ui, concat!("You have the latest version (", env!("CARGO_PKG_VERSION"), ").")),
                    crate::updates::State::Available(r) | crate::updates::State::Downloaded(r, _) | crate::updates::State::InstallFailed(r, ..) => {
                        if ui.button(format!("Version {} is available…", r.version)).clicked() {
                            self.update_window = true;
                        }
                    }
                    crate::updates::State::Downloading(..) => note(ui, "Downloading the update…"),
                    crate::updates::State::Installing(..) => note(ui, "Installing the update…"),
                    crate::updates::State::Installed(r) => {
                        if ui.button(format!("Restart for {}…", r.version)).clicked() {
                            self.update_window = true;
                        }
                    }
                    crate::updates::State::Failed(e) => note(ui, e),
                    crate::updates::State::Idle => {}
                }
            });
            note(ui, format!("New versions come from github.com/{}.", crate::updates::REPO));
        });
        ui.add_space(20.);
        changed |= self.backups_settings(ui);
        ui.add_space(20.);
        self.profiles_settings(ui);
        changed
    }
    fn alerts(&mut self, ctx: &egui::Context) {
        if let Some((id,title))=self.rewards_ui.ui(ctx,&mut self.accounts[0]){self.automation_editor.guide.for_reward(&id,&title);}

        let (changed, commands) = self.automation_editor.ui(
            ctx,
            &mut self.project,
            &mut self.automation_queue,
            &self.alerts,
            &self.accounts.iter().filter(|a|a.connected).map(|a|a.platform).collect::<Vec<_>>(),
        );
        if changed {
            self.changed();
            self.donation_save_due = Some(Instant::now());
        }
        for command in commands {
            self.automation_command(command);
        }
    }
    fn stop_alert_media(&mut self) {
        self.captures.widgets.clear_alerts();
        self.active_alert = None;
        for source in &self.project.sources {
            if let SourceKind::VideoFile(v) = &source.kind
                && (v.automation_only || v.trigger.is_some())
            {
                self.captures.videos.queued.remove(&source.id);
                self.captures.videos.restart.remove(&source.id);
                self.captures.stop_alert_source(source.id);
                self.captures.videos.bus.clear(source.id);
            }
        }
    }
    fn automation_command(&mut self, command: crate::automations::Command) {
        use crate::automations::Command;
        match command {
            Command::Rewards=>{self.rewards_ui.open=true;self.rewards_ui.refresh(&mut self.accounts[0]);},
            Command::Stop => {
                self.automation_queue.clear();
                self.stop_alert_media();
                self.automation_queue.paused = true;
            }
            Command::Skip => {
                self.active_alert = None;
                for id in self.automation_queue.skip() {
                    self.captures.videos.queued.remove(&id);
                    self.captures.videos.restart.remove(&id);
                    self.captures.stop_alert_source(id);
                    self.captures.videos.bus.clear(id);
                }
            }
            Command::Clear => self.automation_queue.clear(),
            Command::NewAlertBox(rule, step) => self.new_alert_box(rule, step),
            Command::SpeechTest => self.speech.say("Thanks for the support! This is how paid messages will sound.".into()),
            Command::SpeechSkip => {
                self.speech.clear();
                if let Some(id) = self.project.preferences.speech.source {
                    self.captures.stop_alert_source(id);
                }
            }
            Command::TestWith(id, details) => {
                let Some(mut rule) = self.project.rules.iter().find(|r| r.id == id).cloned() else {
                    return;
                };
                // Testing a switched-off alert is allowed; every other condition is checked as live.
                rule.enabled = true;
                let event = Event {
                    facts: auroracast_core::automation::Facts {
                        amount: (details.amount * 100.).round() as u64,
                        currency: if rule.options.currency.is_empty() { "USD".into() } else { rule.options.currency.clone() },
                        count: details.count,
                        level: 2,
                        stage: rule.options.hype.first(),
                        ..Default::default()
                    },
                    reward_id: rule.conditions.reward_id.clone(),
                    id: "manual-test".into(),
                    platform: details.platform,
                    kind: rule.event,
                    author: details.author.clone(),
                    text: details.message.clone(),
                    at_ms: now_ms(),
                };
                if let Some(reason) = auroracast_core::check(&rule, &event) {
                    self.automation_editor.notice = format!("Wouldn't play: {reason}.");
                    return;
                }
                self.automation_editor.notice = "Conditions match; playing a preview.".into();
                self.automation_editor.preview_open = true;
                let actions = crate::automations::preview_actions(
                    auroracast_core::automation::actions(&rule.actions, &event),
                    self.live_stream.is_none() && self.live_recording.is_none(),
                );
                if self.automation_queue.mode == crate::automations::Mode::Interrupt {
                    self.stop_alert_media();
                }
                self.automation_queue.enqueue(format!("TEST · {}", rule.name), actions);
            }
            Command::Preview(source) => {
                self.automation_editor.preview_open = true;
                if self.live_stream.is_some() || self.live_recording.is_some() {
                    self.automation_editor.notice =
                        "Media previews are blocked while streaming or recording.".into();
                    return;
                }
                self.automation_queue
                    .enqueue("Media preview".into(), vec![Action::PlayMedia { source }]);
            }
            Command::Test(id) | Command::Run(id) => {
                let Some(rule) = self.project.rules.iter().find(|r| r.id == id).cloned() else {
                    return;
                };
                let test = matches!(command, Command::Test(_));
                if test {
                    self.automation_editor.preview_open = true;
                }
                let event = Event {
                    facts: auroracast_core::automation::Facts {
                        amount: 500,
                        currency: "USD".into(),
                        count: 10,
                        level: 2,
                        stage: rule.options.hype.first(),
                        ..Default::default()
                    },
                    reward_id: rule.conditions.reward_id.clone(),
                    id: "manual".into(),
                    platform: rule.platform.unwrap_or(Platform::Twitch),
                    kind: rule.event,
                    author: self.event_user.clone(),
                    text: self.event_text.clone(),
                    at_ms: now_ms(),
                };
                let mut actions = auroracast_core::automation::actions(&rule.actions, &event);
                if test {
                    actions = crate::automations::preview_actions(
                        actions,
                        self.live_stream.is_none() && self.live_recording.is_none(),
                    );
                }
                if self.automation_queue.mode == crate::automations::Mode::Interrupt {
                    self.stop_alert_media();
                }
                self.automation_queue.enqueue(
                    format!("{} · {}", if test { "TEST" } else { "MANUAL" }, rule.name),
                    actions,
                );
            }
        }
    }
    fn dispatch_automations(&mut self, event: &Event, missed: bool, replay: bool) {
        // The streamer's and the bot's own messages never trigger chat automations (no reply loops).
        if matches!(event.kind,EventKind::ChatMessage|EventKind::ChatCommand)&&self.accounts.iter().chain(std::iter::once(&self.kick_account)).chain(self.bots.iter()).any(|a|a.platform==event.platform&&a.connected&&a.name.eq_ignore_ascii_case(&event.author)){return;}
        // Timers only run on their schedule (timers_tick).
        if event.kind == EventKind::Timer {return;}

        let rules: Vec<_> = self
            .project
            .rules
            .iter()
            .filter(|r| {
                r.options
                    .scene
                    .is_none_or(|id| self.project.canvases.iter().any(|c| c.scene == id))
                    && (!missed || r.options.missed_alerts)
            })
            .cloned()
            .collect();
        let groups = if replay {
            let mut isolated = AlertEngine::default();
            isolated.dispatch_rules(&rules, event)
        } else {
            self.alerts.dispatch_rules(&rules, event)
        };
        if groups.len() > 1 {
            self.automation_queue.log(format!(
                "{} automations matched {}",
                groups.len(),
                event.kind.label()
            ));
        }
        for (id, mut actions) in groups {
            let name = rules
                .iter()
                .find(|r| r.id == id)
                .map(|r| r.name.clone())
                .unwrap_or_default();
            if replay {
                actions = crate::automations::replay_actions(actions);
            }
            if self.automation_queue.mode == crate::automations::Mode::Interrupt {
                self.stop_alert_media();
            }
            self.automation_queue.enqueue(
                format!(
                    "{}{}",
                    if replay {
                        "REPLAY · "
                    } else if missed {
                        "MISSED · "
                    } else {
                        ""
                    },
                    name
                ),
                actions,
            );
        }
    }
    fn shared_alert(&mut self,text:&str,seconds:u32,media:Option<Id>){
        if let Some(id)=self.project.sources.iter().find(|s|matches!(&s.kind,SourceKind::Widget(w) if w.kind==widgets::Kind::AlertBox)).map(|s|s.id){
            self.captures.widgets.target_alert(id,text,seconds,media,&Default::default());
        }else{
            self.automation_queue.log("Visual alert skipped: add Alerts from In this scene > + Add".into());
        }
    }
    /// Runs timers that are due: every N minutes (while live, unless set otherwise), once enough chat came in.
    fn timers_tick(&mut self, live: bool) {
        let now = Instant::now();
        let seen = self.chat_messages_seen;
        let mut due = Vec::new();
        for r in self.project.rules.iter().filter(|r| r.event == EventKind::Timer) {
            let last = self.timer_runs.entry(r.id).or_insert((now, seen));
            // Off, or waiting for the stream: the first run comes a full interval after it starts.
            if !r.enabled || (!live && !r.options.timer_offline) {
                *last = (now, seen);
                continue;
            }
            let interval = Duration::from_secs(u64::from(r.options.timer_minutes.max(1)) * 60);
            if now.duration_since(last.0) >= interval && seen.saturating_sub(last.1) >= u64::from(r.options.timer_messages) {
                *last = (now, seen);
                due.push(r.clone());
            }
        }
        let rules: std::collections::HashSet<Id> = self.project.rules.iter().map(|r| r.id).collect();
        self.timer_runs.retain(|id, _| rules.contains(id));
        for rule in due {
            self.run_timer(&rule);
        }
    }
    fn run_timer(&mut self, rule: &auroracast_core::Rule) {
        // Timer chat with no platform ticked goes to every connected chat.
        let connected: Vec<Platform> = [(Platform::Twitch, self.accounts[0].connected), (Platform::YouTube, self.accounts[1].connected), (Platform::Kick, self.kick_account.connected)]
            .into_iter()
            .filter(|(_, on)| *on)
            .map(|(p, _)| p)
            .collect();
        let event = Event {
            facts: Default::default(),
            reward_id: String::new(),
            id: format!("timer-{}", rule.id),
            platform: connected.first().copied().unwrap_or(Platform::Twitch),
            kind: EventKind::Timer,
            author: String::new(),
            text: String::new(),
            at_ms: now_ms(),
        };
        let actions: Vec<Action> = rule
            .actions
            .iter()
            .cloned()
            .map(|a| match a {
                Action::SendChat { text, platforms, sender } if platforms.is_empty() => Action::SendChat { text, platforms: connected.clone(), sender },
                a => a,
            })
            .collect();
        let actions = auroracast_core::automation::actions(&actions, &event);
        self.automation_queue.enqueue(format!("TIMER · {}", rule.name), actions);
    }
    /// The sound reading aloud plays through, with a "Read aloud" mixer strip; made when first needed.
    fn speech_source(&mut self) -> Id {
        if let Some(id) = self.project.preferences.speech.source.filter(|id| self.project.source(*id).is_some()) {
            return id;
        }
        let id = self.project.id();
        self.project.sources.push(auroracast_core::Source {
            id,
            name: "Read aloud".into(),
            kind: SourceKind::VideoFile(auroracast_core::VideoSettings {
                path: crate::speech::output().display().to_string(),
                audio: true,
                looped: false,
                automation_only: true,
                ..Default::default()
            }),
        });
        let channel = self.project.id();
        self.project.audio.push(auroracast_core::AudioChannel {
            id: channel,
            name: "Read aloud".into(),
            gain_db: 0.,
            muted: false,
            tracks: [true, false, false, false, false, false],
            delay_ms: 0,
            monitor: true,
            monitor_only: false,
            noise_suppression: Default::default(),
            media_source: Some(id),
        });
        // Heard by the streamer too, like alert sounds.
        if self.project.preferences.monitor_device.is_empty() {
            self.project.preferences.monitor_device = crate::audio::DEFAULT_MONITOR.into();
        }
        self.project.preferences.speech.source = Some(id);
        self.changed();
        id
    }
    fn run_actions(&mut self, actions: Vec<Action>) {
        for action in actions {
            match &action {
                Action::DisplayAlert{source,template,media,seconds,subject}=>{self.captures.widgets.target_alert(*source,template,*seconds,*media,subject);},
                Action::PlaySound{source}=>{self.captures.videos.trigger(*source);},
                Action::SendChat{text,platforms,sender}=>{
                    for platform in platforms {
                        let request=format!("automation-{}-{}",now_ms(),platform.name());
                        // The bot posts into the streamer's channel; without a bot, the streamer posts.
                        let channel = match platform {
                            Platform::Twitch => self.accounts[0].user_id.clone(),
                            Platform::YouTube => self.accounts[1].user_id.clone(),
                            Platform::Kick => self.kick_account.user_id.clone(),
                            _ => continue,
                        };
                        let bot = match platform {
                            Platform::Twitch => Some(&mut self.bots[0]),
                            Platform::Kick => Some(&mut self.bots[1]),
                            _ => None,
                        }.filter(|b| *sender == auroracast_core::ChatSender::Bot && b.connected && !channel.is_empty() && b.user_id != channel);
                        let sent = match bot {
                            Some(bot) => bot.post_to(request, text.clone(), &channel),
                            None => match platform {
                                Platform::Twitch => self.accounts[0].post(request, text.clone()),
                                Platform::YouTube => self.accounts[1].post(request, text.clone()),
                                _ => self.kick_account.post(request, text.clone()),
                            },
                        };
                        if !sent {self.automation_queue.log(format!("{} chat not sent: account unavailable",platform.name()));}
                    }
                },
                Action::Wait { .. } => (),
                Action::UpdateWidget {
                    source,
                    text,
                    increment,
                    start_timer,
                } => {
                    self.captures
                        .widgets
                        .update(*source, text, *increment, *start_timer);
                }
                Action::PlayMedia { source } => {
                    if let Some(s) = self.project.source(*source)
                        && let SourceKind::VideoFile(v) = &s.kind
                        && (v.automation_only || v.trigger.is_some())
                        && std::path::Path::new(&v.path).is_file()
                    {
                        self.shared_alert("", 10, Some(*source));
                        self.captures.videos.trigger(*source);
                    } else {
                        self.automation_queue
                            .log(format!("Media {source}: skipped · missing source or file"));
                    }
                }
                Action::ShowAlert { template, seconds } => {
                    self.shared_alert(template, *seconds, None);
                    self.log(format!("Preview alert: {template}"));
                }
                Action::SwitchScene { canvas, scene } => {
                    // Automations go live directly, Studio Mode or not.
                    self.studio_taking = true;
                    self.request_scene(*canvas, *scene);
                    self.studio_taking = false;
                    self.log("Scene switched".into());
                }
            }
        }
    }
}
fn format_time(ms: u64, twenty_four: bool) -> String {
    i64::try_from(ms / 1000)
        .ok()
        .and_then(|seconds| gst::glib::DateTime::from_unix_local(seconds).ok())
        .and_then(|time| {
            time.format(if twenty_four { "%H:%M" } else { "%I:%M %p" })
                .ok()
        })
        .map(|text| text.to_string())
        .unwrap_or_else(|| "—".into())
}

#[cfg(test)]
mod local_chat_time_tests {
    use super::*;
    #[test]
    fn timestamps_use_system_timezone_and_daylight_saving() {
        const CHILD: &str = "ROSORAH_TIMEZONE_TEST";
        if std::env::var(CHILD).as_deref() != Ok("1") {
            let result=std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact","studio::local_chat_time_tests::timestamps_use_system_timezone_and_daylight_saving"])
                .env(CHILD,"1").env("TZ","America/New_York").output().unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stdout)
            );
            return;
        }
        let winter = gst::glib::DateTime::from_iso8601("2026-01-15T12:34:00Z", None)
            .unwrap()
            .to_unix() as u64
            * 1000;
        let summer = gst::glib::DateTime::from_iso8601("2026-07-15T12:34:00Z", None)
            .unwrap()
            .to_unix() as u64
            * 1000;
        assert_eq!(format_time(winter, true), "07:34");
        assert_eq!(format_time(summer, true), "08:34");
        assert_eq!(format_time(summer, false), "08:34 AM");
    }
}

impl eframe::App for Studio {
    fn raw_input_hook(&mut self, _: &egui::Context, input: &mut egui::RawInput) {
        if let Some(what) = self.demo_input {
            let key = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
            // Bottom left, under the Scenes list: away from both canvases and from Layers.
            let away = input.screen_rect.map_or(egui::pos2(20., 880.), |r| egui::pos2(r.left() + 200., r.bottom() - 70.));
            let button = |pressed| egui::Event::PointerButton { pos: away, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            match (what, self.ui_frames) {
                ("press-delete", 12) => input.events.push(key(egui::Key::Delete)),
                ("press-f2", 12) => input.events.push(key(egui::Key::F2)),
                ("copy-scene", 12) => input.events.push(egui::Event::Copy),
                ("copy-scene", 14) => input.events.push(egui::Event::Paste(String::new())),
                ("click-away", 11) => input.events.push(egui::Event::PointerMoved(away)),
                ("click-away", 12) => input.events.push(button(true)),
                ("click-away", 13) => input.events.push(button(false)),
                _ => {}
            }
        }
        // `--page demo-drag`: press on the Sources tab, then carry it most of the way to the middle and hold.
        let Some(Some(tab)) = self.demo_drag else { return };
        let start = tab.center();
        let toward = input.screen_rect.map_or(egui::vec2(-300., 150.), |r| (r.center() - start) * 0.8);
        let pos = match self.ui_frames {
            10 => start,
            11 => {
                input.events.push(egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
                return;
            }
            f @ 12..=24 => start + toward * ((f - 11) as f32 / 13.),
            _ => return,
        };
        input.events.push(egui::Event::PointerMoved(pos));
    }
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.ui_frames += 1;
        self.selection_input(ctx);
        if ctx.input_mut(|i| i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::S))) {
            self.save();
        }
        if self.project.preferences.backups.hourly
            && self.backup_due
            && self.recovery_candidate.is_none()
            && self.last_backup.elapsed() >= Duration::from_secs(3600)
        {
            if let Err(e) = self.back_up("hourly") {
                self.message = e;
            }
            // A failed attempt waits another hour rather than retrying every frame.
            self.last_backup = Instant::now();
        }
        if let Some(rx) = &self.theme_import
            && let Ok(path) = rx.try_recv()
        {
            self.theme_import = None;
            if let Some(path) = path {
                self.import_theme(&path);
            }
        }
        if let Some(rx) = &self.appearance_picker
            && let Ok(path) = rx.try_recv()
        {
            if let Some(path) = path {
                let look = &mut self.project.preferences.workspace.appearance;
                look.backdrop_image = path.to_string_lossy().into();
                look.backdrop = auroracast_core::workspace::Backdrop::Picture;
                self.appearance.error.clear();
                self.dirty = true;
            }
            self.appearance_picker = None;
        }
        self.appearance.frame(ctx, &self.project.preferences.workspace.appearance);
        if self.ui_frames == 1 && !self.advanced() {
            self.sync_vertical_panel();
        }
        // Remember chosen inputs (the mixer edits them directly) without adding undo steps.
        let inputs: std::collections::BTreeMap<Id, String> = self
            .audio
            .selected
            .iter()
            .filter(|(id, input)| !input.is_empty() && self.audio.enabled.get(id) == Some(&true))
            .map(|(id, input)| (*id, input.clone()))
            .collect();
        if inputs != self.project.preferences.audio_inputs {
            self.project.preferences.audio_inputs = inputs;
            self.dirty = true;
        }
        self.browser_chat.runtime_wanted = self.project.sources.iter().any(|s| matches!(s.kind, SourceKind::Browser { .. }));
        if let Some(rx) = &self.recovering
            && let Ok(results) = rx.try_recv()
        {
            self.recovering = None;
            let saved: Vec<_> = results.iter().filter_map(|r| r.as_ref().ok()).collect();
            if let Some(path) = saved.last() {
                self.last_recording = Some((*path).clone());
                self.tell(if saved.len() == 1 {
                    format!("We recovered your last recording: {}", path.display())
                } else {
                    format!("We recovered {} unfinished recordings in {}", saved.len(), self.record_folder)
                });
            } else if let Some(Err(e)) = results.first() {
                self.message = format!("An unfinished recording couldn't be recovered: {e}");
            }
        }
        self.tick_transition(ctx);
        let minimized = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
        let preview_active = !minimized && (self.automation_editor.preview_open ||
            (self.page == Page::Studio && (self.workspace.contains(Panel::Preview) || self.workspace.contains(Panel::Vertical))));
        self.captures.set_preview_active(preview_active);
        self.continuous_edit = ctx.input(|i| i.pointer.is_decidedly_dragging());
        self.project_tools(ctx);
        self.layer_tools_windows(ctx);
        self.delete_confirm_ui(ctx);
        self.utility_windows(ctx);
        if !ctx.wants_keyboard_input() {
            let keys = [
                egui::Key::F1,
                egui::Key::F2,
                egui::Key::F3,
                egui::Key::F4,
                egui::Key::F5,
                egui::Key::F6,
                egui::Key::F7,
                egui::Key::F8,
                egui::Key::F9,
                egui::Key::F10,
                egui::Key::F11,
                egui::Key::F12,
            ];
            let hotkeys: Vec<_> = self
                .project
                .rules
                .iter()
                .filter(|r| {
                    r.enabled
                        && r.options.hotkey > 0
                        && r.options.hotkey <= 12
                        && ctx.input(|i| {
                            i.modifiers.ctrl
                                && i.modifiers.alt
                                && i.key_pressed(keys[r.options.hotkey as usize - 1])
                        })
                })
                .map(|r| r.id)
                .collect();
            for id in hotkeys {
                self.automation_command(crate::automations::Command::Run(id));
            }
        }
        let scheduled = self.automation_queue.tick_media(&self.project, |id| {
            (
                self.captures.running(id)
                    || self.captures.videos.queued.get(&id).is_some_and(|n| *n > 0),
                self.captures.video_failure(id),
            )
        });
        self.run_actions(scheduled);
        if let Some(item) = self.event_history.replay.take() {
            let event = Event {
                facts: item.facts,
                reward_id: item.reward_id,
                id: format!("replay-{}", now_ms()),
                platform: item.platform,
                kind: item.kind,
                author: item.author,
                text: item.text,
                at_ms: now_ms(),
            };
            self.dispatch_automations(&event, false, true);
        }

        if self
            .donation_save_due
            .is_some_and(|at| at.elapsed() > Duration::from_millis(600))
        {
            self.donation_save_due = None;
            self.save();
        }
        if let Some((id, rx)) = &self.slideshow_folder
            && let Ok(path) = rx.try_recv()
        {
            let id = *id;
            self.slideshow_folder = None;
            if let Some(path) = path
                && let Some(Source { kind: SourceKind::Slideshow(show), .. }) = self.project.sources.iter_mut().find(|s| s.id == id)
            {
                show.folder = path.display().to_string();
                self.changed();
            }
        }
        if let Some((id, rx)) = &self.browser_picker
            && let Ok(path) = rx.try_recv()
        {
            let id = *id;
            self.browser_picker = None;
            if let Some(address) = path.and_then(|p| crate::browser_source::address(&p.display().to_string()))
                && let Some(draft) = self.browser_drafts.get_mut(&id)
            {
                draft.0 = address;
            }
        }
        if let Some((scene, index, lut, rx)) = &self.look_picker
            && let Ok(path) = rx.try_recv()
        {
            let (scene, index, lut) = (*scene, *index, *lut);
            self.look_picker = None;
            if let Some(path) = path
                && let Some(layer) = self.project.scenes.iter_mut().find(|s| s.id == scene).and_then(|s| s.layers.get_mut(index))
            {
                let path = path.display().to_string();
                if lut { layer.look.lut = path } else { layer.look.mask = path }
                self.changed();
            }
        }
        if let Some((scene, kind, rx)) = &self.media_picker
            && let Ok(path) = rx.try_recv()
        {
            let (scene, kind) = (*scene, *kind);
            self.media_picker = None;
            if let Some(path) = path
                && let Some(si) = self.project.scenes.iter().position(|s| s.id == scene)
            {
                let folder = path.display().to_string();
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Folder".into());
                match kind {
                    MediaPick::File => self.add_media_file(scene, path),
                    MediaPick::Slideshow => {
                        let count = auroracast_core::folder_files(&folder, &auroracast_core::PICTURE_EXTENSIONS).len();
                        self.add_source_layer(si, &name, SourceKind::Slideshow(auroracast_core::Slideshow { folder, ..Default::default() }), [0.1, 0.1, 0.5, 0.5]);
                        self.tell(format!("Slideshow of {count} pictures added"));
                    }
                    MediaPick::Playlist => {
                        let files = auroracast_core::folder_files(&folder, &auroracast_core::VIDEO_EXTENSIONS);
                        match files.first() {
                            Some(first) => {
                                let settings = auroracast_core::VideoSettings { path: first.clone(), folder, ..Default::default() };
                                self.add_source_layer(si, &name, SourceKind::VideoFile(settings), [0., 0., 1., 1.]);
                                self.tell(format!("Playlist of {} videos added", files.len()));
                            }
                            None => self.tell("That folder has no videos (MP4, MKV, WebM, MOV…)".into()),
                        }
                    }
                }
            }
        }

        self.captures.sync_widgets(&self.project);
        // Viewer counts are only checked while Rosorah is streaming.
        let live = self.live_stream.is_some();
        for account in &mut self.accounts {
            account.tick(&mut self.chat);
            account.set_live(live);
        }
        self.kick_account.tick(&mut self.chat);
        self.kick_account.set_live(live);
        for bot in &mut self.bots {
            bot.tick(&mut self.chat);
            bot.forgotten = false;
        }
        self.automation_editor.bots = self.bots.iter().filter(|b| b.connected).map(|b| b.platform).collect();
        self.timers_tick(live);
        let mut kick_updated = false;
        if std::mem::take(&mut self.kick_account.forgotten) {
            for service in &mut self.stream_config.services {
                if service.provider == crate::streaming::Provider::Kick && service.destination.managed_by_account {
                    service.destination.key.clear();
                    service.destination.managed_by_account = false;
                    kick_updated = true;
                }
            }
        }
        if self.live_stream.is_none() && let Some((server,key)) = self.kick_account.ingest.take() {
            if !self.stream_config.services.iter().any(|s|s.provider == crate::streaming::Provider::Kick) {
                self.stream_config.add_service(crate::streaming::Provider::Kick);
            }
            for service in &mut self.stream_config.services {
                if service.provider == crate::streaming::Provider::Kick && (service.destination.managed_by_account || service.destination.key.is_empty()) {
                    service.destination.server = server.clone();
                    service.destination.key = key.clone();
                    service.destination.managed_by_account = true;
                    kick_updated = true;
                }
            }
        }
        if kick_updated {self.save_stream_settings();}
        self.browser_chat
            .sync_accounts(&self.accounts[0], &self.accounts[1], &self.kick_account);
        self.browser_chat.set_tiktok(&self.stream_config.tiktok_username);
        self.browser_chat.set_costream(
            &self.stream_config.costream_channels,
            &self.accounts[0].shared_chat,
            self.stream_config.costream_invites && self.accounts[0].connected,
        );
        self.costream_notices();
        self.browser_chat.set_context(ctx);
        self.browser_chat.tick(&mut self.chat);
        let mut incoming = std::mem::take(&mut self.browser_chat.activity);
        for account in self.accounts.iter_mut().chain(std::iter::once(&mut self.kick_account)) {
            incoming.append(&mut account.activity);
        }
        let mut donations = self.donations.drain();
        donations.extend(self.kofi.drain());
        incoming.extend(
            donations
                .iter()
                .map(|d| d.activity(&self.project.preferences.donations)),
        );
        for item in incoming {
            // Follows, subs, donations… for remote controllers and scripts.
            if self.remote.is_some() || !self.scripts.is_empty() {
                let name = format!("{:?}", item.kind).to_lowercase();
                self.pending_events.push(serde_json::json!({"event": name, "platform": item.platform.name(), "author": item.author, "text": item.text, "count": item.facts.count}));
            }
            let kind = item.kind;
            let platform = item.platform;
            let event = Event {
                facts: item.facts.clone(),
                id: format!("{}:{}", item.owner, item.id),
                platform,
                kind,
                author: item.author.clone(),
                text: item.text.clone(),
                at_ms: item.at_ms,
                reward_id: item.reward_id.clone(),
            };
            let goal_count = item.goal_count;
            let donation = donations.iter().find(|d| {
                platform == Platform::Donations && d.owner == item.owner && d.id == item.id
            });
            let historical = item.historical;
            if self.event_history.insert_new(item) {
                if !historical {
                    self.captures.widgets.goal_event(&event);
                    self.captures
                        .widgets
                        .activity(platform, &event.author, &event.text);
                    if self.project.preferences.speech.enabled
                        && matches!(kind, EventKind::SuperChat | EventKind::Donation | EventKind::Bits)
                        && let Some(line) = crate::speech::spoken(&event.author, &event.text, &self.project.preferences.speech)
                    {
                        self.speech.say(line);
                    }
                    // Super Chats, donations and Kicks show in chat widgets too, set apart. (Twitch
                    // cheers already arrive as chat messages.)
                    if matches!(kind, EventKind::SuperChat | EventKind::SuperSticker | EventKind::Donation)
                        || (kind == EventKind::Bits && platform == Platform::Kick)
                    {
                        let look = crate::widgets::ChatLook { highlight: "paid", ..Default::default() };
                        self.captures.widgets.chat(platform, &event.author, &event.text, &look);
                    }
                    if !self.automation_queue.paused {
                        self.captures
                            .widgets
                            .alert(platform, kind, &event.author, &event.text);
                    }
                }
                if let Some(donation) = donation
                    && (!historical || self.project.preferences.automation_history.count_missed)
                {
                    self.captures.widgets.donation_at(
                        &donation.currency,
                        donation.hundredths,
                        event.at_ms,
                        historical,
                    );
                    self.donation_save_due = Some(Instant::now());
                }
                if historical {
                    self.dispatch_automations(&event, true, false);
                    continue;
                }
                if goal_count > 0 {
                    self.captures.widgets.subscription(platform, goal_count);
                }
                self.dispatch_automations(&event, false, false);
                for source in &self.project.sources {
                    if let SourceKind::VideoFile(v) = &source.kind
                        && v.enabled
                        && !v.automation_only
                        && !self.automation_queue.paused
                        && v.trigger == Some(kind)
                        && v.platform.is_none_or(|p| p == platform)
                        && self
                            .project
                            .canvases
                            .iter()
                            .filter_map(|c| self.project.scene(c.scene))
                            .any(|scene| {
                                scene
                                    .layers
                                    .iter()
                                    .any(|l| l.source == source.id && l.visible)
                            })
                    {
                        self.captures.videos.trigger(source.id);
                    }
                }
            }
        }
        self.kofi.acknowledge();
        for (platform, owner, id) in self.browser_chat.activity_deleted.drain(..) {
            self.event_history.redact(platform, &owner, &id);
        }
        self.event_history.flush(false);
        for request in self.browser_chat.outgoing.drain(..).collect::<Vec<_>>() {
            let id = request["request"].as_str().unwrap_or("").to_string();
            let text = request["text"].as_str().unwrap_or("").to_string();
            let posted = match request["platform"].as_str() {
                Some("TikTok") => {
                    // No TikTok chat API: the chat browser types into the LIVE page when opted in.
                    let allowed = self.stream_config.tiktok_send;
                    if allowed {
                        self.browser_chat.send(serde_json::json!({"type":"type_message","text":text,"request":id}));
                    }
                    allowed
                }
                Some("Kick") => self.kick_account.post(id.clone(), text),
                Some("YouTube") => self.accounts[1].post(id.clone(), text),
                _ => self.accounts[0].post(id.clone(), text),
            };
            if !posted {
                self.browser_chat.finish_post(serde_json::json!({"type":"post_result","platform":request["platform"],"request":id,"ok":false,"text":"Connect this account in Settings > Accounts. Your draft was kept."}));
            }
        }
        for account in self.accounts.iter_mut().chain(std::iter::once(&mut self.kick_account)).chain(self.bots.iter_mut()) {
            if let Some(result) = account.moderation_results.drain(..).next_back() {
                self.browser_chat.post_status = result;
            }
            if let Some(notice) = account.notices.drain(..).next_back() {
                self.toast = Some(Toast { text: notice, undo: false, until: Instant::now() + Duration::from_secs(8) });
            }
            for result in account.post_results.drain(..) {
                if result["request"].as_str().is_some_and(|s|s.starts_with("automation-")){
                    self.automation_queue.log(format!("{}{} automation chat: {}",account.platform.name(),if account.bot{" bot"}else{""},if result["ok"].as_bool()==Some(true){"sent"}else{result["text"].as_str().unwrap_or("failed")}));
                }else{self.browser_chat.finish_post(result);}
            }
        }
        if self.browser_chat.open_accounts {
            self.browser_chat.open_accounts = false;
            self.page = Page::Settings;
            self.settings = Settings::Accounts;
        }
        {
            let mut updated = false;
            for (index, account) in self.accounts.iter_mut().enumerate() {
                if std::mem::take(&mut account.forgotten) {
                    let dest = if index == 0 {
                        &mut self.stream_config.twitch
                    } else {
                        &mut self.stream_config.youtube
                    };
                    if dest.managed_by_account {
                        dest.key.clear();
                        dest.managed_by_account = false;
                        updated = true;
                    }
                }
            }
            if updated {
                self.save_stream_settings();
            }
        }
        if self.live_stream.is_none() {
            let mut updated = false;
            for (index, account) in self.accounts.iter_mut().enumerate() {
                if let Some((server, key)) = account.ingest.take() {
                    let dest = if index == 0 {
                        &mut self.stream_config.twitch
                    } else {
                        &mut self.stream_config.youtube
                    };
                    if dest.key != key || dest.server != server || !dest.managed_by_account {
                        dest.managed_by_account = true;
                        dest.key = key;
                        dest.server = server;
                        updated = true;
                    }
                }
            }
            if updated {
                self.save_stream_settings();
            }
        }

        let streaming = self.live_stream.is_some();
        if streaming && !self.was_streaming {
            self.chat_session.start(now_ms());
            self.captures.widgets.start_stream();
        }
        self.was_streaming = streaming;
        // Each new message once: profile pictures, chat widgets, automations, and Twitch
        // activity relayed to the browser chat window.
        let fresh: Vec<ChatMessage> = self.chat.take_fresh().into_iter().cloned().collect();
        let mut events = Vec::new();
        for m in &fresh {
            // Twitch account ids let the chat widget find chatters' profile pictures.
            if !m.metadata.user_id.is_empty() {
                crate::avatars::note(m.platform.name(), &m.author, &m.metadata.user_id);
            }
            if m.id.starts_with("event-") {
                self.browser_chat.relay(m);
            }
            events.extend(self.chat_session.messages(m).into_iter().map(|e| (e, m)));
        }
        for (event, m) in events {
            if event.kind == EventKind::ChatMessage {
                self.chat_messages_seen += 1;
                // Emotes from the browser chat reach chat widgets as pictures, not just names.
                let key = format!("{}:{}", event.platform.name(), event.id);
                let text = match self.browser_chat.rich.parts(&key) {
                    Some(parts) if parts.iter().any(|p| p.src.is_some()) => crate::chat_render::marked(parts, 240),
                    _ => event.text.clone(),
                };
                let metadata = &m.metadata;
                let look = crate::widgets::ChatLook { color: &metadata.color, badges: &metadata.badges, highlight: &metadata.highlight };
                self.captures.widgets.chat(event.platform, &event.author, &text, &look);
                // A co-streamer's chat shows, but doesn't run your commands and automations.
                if !metadata.channel.is_empty() {
                    continue;
                }
            }
            self.dispatch_automations(&event, false, false);
        }
        self.audio
            .monitor_device(&self.project.preferences.monitor_device);
        if self.browser_chat.running() || self.browser_chat.installing() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if let Some(rx) = &self.thumbnail_picker
            && let Ok(path) = rx.try_recv()
        {
            if let Some(p) = path {
                self.accounts[1].draft.thumbnail = p.display().to_string();
            }
            self.thumbnail_picker = None;
        }
        let thumb = &self.accounts[1].draft.thumbnail;
        if thumb != &self.thumbnail_preview_path {
            self.thumbnail_preview_path = thumb.clone();
            self.thumbnail_preview = None;
            if !thumb.is_empty() {
                match crate::accounts::thumbnail_preview(thumb) {
                    Ok(img) => {
                        let img = img.thumbnail(480, 270).to_rgba8();
                        self.thumbnail_preview = Some(ctx.load_texture(
                            "broadcast_thumbnail",
                            egui::ColorImage::from_rgba_unmultiplied(
                                [img.width() as usize, img.height() as usize],
                                &img,
                            ),
                            Default::default(),
                        ));
                    }
                    Err(e) => self.message = format!("Thumbnail: {e}"),
                }
            }
        }
        if self.accounts.iter().any(|a| a.connected || a.busy) {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if ctx.input(|i| i.viewport().close_requested())
            && (self.live_stream.is_some() || self.live_recording.is_some())
        {
            if let Some(stream) = &self.live_stream {
                stream.stop();
            }
            if let Some(recording) = &self.live_recording {
                recording.stop();
            }
            self.closing = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        self.streaming_tick(ctx);
        self.recording_tick(ctx);
        if self.closing && self.live_stream.is_none() && self.live_recording.is_none() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let canvas = &self.project.canvases[0];
        self.captures.browser_state(serde_json::json!({
            "streaming": self.live_stream.is_some(),
            "recording": self.live_recording.is_some(),
            "replaybuffer": self.replay_on,
            "virtualcam": self.virtual_camera.is_some(),
            "scene": self.project.scene(canvas.scene).map(|s| s.name.clone()).unwrap_or_default(),
            "width": canvas.width,
            "height": canvas.height,
        }));
        self.captures.sync_browsers(&self.project);
        self.captures.sync_videos(&self.project);
        if self.project.preferences.speech.enabled {
            let id = self.speech_source();
            if self.speech.tick(&self.project.preferences.speech, self.captures.media_playing(id)) {
                self.captures.videos.trigger(id);
            }
        }
        let waiting = self.speech.waiting();
        self.automation_editor.speech_status = match (&self.speech.status, waiting) {
            (status, 0) => status.clone(),
            (status, n) if status.is_empty() => format!("{n} waiting to be read"),
            (status, n) => format!("{status} · {n} waiting"),
        };
        self.audio.media_bus(self.captures.videos.bus.clone());
        let scene = self.project.canvases[self.canvas].scene;
        if self.audio_scene != Some(scene) {
            self.audio.scene(&self.project, scene);
            self.audio_scene = Some(scene);
        }
        self.captures.sync_cameras(&self.project);
        self.captures.sync_shared(&self.project);
        self.captures.tick();
        let mut resized = false;
        for source in &self.project.sources {
            if matches!(source.kind, SourceKind::VideoFile(_) | SourceKind::Ndi(_) | SourceKind::Spout(_))
                && let Some(size) = self.captures.camera_dimensions(source.id)
                && self.video_aspects.get(&source.id) != Some(&size)
            {
                self.video_aspects.insert(source.id, size);
                for scene in &mut self.project.scenes {
                    if let Some(canvas) = self.project.canvases.iter().find(|c| c.scene == scene.id)
                    {
                        for layer in scene
                            .layers
                            .iter_mut()
                            .filter(|l| l.source == source.id && !l.locked)
                        {
                            let ratio = size[0] as f32 / size[1] as f32 * canvas.height as f32
                                / canvas.width as f32;
                            let w = layer.rect[2].min(layer.rect[3] * ratio);
                            let h = w / ratio;
                            layer.rect[0] += (layer.rect[2] - w) / 2.;
                            layer.rect[1] += (layer.rect[3] - h) / 2.;
                            layer.rect[2] = w;
                            layer.rect[3] = h;
                            resized = true;
                        }
                    }
                }
            }
        }
        if resized {
            self.changed();
        }

        let camera_frames = self
            .project
            .sources
            .iter()
            .filter_map(|source| {
                if matches!(source.kind, SourceKind::VideoCamera(_)) {
                    self.captures
                        .camera_dimensions(source.id)
                        .map(|size| (source.id, size))
                } else {
                    None
                }
            })
            .collect();
        if self.camera_aspect.update(&mut self.project, &camera_frames) {
            self.changed();
        }
        self.audio.music_apps(&self.project.preferences.music_apps);
        self.audio.filters(&self.project.preferences.audio_filters);
        self.audio.sync(
            &self.project.audio,
            self.project.preferences.auto_game_audio,
            self.captures.game(),
        );
        if self.audio.active() && !minimized {
            ctx.request_repaint_after(Duration::from_millis(33));
        }
        if preview_active && let Some(gpu) = &mut self.gpu {
            match self.captures.upload(gpu) {
                Ok(true) => {
                    self.preview_size = [0, 0];
                    self.vertical_size = [0, 0];
                }
                Ok(false) => (),
                Err(e) => self.gpu_error = Some(format!("Capture upload failed: {e}")),
            }
        }
        // A slideshow on screen is re-checked twice a second so its pictures change on time.
        if preview_active
            && self.last_slideshow_check.elapsed() >= Duration::from_millis(500)
            && self.project.sources.iter().any(|s| matches!(s.kind, SourceKind::Slideshow(_)))
        {
            self.last_slideshow_check = Instant::now();
            self.revision = true;
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        if preview_active && self.gpu.as_ref().is_some_and(|g| g.animating()) {
            self.preview_size = [0, 0];
            self.vertical_size = [0, 0];
            ctx.request_repaint_after(Duration::from_millis(16));
        }
        let accent = self.accent();
        let ink = crate::appearance::current().on_accent.unwrap_or(Color32::from_rgb(56, 30, 114));
        // Only when it changes: setting the style every frame makes egui copy it every frame.
        if ctx.style().visuals.selection.bg_fill != accent || ctx.style().visuals.selection.stroke.color != ink {
            ctx.style_mut(|s| {
                s.visuals.selection.bg_fill = accent;
                s.visuals.selection.stroke.color = ink;
            });
        }
        if let Some(rx) = &self.recorder {
            match rx.try_recv() {
                Ok(result) => {
                    self.recorder = None;
                    self.message = match result {
                        Ok(path) => format!("Test recordings saved: {}", path.display()),
                        Err(error) => format!(
                            "Recording failed: {}",
                            error.chars().take(160).collect::<String>()
                        ),
                    };
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.recorder = None;
                    self.message = "Recording worker disconnected".into();
                }
                Err(mpsc::TryRecvError::Empty) => (),
            }
        }
        if self
            .active_alert
            .as_ref()
            .is_some_and(|(_, until)| *until <= Instant::now())
        {
            self.active_alert = None;
        }
        self.broadcast_windows(ctx);
        let custom = crate::title_bar::wanted(self.project.preferences.workspace.appearance.title_bar);
        if custom != self.custom_title_bar && self.screenshot.is_none() {
            self.custom_title_bar = custom;
            ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(!custom));
        }
        self.header(ctx);
        self.footer(ctx);
        match self.page {
            Page::Studio => self.main_studio(ctx),
            Page::Settings => self.settings(ctx),
            Page::Alerts => self.alerts(ctx),
        }
        self.panel_windows(ctx);
        self.projector_windows(ctx);
        self.hotkeys_tick(ctx);
        self.replay_tick();
        self.remote_tick();
        self.loading_screen(ctx);
        if self.automation_editor.preview_open {
            let mut open = true;
            let mut vertical = self.automation_editor.preview_vertical;
            crate::automation_preview::show(ctx, &mut open, &mut vertical, |ui, vertical| {
                self.canvas_panel(ui, vertical);
            });
            self.automation_editor.preview_vertical = vertical;
            self.automation_editor.preview_open = open;
        }

        self.widget_editor_window(ctx);
        self.readiness_window(ctx);
        self.go_live_window(ctx);
        self.report_window(ctx);
        self.toast_ui(ctx);
        if self.custom_title_bar {
            crate::title_bar::resize_edges(ctx);
        }
        if let Some(alerts) = self.demo {
            if self.ui_frames == 2 {
                crate::demo::pictures();
                for m in crate::demo::messages() {
                    self.chat.push(m);
                }
                if self.demo_input == Some("costream") {
                    // Shared chat with a co-streamer, one of their viewers' messages, and an invite.
                    self.accounts[0].shared_chat = vec![("novaraider".into(), "NovaRaider".into())];
                    let mut m = crate::demo::messages().remove(0);
                    (m.id, m.author, m.text, m.metadata.channel) = ("demo-shared".into(), "pixelpup".into(), "hi from Nova's chat!".into(), "NovaRaider".into());
                    m.received_ms = crate::activity::now();
                    self.chat.push(m);
                    self.browser_chat.invite = Some(crate::browser_chat::Invite { text: "AliceStreams wants to collaborate.".into(), can_decline: true, close_only: true, link: String::new(), gone: None });
                }
                for item in crate::demo::activity() {
                    self.event_history.insert_new(item);
                }
            }
            if self.ui_frames == 12 {
                crate::demo::widgets(&self.captures.widgets, alerts);
                if self.demo_add {
                    let id = self.project.id();
                    self.project.scenes.push(Scene { id, name: "Just chatting".into(), layers: vec![] });
                    self.add_dialog = Some((id, AddView::Main));
                }
            }
        }
        if self.screenshot.is_some() && !self.screenshot_requested && self.ui_frames >= 20 && self.started.elapsed() > Duration::from_secs(4) && !self.appearance.busy() {
            self.screenshot_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        let screenshot_image = ctx.input(|i| {
            i.events.iter().find_map(|e| {
                if let egui::Event::Screenshot { image, .. } = e {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        if let (Some(path), Some(image)) = (&self.screenshot, screenshot_image) {
            let rgba: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
            match image::save_buffer(
                path,
                &rgba,
                image.size[0] as u32,
                image.size[1] as u32,
                image::ColorType::Rgba8,
            ) {
                Ok(()) => println!("Saved UI screenshot {}", path.display()),
                Err(e) => eprintln!("Screenshot failed: {e}"),
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if !ctx.input(|i| i.pointer.any_down())
            && let Err(e) = self.workspace.save(false)
        {
            self.message = format!("Layout could not be saved: {e}");
        }
        ctx.request_repaint_after(if self.animate && self.page == Page::Studio {
            Duration::from_millis(66)
        } else {
            Duration::from_millis(500)
        });
    }
    fn on_exit(&mut self) {
        self.checkpoint_widgets();
        if self.screenshot.is_none() {
            crate::window_memory::save(&self.window_placement);
        }
        // Normal window close waits in the UI; this also finalizes on other exits.
        if let Some(stream) = &self.live_stream {
            stream.stop();
        }
        if let Some(recording) = &self.live_recording {
            recording.stop();
        }
        drop(self.live_stream.take());
        self.audio.streaming_outlets(Vec::new());
        if let Some(recording) = self.live_recording.take() {
            recording.stop();
            drop(recording);
        }
        self.audio.recording_outlet(None);
        if let Err(e) = self.workspace.save(true) {
            eprintln!("Could not save layout: {e}");
        }
        if self.dirty && self.recovery_candidate.is_none() {
            self.save();
        }
        if self.project.preferences.backups.on_close && self.recovery_candidate.is_none() {
            let _ = self.back_up("on close");
        }
        // Switching profile: start Rosorah again once this one has saved and stopped everything.
        if (self.restart_profile.take().is_some() || self.restart_after_update)
            && let Ok(exe) = std::env::current_exe()
        {
            // After an update replaced the program, Linux names the running file "… (deleted)".
            let exe = std::path::PathBuf::from(exe.to_string_lossy().trim_end_matches(" (deleted)"));
            let mut command = std::process::Command::new(&exe);
            if let Some(dir) = exe.parent() {
                command.current_dir(dir);
            }
            if let Err(e) = command.spawn() {
                eprintln!("Could not restart Rosorah: {e}");
            }
        }
    }
}

struct StudioTabs<'a>(&'a mut Studio);
impl egui_dock::TabViewer for StudioTabs<'_> {
    type Tab = Panel;
    fn minimum_height(&self, ctx: &egui::Context, tab: &Panel) -> f32 {
        if *tab == Panel::Mixer {
            ctx.content_rect().height() * 0.15
        } else {
            0.
        }
    }
    fn animate_leaf(
        &mut self,
        ctx: &egui::Context,
        tab: &mut Panel,
        rect: egui::Rect,
    ) -> egui::Rect {
        // Resizing the window by its edge is handled by the system, so no button looks held:
        // panels follow the window instantly for a moment after its size changes.
        let size = ctx.content_rect().size();
        let now = ctx.input(|i| i.time);
        let resized_at = ctx.data_mut(|d| {
            let (last, at) = d.get_temp_mut_or_insert_with(egui::Id::new("window-size"), || (size, f64::NEG_INFINITY));
            if *last != size {
                *last = size;
                *at = now;
            }
            *at
        });
        let duration = if self.0.project.preferences.workspace.animations
            && !ctx.input(|i| i.pointer.any_down())
            && now - resized_at > 0.4
        {
            0.16
        } else {
            0.
        };
        let id = egui::Id::new(("panel-motion", *tab));
        let values = [rect.min.x, rect.min.y, rect.max.x, rect.max.y];
        let mut v = [0.; 4];
        for i in 0..4 {
            v[i] = ctx.animate_value_with_time(id.with(i), values[i], duration);
        }
        let result = egui::Rect::from_min_max(egui::pos2(v[0], v[1]), egui::pos2(v[2], v[3]));
        self.0.panels_animating |= result != rect;
        result
    }
    fn title(&mut self, tab: &mut Panel) -> egui::WidgetText {
        self.0.panel_title(*tab).into()
    }
    fn id(&mut self, tab: &mut Panel) -> egui::Id {
        egui::Id::new(("studio-panel", *tab))
    }
    fn is_closeable(&self, tab: &Panel) -> bool {
        *tab != Panel::Preview
    }
    fn scroll_bars(&self, _: &Panel) -> [bool; 2] {
        [false, false]
    }
    fn on_tab_button(&mut self, tab: &mut Panel, response: &egui::Response) {
        if *tab == Panel::Sources && let Some(rect @ None) = &mut self.0.demo_drag {
            *rect = Some(response.rect);
        }
        // Noted on press: once the drag gets going, the dock hands us a stand-in that never reports dragging.
        if (response.dragged() || response.is_pointer_button_down_on()) && *tab != Panel::Preview {
            self.0.dragging_panel = Some(*tab);
        }
        if response.double_clicked() {
            self.0.toggle_panel = Some(*tab);
        }
        response.clone().on_hover_cursor(grab_cursor()).on_hover_text(
            "Drag toward an edge to snap. Drop in the center to group tabs. Drag past the window edge for its own window, or drag a floating panel there. Double-click to float or dock.",
        );
    }
    fn context_menu(
        &mut self,
        ui: &mut egui::Ui,
        tab: &mut Panel,
        surface: egui_dock::SurfaceIndex,
        _: egui_dock::NodeIndex,
    ) {
        if !surface.is_main() && ui.button("Dock in workspace").clicked() {
            self.0.toggle_panel = Some(*tab);
            ui.close();
        }
        if *tab != Panel::Preview && ui.button("Open in its own window").clicked() {
            self.0.pop_out_panel = Some(*tab);
            ui.close();
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Panel) {
        crate::appearance::glass(ui, egui::CornerRadius { nw: 0, ne: 0, sw: 16, se: 16 });
        if matches!(tab, Panel::Preview | Panel::Vertical) {
            self.0.canvas_panel(ui, *tab == Panel::Vertical);
            return;
        }
        if *tab == Panel::Mixer {
            // The mixer scrolls its own channel strips so they fit the panel.
            self.0.mixer_panel(ui);
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt(*tab)
            .auto_shrink([false, false])
            .show(ui, |ui| match tab {
                Panel::Scenes => self.0.scenes_panel(ui),
                Panel::Sources => self.0.sources_panel(ui),
                Panel::Broadcast => self.0.broadcast_panel(ui),
                Panel::Chat => self.0.chat_panel(ui),
                Panel::Activity => self.0.event_history.ui(
                    ui,
                    &[&self.0.accounts[0], &self.0.accounts[1], &self.0.kick_account],
                    &[self.0.donations.status().owner, self.0.kofi.status().owner],
                    self.0.stream_config.tiktok_username.trim().trim_start_matches('@'),
                    self.0.project.preferences.automation_history.show_missed,
                ),
                Panel::Preview => self.0.canvas_panel(ui, false),
                Panel::Vertical => self.0.canvas_panel(ui, true),
                Panel::Mixer => self.0.mixer_panel(ui),
                Panel::Web(id) => {
                    let id = *id;
                    let url = self.0.project.preferences.workspace.browser_docks.iter().find(|d| d.id == id).map(|d| d.url.clone());
                    match url {
                        Some(url) => self.0.web_docks.ui(ui, id, &url),
                        None => note(ui, "This web page was removed."),
                    }
                }
            });
    }
}

fn ensure_automatic_source(project: &mut Project) -> bool {
    if !project.preferences.auto_game_capture
        || project
            .sources
            .iter()
            .any(|s| matches!(s.kind, SourceKind::ScreenCapture))
    {
        return false;
    }
    let id = project.id();
    project.sources.push(Source {
        id,
        name: if cfg!(windows) { "Monitor capture" } else { "Automatic renderer capture" }.into(),
        kind: SourceKind::ScreenCapture,
    });
    for scene in &mut project.scenes {
        if project.canvases.iter().any(|c| c.scene == scene.id) {
            scene.layers.push(Layer { crop: [0.;4], group: 0, frame: Default::default(), look: Default::default(),
                source: id,
                rect: [0., 0., 1., 1.],
                visible: true,
                locked: false,
                opacity: 1.,
            });
        }
    }
    true
}

#[cfg(test)]
mod migration_tests {
    use super::*;
    #[test]
    fn automatic_source_is_shared_and_idempotent() {
        let mut project = Project::default();
        assert!(ensure_automatic_source(&mut project));
        assert!(!ensure_automatic_source(&mut project));
        let sources: Vec<_> = project
            .sources
            .iter()
            .filter(|s| matches!(s.kind, SourceKind::ScreenCapture))
            .collect();
        assert_eq!(sources.len(), 1);
        for canvas in &project.canvases {
            let scene = project
                .scenes
                .iter()
                .find(|s| s.id == canvas.scene)
                .unwrap();
            assert_eq!(
                scene
                    .layers
                    .iter()
                    .filter(|l| l.source == sources[0].id)
                    .count(),
                1
            );
        }
        let mut disabled = Project::default();
        disabled.preferences.auto_game_capture = false;
        assert!(!ensure_automatic_source(&mut disabled));
    }

    #[test]
    fn preserves_legacy_and_existing_new_project() {
        let base = std::env::temp_dir().join(format!(
            "rosorah-migration-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let old = base.join("auroracast/project.json");
        std::fs::create_dir_all(old.parent().unwrap()).unwrap();
        Project::default().save(&old).unwrap();
        std::fs::write(old.with_extension("layout.json"), "legacy layout").unwrap();
        let new = migrate_project(&base);
        assert_eq!(std::fs::read(&old).unwrap(), std::fs::read(&new).unwrap());
        assert_eq!(
            std::fs::read_to_string(new.with_extension("layout.json")).unwrap(),
            "legacy layout"
        );
        std::fs::write(&new, "newer project").unwrap();
        assert_eq!(migrate_project(&base), new);
        assert_eq!(std::fs::read_to_string(&new).unwrap(), "newer project");
        assert!(Project::load(&old).is_ok());
        std::fs::remove_dir_all(base).unwrap();
    }
}

include!("widget_editor.rs");

include!("layer_tools.rs");
include!("projectors.rs");
include!("hotkey_page.rs");
include!("remote_page.rs");

include!("layer_properties.rs");

include!("connections.rs");

include!("appearance_settings.rs");

include!("transitions.rs");

include!("go_live.rs");

include!("first_run.rs");

include!("utility_windows.rs");
include!("loading.rs");

#[cfg(test)]
mod sortable_rows_tests {
    use super::*;
    /// Draws three rows below `gap` points of space, returning each row's top edge.
    fn frame(ctx: &egui::Context, time: f64, gap: f32) -> Vec<f32> {
        let mut tops = vec![0.; 3];
        let input = egui::RawInput { time: Some(time), screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400., 600.))), ..Default::default() };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add_space(gap);
                sortable_rows(ui, egui::Id::new("rows"), 3, 36., true, |ui, p| {
                    tops[p] = ui.max_rect().top();
                    ui.label("row")
                });
            });
        });
        tops
    }
    /// Three 30-point items; returns where each was drawn and any finished move.
    fn shift_frame(ctx: &egui::Context, time: f64, events: Vec<egui::Event>) -> (Vec<f32>, Option<(usize, usize)>) {
        let mut moved = None;
        let input = egui::RawInput { time: Some(time), events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400., 600.))), ..Default::default() };
        let output = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut shift = ShiftDrag::begin(ui, egui::Id::new("shift"), false, false, 3);
                for p in 0..3 {
                    shift.item(ui, p, |ui| {
                        let (rect, response) = ui.allocate_exact_size(egui::vec2(200., 30.), egui::Sense::drag());
                        ui.painter().rect_filled(rect, 0., Color32::WHITE);
                        response
                    });
                }
                moved = shift.finish(ui);
            });
        });
        // The painted white boxes carry the visual shift: their tops are where each item was drawn.
        let drawn = output
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                // The held item is drawn a little see-through, so match any light grey too.
                egui::Shape::Rect(r) if r.fill.r() > 150 && r.fill.r() == r.fill.g() && r.fill.g() == r.fill.b() => Some(r.rect.top()),
                _ => None,
            })
            .collect();
        (drawn, moved)
    }
    #[test]
    fn dragged_item_slides_others_aside_and_lands_where_dropped() {
        use egui::{Event, PointerButton, pos2};
        let ctx = egui::Context::default();
        let press = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        let (rest, _) = shift_frame(&ctx, 0.0, vec![]);
        let item0 = pos2(50., rest[0] + 15.);
        shift_frame(&ctx, 0.1, vec![Event::PointerMoved(item0), press(item0, true)]);
        shift_frame(&ctx, 0.2, vec![Event::PointerMoved(item0 + egui::vec2(0., 20.))]);
        let below = item0 + egui::vec2(0., 75.);
        let (held, _) = shift_frame(&ctx, 0.3, vec![Event::PointerMoved(below)]);
        let (held, _) = if held[1] >= rest[1] { shift_frame(&ctx, 0.4, vec![Event::PointerMoved(below)]) } else { (held, None) };
        assert!(held[1] < rest[1] - 10., "item 1 slid up: {held:?} vs {rest:?}");
        assert!(held[0] > rest[0] + 40., "held item follows the pointer: {held:?}");
        let (_, moved) = shift_frame(&ctx, 0.5, vec![press(below, false)]);
        assert_eq!(moved, Some((0, 2)));
    }
    #[test]
    fn rows_move_with_their_panel_without_lagging() {
        let ctx = egui::Context::default();
        let before = frame(&ctx, 0.0, 0.);
        frame(&ctx, 0.5, 0.);
        // The panel's top moves down 120 points (a divider or the window being resized).
        let after = frame(&ctx, 0.51, 120.);
        for (b, a) in before.iter().zip(&after) {
            assert!((a - b - 120.).abs() < 0.01, "row lagged: {b} → {a}");
        }
    }
}

#[cfg(test)]
mod cursor_tests {
    use super::*;
    #[test]
    fn selected_text_is_black_unless_the_accent_is_black() {
        assert_eq!(on_accent(Color32::from_rgb(196, 170, 255)), Color32::BLACK);
        assert_eq!(on_accent(Color32::from_rgb(255, 230, 0)), Color32::BLACK);
        assert_eq!(on_accent(Color32::BLACK), Color32::WHITE);
        assert_eq!(on_accent(Color32::from_rgb(20, 20, 30)), Color32::WHITE);
    }
    #[test]
    fn drag_cursors_return() {
        let expected = if cfg!(windows) { [egui::CursorIcon::PointingHand; 2] } else { [egui::CursorIcon::Grab, egui::CursorIcon::Grabbing] };
        assert_eq!([grab_cursor(), grabbing_cursor()], expected);
    }
}
#[cfg(test)]
mod reuse_preview_tests {
    use super::*;
    #[test]
    fn previews_fit_and_keep_shape() {
        let (w, h) = (1920u32, 1080u32);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
            let x = i as u32 % w;
            px.copy_from_slice(&if x < w / 2 { [255, 0, 0, 255] } else { [0, 0, 255, 255] });
        }
        let picture = preview_picture(&rosorah_capture::Frame { width: w, height: h, rgba }, 320, 180).unwrap();
        assert_eq!(picture.size, [320, 180]);
        assert_eq!(picture.pixels[0], Color32::from_rgb(255, 0, 0));
        assert_eq!(picture.pixels[319], Color32::from_rgb(0, 0, 255));
        let tall = preview_picture(&rosorah_capture::Frame { width: 1080, height: 1920, rgba: vec![9; 1080 * 1920 * 4] }, 320, 180).unwrap();
        assert_eq!(tall.size, [101, 180]);
        assert!(preview_picture(&rosorah_capture::Frame { width: 10, height: 10, rgba: vec![] }, 320, 180).is_none());
    }
}
