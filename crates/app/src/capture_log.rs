//! Bounded, in-memory diagnostics. Never records pixels, audio or environment values.
use std::{
    collections::VecDeque,
    sync::{Mutex, OnceLock},
};
static EVENTS: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();
pub fn event(message: impl Into<String>) {
    let mut events = EVENTS.get_or_init(Default::default).lock().unwrap();
    if events.len() == 100 {
        events.pop_front();
    }
    events.push_back(message.into());
}
pub fn details() -> String {
    format!(
        "Rosorah Studio {}\nDesktop: {}\nSession: {}\n{}",
        env!("CARGO_PKG_VERSION"),
        std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
        std::env::var("XDG_SESSION_TYPE").unwrap_or_default(),
        EVENTS
            .get_or_init(Default::default)
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    )
}
