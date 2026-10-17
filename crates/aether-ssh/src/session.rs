//! Long-lived sessions.
//!
//! A [`Session`] owns one logical server connection and keeps it alive: when the network
//! drops it reconnects with exponential backoff, re-opens every terminal's shell and
//! restores port forwards. A [`Terminal`] is one shell in one pane. Terminals live in the
//! backend, not in a window, so a UI can detach and re-attach (tear-off tabs, webview
//! reloads) and replay what it missed from a bounded ring buffer.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;

use aether_core::error::{ErrorInfo, UserFacing};
use aether_core::known_hosts::{KnownHosts, KnownKey, KnownSource};
use aether_core::model::ForwardRule;
use aether_core::{ConnectionId, TerminalId};
use aether_vault::Secret;
use async_trait::async_trait;
use bytes::Bytes;
use dashmap::DashMap;
use parking_lot::Mutex;
use serde::Serialize;
use tokio::runtime::Handle;
use tokio::sync::{Notify, mpsc, watch};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use ts_rs::TS;

use crate::connection::{ConnectContext, ConnectionInfo, SshConnection};
use crate::delegate::*;
use crate::error::{Result, SshError};
use crate::forward::ForwardStatus;
use crate::handler::DisconnectKind;
use crate::sftp::SftpClient;
use crate::shell::{ShellEvent, ShellHandle};
use crate::spec::{ConnectSpec, PtySize, ShellParams};

/// Bytes of recent output kept so a window can re-attach and catch up.
const RING_CAPACITY: usize = 2 * 1024 * 1024;
/// Unacknowledged bytes allowed in flight to the UI before the network read pauses.
const FLOW_HIGH_WATER: usize = 1024 * 1024;

// ── events ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, TS)]
#[serde(tag = "type", rename_all = "kebab-case", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum SessionState {
    Connecting {
        stage: Option<ConnectStage>,
        hop: Option<HopInfo>,
    },
    Connected {
        info: ConnectionInfo,
    },
    Reconnecting {
        attempt: u32,
        /// 0 means "trying right now".
        max_attempts: u32,
        retry_in_ms: u64,
        reason: String,
    },
    /// The connection is gone and not coming back by itself.
    Disconnected {
        reason: String,
        error: Option<ErrorInfo>,
    },
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "type", rename_all = "kebab-case", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum TerminalState {
    /// Waiting for the connection.
    Pending,
    Running,
    /// The connection dropped; the shell will be reopened after reconnecting.
    Suspended,
    Exited {
        status: Option<u32>,
        signal: Option<String>,
    },
}

/// Receives session-level events (implemented by the app to push them to the UI).
pub trait SessionEvents: Send + Sync + 'static {
    fn session_state(&self, id: ConnectionId, state: &SessionState);
    fn forwards_changed(&self, id: ConnectionId, forwards: &[ForwardStatus]);
    /// Round-trip time in milliseconds, sampled every few seconds while connected.
    fn latency(&self, id: ConnectionId, rtt_ms: u32);
}

/// Receives a terminal's output and state changes. Attached by whichever window shows it.
pub trait TerminalSink: Send + Sync + 'static {
    fn output(&self, data: Bytes);
    fn state(&self, state: &TerminalState);
}

// ── ring buffer & flow control ───────────────────────────────────────────────

#[derive(Default)]
struct Ring {
    chunks: VecDeque<Bytes>,
    len: usize,
}

impl Ring {
    fn push(&mut self, data: Bytes) {
        self.len += data.len();
        self.chunks.push_back(data);
        while self.len > RING_CAPACITY {
            match self.chunks.pop_front() {
                Some(old) => self.len -= old.len(),
                None => break,
            }
        }
    }

    fn snapshot(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.len);
        for c in &self.chunks {
            out.extend_from_slice(c);
        }
        out
    }
}

#[derive(Default)]
struct Flow {
    inflight: AtomicUsize,
    notify: Notify,
}

impl Flow {
    fn ack(&self, bytes: usize) {
        // A compare-exchange loop rather than `fetch_update`/`try_update`: the former is deprecated on new
        // toolchains and the latter doesn't exist on the oldest one we support.
        let mut current = self.inflight.load(Ordering::SeqCst);
        while let Err(seen) = self.inflight.compare_exchange_weak(
            current,
            current.saturating_sub(bytes),
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            current = seen;
        }
        self.notify.notify_waiters();
    }

    fn reset(&self) {
        self.inflight.store(0, Ordering::SeqCst);
        self.notify.notify_waiters();
    }
}

// ── terminal ─────────────────────────────────────────────────────────────────

struct TermCore {
    ring: Ring,
    sink: Option<Arc<dyn TerminalSink>>,
    state: TerminalState,
}

pub struct Terminal {
    pub id: TerminalId,
    params: Mutex<ShellParams>,
    core: Mutex<TermCore>,
    shell: Mutex<Option<ShellHandle>>,
    pump_cancel: Mutex<CancellationToken>,
    flow: Flow,
    closed: AtomicBool,
    /// Set while a shell is being opened, so concurrent callers don't open two.
    starting: AtomicBool,
}

