//! Sessions, terminals, port forwards and prompt answers.

use std::sync::Arc;

use aether_core::error::UserFacing;
use aether_core::model::{ForwardRule, Host};
use aether_core::{ConnectionId, ErrorInfo, PromptId, TerminalId};
use aether_ssh::{ForwardStatus, PtySize, SessionOptions, SessionState, SessionSummary, TerminalSink, TerminalState};
use base64::Engine as _;
use bytes::Bytes;
use parking_lot::Mutex;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, WebviewWindow};
use tracing::{debug, info};

use super::St;
use crate::connect::{host_for_target, parse_target};
use crate::delegate::AppDelegate;
use crate::dto::{AttachedTerminal, OpenSessionRequest, OpenedSession, PromptReply, QuickConnectRequest, TermSize};
use crate::state::{AppState, Res, not_found};

// ── terminal output channel ──────────────────────────────────────────────────

enum Queued {
    Data(Bytes),
    State(TerminalState),
}

struct Gate {
    open: bool,
    queue: Vec<Queued>,
}

/// Forwards terminal output to the webview over a Tauri channel. Output produced while the
/// replay is still being sent is queued, so the UI always sees replay → live, in order.
struct ChannelSink {
    data: Channel<InvokeResponseBody>,
    state: Channel<TerminalState>,
    gate: Mutex<Gate>,
}

impl ChannelSink {
    fn new(data: Channel<InvokeResponseBody>, state: Channel<TerminalState>) -> Arc<Self> {
        Arc::new(Self { data, state, gate: Mutex::new(Gate { open: false, queue: Vec::new() }) })
    }

    fn deliver(&self, item: Queued) {
        match item {
            Queued::Data(bytes) => {
                let _ = self.data.send(InvokeResponseBody::Raw(bytes.to_vec()));
            }
            Queued::State(s) => {
                let _ = self.state.send(s);
            }
        }
    }

    /// Releases anything queued during replay and switches to direct delivery.
    fn open(&self) {
        let mut gate = self.gate.lock();
        for item in std::mem::take(&mut gate.queue) {
            self.deliver(item);
        }
        gate.open = true;
    }
}

impl TerminalSink for ChannelSink {
    fn output(&self, data: Bytes) {
        let mut gate = self.gate.lock();
        if gate.open {
            drop(gate);
            self.deliver(Queued::Data(data));
        } else {
            gate.queue.push(Queued::Data(data));
        }
    }

    fn state(&self, state: &TerminalState) {
        let mut gate = self.gate.lock();
        if gate.open {
            drop(gate);
            self.deliver(Queued::State(state.clone()));
        } else {
            gate.queue.push(Queued::State(state.clone()));
        }
    }
}

// ── opening ──────────────────────────────────────────────────────────────────

async fn open_for_host(
    app: &AppHandle,
    state: &Arc<AppState>,
    window: &WebviewWindow,
    host: Host,
    saved: bool,
    size: TermSize,
) -> Res<OpenedSession> {
    let settings = state.settings.get();
    let resolver = state.resolver(&settings);
    let spec = resolver.spec_for(&host, saved)?;
    let size: PtySize = size.into();
    let shell = resolver.shell_params(&host, size);
    let options = SessionOptions {
        auto_reconnect: host.options.auto_reconnect,
        reconnect_max_attempts: settings.ssh.reconnect_max_attempts,
        forwards: host.forwards.clone(),
    };

    let id = ConnectionId::new();
    state.prompts.set_owner(id, window.label());
    let delegate = Arc::new(AppDelegate {
        app: app.clone(),
        connection_id: id,
        prompts: state.prompts.clone(),
        vault: state.vault.clone(),
    });

    info!(target = %spec.target(), via = spec.jump.len(), "opening session");
    let session = state.sessions.open(id, spec, delegate, options);
    let terminal = session.new_terminal(shell);

    // Remember successful connections (for "recent" lists and sorting).
    let state2 = state.clone();
    let host2 = host.clone();
    let mut rx = session.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            let done = {
                let s = rx.borrow_and_update();
                match &*s {
                    SessionState::Connected { .. } => Some(true),
                    SessionState::Closed => Some(false),
                    _ => None,
                }
            };
            match done {
                Some(true) => {
                    if saved {
                        let _ = state2.hosts.touch_connected(host2.id);
                    } else {
                        let _ = state2.recents.record(&crate::connect::recent_line(&host2));
                    }
                    return;
                }
                Some(false) => return,
                None => {
                    if rx.changed().await.is_err() {
                        return;
                    }
                }
            }
        }
    });

    Ok(OpenedSession { connection_id: id, terminal_id: terminal.id, host, saved })
}

#[tauri::command]
pub async fn session_open(
    app: AppHandle,
    state: St<'_>,
    window: WebviewWindow,
    request: OpenSessionRequest,
) -> Res<OpenedSession> {
    let host = state.hosts.get(request.host_id).ok_or_else(|| not_found("This host"))?;
    open_for_host(&app, state.inner(), &window, host, true, request.size).await
}

/// Connects to something typed into the palette without saving it first.
#[tauri::command]
pub async fn session_quick_connect(
    app: AppHandle,
    state: St<'_>,
    window: WebviewWindow,
    request: QuickConnectRequest,
) -> Res<OpenedSession> {
    let target = parse_target(&request.target)
        .ok_or_else(|| ErrorInfo::invalid("address", "Enter a host like user@example.com or user@10.0.0.5:2222."))?;
    let settings = state.settings.get();
    let host = host_for_target(&target, &settings, &state.hosts);
    let saved = state.hosts.get(host.id).is_some();
    open_for_host(&app, state.inner(), &window, host, saved, request.size).await
}

