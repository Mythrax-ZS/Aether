//! Session supervision: reconnects, replay, flow control and failure handling.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use aether_core::ConnectionId;
use aether_core::known_hosts::KnownHosts;
use aether_core::model::{AuthMethodKind, ForwardKind, ForwardRule};
use bytes::Bytes;
use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Notify, watch};

use crate::forward::ForwardStatus;
use crate::testing::delegate::ScriptedDelegate;
use crate::testing::{TestServer, TestServerOptions};
use crate::*;

// ── a network we can break ───────────────────────────────────────────────────

/// TCP relay in front of a server that can sever connections or silently swallow traffic,
/// the way Wi-Fi drops and NAT timeouts behave.
struct FlakyNetwork {
    addr: SocketAddr,
    target: Arc<Mutex<SocketAddr>>,
    sever: watch::Sender<u64>,
    blackhole: Arc<AtomicBool>,
}

impl FlakyNetwork {
    async fn start(target: SocketAddr) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let target = Arc::new(Mutex::new(target));
        let (sever, _) = watch::channel(0u64);
        let blackhole = Arc::new(AtomicBool::new(false));
        let (t, s, b) = (target.clone(), sever.clone(), blackhole.clone());
        tokio::spawn(async move {
            while let Ok((client, _)) = listener.accept().await {
                let dest = *t.lock();
                let mut gen_rx = s.subscribe();
                let hole = b.clone();
                tokio::spawn(async move {
                    let Ok(upstream) = TcpStream::connect(dest).await else { return };
                    let (cr, cw) = client.into_split();
                    let (ur, uw) = upstream.into_split();
                    tokio::select! {
                        _ = gen_rx.changed() => {}
                        _ = pipe(cr, uw, hole.clone()) => {}
                        _ = pipe(ur, cw, hole) => {}
                    }
                });
            }
        });
        Self { addr, target, sever, blackhole }
    }

    fn port(&self) -> u16 {
        self.addr.port()
    }

    /// Abruptly closes every open connection (new ones are still accepted).
    fn drop_connections(&self) {
        self.sever.send_modify(|g| *g += 1);
    }

    fn set_blackhole(&self, on: bool) {
        self.blackhole.store(on, Ordering::SeqCst);
    }

    fn retarget(&self, addr: SocketAddr) {
        *self.target.lock() = addr;
    }
}

async fn pipe(mut r: impl AsyncReadExt + Unpin, mut w: impl AsyncWriteExt + Unpin, hole: Arc<AtomicBool>) {
    let mut buf = [0u8; 16 * 1024];
    while let Ok(n) = r.read(&mut buf).await {
        if n == 0 {
            break;
        }
        if hole.load(Ordering::SeqCst) {
            continue; // swallowed
        }
        if w.write_all(&buf[..n]).await.is_err() {
            break;
        }
    }
}

// ── collectors ───────────────────────────────────────────────────────────────

#[derive(Default)]
struct Events {
    states: Mutex<Vec<SessionState>>,
    forwards: Mutex<Vec<Vec<ForwardStatus>>>,
    notify: Notify,
}

impl SessionEvents for Events {
    fn session_state(&self, _id: ConnectionId, state: &SessionState) {
        self.states.lock().push(state.clone());
        self.notify.notify_waiters();
    }
    fn forwards_changed(&self, _id: ConnectionId, forwards: &[ForwardStatus]) {
        self.forwards.lock().push(forwards.to_vec());
    }
    fn latency(&self, _id: ConnectionId, _rtt_ms: u32) {}
}

impl Events {
    async fn wait_state(&self, what: &str, pred: impl Fn(&SessionState) -> bool) -> SessionState {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let notified = self.notify.notified();
            if let Some(s) = self.states.lock().iter().rev().find(|s| pred(s)).cloned() {
                return s;
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                panic!("timed out waiting for {what}; states: {:?}", self.states.lock());
            }
        }
    }

    async fn wait_connected_after(&self, seen_before: usize) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let notified = self.notify.notified();
            if self.states.lock().iter().skip(seen_before).any(|s| matches!(s, SessionState::Connected { .. })) {
                return;
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                panic!("never reconnected; states: {:?}", self.states.lock());
            }
        }
    }
}

