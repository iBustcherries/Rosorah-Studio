// Widget design system: theme looks, typography and the drawing every widget shares.
// Each theme keeps its own character (window frames, scanlines, dot matrix, pixel borders,
// broadcast headers); colours come from the widget's own settings, so customising a widget
// still works on top of its theme.

// ---------- Fonts ----------

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Family {
    Inter,
    Mono,
    Condensed,
    Pixel,
    PixelTitle,
    Terminal,
    Handwriting,
    Round,
    Serif,
    SerifDisplay,
    Techno,
    Retro,
    ArcadeFont,
    Roman,
    Heavy,
    Horror,
    Friendly,
}
/// One weight of a bundled family. Inter, JetBrains Mono and Pixelify Sans are variable fonts.
fn family_font(family: Family, weight: u16) -> FontArc {
    static CACHE: OnceLock<Mutex<std::collections::HashMap<(Family, u16), FontArc>>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap();
    cache
        .entry((family, weight))
        .or_insert_with(|| {
            use ab_glyph::{FontVec, VariableFont};
            let bytes: &'static [u8] = match family {
                Family::Inter => include_bytes!("../../../fonts/inter/Inter.ttf"),
                Family::Mono => include_bytes!("../../../fonts/jetbrainsmono/JetBrainsMono.ttf"),
                Family::Condensed if weight >= 800 => include_bytes!("../../../fonts/barlowcondensed/BarlowCondensed-Black.ttf"),
                Family::Condensed if weight >= 600 => include_bytes!("../../../fonts/barlowcondensed/BarlowCondensed-Bold.ttf"),
                Family::Condensed => include_bytes!("../../../fonts/barlowcondensed/BarlowCondensed-Regular.ttf"),
                Family::Pixel => include_bytes!("../../../fonts/pixelifysans/PixelifySans.ttf"),
                Family::PixelTitle if weight >= 600 => include_bytes!("../../../fonts/silkscreen/Silkscreen-Bold.ttf"),
                Family::PixelTitle => include_bytes!("../../../fonts/silkscreen/Silkscreen-Regular.ttf"),
                Family::Terminal => include_bytes!("../../../fonts/vt323/VT323-Regular.ttf"),
                Family::Handwriting => include_bytes!("../../../fonts/caveat/Caveat.ttf"),
                Family::Round => include_bytes!("../../../fonts/fredoka/Fredoka.ttf"),
                Family::Serif => include_bytes!("../../../fonts/ebgaramond/EBGaramond.ttf"),
                Family::SerifDisplay => include_bytes!("../../../fonts/cormorantgaramond/CormorantGaramond.ttf"),
                Family::Techno => include_bytes!("../../../fonts/orbitron/Orbitron.ttf"),
                Family::Retro => include_bytes!("../../../fonts/audiowide/Audiowide-Regular.ttf"),
                Family::ArcadeFont => include_bytes!("../../../fonts/pressstart2p/PressStart2P-Regular.ttf"),
                Family::Roman => include_bytes!("../../../fonts/cinzel/Cinzel.ttf"),
                Family::Heavy => include_bytes!("../../../fonts/archivoblack/ArchivoBlack-Regular.ttf"),
                Family::Horror => include_bytes!("../../../fonts/creepster/Creepster-Regular.ttf"),
                Family::Friendly => include_bytes!("../../../fonts/rubik/Rubik.ttf"),
            };
            match FontVec::try_from_vec(bytes.to_vec()) {
                Ok(mut f) => {
                    f.set_variation(b"wght", f32::from(weight));
                    FontArc::new(f)
                }
                Err(_) => font(FontFace::Sans).clone(),
            }
        })
        .clone()
}

// ---------- Colour ----------

fn mix(a: [u8; 4], b: [u8; 4], t: f32) -> [u8; 4] {
    let t = t.clamp(0., 1.);
    std::array::from_fn(|i| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t).round() as u8)
}
fn fade(c: [u8; 4], a: f32) -> [u8; 4] {
    [c[0], c[1], c[2], (c[3] as f32 * a.clamp(0., 1.)).round() as u8]
}
fn lighten(c: [u8; 4], t: f32) -> [u8; 4] {
    mix(c, [255, 255, 255, c[3]], t)
}
fn darken(c: [u8; 4], t: f32) -> [u8; 4] {
    mix(c, [0, 0, 0, c[3]], t)
}
fn luma(c: [u8; 4]) -> f32 {
    (0.2126 * c[0] as f32 + 0.7152 * c[1] as f32 + 0.0722 * c[2] as f32) / 255.
}
/// Text colour that reads on a filled colour.
fn on_color(c: [u8; 4]) -> [u8; 4] {
    if luma(c) > 0.62 { [18, 20, 26, 255] } else { [255; 4] }
}
fn platform_color(name: &str) -> [u8; 4] {
    match name {
        "Twitch" => [145, 70, 255, 255],
        "YouTube" => [255, 40, 60, 255],
        "Kick" => [83, 252, 24, 255],
        "TikTok" => [254, 44, 85, 255],
        _ => [150, 160, 180, 255],
    }
}
/// Repeatable pseudo-random value in 0–1 for a pixel (textures, sparkles, snow).
fn hash(x: i32, y: i32) -> f32 {
    let mut h = (x as u32).wrapping_mul(374_761_393) ^ (y as u32).wrapping_mul(668_265_263);
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    (h ^ (h >> 16)) as f32 / u32::MAX as f32
}
/// Smooth value noise with cells of `cell` pixels.
fn noise(x: f32, y: f32, cell: f32) -> f32 {
    let (gx, gy) = (x / cell, y / cell);
    let (ix, iy) = (gx.floor() as i32, gy.floor() as i32);
    let (fx, fy) = (gx - gx.floor(), gy - gy.floor());
    let (sx, sy) = (fx * fx * (3. - 2. * fx), fy * fy * (3. - 2. * fy));
    let top = hash(ix, iy) + (hash(ix + 1, iy) - hash(ix, iy)) * sx;
    let bottom = hash(ix, iy + 1) + (hash(ix + 1, iy + 1) - hash(ix, iy + 1)) * sx;
    top + (bottom - top) * sy
}
/// Distance to a rectangle with its top-left and bottom-right corners cut off at 45°.
fn chamfer_distance(px: f32, py: f32, x: f32, y: f32, w: f32, h: f32, cut: f32) -> f32 {
    let bx = rr_distance(px, py, x, y, w, h, 0.);
    let s = std::f32::consts::FRAC_1_SQRT_2;
    let tl = (cut - ((px - x) + (py - y))) * s;
    let br = (((px - x) + (py - y)) - (w + h - cut)) * s;
    bx.max(tl).max(br)
}
fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0., 1.);
    t * t * (3. - 2. * t)
}

// ---------- Drawing ----------

