//! Private key handling: loading (with passphrase prompts), inspection, and generation.

use std::sync::Arc;

use russh::keys::key::safe_rng;
use russh::keys::ssh_key::private::{KeypairData, RsaKeypair};
use russh::keys::ssh_key::{Algorithm, EcdsaCurve, LineEnding, PrivateKey};
use russh::keys::{PublicKeyBase64, decode_secret_key};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use zeroize::Zeroizing;

use aether_core::known_hosts::{fingerprint_sha256, key_bits};

use crate::delegate::{ConnectionDelegate, HopInfo, PromptKind, PromptRequest, SecretKind, SecretLookup};
use crate::error::{Result, SshError};
use crate::spec::KeyRef;

const MAX_PASSPHRASE_ATTEMPTS: u32 = 3;

// ── inspection ───────────────────────────────────────────────────────────────

/// Descriptive facts about a key; contains no private material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct KeyMeta {
    pub algorithm: String,
    pub bits: Option<u32>,
    pub fingerprint: String,
    /// `authorized_keys` line (`ssh-ed25519 AAAA… comment`).
    pub public_key: String,
    pub encrypted: bool,
    pub comment: String,
}

/// Heuristic: does this key text need a passphrase before it can be used?
pub fn looks_encrypted(text: &str) -> bool {
    let t = text.trim_start();
    if t.starts_with("PuTTY-User-Key-File-") {
        return t.lines().find_map(|l| l.strip_prefix("Encryption:")).is_some_and(|v| v.trim() != "none");
    }
    if t.contains("BEGIN ENCRYPTED PRIVATE KEY") || t.contains("Proc-Type: 4,ENCRYPTED") {
        return true;
    }
    if t.contains("BEGIN OPENSSH PRIVATE KEY") {
        return PrivateKey::from_openssh(t).is_ok_and(|k| k.is_encrypted());
    }
    false
}

fn meta_from_private(key: &PrivateKey, encrypted: bool) -> KeyMeta {
    let public = key.public_key();
    let algorithm = public.algorithm().to_string();
    let blob = public.public_key_bytes();
    KeyMeta {
        bits: key_bits(&algorithm, &blob),
        fingerprint: fingerprint_sha256(&blob),
        public_key: public.to_openssh().unwrap_or_default(),
        comment: key.comment().to_string(),
        algorithm,
        encrypted,
    }
}

/// Result of looking at key text without prompting.
pub enum Inspection {
    Meta(KeyMeta),
    /// Encrypted in a format whose public half can't be read without the passphrase.
    NeedsPassphrase,
}

/// Reads descriptive information from key text. OpenSSH-format keys reveal their public
/// half even when encrypted; other encrypted formats need the passphrase.
pub fn inspect_key(text: &str, passphrase: Option<&str>) -> Result<Inspection> {
    let encrypted = looks_encrypted(text);
    if text.contains("BEGIN OPENSSH PRIVATE KEY")
        && let Ok(key) = PrivateKey::from_openssh(text)
    {
        // The public half is readable without the passphrase, but one that was offered still has to be
        // right — otherwise a mistyped passphrase would be saved and only fail at the next connection.
        if let (true, Some(p)) = (key.is_encrypted(), passphrase) {
            key.decrypt(p).map_err(|_| SshError::WrongPassphrase { key: String::new() })?;
        }
        return Ok(Inspection::Meta(meta_from_private(&key, key.is_encrypted())));
    }
    if encrypted && passphrase.is_none() {
        return Ok(Inspection::NeedsPassphrase);
    }
    let key = decode_secret_key(text, passphrase).map_err(|e| {
        if encrypted {
            SshError::WrongPassphrase { key: String::new() }
        } else {
            SshError::KeyUnreadable { key: String::new(), reason: e.to_string() }
        }
    })?;
    Ok(Inspection::Meta(meta_from_private(&key, encrypted)))
}

// ── generation ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum KeyKind {
    Ed25519,
    EcdsaP256,
    EcdsaP384,
    EcdsaP521,
    Rsa2048,
    Rsa3072,
    Rsa4096,
}

pub struct GeneratedKey {
    /// The private key in OpenSSH format (encrypted when a passphrase was supplied).
    pub private_openssh: Zeroizing<String>,
    pub meta: KeyMeta,
}

