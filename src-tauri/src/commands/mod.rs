//! IPC commands. Each is a thin adapter: validate input, call the right crate, map the
//! error to a user-presentable [`ErrorInfo`](aether_core::ErrorInfo).

pub mod app;
pub mod hosts;
pub mod keys;
pub mod local;
pub mod sessions;
pub mod settings;
pub mod sftp;

use std::sync::Arc;

use crate::state::AppState;

/// Shorthand for the shared state parameter every command takes.
pub type St<'a> = tauri::State<'a, Arc<AppState>>;

/// Runs slow, blocking work (file-system calls, key stretching, reading a big config) off the thread that serves the
/// UI. A command that blocks the thread it was called on freezes every window until it returns.
pub(crate) async fn blocking<T: Send + 'static>(
    job: impl FnOnce() -> crate::state::Res<T> + Send + 'static,
) -> crate::state::Res<T> {
    tokio::task::spawn_blocking(job).await.map_err(|e| aether_core::ErrorInfo::internal(e.to_string()))?
}
