//! Which app sounds a mixer strip follows when it isn't tied to one device: music apps
//! automatically, or one app by name. Both keep working when the app restarts.
use crate::audio::Device;
use std::collections::BTreeSet;

/// Follows every recognised music app.
pub const MUSIC: &str = "auto:music";
const APP: &str = "app:";

/// The saved choice that follows one app by its identity.
pub fn app_target(app: &str) -> String {
    format!("{APP}{app}")
}
/// The app a saved choice follows, if it follows one.
pub fn followed_app(target: &str) -> Option<&str> {
    target.strip_prefix(APP).filter(|a| !a.is_empty())
}
/// Whether a saved choice follows apps rather than one fixed device.
pub fn follows(target: &str) -> bool {
    target == MUSIC || followed_app(target).is_some()
}
fn names(d: &Device) -> String {
    format!("{} {}", d.app, d.label)
}
pub fn browser(d: &Device) -> bool {
    crate::music_service::browser(&names(d))
}
/// A short name for an app, for menus and strip names.
pub fn app_name(d: &Device) -> String {
    let label = d.label.split_once(": ").map_or(d.label.as_str(), |(_, rest)| rest).trim();
    let label = label.strip_prefix("Application · ").unwrap_or(label);
    match crate::music_service::music_app(&names(d)) {
        Some(service) => service.to_owned(),
        None if label.is_empty() => d.app.clone(),
        None => label.to_owned(),
    }
}
/// Whether an app's sound counts as music: a known music player, an app that says it plays
/// music, or one the user added. Browsers only on Linux, and only for a tab whose page title is
/// a music site; video in a browser (YouTube, Netflix…) never counts. When unsure, it doesn't.
pub fn is_music(d: &Device, extra: &[String]) -> bool {
    if d.kind != "Stream/Output/Audio" || d.app.is_empty() {
        return false;
    }
    if extra.iter().any(|a| *a == d.app) {
        return true;
    }
    if browser(d) {
        // Windows mixes a browser's tabs into one sound, so a music tab can't be told apart.
        return !cfg!(windows) && crate::music_service::music_page(&d.title).is_some();
    }
    crate::music_service::music_app(&names(d)).is_some() || d.role.eq_ignore_ascii_case("music")
}
/// The app sounds `target` takes right now. Apps another strip follows by name (`claimed`) and the
/// captured game's processes (`game`) are left to those strips, so nothing is heard twice.
pub fn resolve<'a>(target: &str, devices: &'a [Device], extra: &[String], claimed: &[String], game: &BTreeSet<u32>) -> Vec<&'a Device> {
    let apps = devices.iter().filter(|d| d.kind == "Stream/Output/Audio" && !d.app.is_empty());
    if let Some(app) = followed_app(target) {
        return apps.filter(|d| d.app == app).take(16).collect();
    }
    if target != MUSIC {
        return Vec::new();
    }
    apps.filter(|d| is_music(d, extra))
        .filter(|d| !claimed.contains(&d.app))
        .filter(|d| !d.pid.is_some_and(|pid| game.contains(&pid)))
        .take(16)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stream(app: &str, label: &str, title: &str, role: &str, pid: u32) -> Device {
        Device {
            serial: format!("{app}-{title}-{pid}"),
            target: String::new(),
            label: format!("Application: {label}"),
            kind: "Stream/Output/Audio".into(),
            id: pid as u64,
            pid: Some(pid),
            channels: vec!["FL".into(), "FR".into()],
            app: app.into(),
            title: title.into(),
            role: role.into(),
        }
    }
    #[test]
    fn music_apps_are_followed_and_browser_video_never_is() {
        let devices = vec![
            stream("spotify", "Spotify", "Spotify", "Music", 1),
            stream("helium", "Helium", "Playback", "", 2),
            stream("firefox", "Firefox", "Netflix", "", 3),
            stream("firefox", "Firefox", "Song - YouTube Music", "", 4),
            stream("discord", "Discord", "Voice", "", 5),
            stream("rhythmbox", "Rhythmbox", "Track", "Music", 6),
            stream("game", "Game", "Game", "", 7),
        ];
        let picked: Vec<_> = resolve(MUSIC, &devices, &[], &[], &BTreeSet::from([7])).iter().map(|d| d.pid.unwrap()).collect();
        if cfg!(windows) {
            assert_eq!(picked, vec![1, 6]);
        } else {
            // Spotify, the YouTube Music tab and a player that says it plays music; not the
            // unnamed browser sound, Netflix, Discord or the captured game.
            assert_eq!(picked, vec![1, 4, 6]);
        }
    }
    #[test]
    fn added_and_claimed_apps() {
        let devices = vec![stream("discord", "Discord", "Voice", "", 5), stream("spotify", "Spotify", "Spotify", "Music", 1)];
        let extra = vec!["discord".to_owned()];
        assert_eq!(resolve(MUSIC, &devices, &extra, &[], &BTreeSet::new()).len(), 2);
        // Spotify has its own strip, so Music leaves it alone.
        let claimed = vec!["spotify".to_owned()];
        let picked = resolve(MUSIC, &devices, &extra, &claimed, &BTreeSet::new());
        assert_eq!(picked.iter().map(|d| d.app.as_str()).collect::<Vec<_>>(), vec!["discord"]);
        assert_eq!(resolve(&app_target("spotify"), &devices, &[], &[], &BTreeSet::new()).len(), 1);
        assert!(follows(MUSIC) && follows("app:spotify") && !follows("123") && !follows("app:"));
    }
}
#[cfg(all(test, not(windows)))]
mod live_tests {
    /// Reads this computer's real audio streams: cargo test -- --ignored live_music_apps --nocapture
    #[test]
    #[ignore]
    fn live_music_apps() {
        let devices = crate::audio::live_devices().expect("PipeWire");
        for d in devices.iter().filter(|d| d.kind == "Stream/Output/Audio") {
            println!("stream: app={:?} name={:?} title={:?} role={:?} music={}", d.app, super::app_name(d), d.title, d.role, super::is_music(d, &[]));
        }
        let picked: Vec<_> = super::resolve(super::MUSIC, &devices, &[], &[], &Default::default()).iter().map(|d| super::app_name(d)).collect();
        println!("Music apps would capture: {picked:?}");
    }
}
