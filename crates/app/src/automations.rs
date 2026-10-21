//! Bounded non-blocking scheduler and native rule editor. No script execution.
use auroracast_core::{
    Action, EventKind, Id, Platform, Project, Rule, SourceKind,
    automation::{Options, Permission},
};
use eframe::egui::{self, RichText};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum Mode {
    #[default]
    Queue,
    Overlap,
    Interrupt,
}
struct Job {
    name: String,
    actions: VecDeque<Action>,
    next: Instant,
    hold: Instant,
    media: Vec<(Id, Instant)>,
}
#[derive(Default)]
pub struct Scheduler {
    pub paused: bool,
    pub mode: Mode,
    jobs: VecDeque<Job>,
    pub history: VecDeque<String>,
}
impl Scheduler {
    pub fn log(&mut self, s: String) {
        let time = gst::glib::DateTime::now_local()
            .ok()
            .and_then(|d| d.format("%H:%M:%S").ok())
            .map(|s| s.to_string())
            .unwrap_or_default();
        self.history.push_back(format!("{time} · {s}"));
        while self.history.len() > 200 {
            self.history.pop_front();
        }
    }
    pub fn clear(&mut self) {
        self.jobs.clear();
        self.log("Queue cleared".into());
    }
    pub fn skip(&mut self) -> Vec<Id> {
        let media = self
            .jobs
            .pop_front()
            .map(|j| j.media.into_iter().map(|(id, _)| id).collect())
            .unwrap_or_default();
        self.log("Current automation skipped".into());
        media
    }
    pub fn enqueue(&mut self, name: String, actions: Vec<Action>) {
        if actions.is_empty() {
            return;
        }
        if self.jobs.len() >= 100 {
            self.log(format!("{name}: skipped · queue is full"));
            return;
        }
        if self.mode == Mode::Interrupt {
            self.jobs.clear();
        }
        let now = Instant::now();
        self.log(format!("{name}: queued"));
        self.jobs.push_back(Job {
            name,
            actions: actions.into(),
            next: now,
            hold: now,
            media: Vec::new(),
        });
    }
    pub fn len(&self) -> usize {
        self.jobs.len()
    }
    #[cfg(test)]
    pub fn tick(&mut self, p: &Project) -> Vec<Action> {
        self.tick_media(p, |_| (false, None))
    }
    pub fn tick_media(
        &mut self,
        _p: &Project,
        status: impl Fn(Id) -> (bool, Option<String>),
    ) -> Vec<Action> {
        let mut out = Vec::new();
        if self.paused {
            return out;
        }
        let now = Instant::now();
        let count = if self.mode == Mode::Overlap {
            self.jobs.len()
        } else {
            self.jobs.len().min(1)
        };
        for i in 0..count {
            let job = &mut self.jobs[i];
            if now < job.next {
                continue;
            }
            for _ in 0..64 {
                let Some(action) = job.actions.pop_front() else {
                    break;
                };
                match action {
                    Action::Wait { milliseconds } => {
                        job.next = now + Duration::from_millis(milliseconds.min(3600000));
                        job.hold = job.hold.max(job.next);
                        break;
                    }
                    Action::DisplayAlert { seconds,media,.. }=>{job.hold=job.hold.max(now+Duration::from_secs(seconds as u64));if let Some(id)=media{job.media.push((id,now));}},
                    Action::ShowAlert { seconds, .. } => {
                        job.hold = job.hold.max(now + Duration::from_secs(seconds as u64))
                    }
                    Action::PlayMedia { source } | Action::PlaySound { source } => {
                        job.media.push((source, now));
                    }
                    _ => (),
                }
                out.push(action);
            }
        }
        let mut finished = Vec::new();
        for job in &mut self.jobs {
            job.media.retain(|(id, at)| {
                if at.elapsed() < Duration::from_millis(250) {
                    return true;
                }
                let (running, error) = status(*id);
                if let Some(error) = error {
                    finished.push(format!("{}: media failed · {error}", job.name));
                }
                running
            });
        }
        self.jobs.retain(|j| {
            let keep = !j.actions.is_empty() || now < j.hold || !j.media.is_empty();
            if !keep {
                finished.push(format!("{}: action sequence completed", j.name));
            }
            keep
        });
        for s in finished {
            self.log(s);
        }
        out
    }
}
#[derive(Clone)]
pub enum Command {
    Rewards,
    Test(Id),
    /// Test with made-up details: conditions are checked first, then a preview plays if they match.
    TestWith(Id, TestEvent),
    Run(Id),
    Preview(Id),
    Stop,
    Skip,
    Clear,
    /// Make another alert box and show this rule's step (rule, step index) in it.
    NewAlertBox(Id, usize),
    /// Read a sample line aloud with the chosen voice.
    SpeechTest,
    /// Stop the line being read and drop the ones waiting.
    SpeechSkip,
}
#[derive(Clone, Debug)]
pub struct TestEvent {
    pub platform: Platform,
    pub author: String,
    pub amount: f64,
    pub count: u64,
    pub message: String,
}
impl Default for TestEvent {
    fn default() -> Self {
        Self { platform: Platform::Twitch, author: "TestViewer".into(), amount: 5., count: 10, message: "Hello!".into() }
    }
}
/// Events viewers cause that usually show something on stream; everything else is an automation.
pub fn is_alert(e: EventKind) -> bool {
    matches!(
        e,
        EventKind::Donation
            | EventKind::Follow
            | EventKind::Subscription
            | EventKind::Gift
            | EventKind::Bits
            | EventKind::Raid
            | EventKind::Membership
            | EventKind::SuperChat
            | EventKind::SuperSticker
            | EventKind::Subscriber
            | EventKind::HypeTrain
    )
}
/// The list groups alerts and automations by what starts them.
const GROUPS: [(&str, &[EventKind]); 8] = [
    ("Follows", &[EventKind::Follow, EventKind::Subscriber]),
    ("Subs & memberships", &[EventKind::Subscription, EventKind::Gift, EventKind::Membership]),
    ("Bits, donations & Super Chats", &[EventKind::Bits, EventKind::Donation, EventKind::SuperChat, EventKind::SuperSticker]),
    ("Raids & Hype Trains", &[EventKind::Raid, EventKind::HypeTrain]),
    ("Channel points", &[EventKind::Redemption]),
    ("Chat", &[EventKind::ChatCommand, EventKind::ChatMessage, EventKind::FirstMessage, EventKind::FirstTimeChatter]),
    ("Polls & predictions", &[EventKind::Poll, EventKind::Prediction]),
    ("Timers", &[EventKind::Timer]),
];
#[derive(Default)]
pub struct Editor {
    pub selected: usize,
    pub guide: crate::alert_guide::Guide,
    undo: Option<(usize, Rule)>,
    transfer: Option<(bool, String)>,
    file_job: Option<mpsc::Receiver<Result<Option<String>, String>>>,
    pub notice: String,
    /// A file being chosen for a Play sound / Play media step: (rule, step).
    picker: Option<((Id, usize), mpsc::Receiver<Option<PathBuf>>)>,
    pub preview_open: bool,
    pub preview_vertical: bool,
    /// Platforms with a bot account connected, for Send chat steps.
    pub bots: Vec<Platform>,
    /// The selected rule's event, for the variables its steps can use.
    event: EventKind,
    live_preview: bool,
    live_seen: Vec<u8>,
    live_due: Option<Instant>,
    tester: Option<(Id, TestEvent)>,
    samples: Vec<crate::alert_samples::Sample>,
    samples_job: Option<mpsc::Receiver<Result<Vec<crate::alert_samples::Sample>, String>>>,
    /// What reading aloud is doing (a download, an error), and how many lines wait.
    pub speech_status: String,
}
fn rule_menu(ui: &mut egui::Ui, enabled: bool) -> Option<&'static str> {
    let mut result = None;
    for label in [
        if enabled { "Turn off" } else { "Turn on" },
        "Test",
        "Duplicate",
        "Move up",
        "Move down",
        "Export",
        "Delete",
    ] {
        if ui.button(label).clicked() {
            result = Some(label);
            ui.close();
        }
    }
    result
}
/// The variables an event fills in; others would stay empty.
fn variables_for(e: EventKind) -> &'static [&'static str] {
    match e {
        EventKind::Follow | EventKind::Subscriber | EventKind::FirstMessage | EventKind::FirstTimeChatter => &["{user}", "{platform}"],
        EventKind::Subscription | EventKind::Membership => &["{user}", "{message}", "{count}", "{platform}"],
        EventKind::Gift => &["{user}", "{count}", "{platform}"],
        EventKind::Bits => &["{user}", "{bits}", "{message}", "{platform}"],
        EventKind::Donation | EventKind::SuperChat | EventKind::SuperSticker => &["{user}", "{amount}", "{currency}", "{message}", "{platform}"],
        EventKind::Raid => &["{user}", "{viewers}", "{platform}"],
        EventKind::Redemption => &["{user}", "{reward}", "{message}", "{platform}"],
        EventKind::ChatCommand | EventKind::ChatMessage => &["{user}", "{message}", "{platform}"],
        EventKind::HypeTrain | EventKind::Poll | EventKind::Prediction => &["{count}", "{platform}"],
        EventKind::Timer => &[],
    }
}
fn variables(ui: &mut egui::Ui, text: &mut String, event: EventKind) {
    if variables_for(event).is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.small("Insert:");
        for &v in variables_for(event) {
            if ui.small_button(v).clicked() {
                text.push_str(v);
            }
        }
    });
}
/// The "When ..." half of the plain-language summary.
fn when_text(e: EventKind) -> &'static str {
    match e {
        EventKind::Follow => "someone follows",
        EventKind::Subscription => "someone subscribes",
        EventKind::Gift => "someone sends a gift",
        EventKind::Bits => "someone cheers bits",
        EventKind::Raid => "someone raids",
        EventKind::Donation => "someone donates",
        EventKind::Membership => "someone becomes a member",
        EventKind::SuperChat => "someone sends a Super Chat",
        EventKind::SuperSticker => "someone sends a Super Sticker",
        EventKind::Subscriber => "someone subscribes on YouTube",
        EventKind::HypeTrain => "a Hype Train starts",
        EventKind::ChatCommand => "someone types a command",
        EventKind::ChatMessage => "a chat message matches",
        EventKind::FirstMessage => "someone sends their first message this stream",
        EventKind::FirstTimeChatter => "a brand-new chatter speaks",
        EventKind::Redemption => "someone redeems a reward",
        EventKind::Poll => "a poll updates",
        EventKind::Prediction => "a prediction updates",
        EventKind::Timer => "a timer goes off",
    }
}
fn action_label(a: &Action) -> &'static str {
    match a {
        Action::DisplayAlert { .. } => "Show alert",
        Action::ShowAlert { .. } => "Show text",
        Action::PlaySound { .. } => "Play sound",
        Action::PlayMedia { .. } => "Play media",
        Action::SendChat { .. } => "Send chat message",
        Action::Wait { .. } => "Pause",
        Action::UpdateWidget { .. } => "Update widget",
        Action::SwitchScene { .. } => "Switch scene",
    }
}
fn short(text: &str) -> String {
    let t: String = text.chars().take(36).collect();
    if text.chars().count() > 36 { format!("{t}…") } else { t }
}
/// The "... then" half of the summary, e.g. `show "Thanks, {user}!" for 6 s, then play Chime at 80%`.
fn then_text(actions: &[Action], sources: &[auroracast_core::Source], scenes: &[auroracast_core::Scene]) -> String {
    // Media steps name the file they play (a source's name can be left over from an earlier file).
    let name = |id: Id| {
        sources
            .iter()
            .find(|s| s.id == id)
            .map(|s| match &s.kind {
                SourceKind::VideoFile(v) if !s.name.ends_with("(built-in)") => std::path::Path::new(&v.path)
                    .file_name()
                    .map_or_else(|| s.name.clone(), |n| n.to_string_lossy().into_owned()),
                _ => s.name.clone(),
            })
            .unwrap_or_else(|| if id == 0 { "no file chosen".into() } else { "missing media".into() })
    };
    let volume = |id: Id| match sources.iter().find(|s| s.id == id).map(|s| &s.kind) {
        Some(SourceKind::VideoFile(v)) => format!(" at {}%", (v.gain * 100.).round()),
        _ => String::new(),
    };
    let parts: Vec<String> = actions
        .iter()
        .map(|a| match a {
            Action::DisplayAlert { template, seconds, media, .. } => format!(
                "show \u{201c}{}\u{201d}{} for {seconds} s",
                short(template),
                media.map(|m| format!(" with {}", name(m))).unwrap_or_default()
            ),
            Action::ShowAlert { template, seconds } => format!("show \u{201c}{}\u{201d} for {seconds} s", short(template)),
            Action::PlaySound { source } => format!("play {}{}", name(*source), volume(*source)),
            Action::PlayMedia { source } => format!("play {}{}", name(*source), volume(*source)),
            Action::SendChat { text, .. } => format!("say \u{201c}{}\u{201d} in chat", short(text)),
            Action::Wait { milliseconds } => format!("pause {:.1} s", *milliseconds as f64 / 1000.),
            Action::UpdateWidget { source, .. } => format!("update {}", name(*source)),
            Action::SwitchScene { scene, .. } => format!(
                "switch to {}",
                scenes.iter().find(|s| s.id == *scene).map(|s| s.name.as_str()).unwrap_or("a missing scene")
            ),
        })
        .collect();
    if parts.is_empty() { "do nothing yet".into() } else { parts.join(", then ") }
}
fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}
fn ago(ms: u64) -> String {
    match ms / 1000 {
        s if s < 60 => "played just now".into(),
        s if s < 3600 => format!("played {} min ago", s / 60),
        s => format!("played {} h ago", s / 3600),
    }
}
fn mode_label(m: Mode) -> &'static str {
    match m {
        Mode::Queue => "One at a time",
        Mode::Overlap => "Play together",
        Mode::Interrupt => "Newest replaces current",
    }
}
/// Reading paid messages aloud: its settings, in a small menu beside the alert queue.
fn speech_menu(ui: &mut egui::Ui, s: &mut auroracast_core::Speech, status: &str, cmds: &mut Vec<Command>) -> bool {
    let mut changed = false;
    ui.menu_button(if s.enabled { "Read aloud · on" } else { "Read aloud · off" }, |ui| {
        ui.set_min_width(320.);
        changed |= ui.checkbox(&mut s.enabled, "Read paid messages aloud").changed();
        ui.small("Super Chats, donations, cheers and Kicks. The voice runs on this computer; it's downloaded the first time (about 90 MB).");
        ui.horizontal(|ui| {
            ui.label("Voice");
            egui::ComboBox::from_id_salt("speech_voice").selected_text(crate::speech::voice(&s.voice).label).show_ui(ui, |ui| {
                for v in &crate::speech::VOICES {
                    changed |= ui.selectable_value(&mut s.voice, v.id.to_string(), v.label).changed();
                }
            });
        });
        changed |= ui.checkbox(&mut s.read_names, "Say who sent it").changed();
        changed |= ui.add(egui::Slider::new(&mut s.max_chars, 40..=400).text("characters at most")).changed();
        ui.label("Don't read messages with these words (separate with commas):");
        changed |= ui.add(egui::TextEdit::multiline(&mut s.blocked).desired_rows(2).desired_width(f32::INFINITY)).changed();
        ui.horizontal(|ui| {
            if ui.add_enabled(s.enabled, egui::Button::new("Test")).clicked() {
                cmds.push(Command::SpeechTest);
            }
            if ui.button("Skip / stop").on_hover_text("Stops what's being read and drops the rest").clicked() {
                cmds.push(Command::SpeechSkip);
            }
        });
        if !status.is_empty() {
            ui.small(status);
        }
        ui.small("Set its volume with the \"Read aloud\" strip in the mixer.");
    });
    changed
}
/// The Alerts page keeps the queue controls in one small menu that shows what's playing.
pub fn queue_menu(ui: &mut egui::Ui, q: &mut Scheduler, cmds: &mut Vec<Command>) {
    let label = match (q.paused, q.len()) {
        (true, _) => "Alert queue · paused".to_owned(),
        (false, 0) => "Alert queue · idle".to_owned(),
        (false, 1) => "Alert queue · 1 playing".to_owned(),
        (false, n) => format!("Alert queue · {} waiting", n - 1),
    };
    ui.menu_button(label, |ui| {
        ui.set_min_width(420.);
        queue_bar(ui, q, cmds);
    })
    .response
    .on_hover_text("Pause, skip or stop alerts, and choose whether they wait for each other");
}
/// Compact queue controls, shared by the Alerts page and the Broadcast panel.
pub fn queue_bar(ui: &mut egui::Ui, q: &mut Scheduler, cmds: &mut Vec<Command>) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Alert queue").strong());
        if q.paused {
            ui.label(RichText::new("Paused").color(egui::Color32::from_rgb(230, 190, 90)));
        }
        ui.label(match q.len() {
            0 => "idle".to_owned(),
            1 => "1 playing".to_owned(),
            n => format!("1 playing · {} waiting", n - 1),
        });
        if ui.button(if q.paused { "Resume" } else { "Pause" }).on_hover_text("Pause holds new alerts until you resume").clicked() {
            q.paused = !q.paused;
        }
        if ui.add_enabled(q.len() > 0, egui::Button::new("Skip")).on_hover_text("Skip the alert that's playing").clicked() {
            cmds.push(Command::Skip);
        }
        if ui.add_enabled(q.len() > 1, egui::Button::new("Clear")).on_hover_text("Remove alerts that are waiting").clicked() {
            cmds.push(Command::Clear);
        }
        if ui.button(RichText::new("Stop all").color(egui::Color32::LIGHT_RED)).on_hover_text("Stop everything now and pause").clicked() {
            cmds.push(Command::Stop);
        }
        egui::ComboBox::from_id_salt("queue_mode")
            .selected_text(mode_label(q.mode))
            .show_ui(ui, |ui| {
                for m in [Mode::Queue, Mode::Overlap, Mode::Interrupt] {
                    ui.selectable_value(&mut q.mode, m, mode_label(m));
                }
            })
            .response
            .on_hover_text("One at a time waits for each alert to finish. Play together overlaps them. Newest replaces current cuts the playing alert short.");
    });
}
const ALERT_TEMPLATES: [(&str, EventKind, &str); 8] = [
    ("New follower", EventKind::Follow, "Thanks for the follow, {user}!"),
    ("New subscriber", EventKind::Subscription, "{user} just subscribed!"),
    ("Gift", EventKind::Gift, "{user} sent {count} gifts!"),
    ("Bits / cheer", EventKind::Bits, "{user} cheered {bits} bits!"),
    ("Raid", EventKind::Raid, "Welcome {user} and {viewers} raiders!"),
    ("Donation", EventKind::Donation, "Thank you {user} for {amount} {currency}!"),
    ("Super Chat", EventKind::SuperChat, "{user} sent {amount} {currency}: {message}"),
    ("New member", EventKind::Membership, "Welcome to the club, {user}!"),
];
const AUTOMATION_TEMPLATES: [(&str, EventKind, &str); 5] = [
    ("Chat command", EventKind::ChatCommand, "Hello, {user}!"),
    ("Chat timer", EventKind::Timer, "Enjoying the stream? Follow so you don't miss the next one!"),
    ("Welcome first-time chatters", EventKind::FirstMessage, "Welcome, {user}!"),
    ("Channel points reward", EventKind::Redemption, "{user} redeemed a reward!"),
    ("Death counter", EventKind::ChatCommand, "Death counter"),
];
/// A ready-to-use rule: alerts show in the shared Alerts box when one exists.
fn template_rule(p: &mut Project, label: &str, event: EventKind, text: &str) -> Rule {
    let id = p.id();
    let alert_box = p
        .sources
        .iter()
        .find(|s| matches!(&s.kind, SourceKind::Widget(w) if w.kind == auroracast_core::widgets::Kind::AlertBox))
        .map(|s| s.id);
    let widget = p.sources.iter().find(|s| matches!(s.kind, SourceKind::Widget(_))).map(|s| s.id);
    let actions = match (label, event) {
        ("Death counter", _) => match widget {
            Some(source) => vec![Action::UpdateWidget { source, text: String::new(), increment: 1, start_timer: false }],
            None => vec![Action::ShowAlert { template: text.into(), seconds: 5 }],
        },
        (_, EventKind::ChatCommand | EventKind::FirstMessage | EventKind::Timer) => vec![Action::SendChat { text: text.into(), platforms: vec![], sender: Default::default() }],
        _ => match alert_box {
            Some(source) if is_alert(event) => vec![Action::DisplayAlert { source, template: text.into(), media: None, seconds: 6, subject: Default::default() }],
            _ => vec![Action::ShowAlert { template: text.into(), seconds: 5 }],
        },
    };
    Rule {
        id,
        name: label.into(),
        enabled: true,
        platform: None,
        event,
        command: if label == "Death counter" { "!death" } else { "!hello" }.into(),
        cooldown_ms: 3000,
        conditions: Default::default(),
        options: Options { timer_minutes: 15, timer_messages: 5, ..Default::default() },
        actions,
    }
}
/// Adds a built-in sample as an automation-only media source with its own mixer channel.
fn add_sample_source(p: &mut Project, sample: &crate::alert_samples::Sample, layer: bool) -> Id {
    if let Some(existing) = p.sources.iter().find(|s| matches!(&s.kind, SourceKind::VideoFile(v) if v.path == sample.path.to_string_lossy())) {
        return existing.id;
    }
    let id = new_media_source(p, &sample.path.to_string_lossy(), format!("{} (built-in)", sample.name));
    add_layers(p, id, sample.sound, layer);
    id
}
/// Media for a file chosen in a Play sound / Play media step: automation-only, reusing one
/// already added for the same file.
fn add_media_file(p: &mut Project, path: &str, name: String, sound: bool) -> Id {
    if let Some(existing) = p.sources.iter().find(|s| matches!(&s.kind, SourceKind::VideoFile(v) if v.automation_only && v.path == path)) {
        return existing.id;
    }
    let id = new_media_source(p, path, name);
    add_layers(p, id, sound, false);
    id
}
fn new_media_source(p: &mut Project, path: &str, name: String) -> Id {
    let id = p.id();
    p.sources.push(auroracast_core::Source {
        id,
        name: name.clone(),
        kind: SourceKind::VideoFile(auroracast_core::VideoSettings {
            path: path.into(),
            audio: true,
            looped: false,
            automation_only: true,
            ..Default::default()
        }),
    });
    p.add_media_audio(id, name).monitor = true;
    if p.preferences.monitor_device.is_empty() {
        p.preferences.monitor_device = crate::audio::DEFAULT_MONITOR.into();
    }
    id
}
fn add_layers(p: &mut Project, id: Id, sound: bool, layer: bool) {
    // Sounds need a hidden layer to play; animations shown by Play media need a visible one.
    let rect = if sound { [0., 0., 0.001, 0.001] } else { [0.3, 0.2, 0.4, 0.6] };
    if sound || layer {
        let scenes: Vec<Id> = p.canvases.iter().map(|c| c.scene).collect();
        for scene in p.scenes.iter_mut().filter(|s| scenes.contains(&s.id)) {
            scene.layers.push(auroracast_core::Layer { frame: Default::default(), look: Default::default(), source: id, rect, visible: true, locked: sound, opacity: if sound { 0. } else { 1. }, crop: [0.; 4], group: 0 });
        }
    }
}
#[derive(serde::Serialize, serde::Deserialize)]
struct Bundle {
    version: u32,
    rules: Vec<Rule>,
    sources: std::collections::BTreeMap<Id, String>,
    scenes: std::collections::BTreeMap<Id, String>,
    canvases: std::collections::BTreeMap<Id, String>,
}
fn export(p: &Project, rule: &Rule) -> String {
    serde_json::to_string_pretty(&Bundle {
        version: 1,
        rules: vec![rule.clone()],
        sources: p.sources.iter().map(|s| (s.id, s.name.clone())).collect(),
        scenes: p.scenes.iter().map(|s| (s.id, s.name.clone())).collect(),
        canvases: p.canvases.iter().map(|s| (s.id, s.name.clone())).collect(),
    })
    .unwrap_or_default()
}
fn import(p: &mut Project, text: &str) -> Result<(), String> {
    if text.len() > 1024 * 1024 {
        return Err("Import is too large".into());
    }
    let mut b: Bundle = serde_json::from_str(text).map_err(|_| "Invalid automation file")?;
    if b.version != 1 || b.rules.len() > 100 {
        return Err("Unsupported automation bundle".into());
    }
    let map = |id: Id,
               names: &std::collections::BTreeMap<Id, String>,
               targets: Vec<(Id, String)>|
     -> Result<Id, String> {
        let name = names.get(&id).ok_or("Missing source reference")?;
        let ids: Vec<_> = targets.iter().filter(|(_, n)| n == name).collect();
        if ids.len() != 1 {
            return Err(format!(
                "Create or uniquely name '{name}' in this project before importing"
            ));
        }
        Ok(ids[0].0)
    };
    for r in &mut b.rules {
        r.enabled = false;
        r.options.hotkey = 0;
        if let Some(id) = r.options.scene {
            r.options.scene = Some(map(
                id,
                &b.scenes,
                p.scenes.iter().map(|s| (s.id, s.name.clone())).collect(),
            )?);
        }
        if r.actions.len() > 64 || r.cooldown_ms > 3600000 || r.options.per_user_ms > 3600000 {
            return Err("Automation limits exceeded".into());
        }
        for a in &mut r.actions {
            match a {
                Action::PlayMedia { source } | Action::PlaySound { source } | Action::UpdateWidget { source, .. } => {
                    *source = map(
                        *source,
                        &b.sources,
                        p.sources.iter().map(|s| (s.id, s.name.clone())).collect(),
                    )?
                }
                Action::DisplayAlert {source,media,..} => {
                    let targets=p.sources.iter().map(|s|(s.id,s.name.clone())).collect::<Vec<_>>();
                    *source=map(*source,&b.sources,targets.clone())?;
                    if let Some(id)=media {*id=map(*id,&b.sources,targets)?;}
                }
                Action::SwitchScene { canvas, scene } => {
                    *canvas = map(
                        *canvas,
                        &b.canvases,
                        p.canvases.iter().map(|s| (s.id, s.name.clone())).collect(),
                    )?;
                    *scene = map(
                        *scene,
                        &b.scenes,
                        p.scenes.iter().map(|s| (s.id, s.name.clone())).collect(),
                    )?;
                }
                Action::ShowAlert { seconds, .. } if !(1..=60).contains(seconds) => {
                    return Err("Invalid duration".into());
                }
                Action::Wait { milliseconds } if *milliseconds > 3600000 => {
                    return Err("Wait exceeds one hour".into());
                }
                _ => (),
            }
        }
    }
    let mut candidate = p.clone();
    for mut r in b.rules {
        r.id = candidate.id();
        candidate.rules.push(r);
    }
    candidate.validate().map_err(|e| e.to_string())?;
    *p = candidate;
    Ok(())
}
impl Editor {
    pub fn for_page(page:&str)->Self {let mut editor=Self::default();if page=="automation-preview"{editor.preview_open=true;}if page.starts_with("alert-guide"){editor.guide=crate::alert_guide::Guide::design_preview(if page.ends_with("media"){1}else if page.ends_with("finish"){2}else{0});}editor}

    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        p: &mut Project,
        q: &mut Scheduler,
        engine: &auroracast_core::AlertEngine,
        connected: &[Platform],
    ) -> (bool, Vec<Command>) {
        if let Some(rx) = &self.file_job
            && let Ok(result) = rx.try_recv()
        {
            match result {
                Ok(Some(text)) => self.transfer = Some((true, text)),
                Ok(None) => self.notice = "Automation file saved.".into(),
                Err(e) => self.notice = e,
            }
            self.file_job = None;
        }
        if let Some(rx) = &self.samples_job
            && let Ok(result) = rx.try_recv()
        {
            match result {
                Ok(samples) => self.samples = samples,
                Err(e) => self.notice = e,
            }
            self.samples_job = None;
        }
        let before = serde_json::to_vec(&p.rules).unwrap_or_default();
        let mut changed = false;
        let mut cmds = Vec::new();
        if let Some(((rule_id, step), rx)) = &self.picker
            && let Ok(path) = rx.try_recv()
        {
            let (rule_id, step) = (*rule_id, *step);
            self.picker = None;
            if let Some(path) = path
                && let Some(r) = p.rules.iter().position(|r| r.id == rule_id)
            {
                // The step gets its own media for the chosen file, so scene layers using other media stay as they are.
                let sound = matches!(p.rules[r].actions.get(step), Some(Action::PlaySound { .. }));
                let name = path.file_name().map_or_else(|| "Media".into(), |n| n.to_string_lossy().into_owned());
                let id = add_media_file(p, &path.to_string_lossy(), name, sound);
                if let Some(Action::PlaySound { source } | Action::PlayMedia { source }) = p.rules[r].actions.get_mut(step) {
                    *source = id;
                    changed = true;
                }
            }
        }
        if self.selected >= p.rules.len() {
            self.selected = p.rules.len().saturating_sub(1);
        }
        egui::SidePanel::left("automation_list")
            .resizable(true)
            .default_width(270.)
            .min_width(200.)
            .max_width((ctx.content_rect().width() * 0.35).clamp(200., 360.))
            .show(ctx, |ui| {
                crate::appearance::glass(ui, egui::CornerRadius::ZERO);
                changed |= self.sidebar(ui, p, engine, &mut cmds)
            });
        egui::CentralPanel::default().show(ctx, |ui| {
            crate::appearance::glass(ui, egui::CornerRadius::ZERO);
            egui::ScrollArea::vertical().show(ui, |ui| {
                changed |= self.editor(ui, p, q, &mut cmds);
                ui.add_space(10.);
                ui.collapsing("Recent activity", |ui| {
                    for line in q.history.iter().rev().take(25) {
                        ui.small(line);
                    }
                    ui.collapsing("Why alerts played or were skipped", |ui| {
                        for line in engine.history.iter().rev().take(50) {
                            ui.small(line.split_once(" · ").map(|(_, rest)| rest).unwrap_or(line));
                        }
                    });
                });
                if !self.notice.is_empty() {
                    ui.label(&self.notice);
                }
            });
        });
        // Live preview replays the test shortly after the alert stops changing.
        if self.live_preview && let Some(r) = p.rules.get(self.selected) {
            let now = serde_json::to_vec(r).unwrap_or_default();
            if now != self.live_seen {
                self.live_seen = now;
                self.live_due = Some(Instant::now() + Duration::from_millis(700));
            }
            if let Some(due) = self.live_due {
                if Instant::now() >= due {
                    cmds.push(Command::Test(r.id));
                    self.live_due = None;
                } else {
                    ctx.request_repaint_after(due - Instant::now());
                }
            }
        }
        if let Some((id, t)) = &mut self.tester {
            let mut open = true;
            let mut send = false;
            egui::Window::new("Test with details")
                .open(&mut open)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .default_width(360.)
                .collapsible(false)
                .show(ctx, |ui| {
                    egui::Grid::new("test_event").num_columns(2).spacing([10., 8.]).show(ui, |ui| {
                        ui.label("Platform");
                        egui::ComboBox::from_id_salt("test_platform").selected_text(t.platform.name()).show_ui(ui, |ui| {
                            for p in Platform::ALL {
                                ui.selectable_value(&mut t.platform, p, p.name());
                            }
                        });
                        ui.end_row();
                        ui.label("Viewer name");
                        ui.add(egui::TextEdit::singleline(&mut t.author).char_limit(60));
                        ui.end_row();
                        ui.label("Amount");
                        ui.add(egui::DragValue::new(&mut t.amount).speed(0.5).range(0.0..=100000.0).fixed_decimals(2));
                        ui.end_row();
                        ui.label("Count (bits, gifts, raiders)");
                        ui.add(egui::DragValue::new(&mut t.count).range(0..=1000000u64));
                        ui.end_row();
                        ui.label("Message");
                        ui.add(egui::TextEdit::singleline(&mut t.message).char_limit(300));
                        ui.end_row();
                    });
                    ui.small("Checks this alert's conditions with these details, then plays a preview if they match. Goals, scenes and chat are not changed.");
                    send = ui.button("Send test event").clicked();
                });
            if send {
                cmds.push(Command::TestWith(*id, t.clone()));
            }
            if !open {
                self.tester = None;
            }
        }
        if let Some((is_import, text)) = &mut self.transfer {
            let mut close = false;
            egui::Window::new(if *is_import{"Import automation"}else{"Export automation"}).anchor(egui::Align2::CENTER_CENTER,egui::Vec2::ZERO).default_width(600.).show(ctx,|ui|{
  ui.label("Portable JSON · media is matched by source name; account credentials and media paths are excluded.");
  if ui.add_enabled(self.file_job.is_none(),egui::Button::new(if *is_import{"Open JSON file…"}else{"Save JSON file…"})).clicked(){
    let import=*is_import;let content=text.clone();let(tx,rx)=mpsc::channel();self.file_job=Some(rx);
    std::thread::spawn(move||{let result=(||->Result<Option<String>,String>{
        let path=crate::desktop::pick(if import{crate::desktop::Pick::Open}else{crate::desktop::Pick::Save},if import{"Open automation JSON"}else{"Save automation JSON"},"automation.json").map_err(|e|e.to_string())?.ok_or("File operation cancelled")?;
        let path=path.as_path();
        if import{if std::fs::metadata(path).map_err(|_|"Cannot read file")?.len()>1024*1024{return Err("File exceeds 1 MiB".into());}Ok(Some(std::fs::read_to_string(path).map_err(|_|"Cannot read JSON file")?))}else{std::fs::write(path,content).map_err(|_|"Could not save JSON file")?;Ok(None)}
    })();let _=tx.send(result);});
  }ui.add(egui::TextEdit::multiline(text).desired_rows(16).char_limit(1024*1024));
  if *is_import{if ui.button("Import disabled for review").clicked(){match import(p,text){Ok(())=>{self.notice="Imported. Review targets, then turn the automation on.".into();changed=true;close=true;},Err(e)=>self.notice=e}}}else if ui.button("Copy JSON").clicked(){ctx.copy_text(text.clone());}
  if ui.button("Close").clicked(){close=true;}
 });
            if close {
                self.transfer = None;
            }
        }
        changed |= before != serde_json::to_vec(&p.rules).unwrap_or_default();
        if self.guide.open {
            self.request_samples();
            self.guide.samples = self.samples.clone();
        }
        if let Some((id,test))=self.guide.ui(ctx,p,connected){changed=true;self.selected=p.rules.iter().position(|r|r.id==id).unwrap_or(0);if test{cmds.push(Command::Test(id));}}
        (changed, cmds)
    }
    /// Starts generating the built-in samples once, in the background.
    fn request_samples(&mut self) {
        if self.samples.is_empty() && self.samples_job.is_none() {
            let (tx, rx) = mpsc::channel();
            self.samples_job = Some(rx);
            std::thread::spawn(move || {
                let _ = tx.send(crate::alert_samples::ensure_all().map_err(|e| format!("Built-in samples unavailable: {e:#}")));
            });
        }
    }
    /// "Built-in…" menu; returns the chosen sample.
    fn sample_menu(&mut self, ui: &mut egui::Ui, sounds: bool) -> Option<crate::alert_samples::Sample> {
        let mut choice = None;
        ui.menu_button("Built-in…", |ui| {
            self.request_samples();
            let list: Vec<_> = self.samples.iter().filter(|s| s.sound == sounds).cloned().collect();
            if list.is_empty() {
                ui.label(if self.samples_job.is_some() { "Preparing samples…" } else if sounds { "No built-in sounds" } else { "Built-in animations need FFmpeg" });
            }
            for s in list {
                if ui.button(s.name).clicked() {
                    choice = Some(s);
                    ui.close();
                }
            }
        });
        choice
    }
    fn sidebar(&mut self, ui: &mut egui::Ui, p: &mut Project, engine: &auroracast_core::AlertEngine, cmds: &mut Vec<Command>) -> bool {
        let mut changed = false;
        ui.heading("Alerts & automations");
        ui.horizontal(|ui| {
            ui.menu_button("+ New", |ui| {
                ui.label(RichText::new("Alerts").small().weak());
                for &(label, event, text) in ALERT_TEMPLATES.iter() {
                    if ui.button(label).clicked() {
                        let rule = template_rule(p, label, event, text);
                        p.rules.push(rule);
                        self.selected = p.rules.len() - 1;
                        ui.close();
                    }
                }
                ui.separator();
                ui.label(RichText::new("Chat, rewards & timers").small().weak());
                for &(label, event, text) in AUTOMATION_TEMPLATES.iter() {
                    if ui.button(label).clicked() {
                        let rule = template_rule(p, label, event, text);
                        p.rules.push(rule);
                        self.selected = p.rules.len() - 1;
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button("Guided setup…").on_hover_text("Step by step, with your own media and sound").clicked() {
                    self.guide.open = true;
                    ui.close();
                }
            });
            ui.menu_button("...", |ui| {
                if ui.button("Twitch channel point rewards…").clicked() {
                    cmds.push(Command::Rewards);
                    ui.close();
                }
                if ui.button("Import from file…").clicked() {
                    self.transfer = Some((true, String::new()));
                    ui.close();
                }
                ui.separator();
                changed |= ui.checkbox(&mut p.preferences.automation_history.show_missed, "Show events from while Rosorah was closed").changed();
                changed |= ui.checkbox(&mut p.preferences.automation_history.count_missed, "Count those donations toward goals").changed();
            });
        });
        ui.add_space(6.);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
        let mut op = None;
        egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
            for (group, events) in GROUPS {
                if !p.rules.iter().any(|r| events.contains(&r.event)) {
                    continue;
                }
                ui.add_space(4.);
                ui.label(RichText::new(group).small().strong().color(ui.visuals().weak_text_color()));
                for i in 0..p.rules.len() {
                    if !events.contains(&p.rules[i].event) {
                        continue;
                    }
                    let summary = then_text(&p.rules[i].actions, &p.sources, &p.scenes);
                    let r = &mut p.rules[i];
                    let selected = self.selected == i;
                    let fill = if selected { ui.visuals().selection.bg_fill.gamma_multiply(0.22) } else { ui.visuals().faint_bg_color };
                    let frame = egui::Frame::new().fill(fill).corner_radius(10).inner_margin(8).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut r.enabled, "").on_hover_text(if r.enabled { "On" } else { "Off" });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                                ui.menu_button("...", |ui| {
                                    if let Some(a) = rule_menu(ui, r.enabled) {
                                        op = Some((i, a));
                                    }
                                });
                                ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                                    let name = ui.add(egui::Label::new(RichText::new(&r.name).strong()).wrap().sense(egui::Sense::click()));
                                    if name.clicked() {
                                        self.selected = i;
                                    }
                                    name.context_menu(|ui| {
                                        if let Some(a) = rule_menu(ui, r.enabled) {
                                            op = Some((i, a));
                                        }
                                    });
                                });
                            });
                        });
                        ui.add(egui::Label::new(RichText::new(format!("{}.", capitalized(&summary))).small()).wrap());
                        let platform = if r.event == EventKind::Timer {
                            format!("Every {} min", r.options.timer_minutes.max(1))
                        } else {
                            r.platform.map(|p| p.name()).unwrap_or("All platforms").to_owned()
                        };
                        let last = engine.last_fired(r.id).map(|t| ago(now.saturating_sub(t))).unwrap_or_else(|| "not played yet".into());
                        ui.add(egui::Label::new(RichText::new(format!("{platform} · {last}")).small().weak()).truncate());
                    });
                    if frame.response.interact(egui::Sense::click()).clicked() {
                        self.selected = i;
                    }
                    ui.add_space(4.);
                }
            }
            if p.rules.is_empty() {
                ui.small("Nothing yet. Use + New to add an alert, a chat command or a timer.");
            }
        });
        if let Some((i, action)) = op {
            self.selected = i;
            match action {
                "Delete" => {
                    self.undo = Some((i, p.rules.remove(i)));
                    self.selected = i.saturating_sub(1);
                }
                "Duplicate" => {
                    let mut r = p.rules[i].clone();
                    r.id = p.id();
                    r.name.push_str(" copy");
                    r.enabled = false;
                    r.options.hotkey = 0;
                    p.rules.insert(i + 1, r);
                    self.selected = i + 1;
                }
                "Turn off" | "Turn on" => p.rules[i].enabled = !p.rules[i].enabled,
                "Move up" if i > 0 => {
                    p.rules.swap(i, i - 1);
                    self.selected -= 1;
                }
                "Move down" if i + 1 < p.rules.len() => {
                    p.rules.swap(i, i + 1);
                    self.selected += 1;
                }
                "Test" => cmds.push(Command::Test(p.rules[i].id)),
                "Export" => self.transfer = Some((false, export(p, &p.rules[i]))),
                _ => (),
            }
        }
        if self.undo.is_some() && ui.button("Undo delete").clicked() {
            let (i, r) = self.undo.take().unwrap();
            let index = i.min(p.rules.len());
            p.rules.insert(index, r);
            self.selected = index;
        }
        changed
    }
    /// The selected rule: summary sentence, simple conditions, then action cards.
    fn editor(&mut self, ui: &mut egui::Ui, p: &mut Project, q: &mut Scheduler, cmds: &mut Vec<Command>) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                queue_menu(ui, q, cmds);
                changed |= speech_menu(ui, &mut p.preferences.speech, &self.speech_status, cmds);
            });
        });
        let Some(index) = Some(self.selected).filter(|i| p.rules.get(*i).is_some()) else {
            ui.add_space(20.);
            ui.label("Add an alert, a chat command or a timer with + New to get started.");
            return false;
        };
        self.event = p.rules[index].event;
        let rule = &p.rules[index];
        let conflicts = if rule.event == EventKind::Timer { 0 } else { p.rules.iter().filter(|a| a.id != rule.id && a.enabled && a.event == rule.event && (a.platform.is_none() || rule.platform.is_none() || a.platform == rule.platform)).count() };
        let mut sample_choice: Option<(usize, crate::alert_samples::Sample)> = None;
        {
            let r = &mut p.rules[index];
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::TextEdit::singleline(&mut r.name).char_limit(100).desired_width(260.).font(egui::TextStyle::Heading));
                ui.checkbox(&mut r.enabled, "On");
                if ui.button("Test").on_hover_text("Plays a preview. Doesn't change goals, switch scenes or post chat.").clicked() {
                    cmds.push(Command::Test(r.id));
                }
                if ui.button("Run for real").on_hover_text("Runs every step for real, including scene switches and chat messages").clicked() {
                    cmds.push(Command::Run(r.id));
                }
                ui.menu_button("More tests", |ui| {
                    if ui.button("Test as a specific viewer or amount…").on_hover_text("Checks this alert's conditions with details you choose, and says why it wouldn't play").clicked() {
                        let message = if r.event == EventKind::ChatCommand { format!("{} test", r.command) } else { "Hello!".into() };
                        self.tester = Some((r.id, TestEvent { platform: r.platform.unwrap_or(Platform::Twitch), message, ..Default::default() }));
                        ui.close();
                    }
                    let mut live = self.live_preview;
                    if ui.checkbox(&mut live, "Replay the preview after each change").changed() {
                        self.live_preview = live;
                        if live {
                            self.preview_open = true;
                            self.live_seen = serde_json::to_vec(&*r).unwrap_or_default();
                            cmds.push(Command::Test(r.id));
                        }
                    }
                });
            });
            if conflicts > 0 {
                ui.colored_label(egui::Color32::from_rgb(230, 190, 90), format!("{conflicts} other alert(s) also react to this. Add a condition to avoid double alerts."));
            }
            // Plain-language summary; the event and platform are editable in place.
            egui::Frame::new().fill(ui.visuals().faint_bg_color).corner_radius(10).inner_margin(10).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.label("When");
                    egui::ComboBox::from_id_salt("event").selected_text(when_text(r.event)).show_ui(ui, |ui| {
                        for (group, events) in GROUPS {
                            ui.label(RichText::new(group).small().weak());
                            for &e in events {
                                ui.selectable_value(&mut r.event, e, when_text(e));
                            }
                        }
                    });
                    if r.event != EventKind::Timer {
                        ui.label("on");
                        egui::ComboBox::from_id_salt("platform").selected_text(r.platform.map(|p| p.name()).unwrap_or("any platform")).show_ui(ui, |ui| {
                            ui.selectable_value(&mut r.platform, None, "any platform");
                            for p in Platform::ALL {
                                ui.selectable_value(&mut r.platform, Some(p), p.name());
                            }
                        });
                    }
                });
                ui.label(RichText::new(format!("then {}.", then_text(&r.actions, &p.sources, &p.scenes))).strong());
            });
            ui.add_space(6.);
            // The most useful condition for each event, up front.
            ui.horizontal_wrapped(|ui| match r.event {
                EventKind::Donation | EventKind::SuperChat => {
                    ui.label("Only when the amount is at least");
                    let mut amount = r.options.min_amount as f64 / 100.;
                    if ui.add(egui::DragValue::new(&mut amount).speed(0.1).fixed_decimals(2).range(0.0..=1000000000.0)).changed() {
                        r.options.min_amount = (amount * 100.).round() as u64;
                    }
                    ui.label("in");
                    ui.add(egui::TextEdit::singleline(&mut r.options.currency).char_limit(3).desired_width(44.).hint_text("any"));
                    ui.label("currency");
                }
                EventKind::Bits | EventKind::Gift | EventKind::Raid => {
                    ui.label("Only when there are at least");
                    ui.add(egui::DragValue::new(&mut r.options.min_count).range(0..=1000000000u64));
                    ui.label(match r.event { EventKind::Bits => "bits", EventKind::Gift => "gifts", _ => "raiders" });
                }
                EventKind::ChatCommand => {
                    ui.label("Command");
                    ui.add(egui::TextEdit::singleline(&mut r.command).desired_width(140.).hint_text("!hello"));
                }
                EventKind::ChatMessage => {
                    ui.label("Message contains");
                    ui.add(egui::TextEdit::singleline(&mut r.conditions.phrase).desired_width(200.).hint_text("any message"));
                }
                EventKind::Redemption => {
                    ui.label("Reward ID");
                    ui.add(egui::TextEdit::singleline(&mut r.conditions.reward_id).desired_width(200.).hint_text("any reward"));
                    ui.small("Tip: ... > Twitch channel point rewards links a reward without copying its ID.");
                }
                EventKind::Timer => {
                    if r.options.timer_minutes == 0 {
                        r.options.timer_minutes = 15;
                    }
                    ui.label("Every");
                    ui.add(egui::DragValue::new(&mut r.options.timer_minutes).range(1..=240).suffix(" min"));
                    ui.label("if at least");
                    ui.add(egui::DragValue::new(&mut r.options.timer_messages).range(0..=500));
                    ui.label("chat messages came in since last time");
                    ui.checkbox(&mut r.options.timer_offline, "Also while I'm not streaming");
                }
                _ => {}
            });
            let timer = r.event == EventKind::Timer;
            ui.collapsing("Options: conditions, cooldowns, scene and shortcut", |ui| {
                egui::Grid::new("conditions").num_columns(2).spacing([10., 8.]).show(ui, |ui| {
                    if !timer {
                    ui.label("Only for this viewer");
                    ui.add(egui::TextEdit::singleline(&mut r.conditions.author).hint_text("anyone"));
                    ui.end_row();
                    if r.event != EventKind::ChatMessage {
                        ui.label("Message contains");
                        ui.add(egui::TextEdit::singleline(&mut r.conditions.phrase).hint_text("anything"));
                        ui.end_row();
                    }
                    ui.label("Match words as");
                    ui.horizontal(|ui| {
                        use auroracast_core::alert_filters::PhraseMode;
                        for (m, l) in [(PhraseMode::WholeWords, "Whole words"), (PhraseMode::Contains, "Anywhere"), (PhraseMode::Exact, "Exact message")] {
                            ui.selectable_value(&mut r.conditions.phrase_mode, m, l);
                        }
                        ui.checkbox(&mut r.conditions.case_sensitive, "Match capitals");
                    });
                    ui.end_row();
                    ui.label("Wait between alerts");
                    let mut seconds = r.cooldown_ms as f64 / 1000.;
                    if ui.add(egui::DragValue::new(&mut seconds).range(0.0..=3600.0).speed(0.1).suffix(" s")).changed() {
                        r.cooldown_ms = (seconds * 1000.).round() as u64;
                    }
                    ui.end_row();
                    ui.label("Same viewer can trigger again after");
                    let mut seconds = r.options.per_user_ms as f64 / 1000.;
                    if ui.add(egui::DragValue::new(&mut seconds).range(0.0..=3600.0).speed(0.1).suffix(" s")).changed() {
                        r.options.per_user_ms = (seconds * 1000.).round() as u64;
                    }
                    ui.end_row();
                    ui.label("Who can trigger it");
                    egui::ComboBox::from_id_salt("permission")
                        .selected_text(match r.options.permission { Permission::Everyone => "Everyone", Permission::Moderators => "Moderators", Permission::Subscribers => "Subscribers", Permission::SelectedUsers => "Only these viewers" })
                        .show_ui(ui, |ui| {
                            for (x, l) in [(Permission::Everyone, "Everyone"), (Permission::Moderators, "Moderators"), (Permission::Subscribers, "Subscribers"), (Permission::SelectedUsers, "Only these viewers")] {
                                ui.selectable_value(&mut r.options.permission, x, l);
                            }
                        });
                    ui.end_row();
                    if r.options.permission == Permission::SelectedUsers {
                        ui.label("Viewer names, comma separated");
                        ui.text_edit_singleline(&mut r.options.users);
                        ui.end_row();
                    }
                    ui.label("Events from while Rosorah was closed");
                    ui.checkbox(&mut r.options.missed_alerts, "Play these too");
                    ui.end_row();
                    }
                    ui.label("Only while this scene is live");
                    egui::ComboBox::from_id_salt("scope")
                        .selected_text(r.options.scene.and_then(|id| p.scenes.iter().find(|s| s.id == id)).map(|s| s.name.as_str()).unwrap_or(if r.options.scene.is_some() { "Missing scene" } else { "Any scene" }))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut r.options.scene, None, "Any scene");
                            for s in &p.scenes {
                                ui.selectable_value(&mut r.options.scene, Some(s.id), &s.name);
                            }
                        });
                    ui.end_row();
                    ui.label("Keyboard shortcut");
                    egui::ComboBox::from_id_salt("hotkey")
                        .selected_text(if r.options.hotkey == 0 { "None".to_owned() } else { format!("Ctrl+Alt+F{}", r.options.hotkey) })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut r.options.hotkey, 0, "None");
                            for k in 1..=12u8 {
                                ui.selectable_value(&mut r.options.hotkey, k, format!("Ctrl+Alt+F{k}"));
                            }
                        });
                    ui.end_row();
                });
                if !timer {
                    ui.small("Amounts use the original currency. Role conditions need role data from the platform; browser-read chat may not include it.");
                }
            });
            ui.add_space(8.);
            ui.strong("Then");
            let mut remove = None;
            let action_count = r.actions.len();
            // Steps slide aside while one is dragged, showing where it will land.
            let mut shift = crate::studio::ShiftDrag::begin(ui, ui.id().with("automation-steps"), false, p.preferences.workspace.animations, action_count);
            let rule_id = r.id;
            for (i, a) in r.actions.iter_mut().enumerate() {
                shift.item(ui, i, |ui| {
                    let mut grab = None;
                    ui.push_id(i, |ui| {
                        egui::Frame::new().fill(ui.visuals().faint_bg_color).corner_radius(10).inner_margin(10).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                let handle = ui
                                    .add(egui::Label::new(RichText::new(format!("↕  {} · {}", i + 1, action_label(a))).strong()).sense(egui::Sense::click_and_drag()))
                                    .on_hover_text("Drag to reorder")
                                    .on_hover_cursor(crate::studio::grab_cursor());
                                grab = Some(handle);
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.small_button("Remove").clicked() {
                                        remove = Some(i);
                                    }
                                });
                            });
                            changed |= self.action_body(ui, (rule_id, i), a, &mut p.sources, &p.canvases, &p.scenes, &mut p.audio, &mut p.preferences.monitor_device, cmds, &mut sample_choice);
                        });
                        ui.add_space(4.);
                    });
                    grab.unwrap()
                });
            }
            let move_action = shift.finish(ui);
            if let Some(i) = remove {
                r.actions.remove(i);
            } else if let Some((from, to)) = move_action
                && from < action_count
                && to < action_count
            {
                let a = r.actions.remove(from);
                r.actions.insert(to, a);
            }
            ui.horizontal_wrapped(|ui| {
                if r.actions.len() >= 64 {
                    return;
                }
                ui.label("Add a step:");
                if let Some(alert_box) = p.sources.iter().find(|s| matches!(&s.kind, SourceKind::Widget(w) if w.kind == auroracast_core::widgets::Kind::AlertBox)).map(|s| s.id)
                    && ui.button("Show alert").clicked()
                {
                    r.actions.push(Action::DisplayAlert { source: alert_box, template: "Thanks, {user}!".into(), media: None, seconds: 6, subject: Default::default() });
                }
                if ui.button("Show text").clicked() {
                    r.actions.push(Action::ShowAlert { template: "Thanks, {user}!".into(), seconds: 5 });
                }
                if ui.button("Play sound").clicked() {
                    r.actions.push(Action::PlaySound { source: 0 });
                }
                if ui.button("Play media").clicked() {
                    r.actions.push(Action::PlayMedia { source: 0 });
                }
                if ui.button("Send chat").clicked() {
                    let text = if r.event == EventKind::Timer { "Thanks for hanging out!" } else { "Thanks, {user}!" };
                    r.actions.push(Action::SendChat { text: text.into(), platforms: vec![], sender: Default::default() });
                }
                if ui.button("Pause").clicked() {
                    r.actions.push(Action::Wait { milliseconds: 1000 });
                }
                if ui.button("Update widget").clicked() {
                    r.actions.push(Action::UpdateWidget { source: p.sources.iter().find(|s| matches!(s.kind, SourceKind::Widget(_))).map(|s| s.id).unwrap_or(0), text: String::new(), increment: 1, start_timer: false });
                }
                if ui.button("Switch scene").clicked() && !p.canvases.is_empty() {
                    r.actions.push(Action::SwitchScene { canvas: p.canvases[0].id, scene: p.canvases[0].scene });
                }
            });
        }
        // Applied after the rule borrow ends: creating a source needs the whole project.
        if let Some((i, sample)) = sample_choice {
            let play_media = matches!(p.rules[index].actions.get(i), Some(Action::PlayMedia { .. }));
            let id = add_sample_source(p, &sample, play_media);
            match p.rules[index].actions.get_mut(i) {
                Some(Action::DisplayAlert { media, .. }) => *media = Some(id),
                Some(Action::PlaySound { source } | Action::PlayMedia { source }) => *source = id,
                _ => {}
            }
            changed = true;
        }
        changed
    }
    /// Settings for one action step. Returns true when project media/audio settings changed.
    #[allow(clippy::too_many_arguments)]
    fn action_body(
        &mut self,
        ui: &mut egui::Ui,
        (rule_id, i): (Id, usize),
        a: &mut Action,
        sources: &mut [auroracast_core::Source],
        canvases: &[auroracast_core::Canvas],
        scenes: &[auroracast_core::Scene],
        audio: &mut [auroracast_core::AudioChannel],
        monitor_device: &mut String,
        cmds: &mut Vec<Command>,
        sample_choice: &mut Option<(usize, crate::alert_samples::Sample)>,
    ) -> bool {
        let mut changed = false;
        let media_name = |sources: &[auroracast_core::Source], id: Id| sources.iter().find(|s| s.id == id).map(|s| s.name.clone());
        let sound = matches!(a, Action::PlaySound { .. });
        match a {
            Action::DisplayAlert { source, template, media, seconds, .. } => {
                // One shared alert box shows everything; an alert can have a box of its own.
                ui.horizontal(|ui| {
                    ui.label("Show in");
                    let boxes: Vec<(Id, String)> = sources
                        .iter()
                        .filter(|s| matches!(&s.kind, SourceKind::Widget(w) if w.kind == auroracast_core::widgets::Kind::AlertBox))
                        .map(|s| (s.id, s.name.clone()))
                        .collect();
                    let current = boxes.iter().find(|(id, _)| id == source).map_or("Alerts", |(_, n)| n.as_str()).to_string();
                    egui::ComboBox::from_id_salt(("alert_box", i)).selected_text(current).show_ui(ui, |ui| {
                        for (id, name) in &boxes {
                            changed |= ui.selectable_value(source, *id, name).changed();
                        }
                        ui.separator();
                        if ui.button("New alert box…").on_hover_text("Adds another alert box to this scene for this alert; move and size it like any layer").clicked() {
                            cmds.push(Command::NewAlertBox(rule_id, i));
                        }
                    });
                });
                if let Some(s) = sources.iter_mut().find(|s| s.id == *source)
                    && let SourceKind::Widget(w) = &mut s.kind
                {
                    ui.horizontal(|ui| {
                        ui.label("Text position");
                        for layout in [auroracast_core::widgets::AlertLayout::Above, auroracast_core::widgets::AlertLayout::Below, auroracast_core::widgets::AlertLayout::Over] {
                            changed |= ui.selectable_value(&mut w.alert_layout, layout, layout.label()).changed();
                        }
                    });
                }
                ui.label("Message");
                ui.add(egui::TextEdit::multiline(template).desired_rows(2).desired_width(f32::INFINITY));
                variables(ui, template, self.event);
                ui.add(egui::Slider::new(seconds, 1..=60).text("seconds on screen"));
                ui.horizontal_wrapped(|ui| {
                    ui.label("Picture / video");
                    egui::ComboBox::from_id_salt(("alert_media", i))
                        .selected_text(media.and_then(|id| media_name(sources, id)).unwrap_or_else(|| "None".into()))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(media, None, "None");
                            for s in sources.iter().filter(|s| matches!(s.kind, SourceKind::VideoFile(_))) {
                                ui.selectable_value(media, Some(s.id), &s.name);
                            }
                        });
                    if let Some(sample) = self.sample_menu(ui, false) {
                        *sample_choice = Some((i, sample));
                    }
                });
                if let Some(id) = *media {
                    changed |= media_settings(ui, sources, id, false);
                    changed |= alert_monitoring(ui, audio, monitor_device, id);
                }
            }
            Action::PlaySound { source } | Action::PlayMedia { source } => {
                ui.horizontal_wrapped(|ui| {
                    // Just the file this step plays; choosing another gives the step its own media.
                    let path = sources.iter().find(|s| s.id == *source).and_then(|s| match &s.kind {
                        SourceKind::VideoFile(v) => Some(v.path.clone()),
                        _ => None,
                    });
                    match &path {
                        Some(path) => {
                            let file = std::path::Path::new(path).file_name().map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
                            ui.strong(file).on_hover_text(path);
                        }
                        None => {
                            ui.weak("No file chosen");
                        }
                    }
                    let choosing = self.picker.as_ref().is_some_and(|(key, _)| *key == (rule_id, i));
                    if ui.add_enabled(!choosing, egui::Button::new(if path.is_some() { "Change file…" } else { "Choose file…" })).clicked() {
                        let (tx, rx) = mpsc::channel();
                        self.picker = Some(((rule_id, i), rx));
                        std::thread::spawn(move || {
                            let path = crate::desktop::pick(crate::desktop::Pick::Open, if sound { "Choose a sound" } else { "Choose a picture or video" }, "").unwrap_or_else(|e| {
                                crate::capture_log::event(e.to_string());
                                None
                            });
                            let _ = tx.send(path);
                        });
                    }
                    if let Some(sample) = self.sample_menu(ui, sound) {
                        *sample_choice = Some((i, sample));
                    }
                    if path.is_some() {
                        if ui.button("Preview").clicked() {
                            cmds.push(Command::Preview(*source));
                        }
                    }
                });
                changed |= media_settings(ui, sources, *source, !sound);
                changed |= alert_monitoring(ui, audio, monitor_device, *source);
                if !sound {
                    ui.small("Media plays where its layer sits in the live scene; move it in the canvas or its Properties.");
                }
            }
            Action::SendChat { text, platforms, sender } => {
                ui.add(egui::TextEdit::multiline(text).desired_rows(2).desired_width(f32::INFINITY));
                variables(ui, text, self.event);
                ui.horizontal_wrapped(|ui| {
                    ui.label("Send to");
                    // TikTok has no chat-sending API, so automations can't post there.
                    for platform in [Platform::Twitch, Platform::YouTube, Platform::Kick] {
                        let mut yes = platforms.contains(&platform);
                        if ui.checkbox(&mut yes, platform.name()).changed() {
                            platforms.retain(|p| *p != platform);
                            if yes {
                                platforms.push(platform);
                            }
                        }
                    }
                });
                ui.small(if self.event == EventKind::Timer { "Nothing ticked sends to every connected platform." } else { "Nothing ticked replies on the platform that triggered it." });
                ui.horizontal_wrapped(|ui| {
                    ui.label("Send as");
                    ui.selectable_value(sender, auroracast_core::ChatSender::Bot, "My bot");
                    ui.selectable_value(sender, auroracast_core::ChatSender::Streamer, "Me");
                });
                if *sender == auroracast_core::ChatSender::Bot {
                    ui.small(if self.bots.is_empty() {
                        "No bot account is connected (Settings > Accounts > Bot accounts), so this sends as you.".to_owned()
                    } else {
                        format!("Your bot is connected on {}; other platforms send as you.", self.bots.iter().map(|p| p.name()).collect::<Vec<_>>().join(" and "))
                    });
                }
            }
            Action::Wait { milliseconds } => {
                let mut seconds = *milliseconds as f64 / 1000.;
                ui.horizontal(|ui| {
                    ui.label("Wait");
                    if ui.add(egui::DragValue::new(&mut seconds).range(0.0..=3600.0).speed(0.1).suffix(" s")).changed() {
                        *milliseconds = (seconds * 1000.).round() as u64;
                    }
                    ui.label("before the next step");
                });
            }
            Action::ShowAlert { template, seconds } => {
                ui.add(egui::TextEdit::multiline(template).desired_rows(2).desired_width(f32::INFINITY));
                variables(ui, template, self.event);
                ui.add(egui::Slider::new(seconds, 1..=60).text("seconds on screen"));
            }
            Action::UpdateWidget { source, text, increment, start_timer } => {
                egui::ComboBox::from_id_salt(("widget", i))
                    .selected_text(media_name(sources, *source).unwrap_or_else(|| "Choose a widget".into()))
                    .show_ui(ui, |ui| {
                        for s in sources.iter().filter(|s| matches!(s.kind, SourceKind::Widget(_))) {
                            ui.selectable_value(source, s.id, &s.name);
                        }
                    });
                ui.horizontal(|ui| {
                    ui.label("New text");
                    ui.add(egui::TextEdit::singleline(text).hint_text("unchanged"));
                });
                variables(ui, text, self.event);
                ui.horizontal(|ui| {
                    ui.label("Add to counter");
                    ui.add(egui::DragValue::new(increment).range(-100000..=100000));
                    ui.checkbox(start_timer, "Start timer");
                });
            }
            Action::SwitchScene { canvas, scene } => {
                ui.horizontal_wrapped(|ui| {
                    ui.label("On");
                    egui::ComboBox::from_id_salt(("canvas", i))
                        .selected_text(canvases.iter().find(|c| c.id == *canvas).map(|c| c.name.as_str()).unwrap_or("Missing canvas"))
                        .show_ui(ui, |ui| {
                            for c in canvases {
                                ui.selectable_value(canvas, c.id, &c.name);
                            }
                        });
                    ui.label("switch to");
                    egui::ComboBox::from_id_salt(("scene", i))
                        .selected_text(scenes.iter().find(|s| s.id == *scene).map(|s| s.name.as_str()).unwrap_or("Missing scene"))
                        .show_ui(ui, |ui| {
                            for s in scenes {
                                ui.selectable_value(scene, s.id, &s.name);
                            }
                        });
                });
            }
        }
        changed
    }
}
/// Volume as a percentage, length as whole clip or a limit, and where the media is shown.
fn media_settings(ui: &mut egui::Ui, sources: &mut [auroracast_core::Source], id: Id, visual: bool) -> bool {
    let mut changed = false;
    let Some(s) = sources.iter_mut().find(|s| s.id == id) else { return false };
    let SourceKind::VideoFile(v) = &mut s.kind else { return false };
    if !std::path::Path::new(&v.path).is_file() {
        ui.colored_label(egui::Color32::LIGHT_RED, "Media file missing; use Change file…");
    }
    ui.horizontal_wrapped(|ui| {
        changed |= ui.checkbox(&mut v.audio, "Play its sound").changed();
        let mut percent = (v.gain * 100.).round();
        if ui.add(egui::Slider::new(&mut percent, 0.0..=200.0).suffix("%").text("volume")).changed() {
            v.gain = percent / 100.;
            changed = true;
        }
    });
    ui.horizontal_wrapped(|ui| {
        let mut whole = v.duration_secs == 0;
        if ui.radio_value(&mut whole, true, "Whole clip").changed() | ui.radio_value(&mut whole, false, "Stop after").changed() {
            v.duration_secs = if whole { 0 } else { 6 };
            changed = true;
        }
        if !whole {
            changed |= ui.add(egui::DragValue::new(&mut v.duration_secs).range(1..=300).suffix(" s")).changed();
        }
    });
    if visual {
        changed |= ui
            .checkbox(&mut v.automation_only, "Hidden until an alert plays it")
            .on_hover_text("Off: the media is also visible as a normal layer")
            .changed();
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sequential_wait_pause_and_stop() {
        let p = Project::default();
        let mut q = Scheduler::default();
        q.enqueue(
            "one".into(),
            vec![
                Action::Wait { milliseconds: 5000 },
                Action::ShowAlert {
                    template: "after".into(),
                    seconds: 1,
                },
            ],
        );
        q.enqueue(
            "two".into(),
            vec![Action::ShowAlert {
                template: "second".into(),
                seconds: 1,
            }],
        );
        assert!(q.tick(&p).is_empty());
        assert!(q.tick(&p).is_empty());
        q.paused = true;
        q.jobs[0].next = Instant::now();
        assert!(q.tick(&p).is_empty());
        q.paused = false;
        let a = q.tick(&p);
        assert!(matches!(&a[0],Action::ShowAlert{template,..}if template=="after"));
        q.clear();
        assert!(q.tick(&p).is_empty());
    }
    #[test]
    fn overlap_interrupt_and_queue_bounds() {
        let p = Project::default();
        let a = vec![Action::ShowAlert {
            template: "x".into(),
            seconds: 1,
        }];
        let mut q = Scheduler::default();
        q.mode = Mode::Overlap;
        q.enqueue("a".into(), a.clone());
        q.enqueue("b".into(), a.clone());
        assert_eq!(q.tick(&p).len(), 2);
        q.mode = Mode::Interrupt;
        q.enqueue("c".into(), a.clone());
        assert_eq!(q.len(), 1);
        q.mode = Mode::Queue;
        for _ in 0..120 {
            q.enqueue("a".into(), a.clone());
        }
        assert_eq!(q.len(), 100);
    }
    #[test]
    fn queue_waits_for_media_and_skip_targets_current_job() {
        let p = Project::default();
        let mut q = Scheduler::default();
        q.enqueue("clip".into(), vec![Action::PlayMedia { source: 1 }]);
        q.enqueue("next".into(), vec![Action::PlayMedia { source: 2 }]);
        assert_eq!(q.tick_media(&p, |_| (true, None)).len(), 1);
        q.jobs[0].media[0].1 = Instant::now() - Duration::from_secs(1);
        assert!(q.tick_media(&p, |_| (true, None)).is_empty());
        assert_eq!(q.len(), 2);
        assert!(q.tick_media(&p, |_| (false, None)).is_empty());
        assert_eq!(q.len(), 1);
        assert_eq!(q.tick_media(&p, |_| (true, None)).len(), 1);
        assert_eq!(q.skip(), vec![2]);
        assert_eq!(q.len(), 0);
    }
    #[test]
    fn portable_rules_exclude_secrets_remap_ids_and_disable() {
        let mut p = Project::default();
        let text = export(&p, &p.rules[0]);
        assert!(!text.contains("access_token"));
        assert!(!text.contains("stream_key"));
        let old = p.rules.len();
        import(&mut p, &text).unwrap();
        assert_eq!(p.rules.len(), old + 1);
        assert!(!p.rules.last().unwrap().enabled);
        assert_ne!(p.rules[0].id, p.rules.last().unwrap().id);
        assert!(import(&mut p, "{broken").is_err());
    }
}

pub fn preview_actions(actions: Vec<Action>, media_allowed: bool) -> Vec<Action> {
    actions
        .into_iter()
        .filter(|a| {
            matches!(a, Action::ShowAlert { .. } | Action::Wait { .. })
                || (media_allowed && matches!(a, Action::PlayMedia { .. } | Action::PlaySound {..} | Action::DisplayAlert {..}))
        })
        .collect()
}
pub fn replay_actions(actions: Vec<Action>) -> Vec<Action> {
    actions
        .into_iter()
        .filter(|a| !matches!(a, Action::UpdateWidget { .. } | Action::SwitchScene { .. } | Action::SendChat {..}))
        .collect()
}
#[cfg(test)]
mod preview_tests {
    use super::*;
    #[test]
    fn previews_and_replays_cannot_increment_goals_or_switch_scenes() {
        let actions = vec![
            Action::UpdateWidget {
                source: 1,
                text: String::new(),
                increment: 50,
                start_timer: false,
            },
            Action::SwitchScene {
                canvas: 1,
                scene: 2,
            },
            Action::PlayMedia { source: 3 },
            Action::Wait { milliseconds: 1 },
        ];
        let a = preview_actions(actions.clone(), false);
        assert_eq!(a.len(), 1);
        assert!(matches!(a[0], Action::Wait { .. }));
        assert_eq!(replay_actions(actions).len(), 2);
    }
}

#[cfg(test)]
mod layout_regression {
    use super::*;
    #[test]
    fn sidebar_stays_bounded_and_recovers_oversized_saved_width() {
        let ctx = egui::Context::default();
        let mut p = Project::default();
        p.rules[0].name = "A long automation name ".repeat(20);
        let mut editor = Editor::default();
        let mut queue = Scheduler::default();
        let engine = auroracast_core::AlertEngine::default();
        let id = egui::Id::new("automation_list");
        ctx.data_mut(|d| {
            d.insert_persisted(
                id,
                egui::containers::panel::PanelState {
                    rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400., 900.)),
                },
            )
        });
        for _ in 0..180 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1440., 900.),
                )),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                editor.ui(ctx, &mut p, &mut queue, &engine, &[]);
            });
            let width = egui::containers::panel::PanelState::load(&ctx, id)
                .unwrap()
                .rect
                .width();
            assert!(width <= 361., "Sidebar expanded to {width}");
        }
    }
}

