//! Made-up content for screenshots: `--page demo` (or `demo-alerts`, `demo-widgets`, …) fills the
//! studio with a sample scene, chat, activity and alerts under invented names. Only screenshots
//! use it; it never touches accounts.
use auroracast_core::{
    Action, ChatMessage, EventKind, Id, Layer, Platform, Project, Rule, Source, SourceKind,
    alert_filters::ChatMetadata,
    widgets::{Kind, Settings, Theme},
};

/// (platform, name, name colour, badges, highlight, message)
pub type Chatter = (Platform, &'static str, &'static str, &'static [&'static str], &'static str, &'static str);
pub const CHAT: [Chatter; 7] = [
    (Platform::Twitch, "PixelPanda", "#ff7ac6", &["subscriber"], "", "that clutch at the end was unreal 😮"),
    (Platform::YouTube, "Luna Loops", "", &["member"], "", "Hi from Brazil! First time catching you live 👋"),
    (Platform::Kick, "cozycat", "#53fc18", &["vip"], "", "gg everyone, one more round?"),
    (Platform::Twitch, "SamTheSlayer", "#7bb2ff", &["moderator"], "", "Reminder: be nice in chat, have fun!"),
    (Platform::Twitch, "juniper", "#c792ff", &[], "paid", "Cheer500 Here's to the 100 day streak! 🎉"),
    (Platform::TikTok, "dreamy.dan", "", &[], "first", "just found this stream, love the vibes"),
    (Platform::YouTube, "Mika", "#5e84f1", &["moderator"], "", "Clip that!! 🔥"),
];
const PICTURES: [(&str, [u8; 3]); 8] = [
    ("PixelPanda", [236, 72, 153]),
    ("Luna Loops", [86, 156, 255]),
    ("cozycat", [83, 200, 60]),
    ("SamTheSlayer", [255, 140, 50]),
    ("juniper", [145, 70, 255]),
    ("dreamy.dan", [40, 170, 150]),
    ("Mika", [230, 120, 60]),
    ("NovaRaider", [255, 92, 72]),
];

