//! Kick account authorization shares the OS vault and account UI on both platforms.
//! The publisher's OAuth secret exists only on the token broker, never in the app.
//! Kick chat is read through the browser chat window, like Twitch and YouTube; this
//! session sends chat messages and edits the stream title/category through Kick's public API,
//! and receives follows, subs, gifts, Kicks and redemptions through the broker (kick_events).
use super::*;

const API: &str = "https://api.kick.com/public/v1";

pub(super) fn broker(config: &ClientConfig) -> Result<reqwest::Url> {
    let mut url = reqwest::Url::parse(&config.broker_url)
        .context("This build needs its publisher's Kick sign-in service configured")?;
    ensure!(url.scheme() == "https" && url.host_str().is_some()
        && url.username().is_empty() && url.password().is_none()
        && url.query().is_none() && url.fragment().is_none(),
        "Kick sign-in service must be a secure HTTPS address");
    let path = format!("{}/token", url.path().trim_end_matches('/'));
    url.set_path(&path);
    Ok(url)
}

/// Reads the channel, publishes its name/title/category/stream key to the UI and returns the broadcaster id.
fn refresh_channel(client: &Client, token: &Token, tx: &mpsc::SyncSender<Event>) -> Result<u64> {
    let value = json(client.get(format!("{API}/channels")).bearer_auth(&token.access))?;
    let channel = &value["data"][0];
    let id = channel["broadcaster_user_id"].as_u64().context("Kick channel unavailable")?;
    let name = field(channel, "slug")?;
    let _ = tx.send(Event::Connected(name.into(), id.to_string()));
    let category_id = channel["category"]["id"].as_u64().map(|v| v.to_string()).unwrap_or_default();
    let _ = tx.send(Event::Metadata(channel["stream_title"].as_str().unwrap_or_default().into(), category_id.clone()));
    if !category_id.is_empty() {
        let _ = tx.send(Event::Categories(vec![Choice {
            id: category_id,
            name: channel["category"]["name"].as_str().unwrap_or_default().into(),
        }]));
    }
    let stream = &channel["stream"];
    let server = field(stream, "url").context("Kick connected, but streaming access is unavailable")?;
    let key = field(stream, "key").context("Kick connected, but stream-key permission is missing; reconnect")?;
    let url = reqwest::Url::parse(server)?;
    ensure!(matches!(url.scheme(), "rtmp" | "rtmps") && url.host_str().is_some(), "Kick returned an invalid streaming server");
    let _ = tx.send(Event::Ingest(crate::streaming::with_application_path(server), key.into()));
    Ok(id)
}

/// People watching the channel's live stream, or None when it's offline.
fn viewers(client: &Client, token: &Token) -> Result<Option<u64>> {
    let value = json(client.get(format!("{API}/channels")).bearer_auth(&token.access))?;
    let stream = &value["data"][0]["stream"];
    Ok(if stream["is_live"] == true { stream["viewer_count"].as_u64() } else { None })
}

fn post_chat(client: &Client, token: &Token, channel: u64, text: &str) -> Result<()> {
    let text = text.trim();
    ensure!(!text.is_empty() && text.chars().count() <= 500, "Kick messages must be 1–500 characters");
    let value = json(client.post(format!("{API}/chat")).bearer_auth(&token.access)
        .json(&j!({"broadcaster_user_id": channel, "content": text, "type": "user"})))?;
    ensure!(value["data"]["is_sent"] == true, "Kick did not accept the message");
    Ok(())
}

