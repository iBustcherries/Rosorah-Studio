//! Desktop integration through capabilities, never a compositor-name allowlist.
use anyhow::{Context, Result, bail, ensure};
use std::{path::PathBuf, time::Duration, sync::{Mutex,OnceLock,atomic::{AtomicBool,Ordering}}};
use rosorah_capture::Target;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Session {Wayland,X11,Unknown}
pub fn session_from(kind:&str,wayland:bool,x11:bool)->Session {
 match kind.to_ascii_lowercase().as_str(){"wayland"=>Session::Wayland,"x11"=>Session::X11,_ if wayland=>Session::Wayland,_ if x11=>Session::X11,_=>Session::Unknown}
}
pub fn session()->Session{session_from(&std::env::var("XDG_SESSION_TYPE").unwrap_or_default(),std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v|!v.is_empty()),std::env::var_os("DISPLAY").is_some_and(|v|!v.is_empty()))}
pub fn command_available(name:&str)->bool{
 use std::os::unix::fs::PermissionsExt;
 std::env::var_os("PATH").into_iter().flat_map(|p|std::env::split_paths(&p).collect::<Vec<_>>()).any(|dir|std::fs::metadata(dir.join(name)).is_ok_and(|m|m.is_file()&&m.permissions().mode()&0o111!=0))
}
/// Explicit window-only requests never silently become full-desktop capture.
pub fn capture_modes(target:Target,available:u32,cursors:u32,want_cursor:bool)->Result<(u32,u32,Vec<&'static str>)>{
 let sources=match target{Target::AutoFullscreen=>bail!("Automatic capture uses the renderer hook"),Target::Window=>2,Target::Screen=>1,Target::WindowOrScreen=>3}&available;
 ensure!(sources!=0,"This desktop does not advertise the requested capture type. Choose a supported screen/window type in Capture details.");
 let mut notes=Vec::new();if target==Target::WindowOrScreen&&sources==1{notes.push("This desktop offers screen sharing only; its chooser will show screens.");}
 let cursor=if want_cursor&&cursors&2!=0{2}else if cursors&1!=0{if want_cursor{notes.push("This desktop cannot embed the pointer; capture will hide it.");}1}else{bail!("This desktop cannot provide the requested pointer mode. Enable Capture cursor if embedding is supported.")};
 Ok((sources,cursor,notes))
}

#[derive(Clone,Debug)] pub struct Report {pub desktop:String,pub session:Session,pub sources:Option<u32>,pub cursors:Option<u32>,pub portal_error:String,pub file_chooser:bool,pub wallet:bool,pub commands:Vec<(&'static str,bool)>,pub checking:bool}
impl Default for Report{fn default()->Self{Self{desktop:std::env::var("XDG_CURRENT_DESKTOP").or_else(|_|std::env::var("DESKTOP_SESSION")).unwrap_or_else(|_|"Unknown".into()),session:session(),sources:None,cursors:None,portal_error:String::new(),file_chooser:false,wallet:false,commands:vec![],checking:false}}}
static REPORT:OnceLock<Mutex<Report>>=OnceLock::new();static PROBING:AtomicBool=AtomicBool::new(false);
pub fn report()->Report{REPORT.get_or_init(Default::default).lock().unwrap().clone()}
pub fn refresh(){if PROBING.swap(true,Ordering::Relaxed){return}REPORT.get_or_init(Default::default).lock().unwrap().checking=true;std::thread::spawn(||{
 let mut report=Report::default();report.commands=["pw-record","pw-play","pw-dump","pw-link","ffmpeg","kdialog","zenity","xwininfo","xdg-open"].into_iter().map(|n|(n,command_available(n))).collect();
 let result=tokio::runtime::Builder::new_current_thread().enable_all().build().map(|rt|rt.block_on(async{tokio::time::timeout(Duration::from_secs(8),async{
  let bus=match zbus::Connection::session().await{Ok(bus)=>bus,Err(e)=>{report.portal_error=format!("Session bus unavailable: {e}");return}};
  match ashpd::desktop::screencast::Screencast::with_connection(bus.clone()).await{Ok(p)=>{report.sources=p.available_source_types().await.ok().map(|v|v.bits());report.cursors=p.available_cursor_modes().await.ok().map(|v|v.bits());if report.sources.is_none(){report.portal_error="Screen sharing capabilities could not be read".into();}},Err(e)=>report.portal_error=e.to_string()}
  report.file_chooser=match zbus::Proxy::new(&bus,"org.freedesktop.portal.Desktop","/org/freedesktop/portal/desktop","org.freedesktop.portal.FileChooser").await{Ok(p)=>p.get_property::<u32>("version").await.is_ok(),Err(_)=>false};
  if let Ok(proxy)=zbus::fdo::DBusProxy::new(&bus).await{let names=proxy.list_names().await.unwrap_or_default();let activatable=proxy.list_activatable_names().await.unwrap_or_default();report.wallet=names.iter().any(|n|n.as_str()=="org.freedesktop.secrets")||activatable.iter().any(|n|n.as_str()=="org.freedesktop.secrets");}
 }).await}));
 if !matches!(result,Ok(Ok(()))){report.portal_error="Desktop capability check timed out or could not start. Check the desktop portal/session bus and use Refresh.".into();}
 *REPORT.get().unwrap().lock().unwrap()=report;PROBING.store(false,Ordering::Relaxed);
});}

