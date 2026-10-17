//! Error taxonomy shared by every Aether crate.
//!
//! Each crate keeps its own `thiserror` enum for precise handling in Rust, and maps it
//! into an [`ErrorInfo`] at the boundary. `ErrorInfo` is what the UI receives: a stable
//! machine-readable [`ErrorCode`] plus human-readable copy, so the frontend never has to
//! pattern-match on message strings and users never see raw error text like
//! `Os { code: 10061, kind: ConnectionRefused }`.

use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Machine-readable error category. The UI uses it to choose an icon, tone and the
/// recovery actions offered (retry, edit host, re-enter password, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum ErrorCode {
    // ── generic ──────────────────────────────────────────────────────────────
    Internal,
    InvalidInput,
    NotFound,
    Conflict,
    Io,
    Cancelled,
    Unsupported,
    Timeout,
    // ── local storage ────────────────────────────────────────────────────────
    StoreCorrupt,
    VaultUnavailable,
    // ── network ──────────────────────────────────────────────────────────────
    DnsFailure,
    ConnectionRefused,
    ConnectionTimeout,
    NetworkUnreachable,
    ConnectionReset,
    // ── ssh ──────────────────────────────────────────────────────────────────
    HostKeyRejected,
    HostKeyChanged,
    AuthFailed,
    AuthCancelled,
    NoAuthMethods,
    KeyPassphraseRequired,
    KeyPassphraseWrong,
    KeyUnreadable,
    ProtocolError,
    NoCommonAlgorithm,
    ChannelOpenFailed,
    Disconnected,
    JumpHostFailed,
    AgentUnavailable,
    ForwardFailed,
    PermissionDenied,
    Sftp,
}

/// A user-presentable error. This is the single error type that crosses the IPC boundary.
#[derive(Debug, Clone, Serialize, Deserialize, TS, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
#[error("{title}: {message}")]
pub struct ErrorInfo {
    pub code: ErrorCode,
    /// Short headline, e.g. "Connection timed out".
    pub title: String,
    /// One or two sentences explaining what happened in plain language.
    pub message: String,
    /// What the user can do about it.
    pub hint: Option<String>,
    /// Technical detail for a collapsed "Show details" section. Never contains secrets.
    pub detail: Option<String>,
    /// Whether trying the same operation again could plausibly succeed.
    pub retryable: bool,
}

impl ErrorInfo {
    pub fn new(code: ErrorCode, title: impl Into<String>, message: impl Into<String>) -> Self {
        Self { code, title: title.into(), message: message.into(), hint: None, detail: None, retryable: false }
    }

    #[must_use]
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    #[must_use]
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    #[must_use]
    pub fn retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    pub fn internal(detail: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, "Something went wrong", "Aether hit an unexpected problem. Your data is safe.")
            .hint("If this keeps happening, open the logs from Settings → About and file an issue.")
            .detail(detail)
    }

    pub fn invalid(field: &str, reason: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidInput, format!("Invalid {field}"), reason.into())
    }

    pub fn cancelled() -> Self {
        Self::new(ErrorCode::Cancelled, "Cancelled", "The operation was cancelled.")
    }
}

/// Implemented by error enums that can describe themselves to the user.
pub trait UserFacing {
    fn info(&self) -> ErrorInfo;
}

impl UserFacing for ErrorInfo {
    fn info(&self) -> ErrorInfo {
        self.clone()
    }
}

/// Errors raised by `aether-core` itself (storage, parsing, validation).
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("I/O error on {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not parse {what}: {message}")]
    Parse { what: String, message: String },
    #[error("{0} was not found")]
    NotFound(String),
    #[error("invalid {field}: {reason}")]
    Invalid { field: String, reason: String },
    #[error("{0}")]
    Conflict(String),
    #[error("{} is corrupt: {message}", path.display())]
    Corrupt { path: PathBuf, message: String },
}

pub type Result<T, E = CoreError> = std::result::Result<T, E>;

impl CoreError {
    pub fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io { path: path.into(), source }
    }

    pub fn invalid(field: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Invalid { field: field.into(), reason: reason.into() }
    }

    pub fn not_found(what: impl Into<String>) -> Self {
        Self::NotFound(what.into())
    }
}

impl UserFacing for CoreError {
    fn info(&self) -> ErrorInfo {
        match self {
            CoreError::Io { path, source } => {
                let (code, title, hint) = match source.kind() {
                    io::ErrorKind::PermissionDenied => (
                        ErrorCode::PermissionDenied,
                        "Permission denied",
                        Some("Check that Aether is allowed to read and write this location."),
                    ),
                    io::ErrorKind::NotFound => {
                        (ErrorCode::NotFound, "File not found", Some("The file may have been moved or deleted."))
                    }
                    _ => (ErrorCode::Io, "File error", None),
                };
                let mut info = ErrorInfo::new(code, title, format!("Couldn't access {}.", path.display()))
                    .detail(source.to_string());
                if let Some(h) = hint {
                    info = info.hint(h);
                }
                info
            }
            CoreError::Parse { what, message } => {
                ErrorInfo::new(ErrorCode::InvalidInput, format!("Couldn't read {what}"), message.clone())
            }
            CoreError::NotFound(what) => {
                ErrorInfo::new(ErrorCode::NotFound, "Not found", format!("{what} no longer exists."))
            }
            CoreError::Invalid { field, reason } => ErrorInfo::invalid(field, reason.clone()),
            CoreError::Conflict(message) => ErrorInfo::new(ErrorCode::Conflict, "Conflict", message.clone()),
            CoreError::Corrupt { path, message } => ErrorInfo::new(
                ErrorCode::StoreCorrupt,
                "Data file is damaged",
                format!(
                    "{} couldn't be read, so Aether started with defaults.",
                    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
                ),
            )
            .hint("A copy of the damaged file was kept next to it with a .corrupt suffix.")
            .detail(message.clone()),
        }
    }
}

impl From<CoreError> for ErrorInfo {
    fn from(e: CoreError) -> Self {
        e.info()
    }
}
