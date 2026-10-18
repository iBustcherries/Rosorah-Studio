//! Account-scoped activity, independent of the transient chat buffer.
use auroracast_core::{EventKind, Platform};
use eframe::egui;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};
const LIMIT: usize = 5000;
const RETENTION_MS: u64 = 30 * 24 * 60 * 60 * 1000;
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    #[serde(default)]
    pub facts: auroracast_core::automation::Facts,
    #[serde(default)]
    pub reward_id: String,
    #[serde(default)]
    pub goal_count: u64,
    pub platform: Platform,
    pub owner: String,
    pub id: String,
    pub kind: EventKind,
    pub author: String,
    pub text: String,
    pub at_ms: u64,
    #[serde(default)]
    pub historical: bool,
}
pub fn twitch_kind(payload: &Value) -> Option<EventKind> {
    use EventKind::*;
    Some(match payload["subscription"]["type"].as_str()? {
        "channel.follow" => Follow,
        "channel.subscribe" | "channel.subscription.message" => Subscription,
        "channel.subscription.gift" => Gift,
        "channel.cheer" => Bits,
        "channel.raid" => Raid,
        "channel.channel_points_custom_reward_redemption.add" => Redemption,
        "channel.hype_train.begin" | "channel.hype_train.progress" | "channel.hype_train.end" => {
            HypeTrain
        }
        "channel.poll.begin" | "channel.poll.end" => Poll,
        "channel.prediction.begin" | "channel.prediction.end" => Prediction,
        _ => return None,
    })
}
pub fn youtube_kind(value: &str) -> Option<EventKind> {
    Some(match value {
        "super_chat" => EventKind::SuperChat,
        "super_sticker" => EventKind::SuperSticker,
        "membership" => EventKind::Membership,
        "gift" => EventKind::Gift,
        _ => return None,
    })
}
/// TikTok events read from the LIVE page; shares have no alert kind and stay chat-only.
pub fn tiktok_kind(value: &str) -> Option<EventKind> {
    Some(match value {
        "gift" => EventKind::Gift,
        "follow" => EventKind::Follow,
        "subscribe" => EventKind::Subscription,
        _ => return None,
    })
}
pub fn subscribers(v: &Value, owner: &str, historical: bool) -> Vec<Item> {
    v["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let id = s["id"].as_str()?.to_string();
            let author = s["subscriberSnippet"]["title"].as_str()?.to_string();
            let at = gst::glib::DateTime::from_iso8601(s["snippet"]["publishedAt"].as_str()?, None)
                .ok()?
                .to_unix();
            if id.is_empty()
                || author.is_empty()
                || at < 0
                || (at as u64 * 1000) < now().saturating_sub(RETENTION_MS)
            {
                return None;
            }
            Some(Item {
                facts: Default::default(),
                reward_id: String::new(),
                goal_count: 0,
                platform: Platform::YouTube,
                owner: owner.into(),
                id: format!("subscriber-{id}"),
                kind: EventKind::Subscriber,
                text: format!("{author} subscribed to your channel"),
                author,
                at_ms: at as u64 * 1000,
                historical,
            })
        })
        .collect()
}
pub struct History {
    pub replay: Option<Item>,
    items: Vec<Item>,
    dirty: bool,
    last_save: Instant,
    writer: Option<mpsc::SyncSender<Vec<Item>>>,
    done: mpsc::Receiver<Result<(), String>>,
    worker: Option<std::thread::JoinHandle<()>>,
    error: String,
}
impl History {
    pub fn load(path: PathBuf) -> Self {
        let mut items: Vec<Item> = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        items.retain(|i| !i.owner.is_empty() && i.at_ms >= now().saturating_sub(RETENTION_MS));
        items.sort_by_key(|i| i.at_ms);
        if items.len() > LIMIT {
            items.drain(..items.len() - LIMIT);
        }
        let (writer, rx) = mpsc::sync_channel::<Vec<Item>>(1);
        let (tx, done) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            while let Ok(items) = rx.recv() {
                let _ = tx.send(save(&path, &items).map_err(|e| e.to_string()));
            }
        });
        Self {
            replay: None,
            items,
            dirty: true,
            last_save: Instant::now(),
            writer: Some(writer),
            done,
            worker: Some(worker),
            error: String::new(),
        }
    }
    /// True only for a newly received event, never for a replay or an edit.
    pub fn push(&mut self, item: Item) -> bool {
        let live = !item.historical;
        self.insert_new(item) && live
    }
    /// True for a newly stored item, including missed donation history.
    pub fn insert_new(&mut self, mut item: Item) -> bool {
        if item.owner.is_empty()
            || item.id.is_empty()
            || item.at_ms < now().saturating_sub(RETENTION_MS)
        {
            return false;
        }
        item.text = item.text.chars().take(5000).collect();
        item.author = item.author.chars().take(100).collect();
        if let Some(old) = self
            .items
            .iter_mut()
            .find(|i| i.platform == item.platform && i.owner == item.owner && i.id == item.id)
        {
            if old.text != item.text && old.text != "Message removed" {
                old.text = item.text;
                self.dirty = true;
            }
            return false;
        }
        let pos = self.items.partition_point(|i| i.at_ms <= item.at_ms);
        self.items.insert(pos, item);
        self.items
            .retain(|i| i.at_ms >= now().saturating_sub(RETENTION_MS));
        if self.items.len() > LIMIT {
            self.items.drain(..self.items.len() - LIMIT);
        }
        self.dirty = true;
        true
    }
    pub fn redact(&mut self, platform: Platform, owner: &str, id: &str) {
        for item in &mut self.items {
            if item.platform == platform && item.owner == owner && item.id == id {
                item.text = "Message removed".into();
                self.dirty = true;
            }
        }
    }
    pub fn flush(&mut self, force: bool) {
        for result in self.done.try_iter() {
            match result {
                Ok(()) => self.error.clear(),
                Err(e) => {
                    self.error = e;
                    self.dirty = true;
                }
            }
        }
        if self.dirty
            && (force || self.last_save.elapsed() >= Duration::from_secs(2))
            && let Some(tx) = &self.writer
        {
            let sent = if force {
                tx.send(self.items.clone()).is_ok()
            } else {
                tx.try_send(self.items.clone()).is_ok()
            };
            if sent {
                self.dirty = false;
                self.last_save = Instant::now();
            }
        }
    }
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        accounts: &[crate::accounts::Account; 2],
        donation_owners: &[String],
        tiktok: &str,
        show_missed: bool,
    ) {
        ui.heading("Stream activity");
        let connected: Vec<_> = accounts.iter().filter(|a| a.connected).collect();
        ui.label(if connected.is_empty() {
            "Your connected accounts will appear here.".into()
        } else {
            connected
                .iter()
                .map(|a| format!("{} · {}", a.platform.name(), a.name))
                .collect::<Vec<_>>()
                .join("  /  ")
        });
        ui.small("Newest first · Saved on this computer");
        ui.separator();
        let mut count = 0;
        egui::ScrollArea::vertical().id_salt("activity_history").show(ui,|ui| {
            for item in self.items.iter().rev().filter(|i|show_missed || !i.historical).filter(|i| (i.platform==Platform::Donations && donation_owners.contains(&i.owner)) || (i.platform==Platform::TikTok && !tiktok.is_empty() && i.owner==tiktok) || connected.iter().any(|a| a.platform==i.platform && a.user_id==i.owner)) {
                count+=1;
                let row=ui.push_id((&item.owner,&item.id),|ui| {
                    ui.horizontal_wrapped(|ui| {
                        let c=item.platform.color();
                        ui.colored_label(egui::Color32::from_rgb(c[0],c[1],c[2]),item.platform.name());
                        ui.strong(item.kind.label());
                        let time=gst::glib::DateTime::from_unix_local((item.at_ms/1000) as i64).ok().and_then(|d|d.format("%b %d · %H:%M:%S").ok()).map(|d|d.to_string()).unwrap_or_default();
                        ui.small(time);
                    });
                    if item.platform==Platform::YouTube && item.kind!=EventKind::Subscriber { ui.label(format!("{}: {}",item.author,item.text)); } else {ui.label(&item.text);}
                    if item.historical {ui.small(if item.platform==Platform::Donations {"Missed · history only"}else{"Recent history"});}
                    ui.separator();
                });
                row.response.context_menu(|ui|{if ui.button("Replay matching alerts (no goal changes)").clicked(){self.replay=Some(item.clone());ui.close();}});
            }
            if count==0 {ui.label("Waiting for activity. Events from your connected platforms appear together here.");}
        });
        ui.collapsing("About activity history",|ui| {
            ui.small("Up to 5,000 received events from the last 30 days. Events while Rosorah is closed may not be available. History is scoped to the signed-in accounts.");
            if connected.iter().any(|a|a.platform==Platform::YouTube) {ui.small("YouTube: Super Chats, stickers and memberships come from the active broadcast chat. Public subscribers are checked every 5 minutes with a selected broadcast, or every 30 minutes otherwise. Private subscribers cannot be listed.");}
        });
        if !self.error.is_empty() {
            ui.colored_label(
                egui::Color32::LIGHT_RED,
                format!("History could not be saved: {}", self.error),
            );
        }
    }
}
impl Drop for History {
    fn drop(&mut self) {
        self.flush(true);
        self.writer.take();
        if let Some(t) = self.worker.take() {
            let _ = t.join();
        }
    }
}
fn save(path: &std::path::Path, items: &[Item]) -> anyhow::Result<()> {
    use std::io::Write;
    #[cfg(unix)] use std::os::unix::fs::OpenOptionsExt;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension("json.tmp");
    let mut options=std::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)] options.mode(0o600);
    let mut file=options.open(&temp)?;
    serde_json::to_writer(&mut file, items)?;
    file.flush()?;
    std::fs::rename(temp, path)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persisted_dedup_scoping_and_redaction() {
        let path =
            std::env::temp_dir().join(format!("rosorah-activity-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let item = Item {
            facts: Default::default(),
            reward_id: String::new(),
            goal_count: 0,
            platform: Platform::YouTube,
            owner: "owner1".into(),
            id: "event1".into(),
            kind: EventKind::SuperChat,
            author: "Viewer".into(),
            text: "$5 hello".into(),
            at_ms: now(),
            historical: false,
        };
        {
            let mut h = History::load(path.clone());
            assert!(h.push(item.clone()));
            assert!(!h.push(item.clone()));
            let mut second = item.clone();
            second.owner = "owner2".into();
            assert!(h.push(second));
            h.redact(Platform::YouTube, "owner1", "event1");
        }
        {
            let mut h = History::load(path.clone());
            assert_eq!(h.items.len(), 2);
            assert_eq!(h.items[0].text, "Message removed");
            assert!(!h.push(item));
            assert_eq!(h.items[0].text, "Message removed");
        }
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn missed_donation_counts_once_without_becoming_live() {
        let path = std::env::temp_dir().join(format!("rosorah-missed-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let item = Item {
            facts: Default::default(),
            reward_id: String::new(),
            goal_count: 0,
            platform: Platform::Donations,
            owner: "Ko-fi:one".into(),
            id: "payment".into(),
            kind: EventKind::Donation,
            author: "Viewer".into(),
            text: "USD 12".into(),
            at_ms: now(),
            historical: true,
        };
        {
            let mut h = History::load(path.clone());
            assert!(h.insert_new(item.clone()));
            assert!(!h.insert_new(item.clone()));
            assert!(h.items[0].historical);
            assert!(!h.push(item.clone()));
        }
        {
            let mut h = History::load(path.clone());
            assert!(!h.insert_new(item));
        }
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn typed_events_and_subscriber_identity() {
        assert!(youtube_kind("chat").is_none());
        assert_eq!(youtube_kind("gift"), Some(EventKind::Gift));
        let date = gst::glib::DateTime::now_utc()
            .unwrap()
            .format("%Y-%m-%dT%H:%M:%SZ")
            .unwrap();
        let v = serde_json::json!({"items":[{"id":"sub1","snippet":{"title":"Owner","publishedAt":date.as_str()},"subscriberSnippet":{"title":"Subscriber"}}]});
        let rows = subscribers(&v, "owner", true);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].author, "Subscriber");
        assert!(rows[0].historical);
    }
}
