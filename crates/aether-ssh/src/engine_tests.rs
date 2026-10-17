//! End-to-end tests: the real client engine against the in-process test server.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use aether_core::known_hosts::{HostKeyRef, KnownHosts};
use aether_core::model::{AuthMethodKind, ForwardKind, ForwardRule, HostKeyPolicy};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::testing::delegate::ScriptedDelegate;
use crate::testing::{KbdStep, TestServer, TestServerOptions, TestUser};
use crate::*;

struct Env {
    ctx: ConnectContext,
    delegate: ScriptedDelegate,
    known_hosts_path: std::path::PathBuf,
    dir: TempDir,
}

fn env(delegate: ScriptedDelegate) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let known_hosts_path = dir.path().join("known_hosts");
    let ctx = ConnectContext {
        delegate: Arc::new(delegate.clone()),
        known_hosts: Arc::new(KnownHosts::new(&known_hosts_path, None)),
        cancel: CancellationToken::new(),
    };
    Env { ctx, delegate, known_hosts_path, dir }
}

fn spec(server: &TestServer, user: &str, methods: Vec<AuthMethodKind>) -> ConnectSpec {
    let mut s = ConnectSpec::new("127.0.0.1", server.port(), user);
    s.auth.methods = methods;
    // No default identity files: tests must not pick up keys from the developer's machine.
    s.auth.default_identity_files = Vec::new();
    s.options.connect_timeout = Duration::from_secs(10);
    s
}

async fn read_until(rx: &mut mpsc::Receiver<ShellEvent>, needle: &str) -> String {
    let mut acc = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while !acc.contains(needle) {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some(ShellEvent::Data(b))) => acc.push_str(&String::from_utf8_lossy(&b)),
            Ok(Some(_)) => {}
            Ok(None) => panic!("shell ended while waiting for {needle:?}; got {acc:?}"),
            Err(_) => panic!("timed out waiting for {needle:?}; got {acc:?}"),
        }
    }
    acc
}

async fn echo_server() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                while let Ok(n) = s.read(&mut buf).await {
                    if n == 0 || s.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    (addr, task)
}

async fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap().port()
}

// ── login & shell ────────────────────────────────────────────────────────────

#[tokio::test]
async fn password_login_and_interactive_shell() {
    let server =
        TestServer::start(TestServerOptions { banner: Some("Authorized use only".into()), ..Default::default() }).await;
    let env = env(ScriptedDelegate::answering(&["secret"]));
    let conn = SshConnection::connect(&spec(&server, "tester", vec![AuthMethodKind::Password]), &env.ctx)
        .await
        .expect("connect");

    assert_eq!(conn.info().target, format!("tester@127.0.0.1:{}", server.port()));
    assert!(conn.info().server_key.is_some());
    assert!(conn.info().negotiated.is_some(), "negotiated algorithms are reported");
    assert!(env.delegate.banners.lock().iter().any(|b| b.contains("Authorized use only")));
    assert!(env.delegate.progress.lock().contains(&ConnectStage::Ready));

    let params = ShellParams {
        size: PtySize { cols: 100, rows: 30, ..Default::default() },
        env: vec![("LANG".into(), "en_US.UTF-8".into())],
        ..Default::default()
    };
    let (shell, mut rx) = conn.open_shell(&params).await.expect("shell");
    read_until(&mut rx, "$ ").await;

    shell.write("echo hello\r").await.unwrap();
    let out = read_until(&mut rx, "hello\r\n$ ").await;
    assert!(out.contains("echo hello"), "typed characters are echoed: {out:?}");

    shell.write("size\r").await.unwrap();
    read_until(&mut rx, "100x30").await;

    shell.resize(PtySize { cols: 80, rows: 24, ..Default::default() }).await.unwrap();
    shell.write("size\r").await.unwrap();
    read_until(&mut rx, "80x24").await;

    shell.write("env LANG\r").await.unwrap();
    read_until(&mut rx, "en_US.UTF-8").await;

    shell.write("exit 7\r").await.unwrap();
    let mut saw_exit = false;
    while let Some(ev) = rx.recv().await {
        match ev {
            ShellEvent::Exit { status, .. } => {
                assert_eq!(status, Some(7));
                saw_exit = true;
            }
            ShellEvent::Closed => break,
            ShellEvent::Data(_) => {}
        }
    }
    assert!(saw_exit && rx.recv().await.is_none());

    let rec = server.recorded();
    assert_eq!(rec.ptys[0], ("xterm-256color".to_string(), 100, 30));
    assert_eq!(rec.window_changes, vec![(80, 24)]);
    conn.close().await;
}

#[tokio::test]
async fn shell_output_survives_a_large_burst() {
    let server = TestServer::start(TestServerOptions::default()).await;
    let env = env(ScriptedDelegate::answering(&["secret"]));
    let conn =
        SshConnection::connect(&spec(&server, "tester", vec![AuthMethodKind::Password]), &env.ctx).await.unwrap();
    let (shell, mut rx) = conn.open_shell(&ShellParams::default()).await.unwrap();
    read_until(&mut rx, "$ ").await;

    // 3 000 lines × ~11 bytes exceeds the 4 MiB window only if lost; count what arrives.
    shell.write("flood 3000\r").await.unwrap();
    let all = read_until(&mut rx, "line 2999\r\n").await;
    assert!(all.contains("line 0\r\n") && all.contains("line 1500\r\n"));

    shell.write("big 2000000\r").await.unwrap();
    let mut total = 0usize;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while total < 2_000_000 {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some(ShellEvent::Data(b))) => total += b.iter().filter(|c| **c == b'x').count(),
            other => panic!("burst interrupted at {total}: {other:?}"),
        }
    }
    assert!(total >= 2_000_000);
}

// ── authentication ───────────────────────────────────────────────────────────

#[tokio::test]
async fn wrong_passwords_are_retried_then_fail_with_a_clear_error() {
    let server = TestServer::start(TestServerOptions::default()).await;
    let env = env(ScriptedDelegate::answering(&["nope1", "nope2", "nope3"]));
    let err = SshConnection::connect(&spec(&server, "tester", vec![AuthMethodKind::Password]), &env.ctx)
        .await
        .err()
        .expect("must fail");
    match &err {
        SshError::AuthFailed { tried, user, .. } => {
            assert_eq!(user, "tester");
            assert_eq!(tried, &vec!["password".to_string()]);
        }
        other => panic!("expected AuthFailed, got {other:?}"),
    }
    let prompts = env.delegate.prompts.lock();
    assert_eq!(prompts.len(), 3);
    assert_eq!(prompts[0].attempt, 0);
    assert!(prompts[0].error.is_none());
    assert_eq!(prompts[2].attempt, 2);
    assert!(prompts[1].error.as_deref().unwrap().contains("try again"));
}

#[tokio::test]
async fn cancelling_the_prompt_cancels_the_login() {
    let server = TestServer::start(TestServerOptions::default()).await;
    let env = env(ScriptedDelegate::default()); // no answers → prompt returns None
    let err =
        SshConnection::connect(&spec(&server, "tester", vec![AuthMethodKind::Password]), &env.ctx).await.err().unwrap();
    assert!(matches!(err, SshError::AuthCancelled), "{err:?}");
}

