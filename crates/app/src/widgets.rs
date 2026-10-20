//! One small renderer per visible widget; generated frames are shared by all outputs.
use ab_glyph::{Font, FontArc, PxScale, ScaleFont, point};
use auroracast_core::{
    Id, Platform,
    widgets::{FontFace, GoalStyle, Kind, Period, ScoreLayout, Settings, Theme},
};
use rosorah_capture::{Backend, Frame, Producer};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};
#[derive(Clone)]
pub struct State {
    pub settings: Settings,
    pub value: u64,
    pub text: String,
    messages: VecDeque<String>,
    until: Option<Instant>,
    revision: u64,
    pub period_key: String,
    started: Instant,
    stream_started_ms: u64,
    alert: Option<(String, Option<Id>, Instant)>,
    queue: VecDeque<(String, Option<Id>, u32)>,
}
impl State {
    pub fn new(settings: Settings) -> Self {
        Self {
            period_key: period_key(&settings),
            started: Instant::now(),
            stream_started_ms: 0,
            alert: None,
            queue: VecDeque::new(),
            value: settings.initial,
            settings,
            text: String::new(),
            messages: VecDeque::new(),
            until: None,
            revision: 1,
        }
    }
    fn lines(&self) -> String {
        let label = &self.settings.label;
        match self.settings.kind {
            Kind::QrCode => format!("{label}\n{}", self.settings.content),
            Kind::Text => self.settings.content.clone(),
            Kind::AlertBox => self.alert.as_ref().map(|a| a.0.clone()).unwrap_or_default(),
            Kind::Chat => format!(
                "{label}\n{}",
                self.messages.iter().cloned().collect::<Vec<_>>().join("\n")
            ),
            Kind::SubGoal | Kind::CommunityGoal => {
                format!("{label}\n{} / {}", self.value, self.settings.target)
            }
            Kind::DonationGoal => format!(
                "{label}\n{}.{:02} / {}.{:02} {}",
                self.value / 100,
                self.value % 100,
                self.settings.target / 100,
                self.settings.target % 100,
                self.settings.currency
            ),
            Kind::Counter => format!("{label}: {}", self.value),
            Kind::Shoutout
            | Kind::StreamLabels
            | Kind::Schedule
            | Kind::NowPlaying
            | Kind::Poll
            | Kind::Checklist => format!(
                "{label}\n{}",
                if self.text.is_empty() {
                    &self.settings.content
                } else {
                    &self.text
                }
            ),
            Kind::EventTicker | Kind::LatestSupporter | Kind::Credits => format!(
                "{label}\n{}",
                self.messages
                    .iter()
                    .rev()
                    .take(if self.settings.kind == Kind::LatestSupporter {
                        1
                    } else if self.settings.kind == Kind::Credits {
                        50
                    } else {
                        6
                    })
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
            Kind::Announcements => {
                let items: Vec<_> = self
                    .settings
                    .content
                    .lines()
                    .filter(|v| !v.trim().is_empty())
                    .collect();
                format!(
                    "{label}\n{}",
                    if items.is_empty() {
                        "Add announcements in widget settings"
                    } else {
                        items[(self.started.elapsed().as_secs()
                            / u64::from(self.settings.interval.max(1)))
                            as usize
                            % items.len()]
                    }
                )
            }
            Kind::Sports => {
                let (games, status) = crate::sports::snapshot(&self.settings.sports);
                let rows: Vec<_> = games
                    .iter()
                    .map(|g| {
                        format!(
                            "{} · {} {} {} · {}",
                            g.league.label(),
                            g.away,
                            g.score,
                            g.home,
                            g.status
                        )
                    })
                    .collect();
                let body = if rows.is_empty() {
                    "No matching games yesterday, today or tomorrow".into()
                } else if self.settings.sports.layout == ScoreLayout::Cards {
                    rows[(self.started.elapsed().as_secs()
                        / u64::from(self.settings.interval.max(1)))
                        as usize
                        % rows.len()]
                    .clone()
                } else if self.settings.sports.layout == ScoreLayout::Board {
                    let page = (self.started.elapsed().as_secs()
                        / u64::from(self.settings.interval.max(1)))
                        as usize
                        % rows.len().div_ceil(5);
                    rows.iter()
                        .skip(page * 5)
                        .take(5)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    rows.join(if self.settings.sports.layout == ScoreLayout::Ticker {
                        "     •     "
                    } else {
                        "\n"
                    })
                };
                format!("{label} · {status}\n{body}")
            }
            Kind::Timer => {
                let seconds = if self.settings.timer_up {
                    if self.until.is_some() {
                        self.started.elapsed().as_secs()
                    } else {
                        0
                    }
                } else {
                    self.until
                        .map(|t| t.saturating_duration_since(Instant::now()).as_secs())
                        .unwrap_or(u64::from(self.settings.seconds))
                };
                format!("{label}\n{:02}:{:02}", seconds / 60, seconds % 60)
            }
        }
    }
    pub fn refresh_period(&mut self) {
        let key = period_key(&self.settings);
        if self.settings.is_goal() && self.period_key != key {
            self.period_key = key;
            self.value = self.settings.initial;
            self.revision = self.revision.wrapping_add(1);
        }
    }
    fn accepts_count(&mut self) -> bool {
        self.refresh_period();
        self.settings.period != Period::Deadline || !deadline_passed(&self.settings.deadline)
    }
    pub fn update(&mut self, text: &str, increment: i32, start: bool) {
        if !text.is_empty() {
            self.text = text.chars().take(400).collect();
        }
        self.value = self.value.saturating_add_signed(i64::from(increment));
        if start {
            self.started = Instant::now();
            self.until =
                Some(Instant::now() + Duration::from_secs(u64::from(self.settings.seconds)));
        }
        self.revision = self.revision.wrapping_add(1);
    }
}
#[derive(Clone, Default)]
pub struct Hub(pub Arc<Mutex<BTreeMap<Id, Arc<Mutex<State>>>>>);
impl Hub {
    pub fn goal_event(&self, event: &auroracast_core::Event) {
        for state in self.0.lock().unwrap().values() {
            let mut s = state.lock().unwrap();
            if s.settings.kind == Kind::Poll
                && s.settings.platforms.contains(&event.platform)
                && matches!(
                    event.kind,
                    auroracast_core::EventKind::Poll | auroracast_core::EventKind::Prediction
                )
            {
                s.update(&event.text, 0, false);
            }
            if s.settings.kind == Kind::CommunityGoal
                && s.settings.goal_event == Some(event.kind)
                && s.settings.platforms.contains(&event.platform)
                && s.accepts_count()
            {
                use auroracast_core::EventKind;
                let n = match event.kind {
                    EventKind::Bits => event.facts.amount,
                    EventKind::Gift | EventKind::Subscription | EventKind::Membership => {
                        event.facts.count
                    }
                    EventKind::Follow | EventKind::Subscriber => 1,
                    _ => 0,
                };
                s.value = s.value.saturating_add(n);
                s.revision = s.revision.wrapping_add(1);
            }
        }
    }

