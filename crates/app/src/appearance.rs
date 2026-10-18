//! Studio look: Dark/Light palettes, a color or picture behind the panels, and glass panels.
//! The picture's blurred copy is made once in the background; each panel then draws its
//! matching region of that copy, so glass costs nothing extra per frame. egui cannot blur
//! live content, so glass shows the background picture only, not other panels.
use auroracast_core::workspace::{Appearance, Backdrop, BackdropFit, PanelStyle, ThemeMode};
use eframe::egui::{self, Color32, Rect, TextureHandle, pos2};
use std::{cell::Cell, sync::Arc, sync::mpsc};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// Page background (transparent when a backdrop shows through).
    pub bg: Color32,
    /// Panels and cards (translucent for see-through styles).
    pub panel: Color32,
    pub widget: Color32,
    pub widget_hover: Color32,
    pub text: Color32,
    pub muted: Color32,
    /// Chat message bubbles and other inset blocks.
    pub card: Color32,
    pub light: bool,
}
impl Palette {
    pub fn for_theme(theme: ThemeMode) -> Self {
        let rgb = Color32::from_rgb;
        match theme {
            ThemeMode::Dark => Self {
                bg: rgb(19, 19, 24),
                panel: rgb(32, 32, 39),
                widget: rgb(44, 43, 53),
                widget_hover: rgb(61, 57, 73),
                text: rgb(238, 235, 245),
                muted: rgb(188, 182, 201),
                card: rgb(28, 34, 50),
                light: false,
            },
            ThemeMode::Light => Self {
                bg: rgb(232, 231, 238),
                panel: rgb(250, 249, 252),
                widget: rgb(226, 223, 235),
                widget_hover: rgb(210, 205, 226),
                text: rgb(30, 27, 40),
                muted: rgb(92, 86, 110),
                card: rgb(240, 238, 246),
                light: true,
            },
        }
    }
}
thread_local! {
    static CURRENT: Cell<Palette> = Cell::new(Palette::for_theme(ThemeMode::Dark));
}
/// The palette in effect this frame (UI thread).
pub fn current() -> Palette {
    CURRENT.with(|c| c.get())
}

fn with_alpha(c: Color32, alpha: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (alpha.clamp(0., 1.) * 255.) as u8)
}
fn see_through(a: &Appearance) -> bool {
    a.backdrop != Backdrop::None && a.panels != PanelStyle::Solid
}
/// Tint drawn over a panel's glass (or alone, for plain transparency).
fn tint(a: &Appearance, palette: &Palette) -> Color32 {
    let base = (1. - a.transparency).clamp(0.05, 1.);
    let boost = if a.contrast_boost { 0.18 } else { 0. };
    let alpha = match a.panels {
        PanelStyle::Solid => 1.,
        PanelStyle::Transparent => base + boost,
        PanelStyle::Frosted => base * 0.75 + boost,
        PanelStyle::Matte => 0.35 + base * 0.5 + boost,
        PanelStyle::Liquid => base * 0.55 + boost,
    };
    // Frosted glass reads slightly brighter than the panel color.
    let color = if a.panels == PanelStyle::Frosted && !palette.light {
        Color32::from_rgb(palette.panel.r() + 14, palette.panel.g() + 14, palette.panel.b() + 18)
    } else {
        palette.panel
    };
    with_alpha(color, alpha)
}
/// The palette with see-through styles applied.
pub fn effective(a: &Appearance) -> Palette {
    let mut p = Palette::for_theme(a.theme);
    if see_through(a) {
        let t = tint(a, &p);
        p.bg = Color32::TRANSPARENT;
        p.card = with_alpha(p.card, (t.a() as f32 / 255. + 0.1).min(1.));
        p.panel = t;
    }
    p
}
/// Builds the egui style for `a`, keeping fonts and spacing from `base`.
pub fn style(base: &egui::Style, a: &Appearance) -> egui::Style {
    let p = effective(a);
    let solid = Palette::for_theme(a.theme);
    let mut s = base.clone();
    let mut v = if p.light { egui::Visuals::light() } else { egui::Visuals::dark() };
    v.panel_fill = if see_through(a) { p.panel } else { p.bg };
    // Floating windows stay nearly opaque so dialogs are always readable.
    v.window_fill = with_alpha(solid.panel, 0.96);
    v.extreme_bg_color = if p.light { Color32::WHITE } else { solid.bg };
    v.faint_bg_color = with_alpha(solid.card, if see_through(a) { 0.55 } else { 1. });
    v.override_text_color = None;
    v.window_corner_radius = egui::CornerRadius::same(16);
    v.window_stroke = egui::Stroke::NONE;
    for widget in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active] {
        widget.bg_fill = p.widget;
        widget.weak_bg_fill = p.widget;
        widget.bg_stroke = egui::Stroke::NONE;
        widget.fg_stroke = egui::Stroke::new(1.0_f32, p.text);
        widget.corner_radius = egui::CornerRadius::same(20);
    }
    v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0_f32, p.text);
    v.widgets.hovered.bg_fill = p.widget_hover;
    v.widgets.hovered.weak_bg_fill = p.widget_hover;
    s.visuals = v;
    s
}
/// Where the picture sits on screen for each fit.
pub fn place(screen: Rect, size: [usize; 2], fit: BackdropFit) -> Rect {
    let (w, h) = (size[0].max(1) as f32, size[1].max(1) as f32);
    let scale = match fit {
        BackdropFit::Stretch => return screen,
        BackdropFit::Fill => (screen.width() / w).max(screen.height() / h),
        BackdropFit::Fit => (screen.width() / w).min(screen.height() / h),
    };
    Rect::from_center_size(screen.center(), egui::vec2(w * scale, h * scale))
}