/// A window takes over prompts for an existing connection (tab tear-off).
#[tauri::command]
pub fn session_adopt(state: St<'_>, window: WebviewWindow, connection_id: ConnectionId) -> Res<()> {
    state.session(connection_id)?;
    state.prompts.set_owner(connection_id, window.label());
    Ok(())
}

#[tauri::command]
pub async fn session_close(state: St<'_>, connection_id: ConnectionId) -> Res<()> {
    state.prompts.clear_connection(connection_id);
    state.plans.retain(|_, p| p.connection != connection_id);
    state.sessions.close(connection_id).await;
    Ok(())
}

#[tauri::command]
pub fn session_reconnect(state: St<'_>, connection_id: ConnectionId) -> Res<()> {
    state.session(connection_id)?.reconnect();
    Ok(())
}

#[tauri::command]
pub fn sessions_list(state: St<'_>) -> Vec<SessionSummary> {
    state.sessions.list()
}

#[tauri::command]
pub fn session_state(state: St<'_>, connection_id: ConnectionId) -> Res<SessionState> {
    Ok(state.session(connection_id)?.state())
}

// ── terminals ────────────────────────────────────────────────────────────────

/// Adds another shell to an existing connection (a split pane): no second login.
#[tauri::command]
pub fn terminal_new(state: St<'_>, connection_id: ConnectionId, size: TermSize, host: Host) -> Res<TerminalId> {
    let session = state.session(connection_id)?;
    let settings = state.settings.get();
    let params = state.resolver(&settings).shell_params(&host, size.into());
    Ok(session.new_terminal(params).id)
}

fn terminal(state: &AppState, id: TerminalId) -> Res<Arc<aether_ssh::Terminal>> {
    state.sessions.terminal(id).map(|(_, t)| t).ok_or_else(|| not_found("This terminal"))
}

/// Shows a terminal in the calling window. Output (starting with a replay of recent
/// history) arrives on `on_data` as raw bytes; state changes on `on_state`.
#[tauri::command]
pub async fn terminal_attach(
    state: St<'_>,
    terminal_id: TerminalId,
    on_data: Channel<InvokeResponseBody>,
    on_state: Channel<TerminalState>,
) -> Res<AttachedTerminal> {
    let term = terminal(&state, terminal_id)?;
    let sink = ChannelSink::new(on_data.clone(), on_state);
    let (replay, current) = term.attach(sink.clone());
    // Keep IPC messages modest in size.
    for chunk in replay.chunks(64 * 1024) {
        let _ = on_data.send(InvokeResponseBody::Raw(chunk.to_vec()));
    }
    sink.open();
    debug!(%terminal_id, replay = replay.len(), "terminal attached");
    Ok(AttachedTerminal { state: current, replay_bytes: replay.len() })
}

#[tauri::command]
pub fn terminal_detach(state: St<'_>, terminal_id: TerminalId) {
    if let Ok(t) = terminal(&state, terminal_id) {
        t.detach();
    }
}

#[tauri::command]
pub async fn terminal_write(state: St<'_>, terminal_id: TerminalId, data: String) -> Res<()> {
    terminal(&state, terminal_id)?.write(data.into_bytes()).await.map_err(|e| e.info())
}

/// For binary-safe input (mouse reports in legacy encodings), base64 encoded.
#[tauri::command]
pub async fn terminal_write_binary(state: St<'_>, terminal_id: TerminalId, data: String) -> Res<()> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|_| ErrorInfo::invalid("data", "not valid base64"))?;
    terminal(&state, terminal_id)?.write(bytes).await.map_err(|e| e.info())
}

#[tauri::command]
pub async fn terminal_resize(state: St<'_>, terminal_id: TerminalId, size: TermSize) -> Res<()> {
    terminal(&state, terminal_id)?.resize(size.into()).await.map_err(|e| e.info())
}

/// The UI finished rendering `bytes` of output; lets the backend keep sending.
#[tauri::command]
pub fn terminal_ack(state: St<'_>, terminal_id: TerminalId, bytes: usize) {
    if let Ok(t) = terminal(&state, terminal_id) {
        t.ack(bytes);
    }
}

#[tauri::command]
pub async fn terminal_close(state: St<'_>, connection_id: ConnectionId, terminal_id: TerminalId) -> Res<()> {
    state.session(connection_id)?.close_terminal(terminal_id).await;
    Ok(())
}

#[tauri::command]
pub async fn terminal_restart(state: St<'_>, connection_id: ConnectionId, terminal_id: TerminalId) -> Res<()> {
    state.session(connection_id)?.restart_terminal(terminal_id).await.map_err(|e| e.info())
}

// ── port forwards ────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn forward_start(state: St<'_>, connection_id: ConnectionId, rule: ForwardRule) -> Res<ForwardStatus> {
    state.session(connection_id)?.start_forward(rule).await.map_err(|e| e.info())
}

#[tauri::command]
pub async fn forward_stop(state: St<'_>, connection_id: ConnectionId, rule_id: String) -> Res<()> {
    state.session(connection_id)?.stop_forward(&rule_id).await;
    Ok(())
}

#[tauri::command]
pub fn forwards_list(state: St<'_>, connection_id: ConnectionId) -> Res<Vec<ForwardStatus>> {
    Ok(state.session(connection_id)?.forward_statuses())
}

// ── prompts ──────────────────────────────────────────────────────────────────

#[tauri::command]
pub fn prompt_respond(state: St<'_>, prompt_id: PromptId, reply: PromptReply) -> bool {
    state.prompts.respond(prompt_id, reply)
}
