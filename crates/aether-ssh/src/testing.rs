//! An in-process SSH + SFTP server for tests.
//!
//! It speaks the real protocol (via russh's server half), so tests exercise genuine key
//! exchange, authentication, channels, port forwarding and SFTP — without needing `sshd`.
//!
//! The "shell" is a tiny emulator: it echoes typed characters, runs a handful of commands
//! (`echo`, `whoami`, `size`, `env`, `flood`, `big`, `bell`, `title`, `exit`) and prints a
//! `$ ` prompt, which is enough to test terminal plumbing deterministically.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use russh::keys::{Algorithm, PrivateKey, PublicKey, PublicKeyBase64};
use russh::server::{self, Auth, Msg, Server as _, Session};
use russh::{Channel, ChannelId, MethodKind, MethodSet, Pty, Sig};
use russh_sftp::protocol::{Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, Status, StatusCode, Version};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

/// One round of a scripted keyboard-interactive login: a hidden prompt and the answer it wants.
#[derive(Clone)]
pub struct KbdStep {
    pub prompt: String,
    pub answer: String,
    /// A round that asks nothing (a banner, a notice): the client answers with no answers, and the login moves on.
    pub info_only: bool,
}

impl KbdStep {
    pub fn new(prompt: &str, answer: &str) -> Self {
        Self { prompt: prompt.into(), answer: answer.into(), info_only: false }
    }

    /// A round with no question in it.
    pub fn info() -> Self {
        Self { prompt: String::new(), answer: String::new(), info_only: true }
    }
}

/// A login the server accepts.
#[derive(Clone, Default)]
pub struct TestUser {
    pub name: String,
    pub password: Option<String>,
    pub authorized_keys: Vec<PublicKey>,
    /// Accepts keyboard-interactive with a single hidden prompt whose answer must match.
    pub kbdint_answer: Option<String>,
    /// A multi-round keyboard-interactive login, the way PAM stacks do it (password, then a code). Takes precedence
    /// over `kbdint_answer`. A wrong answer is asked for again, up to three times, and then refused.
    pub kbdint_steps: Vec<KbdStep>,
    /// After the keyboard-interactive steps the server still wants the password (partial success, RFC 4252 §5.1): the
    /// steps are accepted, and the login isn't over.
    pub kbdint_then_password: bool,
}

impl TestUser {
    pub fn with_password(name: &str, password: &str) -> Self {
        Self { name: name.into(), password: Some(password.into()), ..Default::default() }
    }

    pub fn with_key(name: &str, key: PublicKey) -> Self {
        Self { name: name.into(), authorized_keys: vec![key], ..Default::default() }
    }
}

#[derive(Clone)]
pub struct TestServerOptions {
    pub users: Vec<TestUser>,
    /// Directory served over SFTP. `None` disables the subsystem.
    pub sftp_root: Option<PathBuf>,
    /// File names that appear in every listing although no such file exists: names a real file system would refuse
    /// (parent-folder tricks written with backslashes, drive letters, device names). Each reads as
    /// [`PHANTOM_CONTENT`]. For testing how clients treat a hostile server.
    pub sftp_phantoms: Vec<String>,
    /// Creating a file whose name contains one of these fails — for testing that a failed save leaves the old file.
    pub sftp_refuse_create: Vec<String>,
    /// Renaming something *to* exactly one of these paths fails once — for testing the swap in a save.
    pub sftp_refuse_rename_to: Vec<String>,
    /// Creating a file whose name contains one of these fails with a plain failure rather than a refusal.
    pub sftp_fail_create: Vec<String>,
    /// Files (by name) that belong to someone else: `(name, uid, gid)`. Everything else belongs to the session user
    /// (uid and gid 1000, named `tester`), and so does anything created over SFTP.
    pub sftp_owners: Vec<(String, u32, u32)>,
    /// Whether the session may give files to other users, as root may.
    pub sftp_can_chown: bool,
    /// Renaming something to a path that ends with one of these *happens*, but the answer is a failure — once. A reply
    /// that never arrived, as far as a client can tell.
    pub sftp_lose_rename_reply_to: Vec<String>,
    /// Renaming something *from* exactly one of these paths is refused, as in a sticky folder.
    pub sftp_deny_rename_of: Vec<String>,
    /// How long each write takes to be answered — a slow disk or link, so a transfer can be interrupted part-way.
    pub sftp_write_delay: Duration,
    /// Permission bits reported for files by name (everything else reports 0o644): `(name, mode)`.
    pub sftp_modes: Vec<(String, u32)>,
    /// Every attempt to change a file's attributes is refused, as by a gateway that allows no `setstat`.
    pub sftp_refuse_setstat: bool,
    /// Renaming something *from* exactly one of these paths fails with a plain failure — what a busy file (a bind-mounted
    /// `/etc/hosts`) answers, as opposed to the refusal of `sftp_deny_rename_of`.
    pub sftp_fail_rename_of: Vec<String>,
    /// After the first rename the server can no longer be asked about files (as if the link had died): every stat fails.
    pub sftp_fail_stat_after_rename: bool,
    /// Allow `direct-tcpip` and `tcpip-forward` requests.
    pub allow_forwarding: bool,
    /// After accepting an X11 request, open an `x11` channel back and behave like a remote X client.
    pub x11_client: bool,
    pub host_key: Option<PrivateKey>,
    pub banner: Option<String>,
    pub max_auth_attempts: usize,
}