/// Clears a flag when dropped.
struct ResetOnDrop<'a>(&'a AtomicBool);

impl Drop for ResetOnDrop<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl Terminal {
    fn new(params: ShellParams) -> Arc<Self> {
        Arc::new(Self {
            id: TerminalId::new(),
            params: Mutex::new(params),
            core: Mutex::new(TermCore { ring: Ring::default(), sink: None, state: TerminalState::Pending }),
            shell: Mutex::new(None),
            pump_cancel: Mutex::new(CancellationToken::new()),
            flow: Flow::default(),
            closed: AtomicBool::new(false),
            starting: AtomicBool::new(false),
        })
    }

    pub fn state(&self) -> TerminalState {
        self.core.lock().state.clone()
    }

    pub fn size(&self) -> PtySize {
        self.params.lock().size
    }

    /// Shows this terminal in a window. Returns what happened so far (to replay) and the
    /// current state. Any previous sink is replaced.
    pub fn attach(&self, sink: Arc<dyn TerminalSink>) -> (Vec<u8>, TerminalState) {
        let mut core = self.core.lock();
        core.sink = Some(sink);
        self.flow.reset();
        (core.ring.snapshot(), core.state.clone())
    }

    /// Stops delivering output to a window (output keeps accumulating in the ring).
    pub fn detach(&self) {
        self.core.lock().sink = None;
        self.flow.reset();
    }

    /// The UI reports that it has finished rendering `bytes` bytes of output.
    pub fn ack(&self, bytes: usize) {
        self.flow.ack(bytes);
    }

    /// Sends input to the shell. Input typed while the terminal is not running (connecting,
    /// reconnecting, exited) is rejected rather than queued, so stale keystrokes never
    /// surprise the user after a reconnect.
    pub async fn write(&self, data: impl Into<Bytes>) -> Result<()> {
        if self.state() != TerminalState::Running {
            return Err(SshError::Disconnected("the terminal is not connected".into()));
        }
        let shell = self.shell.lock().clone();
        match shell {
            Some(s) => s.write(data).await,
            None => Err(SshError::Disconnected("the terminal is not connected".into())),
        }
    }

    pub async fn resize(&self, size: PtySize) -> Result<()> {
        self.params.lock().size = size;
        let shell = self.shell.lock().clone();
        match shell {
            Some(s) => s.resize(size).await,
            None => Ok(()),
        }
    }