/// A stand-in game picture for the canvas.
pub fn background() -> String {
    let path = std::env::temp_dir().join("rosorah-demo-background.png");
    let (w, h) = (1920u32, 1080u32);
    let img = image::RgbaImage::from_fn(w, h, |x, y| {
        let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
        let horizon = 0.62;
        let mut c = if fy < horizon {
            // Dusk sky with a low sun.
            let t = fy / horizon;
            let sky = [40. + 180. * t, 20. + 60. * t, 90. + 40. * (1. - t)];
            let (dx, dy) = (fx - 0.5, (fy - 0.5) * h as f32 / w as f32);
            let sun = (1. - ((dx * dx + dy * dy).sqrt() / 0.12)).clamp(0., 1.);
            let stripes = fy > 0.48 && ((fy * 120.) as u32).is_multiple_of(3);
            if sun > 0. && !stripes { [255., 140. + 80. * (1. - t), 90.] } else { sky }
        } else {
            // A neon grid floor.
            let d = (fy - horizon) / (1. - horizon);
            let line = (((fx - 0.5) / (d + 0.02) * 8.).fract().abs() < 0.04) || ((1. / (d + 0.05)).fract() < 0.08);
            if line { [255., 70., 200.] } else { [25., 10. + 20. * d, 45. + 30. * d] }
        };
        // Mountains on the horizon.
        let ridge = horizon - 0.12 * ((fx * 9.).sin().abs() * 0.6 + (fx * 23.).cos().abs() * 0.4);
        if fy < horizon && fy > ridge {
            c = [30., 15., 60.];
        }
        image::Rgba([c[0] as u8, c[1] as u8, c[2] as u8, 255])
    });
    let _ = img.save(&path);
    path.display().to_string()
}
fn widget(kind: Kind, theme: Theme, label: &str, size: (u32, u32)) -> Settings {
    let mut s = Settings { kind, label: label.into(), ..Default::default() };
    s.apply_theme(theme);
    s.layout_width = size.0;
    s.layout_height = size.1;
    s.platforms = vec![Platform::Twitch, Platform::YouTube, Platform::Kick, Platform::TikTok, Platform::Donations];
    s
}
/// The sample scene: a game picture with chat, a goal and an alert box over it, plus a few alerts.
pub fn project(p: &mut Project) -> Id {
    let scene = p.canvases[0].scene;
    let add = |p: &mut Project, name: &str, kind: SourceKind, rect: [f32; 4]| {
        let id = p.id();
        p.sources.push(Source { id, name: name.into(), kind });
        if let Some(s) = p.scenes.iter_mut().find(|s| s.id == scene) {
            s.layers.push(Layer { crop: [0.; 4], frame: Default::default(), look: Default::default(), group: 0, source: id, rect, visible: true, locked: false, opacity: 1. });
        }
        id
    };
    // Only the sample layers show.
    if let Some(s) = p.scenes.iter_mut().find(|s| s.id == scene) {
        for l in &mut s.layers {
            l.visible = false;
        }
    }
    add(p, "Game", SourceKind::Image { path: background() }, [0., 0., 1., 1.]);
    let mut chat = widget(Kind::Chat, Theme::Unified, "", (560, 700));
    chat.font_size = 24;
    add(p, "Chat", SourceKind::Widget(chat), [0.7, 0.05, 0.28, 0.64]);
    let mut goal = widget(Kind::SubGoal, Theme::Material, "Sub goal: new camera", (620, 150));
    goal.target = 50;
    goal.initial = 37;
    add(p, "Sub goal", SourceKind::Widget(goal), [0.02, 0.04, 0.3, 0.13]);
    let alerts = add(p, "Alerts", SourceKind::Widget(widget(Kind::AlertBox, Theme::Material, "Alerts", (900, 300))), [0.18, 0.62, 0.48, 0.28]);
    for (event, text) in [
        (EventKind::Follow, "Thanks for the follow, {user}!"),
        (EventKind::Subscription, "{user} just subscribed! Welcome to the crew"),
        (EventKind::Bits, "{user} cheered {amount}!"),
        (EventKind::Raid, "{user} is raiding with {count} viewers!"),
        (EventKind::SuperChat, "{user}: {message}"),
    ] {
        let id = p.id();
        p.rules.push(Rule {
            options: Default::default(),
            conditions: Default::default(),
            id,
            name: event.label().into(),
            enabled: true,
            platform: None,
            event,
            command: String::new(),
            cooldown_ms: 3000,
            actions: vec![
                Action::DisplayAlert { source: alerts, template: text.into(), media: None, seconds: 6, subject: Default::default() },
                Action::SendChat { text: format!("{} 💜", text.replace("{amount}", "{amount} Bits")), platforms: vec![], sender: Default::default() },
            ],
        });
    }
    alerts
}
/// Messages for the chat panel.
pub fn messages() -> Vec<ChatMessage> {
    let now = crate::activity::now();
    CHAT.iter()
        .enumerate()
        .map(|(i, (platform, name, color, badges, highlight, text))| ChatMessage {
            metadata: ChatMetadata {
                color: color.to_string(),
                badges: badges.iter().map(|b| b.to_string()).collect(),
                highlight: highlight.to_string(),
                ..Default::default()
            },
            platform: *platform,
            id: format!("demo-{i}"),
            author: name.to_string(),
            text: text.to_string(),
            received_ms: now - (CHAT.len() - i) as u64 * 9000,
            demo: true,
        })
        .collect()
}
/// Activity history: follows, subs, cheers and a raid.
pub fn activity() -> Vec<crate::activity::Item> {
    let now = crate::activity::now();
    [
        (Platform::Twitch, EventKind::Follow, "PixelPanda", "PixelPanda followed"),
        (Platform::YouTube, EventKind::Membership, "Luna Loops", "Luna Loops became a member"),
        (Platform::Twitch, EventKind::Bits, "juniper", "juniper cheered 500 Bits · Here's to the 100 day streak!"),
        (Platform::Kick, EventKind::Subscription, "cozycat", "cozycat subscribed for 3 months"),
        (Platform::Twitch, EventKind::Raid, "NovaRaider", "NovaRaider raided with 42 viewers"),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (platform, kind, author, text))| crate::activity::Item {
        facts: Default::default(),
        reward_id: String::new(),
        goal_count: 0,
        platform,
        owner: "demo".into(),
        id: format!("demo-{i}"),
        kind,
        author: author.into(),
        text: text.into(),
        at_ms: now - (5 - i) as u64 * 60_000,
        historical: false,
    })
    .collect()
}
/// Stand-in profile pictures: a simple head and shoulders on each person's colour.
pub fn pictures() {
    for (name, c) in PICTURES {
        let picture = image::RgbaImage::from_fn(64, 64, |x, y| {
            let (dx, dy) = (x as f32 - 32., y as f32 - 26.);
            let head = dx * dx + dy * dy < 140.;
            let body = (x as f32 - 32.).powi(2) / 500. + (y as f32 - 66.).powi(2) / 420. < 1.;
            let shade = 1. - y as f32 / 200.;
            if head || body {
                image::Rgba([245, 235, 225, 255])
            } else {
                image::Rgba([(c[0] as f32 * shade) as u8, (c[1] as f32 * shade) as u8, (c[2] as f32 * shade) as u8, 255])
            }
        });
        for platform in ["Twitch", "YouTube", "Kick", "TikTok"] {
            crate::avatars::insert(platform, name, picture.clone());
        }
    }
}
/// Fills the on-canvas widgets: chat messages and an alert.
pub fn widgets(hub: &crate::widgets::Hub, alerts: Id) {
    for (platform, name, color, badges, highlight, text) in CHAT.iter().skip(2) {
        let badges: Vec<String> = badges.iter().map(|b| b.to_string()).collect();
        hub.chat(*platform, name, text, &crate::widgets::ChatLook { color, badges: &badges, highlight });
    }
    let subject = auroracast_core::AlertSubject { kind: Some(EventKind::Raid), platform: Some(Platform::Twitch), author: "NovaRaider".into() };
    hub.target_alert(alerts, "NovaRaider is raiding with 42 viewers!", 60, None, &subject);
}