struct Glass {
    mode: PanelStyle,
    blurred: Option<(egui::TextureId, Rect)>,
    tint: Color32,
    grain: Option<egui::TextureId>,
    light: bool,
}
fn glass_id() -> egui::Id {
    egui::Id::new("rosorah-glass")
}
/// Draws this panel's glass behind its content. Call first thing inside a panel.
pub fn glass(ui: &egui::Ui, radius: egui::CornerRadius) {
    let Some(g) = ui.ctx().data(|d| d.get_temp::<Arc<Glass>>(glass_id())) else { return };
    if !matches!(g.mode, PanelStyle::Frosted | PanelStyle::Matte | PanelStyle::Liquid) {
        return;
    }
    let rect = ui.clip_rect();
    let painter = ui.painter();
    if let Some((texture, placed)) = g.blurred {
        let uv = Rect::from_min_max(
            pos2((rect.min.x - placed.min.x) / placed.width(), (rect.min.y - placed.min.y) / placed.height()),
            pos2((rect.max.x - placed.min.x) / placed.width(), (rect.max.y - placed.min.y) / placed.height()),
        );
        painter.add(egui::epaint::RectShape::filled(rect, radius, Color32::WHITE).with_texture(texture, uv));
    }
    painter.rect_filled(rect, radius, g.tint);
    match g.mode {
        PanelStyle::Matte => {
            if let Some(grain) = g.grain {
                let uv = Rect::from_min_max(pos2(0., 0.), pos2(rect.width() / GRAIN as f32, rect.height() / GRAIN as f32));
                painter.add(egui::epaint::RectShape::filled(rect, radius, Color32::from_white_alpha(if g.light { 40 } else { 22 })).with_texture(grain, uv));
            }
        }
        PanelStyle::Liquid => {
            // A soft sheen across the top and a bright rim, like light catching thick glass.
            let sheen = Rect::from_min_max(rect.min, pos2(rect.max.x, rect.min.y + rect.height().min(260.) * 0.45));
            let mut mesh = egui::Mesh::default();
            let top = Color32::from_white_alpha(34);
            mesh.colored_vertex(sheen.left_top(), top);
            mesh.colored_vertex(sheen.right_top(), top);
            mesh.colored_vertex(sheen.right_bottom(), Color32::TRANSPARENT);
            mesh.colored_vertex(sheen.left_bottom(), Color32::TRANSPARENT);
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            painter.add(mesh);
            painter.rect_stroke(rect.shrink(0.5), radius, egui::Stroke::new(1.3_f32, Color32::from_white_alpha(120)), egui::StrokeKind::Inside);
        }
        _ => {
            painter.rect_stroke(rect.shrink(0.5), radius, egui::Stroke::new(1.0_f32, Color32::from_white_alpha(50)), egui::StrokeKind::Inside);
        }
    }
}

const GRAIN: usize = 128;
type Loaded = (egui::ColorImage, Arc<image::RgbaImage>);

