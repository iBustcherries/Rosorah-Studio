//! Lightweight, immediate-mode Material mixer. No timers or duplicated audio streams.
use eframe::egui::{self, Color32, Pos2, Rect, Vec2};

fn level(peak: f32) -> f32 {
    if !peak.is_finite() || peak <= 0. {
        return 0.;
    }
    ((20. * peak.log10() + 60.) / 60.).clamp(0., 1.)
}

pub fn meter(ui: &mut egui::Ui, peaks: [f32; 2], vertical: bool) {
    meter_sized(ui, peaks, vertical, 150.);
}
pub fn meter_sized(ui: &mut egui::Ui, peaks: [f32; 2], vertical: bool, height: f32) {
    let size = if vertical {
        Vec2::new(52., height)
    } else {
        Vec2::new(ui.available_width().max(48.), 38.)
    };
    let (bounds, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter();
    let track = Color32::from_rgb(44, 43, 53);
    for (channel, peak) in peaks.into_iter().enumerate() {
        let bar = if vertical {
            Rect::from_min_size(
                bounds.min + Vec2::new(30. + channel as f32 * 11., 0.),
                Vec2::new(8., height),
            )
        } else {
            Rect::from_min_size(
                bounds.min + Vec2::new(0., channel as f32 * 11.),
                Vec2::new(bounds.width(), 8.),
            )
        };
        painter.rect_filled(bar, 0., track);
        let value = level(peak);
        for (lo, hi, color) in [
            (0., 0.75, Color32::from_rgb(50, 185, 121)),
            (0.75, 0.9, Color32::from_rgb(231, 189, 65)),
            (0.9, 1., Color32::from_rgb(234, 100, 100)),
        ] {
            let end = value.min(hi);
            if end <= lo {
                continue;
            }
            let fill = if vertical {
                Rect::from_min_max(
                    Pos2::new(bar.left(), bar.bottom() - end * bar.height()),
                    Pos2::new(bar.right(), bar.bottom() - lo * bar.height()),
                )
            } else {
                Rect::from_min_max(
                    Pos2::new(bar.left() + lo * bar.width(), bar.top()),
                    Pos2::new(bar.left() + end * bar.width(), bar.bottom()),
                )
            };
            painter.rect_filled(fill, 0., color);
        }
    }
    for db in [-60, -40, -20, -10, 0] {
        let fraction = (db as f32 + 60.) / 60.;
        let (pos, align) = if vertical {
            (
                Pos2::new(
                    bounds.left() + 25.,
                    bounds.bottom() - fraction * bounds.height(),
                ),
                if db == -60 {
                    egui::Align2::RIGHT_BOTTOM
                } else if db == 0 {
                    egui::Align2::RIGHT_TOP
                } else {
                    egui::Align2::RIGHT_CENTER
                },
            )
        } else {
            (
                Pos2::new(bounds.left() + fraction * bounds.width(), bounds.bottom()),
                if db == -60 {
                    egui::Align2::LEFT_BOTTOM
                } else if db == 0 {
                    egui::Align2::RIGHT_BOTTOM
                } else {
                    egui::Align2::CENTER_BOTTOM
                },
            )
        };
        if !vertical && bounds.width() < 200. && (db == -40 || db == -10) {
            continue;
        }
        painter.text(
            pos,
            align,
            db.to_string(),
            egui::FontId::proportional(11.),
            Color32::from_rgb(188, 182, 201),
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn meter_has_finite_logarithmic_scale() {
        assert_eq!(level(0.), 0.);
        assert_eq!(level(f32::NAN), 0.);
        assert_eq!(level(1.), 1.);
        assert_eq!(level(2.), 1.);
        assert!((level(0.1) - 2. / 3.).abs() < 0.0001);
        assert_eq!(level(0.001), 0.);
    }
}
