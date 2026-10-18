use super::*;
#[derive(Clone)]
pub struct Draft {
    pub title: String,
    pub description: String,
    pub privacy: String,
    pub schedule: String,
    pub category: String,
    pub made_for_kids: bool,
    pub audience_set: bool,
    pub auto_start: bool,
    pub thumbnail: String,
    pub stream: String,
}
impl Default for Draft {
    fn default() -> Self {
        Self {
            title: String::new(),
            description: String::new(),
            privacy: "unlisted".into(),
            schedule: String::new(),
            category: "20".into(),
            made_for_kids: false,
            audience_set: false,
            auto_start: false,
            thumbnail: String::new(),
            stream: String::new(),
        }
    }
}
impl Draft {
    pub fn body(&self) -> Result<Value> {
        ensure!(
            !self.title.trim().is_empty() && self.title.chars().count() <= 100,
            "Title must contain 1–100 characters"
        );
        ensure!(
            self.description.chars().count() <= 5000,
            "Description exceeds 5,000 characters"
        );
        ensure!(
            ["public", "unlisted", "private"].contains(&self.privacy.as_str()),
            "Choose visibility"
        );
        ensure!(
            self.audience_set,
            "Choose whether this broadcast is made for kids"
        );
        let time = scheduled_time(&self.schedule)?;
        Ok(
            j!({"snippet":{"title":self.title.trim(),"description":self.description,"scheduledStartTime":time},"status":{"privacyStatus":self.privacy,"selfDeclaredMadeForKids":self.made_for_kids},"contentDetails":{"enableAutoStart":self.auto_start,"enableAutoStop":false,"enableDvr":true,"monitorStream":{"enableMonitorStream":false}}}),
        )
    }
}
// Local wall-clock input is deliberately round-tripped: reject normalized invalid dates/DST gaps.
fn scheduled_time(input: &str) -> Result<String> {
    let local = if input.trim().is_empty() {
        gst::glib::DateTime::from_unix_local((now()/1000+120) as i64)?
    } else {
        let bytes=input.as_bytes();
        ensure!(bytes.len()==16 && bytes[4]==b'-' && bytes[7]==b'-' && bytes[10]==b' ' && bytes[13]==b':' && bytes.iter().enumerate().all(|(i,b)|[4,7,10,13].contains(&i)||b.is_ascii_digit()), "Schedule format: YYYY-MM-DD HH:MM (your computer's local time)");
        let number=|a:usize,b:usize| input[a..b].parse::<i32>();
        let local=gst::glib::DateTime::new(&gst::glib::TimeZone::local(),number(0,4)?,number(5,7)?,number(8,10)?,number(11,13)?,number(14,16)?,0.)?;
        ensure!(local.format("%Y-%m-%d %H:%M")?.as_str()==input,"Invalid date or local daylight-saving time");
        ensure!(local.to_unix()>(now()/1000) as i64,"Schedule must be in the future");
        local
    };
    Ok(local.to_utc()?.format("%Y-%m-%dT%H:%M:%SZ")?.to_string())
}