#[derive(Clone, Copy, Debug)]
struct Area {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}
impl Area {
    fn inset(self, l: f32, t: f32, r: f32, b: f32) -> Self {
        Self { x: self.x + l, y: self.y + t, w: (self.w - l - r).max(1.), h: (self.h - t - b).max(1.) }
    }
    fn bottom(self) -> f32 {
        self.y + self.h
    }
    fn right(self) -> f32 {
        self.x + self.w
    }
}
fn rr_distance(px: f32, py: f32, x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
    let (hw, hh) = (w / 2., h / 2.);
    let r = r.min(hw).min(hh).max(0.);
    let dx = (px - (x + hw)).abs() - hw + r;
    let dy = (py - (y + hh)).abs() - hh + r;
    dx.max(0.).hypot(dy.max(0.)) + dx.max(dy).min(0.) - r
}
/// Coverage of one glyph run, handed to `put` pixel by pixel (kerning and letter spacing included).
fn raster(f: &FontArc, size: f32, text: &str, x: f32, baseline: f32, track: f32, mut put: impl FnMut(i32, i32, f32)) -> f32 {
    let scaled = f.as_scaled(PxScale::from(size));
    let mut pen = x;
    let mut prev = None;
    for ch in text.chars().take(2000) {
        let id = scaled.glyph_id(ch);
        if let Some(p) = prev {
            pen += scaled.kern(p, id);
        }
        if let Some(outline) = f.outline_glyph(id.with_scale_and_position(size, point(pen, baseline))) {
            let b = outline.px_bounds();
            outline.draw(|gx, gy, cov| put(gx as i32 + b.min.x as i32, gy as i32 + b.min.y as i32, cov));
        }
        pen += scaled.h_advance(id) + track;
        prev = Some(id);
    }
    pen - x - if text.is_empty() { 0. } else { track }
}
fn measure(f: &FontArc, size: f32, text: &str, track: f32) -> f32 {
    let scaled = f.as_scaled(PxScale::from(size));
    let mut prev = None;
    let mut w = 0.;
    for ch in text.chars().take(2000) {
        let id = scaled.glyph_id(ch);
        if let Some(p) = prev {
            w += scaled.kern(p, id);
        }
        w += scaled.h_advance(id) + track;
        prev = Some(id);
    }
    if text.is_empty() { 0. } else { w - track }
}
/// A coverage buffer for effects: shadows, glows and dot-matrix text.
struct Mask {
    x0: i32,
    y0: i32,
    w: i32,
    h: i32,
    a: Vec<f32>,
}
impl Mask {
    fn new(x0: i32, y0: i32, w: i32, h: i32) -> Self {
        let (w, h) = (w.max(1), h.max(1));
        Self { x0, y0, w, h, a: vec![0.; (w * h) as usize] }
    }
    fn put(&mut self, x: i32, y: i32, c: f32) {
        let (lx, ly) = (x - self.x0, y - self.y0);
        if lx >= 0 && ly >= 0 && lx < self.w && ly < self.h {
            let i = (ly * self.w + lx) as usize;
            self.a[i] = (self.a[i] + c).min(1.);
        }
    }
    fn blur(&mut self, radius: i32) {
        if radius < 1 {
            return;
        }
        let (w, h) = (self.w as usize, self.h as usize);
        let r = radius as usize;
        for _ in 0..2 {
            let mut tmp = vec![0.; w * h];
            for y in 0..h {
                let row = &self.a[y * w..(y + 1) * w];
                let mut sum: f32 = row[..r.min(w)].iter().sum();
                for x in 0..w {
                    if x + r < w {
                        sum += row[x + r];
                    }
                    if x > r {
                        sum -= row[x - r - 1];
                    }
                    tmp[y * w + x] = sum / (2 * r + 1) as f32;
                }
            }
            for x in 0..w {
                let mut sum: f32 = (0..r.min(h)).map(|y| tmp[y * w + x]).sum();
                for y in 0..h {
                    if y + r < h {
                        sum += tmp[(y + r) * w + x];
                    }
                    if y > r {
                        sum -= tmp[(y - r - 1) * w + x];
                    }
                    self.a[y * w + x] = sum / (2 * r + 1) as f32;
                }
            }
        }
    }
    /// Paints with a three-stop vertical gradient between `top` and `bottom` (screen rows).
    #[allow(clippy::too_many_arguments)]
    fn paint_gradient(&self, p: &mut Paint, top: f32, bottom: f32, a: [u8; 4], b: [u8; 4], c: [u8; 4]) {
        for ly in 0..self.h {
            let t = ((self.y0 + ly) as f32 - top) / (bottom - top).max(1.);
            let color = if t < 0.5 { mix(a, b, t * 2.) } else { mix(b, c, (t - 0.5) * 2.) };
            for lx in 0..self.w {
                let v = self.a[(ly * self.w + lx) as usize];
                if v > 0.003 {
                    p.blend(self.x0 + lx, self.y0 + ly, color, v.min(1.));
                }
            }
        }
    }
    fn paint(&self, p: &mut Paint, c: [u8; 4], dx: i32, dy: i32, gain: f32) {
        for ly in 0..self.h {
            for lx in 0..self.w {
                let a = self.a[(ly * self.w + lx) as usize] * gain;
                if a > 0.003 {
                    p.blend(self.x0 + lx + dx, self.y0 + ly + dy, c, a.min(1.));
                }
            }
        }
    }
}
impl Paint {
    fn bounds(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> (i32, i32, i32, i32) {
        ((x0.floor() as i32).max(0), (y0.floor() as i32).max(0), (x1.ceil() as i32).min(self.width as i32), (y1.ceil() as i32).min(self.height as i32))
    }
    /// A rounded rectangle filled top to bottom from `top` to `bottom`; only rows in `rows` are painted.
    #[allow(clippy::too_many_arguments)]
    fn fill_rows(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, rows: (f32, f32), top: [u8; 4], bottom: [u8; 4]) {
        let (x0, y0, x1, y1) = self.bounds(x, y.max(rows.0), x + w, (y + h).min(rows.1));
        let span = (rows.1 - rows.0).max(1.);
        for py in y0..y1 {
            let c = mix(top, bottom, (py as f32 + 0.5 - rows.0) / span);
            for px in x0..x1 {
                let d = rr_distance(px as f32 + 0.5, py as f32 + 0.5, x, y, w, h, r);
                let cov = (0.5 - d).clamp(0., 1.);
                if cov > 0. {
                    self.blend(px, py, c, cov);
                }
            }
        }
    }
    fn fill_rr(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, top: [u8; 4], bottom: [u8; 4]) {
        self.fill_rows(x, y, w, h, r, (y, y + h), top, bottom);
    }
    /// Left-to-right gradient.
    fn fill_rr_across(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, left: [u8; 4], right: [u8; 4]) {
        let (x0, y0, x1, y1) = self.bounds(x, y, x + w, y + h);
        for py in y0..y1 {
            for px in x0..x1 {
                let d = rr_distance(px as f32 + 0.5, py as f32 + 0.5, x, y, w, h, r);
                let cov = (0.5 - d).clamp(0., 1.);
                if cov > 0. {
                    self.blend(px, py, mix(left, right, (px as f32 - x) / w.max(1.)), cov);
                }
            }
        }
    }
    fn stroke_rr(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, width: f32, c: [u8; 4]) {
        let (x0, y0, x1, y1) = self.bounds(x - 1., y - 1., x + w + 1., y + h + 1.);
        for py in y0..y1 {
            for px in x0..x1 {
                let d = rr_distance(px as f32 + 0.5, py as f32 + 0.5, x, y, w, h, r);
                let cov = (0.5 - d).clamp(0., 1.) * (0.5 + d + width).clamp(0., 1.);
                if cov > 0. {
                    self.blend(px, py, c, cov);
                }
            }
        }
    }
    /// A soft drop shadow under a rounded rectangle.
    fn shadow_rr(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, blur: f32, c: [u8; 4]) {
        let (x0, y0, x1, y1) = self.bounds(x - blur, y - blur, x + w + blur, y + h + blur);
        for py in y0..y1 {
            for px in x0..x1 {
                let d = rr_distance(px as f32 + 0.5, py as f32 + 0.5, x, y, w, h, r);
                let a = 1. - smoothstep(-blur * 0.6, blur, d);
                if a > 0.002 {
                    self.blend(px, py, c, a * a);
                }
            }
        }
    }
    /// Fills any shape given by its signed distance, top-to-bottom gradient.
    #[allow(clippy::too_many_arguments)]
    fn fill_shape(&mut self, x: f32, y: f32, w: f32, h: f32, d: impl Fn(f32, f32) -> f32, top: [u8; 4], bottom: [u8; 4]) {
        let (x0, y0, x1, y1) = self.bounds(x - 1., y - 1., x + w + 1., y + h + 1.);
        for py in y0..y1 {
            let c = mix(top, bottom, (py as f32 - y) / h.max(1.));
            for px in x0..x1 {
                let cov = (0.5 - d(px as f32 + 0.5, py as f32 + 0.5)).clamp(0., 1.);
                if cov > 0. {
                    self.blend(px, py, c, cov);
                }
            }
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn stroke_shape(&mut self, x: f32, y: f32, w: f32, h: f32, d: impl Fn(f32, f32) -> f32, width: f32, c: [u8; 4]) {
        let (x0, y0, x1, y1) = self.bounds(x - 2., y - 2., x + w + 2., y + h + 2.);
        for py in y0..y1 {
            for px in x0..x1 {
                let dd = d(px as f32 + 0.5, py as f32 + 0.5);
                let cov = (0.5 - dd).clamp(0., 1.) * (0.5 + dd + width).clamp(0., 1.);
                if cov > 0. {
                    self.blend(px, py, c, cov);
                }
            }
        }
    }
    /// A glow (or soft shadow) around any shape.
    #[allow(clippy::too_many_arguments)]
    fn glow_shape(&mut self, x: f32, y: f32, w: f32, h: f32, d: impl Fn(f32, f32) -> f32, blur: f32, c: [u8; 4]) {
        let (x0, y0, x1, y1) = self.bounds(x - blur, y - blur, x + w + blur, y + h + blur);
        for py in y0..y1 {
            for px in x0..x1 {
                let dd = d(px as f32 + 0.5, py as f32 + 0.5);
                if dd > 0. {
                    let a = 1. - smoothstep(0., blur, dd);
                    if a > 0.002 {
                        self.blend(px, py, c, a * a);
                    }
                }
            }
        }
    }
    /// A four-point sparkle.
    fn sparkle(&mut self, cx: f32, cy: f32, size: f32, c: [u8; 4]) {
        self.fill_shape(cx - size, cy - size, size * 2., size * 2., |px, py| {
            let (dx, dy) = ((px - cx).abs(), (py - cy).abs());
            (dx.sqrt() + dy.sqrt()) - size.sqrt()
        }, c, c);
    }
    fn circle(&mut self, cx: f32, cy: f32, radius: f32, c: [u8; 4]) {
        self.fill_rr(cx - radius, cy - radius, radius * 2., radius * 2., radius, c, c);
    }
    /// An arc of a ring from `from` to `to` (fractions of a turn, starting at 12 o'clock), round caps.
    #[allow(clippy::too_many_arguments)]
    fn arc(&mut self, cx: f32, cy: f32, radius: f32, thick: f32, from: f32, to: f32, start: [u8; 4], end: [u8; 4]) {
        let (x0, y0, x1, y1) = self.bounds(cx - radius - thick, cy - radius - thick, cx + radius + thick, cy + radius + thick);
        let half = thick / 2.;
        let cap = |t: f32| {
            let a = t * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
            (cx + radius * a.cos(), cy + radius * a.sin())
        };
        let (s, e) = (cap(from), cap(to));
        for py in y0..y1 {
            for px in x0..x1 {
                let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
                let (dx, dy) = (fx - cx, fy - cy);
                let turn = (dy.atan2(dx) + std::f32::consts::FRAC_PI_2).rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU;
                let ring = (dx.hypot(dy) - radius).abs() - half;
                let d = if turn >= from && turn <= to {
                    ring
                } else {
                    ((fx - s.0).hypot(fy - s.1) - half).min((fx - e.0).hypot(fy - e.1) - half)
                };
                let cov = (0.5 - d).clamp(0., 1.);
                if cov > 0. {
                    self.blend(px, py, mix(start, end, ((turn - from) / (to - from).max(0.001)).clamp(0., 1.)), cov);
                }
            }
        }
    }
    fn hline(&mut self, x: f32, y: f32, w: f32, c: [u8; 4]) {
        self.fill_rr(x, y, w, 1., 0., c, c);
    }
    fn vline(&mut self, x: f32, y: f32, h: f32, c: [u8; 4]) {
        self.fill_rr(x, y, 1., h, 0., c, c);
    }
    /// Classic raised (or sunken) bevel, as Windows 98 drew it.
    fn bevel(&mut self, x: f32, y: f32, w: f32, h: f32, raised: bool) {
        let (light, mid, dark, black) = ([255, 255, 255, 255], [223, 223, 223, 255], [128, 128, 128, 255], [10, 10, 10, 255]);
        let (tl, tl2, br, br2) = if raised { (light, mid, black, dark) } else { (dark, black, light, mid) };
        self.hline(x, y, w, tl);
        self.vline(x, y, h, tl);
        self.hline(x + 1., y + 1., w - 2., tl2);
        self.vline(x + 1., y + 1., h - 2., tl2);
        self.hline(x, y + h - 1., w, br);
        self.vline(x + w - 1., y, h, br);
        self.hline(x + 1., y + h - 2., w - 2., br2);
        self.vline(x + w - 2., y + 1., h - 2., br2);
    }
}

// ---------- Look: a theme applied to one widget ----------

#[derive(Clone, Copy, PartialEq)]
enum Role {
    /// Big numbers and hero text.
    Display,
    /// Song titles, chat names, list headings.
    Title,
    Body,
    Strong,
    /// Small uppercase labels ("NOW PLAYING").
    Label,
    Caption,
}
#[derive(Clone, Copy, PartialEq)]
enum Fx {
    Auto,
    Plain,
}
struct Look {
    theme: Theme,
    fg: [u8; 4],
    bg: [u8; 4],
    accent: [u8; 4],
    radius: f32,
    mono: bool,
    bold: bool,
    /// Secondary and tertiary text.
    sec: [u8; 4],
    ter: [u8; 4],
    /// Fill for rows and tracks inside the panel.
    surface: [u8; 4],
    divider: [u8; 4],
    /// Text inside Windows list fields.
    field_fg: [u8; 4],
    /// A light panel (dark text).
    light: bool,
    /// While a see-through panel is drawn, its text goes here so only the frame fades.
    sink: std::cell::RefCell<Option<Paint>>,
    /// The panel title's size, when it shouldn't follow the panel's height.
    title_size: Option<f32>,
}
impl Look {
    fn new(s: &Settings) -> Self {
        let fg = s.foreground;
        let light_bg = s.background[3] > 120 && luma(s.background) > 0.6;
        Self {
            theme: s.theme,
            fg,
            bg: s.background,
            accent: s.accent,
            radius: s.radius as f32,
            mono: s.font == FontFace::Mono,
            bold: s.bold,
            sec: fade(fg, 0.72),
            ter: fade(fg, 0.5),
            surface: if light_bg { [0, 0, 0, 14] } else { fade(fg, 0.07) },
            divider: fade(fg, 0.12),
            field_fg: if matches!(s.theme, Theme::Windows98 | Theme::WindowsXP | Theme::Vaporwave) { [12, 12, 12, 255] } else { fg },
            light: light_bg,
            sink: Default::default(),
            title_size: None,
        }
        .with_accent(s.accent)
    }
    /// Sets the accent, darkening a light one on light backgrounds so accent text stays readable.
    fn with_accent(mut self, accent: [u8; 4]) -> Self {
        // Window themes put accent text straight on light grey; other light themes keep their colour.
        self.accent = if self.retro_window() && luma(accent) > 0.45 { darken(accent, 0.45) } else { accent };
        self
    }
    fn retro_window(&self) -> bool {
        matches!(self.theme, Theme::Windows98 | Theme::WindowsXP | Theme::Vaporwave)
    }
    fn glows(&self) -> bool {
        matches!(self.theme, Theme::Terminal | Theme::Led | Theme::SciFiHud)
    }
    /// A colour turned into readable text on this panel (darker on light panels).
    fn ink(&self, c: [u8; 4]) -> [u8; 4] {
        if self.light { darken(c, 0.35) } else { lighten(c, 0.45) }
    }
    /// Square-cornered themes.
    fn square(&self) -> bool {
        matches!(self.theme, Theme::Pixel | Theme::Windows98 | Theme::Terminal | Theme::Led | Theme::GameBoy | Theme::SciFiHud | Theme::Cyberpunk | Theme::Vaporwave)
    }
    fn font(&self, role: Role) -> FontArc {
        let heavy = |w: u16| if self.bold { (w + 100).min(900) } else { w };
        match self.theme {
            Theme::Led => family_font(Family::Terminal, 400),
            Theme::Pixel => match role {
                Role::Label => family_font(Family::PixelTitle, 700),
                Role::Display => family_font(Family::Pixel, 700),
                Role::Title | Role::Strong => family_font(Family::Pixel, 650),
                Role::Body | Role::Caption => family_font(Family::Pixel, heavy(500)),
            },
            Theme::Terminal => family_font(Family::Mono, match role { Role::Display => 800, Role::Title | Role::Strong | Role::Label => 700, _ => heavy(500) }),
            Theme::GameBoy => match role {
                Role::Display | Role::Label => family_font(Family::ArcadeFont, 400),
                _ => family_font(Family::Pixel, if matches!(role, Role::Title | Role::Strong) { 650 } else { heavy(500) }),
            },
            Theme::Arcade => match role {
                Role::Display | Role::Label | Role::Title => family_font(Family::ArcadeFont, 400),
                _ => family_font(Family::Friendly, if role == Role::Strong { 700 } else { heavy(500) }),
            },
            Theme::CozyCafe => match role {
                Role::Label => family_font(Family::Handwriting, 650),
                Role::Display | Role::Title => family_font(Family::Round, 600),
                Role::Strong => family_font(Family::Friendly, 600),
                _ => family_font(Family::Friendly, heavy(420)),
            },
            Theme::Kawaii => family_font(Family::Round, match role { Role::Display => 650, Role::Title | Role::Label | Role::Strong => 600, _ => heavy(480) }),
            Theme::Winter => match role {
                Role::Display | Role::Title => family_font(Family::Round, 620),
                Role::Label => family_font(Family::Inter, 650),
                Role::Strong => family_font(Family::Inter, 620),
                _ => family_font(Family::Inter, heavy(460)),
            },
            Theme::Cottagecore => match role {
                Role::Display | Role::Title | Role::Label => family_font(Family::SerifDisplay, 700),
                Role::Strong => family_font(Family::Serif, 650),
                _ => family_font(Family::Serif, heavy(500)),
            },
            Theme::Fantasy => match role {
                Role::Display | Role::Title | Role::Label => family_font(Family::Roman, 700),
                Role::Strong => family_font(Family::Serif, 700),
                _ => family_font(Family::Serif, heavy(520)),
            },
            Theme::Cyberpunk | Theme::SciFiHud => match role {
                Role::Display => family_font(Family::Techno, 800),
                Role::Title | Role::Label => family_font(Family::Techno, 650),
                Role::Strong => family_font(Family::Inter, 620),
                _ => family_font(Family::Inter, heavy(460)),
            },
            Theme::Synthwave => match role {
                Role::Display | Role::Title | Role::Label => family_font(Family::Retro, 400),
                Role::Strong => family_font(Family::Inter, 650),
                _ => family_font(Family::Inter, heavy(470)),
            },
            Theme::Neubrutalism => match role {
                Role::Display | Role::Title => family_font(Family::Heavy, 400),
                Role::Label => family_font(Family::Inter, 850),
                Role::Strong => family_font(Family::Inter, 750),
                _ => family_font(Family::Inter, heavy(560)),
            },
            Theme::Spooky => match role {
                Role::Display | Role::Title | Role::Label => family_font(Family::Horror, 400),
                Role::Strong => family_font(Family::Inter, 650),
                _ => family_font(Family::Inter, heavy(470)),
            },
            Theme::Vaporwave => match role {
                Role::Display => family_font(Family::Retro, 400),
                _ => family_font(Family::Inter, match role { Role::Title | Role::Strong | Role::Label => 650, _ => heavy(460) }),
            },
            _ if self.mono => family_font(Family::Mono, match role { Role::Display => 800, Role::Title | Role::Strong | Role::Label => 700, _ => heavy(450) }),
            Theme::Broadcast | Theme::Team => match role {
                Role::Display => family_font(Family::Condensed, 800),
                Role::Title | Role::Label => family_font(Family::Condensed, 700),
                Role::Strong => family_font(Family::Inter, 650),
                Role::Body | Role::Caption => family_font(Family::Inter, heavy(480)),
            },
            _ => family_font(Family::Inter, match role {
                Role::Display => 750,
                Role::Title => 650,
                Role::Strong => 620,
                Role::Label => 650,
                Role::Body => heavy(460),
                Role::Caption => 520,
            }),
        }
    }
    /// Letter spacing as a fraction of the size.
    fn track(&self, role: Role) -> f32 {
        match (role, self.theme) {
            (_, Theme::Led) => 0.04,
            (_, Theme::Arcade | Theme::GameBoy) if matches!(role, Role::Display | Role::Label | Role::Title) => 0.02,
            (Role::Label, Theme::CozyCafe) => 0.,
            (Role::Label, Theme::Cyberpunk | Theme::SciFiHud | Theme::Fantasy) => 0.16,
            (Role::Label | Role::Title, Theme::Spooky) => 0.03,
            (Role::Label, Theme::Pixel) => 0.04,
            (Role::Label, Theme::Windows98 | Theme::WindowsXP) => 0.,
            (Role::Label, _) => 0.11,
            (Role::Display, Theme::Broadcast | Theme::Team) => 0.01,
            (Role::Display, _) => -0.02,
            (Role::Title, _) => -0.005,
            _ => 0.,
        }
    }
    fn upper(&self, role: Role) -> bool {
        role == Role::Label && !self.retro_window() && self.theme != Theme::CozyCafe
            || (matches!(self.theme, Theme::Broadcast | Theme::Team | Theme::Cyberpunk | Theme::SciFiHud) && role == Role::Title)
    }
    fn size_for(&self, role: Role, size: f32) -> f32 {
        // VT323 and the pixel fonts draw small for their size.
        match (self.theme, role) {
            (Theme::Led, _) => size * 1.25,
            (Theme::Pixel, Role::Label) => size * 0.85,
            // Press Start 2P is very wide; Caveat and Creepster run small.
            (Theme::Arcade | Theme::GameBoy, Role::Display | Role::Label | Role::Title) => size * 0.62,
            (Theme::CozyCafe, Role::Label) => size * 1.45,
            (Theme::Spooky, Role::Label | Role::Title | Role::Display) => size * 1.12,
            _ => size,
        }
    }
    fn prepared(&self, role: Role, text: &str) -> String {
        if self.upper(role) { text.to_uppercase() } else { text.to_owned() }
    }
    fn width(&self, role: Role, text: &str, size: f32) -> f32 {
        let size = self.size_for(role, size);
        measure(&self.font(role), size, &self.prepared(role, text), self.track(role) * size)
    }
    /// Draws one line of text (shortened with "…" to `max_w`) with the theme's effects. Returns its width.
    #[allow(clippy::too_many_arguments)]
    fn text(&self, p: &mut Paint, role: Role, text: &str, x: f32, baseline: f32, size: f32, c: [u8; 4], max_w: f32) -> f32 {
        self.text_fx(p, role, text, x, baseline, size, c, max_w, Fx::Auto)
    }
    #[allow(clippy::too_many_arguments)]
    fn text_fx(&self, p: &mut Paint, role: Role, text: &str, x: f32, baseline: f32, size: f32, c: [u8; 4], max_w: f32, fx: Fx) -> f32 {
        if let Some(sink) = self.sink.borrow_mut().as_mut() {
            return self.text_draw(sink, role, text, x, baseline, size, c, max_w, fx);
        }
        self.text_draw(p, role, text, x, baseline, size, c, max_w, fx)
    }
    #[allow(clippy::too_many_arguments)]
    fn text_draw(&self, p: &mut Paint, role: Role, text: &str, x: f32, baseline: f32, size: f32, c: [u8; 4], max_w: f32, fx: Fx) -> f32 {
        let f = self.font(role);
        let size = self.size_for(role, size);
        let track = self.track(role) * size;
        let mut t = self.prepared(role, text).replace('\n', " ");
        if measure(&f, size, &t, track) > max_w {
            while !t.is_empty() && measure(&f, size, &t, track) + measure(&f, size, "…", 0.) > max_w {
                t.pop();
            }
            t = format!("{}…", t.trim_end());
        }
        if t.is_empty() {
            return 0.;
        }
        let width = measure(&f, size, &t, track);
        let styled = matches!(role, Role::Display | Role::Title)
            && matches!(self.theme, Theme::Cyberpunk | Theme::Synthwave | Theme::Arcade | Theme::Neubrutalism | Theme::Spooky);
        if fx == Fx::Auto && styled {
            let pad = (size * 0.6) as i32 + 6;
            let mut m = Mask::new(x as i32 - pad, (baseline - size * 1.1) as i32 - pad, width as i32 + pad * 2, (size * 1.5) as i32 + pad * 2);
            raster(&f, size, &t, x, baseline, track, |px, py, cov| m.put(px, py, cov));
            let top = baseline - size * 0.8;
            match self.theme {
                Theme::Cyberpunk if role == Role::Display => {
                    // A chromatic split, like a glitching screen.
                    let o = (size * 0.035).max(1.) as i32;
                    m.paint(p, [0, 240, 255, 255], -o, 0, 0.75);
                    m.paint(p, self.accent, o, 0, 0.75);
                    m.paint(p, c, 0, 0, 1.);
                }
                Theme::Cyberpunk | Theme::Spooky => {
                    let mut halo = Mask { a: m.a.clone(), ..Mask::new(m.x0, m.y0, m.w, m.h) };
                    halo.blur((size * 0.14).max(2.) as i32);
                    halo.paint(p, self.accent, 0, 0, if self.theme == Theme::Spooky { 1.1 } else { 0.9 });
                    m.paint(p, c, 0, 0, 1.);
                }
                Theme::Synthwave => {
                    let mut halo = Mask { a: m.a.clone(), ..Mask::new(m.x0, m.y0, m.w, m.h) };
                    halo.blur((size * 0.16).max(2.) as i32);
                    halo.paint(p, self.accent, 0, 0, 0.8);
                    m.paint_gradient(p, top, baseline, [255, 255, 255, 255], [255, 150, 90, 255], [255, 80, 170, 255]);
                }
                Theme::Arcade => {
                    let o = (size * 0.07).max(2.) as i32;
                    m.paint(p, [190, 30, 40, 255], o, o, 1.);
                    m.paint_gradient(p, top, baseline, [255, 250, 160, 255], [255, 214, 0, 255], [255, 130, 0, 255]);
                }
                _ => {
                    // Neubrutalism: a hard shadow in the accent colour.
                    let o = (size * 0.04).max(2.) as i32;
                    m.paint(p, self.accent, o, o, 1.);
                    m.paint(p, c, 0, 0, 1.);
                }
            }
            return width;
        }
        if fx == Fx::Auto && (self.theme == Theme::Transparent || self.glows()) {
            let pad = (size * 0.6) as i32 + 4;
            let mut m = Mask::new(x as i32 - pad, (baseline - size * 1.1) as i32 - pad, width as i32 + pad * 2, (size * 1.5) as i32 + pad * 2);
            raster(&f, size, &t, x, baseline, track, |px, py, cov| m.put(px, py, cov));
            if self.theme == Theme::Led && role == Role::Display && size >= 30. {
                self.dots(p, &m, size, c);
                return width;
            }
            let mut halo = Mask { a: m.a.clone(), ..Mask::new(m.x0, m.y0, m.w, m.h) };
            if self.theme == Theme::Transparent {
                halo.blur((size * 0.08).max(2.) as i32);
                halo.paint(p, [0, 0, 0, 230], 0, (size * 0.05).max(1.) as i32, 1.6);
            } else {
                halo.blur((size * 0.16).max(2.) as i32);
                halo.paint(p, c, 0, 0, if self.theme == Theme::SciFiHud { 0.45 } else { 0.9 });
            }
            m.paint(p, c, 0, 0, 1.);
            return width;
        }
        raster(&f, size, &t, x, baseline, track, |px, py, cov| p.blend(px, py, c, cov));
        width
    }
    /// Dot-matrix lettering: lit LEDs where the text covers each cell, with a soft glow.
    fn dots(&self, p: &mut Paint, m: &Mask, size: f32, c: [u8; 4]) {
        let pitch = (size / 10.).clamp(3., 9.);
        let mut lit = Mask::new(m.x0, m.y0, m.w, m.h);
        let mut gy = 0.;
        while gy < m.h as f32 {
            let mut gx = 0.;
            while gx < m.w as f32 {
                let mut sum = 0.;
                let mut n = 0.;
                for yy in gy as i32..(gy + pitch) as i32 {
                    for xx in gx as i32..(gx + pitch) as i32 {
                        if xx < m.w && yy < m.h {
                            sum += m.a[(yy * m.w + xx) as usize];
                            n += 1.;
                        }
                    }
                }
                if n > 0. && sum / n > 0.32 {
                    let (cx, cy) = (m.x0 as f32 + gx + pitch / 2., m.y0 as f32 + gy + pitch / 2.);
                    let r = pitch * 0.42;
                    for yy in (cy - r) as i32 - 1..=(cy + r) as i32 + 1 {
                        for xx in (cx - r) as i32 - 1..=(cx + r) as i32 + 1 {
                            let d = (xx as f32 + 0.5 - cx).hypot(yy as f32 + 0.5 - cy) - r;
                            lit.put(xx, yy, (0.5 - d).clamp(0., 1.));
                        }
                    }
                }
                gx += pitch;
            }
            gy += pitch;
        }
        let mut glow = Mask { a: lit.a.clone(), ..Mask::new(lit.x0, lit.y0, lit.w, lit.h) };
        glow.blur((pitch * 0.9) as i32);
        glow.paint(p, c, 0, 0, 0.8);
        lit.paint(p, lighten(c, 0.35), 0, 0, 1.);
    }
    /// Text aligned within a box: 0 = left, 0.5 = centre, 1 = right.
    #[allow(clippy::too_many_arguments)]
    fn text_in(&self, p: &mut Paint, role: Role, text: &str, a: Area, baseline: f32, size: f32, c: [u8; 4], align: f32) -> f32 {
        let w = self.width(role, text, size).min(a.w);
        self.text(p, role, text, a.x + (a.w - w) * align, baseline, size, c, a.w)
    }
    /// Text and inline pictures (emoji, emotes) on one line.
    #[allow(clippy::too_many_arguments)]
    fn rich(&self, p: &mut Paint, role: Role, text: &str, x: f32, baseline: f32, size: f32, c: [u8; 4], max_w: f32, at_ms: u64) {
        let pic = size * 1.22;
        let mut pen = x;
        for piece in chat_pieces(text) {
            let room = x + max_w - pen;
            if room <= 2. {
                break;
            }
            match &piece {
                ChatPiece::Text(t) => {
                    pen += self.text(p, role, t, pen, baseline, size, c, room) + if t.ends_with(' ') { self.width(role, " ", size) } else { 0. };
                }
                ChatPiece::Emoji(key) => {
                    if pic > room {
                        break;
                    }
                    if let Some(img) = crate::twemoji::rgba(key) {
                        p.picture(&img, pen + 1., baseline - size * 0.98, pic, pic);
                    }
                    pen += pic + 2.;
                }
                ChatPiece::Emote(url, name) => match crate::chat_render::canvas_frame(url, at_ms) {
                    Some(img) => {
                        let w = pic * img.width() as f32 / img.height().max(1) as f32;
                        if w > room {
                            break;
                        }
                        p.picture(&img, pen + 1., baseline - size * 0.98, w, pic);
                        pen += w + 2.;
                    }
                    None => pen += self.text(p, role, name, pen, baseline, size, c, room),
                },
            }
        }
    }
    fn rich_width(&self, role: Role, text: &str, size: f32) -> f32 {
        let pic = size * 1.22;
        chat_pieces(text)
            .iter()
            .map(|p| match p {
                ChatPiece::Text(t) => self.width(role, t, size) + if t.ends_with(' ') { self.width(role, " ", size) } else { 0. },
                ChatPiece::Emoji(_) => pic + 2.,
                ChatPiece::Emote(url, name) => match crate::chat_render::canvas_frame(url, 0) {
                    Some(img) => pic * img.width() as f32 / img.height().max(1) as f32 + 2.,
                    None => self.width(role, name, size),
                },
            })
            .sum()
    }
    /// Word wrap measured in this look's font, pictures included.
    fn wrap(&self, role: Role, text: &str, width: f32, size: f32) -> Vec<String> {
        let mut rows = vec![String::new()];
        for word in text.split_whitespace() {
            let last = rows.last_mut().unwrap();
            if !last.is_empty() && self.rich_width(role, &format!("{last} {word}"), size) > width {
                rows.push(word.into());
            } else {
                if !last.is_empty() {
                    last.push(' ');
                }
                last.push_str(word);
            }
        }
        rows
    }
    /// Digits drawn in equal cells, so counting numbers don't shift sideways.
    #[allow(clippy::too_many_arguments)]
    fn number(&self, p: &mut Paint, text: &str, a: Area, baseline: f32, size: f32, c: [u8; 4], align: f32) {
        let f = self.font(Role::Display);
        let size = self.size_for(Role::Display, size);
        let cell = (0..10).map(|d| measure(&f, size, &d.to_string(), 0.)).fold(0., f32::max);
        let advance = |ch: char| if ch.is_ascii_digit() { cell } else { measure(&f, size, &ch.to_string(), 0.) };
        let total: f32 = text.chars().map(advance).sum::<f32>() + self.track(Role::Display) * size * text.chars().count().saturating_sub(1) as f32;
        let scale = if total > a.w { a.w / total } else { 1. };
        let mut pen = a.x + (a.w - total * scale) * align;
        for ch in text.chars() {
            let w = advance(ch) * scale;
            let glyph = measure(&f, size * scale, &ch.to_string(), 0.);
            self.text_fx(p, Role::Display, &ch.to_string(), pen + (w - glyph) / 2., baseline, size * scale / self.size_for(Role::Display, 1.), c, w + 4., Fx::Auto);
            pen += w + self.track(Role::Display) * size * scale;
        }
    }
    /// The theme's container. Returns the area inside it, below the title (if any).
    /// `field` asks Windows themes for a white list box, as their chat windows had.
    fn panel(&self, p: &mut Paint, title: &str, field: bool) -> Area {
        let opacity = self.bg[3] as f32 / 255.;
        if opacity >= 0.995 {
            return self.panel_inner(p, title, field);
        }
        // A see-through background fades the whole frame (card, shadow, border, texture, window
        // chrome) evenly, and keeps its text crisp.
        let blank = || Paint { pixels: vec![0; p.pixels.len()], width: p.width, height: p.height };
        let solid = Look {
            theme: self.theme,
            fg: self.fg,
            bg: [self.bg[0], self.bg[1], self.bg[2], 255],
            accent: self.accent,
            radius: self.radius,
            mono: self.mono,
            bold: self.bold,
            sec: self.sec,
            ter: self.ter,
            surface: self.surface,
            divider: self.divider,
            field_fg: self.field_fg,
            light: self.light,
            sink: std::cell::RefCell::new(Some(blank())),
            title_size: self.title_size,
        };
        let mut frame = blank();
        let area = solid.panel_inner(&mut frame, title, field);
        let text = solid.sink.take().unwrap_or_else(blank);
        for (layer, gain) in [(&frame, opacity), (&text, 1.)] {
            for (i, px) in layer.pixels.chunks_exact(4).enumerate() {
                if px[3] > 0 && gain > 0. {
                    p.blend((i as u32 % p.width) as i32, (i as u32 / p.width) as i32, [px[0], px[1], px[2], px[3]], gain);
                }
            }
        }
        area
    }
    fn panel_inner(&self, p: &mut Paint, title: &str, field: bool) -> Area {
        let (w, h) = (p.width as f32, p.height as f32);
        let r = self.radius;
        let label = self.title_size.unwrap_or((h * 0.05).clamp(12., 20.));
        match self.theme {
            Theme::Windows98 => {
                let a = Area { x: 2., y: 2., w: w - 4., h: h - 4. };
                p.fill_rr(a.x, a.y, a.w, a.h, 0., self.bg, self.bg);
                p.bevel(a.x, a.y, a.w, a.h, true);
                let bar = (label * 1.6).max(26.);
                p.fill_rr_across(a.x + 4., a.y + 4., a.w - 8., bar, 0., [0, 0, 128, 255], [16, 132, 208, 255]);
                let title = if title.is_empty() { "Rosorah" } else { title };
                self.text_fx(p, Role::Strong, title, a.x + 12., a.y + 4. + bar * 0.7, label, [255; 4], a.w - 110., Fx::Plain);
                for (i, glyph) in ["_", "□", "×"].iter().enumerate() {
                    let bh = bar - 8.;
                    let bx = a.right() - 6. - (3 - i) as f32 * (bh + 6.);
                    p.fill_rr(bx, a.y + 8., bh + 4., bh, 0., [192, 192, 192, 255], [192, 192, 192, 255]);
                    p.bevel(bx, a.y + 8., bh + 4., bh, true);
                    self.text_fx(p, Role::Strong, glyph, bx + bh * 0.3, a.y + 8. + bh * 0.78, bh * 0.75, [0, 0, 0, 255], bh, Fx::Plain);
                }
                let inner = a.inset(8., bar + 12., 8., 8.);
                if field {
                    p.fill_rr(inner.x, inner.y, inner.w, inner.h, 0., [255; 4], [255; 4]);
                    p.bevel(inner.x, inner.y, inner.w, inner.h, false);
                    return inner.inset(10., 8., 10., 8.);
                }
                inner.inset(6., 4., 6., 4.)
            }
            Theme::WindowsXP => {
                let a = Area { x: 1., y: 1., w: w - 2., h: h - 2. };
                let blue = [0, 84, 227, 255];
                p.shadow_rr(a.x, a.y + 3., a.w, a.h, 9., 8., [0, 0, 0, 90]);
                p.fill_rr(a.x, a.y, a.w, a.h, 9., blue, darken(blue, 0.15));
                let bar = (label * 1.8).max(30.);
                p.fill_rows(a.x, a.y, a.w, a.h, 9., (a.y, a.y + bar * 0.35), [61, 149, 255, 255], [0, 88, 238, 255]);
                p.fill_rows(a.x, a.y, a.w, a.h, 9., (a.y + bar * 0.35, a.y + bar), [0, 88, 238, 255], [0, 70, 213, 255]);
                p.hline(a.x + 8., a.y + 1., a.w - 16., [130, 190, 255, 200]);
                let title = if title.is_empty() { "Rosorah" } else { title };
                self.text_fx(p, Role::Title, title, a.x + 13., a.y + bar * 0.68 + 1., label, [10, 30, 110, 200], a.w - 130., Fx::Plain);
                self.text_fx(p, Role::Title, title, a.x + 12., a.y + bar * 0.68, label, [255; 4], a.w - 130., Fx::Plain);
                let bs = bar - 9.;
                for i in 0..3 {
                    let bx = a.right() - 8. - (3 - i) as f32 * (bs + 3.);
                    let (top, bottom) = if i == 2 { ([235, 120, 90, 255], [200, 60, 30, 255]) } else { ([70, 150, 255, 255], [20, 90, 230, 255]) };
                    p.fill_rr(bx, a.y + 5., bs, bs, 4., top, bottom);
                    p.stroke_rr(bx, a.y + 5., bs, bs, 4., 1.2, [255, 255, 255, 230]);
                    let (cx, cy) = (bx + bs / 2., a.y + 5. + bs / 2.);
                    match i {
                        0 => p.fill_rr(cx - 4., cy + 3., 8., 2., 0., [255; 4], [255; 4]),
                        1 => p.stroke_rr(cx - 5., cy - 5., 10., 10., 0., 2., [255; 4]),
                        _ => {
                            for k in 0..9 {
                                let o = k as f32 - 4.;
                                p.fill_rr(cx + o - 1., cy + o - 1., 2.4, 2.4, 1., [255; 4], [255; 4]);
                                p.fill_rr(cx - o - 1., cy + o - 1., 2.4, 2.4, 1., [255; 4], [255; 4]);
                            }
                        }
                    }
                }
                let body = a.inset(4., bar, 4., 4.);
                p.fill_rr(body.x, body.y, body.w, body.h, 0., lighten(self.bg, 0.02), self.bg);
                if field {
                    let f = body.inset(8., 8., 8., 8.);
                    p.fill_rr(f.x, f.y, f.w, f.h, 0., [255; 4], [255; 4]);
                    p.stroke_rr(f.x, f.y, f.w, f.h, 0., 1., [127, 157, 185, 255]);
                    return f.inset(10., 8., 10., 8.);
                }
                body.inset(12., 10., 12., 10.)
            }
            Theme::Terminal => {
                let a = Area { x: 8., y: 8., w: w - 16., h: h - 16. };
                let r = r.max(8.);
                p.shadow_rr(a.x, a.y + 6., a.w, a.h, r, 18., [0, 0, 0, 150]);
                p.fill_rr(a.x, a.y, a.w, a.h, r, lighten(self.bg, 0.03), darken(self.bg, 0.2));
                // Scanlines and a faint phosphor vignette.
                let (x0, y0, x1, y1) = p.bounds(a.x, a.y, a.right(), a.bottom());
                for py in (y0..y1).step_by(3) {
                    for px in x0..x1 {
                        if rr_distance(px as f32 + 0.5, py as f32 + 0.5, a.x, a.y, a.w, a.h, r) < -1. {
                            p.blend(px, py, [0, 0, 0, 255], 0.22);
                        }
                    }
                }
                p.stroke_rr(a.x, a.y, a.w, a.h, r, 1., fade(self.fg, 0.35));
                let bar = (label * 1.8).max(30.);
                p.hline(a.x + 1., a.y + bar, a.w - 2., fade(self.fg, 0.22));
                for i in 0..3 {
                    p.circle(a.x + 18. + i as f32 * 16., a.y + bar / 2., 4.5, fade(self.fg, 0.35 + 0.2 * i as f32));
                }
                let title = format!("rosorah@stream:~$ {}", if title.is_empty() { "widget" } else { title }.to_lowercase());
                let ta = Area { x: a.x + 70., y: a.y, w: a.w - 140., h: bar };
                self.text_in(p, Role::Caption, &title, ta, a.y + bar * 0.64, label * 0.85, self.sec, 0.5);
                a.inset(18., bar + 12., 18., 14.)
            }
            Theme::Led => {
                let a = Area { x: 6., y: 6., w: w - 12., h: h - 12. };
                let r = r.max(10.);
                p.shadow_rr(a.x, a.y + 5., a.w, a.h, r, 14., [0, 0, 0, 160]);
                p.fill_rr(a.x, a.y, a.w, a.h, r, [72, 74, 80, 255], [24, 25, 28, 255]);
                p.stroke_rr(a.x, a.y, a.w, a.h, r, 1., [255, 255, 255, 40]);
                let inner = a.inset(8., 8., 8., 8.);
                p.fill_rr(inner.x, inner.y, inner.w, inner.h, (r - 6.).max(3.), [4, 6, 5, 255], [2, 3, 2, 255]);
                // Unlit LEDs across the whole display.
                let pitch = 6.;
                let mut y = inner.y + 3.;
                while y < inner.bottom() - 2. {
                    let mut x = inner.x + 3.;
                    while x < inner.right() - 2. {
                        p.circle(x, y, 1.3, fade(self.fg, 0.07));
                        x += pitch;
                    }
                    y += pitch;
                }
                let content = inner.inset(16., 12., 16., 12.);
                if !title.is_empty() {
                    self.text(p, Role::Label, title, content.x, content.y + label + 2., label, fade(self.accent, 0.8), content.w);
                    return content.inset(0., label + 14., 0., 0.);
                }
                content
            }
            Theme::Pixel => {
                let u = 4.;
                let a = Area { x: 6., y: if title.is_empty() { 6. } else { 22. }, w: w - 12., h: h - if title.is_empty() { 12. } else { 28. } };
                // Stepped corners: three nested rectangles with notched ends.
                let outline = [0, 0, 0, 200];
                let light = lighten(self.fg, 0.1);
                let notch = |p: &mut Paint, inset: f32, c: [u8; 4]| {
                    let b = a.inset(inset, inset, inset, inset);
                    p.fill_rr(b.x + u, b.y, b.w - 2. * u, b.h, 0., c, c);
                    p.fill_rr(b.x, b.y + u, b.w, b.h - 2. * u, 0., c, c);
                };
                p.fill_rr(a.x + u, a.y + 2. * u, a.w, a.h, 0., [0, 0, 0, 90], [0, 0, 0, 90]);
                notch(p, 0., outline);
                notch(p, u, light);
                notch(p, 2. * u, darken(self.bg, 0.35));
                notch(p, 3. * u, self.bg);
                // A light dither, like a 16-bit dialogue box.
                let inner = a.inset(3. * u + 2., 3. * u + 2., 3. * u + 2., 3. * u + 2.);
                let mut yy = inner.y;
                while yy < inner.bottom() {
                    let mut xx = inner.x + if ((yy - inner.y) / 2.) as i32 % 2 == 0 { 0. } else { 2. };
                    while xx < inner.right() {
                        p.fill_rr(xx, yy, 2., 2., 0., [255, 255, 255, 6], [255, 255, 255, 6]);
                        xx += 4.;
                    }
                    yy += 2.;
                }
                if !title.is_empty() {
                    let tw = self.width(Role::Label, title, label) + 28.;
                    let tab = Area { x: a.x + 18., y: 2., w: tw.min(a.w - 40.), h: label + 22. };
                    p.fill_rr(tab.x + u, tab.y + u, tab.w, tab.h, 0., [0, 0, 0, 90], [0, 0, 0, 90]);
                    p.fill_rr(tab.x, tab.y, tab.w, tab.h, 0., outline, outline);
                    p.fill_rr(tab.x + u, tab.y + u, tab.w - 2. * u, tab.h - 2. * u, 0., light, light);
                    p.fill_rr(tab.x + 2. * u, tab.y + 2. * u, tab.w - 4. * u, tab.h - 4. * u, 0., darken(self.accent, 0.35), darken(self.accent, 0.5));
                    self.text_in(p, Role::Label, title, tab, tab.y + tab.h / 2. + label * 0.4, label, [255, 248, 220, 255], 0.5);
                }
                a.inset(3. * u + 16., 3. * u + if title.is_empty() { 14. } else { 22. }, 3. * u + 16., 3. * u + 14.)
            }
            Theme::Broadcast => {
                let a = Area { x: 12., y: 12., w: w - 24., h: h - 24. };
                let r = r.min(6.);
                p.shadow_rr(a.x, a.y + 8., a.w, a.h, r, 22., [0, 0, 0, 140]);
                p.fill_rr(a.x, a.y, a.w, a.h, r, lighten(self.bg, 0.1), darken(self.bg, 0.25));
                p.fill_rows(a.x, a.y, a.w, a.h, r, (a.y, a.y + a.h * 0.42), [255, 255, 255, 22], [255, 255, 255, 4]);
                p.fill_rows(a.x, a.y, a.w, a.h, r, (a.bottom() - 4., a.bottom()), self.accent, self.accent);
                p.stroke_rr(a.x, a.y, a.w, a.h, r, 1., [255, 255, 255, 26]);
                if title.is_empty() {
                    return a.inset(22., 18., 22., 20.);
                }
                // An angled tab with the title, like a TV score bug.
                let bar = (label * 2.2).max(30.);
                let tw = self.width(Role::Title, title, label * 1.25) + 40.;
                let slant = bar * 0.45;
                for row in 0..bar as i32 {
                    let t = row as f32 / bar;
                    let extra = slant * (1. - t);
                    p.fill_rr(a.x, a.y + row as f32, tw + extra, 1., 0., mix(lighten(self.accent, 0.12), darken(self.accent, 0.2), t), mix(lighten(self.accent, 0.12), darken(self.accent, 0.2), t));
                }
                p.fill_rows(a.x, a.y, a.w, a.h, r, (a.y, a.y + 2.), self.accent, self.accent);
                self.text_fx(p, Role::Title, title, a.x + 18., a.y + bar * 0.5 + label * 0.45, label * 1.25, on_color(self.accent), tw, Fx::Plain);
                a.inset(22., bar + 14., 22., 20.)
            }
            Theme::Team => {
                let a = Area { x: 12., y: 12., w: w - 24., h: h - 24. };
                p.shadow_rr(a.x, a.y + 8., a.w, a.h, r, 22., [0, 0, 0, 130]);
                p.fill_rr(a.x, a.y, a.w, a.h, r, lighten(self.bg, 0.08), darken(self.bg, 0.2));
                let bar = if title.is_empty() { 8. } else { (label * 2.3).max(32.) };
                p.fill_rows(a.x, a.y, a.w, a.h, r, (a.y, a.y + bar), lighten(self.accent, 0.08), darken(self.accent, 0.12));
                // Diagonal stripes across the header.
                let (x0, y0, x1, y1) = p.bounds(a.x, a.y, a.right(), a.y + bar);
                for py in y0..y1 {
                    for px in x0..x1 {
                        if (px + py) % 14 < 5 && rr_distance(px as f32 + 0.5, py as f32 + 0.5, a.x, a.y, a.w, a.h, r) < 0. {
                            p.blend(px, py, [255, 255, 255, 255], 0.09);
                        }
                    }
                }
                p.hline(a.x, a.y + bar, a.w, [0, 0, 0, 90]);
                p.stroke_rr(a.x, a.y, a.w, a.h, r, 1., [255, 255, 255, 30]);
                if !title.is_empty() {
                    self.text_fx(p, Role::Title, title, a.x + 20., a.y + bar * 0.5 + label * 0.5, label * 1.3, on_color(self.accent), a.w - 40., Fx::Plain);
                }
                a.inset(20., bar + 14., 20., 18.)
            }
            Theme::CozyCafe | Theme::Kawaii | Theme::Cottagecore | Theme::CleanLight | Theme::Winter | Theme::Spooky => {
                let a = Area { x: 14., y: 12., w: w - 28., h: h - 30. };
                let (top, bottom) = match self.theme {
                    Theme::Kawaii => (lighten(self.bg, 0.4), mix(self.bg, [228, 216, 255, 255], 0.7)),
                    Theme::Winter => (lighten(self.bg, 0.12), darken(self.bg, 0.25)),
                    Theme::Spooky => (lighten(self.bg, 0.07), darken(self.bg, 0.35)),
                    _ => (lighten(self.bg, 0.05), darken(self.bg, 0.04)),
                };
                let shadow = match self.theme {
                    Theme::CleanLight => [20, 24, 40, 42],
                    Theme::CozyCafe => [90, 50, 20, 70],
                    Theme::Kawaii => [255, 110, 175, 80],
                    Theme::Cottagecore => [40, 50, 30, 60],
                    Theme::Spooky => fade(self.accent, 0.4),
                    _ => [0, 0, 0, 130],
                };
                p.shadow_rr(a.x, a.y + 8., a.w, a.h, r, 24., shadow);
                p.fill_rr(a.x, a.y, a.w, a.h, r, top, bottom);
                let (x0, y0, x1, y1) = p.bounds(a.x, a.y, a.right(), a.bottom());
                let inside = |px: i32, py: i32| rr_distance(px as f32 + 0.5, py as f32 + 0.5, a.x, a.y, a.w, a.h, r);
                match self.theme {
                    Theme::CozyCafe => {
                        // Paper grain, then a stitched border.
                        for py in y0..y1 {
                            for px in x0..x1 {
                                let d = inside(px, py);
                                if d < -1. {
                                    p.blend(px, py, [90, 60, 30, 255], 0.05 * hash(px, py));
                                }
                                let ring = (d + 7.).abs();
                                if ring < 0.9 && ((px + py) / 6) % 2 == 0 {
                                    p.blend(px, py, fade(self.accent, 0.6), 1. - ring);
                                }
                            }
                        }
                    }
                    Theme::Kawaii => {
                        p.stroke_rr(a.x, a.y, a.w, a.h, r, 3., [255, 255, 255, 235]);
                        for (dx, dy, size, c) in [(-28., 22., 8., [255; 4]), (-50., 40., 4.5, lighten(self.accent, 0.3)), (-18., 48., 3., [255; 4])] {
                            p.sparkle(a.right() + dx, a.y + dy, size, c);
                        }
                        p.sparkle(a.x + 22., a.bottom() - 22., 6., [255, 255, 255, 220]);
                        p.sparkle(a.x + 38., a.bottom() - 14., 3., lighten(self.accent, 0.3));
                    }
                    Theme::Cottagecore => {
                        for py in y0..y1 {
                            for px in x0..x1 {
                                if (px % 4 == 0 || py % 4 == 0) && inside(px, py) < -1. {
                                    p.blend(px, py, [60, 50, 20, 255], 0.025);
                                }
                            }
                        }
                        let inner = a.inset(6., 6., 6., 6.);
                        p.stroke_rr(inner.x, inner.y, inner.w, inner.h, (r - 6.).max(2.), 1.2, fade(self.accent, 0.85));
                        let inner = a.inset(10., 10., 10., 10.);
                        p.stroke_rr(inner.x, inner.y, inner.w, inner.h, (r - 10.).max(2.), 0.8, fade(self.accent, 0.5));
                        sprig(p, a.right() - 20., a.y + 18., 1., self.accent);
                        sprig(p, a.x + 20., a.bottom() - 18., -1., self.accent);
                    }
                    Theme::CleanLight => p.stroke_rr(a.x, a.y, a.w, a.h, r, 1., [0, 0, 0, 18]),
                    Theme::Winter => {
                        // Snowflakes, a string of lights and snow along the top.
                        for i in 0..60 {
                            let (fx, fy) = (hash(i, 7), hash(i, 13));
                            let (cx, cy) = (a.x + 10. + fx * (a.w - 20.), a.y + 26. + fy * (a.h - 36.));
                            p.circle(cx, cy, 0.8 + hash(i, 3) * 1.8, [255, 255, 255, (60. + hash(i, 5) * 120.) as u8]);
                        }
                        let wire = a.y + 22.;
                        let colors = [[255, 70, 80, 255], [80, 220, 120, 255], [255, 210, 80, 255], [90, 170, 255, 255]];
                        let mut k = 0;
                        let mut x = a.x + r.max(14.);
                        while x < a.right() - r.max(14.) {
                            let sag = ((x - a.x) / 70. * std::f32::consts::PI).sin().abs() * 6.;
                            p.circle(x, wire + sag, 1., [255, 255, 255, 70]);
                            if ((x - a.x) as i32 / 2) % 17 == 0 {
                                let c = colors[k % 4];
                                p.shadow_rr(x - 4., wire + sag + 2., 8., 10., 4., 8., fade(c, 0.6));
                                p.fill_rr(x - 3., wire + sag + 2., 6., 9., 3., lighten(c, 0.3), c);
                                k += 1;
                            }
                            x += 2.;
                        }
                        p.fill_rows(a.x, a.y, a.w, a.h, r, (a.y, a.y + 6.), [255; 4], [235, 242, 255, 255]);
                        let mut x = a.x + 4.;
                        let mut i = 0;
                        while x < a.right() - 4. {
                            let rad = 5. + hash(i, 1) * 6.;
                            p.fill_rows(x - rad, a.y + 2. - rad, rad * 2., rad * 2., rad, (a.y - 12., a.y + 9. + hash(i, 2) * 4.), [255; 4], [230, 238, 252, 255]);
                            x += rad * 1.3;
                            i += 1;
                        }
                    }
                    _ => {
                        // Spooky: a cobweb in the corner and drips along the top.
                        let web = [255, 255, 255, 70];
                        let (cx, cy) = (a.x + 4., a.y + 4.);
                        for k in 0..5 {
                            let t = 0.25 + k as f32 * 0.0625;
                            let ang = t * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
                            for step in 0..70 {
                                p.circle(cx + ang.cos() * step as f32, cy + ang.sin() * step as f32, 0.5, web);
                            }
                        }
                        for rad in [22., 40., 58.] {
                            p.arc(cx, cy, rad, 1., 0.25, 0.5, web, web);
                        }
                        for i in 0..7 {
                            let x = a.x + a.w * (0.18 + 0.12 * i as f32) + hash(i, 9) * 20.;
                            let len = 8. + hash(i, 4) * 22.;
                            p.fill_rr(x, a.y, 7., len, 3.5, self.accent, darken(self.accent, 0.2));
                            p.circle(x + 3.5, a.y + len, 5., darken(self.accent, 0.2));
                        }
                        p.fill_rows(a.x, a.y, a.w, a.h, r, (a.y, a.y + 4.), self.accent, self.accent);
                        p.stroke_rr(a.x, a.y, a.w, a.h, r, 1., fade(self.accent, 0.45));
                    }
                }
                let pad = (r * 0.55).clamp(18., 26.);
                let top_pad = match self.theme {
                    Theme::Winter => 34.,
                    Theme::Spooky => 26.,
                    _ => pad - 2.,
                };
                let content = a.inset(pad, top_pad, pad, pad - 4.);
                if title.is_empty() {
                    return content;
                }
                let label_c = match self.theme {
                    Theme::CozyCafe | Theme::Kawaii | Theme::Cottagecore | Theme::Spooky => self.ink(self.accent),
                    _ => self.sec,
                };
                let lx = if self.theme == Theme::CleanLight {
                    p.circle(content.x + 4., content.y + label * 0.62, 3.5, self.accent);
                    content.x + 14.
                } else {
                    content.x
                };
                self.text(p, Role::Label, title, lx, content.y + label, label, label_c, content.w - 14.);
                content.inset(0., label + if self.theme == Theme::CozyCafe { 22. } else { 14. }, 0., 0.)
            }
            Theme::Cyberpunk => {
                let a = Area { x: 14., y: 12., w: w - 28., h: h - 28. };
                let cut = (a.h * 0.12).clamp(12., 28.);
                let d = |px: f32, py: f32| chamfer_distance(px, py, a.x, a.y, a.w, a.h, cut);
                p.glow_shape(a.x, a.y, a.w, a.h, d, 18., fade(self.accent, 0.5));
                p.fill_shape(a.x, a.y, a.w, a.h, d, lighten(self.bg, 0.06), darken(self.bg, 0.15));
                let (x0, y0, x1, y1) = p.bounds(a.x, a.y, a.right(), a.bottom());
                for py in (y0..y1).step_by(4) {
                    for px in x0..x1 {
                        if d(px as f32 + 0.5, py as f32 + 0.5) < -1. {
                            p.blend(px, py, [0, 0, 0, 255], 0.14);
                        }
                    }
                }
                p.stroke_shape(a.x, a.y, a.w, a.h, d, 1.6, self.accent);
                let cyan = [0, 240, 255, 255];
                p.fill_rr(a.right() - 54., a.y - 2., 54., 3., 0., cyan, cyan);
                p.fill_rr(a.right() - 3., a.y - 2., 3., 34., 0., cyan, cyan);
                p.fill_rr(a.x, a.bottom() - 1., 54., 3., 0., cyan, cyan);
                p.fill_rr(a.x, a.bottom() - 32., 3., 33., 0., cyan, cyan);
                let content = a.inset(cut + 8., 16., 22., 16.);
                if title.is_empty() {
                    return content;
                }
                self.text(p, Role::Label, title, content.x, content.y + label, label, cyan, content.w);
                p.fill_rr(content.x, content.y + label + 7., 36., 2., 0., self.accent, self.accent);
                content.inset(0., label + 20., 0., 0.)
            }
            Theme::Synthwave => {
                let a = Area { x: 14., y: 12., w: w - 28., h: h - 30. };
                p.shadow_rr(a.x, a.y + 6., a.w, a.h, r, 24., fade(self.accent, 0.45));
                p.fill_rr(a.x, a.y, a.w, a.h, r, lighten(self.bg, 0.04), mix(self.bg, self.accent, 0.38));
                let inside = |x: f32, y: f32| rr_distance(x, y, a.x, a.y, a.w, a.h, r) < -1.;
                // A striped sun sinking behind a neon grid.
                let horizon = a.y + a.h * 0.64;
                let (sx, sr) = (a.right() - a.h * 0.55, a.h * 0.3);
                let (x0, y0, x1, y1) = p.bounds(sx - sr, horizon - sr, sx + sr, horizon);
                for py in y0..y1 {
                    let band = horizon - py as f32;
                    if band < sr * 0.45 && ((band / 5.) as i32) % 2 == 0 {
                        continue;
                    }
                    let c = mix([255, 220, 100, 255], [255, 70, 150, 255], (py as f32 - (horizon - sr)) / sr);
                    for px in x0..x1 {
                        let d = (px as f32 + 0.5 - sx).hypot(py as f32 + 0.5 - horizon) - sr;
                        if d < 0. && inside(px as f32, py as f32) {
                            p.blend(px, py, c, (0.5 - d).clamp(0., 1.) * 0.4);
                        }
                    }
                }
                let grid = fade(self.accent, 0.55);
                p.hline(a.x + 2., horizon, a.w - 4., fade(self.accent, 0.9));
                let depth = a.bottom() - horizon;
                for k in 1..9 {
                    let y = horizon + depth * (k as f32 / 8.).powi(2);
                    let (x0, _, x1, _) = p.bounds(a.x, y, a.right(), y + 1.);
                    for px in x0..x1 {
                        if inside(px as f32, y) {
                            p.blend(px, y as i32, grid, 1.);
                        }
                    }
                }
                let vp = a.x + a.w / 2.;
                for i in -14..=14 {
                    let bx = vp + i as f32 * a.w / 9.;
                    for py in horizon as i32..a.bottom() as i32 {
                        let t = (py as f32 - horizon) / depth;
                        let x = vp + (bx - vp) * t;
                        if inside(x, py as f32) {
                            p.blend(x as i32, py, grid, 1. - x.fract());
                            p.blend(x as i32 + 1, py, grid, x.fract());
                        }
                    }
                }
                p.stroke_rr(a.x, a.y, a.w, a.h, r, 1.2, fade(self.accent, 0.7));
                let pad = (r * 0.55).clamp(16., 24.);
                let content = a.inset(pad, pad - 2., pad, pad - 4.);
                if title.is_empty() {
                    return content;
                }
                self.text(p, Role::Label, title, content.x, content.y + label, label, lighten(self.accent, 0.3), content.w);
                content.inset(0., label + 14., 0., 0.)
            }
            Theme::SciFiHud => {
                let a = Area { x: 10., y: 10., w: w - 20., h: h - 20. };
                p.fill_rr(a.x, a.y, a.w, a.h, 0., self.bg, fade(self.bg, 0.7));
                p.stroke_rr(a.x, a.y, a.w, a.h, 0., 1., fade(self.accent, 0.3));
                let l = (a.h * 0.12).clamp(14., 30.);
                let t = 2.5;
                for (cx, cy, sx, sy) in [(a.x, a.y, 1., 1.), (a.right(), a.y, -1., 1.), (a.x, a.bottom(), 1., -1.), (a.right(), a.bottom(), -1., -1.)] {
                    let hx = if sx > 0. { cx } else { cx - l };
                    let vy = if sy > 0. { cy } else { cy - l };
                    p.fill_rr(hx, if sy > 0. { cy } else { cy - t }, l, t, 0., self.accent, self.accent);
                    p.fill_rr(if sx > 0. { cx } else { cx - t }, vy, t, l, 0., self.accent, self.accent);
                }
                let mut x = a.x + l * 3.;
                while x < a.right() - l * 3. {
                    p.fill_rr(x, a.y + 1., 1., 6., 0., fade(self.accent, 0.45), fade(self.accent, 0.45));
                    x += 18.;
                }
                let content = a.inset(22., 18., 22., 18.);
                if title.is_empty() {
                    return content;
                }
                self.text(p, Role::Label, &format!("// {title}"), content.x, content.y + label, label, self.accent, content.w);
                content.inset(0., label + 16., 0., 0.)
            }
            Theme::Arcade => {
                let a = Area { x: 8., y: if title.is_empty() { 8. } else { 18. }, w: w - 16., h: h - if title.is_empty() { 16. } else { 26. } };
                let r = r.max(8.);
                p.shadow_rr(a.x, a.y + 6., a.w, a.h, r, 16., [0, 0, 0, 160]);
                p.fill_rr(a.x, a.y, a.w, a.h, r, [70, 26, 140, 255], [34, 10, 80, 255]);
                let inner = a.inset(16., 16., 16., 16.);
                p.fill_rr(inner.x, inner.y, inner.w, inner.h, r * 0.6, lighten(self.bg, 0.05), darken(self.bg, 0.3));
                p.stroke_rr(inner.x, inner.y, inner.w, inner.h, r * 0.6, 1.5, [255, 255, 255, 40]);
                // Marquee bulbs around the frame.
                let ring = a.inset(8., 8., 8., 8.);
                let step = 18.;
                let mut k = 0;
                let mut place = |p: &mut Paint, x: f32, y: f32| {
                    let c = if k % 2 == 0 { self.accent } else { [255, 255, 255, 255] };
                    p.circle(x, y, 6., fade(c, 0.25));
                    p.circle(x, y, 3.2, c);
                    k += 1;
                };
                let mut x = ring.x;
                while x <= ring.right() {
                    place(p, x, ring.y);
                    place(p, x, ring.bottom());
                    x += step;
                }
                let mut y = ring.y + step;
                while y < ring.bottom() {
                    place(p, ring.x, y);
                    place(p, ring.right(), y);
                    y += step;
                }
                if !title.is_empty() {
                    let tw = (self.width(Role::Label, title, label) + 40.).min(a.w - 60.);
                    let banner = Area { x: a.x + (a.w - tw) / 2., y: 4., w: tw, h: label * 1.1 + 20. };
                    p.shadow_rr(banner.x, banner.y + 3., banner.w, banner.h, 6., 6., [0, 0, 0, 140]);
                    p.fill_rr(banner.x, banner.y, banner.w, banner.h, 6., [240, 60, 60, 255], [170, 20, 30, 255]);
                    p.stroke_rr(banner.x, banner.y, banner.w, banner.h, 6., 2., [255; 4]);
                    self.text_in(p, Role::Label, title, banner, banner.y + banner.h / 2. + label * 0.3, label, [255; 4], 0.5);
                }
                inner.inset(18., if title.is_empty() { 14. } else { 22. }, 18., 14.)
            }
            Theme::Fantasy => {
                let a = Area { x: 12., y: 12., w: w - 24., h: h - 24. };
                p.shadow_rr(a.x, a.y + 6., a.w, a.h, r, 18., [40, 20, 0, 140]);
                p.fill_rr(a.x, a.y, a.w, a.h, r, lighten(self.bg, 0.05), darken(self.bg, 0.06));
                let (x0, y0, x1, y1) = p.bounds(a.x, a.y, a.right(), a.bottom());
                for py in y0..y1 {
                    for px in x0..x1 {
                        let d = rr_distance(px as f32 + 0.5, py as f32 + 0.5, a.x, a.y, a.w, a.h, r);
                        if d >= 0. {
                            continue;
                        }
                        let n = noise(px as f32, py as f32, 42.) * 0.65 + noise(px as f32, py as f32, 9.) * 0.35;
                        p.blend(px, py, [120, 80, 30, 255], ((n - 0.45).max(0.) * 0.35).min(0.2));
                        if d > -22. {
                            let burn = ((22. + d) / 22.).powi(2);
                            p.blend(px, py, [96, 52, 18, 255], burn * 0.45);
                        }
                    }
                }
                let gold = [196, 152, 60, 255];
                let b = a.inset(9., 9., 9., 9.);
                p.stroke_rr(b.x, b.y, b.w, b.h, 2., 2.2, gold);
                let b2 = a.inset(14., 14., 14., 14.);
                p.stroke_rr(b2.x, b2.y, b2.w, b2.h, 1., 1., fade(gold, 0.7));
                for (cx, cy) in [(b.x, b.y), (b.right(), b.y), (b.x, b.bottom()), (b.right(), b.bottom()), (b.x + b.w / 2., b.y)] {
                    let diamond = |px: f32, py: f32| ((px - cx).abs() + (py - cy).abs() - 7.) * std::f32::consts::FRAC_1_SQRT_2;
                    p.fill_shape(cx - 8., cy - 8., 16., 16., diamond, lighten(gold, 0.3), darken(gold, 0.2));
                    p.stroke_shape(cx - 8., cy - 8., 16., 16., diamond, 1., [80, 50, 10, 255]);
                }
                let content = a.inset(30., 28., 30., 26.);
                if title.is_empty() {
                    return content;
                }
                let tw = self.width(Role::Label, title, label);
                let mid = content.x + content.w / 2.;
                self.text_in(p, Role::Label, title, content, content.y + label, label, self.accent, 0.5);
                for side in [-1., 1.] {
                    let x = mid + side * (tw / 2. + 14.);
                    let len = 40.;
                    p.fill_rr(if side < 0. { x - len } else { x }, content.y + label * 0.62, len, 1.5, 0., gold, gold);
                    p.circle(x + side * len, content.y + label * 0.62 + 0.7, 2.5, gold);
                }
                content.inset(0., label + 16., 0., 0.)
            }
            Theme::Neubrutalism => {
                let off = 8.;
                let a = Area { x: 8., y: 8., w: w - 16. - off, h: h - 16. - off };
                let black = [12, 12, 12, 255];
                p.fill_rr(a.x + off, a.y + off, a.w, a.h, r, black, black);
                p.fill_rr(a.x, a.y, a.w, a.h, r, self.bg, self.bg);
                p.stroke_rr(a.x, a.y, a.w, a.h, r, 3., black);
                let content = a.inset(22., 20., 22., 18.);
                if title.is_empty() {
                    return content;
                }
                let tw = self.width(Role::Label, title, label) + 24.;
                let tag = Area { x: content.x, y: content.y, w: tw, h: label + 16. };
                p.fill_rr(tag.x + 4., tag.y + 4., tag.w, tag.h, 4., black, black);
                p.fill_rr(tag.x, tag.y, tag.w, tag.h, 4., self.accent, self.accent);
                p.stroke_rr(tag.x, tag.y, tag.w, tag.h, 4., 2.5, black);
                self.text_fx(p, Role::Label, title, tag.x + 12., tag.y + tag.h / 2. + label * 0.36, label, black, tw, Fx::Plain);
                content.inset(0., tag.h + 16., 0., 0.)
            }
            Theme::GameBoy => {
                let a = Area { x: 6., y: 6., w: w - 12., h: h - 12. };
                p.shadow_rr(a.x, a.y + 5., a.w, a.h, 14., 12., [0, 0, 0, 120]);
                p.fill_rr(a.x, a.y, a.w, a.h, 14., [206, 206, 200, 255], [178, 178, 172, 255]);
                let slate = a.inset(12., 12., 12., 12.);
                p.fill_rr(slate.x, slate.y, slate.w, slate.h, 8., [92, 94, 114, 255], [72, 74, 92, 255]);
                p.fill_rr(slate.x + 14., slate.y + 9., slate.w * 0.3, 2., 0., [150, 40, 90, 255], [150, 40, 90, 255]);
                p.fill_rr(slate.x + 14., slate.y + 13., slate.w * 0.3, 2., 0., [40, 50, 130, 255], [40, 50, 130, 255]);
                let caption = Area { x: slate.x + slate.w * 0.3 + 24., y: slate.y, w: slate.w * 0.4, h: 22. };
                self.text_fx(p, Role::Caption, "ROSORAH · DOT MATRIX", caption.x, slate.y + 16., 10., [196, 196, 210, 255], caption.w, Fx::Plain);
                p.circle(slate.x + 14., slate.y + slate.h / 2., 4., [230, 30, 40, 255]);
                p.circle(slate.x + 14., slate.y + slate.h / 2., 7., [230, 30, 40, 60]);
                let screen = slate.inset(32., 24., 16., 12.);
                p.fill_rr(screen.x, screen.y, screen.w, screen.h, 2., lighten(self.bg, 0.04), darken(self.bg, 0.06));
                let (x0, y0, x1, y1) = p.bounds(screen.x, screen.y, screen.right(), screen.bottom());
                for py in y0..y1 {
                    for px in x0..x1 {
                        if px % 3 == 0 || py % 3 == 0 {
                            p.blend(px, py, [15, 56, 15, 255], 0.06);
                        }
                    }
                }
                let content = screen.inset(14., 12., 14., 10.);
                if title.is_empty() {
                    return content;
                }
                self.text(p, Role::Label, title, content.x, content.y + label * 0.8, label, self.fg, content.w);
                content.inset(0., label + 10., 0., 0.)
            }
            Theme::Vaporwave => {
                let a = Area { x: 2., y: 2., w: w - 4., h: h - 4. };
                p.fill_rr(a.x, a.y, a.w, a.h, 0., self.bg, self.bg);
                p.bevel(a.x, a.y, a.w, a.h, true);
                let bar = (label * 1.6).max(26.);
                p.fill_rr_across(a.x + 4., a.y + 4., a.w - 8., bar, 0., [255, 113, 206, 255], [1, 205, 254, 255]);
                let title = if title.is_empty() { "aesthetic.exe" } else { title };
                self.text_fx(p, Role::Strong, title, a.x + 13., a.y + 5. + bar * 0.7, label, [120, 30, 110, 160], a.w - 110., Fx::Plain);
                self.text_fx(p, Role::Strong, title, a.x + 12., a.y + 4. + bar * 0.7, label, [255; 4], a.w - 110., Fx::Plain);
                for (i, glyph) in ["_", "□", "×"].iter().enumerate() {
                    let bh = bar - 8.;
                    let bx = a.right() - 6. - (3 - i) as f32 * (bh + 6.);
                    p.fill_rr(bx, a.y + 8., bh + 4., bh, 0., [255, 214, 240, 255], [255, 214, 240, 255]);
                    p.bevel(bx, a.y + 8., bh + 4., bh, true);
                    self.text_fx(p, Role::Strong, glyph, bx + bh * 0.3, a.y + 8. + bh * 0.78, bh * 0.75, [60, 20, 70, 255], bh, Fx::Plain);
                }
                let inner = a.inset(8., bar + 12., 8., 8.);
                p.fill_rr(inner.x, inner.y, inner.w, inner.h, 0., [255, 206, 238, 255], [184, 232, 255, 255]);
                let (x0, y0, x1, y1) = p.bounds(inner.x, inner.y, inner.right(), inner.bottom());
                for py in y0..y1 {
                    for px in x0..x1 {
                        if (px - x0) % 18 == 0 || (py - y0) % 18 == 0 {
                            p.blend(px, py, [255, 255, 255, 255], 0.35);
                        }
                    }
                }
                if field {
                    let f = inner.inset(8., 8., 8., 8.);
                    p.fill_rr(f.x, f.y, f.w, f.h, 0., [255; 4], [255; 4]);
                    p.bevel(f.x, f.y, f.w, f.h, false);
                    return f.inset(10., 8., 10., 8.);
                }
                inner.inset(14., 10., 14., 10.)
            }
            Theme::Transparent => {
                let a = Area { x: 10., y: 10., w: w - 20., h: h - 20. };
                if title.is_empty() {
                    return a;
                }
                self.text(p, Role::Label, title, a.x, a.y + label, label, self.fg, a.w);
                p.fill_rr(a.x, a.y + label + 7., 28., 3., 1.5, self.accent, self.accent);
                a.inset(0., label + 20., 0., 0.)
            }
            _ => {
                // Material, Unified, Cards and Bubbles: an elevated glass card.
                let bubbles = self.theme == Theme::Bubbles;
                let bg = self.bg;
                let r = if bubbles { r.max(24.) } else { r };
                let a = Area { x: 14., y: 12., w: w - 28., h: h - 30. };
                p.shadow_rr(a.x, a.y + 10., a.w, a.h, r, 26., [0, 0, 0, 130]);
                p.fill_rr(a.x, a.y, a.w, a.h, r, lighten(bg, 0.07), darken(bg, 0.08));
                p.fill_rows(a.x, a.y, a.w, a.h, r, (a.y, a.y + a.h * 0.5), [255, 255, 255, 13], [255, 255, 255, 0]);
                match self.theme {
                    Theme::Unified => p.fill_rows(a.x, a.y, a.w, a.h, r, (a.y, a.y + 3.), [145, 70, 255, 255], [145, 70, 255, 255]),
                    Theme::Cards => p.fill_rows(a.x, a.y, a.w, a.h, r, (a.y, a.y + 4.), self.accent, self.accent),
                    _ => {}
                }
                p.stroke_rr(a.x, a.y, a.w, a.h, r, 1., [255, 255, 255, 26]);
                if self.theme == Theme::Unified {
                    // The top edge runs Twitch purple into YouTube red.
                    p.fill_rr_across(a.x + r * 0.6, a.y, a.w - r * 1.2, 3., 1.5, [145, 70, 255, 255], [255, 40, 60, 255]);
                }
                let pad = (r * 0.55).clamp(16., 24.);
                let content = a.inset(pad, pad - 2., pad, pad - 4.);
                if title.is_empty() {
                    return content;
                }
                p.circle(content.x + 4., content.y + label * 0.62, 3.5, self.accent);
                self.text(p, Role::Label, title, content.x + 14., content.y + label, label, self.sec, content.w - 14.);
                content.inset(0., label + 14., 0., 0.)
            }
        }
    }
    /// A progress bar in the theme's style; `t` is 0–1.
    fn bar(&self, p: &mut Paint, a: Area, t: f32) {
        let t = t.clamp(0., 1.);
        match self.theme {
            Theme::Windows98 => {
                p.fill_rr(a.x, a.y, a.w, a.h, 0., [255; 4], [255; 4]);
                p.bevel(a.x, a.y, a.w, a.h, false);
                let inner = a.inset(3., 3., 3., 3.);
                let block = (inner.h * 0.7).max(6.);
                let mut x = inner.x;
                while x + block <= inner.x + inner.w * t + 0.5 {
                    p.fill_rr(x, inner.y, block, inner.h, 0., [0, 0, 128, 255], [0, 0, 128, 255]);
                    x += block + 2.;
                }
            }
            Theme::WindowsXP => {
                p.fill_rr(a.x, a.y, a.w, a.h, 3., [255; 4], [235, 235, 235, 255]);
                p.stroke_rr(a.x, a.y, a.w, a.h, 3., 1., [104, 104, 104, 255]);
                let inner = a.inset(3., 3., 3., 3.);
                let block = (inner.h * 0.6).max(6.);
                let mut x = inner.x;
                while x + block <= inner.x + inner.w * t + 0.5 {
                    p.fill_rr(x, inner.y, block, inner.h, 1., [130, 230, 130, 255], [40, 180, 60, 255]);
                    x += block + 2.;
                }
            }
            Theme::Led => {
                let n = (a.w / (a.h * 0.9)).floor().max(4.) as usize;
                let step = a.w / n as f32;
                for i in 0..n {
                    let on = (i as f32 + 0.5) / n as f32 <= t;
                    let cx = a.x + step * (i as f32 + 0.5);
                    if on {
                        p.circle(cx, a.y + a.h / 2., a.h * 0.55, fade(self.fg, 0.25));
                    }
                    p.circle(cx, a.y + a.h / 2., a.h * 0.32, if on { lighten(self.fg, 0.3) } else { fade(self.fg, 0.12) });
                }
            }
            Theme::Neubrutalism => {
                let black = [12, 12, 12, 255];
                p.fill_rr(a.x + 4., a.y + 4., a.w, a.h, 4., black, black);
                p.fill_rr(a.x, a.y, a.w, a.h, 4., [255; 4], [255; 4]);
                if t > 0. {
                    p.fill_rr(a.x, a.y, a.w * t, a.h, 4., self.accent, self.accent);
                }
                p.stroke_rr(a.x, a.y, a.w, a.h, 4., 2.5, black);
            }
            Theme::Cyberpunk => {
                let cut = a.h * 0.6;
                let track = |px: f32, py: f32| chamfer_distance(px, py, a.x, a.y, a.w, a.h, cut);
                p.fill_shape(a.x, a.y, a.w, a.h, track, [255, 255, 255, 14], [255, 255, 255, 8]);
                p.stroke_shape(a.x, a.y, a.w, a.h, track, 1., fade(self.accent, 0.5));
                let fw = a.w * t;
                if fw > 2. {
                    let fill = |px: f32, py: f32| chamfer_distance(px, py, a.x, a.y, fw, a.h, cut.min(fw / 2.));
                    p.glow_shape(a.x, a.y, fw, a.h, fill, a.h, fade(self.accent, 0.5));
                    let (x0, y0, x1, y1) = p.bounds(a.x, a.y, a.x + fw, a.bottom());
                    for py in y0..y1 {
                        for px in x0..x1 {
                            let cov = (0.5 - fill(px as f32 + 0.5, py as f32 + 0.5)).clamp(0., 1.);
                            if cov > 0. {
                                p.blend(px, py, mix(self.accent, [0, 240, 255, 255], (px as f32 - a.x) / a.w), cov);
                            }
                        }
                    }
                }
            }
            Theme::SciFiHud => {
                p.stroke_rr(a.x, a.y, a.w, a.h, 0., 1., fade(self.accent, 0.5));
                let inner = a.inset(3., 3., 3., 3.);
                if t > 0. {
                    p.shadow_rr(inner.x, inner.y, inner.w * t, inner.h, 0., inner.h, fade(self.accent, 0.4));
                    p.fill_rr(inner.x, inner.y, inner.w * t, inner.h, 0., lighten(self.accent, 0.2), self.accent);
                }
                for k in 1..10 {
                    p.vline(a.x + a.w * k as f32 / 10., a.bottom() + 3., 5., fade(self.accent, 0.5));
                }
            }
            Theme::Fantasy => {
                let gold = [196, 152, 60, 255];
                p.fill_rr(a.x, a.y, a.w, a.h, a.h / 2., [80, 50, 20, 60], [80, 50, 20, 40]);
                if t > 0. {
                    p.fill_rr(a.x, a.y, (a.w * t).max(a.h), a.h, a.h / 2., lighten(self.accent, 0.2), darken(self.accent, 0.25));
                }
                p.stroke_rr(a.x, a.y, a.w, a.h, a.h / 2., 2., gold);
            }
            Theme::Arcade | Theme::Vaporwave | Theme::GameBoy => {
                p.fill_rr(a.x, a.y, a.w, a.h, 2., [0, 0, 0, 90], [0, 0, 0, 60]);
                p.stroke_rr(a.x, a.y, a.w, a.h, 2., 2., if self.theme == Theme::GameBoy { self.fg } else { [255, 255, 255, 120] });
                let inner = a.inset(4., 4., 4., 4.);
                let n = (inner.w / (inner.h * 0.9)).floor().max(4.) as usize;
                let step = inner.w / n as f32;
                for i in 0..((n as f32 * t).round() as usize).min(n) {
                    let pos = i as f32 / n as f32;
                    let c = match self.theme {
                        Theme::Arcade => if pos < 0.5 { mix([255, 60, 60, 255], [255, 214, 0, 255], pos * 2.) } else { mix([255, 214, 0, 255], [70, 220, 90, 255], (pos - 0.5) * 2.) },
                        Theme::Vaporwave => mix([255, 113, 206, 255], [1, 205, 254, 255], pos),
                        _ => self.fg,
                    };
                    p.fill_rr(inner.x + step * i as f32, inner.y, step - 2., inner.h, 0., lighten(c, 0.15), c);
                }
            }
            Theme::Winter | Theme::Kawaii => {
                let r = a.h / 2.;
                p.fill_rr(a.x, a.y, a.w, a.h, r, darken(self.surface, 0.1), self.surface);
                let fw = (a.w * t).max(if t > 0. { a.h } else { 0. });
                if fw > 0. {
                    let (stripe, base) = if self.theme == Theme::Winter { ([255; 4], self.accent) } else { (lighten(self.accent, 0.5), self.accent) };
                    p.shadow_rr(a.x, a.y, fw, a.h, r, a.h * 0.8, fade(base, 0.4));
                    let (x0, y0, x1, y1) = p.bounds(a.x, a.y, a.x + fw, a.bottom());
                    for py in y0..y1 {
                        for px in x0..x1 {
                            let cov = (0.5 - rr_distance(px as f32 + 0.5, py as f32 + 0.5, a.x, a.y, fw, a.h, r)).clamp(0., 1.);
                            if cov > 0. {
                                let on = ((px + py) as f32 / (a.h * 0.8)) as i32 % 2 == 0;
                                p.blend(px, py, if on { stripe } else { base }, cov);
                            }
                        }
                    }
                    p.fill_rows(a.x, a.y, fw, a.h, r, (a.y, a.y + a.h * 0.5), [255, 255, 255, 60], [255, 255, 255, 0]);
                }
            }
            Theme::Terminal | Theme::Pixel => {
                let square = self.theme == Theme::Pixel;
                p.stroke_rr(a.x, a.y, a.w, a.h, 0., if square { 3. } else { 1. }, if square { lighten(self.fg, 0.1) } else { fade(self.fg, 0.5) });
                let inner = a.inset(5., 5., 5., 5.);
                let n = (inner.w / (inner.h * 0.8)).floor().max(4.) as usize;
                let step = inner.w / n as f32;
                for i in 0..((n as f32 * t).round() as usize).min(n) {
                    let c = if square { self.accent } else { self.fg };
                    p.fill_rr(inner.x + step * i as f32, inner.y, step - 3., inner.h, 0., lighten(c, 0.15), c);
                }
            }
            _ => {
                let r = if matches!(self.theme, Theme::Broadcast) { 2. } else { a.h / 2. };
                p.fill_rr(a.x, a.y, a.w, a.h, r, darken(self.surface, 0.2), self.surface);
                p.stroke_rr(a.x, a.y, a.w, a.h, r, 1., [0, 0, 0, 40]);
                let fw = (a.w * t).max(if t > 0. { a.h } else { 0. });
                if fw > 0. {
                    p.shadow_rr(a.x, a.y, fw, a.h, r, a.h * 0.9, fade(self.accent, 0.45));
                    let end = match self.theme {
                        Theme::Synthwave => [255, 170, 70, 255],
                        Theme::Spooky => [150, 70, 210, 255],
                        _ => lighten(self.accent, 0.28),
                    };
                    p.fill_rr_across(a.x, a.y, fw, a.h, r, darken(self.accent, 0.08), end);
                    p.fill_rows(a.x, a.y, fw, a.h, r, (a.y, a.y + a.h * 0.5), [255, 255, 255, 50], [255, 255, 255, 0]);
                    if self.theme == Theme::Team {
                        let (x0, y0, x1, y1) = p.bounds(a.x, a.y, a.x + fw, a.bottom());
                        for py in y0..y1 {
                            for px in x0..x1 {
                                if (px + py) % 12 < 4 && rr_distance(px as f32 + 0.5, py as f32 + 0.5, a.x, a.y, fw, a.h, r) < 0. {
                                    p.blend(px, py, [255, 255, 255, 255], 0.12);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    /// A small rounded tag ("TWITCH", "Q4 2:14").
    #[allow(clippy::too_many_arguments)]
    fn pill(&self, p: &mut Paint, text: &str, right: f32, mid_y: f32, size: f32, fill: [u8; 4], c: [u8; 4]) -> f32 {
        let w = self.width(Role::Label, text, size) + size * 1.4;
        let h = size * 1.75;
        let r = if self.square() { 0. } else { h / 2. };
        if self.theme == Theme::Neubrutalism {
            p.fill_rr(right - w + 2., mid_y - h / 2. + 2., w, h, r, [12, 12, 12, 255], [12, 12, 12, 255]);
            p.fill_rr(right - w, mid_y - h / 2., w, h, r, [255; 4], [255; 4]);
            p.stroke_rr(right - w, mid_y - h / 2., w, h, r, 2., [12, 12, 12, 255]);
            self.text_fx(p, Role::Label, text, right - w + size * 0.7, mid_y + size * 0.36, size, [12, 12, 12, 255], w, Fx::Plain);
            return w;
        }
        p.fill_rr(right - w, mid_y - h / 2., w, h, r, fill, fill);
        self.text_fx(p, Role::Label, text, right - w + size * 0.7, mid_y + size * 0.36, size, c, w, Fx::Plain);
        w
    }
}
/// A cottagecore sprig: a stem with three leaves, leaning left (-1) or right (1).
fn sprig(p: &mut Paint, x: f32, y: f32, lean: f32, c: [u8; 4]) {
    for k in 0..18 {
        p.circle(x - lean * k as f32 * 0.8, y + k as f32 * 0.9, 0.9, darken(c, 0.2));
    }
    for (i, (dx, dy, angle)) in [(-5., 5., -0.7), (-10., 11., 0.6), (-14., 16., -0.5)].into_iter().enumerate() {
        let (cx, cy) = (x + lean * dx, y + dy);
        let a = angle * lean;
        let (len, wid) = (8. - i as f32, 3.4);
        let leaf = |px: f32, py: f32| {
            let (lx, ly) = (px - cx, py - cy);
            let (rx, ry) = (lx * a.cos() + ly * a.sin(), -lx * a.sin() + ly * a.cos());
            ((rx / len).hypot(ry / wid) - 1.) * wid
        };
        p.fill_shape(cx - len, cy - len, len * 2., len * 2., leaf, lighten(c, 0.15), c);
    }
}
fn design_canvas(s: &Settings) -> Paint {
    let width = s.layout_width.clamp(96, 960);
    let height = s.layout_height.clamp(96, 960);
    Paint { pixels: vec![0; (width * height * 4) as usize], width, height }
}
/// The text after the widget's label line.
fn body_of<'a>(s: &Settings, text: &'a str) -> &'a str {
    let body = text.strip_prefix(&s.label).unwrap_or(text).trim_start_matches([':', '\n', ' ']);
    if body.is_empty() { text } else { body }
}
fn title_of(s: &Settings) -> &str {
    if s.label.is_empty() { s.kind.label() } else { &s.label }
}

// ---------- Chat ----------

fn polished_chat(s: &Settings, text: &str, at_ms: u64) -> Frame {
    use auroracast_core::widgets::Layout;
    let l = Look::new(s);
    if matches!(s.theme, Theme::Windows98 | Theme::WindowsXP) && s.layout != Layout::Lines {
        return window_chat(s, l, text, at_ms);
    }
    let mut p = design_canvas(s);
    let log = l.retro_window() || matches!(s.theme, Theme::Terminal | Theme::Led | Theme::GameBoy) || s.layout == Layout::Lines;
    let bubbles = !log && (s.theme == Theme::Bubbles || s.layout == Layout::Bubbles);
    let title = if s.label.is_empty() && !l.retro_window() { "" } else { title_of(s) };
    let area = if bubbles && s.theme != Theme::Material {
        let label = (p.height as f32 * 0.06).clamp(11., 15.);
        let a = Area { x: 12., y: 10., w: p.width as f32 - 24., h: p.height as f32 - 20. };
        if !title.is_empty() && s.theme == Theme::Bubbles {
            let w = l.width(Role::Label, title, label) + 24.;
            p.fill_rr(a.x, a.y, w, label * 2., label, [0, 0, 0, 120], [0, 0, 0, 120]);
            l.text(&mut p, Role::Label, title, a.x + 12., a.y + label * 1.36, label, [255; 4], w);
            a.inset(0., label * 2. + 10., 0., 0.)
        } else if s.theme == Theme::Bubbles {
            a
        } else {
            l.panel(&mut p, title, false)
        }
    } else {
        l.panel(&mut p, title, log)
    };
    let fixed = s.chat_rows.clamp(0, 7) as usize;
    let gap = s.spacing as f32;
    // A fixed number of rows: one line each, sized so exactly that many fit.
    let row_lines = if bubbles { 2.25 } else if log { 1.32 } else { 2.95 };
    let size = if fixed > 0 {
        ((area.h - gap * (fixed as f32 - 1.)) / (fixed as f32 * row_lines)).min(s.font_size as f32).min(area.w / 11.).clamp(9., 44.)
    } else {
        (s.font_size as f32).min(area.w / 11.).min(area.h / 9.5).clamp(13., 44.)
    };
    let fg = if l.retro_window() { l.field_fg } else { l.fg };
    let rows = chat_rows(text, fixed);
    let name_color = |platform: &str| -> [u8; 4] {
        if s.theme == Theme::Unified || l.retro_window() {
            if l.retro_window() { darken(platform_color(platform), 0.25) } else { lighten(platform_color(platform), 0.25) }
        } else if l.glows() || s.theme == Theme::GameBoy {
            if l.light { darken(l.fg, 0.2) } else { lighten(l.fg, 0.25) }
        } else {
            l.ink(l.accent)
        }
    };
    // Lay out newest first from the bottom, keeping what fits.
    struct Row<'a> {
        platform: &'a str,
        author: &'a str,
        lines: Vec<String>,
        h: f32,
    }
    let avatar = if bubbles || log || area.w < 300. { 0. } else { size * 1.45 };
    // Bubbles sit beside the chatter's picture, as in messaging apps.
    let bubble_avatar = if bubbles && area.w >= 300. { size * 1.6 } else { 0. };
    let lane_w = area.w - if bubble_avatar > 0. { bubble_avatar + size * 0.4 } else { 0. };
    // Compact lines get a small picture before the name, except on the text-only themes.
    let inline = if log && !l.retro_window() && !matches!(s.theme, Theme::Terminal | Theme::Led | Theme::GameBoy | Theme::Pixel) && area.w >= 260. { size * 1.15 } else { 0. };
    let mut placed: Vec<Row> = Vec::new();
    let mut total = 0.;
    for &(platform, author, body) in rows.iter().rev() {
        let (lines, h) = if log {
            let lead = l.width(Role::Strong, &format!("{author}  "), size) + if s.theme == Theme::Terminal { l.width(Role::Body, "> ", size) } else { 0. } + if inline > 0. { inline + size * 0.4 } else { 0. };
            let mut lines = l.wrap(Role::Body, body, area.w - lead, size);
            clip_lines(&mut lines);
            let n = lines.len() as f32;
            (lines, n * size * 1.32 + size * 0.38)
        } else if bubbles {
            let mut lines = l.wrap(Role::Body, body, lane_w * 0.78 - size * 1.6, size);
            clip_lines(&mut lines);
            let n = lines.len() as f32;
            (lines, n * size * 1.3 + size * 1.1 + size * 0.95)
        } else {
            let mut lines = l.wrap(Role::Body, body, area.w - avatar - size * 1.9 - if avatar > 0. { size * 0.7 } else { 0. }, size);
            clip_lines(&mut lines);
            let n = lines.len() as f32;
            (lines, size * 0.62 * 1.25 + n * size * 1.24 + size * 0.85)
        };
        if total + h > area.h && !placed.is_empty() {
            break;
        }
        total += h + gap;
        placed.push(Row { platform, author, lines, h });
        if total > area.h {
            break;
        }
    }
    if !s.reverse {
        placed.reverse();
    }
    let mut y = if s.reverse { area.y } else { (area.bottom() - total + gap).max(area.y) };
    for (i, row) in placed.iter().enumerate() {
        let color = name_color(row.platform);
        if log {
            let mut x = area.x;
            let base = y + size * 1.05;
            if s.theme == Theme::Terminal {
                x += l.text(&mut p, Role::Body, "> ", x, base, size, l.ter, 60.);
            }
            if inline > 0. {
                avatar_at(&mut p, &l, row.platform, row.author, x, base - size * 0.92, inline, inline / 2.);
                x += inline + size * 0.4;
            }
            let name = if s.theme == Theme::Terminal { format!("<{}>", row.author) } else { format!("{}:", row.author) };
            let lead = l.text(&mut p, Role::Strong, &name, x, base, size, color, area.w * 0.4) + l.width(Role::Body, " ", size) * 1.6;
            for (j, line) in row.lines.iter().enumerate() {
                l.rich(&mut p, Role::Body, line, x + lead, base + j as f32 * size * 1.32, size, fg, area.right() - x - lead, at_ms);
            }
        } else if bubbles {
            let right = i % 2 == 1;
            let lane = if bubble_avatar > 0. {
                let pic_x = if right { area.right() - bubble_avatar } else { area.x };
                avatar_at(&mut p, &l, row.platform, row.author, pic_x, y + row.h - bubble_avatar, bubble_avatar, bubble_avatar / 2.);
                Area { x: if right { area.x } else { area.x + area.w - lane_w }, w: lane_w, ..area }
            } else {
                area
            };
            let area = lane;
            let caption = (size * 0.55).max(11.);
            let width = row.lines.iter().map(|t| l.rich_width(Role::Body, t, size)).fold(0., f32::max) + size * 1.6;
            let width = width.max(l.width(Role::Strong, row.author, caption) + size * 1.6).min(area.w * 0.8);
            let bh = row.h - caption * 1.7;
            let x = if right { area.right() - width } else { area.x };
            let by = y + caption * 1.6;
            let name_x = if right { area.right() - l.width(Role::Strong, row.author, caption) - 6. } else { area.x + 6. };
            l.text(&mut p, Role::Strong, row.author, name_x, y + caption * 1.1, caption, if s.theme == Theme::Transparent { l.fg } else { lighten(platform_color(row.platform), 0.35) }, area.w);
            let (top, bottom, text_c) = if right {
                (lighten(l.accent, 0.12), darken(l.accent, 0.08), on_color(l.accent))
            } else {
                ([58, 58, 66, 238], [44, 44, 52, 238], [255; 4])
            };
            p.shadow_rr(x, by + 3., width, bh, size * 0.9, 8., [0, 0, 0, 70]);
            p.fill_rr(x, by, width, bh, size * 0.9, top, bottom);
            // The tail at the bottom corner.
            let tail_x = if right { x + width - 6. } else { x + 6. };
            p.circle(tail_x, by + bh - 6., 7., bottom);
            p.circle(if right { tail_x + 7. } else { tail_x - 7. }, by + bh + 1., 3., bottom);
            for (j, line) in row.lines.iter().enumerate() {
                l.rich(&mut p, Role::Body, line, x + size * 0.8, by + size * 1.15 + j as f32 * size * 1.3, size, text_c, width - size * 1.6, at_ms);
            }
        } else {
            // A card per message, with an avatar initial and the platform.
            let r = (l.radius * 0.7).clamp(0., 14.);
            match s.theme {
                Theme::Pixel => {
                    p.fill_rr(area.x, y, area.w, row.h, 0., darken(l.bg, 0.3), darken(l.bg, 0.3));
                    p.stroke_rr(area.x, y, area.w, row.h, 0., 2., fade(l.fg, 0.35));
                }
                Theme::Broadcast | Theme::Team => {
                    p.fill_rr(area.x, y, area.w, row.h, r.min(4.), l.surface, fade(l.surface, 0.6));
                    p.fill_rr(area.x, y, 4., row.h, 0., platform_color(row.platform), platform_color(row.platform));
                }
                Theme::Transparent => {
                    p.fill_rr(area.x, y, area.w, row.h, r, [0, 0, 0, 90], [0, 0, 0, 60]);
                }
                Theme::Neubrutalism => {
                    p.fill_rr(area.x + 4., y + 4., area.w - 4., row.h, 6., [12, 12, 12, 255], [12, 12, 12, 255]);
                    p.fill_rr(area.x, y, area.w - 4., row.h, 6., [255; 4], [255; 4]);
                    p.stroke_rr(area.x, y, area.w - 4., row.h, 6., 2.5, [12, 12, 12, 255]);
                }
                Theme::Cyberpunk => {
                    let cut = (row.h * 0.25).min(14.);
                    let d = |px: f32, py: f32| chamfer_distance(px, py, area.x, y, area.w, row.h, cut);
                    p.fill_shape(area.x, y, area.w, row.h, d, [255, 255, 255, 12], [255, 255, 255, 6]);
                    p.stroke_shape(area.x, y, area.w, row.h, d, 1., fade(l.accent, 0.35));
                    p.fill_rr(area.x, y + cut, 3., row.h - cut * 2., 0., [0, 240, 255, 255], [0, 240, 255, 255]);
                }
                Theme::Fantasy | Theme::Cottagecore => {
                    p.hline(area.x + area.w * 0.1, y + row.h - 1., area.w * 0.8, fade(l.accent, 0.35));
                }
                Theme::CozyCafe | Theme::Kawaii | Theme::CleanLight => {
                    p.fill_rr(area.x, y, area.w, row.h, r.max(10.), [255, 255, 255, 170], [255, 255, 255, 120]);
                    p.stroke_rr(area.x, y, area.w, row.h, r.max(10.), 1., if l.theme == Theme::Kawaii { fade(l.accent, 0.35) } else { [0, 0, 0, 14] });
                }
                Theme::SciFiHud => {
                    p.stroke_rr(area.x, y, area.w, row.h, 0., 1., fade(l.accent, 0.25));
                    p.fill_rr(area.x, y, 3., row.h, 0., l.accent, l.accent);
                }
                _ => {
                    p.fill_rr(area.x, y, area.w, row.h, r, lighten(l.surface, 0.02), l.surface);
                    p.stroke_rr(area.x, y, area.w, row.h, r, 1., [255, 255, 255, 12]);
                }
            }
            let mut x = area.x + size * 0.7;
            let top = y + size * 0.45;
            if avatar > 0. {
                let r = if s.theme == Theme::Pixel { 0. } else if l.square() { 3. } else { avatar / 2. };
                p.shadow_rr(x, top + 2., avatar, avatar, r, 5., [0, 0, 0, 80]);
                avatar_at(&mut p, &l, row.platform, row.author, x, top, avatar, r);
                x += avatar + size * 0.7;
            }
            let name_size = (size * 0.62).max(11.);
            let name_base = top + name_size * 1.05;
            let tag = (name_size * 0.72).max(9.);
            let tag_w = l.pill(&mut p, row.platform, area.right() - size * 0.6, name_base - name_size * 0.35, tag, fade(platform_color(row.platform), 0.22), l.ink(platform_color(row.platform)));
            l.text(&mut p, Role::Title, row.author, x, name_base, name_size, color, area.right() - x - tag_w - size);
            for (j, line) in row.lines.iter().enumerate() {
                l.rich(&mut p, Role::Body, line, x, name_base + size * 1.2 + j as f32 * size * 1.24, size, fg, area.right() - x - size * 0.6, at_ms);
            }
        }
        y += row.h + gap;
    }
    p.finish()
}

/// Longest a chat message gets in the widget.
const CHAT_LINES: usize = 3;
/// Keeps a message to CHAT_LINES lines, ending a cut one with "…".
fn clip_lines(lines: &mut Vec<String>) {
    if lines.len() > CHAT_LINES {
        lines.truncate(CHAT_LINES);
        lines[CHAT_LINES - 1].push('…');
    }
}
/// Whether the chat widget's look has room for chatters' pictures (text-only themes don't).
fn chat_pictures(s: &Settings) -> bool {
    use auroracast_core::widgets::Layout;
    !matches!(s.theme, Theme::Terminal | Theme::Led | Theme::GameBoy | Theme::Vaporwave) && !(s.theme == Theme::Pixel && s.layout == Layout::Lines)
}
/// The chat widget's text as (platform, author, message) rows, the last `fixed` of them (0: all).
fn chat_rows(text: &str, fixed: usize) -> Vec<(&str, &str, &str)> {
    let all: Vec<&str> = text.lines().skip(1).filter(|r| !r.is_empty()).collect();
    all[all.len().saturating_sub(if fixed > 0 { fixed } else { usize::MAX })..]
        .iter()
        .map(|row| {
            let (platform, rest) = row.split_once(" · ").unwrap_or(("", row));
            let (author, body) = rest.split_once(": ").unwrap_or(("", rest));
            (platform, author, body)
        })
        .collect()
}
/// A chatter's profile picture, `d` across with corner radius `r`, or their initial on the
/// platform's colour until the picture loads (or when there is none).
#[allow(clippy::too_many_arguments)]
fn avatar_at(p: &mut Paint, l: &Look, platform: &str, author: &str, x: f32, y: f32, d: f32, r: f32) {
    let n = d.round().max(1.) as u32;
    if let Some(picture) = crate::avatars::get(platform, author) {
        let scaled = image::imageops::resize(picture.as_ref(), n, n, image::imageops::FilterType::Triangle);
        for (px, py, c) in scaled.enumerate_pixels() {
            let edge = rr_distance(px as f32 + 0.5, py as f32 + 0.5, 0., 0., n as f32, n as f32, r);
            p.blend(x.round() as i32 + px as i32, y.round() as i32 + py as i32, c.0, (0.5 - edge).clamp(0., 1.));
        }
        return;
    }
    let pc = platform_color(platform);
    p.fill_rr(x, y, d, d, r, lighten(pc, 0.15), darken(pc, 0.25));
    let initial: String = author.chars().filter(|c| c.is_alphanumeric()).take(1).flat_map(|c| c.to_uppercase()).collect();
    l.text_in(p, Role::Title, &initial, Area { x, y, w: d, h: d }, y + d / 2. + d * 0.19, d * 0.5, on_color(pc), 0.5);
}
/// Windows 98 and XP: each message is its own little window, titled with the chatter's name,
/// with their picture as the message box's icon.
fn window_chat(s: &Settings, mut l: Look, text: &str, at_ms: u64) -> Frame {
    let mut p = design_canvas(s);
    let (w, h) = (p.width as f32, p.height as f32);
    let fixed = s.chat_rows.clamp(0, 7) as usize;
    let gap = (s.spacing as f32).max(4.);
    // Title bars in step with the message text: first a guess from the font size, then from the
    // size the messages end up at.
    l.title_size = Some((s.font_size as f32 * 0.6).clamp(12., 22.));
    // The frame around a window's contents (title bar and borders), measured on a sample window.
    let chrome = |l: &Look| {
        let mut probe = Paint { pixels: vec![0; (w as usize) * 160 * 4], width: w as u32, height: 160 };
        let inner = l.panel_inner(&mut probe, "", false);
        (inner.y, 160. - inner.bottom(), inner.x, w - inner.right())
    };
    let fit = |(top, bottom, _, _): (f32, f32, f32, f32)| {
        if fixed > 0 { ((h - gap * (fixed as f32 - 1.)) / fixed as f32 - top - bottom).max(10.) / 1.3 } else { s.font_size as f32 }
            .min(s.font_size as f32)
            .min(w / 11.)
            .clamp(9., 40.)
    };
    let size = fit(chrome(&l));
    l.title_size = Some((size * 0.6).clamp(11., 22.));
    let (top, bottom, left, right) = chrome(&l);
    let size = fit((top, bottom, 0., 0.));
    let icon = if w >= 260. { (size * 1.9).min(56.) } else { 0. };
    let text_x = if icon > 0. { icon + size * 0.6 } else { 0. };
    let text_w = w - left - right - text_x;
    struct Row<'a> {
        platform: &'a str,
        author: &'a str,
        lines: Vec<String>,
        h: f32,
    }
    let mut placed: Vec<Row> = Vec::new();
    let mut total = 0.;
    for (platform, author, body) in chat_rows(text, fixed).into_iter().rev() {
        let mut lines = l.wrap(Role::Body, body, text_w, size);
        clip_lines(&mut lines);
        let rh = top + bottom + (lines.len() as f32 * size * 1.3 + size * 0.3).max(icon + 2.);
        if total + rh > h && !placed.is_empty() {
            break;
        }
        total += rh + gap;
        placed.push(Row { platform, author, lines, h: rh });
    }
    if !s.reverse {
        placed.reverse();
    }
    let mut y = if s.reverse { 0. } else { (h - total + gap).max(0.) };
    for row in &placed {
        let mut window = Paint { pixels: vec![0; (w as usize) * (row.h.ceil() as usize) * 4], width: w as u32, height: row.h.ceil() as u32 };
        let title = if row.platform.is_empty() { row.author.to_string() } else { format!("{} - {}", row.author, row.platform) };
        let inner = l.panel(&mut window, &title, false);
        if icon > 0. {
            let (r, iy) = (if s.theme == Theme::WindowsXP { 4. } else { 0. }, inner.y + 1.);
            avatar_at(&mut window, &l, row.platform, row.author, inner.x, iy, icon, r);
            if s.theme == Theme::Windows98 {
                window.bevel(inner.x - 1., iy - 1., icon + 2., icon + 2., false);
            }
        }
        for (j, line) in row.lines.iter().enumerate() {
            let base = inner.y + size * 1.05 + j as f32 * size * 1.3;
            l.rich(&mut window, Role::Body, line, inner.x + text_x, base, size, l.field_fg, inner.w - text_x, at_ms);
        }
        let oy = y.round() as i32;
        for (i, px) in window.pixels.chunks_exact(4).enumerate() {
            if px[3] > 0 {
                p.blend((i as u32 % window.width) as i32, oy + (i as u32 / window.width) as i32, [px[0], px[1], px[2], px[3]], 1.);
            }
        }
        y += row.h + gap;
    }
    p.finish()
}

// ---------- Now playing ----------

fn polished_music(s: &Settings, track: Option<&crate::music::Track>) -> Frame {
    let mut p = design_canvas(s);
    let Some(t) = track else { return p.finish() };
    let mut l = Look::new(s);
    if s.music.artwork_colors && t.accent[3] > 0 {
        l = l.with_accent(t.accent);
    }
    let area = l.panel(&mut p, "", false);
    let portrait = area.w / area.h < 1.35;
    let art = if portrait { area.w.min(area.h * 0.58) } else { area.h.min(area.w * 0.36) };
    let (ax, ay) = if portrait { (area.x + (area.w - art) / 2., area.y) } else { (area.x, area.y + (area.h - art) / 2.) };
    let ar = match l.theme {
        Theme::Windows98 | Theme::Pixel | Theme::Led | Theme::Terminal => 0.,
        _ => (l.radius * 0.6).clamp(6., 18.),
    };
    p.shadow_rr(ax, ay + art * 0.04, art, art, ar, art * 0.1, [0, 0, 0, 150]);
    match &t.artwork {
        Some(f) if f.width > 0 && f.height > 0 => {
            let img = image::RgbaImage::from_raw(f.width, f.height, f.rgba.clone()).unwrap_or_default();
            let scaled = image::imageops::resize(&img, art.round().max(1.) as u32, art.round().max(1.) as u32, image::imageops::FilterType::Triangle);
            for (px, py, c) in scaled.enumerate_pixels() {
                let d = rr_distance(px as f32 + 0.5, py as f32 + 0.5, 0., 0., art, art, ar);
                p.blend(ax.round() as i32 + px as i32, ay.round() as i32 + py as i32, c.0, (0.5 - d).clamp(0., 1.));
            }
        }
        _ => {
            // No cover: a record on a gradient sleeve.
            p.fill_rr(ax, ay, art, art, ar, lighten(l.accent, 0.1), darken(l.accent, 0.55));
            let (cx, cy) = (ax + art / 2., ay + art / 2.);
            p.circle(cx, cy, art * 0.36, [16, 16, 20, 255]);
            for k in 1..5 {
                p.stroke_rr(cx - art * 0.08 * k as f32 - art * 0.04, cy - art * 0.08 * k as f32 - art * 0.04, (art * 0.08 * k as f32 + art * 0.04) * 2., (art * 0.08 * k as f32 + art * 0.04) * 2., art, 1., [255, 255, 255, 14]);
            }
            p.circle(cx, cy, art * 0.11, lighten(l.accent, 0.2));
            p.circle(cx, cy, art * 0.02, [16, 16, 20, 255]);
        }
    }
    p.stroke_rr(ax, ay, art, art, ar, 1., [255, 255, 255, 30]);
    let x = if portrait { area.x } else { ax + art + area.h * 0.12 };
    let width = area.right() - x;
    let top = if portrait { ay + art + area.h * 0.06 } else { area.y };
    let space = if portrait { area.bottom() - top } else { area.h };
    let title_size = (s.font_size as f32 * 1.2).min(width / 8.).min(space * 0.26).clamp(16., 64.);
    let artist_size = if s.music.artist_size > 0 { s.music.artist_size as f32 } else { (title_size * 0.62).max(12.) };
    let label = (title_size * 0.38).clamp(12., 18.);
    let compact = space < title_size * 3.4;
    let mut y = top + if portrait { title_size } else { (space - title_size * 2.6 - if s.music.progress && t.length > 0. { title_size } else { 0. }) / 2. };
    if !compact {
        // Equaliser bars beside the status.
        let bars = [0.55, 1., 0.7, 0.85];
        for (i, h) in bars.iter().enumerate() {
            let bh = label * if t.playing { *h } else { 0.3 };
            p.fill_rr(x + i as f32 * (label * 0.32), y - bh, label * 0.2, bh, label * 0.1, l.accent, l.accent);
        }
        l.text(&mut p, Role::Label, if t.playing { "Now playing" } else { "Paused" }, x + label * 1.55, y, label, l.accent, width);
        y += title_size * 1.35;
    } else {
        y += title_size * 0.5;
    }
    l.text(&mut p, Role::Title, &t.title, x, y, title_size, l.fg, width);
    y += artist_size * 1.4;
    // A chosen artist or album font is used as is; one that can't be read falls back to the theme's.
    match crate::fonts::load(&s.music.artist_font) {
        Some(f) => p.line_font(&t.artist, x, y, width, artist_size, l.sec, &f, false),
        None => {
            l.text(&mut p, Role::Body, &t.artist, x, y, artist_size, l.sec, width);
        }
    }
    if s.music.show_album && !t.album.is_empty() {
        let album_size = if s.music.album_size > 0 { s.music.album_size as f32 } else { (artist_size * 0.82).max(11.) };
        y += album_size * 1.35;
        match crate::fonts::load(&s.music.album_font) {
            Some(f) => p.line_font(&t.album, x, y, width, album_size, l.ter, &f, false),
            None => {
                l.text(&mut p, Role::Caption, &t.album, x, y, album_size, l.ter, width);
            }
        }
    }
    if s.music.progress && t.length > 0. && space > title_size * 3. {
        let bh = (title_size * 0.17).clamp(5., 12.);
        let by = (y + title_size * 0.9).min(if portrait { area.bottom() - label * 2.2 } else { area.bottom() - label * 2. - bh });
        l.bar(&mut p, Area { x, y: by, w: width, h: bh }, t.progress());
        let secs = (t.progress() as f64 * t.length) as u64;
        let total = t.length as u64;
        let ty = by + bh + label * 1.5;
        l.text(&mut p, Role::Caption, &format!("{}:{:02}", secs / 60, secs % 60), x, ty, label, l.ter, width / 2.);
        l.text_in(&mut p, Role::Caption, &format!("{}:{:02}", total / 60, total % 60), Area { x, y: ty, w: width, h: label }, ty, label, l.ter, 1.);
    }
    p.finish()
}

// ---------- Goals ----------

fn polished_goal(s: &Settings, value: u64) -> Frame {
    let l = Look::new(s);
    let mut p = design_canvas(s);
    let area = l.panel(&mut p, title_of(s), false);
    let t = (value as f64 / s.target.max(1) as f64).clamp(0., 1.) as f32;
    let amount = |v: u64| if s.kind == Kind::DonationGoal { format!("{}.{:02}", v / 100, v % 100) } else { v.to_string() };
    let unit = if s.kind == Kind::DonationGoal { format!(" {}", s.currency) } else { String::new() };
    let fg = if l.retro_window() { l.field_fg } else { l.fg };
    let sec = if l.retro_window() { fade(l.field_fg, 0.7) } else { l.sec };
    let percent = format!("{}%", (t * 100.).round() as u32);
    let tag = (area.h * 0.075).clamp(12., 18.);
    match s.goal_style {
        GoalStyle::Ring => {
            let d = area.h.min(area.w * 0.45);
            let (cx, cy) = (area.x + d / 2., area.y + area.h / 2.);
            let thick = d * 0.11;
            p.arc(cx, cy, d / 2. - thick, thick, 0., 1., l.surface, l.surface);
            if t > 0. {
                p.arc(cx, cy, d / 2. - thick, thick, 0., t.max(0.01), darken(l.accent, 0.1), lighten(l.accent, 0.3));
            }
            let ring = Area { x: cx - d / 2., y: cy - d / 2., w: d, h: d };
            l.text_in(&mut p, Role::Display, &percent, ring, cy + d * 0.09, d * 0.24, fg, 0.5);
            let tx = area.x + d + area.w * 0.05;
            let side = Area { x: tx, y: area.y, w: area.right() - tx, h: area.h };
            let big = (side.h * 0.3).min(side.w / 5.).clamp(18., 80.);
            l.number(&mut p, &amount(value), Area { w: side.w, ..side }, cy + big * 0.2, big, fg, 0.);
            l.text(&mut p, Role::Body, &format!("of {}{unit}", amount(s.target)), tx, cy + big * 0.2 + big * 0.55, (big * 0.36).max(12.), sec, side.w);
        }
        GoalStyle::Vertical | GoalStyle::Thermometer => {
            let bw = (area.w * 0.09).clamp(16., 46.);
            let bulb = s.goal_style == GoalStyle::Thermometer;
            let bx = area.right() - bw - if bulb { bw * 0.4 } else { 0. };
            let track = Area { x: bx, y: area.y, w: bw, h: area.h - if bulb { bw * 0.9 } else { 0. } };
            p.fill_rr(track.x, track.y, track.w, track.h, bw / 2., darken(l.surface, 0.2), l.surface);
            let fh = (track.h * t).max(if t > 0. { bw } else { 0. });
            p.shadow_rr(track.x, track.bottom() - fh, bw, fh, bw / 2., bw * 0.6, fade(l.accent, 0.4));
            p.fill_rr(track.x, track.bottom() - fh, bw, fh, bw / 2., lighten(l.accent, 0.3), darken(l.accent, 0.1));
            if bulb {
                p.circle(bx + bw / 2., track.bottom() + bw * 0.25, bw * 0.75, l.accent);
                p.circle(bx + bw / 2. - bw * 0.2, track.bottom() + bw * 0.05, bw * 0.18, [255, 255, 255, 70]);
            }
            let side = Area { w: bx - area.x - 20., ..area };
            let big = (side.h * 0.34).min(side.w / 5.).clamp(18., 90.);
            l.number(&mut p, &amount(value), side, area.y + area.h * 0.55, big, fg, 0.);
            l.text(&mut p, Role::Body, &format!("of {}{unit} · {percent}", amount(s.target)), side.x, area.y + area.h * 0.55 + big * 0.6, (big * 0.32).max(12.), sec, side.w);
        }
        GoalStyle::Counter => {
            let big = (area.h * 0.55).min(area.w / 5.).clamp(20., 140.);
            l.number(&mut p, &amount(value), area, area.y + area.h * 0.5 + big * 0.3, big, fg, 0.5);
            let cap = Area { y: area.y + area.h * 0.5 + big * 0.3, ..area };
            l.text_in(&mut p, Role::Caption, &format!("of {}{unit}", amount(s.target)), cap, cap.y + big * 0.38, (big * 0.24).max(12.), sec, 0.5);
        }
        _ => {
            let big = (area.h * 0.36).min(area.w / 6.).clamp(18., 96.);
            let base = area.y + big * 0.86;
            let numbers = Area { w: area.w * 0.7, ..area };
            l.number(&mut p, &amount(value), numbers, base, big, fg, 0.);
            let nw = l.width(Role::Display, &amount(value), big).min(numbers.w);
            l.text(&mut p, Role::Body, &format!("/ {}{unit}", amount(s.target)), area.x + nw + big * 0.22, base, (big * 0.36).max(12.), sec, area.w - nw);
            if !l.retro_window() {
                l.pill(&mut p, &percent, area.right(), area.y + big * 0.5, tag, fade(l.accent, 0.25), l.ink(l.accent));
            }
            let bh = (area.h * 0.13).clamp(8., 30.);
            let by = (base + (area.bottom() - base) * 0.5 - bh / 2.).max(base + 10.);
            let bar = Area { x: area.x, y: by, w: area.w, h: bh };
            match s.goal_style {
                GoalStyle::Segments => {
                    let n = 10;
                    let step = (bar.w + 6.) / n as f32;
                    for i in 0..n {
                        let on = (i as f32) < t * n as f32;
                        let x = bar.x + step * i as f32;
                        if on {
                            p.shadow_rr(x, bar.y, step - 6., bar.h, bar.h * 0.3, bar.h * 0.5, fade(l.accent, 0.35));
                        }
                        p.fill_rr(x, bar.y, step - 6., bar.h, bar.h * 0.3, if on { lighten(l.accent, 0.25) } else { l.surface }, if on { l.accent } else { l.surface });
                    }
                }
                GoalStyle::Hearts | GoalStyle::Garden => {
                    let n = 10;
                    let cell = (bar.w / n as f32).min(area.bottom() - base - 10.);
                    let px = (cell / 9.).floor().max(1.);
                    for i in 0..n {
                        let on = (i as f32) < t * n as f32;
                        let ox = bar.x + bar.w / n as f32 * i as f32 + (bar.w / n as f32 - px * 7.) / 2.;
                        let oy = base + 12.;
                        let c = if on { l.accent } else { fade(l.fg, 0.16) };
                        let sprite: &[&str] = if s.goal_style == GoalStyle::Hearts {
                            &[".##.##.", "#######", "#######", ".#####.", "..###..", "...#..."]
                        } else {
                            &["..#.#..", ".#####.", "..###..", "...#...", ".#.#.#.", "..###..", "...#..."]
                        };
                        for (row, line) in sprite.iter().enumerate() {
                            for (col, ch) in line.chars().enumerate() {
                                if ch == '#' {
                                    let leaf = s.goal_style == GoalStyle::Garden && row >= 3;
                                    let color = if leaf && on { [90, 200, 110, 255] } else { c };
                                    p.fill_rr(ox + col as f32 * px, oy + row as f32 * px, px, px, 0., color, color);
                                }
                            }
                        }
                        if on && s.goal_style == GoalStyle::Hearts {
                            p.fill_rr(ox + px, oy + px, px, px, 0., [255, 255, 255, 140], [255, 255, 255, 140]);
                        }
                    }
                }
                GoalStyle::Milestones => {
                    l.bar(&mut p, bar, t);
                    for k in 1..4 {
                        let mx = bar.x + bar.w * k as f32 / 4.;
                        let reached = t >= k as f32 / 4.;
                        p.circle(mx, bar.y + bar.h / 2., bar.h * 0.75, if reached { lighten(l.accent, 0.4) } else { darken(l.bg, 0.2) });
                        p.stroke_rr(mx - bar.h * 0.75, bar.y + bar.h / 2. - bar.h * 0.75, bar.h * 1.5, bar.h * 1.5, bar.h, 1.5, if reached { [255; 4] } else { l.divider });
                        let mark = Area { x: mx - 40., y: bar.bottom(), w: 80., h: 20. };
                        l.text_in(&mut p, Role::Caption, &amount(s.target * k / 4), mark, bar.bottom() + bar.h + 12., tag, sec, 0.5);
                    }
                }
                _ => l.bar(&mut p, bar, t),
            }
        }
    }
    p.finish()
}

// ---------- Timer and counter ----------

fn polished_number(s: &Settings, text: &str) -> Frame {
    let l = Look::new(s);
    let mut p = design_canvas(s);
    let area = l.panel(&mut p, title_of(s), false);
    let fg = if l.retro_window() { l.field_fg } else { l.fg };
    let value = body_of(s, text).trim();
    let caption = match s.kind {
        Kind::Timer if s.timer_up => "elapsed",
        Kind::Timer => "remaining",
        _ => "total",
    };
    let big = (area.h * 0.62).min(area.w / (value.chars().count().max(2) as f32 * 0.62)).clamp(20., 200.);
    let base = area.y + area.h * 0.5 + big * 0.3;
    l.number(&mut p, value, area, base, big, fg, 0.5);
    if area.h > big * 1.35 {
        let cap = (big * 0.15).clamp(10., 16.);
        l.text_in(&mut p, Role::Label, caption, area, (base + big * 0.36).min(area.bottom() - 2.), cap, if l.retro_window() { fade(fg, 0.6) } else { l.accent }, 0.5);
    }
    p.finish()
}

// ---------- Text cards and lists ----------

fn polished_info(s: &Settings, text: &str, elapsed: f32) -> Frame {
    let l = Look::new(s);
    let mut p = design_canvas(s);
    if text.trim().is_empty() {
        return p.finish();
    }
    let list = matches!(s.kind, Kind::EventTicker | Kind::Schedule | Kind::Checklist | Kind::Poll);
    let area = l.panel(&mut p, if s.kind == Kind::AlertBox { "" } else { title_of(s) }, list);
    let fg = if l.retro_window() { l.field_fg } else { l.fg };
    let sec = if l.retro_window() { fade(l.field_fg, 0.65) } else { l.sec };
    let body = body_of(s, text);
    let size = (s.font_size as f32).min(area.w / 12.).clamp(13., 56.);
    match s.kind {
        Kind::Credits => {
            // A centred roll; lines ending in ":" become small headings.
            let rows: Vec<&str> = body.lines().collect();
            let lh = size * 1.55;
            let total = rows.len() as f32 * lh + area.h;
            let offset = (elapsed * 30.) % total.max(1.);
            for (i, row) in rows.iter().enumerate() {
                let y = area.bottom() + i as f32 * lh - offset;
                if y < area.y + size || y > area.bottom() + size {
                    continue;
                }
                let heading = row.trim_end().ends_with(':');
                let fade_edge = ((y - area.y) / (size * 2.)).min((area.bottom() - y) / (size * 2.)).clamp(0., 1.);
                let c = fade(if heading { l.accent } else { fg }, fade_edge);
                if heading {
                    l.text_in(&mut p, Role::Label, row.trim_end_matches(':'), area, y, size * 0.55, c, 0.5);
                } else {
                    l.text_in(&mut p, Role::Title, row, area, y, size, c, 0.5);
                }
            }
        }
        Kind::QrCode => {
            let payload = s.content.trim();
            let side = area.h.min(area.w * 0.42);
            let tile = Area { x: area.right() - side, y: area.y + (area.h - side) / 2., w: side, h: side };
            match qrcode::QrCode::new(payload.as_bytes()).ok().filter(|_| !payload.is_empty() && payload.len() <= 1024) {
                Some(code) => {
                    let r = if matches!(l.theme, Theme::Pixel | Theme::Windows98) { 0. } else { side * 0.06 };
                    p.shadow_rr(tile.x, tile.y + 4., side, side, r, 10., [0, 0, 0, 90]);
                    p.fill_rr(tile.x, tile.y, side, side, r, [255; 4], [255; 4]);
                    let n = code.width() as f32;
                    let unit = side * 0.86 / n;
                    let ox = tile.x + side * 0.07;
                    let oy = tile.y + side * 0.07;
                    for y in 0..code.width() {
                        for x in 0..code.width() {
                            if code[(x, y)] == qrcode::Color::Dark {
                                p.fill_rr(ox + x as f32 * unit, oy + y as f32 * unit, unit + 0.4, unit + 0.4, 0., [14, 16, 22, 255], [14, 16, 22, 255]);
                            }
                        }
                    }
                }
                None => {
                    p.stroke_rr(tile.x, tile.y, side, side, side * 0.06, 2., l.divider);
                    l.text_in(&mut p, Role::Caption, "Add a link", tile, tile.y + side / 2. + 5., 13., l.ter, 0.5);
                }
            }
            let tw = tile.x - area.x - 20.;
            let mut y = area.y + area.h * 0.42;
            for line in l.wrap(Role::Title, if s.label.is_empty() { "Scan me" } else { &s.label }, tw, size).iter().take(2) {
                l.text(&mut p, Role::Title, line, area.x, y, size, fg, tw);
                y += size * 1.2;
            }
            l.text(&mut p, Role::Body, payload.trim_start_matches("https://").trim_start_matches("http://"), area.x, y + size * 0.2, (size * 0.5).max(12.), l.accent, tw);
        }
        _ if list => {
            let lines: Vec<&str> = body.lines().filter(|r| !r.trim().is_empty()).collect();
            let (question, items) = if s.kind == Kind::Poll && lines.len() > 1 { (Some(lines[0]), &lines[1..]) } else { (None, &lines[..]) };
            let mut y = area.y;
            let size = size.min(area.h / (items.len() as f32 * 1.7 + if question.is_some() { 1.6 } else { 0. }).max(1.)).max(12.);
            if let Some(q) = question {
                l.text(&mut p, Role::Title, q, area.x, y + size * 1.05, size * 1.1, fg, area.w);
                y += size * 1.9;
            }
            let row_h = ((area.bottom() - y) / items.len().max(1) as f32).min(size * 2.1);
            for (i, item) in items.iter().enumerate() {
                if y + row_h > area.bottom() + 2. {
                    break;
                }
                let mid = y + row_h / 2.;
                let base = mid + size * 0.36;
                let mut x = area.x;
                match s.kind {
                    Kind::Checklist => {
                        let done = ["[x]", "x ", "✓", "✔"].iter().any(|m| item.trim_start().to_lowercase().starts_with(m));
                        let label = item.trim_start().trim_start_matches(['[', 'x', 'X', ']', '✓', '✔']).trim();
                        let b = size * 0.95;
                        let r = if l.square() { 0. } else { b * 0.28 };
                        if done {
                            p.fill_rr(x, mid - b / 2., b, b, r, lighten(l.accent, 0.15), l.accent);
                            let c = on_color(l.accent);
                            for k in 0..6 {
                                let k = k as f32 / 5.;
                                let (px, py) = if k < 0.4 { (x + b * (0.24 + k * 0.4), mid + b * (-0.02 + k * 0.5)) } else { (x + b * (0.4 + (k - 0.4) * 0.6), mid + b * (0.18 - (k - 0.4) * 0.75)) };
                                p.circle(px, py, b * 0.07, c);
                            }
                        } else {
                            p.stroke_rr(x, mid - b / 2., b, b, r, 2., fade(fg, 0.55));
                        }
                        x += b + size * 0.6;
                        l.text(&mut p, Role::Body, label, x, base, size, if done { sec } else { fg }, area.right() - x);
                    }
                    Kind::Poll => {
                        let letter = ((b'A' + (i as u8).min(25)) as char).to_string();
                        let d = size * 1.4;
                        p.fill_rr(x, mid - d / 2., d, d, if matches!(l.theme, Theme::Pixel | Theme::Windows98) { 0. } else { d * 0.3 }, fade(l.accent, 0.3), fade(l.accent, 0.2));
                        l.text_in(&mut p, Role::Strong, &letter, Area { x, y: mid - d / 2., w: d, h: d }, base, size * 0.8, l.ink(l.accent), 0.5);
                        x += d + size * 0.6;
                        l.text(&mut p, Role::Body, item, x, base, size, fg, area.right() - x);
                    }
                    Kind::Schedule => {
                        let parts: Vec<&str> = item.split(" · ").collect();
                        if parts.len() >= 2 {
                            let tag = (size * 0.62).max(10.);
                            let w = l.width(Role::Label, parts[0], tag) + tag * 1.4;
                            l.pill(&mut p, parts[0], x + w, mid, tag, fade(l.accent, 0.25), l.ink(l.accent));
                            x += w.max(size * 3.6) + size * 0.6;
                            let time = l.text(&mut p, Role::Strong, parts[1], x, base, size, fg, area.w * 0.3);
                            x += time.max(size * 3.2) + size * 0.8;
                            l.text(&mut p, Role::Body, &parts[2..].join(" · "), x, base, size, sec, area.right() - x);
                        } else {
                            l.text(&mut p, Role::Body, item, x, base, size, fg, area.w);
                        }
                    }
                    _ => {
                        p.circle(x + size * 0.25, mid, size * 0.2, l.accent);
                        x += size * 0.9;
                        l.text(&mut p, Role::Body, item, x, base, size, if i == 0 { fg } else { sec }, area.right() - x);
                    }
                }
                if i + 1 < items.len() && !l.retro_window() {
                    p.hline(area.x, y + row_h - 0.5, area.w, l.divider);
                }
                y += row_h;
            }
        }
        _ => {
            // One statement: shoutouts, the latest supporter, announcements, alerts.
            let center = s.kind == Kind::AlertBox;
            let big = (size * 1.25).min(area.h * 0.42);
            let mut lines = l.wrap(Role::Title, body, area.w - if center { 0. } else { 16. }, big);
            lines.truncate(((area.h / (big * 1.25)).floor() as usize).max(1));
            let block = lines.len() as f32 * big * 1.22;
            let mut y = area.y + (area.h - block) / 2. + big * 0.9;
            if !center && !matches!(l.theme, Theme::Windows98 | Theme::WindowsXP | Theme::Terminal | Theme::Led | Theme::Pixel) {
                p.fill_rr(area.x, area.y + (area.h - block) / 2., 4., block, 2., lighten(l.accent, 0.2), l.accent);
            }
            let x = if center || matches!(l.theme, Theme::Windows98 | Theme::WindowsXP | Theme::Terminal | Theme::Led | Theme::Pixel) { area.x } else { area.x + 18. };
            for line in &lines {
                let a = Area { x, w: area.right() - x, ..area };
                let w = l.rich_width(Role::Title, line, big).min(a.w);
                l.rich(&mut p, Role::Title, line, a.x + if center { (a.w - w) / 2. } else { 0. }, y, big, fg, a.w, 0);
                y += big * 1.22;
            }
        }
    }
    p.finish()
}

// ---------- Sports ----------

fn responsive_sports(s: &Settings, text: &str, elapsed: f32) -> Frame {
    let l = Look::new(s);
    let mut p = design_canvas(s);
    let (header, body) = text.split_once('\n').unwrap_or((text, ""));
    let area = l.panel(&mut p, header, false);
    let fg = if l.retro_window() { l.field_fg } else { l.fg };
    let size = (s.font_size as f32).min(area.h * 0.45).clamp(14., 64.);
    if s.sports.layout == ScoreLayout::Ticker {
        let base = area.y + area.h / 2. + size * 0.36;
        let length = l.width(Role::Title, body, size);
        let cycle = length + area.w * 0.2;
        let offset = (elapsed * s.sports.speed as f32) % cycle.max(1.);
        let first = if s.sports.rightward { offset - cycle } else { -offset };
        let mut clip = design_canvas(s);
        for x in [first, first + cycle] {
            l.text(&mut clip, Role::Title, body, area.x + x, base, size, fg, length + 2.);
        }
        // Keep the scrolling text inside the panel, fading at the edges.
        for py in area.y as i32..area.bottom() as i32 {
            for px in area.x as i32..area.right() as i32 {
                let o = ((py as u32 * p.width + px as u32) * 4) as usize;
                let c = [clip.pixels[o], clip.pixels[o + 1], clip.pixels[o + 2], clip.pixels[o + 3]];
                let edge = ((px as f32 - area.x) / 40.).min((area.right() - px as f32) / 40.).clamp(0., 1.);
                if c[3] > 0 {
                    p.blend(px, py, c, edge);
                }
            }
        }
        return p.finish();
    }
    if !body.contains(" · ") {
        let mut y = area.y + area.h * 0.45;
        for line in l.wrap(Role::Body, body, area.w, size * 0.6) {
            l.text_in(&mut p, Role::Body, &line, area, y, size * 0.6, l.sec, 0.5);
            y += size * 0.8;
        }
        return p.finish();
    }
    let rows: Vec<&str> = body.lines().take(5).collect();
    let row_h = area.h / rows.len().max(1) as f32;
    for (i, row) in rows.iter().enumerate() {
        let y = area.y + i as f32 * row_h;
        let parts: Vec<&str> = row.split(" · ").collect();
        let (league, main) = if parts.len() > 1 { (parts[0], parts[1]) } else { ("", *row) };
        let status = parts.iter().skip(2).copied().collect::<Vec<_>>().join(" · ");
        let card = Area { x: area.x, y: y + 3., w: area.w, h: row_h - 6. };
        if rows.len() > 1 {
            p.fill_rr(card.x, card.y, card.w, card.h, (l.radius * 0.5).min(10.), l.surface, fade(l.surface, 0.6));
        }
        let size = (card.h * 0.42).min(size);
        let mid = card.y + card.h / 2.;
        let tag = (size * 0.45).max(10.);
        let mut x = card.x + 12.;
        if !league.is_empty() {
            let w = l.width(Role::Label, league, tag) + tag * 1.4;
            l.pill(&mut p, league, x + w, mid, tag, fade(l.accent, 0.3), l.ink(l.accent));
            x += w + 16.;
        }
        let status_w = if status.is_empty() { 0. } else { l.width(Role::Label, &status, tag) + tag * 1.4 };
        l.text(&mut p, Role::Display, main, x, mid + size * 0.36, size, fg, card.right() - x - status_w - 24.);
        if !status.is_empty() {
            let live = status.contains('Q') || status.contains("Half") || status.contains("OT");
            l.pill(&mut p, &status, card.right() - 12., mid, tag, if live { fade([235, 57, 64, 255], 0.9) } else { l.surface }, if live { [255; 4] } else { l.sec });
        }
    }
    p.finish()
}