/// Monitoring is independent of stream/recording routing and the video's audio checkbox.
fn alert_monitoring(ui:&mut egui::Ui,audio:&mut [auroracast_core::AudioChannel],monitor_device:&mut String,source:Id)->bool {
    let Some(channel)=audio.iter_mut().find(|c|c.media_source==Some(source))else {ui.small("Add this media to the audio mixer to hear it.");return false;};
    let changed=ui.checkbox(&mut channel.monitor,"Hear on my speakers / headphones").changed();
    if changed&&channel.monitor&&monitor_device.is_empty(){*monitor_device=crate::audio::DEFAULT_MONITOR.into();}
    if channel.monitor{ui.small("Uses the mixer monitoring output. Stream/recording audio is controlled separately.");}
    changed
}
#[cfg(test)]
mod friendly_tests {
    use super::*;
    #[test]
    fn tabs_templates_and_summary_read_naturally() {
        assert!(is_alert(EventKind::Follow) && is_alert(EventKind::Raid));
        assert!(!is_alert(EventKind::ChatCommand) && !is_alert(EventKind::Redemption));
        let mut p = Project::default();
        for (label, event, text) in ALERT_TEMPLATES.into_iter().chain(AUTOMATION_TEMPLATES) {
            let r = template_rule(&mut p, label, event, text);
            assert!(!r.actions.is_empty(), "{label} has no action");
            assert_eq!(is_alert(r.event), ALERT_TEMPLATES.iter().any(|t| t.0 == label));
            p.rules.push(r);
        }
        p.validate().unwrap();
        let actions = vec![
            Action::ShowAlert { template: "Thanks for the follow, {user}!".into(), seconds: 6 },
            Action::Wait { milliseconds: 1500 },
        ];
        assert_eq!(then_text(&actions, &p.sources, &p.scenes), "show \u{201c}Thanks for the follow, {user}!\u{201d} for 6 s, then pause 1.5 s");
        assert_eq!(then_text(&[], &p.sources, &p.scenes), "do nothing yet");
    }
    #[test]
    fn every_event_has_one_group_and_its_own_variables() {
        for e in EventKind::ALL {
            assert_eq!(GROUPS.iter().filter(|(_, events)| events.contains(&e)).count(), 1, "{e:?}");
            assert!(!when_text(e).is_empty());
        }
        assert!(!variables_for(EventKind::Follow).contains(&"{amount}"));
        assert!(variables_for(EventKind::Donation).contains(&"{amount}"));
        assert!(variables_for(EventKind::Redemption).contains(&"{reward}"));
        assert!(variables_for(EventKind::Timer).is_empty());
        let mut p = Project::default();
        let timer = template_rule(&mut p, "Chat timer", EventKind::Timer, "Follow!");
        assert_eq!(timer.options.timer_minutes, 15);
        assert!(matches!(&timer.actions[0], Action::SendChat { sender: auroracast_core::ChatSender::Bot, platforms, .. } if platforms.is_empty()));
        assert_eq!(capitalized("say hi"), "Say hi");
    }
    #[test]
    fn test_with_details_explains_why_a_rule_would_not_play() {
        let mut p = Project::default();
        let mut r = template_rule(&mut p, "Donation", EventKind::Donation, "Thanks");
        r.options.min_amount = 1000;
        let mut event = auroracast_core::Event {
            facts: auroracast_core::automation::Facts { amount: 500, currency: "USD".into(), ..Default::default() },
            reward_id: String::new(),
            id: "t".into(),
            platform: Platform::Twitch,
            kind: EventKind::Donation,
            author: "Rose".into(),
            text: String::new(),
            at_ms: 0,
        };
        assert_eq!(auroracast_core::check(&r, &event), Some("Amount below minimum or unavailable"));
        event.facts.amount = 1500;
        assert_eq!(auroracast_core::check(&r, &event), None);
        r.platform = Some(Platform::Kick);
        assert_eq!(auroracast_core::check(&r, &event), Some("Different platform"));
    }
}
