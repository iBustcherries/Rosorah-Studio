use egui_dock::{DockState, NodeIndex};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Panel {
    Scenes,
    Sources,
    Preview,
    Vertical,
    Mixer,
    Broadcast,
    Chat,
    Activity,
}
impl Panel {
    pub const ALL: [Self; 8] = [
        Self::Scenes,
        Self::Sources,
        Self::Preview,
        Self::Vertical,
        Self::Mixer,
        Self::Broadcast,
        Self::Chat,
        Self::Activity,
    ];
    pub fn title(self) -> &'static str {
        match self {
            Self::Scenes => "Scenes",
            Self::Sources => "Layers",
            Self::Preview => "Main Canvas",
            Self::Vertical => "Vertical Canvas",
            Self::Mixer => "Audio mixer",
            Self::Broadcast => "Broadcast",
            Self::Chat => "Chat",
            Self::Activity => "Activity",
        }
    }
}
#[derive(Serialize, Deserialize)]
struct Saved {
    version: u32,
    dock: DockState<Panel>,
}
pub struct Workspace {
    pub dock: DockState<Panel>,
    path: PathBuf,
    last_check: Instant,
    saved: Vec<u8>,
}
fn default_dock() -> DockState<Panel> {
    let mut dock = DockState::new(vec![Panel::Preview]);
    let [center, left] =
        dock.main_surface_mut()
            .split_left(NodeIndex::root(), 0.14, vec![Panel::Scenes]);
    dock.main_surface_mut()
        .split_below(left, 0.35, vec![Panel::Sources]);
    let [center, right] = dock
        .main_surface_mut()
        .split_right(center, 0.78, vec![Panel::Broadcast]);
    dock.main_surface_mut()
        .split_below(right, 0.50, vec![Panel::Chat, Panel::Activity]);
    let [center, _] = dock
        .main_surface_mut()
        .split_below(center, 0.78, vec![Panel::Mixer]);
    dock.main_surface_mut()
        .split_right(center, 0.73, vec![Panel::Vertical]);
    dock
}
impl Workspace {
    pub fn load(path: PathBuf, window_margin: eframe::egui::Vec2) -> Self {
        let saved = std::fs::read(&path).unwrap_or_default();
        let old_version = serde_json::from_slice::<Saved>(&saved)
            .ok()
            .is_some_and(|s| s.version == 1);
        let mut dock = serde_json::from_slice::<Saved>(&saved)
            .ok()
            .filter(|s| {
                let tabs: Vec<_> = s.dock.iter_all_tabs().map(|(_, t)| *t).collect();
                (1..=3).contains(&s.version)
                    && tabs.contains(&Panel::Preview)
                    && Panel::ALL
                        .iter()
                        .all(|p| tabs.iter().filter(|t| *t == p).count() <= 1)
            })
            .map(|s| s.dock)
            .unwrap_or_else(default_dock);
        if old_version
            && dock.find_tab(&Panel::Vertical).is_none()
            && let Some((surface, node, _)) = dock.find_tab(&Panel::Preview)
        {
            dock[surface].split_right(node, 0.73, vec![Panel::Vertical]);
        }
        if old_version {
            for (_, node) in dock.iter_all_nodes_mut() {
                match node {
                    egui_dock::Node::Vertical(split) if (split.fraction - 0.25).abs() < 0.001 => {
                        split.fraction = 0.50;
                    }
                    egui_dock::Node::Vertical(split) if (split.fraction - 0.65).abs() < 0.001 => {
                        split.fraction = 0.78
                    }
                    egui_dock::Node::Horizontal(split) if (split.fraction - 0.72).abs() < 0.001 => {
                        split.fraction = 0.78
                    }
                    _ => (),
                }
            }
        }
        if serde_json::from_slice::<Saved>(&saved)
            .ok()
            .is_some_and(|s| s.version < 3)
            && dock.find_tab(&Panel::Activity).is_none()
        {
            if let Some((surface, node, _)) = dock.find_tab(&Panel::Chat) {
                dock[surface][node].append_tab(Panel::Activity);
            } else {
                dock.push_to_first_leaf(Panel::Activity);
            }
        }
        // egui window memory is session-local. Restore its position and inner
        // size from the last rendered root rectangle before the first frame.
        for surface in dock.iter_surfaces_mut() {
            if let egui_dock::Surface::Window(tree, state) = surface
                && let Some(rect) = tree.root_node().and_then(|node| node.rect())
                && rect.is_finite()
                && rect.is_positive()
            {
                state
                    .set_position(rect.min - window_margin)
                    .set_size(rect.size());
            }
        }
        Self {
            dock,
            path,
            saved,
            last_check: Instant::now(),
        }
    }
    pub fn contains(&self, panel: Panel) -> bool {
        self.dock.find_tab(&panel).is_some()
    }
    pub fn set_visible(&mut self, panel: Panel, visible: bool) {
        if !visible && panel != Panel::Preview {
            if let Some(location) = self.dock.find_tab(&panel) {
                self.dock.remove_tab(location);
            }
        } else if visible && !self.contains(panel) {
            if panel == Panel::Vertical
                && let Some((surface, node, _)) = self.dock.find_tab(&Panel::Preview)
            {
                self.dock[surface].split_right(node, 0.73, vec![panel]);
            } else {
                self.dock.push_to_first_leaf(panel);
            }
        }
    }
    pub fn toggle_floating(&mut self, panel: Panel, bounds: eframe::egui::Rect) {
        let Some(location) = self.dock.find_tab(&panel) else {
            return;
        };
        if location.0.is_main() {
            let size = eframe::egui::vec2(420., 440.).min(bounds.size() * 0.8);
            let rect = eframe::egui::Rect::from_center_size(bounds.center(), size);
            self.dock.detach_tab(location, rect);
        } else {
            self.dock.remove_tab(location);
            // Find the destination after removal: removing a surface or leaf
            // can change indices. Prefer Preview; otherwise use the main tree.
            if let Some((surface, node, _)) = self.dock.find_tab(&Panel::Preview)
                && surface.is_main()
            {
                self.dock[surface][node].append_tab(panel);
                let last = self.dock[surface][node].tabs_count() - 1;
                self.dock[surface][node]
                    .get_leaf_mut()
                    .unwrap()
                    .set_active_tab(egui_dock::TabIndex(last));
            } else {
                self.dock.push_to_first_leaf(panel);
            }
        }
    }
    pub fn reset(&mut self) {
        self.dock = default_dock();
    }
    pub fn save(&mut self, force: bool) -> anyhow::Result<()> {
        if !force && self.last_check.elapsed() < Duration::from_secs(1) {
            return Ok(());
        }
        self.last_check = Instant::now();
        // Geometry is recomputed each frame. Unshown leaves contain infinite
        // rectangles, which JSON cannot round-trip; persist only finite placeholders.
        let mut dock = self.dock.clone();
        for (_, node) in dock.iter_all_nodes_mut() {
            match node {
                egui_dock::Node::Leaf(leaf) => {
                    if !leaf.rect.is_finite() {
                        leaf.rect = eframe::egui::Rect::ZERO;
                    }
                    if !leaf.viewport.is_finite() {
                        leaf.viewport = eframe::egui::Rect::ZERO;
                    }
                }
                egui_dock::Node::Horizontal(split) | egui_dock::Node::Vertical(split) => {
                    if !split.rect.is_finite() {
                        split.rect = eframe::egui::Rect::ZERO;
                    }
                }
                egui_dock::Node::Empty => {}
            }
        }
        let bytes = serde_json::to_vec(&Saved { version: 3, dock })?;
        if bytes != self.saved {
            if let Some(parent) = self.path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let temp = self.path.with_extension("layout.tmp");
            std::fs::write(&temp, &bytes)?;
            std::fs::rename(temp, &self.path)?;
            self.saved = bytes;
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn float_and_redock_preserves_every_panel() {
        let mut w = Workspace {
            dock: default_dock(),
            path: PathBuf::new(),
            last_check: Instant::now(),
            saved: vec![],
        };
        let bounds = eframe::egui::Rect::from_min_size(
            eframe::egui::pos2(0., 70.),
            eframe::egui::vec2(1100., 600.),
        );
        for panel in Panel::ALL {
            w.toggle_floating(panel, bounds);
            assert!(!w.dock.find_tab(&panel).unwrap().0.is_main());
        }
        for panel in Panel::ALL {
            w.toggle_floating(panel, bounds);
            assert!(w.dock.find_tab(&panel).unwrap().0.is_main());
        }
        assert_eq!(w.dock.iter_all_tabs().count(), Panel::ALL.len());
    }
    #[test]
    fn restore_layout_and_hidden_panels() {
        let path =
            std::env::temp_dir().join(format!("auroracast-layout-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut w = Workspace::load(path.clone(), eframe::egui::Vec2::ZERO);
        assert_eq!(w.dock.iter_all_tabs().count(), Panel::ALL.len());
        w.set_visible(Panel::Chat, false);
        w.set_visible(Panel::Sources, false);
        w.dock.add_window(vec![Panel::Sources]);
        w.save(true).unwrap();
        let mut restored = Workspace::load(path.clone(), eframe::egui::Vec2::ZERO);
        assert!(!restored.contains(Panel::Chat));
        assert!(
            restored.dock.find_tab(&Panel::Sources).unwrap().0 != egui_dock::SurfaceIndex::main()
        );
        restored.set_visible(Panel::Chat, true);
        restored.set_visible(Panel::Chat, true);
        restored.set_visible(Panel::Preview, false);
        assert_eq!(restored.dock.iter_all_tabs().count(), Panel::ALL.len());
        std::fs::write(&path, "invalid").unwrap();
        assert_eq!(
            Workspace::load(path.clone(), eframe::egui::Vec2::ZERO)
                .dock
                .iter_all_tabs()
                .count(),
            Panel::ALL.len()
        );
        std::fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
mod vertical_tests {
    use super::*;
    #[test]
    fn vertical_close_and_restore_are_persisted() {
        let path =
            std::env::temp_dir().join(format!("rosorah-vertical-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut w = Workspace::load(path.clone(), eframe::egui::Vec2::ZERO);
        let main = w.dock.find_tab(&Panel::Preview).unwrap();
        let vertical = w.dock.find_tab(&Panel::Vertical).unwrap();
        assert_ne!(main.1, vertical.1);
        w.set_visible(Panel::Vertical, false);
        w.save(true).unwrap();
        let mut w = Workspace::load(path.clone(), eframe::egui::Vec2::ZERO);
        assert!(!w.contains(Panel::Vertical));
        w.set_visible(Panel::Vertical, true);
        w.set_visible(Panel::Vertical, true);
        assert_eq!(
            w.dock
                .iter_all_tabs()
                .filter(|(_, p)| **p == Panel::Vertical)
                .count(),
            1
        );
        w.save(true).unwrap();
        assert!(Workspace::load(path.clone(), eframe::egui::Vec2::ZERO).contains(Panel::Vertical));
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod activity_migration_tests {
    use super::*;
    #[test]
    fn adds_activity_once_without_resetting_layout_or_reopening_hidden_panel() {
        let path = std::env::temp_dir().join(format!(
            "rosorah-activity-layout-{}.json",
            std::process::id()
        ));
        let mut dock = default_dock();
        dock.remove_tab(dock.find_tab(&Panel::Activity).unwrap());
        std::fs::write(
            &path,
            serde_json::to_vec(&Saved { version: 2, dock }).unwrap(),
        )
        .unwrap();
        let mut w = Workspace::load(path.clone(), eframe::egui::Vec2::ZERO);
        assert!(w.contains(Panel::Activity));
        let a = w.dock.find_tab(&Panel::Activity).unwrap();
        let c = w.dock.find_tab(&Panel::Chat).unwrap();
        assert_eq!((a.0, a.1), (c.0, c.1));
        w.set_visible(Panel::Activity, false);
        w.save(true).unwrap();
        let next = Workspace::load(path.clone(), eframe::egui::Vec2::ZERO);
        assert!(!next.contains(Panel::Activity));
        std::fs::remove_file(path).unwrap();
    }
}