#[tokio::test]
async fn saved_password_is_used_silently_and_a_bad_one_is_forgotten() {
    let server = TestServer::start(TestServerOptions::default()).await;

    let good = ScriptedDelegate::default();
    *good.saved_password.lock() = Some("secret".into());
    let env1 = env(good);
    SshConnection::connect(&spec(&server, "tester", vec![AuthMethodKind::Password]), &env1.ctx).await.unwrap();
    assert!(env1.delegate.prompts.lock().is_empty(), "no prompt when the saved password works");

    let stale = ScriptedDelegate::answering(&["secret"]);
    *stale.saved_password.lock() = Some("old-password".into());
    let env2 = env(stale);
    SshConnection::connect(&spec(&server, "tester", vec![AuthMethodKind::Password]), &env2.ctx).await.unwrap();
    assert_eq!(env2.delegate.forgotten.load(Ordering::SeqCst), 1);
    let prompts = env2.delegate.prompts.lock();
    assert_eq!(prompts.len(), 1);
    assert!(prompts[0].error.as_deref().unwrap().contains("saved password"));
}

#[tokio::test]
async fn remembering_a_password_happens_only_after_it_works() {
    let server = TestServer::start(TestServerOptions::default()).await;
    let mut d = ScriptedDelegate::answering(&["wrong", "secret"]);
    d.remember = true;
    let env = env(d);
    SshConnection::connect(&spec(&server, "tester", vec![AuthMethodKind::Password]), &env.ctx).await.unwrap();
    let remembered = env.delegate.remembered.lock();
    assert_eq!(remembered.len(), 1, "the wrong attempt is never saved");
    assert_eq!(remembered[0].1, "secret");
}

#[tokio::test]
async fn public_key_auth_with_a_file_key() {
    let key = generate_key(KeyKind::Ed25519, "test", None).unwrap();
    let public = russh::keys::PublicKey::from_openssh(&key.meta.public_key).unwrap();
    let server = TestServer::start(TestServerOptions {
        users: vec![TestUser::with_key("keyuser", public)],
        ..Default::default()
    })
    .await;

    let env = env(ScriptedDelegate::default());
    let key_path = env.dir.path().join("id_ed25519");
    std::fs::write(&key_path, key.private_openssh.as_bytes()).unwrap();

    let mut s = spec(&server, "keyuser", vec![AuthMethodKind::PublicKey]);
    s.auth.key = Some(KeyRef::File(key_path));
    let conn = SshConnection::connect(&s, &env.ctx).await.expect("key login");
    let out = conn.exec("whoami", Duration::from_secs(5)).await.unwrap();
    assert_eq!(out.stdout_text(), "keyuser\n");
    assert!(env.delegate.prompts.lock().is_empty(), "unencrypted key needs no prompt");
}

#[tokio::test]
async fn passphrase_protected_keys_prompt_and_can_be_remembered() {
    let key = generate_key(KeyKind::Ed25519, "locked", Some("hunter2")).unwrap();
    let public = russh::keys::PublicKey::from_openssh(&key.meta.public_key).unwrap();
    let server = TestServer::start(TestServerOptions {
        users: vec![TestUser::with_key("keyuser", public)],
        ..Default::default()
    })
    .await;

    let mut d = ScriptedDelegate::answering(&["wrong-passphrase", "hunter2"]);
    d.remember = true;
    let env = env(d);
    let key_path = env.dir.path().join("id_locked");
    std::fs::write(&key_path, key.private_openssh.as_bytes()).unwrap();

    let mut s = spec(&server, "keyuser", vec![AuthMethodKind::PublicKey]);
    s.auth.key = Some(KeyRef::File(key_path));
    SshConnection::connect(&s, &env.ctx).await.expect("login with passphrase");

    let prompts = env.delegate.prompts.lock();
    assert_eq!(prompts.len(), 2);
    assert!(matches!(prompts[0].kind, PromptKind::KeyPassphrase { .. }));
    assert!(prompts[1].error.is_some());
    assert_eq!(env.delegate.remembered.lock().len(), 1);
}

#[tokio::test]
async fn keys_stored_in_the_vault_work() {
    let key = generate_key(KeyKind::Ed25519, "vaulted", None).unwrap();
    let public = russh::keys::PublicKey::from_openssh(&key.meta.public_key).unwrap();
    let server = TestServer::start(TestServerOptions {
        users: vec![TestUser::with_key("keyuser", public)],
        ..Default::default()
    })
    .await;
    let d = ScriptedDelegate::default();
    d.vault_keys.lock().push(("key-1".into(), key.private_openssh.to_string()));
    let env = env(d);

    let mut s = spec(&server, "keyuser", vec![AuthMethodKind::PublicKey]);
    s.auth.key = Some(KeyRef::Vault { key_id: "key-1".into(), label: "Work key".into() });
    SshConnection::connect(&s, &env.ctx).await.expect("vault key login");
}

#[tokio::test]
async fn missing_explicit_key_is_a_clear_error_not_a_silent_skip() {
    let server = TestServer::start(TestServerOptions {
        users: vec![TestUser::with_key("u", generate_public())],
        ..Default::default()
    })
    .await;
    let env = env(ScriptedDelegate::default());
    let mut s = spec(&server, "u", vec![AuthMethodKind::PublicKey]);
    s.auth.key = Some(KeyRef::File(env.dir.path().join("does-not-exist")));
    let err = SshConnection::connect(&s, &env.ctx).await.err().unwrap();
    assert!(matches!(err, SshError::KeyUnreadable { .. }), "{err:?}");
}

fn generate_public() -> russh::keys::PublicKey {
    let g = generate_key(KeyKind::Ed25519, "x", None).unwrap();
    russh::keys::PublicKey::from_openssh(&g.meta.public_key).unwrap()
}

#[tokio::test]
async fn keyboard_interactive_two_factor() {
    let server = TestServer::start(TestServerOptions {
        users: vec![TestUser { name: "mfa".into(), kbdint_answer: Some("123456".into()), ..Default::default() }],
        ..Default::default()
    })
    .await;

    let env_ok = env(ScriptedDelegate::answering(&["123456"]));
    SshConnection::connect(&spec(&server, "mfa", vec![AuthMethodKind::KeyboardInteractive]), &env_ok.ctx)
        .await
        .expect("correct code");
    {
        let prompts = env_ok.delegate.prompts.lock();
        match &prompts[0].kind {
            PromptKind::KeyboardInteractive { prompts: p, .. } => {
                assert_eq!(p[0].text, "Verification code: ");
                assert!(!p[0].echo, "codes are typed hidden");
            }
            other => panic!("{other:?}"),
        }
    }

    let env_bad = env(ScriptedDelegate::answering(&["000000"]));
    let err = SshConnection::connect(&spec(&server, "mfa", vec![AuthMethodKind::KeyboardInteractive]), &env_bad.ctx)
        .await
        .err()
        .unwrap();
    assert!(matches!(err, SshError::AuthFailed { .. }), "{err:?}");
}

fn pam_user(steps: &[(&str, &str)]) -> TestUser {
    TestUser {
        name: "pam".into(),
        kbdint_steps: steps.iter().map(|(prompt, answer)| KbdStep::new(prompt, answer)).collect(),
        ..Default::default()
    }
}

fn pam_spec(server: &TestServer) -> ConnectSpec {
    let mut s = spec(server, "pam", vec![AuthMethodKind::KeyboardInteractive]);
    s.secret_scope = Some("host-1".into());
    s
}

