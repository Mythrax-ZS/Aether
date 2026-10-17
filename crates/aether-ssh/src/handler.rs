//! The russh client handler: host key verification and server-initiated channels.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use aether_core::known_hosts::{HostKeyRef, KnownHosts, KnownSource, Verdict, fingerprint_sha256, key_bits, randomart};
use aether_core::model::{AgentMode, HostKeyPolicy};
use parking_lot::Mutex;
use russh::client::{ChannelOpenHandle, DisconnectReason, Handler, Msg, Session};
use russh::keys::{PublicKeyBase64, PublicKeyOrCertificate};
use russh::{Channel, ChannelOpenFailure, Error as RusshError};
use serde::Serialize;
use tokio::sync::{Notify, watch};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use ts_rs::TS;

use crate::agent;
use crate::delegate::{ConnectionDelegate, HopInfo, HostKeyDecision, HostKeyRequest, HostKeyRequestKind};
use crate::error::HostKeyRejection;
use crate::forward::{self, RemoteForwardTable};

/// Facts about the server's host key, for the connection-details panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ServerKeyInfo {
    pub algorithm: String,
    pub fingerprint: String,
    pub bits: Option<u32>,
}

/// The algorithms agreed during key exchange.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NegotiatedInfo {
    pub kex: String,
    pub host_key: String,
    pub cipher: String,
    pub mac: String,
    pub compression: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisconnectKind {
    /// We closed the connection on purpose.
    Requested,
    /// The server sent a disconnect message (e.g. an admin killed the session).
    RemoteClosed,
    /// The server stopped answering keepalives.
    KeepaliveTimeout,
    /// The socket died or the protocol failed.
    Lost,
}

#[derive(Debug, Clone)]
pub struct DisconnectInfo {
    pub kind: DisconnectKind,
    pub message: String,
}

/// Plain-language reason for a connection that died, shown in the reconnect banner and in
/// the terminal ("early eof" tells the user nothing).
fn describe_loss(e: &RusshError) -> String {
    use std::io::ErrorKind;
    match e {
        RusshError::KeepaliveTimeout => "the server stopped responding".into(),
        RusshError::InactivityTimeout | RusshError::ConnectionTimeout => "the connection timed out".into(),
        RusshError::HUP | RusshError::Disconnect => "the server closed the connection".into(),
        RusshError::IO(io) => match io.kind() {
            ErrorKind::UnexpectedEof
            | ErrorKind::ConnectionReset
            | ErrorKind::ConnectionAborted
            | ErrorKind::BrokenPipe => "the server closed the connection".into(),
            ErrorKind::TimedOut => "the connection timed out".into(),
            _ => io.to_string(),
        },
        RusshError::PacketAuth | RusshError::DecryptionError | RusshError::StrictKeyExchangeViolation { .. } => {
            "the data was corrupted in transit, so the connection was closed to stay safe".into()
        }
        other => other.to_string(),
    }
}

/// State shared between a handler (inside russh's task) and the connection object.
pub(crate) struct HandlerShared {
    pub rejection: Mutex<Option<HostKeyRejection>>,
    pub server_key: Mutex<Option<ServerKeyInfo>>,
    pub negotiated: Mutex<Option<NegotiatedInfo>>,
    pub disconnect_tx: watch::Sender<Option<DisconnectInfo>>,
    /// Set before an intentional close so it isn't reported as a connection loss.
    pub closing: AtomicBool,
    /// Cancelled when the connection ends; forwards and pumps stop with it.
    pub cancel: CancellationToken,
    /// How many questions are waiting on a person right now (host-key prompts happen *inside* the handshake).
    pub prompting: AtomicUsize,
    /// Pinged whenever a question has just been answered, so the handshake clock can start the server's share of the
    /// wait afresh instead of inheriting whatever was left of the old deadline.
    pub answered: Notify,
}

