//! SFTP and transfer-manager tests against the in-process server's real directory.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use aether_core::error::UserFacing;
use aether_core::known_hosts::KnownHosts;
use aether_core::model::AuthMethodKind;
use aether_core::{ConnectionId, TransferId};
use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::testing::delegate::ScriptedDelegate;
use crate::testing::{TestServer, TestServerOptions};
use crate::*;

struct Fixture {
    _server: TestServer,
    remote_root: TempDir,
    local: TempDir,
    _conn: SshConnection,
    client: SftpClient,
}

async fn fixture() -> Fixture {
    fixture_with(|_| {}).await
}

/// A fixture whose server also lists `phantoms`: names no real file system would hold.
async fn fixture_with_phantoms(phantoms: Vec<String>) -> Fixture {
    fixture_with(|options| options.sftp_phantoms = phantoms).await
}

/// A fixture whose server is set up by `configure` (faults to inject, names to invent).
async fn fixture_with(configure: impl FnOnce(&mut TestServerOptions)) -> Fixture {
    let remote_root = tempfile::tempdir().unwrap();
    let mut options = TestServerOptions { sftp_root: Some(remote_root.path().to_path_buf()), ..Default::default() };
    configure(&mut options);
    let server = TestServer::start(options).await;
    let local = tempfile::tempdir().unwrap();
    let ctx = ConnectContext {
        delegate: Arc::new(ScriptedDelegate::answering(&["secret"])),
        known_hosts: Arc::new(KnownHosts::new(local.path().join("known_hosts"), None)),
        cancel: CancellationToken::new(),
    };
    let mut spec = ConnectSpec::new("127.0.0.1", server.port(), "tester");
    spec.auth.methods = vec![AuthMethodKind::Password];
    spec.auth.default_identity_files = Vec::new();
    let conn = SshConnection::connect(&spec, &ctx).await.expect("connect");
    let client = conn.open_sftp().await.expect("sftp");
    Fixture { _server: server, remote_root, local, _conn: conn, client }
}

fn pseudo_random(len: usize, seed: u8) -> Vec<u8> {
    let mut x = u32::from(seed) + 12345;
    (0..len)
        .map(|_| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (x >> 24) as u8
        })
        .collect()
}

fn digest(path: &Path) -> String {
    let bytes = std::fs::read(path).unwrap();
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Sorted `(relative path, size or None for dirs)` of everything under `root`.
fn tree(root: &Path) -> Vec<(String, Option<u64>)> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, Option<u64>)>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let rel = e.path().strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            if e.path().is_dir() {
                out.push((rel, None));
                walk(&e.path(), root, out);
            } else {
                out.push((rel, Some(e.metadata().unwrap().len())));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out
}

#[derive(Default)]
struct Collector {
    infos: Mutex<Vec<TransferInfo>>,
    notify: Notify,
}

impl TransferEvents for Collector {
    fn changed(&self, info: &TransferInfo) {
        self.infos.lock().push(info.clone());
        self.notify.notify_waiters();
    }
}

impl Collector {
    async fn wait_for(&self, id: TransferId, want: TransferState) -> TransferInfo {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            let notified = self.notify.notified();
            if let Some(info) = self.infos.lock().iter().rev().find(|i| i.id == id).filter(|i| i.state == want) {
                return info.clone();
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                let last = self.infos.lock().iter().rev().find(|i| i.id == id).cloned();
                panic!("timed out waiting for {want:?}; last seen: {last:?}");
            }
        }
    }
}

// ── saving a file ────────────────────────────────────────────────────────────

fn names_in(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> =
        std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into()).collect();
    v.sort();
    v
}

#[tokio::test]
async fn saving_replaces_the_file_whole_keeps_its_permissions_and_leaves_no_litter() {
    // Not the default mode: a save that hard-coded one would pass otherwise.
    let f = fixture_with(|o| o.sftp_modes = vec![("conf.ini".into(), 0o640)]).await;
    let root = f.remote_root.path();
    std::fs::write(root.join("conf.ini"), "old\r\ncontent\r\n").unwrap();

    f.client.replace_file("/conf.ini", b"brand new").await.unwrap();
    assert_eq!(std::fs::read_to_string(root.join("conf.ini")).unwrap(), "brand new");
    assert_eq!(names_in(root), ["conf.ini"], "neither the temporary file nor the put-aside original remain");
    // The new file was given the old one's permissions before it took its place.
    let chmods = f._server.recorded().chmods;
    assert!(chmods.iter().any(|(path, mode)| path.contains(".conf.ini.aether-") && *mode == 0o640), "{chmods:?}");

    // A file that doesn't exist yet is simply created.
    f.client.replace_file("/fresh.txt", b"hello").await.unwrap();
    assert_eq!(std::fs::read_to_string(root.join("fresh.txt")).unwrap(), "hello");
    assert_eq!(names_in(root), ["conf.ini", "fresh.txt"]);
}

/// The whole editor round trip as the app performs it: open, edit one word in a text box's terms, save. Everything the
/// text box can't hold — the BOM, the Windows line endings — is back exactly as it was.
#[tokio::test]
async fn a_windows_file_edited_and_saved_changes_only_the_edit() {
    let f = fixture().await;
    let original = b"\xEF\xBB\xBF[core]\r\n\tname = caf\xc3\xa9\r\n\tmode = old\r\n";
    std::fs::write(f.remote_root.path().join("settings.ini"), original).unwrap();

    let stat = f.client.stat("/settings.ini").await.unwrap();
    let (bytes, truncated) = f.client.read_prefix("/settings.ini", 1 << 20).await.unwrap();
    let file = decode_text(&bytes, truncated, FileVersion { size: stat.size, modified: stat.modified }).unwrap();
    assert_eq!(file.text, "[core]\n\tname = café\n\tmode = old\n", "what the text box shows");

    let edited = file.text.replace("old", "new");
    f.client.replace_file("/settings.ini", &encode_text(&edited, file.line_ending, file.bom)).await.unwrap();
    assert_eq!(
        std::fs::read(f.remote_root.path().join("settings.ini")).unwrap(),
        b"\xEF\xBB\xBF[core]\r\n\tname = caf\xc3\xa9\r\n\tmode = new\r\n"
    );
}