/// Plain passwords often travel over keyboard-interactive (PAM). Whatever the person asks to remember must not be
/// written down until the server has said it works, or a typo would be saved for good.
#[tokio::test]
async fn a_password_asked_over_keyboard_interactive_is_remembered_only_once_it_works() {
    let server = TestServer::start(TestServerOptions {
        users: vec![pam_user(&[("Password: ", "secret")])],
        ..Default::default()
    })
    .await;
    let mut d = ScriptedDelegate::answering(&["wrong", "secret"]);
    d.remember = true;
    let env = env(d);
    SshConnection::connect(&pam_spec(&server), &env.ctx).await.expect("second try works");

    let remembered = env.delegate.remembered.lock();
    assert_eq!(remembered.len(), 1, "the wrong attempt is never saved: {remembered:?}");
    assert_eq!(remembered[0].1, "secret");
    let prompts = env.delegate.prompts.lock();
    assert_eq!(prompts.len(), 2);
    assert!(prompts[0].can_remember);
    assert!(prompts[1].error.as_deref().unwrap().contains("try again"), "{:?}", prompts[1].error);
}

#[tokio::test]
async fn nothing_is_offered_for_remembering_when_there_is_nowhere_to_keep_it() {
    let server = TestServer::start(TestServerOptions {
        users: vec![pam_user(&[("Password: ", "secret")]), TestUser::with_password("tester", "secret")],
        ..Default::default()
    })
    .await;
    // A quick-connect target that was never saved has no scope to file a secret under.
    let env_kbd = env(ScriptedDelegate::answering(&["secret"]));
    let mut s = spec(&server, "pam", vec![AuthMethodKind::KeyboardInteractive]);
    s.secret_scope = None;
    SshConnection::connect(&s, &env_kbd.ctx).await.unwrap();
    assert!(!env_kbd.delegate.prompts.lock()[0].can_remember);

    let env_pw = env(ScriptedDelegate::answering(&["secret"]));
    let mut s = spec(&server, "tester", vec![AuthMethodKind::Password]);
    s.secret_scope = None;
    SshConnection::connect(&s, &env_pw.ctx).await.unwrap();
    assert!(!env_pw.delegate.prompts.lock()[0].can_remember);

    let env_saved = env(ScriptedDelegate::answering(&["secret"]));
    let mut s = spec(&server, "tester", vec![AuthMethodKind::Password]);
    s.secret_scope = Some("host-1".into());
    SshConnection::connect(&s, &env_saved.ctx).await.unwrap();
    assert!(env_saved.delegate.prompts.lock()[0].can_remember);
}

#[tokio::test]
async fn a_stale_saved_password_is_forgotten_over_keyboard_interactive_too() {
    let server = TestServer::start(TestServerOptions {
        users: vec![pam_user(&[("Password: ", "secret")])],
        ..Default::default()
    })
    .await;
    let stale = ScriptedDelegate::answering(&["secret"]);
    *stale.saved_password.lock() = Some("old-password".into());
    let stale_env = env(stale);
    SshConnection::connect(&pam_spec(&server), &stale_env.ctx).await.unwrap();

    assert_eq!(stale_env.delegate.forgotten.load(Ordering::SeqCst), 1);
    {
        let prompts = stale_env.delegate.prompts.lock();
        assert_eq!(prompts.len(), 1, "only the retry asks");
        assert!(prompts[0].error.as_deref().unwrap().contains("saved password"), "{:?}", prompts[0].error);
    }

    // A saved password that works is simply used.
    let good = ScriptedDelegate::default();
    *good.saved_password.lock() = Some("secret".into());
    let good_env = env(good);
    SshConnection::connect(&pam_spec(&server), &good_env.ctx).await.unwrap();
    assert!(good_env.delegate.prompts.lock().is_empty());
    assert_eq!(good_env.delegate.forgotten.load(Ordering::SeqCst), 0);
}

/// Password first, then a code: the second question is the next step, not a sign that the password was wrong.
#[tokio::test]
async fn a_second_factor_after_the_password_is_not_reported_as_a_wrong_password() {
    let server = TestServer::start(TestServerOptions {
        users: vec![pam_user(&[("Password: ", "secret"), ("Verification code: ", "123456")])],
        ..Default::default()
    })
    .await;
    let mut d = ScriptedDelegate::answering(&["secret", "123456"]);
    d.remember = true;
    let env = env(d);
    SshConnection::connect(&pam_spec(&server), &env.ctx).await.expect("both steps pass");

    let prompts = env.delegate.prompts.lock();
    assert_eq!(prompts.len(), 2);
    assert!(prompts[1].error.is_none(), "{:?}", prompts[1].error);
    assert!(!prompts[1].can_remember, "a one-time code is never offered for saving");
    let remembered = env.delegate.remembered.lock();
    assert_eq!(remembered.len(), 1, "{remembered:?}");
    assert_eq!(remembered[0].1, "secret");
}

#[tokio::test]
async fn a_wrong_code_at_the_second_step_saves_nothing() {
    let server = TestServer::start(TestServerOptions {
        users: vec![pam_user(&[("Password: ", "secret"), ("Verification code: ", "123456")])],
        ..Default::default()
    })
    .await;
    let mut d = ScriptedDelegate::answering(&["secret", "000000", "000000", "000000"]);
    d.remember = true;
    let env = env(d);
    let err = SshConnection::connect(&pam_spec(&server), &env.ctx).await.err().unwrap();
    assert!(matches!(err, SshError::AuthFailed { .. }), "{err:?}");
    assert!(env.delegate.remembered.lock().is_empty(), "the login never completed, so nothing was saved");
}

/// The server accepts the keyboard-interactive password and asks for another factor (partial success). What was typed
/// was right, so what the person asked to be remembered is remembered even though the login isn't over yet.
#[tokio::test]
async fn a_password_the_server_accepted_as_a_first_factor_is_remembered() {
    let server = TestServer::start(TestServerOptions {
        users: vec![TestUser {
            name: "pam".into(),
            password: Some("second".into()),
            kbdint_steps: vec![KbdStep::new("Password: ", "secret")],
            kbdint_then_password: true,
            ..Default::default()
        }],
        ..Default::default()
    })
    .await;
    let mut d = ScriptedDelegate::answering(&["secret", "second"]);
    d.remember = true;
    let env = env(d);
    let mut s = pam_spec(&server);
    s.auth.methods = vec![AuthMethodKind::KeyboardInteractive, AuthMethodKind::Password];
    SshConnection::connect(&s, &env.ctx).await.expect("both factors are given");

    let remembered = env.delegate.remembered.lock();
    assert_eq!(remembered.len(), 2, "{remembered:?}");
    assert_eq!(remembered[0].1, "secret", "the first factor was kept when the server took it");
    assert_eq!(remembered[1].1, "second");
}

/// Two rounds in a row that ask nothing (notices from the PAM stack) are not "the same questions asked again", and the
/// password prompt that follows is the first ask, not a retry.
#[tokio::test]
async fn rounds_with_nothing_to_ask_are_not_mistaken_for_a_refusal() {
    let server = TestServer::start(TestServerOptions {
        users: vec![TestUser {
            name: "pam".into(),
            kbdint_steps: vec![KbdStep::info(), KbdStep::info(), KbdStep::new("Password: ", "secret")],
            ..Default::default()
        }],
        ..Default::default()
    })
    .await;
    let env = env(ScriptedDelegate::answering(&["secret"]));
    SshConnection::connect(&pam_spec(&server), &env.ctx).await.expect("the password gets in");

    let prompts = env.delegate.prompts.lock();
    assert_eq!(prompts.len(), 1);
    assert!(prompts[0].error.is_none(), "no \"try again\" before anything was tried: {:?}", prompts[0].error);
}