impl HandlerShared {
    pub fn new() -> (Arc<Self>, watch::Receiver<Option<DisconnectInfo>>) {
        let (tx, rx) = watch::channel(None);
        (
            Arc::new(Self {
                rejection: Mutex::new(None),
                server_key: Mutex::new(None),
                negotiated: Mutex::new(None),
                disconnect_tx: tx,
                closing: AtomicBool::new(false),
                cancel: CancellationToken::new(),
                prompting: AtomicUsize::new(0),
                answered: Notify::new(),
            }),
            rx,
        )
    }
}

/// Marks a question as outstanding for as long as it is alive — answered, refused or abandoned alike.
pub(crate) struct PromptGuard<'a>(&'a HandlerShared);

impl<'a> PromptGuard<'a> {
    pub fn new(shared: &'a HandlerShared) -> Self {
        shared.prompting.fetch_add(1, Ordering::SeqCst);
        Self(shared)
    }
}

impl Drop for PromptGuard<'_> {
    fn drop(&mut self) {
        self.0.prompting.fetch_sub(1, Ordering::SeqCst);
        self.0.answered.notify_one();
    }
}

pub(crate) struct ClientHandler {
    pub hop: HopInfo,
    pub delegate: Arc<dyn ConnectionDelegate>,
    pub known_hosts: Arc<KnownHosts>,
    pub policy: HostKeyPolicy,
    pub shared: Arc<HandlerShared>,
    pub remote_forwards: Arc<RemoteForwardTable>,
    /// `Some` only on the final hop when agent forwarding was requested.
    pub agent_forward: Option<AgentMode>,
    /// `Some` only on the final hop when X11 forwarding was requested.
    pub x11: Option<Arc<crate::x11::X11Forward>>,
}

impl ClientHandler {
    async fn verify(&self, algorithm: &str, blob: &[u8]) -> Result<(), HostKeyRejection> {
        let key = HostKeyRef { algorithm, blob };
        let host = &self.hop.host;
        let port = self.hop.port;
        let request = |kind: HostKeyRequestKind| HostKeyRequest {
            hop: self.hop.clone(),
            algorithm: algorithm.to_string(),
            fingerprint: fingerprint_sha256(blob),
            bits: key_bits(algorithm, blob),
            randomart: randomart(algorithm, blob),
            kind,
        };
        // Failing to record a key is not something to wave through: under "accept new" it would mean never noticing a
        // later change, and "trust and save" would have promised something that didn't happen. Stop instead.
        let not_saved = |e: aether_core::CoreError| {
            warn!(error = %e, "could not save host key");
            HostKeyRejection::NotSaved(e.to_string())
        };
        let save = || self.known_hosts.add(host, port, &key).map_err(not_saved);
        // The person reading a fingerprint takes as long as they take; the handshake clock waits for them.
        let ask = |request: HostKeyRequest| async {
            let _waiting = PromptGuard::new(&self.shared);
            self.delegate.verify_host_key(request).await
        };

        match self.known_hosts.lookup(host, port, &key) {
            Verdict::Trusted(source) => {
                debug!(?source, "host key trusted");
                Ok(())
            }
            Verdict::Revoked => Err(HostKeyRejection::Revoked),
            // Not knowing what is on record is not the same as nothing being on record. A first-visit prompt (or a quiet
            // "accept new") would be false, and an impostor's key could pass for a new one. Stop, under every policy.
            Verdict::Unverifiable(unreadable) => {
                warn!(file = %unreadable.path.display(), reason = %unreadable.reason, "known_hosts can't be read");
                Err(HostKeyRejection::Unverifiable {
                    file: unreadable.path.display().to_string(),
                    user_file: unreadable.source == KnownSource::UserSsh,
                    why: unreadable.reason,
                })
            }
            Verdict::Unknown => match self.policy {
                HostKeyPolicy::Strict => Err(HostKeyRejection::UnknownInStrictMode),
                HostKeyPolicy::AcceptNew => {
                    info!(%host, "accepting new host key (accept-new policy)");
                    save()
                }
                HostKeyPolicy::Ask => match ask(request(HostKeyRequestKind::Unknown)).await {
                    HostKeyDecision::TrustAndSave | HostKeyDecision::ReplaceAndTrust => save(),
                    HostKeyDecision::TrustOnce => Ok(()),
                    HostKeyDecision::Reject => Err(HostKeyRejection::Declined),
                },
            },
            // A changed key — or a key of a type we have no record of for a host we do know — is never accepted
            // quietly, under any policy that can ask, and never at all under "strict".
            Verdict::Changed { known } | Verdict::OtherKeyType { known } => {
                if self.policy == HostKeyPolicy::Strict {
                    return Err(HostKeyRejection::Changed);
                }
                match ask(request(HostKeyRequestKind::Changed { known })).await {
                    HostKeyDecision::ReplaceAndTrust => self.known_hosts.replace(host, port, &key).map_err(not_saved),
                    _ => Err(HostKeyRejection::Changed),
                }
            }
        }
    }
}

