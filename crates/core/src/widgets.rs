use crate::Platform;
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Chat,
    SubGoal,
    DonationGoal,
    Timer,
    #[default]
    Counter,
    Shoutout,
    QrCode,
    AlertBox,
    Sports,
    EventTicker,
    LatestSupporter,
    StreamLabels,
    Schedule,
    NowPlaying,
    Poll,
    Checklist,
    Announcements,
    Credits,
    CommunityGoal,
    /// Free text placed on the canvas, in any bundled or installed font.
    Text,
}
impl Kind {
    pub const ALL: [Self; 20] = [
        Self::Text,
        Self::Chat,
        Self::SubGoal,
        Self::DonationGoal,
        Self::Timer,
        Self::Counter,
        Self::Shoutout,
        Self::QrCode,
        Self::AlertBox,
        Self::Sports,
        Self::EventTicker,
        Self::LatestSupporter,
        Self::StreamLabels,
        Self::Schedule,
        Self::NowPlaying,
        Self::Poll,
        Self::Checklist,
        Self::Announcements,
        Self::Credits,
        Self::CommunityGoal,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Chat => "Chat",
            Self::SubGoal => "Subscriber goal",
            Self::DonationGoal => "Donation goal",
            Self::Timer => "Countdown",
            Self::Counter => "Death / custom counter",
            Self::Shoutout => "Shoutout",
            Self::QrCode => "QR code",
            Self::AlertBox => "Alert Box",
            Self::Sports => "Sports scores · NFL / NBA",
            Self::EventTicker => "Recent events",
            Self::LatestSupporter => "Latest supporter",
            Self::StreamLabels => "Stream label",
            Self::Schedule => "Schedule card",
            Self::NowPlaying => "Now playing",
            Self::Poll => "Poll / prediction card",
            Self::Checklist => "Challenge checklist",
            Self::Announcements => "Rotating announcements",
            Self::Credits => "Credits roll",
            Self::CommunityGoal => "Community unlock goal",
            Self::Text => "Text",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub alert_layout: AlertLayout,
    pub automation_only: bool,
    pub kind: Kind,
    pub layout_width:u32,
    pub layout_height:u32,
    pub label: String,
    pub target: u64,
    pub initial: u64,
    pub seconds: u32,
    pub platforms: Vec<Platform>,
    pub currency: String,
    pub foreground: [u8; 4],
    pub background: [u8; 4],
    pub alerts: Vec<AlertStyle>,
    pub fit_media: bool,
    pub timer_up: bool,
    pub goal_event: Option<crate::EventKind>,
    pub theme: Theme,
    pub font: FontFace,
    pub font_size: u32,
    pub bold: bool,
    pub accent: [u8; 4],
    pub radius: u32,
    pub content: String,
    pub interval: u32,
    pub goal_style: GoalStyle,
    pub period: Period,
    pub reset_day: u32,
    pub deadline: String,
    pub sports: Sports,
    pub layout: Layout,
    pub spacing: u32,
    pub reverse: bool,
    #[serde(default = "Music::legacy")]
    pub music: Music,
    /// Text widget: a font file path (bundled or installed); empty uses Roboto.
    pub text_font: String,
    pub align: TextAlign,
    pub outline: [u8; 4],
    /// Outline thickness in pixels; 0 turns it off.
    pub outline_width: u32,
    pub shadow: bool,
    /// Chat: seconds each message stays on screen; 0 keeps it until newer ones push it out.
    pub chat_hide_after: u32,
    /// Chat: show exactly this many of the newest messages, sized to fit; 0 fits as many as the size allows.
    pub chat_rows: u32,
    /// Chat: messages slide and fade in, and out when they go.
    pub chat_animate: bool,
    /// Chat: badges (moderator, subscriber…) before names.
    pub chat_badges: bool,
    /// Chat: names in each chatter's own colour.
    pub chat_name_colors: bool,
    /// Chat: set apart paid, first-time and highlighted messages.
    pub chat_highlights: bool,
    /// Chat: leaves out well-known bots such as Nightbot and StreamElements.
    pub chat_hide_bots: bool,
    /// Chat: chatters whose messages never show, separated by commas or lines.
    pub chat_hidden: String,
}
/// Bots many channels use; their messages are left out of chat widgets unless asked for.
pub const KNOWN_BOTS: [&str; 22] = [
    "nightbot", "streamelements", "streamlabs", "moobot", "fossabot", "wizebot", "sery_bot",
    "soundalerts", "streamstickers", "kofistreambot", "botrix", "botrixoficial", "kickbot",
    "commanderroot", "pokemoncommunitygame", "lumiastream", "blerp", "own3d", "deepbot",
    "phantombot", "coebot", "songlistbot",
];
/// A chatter's name as it's compared: no "@", spaces at the ends or capitals.
fn chatter(name: &str) -> String {
    name.trim().trim_start_matches('@').trim().to_lowercase()
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            alert_layout: AlertLayout::Over, automation_only:false,
            kind: Kind::Counter,
            layout_width:960,layout_height:400,
            label: "Deaths".into(),
            target: 100,
            initial: 0,
            seconds: 300,
            platforms: vec![Platform::Twitch, Platform::YouTube, Platform::Kick, Platform::TikTok],
            currency: "USD".into(),
            foreground: [255, 255, 255, 255],
            background: [20, 26, 40, 210],
            alerts: Vec::new(),
            fit_media: true,
            timer_up: false,
            goal_event: None,
            theme: Theme::Material,
            font: FontFace::Sans,
            font_size: 36,
            bold: false,
            accent: [139, 112, 240, 255],
            radius: 16,
            content: String::new(),
            interval: 8,
            goal_style: GoalStyle::Bar,
            period: Period::Manual,
            reset_day: 1,
            deadline: String::new(),
            sports: Sports::default(),
            layout: Layout::Cards,
            spacing: 10,
            reverse: false,
            music: Music::default(),
            text_font: String::new(),
            align: TextAlign::Left,
            outline: [0, 0, 0, 255],
            outline_width: 0,
            shadow: false,
            chat_hide_after: 0,
            chat_rows: 0,
            chat_animate: true,
            chat_badges: true,
            chat_name_colors: true,
            chat_highlights: true,
            chat_hide_bots: true,
            chat_hidden: String::new(),
        }
    }
}

