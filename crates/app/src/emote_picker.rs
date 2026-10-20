//! The chat composer's picker: Twemoji emoji plus the emotes each connected account can send.
use crate::chat_render::{Emote, RichChat};
use eframe::egui::{self, Sense, vec2};

const CELL: f32 = 42.;

/// One tab of platform emotes.
pub struct Source<'a> {
    pub label: &'a str,
    pub emotes: Vec<Emote>,
    /// Shown when the list is empty or incomplete.
    pub note: String,
}
#[derive(Default)]
pub struct Picker {
    tab: usize,
    search: String,
}
impl Picker {
    /// The picker's contents, inside the popup under its button. Drag the corner to resize;
    /// the size is kept.
    pub fn show(&mut self, ui: &mut egui::Ui, rich: &RichChat, sources: &[Source], wanted: &mut Vec<String>) -> Option<String> {
        let mut picked = None;
        egui::Resize::default()
            .id_salt("emote-picker-size")
            .default_size([440., 520.])
            .min_size([300., 260.])
            .max_size([1000., 1000.])
            .show(ui, |ui| picked = self.contents(ui, rich, sources, wanted));
        picked
    }
    fn contents(&mut self, ui: &mut egui::Ui, rich: &RichChat, sources: &[Source], wanted: &mut Vec<String>) -> Option<String> {
        let mut picked = None;
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.tab, 0, "Emoji");
            for (i, source) in sources.iter().enumerate() {
                ui.selectable_value(&mut self.tab, i + 1, source.label);
            }
        });
        if self.tab > sources.len() {
            self.tab = 0;
        }
        ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Search").desired_width(f32::INFINITY));
        let search = self.search.trim().to_lowercase();
        egui::ScrollArea::vertical().id_salt(("emote-picker", self.tab)).auto_shrink([false, false]).show(ui, |ui| {
            if self.tab == 0 {
                let groups = crate::twemoji::groups();
                for (index, group) in groups.iter().enumerate() {
                    let entries: Vec<_> = crate::twemoji::picker()
                        .iter()
                        .filter(|e| e.group == index && (search.is_empty() || e.name.contains(&search)))
                        .collect();
                    if entries.is_empty() {
                        continue;
                    }
                    ui.label(egui::RichText::new(group).small().weak());
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(2., 2.);
                        for entry in entries {
                            if emoji_cell(ui, &entry.emoji).on_hover_text(&entry.name).clicked() {
                                picked = Some(entry.emoji.clone());
                            }
                        }
                    });
                }
                ui.add_space(6.);
                ui.label(egui::RichText::new("Emoji graphics: Twemoji, © Twitter, Inc and other contributors, CC-BY 4.0").small().weak());
                return;
            }
            let source = &sources[self.tab - 1];
            if !source.note.is_empty() {
                ui.label(egui::RichText::new(&source.note).small().weak());
            }
            let emotes: Vec<_> = source.emotes.iter().filter(|e| search.is_empty() || e.name.to_lowercase().contains(&search)).collect();
            if emotes.is_empty() && source.note.is_empty() {
                ui.label(egui::RichText::new(if search.is_empty() { "No emotes yet." } else { "No matches." }).weak());
            }
            let mut groups: Vec<&str> = Vec::new();
            for e in &emotes {
                if !groups.contains(&e.group.as_str()) {
                    groups.push(&e.group);
                }
            }
            for group in groups {
                ui.label(egui::RichText::new(group).small().weak());
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(2., 2.);
                    for e in emotes.iter().filter(|e| e.group == group) {
                        if emote_cell(ui, rich, e, wanted).on_hover_text(&e.name).clicked() {
                            picked = Some(format!(" {} ", e.insert));
                        }
                    }
                });
            }
        });
        picked
    }
}
fn cell(ui: &mut egui::Ui) -> (egui::Rect, egui::Response, bool) {
    let (rect, response) = ui.allocate_exact_size(vec2(CELL, CELL), Sense::click());
    let visible = ui.is_rect_visible(rect);
    if visible && response.hovered() {
        ui.painter().rect_filled(rect, 4., ui.visuals().widgets.hovered.weak_bg_fill);
    }
    (rect, response, visible)
}
fn emoji_cell(ui: &mut egui::Ui, emoji: &str) -> egui::Response {
    let (rect, response, visible) = cell(ui);
    // Pictures are only decoded once they scroll into view.
    if visible {
        let key: String = emoji.chars().filter(|c| *c != '\u{fe0f}').collect();
        if let Some(t) = crate::twemoji::texture(ui.ctx(), &key) {
            egui::Image::new((t.id(), vec2(32., 32.))).paint_at(ui, rect.shrink(5.));
        }
    }
    response
}
fn emote_cell(ui: &mut egui::Ui, rich: &RichChat, e: &Emote, wanted: &mut Vec<String>) -> egui::Response {
    let (rect, response, visible) = cell(ui);
    if visible {
        match rich.frame(&e.src, 32.) {
            Some((texture, size)) => {
                let size = if size.x > CELL - 4. { size * ((CELL - 4.) / size.x) } else { size };
                egui::Image::new((texture, size)).paint_at(ui, egui::Rect::from_center_size(rect.center(), size));
            }
            None => {
                wanted.push(e.src.clone());
                let name: String = e.name.chars().take(5).collect();
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, name, egui::FontId::proportional(10.), ui.visuals().weak_text_color());
            }
        }
    }
    response
}
/// The picker's text for an emote seen in chat. Kick needs its emote code, read from the picture's address.
pub fn seen_emote(platform: &str, name: &str, src: &str) -> Emote {
    let kick_id = src
        .strip_prefix("https://files.kick.com/emotes/")
        .and_then(|rest| rest.split('/').next())
        .filter(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()));
    let insert = match kick_id {
        Some(id) if platform == "Kick" => format!("[emote:{id}:{name}]"),
        _ => name.to_string(),
    };
    Emote { group: "Seen in chat".into(), name: name.into(), insert, src: src.into() }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn kick_emotes_seen_in_chat_keep_their_code() {
        assert_eq!(seen_emote("Kick", "xqcL", "https://files.kick.com/emotes/37226/fullsize").insert, "[emote:37226:xqcL]");
        assert_eq!(seen_emote("Twitch", "Kappa", "https://static-cdn.jtvnw.net/emoticons/v2/25/default/dark/1.0").insert, "Kappa");
        assert_eq!(seen_emote("Kick", "odd", "https://files.kick.com/emotes/x1/fullsize").insert, "odd");
    }
}
