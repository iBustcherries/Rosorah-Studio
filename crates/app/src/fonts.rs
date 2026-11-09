//! Fonts bundled with Rosorah (the `fonts` folder beside the program) and installed on
//! this computer, for widget text. .ttf/.otf files and the first face of .ttc/.otc
//! collections are listed; names come from the font's own name table, or the file name.
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
        let mut list: Vec<_> = found.into_iter().map(|p| (label(&p, face_name(&p)), p.to_string_lossy().into_owned())).collect();
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

/// The name a font file is listed under: "Liberation Sans Bold Italic". Windows shortens its
/// file names (calibrib.ttf, arialbd.ttf), so listed fonts use the names they carry inside.
pub fn display_name(path: &Path) -> String {
    // Windows paths ignore case (C:\WINDOWS\Fonts is C:\Windows\Fonts).
    let same = |p: &str| Path::new(p) == path || cfg!(windows) && p.replace('/', "\\").eq_ignore_ascii_case(&path.to_string_lossy().replace('/', "\\"));
    match installed().iter().find(|(_, p)| Path::new(p) == path).or_else(|| installed().iter().find(|(_, p)| same(p))) {
        Some((name, _)) => name.clone(),
        None => label(path, None),
    }
}
/// "LiberationSans-BoldItalic.ttf" -> "Liberation Sans Bold Italic", unless the font named itself.
fn label(path: &Path, inner: Option<String>) -> String {
    let stem = inner.unwrap_or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
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
    // Free look-alikes of Microsoft fonts Rosorah can't ship, findable by the familiar name.
    for (prefix, like) in LOOK_ALIKES {
        if name.starts_with(prefix) {
            name.push_str(&format!(" (like {like})"));
        }
    }
    name
}
/// The family and style a font file gives itself ("Calibri" + "Bold" -> "Calibri Bold"), read from
/// its name table without loading the whole file. For collections, the first face.
fn face_name(path: &Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let mut read = |at: u64, len: usize| -> Option<Vec<u8>> {
        file.seek(SeekFrom::Start(at)).ok()?;
        let mut bytes = vec![0; len];
        file.read_exact(&mut bytes).ok()?;
        Some(bytes)
    };
    let u16_at = |b: &[u8], i: usize| b.get(i..i + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
    let u32_at = |b: &[u8], i: usize| b.get(i..i + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    let mut start = 0;
    if read(0, 4)? == b"ttcf" {
        start = u32_at(&read(12, 4)?, 0)? as u64;
    }
    let tables = u16_at(&read(start + 4, 2)?, 0)? as usize;
    let directory = read(start + 12, tables.min(256) * 16)?;
    let record = directory.chunks_exact(16).find(|r| &r[..4] == b"name")?;
    let table = read(u32_at(record, 8)? as u64, (u32_at(record, 12)? as usize).min(1 << 20))?;
    let (count, strings) = (u16_at(&table, 2)? as usize, u16_at(&table, 4)? as usize);
    // English Windows names (UTF-16) first, then Mac Roman ones (read as ASCII).
    let find = |id: u16| -> Option<String> {
        let mut best: Option<(u8, String)> = None;
        for i in 0..count.min(1024) {
            let r = 6 + i * 12;
            let (platform, language, name) = (u16_at(&table, r)?, u16_at(&table, r + 4)?, u16_at(&table, r + 6)?);
            let (len, offset) = (u16_at(&table, r + 8)? as usize, u16_at(&table, r + 10)? as usize);
            let rank = match (platform, language) {
                (3, 0x409) => 0,
                (3, _) => 1,
                (1, 0) => 2,
                _ => continue,
            };
            if name != id || best.as_ref().is_some_and(|(b, _)| *b <= rank) {
                continue;
            }
            let Some(raw) = table.get(strings + offset..strings + offset + len) else { continue };
            let text = if platform == 3 {
                String::from_utf16_lossy(&raw.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect::<Vec<_>>())
            } else {
                raw.iter().filter(|b| b.is_ascii()).map(|&b| b as char).collect()
            };
            let text = text.trim().to_owned();
            if !text.is_empty() {
                best = Some((rank, text));
            }
        }
        best.map(|(_, text)| text)
    };
    // The typographic family and style (16, 17) keep weights such as Semibold apart from the family.
    let family = find(16).or_else(|| find(1))?;
    let style = find(17).or_else(|| find(2)).unwrap_or_default();
    Some(if ["", "regular", "normal", "book", "roman"].contains(&style.to_lowercase().as_str()) || family.to_lowercase().ends_with(&style.to_lowercase()) {
        family
    } else {
        format!("{family} {style}")
    })
}
/// Bundled open fonts standing in for Microsoft ones: (family, the font it matches).
const LOOK_ALIKES: [(&str, &str); 2] = [("Comic Neue", "Comic Sans"), ("Carlito", "Calibri")];

/// The font file best matching a family name such as "Arial bold italic", or `None`.
/// Comic Sans and Calibri fall back to their bundled look-alikes when not installed.
pub fn find(face: &str) -> Option<String> {
    find_exact(face).or_else(|| {
        let lower = face.to_lowercase();
        let (alike, _) = LOOK_ALIKES.iter().find(|(_, like)| lower.starts_with(&like.to_lowercase()))?;
        // Keep only the style ("Comic Sans MS Bold" → "Comic Neue bold").
        let style: String = ["bold", "italic"].iter().filter(|w| lower.contains(*w)).map(|w| format!(" {w}")).collect();
        find_exact(&format!("{alike}{style}"))
    })
}
fn find_exact(face: &str) -> Option<String> {
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
    #[test]
    fn comic_sans_and_calibri_have_bundled_look_alikes() {
        assert_eq!(display_name(Path::new("/f/ComicNeue-Bold.ttf")), "Comic Neue Bold (like Comic Sans)");
        assert_eq!(display_name(Path::new("/f/Carlito-Italic.ttf")), "Carlito Italic (like Calibri)");
        // Debug builds also scan the repository's fonts folder, so the look-alikes are always found.
        assert!(find("Comic Sans MS").is_some());
        assert!(find("Calibri bold").is_some_and(|p| display_name(Path::new(&p)).contains("Bold")));
    }
    #[test]
    fn fonts_are_listed_by_their_own_names() {
        let carlito = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fonts/carlito/Carlito-BoldItalic.ttf");
        assert_eq!(face_name(Path::new(carlito)).as_deref(), Some("Carlito Bold Italic"));
        assert_eq!(face_name(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts/Roboto.ttf"))).as_deref(), Some("Roboto"));
        assert_eq!(face_name(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))), None);
        #[cfg(windows)]
        if Path::new("C:/Windows/Fonts/arialbd.ttf").is_file() {
            assert_eq!(display_name(Path::new("C:/Windows/Fonts/arialbd.ttf")), "Arial Bold");
        }
    }
}
