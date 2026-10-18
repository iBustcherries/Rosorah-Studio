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
}
/// A yes/no question in the middle of the screen. Nothing else can be clicked until it's answered;
/// Esc counts as Cancel. Returns Some(true) for yes, Some(false) for cancel.
fn confirm(ctx: &egui::Context, id: &str, title: &str, body: &str, yes: &str) -> Option<bool> {
    let mut answer = None;
    egui::Modal::new(egui::Id::new(("confirm", id))).frame(solid_window(ctx)).show(ctx, |ui| {
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
/// Guide and checklist windows stay solid even with see-through panels, so they're easy to read.
fn solid_window(ctx: &egui::Context) -> egui::Frame {
    let [r, g, b, _] = panel().to_srgba_unmultiplied();
    egui::Frame::window(&ctx.style()).fill(Color32::from_rgb(r, g, b))
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
                let duration = if animate && !snap { 0.12 } else { 0. };
                ui.ctx().animate_value_with_time(key.with(("y", p)), start.y + slot as f32 * stride, duration)
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
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
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
        .corner_radius(16)
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
    tiktok_diagnostic_wanted: bool,
    appearance: crate::appearance::Renderer,
    appearance_picker: Option<mpsc::Receiver<Option<PathBuf>>>,
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
    web_docks: crate::browser_dock::Docks,
    announcer: crate::announce::Announcer,
    /// Whether this stream has been announced yet.
    announced: bool,
    /// Announcements switched off in the Go live checklist, for this stream only ([Discord, Bluesky]).
    announce_skip: [bool; 2],
    updates: crate::updates::Updates,
    update_window: bool,
    update_checked: bool,
    /// The scene being named inside its own pill: its id, the text so far, and whether the
    /// box has been focused yet.
    naming_scene: Option<(Id, String, bool)>,
    /// Name and address being entered for a new web page panel.
    web_dock_dialog: Option<(String, String)>,
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
    account_config: crate::accounts::Config,
    browser_chat: crate::browser_chat::BrowserChat,
    event_relayed: std::collections::VecDeque<String>,
    event_ids: std::collections::HashSet<String>,
    event_revision: u64,
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
    rewards_ui:crate::rewards_ui::RewardsUi,
    automation_queue: crate::automations::Scheduler,
    chat_session: alert_filters::ChatSession,
    chat_alert_revision: u64,
    was_streaming: bool,
    donation_save_due: Option<Instant>,
    donations: crate::donations_live::Hub,
    kofi: crate::kofi::Hub,
    event_user: String,
    event_text: String,
    active_alert: Option<(String, Instant)>,
    activity: VecDeque<String>,
    message: String,
    image_path: String,
    adding_image: bool,
    adding_video: bool,
    video_picker: Option<mpsc::Receiver<Option<PathBuf>>>,
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
    last_recording: Option<PathBuf>,
    recording_error: String,
    recording_details: String,
    closing: bool,
    record_folder: String,
    screenshot: Option<PathBuf>,
    screenshot_requested: bool,
    /// When the studio opened; screenshots wait a moment so pages and fades can settle.
    started: Instant,
    loading: Option<Loading>,
    ui_frames: u32,
}
impl Studio {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        mut project: Project,
        project_path: PathBuf,
        screenshot: Option<PathBuf>,
        page: String,
    ) -> Self {
        project.prepare_workspace();
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
        style.visuals.selection.stroke = egui::Stroke::new(1., Color32::from_rgb(56, 30, 114));
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
            widget.fg_stroke = egui::Stroke::new(1., Color32::from_rgb(238, 235, 245));
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
        let (stream_config, stream_error) =
            match crate::streaming::Config::load_with_keyring(&crate::streaming::Config::path()) {
                Ok(config) => (config, String::new()),
                Err(e) => (crate::streaming::Config::default(), format!("{e:#}")),
            };
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
        let first_run = !project_path.exists();
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
        // `--page sound-settings` opens the microphone's settings for screenshots.
        let sound_settings = (page == "sound-settings").then(|| project.audio.iter().find(|c| c.name.to_lowercase().contains("mic")).map(|c| c.id)).flatten();
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
            selections:project.canvases.first().and_then(|c|selected.map(|i|(c.scene,vec![i]))).into_iter().collect(),crop_layer:None,pending_layer_delete:None,layer_rename:None,layer_properties:None,tiktok_chat:true,tiktok_diagnostic_wanted:false,appearance:Default::default(),appearance_picker:None,scene_transition:None,transition_picker:None,
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
            project,
            project_path,
            page: match page.as_str() {
                "settings" | "streaming" | "accounts" | "donations" | "announcements" => Page::Settings,
                "automation-preview" | "alerts" | "alert-guide" | "alert-guide-media" | "alert-guide-finish" => Page::Alerts,
                _ => Page::Studio,
            },
            settings: if page == "donations" {
                Settings::Donations
            } else if page == "announcements" {
                Settings::Announcements
            } else if page == "accounts" || page == "streaming" {
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
            web_docks: Default::default(),
            announcer: Default::default(),
            announced: false,
            announce_skip: [false; 2],
            updates: Default::default(),
            // `--page update` (with --screenshot) shows the update window, checking as an old version.
            update_window: page == "update",
            update_checked: false,
            // `--page name-scene` opens the name box in the current scene's pill for screenshots.
            naming_scene: (page == "name-scene").then(|| (scene_for_naming, String::new(), false)),
            web_dock_dialog: None,
            broadcast_window: if page == "add-service" { 3 } else { 0 },
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
            account_config,
            browser_chat: crate::browser_chat::BrowserChat::default(),
            event_relayed: Default::default(),
            event_ids: Default::default(),
            event_revision: 0,
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
            rewards_ui:Default::default(),
            automation_queue: Default::default(),
            chat_session: {
                let mut session = alert_filters::ChatSession::default();
                session.start(now_ms());
                session
            },
            chat_alert_revision: u64::MAX,
            was_streaming: false,
            donation_save_due: None,
            donations: crate::donations_live::Hub::new(),
            kofi: crate::kofi::Hub::new(),
            event_user: "Moe".into(),
            event_text: "!hello".into(),
            active_alert: None,
            activity: VecDeque::new(),
            message: "Ready".into(),
            image_path: String::new(),
            adding_image: false,
            adding_video: false,
            video_picker: None,
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
            last_recording: None,
            recording_error: String::new(),
            recording_details: String::new(),
            closing: false,
            record_folder,
            loading: screenshot.is_none().then(Loading::start),
            screenshot,
            screenshot_requested: false,
            started: Instant::now(),
            ui_frames: 0,
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
    }
    fn accent(&self) -> Color32 {
        let [r, g, b] = self.project.preferences.accent;
        Color32::from_rgb(r, g, b)
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
        if let Some(rx) = self.recovery_write.take() {
            if matches!(
                rx.recv_timeout(Duration::from_secs(3)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ) {
                self.recovery_write = Some(rx);
                self.message =
                    "Recovery save is still finishing. Please save again in a moment.".into();
                return;
            }
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
                ui.horizontal_centered(|ui| {
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
                        if self.advanced() {
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
                                if ui.button("Reset layout").clicked() {
                                    self.reset_layout();
                                    ui.close();
                                }
                            });
                        }
                    }
                    ui.menu_button("Help", |ui| self.help_menu(ui));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| self.live_controls(ui));
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
            let (text, color) = if view.phase == Phase::Starting {
                ("● STARTING…".to_owned(), Color32::from_rgb(230, 190, 90))
            } else {
                (format!("● LIVE {} · {}", clock(view.elapsed), destinations.join("  ")), Color32::from_rgb(255, 92, 92))
            };
            ui.label(RichText::new(text).strong().color(color))
                .on_hover_text("✓ sending · … connecting · ✕ needs attention. Check your platform's dashboard to see what viewers get.");
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
            ui.label(RichText::new("● REC").strong().color(Color32::from_rgb(255, 92, 92)));
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
                crate::updates::State::Downloaded(r, _) => Some(format!("⬆ Install {}", r.version)),
                _ => None,
            };
            if let Some(text) = offer && ui.add(egui::Button::new(RichText::new(text).small().strong().color(Color32::from_rgb(40, 22, 84))).fill(self.accent())).clicked() {
                self.update_window = true;
            }
            ui.separator();ui.label(RichText::new(&self.message).small());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{note(ui,format!("RAM {}",fmt(self.resources.ram)));note(ui,format!("GPU {}",self.resources.gpu.iter().map(|(_,v)|*v).max().map_or("—".into(),|v|format!("{v}%"))));note(ui,format!("CPU {}",fmt(self.resources.cpu)));}).response.on_hover_text("System usage (all applications), updated once per second. GPU shows the busiest reported device/engine. Unsupported counters show —, never a fabricated zero.");
        });});
    }
    fn scene_context(&mut self, ui: &mut egui::Ui, canvas: Id, id: Id) {
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
    fn scenes_panel(&mut self, ui: &mut egui::Ui) {
        let canvas = self.project.canvases[self.canvas].id;
        let active = self.project.canvases[self.canvas].scene;
        let vertical_in_use = self.project.canvases.len() > 1 && (self.advanced() || self.project.preferences.workspace.vertical_video);
        if self.canvas > 0 {
            ui.horizontal_wrapped(|ui| {
                note(ui, format!("{} scenes", self.project.canvases[self.canvas].name));
                if ui.small_button("Back to main video").clicked() {
                    self.canvas = 0;
                    self.layer = None;
                }
            });
        }
        let ids = self.project.canvas_scenes(canvas);
        let live = self.live_stream.is_some() || self.live_recording.is_some();
        let animate = self.project.preferences.workspace.animations;
        let moved = sortable_rows(ui, egui::Id::new(("scene-rows", canvas)), ids.len(), 36., animate, |ui, p| {
            let id = ids[p];
            let name = self.project.scene(id).map(|s| s.name.clone()).unwrap_or_default();
            let on_air = live && active == id;
            let hovered = ui.rect_contains_pointer(ui.max_rect());
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
                let text = if on_air {
                    RichText::new(format!("● LIVE  {name}")).color(Color32::from_rgb(255, 92, 92)).strong()
                } else {
                    RichText::new(name)
                };
                let response = ui
                    .add_sized([(ui.available_width() - 34.).max(30.), 30.], egui::Button::new(text).selected(active == id).truncate().sense(egui::Sense::click_and_drag()))
                    .on_hover_text("Click to switch · drag to reorder · right-click for more");
                if response.clicked() {
                    self.request_scene(canvas, id);
                }
                response.context_menu(|ui| self.scene_context(ui, canvas, id));
                ui.scope(|ui| {
                    if !hovered && !egui::Popup::is_any_open(ui.ctx()) {
                        ui.multiply_opacity(0.);
                    }
                    ui.menu_button("…", |ui| self.scene_context(ui, canvas, id));
                });
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
                    if self.canvas == 0 && ui.button("Edit the vertical video's scenes").clicked() {
                        self.canvas = 1;
                        self.layer = None;
                        ui.close();
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
            State::Available(r) | State::Downloading(r, ..) | State::Downloaded(r, _) => r.clone(),
            // Still finding out; the window opens once there's something to show.
            State::Idle | State::Checking => return,
            _ => {
                self.update_window = false;
                return;
            }
        };
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new("update-window")).frame(solid_window(ctx)).show(ctx, |ui| {
            ui.set_width(460.);
            ui.heading(format!("Rosorah Studio {}", release.version));
            note(ui, concat!("You have ", env!("CARGO_PKG_VERSION"), "."));
            ui.add_space(8.);
            egui::ScrollArea::vertical().max_height(240.).show(ui, |ui| {
                ui.label(release.notes.lines().filter(|l| !l.starts_with('|')).collect::<Vec<_>>().join("\n").replace("**", "").replace("## ", ""));
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
                        if ui.add(self.primary_button("Install now")).on_hover_text("Opens the installer. It asks you to close Rosorah first.").clicked() {
                            let _ = std::process::Command::new(path).spawn();
                        }
                    } else {
                        let command = if path.extension().is_some_and(|e| e == "deb") { "sudo apt install" } else { "sudo dnf install" };
                        let command = format!("{command} \"{}\"", path.display());
                        ui.horizontal_wrapped(|ui| {
                            if ui.add(self.primary_button("Install…")).on_hover_text("Opens it in your software app (Discover or Software)").clicked() {
                                let _ = crate::desktop::open(path);
                            }
                            if ui.button("Copy install command").on_hover_text(&command).clicked() {
                                ui.ctx().copy_text(command.clone());
                                self.tell("Copied. Paste it into a terminal, then restart Rosorah.".into());
                            }
                        });
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
                if !matches!(state, State::Downloading(..)) && ui.button("Skip this version").clicked() {
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
                test = Some([true, false]);
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
                test = Some([false, true]);
            }
        });
        note(ui, "The webhook address and app password are kept in your system's keyring with your stream keys.");
        if let Some(which) = test {
            self.announcer.send(&self.stream_config.announce, preview_stream, which, ui.ctx());
            self.tell("Sending a test announcement…".into());
        }
        if before != self.stream_config.announce {
            self.save_stream_settings();
        }
    }
    fn panel_title(&self, panel: Panel) -> String {
        match panel {
            Panel::Web(id) => self.project.preferences.workspace.browser_docks.iter().find(|d| d.id == id).map_or_else(|| panel.title().into(), |d| d.name.clone()),
            _ => panel.title().into(),
        }
    }
    /// Asks for a name and address, then adds the page as a panel.
    fn web_dock_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some((mut name, mut url)) = self.web_dock_dialog.take() else {
            return;
        };
        let mut done = false;
        let valid = crate::browser_source::valid_url(url.trim());
        egui::Modal::new(egui::Id::new("web-dock-dialog")).frame(solid_window(ctx)).show(ctx, |ui| {
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
        let modal = egui::Modal::new(egui::Id::new("scene-dialog")).frame(solid_window(ctx)).show(ctx, |ui| {
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
        self.scene_dialog = if open && !done {
            Some((id, name, delete))
        } else {
            None
        };
    }
    fn sources_panel(&mut self, ui: &mut egui::Ui) {
        let active = self.project.canvases[self.canvas].scene;
        if self.canvas > 0 {
            note(ui, format!("{} · {}", self.project.canvases[self.canvas].name, self.project.scene(active).map(|s| s.name.as_str()).unwrap_or("Scene")));
        }
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
            let hovered = ui.rect_contains_pointer(ui.max_rect());
            ui.spacing_mut().button_padding = egui::vec2(5., 3.);
            ui.spacing_mut().item_spacing = egui::vec2(4., 2.);
            ui.horizontal(|ui| {
                let eye = egui::Button::new(if visible { RichText::new("👁").color(ui.visuals().strong_text_color()) } else { RichText::new("👁").color(muted()).strikethrough() }).frame(false);
                if ui.add_sized([22., 26.], eye).on_hover_text(if visible { "Showing · click to hide" } else { "Hidden · click to show" }).clicked() {
                    self.project.scenes[si].layers[i].visible = !visible;
                    self.changed();
                }
                let label = if locked { format!("🔒 {name}") } else { name.clone() };
                let response = ui
                    .add_sized(
                        [(ui.available_width() - 28.).max(30.), 26.],
                        egui::Button::new(RichText::new(label).color(if visible { ui.visuals().text_color() } else { muted() }))
                            .sense(if locked { egui::Sense::click() } else { egui::Sense::click_and_drag() })
                            .selected(self.selections.get(&active).is_some_and(|v| v.contains(&i)))
                            .truncate(),
                    )
                    .on_hover_text(if locked { "Locked · right-click to unlock" } else { "Drag to reorder (top appears in front) · double-click to change" });
                if !locked {
                    response.clone().on_hover_cursor(egui::CursorIcon::Grab);
                }
                if response.clicked() {self.select_layer(active,i,ui.input(|i|i.modifiers.shift));}
                if response.double_clicked() {
                    self.select_layer(active, i, false);
                    self.layer_properties = Some((active, i));
                }
                response.context_menu(|ui| self.layer_menu(ui, si, i, &name, &mut dropped));
                ui.scope(|ui| {
                    if !hovered && !egui::Popup::is_any_open(ui.ctx()) {
                        ui.multiply_opacity(0.);
                    }
                    ui.menu_button("…", |ui| self.layer_menu(ui, si, i, &name, &mut dropped));
                });
                response
            })
            .inner
        });
        if let Some((from, to)) = moved {
            dropped = Some((count - 1 - from, count - 1 - to));
        }
        if let Some((from, to)) = dropped.filter(|(from, to)| {
            from != to
                && *from < count
                && *to < count
                && !self.project.scenes[si].layers[*from].locked
        }) {
            let mut origins: Vec<_> = (0..count).collect();
            let original = origins.remove(from);
            origins.insert(to, original);
            let moved = self.project.scenes[si].layers.remove(from);
            self.project.scenes[si].layers.insert(to, moved);
            if let Some(selected)=self.selections.get_mut(&active){*selected=selected.iter().filter_map(|i|origins.iter().position(|old|old==i)).collect();}
            if let Some((scene,index))=self.crop_layer && scene==active {self.crop_layer=origins.iter().position(|old|*old==index).map(|i|(scene,i));}
            if let Some((scene,index))=self.layer_properties && scene==active {self.layer_properties=origins.iter().position(|old|*old==index).map(|i|(scene,i));}
            self.layer = self
                .layer
                .and_then(|selected| origins.iter().position(|old| *old == selected));
            self.changed();
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
        egui::containers::menu::MenuButton::from_button(self.primary_button("+ Add")).ui(ui, |ui| {
            if ui.button("Game, window or screen").clicked() {
                ui.close();
                let id = self.add_source_layer(si, "Window / Screen", SourceKind::ScreenCapture, [0., 0., 1., 1.]);
                #[cfg(windows)] self.captures.set_mode(id,rosorah_capture::Target::AutoFullscreen);
                #[cfg(not(windows))] self.captures.start(id);
                self.remember_capture_mode(id);
            }
            if ui.button("Camera").on_hover_text("Webcam, USB camera or DroidCam; choose which in the window that opens").clicked() {
                ui.close();
                let c = &self.project.canvases[self.canvas];
                let w = 0.28;
                let rect = [0.68, 0.05, w, w * c.width as f32 / c.height as f32 * 9. / 16.];
                self.add_source_layer(si, "Camera", SourceKind::VideoCamera(Default::default()), rect);
                self.layer_properties = Some((self.project.scenes[si].id, self.project.scenes[si].layers.len() - 1));
            }
            ui.separator();
            if ui.button("Media file · video, GIF or sound").clicked() {
                self.adding_video = true;
                self.adding_image = false;
                self.image_path.clear();
                ui.close();
            }
            if ui.button("Image").clicked() {
                ui.close();
                self.adding_image = !self.adding_image;
            }
            if ui.button("Browser source").clicked() {
                ui.close();
                let id = self.add_source_layer(si, "Browser source", SourceKind::Browser { url: String::new() }, [0., 0., 1., 1.]);
                self.project.preferences.browser_sources.insert(id, Default::default());
                self.layer_properties = Some((self.project.scenes[si].id, self.project.scenes[si].layers.len() - 1));
            }
            if ui.button("Text").on_hover_text("Write a note on the canvas in any font").clicked() {
                ui.close();
                self.add_widget(widgets::Kind::Text);
            }
            if ui.button("Color").clicked() {
                ui.close();
                self.add_source_layer(si, "Color source", SourceKind::Color([70, 170, 220, 255]), [0.1, 0.1, 0.3, 0.3]);
            }
            ui.separator();
            if ui.button("Alerts").on_hover_text("Shows follows, subs, donations and other alerts on screen").clicked(){self.add_widget(widgets::Kind::AlertBox);ui.close();}
            ui.menu_button("Widgets", |ui| {
                note(ui, "Goals, timers, counters, chat on screen and more");
                egui::ScrollArea::vertical().max_height(430.).show(ui, |ui| {
                    for kind in widgets::Kind::ALL.into_iter().filter(|k| !matches!(k, widgets::Kind::AlertBox | widgets::Kind::Text)) {
                        if ui.button(kind.label()).clicked() {
                            self.add_widget(kind);
                            ui.close();
                        }
                    }
                });
            });
            // Captures and cameras already set up elsewhere, so one device can appear in several scenes.
            let here: Vec<_> = self.project.scenes[si].layers.iter().map(|l| l.source).collect();
            let reusable: Vec<_> = self
                .project
                .sources
                .iter()
                .filter(|s| matches!(s.kind, SourceKind::ScreenCapture | SourceKind::VideoCamera(_)) && !here.contains(&s.id))
                .map(|s| {
                    let scenes: Vec<_> = self.project.scenes.iter().filter(|sc| sc.layers.iter().any(|l| l.source == s.id)).map(|sc| sc.name.as_str()).collect();
                    let kind = if matches!(s.kind, SourceKind::ScreenCapture) { "capture" } else { "camera" };
                    let place = if scenes.is_empty() { "not in any scene".to_owned() } else { format!("in {}", scenes.join(", ")) };
                    (s.id, format!("{} · {kind} · {place}", s.name))
                })
                .collect();
            if !reusable.is_empty() {
                ui.separator();
                ui.menu_button("Reuse a capture or camera", |ui| {
                    for (id, label) in reusable {
                        if ui.button(label).clicked() {
                            self.project.scenes[si].layers.push(Layer { crop: [0.;4], group: 0, source: id, rect: [0., 0., 1., 1.], visible: true, locked: false, opacity: 1. });
                            let index = self.project.scenes[si].layers.len() - 1;
                            self.layer = Some(index);
                            self.selections.insert(self.project.scenes[si].id, vec![index]);
                            self.changed();
                            ui.close();
                        }
                    }
                });
            }
        });
        if self.adding_video {
            ui.label("Video file");
            ui.add(
                egui::TextEdit::singleline(&mut self.image_path)
                    .hint_text("Choose an MP4, WebM or MOV file"),
            );
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        self.video_picker.is_none(),
                        egui::Button::new("Choose file…"),
                    )
                    .clicked()
                {
                    let (tx, rx) = mpsc::channel();
                    self.video_picker = Some(rx);
                    std::thread::spawn(move || {
                        let path=crate::desktop::pick(crate::desktop::Pick::Open,"Choose video or audio","").unwrap_or_else(|e|{crate::capture_log::event(e.to_string());None});
                        let _ = tx.send(path);
                    });
                }
                if ui
                    .add_enabled(
                        std::path::Path::new(&self.image_path).is_file(),
                        egui::Button::new("Add video"),
                    )
                    .clicked()
                {
                    let id = self.project.id();
                    self.project.sources.push(Source {
                        id,
                        name: std::path::Path::new(&self.image_path)
                            .file_name()
                            .unwrap()
                            .to_string_lossy()
                            .into_owned(),
                        kind: SourceKind::VideoFile(auroracast_core::VideoSettings {
                            path: self.image_path.clone(),
                            ..Default::default()
                        }),
                    });
                    self.project.scenes[si].layers.push(Layer { crop: [0.;4], group: 0,
                        source: id,
                        rect: [0., 0., 1., 1.],
                        visible: true,
                        locked: false,
                        opacity: 1.,
                    });
                    self.layer = Some(self.project.scenes[si].layers.len() - 1);
                    self.selections.insert(self.project.scenes[si].id, vec![self.project.scenes[si].layers.len()-1]);
                    self.adding_video = false;
                    self.changed();
                }
                if ui.button("Cancel").clicked() {
                    self.adding_video = false;
                }
            });
            ui.small("Videos loop by default. Choose an event below to use the video as an alert.");
        }
        if self.adding_image {
            note(ui, "PNG or JPEG path");
            ui.text_edit_singleline(&mut self.image_path);
            if ui.button("Import image").clicked() {
                match image::image_dimensions(&self.image_path) {
                    Ok((w, h)) if w > 0 && h > 0 && w <= 4096 && h <= 4096 => {
                        let id = self.project.id();
                        let name = PathBuf::from(&self.image_path)
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| "Image".into());
                        self.project.sources.push(Source {
                            id,
                            name,
                            kind: SourceKind::Image {
                                path: self.image_path.clone(),
                            },
                        });
                        self.project.scenes[si].layers.push(Layer { crop: [0.;4], group: 0,
                            source: id,
                            rect: [0.1, 0.1, 0.5, 0.5],
                            visible: true,
                            locked: false,
                            opacity: 1.,
                        });
                        self.layer = Some(self.project.scenes[si].layers.len() - 1);
                    self.selections.insert(self.project.scenes[si].id, vec![self.project.scenes[si].layers.len()-1]);
                        self.adding_image = false;
                        self.changed();
                    }
                    Ok(_) => self.message = "Images must be at most 4096 × 4096".into(),
                    Err(e) => self.message = format!("Image could not be opened: {e}"),
                }
            }
        }
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
            ui.label("Include these platforms (select both for a combined widget)");
            for p in [Platform::Twitch, Platform::YouTube, Platform::Donations] {
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
                "Shows up to four new messages as text. Inline emote images are not yet rendered in this canvas widget.",
            );
        }
        if next.kind == widgets::Kind::Shoutout {
            note(
                ui,
                "Use an Update widget action with text such as 'Go follow {user}!' This displays on the canvas; it does not post to chat.",
            );
        }
        if next != *saved {
            self.project
                .sources
                .iter_mut()
                .find(|s| s.id == id)
                .unwrap()
                .kind = SourceKind::Widget(next);
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
                let audio_id = self.project.id();
                self.project.audio.push(auroracast_core::AudioChannel { monitor_only:false,
                    id: audio_id,
                    name,
                    gain_db: 0.,
                    muted: false,
                    tracks: [true, false, false, false, false, false],
                    delay_ms: 0,
                    monitor: false,
                    noise_suppression: Default::default(),
                    media_source: Some(id),
                });
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
        ui.add(
            egui::TextEdit::singleline(&mut draft.0)
                .hint_text("https://… overlay URL")
                .password(true),
        );
        ui.small("Overlay URLs may contain private tokens. HTTP and HTTPS are supported, including local web servers.");
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
        ui.small("Up to 1920×1080 or 1080×1920. Hidden sources stop; showing them reloads the page. Transparency follows the webpage's background.");
        let apply = ui.button("Apply browser settings").clicked();
        let next = draft.clone();
        if apply {
            if !crate::browser_source::valid_url(&next.0) {
                self.message = "Enter a valid HTTP or HTTPS overlay URL".into();
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
                    let audio_id = self.project.id();
                    let name = self.project.source(id).unwrap().name.clone();
                    self.project.audio.push(auroracast_core::AudioChannel { monitor_only:false,
                        id: audio_id,
                        name,
                        gain_db: 0.,
                        muted: false,
                        tracks: [true, false, false, false, false, false],
                        delay_ms: 0,
                        monitor: false,
                        noise_suppression: Default::default(),
                        media_source: Some(id),
                    });
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
                    crate::streaming::Phase::Failed => Color32::LIGHT_RED,
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
        let Some(stream) = &self.live_stream else {
            self.announced = false;
            self.announce_skip = [false; 2];
            return;
        };
        if !self.announced && stream.destinations().iter().any(|d| d.path.is_none() && d.phase == crate::streaming::Phase::Sending) {
            self.announced = true;
            let a = &self.stream_config.announce;
            let which = [a.discord_ready() && !self.announce_skip[0], a.bluesky_ready() && !self.announce_skip[1]];
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
                            ("●", "ready", Color32::from_rgb(110, 210, 140))
                        } else if connected {
                            ("◌", "getting stream key…", Color32::from_rgb(230, 190, 90))
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
            egui::Window::new("Add streaming service")
                .open(&mut open)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
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
            egui::Window::new(format!("{name} connection")).id(egui::Id::new(("service-connection",slot))).anchor(egui::Align2::CENTER_CENTER,egui::Vec2::ZERO).collapsible(false).open(&mut open).default_width(420.).show(ctx,|ui| {
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
        egui::Window::new(if which == 1 {
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
            ui.horizontal(|ui| {
                ui.selectable_value(&mut draft.network,false,"Webcam / USB");
                ui.selectable_value(&mut draft.network,true,"DroidCam Wi-Fi");
            });
            if draft.network {
                ui.label("Phone Wi-Fi IP / hostname"); ui.text_edit_singleline(&mut draft.host);
                ui.horizontal(|ui| { ui.label("Port"); ui.add(egui::DragValue::new(&mut draft.port).range(1..=65535)); });
                ui.checkbox(&mut draft.legacy_url,"Use legacy DroidCam /video address");
                ui.small("Open DroidCam on your phone and use the IP and port shown there. Keep both devices on the same network.");
            } else {
                egui::ComboBox::from_id_salt(("camera_device",id)).selected_text(self.camera_devices.iter().find(|d|d.path==draft.device).map(|d|d.name.as_str()).unwrap_or("Select camera"))
                    .show_ui(ui, |ui| { for device in &self.camera_devices { ui.selectable_value(&mut draft.device, device.path.clone(), format!("{} ({})",device.name,device.path)); } });
                if ui.button("Refresh cameras").clicked() { self.camera_devices=crate::camera::devices(); }
                ui.collapsing("Device path",|ui| {ui.text_edit_singleline(&mut draft.device);});
                ui.small("DroidCam USB or desktop-client connections appear here as a webcam after connecting in the DroidCam Linux client.");
                ui.checkbox(&mut draft.auto_mode,"Auto detect resolution and FPS");
                if draft.auto_mode {
                    ui.small("Uses the camera's available output. For DroidCam, change quality in its desktop client; Rosorah follows the published mode.");
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
        let height =
            (ui.available_height() - if connected.is_empty() { 8. } else { reserve }).max(40.);
        egui::ScrollArea::vertical()
            .id_salt("messages")
            .auto_shrink([false, false])
            .max_height(height)
            .min_scrolled_height(height)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.set_min_height(height);
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
                for m in self
                    .chat
                    .messages()
                    .filter(|m| self.filter.is_none_or(|p| p == m.platform) && (!self.events_only || m.id.starts_with("event-") || m.text.starts_with("★ ")))
                {
                    egui::Frame::new()
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
                        })
                        .response
                        .interact(egui::Sense::click())
                        .context_menu(|ui| message_menu(ui, m, connected.contains(&m.platform.name()), &mut chat_action));
                    ui.add_space(3.);
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
            Some(ChatAction::Clear(Some(platform))) => self.chat.clear_author(platform, ""),
            Some(ChatAction::Clear(None)) => self.chat.clear(),
            None => {}
        }
        let composer_top = ui.cursor().top();
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
            ui.menu_button("😀", |ui| {
                ui.horizontal_wrapped(|ui| {
                    for emoji in ["😀", "😂", "❤️", "🔥", "👋", "🎉", "👍", "🙏", "💜", "👀"] {
                        if ui.button(emoji).clicked() {
                            self.browser_chat.draft.push_str(emoji);
                            ui.close();
                        }
                    }
                });
                let connected = connected_names.clone();
                for platform in &connected {
                    for (name, _) in self.browser_chat.rich.recent(platform) {
                        if ui.button(&name).clicked() {
                            self.browser_chat.draft.push_str(&format!(" {name} "));
                            ui.close();
                        }
                    }
                }
            }).response.on_hover_text("Emoji & emotes");
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
        self.preview(ui, &ctx);
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
        let canvas = self.project.canvases[self.canvas].clone();
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
        if let Some(si) = self
            .project
            .scenes
            .iter()
            .position(|s| s.id == canvas.scene)
            && ui
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
                        self.project.preferences.locked_canvases.contains(&canvas.id),
                    )
                })
                .inner
        {
            self.changed();
        }
        self.canvas_layout_tools(ui);
    }

    fn main_studio(&mut self, ctx: &egui::Context) {
        self.panels_animating = false;
        self.scene_dialog_ui(ctx);
        self.web_dock_dialog_ui(ctx);
        self.update_window_ui(ctx);
        if !self.update_checked && self.screenshot.is_some() && self.update_window {
            self.update_checked = true;
            self.updates.check_as(ctx, "0.0.1");
        }
        if !self.update_checked && self.ui_frames > 2 && self.screenshot.is_none() {
            self.update_checked = true;
            if self.project.preferences.workspace.check_updates {
                self.updates.check(ctx);
            }
        }
        self.web_docks.tick();
        // The page itself stays clear so the backdrop shows in the gaps between docked panels.
        egui::CentralPanel::default().frame(egui::Frame::central_panel(&ctx.style()).fill(bg())).show(ctx, |ui| {
            self.workspace.remember_homes();
            let mut dock =
                std::mem::replace(&mut self.workspace.dock, egui_dock::DockState::new(vec![]));
            let mut style = egui_dock::Style::from_egui(ui.style());
            style.tab_bar.height = 36.;
            style.tab_bar.bg_fill = panel();
            style.tab_bar.corner_radius = egui::CornerRadius {
                nw: 16,
                ne: 16,
                sw: 0,
                se: 0,
            };
            style.tab_bar.hline_color = Color32::TRANSPARENT;
            style.tab.hline_below_active_tab_name = false;
            style.tab.tab_body.bg_fill = panel();
            style.tab.tab_body.stroke = egui::Stroke::NONE;
            style.tab.tab_body.corner_radius = egui::CornerRadius {
                nw: 0,
                ne: 0,
                sw: 16,
                se: 16,
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
                tab.corner_radius = egui::CornerRadius::same(16);
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
            // Simple mode keeps panels where they are; Advanced lets them be dragged, closed and floated.
            let advanced = self.advanced();
            egui_dock::DockArea::new(&mut dock)
                .style(style)
                .draggable_tabs(advanced)
                .show_close_buttons(advanced)
                .tab_context_menus(advanced)
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
            }
            if !ctx.input(|i| i.pointer.any_down()) {
                self.dragging_panel = None;
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
                                .on_hover_cursor(egui::CursorIcon::Grab)
                                .on_hover_text("Drag out of this window and let go to dock it back where it was");
                            if handle.dragged() {
                                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
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
        for channel in self.project.mixer_channels(scene_id) {
            let effect = !advanced && channel.media_source.is_some() && transitions.contains(&channel.media_source);
            if !advanced && self.unconnected(&channel) {
                continue;
            }
            if self.project.is_alert_channel(&channel) || effect {
                if alert_members.is_empty() && effect_members.is_empty() {
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
                ui.colored_label(Color32::LIGHT_RED, error);
            }
        }
        #[derive(Clone, Copy)]
        struct MixerDrag(Id);
        let mut changed = false;
        let dragged = egui::DragAndDrop::payload::<MixerDrag>(ui.ctx()).map(|p| p.0);
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
        let accent = self.accent();
        let sound_settings = &mut self.sound_settings;
        let mut refresh = false;
        let mut draw_channel = |ui: &mut egui::Ui, channel: &mut auroracast_core::AudioChannel| {
            ui.push_id(channel.id, |ui| {
                if dragged == Some(channel.id) {
                    ui.multiply_opacity(0.45);
                }
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
                            ui.add(egui::Label::new(egui::RichText::new(&channel.name).strong()).truncate().sense(egui::Sense::drag()))
                        }).inner;
                        for response in [handle, name] {
                            response.dnd_set_drag_payload(MixerDrag(channel.id));
                            if response.dragged() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                            } else if response.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                            }
                            response.on_hover_text("Drag to reorder");
                        }
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
                    let modal = egui::Modal::new(egui::Id::new(("sound-settings", channel.id))).frame(solid_window(&ctx)).show(&ctx, |ui| {
                    ui.set_width(400.);
                    ui.spacing_mut().item_spacing.y = 6.;
                    ui.heading(&channel.name);
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
                                // Grouped under headings, so the rows don't each repeat what kind of device they are.
                                let groups: &[(&str, &str, fn(&str) -> bool)] = if output {
                                    &[("", "Desktop output: ", |k| k == "Audio/Sink")]
                                } else {
                                    &[
                                        ("Microphones and inputs", "Microphone / input: ", |k| k == "Audio/Source"),
                                        ("Sound playing on this computer", "Desktop output: ", |k| k == "Audio/Sink"),
                                        ("Apps", "Application: ", |k| k != "Audio/Source" && k != "Audio/Sink"),
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
                            });
                        });
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
                        ui.label(RichText::new("Recording tracks").strong());
                        ui.horizontal_wrapped(|ui| {
                            for (i, route) in channel.tracks.iter_mut().enumerate() {
                                changed |= ui.checkbox(route, format!("{}", i + 1)).changed();
                            }
                        });
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
            }).response.rect
        };
        let mut reorder = None;
        let mut drop_target = |ui: &mut egui::Ui, rects: &[egui::Rect]| {
            let Some(id) = dragged else { return };
            let Some(pointer) = ui.ctx().pointer_interact_pos() else { return };
            if !ui.clip_rect().contains(pointer) || rects.is_empty() {
                return;
            }
            let index = rects
                .iter()
                .position(|r| if vertical_audio { pointer.x < r.center().x } else { pointer.y < r.center().y })
                .unwrap_or(rects.len());
            let (before, after) = (index.checked_sub(1).map(|i| rects[i]), rects.get(index).copied());
            let stroke = egui::Stroke::new(3., accent);
            if vertical_audio {
                let x = match (before, after) {
                    (Some(b), Some(a)) => (b.right() + a.left()) / 2.,
                    (Some(b), None) => b.right() + 2.,
                    (None, Some(a)) => a.left() - 2.,
                    (None, None) => return,
                };
                let r = after.or(before).unwrap();
                ui.painter().vline(x, r.y_range(), stroke);
            } else {
                let y = match (before, after) {
                    (Some(b), Some(a)) => (b.bottom() + a.top()) / 2.,
                    (Some(b), None) => b.bottom(),
                    (None, Some(a)) => a.top(),
                    (None, None) => return,
                };
                let r = after.or(before).unwrap();
                ui.painter().hline(r.x_range(), y, stroke);
            }
            if ui.input(|i| i.pointer.any_released()) {
                egui::DragAndDrop::clear_payload(ui.ctx());
                reorder = Some((id, index));
            }
        };
        let mut rects = Vec::with_capacity(scene_channels.len());
        if vertical_audio {
            // One row of strips; extra channels scroll sideways instead of wrapping
            // onto rows below the visible panel.
            egui::ScrollArea::both()
                .id_salt("mixer-strips")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        for channel in &mut scene_channels {
                            let rect = ui
                                .allocate_ui_with_layout(
                                    egui::vec2(108., strip_height),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        ui.set_max_width(108.);
                                        draw_channel(ui, channel)
                                    },
                                )
                                .inner;
                            rects.push(rect);
                        }
                    });
                    drop_target(ui, &rects);
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
                    for channel in &mut scene_channels {
                        rects.push(draw_channel(ui, channel));
                    }
                    drop_target(ui, &rects);
                });
        }
        if refresh {
            self.audio.refresh();
        }
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
                        card().show(ui,|ui|{ui.strong("Audio tracks");note(ui,"Assign each channel to up to six independent output tracks.");egui::Grid::new("routing").striped(true).spacing([24.,16.]).show(ui,|ui|{ui.strong("Source");for n in 1..=6{ui.strong(format!("Track {n}"));}ui.end_row();for channel in &mut self.project.audio{ui.label(&channel.name);for route in &mut channel.tracks{changed|=ui.checkbox(route,"").changed();}ui.end_row();}});ui.add_space(16.);note(ui,"Game audio follows automatic capture. Choose other inputs in Sound. Live video recordings use Track 1, including volume and mute. Enable separate tracks in Advanced audio to include Tracks 2–6 in standalone recordings. Monitoring is opt-in in the mixer.");});
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
            changed |= ui.checkbox(&mut self.project.preferences.workspace.check_updates, "Check for updates when Rosorah starts").changed();
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
                    crate::updates::State::Available(r) | crate::updates::State::Downloaded(r, _) => {
                        if ui.button(format!("Version {} is available…", r.version)).clicked() {
                            self.update_window = true;
                        }
                    }
                    crate::updates::State::Downloading(..) => note(ui, "Downloading the update…"),
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
        if matches!(event.kind,EventKind::ChatMessage|EventKind::ChatCommand)&&self.accounts.iter().any(|a|a.platform==event.platform&&a.connected&&a.name.eq_ignore_ascii_case(&event.author)){return;}

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
            self.captures.widgets.target_alert(id,text,seconds,media);
        }else{
            self.automation_queue.log("Visual alert skipped: add Alerts from In this scene > + Add".into());
        }
    }
    fn run_actions(&mut self, actions: Vec<Action>) {
        for action in actions {
            match &action {
                Action::DisplayAlert{source,template,media,seconds}=>{self.captures.widgets.target_alert(*source,template,*seconds,*media);},
                Action::PlaySound{source}=>{self.captures.videos.trigger(*source);},
                Action::SendChat{text,platforms}=>{
                    for platform in platforms { if let Some(a)=self.accounts.iter_mut().find(|a|a.platform==*platform){
                        let request=format!("automation-{}-{}",now_ms(),platform.name());
                        if !a.post(request,text.clone()){self.automation_queue.log(format!("{} chat not sent: account unavailable",platform.name()));}
                    }}
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
                    self.request_scene(*canvas, *scene);
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
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.ui_frames += 1;
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
        if let Some(rx) = &self.video_picker
            && let Ok(path) = rx.try_recv()
        {
            if let Some(p) = path {
                self.image_path = p.display().to_string();
            }
            self.video_picker = None;
        }

        self.captures.sync_widgets(&self.project);
        for account in &mut self.accounts {
            account.tick(&mut self.chat);
        }
        self.kick_account.tick(&mut self.chat);
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
        for account in self.accounts.iter_mut().chain(std::iter::once(&mut self.kick_account)) {
            if let Some(result) = account.moderation_results.drain(..).last() {
                self.browser_chat.post_status = result;
            }
            for result in account.post_results.drain(..) {
                if result["request"].as_str().is_some_and(|s|s.starts_with("automation-")){
                    self.automation_queue.log(format!("{} automation chat: {}",account.platform.name(),if result["ok"].as_bool()==Some(true){"sent"}else{result["text"].as_str().unwrap_or("failed")}));
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
        if self.chat_alert_revision != self.chat.revision() {
            self.chat_alert_revision = self.chat.revision();
            let events: Vec<_> = self
                .chat
                .messages()
                .flat_map(|m| self.chat_session.messages(m))
                .collect();
            for event in events {
                if event.kind == EventKind::ChatMessage {
                    self.captures
                        .widgets
                        .chat(event.platform, &event.author, &event.text);
                }
                self.dispatch_automations(&event, false, false);
            }
        }
        self.audio
            .monitor_device(&self.project.preferences.monitor_device);
        if self.event_revision != self.chat.revision() {
            for m in self.chat.messages().filter(|m| m.id.starts_with("event-")) {
                if !self.event_ids.contains(&m.id) {
                    self.event_ids.insert(m.id.clone());
                    self.browser_chat.relay(m);
                    self.event_relayed.push_back(m.id.clone());
                }
            }
            while self.event_relayed.len() > 2048 {
                if let Some(id) = self.event_relayed.pop_front() {
                    self.event_ids.remove(&id);
                }
            }
            self.event_revision = self.chat.revision();
        }
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
        self.captures.sync_browsers(&self.project);
        self.captures.sync_videos(&self.project);
        self.audio.media_bus(self.captures.videos.bus.clone());
        let scene = self.project.canvases[self.canvas].scene;
        if self.audio_scene != Some(scene) {
            self.audio.scene(&self.project, scene);
            self.audio_scene = Some(scene);
        }
        self.captures.sync_cameras(&self.project);
        self.captures.tick();
        let mut resized = false;
        for source in &self.project.sources {
            if matches!(source.kind, SourceKind::VideoFile(_))
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
        if preview_active && self.gpu.as_ref().is_some_and(|g| g.animating()) {
            self.preview_size = [0, 0];
            self.vertical_size = [0, 0];
            ctx.request_repaint_after(Duration::from_millis(16));
        }
        let accent = self.accent();
        ctx.style_mut(|s| {
            s.visuals.selection.bg_fill = accent;
            s.visuals.selection.stroke.color = Color32::from_rgb(56, 30, 114);
        });
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
        self.header(ctx);
        self.footer(ctx);
        match self.page {
            Page::Studio => self.main_studio(ctx),
            Page::Settings => self.settings(ctx),
            Page::Alerts => self.alerts(ctx),
        }
        self.panel_windows(ctx);
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
        if self.restart_profile.take().is_some()
            && let Ok(exe) = std::env::current_exe()
        {
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
        let duration = if self.0.project.preferences.workspace.animations
            && !ctx.input(|i| i.pointer.any_down())
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
        if !self.0.advanced() {
            return;
        }
        if response.dragged() && *tab != Panel::Preview {
            self.0.dragging_panel = Some(*tab);
        }
        if response.double_clicked() {
            self.0.toggle_panel = Some(*tab);
        }
        response.clone().on_hover_cursor(egui::CursorIcon::Grab).on_hover_text(
            "Drag toward an edge to snap. Drop in the center to group tabs. Drag past the window edge for its own window. Double-click to float or dock.",
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
            scene.layers.push(Layer { crop: [0.;4], group: 0,
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

include!("layer_properties.rs");

include!("connections.rs");

include!("appearance_settings.rs");

include!("transitions.rs");

include!("go_live.rs");

include!("first_run.rs");

include!("utility_windows.rs");
include!("loading.rs");
