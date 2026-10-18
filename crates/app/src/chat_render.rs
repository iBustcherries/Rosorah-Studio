//! Rich chat stays separate from the persisted project and text/event feed.
use base64::Engine;
use eframe::egui::{self, ColorImage, TextureHandle};
use serde::Deserialize;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};
#[derive(Clone, Deserialize)]
pub struct Part {
    pub text: String,
    pub src: Option<String>,
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
fn safe_image(src: &str) -> bool {
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
        // At most 32 MiB of 64x64 RGBA frames, plus 128 distinct assets.
        while self.assets.len() > 128
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
            ui.add(
                egui::Label::new(egui::RichText::new(fallback).size(size))
                    .selectable(true)
                    .wrap(),
            );
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
                ui.add(
                    egui::Label::new(egui::RichText::new(&part.text).size(size))
                        .selectable(true)
                        .wrap(),
                );
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
}
