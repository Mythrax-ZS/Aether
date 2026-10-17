//! The seam between the engine and whoever is driving it (the Tauri app, or tests).
//!
//! While connecting, the engine needs things only a human or a credential store can supply:
//! a decision about an unknown host key, a password, a key passphrase, answers to a
//! keyboard-interactive challenge. It asks through [`ConnectionDelegate`].

use aether_core::known_hosts::KnownKey;
use aether_vault::Secret;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Which machine in a (possibly multi-hop) connection a prompt or event is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HopInfo {
    /// Zero-based position in the chain; the final target has `index == total - 1`.
    pub index: u32,
    pub total: u32,
    pub label: String,
    pub host: String,
    pub port: u16,
    pub username: String,
}

impl HopInfo {
    pub fn is_jump(&self) -> bool {
        self.index + 1 < self.total
    }
}

// ── host keys ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "type", rename_all = "kebab-case", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum HostKeyRequestKind {
    /// First time Aether sees this host.
    Unknown,
    /// A key is on record for this host but this isn't it.
    Changed { known: Vec<KnownKey> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HostKeyRequest {
    pub hop: HopInfo,
    /// `ssh-ed25519`, `ecdsa-sha2-nistp256`, ...
    pub algorithm: String,
    /// `SHA256:...`
    pub fingerprint: String,
    pub bits: Option<u32>,
    /// OpenSSH-style ASCII art of the fingerprint.
    pub randomart: String,
    pub kind: HostKeyRequestKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum HostKeyDecision {
    /// Connect and remember the key.
    TrustAndSave,
    /// Connect this once without remembering.
    TrustOnce,
    /// Connect and overwrite the recorded key (only honoured for changed keys).
    ReplaceAndTrust,
    Reject,
}

// ── prompts ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InteractivePrompt {
    pub text: String,
    /// Whether the typed answer may be shown on screen (false for passwords/codes).
    pub echo: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "type", rename_all = "kebab-case", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum PromptKind {
    Password,
    KeyPassphrase { key: String },
    KeyboardInteractive { name: String, instructions: String, prompts: Vec<InteractivePrompt> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PromptRequest {
    pub hop: HopInfo,
    pub kind: PromptKind,
    /// 0 on the first ask; increments after each rejected answer.
    pub attempt: u32,
    /// Explains why we're asking again ("Permission denied, please try again.").
    pub error: Option<String>,
    /// Whether a "remember this" option makes sense.
    pub can_remember: bool,
}

/// The user's answers, one per prompt (a single entry for passwords/passphrases).
pub struct PromptResponse {
    pub answers: Vec<Secret>,
    pub remember: bool,
}

/// Identifies a secret the delegate may have saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretLookup {
    /// Opaque scope from [`ConnectSpec::secret_scope`](crate::ConnectSpec) (the host id).
    pub scope: Option<String>,
    pub kind: SecretKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretKind {
    Password,
    /// Passphrase of a private key; the id comes from `KeyRef::passphrase_id`.
    KeyPassphrase {
        key_id: String,
    },
}

// ── progress ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "type", rename_all = "kebab-case", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum ConnectStage {
    Resolving,
    Connecting,
    /// TCP is up; negotiating encryption and checking the host key.
    Handshake,
    Authenticating {
        method: String,
    },
    /// Authenticated; opening the session.
    Establishing,
    Ready,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConnectProgress {
    pub hop: HopInfo,
    pub stage: ConnectStage,
}

#[async_trait]
pub trait ConnectionDelegate: Send + Sync + 'static {
    /// Decide whether to trust a host key the engine can't verify on its own. Only called
    /// for unknown hosts under the "ask" policy, and for changed keys.
    async fn verify_host_key(&self, request: HostKeyRequest) -> HostKeyDecision;

    /// Ask the user for a secret. `None` means they cancelled.
    async fn prompt(&self, request: PromptRequest) -> Option<PromptResponse>;

    /// A previously saved secret, if any.
    fn saved_secret(&self, lookup: &SecretLookup) -> Option<Secret>;

    /// Persist a secret the user asked to remember, after it has been proven correct.
    fn remember_secret(&self, lookup: &SecretLookup, secret: &Secret);

    /// Drop a saved secret that turned out to be wrong.
    fn forget_secret(&self, lookup: &SecretLookup);

    /// Called as the connection advances. Purely informational.
    fn progress(&self, _progress: ConnectProgress) {}

    /// The server sent a pre-authentication banner.
    fn banner(&self, _hop: &HopInfo, _text: &str) {}

    /// Loads private key text for a vault-stored key (the engine doesn't depend on storage).
    fn vault_key(&self, _key_id: &str) -> Option<Secret> {
        None
    }
}
