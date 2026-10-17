//! Metadata about private keys managed by Aether. The private key material itself is
//! never stored here — it lives in the vault (DPAPI-sealed) — only descriptive fields.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::KeyId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum KeyOrigin {
    Imported,
    Generated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct KeyInfo {
    pub id: KeyId,
    pub label: String,
    /// `ssh-ed25519`, `ssh-rsa`, `ecdsa-sha2-nistp256`, ...
    pub algorithm: String,
    /// Key size in bits where meaningful (RSA modulus, ECDSA curve, 256 for Ed25519).
    pub bits: Option<u32>,
    /// `SHA256:...` fingerprint of the public key.
    pub fingerprint: String,
    /// The public key in `authorized_keys` format, ready to copy to a server.
    pub public_key: String,
    /// Whether the private key itself is passphrase-protected.
    pub encrypted: bool,
    pub origin: KeyOrigin,
    pub created_at: i64,
}

/// Persistent list of managed keys (`keys.json`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyRegistry {
    pub version: u32,
    pub keys: Vec<KeyInfo>,
}
