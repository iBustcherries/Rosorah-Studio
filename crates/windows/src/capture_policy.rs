//! Pure capture eligibility rules, shared with native Windows discovery.
pub fn fullscreen(window:[i32;4],monitor:[i32;4])->bool{
 let [l,t,r,b]=window;let [ml,mt,mr,mb]=monitor;
 r>l&&b>t&&mr>ml&&mb>mt&&l<=ml.saturating_add(2)&&t<=mt.saturating_add(2)&&r>=mr.saturating_sub(2)&&b>=mb.saturating_sub(2)
}
pub fn excluded(path:&str)->bool{
 let name=file_name(path);
 // Also Windows tools that often run as administrator: their modules are locked away like an anti-cheat game's.
 matches!(name.as_str(),"explorer.exe"|"chrome.exe"|"msedge.exe"|"firefox.exe"|"steam.exe"|"discord.exe"|"applicationframehost.exe"
  |"taskmgr.exe"|"mmc.exe"|"cmd.exe"|"powershell.exe"|"pwsh.exe"|"windowsterminal.exe"|"conhost.exe"|"regedit.exe"|"msiexec.exe"|"rosorah-studio.exe")
}
/// A game in a window (or borderless at a size other than the screen's): its window comes from a
/// game engine, it's a known game, or it's installed in a game library. Launchers never count.
pub fn windowed_game(path:&str,class:&str,client:(i32,i32))->bool{
 let name=file_name(path);
 if client.0<640||client.1<360||launcher(&name){return false;}
 matches!(class,"UnityWndClass"|"UnrealWindow"|"RiotWindowClass"|"SDL_app"|"GLFW30"|"LWJGL"|"Valve001"|"CryENGINE"|"YYGameMakerYY")
  ||matches!(name.as_str(),"league of legends.exe"|"valorant-win64-shipping.exe"|"hearthstone.exe")
  ||(library(path)&&!class.starts_with("Chrome_WidgetWin"))
}
fn file_name(path:&str)->String{path.rsplit(['\\','/']).next().unwrap_or("").to_ascii_lowercase()}
/// Launchers, crash reporters, and streaming tools that come from game stores or engines (avatars,
/// wallpapers, scaling): never a game, even though Unity draws some of them.
fn launcher(name:&str)->bool{
 matches!(name,"vtube studio.exe"|"warudo.exe"|"animaze.exe"|"vseeface.exe"|"vnyan.exe"|"wallpaper32.exe"|"wallpaper64.exe"|"ui32.exe"
  |"losslessscaling.exe"|"vrmonitor.exe"|"vrserver.exe"|"vrdashboard.exe"
  |"riotclientservices.exe"|"riotclientux.exe"|"riotclientuxrender.exe"|"leagueclient.exe"|"leagueclientux.exe"|"leagueclientuxrender.exe"
  |"epicgameslauncher.exe"|"epicwebhelper.exe"|"eadesktop.exe"|"battle.net.exe"|"upc.exe"|"ubisoftconnect.exe"|"galaxyclient.exe"|"steamwebhelper.exe"
  |"unitycrashhandler64.exe"|"unitycrashhandler32.exe"|"crashreportclient.exe")
}
fn library(path:&str)->bool{
 let path=path.replace('/',"\\").to_ascii_lowercase();
 [r"\steamapps\common\",r"\epic games\",r"\xboxgames\",r"\riot games\",r"\gog galaxy\games\",r"\gog games\",r"\ea games\",r"\ubisoft game launcher\games\"]
  .iter().any(|folder|path.contains(folder))
}
/// The part of a captured window's picture to keep, as x, y, width, height: the game's own picture
/// without title bar and borders. `area` is the client area relative to the captured frame.
pub fn crop(area:[i32;4],frame:(u32,u32))->Option<[u32;4]>{
 let [x,y,w,h]=area;
 let (x,y)=(x.max(0) as u32,y.max(0) as u32);
 let w=(w.max(0) as u32).min(frame.0.saturating_sub(x));
 let h=(h.max(0) as u32).min(frame.1.saturating_sub(y));
 (w>0&&h>0).then_some([x,y,w,h])
}
pub fn renderer(module:&str)->bool{matches!(module.to_ascii_lowercase().as_str(),"vulkan-1.dll"|"opengl32.dll"|"d3d9.dll"|"d3d10.dll"|"d3d11.dll"|"d3d12.dll")}
#[cfg(test)]mod tests{
 use super::*;
 #[test]fn does_not_confuse_maximized_window_with_fullscreen(){assert!(!fullscreen([0,0,1920,1040],[0,0,1920,1080]));assert!(fullscreen([0,0,1920,1080],[0,0,1920,1080]));assert!(!fullscreen([0,0,0,0],[0,0,1920,1080]));}
 #[test]fn supports_secondary_monitor_and_tiny_border_rounding(){assert!(fullscreen([-1920,0,0,1080],[-1920,0,0,1080]));assert!(fullscreen([1,1,1919,1079],[0,0,1920,1080]));assert!(!fullscreen([100,0,1920,1080],[0,0,1920,1080]));}
 #[test]fn browsers_and_shell_are_not_automatic_game_targets(){assert!(excluded(r"C:\Program Files\Browser\CHROME.EXE"));assert!(excluded("explorer.exe"));assert!(excluded(r"C:\Windows\System32\Taskmgr.exe"));assert!(!excluded(r"D:\Games\deadcells.exe"));assert!(!excluded("C:/Games/Ryujinx.exe"));}
 #[test]fn renderer_detection_covers_legacy_and_modern_apis(){for name in ["vulkan-1.dll","OPENGL32.DLL","d3d9.dll","d3d10.dll","d3d11.dll","d3d12.dll"]{assert!(renderer(name));}assert!(!renderer("kernel32.dll"));}
 #[test]fn windowed_games_are_found_by_engine_name_or_library(){
  assert!(windowed_game(r"C:\Program Files (x86)\Hearthstone\Hearthstone.exe","UnityWndClass",(1600,900)));
  assert!(windowed_game(r"C:\Riot Games\League of Legends\Game\League of Legends.exe","RiotWindowClass",(1280,720)));
  assert!(windowed_game(r"C:\Riot Games\VALORANT\live\ShooterGame\Binaries\Win64\VALORANT-Win64-Shipping.exe","UnrealWindow",(1920,1080)));
  assert!(windowed_game(r"D:\SteamLibrary\steamapps\common\Some Game\game.exe","GameWindow",(1280,720)));
  assert!(windowed_game("D:/Games/hearthstone.exe","Other",(1280,720)));
 }
 #[test]fn launchers_apps_and_small_windows_are_not_windowed_games(){
  assert!(!windowed_game(r"C:\Riot Games\League of Legends\LeagueClientUx.exe","RCLIENT",(1280,720)));
  assert!(!windowed_game(r"C:\Riot Games\Riot Client\RiotClientServices.exe","Chrome_WidgetWin_1",(1280,720)));
  assert!(!windowed_game(r"D:\SteamLibrary\steamapps\common\Some Game\launcher.exe","Chrome_WidgetWin_1",(1280,720)));
  assert!(!windowed_game(r"C:\Users\me\AppData\Local\Programs\Spotify\Spotify.exe","Chrome_WidgetWin_1",(1280,720)));
  assert!(!windowed_game(r"C:\Program Files\Editor\editor.exe","EditorWindow",(1920,1080)));
  assert!(!windowed_game(r"C:\Program Files (x86)\Hearthstone\Hearthstone.exe","UnityWndClass",(400,300)));
  assert!(!windowed_game(r"N:\SteamLibrary\steamapps\common\VTube Studio\VTube Studio.exe","UnityWndClass",(1280,720)));
  assert!(!windowed_game(r"D:\SteamLibrary\steamapps\common\wallpaper_engine\wallpaper64.exe","WPEWindow",(1920,1080)));
 }
 #[test]fn crop_keeps_the_client_area_inside_the_frame(){
  assert_eq!(crop([8,31,1600,900],(1616,939)),Some([8,31,1600,900]));
  assert_eq!(crop([0,0,1920,1080],(1920,1080)),Some([0,0,1920,1080]));
  assert_eq!(crop([8,31,1700,950],(1616,939)),Some([8,31,1608,908]));
  assert_eq!(crop([-4,-4,1920,1080],(1920,1080)),Some([0,0,1920,1080]));
  assert_eq!(crop([0,0,0,0],(1920,1080)),None);
 }
}
