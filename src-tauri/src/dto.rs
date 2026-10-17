//! Types that cross the IPC boundary but don't belong to the core/ssh crates. Every type
//! here is exported to TypeScript (`ui/src/lib/bindings`).

use aether_core::model::{Host, Settings};
use aether_core::paths::StorageMode;
use aether_core::{ConnectionId, ErrorInfo, HostId, PromptId, TerminalId};
use aether_ssh::{
    FileVersion, HostKeyDecision, HostKeyRequest, LineEnding, PromptRequest, SessionState, TerminalState, TransferInfo,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use zeroize::Zeroizing;

/// Saving a text file from the SFTP editor.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WriteTextRequest {
    pub path: String,
    /// The edited text, with `\n` line ends as a text box holds it.
    pub text: String,
    /// What the file used when it was opened; put back on save.
    pub line_ending: LineEnding,
    pub bom: bool,
    /// The file's version when it was opened. If the file on the server no longer matches, saving is refused with a
    /// `conflict` error instead of overwriting somebody else's change. `null` saves regardless.
    pub expected: Option<FileVersion>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TermSize {
    pub cols: u32,
    pub rows: u32,
    #[serde(default)]
    pub pixel_width: u32,
    #[serde(default)]
    pub pixel_height: u32,
}

impl From<TermSize> for aether_ssh::PtySize {
    fn from(s: TermSize) -> Self {
        aether_ssh::PtySize {
            cols: s.cols.clamp(2, 2000),
            rows: s.rows.clamp(2, 1000),
            pixel_width: s.pixel_width,
            pixel_height: s.pixel_height,
        }
    }
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OpenSessionRequest {
    pub host_id: HostId,
    pub size: TermSize,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct QuickConnectRequest {
    /// `[user@]host[:port]`, `ssh://user@host:port`, or an alias from `~/.ssh/config`.
    pub target: String,
    pub size: TermSize,
}

/// What a window learns when it starts showing a terminal.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AttachedTerminal {
    pub state: TerminalState,
    /// How many bytes of what the terminal printed before were sent first, on the output channel, to catch the window
    /// up. The window counts them off: replayed history must not ring the bell, send notifications, copy to the
    /// clipboard or answer the queries it contains a second time.
    #[ts(type = "number")]
    pub replay_bytes: usize,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OpenedSession {
    pub connection_id: ConnectionId,
    pub terminal_id: TerminalId,
    pub host: Host,
    /// Whether `host` is a saved host (false for quick-connect targets).
    pub saved: bool,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub debug: bool,
    pub storage_mode: StorageMode,
    pub config_dir: String,
    pub logs_dir: String,
    pub os: String,
    pub vault_backend: String,
    /// Problems found at startup (a damaged data file that was recovered, ...).
    pub warnings: Vec<ErrorInfo>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FontInfo {
    pub family: String,
    pub monospace: bool,
}

// ── events ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SessionStateEvent {
    pub connection_id: ConnectionId,
    pub state: SessionState,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ForwardsEvent {
    pub connection_id: ConnectionId,
    pub forwards: Vec<aether_ssh::ForwardStatus>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LatencyEvent {
    pub connection_id: ConnectionId,
    pub rtt_ms: u32,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BannerEvent {
    pub connection_id: ConnectionId,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(tag = "type", rename_all = "kebab-case", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum PromptPayload {
    HostKey { request: HostKeyRequest },
    Secret { request: PromptRequest },
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PromptEvent {
    pub id: PromptId,
    pub connection_id: ConnectionId,
    pub payload: PromptPayload,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PromptClosedEvent {
    pub id: PromptId,
}

/// The user's answer to a [`PromptEvent`].
/// What was typed into a password or code box. Wiped from memory when dropped, and `Debug` shows nothing — a reply
/// that ends up in a log line or a panic message must not carry the secret with it.
#[derive(Clone)]
pub struct Typed(Zeroizing<String>);

impl Typed {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Typed {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(|s| Typed(Zeroizing::new(s)))
    }
}

impl std::fmt::Debug for Typed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Typed(<{} characters>)", self.0.chars().count())
    }
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(tag = "type", rename_all = "kebab-case", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum PromptReply {
    HostKey {
        decision: HostKeyDecision,
    },
    Answers {
        #[ts(type = "string[]")]
        answers: Vec<Typed>,
        remember: bool,
    },
    Cancel,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TransferEvent {
    pub transfer: TransferInfo,
}

/// A fresh listing of hosts after any change (so every window stays in sync).
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SettingsEvent {
    pub settings: Settings,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TrayConnectEvent {
    pub host_id: HostId,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ArgsEvent {
    /// Command-line arguments of a second launch (`aether user@host`).
    pub args: Vec<String>,
}

// ── ssh config import ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportCandidateDto {
    pub alias: String,
    pub hostname: String,
    pub port: u16,
    pub user: Option<String>,
    pub identity_file: Option<String>,
    pub via: Vec<String>,
    pub uses_proxy_command: bool,
    pub forward_count: u32,
    pub already_imported: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SshConfigPreview {
    pub found: bool,
    pub path: Option<String>,
    pub candidates: Vec<ImportCandidateDto>,
    pub warnings: Vec<String>,
}

// ── keys ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(tag = "type", rename_all = "kebab-case", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum KeyImportSource {
    File { path: String },
    Text { pem: String },
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct KeyImportRequest {
    pub label: String,
    pub source: KeyImportSource,
    /// Needed for encrypted keys whose public half can't be read otherwise.
    pub passphrase: Option<String>,
    pub save_passphrase: bool,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct KeyGenerateRequest {
    pub label: String,
    pub kind: aether_ssh::KeyKind,
    pub comment: String,
    pub passphrase: Option<String>,
    pub save_passphrase: bool,
}

/// What the import dialog learns by looking at a key before importing it.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(tag = "type", rename_all = "kebab-case", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum KeyInspectResult {
    Ready { meta: aether_ssh::KeyMeta },
    NeedsPassphrase,
}

// ── sftp ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RemoteListing {
    /// The canonical path that was listed (resolves `.`, `~`, symlinks).
    pub path: String,
    pub entries: Vec<aether_ssh::RemoteEntry>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlanSummary {
    pub plan_id: String,
    pub file_count: u32,
    #[ts(type = "number")]
    pub total_bytes: u64,
    pub conflicts: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum LocalEntryKind {
    File,
    Dir,
    Other,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LocalEntry {
    pub name: String,
    pub path: String,
    pub kind: LocalEntryKind,
    pub is_symlink: bool,
    pub hidden: bool,
    #[ts(type = "number")]
    pub size: u64,
    #[ts(type = "number | null")]
    pub modified: Option<i64>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LocalListing {
    pub path: String,
    pub parent: Option<String>,
    pub entries: Vec<LocalEntry>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum LocalRootKind {
    Drive,
    Home,
    Desktop,
    Documents,
    Downloads,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LocalRoot {
    pub path: String,
    pub label: String,
    pub kind: LocalRootKind,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn term_sizes_are_clamped_to_sane_values() {
        let huge: aether_ssh::PtySize = TermSize { cols: 100_000, rows: 0, pixel_width: 0, pixel_height: 0 }.into();
        assert_eq!((huge.cols, huge.rows), (2000, 2));
    }

    #[test]
    fn prompt_replies_deserialize_from_the_ui_shape() {
        let r: PromptReply = serde_json::from_str(r#"{"type":"answers","answers":["pw"],"remember":true}"#).unwrap();
        assert!(matches!(r, PromptReply::Answers { remember: true, .. }));
        // …and printing one (a log line, a panic message) shows how much was typed, never what.
        let printed = format!("{r:?}");
        assert!(!printed.contains("pw") && printed.contains("2 characters"), "{printed}");
        let r: PromptReply = serde_json::from_str(r#"{"type":"host-key","decision":"trust-and-save"}"#).unwrap();
        assert!(matches!(r, PromptReply::HostKey { decision: HostKeyDecision::TrustAndSave }));
        let r: PromptReply = serde_json::from_str(r#"{"type":"cancel"}"#).unwrap();
        assert!(matches!(r, PromptReply::Cancel));
    }
}