/// A public key file picked as *the* key, with the agent not among the methods, can't sign anything. That is said in
/// terms of the file, not as "no usable method".
#[tokio::test]
async fn a_public_key_file_chosen_without_the_agent_is_reported_as_unusable() {
    let generated = generate_key(KeyKind::Ed25519, "only-public", None).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let public = dir.path().join("id.pub");
    std::fs::write(&public, format!("{}\n", generated.meta.public_key)).unwrap();

    // A server that takes keys, so that the key method is actually tried.
    let accepted = russh::keys::PublicKey::from_openssh(&generated.meta.public_key).unwrap();
    let server = TestServer::start(TestServerOptions {
        users: vec![TestUser::with_key("keyuser", accepted)],
        ..Default::default()
    })
    .await;
    let env = env(ScriptedDelegate::default());
    let mut s = spec(&server, "keyuser", vec![AuthMethodKind::PublicKey]);
    s.auth.key = Some(KeyRef::File(public.clone()));
    let err = SshConnection::connect(&s, &env.ctx).await.err().expect("a public key can't sign");
    assert!(matches!(&err, SshError::KeyUnreadable { key, .. } if key.contains("id.pub")), "{err:?}");
}

/// The password was accepted; it is the code that was mistyped and asked for again. The refusal belongs to the code, so the
/// password the person asked to keep is still kept — once the login completes.
#[tokio::test]
async fn a_mistyped_code_does_not_cost_the_password_its_place_in_the_vault() {
    let server = TestServer::start(TestServerOptions {
        users: vec![pam_user(&[("Password: ", "secret"), ("Verification code: ", "123456")])],
        ..Default::default()
    })
    .await;
    let mut d = ScriptedDelegate::answering(&["secret", "000000", "123456"]);
    d.remember = true;
    let env = env(d);
    SshConnection::connect(&pam_spec(&server), &env.ctx).await.expect("the code is right the second time");

    let remembered = env.delegate.remembered.lock();
    assert_eq!(remembered.len(), 1, "{remembered:?}");
    assert_eq!(remembered[0].1, "secret");
    let prompts = env.delegate.prompts.lock();
    assert_eq!(prompts.len(), 3);
    assert!(prompts[2].error.is_some(), "the repeated code question says the code was refused");
}

#[tokio::test]
async fn methods_the_server_does_not_offer_are_skipped() {
    // The server only offers passwords; asking for keys first must not break login.
    let server = TestServer::start(TestServerOptions::default()).await;
    let env = env(ScriptedDelegate::answering(&["secret"]));
    let s = spec(&server, "tester", vec![AuthMethodKind::Agent, AuthMethodKind::PublicKey, AuthMethodKind::Password]);
    SshConnection::connect(&s, &env.ctx).await.expect("falls through to password");
}

#[tokio::test]
async fn with_no_usable_credentials_the_error_says_so() {
    let server = TestServer::start(TestServerOptions::default()).await;
    let env = env(ScriptedDelegate::default());
    let err = SshConnection::connect(&spec(&server, "tester", vec![AuthMethodKind::PublicKey]), &env.ctx)
        .await
        .err()
        .unwrap();
    assert!(matches!(err, SshError::NoAuthMethods), "{err:?}");
}

// ── host keys ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn host_key_is_trusted_on_first_use_and_remembered() {
    let server = TestServer::start(TestServerOptions::default()).await;

    let first = env(ScriptedDelegate::answering(&["secret"]));
    let conn =
        SshConnection::connect(&spec(&server, "tester", vec![AuthMethodKind::Password]), &first.ctx).await.unwrap();
    assert_eq!(first.delegate.host_key_prompts.load(Ordering::SeqCst), 1);
    conn.close().await;
    assert!(std::fs::read_to_string(&first.known_hosts_path).unwrap().contains("ssh-ed25519"));

    // A second client sharing the same known_hosts connects without asking.
    let mut second_delegate = ScriptedDelegate::answering(&["secret"]);
    second_delegate.host_key = HostKeyDecision::Reject;
    let second = ConnectContext {
        delegate: Arc::new(second_delegate.clone()),
        known_hosts: first.ctx.known_hosts.clone(),
        cancel: CancellationToken::new(),
    };
    SshConnection::connect(&spec(&server, "tester", vec![AuthMethodKind::Password]), &second)
        .await
        .expect("trusted host connects silently");
    assert_eq!(second_delegate.host_key_prompts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn declining_an_unknown_host_aborts_before_any_credentials_are_sent() {
    let server = TestServer::start(TestServerOptions::default()).await;
    let mut d = ScriptedDelegate::answering(&["secret"]);
    d.host_key = HostKeyDecision::Reject;
    let env = env(d);
    let err =
        SshConnection::connect(&spec(&server, "tester", vec![AuthMethodKind::Password]), &env.ctx).await.err().unwrap();
    assert!(matches!(err, SshError::HostKeyRejected { reason: HostKeyRejection::Declined, .. }), "{err:?}");
    assert!(env.delegate.prompts.lock().is_empty(), "never asked for a password");
    assert!(server.recorded().auth_attempts.is_empty(), "server never saw a login attempt");
}

#[tokio::test]
async fn a_changed_host_key_blocks_until_explicitly_replaced() {
    let server = TestServer::start(TestServerOptions::default()).await;
    let e = env(ScriptedDelegate::answering(&["secret"]));

    // Pretend we trusted a different key for this host before.
    let other = generate_public();
    let blob = russh::keys::PublicKeyBase64::public_key_bytes(&other);
    e.ctx.known_hosts.add("127.0.0.1", server.port(), &HostKeyRef { algorithm: "ssh-ed25519", blob: &blob }).unwrap();

    // 1. "Trust and save" must NOT be enough for a changed key.
    let err =
        SshConnection::connect(&spec(&server, "tester", vec![AuthMethodKind::Password]), &e.ctx).await.err().unwrap();
    assert!(matches!(err, SshError::HostKeyRejected { reason: HostKeyRejection::Changed, .. }), "{err:?}");
    assert!(server.recorded().auth_attempts.is_empty(), "no credentials were sent");

    // 2. An explicit replace works and updates known_hosts.
    let mut d = ScriptedDelegate::answering(&["secret"]);
    d.host_key = HostKeyDecision::ReplaceAndTrust;
    let ctx = ConnectContext {
        delegate: Arc::new(d),
        known_hosts: e.ctx.known_hosts.clone(),
        cancel: CancellationToken::new(),
    };
    SshConnection::connect(&spec(&server, "tester", vec![AuthMethodKind::Password]), &ctx)
        .await
        .expect("replace and trust");
}

/// A host we already have an ECDSA key for suddenly offers only an ed25519 one. That is what a machine-in-the-middle
/// would do to dodge the changed-key warning, so it is never waved through as a first visit — not even under
/// "accept new", and not by a plain "trust and save".
#[tokio::test]
async fn a_server_offering_only_a_new_key_type_is_treated_like_a_change() {
    let server = TestServer::start(TestServerOptions::default()).await;
    let e = env(ScriptedDelegate::answering(&["secret"]));
    let mut ecdsa = Vec::new();
    ecdsa.extend_from_slice(&19u32.to_be_bytes());
    ecdsa.extend_from_slice(b"ecdsa-sha2-nistp256");
    ecdsa.extend_from_slice(&65u32.to_be_bytes());
    ecdsa.extend_from_slice(&[7; 65]);
    e.ctx
        .known_hosts
        .add("127.0.0.1", server.port(), &HostKeyRef { algorithm: "ecdsa-sha2-nistp256", blob: &ecdsa })
        .unwrap();

    let mut s = spec(&server, "tester", vec![AuthMethodKind::Password]);
    s.options.host_key_policy = HostKeyPolicy::AcceptNew;
    let err = SshConnection::connect(&s, &e.ctx).await.err().unwrap();
    assert!(matches!(err, SshError::HostKeyRejected { reason: HostKeyRejection::Changed, .. }), "{err:?}");
    assert_eq!(e.delegate.host_key_prompts.load(Ordering::SeqCst), 1, "a person was asked");
    assert!(server.recorded().auth_attempts.is_empty(), "no credentials were sent");

    // Replacing it is possible, and the old type stays on record next to the new one.
    let mut d = ScriptedDelegate::answering(&["secret"]);
    d.host_key = HostKeyDecision::ReplaceAndTrust;
    let ctx = ConnectContext {
        delegate: Arc::new(d),
        known_hosts: e.ctx.known_hosts.clone(),
        cancel: CancellationToken::new(),
    };
    SshConnection::connect(&s, &ctx).await.expect("replace and trust");
    let recorded = std::fs::read_to_string(&e.known_hosts_path).unwrap();
    assert!(recorded.contains("ecdsa-sha2-nistp256") && recorded.contains("ssh-ed25519"), "{recorded}");
}

/// Reading a fingerprint takes however long it takes. The connect timeout is about the *server*, so a person who needs
/// longer than that to answer must still get a connection.
#[tokio::test]
async fn a_slow_host_key_answer_does_not_use_up_the_connect_timeout() {
    let server = TestServer::start(TestServerOptions::default()).await;
    let mut d = ScriptedDelegate::answering(&["secret"]);
    d.host_key_delay = Duration::from_millis(1300);
    let env = env(d);
    let mut s = spec(&server, "tester", vec![AuthMethodKind::Password]);
    s.options.connect_timeout = Duration::from_millis(500);

    let started = std::time::Instant::now();
    let conn = SshConnection::connect(&s, &env.ctx).await.expect("a slow human is not a dead server");
    assert!(started.elapsed() >= Duration::from_millis(1300), "the question really was open for that long");
    assert_eq!(env.delegate.host_key_prompts.load(Ordering::SeqCst), 1);
    conn.close().await;
}

/// The other half: with nobody being asked, a server that accepts the connection and then says nothing still times
/// out instead of hanging the tab forever.
#[tokio::test]
async fn a_silent_server_times_the_handshake_out() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let held = tokio::spawn(async move {
        let mut keep = Vec::new();
        while let Ok((s, _)) = listener.accept().await {
            keep.push(s);
        }
    });
    let env = env(ScriptedDelegate::default());
    let mut s = ConnectSpec::new("127.0.0.1", port, "tester");
    s.auth.methods = vec![AuthMethodKind::Password];
    s.auth.default_identity_files = Vec::new();
    s.options.connect_timeout = Duration::from_millis(400);

    let started = std::time::Instant::now();
    let err = SshConnection::connect(&s, &env.ctx).await.err().expect("must not hang");
    assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
    assert!(matches!(err, SshError::Disconnected(_)), "{err:?}");
    assert!(err.is_transient(), "a flaky network is worth retrying");
    assert_eq!(env.delegate.host_key_prompts.load(Ordering::SeqCst), 0);
    held.abort();
}

