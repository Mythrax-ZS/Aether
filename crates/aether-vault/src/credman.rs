//! Windows Credential Manager backend (generic credentials).

use windows::Win32::Security::Credentials::{
    CRED_FLAGS, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree, CredReadW,
    CredWriteW,
};
use windows::core::{PCWSTR, PWSTR};

use crate::error::{Result, VaultError};
use crate::secret::{Secret, SecretKey};
use crate::store::SecretStore;

/// `CRED_MAX_CREDENTIAL_BLOB_SIZE`: Credential Manager rejects larger blobs.
const MAX_BLOB: usize = 5 * 512;
/// `ERROR_NOT_FOUND` as an HRESULT.
const HRESULT_NOT_FOUND: i32 = 0x8007_0490_u32 as i32;

pub struct CredentialManagerStore;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn os_error(operation: &'static str, e: windows::core::Error) -> VaultError {
    VaultError::Os { operation, code: e.code().0 as u32, message: e.message() }
}

impl SecretStore for CredentialManagerStore {
    fn put(&self, key: &SecretKey, secret: &Secret) -> Result<()> {
        let bytes = secret.expose();
        if bytes.len() > MAX_BLOB {
            return Err(VaultError::TooLarge { size: bytes.len(), limit: MAX_BLOB });
        }
        let mut target = wide(&key.target_name());
        let mut user = wide("Aether");
        let credential = CREDENTIALW {
            Flags: CRED_FLAGS(0),
            Type: CRED_TYPE_GENERIC,
            TargetName: PWSTR(target.as_mut_ptr()),
            CredentialBlobSize: bytes.len() as u32,
            CredentialBlob: bytes.as_ptr() as *mut u8,
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            UserName: PWSTR(user.as_mut_ptr()),
            ..Default::default()
        };
        // SAFETY: `credential` points at buffers that outlive the call; CredWriteW copies them.
        unsafe { CredWriteW(&credential, 0) }.map_err(|e| os_error("CredWriteW", e))
    }

    fn get(&self, key: &SecretKey) -> Result<Option<Secret>> {
        let target = wide(&key.target_name());
        let mut out: *mut CREDENTIALW = std::ptr::null_mut();
        // SAFETY: `target` is NUL-terminated and `out` receives an OS-allocated struct that
        // we free with CredFree below.
        let result = unsafe { CredReadW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None, &mut out) };
        match result {
            Ok(()) => {
                // SAFETY: on success `out` points to a valid CREDENTIALW.
                let cred = unsafe { &*out };
                let blob = if cred.CredentialBlob.is_null() || cred.CredentialBlobSize == 0 {
                    Vec::new()
                } else {
                    // SAFETY: the OS guarantees `CredentialBlobSize` readable bytes.
                    let copy =
                        unsafe { std::slice::from_raw_parts(cred.CredentialBlob, cred.CredentialBlobSize as usize) }
                            .to_vec();
                    // SAFETY: the same bytes are writable; scrub the OS-owned copy before it is released.
                    unsafe { std::ptr::write_bytes(cred.CredentialBlob, 0, cred.CredentialBlobSize as usize) };
                    copy
                };
                // SAFETY: `out` was allocated by CredReadW.
                unsafe { CredFree(out as *const _) };
                Ok(Some(Secret::new(blob)))
            }
            Err(e) if e.code().0 == HRESULT_NOT_FOUND => Ok(None),
            Err(e) => Err(os_error("CredReadW", e)),
        }
    }

    fn delete(&self, key: &SecretKey) -> Result<bool> {
        let target = wide(&key.target_name());
        // SAFETY: `target` is NUL-terminated.
        match unsafe { CredDeleteW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None) } {
            Ok(()) => Ok(true),
            Err(e) if e.code().0 == HRESULT_NOT_FOUND => Ok(false),
            Err(e) => Err(os_error("CredDeleteW", e)),
        }
    }

    fn backend(&self) -> String {
        "Windows Credential Manager".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::Namespace;

    fn unique_key() -> SecretKey {
        let id = format!("test-{}-{}", std::process::id(), std::time::UNIX_EPOCH.elapsed().unwrap().as_nanos());
        SecretKey::new(Namespace::HostPassword, id).unwrap()
    }

    #[test]
    fn roundtrip_through_the_real_credential_manager() {
        let store = CredentialManagerStore;
        let key = unique_key();
        assert_eq!(store.get(&key).unwrap(), None);
        assert!(!store.delete(&key).unwrap());

        store.put(&key, &Secret::from_text("p@ss wörd ✓")).unwrap();
        assert_eq!(store.get(&key).unwrap().unwrap().expose_text(), Some("p@ss wörd ✓"));
        assert!(store.exists(&key).unwrap());

        // Overwrite.
        store.put(&key, &Secret::from_text("second")).unwrap();
        assert_eq!(store.get(&key).unwrap().unwrap().expose_text(), Some("second"));

        assert!(store.delete(&key).unwrap());
        assert_eq!(store.get(&key).unwrap(), None);
    }

    #[test]
    fn oversized_secrets_are_rejected_cleanly() {
        let key = unique_key();
        let big = Secret::new(vec![7u8; MAX_BLOB + 1]);
        assert!(matches!(CredentialManagerStore.put(&key, &big), Err(VaultError::TooLarge { .. })));
    }
}
