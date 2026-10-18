//! User-scoped Windows Credential Manager. Tokens never enter projects or plain-text files.
use anyhow::{Context, Result, ensure};
use windows::{
    Win32::{Foundation::ERROR_NOT_FOUND, Security::Credentials::*},
    core::{PCWSTR, PWSTR},
};
fn target(key: &str) -> Result<Vec<u16>> {
    ensure!(
        !key.is_empty() && !key.contains('\0') && key.len() <= 1024,
        "Invalid credential key"
    );
    Ok(format!("RosorahStudio/{key}")
        .encode_utf16()
        .chain(Some(0))
        .collect())
}
pub fn read(key: &str) -> Result<Option<Vec<u8>>> {
    let name = target(key)?;
    let mut raw = std::ptr::null_mut();
    unsafe {
        match CredReadW(PCWSTR(name.as_ptr()), CRED_TYPE_GENERIC, 0, &mut raw) {
            Ok(()) => {}
            Err(e) if e.code() == ERROR_NOT_FOUND.to_hresult() => return Ok(None),
            Err(e) => {
                return Err(e).context("Windows Credential Manager could not restore this sign-in");
            }
        }
        struct Free(*mut CREDENTIALW);
        impl Drop for Free {
            fn drop(&mut self) {
                unsafe {
                    CredFree(self.0.cast());
                }
            }
        }
        let _free = Free(raw);
        ensure!(
            !raw.is_null(),
            "Credential Manager returned an empty credential"
        );
        let c = &*raw;
        ensure!(
            c.CredentialBlobSize <= CRED_MAX_CREDENTIAL_BLOB_SIZE,
            "Saved credential is too large"
        );
        if c.CredentialBlobSize == 0 {
            return Ok(Some(Vec::new()));
        }
        ensure!(!c.CredentialBlob.is_null(), "Saved credential is invalid");
        Ok(Some(
            std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize).to_vec(),
        ))
    }
}
pub fn write(key: &str, value: &[u8]) -> Result<()> {
    ensure!(
        value.len() <= CRED_MAX_CREDENTIAL_BLOB_SIZE as usize,
        "Sign-in exceeds Windows Credential Manager's storage limit"
    );
    let mut name = target(key)?;
    let c = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(name.as_mut_ptr()),
        CredentialBlobSize: value.len() as u32,
        CredentialBlob: value.as_ptr() as *mut u8,
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        ..Default::default()
    };
    // LOCAL_MACHINE means persist across this user's logons; it does not grant other users access.
    unsafe { CredWriteW(&c, 0) }.context("Windows Credential Manager could not save this sign-in")
}
pub fn delete(key: &str) -> Result<()> {
    let name = target(key)?;
    match unsafe { CredDeleteW(PCWSTR(name.as_ptr()), CRED_TYPE_GENERIC, 0) } {
        Ok(()) => Ok(()),
        Err(e) if e.code() == ERROR_NOT_FOUND.to_hresult() => Ok(()),
        Err(e) => Err(e).context("Windows Credential Manager could not remove this sign-in"),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_keys_rejected_before_windows_calls() {
        assert!(target("").is_err());
        assert!(target("a\0b").is_err());
        assert!(target(&"x".repeat(1025)).is_err());
    }
    #[test]
    #[ignore = "Writes a unique temporary credential in the current Windows account"]
    fn round_trip() -> Result<()> {
        let key = format!(
            "test/{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        );
        struct Cleanup(String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = delete(&self.0);
            }
        }
        let _cleanup = Cleanup(key.clone());
        assert!(read(&key)?.is_none());
        write(&key, b"test-only")?;
        assert_eq!(read(&key)?.as_deref(), Some(&b"test-only"[..]));
        write(&key, b"replacement")?;
        assert_eq!(read(&key)?.as_deref(), Some(&b"replacement"[..]));
        delete(&key)?;
        assert!(read(&key)?.is_none());
        Ok(())
    }
}
