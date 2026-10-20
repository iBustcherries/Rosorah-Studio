use std::{path::{Path,PathBuf},sync::mpsc,time::Instant};
use anyhow::{Context,Result,ensure};
pub enum Finished {External(crate::external_import::Preview),Saved(PathBuf),Imported(auroracast_core::Project,PathBuf),Located(u64,PathBuf)}
#[derive(Default)]pub struct Tools {
 pub external_preview:Option<crate::external_import::Preview>,pub media_folder:String,
 pub open:bool,pub compatibility:bool,pub audio:bool,pub performance:bool,pub guide:Option<usize>,pub test_until:Option<Instant>,pub test_pending:bool,
 pub job:Option<mpsc::Receiver<Result<Finished,String>>>,pub status:String,pub file:String,pub folder:String,pub recording_file:String,
}
impl Tools {pub fn spawn(&mut self,work:impl FnOnce()->Result<Finished>+Send+'static){let(tx,rx)=mpsc::channel();self.job=Some(rx);self.status="Working…".into();std::thread::spawn(move||{let _=tx.send(work().map_err(|e|format!("{e:#}")));});}}
pub fn picker(save:bool,folder:bool,title:&str)->Result<PathBuf>{
 crate::desktop::pick(if folder{crate::desktop::Pick::Folder}else if save{crate::desktop::Pick::Save}else{crate::desktop::Pick::Open},title,"Rosorah-project.zip")?.context("Selection cancelled")
}

pub fn bundle(project:Option<auroracast_core::Project>,path:PathBuf,parent:Option<PathBuf>)->Result<Finished>{
 use std::io::Write;let data=if let Some(project)=project{serde_json::json!({"mode":"export","project":project,"path":path})}else{serde_json::json!({"mode":"import","path":path,"parent":parent.context("Choose an import folder")?})};
 let mut child=std::process::Command::new(crate::browser_chat::system_python()).arg("-c").arg(include_str!("../../../packaging/project_bundle.py")).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().context("Python 3 is needed for project bundles")?;child.stdin.take().unwrap().write_all(&serde_json::to_vec(&data)?)?;let out=child.wait_with_output()?;let value:serde_json::Value=serde_json::from_slice(&out.stdout).context("Project bundle helper failed")?;ensure!(out.status.success(),"{}",value["error"].as_str().unwrap_or("Bundle operation failed"));let file=PathBuf::from(value["path"].as_str().context("Missing result path")?);
 if data["mode"]=="import"{Ok(Finished::Imported(auroracast_core::Project::load(&file)?,file))}else{Ok(Finished::Saved(file))}
}
pub fn marker(path:&Path)->PathBuf{path.with_extension("mkv.recording.json")}
pub fn mark_recording(path:&Path){let _=std::fs::write(marker(path),b"{\"incomplete\":true}");}
pub fn finish_recording(path:&Path,success:bool){if success||!path.exists(){let _=std::fs::remove_file(marker(path));}}
pub fn incomplete(folder:&Path)->Vec<PathBuf>{std::fs::read_dir(folder).into_iter().flatten().filter_map(Result::ok).map(|e|e.path()).filter(|p|p.file_name().is_some_and(|n|n.to_string_lossy().ends_with(".mkv.recording.json"))).map(|p|PathBuf::from(p.to_string_lossy().trim_end_matches(".recording.json"))).filter(|p|p.is_file()).take(50).collect()}
pub fn remux(path:PathBuf,mp4:bool)->Result<Finished>{
 ensure!(path.is_file(),"Choose a recording file");let stem=path.file_stem().unwrap_or_default().to_string_lossy();let output=path.with_file_name(format!("{stem}-{}.{}",if mp4{"converted"}else{"recovered"},if mp4{"mp4"}else{"mkv"}));ensure!(!output.exists(),"The output already exists; rename it before converting again");
 let temporary=output.with_file_name(format!(".rosorah-convert-{}-{}.{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos(),if mp4{"mp4"}else{"mkv"}));
 struct Cleanup(PathBuf);impl Drop for Cleanup{fn drop(&mut self){let _=std::fs::remove_file(&self.0);}}let _cleanup=Cleanup(temporary.clone());
 let mut command=std::process::Command::new("ffmpeg");command.args(["-hide_banner","-loglevel","error","-nostdin","-n","-i"]).arg(&path).args(["-map","0:v?","-map","0:a?","-c","copy"]);if mp4{command.args(["-movflags","+faststart"]);}command.arg(&temporary);let result=command.output().context("FFmpeg is required")?;
 if !result.status.success(){anyhow::bail!("Conversion failed. The original is unchanged. {}",String::from_utf8_lossy(&result.stderr).chars().take(800).collect::<String>());}
 ensure!(std::fs::metadata(&temporary)?.len()>0,"Conversion produced an empty file");std::fs::hard_link(&temporary,&output).context("Could not save converted recording without replacing an existing file")?;if !mp4{finish_recording(&path,true);}Ok(Finished::Saved(output))
}

#[cfg(test)] mod tests {
 use super::*;
 struct Temp(PathBuf);impl Temp{fn new()->Result<Self>{let path=std::env::temp_dir().join(format!("rosorah-tools-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos()));std::fs::create_dir(&path)?;Ok(Self(path))}fn path(&self)->&Path{&self.0}}impl Drop for Temp{fn drop(&mut self){let _=std::fs::remove_dir_all(&self.0);}}
 #[test] fn portable_bundle_roundtrip_keeps_media_and_avoids_overwrite()->Result<()> {
  let dir=Temp::new()?;let media=dir.path().join("original.png");std::fs::write(&media,b"image bytes")?;
  let mut p=auroracast_core::Project::default();p.sources[0].kind=auroracast_core::SourceKind::Image{path:media.display().to_string()};p.scenes[0].layers[0].crop=[0.1,0.2,0.3,0.1];p.scenes[0].layers[0].group=4;
  let archive=dir.path().join("portable.zip");bundle(Some(p.clone()),archive.clone(),None)?;assert!(bundle(Some(p),archive.clone(),None).is_err());std::fs::remove_file(media)?;
  let Finished::Imported(p,_) = bundle(None,archive,Some(dir.path().into()))?else{panic!("not imported")};
  let auroracast_core::SourceKind::Image{path}=&p.sources[0].kind else{panic!("not image")};assert_eq!(std::fs::read(path)?,b"image bytes");assert_eq!(p.scenes[0].layers[0].crop,[0.1,0.2,0.3,0.1]);assert_eq!(p.scenes[0].layers[0].group,4);Ok(())
 }
 #[test] fn remux_retains_original_and_refuses_existing_destination()->Result<()> {
  let dir=Temp::new()?;let file=dir.path().join("test.mkv");let status=std::process::Command::new("ffmpeg").args(["-hide_banner","-loglevel","error","-f","lavfi","-i","color=c=red:s=64x64:r=10","-t","0.3","-c:v","libx264"]).arg(&file).status()?;ensure!(status.success(),"fixture failed");let original=std::fs::read(&file)?;
  mark_recording(&file);assert_eq!(incomplete(dir.path()),vec![file.clone()]);remux(file.clone(),false)?;assert!(incomplete(dir.path()).is_empty());let Finished::Saved(mp4)=remux(file.clone(),true)?else{panic!("not saved")};assert!(std::fs::metadata(mp4)?.len()>0);assert!(remux(file.clone(),true).is_err());assert_eq!(std::fs::read(file)?,original);Ok(())
 }
}