impl Default for TestServerOptions {
    fn default() -> Self {
        Self {
            users: vec![TestUser::with_password("tester", "secret")],
            sftp_root: None,
            sftp_phantoms: Vec::new(),
            sftp_refuse_create: Vec::new(),
            sftp_refuse_rename_to: Vec::new(),
            sftp_fail_create: Vec::new(),
            sftp_owners: Vec::new(),
            sftp_can_chown: false,
            sftp_lose_rename_reply_to: Vec::new(),
            sftp_deny_rename_of: Vec::new(),
            sftp_write_delay: Duration::ZERO,
            sftp_modes: Vec::new(),
            sftp_refuse_setstat: false,
            sftp_fail_rename_of: Vec::new(),
            sftp_fail_stat_after_rename: false,
            allow_forwarding: true,
            x11_client: false,
            host_key: None,
            banner: None,
            max_auth_attempts: 6,
        }
    }
}

/// Everything the server observed, for assertions.
#[derive(Default, Debug, Clone)]
pub struct Recorded {
    pub connections: u32,
    pub auth_attempts: Vec<String>,
    pub ptys: Vec<(String, u32, u32)>,
    pub window_changes: Vec<(u32, u32)>,
    pub env: Vec<(String, String)>,
    pub exec: Vec<String>,
    pub agent_requests: u32,
    pub chmods: Vec<(String, u32)>,
    /// Files created over SFTP, with the permission bits the client asked for at that moment.
    pub creates: Vec<(String, Option<u32>)>,
    pub forwarded_connections: u32,
    /// `(protocol, cookie, screen)` of each X11 forwarding request.
    pub x11_requests: Vec<(String, String, u32)>,
    /// What the client sent back after the server opened an `x11` channel and acted as a remote X client.
    pub x11_reply: Option<Vec<u8>>,
}

pub struct TestServer {
    pub addr: SocketAddr,
    pub host_key: PrivateKey,
    pub recorded: Arc<Mutex<Recorded>>,
    shutdown: CancellationToken,
}

