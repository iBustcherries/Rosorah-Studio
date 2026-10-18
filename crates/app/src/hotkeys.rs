//! Hotkeys: key combinations that work while Rosorah is focused, and everywhere when the system
//! allows it (Windows: registered with the system; Linux: the desktop's global-shortcuts portal).
use eframe::egui;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

/// A key combination: modifiers plus one key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Combo {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: egui::Key,
}
impl Combo {
    /// Reads "Ctrl+Shift+F5"-style text (as saved in the project).
    pub fn parse(text: &str) -> Option<Self> {
        let mut combo = Combo { ctrl: false, alt: false, shift: false, key: egui::Key::A };
        let mut key = None;
        for part in text.split('+').map(str::trim).filter(|p| !p.is_empty()) {
            match part.to_lowercase().as_str() {
                "ctrl" | "control" => combo.ctrl = true,
                "alt" => combo.alt = true,
                "shift" => combo.shift = true,
                _ => key = egui::Key::from_name(part),
            }
        }
        combo.key = key?;
        Some(combo)
    }
    pub fn text(&self) -> String {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.shift {
            parts.push("Shift");
        }
        parts.push(self.key.name());
        parts.join("+")
    }
    fn modifiers_match(&self, m: &egui::Modifiers) -> bool {
        (m.ctrl || m.command) == self.ctrl && m.alt == self.alt && m.shift == self.shift
    }
    /// Whether this combination was pressed this frame (in Rosorah's own window).
    pub fn pressed(&self, input: &egui::InputState) -> bool {
        input.events.iter().any(|e| matches!(e, egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } if *key == self.key && self.modifiers_match(modifiers)))
    }
    pub fn released(&self, input: &egui::InputState) -> bool {
        input.events.iter().any(|e| matches!(e, egui::Event::Key { key, pressed: false, .. } if *key == self.key))
    }
    /// The XDG shortcut form ("CTRL+SHIFT+F5") offered to the desktop as the suggested keys.
    #[cfg(not(windows))]
    fn trigger(&self) -> String {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("CTRL".to_owned());
        }
        if self.alt {
            parts.push("ALT".to_owned());
        }
        if self.shift {
            parts.push("SHIFT".to_owned());
        }
        parts.push(self.key.name().to_owned());
        parts.join("+")
    }
}
/// A key going down or up, from wherever the system reported it.
#[derive(Clone, Copy, Debug)]
pub enum Event {
    Pressed(usize),
    Released(usize),
}
/// System-wide hotkeys for a list of combinations (indexes match the list).
pub struct Global {
    pub events: mpsc::Receiver<Event>,
    /// Which combinations the system took; those aren't also handled as in-app keys.
    pub registered: Arc<Vec<AtomicBool>>,
    stop: Arc<AtomicBool>,
    pub status: Arc<std::sync::Mutex<String>>,
}
impl Drop for Global {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
impl Global {
    pub fn start(combos: Vec<Combo>, descriptions: Vec<String>) -> Self {
        let (tx, events) = mpsc::channel();
        let registered = Arc::new(combos.iter().map(|_| AtomicBool::new(false)).collect::<Vec<_>>());
        let stop = Arc::new(AtomicBool::new(false));
        let status = Arc::new(std::sync::Mutex::new(String::new()));
        let (r, s, st) = (registered.clone(), stop.clone(), status.clone());
        std::thread::spawn(move || {
            if let Err(e) = platform(combos, descriptions, tx, &r, &s) {
                *st.lock().unwrap() = format!("Hotkeys work while Rosorah is focused (system-wide: {e:#})");
            }
        });
        Self { events, registered, stop, status }
    }
}

#[cfg(windows)]
fn platform(combos: Vec<Combo>, _: Vec<String>, tx: mpsc::Sender<Event>, registered: &[AtomicBool], stop: &AtomicBool) -> anyhow::Result<()> {
    let keys: Vec<(u32, bool, bool, bool)> = combos.iter().map(|c| (virtual_key(c.key), c.ctrl, c.alt, c.shift)).collect();
    rosorah_windows::hotkeys::run(&keys, stop, |index, down| {
        let _ = tx.send(if down { Event::Pressed(index) } else { Event::Released(index) });
    }, |index| registered[index].store(true, Ordering::Relaxed))
}
/// Windows' key code for an egui key.
#[cfg(windows)]
fn virtual_key(key: egui::Key) -> u32 {
    use egui::Key as K;
    let name = key.name();
    match key {
        K::Space => 0x20,
        K::Enter => 0x0D,
        K::Tab => 0x09,
        K::Escape => 0x1B,
        K::Backspace => 0x08,
        K::Insert => 0x2D,
        K::Delete => 0x2E,
        K::Home => 0x24,
        K::End => 0x23,
        K::PageUp => 0x21,
        K::PageDown => 0x22,
        K::ArrowLeft => 0x25,
        K::ArrowUp => 0x26,
        K::ArrowRight => 0x27,
        K::ArrowDown => 0x28,
        _ if name.len() == 1 => name.chars().next().unwrap().to_ascii_uppercase() as u32,
        _ if name.starts_with('F') => name[1..].parse::<u32>().map_or(0, |n| 0x6F + n),
        _ => 0,
    }
}
#[cfg(not(windows))]
fn platform(combos: Vec<Combo>, descriptions: Vec<String>, tx: mpsc::Sender<Event>, registered: &[AtomicBool], stop: &AtomicBool) -> anyhow::Result<()> {
    use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
    use futures_util::StreamExt;
    if combos.is_empty() {
        return Ok(());
    }
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    runtime.block_on(async {
        let portal = GlobalShortcuts::new().await?;
        let session = portal.create_session(Default::default()).await?;
        let shortcuts: Vec<NewShortcut> = combos
            .iter()
            .zip(&descriptions)
            .enumerate()
            .map(|(i, (c, d))| NewShortcut::new(format!("rosorah-{i}"), d.as_str()).preferred_trigger(c.trigger().as_str()))
            .collect();
        // The desktop may ask once to confirm (KDE shows a dialog); after that they're system-wide.
        let bound = portal.bind_shortcuts(&session, &shortcuts, None, Default::default()).await?.response()?;
        for shortcut in bound.shortcuts() {
            if let Some(i) = shortcut.id().strip_prefix("rosorah-").and_then(|n| n.parse::<usize>().ok())
                && let Some(flag) = registered.get(i)
            {
                flag.store(true, Ordering::Relaxed);
            }
        }
        let mut activated = portal.receive_activated().await?;
        let mut deactivated = portal.receive_deactivated().await?;
        let index = |id: &str| id.strip_prefix("rosorah-").and_then(|n| n.parse::<usize>().ok());
        while !stop.load(Ordering::Relaxed) {
            tokio::select! {
                Some(a) = activated.next() => if let Some(i) = index(a.shortcut_id()) { let _ = tx.send(Event::Pressed(i)); },
                Some(d) = deactivated.next() => if let Some(i) = index(d.shortcut_id()) { let _ = tx.send(Event::Released(i)); },
                _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {}
            }
        }
        let _ = session.close().await;
        anyhow::Ok(())
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn combos_read_and_write_the_same() {
        let combo = Combo::parse("Ctrl+Shift+F5").unwrap();
        assert!(combo.ctrl && combo.shift && !combo.alt && combo.key == egui::Key::F5);
        assert_eq!(combo.text(), "Ctrl+Shift+F5");
        assert_eq!(Combo::parse("alt+K").unwrap().text(), "Alt+K");
        assert!(Combo::parse("Ctrl+").is_none());
    }
}
