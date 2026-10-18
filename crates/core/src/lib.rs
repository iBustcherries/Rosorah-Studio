//! Original, platform-independent project, routing, chat and automation models.
pub mod alert_filters;
pub mod automation;
pub mod browser;
pub mod donations;
pub mod widgets;
pub mod layout;
pub mod platform;
pub mod workspace;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, VecDeque},
    path::Path,
};

pub type Id = u64;
pub const PROJECT_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum SourceKind {
    TestPattern,
    ScreenCapture,
    Color([u8; 4]),
    Image { path: String },
    Game,
    Camera,
    VideoCamera(CameraSettings),
    VideoFile(VideoSettings),
    Browser { url: String },
    ChatOverlay,
    Widget(widgets::Settings),
}

impl SourceKind {
    pub fn available(&self) -> bool {
        matches!(
            self,
            Self::ScreenCapture
                | Self::Browser { .. }
                | Self::VideoCamera(_)
                | Self::VideoFile(_)
                | Self::Widget(_)
                | Self::TestPattern
                | Self::Color(_)
                | Self::Image { .. }
        )
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::ScreenCapture => "Window / Screen",
            Self::TestPattern => "Test pattern",
            Self::Color(_) => "Color",
            Self::Image { .. } => "Image",
            Self::Game => "Game capture",
            Self::Camera | Self::VideoCamera(_) => "Camera",
            Self::VideoFile(_) => "Video file",
            Self::Browser { .. } => "Browser",
            Self::ChatOverlay => "Chat overlay",
            Self::Widget(_) => "Widget",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct VideoSettings {
    pub gain: f32,
    pub duration_secs: u32,
    pub canvases: Vec<Id>,
    pub automation_only: bool,
    pub path: String,
    pub looped: bool,
    pub enabled: bool,
    pub audio: bool,
    pub trigger: Option<EventKind>,
    pub platform: Option<Platform>,
}
impl Default for VideoSettings {
    fn default() -> Self {
        Self {
            gain: 1.,
            duration_secs: 0,
            canvases: Vec::new(),
            automation_only: false,
            path: String::new(),
            looped: true,
            enabled: true,
            audio: false,
            trigger: None,
            platform: None,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum NoiseSuppression {
    #[default]
    Off,
    Gentle,
    Strong,
}
impl NoiseSuppression {
    pub const ALL: [Self; 3] = [Self::Off, Self::Gentle, Self::Strong];
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Gentle => "Gentle · fan / hiss",
            Self::Strong => "Strong · noisy room",
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct SceneVolume {
    pub gain_db: f32,
    pub muted: bool,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Transition {
    #[default]
    Cut,
    Fade,
    Wipe,
    /// A video played over the canvas; the scene changes at its cut point.
    Stinger,
    /// Fade to a picture, hold, switch, fade back.
    Screen,
}
impl Transition {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cut => "Instant",
            Self::Fade => "Fade",
            Self::Wipe => "Wipe",
            Self::Stinger => "Stinger video",
            Self::Screen => "Transition screen",
        }
    }
    /// Fade and Wipe blend a snapshot of the old scene in the compositor; the others switch
    /// instantly underneath (Stinger and Screen hide the switch with an overlay).
    pub fn blends(self) -> bool {
        matches!(self, Self::Fade | Self::Wipe)
    }
}
/// Stinger video and transition screen settings. Each media file is an automation-only
/// video source (a picture plays as a still), so it gets the usual mixer channel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TransitionMedia {
    pub stinger: Option<Id>,
    /// When the scene changes, measured from the start of the stinger.
    pub cut_ms: u32,
    pub screen: Option<Id>,
    pub fade_ms: u32,
    pub hold_ms: u32,
}
impl Default for TransitionMedia {
    fn default() -> Self {
        Self { stinger: None, cut_ms: 700, screen: None, fade_ms: 400, hold_ms: 600 }
    }
}
/// What a canvas draws on top of everything during a stinger or screen transition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Overlay {
    pub source: Id,
    pub opacity: f32,
}
fn transition_duration() -> u32 {
    300
}
/// Camera input configuration. Disabled until explicitly connected by the user.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct CameraSettings {
    pub device: String,
    pub network: bool,
    pub host: String,
    pub port: u16,
    pub legacy_url: bool,
    /// Negotiate the device's available output instead of imposing manual size/FPS.
    pub auto_mode: bool,
    /// Fit camera layers to negotiated frame proportions on input-size changes.
    pub auto_aspect: bool,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub enabled: bool,
    pub flip: bool,
    /// Left, top, right, bottom crop in pixels of the negotiated input.
    pub crop: [u32; 4],
}
impl Default for CameraSettings {
    fn default() -> Self {
        Self {
            device: String::new(),
            network: false,
            host: String::new(),
            port: 4747,
            legacy_url: false,
            auto_mode: true,
            auto_aspect: true,
            width: 1280,
            height: 720,
            fps: 60,
            enabled: false,
            flip: false,
            crop: [0; 4],
        }
    }
}
impl CameraSettings {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            (16..=4096).contains(&self.width)
                && (16..=4096).contains(&self.height)
                && (1..=120).contains(&self.fps),
            "Camera size or frame rate is out of range"
        );
        anyhow::ensure!(
            self.crop.iter().all(|v| *v <= 4096),
            "Camera crop is out of range"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Source {
    pub id: Id,
    pub name: String,
    pub kind: SourceKind,
}

/// Normalized canvas coordinates. A source can be referenced by many layers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Layer {
    #[serde(default)]
    pub crop: [f32;4],
    #[serde(default)]
    pub group: u64,
    pub source: Id,
    pub rect: [f32; 4],
    pub visible: bool,
    pub locked: bool,
    pub opacity: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Scene {
    pub id: Id,
    pub name: String,
    pub layers: Vec<Layer>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Canvas {
    pub id: Id,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub scene: Id,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AudioChannel {
    #[serde(default)]
    pub monitor_only:bool,
    pub id: Id,
    pub name: String,
    pub gain_db: f32,
    pub muted: bool,
    pub tracks: [bool; 6],
    #[serde(default)]
    pub delay_ms: u32,
    #[serde(default)]
    pub monitor: bool,
    #[serde(default)]
    pub noise_suppression: NoiseSuppression,
    #[serde(default)]
    pub media_source: Option<Id>,
}

impl AudioChannel {
    pub fn gain(&self) -> f32 {
        if self.muted || self.gain_db <= -100.0 {
            0.0
        } else {
            10.0f32.powf(self.gain_db / 20.0)
        }
    }
}

/// Float PCM routing foundation. Device capture and monitoring are separate adapters.
pub fn mix_audio(
    channels: &[AudioChannel],
    inputs: &[(Id, &[f32])],
    track: usize,
    frames: usize,
) -> Result<Vec<f32>> {
    if track >= 6 {
        bail!("Audio track must be between 1 and 6");
    }
    let mut output = vec![0.0f32; frames];
    for channel in channels.iter().filter(|c| c.tracks[track] && !c.muted && !c.monitor_only) {
        if let Some((_, pcm)) = inputs.iter().find(|(id, _)| *id == channel.id) {
            let gain = channel.gain();
            for (out, sample) in output.iter_mut().zip(pcm.iter()) {
                *out += sample * gain;
            }
        }
    }
    // Hard limiter for the initial implementation; a lookahead limiter comes later.
    for sample in &mut output {
        *sample = sample.clamp(-1.0, 1.0);
    }
    Ok(output)
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Platform {
    Twitch,
    YouTube,
    Kick,
    TikTok,
    Donations,
}
impl Platform {
    pub const ALL: [Self; 5] = [
        Self::Twitch,
        Self::YouTube,
        Self::Kick,
        Self::TikTok,
        Self::Donations,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Twitch => "Twitch",
            Self::YouTube => "YouTube",
            Self::Kick => "Kick",
            Self::TikTok => "TikTok",
            Self::Donations => "Donations",
        }
    }
    pub fn color(self) -> [u8; 3] {
        match self {
            Self::Twitch => [159, 110, 255],
            Self::YouTube => [255, 88, 106],
            Self::Kick => [102, 222, 104],
            Self::TikTok => [70, 219, 227],
            Self::Donations => [255, 198, 92],
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ChatCapabilities {
    pub receive: bool,
    pub send: bool,
    pub delete: bool,
    pub timeout: bool,
    pub ban: bool,
}

/// No connector is registered in v0.1. Credentials must later live in the OS keyring.
pub trait ChatConnector: Send {
    fn platform(&self) -> Platform;
    fn capabilities(&self) -> ChatCapabilities;
    fn send(&mut self, text: &str) -> Result<()>;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    #[serde(default)]
    pub metadata: alert_filters::ChatMetadata,
    pub platform: Platform,
    pub id: String,
    pub author: String,
    pub text: String,
    pub received_ms: u64,
    pub demo: bool,
}

pub struct ChatFeed {
    revision: u64,
    messages: VecDeque<ChatMessage>,
    capacity: usize,
}
impl ChatFeed {
    pub fn new(capacity: usize) -> Self {
        Self {
            revision: 0,
            messages: VecDeque::new(),
            capacity: capacity.max(1),
        }
    }
    pub fn push(&mut self, message: ChatMessage) -> bool {
        if self
            .messages
            .iter()
            .any(|m| m.platform == message.platform && m.id == message.id)
        {
            return false;
        }
        let position = self
            .messages
            .iter()
            .position(|m| m.received_ms > message.received_ms)
            .unwrap_or(self.messages.len());
        self.messages.insert(position, message);
        self.revision = self.revision.wrapping_add(1);
        while self.messages.len() > self.capacity {
            self.messages.pop_front();
        }
        true
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn messages(&self) -> impl Iterator<Item = &ChatMessage> {
        self.messages.iter()
    }
    pub fn remove(&mut self, platform: Platform, id: &str) {
        self.revision = self.revision.wrapping_add(1);
        self.messages
            .retain(|m| m.platform != platform || m.id != id);
    }
    pub fn clear_author(&mut self, platform: Platform, author: &str) {
        self.revision = self.revision.wrapping_add(1);
        self.messages.retain(|m| {
            m.platform != platform || (!author.is_empty() && !m.author.eq_ignore_ascii_case(author))
        });
    }
    pub fn clear(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.messages.clear();
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum EventKind {
    Donation,
    Follow,
    Subscription,
    ChatCommand,
    ChatMessage,
    FirstMessage,
    FirstTimeChatter,
    Gift,
    Bits,
    Raid,
    Redemption,
    Membership,
    SuperChat,
    SuperSticker,
    Subscriber,
    HypeTrain,
    Poll,
    Prediction,
}
impl EventKind {
    pub const ALL: [Self; 18] = [
        Self::Donation,
        Self::Follow,
        Self::Subscription,
        Self::ChatCommand,
        Self::ChatMessage,
        Self::FirstMessage,
        Self::FirstTimeChatter,
        Self::Gift,
        Self::Bits,
        Self::Raid,
        Self::Redemption,
        Self::Membership,
        Self::SuperChat,
        Self::SuperSticker,
        Self::Subscriber,
        Self::HypeTrain,
        Self::Poll,
        Self::Prediction,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Donation => "Donation",
            Self::Follow => "Follow",
            Self::Subscription => "Subscription",
            Self::ChatCommand => "Chat command",
            Self::ChatMessage => "Chat word or phrase",
            Self::FirstMessage => "First message this stream",
            Self::FirstTimeChatter => "First-ever message (Twitch)",
            Self::Gift => "Gifted subscriptions",
            Self::Bits => "Bits / Kicks",
            Self::Raid => "Raid",
            Self::Redemption => "Channel points",
            Self::Membership => "Membership",
            Self::SuperChat => "Super Chat",
            Self::SuperSticker => "Super Sticker",
            Self::Subscriber => "YouTube subscriber",
            Self::HypeTrain => "Hype Train",
            Self::Poll => "Poll",
            Self::Prediction => "Prediction",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Event {
    pub facts: automation::Facts,
    pub reward_id: String,
    pub id: String,
    pub platform: Platform,
    pub kind: EventKind,
    pub author: String,
    pub text: String,
    pub at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Action {
    DisplayAlert { source: Id, template: String, media: Option<Id>, seconds: u32 },
    PlaySound { source: Id },
    SendChat { text: String, platforms: Vec<Platform> },
    Wait {
        milliseconds: u64,
    },
    ShowAlert {
        template: String,
        seconds: u32,
    },
    PlayMedia {
        source: Id,
    },
    UpdateWidget {
        source: Id,
        text: String,
        increment: i32,
        start_timer: bool,
    },
    SwitchScene {
        canvas: Id,
        scene: Id,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rule {
    #[serde(default)]
    pub options: automation::Options,
    #[serde(default)]
    pub conditions: alert_filters::Conditions,
    pub id: Id,
    pub name: String,
    pub enabled: bool,
    pub platform: Option<Platform>,
    pub event: EventKind,
    pub command: String,
    pub cooldown_ms: u64,
    pub actions: Vec<Action>,
}

#[derive(Default)]
pub struct AlertEngine {
    seen: VecDeque<(Platform, String, EventKind)>,
    last_fired: std::collections::BTreeMap<Id, u64>,
    per_user: std::collections::BTreeMap<(Id, String), u64>,
    pub history: VecDeque<String>,
}
/// Why `rule` would not play for `event`, ignoring cooldowns; `None` means it matches.
pub fn check(r: &Rule, event: &Event) -> Option<&'static str> {
    if r.event != event.kind {
        Some("Different event")
    } else if !r.enabled {
        Some("Disabled")
    } else if r.platform.is_some_and(|p| p != event.platform) {
        Some("Different platform")
    } else if !r.conditions.matches(event) {
        Some("Text, user or reward condition did not match")
    } else if let Some(reason) = r.options.reason(event) {
        Some(reason)
    } else if r.event == EventKind::ChatCommand
        && event.text.split_whitespace().next() != Some(r.command.as_str())
    {
        Some("Different command")
    } else {
        None
    }
}
impl AlertEngine {
    /// When `rule` last played (milliseconds since the epoch), this session.
    pub fn last_fired(&self, rule: Id) -> Option<u64> {
        self.last_fired.get(&rule).copied()
    }
    pub fn dispatch(&mut self, rules: &[Rule], event: &Event) -> Vec<Action> {
        self.dispatch_rules(rules, event)
            .into_iter()
            .flat_map(|(_, a)| a)
            .collect()
    }
    pub fn dispatch_rules(&mut self, rules: &[Rule], event: &Event) -> Vec<(Id, Vec<Action>)> {
        let key = (event.platform, event.id.clone(), event.kind);
        if self.seen.contains(&key) {
            return Vec::new();
        }
        self.seen.push_back(key);
        while self.seen.len() > 1024 {
            self.seen.pop_front();
        }
        let mut output = Vec::new();
        for r in rules.iter().filter(|r| r.event == event.kind) {
            let user = (r.id, event.author.to_lowercase());
            let reason = if let Some(reason) = check(r, event) {
                Some(reason)
            } else if self
                .last_fired
                .get(&r.id)
                .is_some_and(|t| event.at_ms.saturating_sub(*t) < r.cooldown_ms)
            {
                Some("Global cooldown")
            } else if self
                .per_user
                .get(&user)
                .is_some_and(|t| event.at_ms.saturating_sub(*t) < r.options.per_user_ms)
            {
                Some("Viewer cooldown")
            } else {
                None
            };
            self.history.push_back(format!(
                "{} · {} · {}",
                event.at_ms,
                r.name,
                reason.unwrap_or("Matched")
            ));
            while self.history.len() > 200 {
                self.history.pop_front();
            }
            if reason.is_some() {
                continue;
            }
            self.last_fired.insert(r.id, event.at_ms);
            if self.per_user.len() >= 8192 {
                self.per_user
                    .retain(|_, t| event.at_ms.saturating_sub(*t) < 3600000);
            }
            if self.per_user.len() >= 8192 {
                self.history
                    .push_back("Viewer cooldown capacity reached; skipped".into());
                continue;
            }
            self.per_user.insert(user, event.at_ms);
            output.push((r.id, automation::actions(&r.actions, event)));
        }
        output
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum RecordingEncoder {
    #[default]
    Auto,
    Software,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum CaptureResolution {
    P720,
    #[default]
    P1080,
    P1440,
    P2160,
    Original,
}
impl CaptureResolution {
    pub const ALL: [Self; 5] = [
        Self::P720,
        Self::P1080,
        Self::P1440,
        Self::P2160,
        Self::Original,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::P720 => "720p · 1280 × 720",
            Self::P1080 => "1080p · 1920 × 1080",
            Self::P1440 => "1440p · 2560 × 1440",
            Self::P2160 => "4K · 3840 × 2160",
            Self::Original => "Original size (up to 4096 × 4096)",
        }
    }
    pub fn wire(self) -> u32 {
        match self {
            Self::P720 => 0,
            Self::P1080 => 1,
            Self::P1440 => 2,
            Self::P2160 => 3,
            Self::Original => 4,
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SnapSettings {
    pub enabled: bool,
    pub canvas_edges: bool,
    pub canvas_center: bool,
    pub sources: bool,
    pub distance: f32,
}
impl Default for SnapSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            canvas_edges: true,
            canvas_center: true,
            sources: true,
            distance: 8.,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub enum LayerOrder {
    Raise,
    Lower,
    Front,
    Back,
}
impl Scene {
    /// Layers are painted back-to-front; preserve the selected layer and all
    /// its properties when moving it through the stack.
    pub fn reorder_layer(&mut self, index: usize, order: LayerOrder) -> Option<usize> {
        let layer = self.layers.get(index)?;
        if layer.locked {
            return None;
        }
        let last = self.layers.len() - 1;
        let target = match order {
            LayerOrder::Raise => (index + 1).min(last),
            LayerOrder::Lower => index.saturating_sub(1),
            LayerOrder::Front => last,
            LayerOrder::Back => 0,
        };
        if target == index {
            return None;
        }
        let layer = self.layers.remove(index);
        self.layers.insert(target, layer);
        Some(target)
    }
}
/// One mixer control for every alert sound: added to each alert's own level.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AlertBus {
    pub gain_db: f32,
    pub muted: bool,
}
/// When Rosorah makes timestamped project backups, and how many it keeps.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BackupPolicy {
    pub hourly: bool,
    pub on_close: bool,
    pub keep: u32,
    /// At start, open the newest backup when it is newer than the saved project.
    pub restore_on_start: bool,
}
impl Default for BackupPolicy {
    fn default() -> Self {
        Self { hourly: false, on_close: true, keep: 24, restore_on_start: true }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preferences {
    #[serde(default)]
    pub backups: BackupPolicy,
    #[serde(default)]
    pub alert_bus: AlertBus,
    #[serde(default)]
    pub locked_canvases: Vec<Id>,
    #[serde(default)]
    pub widget_styles: std::collections::BTreeMap<String, widgets::Style>,
    #[serde(default)]
    pub browser_sources: std::collections::BTreeMap<Id, browser::Settings>,
    #[serde(default)]
    pub chroma_keys: std::collections::BTreeMap<Id, browser::ChromaKey>,
    #[serde(default)]
    pub automation_history: automation::HistoryPolicy,
    #[serde(default)]
    pub workspace: workspace::WorkspaceSettings,
    #[serde(default)]
    pub donations: donations::Setup,
    #[serde(default)]
    pub widget_periods: std::collections::BTreeMap<Id, String>,
    #[serde(default)]
    pub widget_values: std::collections::BTreeMap<Id, u64>,
    #[serde(default)]
    pub scene_audio: std::collections::BTreeMap<Id, std::collections::BTreeMap<Id, SceneVolume>>,
    #[serde(default)]
    pub transition: Transition,
    #[serde(default = "transition_duration")]
    pub transition_ms: u32,
    #[serde(default)]
    pub transition_media: TransitionMedia,
    /// Live transition overlays by canvas id; never saved.
    #[serde(skip)]
    pub overlays: std::collections::BTreeMap<Id, Overlay>,
    #[serde(default)]
    pub snapping: SnapSettings,
    #[serde(default)]
    pub capture_resolution: CaptureResolution,
    pub accent: [u8; 3],
    pub chat_font_size: f32,
    pub timestamps_24h: bool,
    pub auto_game_capture: bool,
    pub auto_game_audio: bool,
    #[serde(default)]
    pub manual_capture_sources: Vec<Id>,
    #[serde(default)]
    pub windows_capture_monitors: std::collections::BTreeMap<Id, String>,
    #[serde(default)]
    pub windows_auto_sources: Vec<Id>,
    #[serde(default)]
    pub windows_capture_cursor: bool,
    #[serde(default)]
    pub recording_directory: String,
    #[serde(default)]
    pub recording_encoder: RecordingEncoder,
    #[serde(default)]
    pub multitrack_recording: bool,
    #[serde(default)]
    pub monitor_device: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub schema_version: u32,
    pub next_id: Id,
    pub sources: Vec<Source>,
    pub scenes: Vec<Scene>,
    pub canvases: Vec<Canvas>,
    pub audio: Vec<AudioChannel>,
    pub rules: Vec<Rule>,
    pub preferences: Preferences,
}

fn layer(source: Id, rect: [f32; 4]) -> Layer {
    Layer { crop: [0.;4], group: 0,
        source,
        rect,
        visible: true,
        locked: false,
        opacity: 1.0,
    }
}

impl Default for Project {
    fn default() -> Self {
        Self {
            schema_version: PROJECT_VERSION,
            next_id: 30,
            sources: vec![
                Source {
                    id: 1,
                    name: "Shared test pattern".into(),
                    kind: SourceKind::TestPattern,
                },
                Source {
                    id: 2,
                    name: "Camera placeholder".into(),
                    kind: SourceKind::Color([139, 91, 229, 255]),
                },
                Source {
                    id: 3,
                    name: "Lower third".into(),
                    kind: SourceKind::Color([12, 18, 34, 215]),
                },
            ],
            scenes: vec![
                Scene {
                    id: 4,
                    name: "Gameplay".into(),
                    layers: vec![
                        layer(1, [0., 0., 1., 1.]),
                        layer(2, [0.76, 0.05, 0.21, 0.25]),
                        layer(3, [0.04, 0.83, 0.5, 0.12]),
                    ],
                },
                Scene {
                    id: 5,
                    name: "Just Chatting".into(),
                    layers: vec![
                        layer(1, [0., 0., 1., 1.]),
                        layer(2, [0.28, 0.12, 0.44, 0.7]),
                    ],
                },
                Scene {
                    id: 6,
                    name: "Be Right Back".into(),
                    layers: vec![layer(3, [0., 0., 1., 1.]), layer(2, [0.25, 0.3, 0.5, 0.4])],
                },
                Scene {
                    id: 7,
                    name: "Vertical".into(),
                    layers: vec![
                        layer(1, [0., 0., 1., 1.]),
                        layer(2, [0.08, 0.04, 0.84, 0.25]),
                    ],
                },
            ],
            canvases: vec![
                Canvas {
                    id: 8,
                    name: "Canvas 1".into(),
                    width: 1920,
                    height: 1080,
                    fps: 60,
                    scene: 4,
                },
                Canvas {
                    id: 9,
                    name: "Canvas 2".into(),
                    width: 1080,
                    height: 1920,
                    fps: 60,
                    scene: 7,
                },
            ],
            audio: vec![
                AudioChannel { monitor_only:false,
                    id: 10,
                    name: "Game audio".into(),
                    gain_db: -6.,
                    muted: false,
                    tracks: [true, true, false, false, false, false],
                    delay_ms: 0,
                    monitor: false,
                    noise_suppression: NoiseSuppression::Off,
                    media_source: None,
                },
                AudioChannel { monitor_only:false,
                    id: 11,
                    name: "Microphone".into(),
                    gain_db: 0.,
                    muted: false,
                    tracks: [true, true, true, false, false, false],
                    delay_ms: 0,
                    monitor: false,
                    noise_suppression: NoiseSuppression::Off,
                    media_source: None,
                },
                AudioChannel { monitor_only:false,
                    id: 12,
                    name: "Music".into(),
                    gain_db: -12.,
                    muted: false,
                    tracks: [true, false, false, true, false, false],
                    delay_ms: 0,
                    monitor: false,
                    noise_suppression: NoiseSuppression::Off,
                    media_source: None,
                },
            ],
            rules: vec![Rule {
                options: Default::default(),
                conditions: Default::default(),
                id: 13,
                name: "Welcome a follower".into(),
                enabled: true,
                platform: None,
                event: EventKind::Follow,
                command: "!hello".into(),
                cooldown_ms: 3000,
                actions: vec![Action::ShowAlert {
                    template: "Thanks for following, {user}!".into(),
                    seconds: 5,
                }],
            }],
            preferences: Preferences {
                backups: Default::default(),
                alert_bus: Default::default(),
                widget_styles: Default::default(),
                locked_canvases:Vec::new(),
                browser_sources: Default::default(),
                chroma_keys: Default::default(),
                workspace: Default::default(),
                automation_history: Default::default(),
                donations: Default::default(),
                widget_periods: Default::default(),
                widget_values: Default::default(),
                scene_audio: Default::default(),
                transition: Transition::Cut,
                transition_ms: 300,
                transition_media: TransitionMedia::default(),
                overlays: Default::default(),
                snapping: SnapSettings::default(),
                capture_resolution: CaptureResolution::default(),
                accent: [208, 188, 255],
                chat_font_size: 14.,
                timestamps_24h: false,
                auto_game_capture: true,
                manual_capture_sources: Vec::new(),
                windows_capture_monitors: Default::default(),
                windows_auto_sources: Default::default(),
                windows_capture_cursor: false,
                recording_directory: String::new(),
                multitrack_recording: false,
                monitor_device: String::new(),
                recording_encoder: RecordingEncoder::Auto,
                auto_game_audio: true,
            },
        }
    }
}

impl Project {
    pub fn id(&mut self) -> Id {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
    pub fn scene(&self, id: Id) -> Option<&Scene> {
        self.scenes.iter().find(|s| s.id == id)
    }
    pub fn source(&self, id: Id) -> Option<&Source> {
        self.sources.iter().find(|s| s.id == id)
    }
    /// Gives every alert sound or video with audio its own mixer channel, so it can be heard.
    /// Returns whether any channel was added.
    pub fn ensure_alert_channels(&mut self) -> bool {
        let mut wanted: Vec<Id> = self.rules.iter().flat_map(|r| r.actions.iter()).filter_map(|a| match a {
            Action::DisplayAlert { media, .. } => *media,
            Action::PlaySound { source } | Action::PlayMedia { source } => Some(*source),
            _ => None,
        }).collect();
        for s in &self.sources {
            if let SourceKind::Widget(w) = &s.kind && w.kind == widgets::Kind::AlertBox {
                wanted.extend(w.alerts.iter().filter_map(|a| a.media));
            }
        }
        wanted.sort_unstable();
        wanted.dedup();
        let mut added = false;
        for id in wanted {
            let Some(name) = self.source(id).and_then(|s| match &s.kind {
                SourceKind::VideoFile(v) if v.audio => Some(s.name.clone()),
                _ => None,
            }) else { continue };
            if self.audio.iter().any(|c| c.media_source == Some(id)) {
                continue;
            }
            let channel = self.id();
            self.audio.push(AudioChannel { id: channel, name, gain_db: 0., muted: false, tracks: [true, false, false, false, false, false], delay_ms: 0, monitor: true, monitor_only: false, noise_suppression: Default::default(), media_source: Some(id) });
            added = true;
        }
        added
    }
    /// Whether this channel plays an alert's sound or media (not a transition or ordinary video).
    pub fn is_alert_channel(&self, c: &AudioChannel) -> bool {
        let Some(id) = c.media_source else { return false };
        let t = &self.preferences.transition_media;
        if t.stinger == Some(id) || t.screen == Some(id) || c.name == "Stinger" || c.name == "Transition screen" {
            return false;
        }
        let triggered = self.source(id).is_some_and(|s| matches!(&s.kind, SourceKind::VideoFile(v) if v.automation_only || v.trigger.is_some()));
        let in_alert_box = self.sources.iter().any(|s| matches!(&s.kind, SourceKind::Widget(w) if w.kind == widgets::Kind::AlertBox && w.alerts.iter().any(|a| a.media == Some(id))));
        let in_rule = self.rules.iter().any(|r| r.actions.iter().any(|a| match a {
            Action::DisplayAlert { media, .. } => *media == Some(id),
            Action::PlaySound { source } | Action::PlayMedia { source } => *source == id,
            _ => false,
        }));
        triggered || in_alert_box || in_rule
    }
    /// Channels as the mixer shows them for a scene, before the shared alert control.
    pub fn mixer_channels(&self, scene: Id) -> Vec<AudioChannel> {
        let mut channels = self.audio.clone();
        if let Some(volumes) = self.preferences.scene_audio.get(&scene) {
            for c in &mut channels {
                // Alert sounds are set by the shared Alerts strip, not per scene.
                if self.is_alert_channel(c) {
                    continue;
                }
                if let Some(v) = volumes.get(&c.id) {
                    c.gain_db = v.gain_db;
                    c.muted = v.muted;
                }
            }
        }
        // File audio follows the canvas scene just like its picture; a playing transition
        // (stinger or screen) is heard while it is on screen.
        // Alert media only makes sound while an alert plays it, whatever the scene.
        for c in &mut channels {
            if c.media_source.is_some()
                && !self.is_alert_channel(c)
                && !self.preferences.overlays.values().any(|o| Some(o.source) == c.media_source)
                && !self.scene(scene).is_some_and(|s| {
                    s.layers
                        .iter()
                        .any(|l| l.visible && (Some(l.source) == c.media_source || self.source(l.source).is_some_and(|source|matches!(&source.kind,SourceKind::Widget(w) if w.kind==widgets::Kind::AlertBox && (w.alerts.iter().any(|a|a.media==c.media_source)||self.rules.iter().any(|r|r.actions.iter().any(|a|matches!(a,Action::DisplayAlert{source,media,..} if *source==l.source && *media==c.media_source))))))))
                })
            {
                c.muted = true;
            }
        }
        channels
    }
    /// What each channel actually plays at in a scene, including the shared alert control.
    pub fn audio_for_scene(&self, scene: Id) -> Vec<AudioChannel> {
        let mut channels = self.mixer_channels(scene);
        let bus = &self.preferences.alert_bus;
        for i in 0..channels.len() {
            if self.is_alert_channel(&channels[i]) {
                let c = &mut channels[i];
                c.gain_db = if c.gain_db <= -100. { c.gain_db } else { (c.gain_db + bus.gain_db).max(-100.) };
                c.muted |= bus.muted;
            }
        }
        channels
    }
    pub fn validate(&self) -> Result<()> {
        self.validate_workspace()?;
        for s in self.preferences.browser_sources.values() {
            s.validate()?;
        }
        for s in self.preferences.chroma_keys.values() {
            s.validate()?;
        }
        if self.schema_version != PROJECT_VERSION {
            bail!("Unsupported project version {}", self.schema_version);
        }
        if self.canvases.is_empty() || self.scenes.is_empty() {
            bail!("A project needs a canvas and a scene");
        }
        for source in &self.sources {
            if let SourceKind::VideoCamera(settings) = &source.kind {
                settings.validate()?;
            }
        }
        let mut ids = BTreeSet::new();
        for id in self
            .sources
            .iter()
            .map(|s| s.id)
            .chain(self.scenes.iter().map(|s| s.id))
            .chain(self.canvases.iter().map(|s| s.id))
            .chain(self.audio.iter().map(|s| s.id))
            .chain(self.rules.iter().map(|s| s.id))
        {
            if !ids.insert(id) || id >= self.next_id {
                bail!("Invalid or duplicate identifier {id}");
            }
        }
        for c in &self.canvases {
            if !(16..=4096).contains(&c.width)
                || !(16..=4096).contains(&c.height)
                || !(1..=120).contains(&c.fps)
                || self.scene(c.scene).is_none()
            {
                bail!("Invalid canvas {}", c.name);
            }
        }
        for s in &self.scenes {
            for l in &s.layers {
                if self.source(l.source).is_none()
                    || !l.crop.iter().all(|v|v.is_finite() && *v>=0. && *v<1.)
                    || l.crop[0]+l.crop[2]>=0.995 || l.crop[1]+l.crop[3]>=0.995
                    || !l.rect.iter().all(|v| v.is_finite())
                    || l.rect[2] <= 0.
                    || l.rect[3] <= 0.
                    || !l.opacity.is_finite()
                    || !(0.0..=1.0).contains(&l.opacity)
                {
                    bail!("Invalid layer in {}", s.name);
                }
            }
        }
        for a in &self.audio {
            if !a.gain_db.is_finite() || !(-100.0..=12.0).contains(&a.gain_db) {
                bail!("Invalid audio gain");
            }
        }
        for volumes in self.preferences.scene_audio.values() {
            for v in volumes.values() {
                if !v.gain_db.is_finite() || !(-100.0..=12.0).contains(&v.gain_db) {
                    bail!("Invalid scene audio volume");
                }
            }
        }
        for s in &self.sources {
            if let SourceKind::VideoFile(v) = &s.kind
                && (!v.gain.is_finite() || !(0.0..=2.0).contains(&v.gain) || v.duration_secs > 300)
            {
                bail!("Invalid alert media volume or duration");
            }
        }
        for r in &self.rules {
            if r.actions.len() > 64
                || r.cooldown_ms > 3600000
                || r.options.per_user_ms > 3600000
                || r.options.hotkey > 12
            {
                bail!("Automation exceeds limits");
            }

            for action in &r.actions {
                match action {
                    Action::SwitchScene { canvas, scene }
                        if !self.canvases.iter().any(|c| c.id == *canvas)
                            || self.scene(*scene).is_none() =>
                    {
                        bail!("Invalid scene action")
                    }
                    Action::DisplayAlert{source,media,..} if !self.source(*source).is_some_and(|s|matches!(&s.kind,SourceKind::Widget(w) if w.kind==widgets::Kind::AlertBox)) || media.is_some_and(|id|self.source(id).is_none())=>{bail!("Alert references a missing alert box or media source")},
                    Action::SendChat{text,platforms} if text.trim().is_empty()||text.chars().count()>500||platforms.iter().any(|p|!matches!(p,Platform::Twitch|Platform::YouTube|Platform::Kick))=>{bail!("Invalid automation chat message or platform")},
                    Action::Wait { milliseconds } if *milliseconds > 3600000 => {
                        bail!("Automation wait exceeds one hour")
                    }
                    Action::ShowAlert { seconds, .. } | Action::DisplayAlert {seconds,..} if !(1..=60).contains(seconds) => {
                        bail!("Invalid alert duration")
                    }
                    _ => (),
                }
            }
        }
        if !self.preferences.chat_font_size.is_finite()
            || !(10.0..=28.0).contains(&self.preferences.chat_font_size)
        {
            bail!("Invalid chat font size");
        }
        Ok(())
    }
    pub fn load(path: &Path) -> Result<Self> {
        let mut project: Self =
            serde_json::from_slice(&std::fs::read(path).context("Read project")?)
                .context("Parse project")?;
        for (index, canvas) in project.canvases.iter_mut().enumerate() {
            if matches!(
                canvas.name.as_str(),
                "Main" | "Main broadcast" | "Vertical" | "Main Canvas" | "Vertical Canvas"
            ) {
                canvas.name = format!("Canvas {}", index + 1);
            }
        }
        project.ensure_alert_channels();
        project.validate()?;
        Ok(project)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension("json.tmp");
        let mut file = std::fs::File::create(&temporary)?;
        use std::io::Write;
        file.write_all(&serde_json::to_vec_pretty(self)?)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)?;
        Ok(())
    }
    pub fn apply(&mut self, action: &Action) {
        if let Action::SwitchScene { canvas, scene } = action
            && self.scene(*scene).is_some()
            && let Some(c) = self.canvases.iter_mut().find(|c| c.id == *canvas)
        {
            c.scene = *scene;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_preferences_load_and_manual_capture_mode_roundtrips() {
        let mut value = serde_json::to_value(Project::default()).unwrap();
        value["preferences"]
            .as_object_mut()
            .unwrap()
            .remove("manual_capture_sources");
        value["preferences"]
            .as_object_mut()
            .unwrap()
            .remove("recording_directory");
        value["preferences"]
            .as_object_mut()
            .unwrap()
            .remove("recording_encoder");
        let mut project: Project = serde_json::from_value(value).unwrap();
        assert_eq!(
            project.preferences.recording_encoder,
            RecordingEncoder::Auto
        );
        project.preferences.recording_encoder = RecordingEncoder::Software;
        assert!(project.preferences.recording_directory.is_empty());
        assert!(project.preferences.manual_capture_sources.is_empty());
        project.preferences.manual_capture_sources.push(42);
        let decoded: Project =
            serde_json::from_str(&serde_json::to_string(&project).unwrap()).unwrap();
        assert_eq!(decoded.preferences.manual_capture_sources, vec![42]);
        assert_eq!(
            decoded.preferences.recording_encoder,
            RecordingEncoder::Software
        );
    }

    #[test]
    fn capture_source_roundtrip_and_shared_identity() {
        let mut project = Project::default();
        let legacy = serde_json::to_string(&project).unwrap();
        serde_json::from_str::<Project>(&legacy)
            .unwrap()
            .validate()
            .unwrap();
        project.sources[0].kind = SourceKind::ScreenCapture;
        let decoded: Project =
            serde_json::from_str(&serde_json::to_string(&project).unwrap()).unwrap();
        decoded.validate().unwrap();
        assert!(matches!(decoded.sources[0].kind, SourceKind::ScreenCapture));
        assert_eq!(
            decoded.scenes[0].layers[0].source,
            decoded.scenes[1].layers[0].source
        );
    }
    #[test]
    fn shared_source_and_independent_scenes() {
        let mut p = Project::default();
        p.validate().unwrap();
        assert_eq!(
            p.scene(p.canvases[0].scene).unwrap().layers[0].source,
            p.scene(p.canvases[1].scene).unwrap().layers[0].source
        );
        p.apply(&Action::SwitchScene {
            canvas: 8,
            scene: 6,
        });
        assert_eq!(p.canvases[0].scene, 6);
        assert_eq!(p.canvases[1].scene, 7);
    }
    #[test]
    fn malformed_project_is_rejected() {
        let mut p = Project::default();
        p.canvases[0].scene = 1000;
        assert!(p.validate().is_err());
        p = Project::default();
        p.sources[1].id = 1;
        assert!(p.validate().is_err());
    }
    #[test]
    fn chat_order_dedupe_and_capacity() {
        let mut f = ChatFeed::new(2);
        let msg = |id: &str, t| ChatMessage {
            metadata: Default::default(),
            platform: Platform::Twitch,
            id: id.into(),
            author: "a".into(),
            text: "hi".into(),
            received_ms: t,
            demo: false,
        };
        assert!(f.push(msg("b", 2)));
        assert!(f.push(msg("a", 1)));
        assert!(!f.push(msg("b", 3)));
        f.push(msg("c", 3));
        assert_eq!(
            f.messages().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["b", "c"]
        );
    }
    #[test]
    fn audio_tracks_mute_gain_and_clipping() {
        let mut p = Project::default();
        p.audio[0].gain_db = 0.;
        let a = [0.75, -0.75];
        let b = [0.75, -0.75];
        assert_eq!(
            mix_audio(&p.audio, &[(10, &a), (11, &b)], 0, 2).unwrap(),
            vec![1., -1.]
        );
        p.audio[1].muted = true;
        assert_eq!(mix_audio(&p.audio, &[(10, &a), (11, &b)], 0, 2).unwrap(), a);
        assert_eq!(
            mix_audio(&p.audio, &[(10, &a)], 2, 2).unwrap(),
            vec![0., 0.]
        );
        assert!(mix_audio(&p.audio, &[], 6, 2).is_err());
        p.audio[0].monitor_only=true;
        assert_eq!(mix_audio(&p.audio,&[(10,&a)],0,2).unwrap(),vec![0.,0.]);
    }
    #[test]
    fn alerts_dedupe_cooldown_and_command_matching() {
        let p = Project::default();
        let mut e = AlertEngine::default();
        let mut event = Event {
            facts: Default::default(),
            reward_id: String::new(),
            id: "a".into(),
            platform: Platform::Twitch,
            kind: EventKind::Follow,
            author: "Moe".into(),
            text: String::new(),
            at_ms: 0,
        };
        let actions = e.dispatch(&p.rules, &event);
        assert_eq!(actions.len(), 1);
        assert!(matches!(&actions[0],Action::ShowAlert{template,..} if template.contains("Moe")));
        assert!(e.dispatch(&p.rules, &event).is_empty());
        event.id = "b".into();
        event.at_ms = 1000;
        assert!(e.dispatch(&p.rules, &event).is_empty());
        event.id = "c".into();
        event.at_ms = 3000;
        assert_eq!(e.dispatch(&p.rules, &event).len(), 1);
        let mut rules = p.rules;
        rules[0].event = EventKind::ChatCommand;
        rules[0].cooldown_ms = 0;
        event.kind = EventKind::ChatCommand;
        event.id = "d".into();
        event.text = "!helloextra".into();
        assert!(e.dispatch(&rules, &event).is_empty());
        event.id = "e".into();
        event.text = "!hello there".into();
        assert_eq!(e.dispatch(&rules, &event).len(), 1);
    }
    #[test]
    fn project_roundtrip_and_invalid_save() {
        let p = Project::default();
        let path =
            std::env::temp_dir().join(format!("auroracast-test-{}.json", std::process::id()));
        p.save(&path).unwrap();
        let q = Project::load(&path).unwrap();
        assert_eq!(q.canvases.len(), 2);
        let mut bad = q;
        bad.canvases[0].width = 0;
        assert!(bad.save(&path).is_err());
        assert_eq!(Project::load(&path).unwrap().canvases[0].width, 1920);
        std::fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
mod layer_edit_tests {
    use super::*;
    #[test]
    fn layer_order_preserves_properties_and_selection() {
        let mut scene = Scene {
            id: 10,
            name: "Layers".into(),
            layers: (1..=4)
                .map(|source| Layer { crop: [0.;4], group: 0,
                    source,
                    rect: [0.1, 0.2, 0.3, 0.4],
                    opacity: 0.6,
                    visible: true,
                    locked: false,
                })
                .collect(),
        };
        let original = serde_json::to_value(&scene.layers[1]).unwrap();
        let selected = scene.reorder_layer(1, LayerOrder::Front).unwrap();
        assert_eq!(selected, 3);
        assert_eq!(
            scene.layers.iter().map(|l| l.source).collect::<Vec<_>>(),
            vec![1, 3, 4, 2]
        );
        assert_eq!(
            serde_json::to_value(&scene.layers[selected]).unwrap(),
            original
        );
        let selected = scene.reorder_layer(selected, LayerOrder::Lower).unwrap();
        assert_eq!(selected, 2);
        let selected = scene.reorder_layer(selected, LayerOrder::Back).unwrap();
        assert_eq!(selected, 0);
        let selected = scene.reorder_layer(selected, LayerOrder::Raise).unwrap();
        assert_eq!(selected, 1);
        assert_eq!(scene.layers[selected].source, 2);
        scene.layers[selected].locked = true;
        assert_eq!(scene.reorder_layer(selected, LayerOrder::Front), None);
        assert_eq!(scene.reorder_layer(100, LayerOrder::Front), None);
        assert_eq!(scene.reorder_layer(3, LayerOrder::Raise), None);
    }
    #[test]
    fn old_projects_gain_snapping_and_new_settings_and_order_round_trip() {
        let mut project = Project::default();
        let mut legacy = serde_json::to_value(&project).unwrap();
        legacy["preferences"]
            .as_object_mut()
            .unwrap()
            .remove("snapping");
        let restored: Project = serde_json::from_value(legacy).unwrap();
        assert_eq!(restored.preferences.snapping, SnapSettings::default());
        project.preferences.snapping = SnapSettings {
            enabled: false,
            canvas_edges: false,
            canvas_center: true,
            sources: false,
            distance: 12.,
        };
        let scene = &mut project.scenes[0];
        scene.layers[0].locked = false;
        let source = scene.layers[0].source;
        scene.reorder_layer(0, LayerOrder::Front);
        let restored: Project =
            serde_json::from_str(&serde_json::to_string(&project).unwrap()).unwrap();
        assert_eq!(restored.preferences.snapping, project.preferences.snapping);
        assert_eq!(restored.scenes[0].layers.last().unwrap().source, source);
        restored.validate().unwrap();
    }
}

#[cfg(test)]
mod camera_model_tests {
    use super::*;
    #[test]
    fn camera_auto_default_migration_and_manual_override() {
        let old = r#"{"device":"/dev/video3","width":1280,"height":720,"fps":30,"enabled":true}"#;
        let mut settings: CameraSettings = serde_json::from_str(old).unwrap();
        assert!(settings.auto_mode);
        assert_eq!(
            (settings.width, settings.height, settings.fps),
            (1280, 720, 30)
        );
        settings.auto_mode = false;
        let restored: CameraSettings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(restored, settings);
    }
    #[test]
    fn camera_settings_roundtrip_and_validation() {
        let mut p = Project::default();
        p.sources[1].kind = SourceKind::VideoCamera(CameraSettings {
            device: "/dev/v4l/by-id/test".into(),
            enabled: true,
            crop: [12, 4, 8, 2],
            flip: true,
            ..Default::default()
        });
        let q: Project = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(q.sources[1].kind, p.sources[1].kind);
        q.validate().unwrap();
        if let SourceKind::VideoCamera(s) = &mut p.sources[1].kind {
            s.fps = 0;
        }
        assert!(p.validate().is_err());
    }
}

#[cfg(test)]
mod scene_audio_tests {
    use super::*;
    #[test]
    fn volumes_are_saved_per_scene_and_media_follows_its_picture() {
        let mut p = Project::default();
        p.preferences.scene_audio.entry(4).or_default().insert(
            11,
            SceneVolume {
                gain_db: -12.,
                muted: false,
            },
        );
        p.preferences.scene_audio.entry(7).or_default().insert(
            11,
            SceneVolume {
                gain_db: 0.,
                muted: true,
            },
        );
        assert_eq!(p.audio_for_scene(4)[1].gain_db, -12.);
        assert!(p.audio_for_scene(7)[1].muted);
        assert_eq!(p.audio[1].gain_db, 0.);
        let source = p.id();
        p.sources.push(Source {
            id: source,
            name: "Background".into(),
            kind: SourceKind::VideoFile(VideoSettings::default()),
        });
        let id = p.id();
        let mut channel = p.audio[2].clone();
        channel.id = id;
        channel.media_source = Some(source);
        p.audio.push(channel);
        p.scenes[0].layers.push(layer(source, [0., 0., 1., 1.]));
        assert!(!p.audio_for_scene(4).last().unwrap().muted);
        assert!(p.audio_for_scene(7).last().unwrap().muted);
        let saved = serde_json::to_vec(&p).unwrap();
        let q: Project = serde_json::from_slice(&saved).unwrap();
        q.validate().unwrap();
        assert_eq!(q.audio_for_scene(4)[1].gain_db, -12.);
        assert!(q.audio_for_scene(7)[1].muted);
    }
}

#[cfg(test)]
mod windows_monitor_preferences_tests {
    use super::*;
    #[test]
    fn old_projects_and_distinct_monitor_selections_round_trip() {
        let mut old=serde_json::to_value(Project::default()).unwrap();
        old["preferences"].as_object_mut().unwrap().remove("windows_capture_monitors");
        old["preferences"].as_object_mut().unwrap().remove("windows_auto_sources");
        old["preferences"].as_object_mut().unwrap().remove("windows_capture_cursor");
        let mut project:Project=serde_json::from_value(old).unwrap();
        assert!(project.preferences.windows_capture_monitors.is_empty());
        assert!(project.preferences.windows_auto_sources.is_empty());
        assert!(!project.preferences.windows_capture_cursor);
        project.preferences.windows_capture_monitors.insert(42,r"\\.\DISPLAY1".into());
        project.preferences.windows_capture_monitors.insert(43,r"\\.\DISPLAY2".into());
        let restored:Project=serde_json::from_slice(&serde_json::to_vec(&project).unwrap()).unwrap();
        assert_eq!(restored.preferences.windows_capture_monitors,project.preferences.windows_capture_monitors);
        assert_ne!(restored.preferences.windows_capture_monitors[&42],restored.preferences.windows_capture_monitors[&43]);
    }
}
#[cfg(test)]
mod transition_media_tests {
    use super::*;
    #[test]
    fn alert_bus_controls_alert_sounds_but_not_the_stinger() {
        let mut p = Project::default();
        let mut channel = |p: &mut Project, name: &str| {
            let id = p.id();
            p.sources.push(Source { id, name: name.into(), kind: SourceKind::VideoFile(VideoSettings { automation_only: true, ..Default::default() }) });
            let audio = p.id();
            p.audio.push(AudioChannel { id: audio, name: name.into(), gain_db: -3., muted: false, tracks: [true, false, false, false, false, false], delay_ms: 0, monitor: false, monitor_only: false, noise_suppression: Default::default(), media_source: Some(id) });
            audio
        };
        let alert = channel(&mut p, "Follow sound");
        let stinger = channel(&mut p, "Stinger");
        p.preferences.alert_bus = AlertBus { gain_db: -6., muted: false };
        let scene = p.canvases[0].scene;
        let find = |p: &Project, id| p.audio_for_scene(scene).into_iter().find(|c| c.id == id).unwrap();
        assert_eq!(find(&p, alert).gain_db, -9.);
        assert_eq!(find(&p, stinger).gain_db, -3.);
        assert!(p.is_alert_channel(&p.audio[p.audio.len() - 2]) && !p.is_alert_channel(p.audio.last().unwrap()));
        p.preferences.alert_bus.muted = true;
        assert!(find(&p, alert).muted);
        assert_eq!(p.mixer_channels(scene).into_iter().find(|c| c.id == alert).unwrap().gain_db, -3.);
        // Per-scene mutes and "not in this scene" never silence an alert sound.
        p.preferences.alert_bus.muted = false;
        p.preferences.scene_audio.entry(scene).or_default().insert(alert, SceneVolume { gain_db: -12., muted: true });
        assert!(!find(&p, alert).muted && find(&p, alert).gain_db == -9.);
        assert!(find(&p, stinger).muted, "the stinger still follows the screen");
    }
    #[test]
    #[ignore = "Checks a real project (ROSORAH_PROJECT) on request"]
    fn real_project_alert_audio() {
        let p = Project::load(std::path::Path::new(&std::env::var("ROSORAH_PROJECT").unwrap())).unwrap();
        for canvas in &p.canvases {
            for c in p.audio_for_scene(canvas.scene).iter().filter(|c| p.is_alert_channel(c)) {
                println!("scene {} · {} (source {:?}) · {} dB · muted {} · monitor {}", canvas.scene, c.name, c.media_source, c.gain_db, c.muted, c.monitor);
            }
        }
    }
    #[test]
    fn alert_media_without_a_channel_gets_one() {
        let mut p = Project::default();
        let id = p.id();
        p.sources.push(Source { id, name: "Follow.mp4".into(), kind: SourceKind::VideoFile(VideoSettings { automation_only: true, audio: true, ..Default::default() }) });
        let rule = p.id();
        p.rules.push(Rule { options: Default::default(), conditions: Default::default(), id: rule, name: "Follow".into(), enabled: true, platform: None, event: EventKind::Follow, command: String::new(), cooldown_ms: 0, actions: vec![Action::PlayMedia { source: id }] });
        assert!(p.ensure_alert_channels());
        let c = p.audio.iter().find(|c| c.media_source == Some(id)).unwrap();
        assert!(c.monitor && c.tracks[0] && p.is_alert_channel(c));
        assert!(!p.ensure_alert_channels());
    }
    #[test]
    fn stinger_sound_plays_only_while_on_screen_and_old_projects_load() {
        let mut p = Project::default();
        let id = p.id();
        p.sources.push(Source { id, name: "Stinger".into(), kind: SourceKind::VideoFile(VideoSettings { automation_only: true, ..Default::default() }) });
        let audio = p.id();
        p.audio.push(AudioChannel { id: audio, name: "Stinger".into(), gain_db: 0., muted: false, tracks: [true, false, false, false, false, false], delay_ms: 0, monitor: false, monitor_only: false, noise_suppression: Default::default(), media_source: Some(id) });
        let scene = p.canvases[0].scene;
        let muted = |p: &Project| p.audio_for_scene(scene).iter().find(|c| c.id == audio).unwrap().muted;
        assert!(muted(&p), "not in the scene and not playing");
        p.preferences.overlays.insert(p.canvases[0].id, Overlay { source: id, opacity: 1. });
        assert!(!muted(&p), "heard while the transition shows");
        assert!(Transition::Fade.blends() && !Transition::Stinger.blends() && !Transition::Screen.blends());
        let mut json = serde_json::to_value(&p).unwrap();
        json["preferences"].as_object_mut().unwrap().remove("transition_media");
        assert!(json["preferences"].get("overlays").is_none(), "overlays are never saved");
        let old: Project = serde_json::from_value(json).unwrap();
        assert_eq!(old.preferences.transition_media, TransitionMedia::default());
    }
}
