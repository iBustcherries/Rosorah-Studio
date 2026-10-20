//! Colour emoji from Twemoji (graphics CC-BY 4.0), packed by packaging/twemoji/build-pack.py.
use eframe::egui::{self, ColorImage, TextureHandle};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, OnceLock},
};
static PACK: &[u8] = include_bytes!("../assets/twemoji/twemoji.pack");
pub struct Entry {
    pub group: usize,
    pub emoji: String,
    pub name: String,
}
struct Index {
    groups: Vec<String>,
    /// Key (the emoji without U+FE0F) → (PNG range, needs U+FE0F when standalone).
    images: HashMap<String, (usize, usize, bool)>,
    starts: HashSet<char>,
    longest: usize,
    picker: Vec<Entry>,
}
fn index() -> &'static Index {
    static INDEX: OnceLock<Index> = OnceLock::new();
    INDEX.get_or_init(|| parse(PACK).unwrap_or(Index { groups: vec![], images: HashMap::new(), starts: HashSet::new(), longest: 0, picker: vec![] }))
}
fn parse(data: &'static [u8]) -> Option<Index> {
    let mut r = Reader { data, at: 0 };
    if r.bytes(4)? != b"RTW1" {
        return None;
    }
    let groups = (0..r.u8()?).map(|_| r.string()).collect::<Option<Vec<_>>>()?;
    let mut images = HashMap::new();
    let mut starts = HashSet::new();
    let mut longest = 0;
    for _ in 0..r.u32()? {
        let key = r.string()?;
        let flags = r.u8()?;
        let len = r.u32()? as usize;
        let start = r.at;
        r.bytes(len)?;
        starts.insert(key.chars().next()?);
        longest = longest.max(key.chars().count());
        images.insert(key, (start, len, flags & 1 != 0));
    }
    let mut picker = Vec::new();
    for _ in 0..r.u32()? {
        let group = r.u8()? as usize;
        let emoji = r.string()?;
        let name = r.string()?;
        picker.push(Entry { group, emoji, name });
    }
    Some(Index { groups, images, starts, longest, picker })
}
struct Reader {
    data: &'static [u8],
    at: usize,
}
impl Reader {
    fn bytes(&mut self, n: usize) -> Option<&'static [u8]> {
        let slice = self.data.get(self.at..self.at + n)?;
        self.at += n;
        Some(slice)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.bytes(1)?[0])
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.bytes(4)?.try_into().ok()?))
    }
    fn string(&mut self) -> Option<String> {
        let n = self.u8()? as usize;
        String::from_utf8(self.bytes(n)?.to_vec()).ok()
    }
}
pub fn groups() -> &'static [String] {
    &index().groups
}
pub fn picker() -> &'static [Entry] {
    &index().picker
}
/// A run of text: plain, or one emoji (its image key).
#[derive(Debug, PartialEq)]
pub enum Piece<'a> {
    Text(&'a str),
    Emoji(&'a str, String),
}
/// Splits text into plain runs and emoji, longest sequence first.
pub fn split(text: &str) -> Vec<Piece<'_>> {
    let index = index();
    let mut pieces = Vec::new();
    let mut plain = 0;
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let first = rest.chars().next().unwrap();
        if index.starts.contains(&first)
            && let Some((len, key)) = longest_match(index, rest)
        {
            if plain < i {
                pieces.push(Piece::Text(&text[plain..i]));
            }
            pieces.push(Piece::Emoji(&text[i..i + len], key));
            i += len;
            plain = i;
            continue;
        }
        i += first.len_utf8();
    }
    if plain < text.len() {
        pieces.push(Piece::Text(&text[plain..]));
    }
    pieces
}
fn longest_match(index: &Index, rest: &str) -> Option<(usize, String)> {
    let mut key = String::new();
    let mut best = None;
    let mut count = 0;
    for (at, c) in rest.char_indices() {
        let end = at + c.len_utf8();
        if c == '\u{fe0f}' {
            // A selector belongs to the emoji before it.
            if let Some((len, _)) = &mut best
                && *len == at
            {
                *len = end;
            }
            continue;
        }
        if count >= index.longest {
            break;
        }
        key.push(c);
        count += 1;
        if let Some(&(_, _, text_default)) = index.images.get(&key) {
            let selected = rest[end..].starts_with('\u{fe0f}');
            if !(text_default && count == 1 && !selected) {
                best = Some((end, key.clone()));
            }
        }
    }
    best
}
pub fn contains(text: &str) -> bool {
    split(text).iter().any(|p| matches!(p, Piece::Emoji(..)))
}
fn png(key: &str) -> Option<&'static [u8]> {
    let &(start, len, _) = index().images.get(key)?;
    PACK.get(start..start + len)
}
/// Decoded 72x72 RGBA, kept for the most recent emoji.
pub fn rgba(key: &str) -> Option<Arc<image::RgbaImage>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<image::RgbaImage>>>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap();
    if let Some(image) = cache.get(key) {
        return Some(image.clone());
    }
    let image = Arc::new(image::load_from_memory_with_format(png(key)?, image::ImageFormat::Png).ok()?.to_rgba8());
    if cache.len() > 512 {
        cache.clear();
    }
    cache.insert(key.into(), image.clone());
    Some(image)
}
pub fn texture(ctx: &egui::Context, key: &str) -> Option<TextureHandle> {
    static TEXTURES: OnceLock<Mutex<HashMap<String, TextureHandle>>> = OnceLock::new();
    let mut textures = TEXTURES.get_or_init(Default::default).lock().unwrap();
    if let Some(t) = textures.get(key) {
        return Some(t.clone());
    }
    let image = rgba(key)?;
    let color = ColorImage::from_rgba_unmultiplied([image.width() as usize, image.height() as usize], image.as_raw());
    let t = ctx.load_texture(format!("twemoji-{key}"), color, egui::TextureOptions::LINEAR);
    textures.insert(key.into(), t.clone());
    Some(t)
}
/// Text with emoji drawn as Twemoji images, for use inside a wrapped horizontal layout.
pub fn text(ui: &mut egui::Ui, text: &str, size: f32) {
    for piece in split(text) {
        match piece {
            Piece::Emoji(raw, key) => match texture(ui.ctx(), &key) {
                Some(t) => {
                    ui.image((t.id(), egui::vec2(size * 1.25, size * 1.25))).on_hover_text(raw);
                }
                None => {
                    ui.add(egui::Label::new(egui::RichText::new(raw).size(size)).selectable(true).wrap());
                }
            },
            Piece::Text(t) => {
                ui.add(egui::Label::new(egui::RichText::new(t).size(size)).selectable(true).wrap());
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pack_loads_and_splits() {
        assert!(index().images.len() > 3000);
        assert!(picker().len() > 1500);
        assert!(!groups().is_empty());
        assert!(rgba("😀").is_some_and(|i| i.width() == 72));
        let pieces = split("hi 😀!");
        assert_eq!(pieces, [Piece::Text("hi "), Piece::Emoji("😀", "😀".into()), Piece::Text("!")]);
        // ZWJ family and skin tone sequences stay one emoji, selectors included.
        assert_eq!(split("👍🏽").len(), 1);
        assert_eq!(split("❤️‍🔥").len(), 1);
        assert_eq!(split("❤️ x").len(), 2);
        // Text-style symbols stay text unless U+FE0F asks for the emoji.
        assert_eq!(split("© 2026"), [Piece::Text("© 2026")]);
        assert!(matches!(split("©️")[0], Piece::Emoji(..)));
        assert_eq!(split("plain text"), [Piece::Text("plain text")]);
        assert!(contains("go 🎉") && !contains("go"));
    }
}