pub fn generate_key(kind: KeyKind, comment: &str, passphrase: Option<&str>) -> Result<GeneratedKey> {
    let mut rng = safe_rng();
    let internal = |e: &dyn std::fmt::Display| SshError::Internal(format!("key generation failed: {e}"));
    let mut key =
        match kind {
            KeyKind::Ed25519 => PrivateKey::random(&mut rng, Algorithm::Ed25519).map_err(|e| internal(&e))?,
            KeyKind::EcdsaP256 => PrivateKey::random(&mut rng, Algorithm::Ecdsa { curve: EcdsaCurve::NistP256 })
                .map_err(|e| internal(&e))?,
            KeyKind::EcdsaP384 => PrivateKey::random(&mut rng, Algorithm::Ecdsa { curve: EcdsaCurve::NistP384 })
                .map_err(|e| internal(&e))?,
            KeyKind::EcdsaP521 => PrivateKey::random(&mut rng, Algorithm::Ecdsa { curve: EcdsaCurve::NistP521 })
                .map_err(|e| internal(&e))?,
            KeyKind::Rsa2048 | KeyKind::Rsa3072 | KeyKind::Rsa4096 => {
                let bits = match kind {
                    KeyKind::Rsa2048 => 2048,
                    KeyKind::Rsa3072 => 3072,
                    _ => 4096,
                };
                let pair = RsaKeypair::random(&mut rng, bits).map_err(|e| internal(&e))?;
                PrivateKey::new(KeypairData::from(pair), comment).map_err(|e| internal(&e))?
            }
        };
    key.set_comment(comment);

    let meta_plain = meta_from_private(&key, false);
    let key = match passphrase.filter(|p| !p.is_empty()) {
        Some(p) => key.encrypt(&mut rng, p).map_err(|e| internal(&e))?,
        None => key,
    };
    let private_openssh = key.to_openssh(LineEnding::LF).map_err(|e| internal(&e))?;
    Ok(GeneratedKey { private_openssh, meta: KeyMeta { encrypted: key.is_encrypted(), ..meta_plain } })
}

// ── loading for authentication ───────────────────────────────────────────────

pub(crate) struct LoadedKey {
    pub key: Arc<PrivateKey>,
    pub name: String,
}

/// Decodes key text, running the (slow) KDF off the async runtime.
async fn decode(
    text: Arc<Zeroizing<String>>,
    passphrase: Option<Zeroizing<String>>,
) -> std::result::Result<PrivateKey, russh::keys::Error> {
    tokio::task::spawn_blocking(move || decode_secret_key(&text, passphrase.as_ref().map(|p| p.as_str())))
        .await
        .unwrap_or(Err(russh::keys::Error::CouldNotReadKey))
}

/// The public key a key file stands for, found without its private half: the file itself when it is a public key
/// (`id.pub`), or the `.pub` file beside a private key file. This is how `IdentityFile key.pub` picks one key out of
/// an agent that holds many (1Password, a YubiKey, Pageant): the agent holds the private half and signs.
pub(crate) async fn public_half(path: &std::path::Path) -> Option<russh::keys::PublicKey> {
    async fn read(path: &std::path::Path) -> Option<russh::keys::PublicKey> {
        let text = tokio::fs::read_to_string(path).await.ok()?;
        russh::keys::PublicKey::from_openssh(text.lines().next()?.trim()).ok()
    }
    if let Some(key) = read(path).await {
        return Some(key);
    }
    let mut beside = path.as_os_str().to_owned();
    beside.push(".pub");
    read(std::path::Path::new(&beside)).await
}

