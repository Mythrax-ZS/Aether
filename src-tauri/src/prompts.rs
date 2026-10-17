//! Asking the user questions mid-connection (host key trust, passwords, 2FA codes).
//!
//! The engine awaits an answer; the broker shows the question in the window that owns the
//! connection and resolves the future when the user responds, cancels, or the connection
//! is closed.

use std::time::Duration;

use aether_core::{ConnectionId, PromptId};
use dashmap::DashMap;
use tauri::{AppHandle, Emitter};
use tokio::sync::oneshot;
use tracing::{debug, warn};

use crate::dto::{PromptClosedEvent, PromptEvent, PromptPayload, PromptReply};

/// A forgotten prompt shouldn't pin a connection forever.
const PROMPT_TIMEOUT: Duration = Duration::from_secs(10 * 60);

struct Pending {
    connection: ConnectionId,
    tx: oneshot::Sender<PromptReply>,
}

pub struct PromptBroker {
    app: AppHandle,
    pending: DashMap<PromptId, Pending>,
    /// Which window shows prompts for a connection.
    owners: DashMap<ConnectionId, String>,
}

impl PromptBroker {
    pub fn new(app: AppHandle) -> Self {
        Self { app, pending: DashMap::new(), owners: DashMap::new() }
    }

    pub fn set_owner(&self, connection: ConnectionId, window: &str) {
        self.owners.insert(connection, window.to_string());
    }

    /// Connections currently owned by `window`.
    pub fn connections_of(&self, window: &str) -> Vec<ConnectionId> {
        self.owners.iter().filter(|o| o.value() == window).map(|o| *o.key()).collect()
    }

    fn owner(&self, connection: ConnectionId) -> String {
        self.owners.get(&connection).map(|o| o.clone()).unwrap_or_else(|| "main".into())
    }

    /// Cancels every open prompt of a connection (it was closed) and forgets its owner.
    pub fn clear_connection(&self, connection: ConnectionId) {
        let ids: Vec<PromptId> = self.pending.iter().filter(|p| p.connection == connection).map(|p| *p.key()).collect();
        for id in ids {
            // Dropping the sender resolves the waiting `ask` with a cancellation.
            self.pending.remove(&id);
            let _ = self.app.emit("prompt:closed", PromptClosedEvent { id });
        }
        self.owners.remove(&connection);
    }

    /// Shows a question and waits for the answer. Returns `Cancel` on timeout or close.
    pub async fn ask(&self, connection: ConnectionId, payload: PromptPayload) -> PromptReply {
        let id = PromptId::new();
        let (tx, rx) = oneshot::channel();
        self.pending.insert(id, Pending { connection, tx });
        // From here on the question comes off the screen however this future ends — answered, timed out, or dropped
        // half-way because the connection attempt it belongs to was cancelled.
        let _open = Open { broker: self, id };

        let event = PromptEvent { id, connection_id: connection, payload };
        let window = self.owner(connection);
        if let Err(e) = self.app.emit_to(window.as_str(), "prompt:request", &event) {
            warn!(error = %e, %window, "owner window unavailable; broadcasting prompt");
            let _ = self.app.emit("prompt:request", &event);
        }
        debug!(%id, %connection, %window, "prompt shown");

        match tokio::time::timeout(PROMPT_TIMEOUT, rx).await {
            Ok(Ok(reply)) => reply,
            Ok(Err(_)) | Err(_) => PromptReply::Cancel,
        }
    }

    /// Delivers the user's answer. Returns false if the prompt is no longer waiting.
    pub fn respond(&self, id: PromptId, reply: PromptReply) -> bool {
        match self.pending.remove(&id) {
            Some((_, pending)) => pending.tx.send(reply).is_ok(),
            None => false,
        }
    }
}

/// Takes a question off the broker — and tells the window to drop its dialog — when the `ask` that opened it is over.
struct Open<'a> {
    broker: &'a PromptBroker,
    id: PromptId,
}

impl Drop for Open<'_> {
    fn drop(&mut self) {
        // Already gone if it was answered (`respond`) or its connection was closed (`clear_connection`).
        if self.broker.pending.remove(&self.id).is_some() {
            let _ = self.broker.app.emit("prompt:closed", PromptClosedEvent { id: self.id });
        }
    }
}
