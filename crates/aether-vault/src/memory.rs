use std::collections::HashMap;

use parking_lot::Mutex;

use crate::error::Result;
use crate::secret::{Secret, SecretKey};
use crate::store::SecretStore;

/// Volatile store for tests and unsupported platforms.
#[derive(Default)]
pub struct MemoryStore {
    secrets: Mutex<HashMap<SecretKey, Secret>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for MemoryStore {
    fn put(&self, key: &SecretKey, secret: &Secret) -> Result<()> {
        self.secrets.lock().insert(key.clone(), secret.clone());
        Ok(())
    }

    fn get(&self, key: &SecretKey) -> Result<Option<Secret>> {
        Ok(self.secrets.lock().get(key).cloned())
    }

    fn delete(&self, key: &SecretKey) -> Result<bool> {
        Ok(self.secrets.lock().remove(key).is_some())
    }

    fn backend(&self) -> String {
        "In-memory (not persisted)".into()
    }
}
