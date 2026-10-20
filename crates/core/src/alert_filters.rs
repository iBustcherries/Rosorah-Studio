//! Literal matching: no user-provided regexes or work on every render frame.
use crate::{ChatMessage, Event, EventKind, Platform};
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChatMetadata {
    #[serde(default)]
    pub moderator: bool,
    #[serde(default)]
    pub subscriber: bool,
    pub user_id: String,
    pub first_ever: bool,
    pub historical: bool,
    /// The chatter's name colour ("#rrggbb"), where the platform gives one.
    #[serde(default)]
    pub color: String,
    /// Badge pictures (https addresses) or roles ("moderator", "vip"…), in the order shown.
    #[serde(default)]
    pub badges: Vec<String>,
    /// "paid", "first" or "highlight": messages the chat widget sets apart.
    #[serde(default)]
    pub highlight: String,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PhraseMode {
    #[default]
    WholeWords,
    Contains,
    Exact,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Conditions {
    pub author: String,
    pub phrase: String,
    pub phrase_mode: PhraseMode,
    pub case_sensitive: bool,
    pub reward_id: String,
}
impl Conditions {
    pub fn matches(&self, event: &Event) -> bool {
        if !self.author.trim().is_empty()
            && self.author.trim().trim_start_matches('@').to_lowercase()
                != event.author.to_lowercase()
        {
            return false;
        }
        if !self.reward_id.trim().is_empty() && self.reward_id.trim() != event.reward_id {
            return false;
        }
        let phrase = self.phrase.trim();
        if phrase.is_empty() {
            return true;
        }
        let (text, needle) = if self.case_sensitive {
            (event.text.clone(), phrase.to_string())
        } else {
            (event.text.to_lowercase(), phrase.to_lowercase())
        };
        match self.phrase_mode {
            PhraseMode::Contains => text.contains(&needle),
            PhraseMode::Exact => text.trim() == needle,
            PhraseMode::WholeWords => text.match_indices(&needle).any(|(start, _)| {
                let end = start + needle.len();
                let word = |c: char| c.is_alphanumeric() || c == '_';
                !text[..start].chars().next_back().is_some_and(word)
                    && !text[end..].chars().next().is_some_and(word)
            }),
        }
    }
}
/// Session identity survives reconnects; explicitly reset at stream start.
#[derive(Default)]
pub struct ChatSession {
    seen: HashSet<(Platform, String)>,
    order: VecDeque<(Platform, String)>,
    users: HashSet<(Platform, String)>,
    started_ms: u64,
}
impl ChatSession {
    pub fn start(&mut self, at_ms: u64) {
        self.users.clear();
        self.started_ms = at_ms;
    }
    pub fn messages(&mut self, message: &ChatMessage) -> Vec<Event> {
        if message.demo
            || message.metadata.historical
            || message.id.starts_with("event-")
            || message.received_ms < self.started_ms
            || message.id.is_empty()
        {
            return Vec::new();
        }
        let key = (message.platform, message.id.clone());
        if !self.seen.insert(key.clone()) {
            return Vec::new();
        }
        self.order.push_back(key);
        while self.order.len() > 8192 {
            if let Some(old) = self.order.pop_front() {
                self.seen.remove(&old);
            }
        }
        let identity = if message.metadata.user_id.is_empty() {
            message.author.to_lowercase()
        } else {
            message.metadata.user_id.clone()
        };
        // Saturate rather than evict users: eviction would generate false arrivals.
        let first = !identity.is_empty()
            && self.users.len() < 100_000
            && self.users.insert((message.platform, identity));
        let mut kinds = vec![EventKind::ChatMessage, EventKind::ChatCommand];
        if first {
            kinds.push(EventKind::FirstMessage);
        }
        if message.metadata.first_ever {
            kinds.push(EventKind::FirstTimeChatter);
        }
        kinds
            .into_iter()
            .map(|kind| Event {
                facts: crate::automation::Facts {
                    moderator: message.metadata.moderator,
                    subscriber: message.metadata.subscriber,
                    ..Default::default()
                },
                id: message.id.clone(),
                platform: message.platform,
                kind,
                author: message.author.clone(),
                text: message.text.clone(),
                at_ms: message.received_ms,
                reward_id: String::new(),
            })
            .collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn message(id: &str) -> ChatMessage {
        ChatMessage {
            metadata: ChatMetadata {
                user_id: "42".into(),
                ..Default::default()
            },
            platform: Platform::Twitch,
            id: id.into(),
            author: "Moe".into(),
            text: "Hello, world!".into(),
            received_ms: 100,
            demo: false,
        }
    }
    #[test]
    fn word_boundaries_case_and_reward() {
        let event = ChatSession::default().messages(&message("a")).remove(0);
        let mut c = Conditions {
            phrase: "hello".into(),
            ..Default::default()
        };
        assert!(c.matches(&event));
        c.phrase = "hell".into();
        assert!(!c.matches(&event));
        c.phrase_mode = PhraseMode::Contains;
        assert!(c.matches(&event));
        c.author = "someone_else".into();
        assert!(!c.matches(&event));
        c.author = "@moe".into();
        c.reward_id = "reward".into();
        assert!(!c.matches(&event));
    }
    #[test]
    fn first_message_reconnect_reset_history() {
        let mut session = ChatSession::default();
        assert!(
            session
                .messages(&message("a"))
                .iter()
                .any(|e| e.kind == EventKind::FirstMessage)
        );
        assert!(session.messages(&message("a")).is_empty());
        assert!(
            !session
                .messages(&message("b"))
                .iter()
                .any(|e| e.kind == EventKind::FirstMessage)
        );
        session.start(99);
        assert!(
            session
                .messages(&message("c"))
                .iter()
                .any(|e| e.kind == EventKind::FirstMessage)
        );
        let mut old = message("d");
        old.metadata.historical = true;
        assert!(session.messages(&old).is_empty());
        session.start(101);
        assert!(session.messages(&message("e")).is_empty());
    }
}

#[cfg(test)]
mod routing_tests {
    use super::*;
    use crate::{Action, AlertEngine, Project, Rule};
    #[test]
    fn same_message_can_fire_phrase_and_arrival_rules_but_not_twice() {
        let mut rules = Project::default().rules;
        rules[0].event = EventKind::ChatMessage;
        rules[0].conditions.phrase = "hello".into();
        rules[0].actions = vec![Action::UpdateWidget {
            source: 1,
            text: "Hi {user}".into(),
            increment: 1,
            start_timer: false,
        }];
        let mut arrival = rules[0].clone();
        arrival.id += 1;
        arrival.event = EventKind::FirstMessage;
        rules.push(arrival);
        let m = ChatMessage {
            platform: Platform::Twitch,
            id: "42".into(),
            author: "Moe".into(),
            text: "hello!".into(),
            received_ms: 10000,
            demo: false,
            metadata: Default::default(),
        };
        let mut engine = AlertEngine::default();
        let mut session = ChatSession::default();
        let actions: Vec<_> = session
            .messages(&m)
            .iter()
            .flat_map(|e| engine.dispatch(&rules, e))
            .collect();
        assert_eq!(actions.len(), 2);
        assert!(matches!(&actions[0],Action::UpdateWidget{text,..} if text=="Hi Moe"));
        assert!(session.messages(&m).is_empty());
    }
    #[test]
    fn old_rules_and_preferences_migrate() {
        let mut data = serde_json::to_value(Project::default()).unwrap();
        data["rules"][0]
            .as_object_mut()
            .unwrap()
            .remove("conditions");
        data["preferences"]
            .as_object_mut()
            .unwrap()
            .remove("donations");
        let p: Project = serde_json::from_value(data).unwrap();
        p.validate().unwrap();
        assert!(p.rules[0].conditions.reward_id.is_empty());
        let _: Rule = p.rules[0].clone();
    }
}