    /// Closes the remote shell and ends the terminal.
    pub async fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.pump_cancel.lock().cancel();
        let shell = self.shell.lock().take();
        if let Some(s) = shell {
            s.close().await;
        }
        self.set_state(TerminalState::Exited { status: None, signal: None });
        self.core.lock().sink = None;
        self.flow.reset();
    }

    /// Writes text to the terminal as if the remote side had sent it (status notices).
    pub fn notice(&self, text: &str) {
        let data = Bytes::from(text.to_string().into_bytes());
        let sink = {
            let mut core = self.core.lock();
            core.ring.push(data.clone());
            core.sink.clone()
        };
        if let Some(sink) = sink {
            sink.output(data);
        }
    }

    fn set_state(&self, state: TerminalState) {
        let sink = {
            let mut core = self.core.lock();
            if core.state == state {
                return;
            }
            core.state = state.clone();
            core.sink.clone()
        };
        if let Some(sink) = sink {
            sink.state(&state);
        }
    }

    /// Opens (or reopens) the shell on `conn` and starts pumping its output.
    async fn start_shell(self: &Arc<Self>, conn: SshConnection) -> Result<()> {
        if self.closed.load(Ordering::SeqCst) || self.state() == TerminalState::Running {
            return Ok(());
        }
        // Only one caller may open the shell; a concurrent attempt is simply redundant.
        if self.starting.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let _reset = ResetOnDrop(&self.starting);
        let params = self.params.lock().clone();
        if params.x11 {
            // Say so up front if there's nothing to draw on: programs would just fail later with a cryptic error.
            if let Some(display) = conn.x11_display()
                && let Err(why) = crate::x11::probe(&display).await
            {
                self.notice(&format!("\r\n\x1b[33m[Aether] X11 forwarding is on, but {why}\x1b[0m\r\n"));
            }
        }
        let (handle, rx) = conn.open_shell(&params).await?;

        // A newer shell replaces any older pump.
        let cancel = CancellationToken::new();
        // The tab may have been closed while the server was setting the shell up. `closed` is checked with the shell
        // slot held, and `close()` takes the slot after setting it, so either the shell is installed in time for
        // `close()` to find it, or it is shut here — it never outlives its terminal.
        let unwanted = {
            let mut shell = self.shell.lock();
            if self.closed.load(Ordering::SeqCst) {
                Some(handle)
            } else {
                let mut slot = self.pump_cancel.lock();
                slot.cancel();
                *slot = cancel.clone();
                *shell = Some(handle);
                None
            }
        };
        if let Some(handle) = unwanted {
            handle.close().await;
            return Ok(());
        }
        self.set_state(TerminalState::Running);
        tokio::spawn(self.clone().pump(rx, conn, cancel));
        Ok(())
    }

    /// A shell that could not be opened. If it was the connection itself that went away, the terminal waits for the
    /// reconnect and gets its shell then; otherwise it ends with the reason, and Enter restarts it.
    async fn open_failed(&self, conn: &SshConnection, error: &SshError) {
        warn!(error = %error, "could not open a shell");
        self.notice(&format!("\r\n\x1b[31m{}\x1b[0m\r\n", error.info().message));
        let connection_died =
            error.is_transient() && tokio::time::timeout(Duration::from_millis(800), conn.closed()).await.is_ok();
        if connection_died {
            self.set_state(TerminalState::Suspended);
        } else {
            self.set_state(TerminalState::Exited { status: None, signal: None });
        }
    }

    async fn deliver(&self, data: Bytes, cancel: &CancellationToken) {
        let sink = {
            let mut core = self.core.lock();
            core.ring.push(data.clone());
            core.sink.clone()
        };
        let Some(sink) = sink else { return };

        // Backpressure: a UI that is behind pauses the network read.
        while self.flow.inflight.load(Ordering::SeqCst) >= FLOW_HIGH_WATER {
            let wake = self.flow.notify.notified();
            tokio::pin!(wake);
            // Register interest before re-checking so an ack can't slip between the two.
            wake.as_mut().enable();
            if self.flow.inflight.load(Ordering::SeqCst) < FLOW_HIGH_WATER {
                break;
            }
            tokio::select! {
                _ = &mut wake => {}
                _ = cancel.cancelled() => return,
            }
            // The window may have detached or been replaced while we waited; the data is in
            // the ring for whoever attaches next.
            let still_current = self.core.lock().sink.as_ref().is_some_and(|s| Arc::ptr_eq(s, &sink));
            if !still_current {
                return;
            }
        }
        self.flow.inflight.fetch_add(data.len(), Ordering::SeqCst);
        sink.output(data);
    }

    async fn pump(self: Arc<Self>, mut rx: mpsc::Receiver<ShellEvent>, conn: SshConnection, cancel: CancellationToken) {
        let mut exit: Option<(Option<u32>, Option<String>)> = None;
        loop {
            let event = tokio::select! {
                _ = cancel.cancelled() => return,
                e = rx.recv() => e,
            };
            match event {
                Some(ShellEvent::Data(data)) => self.deliver(data, &cancel).await,
                Some(ShellEvent::Exit { status, signal }) => exit = Some((status, signal)),
                Some(ShellEvent::Closed) | None => break,
            }
        }
        if cancel.is_cancelled() || self.closed.load(Ordering::SeqCst) {
            return;
        }
        // A shell that ended without an exit status usually means the connection died.
        // Give the disconnect notification a moment to arrive before deciding.
        let lost = exit.is_none() && tokio::time::timeout(Duration::from_millis(800), conn.closed()).await.is_ok();
        if lost {
            self.set_state(TerminalState::Suspended);
        } else {
            let (status, signal) = exit.unwrap_or((None, None));
            *self.shell.lock() = None;
            self.set_state(TerminalState::Exited { status, signal });
        }
    }
}

// ── session ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SessionOptions {
    pub auto_reconnect: bool,
    /// 0 = keep trying until the user stops.
    pub reconnect_max_attempts: u32,
    pub forwards: Vec<ForwardRule>,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self { auto_reconnect: true, reconnect_max_attempts: 12, forwards: Vec::new() }
    }
}

/// Delay before reconnect attempt `attempt` (1-based): 1 s, 2 s, 4 s, 8 s, 15 s, 30 s, then 30 s.
pub fn backoff(attempt: u32) -> Duration {
    const STEPS: [u64; 6] = [1, 2, 4, 8, 15, 30];
    let idx = (attempt.saturating_sub(1) as usize).min(STEPS.len() - 1);
    Duration::from_secs(STEPS[idx])
}

/// Spreads simultaneous reconnects (a server reboot affects every tab) over time.
fn jittered(base: Duration, seed: u64) -> Duration {
    let spread = (base.as_millis() as u64 / 5).max(1);
    base + Duration::from_millis(seed.wrapping_mul(2_654_435_761) % spread)
}

enum Cmd {
    ReconnectNow,
    Close,
}

/// Bookkeeping while automatically reconnecting.
struct Retry {
    /// Attempts made so far in this run of retries.
    attempt: u32,
    reason: String,
}

/// What a connection attempt led to.
enum Step {
    Stop,
    Connected(SshConnection),
    Retry(Retry),
    /// Gave up or hit a problem retrying can't fix; the state already says why.
    Failed,
}

#[derive(Hash, PartialEq, Eq)]
enum CacheKind {
    Password,
    Passphrase(String),
}

/// A host key the user accepted during this session.
struct Pin {
    algorithm: String,
    fingerprint: String,
}