#[tokio::test]
async fn strict_policy_refuses_unknown_hosts_without_asking() {
    let server = TestServer::start(TestServerOptions::default()).await;
    let env = env(ScriptedDelegate::answering(&["secret"]));
    let mut s = spec(&server, "tester", vec![AuthMethodKind::Password]);
    s.options.host_key_policy = HostKeyPolicy::Strict;
    let err = SshConnection::connect(&s, &env.ctx).await.err().unwrap();
    assert!(matches!(err, SshError::HostKeyRejected { reason: HostKeyRejection::UnknownInStrictMode, .. }));
    assert_eq!(env.delegate.host_key_prompts.load(Ordering::SeqCst), 0);
}

// ── exec ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn exec_collects_stdout_stderr_and_status() {
    let server = TestServer::start(TestServerOptions::default()).await;
    let env = env(ScriptedDelegate::answering(&["secret"]));
    let conn =
        SshConnection::connect(&spec(&server, "tester", vec![AuthMethodKind::Password]), &env.ctx).await.unwrap();
    let ok = conn.exec("echo hi there", Duration::from_secs(5)).await.unwrap();
    assert_eq!((ok.stdout_text().as_str(), ok.status), ("hi there\n", Some(0)));
    let bad = conn.exec("fail", Duration::from_secs(5)).await.unwrap();
    assert_eq!(bad.stderr, b"boom\n");
    assert_eq!(bad.status, Some(3));
    assert_eq!(server.recorded().exec, vec!["echo hi there", "fail"]);
}

// ── X11 forwarding ───────────────────────────────────────────────────────────

/// One `.Xauthority` record: family, address, display number, protocol, cookie.
fn xauth_record(number: &str, cookie: &[u8]) -> Vec<u8> {
    let mut out = 256u16.to_be_bytes().to_vec();
    for field in [b"pc".as_slice(), number.as_bytes(), b"MIT-MAGIC-COOKIE-1", cookie] {
        out.extend((field.len() as u16).to_be_bytes());
        out.extend(field);
    }
    out
}

/// A stand-in X server: accepts one connection, records the setup packet and answers.
async fn fake_x_server() -> (u16, tokio::task::JoinHandle<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut packet = vec![0u8; 12 + 20 + 16];
        sock.read_exact(&mut packet).await.unwrap();
        sock.write_all(b"X-SERVER-SAYS-HELLO").await.unwrap();
        packet
    });
    (port, task)
}

#[tokio::test]
async fn x11_programs_on_the_server_reach_the_local_x_server_with_the_real_cookie() {
    let server = TestServer::start(TestServerOptions { x11_client: true, ..Default::default() }).await;
    let (x_port, x_server) = fake_x_server().await;
    // The local display number such that 6000 + n is the fake server's (ephemeral) port.
    let display = x_port - 6000;

    let dir = tempfile::tempdir().unwrap();
    let real_cookie = [0xC0u8; 16];
    let xauth = dir.path().join(".Xauthority");
    std::fs::write(
        &xauth,
        [xauth_record("99", &[0x11; 16]), xauth_record(&display.to_string(), &real_cookie)].concat(),
    )
    .unwrap();

    let mut s = spec(&server, "tester", vec![AuthMethodKind::Password]);
    s.options.x11 = Some(X11Options { display: format!("127.0.0.1:{display}"), xauthority: Some(xauth) });
    let env = env(ScriptedDelegate::answering(&["secret"]));
    let conn = SshConnection::connect(&s, &env.ctx).await.unwrap();
    assert_eq!(conn.x11_display().as_deref(), Some(format!("127.0.0.1:{display}.0").as_str()));

    let (_shell, _rx) = conn.open_shell(&ShellParams { x11: true, ..Default::default() }).await.unwrap();

    // The server was handed a throwaway cookie — never the real one.
    let requests = server.recorded().x11_requests;
    assert_eq!(requests.len(), 1, "{requests:?}");
    let (protocol, issued, _screen) = &requests[0];
    assert_eq!(protocol, "MIT-MAGIC-COOKIE-1");
    assert_eq!(issued.len(), 32);
    assert_ne!(issued, &"c0".repeat(16));

    // The "remote X program" presented that throwaway cookie; the local X server saw the real one.
    let packet =
        tokio::time::timeout(Duration::from_secs(10), x_server).await.expect("X server saw a connection").unwrap();
    assert_eq!(&packet[12..30], b"MIT-MAGIC-COOKIE-1");
    assert_eq!(&packet[32..], &real_cookie, "cookie swapped for the real one");

    // And the X server's answer travelled back through the tunnel.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let reply = loop {
        if let Some(r) = server.recorded().x11_reply {
            break r;
        }
        assert!(tokio::time::Instant::now() < deadline, "no reply came back through the tunnel");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(reply, b"X-SERVER-SAYS-HELLO");
}

