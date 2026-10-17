//! Fully resolved connection requests. The app layer turns saved hosts (plus settings,
//! `~/.ssh/config` and the vault) into these; the engine never touches the stores.

use std::path::PathBuf;
use std::time::Duration;

use aether_core::model::{AgentMode, AuthMethodKind, HostKeyPolicy};

/// Where a private key comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyRef {
    /// A key file on disk.
    File(PathBuf),
    /// A key stored in the vault, identified by its key id.
    Vault { key_id: String, label: String },
}

impl KeyRef {
    /// Short name for prompts and logs ("id_ed25519", "Work laptop key").
    pub fn display_name(&self) -> String {
        match self {
            KeyRef::File(p) => {
                p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
            }
            KeyRef::Vault { label, .. } => label.clone(),
        }
    }

    /// Stable identifier used to look up a saved passphrase.
    pub fn passphrase_id(&self) -> String {
        match self {
            KeyRef::Vault { key_id, .. } => key_id.clone(),
            KeyRef::File(p) => {
                use sha2::{Digest, Sha256};
                let digest = Sha256::digest(p.to_string_lossy().to_lowercase().as_bytes());
                let hex: String = digest.iter().take(12).map(|b| format!("{b:02x}")).collect();
                format!("file-{hex}")
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct AuthSpec {
    /// Methods to try, in order.
    pub methods: Vec<AuthMethodKind>,
    /// Explicit key for public-key auth. When `None`, `default_identity_files` are tried.
    pub key: Option<KeyRef>,
    pub default_identity_files: Vec<PathBuf>,
    /// Don't offer agent keys other than the explicit one.
    pub identities_only: bool,
    pub agent: AgentMode,
}

impl Default for AuthSpec {
    fn default() -> Self {
        Self {
            methods: vec![
                AuthMethodKind::Agent,
                AuthMethodKind::PublicKey,
                AuthMethodKind::KeyboardInteractive,
                AuthMethodKind::Password,
            ],
            key: None,
            default_identity_files: Vec::new(),
            identities_only: false,
            agent: AgentMode::Auto,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TransportOptions {
    pub connect_timeout: Duration,
    /// `None` disables keepalives.
    pub keepalive_interval: Option<Duration>,
    pub keepalive_max: usize,
    pub compression: bool,
    pub nodelay: bool,
    /// Offer legacy algorithms (SHA-1 kex/MAC, CBC ciphers, ssh-rsa) for old devices.
    pub legacy_algorithms: bool,
    /// Accept agent-forwarding channels from the server (the final hop only).
    pub agent_forwarding: bool,
    /// Forward X11 to a local X server (the final hop only). `None` leaves X11 off.
    pub x11: Option<X11Options>,
    pub host_key_policy: HostKeyPolicy,
}

/// Where graphical programs from the server should appear.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct X11Options {
    /// The local X server, e.g. `localhost:0`.
    pub display: String,
    /// Cookie file to read the real MIT cookie from; defaults to `$XAUTHORITY` / `~/.Xauthority`.
    pub xauthority: Option<PathBuf>,
}

impl Default for TransportOptions {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(15),
            keepalive_interval: Some(Duration::from_secs(30)),
            keepalive_max: 3,
            compression: false,
            nodelay: true,
            legacy_algorithms: false,
            agent_forwarding: false,
            x11: None,
            host_key_policy: HostKeyPolicy::Ask,
        }
    }
}

/// Everything needed to reach and log in to one machine (possibly through jump hosts).
#[derive(Debug, Clone)]
pub struct ConnectSpec {
    /// Display name for logs and prompts.
    pub label: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: AuthSpec,
    pub options: TransportOptions,
    /// Jump hosts to traverse first, outermost first. Entries must not have their own `jump`.
    pub jump: Vec<ConnectSpec>,
    /// Run this command and speak SSH over its stdin/stdout instead of opening a TCP socket.
    pub proxy_command: Option<String>,
    /// Opaque scope handed back to the delegate when looking up saved secrets (the host id).
    pub secret_scope: Option<String>,
}

impl ConnectSpec {
    pub fn new(host: impl Into<String>, port: u16, username: impl Into<String>) -> Self {
        let host = host.into();
        Self {
            label: host.clone(),
            host,
            port,
            username: username.into(),
            auth: AuthSpec::default(),
            options: TransportOptions::default(),
            jump: Vec::new(),
            proxy_command: None,
            secret_scope: None,
        }
    }

    /// All hops in connection order, ending with the target itself.
    pub(crate) fn hops(&self) -> Vec<&ConnectSpec> {
        let mut v: Vec<&ConnectSpec> = self.jump.iter().collect();
        v.push(self);
        v
    }

    /// `user@host:port`
    pub fn target(&self) -> String {
        format!("{}@{}:{}", self.username, self.host, self.port)
    }
}

/// Terminal geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtySize {
    pub cols: u32,
    pub rows: u32,
    pub pixel_width: u32,
    pub pixel_height: u32,
}

impl Default for PtySize {
    fn default() -> Self {
        Self { cols: 120, rows: 32, pixel_width: 0, pixel_height: 0 }
    }
}

/// How to start an interactive shell.
#[derive(Debug, Clone)]
pub struct ShellParams {
    pub term: String,
    pub size: PtySize,
    pub env: Vec<(String, String)>,
    /// Ask the server to forward our ssh-agent into this session.
    pub agent_forwarding: bool,
    /// Ask the server to forward X11 (needs `TransportOptions::x11` on the connection).
    pub x11: bool,
    /// Typed into the shell right after it starts.
    pub startup_command: Option<String>,
}

impl Default for ShellParams {
    fn default() -> Self {
        Self {
            term: "xterm-256color".into(),
            size: PtySize::default(),
            env: Vec::new(),
            agent_forwarding: false,
            x11: false,
            startup_command: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passphrase_ids_are_stable_and_path_safe() {
        let a = KeyRef::File(PathBuf::from(r"C:\Users\Me\.ssh\id_ed25519"));
        let b = KeyRef::File(PathBuf::from(r"c:\users\me\.ssh\ID_ED25519"));
        assert_eq!(a.passphrase_id(), b.passphrase_id(), "case-insensitive on Windows paths");
        assert!(a.passphrase_id().chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    }

    #[test]
    fn hops_end_with_target() {
        let mut target = ConnectSpec::new("inner", 22, "u");
        target.jump = vec![ConnectSpec::new("bastion", 22, "j")];
        let hops = target.hops();
        assert_eq!(hops.len(), 2);
        assert_eq!(hops[0].host, "bastion");
        assert_eq!(hops[1].host, "inner");
    }
}
