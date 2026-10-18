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
    fn line(
        &mut self,
        text: &str,
        x: f32,
        y: f32,
        width: f32,
        size: f32,
        c: [u8; 4],
        face: FontFace,
        bold: bool,
    ) {
        self.line_font(text, x, y, width, size, c, font(face), bold);
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
fn canvas() -> Paint {
    Paint {
        pixels: vec![0; 960 * 400 * 4],
        width: 960,
        height: 400,
    }
}
fn tint(c: [u8; 4], amount: i16) -> [u8; 4] {
    [
        (c[0] as i16 + amount).clamp(0, 255) as u8,
        (c[1] as i16 + amount).clamp(0, 255) as u8,
        (c[2] as i16 + amount).clamp(0, 255) as u8,
        c[3],
    ]
}
fn subdued(c: [u8; 4]) -> [u8; 4] {
    [c[0], c[1], c[2], (c[3] as f32 * 0.65) as u8]
}

fn legacy_chat(s: &Settings, text: &str) -> Frame {
    use auroracast_core::widgets::Layout;
    let mut p = canvas();
    let retro = matches!(s.theme, Theme::Windows98 | Theme::WindowsXP);
    let bubbles = s.theme == Theme::Bubbles || s.layout == Layout::Bubbles;
    let r = if s.theme == Theme::Windows98 {
        0.
    } else {
        s.radius as f32
    };
    p.soft_rect(8., 8., 944., 384., r, s.background);
    let header = if retro {
        48.
    } else if s.label.is_empty() {
        0.
    } else {
        46.
    };
    if retro {
        p.soft_rect(8., 8., 944., 384., r, [130, 133, 141, 255]);
        p.soft_rect(10., 10., 940., 380., r, s.background);
        for y in 0..36 {
            p.rect(13, 13 + y, 934, 1, 0, tint(s.accent, (12 - y / 2) as i16));
        }
        p.line(
            if s.label.is_empty() {
                "Live chat"
            } else {
                &s.label
            },
            28.,
            38.,
            750.,
            24.,
            [255; 4],
            s.font,
            true,
        );
        for (i, glyph) in ["–", "□", "×"].iter().enumerate() {
            let x = 843. + i as f32 * 33.;
            p.soft_rect(
                x,
                19.,
                26.,
                24.,
                if s.theme == Theme::WindowsXP { 3. } else { 0. },
                tint(s.background, 20),
            );
            if i == 1 {
                p.rect(x as i32 + 8, 25, 10, 2, 0, s.foreground);
                p.rect(x as i32 + 8, 33, 10, 2, 0, s.foreground);
                p.rect(x as i32 + 8, 25, 2, 10, 0, s.foreground);
                p.rect(x as i32 + 16, 25, 2, 10, 0, s.foreground);
            } else {
                p.line(glyph, x + 6., 37., 20., 20., s.foreground, s.font, true);
            }
        }
    } else if header > 0. {
        p.line(
            &s.label,
            32.,
            40.,
            880.,
            22.,
            subdued(s.foreground),
            s.font,
            true,
        );
    }
    let mut rows: Vec<_> = text.split('\n').skip(1).filter(|t| !t.is_empty()).collect();
    // Retain the newest messages, then choose their display order.
    if rows.len() > 4 {
        rows = rows[rows.len() - 4..].to_vec();
    }
    if s.reverse {
        rows.reverse();
    }
    let size = (s.font_size as f32).clamp(18., 40.);
    let gap = s.spacing.min(24) as f32;
    let available = 364. - header;
    let row_h = ((available - gap * rows.len().saturating_sub(1) as f32)
        / rows.len().max(1) as f32)
        .min(104.);
    let mut y = 20. + header;
    for (i, row) in rows.iter().enumerate() {
        let (platform, rest) = row.split_once(" · ").unwrap_or(("", row));
        let (author, body) = rest.split_once(": ").unwrap_or(("", rest));
        let accent = if s.theme == Theme::Unified {
            if platform == "Twitch" {
                [179, 137, 255, 255]
            } else {
                [255, 123, 134, 255]
            }
        } else {
            s.accent
        };
        let width = if bubbles {
            (body.chars().count() as f32 * size * 0.48 + 80.).clamp(330., 866.)
        } else {
            896.
        };
        let x = if bubbles && i % 2 == 1 {
            928. - width
        } else {
            32.
        };
        if s.layout != Layout::Lines {
            let fill = if bubbles {
                if i % 2 == 1 {
                    s.accent
                } else {
                    if s.background[3] == 0 {
                        [45, 48, 56, 240]
                    } else {
                        tint(s.background, 26)
                    }
                }
            } else {
                tint(s.background, if retro { 8 } else { 10 })
            };
            p.soft_rect(x, y, width, row_h, if retro { 2. } else { r.max(8.) }, fill);
            if !bubbles {
                p.soft_rect(x, y + 12., 3., row_h - 24., 1.5, accent);
            }
        }
        let fg = if bubbles && i % 2 == 1 {
            [255; 4]
        } else {
            s.foreground
        };
        p.line(
            author,
            x + 18.,
            y + 25.,
            width - 140.,
            18.,
            if bubbles { fg } else { accent },
            s.font,
            true,
        );
        p.line(
            platform,
            x + width - 108.,
            y + 25.,
            90.,
            14.,
            subdued(fg),
            s.font,
            false,
        );
        p.text(
            body,
            x + 18.,
            y + 29. + size,
            width - 36.,
            size,
            fg,
            s.font,
            s.bold,
            y + row_h - 9.,
        );
        y += row_h + gap;
    }
    p.finish()
}

fn legacy_music(s: &Settings, track: Option<&crate::music::Track>) -> Frame {
    let mut p = canvas();
    let Some(track) = track else {
        return p.finish();
    };
    let accent = if s.music.artwork_colors {
        track.accent
    } else {
        s.accent
    };
    let large = s.music.large_artwork;
    let (y, h, art) = if large {
        (16., 368., 320.)
    } else {
        (64., 272., 224.)
    };
    let r = s.radius as f32;
    p.soft_rect(9., y + 5., 942., h, r + 3., [0, 0, 0, 35]);
    p.soft_rect(12., y, 936., h, r, s.background);
    // A quiet edge highlight and album-led color, instead of a flat color block.
    p.soft_rect(
        14.,
        y + 2.,
        932.,
        h - 4.,
        (r - 2.).max(0.),
        tint(s.background, 3),
    );
    let ax = 36.;
    let ay = y + 24.;
    for yy in 0..art as i32 {
        for xx in 0..art as i32 {
            let radius = 12.;
            let dx = ((xx as f32 + 0.5 - art / 2.).abs() - art / 2. + radius).max(0.);
            let dy = ((yy as f32 + 0.5 - art / 2.).abs() - art / 2. + radius).max(0.);
            let coverage = (radius + 0.5 - dx.hypot(dy)).clamp(0., 1.);
            let color = if let Some(frame) = &track.artwork {
                let sx = xx as u32 * frame.width / art as u32;
                let sy = yy as u32 * frame.height / art as u32;
                let o = ((sy * frame.width + sx) * 4) as usize;
                frame.rgba[o..o + 4].try_into().unwrap()
            } else {
                // Abstract fallback cover, also used by the clearly labelled sample.
                let wave = ((xx as f32 / art * 5. + yy as f32 / art * 3.).sin() * 0.2 + 0.6)
                    * (1. - yy as f32 / art * 0.35);
                [
                    (accent[0] as f32 * wave) as u8,
                    (accent[1] as f32 * wave) as u8,
                    (accent[2] as f32 * wave) as u8,
                    255,
                ]
            };
            p.blend(ax as i32 + xx, ay as i32 + yy, color, coverage);
        }
    }
    if track.artwork.is_none() {
        p.soft_rect(
            ax + art * 0.26,
            ay + art * 0.26,
            art * 0.48,
            art * 0.48,
            art * 0.24,
            [255, 255, 255, 35],
        );
        p.soft_rect(
            ax + art * 0.45,
            ay + art * 0.45,
            art * 0.1,
            art * 0.1,
            art * 0.05,
            [255, 255, 255, 170],
        );
    }
    let x = ax + art + 32.;
    let width = 908. - x;
    let top = y + if large { 62. } else { 40. };
    p.soft_rect(x, top - 10., 6., 6., 3., accent);
    p.line(
        if track.playing {
            "NOW PLAYING"
        } else {
            "PAUSED"
        },
        x + 16.,
        top,
        width - 16.,
        16.,
        accent,
        s.font,
        true,
    );
    p.line(
        &track.title,
        x,
        top + 57.,
        width,
        s.font_size.clamp(24, 56) as f32,
        s.foreground,
        s.font,
        true,
    );
    // Defaults match the original layout: artist 26 px, album 20 px (album in large layout).
    let artist_size = if s.music.artist_size > 0 { s.music.artist_size as f32 } else { 26. };
    let artist_y = top + 57. + (artist_size * 1.1 + 8.4).max(37.);
    p.line_font(
        &track.artist,
        x,
        artist_y,
        width,
        artist_size,
        subdued(s.foreground),
        &font_or(&s.music.artist_font, s.font),
        false,
    );
    if large || s.music.show_album {
        let album_size = if s.music.album_size > 0 { s.music.album_size as f32 } else { 20. };
        p.line_font(
            &track.album,
            x,
            artist_y + (album_size * 1.2 + 10.).max(34.),
            width,
            album_size,
            subdued(s.foreground),
            &font_or(&s.music.album_font, s.font),
            false,
        );
    }
    if s.music.progress && track.length > 0. {
        let py = y + h - 68.;
        let progress = track.progress();
        p.soft_rect(x, py, width, 4., 2., subdued(tint(s.background, 70)));
        p.soft_rect(x, py, width * progress, 4., 2., accent);
        let time = |v: f64| format!("{}:{:02}", (v.max(0.) as u64) / 60, (v.max(0.) as u64) % 60);
        p.line(
            &time(progress as f64 * track.length),
            x,
            py + 30.,
            100.,
            17.,
            subdued(s.foreground),
            s.font,
            false,
        );
        p.line(
            &time(track.length),
            x + width - 50.,
            py + 30.,
            60.,
            17.,
            subdued(s.foreground),
            s.font,
            false,
        );
    } else {
        p.line(
            &track.identity,
            x,
            y + h - 35.,
            width,
            17.,
            subdued(s.foreground),
            s.font,
            false,
        );
    }
    p.finish()
}

fn polished_info(s: &Settings, text: &str) -> Frame {
    if s.layout_width!=960||s.layout_height!=400{return responsive_info(s,text)}
    let mut p = canvas();
    if text.is_empty() {
        return p.finish();
    }
    let retro = matches!(s.theme, Theme::Windows98 | Theme::WindowsXP);
    let r = s.radius as f32;
    p.soft_rect(12., 24., 936., 352., r, s.background);
    if retro {
        for y in 0..40 {
            p.rect(16, 28 + y, 928, 1, 0, tint(s.accent, (10 - y / 3) as i16));
        }
        p.line(&s.label, 32., 56., 880., 22., [255; 4], s.font, true);
    } else {
        p.soft_rect(44., 60., 5., 23., 2., s.accent);
        p.line(
            if s.label.is_empty() {
                s.kind.label()
            } else {
                &s.label
            },
            65.,
            80.,
            800.,
            22.,
            subdued(s.foreground),
            s.font,
            true,
        );
    }
    let body = text
        .strip_prefix(&s.label)
        .unwrap_or(text)
        .trim_start_matches([':', '\n', ' ']);
    let body = if body.is_empty() { text } else { body };
    if matches!(s.kind, Kind::Timer | Kind::Counter) {
        p.line(
            body,
            44.,
            228.,
            870.,
            s.font_size.clamp(48, 96) as f32,
            s.foreground,
            s.font,
            true,
        );
        p.line(
            if s.kind == Kind::Timer {
                "TIME"
            } else {
                "TOTAL"
            },
            47.,
            306.,
            800.,
            16.,
            s.accent,
            s.font,
            true,
        );
    } else {
        let size = s.font_size.clamp(20, 48) as f32;
        let mut y = 132.;
        for row in body.lines().take(5) {
            p.text(row, 44., y, 872., size, s.foreground, s.font, s.bold, 350.);
            let scaled = font(s.font).as_scaled(PxScale::from(size));
            let length = row
                .chars()
                .map(|c| scaled.h_advance(scaled.glyph_id(c)))
                .sum::<f32>();
            y += ((length / 872.).ceil().max(1.) * size * 1.3) + 16.;
            if y > 350. {
                break;
            }
            if matches!(s.kind, Kind::EventTicker | Kind::Schedule | Kind::Checklist) {
                p.soft_rect(
                    44.,
                    y - size - 7.,
                    872.,
                    1.,
                    0.,
                    [s.foreground[0], s.foreground[1], s.foreground[2], 22],
                );
            }
        }
    }
    p.finish()
}

fn legacy_goal(s: &Settings, value: u64) -> Frame {
    let mut p = canvas();
    let r = s.radius as f32;
    let c = s.accent;
    let dim = [c[0], c[1], c[2], 45];
    let t = (value as f64 / s.target.max(1) as f64).clamp(0., 1.) as f32;
    p.soft_rect(12., 32., 936., 336., r, s.background);
    p.line(
        &s.label,
        44.,
        89.,
        800.,
        26.,
        subdued(s.foreground),
        s.font,
        true,
    );
    let amount = |v: u64| {
        if s.kind == Kind::DonationGoal {
            format!("{}.{:02}", v / 100, v % 100)
        } else {
            v.to_string()
        }
    };
    let value_text = amount(value);
    let scaled = font(s.font).as_scaled(PxScale::from(64.));
    let value_width = value_text
        .chars()
        .map(|c| scaled.h_advance(scaled.glyph_id(c)))
        .sum::<f32>()
        .min(430.);
    let narrow = matches!(
        s.goal_style,
        GoalStyle::Ring | GoalStyle::Vertical | GoalStyle::Thermometer
    );
    p.line(
        &value_text,
        44.,
        172.,
        if narrow { 450. } else { 680. },
        64.,
        s.foreground,
        s.font,
        true,
    );
    p.line(
        &format!(
            "/ {}{}",
            amount(s.target),
            if s.kind == Kind::DonationGoal {
                format!(" {}", s.currency)
            } else {
                String::new()
            }
        ),
        44. + value_width + 18.,
        168.,
        if narrow { 150. } else { 800. - value_width },
        24.,
        subdued(s.foreground),
        s.font,
        false,
    );
    let caption = if value >= s.target {
        "GOAL REACHED".into()
    } else {
        format!("{}% COMPLETE", (t * 100.).round() as u32)
    };
    p.line(
        &caption,
        44.,
        if narrow { 239. } else { 329. },
        650.,
        17.,
        c,
        s.font,
        true,
    );
    match s.goal_style {
        GoalStyle::Bar => {
            p.soft_rect(44., 232., 872., 18., 9., dim);
            p.soft_rect(44., 232., 872. * t, 18., 9., c);
        }
        GoalStyle::Ring => {
            for y in 107..301 {
                for x in 688..882 {
                    let dx = x as f32 - 785.;
                    let dy = y as f32 - 204.;
                    let d = dx.hypot(dy);
                    let coverage = (9.5 - (d - 84.).abs()).clamp(0., 1.);
                    if coverage > 0. {
                        let angle = (dy.atan2(dx) + std::f32::consts::FRAC_PI_2)
                            .rem_euclid(std::f32::consts::TAU)
                            / std::f32::consts::TAU;
                        p.blend(x, y, if angle < t { c } else { dim }, coverage);
                    }
                }
            }
            p.line(
                &format!("{}%", (t * 100.) as u32),
                747.,
                218.,
                120.,
                36.,
                s.foreground,
                s.font,
                true,
            );
        }
        GoalStyle::Vertical | GoalStyle::Thermometer => {
            p.soft_rect(792., 85., 32., 214., 16., dim);
            p.soft_rect(792., 299. - 214. * t, 32., 214. * t, 16., c);
        }
        GoalStyle::Segments | GoalStyle::Milestones => {
            for i in 0..10 {
                p.soft_rect(
                    44. + i as f32 * 88.,
                    230.,
                    76.,
                    22.,
                    5.,
                    if (i as f32) < t * 10. { c } else { dim },
                );
            }
        }
        GoalStyle::Hearts => {
            for i in 0..10 {
                let color = if (i as f32) < t * 10. { c } else { dim };
                let x = 49. + i as f32 * 87.;
                for yy in 0..48 {
                    for xx in 0..52 {
                        let fx = (xx as f32 - 26.) / 23.;
                        let fy = -(yy as f32 - 20.) / 22.;
                        let a = fx * fx + fy * fy - 1.;
                        let v = a * a * a - fx * fx * fy * fy * fy;
                        let cover = (-v * 12.).clamp(0., 1.);
                        if cover > 0. {
                            p.blend(x as i32 + xx, 220 + yy, color, cover);
                        }
                    }
                }
            }
        }
        GoalStyle::Garden => {
            for i in 0..10 {
                let x = 70. + i as f32 * 86.;
                let color = if (i as f32) < t * 10. { c } else { dim };
                p.soft_rect(x, 244., 3., 31., 1.5, color);
                p.soft_rect(x - 15., 233., 17., 16., 8., color);
                p.soft_rect(x + 1., 225., 19., 21., 9., color);
            }
        }
        GoalStyle::Counter => {}
    }
    p.finish()
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
                    "{}\nTwitch · juniper: That transition was so smooth!\nYouTube · Alex: Loving the stream tonight.\nTwitch · mia: One more round?",
                    s.label
                ),
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