impl Handler for ClientHandler {
    type Error = RusshError;

    async fn check_server_key(&mut self, server_public_key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let key = server_public_key.public_key();
        let algorithm = key.algorithm().to_string();
        let blob = key.public_key_bytes();

        *self.shared.server_key.lock() = Some(ServerKeyInfo {
            algorithm: algorithm.clone(),
            fingerprint: fingerprint_sha256(&blob),
            bits: key_bits(&algorithm, &blob),
        });

        match self.verify(&algorithm, &blob).await {
            Ok(()) => Ok(true),
            Err(reason) => {
                info!(host = %self.hop.host, ?reason, "host key rejected");
                *self.shared.rejection.lock() = Some(reason);
                Ok(false)
            }
        }
    }

    async fn kex_done(
        &mut self,
        _shared_secret: Option<&[u8]>,
        names: &russh::Names,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        *self.shared.negotiated.lock() = Some(NegotiatedInfo {
            kex: names.kex.as_ref().to_string(),
            host_key: names.key.to_string(),
            cipher: names.cipher.as_ref().to_string(),
            mac: names.server_mac.as_ref().to_string(),
            compression: format!("{:?}", names.server_compression).to_lowercase(),
        });
        Ok(())
    }

    async fn auth_banner(&mut self, banner: &str, _session: &mut Session) -> Result<(), Self::Error> {
        self.delegate.banner(&self.hop, banner);
        Ok(())
    }

    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: Channel<Msg>,
        connected_address: &str,
        connected_port: u32,
        originator_address: &str,
        originator_port: u32,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        let target = u16::try_from(connected_port).ok().and_then(|p| self.remote_forwards.lookup(p));
        match target {
            Some(target) => {
                debug!(%connected_address, connected_port, %originator_address, originator_port, "remote forward connection");
                tokio::spawn(forward::serve_remote(channel, reply, target));
            }
            None => {
                warn!(connected_port, "server opened a forwarded channel for a port we never requested");
                reply.reject(ChannelOpenFailure::AdministrativelyProhibited).await;
            }
        }
        Ok(())
    }

    async fn server_channel_open_agent_forward(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        let Some(mode) = self.agent_forward else {
            // Never serve the agent unless the user opted in for this host.
            reply.reject(ChannelOpenFailure::AdministrativelyProhibited).await;
            return Ok(());
        };
        tokio::spawn(async move {
            match agent::connect_stream(mode).await {
                Ok(mut local) => {
                    reply.accept().await;
                    let mut remote = channel.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut local, &mut remote).await;
                }
                Err(e) => {
                    debug!(error = %e, "agent forwarding requested but no local agent");
                    reply.reject(ChannelOpenFailure::ConnectFailed).await;
                }
            }
        });
        Ok(())
    }

    async fn server_channel_open_x11(
        &mut self,
        channel: Channel<Msg>,
        originator_address: &str,
        originator_port: u32,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        let Some(x11) = self.x11.clone() else {
            // Never serve X11 unless the user opted in for this host.
            reply.reject(ChannelOpenFailure::AdministrativelyProhibited).await;
            return Ok(());
        };
        debug!(%originator_address, originator_port, "x11 connection from the server");
        tokio::spawn(x11.serve(channel, reply));
        Ok(())
    }

    async fn disconnected(&mut self, reason: DisconnectReason<Self::Error>) -> Result<(), Self::Error> {
        let requested = self.shared.closing.load(Ordering::SeqCst);
        let info = match &reason {
            DisconnectReason::ReceivedDisconnect(d) => DisconnectInfo {
                kind: if requested { DisconnectKind::Requested } else { DisconnectKind::RemoteClosed },
                message: if d.message.trim().is_empty() { format!("{:?}", d.reason_code) } else { d.message.clone() },
            },
            DisconnectReason::Error(e) => DisconnectInfo {
                kind: if requested {
                    DisconnectKind::Requested
                } else if matches!(e, RusshError::KeepaliveTimeout) {
                    DisconnectKind::KeepaliveTimeout
                } else {
                    DisconnectKind::Lost
                },
                message: describe_loss(e),
            },
        };
        debug!(host = %self.hop.host, ?info, "disconnected");
        let _ = self.shared.disconnect_tx.send(Some(info));
        self.shared.cancel.cancel();
        match reason {
            DisconnectReason::Error(e) => Err(e),
            DisconnectReason::ReceivedDisconnect(_) => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delegate::{PromptRequest, PromptResponse, SecretLookup};
    use aether_vault::Secret;
    use async_trait::async_trait;
    use std::sync::atomic::AtomicUsize;

    struct Scripted {
        decision: HostKeyDecision,
        asked: AtomicUsize,
        /// What the handler's `prompting` counter read while a question was open, and the kind of question.
        seen: Mutex<Vec<(usize, HostKeyRequestKind)>>,
        shared: Mutex<Option<Arc<HandlerShared>>>,
    }

    #[async_trait]
    impl ConnectionDelegate for Scripted {
        async fn verify_host_key(&self, r: HostKeyRequest) -> HostKeyDecision {
            self.asked.fetch_add(1, Ordering::SeqCst);
            let open = self.shared.lock().as_ref().map_or(usize::MAX, |s| s.prompting.load(Ordering::SeqCst));
            self.seen.lock().push((open, r.kind));
            self.decision
        }
        async fn prompt(&self, _r: PromptRequest) -> Option<PromptResponse> {
            None
        }
        fn saved_secret(&self, _l: &SecretLookup) -> Option<Secret> {
            None
        }
        fn remember_secret(&self, _l: &SecretLookup, _s: &Secret) {}
        fn forget_secret(&self, _l: &SecretLookup) {}
    }

    fn blob(fill: u8) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&11u32.to_be_bytes());
        b.extend_from_slice(b"ssh-ed25519");
        b.extend_from_slice(&32u32.to_be_bytes());
        b.extend_from_slice(&[fill; 32]);
        b
    }

    fn handler(
        dir: &std::path::Path,
        policy: HostKeyPolicy,
        decision: HostKeyDecision,
    ) -> (ClientHandler, Arc<Scripted>) {
        let delegate = Arc::new(Scripted {
            decision,
            asked: AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
            shared: Mutex::new(None),
        });
        let (shared, _rx) = HandlerShared::new();
        *delegate.shared.lock() = Some(shared.clone());
        (
            ClientHandler {
                hop: HopInfo {
                    index: 0,
                    total: 1,
                    label: "t".into(),
                    host: "example.com".into(),
                    port: 22,
                    username: "u".into(),
                },
                delegate: delegate.clone(),
                known_hosts: Arc::new(KnownHosts::new(dir.join("kh"), None)),
                policy,
                shared,
                remote_forwards: Arc::default(),
                agent_forward: None,
                x11: None,
            },
            delegate,
        )
    }

    #[tokio::test]
    async fn unknown_host_trust_and_save_is_remembered() {
        let dir = tempfile::tempdir().unwrap();
        let (h, delegate) = handler(dir.path(), HostKeyPolicy::Ask, HostKeyDecision::TrustAndSave);
        let b = blob(1);
        assert!(h.verify("ssh-ed25519", &b).await.is_ok());
        assert_eq!(delegate.asked.load(Ordering::SeqCst), 1);
        // Second connection: trusted silently.
        assert!(h.verify("ssh-ed25519", &b).await.is_ok());
        assert_eq!(delegate.asked.load(Ordering::SeqCst), 1, "no second prompt");
    }

    #[tokio::test]
    async fn trust_once_is_not_remembered() {
        let dir = tempfile::tempdir().unwrap();
        let (h, delegate) = handler(dir.path(), HostKeyPolicy::Ask, HostKeyDecision::TrustOnce);
        let b = blob(1);
        h.verify("ssh-ed25519", &b).await.unwrap();
        h.verify("ssh-ed25519", &b).await.unwrap();
        assert_eq!(delegate.asked.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn declining_rejects() {
        let dir = tempfile::tempdir().unwrap();
        let (h, _) = handler(dir.path(), HostKeyPolicy::Ask, HostKeyDecision::Reject);
        assert_eq!(h.verify("ssh-ed25519", &blob(1)).await, Err(HostKeyRejection::Declined));
    }

    #[tokio::test]
    async fn strict_policy_never_asks() {
        let dir = tempfile::tempdir().unwrap();
        let (h, delegate) = handler(dir.path(), HostKeyPolicy::Strict, HostKeyDecision::TrustAndSave);
        assert_eq!(h.verify("ssh-ed25519", &blob(1)).await, Err(HostKeyRejection::UnknownInStrictMode));
        assert_eq!(delegate.asked.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn accept_new_saves_without_asking_but_still_blocks_changes() {
        let dir = tempfile::tempdir().unwrap();
        let (h, delegate) = handler(dir.path(), HostKeyPolicy::AcceptNew, HostKeyDecision::Reject);
        h.verify("ssh-ed25519", &blob(1)).await.unwrap();
        assert_eq!(delegate.asked.load(Ordering::SeqCst), 0);
        // A different key for the same host now asks, and the scripted answer is Reject.
        assert_eq!(h.verify("ssh-ed25519", &blob(2)).await, Err(HostKeyRejection::Changed));
        assert_eq!(delegate.asked.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn changed_key_requires_explicit_replace() {
        let dir = tempfile::tempdir().unwrap();
        let (h, _) = handler(dir.path(), HostKeyPolicy::Ask, HostKeyDecision::TrustAndSave);
        h.verify("ssh-ed25519", &blob(1)).await.unwrap();

        // "Trust and save" is not enough to override a changed key.
        assert_eq!(h.verify("ssh-ed25519", &blob(2)).await, Err(HostKeyRejection::Changed));

        let (h2, _) = handler(dir.path(), HostKeyPolicy::Ask, HostKeyDecision::ReplaceAndTrust);
        h2.verify("ssh-ed25519", &blob(2)).await.unwrap();
        // The new key is now the trusted one.
        assert!(h2.verify("ssh-ed25519", &blob(2)).await.is_ok());
        assert_eq!(h.verify("ssh-ed25519", &blob(1)).await, Err(HostKeyRejection::Changed));
    }

    fn ecdsa_blob(fill: u8) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&19u32.to_be_bytes());
        b.extend_from_slice(b"ecdsa-sha2-nistp256");
        b.extend_from_slice(&65u32.to_be_bytes());
        b.extend_from_slice(&[fill; 65]);
        b
    }

    const ECDSA: &str = "ecdsa-sha2-nistp256";

    /// A host we know, offering only a key type we have no record of, is what an impostor would do to dodge the
    /// "changed" warning — so "accept new" must not wave it through as if it were a first visit.
    #[tokio::test]
    async fn a_new_key_type_for_a_known_host_is_asked_about_even_under_accept_new() {
        let dir = tempfile::tempdir().unwrap();
        let (h, delegate) = handler(dir.path(), HostKeyPolicy::AcceptNew, HostKeyDecision::Reject);
        h.verify("ssh-ed25519", &blob(1)).await.unwrap();
        assert_eq!(delegate.asked.load(Ordering::SeqCst), 0, "the first visit is silent");

        assert_eq!(h.verify(ECDSA, &ecdsa_blob(7)).await, Err(HostKeyRejection::Changed));
        assert_eq!(delegate.asked.load(Ordering::SeqCst), 1);
        match &delegate.seen.lock()[0].1 {
            HostKeyRequestKind::Changed { known } => assert_eq!(known[0].algorithm, "ssh-ed25519"),
            other => panic!("expected the stronger prompt, got {other:?}"),
        }
        // Declining recorded nothing.
        assert!(matches!(
            h.known_hosts.lookup("example.com", 22, &HostKeyRef { algorithm: ECDSA, blob: &ecdsa_blob(7) }),
            Verdict::OtherKeyType { .. }
        ));
    }

    #[tokio::test]
    async fn strict_policy_refuses_a_new_key_type_without_asking() {
        let dir = tempfile::tempdir().unwrap();
        let (h, delegate) = handler(dir.path(), HostKeyPolicy::Strict, HostKeyDecision::ReplaceAndTrust);
        h.known_hosts.add("example.com", 22, &HostKeyRef { algorithm: "ssh-ed25519", blob: &blob(1) }).unwrap();
        assert_eq!(h.verify(ECDSA, &ecdsa_blob(7)).await, Err(HostKeyRejection::Changed));
        assert_eq!(delegate.asked.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_new_key_type_needs_an_explicit_replace_and_then_both_types_work() {
        let dir = tempfile::tempdir().unwrap();
        let (first, _) = handler(dir.path(), HostKeyPolicy::Ask, HostKeyDecision::TrustAndSave);
        first.verify("ssh-ed25519", &blob(1)).await.unwrap();

        // "Trust and save" is the answer to a first-visit question, not to this one.
        let (h, _) = handler(dir.path(), HostKeyPolicy::Ask, HostKeyDecision::TrustAndSave);
        assert_eq!(h.verify(ECDSA, &ecdsa_blob(7)).await, Err(HostKeyRejection::Changed));
        let (h, _) = handler(dir.path(), HostKeyPolicy::Ask, HostKeyDecision::TrustOnce);
        assert_eq!(h.verify(ECDSA, &ecdsa_blob(7)).await, Err(HostKeyRejection::Changed));

        let (h, _) = handler(dir.path(), HostKeyPolicy::Ask, HostKeyDecision::ReplaceAndTrust);
        h.verify(ECDSA, &ecdsa_blob(7)).await.unwrap();
        let (quiet, delegate) = handler(dir.path(), HostKeyPolicy::Ask, HostKeyDecision::Reject);
        quiet.verify(ECDSA, &ecdsa_blob(7)).await.unwrap();
        quiet.verify("ssh-ed25519", &blob(1)).await.unwrap();
        assert_eq!(delegate.asked.load(Ordering::SeqCst), 0, "both are on record now");
    }

    /// If the key can't be written down, the connection stops: "trust and save" must not quietly mean "trust once", and
    /// under "accept new" a key that was never recorded could never be noticed changing.
    #[tokio::test]
    async fn a_key_that_cannot_be_recorded_stops_the_connection() {
        let dir = tempfile::tempdir().unwrap();
        // A name that can't be written to known_hosts (a pattern, not a host): nothing can be recorded for it.
        let (mut h, _) = handler(dir.path(), HostKeyPolicy::AcceptNew, HostKeyDecision::Reject);
        h.hop.host = "*.example.com".into();
        assert!(matches!(h.verify("ssh-ed25519", &blob(1)).await, Err(HostKeyRejection::NotSaved(_))));

        let (mut h, _) = handler(dir.path(), HostKeyPolicy::Ask, HostKeyDecision::TrustAndSave);
        h.hop.host = "*.example.com".into();
        assert!(matches!(h.verify("ssh-ed25519", &blob(1)).await, Err(HostKeyRejection::NotSaved(_))));

        // "Connect once" was asked for explicitly and records nothing, so it has nothing to fail at.
        let (mut h, _) = handler(dir.path(), HostKeyPolicy::Ask, HostKeyDecision::TrustOnce);
        h.hop.host = "*.example.com".into();
        assert_eq!(h.verify("ssh-ed25519", &blob(1)).await, Ok(()));
    }

    /// A list of recorded keys that is there but can't be read says nothing about this server. It is not "never seen":
    /// a first-visit prompt would be false, "accept new" would wave through an impostor's key, and a "trust once" would
    /// skip the check that matters. Every policy stops, and nobody is asked.
    #[tokio::test]
    async fn a_known_hosts_file_that_cannot_be_read_stops_the_connection() {
        let dir = tempfile::tempdir().unwrap();
        // The known_hosts "file" is a directory: reading it fails with something other than "not found".
        std::fs::create_dir(dir.path().join("kh")).unwrap();

        for (policy, decision) in [
            (HostKeyPolicy::AcceptNew, HostKeyDecision::Reject),
            (HostKeyPolicy::Ask, HostKeyDecision::TrustAndSave),
            (HostKeyPolicy::Ask, HostKeyDecision::TrustOnce),
            (HostKeyPolicy::Strict, HostKeyDecision::TrustOnce),
        ] {
            let (h, delegate) = handler(dir.path(), policy, decision);
            let rejected = h.verify("ssh-ed25519", &blob(1)).await;
            assert!(
                matches!(rejected, Err(HostKeyRejection::Unverifiable { user_file: false, .. })),
                "{policy:?}: {rejected:?}"
            );
            assert_eq!(delegate.asked.load(Ordering::SeqCst), 0, "nobody is asked to trust what can't be checked");
        }
    }

    /// The handshake deadline pauses while a person is being asked; the counter that drives that must be up for exactly
    /// as long as the question is open, on every path out of it.
    #[tokio::test]
    async fn the_prompt_counter_is_up_only_while_a_question_is_open() {
        let dir = tempfile::tempdir().unwrap();
        let (h, delegate) = handler(dir.path(), HostKeyPolicy::Ask, HostKeyDecision::TrustAndSave);
        assert_eq!(h.shared.prompting.load(Ordering::SeqCst), 0);
        h.verify("ssh-ed25519", &blob(1)).await.unwrap();
        assert_eq!(delegate.seen.lock()[0].0, 1, "up during the question");
        assert_eq!(h.shared.prompting.load(Ordering::SeqCst), 0, "down afterwards");

        // A changed-key question counts the same way, and a refusal lets go too.
        let (h, delegate) = handler(dir.path(), HostKeyPolicy::Ask, HostKeyDecision::Reject);
        h.verify("ssh-ed25519", &blob(2)).await.unwrap_err();
        assert_eq!(delegate.seen.lock()[0].0, 1);
        assert_eq!(h.shared.prompting.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn the_prompt_guard_lets_go_however_it_is_dropped() {
        let (shared, _rx) = HandlerShared::new();
        {
            let _a = PromptGuard::new(&shared);
            let _b = PromptGuard::new(&shared);
            assert_eq!(shared.prompting.load(Ordering::SeqCst), 2, "questions can overlap");
        }
        assert_eq!(shared.prompting.load(Ordering::SeqCst), 0);
        // Letting go wakes the handshake clock exactly once (the permit is stored until it listens).
        tokio::time::timeout(std::time::Duration::from_secs(1), shared.answered.notified())
            .await
            .expect("the clock is told that a question was answered");
    }
}