pub(crate) fn thumbnail_bytes(path: &str) -> Result<(Vec<u8>, &'static str)> {
    use std::io::Read;
    let mut bytes = vec![];
    std::fs::File::open(path)?
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 2 * 1024 * 1024,
        "YouTube thumbnails must be at most 2 MiB"
    );
    let mime = match image::guess_format(&bytes)? {
        image::ImageFormat::Png => "image/png",
        image::ImageFormat::Jpeg => "image/jpeg",
        _ => anyhow::bail!("Choose a JPEG or PNG thumbnail"),
    };
    let mut reader = image::ImageReader::new(std::io::Cursor::new(&bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    reader.decode()?;
    Ok((bytes, mime))
}
impl Session {
    pub(super) fn bind_broadcast(
        &self,
        id: &str,
        stream: &str,
        client: &Client,
        token: &Token,
    ) -> Result<()> {
        json(
            self.yt(client, token, reqwest::Method::POST, "liveBroadcasts/bind")
                .query(&[
                    ("id", id),
                    ("streamId", stream),
                    ("part", "id,contentDetails"),
                ]),
        )?;
        Ok(())
    }
    pub(super) fn upload_thumbnail(
        &self,
        id: &str,
        path: &str,
        client: &Client,
        token: &Token,
    ) -> Result<()> {
        let (bytes, mime) = thumbnail_bytes(path)?;
        json(
            client
                .post("https://www.googleapis.com/upload/youtube/v3/thumbnails/set")
                .bearer_auth(&token.access)
                .query(&[("videoId", id), ("uploadType", "media")])
                .header("Content-Type", mime)
                .body(bytes),
        )?;
        Ok(())
    }
    pub(super) fn create_broadcast(
        &mut self,
        draft: &Draft,
        client: &Client,
        token: &Token,
        tx: &mpsc::SyncSender<Event>,
    ) -> Result<()> {
        ensure!(self.platform == Platform::YouTube, "YouTube only");
        let body = draft.body()?;
        if !draft.thumbnail.is_empty() {
            thumbnail_bytes(&draft.thumbnail)?;
        }
        let v = json(
            self.yt(client, token, reqwest::Method::POST, "liveBroadcasts")
                .query(&[("part", "snippet,status,contentDetails")])
                .json(&body),
        )?;
        let id = field(&v, "id")?.to_string();
        self.broadcast = id.clone();
        let _ = tx.try_send(Event::Selected(id.clone()));
        let _ = tx.try_send(Event::Metadata(draft.title.clone(), draft.category.clone()));
        // Keep the created ID even if a follow-up action fails; never automatically create a duplicate.
        let result = (|| -> Result<()> {
            let stream = if draft.stream.is_empty() {
                let value = json(self.yt(client, token, reqwest::Method::POST, "liveStreams")
                    .query(&[("part", "snippet,cdn,contentDetails")])
                    .json(&j!({"snippet":{"title":format!("Rosorah · {}",draft.title)},"cdn":{"ingestionType":"rtmp","resolution":"variable","frameRate":"variable"},"contentDetails":{"isReusable":true}})))?;
                field(&value, "id")?.to_string()
            } else {
                draft.stream.clone()
            };
            self.bind_broadcast(&id, &stream, client, token)?;
            if !draft.category.is_empty() {
                let v = json(
                    self.yt(client, token, reqwest::Method::GET, "videos")
                        .query(&[("part", "snippet"), ("id", id.as_str())]),
                )?;
                let body = updated_video(&v["items"][0], &draft.title, &draft.category)?;
                json(
                    self.yt(client, token, reqwest::Method::PUT, "videos")
                        .query(&[("part", "snippet")])
                        .json(&body),
                )?;
            }
            if !draft.thumbnail.is_empty() {
                self.upload_thumbnail(&id, &draft.thumbnail, client, token)?;
            }
            Ok(())
        })();
        if let Err(e) = self.select(id.clone(), client, token, tx) {
            let _ = tx.try_send(Event::Status(format!(
                "Broadcast created; details will reconnect automatically: {e}"
            )));
        }
        self.next_discovery = Instant::now() + Duration::from_secs(120);
        let detail = match result {
            Ok(()) => "Created and linked. Chat follows this broadcast and its stream key is supplied automatically.".into(),
            Err(e) => format!(
                "Created broadcast {id}, but a follow-up failed: {e}. Use the existing broadcast; do not create it again."
            ),
        };
        let _ = tx.try_send(Event::Status(detail));
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn creation_is_explicit_and_validated() {
        let mut d = Draft {
            title: "Test".into(),
            ..Default::default()
        };
        assert!(d.body().is_err());
        d.audience_set = true;
        let b = d.body().unwrap();
        assert_eq!(b["contentDetails"]["enableAutoStart"], false);
        assert_eq!(b["status"]["privacyStatus"], "unlisted");
        d.schedule = "2099-02-31 13:00".into();
        assert!(d.body().is_err());
    }
    #[test]
    fn schedule_rejects_invalid_dates_and_keeps_utc_output() {
        assert!(scheduled_time("").unwrap().ends_with('Z'));
        for bad in ["2099-02-31 13:00", "2099-13-01 13:00", "2001-01-01 13:00", "2099-02-28 25:00", "2099-02-28 13:00extra", "💥💥💥💥"] {
            assert!(scheduled_time(bad).is_err(), "accepted {bad}");
        }
        assert!(scheduled_time("2099-02-28 13:00").unwrap().ends_with('Z'));
    }
    #[test]
    fn thumbnail_checks_content_not_extension() {
        let p = std::env::temp_dir().join(format!("badthumb-{}.jpg", std::process::id()));
        std::fs::write(&p, b"not an image").unwrap();
        assert!(thumbnail_bytes(p.to_str().unwrap()).is_err());
        let _ = std::fs::remove_file(p);
    }
}
