//! SSH ky management: import, generate, export public keys, install on a server, and the
//! known-hosts manager.

use std::time::Duration;

use aether_core::error::{ErrorCode, UserFacing};
use aether_core::known_hosts::KnownHostEntry;
use aether_core::model::{AuthConfig, KeyInfo, KeyOrigin, KeySource};
use aether_core::util::now_ms;
use aether_core::{ConnectionId, ErrorInfo, KeyId};
use aether_ssh::{Inspection, SshError};
use aether_vault::{Namespace, Secret, SecretKey};

use super::app::map_core;
use super::{St, blocking};
use crate::dto::{KeyGenerateRequest, KeyImportRequest, KeyImportSource, KeyInspectResult};
use crate::state::{Res, not_found};

fn vault_key(ns: Namespace, id: KeyId) -> Res<SecretKey> {
    SecretKey::new(ns, id.to_string()).map_err(|e| ErrorInfo::internal(e.to_string()))
}

fn vault_err(e: aether_vault::VaultError) -> ErrorInfo {
    ErrorInfo::new(
        ErrorCode::VaultUnavailable,
        "Secure storage problem",
        "Aether couldn't write to the Windows credential store.",
    )
    .detail(e.to_string())
}

#[tauri::command]
pub fn keys_list(state: St<'_>) -> Vec<KeyInfo> {
    state.keys.list()
}

fn read_key_source(source: &KeyImportSource) -> Res<String> {
    match source {
        KeyImportSource::Text { pem } => Ok(pem.clone()),
        KeyImportSource::File { path } => {
            let p = aether_core::util::expand_tilde(path);
            std::fs::read_to_string(&p).map_err(|e| {
                ErrorInfo::new(
                    ErrorCode::KeyUnreadable,
                    "Can't read that file",
                    format!("{} couldn't be opened as a key file.", p.display()),
                )
                .detail(e.to_string())
            })
        }
    }
}

/// Looks at a key without importing it, so the dialog can show its fingerprint (and ask
/// for a passphrase when the format needs one). Decrypting is slow on purpose (the passphrase is stretched), so it
/// runs off the UI thread.
#[tauri::command]
pub async fn key_inspect(source: KeyImportSource, passphrase: Option<String>) -> Res<KeyInspectResult> {
    blocking(move || {
        let text = read_key_source(&source)?;
        match aether_ssh::inspect_key(&text, passphrase.as_deref().filter(|p| !p.is_empty())) {
            Ok(Inspection::Meta(meta)) => Ok(KeyInspectResult::Ready { meta }),
            Ok(Inspection::NeedsPassphrase) => Ok(KeyInspectResult::NeedsPassphrase),
            Err(e) => Err(e.info()),
        }
    })
    .await
}

/// Stores key material (and optionally its passphrase) and registers it.
fn register_key(
    state: &St<'_>,
    label: &str,
    text: &str,
    meta: &aether_ssh::KeyMeta,
    passphrase: Option<&str>,
    save_passphrase: bool,
    origin: KeyOrigin,
) -> Res<KeyInfo> {
    let id = KeyId::new();
    let material = vault_key(Namespace::KeyMaterial, id)?;
    state.vault.put(&material, &Secret::from_text(text)).map_err(vault_err)?;

    let mut stored_pass = None;
    if save_passphrase && let Some(p) = passphrase.filter(|p| !p.is_empty()) {
        let key = vault_key(Namespace::KeyPassphrase, id)?;
        state.vault.put(&key, &Secret::from_text(p)).map_err(vault_err)?;
        stored_pass = Some(key);
    }

    let info = KeyInfo {
        id,
        label: label.to_string(),
        algorithm: meta.algorithm.clone(),
        bits: meta.bits,
        fingerprint: meta.fingerprint.clone(),
        public_key: meta.public_key.clone(),
        encrypted: meta.encrypted,
        origin,
        created_at: now_ms(),
    };
    state.keys.add(info).map_err(|e| {
        // Registration failed (e.g. duplicate): don't leave orphaned secrets behind.
        let _ = state.vault.delete(&material);
        if let Some(k) = &stored_pass {
            let _ = state.vault.delete(k);
        }
        map_core(e)
    })
}

#[tauri::command]
pub async fn key_import(state: St<'_>, request: KeyImportRequest) -> Res<KeyInfo> {
    let (source, supplied) = (request.source.clone(), request.passphrase.clone());
    let (text, meta) = blocking(move || {
        let text = read_key_source(&source)?;
        let passphrase = supplied.as_deref().filter(|p| !p.is_empty());
        match aether_ssh::inspect_key(&text, passphrase).map_err(|e| e.info())? {
            Inspection::Meta(m) => Ok((text, m)),
            Inspection::NeedsPassphrase => Err(ErrorInfo::new(
                ErrorCode::KeyPassphraseRequired,
                "This key is protected by a passphrase",
                "Enter its passphrase to import it.",
            )),
        }
    })
    .await?;
    let passphrase = request.passphrase.as_deref().filter(|p| !p.is_empty());
    let label = if request.label.trim().is_empty() { meta.comment.clone() } else { request.label.clone() };
    register_key(&state, &label, &text, &meta, passphrase, request.save_passphrase, KeyOrigin::Imported)
}

