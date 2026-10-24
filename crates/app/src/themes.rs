//! Custom themes: TOML files in the themes folder that anyone can write, edit and share.
//! A theme starts from Dark or Light and changes only what it lists, so a three-line file is a
//! valid theme. The chosen theme's file is watched, and saving it updates Rosorah right away.
//! The format is documented in docs/themes.md.
use crate::appearance::Palette;
use auroracast_core::workspace::{Appearance, Backdrop, BackdropFit, PanelStyle, ThemeMode};
use eframe::egui::Color32;
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Where theme files live: ~/.config/rosorah-studio/themes, %APPDATA%\rosorah-studio\themes.
pub fn folder() -> PathBuf {
    auroracast_core::platform::config_root().join("rosorah-studio/themes")
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct File {
    name: Option<String>,
    author: Option<String>,
    base: Option<String>,
    #[serde(default)]
    colors: Colors,
    #[serde(default)]
    shape: Shape,
    background: Option<Background>,
}
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Colors {
    background: Option<String>,
    panel: Option<String>,
    card: Option<String>,
    button: Option<String>,
    button_hover: Option<String>,
    text: Option<String>,
    muted_text: Option<String>,
    accent: Option<String>,
    on_accent: Option<String>,
    good: Option<String>,
    warning: Option<String>,
    error: Option<String>,
    live: Option<String>,
}
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Shape {
    panel_corners: Option<u8>,
    button_corners: Option<u8>,
}
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Background {
    color: Option<String>,
    picture: Option<String>,
    fit: Option<String>,
    panels: Option<String>,
    transparency: Option<u8>,
    clarity: Option<u8>,
}

/// A theme read from its file, checked and ready to apply.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub name: String,
    pub author: String,
    base: ThemeMode,
    palette: Palette,
    accent: Option<Color32>,
    on_accent: Option<Color32>,
    backdrop: Option<Look>,
}
/// What a theme puts behind the panels; copied into the user's settings when they choose it.
#[derive(Clone, Debug, PartialEq)]
struct Look {
    backdrop: Backdrop,
    color: [u8; 3],
    picture: String,
    fit: Option<BackdropFit>,
    panels: Option<PanelStyle>,
    transparency: Option<f32>,
    clarity: Option<f32>,
}

/// "#rgb", "#rrggbb" or "#rrggbbaa".
fn color(key: &str, value: &Option<String>) -> Result<Option<Color32>, String> {
    let Some(value) = value else { return Ok(None) };
    let hex = value.trim().trim_start_matches('#');
    let digits: Option<Vec<u8>> = match hex.len() {
        3 => hex.chars().map(|c| c.to_digit(16).map(|d| d as u8 * 17)).collect(),
        6 | 8 => (0..hex.len()).step_by(2).map(|i| hex.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok())).collect(),
        _ => None,
    };
    match digits.as_deref() {
        Some([r, g, b]) => Ok(Some(Color32::from_rgb(*r, *g, *b))),
        Some([r, g, b, a]) => Ok(Some(Color32::from_rgba_unmultiplied(*r, *g, *b, *a))),
        _ => Err(format!("{key} = \"{value}\" isn't a colour; use \"#rrggbb\"")),
    }
}
fn choice<T: Copy>(key: &str, value: &Option<String>, options: &[(&str, T)]) -> Result<Option<T>, String> {
    let Some(value) = value else { return Ok(None) };
    let wanted = value.trim().to_ascii_lowercase();
    options.iter().find(|(name, _)| *name == wanted).map(|(_, v)| Some(*v)).ok_or_else(|| {
        let names: Vec<_> = options.iter().map(|(n, _)| format!("\"{n}\"")).collect();
        format!("{key} = \"{value}\": use one of {}", names.join(", "))
    })
}