#[derive(Default)]
struct Screen {
    output: Mutex<Vec<u8>>,
    states: Mutex<Vec<TerminalState>>,
    chunks: AtomicUsize,
    notify: Notify,
}

impl TerminalSink for Screen {
    fn output(&self, data: Bytes) {
        self.output.lock().extend_from_slice(&data);
        self.chunks.fetch_add(1, Ordering::SeqCst);
        self.notify.notify_waiters();
    }
    fn state(&self, state: &TerminalState) {
        self.states.lock().push(state.clone());
        self.notify.notify_waiters();
    }
}

impl Screen {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.output.lock()).into_owned()
    }

    async fn wait_text(&self, needle: &str) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let notified = self.notify.notified();
            if self.text().contains(needle) {
                return;
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                panic!("timed out waiting for {needle:?}; screen: {:?}", self.text());
            }
        }
    }
}

struct Harness {
    manager: Arc<SessionManager>,
    events: Arc<Events>,
    delegate: ScriptedDelegate,
    _dir: tempfile::TempDir,
}

fn harness(answers: &[&str]) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let events = Arc::new(Events::default());
    let manager = SessionManager::new(Arc::new(KnownHosts::new(dir.path().join("kh"), None)), events.clone());
    Harness { manager, events, delegate: ScriptedDelegate::answering(answers), _dir: dir }
}

fn spec(port: u16) -> ConnectSpec {
    let mut s = ConnectSpec::new("127.0.0.1", port, "tester");
    s.auth.methods = vec![AuthMethodKind::Password];
    s.auth.default_identity_files = Vec::new();
    s.options.connect_timeout = Duration::from_secs(5);
    s
}

// ── tests ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_session_connects_and_its_terminal_comes_alive() {
    let server = TestServer::start(TestServerOptions::default()).await;
    let h = harness(&["secret"]);
    let session = h.manager.open(
        ConnectionId::new(),
        spec(server.port()),
        Arc::new(h.delegate.clone()),
        SessionOptions::default(),
    );
    let term = session.new_terminal(ShellParams::default());
    let screen = Arc::new(Screen::default());
    let (replay, state) = term.attach(screen.clone());
    assert!(replay.is_empty());
    assert_eq!(state, TerminalState::Pending);

    let connected = h.events.wait_state("connected", |s| matches!(s, SessionState::Connected { .. })).await;
    assert!(matches!(connected, SessionState::Connected { .. }));
    screen.wait_text("$ ").await;
    assert_eq!(term.state(), TerminalState::Running);

    term.write("echo alive\r").await.unwrap();
    screen.wait_text("alive\r\n$ ").await;

    // The progress states were surfaced on the way.
    let states = h.events.states.lock().clone();
    assert!(
        states
            .iter()
            .any(|s| matches!(s, SessionState::Connecting { stage: Some(ConnectStage::Authenticating { .. }), .. }))
    );

    h.manager.close(session.id).await;
    assert!(matches!(session.state(), SessionState::Closed));
    assert!(h.manager.list().is_empty());
}

/// Regression: a split pane adds a terminal from the UI thread, which has no ambient Tokio
/// context. That used to panic ("there is no reactor running") and take the whole app down.
#[tokio::test]
async fn a_terminal_can_be_added_from_a_thread_without_a_runtime() {
    let server = TestServer::start(TestServerOptions::default()).await;
    let h = harness(&["secret"]);
    let session = h.manager.open(
        ConnectionId::new(),
        spec(server.port()),
        Arc::new(h.delegate.clone()),
        SessionOptions::default(),
    );
    let first = session.new_terminal(ShellParams::default());
    let screen = Arc::new(Screen::default());
    first.attach(screen.clone());
    screen.wait_text("$ ").await;

    let s = session.clone();
    let split = std::thread::spawn(move || s.new_terminal(ShellParams::default()))
        .join()
        .expect("new_terminal must not panic off-runtime");

    let second = Arc::new(Screen::default());
    split.attach(second.clone());
    second.wait_text("$ ").await;
    split.write("echo split-works\r").await.unwrap();
    second.wait_text("split-works\r\n$ ").await;
    h.manager.close(session.id).await;
}