/// Remembers secrets typed during this session so reconnects don't ask again, holds on to the host keys accepted
/// "just this once" so a reconnect can tell the same server from a different one, and turns connection progress into
/// session state.
struct SessionDelegate {
    inner: Arc<dyn ConnectionDelegate>,
    cache: Mutex<HashMap<(u32, CacheKind), Secret>>,
    /// Keys accepted for this session only, by hop. Nothing was written to known_hosts for them, so without this a
    /// reconnect would either ask the same question again or — worse — take a *different* key for a first visit.
    pinned: Mutex<HashMap<u32, Pin>>,
    replay: AtomicBool,
    session: OnceLock<Weak<Session>>,
}

impl SessionDelegate {
    /// Drops what was typed for a hop: after its host key changed, those secrets must not follow the connection to
    /// the new key without the person typing them again.
    fn forget_typed(&self, hop: u32) {
        self.cache.lock().retain(|(h, _), _| *h != hop);
    }
}

#[async_trait]
impl ConnectionDelegate for SessionDelegate {
    async fn verify_host_key(&self, mut request: HostKeyRequest) -> HostKeyDecision {
        let hop = request.hop.index;
        let first_visit = matches!(request.kind, HostKeyRequestKind::Unknown);
        // True when the question was rewritten from "new server" to "changed", so a "replace" answer can be mapped
        // back to what it can mean here: there is nothing saved to replace.
        let mut rewritten = false;

        if first_visit {
            let accepted = self.pinned.lock().get(&hop).map(|p| (p.algorithm.clone(), p.fingerprint.clone()));
            if let Some((algorithm, fingerprint)) = accepted {
                if fingerprint == request.fingerprint {
                    // The server the person already vouched for, back after a drop.
                    return HostKeyDecision::TrustOnce;
                }
                // Someone vouched for a different key a moment ago: this is a change, whatever the saved list says.
                request.kind = HostKeyRequestKind::Changed {
                    known: vec![KnownKey { algorithm, fingerprint, source: KnownSource::Aether, line: 0 }],
                };
                rewritten = true;
            }
        }
        if !first_visit || rewritten {
            self.forget_typed(hop);
        }

        let (algorithm, fingerprint) = (request.algorithm.clone(), request.fingerprint.clone());
        let decision = self.inner.verify_host_key(request).await;
        // "Replace the saved key" while nothing was saved is just "this key, for now": don't write it down.
        let decision = match decision {
            HostKeyDecision::ReplaceAndTrust if rewritten => HostKeyDecision::TrustOnce,
            other => other,
        };
        // Only "just this once" needs remembering here. A key that is saved is found in the saved list next time; and one
        // that was meant to be saved but couldn't be must be asked about (and saved) again, not waved through from here.
        if matches!(decision, HostKeyDecision::TrustOnce) {
            self.pinned.lock().insert(hop, Pin { algorithm, fingerprint });
        }
        decision
    }

    async fn prompt(&self, request: PromptRequest) -> Option<PromptResponse> {
        let key = match &request.kind {
            PromptKind::Password => Some((request.hop.index, CacheKind::Password)),
            PromptKind::KeyPassphrase { key } => Some((request.hop.index, CacheKind::Passphrase(key.clone()))),
            // One-time codes must never be replayed.
            PromptKind::KeyboardInteractive { .. } => None,
        };
        if self.replay.load(Ordering::SeqCst)
            && request.attempt == 0
            && let Some(k) = &key
            && let Some(secret) = self.cache.lock().get(k)
        {
            return Some(PromptResponse { answers: vec![secret.clone()], remember: false });
        }
        let response = self.inner.prompt(request).await?;
        if let (Some(k), Some(answer)) = (key, response.answers.first()) {
            self.cache.lock().insert(k, answer.clone());
        }
        Some(response)
    }

    fn saved_secret(&self, lookup: &SecretLookup) -> Option<Secret> {
        self.inner.saved_secret(lookup)
    }
    fn remember_secret(&self, lookup: &SecretLookup, secret: &Secret) {
        self.inner.remember_secret(lookup, secret)
    }
    fn forget_secret(&self, lookup: &SecretLookup) {
        self.inner.forget_secret(lookup)
    }
    fn progress(&self, progress: ConnectProgress) {
        if let Some(session) = self.session.get().and_then(Weak::upgrade) {
            session.on_progress(&progress);
        }
        self.inner.progress(progress)
    }
    fn banner(&self, hop: &HopInfo, text: &str) {
        self.inner.banner(hop, text)
    }
    fn vault_key(&self, key_id: &str) -> Option<Secret> {
        self.inner.vault_key(key_id)
    }
}