impl TestServer {
    pub async fn start(options: TestServerOptions) -> TestServer {
        let host_key = options.host_key.clone().unwrap_or_else(|| {
            PrivateKey::random(&mut russh::keys::key::safe_rng(), Algorithm::Ed25519).expect("host key")
        });
        let methods = offered_methods(&options);
        let config = Arc::new(server::Config {
            keys: vec![host_key.clone()],
            methods,
            auth_rejection_time: Duration::from_millis(10),
            auth_rejection_time_initial: Some(Duration::ZERO),
            max_auth_attempts: options.max_auth_attempts,
            inactivity_timeout: Some(Duration::from_secs(120)),
            ..Default::default()
        });

        let recorded = Arc::new(Mutex::new(Recorded::default()));
        let socket = TcpListener::bind("127.0.0.1:0").await.expect("bind test server");
        let addr = socket.local_addr().expect("local addr");
        let shutdown = CancellationToken::new();

        let mut factory = Factory { options: Arc::new(options), recorded: recorded.clone() };
        let stop = shutdown.clone();
        tokio::spawn(async move {
            let running = factory.run_on_socket(config, &socket);
            tokio::select! {
                _ = stop.cancelled() => {}
                _ = running => {}
            }
        });

        TestServer { addr, host_key, recorded, shutdown }
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// Wire-format blob of the server's host key (what known_hosts stores).
    pub fn host_key_blob(&self) -> Vec<u8> {
        self.host_key.public_key().public_key_bytes()
    }

    pub fn recorded(&self) -> Recorded {
        self.recorded.lock().clone()
    }

    pub fn stop(&self) {
        self.shutdown.cancel();
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

/// The authentication methods this server advertises, derived from its users.
fn offered_methods(options: &TestServerOptions) -> MethodSet {
    let mut methods = MethodSet::empty();
    for user in &options.users {
        if user.password.is_some() {
            methods.push(MethodKind::Password);
        }
        if !user.authorized_keys.is_empty() {
            methods.push(MethodKind::PublicKey);
        }
        if user.kbdint_answer.is_some() || !user.kbdint_steps.is_empty() {
            methods.push(MethodKind::KeyboardInteractive);
        }
    }
    methods
}

#[derive(Clone)]
struct Factory {
    options: Arc<TestServerOptions>,
    recorded: Arc<Mutex<Recorded>>,
}

impl server::Server for Factory {
    type Handler = Session_;

    fn new_client(&mut self, _peer: Option<SocketAddr>) -> Session_ {
        self.recorded.lock().connections += 1;
        Session_ {
            options: self.options.clone(),
            recorded: self.recorded.clone(),
            user: None,
            channels: HashMap::new(),
            shells: HashMap::new(),
            sizes: HashMap::new(),
            envs: HashMap::new(),
            kbd_step: 0,
            kbd_misses: 0,
        }
    }
}

struct ShellState {
    line: Vec<u8>,
}

/// One client connection.
struct Session_ {
    options: Arc<TestServerOptions>,
    recorded: Arc<Mutex<Recorded>>,
    user: Option<String>,
    /// Where a scripted keyboard-interactive login stands.
    kbd_step: usize,
    kbd_misses: u32,
    channels: HashMap<ChannelId, Channel<Msg>>,
    shells: HashMap<ChannelId, ShellState>,
    sizes: HashMap<ChannelId, (u32, u32)>,
    envs: HashMap<ChannelId, Vec<(String, String)>>,
}

impl Session_ {
    fn note(&self, what: impl Into<String>) {
        self.recorded.lock().auth_attempts.push(what.into());
    }

    fn find_user(&self, name: &str) -> Option<&TestUser> {
        self.options.users.iter().find(|u| u.name == name)
    }

    /// Like OpenSSH, keep advertising every method after a failed attempt so clients can
    /// retry (russh's default would drop the method that just failed).
    fn reject(&self) -> Auth {
        Auth::Reject { proceed_with_methods: Some(offered_methods(&self.options)), partial_success: false }
    }

    fn run_command(&self, channel: ChannelId, line: &str) -> (Vec<u8>, Option<u32>) {
        let mut parts = line.trim().splitn(2, ' ');
        let cmd = parts.next().unwrap_or("");
        let arg = parts.next().unwrap_or("");
        let user = self.user.clone().unwrap_or_default();
        let text = |s: String| (s.replace('\n', "\r\n").into_bytes(), None);
        match cmd {
            "" => (Vec::new(), None),
            "exit" => (Vec::new(), Some(arg.trim().parse().unwrap_or(0))),
            "echo" => text(format!("{arg}\n")),
            "whoami" => text(format!("{user}\n")),
            "size" => {
                let (c, r) = self.sizes.get(&channel).copied().unwrap_or((0, 0));
                text(format!("{c}x{r}\n"))
            }
            "env" => {
                let value = self
                    .envs
                    .get(&channel)
                    .and_then(|e| e.iter().find(|(k, _)| k == arg.trim()))
                    .map(|(_, v)| v.clone())
                    .unwrap_or_default();
                text(format!("{value}\n"))
            }
            "flood" => {
                let n: usize = arg.trim().parse().unwrap_or(100);
                let mut out = String::new();
                for i in 0..n {
                    out.push_str(&format!("line {i}\r\n"));
                }
                (out.into_bytes(), None)
            }
            "big" => {
                let n: usize = arg.trim().parse().unwrap_or(1024);
                (vec![b'x'; n], None)
            }
            "bell" => (vec![7], None),
            "title" => (format!("\x1b]0;{arg}\x07").into_bytes(), None),
            other => text(format!("{other}: command not found\n")),
        }
    }
}

impl server::Handler for Session_ {
    type Error = russh::Error;

    async fn authentication_banner(&mut self) -> Result<Option<String>, Self::Error> {
        Ok(self.options.banner.clone())
    }

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        self.note(format!("password:{user}"));
        let ok = self.find_user(user).and_then(|u| u.password.as_deref()) == Some(password);
        if ok {
            self.user = Some(user.to_string());
            Ok(Auth::Accept)
        } else {
            Ok(self.reject())
        }
    }

    async fn auth_publickey(&mut self, user: &str, key: &PublicKey) -> Result<Auth, Self::Error> {
        // With the key's fingerprint, so a test can say *which* keys were offered.
        self.note(format!(
            "publickey:{user}:{}",
            aether_core::known_hosts::fingerprint_sha256(&key.public_key_bytes())
        ));
        let ok = self.find_user(user).is_some_and(|u| u.authorized_keys.iter().any(|k| k.key_data() == key.key_data()));
        if ok {
            self.user = Some(user.to_string());
            Ok(Auth::Accept)
        } else {
            Ok(self.reject())
        }
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        user: &str,
        _submethods: &str,
        response: Option<server::Response<'a>>,
    ) -> Result<Auth, Self::Error> {
        self.note(format!("keyboard-interactive:{user}"));
        let steps = self.find_user(user).map(|u| u.kbdint_steps.clone()).unwrap_or_default();
        if !steps.is_empty() {
            let ask = |step: &KbdStep| Auth::Partial {
                name: "PAM".into(),
                instructions: "".into(),
                prompts: if step.info_only { vec![] } else { vec![(step.prompt.clone().into(), false)] }.into(),
            };
            return Ok(match response {
                None => {
                    self.kbd_step = 0;
                    self.kbd_misses = 0;
                    ask(&steps[0])
                }
                Some(mut answers) => {
                    let given = answers.next().map(|b| String::from_utf8_lossy(&b).into_owned());
                    if steps[self.kbd_step].info_only || given.as_deref() == Some(steps[self.kbd_step].answer.as_str())
                    {
                        self.kbd_step += 1;
                        self.kbd_misses = 0;
                        if self.kbd_step >= steps.len() {
                            if self.find_user(user).is_some_and(|u| u.kbdint_then_password) {
                                let mut next = MethodSet::empty();
                                next.push(MethodKind::Password);
                                Auth::Reject { proceed_with_methods: Some(next), partial_success: true }
                            } else {
                                self.user = Some(user.to_string());
                                Auth::Accept
                            }
                        } else {
                            ask(&steps[self.kbd_step])
                        }
                    } else {
                        self.kbd_misses += 1;
                        if self.kbd_misses >= 3 { self.reject() } else { ask(&steps[self.kbd_step]) }
                    }
                }
            });
        }
        let Some(expected) = self.find_user(user).and_then(|u| u.kbdint_answer.clone()) else {
            return Ok(self.reject());
        };
        match response {
            None => Ok(Auth::Partial {
                name: "Two-factor".into(),
                instructions: "".into(),
                prompts: vec![("Verification code: ".into(), false)].into(),
            }),
            Some(mut answers) => {
                let given = answers.next().map(|b| String::from_utf8_lossy(&b).into_owned());
                if given.as_deref() == Some(expected.as_str()) {
                    self.user = Some(user.to_string());
                    Ok(Auth::Accept)
                } else {
                    Ok(self.reject())
                }
            }
        }
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        term: &str,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.recorded.lock().ptys.push((term.to_string(), col_width, row_height));
        self.sizes.insert(channel, (col_width, row_height));
        session.channel_success(channel)?;
        Ok(())
    }

    async fn x11_request(
        &mut self,
        channel: ChannelId,
        _single_connection: bool,
        x11_auth_protocol: &str,
        x11_auth_cookie: &str,
        x11_screen_number: u32,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.recorded.lock().x11_requests.push((
            x11_auth_protocol.to_string(),
            x11_auth_cookie.to_string(),
            x11_screen_number,
        ));
        session.channel_success(channel)?;
        if !self.options.x11_client {
            return Ok(());
        }
        // Play the part of a remote X program: connect back over an `x11` channel and present the
        // cookie the client gave us, exactly as `xeyes` would via the server's xauth entry.
        let handle = session.handle();
        let recorded = self.recorded.clone();
        let cookie_hex = x11_auth_cookie.to_string();
        tokio::spawn(async move {
            let Ok(channel) = handle.channel_open_x11("127.0.0.1", 6000).await else { return };
            let mut stream = channel.into_stream();
            let cookie: Vec<u8> = (0..cookie_hex.len() / 2)
                .filter_map(|i| u8::from_str_radix(&cookie_hex[i * 2..i * 2 + 2], 16).ok())
                .collect();
            let mut setup = vec![b'B', 0, 0, 11, 0, 0, 0, 18, 0, cookie.len() as u8, 0, 0];
            setup.extend(b"MIT-MAGIC-COOKIE-1\0\0");
            setup.extend(&cookie);
            if tokio::io::AsyncWriteExt::write_all(&mut stream, &setup).await.is_err() {
                return;
            }
            let mut buf = [0u8; 64];
            if let Ok(n) = tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await {
                recorded.lock().x11_reply = Some(buf[..n].to_vec());
            }
        });
        Ok(())
    }

    async fn env_request(
        &mut self,
        channel: ChannelId,
        name: &str,
        value: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.recorded.lock().env.push((name.to_string(), value.to_string()));
        self.envs.entry(channel).or_default().push((name.to_string(), value.to_string()));
        session.channel_success(channel)?;
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        channel: ChannelId,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.recorded.lock().window_changes.push((col_width, row_height));
        self.sizes.insert(channel, (col_width, row_height));
        Ok(())
    }

    async fn agent_request(&mut self, _channel: ChannelId, _session: &mut Session) -> Result<bool, Self::Error> {
        self.recorded.lock().agent_requests += 1;
        Ok(true)
    }

    async fn shell_request(&mut self, channel: ChannelId, session: &mut Session) -> Result<(), Self::Error> {
        // The emulator works from `data` callbacks; the channel object isn't needed.
        self.channels.remove(&channel);
        session.channel_success(channel)?;
        self.shells.insert(channel, ShellState { line: Vec::new() });
        session.data(channel, &b"Welcome to the Aether test server\r\n$ "[..])?;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.remove(&channel);
        let command = String::from_utf8_lossy(data).into_owned();
        self.recorded.lock().exec.push(command.clone());
        session.channel_success(channel)?;

        let user = self.user.clone().unwrap_or_default();
        let (stdout, stderr, status): (String, String, u32) = match command.split_once(' ') {
            Some(("echo", rest)) => (format!("{rest}\n"), String::new(), 0),
            _ if command == "whoami" => (format!("{user}\n"), String::new(), 0),
            _ if command == "fail" => (String::new(), "boom\n".into(), 3),
            _ => (format!("ran: {command}\n"), String::new(), 0),
        };
        if !stdout.is_empty() {
            session.data(channel, stdout.into_bytes())?;
        }
        if !stderr.is_empty() {
            session.extended_data(channel, 1, stderr.into_bytes())?;
        }
        session.exit_status_request(channel, status)?;
        session.eof(channel)?;
        session.close(channel)?;
        Ok(())
    }

    async fn data(&mut self, channel: ChannelId, data: &[u8], session: &mut Session) -> Result<(), Self::Error> {
        if !self.shells.contains_key(&channel) {
            return Ok(());
        }
        let mut out: Vec<u8> = Vec::new();
        let mut exit: Option<u32> = None;
        for &b in data {
            match b {
                b'\r' | b'\n' => {
                    out.extend_from_slice(b"\r\n");
                    let line = {
                        let shell = self.shells.get_mut(&channel).expect("checked");
                        String::from_utf8_lossy(&std::mem::take(&mut shell.line)).into_owned()
                    };
                    let (output, code) = self.run_command(channel, &line);
                    out.extend_from_slice(&output);
                    if let Some(code) = code {
                        exit = Some(code);
                        break;
                    }
                    out.extend_from_slice(b"$ ");
                }
                0x03 => {
                    if let Some(shell) = self.shells.get_mut(&channel) {
                        shell.line.clear();
                    }
                    out.extend_from_slice(b"^C\r\n$ ");
                }
                0x7f | 0x08 => {
                    if let Some(shell) = self.shells.get_mut(&channel)
                        && shell.line.pop().is_some()
                    {
                        out.extend_from_slice(b"\x08 \x08");
                    }
                }
                other => {
                    if let Some(shell) = self.shells.get_mut(&channel) {
                        shell.line.push(other);
                    }
                    out.push(other);
                }
            }
        }
        if !out.is_empty() {
            session.data(channel, out)?;
        }
        if let Some(code) = exit {
            self.shells.remove(&channel);
            session.exit_status_request(channel, code)?;
            session.eof(channel)?;
            session.close(channel)?;
        }
        Ok(())
    }

    async fn signal(&mut self, _channel: ChannelId, _signal: Sig, _session: &mut Session) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let Some(root) = self.options.sftp_root.clone().filter(|_| name == "sftp") else {
            session.channel_failure(channel)?;
            return Ok(());
        };
        let Some(chan) = self.channels.remove(&channel) else {
            session.channel_failure(channel)?;
            return Ok(());
        };
        session.channel_success(channel)?;
        let handler = SftpFs::new(root, self.recorded.clone(), self.options.clone());
        tokio::spawn(async move {
            russh_sftp::server::run(chan.into_stream(), handler).await;
        });
        Ok(())
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host_to_connect: &str,
        port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if !self.options.allow_forwarding {
            reply.reject(russh::ChannelOpenFailure::AdministrativelyProhibited).await;
            return Ok(());
        }
        let target = format!("{host_to_connect}:{port_to_connect}");
        let recorded = self.recorded.clone();
        tokio::spawn(async move {
            match tokio::net::TcpStream::connect(&target).await {
                Ok(mut tcp) => {
                    recorded.lock().forwarded_connections += 1;
                    reply.accept().await;
                    let mut ch = channel.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut tcp, &mut ch).await;
                }
                Err(_) => reply.reject(russh::ChannelOpenFailure::ConnectFailed).await,
            }
        });
        Ok(())
    }