#[tokio::test]
async fn a_reattached_window_catches_up_from_the_replay_buffer() {
    let server = TestServer::start(TestServerOptions::default()).await;
    let h = harness(&["secret"]);
    let session = h.manager.open(
        ConnectionId::new(),
        spec(server.port()),
        Arc::new(h.delegate.clone()),
        SessionOptions::default(),
    );
    let term = session.new_terminal(ShellParams::default());
    let first = Arc::new(Screen::default());
    term.attach(first.clone());
    first.wait_text("$ ").await;
    term.write("echo remember-me\r").await.unwrap();
    first.wait_text("remember-me\r\n$ ").await;

    // The window goes away (e.g. a tab is torn off); output continues to accumulate.
    term.detach();
    term.write("echo while-away\r").await.unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;

    let second = Arc::new(Screen::default());
    let (replay, state) = term.attach(second.clone());
    let replayed = String::from_utf8_lossy(&replay);
    assert!(replayed.contains("remember-me") && replayed.contains("while-away"), "{replayed:?}");
    assert_eq!(state, TerminalState::Running);

    // And live output flows to the new window only.
    term.write("echo live\r").await.unwrap();
    second.wait_text("live\r\n$ ").await;
    assert!(!first.text().contains("while-away"));
}

#[tokio::test]
async fn a_slow_window_applies_backpressure_to_the_network() {
    let server = TestServer::start(TestServerOptions::default()).await;
    let h = harness(&["secret"]);
    let session = h.manager.open(
        ConnectionId::new(),
        spec(server.port()),
        Arc::new(h.delegate.clone()),
        SessionOptions::default(),
    );
    let term = session.new_terminal(ShellParams::default());
    let screen = Arc::new(Screen::default());
    term.attach(screen.clone());
    screen.wait_text("$ ").await;

    term.write("big 6000000\r").await.unwrap();
    // The window never acknowledges: delivery must stall near the high-water mark.
    tokio::time::sleep(Duration::from_secs(1)).await;
    let stalled_at = screen.output.lock().len();
    assert!(stalled_at < 2_200_000, "pump kept sending past the watermark ({stalled_at} bytes)");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(screen.output.lock().len(), stalled_at, "and it stays paused");

    // Acknowledging lets the rest through, intact.
    let mut acked = 0usize;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let now = screen.output.lock().len();
        if now > acked {
            term.ack(now - acked);
            acked = now;
        }
        if screen.text().matches('x').count() >= 6_000_000 {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "stalled at {acked} bytes");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn the_connection_is_restored_after_the_network_drops() {
    let server = TestServer::start(TestServerOptions::default()).await;
    let net = FlakyNetwork::start(server.addr).await;
    let h = harness(&["secret"]);
    let session =
        h.manager.open(ConnectionId::new(), spec(net.port()), Arc::new(h.delegate.clone()), SessionOptions::default());
    let term = session.new_terminal(ShellParams::default());
    let screen = Arc::new(Screen::default());
    term.attach(screen.clone());
    h.events.wait_state("connected", |s| matches!(s, SessionState::Connected { .. })).await;
    screen.wait_text("$ ").await;
    term.write("echo before\r").await.unwrap();
    screen.wait_text("before\r\n$ ").await;

    let seen = h.events.states.lock().len();
    net.drop_connections();

    let reconnecting = h.events.wait_state("reconnecting", |s| matches!(s, SessionState::Reconnecting { .. })).await;
    match reconnecting {
        SessionState::Reconnecting { attempt, .. } => assert_eq!(attempt, 1),
        _ => unreachable!(),
    }
    h.events.wait_connected_after(seen).await;

    // The shell was reopened in the same terminal, and the user was told.
    screen.wait_text("reconnected").await;
    assert!(screen.text().contains("connection lost"));
    // Input is refused until the new shell is up (and nothing is queued behind the user's back).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while term.state() != TerminalState::Running {
        assert!(tokio::time::Instant::now() < deadline, "shell never reopened");
        assert!(term.write("x").await.is_err());
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    term.write("echo after\r").await.unwrap();
    screen.wait_text("after\r\n$ ").await;

    // The password was typed once; the reconnect reused it silently.
    assert_eq!(h.delegate.prompts.lock().len(), 1);
    assert_eq!(server.recorded().connections, 2);
    assert!(term.state() == TerminalState::Running);
}

/// "Connect once" writes nothing down — but within the session the same server must not be asked about again every
/// time the network blips.
#[tokio::test]
async fn a_key_accepted_once_is_remembered_for_the_rest_of_the_session() {
    let server = TestServer::start(TestServerOptions::default()).await;
    let net = FlakyNetwork::start(server.addr).await;
    let mut h = harness(&["secret"]);
    h.delegate.host_key = HostKeyDecision::TrustOnce;
    let session =
        h.manager.open(ConnectionId::new(), spec(net.port()), Arc::new(h.delegate.clone()), SessionOptions::default());
    h.events.wait_state("connected", |s| matches!(s, SessionState::Connected { .. })).await;
    assert_eq!(h.delegate.host_key_prompts.load(Ordering::SeqCst), 1);

    let seen = h.events.states.lock().len();
    net.drop_connections();
    h.events.wait_connected_after(seen).await;
    assert_eq!(h.delegate.host_key_prompts.load(Ordering::SeqCst), 1, "no second question for the same key");
    assert_eq!(h.delegate.prompts.lock().len(), 1, "and the password was reused");
    assert!(!h._dir.path().join("kh").exists(), "'once' wrote nothing down");
    h.manager.close(session.id).await;
}

/// The reconnect lands on a different server (same address, other key). Nothing is on record for the host, which
/// would make it look like a first visit — but a person vouched for a *different* key a moment ago, so it is a change,
/// and what was typed for the old one must not follow the connection to the new one.
#[tokio::test]
async fn a_different_key_after_connect_once_is_a_change_and_the_password_is_asked_for_again() {
    let a = TestServer::start(TestServerOptions::default()).await;
    let b = TestServer::start(TestServerOptions::default()).await;
    let net = FlakyNetwork::start(a.addr).await;
    let mut h = harness(&["secret", "secret"]);
    h.delegate.host_key = HostKeyDecision::TrustOnce;
    h.delegate.host_key_on_change = Some(HostKeyDecision::ReplaceAndTrust);
    let session =
        h.manager.open(ConnectionId::new(), spec(net.port()), Arc::new(h.delegate.clone()), SessionOptions::default());
    h.events.wait_state("connected", |s| matches!(s, SessionState::Connected { .. })).await;
    assert_eq!(h.delegate.prompts.lock().len(), 1);

    let seen = h.events.states.lock().len();
    net.retarget(b.addr);
    net.drop_connections();
    h.events.wait_connected_after(seen).await;

    let asked = h.delegate.host_key_requests.lock().clone();
    assert_eq!(asked.len(), 2, "{asked:?}");
    assert_eq!(asked[0].kind, HostKeyRequestKind::Unknown);
    match &asked[1].kind {
        HostKeyRequestKind::Changed { known } => {
            assert_eq!(known.len(), 1);
            assert_eq!(known[0].fingerprint, asked[0].fingerprint, "compared with what the person accepted");
        }
        other => panic!("expected a change, got {other:?}"),
    }
    assert_ne!(asked[1].fingerprint, asked[0].fingerprint);
    assert_eq!(h.delegate.prompts.lock().len(), 2, "the password was typed again for the new key");
    assert!(!h._dir.path().join("kh").exists(), "replacing what was never saved means 'this once', not 'save'");
    h.manager.close(session.id).await;
}

#[tokio::test]
async fn refusing_a_different_key_keeps_the_password_away_from_it() {
    let a = TestServer::start(TestServerOptions::default()).await;
    let b = TestServer::start(TestServerOptions::default()).await;
    let net = FlakyNetwork::start(a.addr).await;
    let mut h = harness(&["secret", "secret"]);
    h.delegate.host_key = HostKeyDecision::TrustOnce;
    h.delegate.host_key_on_change = Some(HostKeyDecision::Reject);
    let session =
        h.manager.open(ConnectionId::new(), spec(net.port()), Arc::new(h.delegate.clone()), SessionOptions::default());
    h.events.wait_state("connected", |s| matches!(s, SessionState::Connected { .. })).await;

    net.retarget(b.addr);
    net.drop_connections();
    let state = h.events.wait_state("refused", |s| matches!(s, SessionState::Disconnected { .. })).await;
    match state {
        SessionState::Disconnected { error: Some(e), .. } => assert!(!e.title.is_empty(), "{e:?}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(h.delegate.prompts.lock().len(), 1, "no second password prompt, let alone a replayed one");
    assert!(b.recorded().auth_attempts.is_empty(), "nothing was ever sent to the new key");
    assert_eq!(b.recorded().connections, 1);

    // It is the person's call now: no automatic retries pile up behind their back.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(b.recorded().connections, 1);
    drop(session);
}

#[tokio::test]
async fn local_port_forwards_come_back_after_a_reconnect() {
    let server = TestServer::start(TestServerOptions::default()).await;
    let net = FlakyNetwork::start(server.addr).await;
    let echo = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let echo_port = echo.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = echo.accept().await {
            tokio::spawn(async move {
                let mut b = [0u8; 256];
                while let Ok(n) = s.read(&mut b).await {
                    if n == 0 || s.write_all(&b[..n]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    let fwd_port = TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap().port();

    let h = harness(&["secret"]);
    let options = SessionOptions {
        forwards: vec![ForwardRule {
            kind: ForwardKind::Local,
            bind_port: fwd_port,
            dest_host: "127.0.0.1".into(),
            dest_port: echo_port,
            auto_start: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let session = h.manager.open(ConnectionId::new(), spec(net.port()), Arc::new(h.delegate.clone()), options);
    h.events.wait_state("connected", |s| matches!(s, SessionState::Connected { .. })).await;

    async fn ping(port: u16) -> bool {
        let Ok(mut s) = TcpStream::connect(("127.0.0.1", port)).await else { return false };
        if s.write_all(b"ping").await.is_err() {
            return false;
        }
        let mut buf = [0u8; 4];
        matches!(tokio::time::timeout(Duration::from_secs(3), s.read_exact(&mut buf)).await, Ok(Ok(_)))
            && &buf == b"ping"
    }
    assert!(ping(fwd_port).await, "auto-started forward works");

    let seen = h.events.states.lock().len();
    net.drop_connections();
    h.events.wait_connected_after(seen).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(ping(fwd_port).await, "forward restored after reconnect");
    assert_eq!(session.forward_statuses().len(), 1);
}

#[tokio::test]
async fn a_silent_network_death_is_caught_by_keepalives() {
    let server = TestServer::start(TestServerOptions::default()).await;
    let net = FlakyNetwork::start(server.addr).await;
    let h = harness(&["secret"]);
    let mut s = spec(net.port());
    s.options.keepalive_interval = Some(Duration::from_millis(500));
    s.options.keepalive_max = 2;
    let ctx = ConnectContext {
        delegate: Arc::new(h.delegate.clone()),
        known_hosts: Arc::new(KnownHosts::new(h._dir.path().join("kh2"), None)),
        cancel: Default::default(),
    };
    let conn = SshConnection::connect(&s, &ctx).await.unwrap();
    net.set_blackhole(true); // packets vanish; no FIN, no RST
    let info = tokio::time::timeout(Duration::from_secs(15), conn.closed())
        .await
        .expect("keepalives should notice within a few seconds");
    assert_eq!(info.kind, DisconnectKind::KeepaliveTimeout, "{info:?}");
}

#[tokio::test]
async fn it_gives_up_after_the_configured_attempts_and_says_so() {
    let server = TestServer::start(TestServerOptions::default()).await;
    let net = FlakyNetwork::start(server.addr).await;
    let h = harness(&["secret"]);
    let options = SessionOptions { reconnect_max_attempts: 2, ..Default::default() };
    let session = h.manager.open(ConnectionId::new(), spec(net.port()), Arc::new(h.delegate.clone()), options);
    h.events.wait_state("connected", |s| matches!(s, SessionState::Connected { .. })).await;

    // Take the server away for good: point the relay at a dead port and cut the line.
    let dead = TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap();
    net.retarget(dead);
    net.drop_connections();

    let state = h.events.wait_state("given up", |s| matches!(s, SessionState::Disconnected { .. })).await;
    match state {
        SessionState::Disconnected { reason, .. } => assert!(reason.contains("Gave up after 2 attempts"), "{reason}"),
        _ => unreachable!(),
    }
    let attempts: Vec<u32> = h
        .events
        .states
        .lock()
        .iter()
        .filter_map(|s| match s {
            SessionState::Reconnecting { attempt, retry_in_ms, .. } if *retry_in_ms > 0 => Some(*attempt),
            _ => None,
        })
        .collect();
    assert_eq!(attempts, vec![1, 2]);

    // The user can still bring it back by hand once the server is reachable again.
    net.retarget(server.addr);
    let seen = h.events.states.lock().len();
    session.reconnect();
    h.events.wait_connected_after(seen).await;
}

#[tokio::test]
async fn a_failed_first_connection_waits_for_the_user_instead_of_retrying() {
    let h = harness(&["secret"]);
    let dead = TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap();
    let net = FlakyNetwork::start(dead).await;
    let session =
        h.manager.open(ConnectionId::new(), spec(net.port()), Arc::new(h.delegate.clone()), SessionOptions::default());

    let state = h.events.wait_state("failure", |s| matches!(s, SessionState::Disconnected { .. })).await;
    match state {
        SessionState::Disconnected { error: Some(e), .. } => {
            assert!(!e.title.is_empty() && e.retryable, "{e:?}");
        }
        other => panic!("{other:?}"),
    }
    assert!(!h.events.states.lock().iter().any(|s| matches!(s, SessionState::Reconnecting { .. })));

    // Fix the "network" and press Reconnect.
    let server = TestServer::start(TestServerOptions::default()).await;
    net.retarget(server.addr);
    let seen = h.events.states.lock().len();
    session.reconnect();
    h.events.wait_connected_after(seen).await;
}

#[tokio::test]
async fn wrong_credentials_are_reported_not_retried() {
    let server = TestServer::start(TestServerOptions::default()).await;
    let h = harness(&["x", "y", "z"]);
    let session = h.manager.open(
        ConnectionId::new(),
        spec(server.port()),
        Arc::new(h.delegate.clone()),
        SessionOptions::default(),
    );
    let state = h.events.wait_state("auth failure", |s| matches!(s, SessionState::Disconnected { .. })).await;
    match state {
        SessionState::Disconnected { error: Some(e), .. } => {
            assert_eq!(e.code, aether_core::ErrorCode::AuthFailed);
        }
        other => panic!("{other:?}"),
    }
    drop(session);
}

#[tokio::test]
async fn exiting_the_shell_ends_the_terminal_but_not_the_connection() {
    let server = TestServer::start(TestServerOptions::default()).await;
    let h = harness(&["secret"]);
    let session = h.manager.open(
        ConnectionId::new(),
        spec(server.port()),
        Arc::new(h.delegate.clone()),
        SessionOptions::default(),
    );
    let term = session.new_terminal(ShellParams::default());
    let screen = Arc::new(Screen::default());
    term.attach(screen.clone());
    screen.wait_text("$ ").await;
    term.write("exit 3\r").await.unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !matches!(term.state(), TerminalState::Exited { .. }) {
        assert!(tokio::time::Instant::now() < deadline, "state: {:?}", term.state());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(term.state(), TerminalState::Exited { status: Some(3), signal: None });
    assert!(matches!(session.state(), SessionState::Connected { .. }), "connection stays up for SFTP etc.");

    // Pressing Enter on the "session ended" screen restarts the shell.
    session.restart_terminal(term.id).await.unwrap();
    assert_eq!(term.state(), TerminalState::Running);
    screen.wait_text("Welcome").await;
}

#[tokio::test]
async fn closing_one_terminal_leaves_the_others_running() {
    let server = TestServer::start(TestServerOptions::default()).await;
    let h = harness(&["secret"]);
    let session = h.manager.open(
        ConnectionId::new(),
        spec(server.port()),
        Arc::new(h.delegate.clone()),
        SessionOptions::default(),
    );
    let a = session.new_terminal(ShellParams::default());
    let b = session.new_terminal(ShellParams::default());
    let (sa, sb) = (Arc::new(Screen::default()), Arc::new(Screen::default()));
    a.attach(sa.clone());
    b.attach(sb.clone());
    sa.wait_text("$ ").await;
    sb.wait_text("$ ").await;
    assert_eq!(server.recorded().connections, 1, "split panes share one connection");

    session.close_terminal(a.id).await;
    assert_eq!(session.terminals().len(), 1);
    b.write("echo still-here\r").await.unwrap();
    sb.wait_text("still-here").await;
}

/// The SSH connection is fine; its SFTP channel is not (the server restarted its sftp-server, an idle subsystem was
/// dropped). The session notices from the failure and opens a new channel instead of handing out the dead one.
#[tokio::test]
async fn an_sftp_channel_that_died_is_replaced_without_reconnecting() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("hello.txt"), "hi").unwrap();
    let server =
        TestServer::start(TestServerOptions { sftp_root: Some(root.path().to_path_buf()), ..Default::default() }).await;
    let h = harness(&["secret"]);
    let session = h.manager.open(
        ConnectionId::new(),
        spec(server.port()),
        Arc::new(h.delegate.clone()),
        SessionOptions::default(),
    );
    h.events.wait_state("connected", |s| matches!(s, SessionState::Connected { .. })).await;

    let first = session.sftp().await.unwrap();
    assert!(!first.is_broken());
    first.close().await; // the subsystem's channel goes away underneath it
    let err = first.list("/").await.unwrap_err();
    assert!(first.is_broken(), "{err:?} should have marked the channel as gone");
    assert!(err.is_transient(), "{err:?}");

    let second = session.sftp().await.expect("a new channel on the same connection");
    assert!(!second.is_broken());
    assert_eq!(second.list("/").await.unwrap()[0].name, "hello.txt");
    assert_eq!(server.recorded().connections, 1, "the SSH connection was never re-made");
    h.manager.close(session.id).await;
}

#[tokio::test]
async fn sftp_is_available_on_a_session_and_reopened_after_reconnects() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("hello.txt"), "hi").unwrap();
    let server =
        TestServer::start(TestServerOptions { sftp_root: Some(root.path().to_path_buf()), ..Default::default() }).await;
    let net = FlakyNetwork::start(server.addr).await;
    let h = harness(&["secret"]);
    let session =
        h.manager.open(ConnectionId::new(), spec(net.port()), Arc::new(h.delegate.clone()), SessionOptions::default());
    h.events.wait_state("connected", |s| matches!(s, SessionState::Connected { .. })).await;

    let sftp = session.sftp().await.unwrap();
    assert_eq!(sftp.list("/").await.unwrap()[0].name, "hello.txt");

    let seen = h.events.states.lock().len();
    net.drop_connections();
    h.events.wait_connected_after(seen).await;
    let sftp = session.sftp().await.expect("a fresh SFTP session on the new connection");
    assert_eq!(sftp.list("/").await.unwrap().len(), 1);
}