impl Theme {
    /// Reads a theme file. Pictures are looked for next to it.
    pub fn read(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("Can't read {}: {e}", path.display()))?;
        Self::parse(&text, path.parent().unwrap_or(Path::new(".")), &stem(path))
    }
    fn parse(text: &str, dir: &Path, fallback_name: &str) -> Result<Self, String> {
        let file: File = toml::from_str(text).map_err(|e| e.to_string())?;
        let base = choice("base", &file.base, &[("dark", ThemeMode::Dark), ("light", ThemeMode::Light)])?.unwrap_or_default();
        let c = &file.colors;
        let mut palette = Palette::for_theme(base);
        for (key, value, slot) in [
            ("background", &c.background, &mut palette.bg),
            ("panel", &c.panel, &mut palette.panel),
            ("card", &c.card, &mut palette.card),
            ("button", &c.button, &mut palette.widget),
            ("button_hover", &c.button_hover, &mut palette.widget_hover),
            ("text", &c.text, &mut palette.text),
            ("muted_text", &c.muted_text, &mut palette.muted),
            ("good", &c.good, &mut palette.good),
            ("warning", &c.warning, &mut palette.warning),
            ("error", &c.error, &mut palette.error),
            ("live", &c.live, &mut palette.live),
        ] {
            if let Some(v) = color(key, value)? {
                *slot = v;
            }
        }
        palette.panel_corners = file.shape.panel_corners.unwrap_or(palette.panel_corners).min(40);
        palette.button_corners = file.shape.button_corners.unwrap_or(palette.button_corners).min(40);
        let backdrop = match &file.background {
            None => None,
            Some(b) => {
                let fit = choice("fit", &b.fit, &[("fill", BackdropFit::Fill), ("fit", BackdropFit::Fit), ("stretch", BackdropFit::Stretch)])?;
                let panels = choice(
                    "panels",
                    &b.panels,
                    &[("solid", PanelStyle::Solid), ("transparent", PanelStyle::Transparent), ("frosted", PanelStyle::Frosted), ("matte", PanelStyle::Matte), ("liquid", PanelStyle::Liquid)],
                )?;
                let picture = match b.picture.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
                    // Only files beside the theme: a shared theme can't point anywhere else.
                    Some(p) if Path::new(p).components().any(|c| !matches!(c, std::path::Component::Normal(_))) => {
                        return Err(format!("picture = \"{p}\": put the picture next to the theme file and give just its name"));
                    }
                    Some(p) => dir.join(p).to_string_lossy().into_owned(),
                    None => String::new(),
                };
                let color = color("color", &b.color)?;
                Some(Look {
                    backdrop: if !picture.is_empty() { Backdrop::Picture } else if color.is_some() { Backdrop::Color } else { Backdrop::None },
                    color: color.map_or([40, 32, 72], |c| [c.r(), c.g(), c.b()]),
                    picture,
                    fit,
                    panels,
                    transparency: b.transparency.map(|t| t.min(95) as f32 / 100.),
                    clarity: b.clarity.map(|t| t.min(100) as f32 / 100.),
                })
            }
        };
        Ok(Self {
            name: file.name.map(|n| n.trim().chars().take(60).collect()).filter(|n: &String| !n.is_empty()).unwrap_or_else(|| fallback_name.to_owned()),
            author: file.author.map(|a| a.trim().chars().take(60).collect()).unwrap_or_default(),
            base,
            palette,
            accent: color("accent", &c.accent)?,
            on_accent: color("on_accent", &c.on_accent)?,
            backdrop,
        })
    }
    /// The theme's colours, in place of the built-in Dark or Light ones.
    pub fn palette(&self) -> Palette {
        Palette { accent: self.accent, on_accent: self.on_accent, ..self.palette }
    }
    pub fn base(&self) -> ThemeMode {
        self.base
    }
    pub fn sets_accent(&self) -> bool {
        self.accent.is_some()
    }
    /// Choosing the theme also takes its background and panel style, if it has them.
    pub fn apply_backdrop(&self, a: &mut Appearance) {
        a.theme = self.base;
        let Some(look) = &self.backdrop else { return };
        a.backdrop = look.backdrop;
        a.backdrop_color = look.color;
        if !look.picture.is_empty() {
            a.backdrop_image = look.picture.clone();
        }
        if let Some(fit) = look.fit {
            a.backdrop_fit = fit;
        }
        if let Some(panels) = look.panels {
            a.panels = panels;
        }
        if let Some(t) = look.transparency {
            a.transparency = t;
        }
        if let Some(c) = look.clarity {
            a.clarity = c;
        }
    }
}
fn stem(path: &Path) -> String {
    // A theme in its own folder is named after the folder.
    let file = if path.file_name().is_some_and(|n| n == "theme.toml") { path.parent().unwrap_or(path) } else { path };
    file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Every theme in the folder: "name.toml" files, and "name/theme.toml" for themes that come with
/// pictures. Each is (id, path), the id being what Appearance.custom_theme stores.
pub fn list() -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(folder()) else { return found };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() && path.join("theme.toml").is_file() {
            found.push((format!("{name}/theme.toml"), path.join("theme.toml")));
        } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("toml")) {
            found.push((name, path));
        }
    }
    found.sort_by_key(|(id, _)| id.to_lowercase());
    found
}
/// The file for a stored theme id, refusing anything outside the themes folder.
pub fn path(id: &str) -> Option<PathBuf> {
    let relative = Path::new(id);
    relative.components().all(|c| matches!(c, std::path::Component::Normal(_))).then(|| folder().join(relative))
}

