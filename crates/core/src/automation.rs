use crate::{Action, Event, Id};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Facts {
    pub amount_hidden: bool,
    pub amount: u64,
    pub currency: String,
    pub count: u64,
    pub moderator: bool,
    pub subscriber: bool,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Permission {
    #[default]
    Everyone,
    Moderators,
    Subscribers,
    SelectedUsers,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    pub min_amount: u64,
    pub currency: String,
    pub min_count: u64,
    pub permission: Permission,
    pub users: String,
    pub per_user_ms: u64,
    pub scene: Option<Id>,
    pub hotkey: u8,
    pub missed_alerts: bool,
}
impl Options {
    pub fn reason(&self, e: &Event) -> Option<&'static str> {
        if self.min_amount > e.facts.amount {
            return Some("Amount below minimum or unavailable");
        }
        if !self.currency.is_empty() && !self.currency.eq_ignore_ascii_case(&e.facts.currency) {
            return Some("Currency does not match");
        }
        if self.min_count > e.facts.count {
            return Some("Count below minimum or unavailable");
        }
        let allowed = match self.permission {
            Permission::Everyone => true,
            Permission::Moderators => e.facts.moderator,
            Permission::Subscribers => e.facts.subscriber,
            Permission::SelectedUsers => self.users.split(',').any(|u| {
                u.trim()
                    .trim_start_matches('@')
                    .eq_ignore_ascii_case(&e.author)
            }),
        };
        if !allowed {
            return Some("Viewer permission does not match or is unavailable");
        }
        None
    }
}
pub fn expand(text: &str, e: &Event) -> String {
    let amount = if e.facts.amount_hidden {
        String::new()
    } else {
        format!("{}.{:02}", e.facts.amount / 100, e.facts.amount % 100)
    };
    let count = e.facts.count.to_string();
    let mut output = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        output.push_str(&rest[..start]);
        let tail = &rest[start..];
        let Some(end) = tail.find('}') else {
            output.push_str(tail);
            return output;
        };
        let key = &tail[..=end];
        let value = match key {
            "{user}" => e.author.as_str(),
            "{platform}" => e.platform.name(),
            "{message}" => e.text.as_str(),
            "{amount}" => amount.as_str(),
            "{currency}" => {
                if e.facts.amount_hidden {
                    ""
                } else {
                    e.facts.currency.as_str()
                }
            }
            "{count}" | "{bits}" | "{viewers}" => count.as_str(),
            "{reward}" => e.reward_id.as_str(),
            _ => key,
        };
        output.push_str(value);
        rest = &tail[end + 1..];
    }
    output.push_str(rest);
    output
}
pub fn actions(actions: &[Action], e: &Event) -> Vec<Action> {
    actions
        .iter()
        .map(|a| match a {
            Action::DisplayAlert { source, template, media, seconds } => Action::DisplayAlert { source: *source, template: expand(template,e), media:*media, seconds:*seconds },
            Action::SendChat { text, platforms } => Action::SendChat { text:expand(text,e), platforms: if platforms.is_empty(){vec![e.platform]}else{platforms.clone()} },
            Action::ShowAlert { template, seconds } => Action::ShowAlert {
                template: expand(template, e),
                seconds: *seconds,
            },
            Action::UpdateWidget {
                source,
                text,
                increment,
                start_timer,
            } => Action::UpdateWidget {
                source: *source,
                text: expand(text, e),
                increment: *increment,
                start_timer: *start_timer,
            },
            a => a.clone(),
        })
        .collect()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct HistoryPolicy {
    pub show_missed: bool,
    pub count_missed: bool,
}
impl Default for HistoryPolicy {
    fn default() -> Self {
        Self {
            show_missed: true,
            count_missed: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AlertEngine, EventKind, Platform, Project};
    fn event(id: &str, user: &str, time: u64) -> Event {
        Event {
            facts: Facts {
                amount: 500,
                currency: "USD".into(),
                count: 600,
                ..Default::default()
            },
            reward_id: String::new(),
            id: id.into(),
            platform: Platform::Twitch,
            kind: EventKind::Bits,
            author: user.into(),
            text: "hello".into(),
            at_ms: time,
        }
    }
    #[test]
    fn numeric_permission_cooldown_and_duplicates() {
        let mut r = Project::default().rules.remove(0);
        r.event = EventKind::Bits;
        r.platform = None;
        r.cooldown_ms = 0;
        r.options.min_count = 500;
        r.options.per_user_ms = 1000;
        let mut engine = AlertEngine::default();
        assert!(
            !engine
                .dispatch(&[r.clone()], &event("1", "a", 1000))
                .is_empty()
        );
        assert!(
            engine
                .dispatch(&[r.clone()], &event("1", "a", 3000))
                .is_empty()
        );
        assert!(
            engine
                .dispatch(&[r.clone()], &event("2", "a", 1500))
                .is_empty()
        );
        assert!(
            !engine
                .dispatch(&[r.clone()], &event("3", "b", 1500))
                .is_empty()
        );
        r.options.permission = Permission::Moderators;
        assert!(
            engine
                .dispatch(&[r.clone()], &event("4", "c", 4000))
                .is_empty()
        );
        let mut trusted = event("5", "c", 5000);
        trusted.facts.moderator = true;
        assert!(!engine.dispatch(&[r], &trusted).is_empty());
    }
    #[test]
    fn variables_are_literal_and_respect_privacy() {
        let mut e = event("1", "{not_code}", 1000);
        assert_eq!(
            expand("{user} {amount} {currency} {bits}", &e),
            "{not_code} 5.00 USD 600"
        );
        e.facts.amount_hidden = true;
        assert_eq!(expand("{amount}{currency}", &e), "");
    }
    #[test]
    fn legacy_rule_options_and_video_defaults() {
        let mut v = serde_json::to_value(Project::default()).unwrap();
        v["rules"][0].as_object_mut().unwrap().remove("options");
        v["preferences"]
            .as_object_mut()
            .unwrap()
            .remove("automation_history");
        let p: Project = serde_json::from_value(v).unwrap();
        p.validate().unwrap();
        assert!(p.preferences.automation_history.count_missed);
        assert_eq!(p.rules[0].options.hotkey, 0);
    }
}
