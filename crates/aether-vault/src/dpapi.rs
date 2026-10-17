//! DPAPI-sealed secrets stored as files.
//!
//! `CryptProtectData` encrypts with a key derived from the Windows user's logon
//! credentials, so a sealed file is useless to other users, to other machines, and to
//! anyone who copies the file off disk. An application-specific entropy value is mixed in
//! as defence in depth.

use std::fs;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};
use windows::core::PCWSTR;
use zeroize::Zeroizing;

use crate::error::{Result, VaultError};
use crate::secret::{Secret, SecretKey};
use crate::store::SecretStore;

const ENTROPY: &[u8] = b"Aether/vault/v1";

fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 }
}

/// Encrypts `data` for the current Windows user.
pub fn dpapi_protect(data: &[u8]) -> Result<Vec<u8>> {
    let input = blob(data);
    let entropy = blob(ENTROPY);
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: all pointers reference live buffers; `output` is filled by the OS.
    unsafe {
        CryptProtectData(&input, PCWSTR::null(), Some(&entropy), None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut output)
    }
    .map_err(|e| VaultError::Os {
        operation: "CryptProtectData",
        code: e.code().0 as u32,
        message: e.message(),
    })?;
    // SAFETY: on success `pbData` points at `cbData` bytes allocated with LocalAlloc.
    let sealed = unsafe { os_bytes(&output) }.to_vec();
    unsafe { LocalFree(Some(HLOCAL(output.pbData as *mut _))) };
    Ok(sealed)
}

/// The bytes an OS call returned in `blob` — empty when there are none. (The pointer of a zero-length result may be
/// null, and a null pointer is not a valid slice even when empty.)
///
/// # Safety
/// `blob` must come from a successful call, so that a non-null `pbData` points at `cbData` readable bytes.
unsafe fn os_bytes(blob: &CRYPT_INTEGER_BLOB) -> &[u8] {
    if blob.pbData.is_null() || blob.cbData == 0 {
        &[]
    } else {
        // SAFETY: promised by the caller.
        unsafe { std::slice::from_raw_parts(blob.pbData, blob.cbData as usize) }
    }
}

/// Decrypts data produced by [`dpapi_protect`] for the same Windows user.
pub fn dpapi_unprotect(sealed: &[u8]) -> Result<Secret> {
    let input = blob(sealed);
    let entropy = blob(ENTROPY);
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: as above.
    unsafe { CryptUnprotectData(&input, None, Some(&entropy), None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut output) }
        .map_err(|_| VaultError::Undecryptable)?;
    // SAFETY: on success `pbData` points at `cbData` readable (and writable) bytes, or is null for an empty secret.
    let plain = Zeroizing::new(unsafe { os_bytes(&output) }.to_vec());
    if !output.pbData.is_null() {
        // SAFETY: as above. Scrub the OS-owned plaintext copy before releasing it.
        unsafe {
            std::ptr::write_bytes(output.pbData, 0, output.cbData as usize);
            LocalFree(Some(HLOCAL(output.pbData as *mut _)));
        }
    }
    Ok(Secret::new(plain.to_vec()))
}

/// Stores each secret as `<dir>/<namespace>/<id>.bin`, sealed with DPAPI.
pub struct DpapiFileStore {
    dir: PathBuf,
}

impl DpapiFileStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn path_for(&self, key: &SecretKey) -> PathBuf {
        self.dir.join(key.namespace.as_str()).join(format!("{}.bin", key.id))
    }
}

