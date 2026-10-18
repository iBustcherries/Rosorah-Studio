use egui_dock::{DockState, NodeIndex};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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
            Self::Preview => "Canvas 1",
            Self::Vertical => "Canvas 2",
            Self::Mixer => "Audio mixer",
            Self::Broadcast => "Broadcast",
            Self::Chat => "Chat",
            Self::Activity => "Activity",
        }
    }
}
/// Where a panel last sat in the main workspace, so docking it again puts it back there.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Home {
    /// A tab beside these panels, at this position in their tab group.
    Tab { with: Vec<Panel>, index: usize },
    /// Its own area, split against the area holding `beside`, on `side` of it.
    Split { beside: Vec<Panel>, side: Side, fraction: f32 },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Left,
    Right,
    Above,
    Below,
}
#[derive(Serialize, Deserialize)]
struct Saved {
    version: u32,
    dock: DockState<Panel>,
    /// Panels shown in their own native windows instead of the dock.
    #[serde(default)]
    windows: Vec<Panel>,
    #[serde(default)]
    homes: std::collections::BTreeMap<Panel, Home>,
}
pub struct Workspace {
    pub dock: DockState<Panel>,
    pub windows: Vec<Panel>,
    pub homes: std::collections::BTreeMap<Panel, Home>,
    path: PathBuf,
    last_check: Instant,
    saved: Vec<u8>,
}
fn default_dock() -> DockState<Panel> {
    let mut dock = DockState::new(vec![Panel::Preview]);
    let [top, bottom] =
        dock.main_surface_mut()
            .split_below(NodeIndex::root(), 0.55, vec![Panel::Scenes]);
    dock.main_surface_mut()
        .split_right(bottom, 0.19, vec![Panel::Mixer]);
    let [center, _] =
        dock.main_surface_mut()
            .split_left(top, 0.19, vec![Panel::Chat, Panel::Activity]);
    let [center, right] = dock
        .main_surface_mut()
        .split_right(center, 0.77, vec![Panel::Broadcast]);
    dock.main_surface_mut()
        .split_below(right, 0.45, vec![Panel::Sources]);
    dock.main_surface_mut()
        .split_right(center, 0.77, vec![Panel::Vertical]);
    dock
}
/// Every panel in the subtree rooted at `index`.
fn subtree_tabs(tree: &egui_dock::Tree<Panel>, index: NodeIndex) -> Vec<Panel> {
    tree.iter()
        .enumerate()
        .filter(|(i, _)| {
            let mut n = Some(NodeIndex(*i));
            while let Some(at) = n {
                if at == index {
                    return true;
                }
                n = at.parent();
            }
            false
        })
        .filter_map(|(_, node)| node.tabs())
        .flatten()
        .copied()
        .collect()
}
/// The smallest subtree holding all of `nodes`.
fn common_ancestor(nodes: &[NodeIndex]) -> Option<NodeIndex> {
    let path = |n: NodeIndex| {
        let mut chain = vec![n];
        while let Some(p) = chain.last().unwrap().parent() {
            chain.push(p);
        }
        chain
    };
    let first = path(*nodes.first()?);
    first.into_iter().find(|candidate| nodes.iter().all(|n| path(*n).contains(candidate)))
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
                (1..=5).contains(&s.version)
                    && tabs.contains(&Panel::Preview)
                    && Panel::ALL
                        .iter()
                        .all(|p| tabs.iter().filter(|t| *t == p).count() <= 1)
            })
            .map(|s| s.dock)
            .unwrap_or_else(default_dock);
        let mut windows = Vec::new();
        for panel in serde_json::from_slice::<Saved>(&saved).map(|s| s.windows).unwrap_or_default() {
            if panel != Panel::Preview && !windows.contains(&panel) && dock.find_tab(&panel).is_none() {
                windows.push(panel);
            }
        }
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
        if serde_json::from_slice::<Saved>(&saved)
            .ok()
            .is_some_and(|s| s.version == 3)
        {
            let backup = path.with_extension("pre-019.json");
            if !backup.exists() {
                let _ = std::fs::write(&backup, &saved);
            }
            let hidden: Vec<_> = Panel::ALL
                .into_iter()
                .filter(|p| dock.find_tab(p).is_none())
                .collect();
            dock = default_dock();
            for panel in hidden {
                if let Some(at) = dock.find_tab(&panel) {
                    dock.remove_tab(at);
                }
            }
        }
        if serde_json::from_slice::<Saved>(&saved)
            .ok()
            .is_some_and(|s| s.version == 4)
        {
            let backup = path.with_extension("pre-020.json");
            if !backup.exists() {
                let _ = std::fs::write(&backup, &saved);
            }
            let hidden: Vec<_> = Panel::ALL
                .into_iter()
                .filter(|p| dock.find_tab(p).is_none())
                .collect();
            dock = default_dock();
            for panel in hidden {
                if let Some(at) = dock.find_tab(&panel) {
                    dock.remove_tab(at);
                }
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
        let homes = serde_json::from_slice::<Saved>(&saved).map(|s| s.homes).unwrap_or_default();
        Self {
            dock,
            windows,
            homes,
            path,
            saved,
            last_check: Instant::now(),
        }
    }
    pub fn contains(&self, panel: Panel) -> bool {
        self.dock.find_tab(&panel).is_some() || self.windows.contains(&panel)
    }
    /// Moves a docked or floating panel into its own native window.
    pub fn pop_out(&mut self, panel: Panel) {
        if panel == Panel::Preview || self.windows.contains(&panel) {
            return;
        }
        if let Some(location) = self.dock.find_tab(&panel) {
            self.dock.remove_tab(location);
        }
        self.windows.push(panel);
    }
    /// Returns a panel from its own window to where it last sat in the workspace.
    pub fn dock_window(&mut self, panel: Panel) {
        if !self.windows.contains(&panel) {
            return;
        }
        self.windows.retain(|p| *p != panel);
        self.dock_home(panel);
    }
    /// Records where every panel in the main workspace sits. Called before the dock is
    /// drawn, so a panel dragged, floated or popped out keeps its last docked position.
    pub fn remember_homes(&mut self) {
        let tree = self.dock.main_surface();
        let mut homes = Vec::new();
        for (index, node) in tree.iter().enumerate() {
            let Some(tabs) = node.tabs() else { continue };
            let index = NodeIndex(index);
            for (position, panel) in tabs.iter().enumerate() {
                let others: Vec<_> = tabs.iter().filter(|t| *t != panel).copied().collect();
                let home = if !others.is_empty() {
                    Home::Tab { with: others, index: position }
                } else if let Some(parent) = index.parent() {
                    let left = index == parent.left();
                    let sibling = if left { parent.right() } else { parent.left() };
                    let (fraction, vertical) = match &tree[parent] {
                        egui_dock::Node::Horizontal(split) => (split.fraction, false),
                        egui_dock::Node::Vertical(split) => (split.fraction, true),
                        _ => continue,
                    };
                    let side = match (vertical, left) {
                        (false, true) => Side::Left,
                        (false, false) => Side::Right,
                        (true, true) => Side::Above,
                        (true, false) => Side::Below,
                    };
                    Home::Split { beside: subtree_tabs(tree, sibling), side, fraction }
                } else {
                    continue;
                };
                homes.push((*panel, home));
            }
        }
        self.homes.extend(homes);
    }
    /// Puts a panel that is not in the dock back at its remembered home, or beside Canvas 1.
    fn dock_home(&mut self, panel: Panel) {
        let main = egui_dock::SurfaceIndex::main();
        match self.homes.get(&panel).cloned() {
            Some(Home::Tab { with, index }) => {
                if let Some((_, node, _)) = with.iter().filter_map(|p| self.dock.find_tab(p)).find(|(s, _, _)| s.is_main()) {
                    let leaf = self.dock[main][node].get_leaf_mut().unwrap();
                    let at = index.min(leaf.tabs.len());
                    leaf.tabs.insert(at, panel);
                    leaf.set_active_tab(egui_dock::TabIndex(at));
                    return;
                }
            }
            Some(Home::Split { beside, side, fraction }) => {
                let leaves: Vec<_> = beside.iter().filter_map(|p| self.dock.find_tab(p)).filter(|(s, _, _)| s.is_main()).map(|(_, n, _)| n).collect();
                if let Some(target) = common_ancestor(&leaves) {
                    let split = match side {
                        Side::Left => egui_dock::Split::Left,
                        Side::Right => egui_dock::Split::Right,
                        Side::Above => egui_dock::Split::Above,
                        Side::Below => egui_dock::Split::Below,
                    };
                    self.dock[main].split(target, split, fraction.clamp(0.05, 0.95), egui_dock::Node::leaf(panel));
                    return;
                }
            }
            None => (),
        }
        if let Some((surface, node, _)) = self.dock.find_tab(&Panel::Preview)
            && surface.is_main()
        {
            self.dock[surface][node].append_tab(panel);
        } else {
            self.dock.push_to_first_leaf(panel);
        }
    }
    pub fn set_visible(&mut self, panel: Panel, visible: bool) {
        if !visible {
            self.windows.retain(|p| *p != panel);
        }
        if !visible && panel != Panel::Preview {
            if let Some(location) = self.dock.find_tab(&panel) {
                self.dock.remove_tab(location);
            }
        } else if visible && !self.contains(panel) {
            if self.homes.contains_key(&panel) {
                self.dock_home(panel);
            } else if panel == Panel::Vertical
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
            self.dock_home(panel);
        }
    }
    pub fn reset(&mut self) {
        self.dock = default_dock();
        self.windows.clear();
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
        let bytes = serde_json::to_vec(&Saved { version: 5, dock, windows: self.windows.clone(), homes: self.homes.clone() })?;
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
    fn docking_again_returns_every_panel_to_its_home() {
        let mut w = Workspace { dock: default_dock(), windows: vec![], homes: Default::default(), path: PathBuf::new(), last_check: Instant::now(), saved: vec![] };
        let layout = |w: &Workspace| -> Vec<(usize, Vec<Panel>)> {
            w.dock.main_surface().iter().enumerate().filter_map(|(i, n)| n.tabs().map(|t| (i, t.to_vec()))).collect()
        };
        let original = layout(&w);
        for panel in Panel::ALL.into_iter().filter(|p| *p != Panel::Preview) {
            w.remember_homes();
            w.pop_out(panel);
            assert_ne!(layout(&w), original);
            w.dock_window(panel);
            assert_eq!(layout(&w), original, "{panel:?} did not return home");
        }
    }
    #[test]
    fn native_windows_round_trip_and_redock() {
        let path = std::env::temp_dir().join(format!("rosorah-windows-{}.json", std::process::id()));
        let mut w = Workspace::load(path.clone(), eframe::egui::Vec2::ZERO);
        w.pop_out(Panel::Chat);
        w.pop_out(Panel::Preview);
        assert_eq!(w.windows, [Panel::Chat]);
        assert!(w.contains(Panel::Chat) && w.dock.find_tab(&Panel::Chat).is_none());
        w.save(true).unwrap();
        let mut w = Workspace::load(path.clone(), eframe::egui::Vec2::ZERO);
        assert_eq!(w.windows, [Panel::Chat]);
        w.dock_window(Panel::Chat);
        assert!(w.windows.is_empty() && w.dock.find_tab(&Panel::Chat).is_some());
        w.pop_out(Panel::Mixer);
        w.set_visible(Panel::Mixer, false);
        assert!(!w.contains(Panel::Mixer));
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn float_and_redock_preserves_every_panel() {
        let mut w = Workspace {
            dock: default_dock(),
            windows: vec![],
            homes: Default::default(),
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
        // Real saved layouts have finite geometry. An unrendered tree contains
        // infinities, which serialize to null and would test reset, not migration.
        for (_, node) in dock.iter_all_nodes_mut() {
            node.set_rect(eframe::egui::Rect::ZERO);
            if let Some(leaf) = node.get_leaf_mut() {
                leaf.viewport = eframe::egui::Rect::ZERO;
            }
        }
        std::fs::write(
            &path,
            serde_json::to_vec(&Saved { version: 2, dock, windows: vec![], homes: Default::default() }).unwrap(),
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