/// A secret saved over must not sit readable under the server's default mode while the new contents arrive: the temporary
/// file is created with the old file's permissions, not given them afterwards.
#[tokio::test]
async fn the_new_file_is_created_with_the_old_ones_permissions_from_the_start() {
    let f = fixture_with(|o| o.sftp_modes = vec![(".env".into(), 0o600)]).await;
    let root = f.remote_root.path();
    std::fs::write(root.join(".env"), "TOKEN=old").unwrap();

    f.client.replace_file("/.env", b"TOKEN=new").await.unwrap();
    assert_eq!(std::fs::read_to_string(root.join(".env")).unwrap(), "TOKEN=new");
    let creates = f._server.recorded().creates;
    let temp = creates.iter().find(|(path, _)| path.contains(".env.aether-")).expect("a temporary file was made");
    assert_eq!(temp.1, Some(0o600), "asked for at creation: {creates:?}");

    // A file that is new has no permissions to keep, so none are asked for.
    f.client.replace_file("/fresh.txt", b"hello").await.unwrap();
    let creates = f._server.recorded().creates;
    let fresh = creates.iter().find(|(path, _)| path.contains("fresh.txt.aether-")).expect("a temporary file was made");
    assert_eq!(fresh.1, None, "{creates:?}");
}

/// A gateway that allows no `setstat` can't give the new file the old one's permissions. The old one is written over.
#[tokio::test]
async fn a_server_that_will_not_change_permissions_gets_an_in_place_save() {
    let f = fixture_with(|o| o.sftp_refuse_setstat = true).await;
    let root = f.remote_root.path();
    std::fs::write(root.join("conf.ini"), "old").unwrap();

    f.client.replace_file("/conf.ini", b"edited").await.unwrap();
    assert_eq!(std::fs::read_to_string(root.join("conf.ini")).unwrap(), "edited");
    assert_eq!(names_in(root), ["conf.ini"], "the temporary file was cleared away");
}

/// Whatever the reason the server won't move the file — a bind-mounted /etc/hosts answers with a plain failure, not a
/// refusal — it can still be written.
#[tokio::test]
async fn a_file_the_server_cannot_move_for_any_reason_gets_an_in_place_save() {
    let f = fixture_with(|o| o.sftp_fail_rename_of = vec!["/conf.ini".into()]).await;
    let root = f.remote_root.path();
    std::fs::write(root.join("conf.ini"), "old").unwrap();

    f.client.replace_file("/conf.ini", b"edited").await.unwrap();
    assert_eq!(std::fs::read_to_string(root.join("conf.ini")).unwrap(), "edited");
    assert_eq!(names_in(root), ["conf.ini"]);
}

/// Saving "over" a folder is a mistake, not an invitation to move the folder aside.
#[tokio::test]
async fn a_folder_is_never_replaced_by_a_file() {
    let f = fixture().await;
    let root = f.remote_root.path();
    std::fs::create_dir(root.join("docs")).unwrap();
    std::fs::write(root.join("docs").join("keep.txt"), "kept").unwrap();

    let err = f.client.replace_file("/docs", b"oops").await.unwrap_err();
    assert!(matches!(&err, SshError::Invalid(m) if m.contains("is a folder")), "{err:?}");
    assert_eq!(std::fs::read_to_string(root.join("docs").join("keep.txt")).unwrap(), "kept");
    assert_eq!(names_in(root), ["docs"]);
}

/// The link goes at the worst moment: the new file is in place, but nobody can ask the server so (the fake server stops
/// answering stat once something has been renamed). The error says where the files are instead of guessing.
#[tokio::test]
async fn a_save_cut_off_at_the_worst_moment_says_where_the_files_are() {
    let f = fixture_with(|o| {
        o.sftp_lose_rename_reply_to = vec!["/conf.ini".into()];
        o.sftp_fail_stat_after_rename = true;
    })
    .await;
    let root = f.remote_root.path();
    std::fs::write(root.join("conf.ini"), "precious").unwrap();

    let err = f.client.replace_file("/conf.ini", b"replacement").await.unwrap_err();
    match &err {
        SshError::SaveInterrupted { target, original, new } => {
            assert_eq!(target, "/conf.ini");
            assert!(original.contains(".conf.ini.aether-") && original.ends_with(".old"), "{original}");
            assert!(new.contains(".conf.ini.aether-") && new.ends_with(".tmp"), "{new}");
        }
        other => panic!("expected SaveInterrupted, got {other:?}"),
    }
    let info = err.info();
    assert!(info.message.contains("/conf.ini") && info.message.contains(".old"), "{}", info.message);
    // Nothing was thrown away: the original is under the name the error gave.
    let kept: Vec<String> = names_in(root).into_iter().filter(|n| n.ends_with(".old")).collect();
    assert_eq!(kept.len(), 1);
    assert_eq!(std::fs::read_to_string(root.join(&kept[0])).unwrap(), "precious");
}

