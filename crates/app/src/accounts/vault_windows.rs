//! Windows implementation of the shared account vault contract.
use super::*;
use rosorah_windows::credentials;
fn key(platform: Platform, client: &str) -> String {
    format!("{}/{}", platform.name(), client)
}
fn marker(platform: Platform, client: &str) -> String {
    format!("remember/{}", key(platform, client))
}
pub(super) fn remembered(platform: Platform, client: &str) -> bool {
    credentials::read(&marker(platform, client))
        .ok()
        .flatten()
        .is_some()
}
pub(super) fn load(
    platform: Platform,
    config: &ClientConfig,
    cancel: &AtomicBool,
) -> Result<Option<Token>> {
    ensure!(!cancel.load(Ordering::Relaxed), "Sign-in cancelled");
    credentials::read(&key(platform, &config.vault_id()))?
        .map(|v| serde_json::from_slice(&v).context("Saved sign-in could not be read"))
        .transpose()
}
pub(super) fn save(
    platform: Platform,
    config: &ClientConfig,
    token: &Token,
    cancel: &AtomicBool,
) -> Result<()> {
    ensure!(!cancel.load(Ordering::Relaxed), "Sign-in cancelled");
    ensure!(
        !token.refresh.is_empty(),
        "This account did not provide a renewable sign-in"
    );
    credentials::write(&key(platform, &config.vault_id()), &serde_json::to_vec(token)?)?;
    if !cancel.load(Ordering::Relaxed) {
        credentials::write(&marker(platform, &config.vault_id()), b"1")?;
    }
    Ok(())
}
pub(super) fn forget_marker(platform: Platform, config: &ClientConfig) {
    let _ = credentials::delete(&marker(platform, &config.vault_id()));
}
pub(super) fn forget(platform: Platform, config: &ClientConfig) {
    forget_marker(platform, config);
    let _ = credentials::delete(&key(platform, &config.vault_id()));
}
fn secret(
    client: &str,
    value: Option<&str>,
    remove: bool,
    cancel: &AtomicBool,
) -> Result<Option<String>> {
    ensure!(
        !cancel.load(Ordering::Relaxed),
        "Credential operation cancelled"
    );
    let key = key(Platform::Donations, client);
    if remove {
        credentials::delete(&key)?;
        return Ok(None);
    }
    if let Some(value) = value {
        credentials::write(&key, value.as_bytes())?;
        return Ok(None);
    }
    credentials::read(&key)?
        .map(|v| String::from_utf8(v).context("Saved connection could not be read"))
        .transpose()
}
pub(super) fn donation_secret(
    value: Option<&str>,
    remove: bool,
    cancel: &AtomicBool,
) -> Result<Option<String>> {
    secret("donations-v1", value, remove, cancel)
}
pub(super) fn kofi_secret(
    value: Option<&str>,
    remove: bool,
    cancel: &AtomicBool,
) -> Result<Option<String>> {
    secret("kofi-v1", value, remove, cancel)
}
pub(super) fn stream_keys_secret(
    value: Option<&str>,
    remove: bool,
    cancel: &AtomicBool,
) -> Result<Option<String>> {
    secret("stream-keys-v1", value, remove, cancel)
}
