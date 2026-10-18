use super::*;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    net::TcpListener,
};
pub(super) const TWITCH_SCOPES: &str = "user:write:chat channel:read:stream_key channel:manage:broadcast moderator:read:followers channel:read:subscriptions bits:read channel:read:redemptions channel:manage:redemptions channel:read:hype_train channel:read:polls channel:read:predictions moderator:manage:banned_users moderator:manage:chat_messages";
pub(super) fn random() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("System random generator unavailable: {e}"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
pub(super) fn token(value: &Value, old_refresh: &str) -> Result<Token> {
    Ok(Token {
        access: field(value, "access_token")?.into(),
        refresh: value["refresh_token"]
            .as_str()
            .unwrap_or(old_refresh)
            .into(),
        expiry: now() / 1000 + value["expires_in"].as_u64().or_else(||value["expires_in"].as_str().and_then(|s|s.parse().ok())).unwrap_or(3600),
    })
}
pub(super) fn refresh(
    client: &Client,
    config: &ClientConfig,
    platform: Platform,
    old: &Token,
) -> Result<Token> {
    ensure!(!old.refresh.is_empty(), "Session expired; sign in again");
    if platform == Platform::Kick {
        return token(&json(client.post(kick::broker(config)?).json(&j!({
            "grant_type":"refresh_token", "client_id":config.id, "refresh_token":old.refresh
        })))?, &old.refresh);
    }
    let endpoint = if platform == Platform::Twitch {
        "https://id.twitch.tv/oauth2/token"
    } else {
        "https://oauth2.googleapis.com/token"
    };
    let mut form = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", old.refresh.as_str()),
        ("client_id", config.id.as_str()),
    ];
    if platform == Platform::YouTube && !config.secret.is_empty() {
        form.push(("client_secret", config.secret.as_str()));
    }
    token(&json(client.post(endpoint).form(&form))?, &old.refresh)
}
pub(super) fn login(
    client: &Client,
    config: &ClientConfig,
    platform: Platform,
    tx: &mpsc::SyncSender<Event>,
    cancel: &AtomicBool,
) -> Result<Token> {
    ensure!(
        !config.id.trim().is_empty(),
        "Application registration is required; enter the OAuth client ID in Account setup"
    );
    if platform == Platform::Twitch {
        twitch(client, config, tx, cancel)
    } else {
        loopback(client, config, platform, tx, cancel)
    }
}
fn twitch(
    client: &Client,
    config: &ClientConfig,
    tx: &mpsc::SyncSender<Event>,
    cancel: &AtomicBool,
) -> Result<Token> {
    let value = json(
        client
            .post("https://id.twitch.tv/oauth2/device")
            .form(&[("client_id", config.id.as_str()), ("scopes", TWITCH_SCOPES)]),
    )?;
    let code = field(&value, "device_code")?;
    let uri = field(&value, "verification_uri")?;
    let user_code = field(&value, "user_code")?;
    let url = reqwest::Url::parse(uri)?;
    ensure!(
        url.scheme() == "https" && url.host_str() == Some("www.twitch.tv"),
        "Unexpected Twitch authorization address"
    );
    let _ = tx.send(Event::Authorize(
        uri.into(),
        format!("Enter code {user_code} in your browser"),
    ));
    let deadline =
        Instant::now() + Duration::from_secs(value["expires_in"].as_u64().unwrap_or(600).min(1800));
    let mut interval = value["interval"].as_u64().unwrap_or(5).max(5);
    while Instant::now() < deadline && !cancel.load(Ordering::Relaxed) {
        if wait(cancel, Duration::from_secs(interval)) {
            break;
        }
        let response = client
            .post("https://id.twitch.tv/oauth2/token")
            .form(&[
                ("client_id", config.id.as_str()),
                ("device_code", code),
                ("scopes", TWITCH_SCOPES),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ])
            .send()?;
        let status = response.status();
        let value: Value = response.json()?;
        if status.is_success() {
            return token(&value, "");
        }
        match value["message"].as_str().or(value["error"].as_str()) {
            Some("authorization_pending") => (),
            Some("slow_down") => interval += 5,
            _ => anyhow::bail!("Twitch authorization failed or expired; try signing in again"),
        }
    }
    anyhow::bail!("Sign-in cancelled or expired")
}
pub(super) fn callback_code(target: &str, state: &str) -> Result<String> {
    ensure!(target.starts_with('/') && !target.starts_with("//"), "Invalid OAuth callback target");
    let url = reqwest::Url::parse(&format!("http://127.0.0.1{target}"))?;
    ensure!(url.path() == "/oauth/callback", "Not an OAuth callback");
    let mut pairs = std::collections::HashMap::new();
    for (key,value) in url.query_pairs().into_owned() {
        ensure!(pairs.insert(key,value).is_none(), "Duplicate OAuth callback parameter");
    }
    ensure!(
        pairs.get("state").map(String::as_str) == Some(state),
        "OAuth state mismatch"
    );
    ensure!(!pairs.contains_key("error"), "Authorization was declined");
    pairs
        .get("code")
        .filter(|code| !code.is_empty())
        .cloned()
        .context("Missing authorization code")
}
fn loopback(
    client: &Client,
    config: &ClientConfig,
    platform: Platform,
    tx: &mpsc::SyncSender<Event>,
    cancel: &AtomicBool,
) -> Result<Token> {
    let is_kick = platform == Platform::Kick;
    if is_kick { kick::broker(config)?; }
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, if is_kick {17653} else {0}))
        .context("Cannot open local sign-in callback")?;
    listener.set_nonblocking(true)?;
    let redirect = format!(
        "http://{}:{}/oauth/callback",
        if is_kick {"localhost"} else {"127.0.0.1"},
        listener.local_addr()?.port()
    );
    let state = random()?;
    let verifier = random()?;
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let mut url = reqwest::Url::parse(if is_kick {"https://id.kick.com/oauth/authorize"} else {"https://accounts.google.com/o/oauth2/v2/auth"})?;
    url.query_pairs_mut().extend_pairs([
        ("client_id", config.id.as_str()),
        ("redirect_uri", &redirect),
        ("response_type", "code"),
        ("scope", if is_kick {"user:read channel:read streamkey:read chat:write channel:write moderation:ban moderation:chat_message:manage events:subscribe"} else {"https://www.googleapis.com/auth/youtube.force-ssl"}),
        ("state", &state),
        ("code_challenge", &challenge),
        ("code_challenge_method", "S256"),
    ]);
    if !is_kick {url.query_pairs_mut().extend_pairs([("access_type","offline"),("prompt","consent")]);}
    let _ = tx.send(Event::Authorize(
        url.into(),
        format!("Complete {} sign-in in your browser", if is_kick {"Kick"} else {"Google"}),
    ));
    let deadline = Instant::now() + Duration::from_secs(600);
    while Instant::now() < deadline && !cancel.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                stream.set_write_timeout(Some(Duration::from_secs(2)))?;
                let mut bytes = [0u8; 8192];
                let mut count = 0;
                while count < bytes.len() && !bytes[..count].contains(&b'\n') {
                    match stream.read(&mut bytes[count..]) {
                        Ok(0) => break,
                        Ok(n) => count += n,
                        Err(_) => break,
                    }
                }
                let request = String::from_utf8_lossy(&bytes[..count]);
                let target = request
                    .lines()
                    .next()
                    .and_then(|l| l.strip_prefix("GET "))
                    .and_then(|l| l.split_whitespace().next())
                    .unwrap_or("");
                let result = callback_code(target, &state);
                let body = if result.is_ok() {
                    "Sign-in received. You can return to Rosorah Studio."
                } else {
                    "Invalid or declined sign-in. Return to Rosorah and try again."
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(),
                    body
                );
                if let Ok(code) = result {
                    if is_kick {
                        return token(&json(client.post(kick::broker(config)?).json(&j!({
                            "grant_type":"authorization_code", "client_id":config.id,
                            "code":code, "code_verifier":verifier, "redirect_uri":redirect
                        })))?, "");
                    }
                    let mut form = vec![
                        ("client_id", config.id.as_str()),
                        ("code", &code),
                        ("code_verifier", &verifier),
                        ("grant_type", "authorization_code"),
                        ("redirect_uri", &redirect),
                    ];
                    if !config.secret.is_empty() {
                        form.push(("client_secret", &config.secret));
                    }
                    return token(
                        &json(
                            client
                                .post("https://oauth2.googleapis.com/token")
                                .form(&form),
                        )?,
                        "",
                    );
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                wait(cancel, Duration::from_millis(100));
            }
            Err(e) => return Err(e.into()),
        }
    }
    anyhow::bail!("Sign-in cancelled or expired")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_requires_state_and_exact_path() {
        assert_eq!(
            callback_code("/oauth/callback?code=abc%2Bdef&state=secret", "secret").unwrap(),
            "abc+def"
        );
        for s in [
            "/oauth/callback?code=a",
            "/oauth/callback?code=a&state=bad",
            "/?code=a&state=secret",
            "/oauth/callback?error=access_denied&state=secret",
            "/oauth/callback?code=&state=secret",
            "/oauth/callback?code=a&state=secret&state=other",
            "//evil.example/oauth/callback?code=a&state=secret",
        ] {
            assert!(callback_code(s, "secret").is_err());
        }
    }
    #[test]
    fn pkce_randomness_is_url_safe_and_unique() {
        let a=random().unwrap(); let b=random().unwrap();
        assert_eq!(a.len(),43); assert_ne!(a,b);
        assert!(a.bytes().all(|b|b.is_ascii_alphanumeric() || b==b'_' || b==b'-'));
    }
    #[test]
    fn refresh_rotation_preserves_google_refresh_when_omitted() {
        assert_eq!(
            token(
                &serde_json::json!({"access_token":"a","expires_in":20}),
                "old"
            )
            .unwrap()
            .refresh,
            "old"
        );
        assert_eq!(
            token(
                &serde_json::json!({"access_token":"a","refresh_token":"new"}),
                "old"
            )
            .unwrap()
            .refresh,
            "new"
        );
    }
}