/// The reason saves go through a temporary file: writing in place truncates first, so a failure halfway leaves a
/// damaged file. Here the new contents can't even be started, and the old file is exactly as it was.
#[tokio::test]
async fn a_save_that_fails_early_leaves_the_original_exactly_as_it_was() {
    let f = fixture_with(|o| o.sftp_fail_create = vec![".aether-".into()]).await;
    let root = f.remote_root.path();
    std::fs::write(root.join("conf.ini"), "precious").unwrap();

    assert!(f.client.replace_file("/conf.ini", b"replacement").await.is_err());
    assert_eq!(std::fs::read_to_string(root.join("conf.ini")).unwrap(), "precious");
    assert_eq!(names_in(root), ["conf.ini"]);
}

/// A folder that won't take new files still lets you edit the files in it: the contents are written in place.
#[tokio::test]
async fn a_folder_closed_to_new_files_gets_an_in_place_save() {
    let f = fixture_with(|o| o.sftp_refuse_create = vec![".aether-".into()]).await;
    let root = f.remote_root.path();
    std::fs::write(root.join("conf.ini"), "old").unwrap();

    f.client.replace_file("/conf.ini", b"edited").await.unwrap();
    assert_eq!(std::fs::read_to_string(root.join("conf.ini")).unwrap(), "edited");
    assert_eq!(names_in(root), ["conf.ini"]);

    // Where there is no old file to write over, the refusal is the answer.
    assert!(f.client.replace_file("/new.ini", b"x").await.is_err());
}

/// Sticky folders let you add files but not move other people's. The file itself may still be written.
#[tokio::test]
async fn a_folder_that_will_not_move_the_original_gets_an_in_place_save() {
    let f = fixture_with(|o| o.sftp_deny_rename_of = vec!["/conf.ini".into()]).await;
    let root = f.remote_root.path();
    std::fs::write(root.join("conf.ini"), "old").unwrap();

    f.client.replace_file("/conf.ini", b"edited").await.unwrap();
    assert_eq!(std::fs::read_to_string(root.join("conf.ini")).unwrap(), "edited");
    assert_eq!(names_in(root), ["conf.ini"], "the temporary file was cleared away");
}

/// A swap makes a new file, which belongs to whoever saved. Someone else's file you're allowed to write must stay
/// theirs, so it is written in place — a web server's config saved by an admin must not become the admin's.
#[tokio::test]
async fn saving_someone_elses_file_writes_in_place_and_keeps_the_owner() {
    let f = fixture_with(|o| o.sftp_owners = vec![("conf.ini".into(), 33, 33)]).await;
    let root = f.remote_root.path();
    std::fs::write(root.join("conf.ini"), "old").unwrap();

    f.client.replace_file("/conf.ini", b"edited").await.unwrap();
    assert_eq!(std::fs::read_to_string(root.join("conf.ini")).unwrap(), "edited");
    assert_eq!(names_in(root), ["conf.ini"]);
    assert_eq!(f.client.stat("/conf.ini").await.unwrap().owner.as_deref(), Some("33"), "still theirs");
    assert!(f._server.recorded().chmods.is_empty(), "no new file was made up, so none was given permissions");
}

/// Where the session may give files away (root), a swap can hand the new file back to the old owner — so it is used,
/// and the owner is the same afterwards.
#[tokio::test]
async fn saving_as_root_keeps_the_owner_through_the_swap() {
    let f = fixture_with(|o| {
        o.sftp_owners = vec![("conf.ini".into(), 33, 33)];
        o.sftp_can_chown = true;
    })
    .await;
    let root = f.remote_root.path();
    std::fs::write(root.join("conf.ini"), "old").unwrap();

    f.client.replace_file("/conf.ini", b"edited").await.unwrap();
    assert_eq!(std::fs::read_to_string(root.join("conf.ini")).unwrap(), "edited");
    assert_eq!(names_in(root), ["conf.ini"]);
    assert_eq!(f.client.stat("/conf.ini").await.unwrap().owner.as_deref(), Some("33"));
    let chmods = f._server.recorded().chmods;
    assert!(chmods.iter().any(|(path, _)| path.contains(".conf.ini.aether-")), "it went through a swap: {chmods:?}");
}

/// The server put the original aside but the answer never came. The link is fine, so what is where is found out and the
/// original is put back; swapping evidently doesn't work for this file, so it is written over in place, and the save
/// succeeds. Nothing was ever lost along the way.
#[tokio::test]
async fn a_lost_answer_to_putting_the_original_aside_is_put_right_and_the_save_goes_on() {
    let f = fixture_with(|o| o.sftp_lose_rename_reply_to = vec![".old".into()]).await;
    let root = f.remote_root.path();
    std::fs::write(root.join("conf.ini"), "precious").unwrap();

    f.client.replace_file("/conf.ini", b"replacement").await.unwrap();
    assert_eq!(std::fs::read_to_string(root.join("conf.ini")).unwrap(), "replacement");
    assert_eq!(names_in(root), ["conf.ini"], "nothing was left lying around");
}

/// The same, with a server that then can't be asked what happened: the error doesn't pretend to know.
#[tokio::test]
async fn a_lost_answer_nobody_can_check_loses_nothing_either() {
    let f = fixture_with(|o| {
        o.sftp_lose_rename_reply_to = vec![".old".into()];
        o.sftp_fail_stat_after_rename = true;
    })
    .await;
    let root = f.remote_root.path();
    std::fs::write(root.join("conf.ini"), "precious").unwrap();

    let err = f.client.replace_file("/conf.ini", b"replacement").await.unwrap_err();
    assert!(matches!(err, SshError::SaveInterrupted { .. }), "{err:?}");
    // The original was put aside, and is right there under the name the error gave.
    let kept: Vec<String> = names_in(root).into_iter().filter(|n| n.ends_with(".old")).collect();
    assert_eq!(kept.len(), 1, "{:?}", names_in(root));
    assert_eq!(std::fs::read_to_string(root.join(&kept[0])).unwrap(), "precious");
}