impl SecretStore for DpapiFileStore {
    fn put(&self, key: &SecretKey, secret: &Secret) -> Result<()> {
        use std::io::Write;
        let path = self.path_for(key);
        let parent = path.parent().unwrap_or(Path::new("."));
        fs::create_dir_all(parent).map_err(|e| VaultError::io(parent, e))?;
        let sealed = dpapi_protect(secret.expose())?;
        // Written next to the destination under a name nobody else is using (two saves of the same secret must not
        // trample one temporary file), flushed to disk, and only then moved into place: after a crash or a power cut
        // the secret is either the old one or the new one, never an empty file.
        let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4().simple()));
        let written = (|| {
            let mut file = fs::File::create(&tmp)?;
            file.write_all(&sealed)?;
            file.sync_all()
        })();
        if let Err(e) = written {
            let _ = fs::remove_file(&tmp);
            return Err(VaultError::io(&tmp, e));
        }
        fs::rename(&tmp, &path).map_err(|e| {
            let _ = fs::remove_file(&tmp);
            VaultError::io(&path, e)
        })
    }

    /// Whether a secret is stored, without decrypting it (or failing because it can't be).
    fn exists(&self, key: &SecretKey) -> Result<bool> {
        let path = self.path_for(key);
        match fs::metadata(&path) {
            Ok(meta) => Ok(meta.is_file()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(VaultError::io(path, e)),
        }
    }

    fn get(&self, key: &SecretKey) -> Result<Option<Secret>> {
        let path = self.path_for(key);
        match fs::read(&path) {
            Ok(sealed) => dpapi_unprotect(&sealed).map(Some),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(VaultError::io(path, e)),
        }
    }

    fn delete(&self, key: &SecretKey) -> Result<bool> {
        let path = self.path_for(key);
        match fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(VaultError::io(path, e)),
        }
    }

    fn backend(&self) -> String {
        "Windows DPAPI (encrypted files)".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::Namespace;

    #[test]
    fn protect_unprotect_roundtrip() {
        let sealed = dpapi_protect(b"top secret").unwrap();
        assert_ne!(sealed, b"top secret");
        assert_eq!(dpapi_unprotect(&sealed).unwrap().expose(), b"top secret");
    }

    #[test]
    fn tampered_data_fails_cleanly() {
        let mut sealed = dpapi_protect(b"x").unwrap();
        let last = sealed.len() - 1;
        sealed[last] ^= 0xFF;
        assert!(matches!(dpapi_unprotect(&sealed), Err(VaultError::Undecryptable)));
        assert!(matches!(dpapi_unprotect(b"not a blob"), Err(VaultError::Undecryptable)));
    }

    /// An empty secret has no bytes to point at: the OS may hand back a null pointer for it.
    #[test]
    fn an_empty_secret_round_trips() {
        let sealed = dpapi_protect(b"").unwrap();
        assert!(!sealed.is_empty(), "the sealed form has structure even when the secret is empty");
        let back = dpapi_unprotect(&sealed).unwrap();
        assert!(back.is_empty());

        let dir = tempfile::tempdir().unwrap();
        let store = DpapiFileStore::new(dir.path());
        let key = SecretKey::new(Namespace::HostPassword, "empty").unwrap();
        store.put(&key, &Secret::new(Vec::new())).unwrap();
        assert!(store.get(&key).unwrap().unwrap().is_empty());
    }

    #[test]
    fn saving_leaves_no_temporary_files_and_survives_being_done_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = std::sync::Arc::new(DpapiFileStore::new(dir.path()));
        let key = SecretKey::new(Namespace::KeyMaterial, "k").unwrap();
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let (store, key) = (store.clone(), key.clone());
                std::thread::spawn(move || {
                    for round in 0..10 {
                        store.put(&key, &Secret::from_text(&format!("secret-{i}-{round}"))).unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let names: Vec<String> = fs::read_dir(dir.path().join("key-material"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["k.bin"], "only the secret itself is left behind");
        let text = store.get(&key).unwrap().unwrap();
        assert!(text.expose_text().unwrap().starts_with("secret-"), "and it is one whole value, not a blend");
    }

    #[test]
    fn existence_is_checked_without_decrypting() {
        let dir = tempfile::tempdir().unwrap();
        let store = DpapiFileStore::new(dir.path());
        let key = SecretKey::new(Namespace::HostPassword, "h").unwrap();
        assert!(!store.exists(&key).unwrap());
        store.put(&key, &Secret::from_text("pw")).unwrap();
        assert!(store.exists(&key).unwrap());

        // A file that can no longer be decrypted (moved from another PC, say) still *exists*; reading it is what fails.
        fs::write(dir.path().join("host-password").join("h.bin"), b"not sealed by this user").unwrap();
        assert!(store.exists(&key).unwrap());
        assert!(matches!(store.get(&key), Err(VaultError::Undecryptable)));
        assert!(store.delete(&key).unwrap());
        assert!(!store.exists(&key).unwrap());
    }

    #[test]
    fn file_store_never_writes_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let store = DpapiFileStore::new(dir.path());
        let key = SecretKey::new(Namespace::KeyMaterial, "key-1").unwrap();
        let pem = "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n-----END OPENSSH PRIVATE KEY-----\n";
        // Bigger than Credential Manager would allow.
        let big = pem.repeat(100);

        assert_eq!(store.get(&key).unwrap(), None);
        store.put(&key, &Secret::from_text(&big)).unwrap();

        let on_disk = fs::read(dir.path().join("key-material").join("key-1.bin")).unwrap();
        assert!(!String::from_utf8_lossy(&on_disk).contains("BEGIN OPENSSH"));
        assert_eq!(store.get(&key).unwrap().unwrap().expose_text(), Some(big.as_str()));
        assert!(store.delete(&key).unwrap());
        assert_eq!(store.get(&key).unwrap(), None);
    }
}
