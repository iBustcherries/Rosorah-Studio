// Native widget design primitives. No browser or per-frame image decoding.
impl Paint {
    fn soft_rect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, c: [u8; 4]) {
        if w <= 0. || h <= 0. {
            return;
        }
        let r = r.min(w / 2.).min(h / 2.).max(0.);
        for py in (y.floor() as i32).max(0)..((y + h).ceil() as i32).min(self.height as i32) {
            for px in (x.floor() as i32).max(0)..((x + w).ceil() as i32).min(self.width as i32) {
                let dx = (px as f32 + 0.5 - x - w / 2.).abs() - w / 2. + r;
                let dy = (py as f32 + 0.5 - y - h / 2.).abs() - h / 2. + r;
                let distance = dx.max(0.).hypot(dy.max(0.)) + dx.max(dy).min(0.) - r;
                self.blend(px, py, c, (0.5 - distance).clamp(0., 1.));
            }
        }
    }
    /// One line in a specific font, shortened with "…" to fit `width`.
    #[allow(clippy::too_many_arguments)]
    fn line_font(
        &mut self,
        text: &str,
        x: f32,
        y: f32,
        width: f32,
        size: f32,
        c: [u8; 4],
        f: &FontArc,
        bold: bool,
    ) {
        let scaled = f.as_scaled(PxScale::from(size));
        let measure = |t: &str| {
            t.chars()
                .map(|c| scaled.h_advance(scaled.glyph_id(c)))
                .sum::<f32>()
        };
        let mut text = text.replace('\n', " ");
        if measure(&text) > width {
            while !text.is_empty() && measure(&text) + measure("…") > width {
                text.pop();
            }
            text.push('…');
        }
        self.text_font(&text, x, y, width + 1., size, c, f, bold, y);
    }
    fn finish(self) -> Frame {
        Frame {
            width: self.width,
            height: self.height,
            rgba: self.pixels,
        }
    }
}
/// Splits text into lines that fit `width`, breaking at spaces (or inside a word too long to fit).
fn wrap_lines(text: &str, f: &FontArc, size: f32, width: f32) -> Vec<String> {
    let scaled = f.as_scaled(PxScale::from(size));
    let measure = |t: &str| t.chars().map(|c| scaled.h_advance(scaled.glyph_id(c))).sum::<f32>();
    let mut out = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split(' ') {
            let next = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if measure(&next) <= width || line.is_empty() && measure(word) <= width {
                line = next;
                continue;
            }
            if !line.is_empty() {
                out.push(std::mem::take(&mut line));
            }
            for c in word.chars() {
                if !line.is_empty() && measure(&format!("{line}{c}")) > width {
                    out.push(std::mem::take(&mut line));
                }
                line.push(c);
            }
        }
        out.push(line);
    }
    out
}
/// The Text widget: wrapped, aligned text with an optional outline and drop shadow.
fn text_card(s: &Settings, text: &str) -> Frame {
    let (width, height) = (s.layout_width.clamp(64, 3840), s.layout_height.clamp(32, 2160));
    let mut p = Paint { pixels: vec![0; (width * height * 4) as usize], width, height };
    let f = font_or(&s.text_font, FontFace::Sans);
    let pad = 12. + s.outline_width.min(24) as f32;
    let (box_w, box_h) = (width as f32 - pad * 2., height as f32 - pad * 2.);
    // 0 px fits the largest size whose wrapped lines fill the box.
    let fits = |size: f32| {
        let lines = wrap_lines(text, &f, size, box_w);
        lines.len() as f32 * size * 1.2 <= box_h
    };
    let size = if s.font_size == 0 {
        let (mut lo, mut hi) = (8., 600f32.min(box_h));
        for _ in 0..14 {
            let mid = (lo + hi) / 2.;
            if fits(mid) { lo = mid } else { hi = mid }
        }
        lo
    } else {
        s.font_size as f32
    };
    if s.background[3] > 0 {
        p.soft_rect(0., 0., width as f32, height as f32, s.radius as f32, s.background);
    }
    let scaled = f.as_scaled(PxScale::from(size));
    let measure = |t: &str| t.chars().map(|c| scaled.h_advance(scaled.glyph_id(c))).sum::<f32>();
    // Glyph coverage, drawn once and reused for the shadow, outline and fill.
    let (w, h) = (width as usize, height as usize);
    let mut mask = vec![0f32; w * h];
    for (i, line) in wrap_lines(text, &f, size, box_w).iter().enumerate() {
        let baseline = pad + scaled.ascent() + i as f32 * size * 1.2;
        if baseline - scaled.ascent() > height as f32 {
            break;
        }
        let x = match s.align {
            auroracast_core::widgets::TextAlign::Left => pad,
            auroracast_core::widgets::TextAlign::Center => pad + (box_w - measure(line)) / 2.,
            auroracast_core::widgets::TextAlign::Right => pad + box_w - measure(line),
        };
        let mut px = x;
        for ch in line.chars() {
            let id = scaled.glyph_id(ch);
            if let Some(outline) = f.outline_glyph(id.with_scale_and_position(size, point(px, baseline))) {
                let b = outline.px_bounds();
                outline.draw(|gx, gy, cov| {
                    for dx in 0..=i32::from(s.bold) {
                        let (x, y) = (gx as i32 + b.min.x as i32 + dx, gy as i32 + b.min.y as i32);
                        if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
                            let m = &mut mask[y as usize * w + x as usize];
                            *m = m.max(cov);
                        }
                    }
                });
            }
            px += scaled.h_advance(id);
        }
    }
    let ring = if s.outline_width > 0 { dilate(&mask, w, h, s.outline_width.min(24)) } else { Vec::new() };
    if s.shadow {
        let source = if ring.is_empty() { &mask } else { &ring };
        let offset = (size / 16.).clamp(2., 10.) as usize;
        for y in offset..h {
            for x in offset..w {
                let cov = source[(y - offset) * w + x - offset];
                if cov > 0. {
                    p.blend(x as i32, y as i32, [0, 0, 0, 150], cov);
                }
            }
        }
    }
    for (i, (&fill, &edge)) in mask.iter().zip(ring.iter().chain(std::iter::repeat(&0.))).enumerate() {
        let (x, y) = ((i % w) as i32, (i / w) as i32);
        if edge > 0. {
            p.blend(x, y, s.outline, edge);
        }
        if fill > 0. {
            p.blend(x, y, s.foreground, fill);
        }
    }
    p.finish()
}
/// Grows coverage by `radius` pixels, alternating square and cross steps for a rounded edge.
fn dilate(mask: &[f32], w: usize, h: usize, radius: u32) -> Vec<f32> {
    let mut a = mask.to_vec();
    let mut b = vec![0f32; a.len()];
    for step in 0..radius {
        for y in 0..h {
            for x in 0..w {
                let mut m = a[y * w + x];
                for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)] {
                    if step % 2 == 1 && dx != 0 && dy != 0 {
                        continue;
                    }
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                        m = m.max(a[ny as usize * w + nx as usize]);
                    }
                }
                b[y * w + x] = m;
            }
        }
        std::mem::swap(&mut a, &mut b);
    }
    a
}
pub fn preview(s: &Settings, sample: bool) -> anyhow::Result<Frame> {
    if s.kind == Kind::NowPlaying {
        if sample {
            return Ok(polished_music(s, Some(&crate::music::sample())));
        }
        if s.music.automatic {
            return Ok(polished_music(s, crate::music::current(&s.music).as_ref()));
        }
    }
    if sample {
        if s.kind == Kind::Sports {return Ok(responsive_sports(s,"Sports · Sample scores\nNBA · BOS 108 – 102 NYK · Q4 · 2:14",0.));}
        if s.kind == Kind::Chat {
            return Ok(polished_chat(
                s,
                &format!(
                    "{}\nTwitch · juniper: That transition was so smooth! 🔥\nYouTube · Alex: Loving the stream tonight 😍\nTwitch · mia: One more round? 🎮",
                    s.label
                ),
                0,
            ));
        }
        if s.is_goal() {
            return Ok(polished_goal(s, s.target.saturating_mul(65) / 100));
        }
    }
    let state = State::new(s.clone());
    let text = if sample && s.kind == Kind::AlertBox {
        "Juniper just subscribed!".into()
    } else {
        state.lines()
    };
    render(s, &text)
}

