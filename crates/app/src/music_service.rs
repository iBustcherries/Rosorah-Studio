//! Recognizes media sessions from music services so Now Playing ignores videos,
//! streams and other audio that publishes desktop media controls.

/// Desktop apps, matched against the lowercase player id and name.
const APPS: &[(&str, &str)] = &[
    ("spotify", "Spotify"),
    ("plexamp", "Plexamp"),
    ("tidal", "TIDAL"),
    ("amazonmusic", "Amazon Music"),
    ("amazon music", "Amazon Music"),
    ("amazon-music", "Amazon Music"),
    ("applemusic", "Apple Music"),
    ("apple music", "Apple Music"),
    ("itunes", "Apple Music"),
    ("cider", "Apple Music"),
    ("youtubemusic", "YouTube Music"),
    ("youtube music", "YouTube Music"),
    ("youtube-music", "YouTube Music"),
    ("ytmdesktop", "YouTube Music"),
    ("ytmusic", "YouTube Music"),
    // Chrome/Edge installed YouTube Music web app.
    ("cinhimbnkkaeohfgghhklpknlkffjgod", "YouTube Music"),
];
/// Web players, matched against page, track or artwork addresses when available.
const SITES: &[(&str, &str)] = &[
    ("open.spotify.com", "Spotify"),
    ("scdn.co", "Spotify"),
    ("music.youtube.com", "YouTube Music"),
    ("music.amazon.", "Amazon Music"),
    ("music.apple.com", "Apple Music"),
    ("mzstatic.com", "Apple Music"),
    ("tidal.com", "TIDAL"),
    ("listen.plex.tv", "Plexamp"),
];
/// Page title endings browsers fall back to when a site sends no metadata.
const TITLES: &[(&str, &str)] = &[
    ("- youtube music", "YouTube Music"),
    ("| spotify", "Spotify"),
    ("- spotify", "Spotify"),
    ("amazon music", "Amazon Music"),
    ("- apple music", "Apple Music"),
    ("on tidal", "TIDAL"),
    ("| tidal", "TIDAL"),
];
const BROWSERS: &[&str] = &[
    "chrom", "helium", "firefox", "308046b0af4a39cb", "msedge", "microsoft-edge", "brave", "opera",
    "vivaldi", "librewolf", "floorp", "waterfox", "epiphany", "falkon", "thorium", "zen",
];

/// The music service a desktop app is, from its identifiers (program, app id, name).
pub fn music_app(names: &str) -> Option<&'static str> {
    let names = names.to_lowercase();
    APPS.iter().find(|(app, _)| names.contains(app)).map(|(_, name)| *name)
}
/// Whether identifiers belong to a web browser.
pub fn browser(names: &str) -> bool {
    let names = names.to_lowercase();
    BROWSERS.iter().any(|b| names.contains(b))
}
/// The music site a browser page title belongs to, if the title says so.
pub fn music_page(title: &str) -> Option<&'static str> {
    let lower = title.trim().to_lowercase();
    TITLES.iter().find(|(end, _)| lower.ends_with(end)).map(|(_, name)| *name)
}
/// The music service behind a media session, if it is one. `names` are the player's
/// identifiers (bus name, app id, display name); `urls` are any page or artwork URLs.
pub fn service(names: &[&str], urls: &[&str], title: &str, artist: &str, album: &str) -> Option<&'static str> {
    let names = names.join(" ").to_lowercase();
    let urls = urls.join(" ").to_lowercase();
    if let Some((_, name)) = SITES.iter().find(|(site, _)| urls.contains(site)) {
        return Some(name);
    }
    // Electron apps such as tidal-hifi publish as Chromium, so app names come first.
    if let Some((_, name)) = APPS.iter().find(|(app, _)| names.contains(app)) {
        return Some(name);
    }
    if !BROWSERS.iter().any(|b| names.contains(b)) {
        return None;
    }
    let lower = title.trim().to_lowercase();
    if let Some((_, name)) = TITLES.iter().find(|(end, _)| lower.ends_with(end)) {
        return Some(name);
    }
    // Browsers usually hide which site is playing. The music services above all send
    // artist and album metadata; video sites like YouTube, Twitch and Kick send no album.
    (!artist.trim().is_empty() && !album.trim().is_empty()).then_some("Web music player")
}

#[cfg(test)]
mod tests {
    use super::service;
    #[test]
    fn desktop_music_apps() {
        assert_eq!(service(&["org.mpris.MediaPlayer2.spotify", "Spotify"], &[], "Song", "", ""), Some("Spotify"));
        assert_eq!(service(&["Plexamp.exe"], &[], "Song", "A", ""), Some("Plexamp"));
        assert_eq!(service(&["AppleInc.AppleMusicWin_nzyj5cx40ttqa!App"], &[], "Song", "A", "B"), Some("Apple Music"));
        assert_eq!(service(&["org.mpris.MediaPlayer2.chromium.instance42", "tidal-hifi"], &[], "Song", "", ""), Some("TIDAL"));
        assert_eq!(service(&["org.mpris.MediaPlayer2.tidal-hifi", "tidal-hifi"], &[], "Song", "", ""), Some("TIDAL"));
        assert_eq!(service(&["org.mpris.MediaPlayer2.vlc", "VLC media player"], &[], "Movie", "", ""), None);
    }
    #[test]
    fn browser_sessions() {
        assert_eq!(service(&["MSEdge"], &[], "Video", "Some channel", ""), None);
        assert_eq!(service(&["Chrome"], &[], "Song", "Artist", "Album"), Some("Web music player"));
        assert_eq!(service(&["firefox.instance_1"], &["https://music.youtube.com/watch?v=x"], "Song", "", ""), Some("YouTube Music"));
        assert_eq!(service(&["Chrome"], &[], "Song - YouTube Music", "", ""), Some("YouTube Music"));
        assert_eq!(service(&["Chrome._crx_cinhimbnkkaeohfgghhklpknlkffjgod"], &[], "Song", "", ""), Some("YouTube Music"));
    }
}