pub struct Session {
    pub id: ConnectionId,
    spec: Arc<ConnectSpec>,
    options: SessionOptions,
    events: Arc<dyn SessionEvents>,
    known_hosts: Arc<KnownHosts>,
    delegate: Arc<SessionDelegate>,
    state: watch::Sender<SessionState>,
    conn: Mutex<Option<SshConnection>>,
    terminals: Mutex<Vec<Arc<Terminal>>>,
    sftp: tokio::sync::Mutex<Option<SftpClient>>,
    /// Forwards that should be running; restored after every reconnect.
    wanted_forwards: Mutex<HashMap<String, ForwardRule>>,
    commands: mpsc::UnboundedSender<Cmd>,
    cancel: CancellationToken,
    /// The runtime this session lives on. Background work is spawned here so a session can be
    /// driven from any thread (a UI thread has no ambient Tokio context).
    runtime: Handle,
}

impl Session {
    pub fn state(&self) -> SessionState {
        self.state.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<SessionState> {
        self.state.subscribe()
    }

    pub fn label(&self) -> &str {
        &self.spec.label
    }

    pub fn target(&self) -> String {
        self.spec.target()
    }

    /// The live connection, if currently connected.
    pub fn connection(&self) -> Option<SshConnection> {
        self.conn.lock().clone().filter(|c| !c.is_closed())
    }

    fn set_state(&self, state: SessionState) {
        self.events.session_state(self.id, &state);
        self.state.send_replace(state);
    }

    /// Reflects connection progress ("Authenticating…") while an initial connect is running.
    fn on_progress(&self, progress: &ConnectProgress) {
        if matches!(self.state(), SessionState::Connecting { .. }) {
            self.set_state(SessionState::Connecting {
                stage: Some(progress.stage.clone()),
                hop: Some(progress.hop.clone()),
            });
        }
    }

    pub fn terminals(&self) -> Vec<Arc<Terminal>> {
        self.terminals.lock().clone()
    }

    pub fn terminal(&self, id: TerminalId) -> Option<Arc<Terminal>> {
        self.terminals.lock().iter().find(|t| t.id == id).cloned()
    }

    /// Adds a terminal. Its shell starts as soon as (or if already) connected.
    pub fn new_terminal(self: &Arc<Self>, params: ShellParams) -> Arc<Terminal> {
        let terminal = Terminal::new(params);
        self.terminals.lock().push(terminal.clone());
        if let Some(conn) = self.connection() {
            let t = terminal.clone();
            let session = self.clone();
            self.runtime.spawn(async move {
                if let Err(e) = t.start_shell(conn.clone()).await {
                    t.open_failed(&conn, &e).await;
                }
                drop(session);
            });
        }
        terminal
    }

    /// Reopens the shell of a terminal that exited (user pressed Enter on "session ended").
    pub async fn restart_terminal(self: &Arc<Self>, id: TerminalId) -> Result<()> {
        let t = self.terminal(id).ok_or_else(|| SshError::Invalid("unknown terminal".into()))?;
        let conn = self.connection().ok_or_else(|| SshError::Disconnected("not connected".into()))?;
        t.start_shell(conn).await
    }

    pub async fn close_terminal(&self, id: TerminalId) {
        let t = {
            let mut list = self.terminals.lock();
            list.iter().position(|t| t.id == id).map(|i| list.remove(i))
        };
        if let Some(t) = t {
            t.close().await;
        }
    }

    /// Asks the supervisor to try again immediately (user pressed "Reconnect").
    pub fn reconnect(&self) {
        let _ = self.commands.send(Cmd::ReconnectNow);
    }

    /// The SFTP client for this session, opened on first use.
    pub async fn sftp(&self) -> Result<SftpClient> {
        let mut slot = self.sftp.lock().await;
        // A client whose channel died or hung is not handed out again: the connection can be fine while its SFTP
        // channel is not (the server restarted its sftp-server, or dropped an idle subsystem).
        if let Some(client) = slot.as_ref().filter(|c| !c.is_broken()) {
            return Ok(client.clone());
        }
        if let Some(stale) = slot.take() {
            // Let go of it without waiting on a channel that may be hung.
            self.runtime.spawn(async move {
                let _ = tokio::time::timeout(Duration::from_secs(3), stale.close()).await;
            });
        }
        let conn = self.connection().ok_or_else(|| SshError::Disconnected("not connected".into()))?;
        let client = conn.open_sftp().await?;
        *slot = Some(client.clone());
        Ok(client)
    }

    pub async fn start_forward(&self, rule: ForwardRule) -> Result<ForwardStatus> {
        let conn = self.connection().ok_or_else(|| SshError::Disconnected("not connected".into()))?;
        let status = conn.forwards().start(rule.clone()).await?;
        self.wanted_forwards.lock().insert(rule.id.clone(), rule);
        self.events.forwards_changed(self.id, &conn.forwards().statuses());
        Ok(status)
    }

    pub async fn stop_forward(&self, id: &str) {
        self.wanted_forwards.lock().remove(id);
        if let Some(conn) = self.connection() {
            conn.forwards().stop(id).await;
            self.events.forwards_changed(self.id, &conn.forwards().statuses());
        }
    }

    pub fn forward_statuses(&self) -> Vec<ForwardStatus> {
        self.connection().map(|c| c.forwards().statuses()).unwrap_or_default()
    }

    /// Ends the session: closes every terminal and the connection.
    pub async fn close(&self) {
        let _ = self.commands.send(Cmd::Close);
        self.cancel.cancel();
        let terminals: Vec<_> = self.terminals.lock().drain(..).collect();
        for t in terminals {
            t.close().await;
        }
        let conn = self.conn.lock().take();
        if let Some(conn) = conn {
            conn.close().await;
        }
        self.set_state(SessionState::Closed);
    }

    fn ctx(&self) -> ConnectContext {
        ConnectContext {
            delegate: self.delegate.clone(),
            known_hosts: self.known_hosts.clone(),
            cancel: self.cancel.clone(),
        }
    }

    // ── supervisor ──────────────────────────────────────────────────────────

    async fn supervise(self: Arc<Self>, mut commands: mpsc::UnboundedReceiver<Cmd>) {
        let mut ever_connected = false;
        // `Some` while we are automatically reconnecting after a loss.
        let mut retry: Option<Retry> = None;

        'main: loop {
            match self.attempt(&mut commands, ever_connected, retry.take()).await {
                Step::Stop => break 'main,
                Step::Retry(next) => {
                    retry = Some(next);
                    continue 'main;
                }
                Step::Failed => {}
                Step::Connected(conn) => {
                    self.on_connected(conn.clone(), !ever_connected).await;
                    ever_connected = true;

                    // Stay connected until the connection ends or the session is closed.
                    let lost = loop {
                        tokio::select! {
                            info = conn.closed() => break Some(info),
                            _ = self.cancel.cancelled() => break None,
                            cmd = commands.recv() => match cmd {
                                Some(Cmd::ReconnectNow) => continue, // already connected
                                Some(Cmd::Close) | None => break None,
                            },
                        }
                    };
                    let Some(lost) = lost else { break 'main };
                    if lost.kind == DisconnectKind::Requested {
                        break 'main;
                    }
                    info!(target = %self.spec.target(), reason = %lost.message, "connection lost");
                    self.on_lost(&lost.message);
                    if self.options.auto_reconnect {
                        retry = Some(Retry { attempt: 0, reason: lost.message });
                        continue 'main;
                    }
                    self.set_state(SessionState::Disconnected { reason: lost.message, error: None });
                }
            }

            // Disconnected for good (for now): wait for the user to retry or close.
            match commands.recv().await {
                Some(Cmd::ReconnectNow) => continue 'main,
                Some(Cmd::Close) | None => break 'main,
            }
        }
        debug!(target = %self.spec.target(), "supervisor finished");
    }

    /// One connection attempt, preceded by a backoff delay when auto-reconnecting.
    async fn attempt(
        &self,
        commands: &mut mpsc::UnboundedReceiver<Cmd>,
        ever_connected: bool,
        retry: Option<Retry>,
    ) -> Step {
        let mut attempt_no = 0;
        if let Some(r) = &retry {
            attempt_no = r.attempt + 1;
            let max = self.options.reconnect_max_attempts;
            if max > 0 && attempt_no > max {
                self.set_state(SessionState::Disconnected {
                    reason: format!("Gave up after {max} attempts. {}", r.reason),
                    error: None,
                });
                return Step::Failed;
            }
            let delay = jittered(backoff(attempt_no), u64::from(attempt_no) ^ self.id.0.as_u128() as u64);
            self.set_state(SessionState::Reconnecting {
                attempt: attempt_no,
                max_attempts: max,
                retry_in_ms: delay.as_millis() as u64,
                reason: r.reason.clone(),
            });
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = self.cancel.cancelled() => return Step::Stop,
                cmd = commands.recv() => match cmd {
                    Some(Cmd::ReconnectNow) => {}
                    Some(Cmd::Close) | None => return Step::Stop,
                },
            }
            self.set_state(SessionState::Reconnecting {
                attempt: attempt_no,
                max_attempts: max,
                retry_in_ms: 0,
                reason: r.reason.clone(),
            });
        } else {
            self.set_state(SessionState::Connecting { stage: None, hop: None });
        }

        // Replay cached passwords only when re-establishing a connection that worked before.
        self.delegate.replay.store(ever_connected, Ordering::SeqCst);
        match SshConnection::connect(&self.spec, &self.ctx()).await {
            Ok(conn) => Step::Connected(conn),
            Err(SshError::Cancelled) => Step::Stop,
            Err(e) => {
                // Problems that retrying can't fix: stop and let the user act.
                if retry.is_some() && !e.needs_user() {
                    debug!(error = %e, attempt = attempt_no, "reconnect attempt failed");
                    Step::Retry(Retry { attempt: attempt_no, reason: e.to_string() })
                } else {
                    warn!(target = %self.spec.target(), error = %e, "connect failed");
                    self.set_state(SessionState::Disconnected { reason: e.to_string(), error: Some(e.info()) });
                    Step::Failed
                }
            }
        }
    }