    pub fn preview_alert(&self, id: Id) {
        if let Some(state) = self.0.lock().unwrap().get(&id) {
            let mut s = state.lock().unwrap();
            let media = s.settings.alerts.first().and_then(|a| a.media);
            s.queue.push_back((
                "Preview · Thanks for supporting the stream!".into(),
                media,
                5,
            ));
        }
    }

    pub fn alert(
        &self,
        platform: Platform,
        event: auroracast_core::EventKind,
        author: &str,
        text: &str,
    ) {
        for state in self.0.lock().unwrap().values() {
            let mut s = state.lock().unwrap();
            if s.settings.kind != Kind::AlertBox || s.settings.automation_only || !s.settings.platforms.contains(&platform) {
                continue;
            }
            let rule = s.settings.alerts.iter().find(|r| r.event == event).cloned();
            let (text, media, seconds) = if let Some(rule) = rule {
                (
                    rule.text
                        .replace("{user}", author)
                        .replace("{event}", event.label())
                        .replace("{message}", text),
                    rule.media,
                    rule.seconds,
                )
            } else {
                (format!("{} · {}\n{}", author, event.label(), text), None, 6)
            };
            if s.queue.len() < 32 {
                s.queue.push_back((text, media, seconds.clamp(1, 60)));
            }
        }
    }
    pub fn target_alert(&self,id:Id,text:&str,seconds:u32,media:Option<Id>){
        if let Some(s)=self.0.lock().unwrap().get(&id){let mut s=s.lock().unwrap();if s.settings.kind==Kind::AlertBox&&s.queue.len()<32{s.queue.push_back((text.chars().take(400).collect(),media,seconds.clamp(1,60)));}}
    }
    pub fn clear_alerts(&self) {
        for state in self.0.lock().unwrap().values() {
            let mut s = state.lock().unwrap();
            s.queue.clear();
            s.alert = None;
            s.revision = s.revision.wrapping_add(1);
        }
    }
    pub fn tick_alerts(&self, project: &auroracast_core::Project) -> Vec<Id> {
        let mut start = Vec::new();
        for (id, state) in self.0.lock().unwrap().iter() {
            let mut s = state.lock().unwrap();
            if s.settings.kind != Kind::AlertBox {
                continue;
            }
            let visible = project
                .canvases
                .iter()
                .filter_map(|c| project.scene(c.scene))
                .any(|scene| scene.layers.iter().any(|l| l.source == *id && l.visible));
            if !visible {
                s.queue.clear();
                s.alert = None;
                continue;
            }
            if s.alert.as_ref().is_some_and(|a| Instant::now() >= a.2) {
                s.alert = None;
                s.revision = s.revision.wrapping_add(1);
            }
            if s.alert.is_none()
                && let Some((text, media, seconds)) = s.queue.pop_front()
            {
                if let Some(id) = media {
                    start.push(id)
                }
                s.alert = Some((
                    text,
                    media,
                    Instant::now() + Duration::from_secs(seconds as u64),
                ));
                s.revision = s.revision.wrapping_add(1);
            }
        }
        start
    }
    pub fn active_media(&self) -> Vec<Id> {
        self.0
            .lock()
            .unwrap()
            .values()
            .filter_map(|s| s.lock().unwrap().alert.as_ref().and_then(|a| a.1))
            .collect()
    }

