//! A floating preview must not feed its canvas' minimum content size back into the window.
use eframe::egui;

pub fn show(
    ctx: &egui::Context,
    open: &mut bool,
    vertical: &mut bool,
    mut canvas: impl FnMut(&mut egui::Ui, bool),
) -> Option<egui::Rect> {
    let screen = ctx.content_rect().size();
    let maximum = egui::vec2(
        (screen.x - 48.).clamp(240., 960.),
        (screen.y - 80.).clamp(180., 700.),
    );
    crate::studio::pop_window(ctx, "Automation preview")
        // Discard the old window's saved auto-expanded geometry once.
        .id(egui::Id::new("automation_preview_bounded_v2"))
        .open(open)
        .default_size(egui::vec2(640., 420.).min(maximum))
        .min_size(egui::vec2(240., 180.).min(maximum))
        .max_size(maximum)
        .resizable(true)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(vertical, false, "Main video");
                ui.selectable_value(vertical, true, "Vertical video");
            });
            let size = ui.available_size().min(maximum).max(egui::vec2(1., 1.));
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            // Unlike allocate_ui/scope, new_child does not propagate content overflow to the parent.
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt("preview_canvas")
                    .max_rect(rect),
            );
            child.set_clip_rect(rect.intersect(ui.clip_rect()));
            child.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            canvas(&mut child, *vertical);
        })
        .map(|response| response.response.rect)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overflowing_canvas_cannot_expand_floating_window() {
        for screen in [egui::vec2(1440., 900.), egui::vec2(800., 600.)] {
            let ctx = egui::Context::default();
            let mut open = true;
            let mut vertical = false;
            let mut settled = None;
            for frame in 0..80 {
                let mut bounds = None;
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, screen)),
                        ..Default::default()
                    },
                    |ctx| {
                        bounds = show(ctx, &mut open, &mut vertical, |ui, _| {
                            // Oversized content reproduces the old min-size feedback regardless of GPU availability.
                            ui.allocate_exact_size(
                                egui::vec2(
                                    ui.available_width() + 100.,
                                    ui.available_height() + 50.,
                                ),
                                egui::Sense::hover(),
                            );
                        });
                    },
                );
                let size = bounds.unwrap().size();
                assert!(
                    size.x < screen.x && size.y < screen.y,
                    "preview escaped the screen: {size:?}"
                );
                if frame == 5 {
                    settled = Some(size);
                }
                if frame > 5 {
                    assert!(
                        (size - settled.unwrap()).length() < 1.,
                        "preview kept growing: {size:?}"
                    );
                }
            }
        }
    }
}
