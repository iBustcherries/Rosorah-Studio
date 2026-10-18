//! Remembers where the studio window was, how big, and whether it was maximised or fullscreen,
//! so it opens the same way next time. Wayland doesn't let apps place windows, so there only
//! the size and state come back; the position is restored on Windows and X11.
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Placement {
    /// Top-left corner of the window, when the system reports it.
    pub position: Option<[f32; 2]>,
    /// Size of the window's contents while it was a normal window (kept while maximised, so
    /// un-maximising returns to it).
    pub size: Option<[f32; 2]>,
    pub maximized: bool,
    pub fullscreen: bool,
}
fn path() -> PathBuf {
    auroracast_core::platform::config_root().join("rosorah-studio/window.json")
}
pub fn load() -> Placement {
    std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}
pub fn save(placement: &Placement) {
    let path = path();
    if let (Some(dir), Ok(bytes)) = (path.parent(), serde_json::to_vec_pretty(placement)) {
        let _ = std::fs::create_dir_all(dir);
        let temp = path.with_extension("tmp");
        if std::fs::write(&temp, bytes).is_ok() {
            let _ = std::fs::rename(temp, path);
        }
    }
}
/// The window as it opens: the remembered size, place and state, within sensible limits.
pub fn apply(builder: egui::ViewportBuilder, placement: &Placement) -> egui::ViewportBuilder {
    let mut builder = builder;
    if let Some([w, h]) = placement.size.filter(|[w, h]| w.is_finite() && h.is_finite()) {
        builder = builder.with_inner_size([w.clamp(1100., 7680.), h.clamp(700., 4320.)]);
    }
    // A position far off any screen (a monitor that was unplugged) is ignored.
    if let Some([x, y]) = placement.position.filter(|[x, y]| x.is_finite() && y.is_finite() && x.abs() < 16_000. && y.abs() < 16_000.) {
        builder = builder.with_position([x, y]);
    }
    builder.with_maximized(placement.maximized).with_fullscreen(placement.fullscreen)
}
/// Updates `placement` from the window as it is now. Returns whether anything changed.
pub fn track(ctx: &egui::Context, placement: &mut Placement) -> bool {
    let before = placement.clone();
    ctx.input(|i| {
        let v = i.viewport();
        if v.minimized == Some(true) {
            return;
        }
        let maximized = v.maximized == Some(true);
        let fullscreen = v.fullscreen == Some(true);
        placement.maximized = maximized;
        placement.fullscreen = fullscreen;
        // Size and place are only taken from a normal window, so restoring it un-maximised works.
        if !maximized && !fullscreen {
            if let Some(inner) = v.inner_rect {
                placement.size = Some([inner.width().round(), inner.height().round()]);
            }
            if let Some(outer) = v.outer_rect {
                placement.position = Some([outer.min.x.round(), outer.min.y.round()]);
            }
        }
    });
    *placement != before
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remembered_state_round_trips_and_bad_values_are_ignored() {
        let placement = Placement { position: Some([100., 50.]), size: Some([1600., 1000.]), maximized: false, fullscreen: true };
        let text = serde_json::to_string(&placement).unwrap();
        assert_eq!(serde_json::from_str::<Placement>(&text).unwrap(), placement);
        assert_eq!(serde_json::from_str::<Placement>("{}").unwrap(), Placement::default());
        let builder = apply(egui::ViewportBuilder::default(), &placement);
        assert_eq!(builder.fullscreen, Some(true));
        assert_eq!(builder.inner_size, Some(egui::vec2(1600., 1000.)));
        let far = Placement { position: Some([90_000., 0.]), size: Some([10., 10.]), ..Default::default() };
        let builder = apply(egui::ViewportBuilder::default(), &far);
        assert_eq!(builder.position, None);
        assert_eq!(builder.inner_size, Some(egui::vec2(1100., 700.)));
    }
}
