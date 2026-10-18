//! Saved editing preferences; never evaluated by the frame capture hot path.
use crate::{Id, Project, Scene};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspaceSettings {
    pub scenes: BTreeMap<Id, Vec<Id>>,
    pub scene_links: BTreeMap<Id, Id>,
    pub link_scenes: bool,
    pub vertical_audio: bool,
    pub animations: bool,
    pub chat_destinations: [bool; 2],
    /// Kept separate from the Twitch/YouTube pair so existing projects keep loading.
    pub kick_chat: bool,
    pub appearance: Appearance,
    /// Shows every option (tracks, docking, encoders…). Off keeps the studio simple.
    pub advanced: bool,
    /// Shows the second (vertical) video in the studio.
    pub vertical_video: bool,
    /// Studio Mode: scenes are prepared in a preview, then sent live with Transition.
    pub studio_mode: bool,
    /// Web pages added as panels from the Panels menu.
    pub browser_docks: Vec<BrowserDock>,
    /// Looks for a newer Rosorah when the studio opens.
    pub check_updates: bool,
    /// A version the user chose to skip; its reminder stays hidden.
    pub skipped_update: String,
    /// Sound panel strips hidden from view (still heard on stream).
    pub hidden_sounds: BTreeSet<Id>,
    /// Names given to sound panel strips; the channel keeps its own name underneath.
    pub sound_names: BTreeMap<Id, String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrowserDock {
    pub id: u64,
    pub name: String,
    pub url: String,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Backdrop {
    /// The theme's own background color.
    #[default]
    None,
    Color,
    Picture,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackdropFit {
    /// Cover the window, cropping the picture's edges.
    #[default]
    Fill,
    /// Show the whole picture, with bars where needed.
    Fit,
    Stretch,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PanelStyle {
    #[default]
    Solid,
    Transparent,
    Frosted,
    Matte,
    Liquid,
}
/// Studio look: theme, what sits behind the panels, and how the panels show it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub theme: ThemeMode,
    pub backdrop: Backdrop,
    pub backdrop_color: [u8; 3],
    pub backdrop_image: String,
    pub backdrop_fit: BackdropFit,
    pub panels: PanelStyle,
    /// 0 = opaque panels, 1 = fully see-through.
    pub transparency: f32,
    /// 0 = most blurred, 1 = sharp.
    pub clarity: f32,
    /// A stronger tint so text stays readable over busy pictures.
    pub contrast_boost: bool,
}
impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: ThemeMode::Dark,
            backdrop: Backdrop::None,
            backdrop_color: [40, 32, 72],
            backdrop_image: String::new(),
            backdrop_fit: BackdropFit::Fill,
            panels: PanelStyle::Solid,
            transparency: 0.4,
            clarity: 0.5,
            contrast_boost: false,
        }
    }
}
impl Default for WorkspaceSettings {
    fn default() -> Self {
        Self {
            scenes: BTreeMap::new(),
            scene_links: BTreeMap::new(),
            link_scenes: true,
            vertical_audio: false,
            animations: true,
            chat_destinations: [true, true],
            kick_chat: true,
            appearance: Appearance::default(),
            advanced: false,
            vertical_video: true,
            studio_mode: false,
            browser_docks: Vec::new(),
            check_updates: true,
            skipped_update: String::new(),
            hidden_sounds: BTreeSet::new(),
            sound_names: BTreeMap::new(),
        }
    }
}
impl Project {
    /// One-time migration retains every scene, separating any shared active scene.
    pub fn prepare_workspace(&mut self) {
        if !self.preferences.workspace.scenes.is_empty() {
            return;
        }
        let mut assigned = BTreeSet::new();
        for index in 0..self.canvases.len() {
            let mut scene = self.canvases[index].scene;
            if assigned.contains(&scene) {
                let original = self.scene(scene).cloned().unwrap();
                scene = self.id();
                self.scenes.push(Scene {
                    id: scene,
                    ..original
                });
                if let Some(audio) = self
                    .preferences
                    .scene_audio
                    .get(&self.canvases[index].scene)
                    .cloned()
                {
                    self.preferences.scene_audio.insert(scene, audio);
                }
                self.canvases[index].scene = scene;
            }
            assigned.insert(scene);
            self.preferences
                .workspace
                .scenes
                .insert(self.canvases[index].id, vec![scene]);
        }
        if let Some(first) = self.canvases.first() {
            let rest: Vec<_> = self
                .scenes
                .iter()
                .filter(|s| !assigned.contains(&s.id))
                .map(|s| s.id)
                .collect();
            self.preferences
                .workspace
                .scenes
                .get_mut(&first.id)
                .unwrap()
                .extend(rest);
        }
        // Existing vertical layout stays intact; new alternatives share source assets,
        // but have independent layer transforms and scene audio values.
        if self.canvases.len() > 1 {
            let main = self.canvases[0].id;
            let vertical = self.canvases[1].id;
            let first = self.canvases[0].scene;
            let counterpart = self.canvases[1].scene;
            self.preferences
                .workspace
                .scene_links
                .insert(first, counterpart);
            for original in self
                .canvas_scenes(main)
                .into_iter()
                .filter(|id| *id != first)
            {
                let mut copy = self.scene(original).unwrap().clone();
                copy.id = self.id();
                let id = copy.id;
                self.scenes.push(copy);
                self.preferences
                    .workspace
                    .scenes
                    .get_mut(&vertical)
                    .unwrap()
                    .push(id);
                self.preferences.workspace.scene_links.insert(original, id);
                if let Some(audio) = self.preferences.scene_audio.get(&original).cloned() {
                    self.preferences.scene_audio.insert(id, audio);
                }
            }
        }
    }
    pub fn canvas_scenes(&self, canvas: Id) -> Vec<Id> {
        self.preferences
            .workspace
            .scenes
            .get(&canvas)
            .cloned()
            .unwrap_or_else(|| self.scenes.iter().map(|s| s.id).collect())
    }
    pub fn switch_scene(&mut self, canvas: Id, scene: Id) {
        if !self.canvas_scenes(canvas).contains(&scene) || self.scene(scene).is_none() {
            return;
        }
        if let Some(c) = self.canvases.iter_mut().find(|c| c.id == canvas) {
            c.scene = scene;
        }
        if self.preferences.workspace.link_scenes
            && self.canvases.first().is_some_and(|c| c.id == canvas)
            && let Some(linked) = self.preferences.workspace.scene_links.get(&scene).copied()
            && self.scene(linked).is_some()
        {
            for c in self.canvases.iter_mut().skip(1) {
                if self
                    .preferences
                    .workspace
                    .scenes
                    .get(&c.id)
                    .is_some_and(|ids| ids.contains(&linked))
                {
                    c.scene = linked;
                }
            }
        }
    }
    pub fn duplicate_scene(&mut self, canvas: Id, original: Id) -> Option<Id> {
        if !self.canvas_scenes(canvas).contains(&original) {
            return None;
        }
        let mut scene = self.scene(original)?.clone();
        scene.id = self.id();
        let base = format!("{} copy", scene.name);
        scene.name = base.clone();
        let mut n = 2;
        while self.scenes.iter().any(|s| s.name == scene.name) {
            scene.name = format!("{base} {n}");
            n += 1;
        }
        let id = scene.id;
        self.scenes.push(scene);
        let ids = self.preferences.workspace.scenes.get_mut(&canvas)?;
        let index = ids.iter().position(|i| *i == original)?;
        ids.insert(index + 1, id);
        if let Some(audio) = self.preferences.scene_audio.get(&original).cloned() {
            self.preferences.scene_audio.insert(id, audio);
        }
        Some(id)
    }
    pub fn delete_scene(&mut self, canvas: Id, scene: Id) -> bool {
        let ids = self.canvas_scenes(canvas);
        if ids.len() < 2 || !ids.contains(&scene) {
            return false;
        }
        // A project without saved scene lists uses every scene; write that list down first.
        let list = self.preferences.workspace.scenes.entry(canvas).or_insert(ids);
        list.retain(|id| *id != scene);
        let fallback = list[0];
        for c in &mut self.canvases {
            if c.scene == scene {
                c.scene = fallback;
            }
        }
        self.scenes.retain(|s| s.id != scene);
        self.preferences.scene_audio.remove(&scene);
        self.preferences
            .workspace
            .scene_links
            .retain(|a, b| *a != scene && *b != scene);
        for rule in &mut self.rules {
            rule.actions
                .retain(|a| !matches!(a,crate::Action::SwitchScene{scene:id,..} if *id==scene));
        }
        true
    }
    pub fn move_scene(&mut self, canvas: Id, scene: Id, to: usize) {
        if let Some(ids) = self.preferences.workspace.scenes.get_mut(&canvas)
            && let Some(from) = ids.iter().position(|id| *id == scene)
        {
            let id = ids.remove(from);
            ids.insert(to.min(ids.len()), id);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scenes_delete_without_saved_scene_lists() {
        let mut p = Project::default();
        let id = p.id();
        p.scenes.push(Scene { id, name: "Imported".into(), layers: vec![] });
        assert!(p.preferences.workspace.scenes.is_empty());
        let canvas = p.canvases[0].id;
        assert!(p.delete_scene(canvas, id));
        assert!(p.scene(id).is_none() && !p.canvas_scenes(canvas).contains(&id));
    }
    #[test]
    fn scene_lifecycle_keeps_links_stable() {
        let mut p = Project::default();
        p.prepare_workspace();
        let main = p.canvases[0].id;
        let vert = p.canvases[1].id;
        let a = p.canvases[0].scene;
        let b = p.canvases[1].scene;
        let copy = p.duplicate_scene(main, a).unwrap();
        p.preferences.workspace.scene_links.insert(copy, b);
        p.move_scene(main, copy, 0);
        p.switch_scene(main, copy);
        assert_eq!(p.canvases[1].scene, b);
        p.preferences.workspace.link_scenes = false;
        let vc = p.duplicate_scene(vert, b).unwrap();
        p.switch_scene(vert, vc);
        p.switch_scene(main, a);
        assert_eq!(p.canvases[1].scene, vc);
        assert!(p.delete_scene(main, copy));
        assert!(!p.preferences.workspace.scene_links.contains_key(&copy));
        p.validate().unwrap();
    }
}

impl Project {
    pub(crate) fn validate_workspace(&self) -> anyhow::Result<()> {
        let w = &self.preferences.workspace;
        if w.scenes.is_empty() {
            return Ok(());
        }
        let mut owned = BTreeSet::new();
        for c in &self.canvases {
            let Some(ids) = w.scenes.get(&c.id) else {
                anyhow::bail!("Missing scenes for canvas {}", c.name);
            };
            if ids.is_empty() || !ids.contains(&c.scene) {
                anyhow::bail!("Invalid scene selection for {}", c.name);
            }
            for id in ids {
                if self.scene(*id).is_none() || !owned.insert(*id) {
                    anyhow::bail!("Invalid or shared canvas scene {id}");
                }
            }
        }
        for (from, to) in &w.scene_links {
            if !self
                .canvases
                .first()
                .is_some_and(|c| w.scenes[&c.id].contains(from))
                || !self
                    .canvases
                    .iter()
                    .skip(1)
                    .any(|c| w.scenes[&c.id].contains(to))
            {
                anyhow::bail!("Invalid linked scenes");
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod migration_tests {
    use super::*;
    #[test]
    fn migration_retains_originals_and_is_idempotent() {
        let mut p = Project::default();
        let old: Vec<_> = p.scenes.iter().map(|s| s.id).collect();
        let vertical = p.canvases[1].scene;
        p.prepare_workspace();
        let count = p.scenes.len();
        p.prepare_workspace();
        assert_eq!(p.scenes.len(), count);
        assert_eq!(p.canvases[1].scene, vertical);
        assert!(old.iter().all(|id| p.scene(*id).is_some()));
        let encoded = serde_json::to_string(&p).unwrap();
        let restored: Project = serde_json::from_str(&encoded).unwrap();
        restored.validate().unwrap();
        assert_eq!(
            p.preferences.workspace.scenes,
            restored.preferences.workspace.scenes
        );
        for canvas in p.canvases.clone() {
            let ids = p.canvas_scenes(canvas.id);
            for id in ids.iter().skip(1) {
                assert!(p.delete_scene(canvas.id, *id));
            }
            assert!(!p.delete_scene(canvas.id, ids[0]));
        }
        p.validate().unwrap();
    }
}
