//! Byte streams SSH runs over: plain TCP, or a `ProxyCommand` child process.

use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use parking_lot::Mutex;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, BufReader, ReadBuf};
use tokio::net::TcpStream;
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::time::{Instant, timeout_at};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::error::{Result, SshError};

/// Resolves `host` and connects, trying every address within `timeout` overall.
pub(crate) async fn connect_tcp(
    host: &str,
    port: u16,
    timeout: Duration,
    nodelay: bool,
    cancel: &CancellationToken,
) -> Result<TcpStream> {
    let deadline = Instant::now() + timeout;

    let addrs: Vec<SocketAddr> = tokio::select! {
        _ = cancel.cancelled() => return Err(SshError::Cancelled),
        r = timeout_at(deadline, tokio::net::lookup_host((host, port))) => match r {
            Err(_) => return Err(SshError::Dns {
                host: host.into(),
                message: "the DNS lookup timed out".into(),
            }),
            Ok(Err(e)) => return Err(SshError::Dns { host: host.into(), message: e.to_string() }),
            Ok(Ok(it)) => it.collect(),
        }
    };
    if addrs.is_empty() {
        return Err(SshError::Dns { host: host.into(), message: "no addresses were returned".into() });
    }

    let mut last_err: Option<SshError> = None;
    for (i, addr) in addrs.iter().enumerate() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        // Leave time for the remaining addresses, but never less than 2 s per attempt.
        let left = (addrs.len() - i) as u32;
        let per_attempt = (remaining / left).max(Duration::from_secs(2)).min(remaining);

        let attempt = tokio::select! {
            _ = cancel.cancelled() => return Err(SshError::Cancelled),
            r = tokio::time::timeout(per_attempt, TcpStream::connect(addr)) => r,
        };
        match attempt {
            Ok(Ok(stream)) => {
                if nodelay {
                    let _ = stream.set_nodelay(true);
                }
                debug!(%addr, "tcp connected");
                return Ok(stream);
            }
            Ok(Err(e)) => {
                debug!(%addr, error = %e, "tcp connect failed");
                let err = SshError::from_io(e, host, port);
                // A refusal is more informative than a timeout from another address.
                if !matches!(last_err, Some(SshError::ConnectionRefused { .. })) {
                    last_err = Some(err);
                }
            }
            Err(_) => {
                debug!(%addr, "tcp connect timed out");
                if last_err.is_none() {
                    last_err = Some(SshError::ConnectTimeout { host: host.into(), port });
                }
            }
        }
    }
    Err(last_err.unwrap_or(SshError::ConnectTimeout { host: host.into(), port }))
}

/// SSH over the stdin/stdout of a child process (`ProxyCommand`).
pub(crate) struct ProxyStream {
    stdin: ChildStdin,
    stdout: ChildStdout,
    // Held so the process is killed when the stream is dropped (kill_on_drop).
    _child: Child,
    /// Last lines the command printed on stderr, for diagnosing failures.
    pub(crate) stderr_tail: Arc<Mutex<String>>,
}

impl AsyncRead for ProxyStream {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stdout).poll_read(cx, buf)
    }
}

impl AsyncWrite for ProxyStream {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.stdin).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stdin).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stdin).poll_shutdown(cx)
    }
}

/// A ProxyCommand that relays stdin/stdout to `%h:%p`, like `nc` or `ssh -W` (tests only).
#[cfg(all(test, windows))]
pub(crate) fn tcp_relay_command() -> String {
    let script = "$c=[Net.Sockets.TcpClient]::new('%h',%p);$s=$c.GetStream();\
                  $i=[Console]::OpenStandardInput();$o=[Console]::OpenStandardOutput();\
                  $a=$i.CopyToAsync($s);$b=$s.CopyToAsync($o);\
                  [Threading.Tasks.Task]::WaitAny(@($a,$b))|Out-Null";
    format!("powershell -NoProfile -NonInteractive -Command \"{script}\"")
}

