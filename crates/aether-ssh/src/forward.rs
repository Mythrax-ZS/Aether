//! Port forwarding: local (`-L`), remote (`-R`) and dynamic/SOCKS (`-D`).

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use aether_core::model::{ForwardKind, ForwardRule};
use parking_lot::Mutex;
use russh::client::{ChannelOpenHandle, Handle, Msg};
use russh::{Channel, ChannelOpenFailure};
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use ts_rs::TS;

use crate::error::{Result, SshError};
use crate::handler::ClientHandler;
use crate::socks::{self, Reply};

/// Live counters shared between a forward and its connections.
#[derive(Default)]
pub struct ForwardStats {
    pub active: AtomicU32,
    pub total: AtomicU64,
    pub bytes_up: AtomicU64,
    pub bytes_down: AtomicU64,
    /// Connections that never got through (the server said no, or the target was unreachable).
    pub refused: AtomicU64,
    /// Why the most recent one failed, in words a person can act on.
    pub last_error: Mutex<Option<String>>,
}

impl ForwardStats {
    pub(crate) fn note_refusal(&self, why: String) {
        self.refused.fetch_add(1, Ordering::Relaxed);
        *self.last_error.lock() = Some(why);
    }
}

/// Explains a failed `direct-tcpip` open the way a user would want to hear it.
fn explain_open_failure(error: &russh::Error, dest: &str) -> String {
    match error {
        russh::Error::ChannelOpenFailure(ChannelOpenFailure::AdministrativelyProhibited) => {
            "The server doesn't allow port forwarding. Its sshd_config needs AllowTcpForwarding (and a matching PermitOpen).".into()
        }
        russh::Error::ChannelOpenFailure(ChannelOpenFailure::ConnectFailed) => {
            format!("The server couldn't connect to {dest}. Is anything listening there?")
        }
        russh::Error::ChannelOpenFailure(ChannelOpenFailure::ResourceShortage) => {
            "The server has too many open channels right now.".into()
        }
        other => format!("Couldn't open the tunnel to {dest}: {other}"),
    }
}

/// Where connections arriving on a remote-forwarded port should be sent locally.
#[derive(Clone)]
pub(crate) struct RemoteTarget {
    pub host: String,
    pub port: u16,
    pub stats: Arc<ForwardStats>,
}

#[derive(Default)]
pub(crate) struct RemoteForwardTable {
    map: Mutex<HashMap<u16, RemoteTarget>>,
}

impl RemoteForwardTable {
    pub fn insert(&self, port: u16, target: RemoteTarget) {
        self.map.lock().insert(port, target);
    }
    pub fn remove(&self, port: u16) {
        self.map.lock().remove(&port);
    }
    pub fn lookup(&self, port: u16) -> Option<RemoteTarget> {
        self.map.lock().get(&port).cloned()
    }
}

/// Wraps the SSH side of a pipe to count traffic as it flows.
struct Counted<S> {
    inner: S,
    stats: Arc<ForwardStats>,
}

