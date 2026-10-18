//! Bounded project editing history. Stream counters are deliberately outside undo.
use auroracast_core::Project;
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
pub struct History {
    baseline: Project,
    undo: VecDeque<Project>,
    redo: Vec<Project>,
    last: Option<Instant>,
    bytes: usize,
}
impl History {
    pub fn new(project: &Project) -> Self {
        Self {
            baseline: project.clone(),
            undo: VecDeque::new(),
            redo: Vec::new(),
            last: None,
            bytes: 0,
        }
    }
    pub fn record(&mut self, project: &Project, continuous: bool) {
        let old = serde_json::to_vec(&self.baseline).unwrap_or_default();
        let new = serde_json::to_vec(project).unwrap_or_default();
        if old == new {
            return;
        }
        let coalesce = continuous
            && self
                .last
                .is_some_and(|t| t.elapsed() < Duration::from_millis(450));
        if !coalesce {
            self.bytes += old.len();
            self.undo.push_back(self.baseline.clone());
            while self.undo.len() > 40 || self.bytes > 16 * 1024 * 1024 {
                if let Some(p) = self.undo.pop_front() {
                    self.bytes = self
                        .bytes
                        .saturating_sub(serde_json::to_vec(&p).map_or(0, |v| v.len()));
                } else {
                    break;
                }
            }
        }
        self.baseline = project.clone();
        self.redo.clear();
        self.last = continuous.then(Instant::now);
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo(&mut self, current: &Project) -> Option<Project> {
        let p = self.undo.pop_back()?;
        self.bytes = self
            .bytes
            .saturating_sub(serde_json::to_vec(&p).map_or(0, |v| v.len()));
        self.redo.push(current.clone());
        Some(self.restore(p, current))
    }
    pub fn redo(&mut self, current: &Project) -> Option<Project> {
        let p = self.redo.pop()?;
        self.undo.push_back(current.clone());
        self.bytes += serde_json::to_vec(current).map_or(0, |v| v.len());
        Some(self.restore(p, current))
    }
    fn restore(&mut self, mut p: Project, current: &Project) -> Project {
        p.next_id = p.next_id.max(current.next_id);
        p.preferences.widget_values = current.preferences.widget_values.clone();
        p.preferences.widget_periods = current.preferences.widget_periods.clone();
        self.baseline = p.clone();
        self.last = None;
        p
    }
}
pub fn recovery_path(path: &Path) -> PathBuf {
    path.with_extension("recovery.json")
}
pub fn save_with_backups(project: &Project, path: &Path) -> anyhow::Result<()> {
    // A valid old project is retained before a new one replaces it.
    if path.is_file() && Project::load(path).is_ok() {
        let folder = path.with_extension("backups");
        std::fs::create_dir_all(&folder)?;
        for n in (1..5).rev() {
            let a = folder.join(format!("{n}.json"));
            let b = folder.join(format!("{}.json", n + 1));
            if a.exists() {
                std::fs::copy(a, b)?;
            }
        }
        std::fs::copy(path, folder.join("1.json"))?;
    }
    project.save(path)?;
    let recovery = recovery_path(path);
    if recovery.exists() {
        std::fs::remove_file(recovery)?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn undo_preserves_live_totals_and_redo() {
        let mut p = Project::default();
        let mut h = History::new(&p);
        let old = p.sources[0].name.clone();
        p.sources[0].name = "Changed".into();
        h.record(&p, false);
        p.preferences.widget_values.insert(50, 123);
        let restored = h.undo(&p).unwrap();
        assert_eq!(restored.sources[0].name, old);
        assert_eq!(restored.preferences.widget_values[&50], 123);
        assert_eq!(h.redo(&restored).unwrap().sources[0].name, "Changed");
    }
    #[test]
    fn continuous_edits_are_one_step() {
        let mut p = Project::default();
        let mut h = History::new(&p);
        p.sources[0].name = "A".into();
        h.record(&p, true);
        p.sources[0].name = "B".into();
        h.record(&p, true);
        h.undo(&p).unwrap();
        assert!(!h.can_undo());
    }
    #[test]
    fn save_retains_valid_previous_project_and_clears_recovery() {
        let dir = std::env::temp_dir().join(format!(
            "rosorah-backup-test-{}-{}",
            std::process::id(),
            crate::activity::now()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("project.json");
        let mut p = Project::default();
        let original = p.sources[0].name.clone();
        p.save(&path).unwrap();
        p.sources[0].name = "Recovered".into();
        p.save(&recovery_path(&path)).unwrap();
        save_with_backups(&p, &path).unwrap();
        assert!(!recovery_path(&path).exists());
        assert_eq!(
            Project::load(&path.with_extension("backups").join("1.json"))
                .unwrap()
                .sources[0]
                .name,
            original
        );
        assert_eq!(Project::load(&path).unwrap().sources[0].name, "Recovered");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
