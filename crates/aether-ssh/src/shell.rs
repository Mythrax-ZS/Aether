//! Interactive shell channels and one-shot command execution.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use russh::client::Msg;
use russh::{ChannelMsg, ChannelReadHalf, ChannelWriteHalf, Pty};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, trace};

use crate::connection::{SshConnection, channel_open_error};
use crate::error::{Result, SshError};
use crate::spec::{PtySize, ShellParams};

/// What a shell produces.
#[derive(Debug)]
pub enum ShellEvent {
    /// Terminal output (stdout and stderr are merged on a PTY).
    Data(Bytes),
    /// The remote process exited.
    Exit { status: Option<u32>, signal: Option<String> },
    /// The channel is closed; no more events will follow.
    Closed,
}

/// Write side of a shell. Cheap to clone.
#[derive(Clone)]
pub struct ShellHandle {
    write: Arc<ChannelWriteHalf<Msg>>,
    cancel: CancellationToken,
}

impl ShellHandle {
    pub async fn write(&self, data: impl Into<Bytes>) -> Result<()> {
        self.write.data_bytes(data).await.map_err(SshError::from)
    }

    pub async fn resize(&self, size: PtySize) -> Result<()> {
        self.write
            .window_change(size.cols, size.rows, size.pixel_width, size.pixel_height)
            .await
            .map_err(SshError::from)
    }

    /// Sends EOF (like Ctrl+D at the transport level) and closes the channel.
    pub async fn close(&self) {
        let _ = self.write.eof().await;
        let _ = self.write.close().await;
        self.cancel.cancel();
    }
}

/// Modes that make the remote tty behave like a modern terminal.
fn terminal_modes() -> Vec<(Pty, u32)> {
    vec![
        (Pty::ICRNL, 1),
        (Pty::IXON, 1),
        (Pty::IXANY, 1),
        (Pty::IMAXBEL, 1),
        (Pty::IUTF8, 1),
        (Pty::ISIG, 1),
        (Pty::ICANON, 1),
        (Pty::ECHO, 1),
        (Pty::ECHOE, 1),
        (Pty::ECHOK, 1),
        (Pty::ECHOCTL, 1),
        (Pty::ECHOKE, 1),
        (Pty::IEXTEN, 1),
        (Pty::OPOST, 1),
        (Pty::ONLCR, 1),
        (Pty::CS8, 1),
        (Pty::TTY_OP_ISPEED, 38_400),
        (Pty::TTY_OP_OSPEED, 38_400),
    ]
}

