//! Fonts bundled with Rosorah (the `fonts` folder beside the program) and installed on
//! this computer, for widget text. .ttf/.otf files and the first face of .ttc/.otc
//! collections are listed; names come from the file name.
use ab_glyph::FontArc;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

/// (display name, file path), sorted by name. Scanned once.
pub fn installed() -> &'static [(String, String)] {
    static LIST: OnceLock<Vec<(String, String)>> = OnceLock::new();
    LIST.get_or_init(|| {
        let mut found = Vec::new();
        for dir in folders() {
            scan(&dir, 0, &mut found);
        }
        let mut list: Vec<_> = found.into_iter().map(|p| (display_name(&p), p.to_string_lossy().into_owned())).collect();
        list.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()).then(a.1.cmp(&b.1)));
        list.dedup_by(|a, b| a.0 == b.0);
        list
    })
}

fn folders() -> Vec<PathBuf> {
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let mut bundled = vec![crate::browser_chat::base().with_file_name("fonts")];
    if cfg!(debug_assertions) {
        bundled.push(PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fonts")));
    }
    bundled.extend(system_folders(env));
    bundled
}
fn system_folders(env: impl Fn(&str) -> Option<PathBuf>) -> Vec<PathBuf> {
    if cfg!(windows) {
        let mut dirs = vec![env("WINDIR").unwrap_or_else(|| PathBuf::from("C:/Windows")).join("Fonts")];
        dirs.extend(env("LOCALAPPDATA").map(|d| d.join("Microsoft/Windows/Fonts")));
        dirs
    } else {
        let mut dirs = vec![PathBuf::from("/usr/share/fonts"), PathBuf::from("/usr/local/share/fonts")];
        if let Some(home) = env("HOME") {
            dirs.push(env("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local/share")).join("fonts"));
            dirs.push(home.join(".fonts"));
        }
        dirs
    }
}

fn scan(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth < 5 {
                scan(&path, depth + 1, out);
            }
        } else if path.extension().and_then(|e| e.to_str()).is_some_and(|e| ["ttf", "otf", "ttc", "otc"].iter().any(|x| e.eq_ignore_ascii_case(x))) {
            out.push(path);
        }
    }
}

/// "LiberationSans-BoldItalic.ttf" -> "Liberation Sans Bold Italic".
pub fn display_name(path: &Path) -> String {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut out = String::new();
    let mut prev: Option<char> = None;
    for c in stem.chars() {
        let c = if c == '-' || c == '_' { ' ' } else { c };
        if c.is_uppercase() && prev.is_some_and(|p| p.is_lowercase()) {
            out.push(' ');
        }
        if !(c == ' ' && out.ends_with(' ')) {
            out.push(c);
        }
        prev = Some(c);
    }
    let mut name = out.trim().to_owned();
    if let Some(first) = name.get(0..1) {
        name.replace_range(0..1, &first.to_uppercase());
    }
    name
}

/// The font file best matching a family name such as "Arial bold italic", or `None`.
pub fn find(face: &str) -> Option<String> {
    let lower = face.to_lowercase();
    let bold = lower.ends_with(" bold") || lower.contains(" bold ");
    let italic = lower.ends_with(" italic");
    let family: String = lower.replace(" italic", "").replace(" bold", "").chars().filter(|c| c.is_alphanumeric()).collect();
    if family.is_empty() {
        return None;
    }
    let squash = |s: &str| s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect::<String>();
    installed()
        .iter()
        .filter(|(name, _)| squash(name).starts_with(&family))
        .min_by_key(|(name, _)| {
            let n = name.to_lowercase();
            let style = n.contains("bold") as i32 != bold as i32;
            let slant = (n.contains("italic") || n.contains("oblique")) as i32 != italic as i32;
            // Prefer the matching style, then the plain face, then the shortest name.
            (style as i32 * 4 + slant as i32 * 2, (!n.contains("regular")) as i32, n.len())
        })
        .map(|(_, path)| path.clone())
}
/// Loads a font file once; `None` if it is missing or unreadable.
pub fn load(path: &str) -> Option<FontArc> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<FontArc>>>> = OnceLock::new();
    if path.is_empty() {
        return None;
    }
    let mut cache = CACHE.get_or_init(Default::default).lock().ok()?;
    cache
        .entry(path.to_owned())
        .or_insert_with(|| {
            std::fs::read(path).ok().and_then(|bytes| ab_glyph::FontVec::try_from_vec_and_index(bytes, 0).ok()).map(FontArc::new)
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_are_readable_and_bad_files_fall_back() {
        assert_eq!(display_name(Path::new("/f/LiberationSans-BoldItalic.ttf")), "Liberation Sans Bold Italic");
        assert_eq!(display_name(Path::new("/f/comfortaa_light.otf")), "Comfortaa light");
        assert!(load("/no/such/font.ttf").is_none());
        assert!(load("").is_none());
        assert!(find("").is_none());
        let roboto = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts/Roboto.ttf");
        assert!(load(roboto).is_some());
    }
}