/// The swap itself happened and only the answer was lost. The new contents are in place, so the save is not reported
/// as failed (which would have the person save again over a file that is fine).
#[tokio::test]
async fn a_lost_answer_to_the_swap_is_not_a_failed_save() {
    let f = fixture_with(|o| o.sftp_lose_rename_reply_to = vec!["/conf.ini".into()]).await;
    let root = f.remote_root.path();
    std::fs::write(root.join("conf.ini"), "precious").unwrap();

    f.client.replace_file("/conf.ini", b"replacement").await.unwrap();
    assert_eq!(std::fs::read_to_string(root.join("conf.ini")).unwrap(), "replacement");
    assert_eq!(names_in(root), ["conf.ini"], "the put-aside original was cleared away");
}

/// The new contents are written; moving them into place fails after the original was put aside. It comes back.
#[tokio::test]
async fn a_save_that_fails_at_the_swap_puts_the_original_back() {
    let f = fixture_with(|o| o.sftp_refuse_rename_to = vec!["/conf.ini".into()]).await;
    let root = f.remote_root.path();
    std::fs::write(root.join("conf.ini"), "precious").unwrap();

    assert!(f.client.replace_file("/conf.ini", b"replacement").await.is_err());
    assert_eq!(std::fs::read_to_string(root.join("conf.ini")).unwrap(), "precious", "nothing was lost");
    assert_eq!(names_in(root), ["conf.ini"], "and nothing was left lying around");

    // The refusal was one-off: the next save goes through.
    f.client.replace_file("/conf.ini", b"replacement").await.unwrap();
    assert_eq!(std::fs::read_to_string(root.join("conf.ini")).unwrap(), "replacement");
}

// ── file operations ──────────────────────────────────────────────────────────

/// A folder with an absurd number of items is refused with an explanation rather than sent to the window whole.
#[tokio::test]
async fn a_folder_with_too_many_items_is_refused_not_listed() {
    let f = fixture().await;
    for i in 0..6 {
        std::fs::write(f.remote_root.path().join(format!("f{i}.txt")), "x").unwrap();
    }
    assert_eq!(f.client.list_up_to("/", 6).await.unwrap().len(), 6, "right at the limit is fine");
    let err = f.client.list_up_to("/", 5).await.unwrap_err();
    assert!(matches!(&err, SshError::Invalid(m) if m.contains("more than 5 items")), "{err:?}");
}

#[tokio::test]
async fn lists_and_manipulates_remote_files() {
    let f = fixture().await;
    let root = f.remote_root.path();
    std::fs::write(root.join("readme.txt"), "hello").unwrap();
    std::fs::create_dir(root.join("docs")).unwrap();

    assert_eq!(f.client.home(), "/");
    let mut list = f.client.list("/").await.unwrap();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(list.len(), 2);
    assert_eq!((list[0].name.as_str(), list[0].kind), ("docs", EntryKind::Dir));
    assert_eq!((list[1].name.as_str(), list[1].kind, list[1].size), ("readme.txt", EntryKind::File, 5));
    assert_eq!(list[1].path, "/readme.txt");

    f.client.mkdir("/made").await.unwrap();
    f.client.mkdir_all("/a/b/c").await.unwrap();
    assert!(root.join("a/b/c").is_dir());
    f.client.mkdir_all("/a/b/c").await.expect("idempotent");

    f.client.rename("/readme.txt", "/docs/renamed.txt").await.unwrap();
    assert!(root.join("docs/renamed.txt").is_file() && !root.join("readme.txt").exists());

    f.client.chmod("/docs/renamed.txt", 0o600).await.unwrap();
    assert_eq!(f._server.recorded().chmods, vec![("/docs/renamed.txt".to_string(), 0o600)]);

    let st = f.client.stat("/docs/renamed.txt").await.unwrap();
    assert_eq!(st.size, 5);
    assert!(f.client.exists("/docs").await.unwrap());
    assert!(!f.client.exists("/nope").await.unwrap());

    f.client.remove_file("/docs/renamed.txt").await.unwrap();
    f.client.remove_dir("/made").await.unwrap();
    assert!(!root.join("made").exists());

    let err = f.client.stat("/missing").await.err().unwrap();
    assert!(matches!(err, SshError::Sftp(_)));
}

#[tokio::test]
async fn recursive_removal_deletes_whole_trees() {
    let f = fixture().await;
    let root = f.remote_root.path();
    std::fs::create_dir_all(root.join("t/x/y")).unwrap();
    std::fs::write(root.join("t/a.txt"), "a").unwrap();
    std::fs::write(root.join("t/x/y/b.txt"), "b").unwrap();
    f.client.remove_recursive("/t").await.unwrap();
    assert!(!root.join("t").exists());
}

#[tokio::test]
async fn small_file_read_and_write() {
    let f = fixture().await;
    f.client.write_file("/note.txt", b"line one\nline two\n").await.unwrap();
    let (data, truncated) = f.client.read_prefix("/note.txt", 1024).await.unwrap();
    assert_eq!(data, b"line one\nline two\n");
    assert!(!truncated);
    let (data, truncated) = f.client.read_prefix("/note.txt", 4).await.unwrap();
    assert_eq!(data, b"line");
    assert!(truncated);
}

// ── single-file transfers ────────────────────────────────────────────────────

