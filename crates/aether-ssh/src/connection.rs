//! An authenticated SSH connection, possibly reached through jump hosts.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use aether_core::known_hosts::KnownHosts;
use aether_core::util::now_ms;
use russh::Disconnect;
use russh::client::{self, Handle};
use serde::Serialize;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use ts_rs::TS;

use crate::auth::{self, AuthCtx};
use crate::config;
use crate::delegate::{ConnectProgress, ConnectStage, ConnectionDelegate, HopInfo};
use crate::error::{HostKeyRejection, Result, SshError};
use crate::forward::{ForwardManager, RemoteForwardTable};
use crate::handler::{ClientHandler, DisconnectInfo, HandlerShared, NegotiatedInfo, ServerKeyInfo};
use crate::spec::ConnectSpec;
use crate::transport;

/// What the engine needs from its environment to make a connection.
#[derive(Clone)]
pub struct ConnectContext {
    pub delegate: Arc<dyn ConnectionDelegate>,
    pub known_hosts: Arc<KnownHosts>,
    pub cancel: CancellationToken,
}

/// Descriptive facts about an established connection.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConnectionInfo {
    /// `user@host:port` of the final target.
    pub target: String,
    /// Labels of the jump hosts traversed, outermost first.
    pub via: Vec<String>,
    pub server_key: Option<ServerKeyInfo>,
    pub negotiated: Option<NegotiatedInfo>,
    #[ts(type = "number")]
    pub connected_at: i64,
}

pub(crate) struct ConnInner {
    pub handle: Arc<Handle<ClientHandler>>,
    /// Intermediate hops; held so their sessions stay alive as long as the target's.
    _jumps: Vec<Handle<ClientHandler>>,
    pub shared: Arc<HandlerShared>,
    pub disconnect_rx: watch::Receiver<Option<DisconnectInfo>>,
    pub forwards: ForwardManager,
    pub info: ConnectionInfo,
    pub x11: Option<Arc<crate::x11::X11Forward>>,
}

impl Drop for ConnInner {
    fn drop(&mut self) {
        self.shared.closing.store(true, Ordering::SeqCst);
        self.shared.cancel.cancel();
    }
}

/// Cheaply cloneable handle to a live connection.
#[derive(Clone)]
pub struct SshConnection {
    pub(crate) inner: Arc<ConnInner>,
}

struct HopResult {
    handle: Handle<ClientHandler>,
    shared: Arc<HandlerShared>,
    remote_forwards: Arc<RemoteForwardTable>,
    disconnect_rx: watch::Receiver<Option<DisconnectInfo>>,
    x11: Option<Arc<crate::x11::X11Forward>>,
}

fn hop_info(index: usize, total: usize, spec: &ConnectSpec) -> HopInfo {
    HopInfo {
        index: index as u32,
        total: total as u32,
        label: spec.label.clone(),
        host: spec.host.clone(),
        port: spec.port,
        username: spec.username.clone(),
    }
}

impl SshConnection {
    /// Connects and authenticates, traversing any configured jump hosts.
    pub async fn connect(spec: &ConnectSpec, ctx: &ConnectContext) -> Result<SshConnection> {
        let hops = spec.hops();
        let total = hops.len();
        let mut jumps: Vec<Handle<ClientHandler>> = Vec::new();
        let mut final_hop: Option<HopResult> = None;

        for (index, hop) in hops.iter().enumerate() {
            let is_last = index + 1 == total;
            let info = hop_info(index, total, hop);
            let previous = jumps.last();
            let result = connect_hop(hop, &info, previous, is_last, ctx).await;
            match result {
                Ok(r) if is_last => final_hop = Some(r),
                Ok(r) => jumps.push(r.handle),
                Err(e) if !is_last => {
                    return Err(match e {
                        // Keep user decisions and cancellations as they are.
                        SshError::Cancelled | SshError::AuthCancelled => e,
                        other => SshError::JumpHost { hop: hop.label.clone(), source: Box::new(other) },
                    });
                }
                Err(e) => return Err(e),
            }
        }

        let hop = final_hop.ok_or_else(|| SshError::Internal("empty hop list".into()))?;
        let handle = Arc::new(hop.handle);
        let info = ConnectionInfo {
            target: spec.target(),
            via: spec.jump.iter().map(|j| j.label.clone()).collect(),
            server_key: hop.shared.server_key.lock().clone(),
            negotiated: hop.shared.negotiated.lock().clone(),
            connected_at: now_ms(),
        };
        ctx.delegate.progress(ConnectProgress { hop: hop_info(total - 1, total, spec), stage: ConnectStage::Ready });
        info!(target = %info.target, via = ?info.via, "connected");

        let forwards = ForwardManager::new(handle.clone(), hop.remote_forwards, hop.shared.cancel.clone());
        Ok(SshConnection {
            inner: Arc::new(ConnInner {
                handle,
                _jumps: jumps,
                shared: hop.shared,
                disconnect_rx: hop.disconnect_rx,
                forwards,
                info,
                x11: hop.x11,
            }),
        })
    }