/// Does this file stand for a key that lives elsewhere — in an agent? True for a public key file, and for a private key
/// file that isn't there while its `.pub` is. Such a file has nothing to load.
pub(crate) async fn names_an_agent_key(path: &std::path::Path) -> bool {
    match tokio::fs::read_to_string(path).await {
        Ok(text) => text.lines().next().is_some_and(|l| russh::keys::PublicKey::from_openssh(l.trim()).is_ok()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => public_half(path).await.is_some(),
        Err(_) => false,
    }
}

/// Loads a key, asking for its passphrase if needed. Returns `Ok(None)` when a *file* key
/// doesn't exist (so default identity files can be probed silently).
pub(crate) async fn load_key(
    key_ref: &KeyRef,
    delegate: &dyn ConnectionDelegate,
    hop: &HopInfo,
    scope: Option<&str>,
    interactive: bool,
) -> Result<Option<LoadedKey>> {
    let name = key_ref.display_name();
    let text: Zeroizing<String> = match key_ref {
        KeyRef::File(path) => match tokio::fs::read_to_string(path).await {
            Ok(t) => Zeroizing::new(t),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(SshError::KeyUnreadable { key: name, reason: e.to_string() });
            }
        },
        KeyRef::Vault { key_id, .. } => {
            let secret = delegate.vault_key(key_id).ok_or_else(|| SshError::KeyUnreadable {
                key: name.clone(),
                reason: "the key is missing from the secure vault".into(),
            })?;
            Zeroizing::new(
                secret
                    .expose_text()
                    .ok_or_else(|| SshError::KeyUnreadable {
                        key: name.clone(),
                        reason: "stored key is not valid text".into(),
                    })?
                    .to_string(),
            )
        }
    };
    let text = Arc::new(text);

    if !looks_encrypted(&text) {
        let key = decode(text, None)
            .await
            .map_err(|e| SshError::KeyUnreadable { key: name.clone(), reason: e.to_string() })?;
        return Ok(Some(LoadedKey { key: Arc::new(key), name }));
    }

    let lookup = SecretLookup {
        scope: scope.map(str::to_string),
        kind: SecretKind::KeyPassphrase { key_id: key_ref.passphrase_id() },
    };

    // 1. A saved passphrase.
    if let Some(saved) = delegate.saved_secret(&lookup)
        && let Some(p) = saved.expose_text()
    {
        match decode(text.clone(), Some(Zeroizing::new(p.to_string()))).await {
            Ok(key) => return Ok(Some(LoadedKey { key: Arc::new(key), name })),
            Err(_) => delegate.forget_secret(&lookup),
        }
    }

    if !interactive {
        return Err(SshError::KeyUnreadable { key: name, reason: "the key is protected by a passphrase".into() });
    }

    // 2. Ask the user.
    let mut error: Option<String> = None;
    for attempt in 0..MAX_PASSPHRASE_ATTEMPTS {
        let response = delegate
            .prompt(PromptRequest {
                hop: hop.clone(),
                kind: PromptKind::KeyPassphrase { key: name.clone() },
                attempt,
                error: error.take(),
                // Without a saved host there is nowhere to keep it.
                can_remember: scope.is_some(),
            })
            .await
            .ok_or(SshError::AuthCancelled)?;
        let Some(answer) = response.answers.first() else {
            return Err(SshError::AuthCancelled);
        };
        let passphrase = Zeroizing::new(answer.expose_text().unwrap_or_default().to_string());
        match decode(text.clone(), Some(passphrase)).await {
            Ok(key) => {
                if response.remember {
                    delegate.remember_secret(&lookup, answer);
                }
                return Ok(Some(LoadedKey { key: Arc::new(key), name }));
            }
            Err(_) => error = Some("That passphrase didn't unlock the key. Try again.".into()),
        }
    }
    Err(SshError::WrongPassphrase { key: name })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_ed25519_is_usable_and_described() {
        let g = generate_key(KeyKind::Ed25519, "test@aether", None).unwrap();
        assert!(g.private_openssh.contains("BEGIN OPENSSH PRIVATE KEY"));
        assert_eq!(g.meta.algorithm, "ssh-ed25519");
        assert_eq!(g.meta.bits, Some(256));
        assert!(g.meta.public_key.starts_with("ssh-ed25519 "));
        assert!(g.meta.public_key.ends_with("test@aether"));
        assert!(g.meta.fingerprint.starts_with("SHA256:"));
        assert!(!g.meta.encrypted);

        // Round trip through the decoder russh uses for authentication.
        let back = decode_secret_key(&g.private_openssh, None).unwrap();
        assert_eq!(back.public_key().to_openssh().unwrap(), g.meta.public_key);
    }

    #[test]
    fn passphrase_protected_keys_are_detected_and_unlock() {
        let g = generate_key(KeyKind::Ed25519, "locked", Some("correct horse")).unwrap();
        assert!(g.meta.encrypted);
        assert!(looks_encrypted(&g.private_openssh));
        assert!(decode_secret_key(&g.private_openssh, None).is_err());
        assert!(decode_secret_key(&g.private_openssh, Some("wrong")).is_err());
        let ok = decode_secret_key(&g.private_openssh, Some("correct horse")).unwrap();
        assert_eq!(ok.public_key().to_openssh().unwrap(), g.meta.public_key);

        // The public half of an encrypted OpenSSH key is readable without the passphrase.
        match inspect_key(&g.private_openssh, None).unwrap() {
            Inspection::Meta(m) => {
                assert!(m.encrypted);
                assert_eq!(m.fingerprint, g.meta.fingerprint);
            }
            Inspection::NeedsPassphrase => panic!("OpenSSH keys expose their public half"),
        }

        // A passphrase that is offered gets checked rather than trusted.
        assert!(matches!(inspect_key(&g.private_openssh, Some("wrong")), Err(SshError::WrongPassphrase { .. })));
        assert!(matches!(inspect_key(&g.private_openssh, Some("correct horse")), Ok(Inspection::Meta(_))));
    }

    #[test]
    fn ecdsa_and_rsa_generation() {
        let e = generate_key(KeyKind::EcdsaP256, "e", None).unwrap();
        assert_eq!(e.meta.algorithm, "ecdsa-sha2-nistp256");
        assert_eq!(e.meta.bits, Some(256));
        let r = generate_key(KeyKind::Rsa2048, "r", None).unwrap();
        assert_eq!(r.meta.algorithm, "ssh-rsa");
        assert_eq!(r.meta.bits, Some(2048));
    }

    #[test]
    fn ppk_encryption_is_detected_from_the_header() {
        assert!(looks_encrypted("PuTTY-User-Key-File-3: ssh-ed25519\nEncryption: aes256-cbc\n"));
        assert!(!looks_encrypted("PuTTY-User-Key-File-3: ssh-ed25519\nEncryption: none\n"));
        assert!(!looks_encrypted("garbage"));
    }

    #[test]
    fn garbage_is_reported_as_unreadable() {
        assert!(matches!(inspect_key("not a key", None), Err(SshError::KeyUnreadable { .. })));
    }
}