/// Owns the backdrop textures and keeps the style in sync with the settings.
#[derive(Default)]
pub struct Renderer {
    applied: Option<Appearance>,
    picture: Option<(String, TextureHandle, [usize; 2], Arc<image::RgbaImage>)>,
    loading: Option<(String, mpsc::Receiver<Result<Loaded, String>>)>,
    blur: Option<((String, PanelStyle, u8), TextureHandle)>,
    blurring: Option<((String, PanelStyle, u8), mpsc::Receiver<egui::ColorImage>)>,
    grain: Option<TextureHandle>,
    pub error: String,
}
impl Renderer {
    /// True while the picture or its glass copy is still being prepared.
    pub fn busy(&self) -> bool {
        self.loading.is_some() || self.blurring.is_some()
    }
    /// Call once per frame before any panel is shown.
    pub fn frame(&mut self, ctx: &egui::Context, a: &Appearance) {
        let palette = effective(a);
        CURRENT.with(|c| c.set(palette));
        if self.applied.as_ref() != Some(a) {
            ctx.set_style(style(&ctx.style(), a));
            self.applied = Some(a.clone());
        }
        self.load_picture(ctx, a);
        self.make_glass(ctx, a);
        let screen = ctx.content_rect();
        let painter = ctx.layer_painter(egui::LayerId::background());
        let solid = Palette::for_theme(a.theme);
        let base = match a.backdrop {
            Backdrop::Color => Color32::from_rgb(a.backdrop_color[0], a.backdrop_color[1], a.backdrop_color[2]),
            _ => solid.bg,
        };
        painter.rect_filled(screen, 0., base);
        let mut placed = None;
        if a.backdrop == Backdrop::Picture
            && let Some((_, texture, size, _)) = &self.picture
        {
            let rect = place(screen, *size, a.backdrop_fit);
            painter.image(texture.id(), rect, Rect::from_min_max(pos2(0., 0.), pos2(1., 1.)), Color32::WHITE);
            placed = Some(rect);
        }
        let blurred = match (&self.blur, placed) {
            (Some((key, texture)), Some(rect)) if key.1 == a.panels => Some((texture.id(), rect)),
            _ => None,
        };
        let glass = Glass {
            mode: if see_through(a) { a.panels } else { PanelStyle::Solid },
            blurred,
            tint: palette.panel,
            grain: self.grain.as_ref().map(|t| t.id()),
            light: palette.light,
        };
        ctx.data_mut(|d| d.insert_temp(glass_id(), Arc::new(glass)));
        if self.loading.is_some() || self.blurring.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
    fn load_picture(&mut self, ctx: &egui::Context, a: &Appearance) {
        let want = (a.backdrop == Backdrop::Picture && !a.backdrop_image.trim().is_empty()).then(|| a.backdrop_image.trim().to_owned());
        let Some(path) = want else {
            self.picture = None;
            self.blur = None;
            return;
        };
        if let Some((loading, rx)) = &self.loading
            && let Ok(result) = rx.try_recv()
        {
            let loading = loading.clone();
            self.loading = None;
            match result {
                Ok((image, small)) => {
                    let size = image.size;
                    let texture = ctx.load_texture("rosorah-backdrop", image, egui::TextureOptions::LINEAR);
                    self.picture = Some((loading, texture, size, small));
                    self.blur = None;
                    self.error.clear();
                }
                Err(e) => self.error = e,
            }
        }
        let current = self.picture.as_ref().is_some_and(|p| p.0 == path);
        let pending = self.loading.as_ref().is_some_and(|l| l.0 == path);
        if !current && !pending && !self.error_for(&path) {
            let (tx, rx) = mpsc::channel();
            let file = path.clone();
            std::thread::spawn(move || {
                let _ = tx.send(load(&file));
            });
            self.loading = Some((path, rx));
        }
    }
    fn error_for(&self, path: &str) -> bool {
        !self.error.is_empty() && self.error.contains(path)
    }
    fn make_glass(&mut self, ctx: &egui::Context, a: &Appearance) {
        if a.panels == PanelStyle::Matte && self.grain.is_none() {
            self.grain = Some(ctx.load_texture(
                "rosorah-grain",
                grain(),
                egui::TextureOptions { wrap_mode: egui::TextureWrapMode::Repeat, ..egui::TextureOptions::LINEAR },
            ));
        }
        let Some((path, _, _, small)) = &self.picture else { return };
        if !matches!(a.panels, PanelStyle::Frosted | PanelStyle::Matte | PanelStyle::Liquid) {
            return;
        }
        let key = (path.clone(), a.panels, (a.clarity.clamp(0., 1.) * 20.).round() as u8);
        if let Some((pending, rx)) = &self.blurring
            && let Ok(image) = rx.try_recv()
        {
            let pending = pending.clone();
            self.blurring = None;
            self.blur = Some((pending, ctx.load_texture("rosorah-backdrop-glass", image, egui::TextureOptions::LINEAR)));
        }
        let done = self.blur.as_ref().is_some_and(|b| b.0 == key);
        let busy = self.blurring.as_ref().is_some_and(|b| b.0 == key);
        if !done && !busy && self.blurring.is_none() {
            let (tx, rx) = mpsc::channel();
            let (small, mode, clarity) = (small.clone(), a.panels, a.clarity);
            std::thread::spawn(move || {
                let _ = tx.send(blurred(&small, mode, clarity));
            });
            self.blurring = Some((key, rx));
        }
    }
}

/// Decodes the picture: a display copy (up to 2560 px) and a small copy for blurring.
fn load(path: &str) -> Result<Loaded, String> {
    let image = image::open(path).map_err(|e| format!("Could not open {path}: {e}"))?.to_rgba8();
    let fit = |img: &image::RgbaImage, max: u32| {
        let scale = (max as f32 / img.width().max(img.height()) as f32).min(1.);
        image::imageops::resize(img, ((img.width() as f32 * scale) as u32).max(1), ((img.height() as f32 * scale) as u32).max(1), image::imageops::FilterType::Triangle)
    };
    let display = fit(&image, 2560);
    let small = fit(&display, 640);
    let color = egui::ColorImage::from_rgba_unmultiplied([display.width() as usize, display.height() as usize], display.as_raw());
    Ok((color, Arc::new(small)))
}
/// The glass copy: more blur for less clarity; matte is also softened and desaturated.
pub fn blurred(small: &image::RgbaImage, mode: PanelStyle, clarity: f32) -> egui::ColorImage {
    let haze = 1. - clarity.clamp(0., 1.);
    let sigma = match mode {
        PanelStyle::Liquid => 0.6 + haze * 5.,
        PanelStyle::Matte => 6. + haze * 14.,
        _ => 2. + haze * 10.,
    };
    let mut out = image::imageops::blur(small, sigma);
    if mode == PanelStyle::Matte {
        for p in out.pixels_mut() {
            let gray = 0.3 * p[0] as f32 + 0.59 * p[1] as f32 + 0.11 * p[2] as f32;
            for c in 0..3 {
                p[c] = (p[c] as f32 * 0.6 + gray * 0.4) as u8;
            }
        }
    }
    egui::ColorImage::from_rgba_unmultiplied([out.width() as usize, out.height() as usize], out.as_raw())
}
/// Fine monochrome noise for matte glass, tiled across panels.
fn grain() -> egui::ColorImage {
    let mut pixels = Vec::with_capacity(GRAIN * GRAIN * 4);
    let mut seed: u32 = 0x9e37_79b9;
    for _ in 0..GRAIN * GRAIN {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let v = (seed >> 24) as u8;
        pixels.extend_from_slice(&[v, v, v, (seed >> 16) as u8 / 2]);
    }
    egui::ColorImage::from_rgba_unmultiplied([GRAIN, GRAIN], &pixels)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn solid_and_light_palettes_and_see_through_tints() {
        let mut a = Appearance::default();
        assert_eq!(effective(&a), Palette::for_theme(ThemeMode::Dark));
        a.theme = ThemeMode::Light;
        let light = effective(&a);
        assert!(light.light && light.text.r() < 60 && light.bg.r() > 200);
        // Transparency needs something behind the panels.
        a.panels = PanelStyle::Frosted;
        assert_eq!(effective(&a).panel.a(), 255);
        a.backdrop = Backdrop::Color;
        a.transparency = 0.6;
        let glass = effective(&a);
        assert!(glass.panel.a() < 200 && glass.bg == Color32::TRANSPARENT);
        a.contrast_boost = true;
        assert!(effective(&a).panel.a() > glass.panel.a());
    }
    #[test]
    fn picture_fits_and_glass_blurs_by_clarity() {
        let screen = Rect::from_min_size(pos2(0., 0.), egui::vec2(1600., 900.));
        let fill = place(screen, [1000, 1000], BackdropFit::Fill);
        assert!(fill.width() >= 1600. && fill.height() >= 900.);
        let fit = place(screen, [1000, 1000], BackdropFit::Fit);
        assert_eq!(fit.height(), 900.);
        assert_eq!(place(screen, [10, 10], BackdropFit::Stretch), screen);
        let mut img = image::RgbaImage::new(64, 64);
        for (x, _, p) in img.enumerate_pixels_mut() {
            *p = if x < 32 { image::Rgba([0, 0, 0, 255]) } else { image::Rgba([255, 255, 255, 255]) };
        }
        let edge = |c: &egui::ColorImage| c.pixels[32 * 64 + 31].r();
        let sharp = edge(&blurred(&img, PanelStyle::Liquid, 1.));
        let hazy = edge(&blurred(&img, PanelStyle::Frosted, 0.));
        assert!(hazy > sharp, "less clarity should blur more ({sharp} vs {hazy})");
    }
}
