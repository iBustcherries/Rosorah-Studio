//! Rich chat stays separate from the persisted project and text/event feed.
use base64::Engine;
use eframe::egui::{self, ColorImage, TextureHandle};
use serde::Deserialize;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
#[derive(Clone, Deserialize)]
pub struct Part {
    pub text: String,
    pub src: Option<String>,
}
/// An emote the streamer can send, for the chat emote picker.
#[derive(Clone, Debug, PartialEq)]
pub struct Emote {
    pub group: String,
    pub name: String,
    /// The text that goes into the message.
    pub insert: String,
    pub src: String,
}
struct Frame {
    texture: TextureHandle,
    delay: u64,
}
struct Asset {
    url: String,
    frames: Vec<Frame>,
    started: Instant,
}
#[derive(Default)]
pub struct RichChat {
    messages: VecDeque<(String, Vec<Part>)>,
    assets: VecDeque<Asset>,
}
/// Emote frames for the on-canvas chat widget, which draws on its own thread.
type CanvasFrames = Vec<(Arc<image::RgbaImage>, u64)>;
fn canvas() -> &'static Mutex<VecDeque<(String, CanvasFrames)>> {
    static CANVAS: OnceLock<Mutex<VecDeque<(String, CanvasFrames)>>> = OnceLock::new();
    CANVAS.get_or_init(Default::default)
}
static CANVAS_REVISION: AtomicU64 = AtomicU64::new(0);
/// Changes whenever an emote picture arrives, so chat widgets redraw with it.
pub fn canvas_revision() -> u64 {
    CANVAS_REVISION.load(Ordering::Relaxed)
}
/// A picture the chat widget shows (such as a chatter's) arrived.
pub fn canvas_changed() {
    CANVAS_REVISION.fetch_add(1, Ordering::Relaxed);
}
/// The frame of an emote to show `at_ms` into its animation.
pub fn canvas_frame(url: &str, at_ms: u64) -> Option<Arc<image::RgbaImage>> {
    let canvas = canvas().lock().unwrap();
    let (_, frames) = canvas.iter().find(|(u, f)| u == url && !f.is_empty())?;
    let cycle = frames.iter().map(|f| f.1).sum::<u64>().max(1);
    let mut offset = at_ms % cycle;
    for (frame, delay) in frames {
        if offset < *delay {
            return Some(frame.clone());
        }
        offset -= delay;
    }
    Some(frames[0].0.clone())
}
pub fn canvas_animated(url: &str) -> bool {
    canvas().lock().unwrap().iter().any(|(u, f)| u == url && f.len() > 1)
}
/// Emotes travel through widget text as U+E000 url U+E001 name U+E002 (private-use characters).
pub const EMOTE: [char; 3] = ['\u{e000}', '\u{e001}', '\u{e002}'];
/// Message text with its emotes marked, at most `limit` visible characters.
pub fn marked(parts: &[Part], limit: usize) -> String {
    let mut out = String::new();
    let mut left = limit;
    for part in parts {
        if left == 0 {
            break;
        }
        match &part.src {
            Some(src) => {
                let name: String = part.text.chars().filter(|c| !c.is_whitespace() && !EMOTE.contains(c)).take(100).collect();
                out.push(EMOTE[0]);
                out.push_str(src);
                out.push(EMOTE[1]);
                out.push_str(&name);
                out.push(EMOTE[2]);
                left -= 1;
            }
            None => {
                let text: String = part.text.chars().filter(|c| !EMOTE.contains(c)).take(left).collect();
                left -= text.chars().count();
                out.push_str(&text);
            }
        }
    }
    out
}
pub fn safe_image(src: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(src) else {
        return false;
    };
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none_or(|p| p == 443)
        && (matches!(
            url.host_str(),
            Some(
                "static-cdn.jtvnw.net"
                    | "yt3.ggpht.com"
                    | "yt3.googleusercontent.com"
                    | "www.gstatic.com"
                    | "fonts.gstatic.com"
                    | "i.ytimg.com"
                    | "files.kick.com"
            )
        ) || (url.host_str() == Some("www.youtube.com")
            && url.path().starts_with("/s/gaming/emoji/")))
        && src.len() <= 2048
}
impl RichChat {
    pub fn message(&mut self, key: String, value: &serde_json::Value) {
        let Ok(mut parts) = serde_json::from_value::<Vec<Part>>(value.clone()) else {
            return;
        };
        parts.truncate(128);
        for p in &mut parts {
            p.text = p.text.chars().take(5000).collect();
            if p.src.as_ref().is_some_and(|s| !safe_image(s)) {
                p.src = None;
            }
        }
        self.messages.retain(|(k, _)| k != &key);
        self.messages.push_back((key, parts));
        while self.messages.len() > 500 {
            self.messages.pop_front();
        }
    }
    pub fn image(&mut self, ctx: &egui::Context, value: &serde_json::Value) {
        let Some(url) = value["url"].as_str().filter(|s| safe_image(s)) else {
            return;
        };
        let Some(png) = value["png"].as_str().filter(|s| s.len() <= 65536) else {
            return;
        };
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(png) else {
            return;
        };
        let Ok(img) = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png) else {
            return;
        };
        if img.width() > 64 || img.height() > 64 || img.width() == 0 || img.height() == 0 {
            return;
        }
        let index = value["index"].as_u64().unwrap_or(999) as usize;
        if index >= 120 {
            return;
        }
        if index == 0 {
            self.assets.retain(|a| a.url != url);
            self.assets.push_back(Asset {
                url: url.into(),
                frames: Vec::new(),
                started: Instant::now(),
            });
        }
        let Some(asset) = self.assets.iter_mut().find(|a| a.url == url) else {
            return;
        };
        if index != asset.frames.len() {
            return;
        }
        let rgba = img.to_rgba8();
        {
            let delay = value["delay"].as_u64().unwrap_or(100).clamp(20, 10000);
            let mut canvas = canvas().lock().unwrap();
            if index == 0 {
                canvas.retain(|(u, _)| u != url);
                canvas.push_back((url.into(), Vec::new()));
                while canvas.len() > 256 {
                    canvas.pop_front();
                }
            }
            if let Some((_, frames)) = canvas.iter_mut().find(|(u, _)| u == url)
                && frames.len() == index
            {
                frames.push((Arc::new(rgba.clone()), delay));
                CANVAS_REVISION.fetch_add(1, Ordering::Relaxed);
            }
        }
        let color = ColorImage::from_rgba_unmultiplied(
            [img.width() as usize, img.height() as usize],
            rgba.as_raw(),
        );
        asset.frames.push(Frame {
            texture: ctx.load_texture(
                format!("chat-emote-{url}-{index}"),
                color,
                egui::TextureOptions::LINEAR,
            ),
            delay: value["delay"].as_u64().unwrap_or(100).clamp(20, 10000),
        });
        // At most 32 MiB of 64x64 RGBA frames, plus 512 distinct assets (chat and the emote picker).
        while self.assets.len() > 512
            || self.assets.iter().map(|a| a.frames.len()).sum::<usize>() > 2048
        {
            self.assets.pop_front();
        }
    }
    pub fn recent(&self, platform: &str) -> Vec<(String, String)> {
        let mut result = Vec::new();
        for (key, parts) in self
            .messages
            .iter()
            .rev()
            .filter(|(k, _)| k.starts_with(platform))
        {
            let _ = key;
            for part in parts
                .iter()
                .filter(|p| p.src.is_some() && !p.text.is_empty())
            {
                if !result.iter().any(|(text, _)| text == &part.text) {
                    result.push((part.text.clone(), part.src.clone().unwrap()));
                }
                if result.len() >= 40 {
                    return result;
                }
            }
        }
        result
    }
    pub fn parts(&self, key: &str) -> Option<&[Part]> {
        self.messages.iter().rev().find(|(k, _)| k == key).map(|(_, p)| p.as_slice())
    }
    pub fn loaded(&self, src: &str) -> bool {
        self.assets.iter().any(|a| a.url == src && !a.frames.is_empty())
    }
    /// The current frame of a loaded emote, `size` points tall.
    pub fn frame(&self, src: &str, size: f32) -> Option<(egui::TextureId, egui::Vec2)> {
        let asset = self.assets.iter().find(|a| a.url == src && !a.frames.is_empty())?;
        let frame = &asset.frames[0];
        let dims = frame.texture.size_vec2();
        Some((frame.texture.id(), dims * (size / dims.y)))
    }
    fn emote(&self, ui: &mut egui::Ui, src: &str, size: f32) -> bool {
        let Some(asset) = self
            .assets
            .iter()
            .find(|a| a.url == src && !a.frames.is_empty())
        else {
            return false;
        };
        let cycle = asset.frames.iter().map(|f| f.delay).sum::<u64>();
        let mut offset = (asset.started.elapsed().as_millis() % cycle as u128) as u64;
        let frame = asset
            .frames
            .iter()
            .find(|f| {
                if offset < f.delay {
                    true
                } else {
                    offset -= f.delay;
                    false
                }
            })
            .unwrap_or(&asset.frames[0]);
        let dims = frame.texture.size_vec2();
        ui.image((frame.texture.id(), dims * (size / dims.y)));
        if asset.frames.len() > 1 {
            ui.ctx()
                .request_repaint_after(Duration::from_millis(frame.delay - offset));
        }
        true
    }
    pub fn draw(&self, ui: &mut egui::Ui, key: &str, fallback: &str, size: f32) {
        let Some((_, parts)) = self
            .messages
            .iter()
            .find(|(k, p)| k == key && !p.is_empty())
        else {
            if crate::twemoji::contains(fallback) {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.;
                    crate::twemoji::text(ui, fallback, size);
                });
            } else {
                ui.add(
                    egui::Label::new(egui::RichText::new(fallback).size(size))
                        .selectable(true)
                        .wrap(),
                );
            }
            return;
        };
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.;
            for part in parts {
                if let Some(src) = &part.src
                    && self.emote(ui, src, size * 1.7)
                {
                    continue;
                }
                crate::twemoji::text(ui, &part.text, size);
            }
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn untrusted_images_and_bounded_messages() {
        for bad in [
            "http://static-cdn.jtvnw.net/x",
            "https://static-cdn.jtvnw.net.evil.test/x",
            "file:///etc/passwd",
            "https://user@www.gstatic.com/x",
        ] {
            assert!(!safe_image(bad));
        }
        let mut rich = RichChat::default();
        for i in 0..510 {
            rich.message(
                i.to_string(),
                &serde_json::json!([{"text":"👋 Kappa","src":"https://evil.test/x"}]),
            );
        }
        assert_eq!(rich.messages.len(), 500);
        assert!(rich.messages[0].1[0].src.is_none());
        rich.message("509".into(),&serde_json::json!([{"text":"Kappa","src":"https://static-cdn.jtvnw.net/emoticons/test"}]));
        assert_eq!(rich.messages.len(), 500);
        assert!(rich.messages.back().unwrap().1[0].src.is_some());
    }
    #[test]
    fn marked_text_keeps_emotes_and_limits_length() {
        let parts = [
            Part { text: "hi ".into(), src: None },
            Part { text: "Kappa".into(), src: Some("https://static-cdn.jtvnw.net/e/1".into()) },
            Part { text: " there".into(), src: None },
        ];
        assert_eq!(marked(&parts, 240), "hi \u{e000}https://static-cdn.jtvnw.net/e/1\u{e001}Kappa\u{e002} there");
        assert_eq!(marked(&parts, 4), "hi \u{e000}https://static-cdn.jtvnw.net/e/1\u{e001}Kappa\u{e002}");
        assert_eq!(marked(&[Part { text: "a\u{e000}b".into(), src: None }], 9), "ab");
    }
}