    /// The local X display this connection forwards to, if X11 forwarding is enabled.
    pub fn x11_display(&self) -> Option<String> {
        self.inner.x11.as_ref().map(|x| {
            let d = x.display();
            format!("{}:{}.{}", d.host, d.display, d.screen)
        })
    }

    pub fn info(&self) -> &ConnectionInfo {
        &self.inner.info
    }

    pub fn forwards(&self) -> &ForwardManager {
        &self.inner.forwards
    }

    /// True once the connection has ended for any reason.
    pub fn is_closed(&self) -> bool {
        self.inner.handle.is_closed() || self.inner.disconnect_rx.borrow().is_some()
    }

    /// Resolves when the connection ends, with the reason.
    pub async fn closed(&self) -> DisconnectInfo {
        let mut rx = self.inner.disconnect_rx.clone();
        loop {
            if let Some(info) = rx.borrow_and_update().clone() {
                return info;
            }
            if rx.changed().await.is_err() {
                return DisconnectInfo {
                    kind: crate::handler::DisconnectKind::Lost,
                    message: "the connection ended".into(),
                };
            }
        }
    }

    /// Round-trip time to the server, measured with an SSH-level ping.
    pub async fn ping(&self) -> Result<Duration> {
        let started = Instant::now();
        tokio::time::timeout(Duration::from_secs(10), self.inner.handle.send_ping())
            .await
            .map_err(|_| SshError::KeepaliveTimeout)??;
        Ok(started.elapsed())
    }

    /// Closes the connection deliberately.
    pub async fn close(&self) {
        self.inner.shared.closing.store(true, Ordering::SeqCst);
        self.inner.forwards.stop_all().await;
        let _ = self.inner.handle.disconnect(Disconnect::ByApplication, "closed by user", "en").await;
        self.inner.shared.cancel.cancel();
    }

    /// Opens a raw TCP tunnel to `host:port` through the server.
    pub async fn open_tunnel(&self, host: &str, port: u16) -> Result<russh::ChannelStream<client::Msg>> {
        let channel = self
            .inner
            .handle
            .channel_open_direct_tcpip(host.to_string(), u32::from(port), "127.0.0.1", 0)
            .await
            .map_err(|e| channel_open_error(&format!("a connection to {host}:{port}"), e))?;
        Ok(channel.into_stream())
    }
}

/// Maps a failed channel-open into a readable error.
pub(crate) fn channel_open_error(what: &str, e: russh::Error) -> SshError {
    match e {
        russh::Error::ChannelOpenFailure(reason) => {
            SshError::ChannelOpen { what: what.to_string(), reason: reason.description().to_lowercase() }
        }
        other => SshError::from(other),
    }
}

