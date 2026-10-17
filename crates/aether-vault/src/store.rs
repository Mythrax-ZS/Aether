use std::path::Path;
use std::sync::Arc;

use crate::error::Result;
use crate::secret::{Secret, SecretKey};

/// A place to keep secrets. Implementations are synchronous: the OS calls involved take
/// microseconds, and callers can wrap them in `spawn_blocking` if they care.
pub trait SecretStore: Send + Sync + 'static {
    fn put(&self, key: &SecretKey, secret: &Secret) -> Result<()>;
    fn get(&self, key: &SecretKey) -> Result<Option<Secret>>;
    /// Returns whether a secret existed.
    fn delete(&self, key: &SecretKey) -> Result<bool>;

    fn exists(&self, key: &SecretKey) -> Result<bool> {
        Ok(self.get(key)?.is_some())
    }

    /// Human-readable backend name for the Settings → Security page.
    fn backend(&self) -> String;
}

/// Routes each secret to the backend best suited for it: small secrets to `small`,
/// bulky ones (private keys) to `bulky`.
pub struct CompositeStore {
    small: Arc<dyn SecretStore>,
    bulky: Arc<dyn SecretStore>,
}

impl CompositeStore {
    pub fn new(small: Arc<dyn SecretStore>, bulky: Arc<dyn SecretStore>) -> Self {
        Self { small, bulky }
    }

    fn route(&self, key: &SecretKey) -> &dyn SecretStore {
        if key.namespace.is_bulky() { self.bulky.as_ref() } else { self.small.as_ref() }
    }
}

impl SecretStore for CompositeStore {
    fn put(&self, key: &SecretKey, secret: &Secret) -> Result<()> {
        self.route(key).put(key, secret)
    }
    fn get(&self, key: &SecretKey) -> Result<Option<Secret>> {
        self.route(key).get(key)
    }
    fn delete(&self, key: &SecretKey) -> Result<bool> {
        self.route(key).delete(key)
    }
    fn exists(&self, key: &SecretKey) -> Result<bool> {
        self.route(key).exists(key)
    }
    fn backend(&self) -> String {
        let (a, b) = (self.small.backend(), self.bulky.backend());
        if a == b { a } else { format!("{a} + {b}") }
    }
}

/// Opens the best available store for this machine.
///
/// * Installed mode on Windows: Credential Manager for passwords/passphrases, DPAPI files
///   under `secrets_dir` for private keys.
/// * Portable mode on Windows: DPAPI files for everything, so no state is left in the
///   machine's credential store.
/// * Elsewhere (development only): an in-memory store that forgets everything on exit.
#[allow(unused_variables)]
pub fn open_default(secrets_dir: &Path, portable: bool) -> Arc<dyn SecretStore> {
    #[cfg(windows)]
    {
        let files: Arc<dyn SecretStore> = Arc::new(crate::dpapi::DpapiFileStore::new(secrets_dir));
        if portable {
            files
        } else {
            Arc::new(CompositeStore::new(Arc::new(crate::credman::CredentialManagerStore), files))
        }
    }
    #[cfg(not(windows))]
    {
        tracing::warn!("no OS secret store on this platform; secrets will not persist");
        Arc::new(crate::memory::MemoryStore::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryStore;
    use crate::secret::Namespace;

    #[test]
    fn composite_routes_by_namespace() {
        let small = Arc::new(MemoryStore::new());
        let bulky = Arc::new(MemoryStore::new());
        let c = CompositeStore::new(small.clone(), bulky.clone());

        let pw = SecretKey::new(Namespace::HostPassword, "h1").unwrap();
        let key = SecretKey::new(Namespace::KeyMaterial, "k1").unwrap();
        c.put(&pw, &Secret::from_text("pw")).unwrap();
        c.put(&key, &Secret::from_text("-----BEGIN-----")).unwrap();

        assert!(small.exists(&pw).unwrap() && !bulky.exists(&pw).unwrap());
        assert!(bulky.exists(&key).unwrap() && !small.exists(&key).unwrap());
        assert_eq!(c.get(&pw).unwrap().unwrap().expose_text(), Some("pw"));
        assert!(c.delete(&key).unwrap());
        assert!(!c.delete(&key).unwrap());
    }
}