    async fn on_connected(self: &Arc<Self>, conn: SshConnection, first_time: bool) {
        *self.conn.lock() = Some(conn.clone());
        *self.sftp.lock().await = None;
        self.set_state(SessionState::Connected { info: conn.info().clone() });

        // Reopen every shell that was waiting or whose connection dropped.
        for t in self.terminals() {
            let needs_shell = matches!(t.state(), TerminalState::Pending | TerminalState::Suspended);
            if !needs_shell {
                continue;
            }
            if !first_time {
                t.notice("\r\n\x1b[2;32m── reconnected ──\x1b[0m\r\n");
            }
            if let Err(e) = t.start_shell(conn.clone()).await {
                t.open_failed(&conn, &e).await;
            }
        }

        // Forwards: configured auto-start rules on first connect, previously running ones after.
        let mut rules: Vec<ForwardRule> = if first_time {
            let mut wanted = self.wanted_forwards.lock();
            for r in self.options.forwards.iter().filter(|r| r.auto_start) {
                wanted.insert(r.id.clone(), r.clone());
            }
            wanted.values().cloned().collect()
        } else {
            self.wanted_forwards.lock().values().cloned().collect()
        };
        rules.sort_by(|a, b| a.id.cmp(&b.id));
        for rule in rules {
            if let Err(e) = conn.forwards().start(rule.clone()).await {
                warn!(rule = %rule.describe(), error = %e, "forward could not start");
            }
        }
        self.events.forwards_changed(self.id, &conn.forwards().statuses());

        // Latency monitor for the status bar.
        let events = self.events.clone();
        let id = self.id;
        let monitor_conn = conn.clone();
        let cancel = self.cancel.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(5));
            loop {
                tokio::select! {
                    _ = tick.tick() => {}
                    _ = cancel.cancelled() => return,
                    _ = monitor_conn.closed() => return,
                }
                if let Ok(rtt) = monitor_conn.ping().await {
                    events.latency(id, rtt.as_millis().min(u128::from(u32::MAX)) as u32);
                }
            }
        });

        // Traffic counters for the forwards panel: pushed once a second, and only when something moved.
        let events = self.events.clone();
        let stats_conn = conn.clone();
        let cancel = self.cancel.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut last = 0u64;
            loop {
                tokio::select! {
                    _ = tick.tick() => {}
                    _ = cancel.cancelled() => return,
                    _ = stats_conn.closed() => return,
                }
                let statuses = stats_conn.forwards().statuses();
                // A cheap fingerprint of every counter; unchanged means nothing to tell the UI.
                let print = statuses.iter().fold(0u64, |acc, s| {
                    acc.rotate_left(5)
                        ^ s.total_connections
                        ^ s.bytes_up.rotate_left(13)
                        ^ s.bytes_down.rotate_left(27)
                        ^ u64::from(s.active_connections).rotate_left(41)
                        ^ s.refused.rotate_left(51)
                });
                if print != last {
                    last = print;
                    events.forwards_changed(id, &statuses);
                }
            }
        });
    }

    fn on_lost(&self, reason: &str) {
        *self.conn.lock() = None;
        for t in self.terminals() {
            // The old channel is dead; drop its handle so nothing writes into it.
            *t.shell.lock() = None;
            if matches!(t.state(), TerminalState::Running | TerminalState::Pending) {
                t.set_state(TerminalState::Suspended);
            }
            t.notice(&format!("\r\n\x1b[2;31m── connection lost: {reason} ──\x1b[0m\r\n"));
        }
    }
}