#[tauri::command]
pub async fn key_generate(state: St<'_>, request: KeyGenerateRequest) -> Res<KeyInfo> {
    let (kind, comment, passphrase) = (request.kind, request.comment.clone(), request.passphrase.clone());
    // RSA-4096 takes a few seconds; keep the UI thread free.
    let generated =
        tokio::task::spawn_blocking(move || aether_ssh::generate_key(kind, &comment, passphrase.as_deref()))
            .await
            .map_err(|e| ErrorInfo::internal(e.to_string()))?
            .map_err(|e| e.info())?;
    register_key(
        &state,
        &request.label,
        &generated.private_openssh,
        &generated.meta,
        request.passphrase.as_deref(),
        request.save_passphrase,
        KeyOrigin::Generated,
    )
}

#[tauri::command]
pub fn key_rename(state: St<'_>, id: KeyId, label: String) -> Res<KeyInfo> {
    state.keys.rename(id, &label).map_err(map_core)
}

/// Deletes a key, its stored passphrase, and clears it from any host that used it.
#[tauri::command]
pub fn key_delete(state: St<'_>, id: KeyId) -> Res<()> {
    state.keys.remove(id).map_err(map_core)?;
    let _ = state.vault.delete(&vault_key(Namespace::KeyMaterial, id)?);
    let _ = state.vault.delete(&vault_key(Namespace::KeyPassphrase, id)?);
    for host in state.hosts.snapshot().hosts {
        if matches!(&host.auth.key, Some(KeySource::Vault { key_id }) if *key_id == id) {
            let mut h = host;
            h.auth = AuthConfig { key: None, ..h.auth };
            let _ = state.hosts.upsert(h);
        }
    }
    Ok(())
}

#[tauri::command]
pub fn key_public(state: St<'_>, id: KeyId) -> Res<String> {
    state.keys.get(id).map(|k| k.public_key).ok_or_else(|| not_found("This key"))
}

#[tauri::command]
pub fn key_export_public(state: St<'_>, id: KeyId, path: String) -> Res<()> {
    let key = state.keys.get(id).ok_or_else(|| not_found("This key"))?;
    std::fs::write(&path, format!("{}\n", key.public_key)).map_err(|e| {
        ErrorInfo::new(ErrorCode::Io, "Couldn't save the file", format!("{path} couldn't be written."))
            .detail(e.to_string())
    })
}

/// Appends a public key to the server's `~/.ssh/authorized_keys` (like `ssh-copy-id`),
/// over an already-authenticated session.
#[tauri::command]
pub async fn key_install(state: St<'_>, connection_id: ConnectionId, id: KeyId) -> Res<()> {
    let key = state.keys.get(id).ok_or_else(|| not_found("This key"))?;
    let session = state.session(connection_id)?;
    let conn = session.connection().ok_or_else(|| SshError::Disconnected("not connected".into()).info())?;

    // The public key line is made of base64 plus a comment; restrict the comment so the
    // line can be embedded in single quotes without any escaping.
    let mut parts = key.public_key.splitn(3, ' ');
    let (algo, blob) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let comment: String = parts
        .next()
        .unwrap_or("aether")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || "@._-".contains(c) { c } else { '_' })
        .collect();
    let valid_part = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "+/=-@.".contains(c));
    if !valid_part(algo) || !valid_part(blob) {
        return Err(ErrorInfo::invalid("public key", "this key can't be installed automatically"));
    }
    let line = format!("{algo} {blob} {comment}");
    let script = format!(
        "umask 077; mkdir -p ~/.ssh && touch ~/.ssh/authorized_keys && \
         (grep -qxF '{line}' ~/.ssh/authorized_keys || printf '%s\\n' '{line}' >> ~/.ssh/authorized_keys) && \
         chmod 700 ~/.ssh && chmod 600 ~/.ssh/authorized_keys && echo aether-ok"
    );
    let out = conn.exec(&script, Duration::from_secs(20)).await.map_err(|e| e.info())?;
    if out.status == Some(0) && out.stdout_text().contains("aether-ok") {
        Ok(())
    } else {
        Err(ErrorInfo::new(
            ErrorCode::PermissionDenied,
            "Couldn't install the key",
            "The server didn't accept the command. It may use a restricted shell.",
        )
        .detail(String::from_utf8_lossy(&out.stderr).into_owned()))
    }
}

// ── known hosts ──────────────────────────────────────────────────────────────

#[tauri::command]
pub fn known_hosts_list(state: St<'_>) -> Vec<KnownHostEntry> {
    state.known_hosts.list()
}

/// Forgets one key from Aether's own file. The line number is only a snapshot from when the list was drawn, so the
/// fingerprint travels with it and has to still match.
#[tauri::command]
pub fn known_host_remove(state: St<'_>, line: u32, fingerprint: String) -> Res<usize> {
    state.known_hosts.remove_line(line, &fingerprint).map_err(map_core)
}
