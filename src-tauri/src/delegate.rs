//! Connects the SSH engine to the app: prompts go to the UI, secrets to the vault.

use std::sync::Arc;

use aether_core::ConnectionId;
use aether_ssh::{
    ConnectionDelegate, HopInfo, HostKeyDecision, HostKeyRequest, PromptRequest, PromptResponse, SecretKind,
    SecretLookup,
};
use aether_vault::{Namespace, Secret, SecretKey, SecretStore};
use async_trait::async_trait;
use tauri::{AppHandle, Emitter};
use tracing::warn;

use crate::dto::{BannerEvent, PromptPayload, PromptReply};
use crate::prompts::PromptBroker;

pub struct AppDelegate {
    pub app: AppHandle,
    pub connection_id: ConnectionId,
    pub prompts: Arc<PromptBroker>,
    pub vault: Arc<dyn SecretStore>,
}

/// Maps an engine lookup onto a vault address. Hosts without a scope (quick-connect
/// targets that were never saved) have nowhere to keep secrets, by design.
fn secret_key(lookup: &SecretLookup) -> Option<SecretKey> {
    match &lookup.kind {
        SecretKind::Password => SecretKey::new(Namespace::HostPassword, lookup.scope.clone()?).ok(),
        SecretKind::KeyPassphrase { key_id } => SecretKey::new(Namespace::KeyPassphrase, key_id.clone()).ok(),
    }
}

#[async_trait]
impl ConnectionDelegate for AppDelegate {
    async fn verify_host_key(&self, request: HostKeyRequest) -> HostKeyDecision {
        match self.prompts.ask(self.connection_id, PromptPayload::HostKey { request }).await {
            PromptReply::HostKey { decision } => decision,
            _ => HostKeyDecision::Reject,
        }
    }

    async fn prompt(&self, request: PromptRequest) -> Option<PromptResponse> {
        match self.prompts.ask(self.connection_id, PromptPayload::Secret { request }).await {
            PromptReply::Answers { answers, remember } => Some(PromptResponse {
                answers: answers.iter().map(|a| Secret::from_text(a.as_str())).collect(),
                remember,
            }),
            _ => None,
        }
    }

    fn saved_secret(&self, lookup: &SecretLookup) -> Option<Secret> {
        let key = secret_key(lookup)?;
        match self.vault.get(&key) {
            Ok(secret) => secret,
            Err(e) => {
                warn!(error = %e, "could not read a saved secret");
                None
            }
        }
    }

    fn remember_secret(&self, lookup: &SecretLookup, secret: &Secret) {
        if let Some(key) = secret_key(lookup)
            && let Err(e) = self.vault.put(&key, secret)
        {
            warn!(error = %e, "could not save a secret");
        }
    }

    fn forget_secret(&self, lookup: &SecretLookup) {
        if let Some(key) = secret_key(lookup) {
            let _ = self.vault.delete(&key);
        }
    }

    fn banner(&self, _hop: &HopInfo, text: &str) {
        let _ =
            self.app.emit("session:banner", BannerEvent { connection_id: self.connection_id, text: text.to_string() });
    }

    fn vault_key(&self, key_id: &str) -> Option<Secret> {
        let key = SecretKey::new(Namespace::KeyMaterial, key_id).ok()?;
        self.vault.get(&key).ok().flatten()
    }
}
