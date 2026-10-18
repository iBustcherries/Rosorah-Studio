use anyhow::{Context,Result,ensure};
use auroracast_core::Project;
use std::{path::PathBuf,io::Write};
#[derive(serde::Deserialize)]pub struct Preview {pub project:Project,pub warnings:Vec<String>,pub missing:Vec<String>}
/// OBS's settings folder, if OBS has been used on this computer (including the Flatpak).
pub fn obs_folder()->Option<PathBuf>{
 let home=std::env::var_os("HOME").map(PathBuf::from);
 let mut options=Vec::new();
 if let Some(appdata)=std::env::var_os("APPDATA"){options.push(PathBuf::from(appdata).join("obs-studio"));}
 if let Some(config)=std::env::var_os("XDG_CONFIG_HOME"){options.push(PathBuf::from(config).join("obs-studio"));}
 if let Some(home)=&home{options.push(home.join(".config/obs-studio"));options.push(home.join(".var/app/com.obsproject.Studio/config/obs-studio"));}
 options.into_iter().find(|p|p.join("basic/scenes").is_dir())
}
/// Firebot's v5 data folder, if Firebot has been used on this computer.
pub fn firebot_folder()->Option<PathBuf>{
 let home=std::env::var_os("HOME").map(PathBuf::from);
 let mut options=Vec::new();
 if let Some(appdata)=std::env::var_os("APPDATA"){options.push(PathBuf::from(appdata).join("Firebot/v5"));}
 if let Some(config)=std::env::var_os("XDG_CONFIG_HOME"){options.push(PathBuf::from(config).join("Firebot/v5"));}
 if let Some(home)=&home{options.push(home.join(".config/Firebot/v5"));}
 options.into_iter().find(|p|p.join("profiles").is_dir())
}
/// OBS text sources name a font ("face:Arial bold"); use the closest bundled or installed font.
fn resolve_fonts(preview:&mut Preview){
 for source in &mut preview.project.sources{
  let auroracast_core::SourceKind::Widget(w)=&mut source.kind else{continue};
  let Some(face)=w.text_font.strip_prefix("face:").map(str::to_owned) else{continue};
  match crate::fonts::find(&face){Some(path)=>w.text_font=path,None=>{w.text_font.clear();preview.warnings.push(format!("{}: font \"{face}\" isn't installed; Roboto is used",source.name));}}
 }
}
pub fn read(path:PathBuf,media_folder:String)->Result<crate::utilities::Finished>{
 let data=serde_json::json!({"path":path,"media_folder":media_folder,"project":Project::default()});
 let mut child=std::process::Command::new(crate::browser_chat::system_python()).arg("-c").arg(include_str!("../../../packaging/import_external.py")).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().context("Python 3 is required for backup imports")?;
 child.stdin.take().unwrap().write_all(&serde_json::to_vec(&data)?)?;let out=child.wait_with_output()?;
 let value:serde_json::Value=serde_json::from_slice(&out.stdout).context("Could not read the backup")?;
 ensure!(out.status.success(),"{}",value["error"].as_str().unwrap_or("Import failed"));
 let mut preview:Preview=serde_json::from_value(value)?;resolve_fonts(&mut preview);preview.project.validate()?;
 Ok(crate::utilities::Finished::External(preview))
}
pub fn save(project:Project,path:PathBuf)->Result<crate::utilities::Finished>{
 project.validate()?;
 let bytes=serde_json::to_vec_pretty(&project)?;
 let mut file=std::fs::OpenOptions::new().create_new(true).write(true).open(&path).context("Choose a new filename; existing projects are never overwritten")?;
 file.write_all(&bytes)?;file.sync_all()?;
 Ok(crate::utilities::Finished::Imported(project,path))
}
#[cfg(test)]
mod tests {
    use auroracast_core::{Action, SourceKind};
    #[test]
    fn obs_and_firebot_features_convert() {
        let dir = std::env::temp_dir().join(format!("rosorah-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("basic/scenes")).unwrap();
        std::fs::create_dir_all(dir.join("profiles/Main/events")).unwrap();
        std::fs::create_dir_all(dir.join("profiles/Main/counters")).unwrap();
        let obs = serde_json::json!({
            "name": "Show", "current_scene": "Main", "current_transition": "Stinger", "transition_duration": 400,
            "scene_order": [{"name": "Main"}, {"name": "Inner"}],
            "transitions": [{"name": "Stinger", "id": "obs_stinger_transition", "settings": {"path": "/no/stinger.webm", "transition_point": 600}}],
            "AuxAudioDevice1": {"name": "Mic/Aux", "id": "pulse_input_capture", "volume": 0.5},
            "sources": [
                {"name": "Main", "id": "scene", "settings": {"items": [
                    {"name": "Cam", "pos": {"x": 0, "y": 0}, "scale": {"x": 0.5, "y": 0.5}, "align": 5, "crop_left": 192, "visible": true},
                    {"name": "Title", "pos": {"x": 960, "y": 540}, "scale": {"x": 1, "y": 1}, "align": 0},
                    {"name": "Inner", "pos": {"x": 960, "y": 0}, "scale": {"x": 0.5, "y": 0.5}, "align": 5}]}},
                {"name": "Inner", "id": "scene", "settings": {"items": [{"name": "Backdrop", "pos": {"x": 0, "y": 0}, "scale": {"x": 1, "y": 1}, "align": 5}]}},
                {"name": "Cam", "id": "v4l2_input", "settings": {"resolution": "1920x1080"},
                 "filters": [{"id": "chroma_key_filter_v2", "settings": {"key_color_type": "green", "similarity": 300}}]},
                {"name": "Title", "id": "text_ft2_source_v2", "settings": {"text": "Hello", "font": {"face": "Bebas Neue", "size": 72}, "outline": true}},
                {"name": "Backdrop", "id": "color_source_v3", "settings": {"color": 4278190335u64, "width": 1920, "height": 1080}}]});
        std::fs::write(dir.join("basic/scenes/Show.json"), obs.to_string()).unwrap();
        std::fs::write(dir.join("profiles/Main/counters/counters.json"), r#"{"c1":{"id":"c1","name":"Deaths","value":3}}"#).unwrap();
        std::fs::write(dir.join("profiles/Main/events/events.json"), r#"{"mainEvents":[{"name":"Follow","eventId":"follow","sourceId":"twitch","effects":{"list":[
            {"type":"firebot:chat","message":"Thanks $username!"},{"type":"firebot:update-counter","counterId":"c1","mode":"increment","value":"1"},
            {"type":"ebiggz:obs-change-scene","sceneName":"Inner"}]}}]}"#).unwrap();
        let Ok(crate::utilities::Finished::External(p)) = super::read(dir.clone(), String::new()) else { panic!("import failed") };
        let main = &p.project.scenes[0];
        assert_eq!(main.name, "Main");
        let source = |name: &str| p.project.sources.iter().find(|s| s.name == name).unwrap();
        let cam = main.layers.iter().find(|l| l.source == source("Cam").id).unwrap();
        assert!((cam.crop[0] - 0.1).abs() < 1e-4 && (cam.rect[2] - (1920. - 192.) * 0.5 / 1920.).abs() < 1e-4);
        assert!(p.project.preferences.chroma_keys.get(&source("Cam").id).is_some_and(|k| k.enabled && (k.similarity - 0.3).abs() < 1e-4));
        let backdrop = main.layers.iter().find(|l| l.source == source("Backdrop").id).expect("nested scene flattened");
        assert!((backdrop.rect[0] - 0.5).abs() < 1e-4 && (backdrop.rect[2] - 0.5).abs() < 1e-4);
        let SourceKind::Widget(text) = &source("Title").kind else { panic!("text is a widget") };
        assert_eq!((text.content.as_str(), text.font_size), ("Hello", 72));
        assert!(text.text_font.to_lowercase().contains("bebas"), "bundled font matched: {}", text.text_font);
        assert_eq!(p.project.preferences.transition, auroracast_core::Transition::Stinger);
        assert_eq!(p.project.preferences.transition_media.cut_ms, 600);
        assert!(p.project.audio.iter().any(|c| c.name.starts_with("Mic") && (c.gain_db + 6.02).abs() < 0.1));
        let rule = &p.project.rules[0];
        assert!(!rule.enabled && rule.platform == Some(auroracast_core::Platform::Twitch));
        assert!(matches!(&rule.actions[0], Action::SendChat { text, .. } if text == "Thanks {user}!"));
        assert!(matches!(&rule.actions[1], Action::UpdateWidget { source, increment: 1, .. } if *source == self::source_id(&p, "Deaths")));
        let inner = p.project.scenes.iter().find(|s| s.name == "Inner").unwrap().id;
        assert!(matches!(&rule.actions[2], Action::SwitchScene { scene, .. } if *scene == inner));
        std::fs::remove_dir_all(dir).unwrap();
    }
    fn source_id(p: &super::Preview, name: &str) -> u64 {
        p.project.sources.iter().find(|s| s.name == name).unwrap().id
    }
    /// Imports a real OBS or Firebot folder read-only and prints what came across.
    #[test]
    #[ignore = "Reads ROSORAH_IMPORT_PATH on request"]
    fn import_real_folder() {
        let path = std::env::var("ROSORAH_IMPORT_PATH").expect("folder or file to import");
        let Ok(crate::utilities::Finished::External(p)) = super::read(path.into(), String::new()) else { panic!("import failed") };
        println!("scenes {} · sources {} · layers {} · audio {} · automations {}", p.project.scenes.len(), p.project.sources.len(),
            p.project.scenes.iter().map(|s| s.layers.len()).sum::<usize>(), p.project.audio.len(), p.project.rules.len());
        println!("transition {:?} · canvas {}x{}@{}", p.project.preferences.transition, p.project.canvases[0].width, p.project.canvases[0].height, p.project.canvases[0].fps);
        for w in &p.warnings { println!("WARN {w}"); }
        for m in &p.missing { println!("MISSING {m}"); }
    }
}