/// What the tokens of a ProxyCommand stand for.
pub(crate) struct ProxyTokens<'a> {
    /// `%h`: the host being reached.
    pub host: &'a str,
    /// `%p`: its port.
    pub port: u16,
    /// `%r`: the user it signs in as.
    pub user: &'a str,
    /// `%n`: what the person called it — what they would have typed after `ssh`.
    pub name: &'a str,
    /// `%u`: who is running Aether.
    pub local_user: &'a str,
    /// `%d`: that person's home folder.
    pub home: &'a str,
}

/// Expands `%h`, `%p`, `%r`, `%n`, `%u`, `%d` and `%%` in a ProxyCommand, in one pass: what is substituted is never
/// scanned again, so a value can't introduce a token of its own. Anything else after a `%` is left as it was typed.
///
/// The command line is read by a shell, so what goes into it from the host's side — its address, the user name, the name
/// it was given — must be nothing but a name. Anything else — `&`, `;`, quotes, `%`, a leading `-` — is refused rather
/// than escaped: escaping differs between `cmd.exe`, `sh` and whatever program the command starts, and there is no
/// legitimate host name that needs it. (The person's own user name and home folder come from their own machine, not from
/// a config file; only control characters are refused there.)
pub(crate) fn expand_proxy_command(template: &str, t: &ProxyTokens<'_>) -> Result<String> {
    let name = |what: &str, value: &str| -> Result<()> {
        if aether_core::util::is_command_safe_hostname(value) {
            Ok(())
        } else {
            Err(SshError::Invalid(format!(
                "“{value}” can't be handed to a proxy command as {what}: it may only contain ASCII letters, digits, dots, dashes, underscores and colons (use the punycode form for international names)"
            )))
        }
    };
    let local = |what: &str, value: &str| -> Result<()> {
        if value.chars().any(char::is_control) {
            Err(SshError::Invalid(format!(
                "{what} contains a control character, so it can't be handed to a proxy command"
            )))
        } else {
            Ok(())
        }
    };

    let mut out = String::with_capacity(template.len() + 16);
    let mut chars = template.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('h') => {
                name("the host name", t.host)?;
                out.push_str(t.host)
            }
            Some('p') => out.push_str(&t.port.to_string()),
            Some('r') => {
                name("the user name", t.user)?;
                out.push_str(t.user)
            }
            Some('n') => {
                name("the host's name", t.name)?;
                out.push_str(t.name)
            }
            Some('u') => {
                local("Your user name", t.local_user)?;
                out.push_str(t.local_user)
            }
            Some('d') => {
                local("Your home folder", t.home)?;
                out.push_str(t.home)
            }
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    Ok(out)
}

/// Where a proxy command starts. Not the app's own folder or whatever the process inherited: on Windows `cmd.exe`
/// looks for programs in the current directory first, and that must not be a place anyone else can drop an `ssh.exe`.
fn proxy_working_dir() -> std::path::PathBuf {
    aether_core::util::home_dir().filter(|d| d.is_dir()).unwrap_or_else(std::env::temp_dir)
}

pub(crate) fn spawn_proxy(template: &str, tokens: &ProxyTokens<'_>) -> Result<ProxyStream> {
    spawn_proxy_in(&proxy_working_dir(), template, tokens)
}