/// Waits for the server's yes/no answer to a channel request, keeping any data it sends
/// in the meantime.
async fn expect_reply(read: &mut ChannelReadHalf, what: &str, early: &mut Vec<Bytes>) -> Result<()> {
    let wait = async {
        loop {
            match read.wait().await {
                Some(ChannelMsg::Success) => return Ok(()),
                Some(ChannelMsg::Failure) => {
                    return Err(SshError::ChannelOpen { what: what.to_string(), reason: "the server said no".into() });
                }
                Some(ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. }) => early.push(data),
                Some(ChannelMsg::Close) | None => {
                    return Err(SshError::Disconnected("the session closed during setup".into()));
                }
                Some(_) => {}
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(20), wait)
        .await
        .map_err(|_| SshError::Disconnected(format!("timed out waiting for the server to accept {what}")))?
}

impl SshConnection {
    /// Opens an interactive shell with a PTY. Output arrives on the returned receiver;
    /// the receiver applies backpressure — if it isn't drained, reading from the network
    /// pauses.
    pub async fn open_shell(&self, params: &ShellParams) -> Result<(ShellHandle, mpsc::Receiver<ShellEvent>)> {
        let channel =
            self.inner.handle.channel_open_session().await.map_err(|e| channel_open_error("a terminal session", e))?;
        let (mut read, write) = channel.split();
        let write = Arc::new(write);
        let mut early: Vec<Bytes> = Vec::new();

        if params.x11
            && let Some(x11) = &self.inner.x11
        {
            // Hand the server a throwaway cookie for the remote $DISPLAY; the real one never leaves
            // this PC. No reply requested (like OpenSSH): a server that forbids X11 simply won't
            // open channels.
            let _ = write.request_x11(false, false, crate::x11::PROTOCOL, x11.cookie_hex(), x11.screen()).await;
        }
        if params.agent_forwarding {
            // No reply requested: servers that forbid it simply won't open agent channels.
            let _ = write.agent_forward(false).await;
        }
        let size = params.size;
        write
            .request_pty(
                true,
                &params.term,
                size.cols,
                size.rows,
                size.pixel_width,
                size.pixel_height,
                &terminal_modes(),
            )
            .await?;
        expect_reply(&mut read, "a pseudo-terminal", &mut early).await?;

        for (name, value) in &params.env {
            // Servers usually only accept LANG/LC_*; refusals are silent by design.
            let _ = write.set_env(false, name.clone(), value.clone()).await;
        }

        write.request_shell(true).await?;
        expect_reply(&mut read, "a shell", &mut early).await?;

        if let Some(cmd) = params.startup_command.as_ref().filter(|c| !c.trim().is_empty()) {
            write.data_bytes(Bytes::from(format!("{cmd}\r"))).await?;
        }

        let (tx, rx) = mpsc::channel(64);
        let cancel = self.inner.shared.cancel.child_token();
        tokio::spawn(read_loop(read, tx, early, cancel.clone()));
        Ok((ShellHandle { write, cancel }, rx))
    }

    /// Runs one command and collects its output.
    pub async fn exec(&self, command: &str, timeout: Duration) -> Result<ExecOutput> {
        let channel =
            self.inner.handle.channel_open_session().await.map_err(|e| channel_open_error("a command channel", e))?;
        let (mut read, write) = channel.split();
        write.exec(true, command.as_bytes().to_vec()).await?;

        let collect = async {
            let mut out = ExecOutput::default();
            loop {
                match read.wait().await {
                    Some(ChannelMsg::Data { data }) => out.stdout.extend_from_slice(&data),
                    Some(ChannelMsg::ExtendedData { data, .. }) => out.stderr.extend_from_slice(&data),
                    Some(ChannelMsg::ExitStatus { exit_status }) => out.status = Some(exit_status),
                    Some(ChannelMsg::Failure) => {
                        return Err(SshError::ChannelOpen {
                            what: "a command".into(),
                            reason: "the server refused to run it".into(),
                        });
                    }
                    Some(ChannelMsg::Close) | None => break,
                    Some(_) => {}
                }
            }
            Ok(out)
        };
        let result = tokio::time::timeout(timeout, collect).await;
        let _ = write.close().await;
        result.map_err(|_| SshError::Disconnected("the command timed out".into()))?
    }
}

#[derive(Debug, Default, Clone)]
pub struct ExecOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub status: Option<u32>,
}

impl ExecOutput {
    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
}

async fn read_loop(
    mut read: ChannelReadHalf,
    tx: mpsc::Sender<ShellEvent>,
    early: Vec<Bytes>,
    cancel: CancellationToken,
) {
    for data in early {
        if tx.send(ShellEvent::Data(data)).await.is_err() {
            return;
        }
    }
    loop {
        let msg = tokio::select! {
            _ = cancel.cancelled() => break,
            m = read.wait() => m,
        };
        match msg {
            Some(ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. }) => {
                trace!(len = data.len(), "shell output");
                // A full queue means the consumer is behind; waiting here pauses the
                // network read, which is exactly the flow control we want.
                if tx.send(ShellEvent::Data(data)).await.is_err() {
                    break;
                }
            }
            Some(ChannelMsg::ExitStatus { exit_status }) => {
                debug!(exit_status, "shell exited");
                if tx.send(ShellEvent::Exit { status: Some(exit_status), signal: None }).await.is_err() {
                    break;
                }
            }
            Some(ChannelMsg::ExitSignal { signal_name, .. }) => {
                let name = format!("{signal_name:?}");
                if tx.send(ShellEvent::Exit { status: None, signal: Some(name) }).await.is_err() {
                    break;
                }
            }
            Some(ChannelMsg::Close) | None => break,
            Some(_) => {}
        }
    }
    let _ = tx.send(ShellEvent::Closed).await;
}