#[tokio::test]
async fn x11_is_off_unless_the_host_asks_for_it() {
    let server = TestServer::start(TestServerOptions { x11_client: true, ..Default::default() }).await;
    let (conn, _env) = connected(&server).await;
    assert!(conn.x11_display().is_none());
    let (_shell, _rx) = conn.open_shell(&ShellParams { x11: true, ..Default::default() }).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    // Nothing was requested, so a hostile server has nothing to connect through.
    assert!(server.recorded().x11_requests.is_empty());
}

// ── port forwarding ──────────────────────────────────────────────────────────

async fn connected(server: &TestServer) -> (SshConnection, Env) {
    let env = env(ScriptedDelegate::answering(&["secret"]));
    let conn = SshConnection::connect(&spec(server, "tester", vec![AuthMethodKind::Password]), &env.ctx).await.unwrap();
    (conn, env)
}

async fn echo_roundtrip(addr: &str, payload: &[u8]) -> Vec<u8> {
    let mut s = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(addr))
        .await
        .expect("connect timeout")
        .expect("connect");
    s.write_all(payload).await.unwrap();
    let mut buf = vec![0u8; payload.len()];
    tokio::time::timeout(Duration::from_secs(5), s.read_exact(&mut buf)).await.expect("read timeout").expect("read");
    buf
}

#[tokio::test]
async fn local_forward_tunnels_tcp_through_the_server() {
    let server = TestServer::start(TestServerOptions::default()).await;
    let (conn, _env) = connected(&server).await;
    let (echo, _task) = echo_server().await;

    let port = free_port().await;
    let rule = ForwardRule {
        kind: ForwardKind::Local,
        bind_port: port,
        dest_host: "127.0.0.1".into(),
        dest_port: echo.port(),
        ..Default::default()
    };
    let status = conn.forwards().start(rule.clone()).await.expect("start forward");
    assert_eq!(status.bound_port, port);

    let got = echo_roundtrip(&format!("127.0.0.1:{port}"), b"through the tunnel").await;
    assert_eq!(got, b"through the tunnel");
    assert_eq!(server.recorded().forwarded_connections, 1);

    let stats = conn.forwards().statuses();
    assert_eq!(stats[0].total_connections, 1);
    assert!(stats[0].bytes_up >= 18 && stats[0].bytes_down >= 18);

    // Starting twice is refused; stopping releases the port.
    assert!(conn.forwards().start(rule).await.is_err());
    assert!(conn.forwards().stop(&status.id).await);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(TcpStream::connect(format!("127.0.0.1:{port}")).await.is_err(), "listener closed");
}

#[tokio::test]
async fn a_server_that_forbids_forwarding_is_explained_not_silent() {
    let server = TestServer::start(TestServerOptions { allow_forwarding: false, ..Default::default() }).await;
    let (conn, _env) = connected(&server).await;
    let port = free_port().await;
    let rule = ForwardRule {
        kind: ForwardKind::Local,
        bind_port: port,
        dest_host: "127.0.0.1".into(),
        dest_port: 9,
        ..Default::default()
    };
    conn.forwards().start(rule).await.expect("the local listener starts regardless");

    // A client connects; the server refuses to open the tunnel, so the socket just closes.
    let mut s = TcpStream::connect(format!("127.0.0.1:{port}")).await.unwrap();
    let mut buf = [0u8; 8];
    assert_eq!(s.read(&mut buf).await.unwrap_or(0), 0, "connection is closed without data");

    // …but the forward now says why, so the UI can tell the user instead of leaving them guessing.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let status = loop {
        let st = conn.forwards().statuses().remove(0);
        if st.refused > 0 || tokio::time::Instant::now() > deadline {
            break st;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(status.refused, 1);
    let why = status.last_error.expect("a reason is recorded");
    assert!(why.contains("AllowTcpForwarding"), "{why}");
}

#[tokio::test]
async fn local_forward_reports_port_conflicts_in_plain_language() {
    let server = TestServer::start(TestServerOptions::default()).await;
    let (conn, _env) = connected(&server).await;
    let blocker = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = blocker.local_addr().unwrap().port();
    let rule = ForwardRule { bind_port: port, dest_port: 80, ..Default::default() };
    let err = conn.forwards().start(rule).await.err().unwrap();
    assert!(err.to_string().contains("already in use"), "{err}");
}

#[tokio::test]
async fn dynamic_forward_speaks_socks5() {
    let server = TestServer::start(TestServerOptions::default()).await;
    let (conn, _env) = connected(&server).await;
    let (echo, _task) = echo_server().await;

    let port = free_port().await;
    conn.forwards()
        .start(ForwardRule { kind: ForwardKind::Dynamic, bind_port: port, ..Default::default() })
        .await
        .unwrap();

    let mut s = TcpStream::connect(format!("127.0.0.1:{port}")).await.unwrap();
    s.write_all(&[5, 1, 0]).await.unwrap();
    let mut method = [0u8; 2];
    s.read_exact(&mut method).await.unwrap();
    assert_eq!(method, [5, 0]);

    let mut req = vec![5, 1, 0, 1, 127, 0, 0, 1];
    req.extend_from_slice(&echo.port().to_be_bytes());
    s.write_all(&req).await.unwrap();
    let mut reply = [0u8; 10];
    s.read_exact(&mut reply).await.unwrap();
    assert_eq!(&reply[..2], &[5, 0], "CONNECT succeeded");

    s.write_all(b"socks says hi").await.unwrap();
    let mut buf = [0u8; 13];
    s.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"socks says hi");
}

#[tokio::test]
async fn dynamic_forward_reports_unreachable_targets() {
    let server = TestServer::start(TestServerOptions::default()).await;
    let (conn, _env) = connected(&server).await;
    let dead = free_port().await;
    let port = free_port().await;
    conn.forwards()
        .start(ForwardRule { kind: ForwardKind::Dynamic, bind_port: port, ..Default::default() })
        .await
        .unwrap();
    let mut s = TcpStream::connect(format!("127.0.0.1:{port}")).await.unwrap();
    s.write_all(&[5, 1, 0]).await.unwrap();
    let mut method = [0u8; 2];
    s.read_exact(&mut method).await.unwrap();
    let mut req = vec![5, 1, 0, 1, 127, 0, 0, 1];
    req.extend_from_slice(&dead.to_be_bytes());
    s.write_all(&req).await.unwrap();
    let mut reply = [0u8; 10];
    s.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply[1], 5, "connection refused");
}