#[tokio::test]
async fn a_large_file_survives_a_round_trip_intact() {
    let f = fixture().await;
    let original = f.local.path().join("big.bin");
    std::fs::write(&original, pseudo_random(6 * 1024 * 1024 + 123, 7)).unwrap();

    let progress = AtomicU64::new(0);
    let counter = |n: u64| {
        progress.fetch_add(n, Ordering::Relaxed);
    };
    let started = std::time::Instant::now();
    let sent = f.client.upload_file(&original, "/big.bin", 0, &counter, &CancellationToken::new()).await.unwrap();
    assert_eq!(sent, 6 * 1024 * 1024 + 123);
    assert_eq!(progress.load(Ordering::Relaxed), sent);
    assert_eq!(digest(&original), digest(&f.remote_root.path().join("big.bin")));

    let back = f.local.path().join("back.bin");
    f.client.download_file("/big.bin", &back, 0, &|_| {}, &CancellationToken::new()).await.unwrap();
    assert_eq!(digest(&original), digest(&back));
    eprintln!("6 MiB up+down in {:?}", started.elapsed());
}

#[tokio::test]
async fn interrupted_uploads_and_downloads_resume_from_the_right_offset() {
    let f = fixture().await;
    let data = pseudo_random(2_000_000, 3);
    let local = f.local.path().join("src.bin");
    std::fs::write(&local, &data).unwrap();

    // Upload: the server already holds the first 700 000 bytes.
    std::fs::write(f.remote_root.path().join("part.bin"), &data[..700_000]).unwrap();
    let sent = f.client.upload_file(&local, "/part.bin", 700_000, &|_| {}, &CancellationToken::new()).await.unwrap();
    assert_eq!(sent, 1_300_000, "only the missing tail travels");
    assert_eq!(std::fs::read(f.remote_root.path().join("part.bin")).unwrap(), data);

    // Download: the local file holds the first 900 000 bytes.
    let dest = f.local.path().join("down.bin");
    std::fs::write(&dest, &data[..900_000]).unwrap();
    let got = f.client.download_file("/part.bin", &dest, 900_000, &|_| {}, &CancellationToken::new()).await.unwrap();
    assert_eq!(got, 1_100_000);
    assert_eq!(std::fs::read(&dest).unwrap(), data);
}

#[tokio::test]
async fn cancelling_a_transfer_stops_it_promptly() {
    let f = fixture().await;
    let local = f.local.path().join("huge.bin");
    std::fs::write(&local, pseudo_random(8_000_000, 1)).unwrap();
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    let seen = AtomicU64::new(0);
    let on_bytes = move |n: u64| {
        if seen.fetch_add(n, Ordering::Relaxed) + n > 1_000_000 {
            trigger.cancel();
        }
    };
    let err = f.client.upload_file(&local, "/huge.bin", 0, &on_bytes, &cancel).await.err().unwrap();
    assert!(matches!(err, SshError::Cancelled), "{err:?}");
    let on_server = std::fs::metadata(f.remote_root.path().join("huge.bin")).map(|m| m.len()).unwrap_or(0);
    assert!(on_server < 8_000_000, "stopped early ({on_server} bytes arrived)");
}

// ── transfer manager ─────────────────────────────────────────────────────────

fn make_local_tree(root: &Path) {
    std::fs::create_dir_all(root.join("project/src/deep")).unwrap();
    std::fs::create_dir_all(root.join("project/empty")).unwrap();
    std::fs::write(root.join("project/README.md"), "# project\n").unwrap();
    std::fs::write(root.join("project/src/main.rs"), pseudo_random(5000, 1)).unwrap();
    std::fs::write(root.join("project/src/deep/data.bin"), pseudo_random(300_000, 2)).unwrap();
    std::fs::write(root.join("project/src/deep/zero.txt"), "").unwrap();
}

#[tokio::test]
async fn whole_directory_trees_upload_and_download() {
    let f = fixture().await;
    let src = f.local.path().join("src_tree");
    make_local_tree(&src);

    let events = Arc::new(Collector::default());
    let manager = TransferManager::new(events.clone(), 2);

    // Upload the folder.
    let plan = plan_upload(&f.client, &[src.join("project")], "/").await.unwrap();
    assert_eq!(plan.file_count, 4);
    assert!(plan.conflicts.is_empty());
    let up = manager.start(TransferRequest {
        connection: ConnectionId::new(),
        client: f.client.clone(),
        direction: TransferDirection::Upload,
        plan,
        policy: ConflictPolicy::Overwrite,
        options: TransferOptions::default(),
        reveal: None,
    });
    let done = events.wait_for(up, TransferState::Completed).await;
    assert_eq!(done.files_done, 4);
    assert_eq!(done.bytes_done, done.bytes_total);
    assert_eq!(done.title, "project");
    assert_eq!(tree(&src), tree(f.remote_root.path()), "remote mirrors local");

    // Download it somewhere else.
    let dst = f.local.path().join("dst_tree");
    std::fs::create_dir_all(&dst).unwrap();
    let plan = plan_download(&f.client, &["/project".to_string()], &dst).await.unwrap();
    assert_eq!(plan.file_count, 4);
    let down = manager.start(TransferRequest {
        connection: ConnectionId::new(),
        client: f.client.clone(),
        direction: TransferDirection::Download,
        plan,
        policy: ConflictPolicy::Overwrite,
        options: TransferOptions::default(),
        reveal: Some(dst.display().to_string()),
    });
    let done = events.wait_for(down, TransferState::Completed).await;
    assert_eq!(done.reveal.as_deref(), Some(dst.display().to_string().as_str()));
    assert_eq!(tree(&dst), tree(&src), "round trip preserves the tree, including empty folders");
    assert_eq!(digest(&dst.join("project/src/deep/data.bin")), digest(&src.join("project/src/deep/data.bin")));
    assert!(!tree(&dst).iter().any(|(p, _)| p.ends_with(".aetherpart")), "no partial files are left behind");
}

