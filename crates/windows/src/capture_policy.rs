//! Pure capture eligibility rules, shared with native Windows discovery.
pub fn fullscreen(window:[i32;4],monitor:[i32;4])->bool{
 let [l,t,r,b]=window;let [ml,mt,mr,mb]=monitor;
 r>l&&b>t&&mr>ml&&mb>mt&&l<=ml.saturating_add(2)&&t<=mt.saturating_add(2)&&r>=mr.saturating_sub(2)&&b>=mb.saturating_sub(2)
}
pub fn excluded(path:&str)->bool{
 let name=path.rsplit(['\\','/']).next().unwrap_or("").to_ascii_lowercase();
 matches!(name.as_str(),"explorer.exe"|"chrome.exe"|"msedge.exe"|"firefox.exe"|"steam.exe"|"discord.exe"|"applicationframehost.exe")
}
pub fn renderer(module:&str)->bool{matches!(module.to_ascii_lowercase().as_str(),"vulkan-1.dll"|"opengl32.dll"|"d3d9.dll"|"d3d10.dll"|"d3d11.dll"|"d3d12.dll")}
#[cfg(test)]mod tests{
 use super::*;
 #[test]fn does_not_confuse_maximized_window_with_fullscreen(){assert!(!fullscreen([0,0,1920,1040],[0,0,1920,1080]));assert!(fullscreen([0,0,1920,1080],[0,0,1920,1080]));assert!(!fullscreen([0,0,0,0],[0,0,1920,1080]));}
 #[test]fn supports_secondary_monitor_and_tiny_border_rounding(){assert!(fullscreen([-1920,0,0,1080],[-1920,0,0,1080]));assert!(fullscreen([1,1,1919,1079],[0,0,1920,1080]));assert!(!fullscreen([100,0,1920,1080],[0,0,1920,1080]));}
 #[test]fn browsers_and_shell_are_not_automatic_game_targets(){assert!(excluded(r"C:\Program Files\Browser\CHROME.EXE"));assert!(excluded("explorer.exe"));assert!(!excluded(r"D:\Games\deadcells.exe"));assert!(!excluded("C:/Games/Ryujinx.exe"));}
 #[test]fn renderer_detection_covers_legacy_and_modern_apis(){for name in ["vulkan-1.dll","OPENGL32.DLL","d3d9.dll","d3d10.dll","d3d11.dll","d3d12.dll"]{assert!(renderer(name));}assert!(!renderer("kernel32.dll"));}
}