    pub fn start_stream(&self) {
        for state in self.0.lock().unwrap().values() {
            let mut s = state.lock().unwrap();
            if s.settings.is_goal() && s.settings.period == Period::Stream {
                s.stream_started_ms = crate::activity::now();
                s.value = s.settings.initial;
                s.revision = s.revision.wrapping_add(1);
            }
        }
    }
    pub fn activity(&self, platform: Platform, author: &str, text: &str) {
        for state in self.0.lock().unwrap().values() {
            let mut s = state.lock().unwrap();
            if matches!(
                s.settings.kind,
                Kind::EventTicker | Kind::LatestSupporter | Kind::Credits
            ) && s.settings.platforms.contains(&platform)
            {
                s.messages.push_back(format!(
                    "{} · {} · {}",
                    platform.name(),
                    author,
                    text.chars().take(180).collect::<String>()
                ));
                while s.messages.len() > 50 {
                    s.messages.pop_front();
                }
                s.revision = s.revision.wrapping_add(1);
            }
        }
    }
    pub fn update(&self, id: Id, text: &str, increment: i32, start: bool) {
        if let Some(state) = self.0.lock().unwrap().get(&id) {
            state.lock().unwrap().update(text, increment, start);
        }
    }
    pub fn reset(&self, id: Id) {
        if let Some(state) = self.0.lock().unwrap().get(&id) {
            let mut s = state.lock().unwrap();
            let settings = s.settings.clone();
            *s = State::new(settings);
        }
    }
    pub fn chat(&self, platform: Platform, author: &str, text: &str) {
        for state in self.0.lock().unwrap().values() {
            let mut s = state.lock().unwrap();
            if s.settings.kind == Kind::Chat && s.settings.platforms.contains(&platform) {
                s.messages.push_back(format!(
                    "{} · {}: {}",
                    platform.name(),
                    author,
                    text.chars().take(240).collect::<String>()
                ));
                while s.messages.len() > 6 {
                    s.messages.pop_front();
                }
                s.revision = s.revision.wrapping_add(1);
            }
        }
    }
    #[cfg(test)]
    pub fn donation(&self, currency: &str, hundredths: u64) {
        self.donation_at(currency, hundredths, crate::activity::now(), false);
    }
    pub fn donation_at(&self, currency: &str, hundredths: u64, at_ms: u64, historical: bool) {
        for state in self.0.lock().unwrap().values() {
            let mut s = state.lock().unwrap();
            let current_period = match s.settings.period {
                Period::Week | Period::Month => {
                    gst::glib::DateTime::from_unix_local((at_ms / 1000) as i64).is_ok_and(|date| {
                        period_for_date(&s.settings, &date) == period_key(&s.settings)
                    })
                }
                Period::Stream => {
                    !historical || (s.stream_started_ms > 0 && at_ms >= s.stream_started_ms)
                }
                _ => true,
            };
            if current_period
                && s.accepts_count()
                && s.settings.kind == Kind::DonationGoal
                && s.settings.currency.eq_ignore_ascii_case(currency)
            {
                s.value = s.value.saturating_add(hundredths);
                s.revision = s.revision.wrapping_add(1);
            }
        }
    }
    pub fn subscription(&self, platform: Platform, count: u64) {
        for state in self.0.lock().unwrap().values() {
            let mut s = state.lock().unwrap();
            if s.accepts_count()
                && s.settings.kind == Kind::SubGoal
                && s.settings.platforms.contains(&platform)
            {
                s.value = s.value.saturating_add(count);
                s.revision = s.revision.wrapping_add(1);
            }
        }
    }
}
pub fn period_key(s: &Settings) -> String {
    let Ok(now) = gst::glib::DateTime::now_local() else {
        return String::new();
    };
    period_for_date(s, &now)
}
fn period_for_date(s: &Settings, now: &gst::glib::DateTime) -> String {
    match s.period {
        Period::Week => {
            let back = (now.day_of_week() as u32 + 7 - s.reset_day.clamp(1, 7)) % 7;
            now.add_days(-(back as i32))
                .unwrap()
                .format("%Y-%m-%d")
                .unwrap()
                .to_string()
        }
        Period::Month => now.format("%Y-%m").unwrap().to_string(),
        _ => format!("{:?}:{}", s.period, s.deadline),
    }
}
fn deadline_passed(date: &str) -> bool {
    if date.len() != 10 {
        return false;
    }
    gst::glib::DateTime::now_local()
        .ok()
        .and_then(|d| d.format("%Y-%m-%d").ok())
        .is_some_and(|d| d.as_str() > date)
}
pub struct Renderer(pub Arc<Mutex<State>>, pub rosorah_capture::FrameReader);
impl Backend for Renderer {
    fn run(
        self: Box<Self>,
        out: Producer,
        cancel: tokio::sync::watch::Receiver<bool>,
        _: rosorah_capture::Options,
    ) -> anyhow::Result<()> {
        let mut previous = None;
        let mut music_identity = String::new();
        let mut last_frame: Option<Frame> = None;
        let mut fade_from: Option<Frame> = None;
        let mut fade_start = Instant::now();
        let epoch = Instant::now();
        while !*cancel.borrow() {
            let (settings, lines, revision, media) = {
                let mut s = self.0.lock().unwrap();
                s.refresh_period();
                (
                    s.settings.clone(),
                    s.lines(),
                    s.revision,
                    s.alert.as_ref().and_then(|a| a.1),
                )
            };
            let track = if settings.kind == Kind::NowPlaying && settings.music.automatic {
                crate::music::current(&settings.music)
            } else {
                None
            };
            let identity = track
                .as_ref()
                .map(|t| {
                    format!(
                        "{}|{}|{}|{}|{}",
                        t.player,
                        t.title,
                        t.artist,
                        t.playing,
                        t.artwork.is_some()
                    )
                })
                .unwrap_or_default();
            if settings.kind == Kind::NowPlaying && identity != music_identity {
                fade_from = if settings.music.animate {
                    last_frame.take()
                } else {
                    None
                };
                fade_start = Instant::now();
                music_identity = identity.clone();
            }
            let fading = fade_from.is_some() && fade_start.elapsed() < Duration::from_millis(250);
            if !fading {
                fade_from = None;
            }
            let animate = fading
                || settings.kind == Kind::Credits
                || media.is_some()
                || (settings.kind == Kind::Sports && settings.sports.layout == ScoreLayout::Ticker);
            let tick = if animate {
                epoch.elapsed().as_millis() / 50
            } else if settings.kind == Kind::NowPlaying
                && track.as_ref().is_some_and(|t| t.playing && t.length > 0.)
            {
                epoch.elapsed().as_secs() as u128
            } else {
                0
            };
            let key = (revision, lines.clone(), tick, identity);
            if previous.as_ref() != Some(&key) {
                let mut frame = if settings.kind == Kind::NowPlaying && settings.music.automatic {
                    polished_music(&settings, track.as_ref())
                } else {
                    render_frame(
                        &settings,
                        &lines,
                        self.0.lock().unwrap().value,
                        epoch.elapsed().as_secs_f32(),
                    )?
                };
                if let Some(old) = &fade_from && old.width==frame.width && old.height==frame.height {
                    let t = (fade_start.elapsed().as_secs_f32() / 0.25).clamp(0., 1.);
                    for (a, b) in frame.rgba.chunks_exact_mut(4).zip(old.rgba.chunks_exact(4)) {
                        let aa = a[3] as f32 / 255. * t;
                        let ba = b[3] as f32 / 255. * (1. - t);
                        let alpha = aa + ba;
                        for c in 0..3 {
                            a[c] = if alpha > 0. {
                                ((a[c] as f32 * aa + b[c] as f32 * ba) / alpha) as u8
                            } else {
                                0
                            };
                        }
                        a[3] = (alpha * 255.) as u8;
                    }
                }
                if settings.kind == Kind::NowPlaying && !fading {
                    last_frame = Some(Frame {
                        width: frame.width,
                        height: frame.height,
                        rgba: frame.rgba.clone(),
                    });
                }
                if let Some(id) = media
                    && let Some((_, Some(video))) = self
                        .1
                        .latest()
                        .into_iter()
                        .find(|(source, _)| *source == id)
                {
                    compose_media_layout(&mut frame, &video, settings.fit_media, settings.alert_layout);
                }
                out.publish(frame)?;
                previous = Some(key);
            }
            std::thread::sleep(Duration::from_millis(if animate { 50 } else { 100 }));
        }
        Ok(())
    }
}
/// A chosen installed font, or the widget's own font when none is set or it can't load.
fn font_or(path: &str, face: FontFace) -> FontArc {
    crate::fonts::load(path).unwrap_or_else(|| font(face).clone())
}
fn font(face: FontFace) -> &'static FontArc {
    static MONO: OnceLock<FontArc> = OnceLock::new();
    static SANS: OnceLock<FontArc> = OnceLock::new();
    match face {
        FontFace::Mono => MONO.get_or_init(|| {
            let fonts = eframe::egui::FontDefinitions::default();
            FontArc::try_from_vec(fonts.font_data["Hack"].font.to_vec()).expect("bundled font")
        }),
        FontFace::Sans => SANS.get_or_init(|| {
            FontArc::try_from_slice(include_bytes!("../assets/fonts/Roboto.ttf"))
                .expect("bundled Roboto")
        }),
    }
}
struct Paint {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
}
impl Paint {
    fn blend(&mut self, x: i32, y: i32, c: [u8; 4], coverage: f32) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let o = ((y as u32 * self.width + x as u32) * 4) as usize;
        let a = coverage * c[3] as f32 / 255.;
        let b = self.pixels[o + 3] as f32 / 255.;
        let alpha = a + b * (1. - a);
        for i in 0..3 {
            self.pixels[o + i] = if alpha > 0. {
                ((c[i] as f32 * a + self.pixels[o + i] as f32 * b * (1. - a)) / alpha).round() as u8
            } else {
                0
            };
        }
        self.pixels[o + 3] = (alpha * 255.).round() as u8;
    }
    fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, r: i32, c: [u8; 4]) {
        let r = r.min(w / 2).min(h / 2).max(0);
        for py in y.max(0)..(y + h).min(self.height as i32) {
            for px in x.max(0)..(x + w).min(self.width as i32) {
                let dx = (x + r - px).max(px - (x + w - r - 1)).max(0);
                let dy = (y + r - py).max(py - (y + h - r - 1)).max(0);
                if dx * dx + dy * dy <= r * r {
                    self.blend(px, py, c, 1.);
                }
            }
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn text(
        &mut self,
        s: &str,
        x: f32,
        y: f32,
        max_w: f32,
        size: f32,
        c: [u8; 4],
        face: FontFace,
        bold: bool,
        max_y: f32,
    ) {
        self.text_font(s, x, y, max_w, size, c, font(face), bold, max_y);
    }
    #[allow(clippy::too_many_arguments)]
    fn text_font(
        &mut self,
        s: &str,
        x: f32,
        y: f32,
        max_w: f32,
        size: f32,
        c: [u8; 4],
        f: &FontArc,
        bold: bool,
        max_y: f32,
    ) {
        let scaled = f.as_scaled(PxScale::from(size));
        let mut px = x;
        let mut py = y;
        for ch in s.chars().take(2000) {
            if ch == '\n' {
                px = x;
                py += size * 1.3;
                continue;
            }
            let id = scaled.glyph_id(ch);
            let advance = scaled.h_advance(id);
            if px + advance > x + max_w {
                px = x;
                py += size * 1.3;
            }
            if py > max_y {
                break;
            }
            if let Some(outline) = f.outline_glyph(id.with_scale_and_position(size, point(px, py)))
            {
                let b = outline.px_bounds();
                outline.draw(|gx, gy, cov| {
                    let x = gx as i32 + b.min.x as i32;
                    let y = gy as i32 + b.min.y as i32;
                    self.blend(x, y, c, cov);
                    if bold {
                        self.blend(x + 1, y, c, cov);
                    }
                });
            }
            px += advance;
        }
    }
}
pub fn render(settings: &Settings, text: &str) -> anyhow::Result<Frame> {
    render_value(settings, text, settings.initial)
}
fn render_value(s: &Settings, text: &str, value: u64) -> anyhow::Result<Frame> {
    render_frame(s, text, value, 0.)
}
include!("widget_paint.rs");
include!("responsive_widgets.rs");
fn render_frame(s: &Settings, text: &str, value: u64, elapsed: f32) -> anyhow::Result<Frame> {
    if s.kind == Kind::Text {
        return Ok(text_card(s, text));
    }
    if s.kind==Kind::Sports{return Ok(responsive_sports(s,text,elapsed))}
    if s.kind == Kind::Chat {
        return Ok(polished_chat(s, text));
    }
    if s.is_goal() {
        return Ok(polished_goal(s, value));
    }
    if s.kind == Kind::NowPlaying {
        if s.music.automatic {
            return Ok(polished_music(s, crate::music::current(&s.music).as_ref()));
        }
        let mut track = crate::music::sample();
        let mut rows = text.lines();
        rows.next();
        track.title = rows.next().unwrap_or("").into();
        track.artist = rows.collect::<Vec<_>>().join(" · ");
        track.album.clear();
        track.identity = "Manual track".into();
        track.length = 0.;
        return Ok(polished_music(s, Some(&track)));
    }
    if !matches!(s.kind, Kind::QrCode | Kind::Sports | Kind::Credits) {
        return Ok(polished_info(s, text));
    }
    // An alert box without text is a clear picture at the box's own size, so media fits its shape.
    let (width, height) = if s.kind == Kind::AlertBox { (s.layout_width.clamp(96, 960), s.layout_height.clamp(96, 960)) } else { (960, 400) };
    let mut p = Paint {
        pixels: vec![0; (width * height * 4) as usize],
        width,
        height,
    };
    if s.kind == Kind::AlertBox && text.is_empty() {
        return Ok(Frame {
            width,
            height,
            rgba: p.pixels,
        });
    }
    p.pixels = s.background.repeat((width * height) as usize);
    let radius = s.radius.min(40) as i32;
    for y in 0..radius {
        for x in 0..radius {
            if (radius - x) * (radius - x) + (radius - y) * (radius - y) > radius * radius {
                for (px, py) in [(x, y), (959 - x, y), (x, 399 - y), (959 - x, 399 - y)] {
                    let o = ((py * 960 + px) * 4) as usize;
                    p.pixels[o..o + 4].fill(0);
                }
            }
        }
    }

    let classic = matches!(s.theme, Theme::Windows98 | Theme::WindowsXP);
    if classic {
        p.rect(
            3,
            3,
            954,
            46,
            if s.theme == Theme::WindowsXP { 6 } else { 0 },
            s.accent,
        );
        p.rect(4, 398, 952, 2, 0, [80, 80, 80, 255]);
        p.rect(0, 0, 960, 2, 0, [255, 255, 255, 255]);
    }
    let size = s.font_size.clamp(16, 96) as f32;
    if s.kind == Kind::AlertBox {
        let (top,bottom)=match s.alert_layout { auroracast_core::widgets::AlertLayout::Above=>(24.,115.),auroracast_core::widgets::AlertLayout::Below=>(294.,395.),auroracast_core::widgets::AlertLayout::Over=>(145.,390.) };
        p.text(text,24.,top,912.,size,s.foreground,s.font,s.bold,bottom);
    } else if s.kind == Kind::Credits {
        let rows: Vec<_> = text.lines().collect();
        let line_height = size * 1.5;
        let total = rows.len() as f32 * line_height + 400.;
        let offset = (elapsed * 30.) % total.max(1.);
        for (i, row) in rows.iter().enumerate() {
            let y = 400. + i as f32 * line_height - offset;
            if y > 0. && y < 430. {
                p.text(row, 24., y, 912., size, s.foreground, s.font, s.bold, 420.);
            }
        }
    } else if s.kind == Kind::QrCode {
        let payload = s.content.trim();
        if payload.len() <= 1024
            && !payload.is_empty()
            && let Ok(code) = qrcode::QrCode::new(payload.as_bytes())
        {
            let n = code.width() as i32;
            let unit = (350 / (n + 8)).max(1);
            let width = (n + 8) * unit;
            p.rect(960 - width - 24, 20, width, width, 0, [255; 4]);
            for y in 0..n {
                for x in 0..n {
                    if code[(x as usize, y as usize)] == qrcode::Color::Dark {
                        p.rect(
                            960 - width - 24 + (x + 4) * unit,
                            20 + (y + 4) * unit,
                            unit,
                            unit,
                            0,
                            [0, 0, 0, 255],
                        );
                    }
                }
            }
            p.text(
                &s.label,
                24.,
                80.,
                500.,
                size,
                s.foreground,
                s.font,
                s.bold,
                380.,
            );
        } else {
            p.text(
                "Enter a URL or text (up to 1024 bytes)",
                24.,
                80.,
                900.,
                30.,
                s.foreground,
                s.font,
                false,
                380.,
            );
        }
    } else if s.kind == Kind::Sports && s.sports.layout == ScoreLayout::Ticker {
        let (title, body) = text.split_once('\n').unwrap_or((text, ""));
        p.text(title, 24., 40., 912., 26., s.foreground, s.font, true, 50.);
        let scaled = font(s.font).as_scaled(PxScale::from(size));
        let length: f32 = body
            .chars()
            .take(2000)
            .map(|c| scaled.h_advance(scaled.glyph_id(c)))
            .sum();
        let cycle = length + 120.;
        let offset = (elapsed * s.sports.speed.clamp(10, 160) as f32) % cycle.max(1.);
        let first = if s.sports.rightward {
            offset - cycle
        } else {
            -offset
        };
        for x in [first, first + cycle] {
            p.text(
                body,
                x + 24.,
                180.,
                length + 1.,
                size,
                s.foreground,
                s.font,
                s.bold,
                240.,
            );
        }
    } else if s.kind == Kind::Chat {
        let mut lines = text.lines();
        let title = lines.next().unwrap_or("");
        p.text(
            title,
            20.,
            35.,
            920.,
            26.,
            if classic { [255; 4] } else { s.foreground },
            s.font,
            true,
            45.,
        );
        let mut rows: Vec<_> = lines.collect();
        if s.reverse {
            rows.reverse();
        }
        let mut y = 58.;
        for (i, row) in rows.iter().rev().take(6).rev().enumerate() {
            let h = (size * 1.3 + 14.).min(74.);
            if y + h > 395. {
                break;
            }
            let accent = if s.theme == Theme::Unified {
                if row.starts_with("Twitch") {
                    [145, 70, 255, 235]
                } else {
                    [220, 40, 50, 235]
                }
            } else {
                s.accent
            };
            let width = if s.layout == auroracast_core::widgets::Layout::Bubbles
                || s.theme == Theme::Bubbles
            {
                (row.chars().count() as f32 * size * 0.53 + 30.).clamp(150., 910.)
            } else {
                920.
            };
            let x = if s.theme == Theme::Bubbles && i % 2 == 1 {
                940. - width
            } else {
                20.
            };
            if s.layout != auroracast_core::widgets::Layout::Lines {
                let mut fill = accent;
                fill[3] = if s.theme == Theme::Bubbles { 230 } else { 65 };
                p.rect(
                    x as i32,
                    y as i32,
                    width as i32,
                    h as i32,
                    s.radius as i32,
                    fill,
                );
                p.rect(x as i32, y as i32, 4, h as i32, 0, accent);
            }
            p.text(
                row,
                x + 12.,
                y + size,
                width - 24.,
                size,
                s.foreground,
                s.font,
                s.bold,
                y + h - 2.,
            );
            y += h + s.spacing.min(30) as f32;
        }
    } else {
        p.text(
            text,
            24.,
            if classic { 34. } else { size + 14. },
            912.,
            if classic { size.min(28.) } else { size },
            s.foreground,
            s.font,
            s.bold,
            if s.is_goal() { 225. } else { 385. },
        );
    }
    if s.is_goal() {
        let t = (value as f64 / s.target.max(1) as f64).clamp(0., 1.) as f32;
        let c = s.accent;
        let dim = [c[0], c[1], c[2], 55];
        match s.goal_style {
            GoalStyle::Bar => {
                p.rect(24, 286, 912, 64, 12, dim);
                p.rect(24, 286, (912. * t) as i32, 64, 12, c);
            }
            GoalStyle::Vertical | GoalStyle::Thermometer => {
                p.rect(840, 55, 64, 310, 30, dim);
                p.rect(840, 365 - (310. * t) as i32, 64, (310. * t) as i32, 30, c);
            }
            GoalStyle::Ring => {
                for y in 215..390 {
                    for x in 390..565 {
                        let dx = x as f32 - 477.;
                        let dy = y as f32 - 302.;
                        let d = (dx * dx + dy * dy).sqrt();
                        if (57.0..=85.0).contains(&d) {
                            let angle = (dy.atan2(dx) + std::f32::consts::FRAC_PI_2)
                                .rem_euclid(std::f32::consts::TAU)
                                / std::f32::consts::TAU;
                            p.blend(x, y, if angle <= t { c } else { dim }, 1.);
                        }
                    }
                }
            }
            GoalStyle::Counter => {}
            GoalStyle::Segments | GoalStyle::Milestones => {
                for i in 0..10 {
                    p.rect(
                        24 + i * 92,
                        286,
                        80,
                        64,
                        5,
                        if (i as f32) < t * 10. { c } else { dim },
                    );
                }
            }
            GoalStyle::Hearts => {
                for i in 0..10 {
                    let color = if (i as f32) < t * 10. { c } else { dim };
                    let x = 25 + i * 92;
                    p.rect(x, 284, 28, 22, 5, color);
                    p.rect(x + 28, 284, 28, 22, 5, color);
                    for yy in 0..28 {
                        p.rect(x + yy, 305 + yy, 56 - 2 * yy, 1, 0, color);
                    }
                }
            }
            GoalStyle::Garden => {
                for i in 0..10 {
                    let x = 50 + i * 90;
                    let on = (i as f32) < t * 10.;
                    p.rect(x, 320, 8, 45, 0, if on { [60, 185, 95, 255] } else { dim });
                    p.rect(x - 18, 302, 44, 28, 14, if on { c } else { dim });
                }
            }
        }
    }
    Ok(Frame {
        width,
        height,
        rgba: p.pixels,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn widget_output_has_text_and_valid_rgba() {
        let settings = Settings {
            background: [0, 0, 0, 0],
            ..Default::default()
        };
        let frame = render(&settings, "Deaths: 3").unwrap();
        assert_eq!(frame.rgba.len(), 960 * 400 * 4);
        assert!(frame.rgba.chunks_exact(4).any(|p| p[3] > 0));
        assert_eq!(frame.rgba[3], 0);
    }
    #[test]
    fn counter_saturates_and_goal_is_scoped() {
        let hub = Hub::default();
        let mut s = Settings {
            kind: Kind::SubGoal,
            ..Default::default()
        };
        s.platforms = vec![Platform::Twitch];
        hub.0
            .lock()
            .unwrap()
            .insert(1, Arc::new(Mutex::new(State::new(s))));
        hub.subscription(Platform::YouTube, 5);
        hub.subscription(Platform::Twitch, 2);
        assert_eq!(hub.0.lock().unwrap()[&1].lock().unwrap().value, 2);
        hub.update(1, "", -3, false);
        assert_eq!(hub.0.lock().unwrap()[&1].lock().unwrap().value, 0);
    }
}

/// Font (installed fonts, or the widget font) and size (0 = automatic) for one text line.
fn music_font_row(ui: &mut eframe::egui::Ui, label: &str, font: &mut String, size: &mut u32) {
    use eframe::egui;
    ui.horizontal_wrapped(|ui| {
        ui.label(label);
        let current = if font.is_empty() {
            "Widget font".to_owned()
        } else {
            crate::fonts::display_name(std::path::Path::new(font.as_str()))
        };
        egui::ComboBox::from_id_salt(("music_font", label))
            .selected_text(current)
            .width(200.)
            .height(320.)
            .show_ui(ui, |ui| {
                ui.selectable_value(font, String::new(), "Widget font");
                for (name, path) in crate::fonts::installed() {
                    ui.selectable_value(font, path.clone(), name);
                }
            });
        ui.add(
            egui::DragValue::new(size)
                .range(0..=160)
                .custom_formatter(|v, _| if v == 0. { "Auto size".into() } else { format!("{v} px") }),
        )
        .on_hover_text("Drag or type a size; 0 sizes it automatically");
    });
    if !font.is_empty() && crate::fonts::load(font).is_none() {
        ui.colored_label(egui::Color32::LIGHT_RED, "This font can't be loaded; the widget font is used.");
    }
}
/// Searchable list of every bundled and installed font, each name drawn in its own font when possible.
fn font_picker(ui: &mut eframe::egui::Ui, font: &mut String) {
    use eframe::egui;
    let current = if font.is_empty() { "Roboto".to_owned() } else { crate::fonts::display_name(std::path::Path::new(font.as_str())) };
    let search_id = ui.id().with("font-search");
    let mut search: String = ui.data_mut(|d| d.get_temp(search_id).unwrap_or_default());
    ui.horizontal(|ui| {
        ui.label("Font");
        egui::ComboBox::from_id_salt("text_font").selected_text(current).width(240.).height(420.).show_ui(ui, |ui| {
            ui.add(egui::TextEdit::singleline(&mut search).hint_text("Search fonts…"));
            ui.separator();
            let needle = search.to_lowercase();
            if needle.is_empty() || "roboto".contains(&needle) {
                ui.selectable_value(font, String::new(), "Roboto");
            }
            for (name, path) in crate::fonts::installed().iter().filter(|(n, _)| n.to_lowercase().contains(&needle)) {
                ui.selectable_value(font, path.clone(), name);
            }
        });
    });
    ui.data_mut(|d| d.insert_temp(search_id, search));
    ui.small(format!("{} fonts · bundled with Rosorah and installed on this computer", crate::fonts::installed().len() + 1));
    if !font.is_empty() && crate::fonts::load(font).is_none() {
        ui.colored_label(egui::Color32::LIGHT_RED, "This font can't be loaded; Roboto is used.");
    }
}
fn text_controls(ui: &mut eframe::egui::Ui, s: &mut Settings) {
    use auroracast_core::widgets::*;
    use eframe::egui;
    ui.label("Text");
    ui.add(egui::TextEdit::multiline(&mut s.content).desired_rows(3).desired_width(f32::INFINITY).hint_text("Type your note…"));
    font_picker(ui, &mut s.text_font);
    ui.horizontal(|ui| {
        ui.label("Size");
        ui.add(egui::DragValue::new(&mut s.font_size).range(0..=600)
            .custom_formatter(|v, _| if v == 0. { "Fit to box".into() } else { format!("{v} px") }))
            .on_hover_text("0 makes the text as large as the box allows");
        ui.checkbox(&mut s.bold, "Bold");
    });
    ui.horizontal(|ui| {
        ui.label("Align");
        for &a in TextAlign::ALL {
            ui.selectable_value(&mut s.align, a, a.label());
        }
    });
    ui.horizontal(|ui| {
        ui.label("Color");
        ui.color_edit_button_srgba_unmultiplied(&mut s.foreground);
        ui.label("Background");
        ui.color_edit_button_srgba_unmultiplied(&mut s.background);
    });
    ui.horizontal(|ui| {
        ui.label("Outline");
        ui.add(egui::DragValue::new(&mut s.outline_width).range(0..=24).suffix(" px"));
        ui.color_edit_button_srgba_unmultiplied(&mut s.outline);
        ui.checkbox(&mut s.shadow, "Drop shadow");
    });
    if s.background[3] > 0 {
        ui.add(egui::Slider::new(&mut s.radius, 0..=60).text("Corner radius"));
    }
}
pub fn controls(ui: &mut eframe::egui::Ui, s: &mut Settings) {
    use auroracast_core::widgets::*;
    use eframe::egui;
    if s.kind == Kind::Text {
        text_controls(ui, s);
        return;
    }
    macro_rules! choice {
        ($field:ident,$ty:ident,$label:literal) => {
            ui.horizontal(|ui| {
                ui.label($label);
                egui::ComboBox::from_id_salt(stringify!($field))
                    .selected_text(s.$field.label())
                    .show_ui(ui, |ui| {
                        for &v in $ty::ALL {
                            ui.selectable_value(&mut s.$field, v, v.label());
                        }
                    });
            });
        };
    }
    let before = s.theme;
    choice!(theme, Theme, "Theme");
    if s.theme != before {
        s.apply_theme(s.theme);
    }
    choice!(font, FontFace, "Font");
    ui.horizontal(|ui| {
        ui.add(
            egui::DragValue::new(&mut s.font_size)
                .range(16..=96)
                .suffix(" px"),
        );
        ui.checkbox(&mut s.bold, "Bold");
        ui.label("Accent");
        ui.color_edit_button_srgba_unmultiplied(&mut s.accent);
    });
    ui.add(egui::Slider::new(&mut s.radius, 0..=40).text("Corner radius"));
    if s.kind == Kind::NowPlaying {
        ui.separator();
        ui.strong("Music source");
        ui.checkbox(&mut s.music.automatic, "Detect desktop music automatically");
        if s.music.automatic {
            let (players, status) = crate::music::players();
            egui::ComboBox::from_id_salt("music-player")
                .selected_text(if s.music.player.is_empty() {
                    "Automatic · playing app".into()
                } else {
                    players
                        .iter()
                        .find(|p| p.0 == s.music.player)
                        .map(|p| p.1.clone())
                        .unwrap_or_else(|| "Selected player unavailable".into())
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut s.music.player,
                        String::new(),
                        "Automatic · playing app",
                    );
                    for (id, name) in players {
                        ui.selectable_value(&mut s.music.player, id, name);
                    }
                });
            if s.music.player.is_empty() {
                ui.checkbox(&mut s.music.music_apps_only, "Only music apps")
                    .on_hover_text("Spotify, Amazon Music, Apple Music, TIDAL, YouTube Music and Plexamp, as apps or in a browser. Videos and other browser audio are ignored.");
            }
            ui.small(status);
            if ui.small_button("Check players").clicked() {
                crate::music::reconnect();
            }
            ui.small("Uses Linux desktop media controls (MPRIS). Start playback in your music app or browser. Detection depends on the app exposing track information; no separate music login is needed.");
        }
        ui.checkbox(&mut s.music.hide_paused, "Hide when paused or stopped");
        ui.checkbox(&mut s.music.artwork_colors, "Use album colors");
        ui.checkbox(&mut s.music.large_artwork, "Large artwork layout");
        ui.checkbox(&mut s.music.progress, "Show playback progress");
        ui.checkbox(&mut s.music.animate, "Fade between tracks");
        ui.separator();
        ui.strong("Artist & album text");
        music_font_row(ui, "Artist", &mut s.music.artist_font, &mut s.music.artist_size);
        ui.checkbox(&mut s.music.show_album, "Show album");
        if s.music.show_album {
            music_font_row(ui, "Album", &mut s.music.album_font, &mut s.music.album_size);
        }
    }
    if s.kind == Kind::Timer {
        ui.checkbox(&mut s.timer_up, "Count up (stopwatch)");
    }
    if s.kind == Kind::Chat {
        choice!(layout, Layout, "Layout");
        ui.add(egui::Slider::new(&mut s.spacing, 0..=30).text("Message spacing"));
        ui.checkbox(&mut s.reverse, "Newest message first");
    }
    if s.kind == Kind::CommunityGoal {
        egui::ComboBox::from_id_salt("goal_event")
            .selected_text(
                s.goal_event
                    .map(|k| k.label())
                    .unwrap_or("Manual / automation increments"),
            )
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut s.goal_event, None, "Manual / automation increments");
                for kind in [
                    auroracast_core::EventKind::Follow,
                    auroracast_core::EventKind::Bits,
                    auroracast_core::EventKind::Gift,
                    auroracast_core::EventKind::Subscription,
                    auroracast_core::EventKind::Membership,
                    auroracast_core::EventKind::Subscriber,
                ] {
                    ui.selectable_value(&mut s.goal_event, Some(kind), kind.label());
                }
            });
        ui.small("Counts new verified events, not a fetched lifetime total. Hidden/private YouTube subscribers cannot be counted.");
    }
    if s.kind == Kind::Checklist {
        let mut rows: Vec<String> = s.content.lines().map(String::from).collect();
        let mut changed = false;
        for row in &mut rows {
            let mut done = row.starts_with("[x]");
            let text = row
                .trim_start_matches("[x]")
                .trim_start_matches("[ ]")
                .trim()
                .to_string();
            if ui.checkbox(&mut done, &text).changed() {
                *row = format!("{} {}", if done { "[x]" } else { "[ ]" }, text);
                changed = true;
            }
        }
        if changed {
            s.content = rows.join("\n");
        }
    }
    if s.is_goal() {
        choice!(goal_style, GoalStyle, "Goal appearance");
        choice!(period, Period, "Reset schedule");
        if s.period == Period::Week {
            ui.add(
                egui::DragValue::new(&mut s.reset_day)
                    .range(1..=7)
                    .prefix("Reset day (Monday 1 – Sunday 7): "),
            );
        }
        if s.period == Period::Deadline {
            ui.label("End date (YYYY-MM-DD, inclusive, system timezone)");
            ui.text_edit_singleline(&mut s.deadline);
        }
        ui.small(
            "Stream goals reset only when a new stream starts. Calendar resets use system time.",
        );
    }
    if matches!(
        s.kind,
        Kind::QrCode
            | Kind::StreamLabels
            | Kind::Schedule
            | Kind::NowPlaying
            | Kind::Poll
            | Kind::Checklist
            | Kind::Announcements
            | Kind::Shoutout
    ) && !(s.kind == Kind::NowPlaying && s.music.automatic)
    {
        ui.label(match s.kind {
            Kind::Announcements => "One announcement per line",
            Kind::Checklist => "Checklist text (use [ ] and [x])",
            Kind::Schedule => "Schedule text, including your timezone",
            Kind::NowPlaying => "Track title on the first line, artist on the second",
            Kind::Poll => "Question and results (manual or Update widget automation)",
            _ => "Display text",
        });
        ui.text_edit_multiline(&mut s.content);
    }
    if matches!(s.kind, Kind::Announcements | Kind::Sports) {
        ui.add(egui::Slider::new(&mut s.interval, 2..=60).text("Seconds per card"));
    }
    if s.kind == Kind::Sports {
        ui.separator();
        ui.label("Leagues");
        ui.horizontal(|ui| {
            for &league in League::ALL {
                let mut on = s.sports.leagues.contains(&league);
                if ui.checkbox(&mut on, league.label()).changed() {
                    s.sports.leagues.retain(|v| *v != league);
                    if on {
                        s.sports.leagues.push(league);
                    }
                }
            }
        });
        ui.label("Teams (comma-separated names or abbreviations; empty = all)");
        ui.text_edit_singleline(&mut s.sports.teams);
        ui.horizontal(|ui| {
            ui.checkbox(&mut s.sports.live, "Live");
            ui.checkbox(&mut s.sports.upcoming, "Upcoming");
            ui.checkbox(&mut s.sports.finals, "Final");
        });
        egui::ComboBox::from_id_salt("score_layout")
            .selected_text(s.sports.layout.label())
            .show_ui(ui, |ui| {
                for &v in ScoreLayout::ALL {
                    ui.selectable_value(&mut s.sports.layout, v, v.label());
                }
            });
        if s.sports.layout == ScoreLayout::Ticker {
            ui.add(egui::Slider::new(&mut s.sports.speed, 10..=160).text("Scroll pixels / second"));
            ui.checkbox(&mut s.sports.rightward, "Scroll right");
        }
        ui.small("Yesterday, today and tomorrow. Scores are delayed by provider updates; no live game clock. All widgets share one cache.");
        ui.label(crate::sports::status());
        ui.hyperlink_to("Get a free BALLDONTLIE key", "https://app.balldontlie.io/");
        let id = ui.make_persistent_id("sports_key_entry");
        let mut token = ui
            .ctx()
            .data_mut(|d| d.get_temp::<String>(id).unwrap_or_default());
        ui.add(
            egui::TextEdit::singleline(&mut token)
                .password(true)
                .hint_text("Paste API key"),
        );
        if ui.button("Save sports connection").clicked() && !token.trim().is_empty() {
            match crate::sports::configure(&token) {
                Ok(()) => token.clear(),
                Err(e) => {
                    ui.colored_label(egui::Color32::LIGHT_RED, e.to_string());
                }
            }
        }
        if crate::sports::connected() && ui.button("Disconnect sports").clicked() {
            let _ = crate::sports::configure("");
        }
        ui.ctx().data_mut(|d| d.insert_temp(id, token));
    }
}