fn spawn_proxy_in(dir: &std::path::Path, template: &str, tokens: &ProxyTokens<'_>) -> Result<ProxyStream> {
    let command_line = expand_proxy_command(template, tokens)?;
    debug!(command = %command_line, "starting proxy command");

    #[cfg(windows)]
    let mut cmd = {
        let mut c = Command::new("cmd.exe");
        // `raw_arg` keeps the user's quoting intact; the extra outer quotes follow cmd's /C rules.
        c.raw_arg("/D").raw_arg("/S").raw_arg("/C").raw_arg(format!("\"{command_line}\""));
        // CREATE_NO_WINDOW: don't flash a console window.
        c.creation_flags(0x0800_0000);
        // Don't search the current directory for the program (cmd.exe honours this, as does CreateProcess).
        c.env("NoDefaultCurrentDirectoryInExePath", "1");
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = Command::new("sh");
        c.arg("-c").arg(&command_line);
        c
    };
    cmd.current_dir(dir);

    let mut child = cmd
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| SshError::ProxyCommand(format!("could not start `{command_line}`: {e}")))?;

    let stdin = child.stdin.take().ok_or_else(|| SshError::ProxyCommand("no stdin".into()))?;
    let stdout = child.stdout.take().ok_or_else(|| SshError::ProxyCommand("no stdout".into()))?;
    let tail = Arc::new(Mutex::new(String::new()));
    if let Some(stderr) = child.stderr.take() {
        let tail = tail.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                warn!(target: "aether_ssh::proxy", "{line}");
                let mut t = tail.lock();
                if t.len() > 2048 {
                    let cut = t.len() - 1024;
                    let boundary = (cut..t.len()).find(|i| t.is_char_boundary(*i)).unwrap_or(t.len());
                    t.drain(..boundary);
                }
                t.push_str(&line);
                t.push('\n');
            }
        });
    }
    Ok(ProxyStream { stdin, stdout, _child: child, stderr_tail: tail })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn tokens<'a>(host: &'a str, port: u16) -> ProxyTokens<'a> {
        ProxyTokens { host, port, user: "svc", name: "db-alias", local_user: "me", home: "C:/Users/me" }
    }

    #[test]
    fn proxy_tokens_expand() {
        assert_eq!(
            expand_proxy_command("ssh -W %h:%p jump && echo 100%%", &tokens("db", 2222)).unwrap(),
            "ssh -W db:2222 jump && echo 100%"
        );
        // Every token OpenSSH offers for this option has a value.
        assert_eq!(
            expand_proxy_command("x %h %p %r %n %u %d", &tokens("db", 22)).unwrap(),
            "x db 22 svc db-alias me C:/Users/me"
        );
        // Unknown tokens are left as they are, and a template that never mentions %h doesn't care about the name.
        assert_eq!(expand_proxy_command("echo %x %", &tokens("db", 22)).unwrap(), "echo %x %");
        assert_eq!(expand_proxy_command("jumpscript.cmd", &tokens("anything & goes", 22)).unwrap(), "jumpscript.cmd");
    }

    /// Whatever the host's side supplies lands inside a command line: only names get in, and nothing that is
    /// substituted is expanded again.
    #[test]
    fn proxy_commands_never_receive_shell_syntax_from_the_hosts_side() {
        for evil in [
            "db & calc",
            "db;rm -rf ~",
            "db|more",
            "db`x`",
            "$(x)",
            "db\"x",
            "db'x",
            "db>out",
            "db<in",
            "db^x",
            "%PATH%",
            "-oProxyCommand=x",
            "db
x",
            "db x",
            "",
            "münchen.example",
            "host＆calc",
            "a%pb",
        ] {
            let result = expand_proxy_command("ssh -W %h:%p jump", &tokens(evil, 22));
            assert!(matches!(result, Err(SshError::Invalid(_))), "{evil:?} as the host -> {result:?}");
            let user = ProxyTokens { user: evil, ..tokens("db", 22) };
            let result = expand_proxy_command("ssh -l %r jump", &user);
            assert!(matches!(result, Err(SshError::Invalid(_))), "{evil:?} as the user -> {result:?}");
            let name = ProxyTokens { name: evil, ..tokens("db", 22) };
            let result = expand_proxy_command("echo %n", &name);
            assert!(matches!(result, Err(SshError::Invalid(_))), "{evil:?} as the name -> {result:?}");
        }
        // A literal percent sign in the *template* is still written with %%; the substituted value is not rescanned.
        assert_eq!(expand_proxy_command("echo %h %%p", &tokens("db", 7)).unwrap(), "echo db %p");
        // Real names and addresses pass.
        for good in ["db.example.com", "10.0.0.5", "fe80::1", "xn--mnchen-3ya.example", "web_01-a"] {
            assert!(expand_proxy_command("nc %h %p", &tokens(good, 22)).is_ok(), "{good}");
        }
        // The person's own name and folder are theirs to have odd characters in (a space, a backslash)...
        let mine = ProxyTokens { local_user: "Jo Smith", home: r"C:\Users\Jo Smith", ..tokens("db", 22) };
        assert_eq!(expand_proxy_command("%u %d", &mine).unwrap(), r"Jo Smith C:\Users\Jo Smith");
        // ...but not control characters.
        let odd = ProxyTokens {
            local_user: "me
x",
            ..tokens("db", 22)
        };
        assert!(expand_proxy_command("%u", &odd).is_err());
    }

    async fn output_of(command: &str) -> String {
        let mut stream = spawn_proxy(command, &tokens("127.0.0.1", 22)).unwrap();
        let mut out = String::new();
        tokio::time::timeout(Duration::from_secs(10), stream.read_to_string(&mut out))
            .await
            .expect("the command finishes")
            .unwrap();
        out.trim().to_string()
    }

    /// A proxy command starts in the user's own folder, not wherever the app happened to be launched from — on Windows
    /// `cmd.exe` would otherwise run an `ssh.exe` that someone left in that folder.
    #[tokio::test]
    async fn proxy_commands_start_in_the_users_own_folder() {
        let printed = output_of(if cfg!(windows) { "cd" } else { "pwd" }).await;
        let home = aether_core::util::home_dir().unwrap();
        assert_eq!(
            std::path::PathBuf::from(&printed).canonicalize().unwrap(),
            home.canonicalize().unwrap(),
            "ran in {printed}"
        );
    }

    /// A program that someone left in the folder the command starts in must not be what runs. (Without the setting
    /// `cmd.exe` looks in the current folder first, which the first half of this test shows.)
    #[cfg(windows)]
    #[tokio::test]
    async fn a_program_left_in_the_current_folder_is_not_run() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("planted.bat"),
            "@echo planted-ran