#[tokio::test]
async fn conflicts_are_detected_and_each_policy_does_what_it_says() {
    let f = fixture().await;
    let local = f.local.path().join("report.txt");
    std::fs::write(&local, "NEW CONTENT").unwrap();
    std::fs::write(f.remote_root.path().join("report.txt"), "old content").unwrap();

    let events = Arc::new(Collector::default());
    let manager = TransferManager::new(events.clone(), 2);

    let plan = plan_upload(&f.client, std::slice::from_ref(&local), "/").await.unwrap();
    assert_eq!(plan.conflicts, vec!["/report.txt"]);

    // Skip: the server's copy is untouched, progress still completes.
    let id = manager.start(TransferRequest {
        connection: ConnectionId::new(),
        client: f.client.clone(),
        direction: TransferDirection::Upload,
        plan: plan.clone(),
        policy: ConflictPolicy::Skip,
        options: TransferOptions::default(),
        reveal: None,
    });
    let done = events.wait_for(id, TransferState::Completed).await;
    assert_eq!(done.skipped, 1);
    assert_eq!(std::fs::read_to_string(f.remote_root.path().join("report.txt")).unwrap(), "old content");

    // Keep both: the upload lands beside the original.
    let id = manager.start(TransferRequest {
        connection: ConnectionId::new(),
        client: f.client.clone(),
        direction: TransferDirection::Upload,
        plan: plan.clone(),
        policy: ConflictPolicy::KeepBoth,
        options: TransferOptions::default(),
        reveal: None,
    });
    events.wait_for(id, TransferState::Completed).await;
    assert_eq!(std::fs::read_to_string(f.remote_root.path().join("report (1).txt")).unwrap(), "NEW CONTENT");
    assert_eq!(std::fs::read_to_string(f.remote_root.path().join("report.txt")).unwrap(), "old content");

    // Overwrite replaces it.
    let id = manager.start(TransferRequest {
        connection: ConnectionId::new(),
        client: f.client.clone(),
        direction: TransferDirection::Upload,
        plan,
        policy: ConflictPolicy::Overwrite,
        options: TransferOptions::default(),
        reveal: None,
    });
    events.wait_for(id, TransferState::Completed).await;
    assert_eq!(std::fs::read_to_string(f.remote_root.path().join("report.txt")).unwrap(), "NEW CONTENT");

    // Downloads detect local conflicts too.
    let dest = f.local.path().join("dl");
    std::fs::create_dir_all(&dest).unwrap();
    std::fs::write(dest.join("report.txt"), "local old").unwrap();
    let plan = plan_download(&f.client, &["/report.txt".to_string()], &dest).await.unwrap();
    assert_eq!(plan.conflicts.len(), 1);
    let id = manager.start(TransferRequest {
        connection: ConnectionId::new(),
        client: f.client.clone(),
        direction: TransferDirection::Download,
        plan,
        policy: ConflictPolicy::KeepBoth,
        options: TransferOptions::default(),
        reveal: None,
    });
    events.wait_for(id, TransferState::Completed).await;
    assert_eq!(std::fs::read_to_string(dest.join("report (1).txt")).unwrap(), "NEW CONTENT");
    assert_eq!(std::fs::read_to_string(dest.join("report.txt")).unwrap(), "local old");
}