macro_rules! choices {
    ($name:ident, $default:ident, $($variant:ident => $label:literal),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
        pub enum $name { $($variant),+ }
        impl Default for $name { fn default() -> Self { Self::$default } }
        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];
            pub fn label(self) -> &'static str { match self { $(Self::$variant => $label),+ } }
        }
    }
}
choices!(Theme, Material, Material=>"Material", Unified=>"Unified chat", Windows98=>"Windows 98", WindowsXP=>"Windows XP", Bubbles=>"Phone bubbles", Transparent=>"Minimal transparent", Cards=>"Stream cards", Terminal=>"Retro terminal", Pixel=>"Pixel RPG", Broadcast=>"Broadcast sports", Team=>"Team colors", Led=>"Retro LED", CozyCafe=>"Cozy café", Kawaii=>"Pastel kawaii", Cottagecore=>"Cottagecore", Cyberpunk=>"Cyberpunk neon", Synthwave=>"Synthwave", SciFiHud=>"Sci-fi HUD", Arcade=>"Arcade", Fantasy=>"Fantasy parchment", Neubrutalism=>"Neubrutalism", CleanLight=>"Clean light", GameBoy=>"Game Boy", Vaporwave=>"Vaporwave", Spooky=>"Spooky", Winter=>"Winter holiday");
choices!(FontFace, Sans, Sans=>"Roboto", Mono=>"Monospace");
choices!(TextAlign, Left, Left=>"Left", Center=>"Center", Right=>"Right");
choices!(Layout, Cards, Cards=>"Cards", Lines=>"Compact lines", Bubbles=>"Bubbles");
choices!(GoalStyle, Bar, Bar=>"Horizontal bar", Vertical=>"Vertical bar", Ring=>"Progress ring", Thermometer=>"Thermometer", Milestones=>"Milestones", Segments=>"Segmented", Counter=>"Counter card", Hearts=>"Pixel hearts", Garden=>"Growing garden");
choices!(Period, Manual, Manual=>"Manual reset", Stream=>"Per stream", Week=>"Weekly", Month=>"Monthly", Deadline=>"Until date");
choices!(League, Nba, Nfl=>"NFL", Nba=>"NBA");
choices!(ScoreLayout, Cards, Cards=>"Rotating cards", Board=>"Scoreboard", Ticker=>"Scrolling ticker");
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sports {
    pub leagues: Vec<League>,
    pub teams: String,
    pub live: bool,
    pub upcoming: bool,
    pub finals: bool,
    pub layout: ScoreLayout,
    pub speed: u32,
    pub rightward: bool,
}
impl Default for Sports {
    fn default() -> Self {
        Self {
            leagues: vec![League::Nfl, League::Nba],
            teams: String::new(),
            live: true,
            upcoming: true,
            finals: true,
            layout: ScoreLayout::Cards,
            speed: 60,
            rightward: false,
        }
    }
}
impl Settings {
    /// Whether a chat widget leaves out this chatter's messages.
    pub fn hides_chatter(&self, name: &str) -> bool {
        let name = chatter(name);
        (self.chat_hide_bots && KNOWN_BOTS.contains(&name.as_str()))
            || self.chat_hidden.split([',', '\n']).any(|n| !n.trim().is_empty() && chatter(n) == name)
    }
    /// Adds a chatter to this widget's hidden list, unless they're on it already.
    pub fn hide_chatter(&mut self, name: &str) {
        let name = name.trim().trim_start_matches('@').trim();
        if name.is_empty() || self.chat_hidden.split([',', '\n']).any(|n| chatter(n) == chatter(name)) {
            return;
        }
        if !self.chat_hidden.trim().is_empty() {
            self.chat_hidden = format!("{}, ", self.chat_hidden.trim_end().trim_end_matches(','));
        }
        self.chat_hidden.push_str(name);
    }
    pub fn is_goal(&self) -> bool {
        matches!(
            self.kind,
            Kind::SubGoal | Kind::DonationGoal | Kind::CommunityGoal
        )
    }
    pub fn apply_theme(&mut self, theme: Theme) {
        self.theme = theme;
        let (fg, bg, accent, radius, font) = match theme {
            Theme::Windows98 => (
                [0, 0, 0, 255],
                [192, 192, 192, 255],
                [0, 0, 128, 255],
                0,
                FontFace::Sans,
            ),
            Theme::WindowsXP => (
                [15, 25, 40, 255],
                [236, 233, 216, 255],
                [0, 84, 227, 255],
                8,
                FontFace::Sans,
            ),
            Theme::Bubbles => (
                [255, 255, 255, 255],
                [0, 0, 0, 0],
                [0, 122, 255, 240],
                26,
                FontFace::Sans,
            ),
            Theme::Transparent => (
                [255, 255, 255, 255],
                [0, 0, 0, 0],
                [139, 112, 240, 255],
                0,
                FontFace::Sans,
            ),
            Theme::Terminal | Theme::Led => (
                [60, 255, 115, 255],
                [4, 14, 8, 230],
                [60, 255, 115, 255],
                0,
                FontFace::Mono,
            ),
            Theme::Pixel => (
                [255, 242, 190, 255],
                [25, 21, 52, 240],
                [233, 173, 78, 255],
                0,
                FontFace::Mono,
            ),
            Theme::Broadcast => (
                [255, 255, 255, 255],
                [12, 25, 44, 245],
                [235, 57, 64, 255],
                4,
                FontFace::Sans,
            ),
            Theme::Team => (
                [255, 255, 255, 255],
                [15, 28, 50, 230],
                [35, 132, 230, 255],
                12,
                FontFace::Sans,
            ),
            Theme::CozyCafe => ([74, 52, 40, 255], [247, 238, 224, 255], [196, 120, 72, 255], 18, FontFace::Sans),
            Theme::Kawaii => ([96, 62, 112, 255], [255, 238, 247, 255], [255, 128, 182, 255], 26, FontFace::Sans),
            Theme::Cottagecore => ([46, 62, 40, 255], [239, 236, 220, 255], [112, 142, 88, 255], 12, FontFace::Sans),
            Theme::Cyberpunk => ([236, 242, 255, 255], [12, 8, 26, 238], [255, 42, 190, 255], 0, FontFace::Sans),
            Theme::Synthwave => ([255, 232, 250, 255], [30, 10, 54, 242], [255, 94, 170, 255], 10, FontFace::Sans),
            Theme::SciFiHud => ([200, 246, 255, 255], [6, 20, 32, 190], [0, 220, 255, 255], 0, FontFace::Sans),
            Theme::Arcade => ([255, 255, 255, 255], [12, 12, 22, 245], [255, 214, 0, 255], 6, FontFace::Sans),
            Theme::Fantasy => ([62, 40, 22, 255], [236, 220, 178, 255], [150, 32, 32, 255], 4, FontFace::Sans),
            Theme::Neubrutalism => ([12, 12, 12, 255], [255, 236, 110, 255], [255, 96, 96, 255], 10, FontFace::Sans),
            Theme::CleanLight => ([22, 26, 34, 255], [255, 255, 255, 248], [79, 70, 229, 255], 16, FontFace::Sans),
            Theme::GameBoy => ([15, 56, 15, 255], [139, 172, 15, 255], [48, 98, 48, 255], 0, FontFace::Sans),
            Theme::Vaporwave => ([40, 24, 70, 255], [255, 236, 250, 255], [255, 113, 206, 255], 0, FontFace::Sans),
            Theme::Spooky => ([244, 232, 220, 255], [22, 12, 26, 242], [255, 122, 24, 255], 14, FontFace::Sans),
            Theme::Winter => ([255, 255, 255, 255], [14, 32, 62, 238], [222, 44, 64, 255], 16, FontFace::Sans),
            _ => (
                [255, 255, 255, 255],
                [20, 26, 40, 210],
                [139, 112, 240, 255],
                16,
                FontFace::Sans,
            ),
        };
        self.foreground = fg;
        self.background = bg;
        self.accent = accent;
        self.radius = radius;
        self.font = font;
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AlertStyle {
    pub event: crate::EventKind,
    pub media: Option<crate::Id>,
    pub text: String,
    pub seconds: u32,
}
impl Default for AlertStyle {
    fn default() -> Self {
        Self {
            event: crate::EventKind::Follow,
            media: None,
            text: "{user} · {event}".into(),
            seconds: 6,
        }
    }
}

/// Appearance only: applying a preset never changes a goal, account or player.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Style {
    pub theme: Theme,
    pub font: FontFace,
    pub font_size: u32,
    pub bold: bool,
    pub foreground: [u8; 4],
    pub background: [u8; 4],
    pub accent: [u8; 4],
    pub radius: u32,
    pub layout: Layout,
    pub spacing: u32,
    pub reverse: bool,
}
impl Style {
    pub fn from_settings(s: &Settings) -> Self {
        Self {
            theme: s.theme,
            font: s.font,
            font_size: s.font_size,
            bold: s.bold,
            foreground: s.foreground,
            background: s.background,
            accent: s.accent,
            radius: s.radius,
            layout: s.layout,
            spacing: s.spacing,
            reverse: s.reverse,
        }
    }
    pub fn apply(&self, s: &mut Settings) {
        s.theme = self.theme;
        s.font = self.font;
        s.font_size = self.font_size;
        s.bold = self.bold;
        s.foreground = self.foreground;
        s.background = self.background;
        s.accent = self.accent;
        s.radius = self.radius;
        s.layout = self.layout;
        s.spacing = self.spacing;
        s.reverse = self.reverse;
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Music {
    pub automatic: bool,
    pub player: String,
    pub hide_paused: bool,
    pub artwork_colors: bool,
    pub large_artwork: bool,
    pub progress: bool,
    pub animate: bool,
    /// Installed font file for the artist line; empty uses the widget font.
    pub artist_font: String,
    /// Artist text size in pixels; 0 sizes it automatically.
    pub artist_size: u32,
    pub show_album: bool,
    pub album_font: String,
    pub album_size: u32,
    /// Automatic detection only shows tracks from music apps and sites.
    pub music_apps_only: bool,
}
impl Default for Music {
    fn default() -> Self {
        Self {
            automatic: true,
            player: String::new(),
            hide_paused: true,
            artwork_colors: true,
            large_artwork: false,
            progress: true,
            animate: true,
            artist_font: String::new(),
            artist_size: 0,
            show_album: false,
            album_font: String::new(),
            album_size: 0,
            music_apps_only: true,
        }
    }
}
impl Music {
    fn legacy() -> Self {
        Self {
            automatic: false,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod polish_tests {
    use super::*;
    #[test]
    fn older_manual_music_remains_manual() {
        let s: Settings =
            serde_json::from_str(r#"{"kind":"NowPlaying","content":"Track\nArtist"}"#).unwrap();
        assert!(!s.music.automatic);
        assert_eq!(s.content, "Track\nArtist");
        assert!(Settings::default().music.automatic);
    }
    #[test]
    fn style_does_not_replace_content_or_goals() {
        let mut source = Settings::default();
        source.apply_theme(Theme::WindowsXP);
        let style = Style::from_settings(&source);
        let mut target = Settings {
            target: 250,
            content: "Keep me".into(),
            ..Default::default()
        };
        target.music.player = "org.mpris.MediaPlayer2.spotify".into();
        style.apply(&mut target);
        assert_eq!(target.theme, Theme::WindowsXP);
        assert_eq!(target.target, 250);
        assert_eq!(target.content, "Keep me");
        assert_eq!(target.music.player, "org.mpris.MediaPlayer2.spotify");
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlertLayout { Above, Below, #[default] Over }
impl AlertLayout { pub fn label(self)-> &'static str { match self { Self::Above=>"Above media",Self::Below=>"Below media",Self::Over=>"Over media" } } }