    async fn tcpip_forward(
        &mut self,
        address: &str,
        port: &mut u32,
        session: &mut Session,
    ) -> Result<bool, Self::Error> {
        if !self.options.allow_forwarding {
            return Ok(false);
        }
        let Ok(listener) = TcpListener::bind(("127.0.0.1", *port as u16)).await else {
            return Ok(false);
        };
        let actual = listener.local_addr().map(|a| a.port()).unwrap_or(0);
        *port = u32::from(actual);
        let handle = session.handle();
        let address = address.to_string();
        let recorded = self.recorded.clone();
        tokio::spawn(async move {
            while let Ok((tcp, peer)) = listener.accept().await {
                let handle = handle.clone();
                let address = address.clone();
                let recorded = recorded.clone();
                tokio::spawn(async move {
                    let Ok(channel) = handle
                        .channel_open_forwarded_tcpip(
                            address,
                            u32::from(actual),
                            peer.ip().to_string(),
                            u32::from(peer.port()),
                        )
                        .await
                    else {
                        return;
                    };
                    recorded.lock().forwarded_connections += 1;
                    let mut tcp = tcp;
                    let mut ch = channel.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut tcp, &mut ch).await;
                });
            }
        });
        Ok(true)
    }
}

// ── SFTP over a real directory ───────────────────────────────────────────────

