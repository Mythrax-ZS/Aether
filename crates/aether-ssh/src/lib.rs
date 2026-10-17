//! # aether-ssh
//!
//! Aether's SSH engine, built on [`russh`]. It knows nothing about windows, webviews or
//! JSON files: callers describe what to connect to with a [`ConnectSpec`], supply a
//! [`ConnectionDelegate`] to answer questions (host keys, passwords), and receive terminal
//! output, port-forward statistics and SFTP results as plain Rust values.
//!
//! ```text
//!   ConnectSpec ──► SshConnection::connect ──► SshConnection
//!                     │  transport (TCP / ProxyCommand / jump channel)
//!                     │  handler   (host key policy, forwarded channels)
//!                     │  auth      (agent → keys → kbd-interactive → password)
//!                     ▼
//!                  open_shell · exec · forwards() · open_sftp
//! ```

mod agent;
mod auth;
mod config;
mod connection;
mod delegate;
mod error;
mod forward;
mod handler;
mod keys;
mod session;
mod sftp;
mod shell;
mod socks;
mod spec;
mod text;
mod transfers;
mod transport;
mod x11;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

#[cfg(test)]
mod engine_tests;

#[cfg(test)]
mod sftp_tests;

#[cfg(test)]
mod session_tests;

pub use agent::{OPENSSH_AGENT_PIPE, agent_available};
pub use connection::{ConnectContext, ConnectionInfo, SshConnection};
pub use delegate::{
    ConnectProgress, ConnectStage, ConnectionDelegate, HopInfo, HostKeyDecision, HostKeyRequest, HostKeyRequestKind,
    InteractivePrompt, PromptKind, PromptRequest, PromptResponse, SecretKind, SecretLookup,
};
pub use error::{HostKeyRejection, Result, SshError};
pub use forward::{ForwardManager, ForwardState, ForwardStatus};
pub use handler::{DisconnectInfo, DisconnectKind, NegotiatedInfo, ServerKeyInfo};
pub use keys::{GeneratedKey, Inspection, KeyKind, KeyMeta, generate_key, inspect_key, looks_encrypted};
pub use session::{
    Session, SessionEvents, SessionManager, SessionOptions, SessionState, SessionSummary, Terminal, TerminalSink,
    TerminalState, backoff,
};
pub use sftp::{DiskUsage, EntryKind, RemoteEntry, SftpClient, file_name_remote, join_remote, parent_remote};
pub use shell::{ExecOutput, ShellEvent, ShellHandle};
pub use spec::{AuthSpec, ConnectSpec, KeyRef, PtySize, ShellParams, TransportOptions, X11Options};
pub use text::{FileVersion, LineEnding, NotEditable, RemoteText, decode as decode_text, encode as encode_text};
pub use transfers::{
    ConflictPolicy, Plan, TransferDirection, TransferEvents, TransferInfo, TransferManager, TransferOptions,
    TransferRequest, TransferState, plan_download, plan_upload,
};
pub use x11::{LocalDisplay, probe as probe_x11_display};