// ── manager ──────────────────────────────────────────────────────────────────

/// Summary row for listing sessions.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SessionSummary {
    pub id: ConnectionId,
    pub label: String,
    pub target: String,
    pub state: SessionState,
    pub terminals: Vec<TerminalId>,
}

pub struct SessionManager {
    sessions: DashMap<ConnectionId, Arc<Session>>,
    known_hosts: Arc<KnownHosts>,
    events: Arc<dyn SessionEvents>,
}

impl SessionManager {
    pub fn new(known_hosts: Arc<KnownHosts>, events: Arc<dyn SessionEvents>) -> Arc<Self> {
        Arc::new(Self { sessions: DashMap::new(), known_hosts, events })
    }

    /// Starts a session and begins connecting in the background.
    pub fn open(
        &self,
        id: ConnectionId,
        spec: ConnectSpec,
        delegate: Arc<dyn ConnectionDelegate>,
        options: SessionOptions,
    ) -> Arc<Session> {
        // Opening a session starts its supervisor, so it must be called on the runtime.
        let runtime = Handle::current();
        let (commands, rx) = mpsc::unbounded_channel();
        let (state_tx, _) = watch::channel(SessionState::Connecting { stage: None, hop: None });
        let session_delegate = Arc::new(SessionDelegate {
            inner: delegate,
            cache: Mutex::new(HashMap::new()),
            pinned: Mutex::new(HashMap::new()),
            replay: AtomicBool::new(false),
            session: OnceLock::new(),
        });
        let session = Arc::new(Session {
            id,
            spec: Arc::new(spec),
            options,
            events: self.events.clone(),
            known_hosts: self.known_hosts.clone(),
            delegate: session_delegate.clone(),
            state: state_tx,
            conn: Mutex::new(None),
            terminals: Mutex::new(Vec::new()),
            sftp: tokio::sync::Mutex::new(None),
            wanted_forwards: Mutex::new(HashMap::new()),
            commands,
            cancel: CancellationToken::new(),
            runtime: runtime.clone(),
        });

        // Surface connection progress ("Authenticating…") as session state.
        let _ = session_delegate.session.set(Arc::downgrade(&session));

        self.sessions.insert(id, session.clone());
        runtime.spawn(session.clone().supervise(rx));
        session
    }