fn compose_media_layout(frame: &mut Frame, video: &Frame, fit: bool, layout:auroracast_core::widgets::AlertLayout) {
    if video.width == 0 || video.height == 0 {
        return;
    }
    let sx = frame.width as f32 / video.width as f32;
    let reserved=if layout==auroracast_core::widgets::AlertLayout::Over{0.}else{frame.height as f32*0.3};
    let offset=if layout==auroracast_core::widgets::AlertLayout::Above{reserved}else{0.};
    let sy = (frame.height as f32-reserved) / video.height as f32;
    let scale = if fit { sx.min(sy) } else { sx.max(sy) };
    let w = video.width as f32 * scale;
    let h = video.height as f32 * scale;
    let dx = (frame.width as f32 - w) / 2.;
    let dy = offset+(frame.height as f32-reserved-h)/2.;
    for y in 0..frame.height {
        for x in 0..frame.width {
            if (y as f32)<offset || (y as f32)>=offset+frame.height as f32-reserved {continue;}
            let vx = (x as f32 - dx) / scale;
            let vy = (y as f32 - dy) / scale;
            if vx < 0. || vy < 0. || vx >= video.width as f32 || vy >= video.height as f32 {
                continue;
            }
            let a = ((y * frame.width + x) * 4) as usize;
            let b = ((vy as u32 * video.width + vx as u32) * 4) as usize;
            let overlay = frame.rgba[a + 3] as f32 / 255.;
            let base = video.rgba[b + 3] as f32 / 255.;
            let alpha = overlay + base * (1. - overlay);
            for c in 0..3 {
                frame.rgba[a + c] = if alpha > 0. {
                    ((frame.rgba[a + c] as f32 * overlay
                        + video.rgba[b + c] as f32 * base * (1. - overlay))
                        / alpha) as u8
                } else {
                    0
                }
            }
            frame.rgba[a + 3] = (alpha * 255.) as u8;
        }
    }
}