#[derive(Clone,Copy)]pub enum Pick {Open,Save,Folder}
fn dialog_order(desktop:&str)->[&'static str;2]{if desktop.split(':').any(|v|v.eq_ignore_ascii_case("KDE")||v.eq_ignore_ascii_case("Plasma")){["kdialog","zenity"]}else{["zenity","kdialog"]}}
fn local_uri(uri:&str)->Result<PathBuf>{
 use std::os::unix::ffi::OsStringExt;
 let path=uri.strip_prefix("file://localhost/").map(|p|format!("/{p}")).or_else(||uri.strip_prefix("file:///").map(|p|format!("/{p}"))).context("The chooser returned a remote file. Download it locally before adding it.")?;
 let mut out=Vec::new();let bytes=path.as_bytes();let mut i=0;while i<bytes.len(){if bytes[i]==b'%'{ensure!(i+2<bytes.len(),"Invalid file URI");let hex=std::str::from_utf8(&bytes[i+1..i+3])?;out.push(u8::from_str_radix(hex,16)?);i+=3;}else{out.push(bytes[i]);i+=1;}}ensure!(!out.contains(&0),"Invalid file URI");Ok(std::ffi::OsString::from_vec(out).into())
}
/// Runs only in caller-owned worker threads; cancel never opens a second dialog.
pub fn pick(mode:Pick,title:&str,suggested:&str)->Result<Option<PathBuf>>{
 use ashpd::desktop::{file_chooser::{FileChooserProxy,OpenFileOptions,SaveFileOptions},ResponseError};
 let runtime=tokio::runtime::Builder::new_current_thread().enable_all().build()?;
 let portal=runtime.block_on(async{
  let proxy=tokio::time::timeout(Duration::from_secs(4),async{let bus=zbus::Connection::session().await.map_err(ashpd::Error::from)?;FileChooserProxy::with_connection(bus).await}).await;
  let proxy=match proxy{Ok(Ok(proxy))=>proxy,Ok(Err(e))=>{crate::capture_log::event(format!("File chooser portal: {e}"));return None},Err(_)=>{crate::capture_log::event("File chooser portal connection timed out");return None}};
  let response=match mode{Pick::Save=>proxy.save_file(None,title,SaveFileOptions::default().set_current_name(suggested)).await,_=>proxy.open_file(None,title,OpenFileOptions::default().set_multiple(false).set_directory(matches!(mode,Pick::Folder))).await};
  Some(response.and_then(|r|r.response()))
 });
 match portal{Some(Ok(files))=>return files.uris().first().map(|u|local_uri(u.as_str())).transpose(),Some(Err(ashpd::Error::Response(ResponseError::Cancelled)))=>return Ok(None),Some(Err(e))=>{crate::capture_log::event(format!("File chooser portal unavailable: {e}; trying a desktop dialog."));},None=>{}}
 for tool in dialog_order(&std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default()){
  let mut cmd=std::process::Command::new(tool);if tool=="kdialog"{cmd.args([match mode{Pick::Open=>"--getopenfilename",Pick::Save=>"--getsavefilename",Pick::Folder=>"--getexistingdirectory"},if matches!(mode,Pick::Save){suggested}else{"."},"--title",title]);}else{cmd.args(["--file-selection","--title",title]);match mode{Pick::Save=>{cmd.args(["--save","--confirm-overwrite","--filename",suggested]);},Pick::Folder=>{cmd.arg("--directory");},Pick::Open=>{}}}
  match cmd.output(){Ok(out)if out.status.success()=>{let value=String::from_utf8(out.stdout)?;let path=value.trim_end_matches(['\r','\n']);return Ok((!path.is_empty()).then(||PathBuf::from(path)));},Ok(out)if out.status.code()==Some(1)=>return Ok(None),_=>continue}
 }
 bail!("No file chooser is available. Enable the desktop FileChooser portal or install Zenity/KDialog; you can also enter a file path directly.")
}

#[cfg(test)]mod tests{
 use super::*;
 #[test]fn session_detection_prefers_wayland_over_xwayland(){assert_eq!(session_from("",true,true),Session::Wayland);assert_eq!(session_from("x11",true,true),Session::X11);assert_eq!(session_from("",false,false),Session::Unknown);}
 #[test]fn only_advertised_modes_are_requested(){assert_eq!(capture_modes(Target::WindowOrScreen,1,1,true).unwrap().0,1);assert!(capture_modes(Target::Window,1,3,true).is_err());assert!(capture_modes(Target::Screen,2,3,true).is_err());assert!(capture_modes(Target::Screen,1,2,false).is_err());assert_eq!(capture_modes(Target::Window,3,3,true).unwrap().1,2);}
 #[test]fn paths_decode_without_losing_spaces_or_unicode(){assert_eq!(local_uri("file:///tmp/a%20b%23c.png").unwrap(),PathBuf::from("/tmp/a b#c.png"));assert!(local_uri("https://example.com/a").is_err());assert!(local_uri("file://other-host/a").is_err());assert!(local_uri("file:///bad%00path").is_err());assert!(local_uri("file:///bad%GG").is_err());}
 #[test]fn fallback_matches_desktop_not_distribution(){assert_eq!(dialog_order("ubuntu:GNOME"),["zenity","kdialog"]);assert_eq!(dialog_order("KDE"),["kdialog","zenity"]);assert_eq!(dialog_order("COSMIC"),["zenity","kdialog"]);}
}