#[tokio::test]
async fn remote_forward_exposes_a_local_service_on_the_server() {
    let server = TestServer::start(TestServerOptions::default()).await;
    let (conn, _env) = connected(&server).await;
    let (echo, _task) = echo_server().await;

    let status = conn
        .forwards()
        .start(ForwardRule {
            kind: ForwardKind::Remote,
            bind_host: "127.0.0.1".into(),
            bind_port: 0, // let the server pick
            dest_host: "127.0.0.1".into(),
            dest_port: echo.port(),
            ..Default::default()
        })
        .await
        .expect("remote forward");
    assert_ne!(status.bound_port, 0);

    let got = echo_roundtrip(&format!("127.0.0.1:{}", status.bound_port), b"reverse tunnel").await;
    assert_eq!(got, b"reverse tunnel");
}

// ── jump hosts ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn connecting_through_a_jump_host_uses_each_hops_credentials() {
    let bastion = TestServer::start(TestServerOptions {
        users: vec![TestUser::with_password("jumper", "jump-pw")],
        ..Default::default()
    })
    .await;
    let target = TestServer::start(TestServerOptions {
        users: vec![TestUser::with_password("inner", "inner-pw")],
        ..Default::default()
    })
    .await;

    let mut hop = spec(&bastion, "jumper", vec![AuthMethodKind::Password]);
    hop.label = "bastion".into();
    let mut t = spec(&target, "inner", vec![AuthMethodKind::Password]);
    t.label = "db".into();
    t.jump = vec![hop];

    let env = env(ScriptedDelegate::answering(&["jump-pw", "inner-pw"]));
    let conn = SshConnection::connect(&t, &env.ctx).await.expect("two-hop connect");
    assert_eq!(conn.info().via, vec!["bastion"]);
    assert_eq!(conn.exec("whoami", Duration::from_secs(5)).await.unwrap().stdout_text(), "inner\n");
    assert_eq!(bastion.recorded().forwarded_connections, 1, "target traffic rode the bastion");

    // Each prompt was labelled with the hop it belongs to.
    let prompts = env.delegate.prompts.lock();
    assert_eq!(prompts[0].hop.label, "bastion");
    assert!(prompts[0].hop.is_jump());
    assert_eq!(prompts[1].hop.label, "db");
    assert!(!prompts[1].hop.is_jump());
}

#[tokio::test]
async fn a_failing_jump_host_is_attributed_correctly() {
    let target = TestServer::start(TestServerOptions::default()).await;
    let dead = free_port().await;
    let mut hop = ConnectSpec::new("127.0.0.1", dead, "jumper");
    hop.label = "bastion".into();
    hop.options.connect_timeout = Duration::from_secs(5);
    let mut t = spec(&target, "tester", vec![AuthMethodKind::Password]);
    t.jump = vec![hop];

    let env = env(ScriptedDelegate::answering(&["secret"]));
    let err = SshConnection::connect(&t, &env.ctx).await.err().unwrap();
    match err {
        SshError::JumpHost { hop, source } => {
            assert_eq!(hop, "bastion");
            assert!(matches!(*source, SshError::ConnectionRefused { .. }));
        }
        other => panic!("expected JumpHost, got {other:?}"),
    }
}

// ── ProxyCommand ─────────────────────────────────────────────────────────────

/// A complete SSH session carried over a child process's stdin/stdout, exactly how
/// `ProxyCommand ssh -W %h:%p bastion` or `cloudflared access ssh` work.
#[cfg(windows)]
#[tokio::test]
async fn a_full_ssh_session_runs_over_a_proxy_command() {
    let server = TestServer::start(TestServerOptions::default()).await;
    let env = env(ScriptedDelegate::answering(&["secret"]));
    let mut s = spec(&server, "tester", vec![AuthMethodKind::Password]);
    s.proxy_command = Some(crate::transport::tcp_relay_command());
    s.options.connect_timeout = Duration::from_secs(30);

    let conn = SshConnection::connect(&s, &env.ctx).await.expect("connect via ProxyCommand");
    let out = conn.exec("echo via proxy", Duration::from_secs(10)).await.unwrap();
    assert_eq!(out.stdout_text(), "via proxy\n");
    conn.close().await;
}

/// When the command dies before SSH starts, what it printed is the explanation worth showing.
#[tokio::test]
async fn a_proxy_command_that_dies_explains_itself() {
    let server = TestServer::start(TestServerOptions::default()).await;
    let env = env(ScriptedDelegate::answering(&["secret"]));
    let mut s = spec(&server, "tester", vec![AuthMethodKind::Password]);
    // The same line works for `cmd /C` and `sh -c`.
    s.proxy_command = Some("echo no route to the bastion 1>&2 && exit 1".into());
    s.options.connect_timeout = Duration::from_secs(20);

    let err = SshConnection::connect(&s, &env.ctx).await.err().expect("the proxy never speaks SSH");
    match err {
        SshError::ProxyCommand(why) => {
            assert!(why.contains("no route to the bastion"), "stderr should be shown: {why}")
        }
        other => panic!("expected a proxy command error, got {other:?}"),
    }
}

// ── lifecycle ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn deliberate_close_is_reported_as_requested() {
    let server = TestServer::start(TestServerOptions::default()).await;
    let (conn, _env) = connected(&server).await;
    assert!(!conn.is_closed());
    conn.close().await;
    let info = tokio::time::timeout(Duration::from_secs(5), conn.closed()).await.unwrap();
    assert_eq!(info.kind, DisconnectKind::Requested);
    assert!(conn.is_closed());
}

#[tokio::test]
async fn a_dying_server_is_noticed() {
    let server = TestServer::start(TestServerOptions::default()).await;
    let (conn, _env) = connected(&server).await;
    server.stop();
    // The listener is gone; sessions it spawned are torn down with it.
    let info =
        tokio::time::timeout(Duration::from_secs(10), conn.closed()).await.expect("disconnect should be detected");
    assert_ne!(info.kind, DisconnectKind::Requested);
}

#[tokio::test]
async fn ping_measures_latency() {
    let server = TestServer::start(TestServerOptions::default()).await;
    let (conn, _env) = connected(&server).await;
    let rtt = conn.ping().await.unwrap();
    assert!(rtt < Duration::from_secs(2));
}

#[tokio::test]
async fn connection_failures_are_classified() {
    let env = env(ScriptedDelegate::default());
    let dead = free_port().await;
    let mut s = ConnectSpec::new("127.0.0.1", dead, "u");
    s.options.connect_timeout = Duration::from_secs(3);
    let err = SshConnection::connect(&s, &env.ctx).await.err().unwrap();
    assert!(matches!(err, SshError::ConnectionRefused { .. }), "{err:?}");
    assert!(err.is_transient());
}

// ── SSH agent ────────────────────────────────────────────────────────────────

/// A stand-in for the Windows OpenSSH agent: a named pipe served by `russh`'s agent implementation, loaded
/// with a key the same way `ssh-add` would.
#[cfg(windows)]
mod fake_agent {
    use futures::StreamExt;
    use russh::keys::PrivateKey;
    use russh::keys::agent::client::AgentClient;
    use russh::keys::agent::server;
    use tokio::net::windows::named_pipe::ServerOptions;

    #[derive(Clone)]
    struct AllowAll;
    impl server::Agent for AllowAll {}