enum OpenHandle {
    Dir {
        entries: Vec<File>,
        sent: bool,
    },
    File(std::fs::File),
    /// A phantom file: its bytes are [`PHANTOM_CONTENT`].
    Memory,
}

/// What every phantom file contains.
pub const PHANTOM_CONTENT: &[u8] = b"phantom";

struct SftpFs {
    root: PathBuf,
    handles: HashMap<String, OpenHandle>,
    next: u64,
    recorded: Arc<Mutex<Recorded>>,
    phantoms: Vec<String>,
    refuse_create: Vec<String>,
    refuse_rename_to: Vec<String>,
    fail_create: Vec<String>,
    foreign_owners: Vec<(String, u32, u32)>,
    can_chown: bool,
    lose_rename_reply_to: Vec<String>,
    deny_rename_of: Vec<String>,
    write_delay: Duration,
    modes: Vec<(String, u32)>,
    refuse_setstat: bool,
    fail_rename_of: Vec<String>,
    fail_stat_after_rename: bool,
    /// Has any rename happened yet?
    renamed: bool,
    /// Who owns what, once something has been created, moved or given away.
    owners: HashMap<PathBuf, (u32, u32)>,
}

/// Who the session is, and so who owns whatever it creates.
const SESSION_OWNER: (u32, u32) = (1000, 1000);

fn ok(id: u32) -> Status {
    Status { id, status_code: StatusCode::Ok, error_message: "Ok".into(), language_tag: "en-US".into() }
}

fn status_for(e: &std::io::Error) -> StatusCode {
    match e.kind() {
        std::io::ErrorKind::NotFound => StatusCode::NoSuchFile,
        std::io::ErrorKind::PermissionDenied => StatusCode::PermissionDenied,
        _ => StatusCode::Failure,
    }
}

impl SftpFs {
    fn new(root: PathBuf, recorded: Arc<Mutex<Recorded>>, options: Arc<TestServerOptions>) -> Self {
        Self {
            root,
            handles: HashMap::new(),
            next: 1,
            recorded,
            phantoms: options.sftp_phantoms.clone(),
            refuse_create: options.sftp_refuse_create.clone(),
            refuse_rename_to: options.sftp_refuse_rename_to.clone(),
            fail_create: options.sftp_fail_create.clone(),
            foreign_owners: options.sftp_owners.clone(),
            can_chown: options.sftp_can_chown,
            lose_rename_reply_to: options.sftp_lose_rename_reply_to.clone(),
            deny_rename_of: options.sftp_deny_rename_of.clone(),
            write_delay: options.sftp_write_delay,
            modes: options.sftp_modes.clone(),
            refuse_setstat: options.sftp_refuse_setstat,
            fail_rename_of: options.sftp_fail_rename_of.clone(),
            fail_stat_after_rename: options.sftp_fail_stat_after_rename,
            renamed: false,
            owners: HashMap::new(),
        }
    }

