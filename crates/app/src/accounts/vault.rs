//! Refresh tokens stay in the desktop Secret Service (KWallet/GNOME Keyring), never project files.
use super::*;
use libloading::Library;
use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    fs,
    io::Write,
    os::unix::fs::OpenOptionsExt,
    ptr,
    sync::{Mutex, OnceLock},
};
static MARKERS: Mutex<()> = Mutex::new(());
fn marker_path() -> std::path::PathBuf {
    Config::path().with_file_name("remembered-accounts.json")
}
pub(super) fn remembered(platform: Platform, client: &str) -> bool {
    fs::read(marker_path())
        .ok()
        .and_then(|v| serde_json::from_slice::<Value>(&v).ok())
        .is_some_and(|v| v[platform.name()].as_str() == Some(client))
}
fn mark(platform: Platform, client: &str, enabled: bool) -> Result<()> {
    let _guard = MARKERS.lock().unwrap_or_else(|e| e.into_inner());
    let path = marker_path();
    let mut value = fs::read(&path)
        .ok()
        .and_then(|v| serde_json::from_slice::<Value>(&v).ok())
        .filter(Value::is_object)
        .unwrap_or(j!({}));
    if enabled {
        value[platform.name()] = client.into();
    } else if value[platform.name()].as_str() == Some(client) {
        value.as_object_mut().unwrap().remove(platform.name());
    }
    fs::create_dir_all(path.parent().unwrap())?;
    let temp = path.with_extension("tmp");
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&temp)?;
    file.write_all(&serde_json::to_vec(&value)?)?;
    file.sync_all()?;
    fs::rename(temp, path)?;
    Ok(())
}
// GLib retains static type names, enum data, and function pointers from libsecret.
// Dropping a per-operation Library unloads that data while GLib still references it.
// Keep one handle for the process lifetime; static OnceLock values are not dropped.
static SECRET_LIBRARY: OnceLock<std::result::Result<Library, String>> = OnceLock::new();
struct Vault {
    lib: &'static Library,
    schema: *mut c_void,
}
impl Vault {
    fn new() -> Result<Self> {
        // SAFETY: fixed system library and documented libsecret C ABI. No project-supplied library paths.
        unsafe {
            let lib = SECRET_LIBRARY
                .get_or_init(|| {
                    Library::new("libsecret-1.so.0")
                        .map_err(|_| "Desktop wallet is unavailable".to_string())
                })
                .as_ref()
                .map_err(|message| anyhow::anyhow!("{message}"))?;
            let new = lib.get::<unsafe extern "C" fn(*const c_char, c_int, ...) -> *mut c_void>(
                b"secret_schema_new\0",
            )?;
            let schema = new(
                c"io.rosorah.Account".as_ptr(),
                0,
                c"platform".as_ptr(),
                0_i32,
                c"client".as_ptr(),
                0_i32,
                ptr::null::<c_char>(),
            );
            ensure!(!schema.is_null(), "Cannot initialize desktop wallet");
            Ok(Self { lib, schema })
        }
    }
    fn operation(
        &self,
        platform: Platform,
        client: &str,
        password: Option<&str>,
        clear: bool,
        cancel: &AtomicBool,
    ) -> Result<Option<String>> {
        let platform = CString::new(platform.name())?;
        let client = CString::new(client)?;
        let password = password.map(CString::new).transpose()?;
        unsafe {
            let new = self
                .lib
                .get::<unsafe extern "C" fn() -> *mut c_void>(b"g_cancellable_new\0")?;
            let cancel_fn = *self
                .lib
                .get::<unsafe extern "C" fn(*mut c_void)>(b"g_cancellable_cancel\0")?;
            let unref = *self
                .lib
                .get::<unsafe extern "C" fn(*mut c_void)>(b"g_object_unref\0")?;
            let error_free = *self
                .lib
                .get::<unsafe extern "C" fn(*mut c_void)>(b"g_error_free\0")?;
            let cancellable = new();
            ensure!(!cancellable.is_null(), "Cannot open desktop wallet");
            let done = AtomicBool::new(false);
            let pointer = cancellable as usize;
            let result = std::thread::scope(|scope| -> Result<Option<String>> {
                scope.spawn(|| {
                    let start = Instant::now();
                    while !done.load(Ordering::Relaxed) {
                        if cancel.load(Ordering::Relaxed)
                            || start.elapsed() > Duration::from_secs(60)
                        {
                            cancel_fn(pointer as *mut c_void);
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(50));
                    }
                });
                let result = (|| -> Result<Option<String>> {
                    let mut error = ptr::null_mut();
                    if clear {
                        let f = self.lib.get::<unsafe extern "C" fn(
                            *const c_void,
                            *mut c_void,
                            *mut *mut c_void,
                            ...
                        ) -> c_int>(
                            b"secret_password_clear_sync\0"
                        )?;
                        f(
                            self.schema,
                            cancellable,
                            &mut error,
                            c"platform".as_ptr(),
                            platform.as_ptr(),
                            c"client".as_ptr(),
                            client.as_ptr(),
                            ptr::null::<c_char>(),
                        );
                    } else if let Some(password) = &password {
                        let f = self.lib.get::<unsafe extern "C" fn(
                            *const c_void,
                            *const c_char,
                            *const c_char,
                            *const c_char,
                            *mut c_void,
                            *mut *mut c_void,
                            ...
                        ) -> c_int>(
                            b"secret_password_store_sync\0"
                        )?;
                        let ok = f(
                            self.schema,
                            ptr::null(),
                            c"Rosorah Studio account".as_ptr(),
                            password.as_ptr(),
                            cancellable,
                            &mut error,
                            c"platform".as_ptr(),
                            platform.as_ptr(),
                            c"client".as_ptr(),
                            client.as_ptr(),
                            ptr::null::<c_char>(),
                        );
                        if !error.is_null() {
                            error_free(error);
                            anyhow::bail!("Desktop wallet could not save this sign-in");
                        }
                        ensure!(ok != 0, "Desktop wallet could not save this sign-in");
                    } else {
                        let f = self.lib.get::<unsafe extern "C" fn(
                            *const c_void,
                            *mut c_void,
                            *mut *mut c_void,
                            ...
                        )
                            -> *mut c_char>(
                            b"secret_password_lookup_sync\0"
                        )?;
                        let free = self
                            .lib
                            .get::<unsafe extern "C" fn(*mut c_char)>(b"secret_password_free\0")?;
                        let value = f(
                            self.schema,
                            cancellable,
                            &mut error,
                            c"platform".as_ptr(),
                            platform.as_ptr(),
                            c"client".as_ptr(),
                            client.as_ptr(),
                            ptr::null::<c_char>(),
                        );
                        if !error.is_null() {
                            error_free(error);
                            if !value.is_null() {
                                free(value);
                            }
                            anyhow::bail!("Unlock your desktop wallet to restore this account");
                        }
                        if value.is_null() {
                            return Ok(None);
                        }
                        let text = CStr::from_ptr(value).to_string_lossy().into_owned();
                        free(value);
                        return Ok(Some(text));
                    }
                    if !error.is_null() {
                        error_free(error);
                        anyhow::bail!("Desktop wallet operation failed");
                    }
                    Ok(None)
                })();
                done.store(true, Ordering::Relaxed);
                result
            });
            unref(cancellable);
            result
        }
    }
}
impl Drop for Vault {
    fn drop(&mut self) {
        unsafe {
            if let Ok(f) = self
                .lib
                .get::<unsafe extern "C" fn(*mut c_void)>(b"secret_schema_unref\0")
            {
                f(self.schema);
            }
        }
    }
}
pub(super) fn load(
    platform: Platform,
    config: &ClientConfig,
    cancel: &AtomicBool,
) -> Result<Option<Token>> {
    let value = Vault::new()?.operation(platform, &config.vault_id(), None, false, cancel)?;
    value
        .map(|v| serde_json::from_str(&v).context("Saved sign-in could not be read"))
        .transpose()
}
pub(super) fn save(
    platform: Platform,
    config: &ClientConfig,
    token: &Token,
    cancel: &AtomicBool,
) -> Result<()> {
    ensure!(
        !token.refresh.is_empty(),
        "This account did not provide a renewable sign-in"
    );
    Vault::new()?.operation(
        platform,
        &config.vault_id(),
        Some(&serde_json::to_string(token)?),
        false,
        cancel,
    )?;
    if !cancel.load(Ordering::Relaxed) {
        mark(platform, &config.vault_id(), true)?;
    }
    Ok(())
}
pub(super) fn forget_marker(platform: Platform, config: &ClientConfig) {
    let _ = mark(platform, &config.vault_id(), false);
}
pub(super) fn forget(platform: Platform, config: &ClientConfig) {
    let _ = mark(platform, &config.vault_id(), false);
    if let Ok(vault) = Vault::new() {
        let _ = vault.operation(platform, &config.vault_id(), None, true, &AtomicBool::new(false));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real libsecret/GObject calls in a child process: a use-after-dlclose must fail
    // this regression without taking down the entire test runner. No wallet access.
    #[test]
    fn library_outlives_wallet_handles() -> Result<()> {
        const CHILD: &str = "ROSORAH_VAULT_LIFETIME_TEST";
        if std::env::var(CHILD).as_deref() != Ok("1") {
            let result = std::process::Command::new(std::env::current_exe()?)
                .args([
                    "--exact",
                    "accounts::vault::tests::library_outlives_wallet_handles",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .output()?;
            ensure!(
                result.status.success(),
                "Wallet library lifetime regression: {}\n{}",
                result.status,
                String::from_utf8_lossy(&result.stderr)
            );
            return Ok(());
        }
        unsafe {
            let limit = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            libc::setrlimit(libc::RLIMIT_CORE, &limit);
            let gobject = Library::new("libgobject-2.0.so.0")?;
            let lookup = *gobject
                .get::<unsafe extern "C" fn(*const c_char) -> usize>(b"g_type_from_name\0")?;
            for _ in 0..100 {
                let vault = Vault::new()?;
                assert_ne!(lookup(c"SecretSchemaAttributeType".as_ptr()), 0);
                drop(vault);
                // GLib retains static type names and callbacks after a schema is freed.
                assert_ne!(lookup(c"SecretSchemaAttributeType".as_ptr()), 0);
            }
            std::thread::scope(|scope| {
                for _ in 0..8 {
                    scope.spawn(move || {
                        for _ in 0..100 {
                            drop(Vault::new().unwrap());
                            assert_ne!(lookup(c"SecretSchemaAttributeType".as_ptr()), 0);
                        }
                    });
                }
            });
        }
        Ok(())
    }
}

pub(super) fn donation_secret(
    value: Option<&str>,
    remove: bool,
    cancel: &AtomicBool,
) -> Result<Option<String>> {
    Vault::new()?.operation(Platform::Donations, "donations-v1", value, remove, cancel)
}

pub(super) fn kofi_secret(
    value: Option<&str>,
    remove: bool,
    cancel: &AtomicBool,
) -> Result<Option<String>> {
    Vault::new()?.operation(Platform::Donations, "kofi-v1", value, remove, cancel)
}

pub(super) fn stream_keys_secret(
    value: Option<&str>,
    remove: bool,
    cancel: &AtomicBool,
) -> Result<Option<String>> {
    Vault::new()?.operation(Platform::Donations, "stream-keys-v1", value, remove, cancel)
}