#[cfg(test)]
mod widget_regressions {
    use super::*;
    #[test]
    fn old_settings_load_with_defaults() {
        let s: Settings = serde_json::from_str(r#"{"kind":"Chat","label":"Chat"}"#).unwrap();
        assert_eq!(s.font_size, 36);
        assert_eq!(s.period, Period::Manual);
    }
    #[test]
    fn old_donation_does_not_enter_current_week() {
        let h = Hub::default();
        h.0.lock().unwrap().insert(
            1,
            Arc::new(Mutex::new(State::new(Settings {
                kind: Kind::DonationGoal,
                period: Period::Week,
                ..Default::default()
            }))),
        );
        h.donation_at("USD", 1000, 1, true);
        assert_eq!(h.0.lock().unwrap()[&1].lock().unwrap().value, 0);
        h.donation_at("USD", 1000, crate::activity::now(), false);
        assert_eq!(h.0.lock().unwrap()[&1].lock().unwrap().value, 1000);
    }
    #[test]
    fn weekly_reset_happens_once() {
        let mut s = State::new(Settings {
            kind: Kind::SubGoal,
            period: Period::Week,
            initial: 5,
            ..Default::default()
        });
        s.period_key = "old week".into();
        s.value = 88;
        s.refresh_period();
        assert_eq!(s.value, 5);
        s.value = 6;
        s.refresh_period();
        assert_eq!(s.value, 6);
    }
    #[test]
    fn preview_alert_does_not_change_goal() {
        let h = Hub::default();
        h.0.lock().unwrap().insert(
            1,
            Arc::new(Mutex::new(State::new(Settings {
                kind: Kind::DonationGoal,
                initial: 1200,
                ..Default::default()
            }))),
        );
        h.0.lock().unwrap().insert(
            2,
            Arc::new(Mutex::new(State::new(Settings {
                kind: Kind::AlertBox,
                ..Default::default()
            }))),
        );
        h.preview_alert(2);
        assert_eq!(h.0.lock().unwrap()[&1].lock().unwrap().value, 1200);
        assert_eq!(h.0.lock().unwrap()[&2].lock().unwrap().queue.len(), 1);
    }
    #[test]
    fn idle_alert_is_fully_transparent() {
        let f = render(
            &Settings {
                kind: Kind::AlertBox,
                ..Default::default()
            },
            "",
        )
        .unwrap();
        assert!(f.rgba.chunks_exact(4).all(|p| p[3] == 0));
    }
    #[test]
    fn all_themes_and_goal_styles_render() {
        for &theme in Theme::ALL {
            let mut s = Settings {
                kind: Kind::Chat,
                ..Default::default()
            };
            s.apply_theme(theme);
            let f = render(
                &s,
                "Chat\nTwitch · viewer: Hello everyone!\nYouTube · viewer: Looking good",
            )
            .unwrap();
            assert_eq!(f.rgba.len(), 960 * 400 * 4);
            if let Ok(dir) = std::env::var("ROSORAH_WIDGET_QA") {
                std::fs::create_dir_all(&dir).unwrap();
                image::save_buffer(
                    std::path::Path::new(&dir).join(format!("chat-{theme:?}.png")),
                    &f.rgba,
                    f.width,
                    f.height,
                    image::ColorType::Rgba8,
                )
                .unwrap();
            }
        }
        for &style in GoalStyle::ALL {
            let s = Settings {
                kind: Kind::CommunityGoal,
                goal_style: style,
                ..Default::default()
            };
            let f = render_value(&s, "Community goal\n60 / 100", 60).unwrap();
            assert_eq!(f.rgba.len(), 960 * 400 * 4);
            if let Ok(dir) = std::env::var("ROSORAH_WIDGET_QA") {
                image::save_buffer(
                    std::path::Path::new(&dir).join(format!("goal-{style:?}.png")),
                    &f.rgba,
                    f.width,
                    f.height,
                    image::ColorType::Rgba8,
                )
                .unwrap();
            }
        }
    }
    #[test]
    fn media_fits_and_preserves_text_alpha() {
        let mut f = Frame {
            width: 4,
            height: 4,
            rgba: vec![0; 64],
        };
        let v = Frame {
            width: 2,
            height: 1,
            rgba: [255, 0, 0, 255].repeat(2),
        };
        compose_media_layout(&mut f, &v, true, auroracast_core::widgets::AlertLayout::Over);
        assert_eq!(f.rgba[3], 0);
        assert_eq!(f.rgba[(1 * 4 + 1) * 4], 255);
    }
}
#[cfg(test)]mod alert_layout_tests{use super::*;use auroracast_core::widgets::AlertLayout;
 #[test]fn below_text_region_is_never_covered_by_media(){let mut f=Frame{width:10,height:10,rgba:vec![0;400]};let video=Frame{width:1,height:1,rgba:vec![255,0,0,255]};compose_media_layout(&mut f,&video,false,AlertLayout::Below);assert_eq!(f.rgba[3],255);assert_eq!(f.rgba[(9*10)*4+3],0);}
 #[test]fn targeted_alerts_do_not_broadcast_to_other_boxes(){let hub=Hub::default();for id in [1,2]{hub.0.lock().unwrap().insert(id,Arc::new(Mutex::new(State::new(Settings{kind:Kind::AlertBox,..Default::default()}))));}hub.target_alert(1,"Hello",5,None);assert_eq!(hub.0.lock().unwrap()[&1].lock().unwrap().queue.len(),1);assert!(hub.0.lock().unwrap()[&2].lock().unwrap().queue.is_empty());}
}
#[cfg(test)]
mod music_text_tests {
    use super::*;
    #[test]
    fn artist_and_album_fonts_and_sizes_change_the_widget() {
        let mut track = crate::music::sample();
        track.album = "Midnight Album".into();
        let base = Settings { kind: Kind::NowPlaying, ..Default::default() };
        let draw = |s: &Settings| polished_music(s, Some(&track)).rgba;
        let plain = draw(&base);
        let mut bigger = base.clone();
        bigger.music.artist_size = 40;
        assert_ne!(draw(&bigger), plain, "artist size should change the drawing");
        let mut album = base.clone();
        album.music.show_album = true;
        assert_ne!(draw(&album), plain, "album line should appear");
        let mut font = base.clone();
        font.music.artist_font = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts/Roboto.ttf").into();
        font.font = FontFace::Mono;
        let mono = Settings { font: FontFace::Mono, ..base.clone() };
        assert_ne!(draw(&font), draw(&mono), "a chosen artist font should differ from the widget font");
        let mut missing = base.clone();
        missing.music.artist_font = "/no/such/font.ttf".into();
        assert_eq!(draw(&missing), plain, "an unreadable font falls back to the widget font");
    }
}