async fn connect_hop(
    spec: &ConnectSpec,
    info: &HopInfo,
    previous: Option<&Handle<ClientHandler>>,
    is_last: bool,
    ctx: &ConnectContext,
) -> Result<HopResult> {
    let delegate = &ctx.delegate;
    let timeout = spec.options.connect_timeout;
    let progress = |stage| {
        delegate.progress(ConnectProgress { hop: info.clone(), stage });
    };

    let known_types = ctx.known_hosts.known_algorithms(&spec.host, spec.port);
    let client_config = config::client_config(&spec.options, &known_types);

    let (shared, disconnect_rx) = HandlerShared::new();
    let remote_forwards = Arc::new(RemoteForwardTable::default());
    let x11 = match spec.options.x11.as_ref().filter(|_| is_last) {
        Some(options) => match crate::x11::X11Forward::new(options) {
            Ok(forward) => Some(Arc::new(forward)),
            Err(why) => {
                // A bad display address shouldn't stop you from connecting.
                warn!(%why, "X11 forwarding disabled");
                None
            }
        },
        None => None,
    };
    let handler = ClientHandler {
        hop: info.clone(),
        delegate: delegate.clone(),
        known_hosts: ctx.known_hosts.clone(),
        policy: spec.options.host_key_policy,
        shared: shared.clone(),
        remote_forwards: remote_forwards.clone(),
        agent_forward: (is_last && spec.options.agent_forwarding).then_some(spec.auth.agent),
        x11: x11.clone(),
    };

    // 1. Get a byte stream to the hop.
    progress(ConnectStage::Resolving);
    let connect_result = match (previous, &spec.proxy_command) {
        (Some(prev), _) => {
            progress(ConnectStage::Connecting);
            let channel = tokio::time::timeout(
                timeout,
                prev.channel_open_direct_tcpip(spec.host.clone(), u32::from(spec.port), "127.0.0.1", 0),
            )
            .await
            .map_err(|_| SshError::ConnectTimeout { host: spec.host.clone(), port: spec.port })?
            .map_err(|e| channel_open_error(&format!("a tunnel to {}:{}", spec.host, spec.port), e))?;
            progress(ConnectStage::Handshake);
            handshake(client_config, channel.into_stream(), handler, timeout, &ctx.cancel).await
        }
        (None, Some(cmd)) if !cmd.trim().is_empty() => {
            progress(ConnectStage::Connecting);
            let local_user = aether_core::util::local_username();
            let home = aether_core::util::home_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
            let tokens = transport::ProxyTokens {
                host: &spec.host,
                port: spec.port,
                user: &spec.username,
                name: &spec.label,
                local_user: &local_user,
                home: &home,
            };
            let stream = transport::spawn_proxy(cmd, &tokens)?;
            let stderr = stream.stderr_tail.clone();
            progress(ConnectStage::Handshake);
            match handshake(client_config, stream, handler, timeout, &ctx.cancel).await {
                Err(e) => Err(explain_proxy_exit(e, cmd, &stderr).await),
                ok => ok,
            }
        }
        (None, _) => {
            let stream =
                transport::connect_tcp(&spec.host, spec.port, timeout, spec.options.nodelay, &ctx.cancel).await?;
            progress(ConnectStage::Handshake);
            handshake(client_config, stream, handler, timeout, &ctx.cancel).await
        }
    };

    let mut handle = match connect_result {
        Ok(h) => h,
        Err(e) => {
            // A refusal recorded by the handler is more precise than russh's generic error.
            if let Some(reason) = shared.rejection.lock().clone() {
                return Err(SshError::HostKeyRejected { host: spec.host.clone(), reason });
            }
            return Err(e);
        }
    };
    debug!(host = %spec.host, "ssh handshake complete");

    // 2. Log in.
    let auth_ctx = AuthCtx { delegate, hop: info, spec, cancel: &ctx.cancel };
    if let Err(e) = auth::authenticate(&mut handle, &auth_ctx).await {
        let _ = handle.disconnect(Disconnect::ByApplication, "authentication failed", "en").await;
        return Err(e);
    }
    progress(ConnectStage::Establishing);

    // A host key rejection can also surface after the fact on re-key; surface it clearly.
    if matches!(shared.rejection.lock().clone(), Some(HostKeyRejection::Changed)) {
        return Err(SshError::HostKeyRejected { host: spec.host.clone(), reason: HostKeyRejection::Changed });
    }

    Ok(HopResult { handle, shared, remote_forwards, disconnect_rx, x11 })
}

/// A proxy command that dies before the handshake finishes usually says why on stderr, which is far more
/// useful than "the server closed the connection".
async fn explain_proxy_exit(err: SshError, command: &str, stderr: &parking_lot::Mutex<String>) -> SshError {
    if matches!(err, SshError::Cancelled) {
        return err;
    }
    // The stderr reader runs on its own task; let it drain what the process printed on its way out.
    tokio::time::sleep(Duration::from_millis(150)).await;
    let said = stderr.lock().trim().to_string();
    if said.is_empty() {
        return err;
    }
    SshError::ProxyCommand(format!(
        "`{command}` stopped before the connection was made and said:
{said}"
    ))
}

async fn handshake<S>(
    config: Arc<client::Config>,
    stream: S,
    handler: ClientHandler,
    timeout: Duration,
    cancel: &CancellationToken,
) -> Result<Handle<ClientHandler>>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let shared = handler.shared.clone();
    let connect = client::connect_stream(config, stream, handler);
    tokio::pin!(connect);
    // The clock measures how long *the server* takes. The host-key question is asked in the middle of the handshake
    // and a person may need minutes to compare a fingerprint, so while one is outstanding the deadline keeps moving,
    // and once it is answered the server gets a full timeout again for what is left of the handshake.
    let mut deadline = tokio::time::Instant::now() + timeout;
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return Err(SshError::Cancelled),
            r = &mut connect => return r.map_err(SshError::from),
            _ = shared.answered.notified() => deadline = tokio::time::Instant::now() + timeout,
            _ = tokio::time::sleep_until(deadline) => {
                if shared.prompting.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                    return Err(SshError::Disconnected("the SSH handshake timed out".into()));
                }
                deadline = tokio::time::Instant::now() + timeout;
            }
        }
    }
}