pub fn compositor_process(name:&str)->bool{matches!(name,"rosorah-studio"|"kwin_wayland"|"kwin_x11"|"plasmashell"|"gnome-shell"|"mutter"|"cinnamon"|"muffin"|"cosmic-comp"|"cosmic-panel"|"cosmic-dock"|"cosmic-launcher"|"xfwm4"|"xfdesktop"|"xfce4-panel"|"marco"|"mate-panel"|"sway"|"Hyprland"|"hyprland"|"wayfire"|"labwc"|"weston"|"weston-desktop-shell"|"river"|"niri"|"picom"|"compiz"|"openbox"|"awesome"|"i3"|"bspwm"|"qtile"|"Xorg"|"Xwayland")}
pub fn renderer_process_allowed(pid:u32)->bool{std::fs::read_link(format!("/proc/{pid}/exe")).ok().and_then(|p|p.file_name().map(|n|!compositor_process(&n.to_string_lossy()))).unwrap_or(false)}
#[cfg(test)]mod compositor_tests{use super::*;#[test]fn desktop_compositors_never_become_auto_game_sources(){for name in["cosmic-comp","cinnamon","sway","Hyprland","xfwm4","gnome-shell","kwin_wayland"]{assert!(compositor_process(name));}assert!(!compositor_process("deadcells"));assert!(!compositor_process("cosmic-ray-game"));}}