impl<S: AsyncRead + Unpin> AsyncRead for Counted<S> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let poll = Pin::new(&mut self.inner).poll_read(cx, buf);
        if poll.is_ready() {
            let n = buf.filled().len() - before;
            self.stats.bytes_down.fetch_add(n as u64, Ordering::Relaxed);
        }
        poll
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Counted<S> {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        let poll = Pin::new(&mut self.inner).poll_write(cx, buf);
        if let Poll::Ready(Ok(n)) = &poll {
            self.stats.bytes_up.fetch_add(*n as u64, Ordering::Relaxed);
        }
        poll
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

struct ActiveGuard(Arc<ForwardStats>);

impl ActiveGuard {
    fn new(stats: Arc<ForwardStats>) -> Self {
        stats.active.fetch_add(1, Ordering::Relaxed);
        stats.total.fetch_add(1, Ordering::Relaxed);
        Self(stats)
    }
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Shuttles bytes between a local stream and an SSH channel until either side closes.
pub(crate) async fn pump<L>(mut local: L, channel: Channel<Msg>, stats: Arc<ForwardStats>)
where
    L: AsyncRead + AsyncWrite + Unpin + Send,
{
    let _guard = ActiveGuard::new(stats.clone());
    let mut remote = Counted { inner: channel.into_stream(), stats };
    if let Err(e) = tokio::io::copy_bidirectional(&mut local, &mut remote).await {
        // Resets and aborts are routine when a browser tab closes mid-transfer.
        debug!(error = %e, "forwarded connection ended with an error");
    }
}

/// Handles a channel the server opened for a remote forward (`forwarded-tcpip`).
pub(crate) async fn serve_remote(channel: Channel<Msg>, reply: ChannelOpenHandle, target: RemoteTarget) {
    let connect =
        tokio::time::timeout(Duration::from_secs(10), TcpStream::connect((target.host.as_str(), target.port))).await;
    match connect {
        Ok(Ok(socket)) => {
            let _ = socket.set_nodelay(true);
            reply.accept().await;
            pump(socket, channel, target.stats).await;
        }
        other => {
            debug!(host = %target.host, port = target.port, result = ?other.map(|r| r.map(|_| ())), "local side of remote forward unreachable");
            target.stats.note_refusal(format!("Nothing on this PC answered at {}:{}.", target.host, target.port));
            reply.reject(ChannelOpenFailure::ConnectFailed).await;
        }
    }
}

// ── status reporting ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum ForwardState {
    Listening,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ForwardStatus {
    pub id: String,
    pub rule: ForwardRule,
    pub state: ForwardState,
    /// The port actually bound (differs from the rule when a remote forward used port 0).
    pub bound_port: u16,
    pub active_connections: u32,
    #[ts(type = "number")]
    pub total_connections: u64,
    #[ts(type = "number")]
    pub bytes_up: u64,
    #[ts(type = "number")]
    pub bytes_down: u64,
    /// Connections that couldn't be carried (see `last_error`).
    #[ts(type = "number")]
    pub refused: u64,
    /// Why the latest refused connection failed; `None` while everything works.
    pub last_error: Option<String>,
}

struct Active {
    rule: ForwardRule,
    bound_port: u16,
    stats: Arc<ForwardStats>,
    cancel: CancellationToken,
}

impl Active {
    fn status(&self) -> ForwardStatus {
        ForwardStatus {
            id: self.rule.id.clone(),
            rule: self.rule.clone(),
            state: ForwardState::Listening,
            bound_port: self.bound_port,
            active_connections: self.stats.active.load(Ordering::Relaxed),
            total_connections: self.stats.total.load(Ordering::Relaxed),
            bytes_up: self.stats.bytes_up.load(Ordering::Relaxed),
            bytes_down: self.stats.bytes_down.load(Ordering::Relaxed),
            refused: self.stats.refused.load(Ordering::Relaxed),
            last_error: self.stats.last_error.lock().clone(),
        }
    }
}

/// Owns every forward on one SSH connection.
pub struct ForwardManager {
    handle: Arc<Handle<ClientHandler>>,
    table: Arc<RemoteForwardTable>,
    /// Cancelled when the connection goes away; all listeners stop with it.
    conn_cancel: CancellationToken,
    active: Mutex<HashMap<String, Active>>,
}

fn normalise_bind(host: &str) -> &str {
    match host {
        "" | "localhost" => "127.0.0.1",
        "*" => "0.0.0.0",
        other => other.trim_matches(['[', ']']),
    }
}

impl ForwardManager {
    pub(crate) fn new(
        handle: Arc<Handle<ClientHandler>>,
        table: Arc<RemoteForwardTable>,
        conn_cancel: CancellationToken,
    ) -> Self {
        Self { handle, table, conn_cancel, active: Mutex::new(HashMap::new()) }
    }

    pub fn statuses(&self) -> Vec<ForwardStatus> {
        let mut v: Vec<_> = self.active.lock().values().map(Active::status).collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    pub fn is_active(&self, id: &str) -> bool {
        self.active.lock().contains_key(id)
    }

    /// Starts forwarding. The rule's `id` must be unique among running forwards.
    pub async fn start(&self, rule: ForwardRule) -> Result<ForwardStatus> {
        rule.validate().map_err(|e| SshError::Forward(e.to_string()))?;
        if self.is_active(&rule.id) {
            return Err(SshError::Forward("that forward is already running".into()));
        }
        let stats = Arc::new(ForwardStats::default());
        let cancel = self.conn_cancel.child_token();

        let bound_port = match rule.kind {
            ForwardKind::Local | ForwardKind::Dynamic => {
                let listener = bind_listener(&rule).await?;
                let port = listener.local_addr().map(|a| a.port()).unwrap_or(rule.bind_port);
                self.spawn_listener(listener, rule.clone(), stats.clone(), cancel.clone());
                port
            }
            ForwardKind::Remote => {
                let requested = rule.bind_port;
                let port = self
                    .handle
                    .tcpip_forward(normalise_remote_bind(&rule.bind_host), u32::from(requested))
                    .await
                    .map_err(|e| match e {
                        russh::Error::RequestDenied => SshError::Forward(format!(
                            "the server refused to listen on {}:{} (check AllowTcpForwarding and GatewayPorts)",
                            rule.bind_host, requested
                        )),
                        other => SshError::from(other),
                    })?;
                let port = if requested == 0 { port as u16 } else { requested };
                self.table.insert(
                    port,
                    RemoteTarget { host: rule.dest_host.clone(), port: rule.dest_port, stats: stats.clone() },
                );
                port
            }
        };

        info!(rule = %rule.describe(), bound_port, "forward started");
        let active = Active { rule, bound_port, stats, cancel };
        let status = active.status();
        self.active.lock().insert(active.rule.id.clone(), active);
        Ok(status)
    }

    /// Stops a running forward. Returns whether it existed.
    pub async fn stop(&self, id: &str) -> bool {
        let Some(active) = self.active.lock().remove(id) else {
            return false;
        };
        active.cancel.cancel();
        if active.rule.kind == ForwardKind::Remote {
            self.table.remove(active.bound_port);
            let _ = self
                .handle
                .cancel_tcpip_forward(normalise_remote_bind(&active.rule.bind_host), u32::from(active.bound_port))
                .await;
        }
        info!(rule = %active.rule.describe(), "forward stopped");
        true
    }

    pub async fn stop_all(&self) {
        let ids: Vec<String> = self.active.lock().keys().cloned().collect();
        for id in ids {
            self.stop(&id).await;
        }
    }

    fn spawn_listener(
        &self,
        listener: TcpListener,
        rule: ForwardRule,
        stats: Arc<ForwardStats>,
        cancel: CancellationToken,
    ) {
        let handle = self.handle.clone();
        tokio::spawn(async move {
            loop {
                let (socket, peer) = tokio::select! {
                    _ = cancel.cancelled() => break,
                    accepted = listener.accept() => match accepted {
                        Ok(x) => x,
                        Err(e) => {
                            warn!(error = %e, "accept failed");
                            tokio::time::sleep(Duration::from_millis(100)).await;
                            continue;
                        }
                    },
                };
                let _ = socket.set_nodelay(true);
                let handle = handle.clone();
                let rule = rule.clone();
                let stats = stats.clone();
                let cancel = cancel.child_token();
                tokio::spawn(async move {
                    tokio::select! {
                        _ = cancel.cancelled() => {}
                        _ = serve_local(handle, rule, socket, peer, stats) => {}
                    }
                });
            }
            debug!("forward listener stopped");
        });
    }
}

fn normalise_remote_bind(host: &str) -> String {
    match host {
        "" | "localhost" => "localhost".to_string(),
        "*" => "0.0.0.0".to_string(),
        other => other.to_string(),
    }
}

async fn bind_listener(rule: &ForwardRule) -> Result<TcpListener> {
    let host = normalise_bind(&rule.bind_host);
    TcpListener::bind((host, rule.bind_port)).await.map_err(|e| {
        let why = match e.kind() {
            std::io::ErrorKind::AddrInUse => {
                format!("port {} is already in use on this computer", rule.bind_port)
            }
            std::io::ErrorKind::PermissionDenied => {
                format!("Windows doesn't allow listening on {host}:{} (it may be reserved or blocked)", rule.bind_port)
            }
            _ => format!("couldn't listen on {host}:{}: {e}", rule.bind_port),
        };
        SshError::Forward(why)
    })
}

async fn serve_local(
    handle: Arc<Handle<ClientHandler>>,
    rule: ForwardRule,
    mut socket: TcpStream,
    peer: std::net::SocketAddr,
    stats: Arc<ForwardStats>,
) {
    let origin = peer.ip().to_string();
    match rule.kind {
        ForwardKind::Local => {
            match handle
                .channel_open_direct_tcpip(
                    rule.dest_host.clone(),
                    u32::from(rule.dest_port),
                    origin,
                    u32::from(peer.port()),
                )
                .await
            {
                Ok(channel) => pump(socket, channel, stats).await,
                Err(e) => {
                    debug!(error = %e, dest = %rule.dest_host, "server refused direct-tcpip");
                    stats.note_refusal(explain_open_failure(&e, &format!("{}:{}", rule.dest_host, rule.dest_port)));
                }
            }
        }
        ForwardKind::Dynamic => {
            let request = match tokio::time::timeout(Duration::from_secs(10), socks::handshake(&mut socket)).await {
                Ok(Ok(r)) => r,
                Ok(Err(e)) => {
                    debug!(error = %e, "socks handshake failed");
                    return;
                }
                Err(_) => return,
            };
            match handle
                .channel_open_direct_tcpip(
                    request.host.clone(),
                    u32::from(request.port),
                    origin,
                    u32::from(peer.port()),
                )
                .await
            {
                Ok(channel) => {
                    if socks::reply(&mut socket, request.version, Reply::Succeeded).await.is_ok() {
                        pump(socket, channel, stats).await;
                    }
                }
                Err(e) => {
                    let code = match &e {
                        russh::Error::ChannelOpenFailure(ChannelOpenFailure::ConnectFailed) => Reply::ConnectionRefused,
                        russh::Error::ChannelOpenFailure(ChannelOpenFailure::AdministrativelyProhibited) => {
                            Reply::NotAllowed
                        }
                        _ => Reply::GeneralFailure,
                    };
                    debug!(error = %e, host = %request.host, "socks connect failed");
                    stats.note_refusal(explain_open_failure(&e, &format!("{}:{}", request.host, request.port)));
                    let _ = socks::reply(&mut socket, request.version, code).await;
                }
            }
        }
        ForwardKind::Remote => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};

    #[tokio::test]
    async fn counted_streams_tally_both_directions() {
        let stats = Arc::new(ForwardStats::default());
        let (a, mut b) = duplex(64);
        let mut counted = Counted { inner: a, stats: stats.clone() };

        counted.write_all(b"hello").await.unwrap();
        let mut buf = [0u8; 5];
        b.read_exact(&mut buf).await.unwrap();

        b.write_all(b"worlds").await.unwrap();
        let mut buf = [0u8; 6];
        counted.read_exact(&mut buf).await.unwrap();

        assert_eq!(stats.bytes_up.load(Ordering::Relaxed), 5);
        assert_eq!(stats.bytes_down.load(Ordering::Relaxed), 6);
    }

    #[test]
    fn active_guard_tracks_connections() {
        let stats = Arc::new(ForwardStats::default());
        {
            let _a = ActiveGuard::new(stats.clone());
            let _b = ActiveGuard::new(stats.clone());
            assert_eq!(stats.active.load(Ordering::Relaxed), 2);
        }
        assert_eq!(stats.active.load(Ordering::Relaxed), 0);
        assert_eq!(stats.total.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn bind_addresses_are_normalised() {
        assert_eq!(normalise_bind(""), "127.0.0.1");
        assert_eq!(normalise_bind("localhost"), "127.0.0.1");
        assert_eq!(normalise_bind("*"), "0.0.0.0");
        assert_eq!(normalise_bind("[::1]"), "::1");
    }

    #[test]
    fn remote_table_lookup() {
        let t = RemoteForwardTable::default();
        t.insert(9000, RemoteTarget { host: "h".into(), port: 1, stats: Arc::default() });
        assert_eq!(t.lookup(9000).unwrap().host, "h");
        t.remove(9000);
        assert!(t.lookup(9000).is_none());
    }
}
