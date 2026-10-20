//! A GNOME (Adwaita) style title bar. GNOME doesn't draw title bars for apps, so there Rosorah
//! turns the window frame off and its header becomes the title bar, as in GNOME's own apps.
//! Other desktops (KDE and the rest) and Windows keep drawing their own frame.
use auroracast_core::workspace::TitleBar;
use eframe::egui::{self, CursorIcon, ResizeDirection, Sense, ViewportCommand, vec2};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Button {
    Minimize,
    Maximize,
    Close,
}
/// Window buttons on each side of the header, as GNOME is set up to show them.
#[derive(Debug, PartialEq)]
pub struct Buttons {
    pub left: Vec<Button>,
    pub right: Vec<Button>,
}
/// True when the session's desktop is GNOME (Ubuntu's default session reports "ubuntu:GNOME").
/// GNOME Flashback draws its own title bars, so it doesn't count.
pub fn gnome() -> bool {
    !cfg!(windows) && std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| on_gnome(&d))
}
fn on_gnome(desktop: &str) -> bool {
    let parts = || desktop.split(':');
    parts().any(|d| d.eq_ignore_ascii_case("GNOME")) && !parts().any(|d| d.eq_ignore_ascii_case("GNOME-Flashback"))
}
/// Whether Rosorah draws the title bar itself.
pub fn wanted(setting: TitleBar) -> bool {
    match setting {
        TitleBar::Automatic => gnome(),
        TitleBar::Always => !cfg!(windows),
        TitleBar::Never => false,
    }
}
pub fn buttons() -> &'static Buttons {
    static BUTTONS: OnceLock<Buttons> = OnceLock::new();
    BUTTONS.get_or_init(|| {
        let layout = std::process::Command::new("gsettings")
            .args(["get", "org.gnome.desktop.wm.preferences", "button-layout"])
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
        parse(layout.as_deref().unwrap_or(":minimize,maximize,close"))
    })
}
/// Reads GNOME's button-layout setting, such as 'appmenu:close' or ':minimize,maximize,close'.
fn parse(layout: &str) -> Buttons {
    let layout = layout.trim().trim_matches('\'');
    let (left, right) = layout.split_once(':').unwrap_or(("", layout));
    let side = |s: &str| {
        s.split(',')
            .filter_map(|b| match b.trim() {
                "minimize" => Some(Button::Minimize),
                "maximize" => Some(Button::Maximize),
                "close" => Some(Button::Close),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let buttons = Buttons { left: side(left), right: side(right) };
    // A layout without a close button would leave no way to close the window from the header.
    if buttons.left.contains(&Button::Close) || buttons.right.contains(&Button::Close) {
        buttons
    } else {
        Buttons { left: buttons.left, right: [buttons.right, vec![Button::Close]].concat() }
    }
}
/// Makes the header's empty space move the window, and a double-click maximize it.
/// Call before adding the header's own widgets, which then sit on top.
pub fn drag_area(ui: &mut egui::Ui) {
    let response = ui.interact(ui.max_rect(), ui.id().with("title-bar-drag"), Sense::click_and_drag());
    if response.double_clicked() {
        let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx().send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    } else if response.drag_started_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
}
/// Adwaita's round window buttons, in the given order.
pub fn show_buttons(ui: &mut egui::Ui, buttons: &[Button]) {
    for &button in buttons {
        let (rect, response) = ui.allocate_exact_size(vec2(24., 24.), Sense::click());
        let visuals = ui.visuals();
        let fill = if response.is_pointer_button_down_on() {
            visuals.widgets.active.weak_bg_fill
        } else if response.hovered() {
            visuals.widgets.hovered.weak_bg_fill
        } else {
            visuals.widgets.inactive.weak_bg_fill
        };
        let stroke = egui::Stroke::new(1.5_f32, visuals.strong_text_color());
        let painter = ui.painter();
        let c = rect.center();
        painter.circle_filled(c, 12., fill);
        let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
        match button {
            Button::Minimize => {
                painter.line_segment([c + vec2(-4., 3.5), c + vec2(4., 3.5)], stroke);
            }
            Button::Maximize if maximized => {
                // Restore: two overlapping squares.
                painter.rect_stroke(egui::Rect::from_center_size(c + vec2(-1., 1.), vec2(7., 7.)), 1., stroke, egui::StrokeKind::Middle);
                painter.line_segment([c + vec2(-1.5, -4.5), c + vec2(4.5, -4.5)], stroke);
                painter.line_segment([c + vec2(4.5, -4.5), c + vec2(4.5, 1.5)], stroke);
            }
            Button::Maximize => {
                painter.rect_stroke(egui::Rect::from_center_size(c, vec2(8., 8.)), 1., stroke, egui::StrokeKind::Middle);
            }
            Button::Close => {
                painter.line_segment([c + vec2(-4., -4.), c + vec2(4., 4.)], stroke);
                painter.line_segment([c + vec2(4., -4.), c + vec2(-4., 4.)], stroke);
            }
        }
        let response = response.on_hover_text(match button {
            Button::Minimize => "Minimize",
            Button::Maximize if maximized => "Restore",
            Button::Maximize => "Maximize",
            Button::Close => "Close",
        });
        if response.clicked() {
            ui.ctx().send_viewport_cmd(match button {
                Button::Minimize => ViewportCommand::Minimized(true),
                Button::Maximize => ViewportCommand::Maximized(!maximized),
                Button::Close => ViewportCommand::Close,
            });
        }
        ui.add_space(4.);
    }
}
/// Without a window frame the window's edges have to offer resizing themselves.
/// Call after everything else is drawn, so the resize pointer wins at the very edge.
pub fn resize_edges(ctx: &egui::Context) {
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false) || i.viewport().fullscreen.unwrap_or(false)) {
        return;
    }
    let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else { return };
    let rect = ctx.content_rect();
    let edge = 6.;
    let corner = 16.;
    let (left, right) = (pos.x - rect.left() < edge, rect.right() - pos.x < edge);
    let (top, bottom) = (pos.y - rect.top() < edge, rect.bottom() - pos.y < edge);
    // Corners are easier to grab a little further along each edge.
    let (near_left, near_right) = (pos.x - rect.left() < corner, rect.right() - pos.x < corner);
    let (near_top, near_bottom) = (pos.y - rect.top() < corner, rect.bottom() - pos.y < corner);
    let direction = match () {
        _ if (top && near_left) || (left && near_top) => ResizeDirection::NorthWest,
        _ if (top && near_right) || (right && near_top) => ResizeDirection::NorthEast,
        _ if (bottom && near_left) || (left && near_bottom) => ResizeDirection::SouthWest,
        _ if (bottom && near_right) || (right && near_bottom) => ResizeDirection::SouthEast,
        _ if top => ResizeDirection::North,
        _ if bottom => ResizeDirection::South,
        _ if left => ResizeDirection::West,
        _ if right => ResizeDirection::East,
        _ => return,
    };
    ctx.set_cursor_icon(match direction {
        ResizeDirection::North => CursorIcon::ResizeNorth,
        ResizeDirection::South => CursorIcon::ResizeSouth,
        ResizeDirection::East => CursorIcon::ResizeEast,
        ResizeDirection::West => CursorIcon::ResizeWest,
        ResizeDirection::NorthEast => CursorIcon::ResizeNorthEast,
        ResizeDirection::NorthWest => CursorIcon::ResizeNorthWest,
        ResizeDirection::SouthEast => CursorIcon::ResizeSouthEast,
        ResizeDirection::SouthWest => CursorIcon::ResizeSouthWest,
    });
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(direction));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_gnome_desktops_and_button_layouts() {
        assert!(on_gnome("GNOME") && on_gnome("ubuntu:GNOME") && on_gnome("GNOME-Classic:GNOME"));
        assert!(!on_gnome("KDE") && !on_gnome("X-Cinnamon") && !on_gnome("Budgie:GNOME-Flashback") && !on_gnome("COSMIC") && !on_gnome(""));
        assert!(!on_gnome("GNOME-Flashback:GNOME"));
        assert_eq!(parse("'appmenu:close'"), Buttons { left: vec![], right: vec![Button::Close] });
        assert_eq!(parse("'icon:minimize,maximize,close'\n").right, [Button::Minimize, Button::Maximize, Button::Close]);
        assert_eq!(parse("'close,minimize:'"), Buttons { left: vec![Button::Close, Button::Minimize], right: vec![] });
        assert_eq!(parse("'appmenu:'").right, [Button::Close]);
        assert!(!wanted(TitleBar::Never));
    }
}