#[cfg(test)]mod portal_tests{
 use super::*;use std::{collections::HashMap,sync::Arc};use zbus::zvariant::{OwnedValue,OwnedObjectPath,Value};
 struct Screen;
 #[zbus::interface(name="org.freedesktop.portal.ScreenCast")]
 impl Screen{#[zbus(property,name="version")]fn version(&self)->u32{4}#[zbus(property)]fn available_source_types(&self)->u32{1}#[zbus(property)]fn available_cursor_modes(&self)->u32{1}}
 struct Chooser{cancel:Arc<AtomicBool>}
 #[zbus::interface(name="org.freedesktop.portal.FileChooser")]
 impl Chooser{
  #[zbus(property,name="version")]fn version(&self)->u32{4}
  async fn open_file(&self,_parent:&str,_title:&str,options:HashMap<String,OwnedValue>,#[zbus(header)]header:zbus::message::Header<'_>,#[zbus(connection)]connection:&zbus::Connection)->zbus::fdo::Result<OwnedObjectPath>{
   let token=options.get("handle_token").unwrap().downcast_ref::<&str>().unwrap();let sender=header.sender().unwrap().as_str().trim_start_matches(':').replace('.',"_");let path=OwnedObjectPath::try_from(format!("/org/freedesktop/portal/desktop/request/{sender}/{token}")).unwrap();let output=path.clone();let connection=connection.clone();let cancelled=self.cancel.load(Ordering::Relaxed);
   tokio::spawn(async move{tokio::time::sleep(Duration::from_millis(40)).await;let mut results=HashMap::new();if !cancelled{results.insert("uris",Value::new(vec!["file:///tmp/hello%20world.png"]));}connection.emit_signal(None::<&str>,output,"org.freedesktop.portal.Request","Response",&(if cancelled{1u32}else{0u32},results)).await.unwrap();});Ok(path)
  }
 }
 #[test]#[ignore="Requires an isolated D-Bus session; uses mock portal, no desktop dialogs"]
 fn portal_selection_cancel_and_capability_probe()->Result<()>{
  let cancel=Arc::new(AtomicBool::new(false));let _service=zbus::blocking::connection::Builder::session()?.name("org.freedesktop.portal.Desktop")?.serve_at("/org/freedesktop/portal/desktop",Screen)?.serve_at("/org/freedesktop/portal/desktop",Chooser{cancel:cancel.clone()})?.build()?;
  assert_eq!(pick(Pick::Open,"Test chooser","").map_err(|e|anyhow::anyhow!("{e}\n{}",crate::capture_log::details()))?,Some(PathBuf::from("/tmp/hello world.png")));cancel.store(true,Ordering::Relaxed);assert_eq!(pick(Pick::Open,"Cancelled chooser","")?,None);
  refresh();let start=std::time::Instant::now();while report().checking{ensure!(start.elapsed()<Duration::from_secs(10),"Probe did not finish");std::thread::sleep(Duration::from_millis(20));}let r=report();assert_eq!(r.sources,Some(1));assert_eq!(r.cursors,Some(1));assert!(r.file_chooser);Ok(())
 }
}

pub fn open(target:impl AsRef<std::ffi::OsStr>)->std::io::Result<std::process::Child>{
 let target=target.as_ref();std::process::Command::new("xdg-open").arg(target).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().or_else(|e|{if e.kind()!=std::io::ErrorKind::NotFound{return Err(e)}std::process::Command::new("gio").arg("open").arg(target).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn()})
}
