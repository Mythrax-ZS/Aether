//! Metadata registry for keys managed by Aether. Private key material is *not* stored
//! here — see `aether-vault`.

use std::path::PathBuf;

use parking_lot::RwLock;

use super::json_file::JsonFile;
use crate::error::{CoreError, Result};
use crate::ids::KeyId;
use crate::model::{KeyInfo, KeyRegistry};

pub struct KeyStore {
    file: JsonFile,
    data: RwLock<KeyRegistry>,
}

impl KeyStore {
    pub fn open(path: impl Into<PathBuf>) -> Self {
        Self::open_reporting(path).0
    }

    /// Like [`KeyStore::open`], also saying whether the file had to be recovered (or could not be read).
    pub fn open_reporting(path: impl Into<PathBuf>) -> (Self, Option<CoreError>) {
        let file = JsonFile::new(path);
        let loaded = file.load_or_default::<KeyRegistry>();
        (Self { file, data: RwLock::new(loaded.value) }, loaded.recovery)
    }

    pub fn list(&self) -> Vec<KeyInfo> {
        let mut keys = self.data.read().keys.clone();
        keys.sort_by_key(|k| k.label.to_lowercase());
        keys
    }

    pub fn get(&self, id: KeyId) -> Option<KeyInfo> {
        self.data.read().keys.iter().find(|k| k.id == id).cloned()
    }

    /// Registers a key. Importing the same key twice is refused with a helpful message.
    pub fn add(&self, mut info: KeyInfo) -> Result<KeyInfo> {
        info.label = info.label.trim().to_string();
        if info.label.is_empty() {
            return Err(CoreError::invalid("key name", "give the key a name"));
        }
        self.transact(|data| {
            if let Some(existing) = data.keys.iter().find(|k| k.fingerprint == info.fingerprint) {
                return Err(CoreError::Conflict(format!("This key is already saved as “{}”.", existing.label)));
            }
            data.version = 1;
            data.keys.push(info.clone());
            Ok(info)
        })
    }

    pub fn rename(&self, id: KeyId, label: &str) -> Result<KeyInfo> {
        let label = label.trim();
        if label.is_empty() {
            return Err(CoreError::invalid("key name", "give the key a name"));
        }
        self.transact(|data| {
            let key = data.keys.iter_mut().find(|k| k.id == id).ok_or_else(|| CoreError::not_found("key"))?;
            key.label = label.to_string();
            Ok(key.clone())
        })
    }

    pub fn remove(&self, id: KeyId) -> Result<KeyInfo> {
        self.transact(|data| {
            let pos = data.keys.iter().position(|k| k.id == id).ok_or_else(|| CoreError::not_found("key"))?;
            Ok(data.keys.remove(pos))
        })
    }

    /// Applies `change` to a copy and keeps it only once it has reached the disk, so that a failed save doesn't leave
    /// the list on screen ahead of the file.
    fn transact<T>(&self, change: impl FnOnce(&mut KeyRegistry) -> Result<T>) -> Result<T> {
        let mut data = self.data.write();
        let mut next = data.clone();
        let out = change(&mut next)?;
        self.file.save(&next)?;
        *data = next;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::KeyOrigin;

    fn info(label: &str, fp: &str) -> KeyInfo {
        KeyInfo {
            id: KeyId::new(),
            label: label.into(),
            algorithm: "ssh-ed25519".into(),
            bits: Some(256),
            fingerprint: fp.into(),
            public_key: "ssh-ed25519 AAAA test".into(),
            encrypted: false,
            origin: KeyOrigin::Generated,
            created_at: 1,
        }
    }

    #[test]
    fn add_rename_remove_persist() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys.json");
        let store = KeyStore::open(&path);
        let a = store.add(info("Work laptop", "SHA256:aaa")).unwrap();
        store.add(info("Backup", "SHA256:bbb")).unwrap();
        assert_eq!(store.list().iter().map(|k| k.label.as_str()).collect::<Vec<_>>(), ["Backup", "Work laptop"]);

        store.rename(a.id, "Laptop").unwrap();
        assert_eq!(KeyStore::open(&path).get(a.id).unwrap().label, "Laptop");

        store.remove(a.id).unwrap();
        assert!(store.get(a.id).is_none());
        assert!(store.remove(a.id).is_err());
    }

    #[test]
    fn duplicate_fingerprints_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::open(dir.path().join("keys.json"));
        store.add(info("First", "SHA256:same")).unwrap();
        let err = store.add(info("Second", "SHA256:same")).err().unwrap();
        assert!(err.to_string().contains("First"));
    }
}