    pub fn get(&self, id: ConnectionId) -> Option<Arc<Session>> {
        self.sessions.get(&id).map(|s| s.clone())
    }

    pub fn terminal(&self, id: TerminalId) -> Option<(Arc<Session>, Arc<Terminal>)> {
        self.sessions.iter().find_map(|s| s.terminal(id).map(|t| (s.clone(), t)))
    }

    pub async fn close(&self, id: ConnectionId) {
        if let Some((_, session)) = self.sessions.remove(&id) {
            session.close().await;
        }
    }

    pub async fn close_all(&self) {
        let ids: Vec<ConnectionId> = self.sessions.iter().map(|s| s.id).collect();
        for id in ids {
            self.close(id).await;
        }
    }

    pub fn list(&self) -> Vec<SessionSummary> {
        self.sessions
            .iter()
            .map(|s| SessionSummary {
                id: s.id,
                label: s.label().to_string(),
                target: s.target(),
                state: s.state(),
                terminals: s.terminals().iter().map(|t| t.id).collect(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_then_plateaus() {
        let secs: Vec<u64> = (1..=9).map(|a| backoff(a).as_secs()).collect();
        assert_eq!(secs, vec![1, 2, 4, 8, 15, 30, 30, 30, 30]);
        assert_eq!(backoff(0).as_secs(), 1);
    }

    #[test]
    fn jitter_stays_within_twenty_percent() {
        for seed in 0..200 {
            let d = jittered(Duration::from_secs(10), seed);
            assert!(d >= Duration::from_secs(10) && d < Duration::from_millis(12_000), "{d:?}");
        }
    }

    #[test]
    fn ring_keeps_only_the_most_recent_output() {
        let mut ring = Ring::default();
        let chunk = Bytes::from(vec![b'a'; 512 * 1024]);
        for _ in 0..6 {
            ring.push(chunk.clone());
        }
        ring.push(Bytes::from_static(b"tail"));
        assert!(ring.len <= RING_CAPACITY + chunk.len());
        let snap = ring.snapshot();
        assert!(snap.ends_with(b"tail"));
        assert!(snap.len() <= RING_CAPACITY);
    }

    fn host_key_question(fingerprint: &str) -> HostKeyRequest {
        HostKeyRequest {
            hop: crate::delegate::HopInfo {
                index: 0,
                total: 1,
                label: "t".into(),
                host: "example.com".into(),
                port: 22,
                username: "u".into(),
            },
            algorithm: "ssh-ed25519".into(),
            fingerprint: fingerprint.into(),
            bits: Some(256),
            randomart: String::new(),
            kind: HostKeyRequestKind::Unknown,
        }
    }

    /// A key accepted "just this once" is remembered for the session, so a dropped connection doesn't ask again. A key
    /// that was meant to be *saved* is not: if saving failed, the next attempt has to ask (and try to save) again rather
    /// than be waved through on a pin; if it worked, the saved list answers.
    #[tokio::test]
    async fn only_a_key_accepted_for_just_this_once_is_pinned_to_the_session() {
        use crate::testing::delegate::ScriptedDelegate;
        for (decision, pinned) in [
            (HostKeyDecision::TrustOnce, true),
            (HostKeyDecision::TrustAndSave, false),
            (HostKeyDecision::ReplaceAndTrust, false),
            (HostKeyDecision::Reject, false),
        ] {
            let scripted = ScriptedDelegate { host_key: decision, ..Default::default() };
            let delegate = SessionDelegate {
                inner: Arc::new(scripted.clone()),
                cache: Mutex::new(HashMap::new()),
                pinned: Mutex::new(HashMap::new()),
                replay: AtomicBool::new(false),
                session: OnceLock::new(),
            };
            delegate.verify_host_key(host_key_question("SHA256:aaa")).await;
            let again = delegate.verify_host_key(host_key_question("SHA256:aaa")).await;
            let asked = scripted.host_key_prompts.load(Ordering::SeqCst);
            assert_eq!(asked, if pinned { 1 } else { 2 }, "{decision:?} was asked {asked} time(s)");
            if pinned {
                assert_eq!(again, HostKeyDecision::TrustOnce, "the same key again is waved through, once");
            }
        }
    }

    #[test]
    fn flow_ack_never_underflows() {
        let flow = Flow::default();
        flow.inflight.store(10, Ordering::SeqCst);
        flow.ack(100);
        assert_eq!(flow.inflight.load(Ordering::SeqCst), 0);
    }
}