",
        )
        .unwrap();

        // (The variable may already be set where the tests run; the control must not have it.)
        let control = std::process::Command::new("cmd.exe")
            .args(["/D", "/S", "/C", "planted"])
            .env_remove("NoDefaultCurrentDirectoryInExePath")
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&control.stdout).contains("planted-ran"),
            "the planted file would have been found"
        );

        let mut stream = spawn_proxy_in(dir.path(), "planted", &tokens("db", 22)).unwrap();
        let mut out = String::new();
        let _ = tokio::time::timeout(Duration::from_secs(10), stream.read_to_string(&mut out)).await;
        assert!(!out.contains("planted-ran"), "the planted program ran: {out:?}");
    }

    #[tokio::test]
    async fn connects_to_a_local_listener() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            s.write_all(b"hi").await.unwrap();
        });
        let mut s =
            connect_tcp("127.0.0.1", port, Duration::from_secs(5), true, &CancellationToken::new()).await.unwrap();
        let mut buf = [0u8; 2];
        s.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"hi");
    }

    #[tokio::test]
    async fn refused_connections_are_classified() {
        // Grab a free port, then close the listener so nothing is accepting on it.
        let port = {
            let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
            l.local_addr().unwrap().port()
        };
        let err =
            connect_tcp("127.0.0.1", port, Duration::from_secs(5), true, &CancellationToken::new()).await.unwrap_err();
        assert!(matches!(err, SshError::ConnectionRefused { .. }), "{err:?}");
    }

    #[tokio::test]
    async fn unresolvable_hosts_are_dns_errors() {
        let err = connect_tcp(
            "definitely-not-a-real-host.invalid",
            22,
            Duration::from_secs(8),
            true,
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, SshError::Dns { .. }), "{err:?}");
    }

    #[tokio::test]
    async fn cancellation_is_honoured() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        // 10.255.255.1 is a black hole on most networks; cancellation wins regardless.
        let err = connect_tcp("10.255.255.1", 22, Duration::from_secs(30), true, &cancel).await.unwrap_err();
        assert!(matches!(err, SshError::Cancelled), "{err:?}");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn proxy_command_relays_bytes_both_ways() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 5];
            s.read_exact(&mut buf).await.unwrap();
            assert_eq!(&buf, b"hello");
            s.write_all(b"world").await.unwrap();
            // Keep the socket open until the relay is torn down.
            tokio::time::sleep(Duration::from_secs(10)).await;
        });

        let mut p = spawn_proxy(&tcp_relay_command(), &tokens("127.0.0.1", port)).unwrap();
        p.write_all(b"hello").await.unwrap();
        p.flush().await.unwrap();
        let mut buf = [0u8; 5];
        tokio::time::timeout(Duration::from_secs(20), p.read_exact(&mut buf)).await.expect("relay timed out").unwrap();
        assert_eq!(&buf, b"world");
    }
}