    /// Who owns the file at `real`: whoever it was given to or created by, else someone else for the names listed in
    /// the options, else the session user.
    fn owner_of(&self, real: &Path) -> (u32, u32) {
        if let Some(owner) = self.owners.get(real) {
            return *owner;
        }
        let name = real.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.foreign_owners.iter().find(|(n, ..)| *n == name).map(|(_, uid, gid)| (*uid, *gid)).unwrap_or(SESSION_OWNER)
    }

    /// A file's attributes as this server reports them.
    fn attrs_for(&self, real: &Path, meta: &std::fs::Metadata) -> FileAttributes {
        let mut a = Self::attrs_of(meta);
        let (uid, gid) = self.owner_of(real);
        a.uid = Some(uid);
        a.gid = Some(gid);
        if (uid, gid) != SESSION_OWNER {
            a.user = None;
            a.group = None;
        }
        if !meta.is_dir() {
            let name = real.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if let Some((_, mode)) = self.modes.iter().find(|(n, _)| *n == name) {
                a.permissions = Some((mode & 0o7777) | 0o100_000);
            }
        }
        a
    }

    /// Is `path` one of the phantom names (its last `/`-separated component, taken literally)?
    fn is_phantom(&self, path: &str) -> bool {
        let name = path.rsplit('/').next().unwrap_or("");
        self.phantoms.iter().any(|p| p == name)
    }

    fn phantom_attrs() -> FileAttributes {
        FileAttributes {
            size: Some(PHANTOM_CONTENT.len() as u64),
            permissions: Some(0o644 | 0o100_000),
            user: Some("tester".into()),
            group: Some("tester".into()),
            ..FileAttributes::default()
        }
    }

    /// Maps a client path onto the served directory, never escaping it.
    fn real(&self, path: &str) -> PathBuf {
        let mut out = self.root.clone();
        let mut depth = 0usize;
        for comp in Path::new(path.trim_start_matches('/')).components() {
            match comp {
                Component::Normal(c) => {
                    out.push(c);
                    depth += 1;
                }
                Component::ParentDir if depth > 0 => {
                    out.pop();
                    depth -= 1;
                }
                _ => {}
            }
        }
        out
    }

    fn virtual_path(&self, real: &Path) -> String {
        let rel = real.strip_prefix(&self.root).unwrap_or(Path::new(""));
        let s = rel.to_string_lossy().replace('\\', "/");
        format!("/{}", s.trim_start_matches('/'))
    }

    fn handle_id(&mut self) -> String {
        self.next += 1;
        format!("h{}", self.next)
    }

    fn attrs_of(meta: &std::fs::Metadata) -> FileAttributes {
        let mut a = FileAttributes::from(meta);
        if meta.is_dir() {
            a.permissions = Some(0o755 | 0o040_000);
        } else {
            a.permissions = Some(0o644 | 0o100_000);
        }
        a.user = Some("tester".into());
        a.group = Some("tester".into());
        a
    }
}

