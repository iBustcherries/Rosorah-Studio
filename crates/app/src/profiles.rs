//! Profiles are separate projects, each with its own layout and backups. "Default" is the
//! original project.json; others live in profiles/<name>.json. The last one used opens at start.
use anyhow::{Context, Result, ensure};
use auroracast_core::Project;
use std::path::{Path, PathBuf};

pub const DEFAULT: &str = "Default";

fn root() -> PathBuf {
    crate::studio::default_project_path().parent().map(Path::to_path_buf).unwrap_or_default()
}
fn marker() -> PathBuf {
    root().join("current-profile")
}
pub fn path(name: &str) -> PathBuf {
    if name == DEFAULT { crate::studio::default_project_path() } else { root().join("profiles").join(format!("{name}.json")) }
}
/// The profile Rosorah last used, if it still exists.
pub fn current() -> String {
    let name = std::fs::read_to_string(marker()).unwrap_or_default().trim().to_string();
    if !name.is_empty() && list().contains(&name) { name } else { DEFAULT.into() }
}
pub fn set_current(name: &str) -> Result<()> {
    std::fs::write(marker(), name).context("Could not remember the profile")
}
/// Every profile, Default first.
pub fn list() -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(root().join("profiles"))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json") && !p.to_string_lossy().ends_with(".layout.json"))
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.insert(0, DEFAULT.into());
    names
}
pub fn valid_name(name: &str) -> Result<String> {
    let name = name.trim();
    ensure!(!name.is_empty() && name.chars().count() <= 40, "Use a profile name of 1–40 characters");
    ensure!(name.chars().all(|c| c.is_alphanumeric() || " -_'&()".contains(c)), "Use letters, numbers, spaces and - _ ' & ( ) in profile names");
    ensure!(!list().iter().any(|n| n.eq_ignore_ascii_case(name)), "A profile with that name already exists");
    Ok(name.to_string())
}
/// Creates a profile from `project` (a copy of the current one, or a new empty project).
pub fn create(name: &str, project: &Project) -> Result<()> {
    let name = valid_name(name)?;
    let file = path(&name);
    std::fs::create_dir_all(file.parent().unwrap())?;
    project.save(&file)
}
/// Renames a profile with its layout and backups. Default can't be renamed.
pub fn rename(old: &str, new: &str) -> Result<()> {
    ensure!(old != DEFAULT, "The Default profile can't be renamed");
    let new = valid_name(new)?;
    let (from, to) = (path(old), path(&new));
    std::fs::rename(&from, &to)?;
    for ext in ["layout.json", "backups"] {
        let _ = std::fs::rename(from.with_extension(ext), to.with_extension(ext));
    }
    if current() == old {
        set_current(&new)?;
    }
    Ok(())
}
/// Deletes a profile, its layout and its backups. Default and the open profile can't be deleted.
pub fn delete(name: &str, open: &str) -> Result<()> {
    ensure!(name != DEFAULT && name != open, "Switch to another profile before deleting this one");
    let file = path(name);
    std::fs::remove_file(&file)?;
    let _ = std::fs::remove_file(file.with_extension("layout.json"));
    let _ = std::fs::remove_dir_all(file.with_extension("backups"));
    Ok(())
}

/// Loads the project for start-up. When it can't be read, or a backup is newer than it
/// (and the project allows it), the newest readable backup opens instead. Returns a note
/// to show when that happened.
pub fn open(path: &Path) -> Result<(Project, Option<String>)> {
    if !path.exists() {
        return Ok((Project::default(), None));
    }
    let saved = Project::load(path);
    let use_backups = saved.as_ref().map_or(true, |p| p.preferences.backups.restore_on_start);
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    if use_backups {
        for entry in crate::backups::list(path) {
            if saved.is_ok() && modified.is_some_and(|m| entry.modified <= m) {
                break;
            }
            if let Ok(project) = crate::backups::load(&entry.path) {
                let why = if saved.is_ok() { "it is newer than the saved project" } else { "the saved project couldn't be read" };
                return Ok((project, Some(format!("Opened backup {} because {why}", entry.label))));
            }
        }
    }
    saved.map(|p| (p, None))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn newer_or_readable_backup_opens_at_start() {
        let dir = std::env::temp_dir().join(format!("rosorah-open-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("project.json");
        let mut project = Project::default();
        project.scenes[0].name = "Saved".into();
        project.save(&file).unwrap();
        assert!(open(&file).unwrap().1.is_none(), "no backups: the saved project opens");
        std::thread::sleep(std::time::Duration::from_millis(20));
        project.scenes[0].name = "Backed up".into();
        crate::backups::create(&project, &file, "hourly", 5).unwrap();
        let (opened, note) = open(&file).unwrap();
        assert_eq!(opened.scenes[0].name, "Backed up");
        assert!(note.is_some());
        project.preferences.backups.restore_on_start = false;
        std::thread::sleep(std::time::Duration::from_millis(20));
        project.scenes[0].name = "Saved later".into();
        project.save(&file).unwrap();
        assert_eq!(open(&file).unwrap().0.scenes[0].name, "Saved later");
        std::fs::write(&file, "not json").unwrap();
        assert_eq!(open(&file).unwrap().0.scenes[0].name, "Backed up", "a damaged project falls back to its backup");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