#[cfg(test)]
mod design_tests {
    use super::*;
    #[test]
    fn text_widget_wraps_fits_and_outlines() {
        let mut s = Settings { kind: Kind::Text, content: "Be right back — grabbing snacks".into(), font_size: 0, ..Default::default() };
        s.background = [0; 4];
        let plain = text_card(&s, &s.content);
        assert_eq!(plain.rgba.len(), 960 * 400 * 4);
        let covered = |f: &Frame| f.rgba.chunks_exact(4).filter(|p| p[3] > 0).count();
        assert!(covered(&plain) > 1000);
        s.outline_width = 6;
        assert!(covered(&text_card(&s, &s.content)) > covered(&plain));
        let f = font(FontFace::Sans);
        assert!(wrap_lines("one two three four", f, 40., 120.).len() >= 2);
        assert_eq!(wrap_lines("", f, 40., 120.), [""]);
        let empty = Settings { kind: Kind::Text, background: [0; 4], ..Default::default() };
        assert!(text_card(&empty, "").rgba.iter().all(|b| *b == 0));
    }
    #[test]
    fn no_player_means_transparent_music() {
        let s = Settings {
            kind: Kind::NowPlaying,
            ..Default::default()
        };
        assert!(polished_music(&s, None).rgba.iter().all(|b| *b == 0));
    }
    #[test]
    fn every_theme_and_goal_has_valid_output() {
        for &theme in Theme::ALL {
            let mut s = Settings {
                kind: Kind::Chat,
                ..Default::default()
            };
            s.apply_theme(theme);
            let frame = preview(&s, true).unwrap();
            assert_eq!(frame.rgba.len(), 960 * 400 * 4);
            assert!(frame.rgba.chunks_exact(4).any(|p| p[3] > 0));
        }
        for &style in GoalStyle::ALL {
            let s = Settings {
                kind: Kind::SubGoal,
                goal_style: style,
                target: 0,
                ..Default::default()
            };
            let f = polished_goal(&s, u64::MAX);
            assert_eq!(f.rgba.len(), 960 * 400 * 4);
        }
    }
    #[test]
    #[ignore = "Produces a text widget review image on request"]
    fn export_text_review() {
        let folder = std::path::PathBuf::from(std::env::var("ROSORAH_DESIGN_OUTPUT").expect("design output directory"));
        let font = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fonts/bangers/Bangers-Regular.ttf");
        let s = Settings { kind: Kind::Text, text_font: font.into(), font_size: 0, background: [30, 40, 70, 255],
            outline_width: 5, shadow: true, align: auroracast_core::widgets::TextAlign::Center,
            content: "Be right back!\nGrabbing snacks".into(), ..Default::default() };
        let f = text_card(&s, &s.content);
        image::save_buffer(folder.join("text.png"), &f.rgba, f.width, f.height, image::ColorType::Rgba8).unwrap();
    }
    #[test]
    #[ignore = "Produces design review images on request"]
    fn export_design_review() {
        let folder = std::path::PathBuf::from(
            std::env::var("ROSORAH_DESIGN_OUTPUT").expect("design output directory"),
        );
        std::fs::create_dir_all(&folder).unwrap();
        for (name, kind, theme) in [
            ("music", Kind::NowPlaying, Theme::Material),
            ("unified", Kind::Chat, Theme::Unified),
            ("win98", Kind::Chat, Theme::Windows98),
            ("winxp", Kind::Chat, Theme::WindowsXP),
            ("bubbles", Kind::Chat, Theme::Bubbles),
            ("goal", Kind::SubGoal, Theme::Material),
        ] {
            let mut s = Settings {
                kind,
                label: if kind == Kind::SubGoal {
                    "Our next milestone".into()
                } else {
                    "Live chat".into()
                },
                font_size: if kind == Kind::NowPlaying { 42 } else { 28 },
                ..Default::default()
            };
            s.apply_theme(theme);
            let f = preview(&s, true).unwrap();
            image::save_buffer(
                folder.join(format!("{name}.png")),
                &f.rgba,
                f.width,
                f.height,
                image::ColorType::Rgba8,
            )
            .unwrap();
        }
        let mut project = auroracast_core::Project::default();
        project.preferences.auto_game_capture = false;
        project.sources.clear();
        for scene in &mut project.scenes {
            scene.layers.clear();
        }
        let id = project.id();
        project.sources.push(auroracast_core::Source {
            id,
            name: "Now playing".into(),
            kind: auroracast_core::SourceKind::Widget(Settings {
                kind: Kind::NowPlaying,
                label: "Now playing".into(),
                font_size: 42,
                ..Default::default()
            }),
        });
        project.save(&folder.join("review-project.json")).unwrap();
    }
}