impl russh_sftp::server::Handler for SftpFs {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    async fn init(&mut self, _version: u32, _ext: HashMap<String, String>) -> Result<Version, Self::Error> {
        Ok(Version::new())
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        let real = self.real(&path);
        Ok(Name { id, files: vec![File::dummy(self.virtual_path(&real))] })
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, Self::Error> {
        let real = self.real(&path);
        let rd = std::fs::read_dir(&real).map_err(|e| status_for(&e))?;
        let mut entries = Vec::new();
        for entry in rd.flatten() {
            if let Ok(meta) = entry.metadata() {
                entries.push(File::new(
                    entry.file_name().to_string_lossy().into_owned(),
                    self.attrs_for(&entry.path(), &meta),
                ));
            }
        }
        for name in &self.phantoms {
            entries.push(File::new(name.clone(), Self::phantom_attrs()));
        }
        let handle = self.handle_id();
        self.handles.insert(handle.clone(), OpenHandle::Dir { entries, sent: false });
        Ok(Handle { id, handle })
    }

    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, Self::Error> {
        match self.handles.get_mut(&handle) {
            Some(OpenHandle::Dir { entries, sent }) if !*sent => {
                *sent = true;
                Ok(Name { id, files: std::mem::take(entries) })
            }
            Some(OpenHandle::Dir { .. }) => Err(StatusCode::Eof),
            _ => Err(StatusCode::Failure),
        }
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, Self::Error> {
        self.handles.remove(&handle);
        Ok(ok(id))
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        pflags: OpenFlags,
        attrs: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        if self.is_phantom(&filename) {
            let handle = self.handle_id();
            self.handles.insert(handle.clone(), OpenHandle::Memory);
            return Ok(Handle { id, handle });
        }
        if pflags.contains(OpenFlags::CREATE) && self.refuse_create.iter().any(|p| filename.contains(p.as_str())) {
            return Err(StatusCode::PermissionDenied);
        }
        if pflags.contains(OpenFlags::CREATE) && self.fail_create.iter().any(|p| filename.contains(p.as_str())) {
            return Err(StatusCode::Failure);
        }
        if pflags.contains(OpenFlags::CREATE) {
            self.recorded.lock().creates.push((filename.clone(), attrs.permissions.map(|m| m & 0o7777)));
        }
        let real = self.real(&filename);
        let existed = real.exists();
        let opts: std::fs::OpenOptions = pflags.into();
        let file = opts.open(&real).map_err(|e| status_for(&e))?;
        if !existed {
            // Whatever the session creates is its own.
            self.owners.insert(real, SESSION_OWNER);
        }
        let handle = self.handle_id();
        self.handles.insert(handle.clone(), OpenHandle::File(file));
        Ok(Handle { id, handle })
    }

    async fn read(&mut self, id: u32, handle: String, offset: u64, len: u32) -> Result<Data, Self::Error> {
        use std::io::{Read, Seek, SeekFrom};
        if matches!(self.handles.get(&handle), Some(OpenHandle::Memory)) {
            let start = (offset as usize).min(PHANTOM_CONTENT.len());
            let end = (start + len as usize).min(PHANTOM_CONTENT.len());
            if start == end {
                return Err(StatusCode::Eof);
            }
            return Ok(Data { id, data: PHANTOM_CONTENT[start..end].to_vec() });
        }
        let Some(OpenHandle::File(file)) = self.handles.get_mut(&handle) else {
            return Err(StatusCode::Failure);
        };
        file.seek(SeekFrom::Start(offset)).map_err(|e| status_for(&e))?;
        let mut buf = vec![0u8; len as usize];
        let n = file.read(&mut buf).map_err(|e| status_for(&e))?;
        if n == 0 {
            return Err(StatusCode::Eof);
        }
        buf.truncate(n);
        Ok(Data { id, data: buf })
    }

    async fn write(&mut self, id: u32, handle: String, offset: u64, data: Vec<u8>) -> Result<Status, Self::Error> {
        use std::io::{Seek, SeekFrom, Write};
        if !self.write_delay.is_zero() {
            tokio::time::sleep(self.write_delay).await;
        }
        let Some(OpenHandle::File(file)) = self.handles.get_mut(&handle) else {
            return Err(StatusCode::Failure);
        };
        file.seek(SeekFrom::Start(offset)).map_err(|e| status_for(&e))?;
        file.write_all(&data).map_err(|e| status_for(&e))?;
        Ok(ok(id))
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        if self.fail_stat_after_rename && self.renamed {
            return Err(StatusCode::Failure);
        }
        if self.is_phantom(&path) {
            return Ok(Attrs { id, attrs: Self::phantom_attrs() });
        }
        let real = self.real(&path);
        let meta = std::fs::metadata(&real).map_err(|e| status_for(&e))?;
        Ok(Attrs { id, attrs: self.attrs_for(&real, &meta) })
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        if self.fail_stat_after_rename && self.renamed {
            return Err(StatusCode::Failure);
        }
        if self.is_phantom(&path) {
            return Ok(Attrs { id, attrs: Self::phantom_attrs() });
        }
        let real = self.real(&path);
        let meta = std::fs::symlink_metadata(&real).map_err(|e| status_for(&e))?;
        Ok(Attrs { id, attrs: self.attrs_for(&real, &meta) })
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, Self::Error> {
        if matches!(self.handles.get(&handle), Some(OpenHandle::Memory)) {
            return Ok(Attrs { id, attrs: Self::phantom_attrs() });
        }
        let Some(OpenHandle::File(file)) = self.handles.get(&handle) else {
            return Err(StatusCode::Failure);
        };
        let meta = file.metadata().map_err(|e| status_for(&e))?;
        Ok(Attrs { id, attrs: Self::attrs_of(&meta) })
    }

    async fn setstat(&mut self, id: u32, path: String, attrs: FileAttributes) -> Result<Status, Self::Error> {
        if self.refuse_setstat {
            return Err(StatusCode::PermissionDenied);
        }
        if let Some(mode) = attrs.permissions {
            self.recorded.lock().chmods.push((path.clone(), mode & 0o7777));
        }
        let real = self.real(&path);
        if !real.exists() {
            return Err(StatusCode::NoSuchFile);
        }
        if let (Some(uid), Some(gid)) = (attrs.uid, attrs.gid) {
            // Only root may give a file away.
            if !self.can_chown && (uid, gid) != self.owner_of(&real) {
                return Err(StatusCode::PermissionDenied);
            }
            self.owners.insert(real, (uid, gid));
        }
        Ok(ok(id))
    }

    async fn mkdir(&mut self, id: u32, path: String, _attrs: FileAttributes) -> Result<Status, Self::Error> {
        std::fs::create_dir(self.real(&path)).map_err(|e| status_for(&e))?;
        Ok(ok(id))
    }

    async fn rmdir(&mut self, id: u32, path: String) -> Result<Status, Self::Error> {
        std::fs::remove_dir(self.real(&path)).map_err(|e| status_for(&e))?;
        Ok(ok(id))
    }

    async fn remove(&mut self, id: u32, filename: String) -> Result<Status, Self::Error> {
        std::fs::remove_file(self.real(&filename)).map_err(|e| status_for(&e))?;
        Ok(ok(id))
    }

    async fn rename(&mut self, id: u32, oldpath: String, newpath: String) -> Result<Status, Self::Error> {
        // Like OpenSSH's sftp-server, plain SSH_FXP_RENAME will not replace something that is already there. (Where
        // the local file system would — Windows' `rename` does — a test would pass for a reason real servers don't
        // share.)
        let (from, to) = (self.real(&oldpath), self.real(&newpath));
        if self.deny_rename_of.contains(&oldpath) {
            return Err(StatusCode::PermissionDenied);
        }
        if self.fail_rename_of.contains(&oldpath) {
            return Err(StatusCode::Failure);
        }
        if let Some(i) = self.refuse_rename_to.iter().position(|p| *p == newpath) {
            self.refuse_rename_to.remove(i); // once: whatever tries again afterwards gets through
            return Err(StatusCode::Failure);
        }
        if to.symlink_metadata().is_ok() {
            return Err(StatusCode::Failure);
        }
        let owner = self.owner_of(&from);
        std::fs::rename(&from, &to).map_err(|e| status_for(&e))?;
        self.renamed = true;
        // A file keeps its owner when it moves.
        self.owners.remove(&from);
        self.owners.insert(to, owner);
        if let Some(i) = self.lose_rename_reply_to.iter().position(|p| newpath.ends_with(p.as_str())) {
            self.lose_rename_reply_to.remove(i);
            return Err(StatusCode::Failure);
        }
        Ok(ok(id))
    }
}

// ── convenience for tests ────────────────────────────────────────────────────

/// A delegate that answers prompts from a script.
pub mod delegate {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use aether_vault::Secret;
    use async_trait::async_trait;
    use parking_lot::Mutex;

    use crate::delegate::*;

    #[derive(Clone)]
    pub struct ScriptedDelegate {
        pub host_key: HostKeyDecision,
        /// The answer to a "key changed" question, when it should differ from `host_key`.
        pub host_key_on_change: Option<HostKeyDecision>,
        /// How long the "person" takes to answer the host-key question.
        pub host_key_delay: std::time::Duration,
        /// Every host-key question asked, in order.
        pub host_key_requests: Arc<Mutex<Vec<HostKeyRequest>>>,
        pub answers: Arc<Mutex<Vec<String>>>,
        pub remember: bool,
        pub saved_password: Arc<Mutex<Option<String>>>,
        pub vault_keys: Arc<Mutex<Vec<(String, String)>>>,
        pub host_key_prompts: Arc<AtomicUsize>,
        pub prompts: Arc<Mutex<Vec<PromptRequest>>>,
        pub progress: Arc<Mutex<Vec<ConnectStage>>>,
        pub banners: Arc<Mutex<Vec<String>>>,
        pub remembered: Arc<Mutex<Vec<(SecretLookup, String)>>>,
        pub forgotten: Arc<AtomicUsize>,
    }

    impl Default for ScriptedDelegate {
        fn default() -> Self {
            Self {
                host_key: HostKeyDecision::TrustAndSave,
                host_key_on_change: None,
                host_key_delay: std::time::Duration::ZERO,
                host_key_requests: Arc::default(),
                answers: Arc::default(),
                remember: false,
                saved_password: Arc::default(),
                vault_keys: Arc::default(),
                host_key_prompts: Arc::default(),
                prompts: Arc::default(),
                progress: Arc::default(),
                banners: Arc::default(),
                remembered: Arc::default(),
                forgotten: Arc::default(),
            }
        }
    }

    impl ScriptedDelegate {
        pub fn answering(answers: &[&str]) -> Self {
            let d = Self::default();
            *d.answers.lock() = answers.iter().rev().map(|s| s.to_string()).collect();
            d
        }
    }

    #[async_trait]
    impl ConnectionDelegate for ScriptedDelegate {
        async fn verify_host_key(&self, request: HostKeyRequest) -> HostKeyDecision {
            self.host_key_prompts.fetch_add(1, Ordering::SeqCst);
            let changed = matches!(request.kind, HostKeyRequestKind::Changed { .. });
            self.host_key_requests.lock().push(request);
            if !self.host_key_delay.is_zero() {
                tokio::time::sleep(self.host_key_delay).await;
            }
            match self.host_key_on_change {
                Some(decision) if changed => decision,
                _ => self.host_key,
            }
        }

        async fn prompt(&self, request: PromptRequest) -> Option<PromptResponse> {
            let count = match &request.kind {
                PromptKind::KeyboardInteractive { prompts, .. } => prompts.len().max(1),
                _ => 1,
            };
            self.prompts.lock().push(request);
            let mut answers = self.answers.lock();
            let mut out = Vec::new();
            for _ in 0..count {
                out.push(Secret::from_text(&answers.pop()?));
            }
            Some(PromptResponse { answers: out, remember: self.remember })
        }

        fn saved_secret(&self, lookup: &SecretLookup) -> Option<Secret> {
            match lookup.kind {
                SecretKind::Password => self.saved_password.lock().as_deref().map(Secret::from_text),
                _ => None,
            }
        }

        fn remember_secret(&self, lookup: &SecretLookup, secret: &Secret) {
            self.remembered.lock().push((lookup.clone(), secret.expose_text().unwrap_or_default().to_string()));
        }

        fn forget_secret(&self, _lookup: &SecretLookup) {
            self.forgotten.fetch_add(1, Ordering::SeqCst);
            *self.saved_password.lock() = None;
        }

        fn progress(&self, progress: ConnectProgress) {
            self.progress.lock().push(progress.stage);
        }

        fn banner(&self, _hop: &HopInfo, text: &str) {
            self.banners.lock().push(text.to_string());
        }

        fn vault_key(&self, key_id: &str) -> Option<Secret> {
            self.vault_keys.lock().iter().find(|(id, _)| id == key_id).map(|(_, text)| Secret::from_text(text))
        }
    }
}
