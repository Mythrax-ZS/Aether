//! Errors raised by the SSH engine, with friendly [`UserFacing`] descriptions.

use std::io;

use aether_core::error::{ErrorCode, ErrorInfo, UserFacing};

/// Why a server's host key was not accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKeyRejection {
    /// The user chose not to trust a new host.
    Declined,
    /// The recorded key differs from the one presented (possible interception).
    Changed,
    /// The key is marked `@revoked`.
    Revoked,
    /// Policy is "strict" and the host isn't in known_hosts.
    UnknownInStrictMode,
    /// The key was accepted but could not be recorded (the carried text says why). Connecting anyway would make
    /// "trust and remember" a promise that wasn't kept.
    NotSaved(String),
    /// A file of recorded keys exists but couldn't be read, so there is no telling whether this key is the one on record
    /// (or revoked). Carrying on would treat a changed key as a first visit.
    Unverifiable {
        file: String,
        /// Is it the user's own `~/.ssh/known_hosts` (which Aether can be told to stop reading)?
        user_file: bool,
        why: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum SshError {
    #[error("could not resolve host `{host}`: {message}")]
    Dns { host: String, message: String },
    #[error("connection to {host}:{port} was refused")]
    ConnectionRefused { host: String, port: u16 },
    #[error("connecting to {host}:{port} timed out")]
    ConnectTimeout { host: String, port: u16 },
    #[error("network unreachable: {0}")]
    Unreachable(String),
    #[error("connection reset by peer")]
    ConnectionReset,
    #[error("host key for {host} was not accepted ({reason:?})")]
    HostKeyRejected { host: String, reason: HostKeyRejection },
    #[error("authentication failed for {user}@{host}")]
    AuthFailed { host: String, user: String, tried: Vec<String> },
    #[error("authentication cancelled")]
    AuthCancelled,
    #[error("wrong passphrase for key `{key}`")]
    WrongPassphrase { key: String },
    #[error("cannot use key `{key}`: {reason}")]
    KeyUnreadable { key: String, reason: String },
    #[error("no authentication method is available")]
    NoAuthMethods,
    #[error("ssh protocol error: {0}")]
    Protocol(String),
    #[error("no common {kind} algorithm (ours: {ours:?}, theirs: {theirs:?})")]
    NoCommonAlgorithm { kind: String, ours: Vec<String>, theirs: Vec<String> },
    #[error("server refused to open {what}: {reason}")]
    ChannelOpen { what: String, reason: String },
    #[error("connection lost: {0}")]
    Disconnected(String),
    #[error("keepalive timed out")]
    KeepaliveTimeout,
    /// The connection dropped while a file was being swapped for its new version, and it can't be told which of the two
    /// is where the file belongs. Nothing is lost; this says where to look.
    #[error("the connection dropped while {target} was being replaced")]
    SaveInterrupted { target: String, original: String, new: String },
    #[error("jump host `{hop}` failed: {source}")]
    JumpHost {
        hop: String,
        #[source]
        source: Box<SshError>,
    },
    #[error("ssh-agent is not available: {0}")]
    AgentUnavailable(String),
    #[error("port forward failed: {0}")]
    Forward(String),
    #[error("sftp error: {0}")]
    Sftp(String),
    #[error("proxy command failed: {0}")]
    ProxyCommand(String),
    #[error("cancelled")]
    Cancelled,
    #[error("{0}")]
    Invalid(String),
    #[error("i/o error: {0}")]
    Io(#[from] io::Error),
    #[error("vault: {0}")]
    Vault(#[from] aether_vault::VaultError),
    #[error("internal error: {0}")]
    Internal(String),
}

pub type Result<T, E = SshError> = std::result::Result<T, E>;

/// Does this SFTP error text say that the *channel* is gone or stuck — a timeout, an I/O failure, the session task
/// having ended — as opposed to the server answering "no" to one request? (russh-sftp reports these as plain strings.)
pub(crate) fn sftp_channel_is_gone(message: &str) -> bool {
    ["Timeout", "I/O", "SendError", "RecvError", "sender dropped", "session closed"]
        .iter()
        .any(|prefix| message.starts_with(prefix))
}

impl SshError {
    /// Maps an OS socket error to the most specific variant.
    pub fn from_io(e: io::Error, host: &str, port: u16) -> Self {
        use io::ErrorKind as K;
        match e.kind() {
            K::ConnectionRefused => SshError::ConnectionRefused { host: host.into(), port },
            K::TimedOut => SshError::ConnectTimeout { host: host.into(), port },
            K::ConnectionReset | K::ConnectionAborted | K::BrokenPipe | K::UnexpectedEof => SshError::ConnectionReset,
            _ => match e.raw_os_error() {
                // WSAENETUNREACH, WSAEHOSTUNREACH, WSAENETDOWN
                Some(10051 | 10065 | 10050) => SshError::Unreachable(e.to_string()),
                // WSAHOST_NOT_FOUND, WSATRY_AGAIN, WSANO_DATA
                Some(11001 | 11002 | 11004) => SshError::Dns { host: host.into(), message: e.to_string() },
                _ => SshError::Io(e),
            },
        }
    }

    /// True when the failure came from a lost connection (as opposed to a user decision or
    /// misconfiguration), i.e. when reconnecting could help.
    pub fn is_transient(&self) -> bool {
        match self {
            // A hop's failure is that hop's kind of failure, just reported with its position in the chain.
            SshError::JumpHost { source, .. } => source.is_transient(),
            // The SFTP channel timing out or going away is the connection's doing; a "permission denied" is not.
            SshError::Sftp(message) => sftp_channel_is_gone(message),
            // Reading or writing a remote file over SFTP reports a dropped connection as a plain I/O error. An end of
            // file or a timeout can as well be a local network share or a removable drive, which is no reason to give up
            // on every other file.
            SshError::Io(e) => {
                use io::ErrorKind as K;
                matches!(e.kind(), K::ConnectionReset | K::ConnectionAborted | K::BrokenPipe)
                    || sftp_channel_is_gone(&e.to_string())
            }
            _ => matches!(
                self,
                SshError::ConnectionReset
                    | SshError::ConnectTimeout { .. }
                    | SshError::Unreachable(_)
                    | SshError::Disconnected(_)
                    | SshError::KeepaliveTimeout
                    | SshError::ConnectionRefused { .. }
                    | SshError::Dns { .. }
            ),
        }
    }

    /// True for problems that trying again can't fix and only the person can: a refused host key, a wrong or
    /// cancelled login, an unreadable key, a server that shares no algorithm with us. The supervisor stops retrying
    /// on these — including when it was a jump host that said no.
    pub fn needs_user(&self) -> bool {
        match self {
            SshError::JumpHost { source, .. } => source.needs_user(),
            _ => matches!(
                self,
                SshError::AuthCancelled
                    | SshError::HostKeyRejected { .. }
                    | SshError::AuthFailed { .. }
                    | SshError::WrongPassphrase { .. }
                    | SshError::KeyUnreadable { .. }
                    | SshError::NoAuthMethods
                    | SshError::NoCommonAlgorithm { .. }
                    | SshError::Invalid(_)
            ),
        }
    }
}

impl From<russh::Error> for SshError {
    fn from(e: russh::Error) -> Self {
        use russh::Error as E;
        match e {
            E::KeepaliveTimeout => SshError::KeepaliveTimeout,
            E::HUP | E::Disconnect | E::SendError | E::RecvError => {
                SshError::Disconnected("the server closed the connection".into())
            }
            E::ConnectionTimeout | E::InactivityTimeout => SshError::Disconnected("the connection timed out".into()),
            E::IO(io) => SshError::from_io(io, "", 0),
            E::NoCommonAlgo { kind, ours, theirs } => {
                SshError::NoCommonAlgorithm { kind: format!("{kind:?}").to_lowercase(), ours, theirs }
            }
            E::ChannelOpenFailure(reason) => {
                SshError::ChannelOpen { what: "a channel".into(), reason: format!("{reason:?}") }
            }
            E::Keys(k) => SshError::KeyUnreadable { key: String::new(), reason: k.to_string() },
            other => SshError::Protocol(other.to_string()),
        }
    }
}

impl From<russh::keys::Error> for SshError {
    fn from(e: russh::keys::Error) -> Self {
        SshError::KeyUnreadable { key: String::new(), reason: e.to_string() }
    }
}

impl From<russh_sftp::client::error::Error> for SshError {
    fn from(e: russh_sftp::client::error::Error) -> Self {
        SshError::Sftp(e.to_string())
    }
}

impl UserFacing for SshError {
    fn info(&self) -> ErrorInfo {
        let detail = self.to_string();
        match self {
            SshError::Dns { host, .. } => ErrorInfo::new(
                ErrorCode::DnsFailure,
                "Can't find that host",
                format!("“{host}” couldn't be resolved to an IP address."),
            )
            .hint("Check the spelling, your network connection, and any VPN or DNS settings.")
            .retryable(true),

            SshError::ConnectionRefused { host, port } => ErrorInfo::new(
                ErrorCode::ConnectionRefused,
                "Connection refused",
                format!("{host} answered on port {port}, but nothing there is accepting SSH connections."),
            )
            .hint("Make sure the SSH server is running and that the port number is correct.")
            .retryable(true),

            SshError::ConnectTimeout { host, port } => ErrorInfo::new(
                ErrorCode::ConnectionTimeout,
                "Connection timed out",
                format!("No response from {host}:{port}."),
            )
            .hint("The host may be offline, a firewall may be dropping packets, or the address may be wrong.")
            .retryable(true),

            SshError::Unreachable(_) => ErrorInfo::new(
                ErrorCode::NetworkUnreachable,
                "Network unreachable",
                "Your computer has no route to that host.",
            )
            .hint("Check your network connection and VPN.")
            .retryable(true),

            SshError::ConnectionReset => ErrorInfo::new(
                ErrorCode::ConnectionReset,
                "Connection lost",
                "The connection was closed unexpectedly.",
            )
            .retryable(true),

            SshError::HostKeyRejected { host, reason } => match reason {
                HostKeyRejection::Changed => ErrorInfo::new(
                    ErrorCode::HostKeyChanged,
                    "Host key changed — connection blocked",
                    format!("{host} presented a different key than the one you trusted before. This can mean someone is intercepting the connection, or that the server was reinstalled."),
                )
                .hint("If you expected this (for example the server was rebuilt), remove its old key under Known hosts and connect again."),
                HostKeyRejection::Revoked => ErrorInfo::new(
                    ErrorCode::HostKeyRejected,
                    "Host key revoked",
                    format!("The key presented by {host} is marked as revoked in your known_hosts."),
                ),
                HostKeyRejection::UnknownInStrictMode => ErrorInfo::new(
                    ErrorCode::HostKeyRejected,
                    "Unknown host",
                    format!("{host} isn't in your known hosts and strict host key checking is enabled."),
                )
                .hint("Change the host key policy in Settings → SSH, or add the host key manually."),
                HostKeyRejection::Declined => ErrorInfo::new(
                    ErrorCode::HostKeyRejected,
                    "Host not trusted",
                    format!("You chose not to trust {host}, so the connection was not made."),
                ),
                HostKeyRejection::NotSaved(why) => ErrorInfo::new(
                    ErrorCode::HostKeyRejected,
                    "Couldn't remember the server's key",
                    format!("Aether couldn't save {host}'s key, so it stopped instead of trusting it without a record."),
                )
                .hint("Check that Aether's config folder is writable and not full, then try again — or choose “Connect once”.")
                .detail(why.clone()),
                HostKeyRejection::Unverifiable { file, user_file, why } => ErrorInfo::new(
                    ErrorCode::HostKeyRejected,
                    "Couldn't check the server's key",
                    format!(
                        "Aether couldn't read {file}, so it can't tell whether {host}'s key is the one you trusted before. It stopped instead of guessing."
                    ),
                )
                .hint(if *user_file {
                    "Fix that file's permissions, or turn off “Also trust ~/.ssh/known_hosts” in Settings → SSH, then try again."
                } else {
                    "Make sure the file isn't open in another program and that you can read it, then try again."
                })
                .detail(why.clone()),
            },

            SshError::AuthFailed { host, user, tried } => ErrorInfo::new(
                ErrorCode::AuthFailed,
                "Authentication failed",
                if tried.is_empty() {
                    format!("{host} didn't accept any login method for {user}.")
                } else {
                    format!(
                        "{host} didn't accept {user}'s login. Tried: {}.",
                        tried.join(", ")
                    )
                },
            )
            .hint("Check the username and password, or make sure your public key is listed in ~/.ssh/authorized_keys on the server.")
            .retryable(true),

            SshError::AuthCancelled => ErrorInfo::new(
                ErrorCode::AuthCancelled,
                "Sign-in cancelled",
                "You cancelled the sign-in.",
            ),

            SshError::WrongPassphrase { key } => ErrorInfo::new(
                ErrorCode::KeyPassphraseWrong,
                "Wrong passphrase",
                format!("The passphrase for “{key}” didn't unlock it."),
            ),

            SshError::KeyUnreadable { key, reason } => ErrorInfo::new(
                ErrorCode::KeyUnreadable,
                "Can't use that key",
                if key.is_empty() {
                    "The key couldn't be read.".to_string()
                } else {
                    format!("“{key}” couldn't be read.")
                },
            )
            .hint("Supported formats: OpenSSH, PEM (PKCS#1/PKCS#8) and PuTTY .ppk.")
            .detail(reason.clone()),

            SshError::NoAuthMethods => ErrorInfo::new(
                ErrorCode::NoAuthMethods,
                "No way to sign in",
                "None of the host's login methods can be used with the credentials available.",
            )
            .hint("Edit the host and add a password or a key, or start your ssh-agent."),

            SshError::NoCommonAlgorithm { kind, .. } => ErrorInfo::new(
                ErrorCode::NoCommonAlgorithm,
                "No compatible encryption",
                format!("Aether and the server have no {kind} algorithm in common."),
            )
            .hint("This is typical for older routers and appliances — turn on “Allow legacy algorithms” in the host's Network settings."),

            SshError::Protocol(_) => ErrorInfo::new(
                ErrorCode::ProtocolError,
                "SSH protocol error",
                "The server sent something Aether couldn't understand.",
            )
            .retryable(true),

            SshError::ChannelOpen { what, reason } => ErrorInfo::new(
                ErrorCode::ChannelOpenFailed,
                "The server refused the request",
                format!("It wouldn't open {what} ({reason})."),
            )
            .hint("The server's configuration may forbid this (for example AllowTcpForwarding or MaxSessions)."),

            SshError::SaveInterrupted { target, original, new } => ErrorInfo::new(
                ErrorCode::Disconnected,
                "The save was interrupted",
                format!(
                    "The connection dropped while {target} was being replaced. If it is missing, your original is saved as {original}; the new contents are in {new}."
                ),
            )
            .hint(format!("Reconnect, then rename whichever of the two you want back to {target}.")),

            SshError::Disconnected(_) | SshError::KeepaliveTimeout => ErrorInfo::new(
                ErrorCode::Disconnected,
                "Connection lost",
                "The connection to the server was lost.",
            )
            .retryable(true),

            SshError::JumpHost { hop, source } => {
                let inner = source.info();
                ErrorInfo::new(
                    ErrorCode::JumpHostFailed,
                    format!("Couldn't reach the jump host “{hop}”"),
                    inner.message,
                )
                .hint(inner.hint.unwrap_or_else(|| "Fix the jump host first, then try again.".into()))
                .retryable(inner.retryable)
            }

            SshError::AgentUnavailable(why) => ErrorInfo::new(
                ErrorCode::AgentUnavailable,
                "ssh-agent isn't running",
                "Aether couldn't reach an SSH agent.",
            )
            .hint("Start the “OpenSSH Authentication Agent” service, or run Pageant.")
            .detail(why.clone()),

            SshError::Forward(why) => ErrorInfo::new(
                ErrorCode::ForwardFailed,
                "Port forward failed",
                why.clone(),
            ),

            // SFTP statuses arrive as "<Status name>: <text>"; name the common ones plainly.
            SshError::Sftp(status) => {
                let s = status.to_ascii_lowercase();
                if s.starts_with("no such file") {
                    ErrorInfo::new(ErrorCode::NotFound, "That item isn't there", "It was moved or deleted on the server.")
                        .hint("Refresh the folder and try again.")
                } else if s.starts_with("permission denied") {
                    ErrorInfo::new(
                        ErrorCode::PermissionDenied,
                        "Permission denied",
                        "The server doesn't let your account do that here.",
                    )
                    .hint("Check the permissions of the folder, or choose a location you own.")
                } else if s.starts_with("failure") {
                    // OpenSSH answers every unclassified problem with the same generic status.
                    ErrorInfo::new(
                        ErrorCode::Sftp,
                        "The server refused that",
                        "The operation failed on the server — commonly because something with that name already exists, a folder isn't empty, or the disk is full.",
                    )
                    .hint("Check for a name clash or free space, then try again.")
                } else if s.starts_with("timeout") || s.starts_with("no connection") || s.starts_with("connection lost") {
                    ErrorInfo::new(ErrorCode::Disconnected, "Lost contact with the server", "The file server stopped answering.")
                        .hint("Wait for the connection to come back, then try again.")
                        .retryable(true)
                } else {
                    ErrorInfo::new(ErrorCode::Sftp, "File transfer error", "The SFTP server reported a problem.")
                }
            }

            SshError::ProxyCommand(why) => ErrorInfo::new(
                ErrorCode::ConnectionRefused,
                "Proxy command failed",
                "The ProxyCommand couldn't start or exited before the connection was made.",
            )
            .detail(why.clone())
            .retryable(true),

            SshError::Cancelled => ErrorInfo::cancelled(),
            SshError::Invalid(why) => ErrorInfo::new(ErrorCode::InvalidInput, "Invalid request", why.clone()),

            SshError::Io(e) => {
                let code = if e.kind() == io::ErrorKind::PermissionDenied {
                    ErrorCode::PermissionDenied
                } else {
                    ErrorCode::Io
                };
                ErrorInfo::new(code, "I/O error", e.to_string())
            }

            SshError::Vault(_) => ErrorInfo::new(
                ErrorCode::VaultUnavailable,
                "Secure storage problem",
                "Aether couldn't read or write its secure credential storage.",
            ),

            SshError::Internal(_) => ErrorInfo::internal(""),
        }
        .detail_if_missing(detail)
    }
}

trait DetailExt {
    fn detail_if_missing(self, detail: String) -> Self;
}

impl DetailExt for ErrorInfo {
    fn detail_if_missing(mut self, detail: String) -> Self {
        if self.detail.as_deref().is_none_or(str::is_empty) {
            self.detail = Some(detail);
        }
        self
    }
}

impl From<SshError> for ErrorInfo {
    fn from(e: SshError) -> Self {
        e.info()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_errors_map_to_specific_variants() {
        let refused = io::Error::from(io::ErrorKind::ConnectionRefused);
        assert!(matches!(SshError::from_io(refused, "h", 22), SshError::ConnectionRefused { .. }));
        let dns = io::Error::from_raw_os_error(11001);
        assert!(matches!(SshError::from_io(dns, "h", 22), SshError::Dns { .. }));
        let unreachable = io::Error::from_raw_os_error(10051);
        assert!(matches!(SshError::from_io(unreachable, "h", 22), SshError::Unreachable(_)));
    }

    #[test]
    fn user_messages_are_helpful_and_never_empty() {
        let errors = [
            SshError::Dns { host: "x".into(), message: "m".into() },
            SshError::ConnectionRefused { host: "x".into(), port: 22 },
            SshError::ConnectTimeout { host: "x".into(), port: 22 },
            SshError::AuthFailed { host: "x".into(), user: "u".into(), tried: vec!["password".into()] },
            SshError::HostKeyRejected { host: "x".into(), reason: HostKeyRejection::Changed },
            SshError::KeepaliveTimeout,
            SshError::NoCommonAlgorithm { kind: "kex".into(), ours: vec![], theirs: vec![] },
        ];
        for e in errors {
            let info = e.info();
            assert!(!info.title.is_empty() && !info.message.is_empty(), "{e:?}");
            assert!(info.detail.is_some(), "technical detail is always attached");
        }
    }

    #[test]
    fn sftp_statuses_are_explained_in_plain_language() {
        let missing = SshError::Sftp("No such file: No such file".into()).info();
        assert_eq!(missing.code, ErrorCode::NotFound);
        let denied = SshError::Sftp("Permission denied: Permission denied".into()).info();
        assert_eq!(denied.code, ErrorCode::PermissionDenied);
        assert!(denied.hint.is_some());
        let generic = SshError::Sftp("Failure: Failure".into()).info();
        assert_eq!(generic.code, ErrorCode::Sftp);
        assert!(generic.message.contains("already exists"), "{}", generic.message);
        let lost = SshError::Sftp("Timeout".into()).info();
        assert!(lost.retryable);
        // The raw status stays available for "technical details".
        assert!(generic.detail.as_deref().is_some_and(|d| d.contains("Failure")), "{:?}", generic.detail);
    }

    #[test]
    fn transient_classification() {
        assert!(SshError::KeepaliveTimeout.is_transient());
        assert!(SshError::ConnectionReset.is_transient());
        assert!(!SshError::AuthCancelled.is_transient());
        assert!(!SshError::HostKeyRejected { host: "h".into(), reason: HostKeyRejection::Changed }.is_transient());

        // A remote file's reads and writes report a dropped connection as plain I/O errors.
        assert!(SshError::Io(io::Error::other("sender dropped")).is_transient());
        assert!(SshError::Io(io::Error::from(io::ErrorKind::BrokenPipe)).is_transient());
        assert!(!SshError::Io(io::Error::new(io::ErrorKind::PermissionDenied, "access is denied")).is_transient());
        // ...but an end of file or a timeout is as likely a network share or a removable drive on this side.
        assert!(!SshError::Io(io::Error::from(io::ErrorKind::UnexpectedEof)).is_transient());
        assert!(!SshError::Io(io::Error::from(io::ErrorKind::TimedOut)).is_transient());
    }

    /// The reconnect loop gives up on anything only a person can fix — and that must hold when the hop that said no
    /// was a jump host, not just the last one.
    #[test]
    fn a_jump_hosts_failure_is_classified_like_its_cause() {
        let jump = |source: SshError| SshError::JumpHost { hop: "bastion".into(), source: Box::new(source) };
        let denied =
            || SshError::AuthFailed { host: "bastion".into(), user: "u".into(), tried: vec!["password".into()] };
        let changed = || SshError::HostKeyRejected { host: "bastion".into(), reason: HostKeyRejection::Changed };

        for fatal in [denied(), changed(), SshError::AuthCancelled, SshError::NoAuthMethods] {
            let plain = fatal.needs_user();
            assert!(plain, "{fatal:?}");
            assert!(jump(fatal).needs_user());
        }
        assert!(!jump(SshError::ConnectionReset).needs_user());
        assert!(!SshError::KeepaliveTimeout.needs_user());

        assert!(jump(SshError::ConnectTimeout { host: "bastion".into(), port: 22 }).is_transient());
        assert!(!jump(denied()).is_transient());
        // Nested chains unwrap all the way down.
        assert!(jump(jump(changed())).needs_user());
    }

    #[test]
    fn a_dead_sftp_channel_is_the_connections_problem_a_refusal_is_not() {
        for gone in [
            "Timeout",
            "I/O: broken pipe",
            "SendError: channel closed",
            "RecvError: channel closed",
            "sender dropped",
            "session closed",
        ] {
            assert!(SshError::Sftp(gone.into()).is_transient(), "{gone}");
        }
        for refusal in ["Permission denied: Permission denied", "No such file: No such file", "Failure: Failure"] {
            assert!(!SshError::Sftp(refusal.into()).is_transient(), "{refusal}");
        }
    }
}