#[tokio::test]
async fn jobs_can_be_cancelled_and_leave_nothing_half_written() {
    let f = fixture().await;
    let big = f.remote_root.path().join("big.bin");
    std::fs::write(&big, pseudo_random(160_000_000, 9)).unwrap();

    let events = Arc::new(Collector::default());
    let manager = TransferManager::new(events.clone(), 1);
    let dest = f.local.path().join("out");
    std::fs::create_dir_all(&dest).unwrap();
    let plan = plan_download(&f.client, &["/big.bin".to_string()], &dest).await.unwrap();
    let id = manager.start(TransferRequest {
        connection: ConnectionId::new(),
        client: f.client.clone(),
        direction: TransferDirection::Download,
        plan,
        policy: ConflictPolicy::Overwrite,
        options: TransferOptions::default(),
        reveal: None,
    });

    // Wait until it is demonstrably running, then cancel.
    loop {
        if manager.list().iter().any(|i| i.id == id && i.bytes_done > 1_000_000) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    manager.cancel(id);
    let done = events.wait_for(id, TransferState::Cancelled).await;
    assert!(done.bytes_done < done.bytes_total);
    assert!(tree(&dest).is_empty(), "partial file removed: {:?}", tree(&dest));
}

#[tokio::test]
async fn paused_jobs_resume_where_they_stopped() {
    let f = fixture().await;
    let data = pseudo_random(160_000_000, 5);
    std::fs::write(f.remote_root.path().join("movie.bin"), &data).unwrap();

    let events = Arc::new(Collector::default());
    let manager = TransferManager::new(events.clone(), 1);
    let dest = f.local.path().join("out");
    std::fs::create_dir_all(&dest).unwrap();
    let plan = plan_download(&f.client, &["/movie.bin".to_string()], &dest).await.unwrap();
    let id = manager.start(TransferRequest {
        connection: ConnectionId::new(),
        client: f.client.clone(),
        direction: TransferDirection::Download,
        plan,
        policy: ConflictPolicy::Overwrite,
        options: TransferOptions::default(),
        reveal: None,
    });

    loop {
        if manager.list().iter().any(|i| i.id == id && i.bytes_done > 1_000_000) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    manager.pause(id);
    let paused = events.wait_for(id, TransferState::Paused).await;
    assert!(paused.bytes_done < paused.bytes_total);
    assert!(dest.join("movie.bin.aetherpart").exists(), "partial data is kept for resuming");

    // The app's UI thread has no Tokio context; resuming from it must not panic.
    std::thread::scope(|s| {
        s.spawn(|| manager.resume(id, None)).join().expect("resume must work from a thread without a runtime");
    });
    events.wait_for(id, TransferState::Completed).await;
    assert_eq!(std::fs::read(dest.join("movie.bin")).unwrap(), data);
}

#[tokio::test]
async fn failures_are_reported_per_job_without_stopping_the_others() {
    let f = fixture().await;
    let ok_file = f.local.path().join("fine.txt");
    std::fs::write(&ok_file, "ok").unwrap();
    // A destination folder that doesn't exist on the server makes uploads there fail.
    let events = Arc::new(Collector::default());
    let manager = TransferManager::new(events.clone(), 2);

    let bad_plan = plan_upload(&f.client, std::slice::from_ref(&ok_file), "/no/such/dir").await.unwrap();
    let bad = manager.start(TransferRequest {
        connection: ConnectionId::new(),
        client: f.client.clone(),
        direction: TransferDirection::Upload,
        plan: bad_plan,
        policy: ConflictPolicy::Overwrite,
        options: TransferOptions::default(),
        reveal: None,
    });
    let good_plan = plan_upload(&f.client, &[ok_file], "/").await.unwrap();
    let good = manager.start(TransferRequest {
        connection: ConnectionId::new(),
        client: f.client.clone(),
        direction: TransferDirection::Upload,
        plan: good_plan,
        policy: ConflictPolicy::Overwrite,
        options: TransferOptions::default(),
        reveal: None,
    });

    let failed = events.wait_for(bad, TransferState::Failed).await;
    assert!(failed.error.is_some());
    events.wait_for(good, TransferState::Completed).await;
    assert!(f.remote_root.path().join("fine.txt").is_file());

    manager.dismiss(bad);
    assert!(manager.list().iter().all(|i| i.id != bad));
}

/// The connection drops in the middle of a transfer. That is one failure with one error — not one per file in flight and
/// another for the job, which read as "4 items failed" for a single dropped connection.
#[tokio::test]
async fn a_lost_connection_is_one_error_not_many() {
    let f = fixture_with(|o| o.sftp_write_delay = Duration::from_millis(5)).await;
    let folder = f.local.path().join("batch");
    std::fs::create_dir(&folder).unwrap();
    for (i, seed) in (1u8..=5).enumerate() {
        std::fs::write(folder.join(format!("part-{i}.bin")), pseudo_random(2_000_000, seed)).unwrap();
    }
    let big = folder;
    let events = Arc::new(Collector::default());
    let manager = TransferManager::new(events.clone(), 2);
    let plan = plan_upload(&f.client, std::slice::from_ref(&big), "/").await.unwrap();
    let id = manager.start(TransferRequest {
        connection: ConnectionId::new(),
        client: f.client.clone(),
        direction: TransferDirection::Upload,
        plan,
        policy: ConflictPolicy::Overwrite,
        options: TransferOptions::default(),
        reveal: None,
    });

    // Cut the connection once some of the file is across.
    let started = tokio::time::Instant::now();
    while !events.infos.lock().iter().any(|i| i.id == id && i.bytes_done > 0) {
        assert!(started.elapsed() < Duration::from_secs(30), "the transfer never started");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    f._conn.close().await;

    let failed = events.wait_for(id, TransferState::Failed).await;
    let error = failed.error.expect("it says why");
    assert_ne!(error.title, "Some items weren't transferred", "one drop, one error: {error:?}");
}

/// The connection drops mid-transfer and comes back as a new one. The job carries on over the new channel it is handed,
/// and the file ends up whole.
#[tokio::test]
async fn a_failed_transfer_carries_on_over_a_fresh_connection() {
    let f = fixture_with(|o| o.sftp_write_delay = Duration::from_millis(5)).await;
    let big = f.local.path().join("big.bin");
    std::fs::write(&big, pseudo_random(8_000_000, 5)).unwrap();
    let events = Arc::new(Collector::default());
    let manager = TransferManager::new(events.clone(), 2);
    let plan = plan_upload(&f.client, std::slice::from_ref(&big), "/").await.unwrap();
    let id = manager.start(TransferRequest {
        connection: ConnectionId::new(),
        client: f.client.clone(),
        direction: TransferDirection::Upload,
        plan,
        policy: ConflictPolicy::Overwrite,
        options: TransferOptions::default(),
        reveal: None,
    });
    let started = tokio::time::Instant::now();
    while !events.infos.lock().iter().any(|i| i.id == id && i.bytes_done > 0) {
        assert!(started.elapsed() < Duration::from_secs(30), "the transfer never started");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    f._conn.close().await;
    events.wait_for(id, TransferState::Failed).await;
    let remote = f.remote_root.path().join("big.bin");
    assert!(std::fs::metadata(&remote).map(|m| m.len()).unwrap_or(0) < 8_000_000, "only part of it got across");

    // The same server, a new connection: its channel is what the job is given.
    let ctx = ConnectContext {
        delegate: Arc::new(ScriptedDelegate::answering(&["secret"])),
        known_hosts: Arc::new(KnownHosts::new(f.local.path().join("known_hosts_2"), None)),
        cancel: CancellationToken::new(),
    };
    let mut spec = ConnectSpec::new("127.0.0.1", f._server.port(), "tester");
    spec.auth.methods = vec![AuthMethodKind::Password];
    spec.auth.default_identity_files = Vec::new();
    let conn = SshConnection::connect(&spec, &ctx).await.expect("the new connection");
    let fresh = conn.open_sftp().await.expect("sftp on it");
    manager.resume(id, Some(fresh));
    events.wait_for(id, TransferState::Completed).await;
    assert_eq!(digest(&big), digest(&remote), "the file is whole");
}

// ── a hostile server ─────────────────────────────────────────────────────────

/// A server decides what its files are called. A client that joins those names onto the download folder lets it write
/// anywhere the user can: `..\..\Startup\x.bat`, `C:\x`, `\host\share\x` (which also makes Windows authenticate to
/// that host), `file:stream` (an NTFS alternate data stream), device names such as `aux.c`.
#[tokio::test]
async fn names_chosen_by_the_server_cannot_escape_the_download_folder() {
    let hostile: Vec<String> = [
        r"..\..\escaped.txt",
        r"..\escaped-too.txt",
        r"C:\aether-escape-test.txt",
        r"\\localhost\share\x.txt",
        "aux.c",
        "NUL",
        "CON.txt",
        "report: Q4?.txt",
        "trailing.dots...",
        "Readme",
        "README",
        "readme",
        "fine.txt",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let f = fixture_with_phantoms(hostile.clone()).await;
    std::fs::create_dir(f.remote_root.path().join("d")).unwrap();

    let dest = f.local.path().join("out");
    std::fs::create_dir_all(&dest).unwrap();
    let plan = plan_download(&f.client, &["/d".to_string()], &dest).await.unwrap();

    // Every local path the plan will touch is the destination plus plain names, nothing more.
    assert_eq!(plan.file_count as usize, hostile.len(), "every listed name is still downloaded, under a safe name");
    for item in &plan.items {
        let rel = item.local.strip_prefix(&dest).unwrap_or_else(|_| panic!("{:?} left the folder", item.local));
        assert!(rel.components().all(|c| matches!(c, std::path::Component::Normal(_))), "{:?}", item.local);
    }

    let manager = TransferManager::new(Arc::new(Collector::default()), 1);
    let events = Arc::new(Collector::default());
    let manager = {
        drop(manager);
        TransferManager::new(events.clone(), 1)
    };
    let id = manager.start(TransferRequest {
        connection: ConnectionId::new(),
        client: f.client.clone(),
        direction: TransferDirection::Download,
        plan,
        policy: ConflictPolicy::Overwrite,
        options: TransferOptions::default(),
        reveal: None,
    });
    let done = events.wait_for(id, TransferState::Completed).await;
    assert_eq!(done.files_done as usize, hostile.len());

    // Nothing appeared next to the download folder...
    let mut beside: Vec<String> = std::fs::read_dir(f.local.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    beside.sort();
    assert_eq!(beside, vec!["known_hosts".to_string(), "out".to_string()], "something was written outside the folder");

    // ...and inside it there are exactly the files we asked for, all with distinct names (case-insensitively).
    let inside: Vec<String> =
        tree(&dest).into_iter().filter(|(_, size)| size.is_some()).map(|(p, _)| p.to_lowercase()).collect();
    let unique: std::collections::HashSet<_> = inside.iter().collect();
    assert_eq!(inside.len(), hostile.len(), "{inside:?}");
    assert_eq!(unique.len(), inside.len(), "two server names collapsed into one local file: {inside:?}");
    assert!(inside.iter().all(|p| p.starts_with("d/")), "{inside:?}");
}

/// "Resume" continues what *this job* was writing. It must not graft the tail of a new file onto whatever happens to be
/// on the server under that name (here: an older, shorter copy of a file that was still queued when the pause came).
#[tokio::test]
async fn resuming_an_upload_never_grafts_onto_a_file_the_job_did_not_write() {
    let f = fixture().await;
    let src = f.local.path().join("up");
    std::fs::create_dir_all(&src).unwrap();
    let big = pseudo_random(160_000_000, 7);
    let small = pseudo_random(300_000, 8);
    std::fs::write(src.join("a-big.bin"), &big).unwrap();
    std::fs::write(src.join("b-small.bin"), &small).unwrap();
    // The server already has an older, shorter b-small.bin.
    std::fs::write(f.remote_root.path().join("b-small.bin"), pseudo_random(100_000, 9)).unwrap();

    let events = Arc::new(Collector::default());
    let manager = TransferManager::new(events.clone(), 1);
    let plan = plan_upload(&f.client, &[src.join("a-big.bin"), src.join("b-small.bin")], "/").await.unwrap();
    let id = manager.start(TransferRequest {
        connection: ConnectionId::new(),
        client: f.client.clone(),
        direction: TransferDirection::Upload,
        plan,
        policy: ConflictPolicy::Overwrite,
        options: TransferOptions { concurrency: 1, ..Default::default() },
        reveal: None,
    });

    // Pause while the big file is still going, so b-small.bin has not been touched yet.
    loop {
        if manager.list().iter().any(|i| i.id == id && i.bytes_done > 1_000_000) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    manager.pause(id);
    events.wait_for(id, TransferState::Paused).await;
    manager.resume(id, None);
    let done = events.wait_for(id, TransferState::Completed).await;

    assert_eq!(std::fs::read(f.remote_root.path().join("b-small.bin")).unwrap(), small, "the small file arrived whole");
    assert_eq!(
        std::fs::read(f.remote_root.path().join("a-big.bin")).unwrap(),
        big,
        "and the big one continued correctly"
    );
    // Progress is rebuilt on resume rather than carried over, so it can't exceed the total.
    assert!(done.bytes_done <= done.bytes_total, "{} > {}", done.bytes_done, done.bytes_total);
}
