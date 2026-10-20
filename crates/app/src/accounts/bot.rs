//! A bot account: a second sign-in (kept apart from the streamer's in the vault) that only sends
//! chat messages into the streamer's channel, for automations and timers.
use super::*;

/// The bot's own name and user id.
fn identity(client: &Client, config: &ClientConfig, platform: Platform, token: &Token) -> Result<(String, String)> {
    if platform == Platform::Twitch {
        let v = json(client.get("https://api.twitch.tv/helix/users").header("Client-Id", &config.id).bearer_auth(&token.access))?;
        let user = &v["data"][0];
        Ok((field(user, "display_name").or_else(|_| field(user, "login"))?.into(), field(user, "id")?.into()))
    } else {
        let v = json(client.get("https://api.kick.com/public/v1/users").bearer_auth(&token.access))?;
        let user = &v["data"][0];
        let id = user["user_id"].as_u64().context("Kick account unavailable")?;
        Ok((field(user, "name")?.into(), id.to_string()))
    }
}
/// Sends `text` as the bot into `channel` (the streamer's user id).
fn send(client: &Client, config: &ClientConfig, platform: Platform, token: &Token, bot: &str, channel: &str, text: &str) -> Result<()> {
    let text = text.trim();
    ensure!(!text.is_empty() && text.chars().count() <= 500, "Bot messages must be 1–500 characters");
    ensure!(!channel.is_empty(), "Connect your own {} account too, so the bot knows which chat to post in", platform.name());
    ensure!(channel != bot, "The bot is signed in as your own account. Disconnect it and sign in as the bot instead");
    if platform == Platform::Twitch {
        let v = json(
            client
                .post("https://api.twitch.tv/helix/chat/messages")
                .header("Client-Id", &config.id)
                .bearer_auth(&token.access)
                .json(&j!({"broadcaster_id": channel, "sender_id": bot, "message": text})),
        )?;
        ensure!(
            v["data"][0]["is_sent"] == true,
            "Twitch didn't send the bot's message ({}). Check that the bot isn't banned or timed out, and your chat settings",
            v["data"][0]["drop_reason"]["message"].as_str().unwrap_or("no reason given").chars().take(120).collect::<String>()
        );
    } else {
        let channel: u64 = channel.parse().context("Kick channel unknown")?;
        let v = json(
            client
                .post("https://api.kick.com/public/v1/chat")
                .bearer_auth(&token.access)
                .json(&j!({"broadcaster_user_id": channel, "content": text, "type": "user"})),
        )?;
        ensure!(v["data"]["is_sent"] == true, "Kick didn't accept the bot's message");
    }
    Ok(())
}
pub(super) fn session(
    platform: Platform,
    client: &Client,
    config: &ClientConfig,
    mut token: Token,
    tx: &mpsc::SyncSender<Event>,
    commands: mpsc::Receiver<Command>,
    cancel: &AtomicBool,
) -> Result<()> {
    ensure!(matches!(platform, Platform::Twitch | Platform::Kick), "Bot accounts work on Twitch and Kick");
    let (name, id) = retry_network(cancel, tx, || identity(client, config, platform, &token))?;
    let _ = tx.send(Event::Connected(name, id.clone()));
    let mut retry_at = Instant::now();
    while !cancel.load(Ordering::Relaxed) {
        if token.expiry <= now() / 1000 + 90 && Instant::now() >= retry_at {
            match oauth::refresh(client, config, platform, &token) {
                Ok(next) => {
                    token = next;
                    let _ = keep(platform, config, &token);
                }
                Err(e) if revoked(&e) => anyhow::bail!("The bot's sign-in expired or was revoked. Connect the bot again in Settings > Accounts."),
                Err(_) => {
                    retry_at = Instant::now() + Duration::from_secs(30);
                    let _ = tx.try_send(Event::Status("Bot sign-in renewal failed; retrying in 30 seconds".into()));
                }
            }
        }
        match commands.recv_timeout(Duration::from_millis(100)) {
            Ok(Command::Post { request, text, broadcast }) => {
                let (ok, text) = match send(client, config, platform, &token, &id, &broadcast, &text) {
                    Ok(()) => (true, "Sent".to_string()),
                    Err(e) => (false, format!("{e}")),
                };
                let _ = tx.send(Event::PostResult(j!({"type":"post_result","platform":platform.name(),"request":request,"ok":ok,"text":text})));
            }
            Ok(_) => (),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => (),
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bot_sign_ins_are_stored_apart_from_the_streamer() {
        let streamer = ClientConfig { id: "abc".into(), ..Default::default() };
        let bot = ClientConfig { bot: true, ..streamer.clone() };
        assert_eq!(streamer.vault_id(), "abc");
        assert_eq!(bot.vault_id(), "abc#bot");
        // Saved settings never carry the flag.
        assert!(!serde_json::to_string(&bot).unwrap().contains("bot"));
    }
}
