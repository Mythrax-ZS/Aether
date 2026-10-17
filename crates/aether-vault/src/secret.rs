use std::fmt;

use zeroize::Zeroizing;

use crate::error::{Result, VaultError};

/// A category of secret. Namespaces keep ids from colliding and let the composite store
/// route each kind to the most suitable backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Namespace {
    /// A host's saved login password. Id: the host id.
    HostPassword,
    /// The passphrase of a private key. Id: a key id, or a hash of the key file path.
    KeyPassphrase,
    /// Private key material imported into the vault. Id: the key id.
    KeyMaterial,
}

impl Namespace {
    pub fn as_str(self) -> &'static str {
        match self {
            Namespace::HostPassword => "host-password",
            Namespace::KeyPassphrase => "key-passphrase",
            Namespace::KeyMaterial => "key-material",
        }
    }

    /// Whether secrets of this kind may exceed Credential Manager's blob limit.
    pub fn is_bulky(self) -> bool {
        matches!(self, Namespace::KeyMaterial)
    }
}

/// Address of a secret: a namespace plus an identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SecretKey {
    pub namespace: Namespace,
    pub id: String,
}

impl SecretKey {
    /// Ids become part of Windows credential names and file names, so they are restricted
    /// to a conservative alphabet — this also rules out path traversal.
    pub fn new(namespace: Namespace, id: impl Into<String>) -> Result<Self> {
        let id = id.into();
        let valid =
            !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !valid {
            return Err(VaultError::InvalidId(id));
        }
        Ok(Self { namespace, id })
    }

    /// The credential name used in Windows Credential Manager.
    pub fn target_name(&self) -> String {
        format!("Aether/{}/{}", self.namespace.as_str(), self.id)
    }
}

/// Secret bytes. Zeroed on drop; `Debug` never reveals the contents.
#[derive(Clone)]
pub struct Secret(Zeroizing<Vec<u8>>);

impl Secret {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(Zeroizing::new(bytes))
    }

    pub fn from_text(text: &str) -> Self {
        Self::new(text.as_bytes().to_vec())
    }

    pub fn expose(&self) -> &[u8] {
        &self.0
    }

    /// The secret as UTF-8 text, if it is valid UTF-8.
    pub fn expose_text(&self) -> Option<&str> {
        std::str::from_utf8(&self.0).ok()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Secret(<{} bytes>)", self.0.len())
    }
}

impl PartialEq for Secret {
    fn eq(&self, other: &Self) -> bool {
        // Constant-time-ish comparison; these are never compared on a hot path.
        self.0.len() == other.0.len() && self.0.iter().zip(other.0.iter()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_leaks() {
        let s = Secret::from_text("hunter2");
        assert!(!format!("{s:?}").contains("hunter2"));
    }

    #[test]
    fn ids_are_validated() {
        assert!(SecretKey::new(Namespace::HostPassword, "7d444840-9dc0-11d1-b245-5ffdce74fad2").is_ok());
        for bad in ["", "../x", "a/b", "a b", "a\\b", "x\0"] {
            assert!(SecretKey::new(Namespace::HostPassword, bad).is_err(), "{bad:?}");
        }
        assert!(SecretKey::new(Namespace::HostPassword, "a".repeat(200)).is_err());
    }

    #[test]
    fn target_names_are_namespaced() {
        let k = SecretKey::new(Namespace::KeyPassphrase, "abc").unwrap();
        assert_eq!(k.target_name(), "Aether/key-passphrase/abc");
    }

    #[test]
    fn equality() {
        assert_eq!(Secret::from_text("a"), Secret::from_text("a"));
        assert_ne!(Secret::from_text("a"), Secret::from_text("b"));
        assert_ne!(Secret::from_text("a"), Secret::from_text("ab"));
    }
}