    pub async fn start(pipe: &str, keys: Vec<PrivateKey>) {
        let first = ServerOptions::new()
            .first_pipe_instance(true)
            .create(pipe)
            .unwrap_or_else(|e| panic!("create the pipe {pipe:?}: {e}"));
        // Prepare the next instance before serving this one, so a client never finds the pipe busy.
        let listener = futures::stream::unfold((first, pipe.to_string()), |(current, name)| async move {
            let next = ServerOptions::new().create(&name).expect("create the next pipe instance");
            let connected = current.connect().await.map(|()| current);
            Some((connected, (next, name)))
        })
        .boxed();
        tokio::spawn(async move {
            let _ = server::serve(listener, AllowAll).await;
        });

        let mut client = AgentClient::connect_named_pipe(pipe).await.expect("reach the fake agent");
        for key in keys {
            client.add_identity(&key, &[]).await.expect("load the key into the agent");
        }
    }
}

/// The agent is found through an environment variable, so tests that use one take turns.
#[cfg(windows)]
static AGENT_ENV: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The agent holds the private key; Aether only ever asks it to sign. This is the path "agent" sign-in takes
/// with the Windows OpenSSH agent, 1Password, KeePassXC and friends, all of which expose a pipe.
#[cfg(windows)]
#[tokio::test]
async fn an_agent_can_sign_in_for_us_without_aether_seeing_the_key() {
    let _turn = AGENT_ENV.lock().await;
    let generated = generate_key(KeyKind::Ed25519, "held-by-the-agent", None).unwrap();
    let public = russh::keys::PublicKey::from_openssh(&generated.meta.public_key).unwrap();
    let private = russh::keys::decode_secret_key(&generated.private_openssh, None).unwrap();

    let pipe = format!(r"\\.\pipe\aether-test-agent-{}", uuid::Uuid::new_v4().simple());
    fake_agent::start(&pipe, vec![private]).await;
    // `connect_openssh` honours SSH_AUTH_SOCK when it names a pipe (the same convention Git for Windows uses).
    // SAFETY: only this test reads a pipe-valued SSH_AUTH_SOCK, and it sets it before connecting.
    unsafe { std::env::set_var("SSH_AUTH_SOCK", &pipe) };

    let server = TestServer::start(TestServerOptions {
        users: vec![TestUser::with_key("agentuser", public)],
        ..Default::default()
    })
    .await;
    let env = env(ScriptedDelegate::default());
    let mut s = spec(&server, "agentuser", vec![AuthMethodKind::Agent]);
    s.auth.agent = aether_core::model::AgentMode::OpenSsh;

    let conn = SshConnection::connect(&s, &env.ctx).await.expect("the key the agent holds is accepted");
    let out = conn.exec("whoami", Duration::from_secs(5)).await.unwrap();
    assert_eq!(out.stdout_text(), "agentuser\n");
    assert!(env.delegate.prompts.lock().is_empty(), "nothing is asked of the user");
    conn.close().await;
}

/// `IdentitiesOnly yes` with `IdentityFile key.pub` is how an agent that holds many keys (1Password, a YubiKey,
/// Pageant) is told which one to use: the agent signs, for that key only, and nothing else it holds is offered to the
/// server — each offer is an attempt that counts toward the server's limit.
#[cfg(windows)]
#[tokio::test]
async fn identities_only_with_a_public_key_file_picks_one_key_out_of_the_agent() {
    let _turn = AGENT_ENV.lock().await;
    let wanted = generate_key(KeyKind::Ed25519, "wanted", None).unwrap();
    let other = generate_key(KeyKind::Ed25519, "other", None).unwrap();
    let absent = generate_key(KeyKind::Ed25519, "never-added-to-the-agent", None).unwrap();
    let private = |k: &keys::GeneratedKey| russh::keys::decode_secret_key(&k.private_openssh, None).unwrap();

    let pipe = format!(r"\\.\pipe\aether-test-agent-{}", uuid::Uuid::new_v4().simple());
    // The key nobody asked for comes first, so that taking "the first key the agent has" would be caught.
    fake_agent::start(&pipe, vec![private(&other), private(&wanted)]).await;
    // SAFETY: tests that read a pipe-valued SSH_AUTH_SOCK take turns (AGENT_ENV) and set it before connecting.
    unsafe { std::env::set_var("SSH_AUTH_SOCK", &pipe) };

    let dir = tempfile::tempdir().unwrap();
    let wanted_pub = dir.path().join("wanted.pub");
    std::fs::write(&wanted_pub, format!("{}\n", wanted.meta.public_key)).unwrap();
    let absent_pub = dir.path().join("absent.pub");
    std::fs::write(&absent_pub, format!("{}\n", absent.meta.public_key)).unwrap();

    let server = TestServer::start(TestServerOptions {
        users: vec![TestUser::with_key(
            "agentuser",
            russh::keys::PublicKey::from_openssh(&wanted.meta.public_key).unwrap(),
        )],
        ..Default::default()
    })
    .await;
    // The fingerprints of the keys offered to the server, in order.
    let offered = |server: &TestServer| -> Vec<String> {
        server
            .recorded()
            .auth_attempts
            .iter()
            .filter_map(|a| a.strip_prefix("publickey:agentuser:"))
            .map(String::from)
            .collect()
    };
    let fingerprint = |k: &keys::GeneratedKey| {
        let public = russh::keys::PublicKey::from_openssh(&k.meta.public_key).unwrap();
        aether_core::known_hosts::fingerprint_sha256(&russh::keys::PublicKeyBase64::public_key_bytes(&public))
    };
    let (wanted_fp, other_fp) = (fingerprint(&wanted), fingerprint(&other));
    let env = env(ScriptedDelegate::default());
    let configured = |key: &std::path::Path, identities_only: bool| {
        let mut s = spec(&server, "agentuser", vec![AuthMethodKind::Agent, AuthMethodKind::PublicKey]);
        s.auth.agent = aether_core::model::AgentMode::OpenSsh;
        s.auth.identities_only = identities_only;
        s.auth.key = Some(KeyRef::File(key.to_path_buf()));
        s
    };

    // The public file names the wanted key: it signs in, and the other key is never offered. (The agent lists its keys
    // in no fixed order, so this is done several times: taking whatever comes first would be caught.)
    for _ in 0..8 {
        let conn =
            SshConnection::connect(&configured(&wanted_pub, true), &env.ctx).await.expect("the named key signs in");
        conn.close().await;
    }
    assert_eq!(offered(&server), vec![wanted_fp.clone(); 8], "only the key the .pub file names was offered");

    // A file naming a key the agent doesn't hold: nothing is offered — and it is not reported as an unreadable key.
    let err =
        SshConnection::connect(&configured(&absent_pub, true), &env.ctx).await.err().expect("nothing to sign with");
    assert!(matches!(err, SshError::NoAuthMethods), "{err:?}");
    assert_eq!(offered(&server).len(), 8, "no other key was tried in its place");

    // Without IdentitiesOnly the agent's keys are all fair game, and a .pub file is still not a reason to fail.
    let conn =
        SshConnection::connect(&configured(&absent_pub, false), &env.ctx).await.expect("the agent's keys are tried");
    conn.close().await;
    let seen = offered(&server);
    assert!(seen[8..].contains(&wanted_fp), "{seen:?}");
    assert!(!seen[..8].contains(&other_fp), "{seen:?}");
}