/// Kick's chat page shows usernames only; a user's id is their channel's broadcaster id.
/// Message ids arrive separately through Kick activity, matched to the chat window's text.
fn moderate(client: &Client, token: &Token, channel: u64, events: &kick_events::Events, m: &Moderation) -> Result<()> {
    ensure!(m.action != ModAction::ClearChat, "Kick doesn't allow clearing chat from other apps; use kick.com");
    if m.action == ModAction::Delete {
        let id = events.message_id(&m.author, &m.text).context(
            "Rosorah hasn't received this message's Kick ID. Only messages sent while Kick activity is connected can be deleted")?;
        json(client.delete(format!("{API}/chat/{id}")).bearer_auth(&token.access)).map_err(|e| {
            if e.to_string().contains("HTTP 401") || e.to_string().contains("HTTP 403") {
                anyhow::anyhow!("{e} Enable the moderation:chat_message:manage permission for your Kick app, then sign out of Kick and connect again.")
            } else {
                e
            }
        })?;
        return Ok(());
    }
    let name = m.user.trim();
    ensure!(
        !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
        "This message has no Kick username"
    );
    let slug = name.to_ascii_lowercase().replace('_', "-");
    let user = json(client.get(format!("{API}/channels")).bearer_auth(&token.access).query(&[("slug", slug.as_str())]))?;
    let target = user["data"][0]["broadcaster_user_id"].as_u64().context("Kick user not found")?;
    let mut body = j!({"broadcaster_user_id": channel, "user_id": target});
    let request = if m.action == ModAction::Unban {
        client.delete(format!("{API}/moderation/bans"))
    } else {
        if let ModAction::Timeout(seconds) = m.action {
            // Kick timeouts are in whole minutes, 1 minute to 7 days.
            body["duration"] = seconds.div_ceil(60).clamp(1, 10080).into();
        }
        client.post(format!("{API}/moderation/bans"))
    };
    json(request.bearer_auth(&token.access).json(&body)).map_err(|e| {
        if e.to_string().contains("HTTP 401") || e.to_string().contains("HTTP 403") {
            anyhow::anyhow!("{e} Enable the moderation:ban permission for your Kick app, then sign out of Kick and connect again.")
        } else {
            e
        }
    })?;
    Ok(())
}

fn search_categories(client: &Client, token: &Token, query: &str) -> Result<Vec<Choice>> {
    let query = query.trim();
    ensure!(!query.is_empty(), "Enter a category to search for");
    let value = json(client.get(format!("{API}/categories")).bearer_auth(&token.access).query(&[("q", query)]))?;
    Ok(value["data"].as_array().into_iter().flatten().filter_map(|c| Some(Choice {
        id: c["id"].as_u64()?.to_string(),
        name: c["name"].as_str()?.into(),
    })).take(30).collect())
}

fn update_channel(client: &Client, token: &Token, title: &str, category: &str) -> Result<()> {
    let title = title.trim();
    ensure!(!title.is_empty() && title.chars().count() <= 140, "Title is empty or too long");
    let mut body = j!({"stream_title": title});
    if let Ok(id) = category.parse::<u64>() && id > 0 {
        body["category_id"] = j!(id);
    }
    json(client.patch(format!("{API}/channels")).bearer_auth(&token.access).json(&body))?;
    Ok(())
}

