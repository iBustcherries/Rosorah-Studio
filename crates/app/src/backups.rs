//! Timestamped project backups kept beside the project file, in "<project>.backups".
//! The numbered 1–5.json copies written by every save are listed too.
use anyhow::{Context, Result};
use auroracast_core::Project;
use std::path::{Path, PathBuf};

pub struct Entry {
    pub path: PathBuf,
    pub label: String,
    pub modified: std::time::SystemTime,
}

fn folder(project: &Path) -> PathBuf {
    project.with_extension("backups")
}

/// Saves a copy of `project` named after the local time and why it was made, then
/// removes the oldest timestamped backups beyond `keep`.
pub fn create(project: &Project, project_path: &Path, reason: &str, keep: u32) -> Result<PathBuf> {
    let dir = folder(project_path);
    std::fs::create_dir_all(&dir).context("Could not create the backups folder")?;
    let stamp = gst::glib::DateTime::now_local()
        .and_then(|t| t.format("%Y-%m-%d %H-%M-%S"))
        .map(|s| s.to_string())
        .unwrap_or_else(|_| crate::activity::now().to_string());
    let mut path = dir.join(format!("backup {stamp} {reason}.json"));
    for n in 2.. {
        if !path.exists() {
            break;
        }
        path = dir.join(format!("backup {stamp} {reason} {n}.json"));
    }
    project.save(&path)?;
    let mut stamped: Vec<_> = list(project_path).into_iter().filter(|e| is_stamped(&e.path)).collect();
    stamped.sort_by_key(|e| std::cmp::Reverse(e.modified));
    for old in stamped.into_iter().skip(keep.max(1) as usize) {
        let _ = std::fs::remove_file(old.path);
    }
    Ok(path)
}

fn is_stamped(path: &Path) -> bool {
    path.file_name().is_some_and(|n| n.to_string_lossy().starts_with("backup "))
}

/// Every backup for this project, newest first.
pub fn list(project_path: &Path) -> Vec<Entry> {
    let mut out: Vec<_> = std::fs::read_dir(folder(project_path))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .filter_map(|path| {
            let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok()?;
            let stem = path.file_stem()?.to_string_lossy().into_owned();
            let label = match stem.strip_prefix("backup ") {
                Some(rest) => rest.to_string(),
                None if stem.parse::<u32>().is_ok() => format!("Earlier save #{stem}"),
                None => return None,
            };
            Some(Entry { path, label, modified })
        })
        .collect();
    out.sort_by_key(|e| std::cmp::Reverse(e.modified));
    out
}

pub fn load(path: &Path) -> Result<Project> {
    let project = Project::load(path).context("This backup could not be read")?;
    project.validate()?;
    Ok(project)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backups_are_listed_newest_first_and_pruned() {
        let dir = std::env::temp_dir().join(format!("rosorah-backups-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let project_path = dir.join("show.json");
        let project = Project::default();
        for _ in 0..4 {
            create(&project, &project_path, "manual", 3).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        std::fs::write(folder(&project_path).join("1.json"), serde_json::to_vec(&project).unwrap()).unwrap();
        let entries = list(&project_path);
        assert_eq!(entries.iter().filter(|e| is_stamped(&e.path)).count(), 3);
        assert!(entries.iter().any(|e| e.label == "Earlier save #1"));
        assert!(load(&entries[0].path).is_ok());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