fn hex(c: Color32) -> String {
    if c.a() == 255 {
        format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
    } else {
        let [r, g, b, a] = c.to_srgba_unmultiplied();
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
}
/// A complete, commented theme file for the given look: the starting point for a new theme.
pub fn template(name: &str, base: ThemeMode, p: &Palette, accent: Color32, on_accent: Color32) -> String {
    format!(
        r##"# Rosorah Studio theme. Save this file and Rosorah updates straight away.
# Anything you delete falls back to the "base" theme, so a theme can be just a few lines.
# Full guide: https://github.com/iBustcherries/Rosorah-Studio/blob/master/docs/themes.md

name = "{name}"
author = ""
base = "{base}"            # "dark" or "light"

[colors]                   # "#rrggbb", or "#rrggbbaa" with see-through
background = "{bg}"     # behind the panels
panel = "{panel}"          # panels and windows
card = "{card}"           # chat messages and other blocks inside panels
button = "{widget}"
button_hover = "{hover}"
text = "{text}"
muted_text = "{muted}"     # hints and secondary text
accent = "{accent}"         # selected buttons and highlights (overrides Settings' accent colour)
on_accent = "{on_accent}"      # text on accent-coloured buttons
good = "{good}"           # connected, ready
warning = "{warning}"
error = "{error}"
live = "{live}"           # LIVE, REC and the program outline

[shape]
panel_corners = {panel_corners}         # 0 = square
button_corners = {button_corners}

# Optional: what the theme puts behind the panels when someone chooses it.
# [background]
# picture = "background.jpg"   # next to this file (keep the theme in its own folder: my-theme/theme.toml)
# color = "#282048"            # used when there's no picture
# fit = "fill"                 # fill, fit or stretch
# panels = "frosted"           # solid, transparent, frosted, matte or liquid
# transparency = 40            # 0-95
# clarity = 50                 # 0-100, how sharp the picture looks through glass
"##,
        base = if base == ThemeMode::Light { "light" } else { "dark" },
        bg = hex(p.bg),
        panel = hex(p.panel),
        card = hex(p.card),
        widget = hex(p.widget),
        hover = hex(p.widget_hover),
        text = hex(p.text),
        muted = hex(p.muted),
        accent = hex(accent),
        on_accent = hex(on_accent),
        good = hex(p.good),
        warning = hex(p.warning),
        error = hex(p.error),
        live = hex(p.live),
        panel_corners = p.panel_corners,
        button_corners = p.button_corners,
    )
}
/// A file name in the themes folder that isn't taken yet: "my-theme.toml", "my-theme-2.toml"…
pub fn free_name(base: &str) -> String {
    let slug: String = base.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let slug = slug.trim_matches('-');
    let slug = if slug.is_empty() { "theme" } else { slug };
    (1..)
        .map(|n| if n == 1 { format!("{slug}.toml") } else { format!("{slug}-{n}.toml") })
        .find(|name| !folder().join(name).exists() && !folder().join(name.trim_end_matches(".toml")).exists())
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_short_theme_changes_only_what_it_lists() {
        let t = Theme::parse("base = \"light\"\n[colors]\naccent = \"#ff8800\"\npanel = \"#123\"\n", Path::new("/themes"), "orange").unwrap();
        let light = Palette::for_theme(ThemeMode::Light);
        let p = t.palette();
        assert_eq!(t.name, "orange");
        assert_eq!(p.panel, Color32::from_rgb(0x11, 0x22, 0x33));
        assert_eq!(p.text, light.text);
        assert_eq!(p.accent, Some(Color32::from_rgb(255, 136, 0)));
        assert!(p.light);
    }
    #[test]
    fn mistakes_are_explained() {
        let typo = Theme::parse("[colors]\nbackgroud = \"#000000\"\n", Path::new("."), "x").unwrap_err();
        assert!(typo.contains("backgroud"), "{typo}");
        let bad = Theme::parse("[colors]\ntext = \"blue\"\n", Path::new("."), "x").unwrap_err();
        assert!(bad.contains("text") && bad.contains("#rrggbb"), "{bad}");
        let escape = Theme::parse("[background]\npicture = \"../../secret.png\"\n", Path::new("."), "x").unwrap_err();
        assert!(escape.contains("next to the theme"), "{escape}");
        assert!(path("../outside.toml").is_none() && path("mine/theme.toml").is_some());
    }
    #[test]
    fn the_template_reads_back_as_the_same_look() {
        let p = Palette::for_theme(ThemeMode::Dark);
        let text = template("Mine", ThemeMode::Dark, &p, Color32::from_rgb(1, 2, 3), Color32::BLACK);
        let t = Theme::parse(&text, Path::new("."), "x").unwrap();
        assert_eq!(t.name, "Mine");
        assert_eq!(Palette { accent: None, on_accent: None, ..t.palette() }, p);
        assert_eq!(t.palette().accent, Some(Color32::from_rgb(1, 2, 3)));
    }
    #[test]
    fn background_is_applied_on_choosing() {
        let t = Theme::parse("[background]\npicture = \"bg.png\"\npanels = \"frosted\"\ntransparency = 30\n", Path::new("/t/mine"), "mine").unwrap();
        let mut a = Appearance::default();
        t.apply_backdrop(&mut a);
        assert_eq!((a.backdrop, a.panels), (Backdrop::Picture, PanelStyle::Frosted));
        assert!(a.backdrop_image.ends_with("bg.png") && (a.transparency - 0.3).abs() < 1e-6);
    }
}
