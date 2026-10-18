//! Platform-neutral locations used by shared account and project settings.
use std::path::PathBuf;
pub fn config_root()->PathBuf {
 config_root_for(cfg!(target_os="windows"),|key|std::env::var_os(key).map(PathBuf::from))
}
fn config_root_for(windows:bool,get:impl Fn(&str)->Option<PathBuf>)->PathBuf{
 if windows {get("APPDATA").or_else(||get("USERPROFILE").map(|p|p.join("AppData/Roaming"))).unwrap_or_else(||PathBuf::from("."))}
 else {get("XDG_CONFIG_HOME").filter(|p|p.is_absolute()).unwrap_or_else(||get("HOME").unwrap_or_else(||PathBuf::from(".")).join(".config"))}
}
#[cfg(test)]mod tests{use super::*;#[test]fn windows_uses_roaming_and_linux_keeps_xdg(){assert_eq!(config_root_for(true,|k|(k=="APPDATA").then(||PathBuf::from("C:/Users/Moe/AppData/Roaming"))),PathBuf::from("C:/Users/Moe/AppData/Roaming"));assert_eq!(config_root_for(false,|k|(k=="XDG_CONFIG_HOME").then(||PathBuf::from("/tmp/config"))),PathBuf::from("/tmp/config"));}}