pub(super) fn session(client: &Client, config: &ClientConfig, mut token: Token,
    tx: &mpsc::SyncSender<Event>, commands: mpsc::Receiver<Command>, cancel: &AtomicBool) -> Result<()> {
    let identity = json(client.post("https://id.kick.com/oauth/token/introspect").bearer_auth(&token.access))?;
    ensure!(identity["data"]["active"] == true && identity["data"]["client_id"].as_str() == Some(config.id.as_str()),
        "Kick session is invalid or belongs to another application");
    let mut channel = retry_network(cancel, tx, || refresh_channel(client, &token, tx))?;
    let mut events = kick_events::Events::default();
    // Sign-ins made before chat and activity support lack those permissions.
    let granted = identity["data"]["scope"].as_str().unwrap_or_default();
    let upgraded = ["chat:write", "channel:write", "events:subscribe", "moderation:chat_message:manage"].iter().all(|s| granted.contains(s));
    if !granted.is_empty() && !upgraded {
        let _ = tx.send(Event::Status("Kick connected. Sign out and connect Kick again once to allow chat, title changes, activity and deleting messages.".into()));
    }
    let mut retry_at = Instant::now();
    let mut watching = ViewerCheck::default();
    while !cancel.load(Ordering::Relaxed) {
        if token.expiry <= now()/1000 + 90 && Instant::now() >= retry_at {
            match oauth::refresh(client, config, Platform::Kick, &token) {
                Ok(next) => {
                    token = next;
                    let _ = keep(Platform::Kick, config, &token);
                    // The broker authorizes the activity connection with the token it was opened with.
                    events.reset();
                }
                Err(e) if transient(&e) => {
                    retry_at = Instant::now() + Duration::from_secs(30);
                    let _ = tx.try_send(Event::Status("Kick connection interrupted; retrying in 30 seconds".into()));
                }
                Err(e) if revoked(&e) => anyhow::bail!("Kick sign-in expired or was revoked. Reconnect in Accounts."),
                Err(_) => {
                    retry_at = Instant::now() + Duration::from_secs(30);
                    let _ = tx.try_send(Event::Status("Kick sign-in renewal failed; retrying in 30 seconds".into()));
                }
            }
        }
        match commands.recv_timeout(Duration::from_millis(100)) {
            Ok(Command::Refresh | Command::Discover | Command::Reconnect) => {
                events.reset();
                match refresh_channel(client, &token, tx) {
                    Ok(id) => channel = id,
                    Err(e) => {let _ = tx.send(Event::Status(e.to_string()));}
                }
            }
            Ok(Command::Moderate(action)) => {
                let text = match moderate(client, &token, channel, &events, &action) {
                    Ok(()) => action.done(),
                    Err(e) => format!("Kick moderation failed: {e}"),
                };
                let _ = tx.send(Event::Moderated(text));
            }
            Ok(Command::Post {request, text, ..}) => {
                let (ok, text) = match post_chat(client, &token, channel, &text) {
                    Ok(()) => (true, "Sent".to_string()),
                    Err(e) => (false, format!("{e}. If permission is missing, sign out and connect Kick again. Your draft was kept; messages are never automatically resent.")),
                };
                let _ = tx.send(Event::PostResult(j!({"type":"post_result","platform":"Kick","request":request,"ok":ok,"text":text})));
            }
            Ok(Command::Search(query)) => match search_categories(client, &token, &query) {
                Ok(choices) => {let _ = tx.send(Event::Categories(choices));}
                Err(e) => {let _ = tx.send(Event::Status(e.to_string()));}
            },
            Ok(Command::Update {title, category}) => match update_channel(client, &token, &title, &category) {
                Ok(()) => {
                    let _ = tx.send(Event::Metadata(title.trim().into(), category));
                    let _ = tx.send(Event::Status("Stream title and category updated".into()));
                }
                Err(e) => {let _ = tx.send(Event::Status(format!("{e}. If permission is missing, sign out and connect Kick again.")));}
            },
            Ok(Command::Live(live)) => if live { watching.start() } else { watching.stop(tx) },
            Ok(_) => {let _ = tx.send(Event::Status("This option is not available for Kick".into()));}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => (),
        }
        events.poll(client, config, &token, channel, tx);
        if watching.due() {
            watching.done(viewers(client, &token), tx);
        }
    }
    Ok(())
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn broker_requires_https_and_no_embedded_credentials() {
        for url in ["http://example.com", "https://secret@example.com", "https://example.com?key=secret", "https://example.com/#fragment"] {
            assert!(broker(&ClientConfig {broker_url:url.into(), ..Default::default()}).is_err());
        }
        assert_eq!(broker(&ClientConfig {broker_url:"https://auth.example.com/kick/".into(), ..Default::default()}).unwrap().as_str(), "https://auth.example.com/kick/token");
    }
}
