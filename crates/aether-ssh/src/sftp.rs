//! SFTP: directory listing, file operations and single-file transfers.
//!
//! Recursive transfers, queues, pause/resume and progress events live in
//! [`transfers`](crate::transfers); this module is the thin typed layer over `russh-sftp`.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, UNIX_EPOCH};

use futures::StreamExt;
use russh::ChannelMsg;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::client::{Config as SftpConfig, SftpSession};
use russh_sftp::protocol::{FileAttributes, FileType, OpenFlags, StatusCode};
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;
use ts_rs::TS;

use crate::connection::{SshConnection, channel_open_error};
use crate::error::{Result, SshError};

/// Chunk size for streaming file contents.
const CHUNK: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
    Other,
}

/// One item in a remote directory listing.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RemoteEntry {
    pub name: String,
    pub path: String,
    /// What the entry is. For symlinks this is the kind of the *target* when it could be
    /// resolved, so a link to a folder behaves like a folder.
    pub kind: EntryKind,
    pub is_symlink: bool,
    #[ts(type = "number")]
    pub size: u64,
    /// Modification time, milliseconds since the Unix epoch.
    #[ts(type = "number | null")]
    pub modified: Option<i64>,
    /// Permission bits (`0o755` = 493).
    pub mode: Option<u32>,
    pub owner: Option<String>,
    pub group: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiskUsage {
    #[ts(type = "number")]
    pub total: u64,
    #[ts(type = "number")]
    pub free: u64,
}

/// Joins POSIX path segments, collapsing duplicate slashes.
pub fn join_remote(dir: &str, name: &str) -> String {
    if name.starts_with('/') {
        return name.to_string();
    }
    if dir.is_empty() || dir == "." {
        return name.to_string();
    }
    format!("{}/{}", dir.trim_end_matches('/'), name)
}

/// The parent directory of a POSIX path (`/` stays `/`).
pub fn parent_remote(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) => "/".to_string(),
        Some(i) => trimmed[..i].to_string(),
        None => {
            if path.starts_with('/') {
                "/".into()
            } else {
                ".".into()
            }
        }
    }
}

pub fn file_name_remote(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or(path)
}

fn to_millis(secs: Option<u32>) -> Option<i64> {
    secs.map(|s| i64::from(s) * 1000)
}

fn kind_of(attrs: &FileAttributes) -> EntryKind {
    match attrs.file_type() {
        FileType::Dir => EntryKind::Dir,
        FileType::File => EntryKind::File,
        FileType::Symlink => EntryKind::Symlink,
        FileType::Other => EntryKind::Other,
    }
}

fn entry(parent: &str, name: String, attrs: &FileAttributes) -> RemoteEntry {
    let kind = kind_of(attrs);
    RemoteEntry {
        path: join_remote(parent, &name),
        is_symlink: kind == EntryKind::Symlink,
        kind,
        size: attrs.size.unwrap_or(0),
        modified: to_millis(attrs.mtime),
        mode: attrs.permissions.map(|p| p & 0o7777),
        owner: attrs.user.clone().or_else(|| attrs.uid.map(|u| u.to_string())),
        group: attrs.group.clone().or_else(|| attrs.gid.map(|g| g.to_string())),
        name,
    }
}

/// How an attempt to save a file by swapping a new one in ended.
enum Swap {
    Done,
    /// The swap would have changed who owns the file, or the folder is closed to us: write in place instead.
    Unsuitable,
}

/// What a failed rename in a save turned out to have done.
#[derive(Debug, PartialEq, Eq)]
enum Settled {
    /// The original is in its place (it may have been put aside and was brought back).
    Original,
    /// The new file is in its place.
    NewIsIn,
    /// The server couldn't be asked (the link is gone), so nothing is known about what is where.
    Unknown,
}

/// Progress callback for a single file: `(bytes_done_in_this_call_so_far)`.
pub type ByteProgress<'a> = &'a (dyn Fn(u64) + Send + Sync);

#[derive(Clone)]
pub struct SftpClient {
    session: Arc<SftpSession>,
    home: String,
    /// Set once a request found the SFTP channel gone or stuck. The SSH connection may be perfectly alive, so the
    /// session opens a fresh SFTP channel on the next use instead of handing out this one forever.
    broken: Arc<AtomicBool>,
}

/// An SFTP error that means the channel itself is gone or stuck, as opposed to the server refusing one request.
fn channel_is_gone(e: &SftpError) -> bool {
    !matches!(e, SftpError::Status(_)) && crate::error::sftp_channel_is_gone(&e.to_string())
}

impl SshConnection {
    /// Starts the SFTP subsystem on a new channel.
    pub async fn open_sftp(&self) -> Result<SftpClient> {
        let mut channel =
            self.inner.handle.channel_open_session().await.map_err(|e| channel_open_error("an SFTP channel", e))?;
        channel.request_subsystem(true, "sftp").await?;

        // Wait for the yes/no so "SFTP is disabled" is a clear error, not a mystery EOF.
        let answer = async {
            loop {
                match channel.wait().await {
                    Some(ChannelMsg::Success) => return Ok(()),
                    Some(ChannelMsg::Failure) | Some(ChannelMsg::Close) | None => {
                        return Err(SshError::ChannelOpen {
                            what: "the SFTP subsystem".into(),
                            reason: "SFTP is not enabled for this account on the server".into(),
                        });
                    }
                    Some(_) => {}
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(20), answer)
            .await
            .map_err(|_| SshError::Disconnected("timed out starting SFTP".into()))??;

        let config = SftpConfig {
            max_concurrent_reads: 64,
            max_concurrent_writes: 64,
            request_timeout_secs: 60,
            ..Default::default()
        };
        let session = SftpSession::new_with_config(channel.into_stream(), config).await?;
        let home = session.canonicalize(".").await.unwrap_or_else(|_| "/".into());
        Ok(SftpClient { session: Arc::new(session), home, broken: Arc::new(AtomicBool::new(false)) })
    }
}

/// How long a whole directory read may take, however many entries there are. (Each request also has its own timeout.)
const LIST_TIMEOUT: Duration = Duration::from_secs(120);
/// More items than this in one folder is more than the file browser can show: one message of that size would stall the
/// window, so it says so instead.
const MAX_LIST_ENTRIES: usize = 100_000;
/// How many links are looked up at once, and for how long in all. A folder of thousands of symlinks must neither flood
/// the server with requests nor hold up the listing: links that don't answer in time stay shown as links.
const LINK_LOOKUPS_AT_ONCE: usize = 24;
const LINK_LOOKUP_BUDGET: Duration = Duration::from_secs(8);

impl SftpClient {
    /// The server-side default directory (usually the user's home).
    pub fn home(&self) -> &str {
        &self.home
    }

    /// Has a request found this client's channel gone or stuck? Then the session should open a new one.
    pub fn is_broken(&self) -> bool {
        self.broken.load(Ordering::SeqCst)
    }

    /// Turns a library error into ours, noting when the channel itself is the problem.
    fn note(&self, e: SftpError) -> SshError {
        if channel_is_gone(&e) {
            self.broken.store(true, Ordering::SeqCst);
        }
        SshError::from(e)
    }

    /// Turns a library result into ours, noting when the channel itself is the problem.
    fn guard<T>(&self, result: std::result::Result<T, SftpError>) -> Result<T> {
        result.map_err(|e| self.note(e))
    }

    pub async fn canonicalize(&self, path: &str) -> Result<String> {
        self.guard(self.session.canonicalize(path).await)
    }

    /// Lists a directory. Entries are unsorted; symlinks are resolved so links to folders
    /// can be opened like folders.
    pub async fn list(&self, path: &str) -> Result<Vec<RemoteEntry>> {
        self.list_up_to(path, MAX_LIST_ENTRIES).await
    }

    /// [`SftpClient::list`], refusing a folder with more than `limit` items.
    pub(crate) async fn list_up_to(&self, path: &str, limit: usize) -> Result<Vec<RemoteEntry>> {
        let read = tokio::time::timeout(LIST_TIMEOUT, self.session.read_dir(path)).await.map_err(|_| {
            self.broken.store(true, Ordering::SeqCst);
            SshError::Sftp(format!("Timeout: reading {path} took longer than {} seconds", LIST_TIMEOUT.as_secs()))
        })?;
        let dir = self.guard(read)?;
        // One more than the limit is enough to know it is over.
        let mut entries: Vec<RemoteEntry> =
            dir.take(limit.saturating_add(1)).map(|e| entry(path, e.file_name(), &e.metadata())).collect();
        if entries.len() > limit {
            return Err(SshError::Invalid(format!(
                "{path} holds more than {limit} items, which is more than Aether can show in one folder. Use a terminal to work with it."
            )));
        }

        // Look the links up a few at a time, within a time budget (a stat follows the link).
        let targets: Vec<(usize, String)> =
            entries.iter().enumerate().filter(|(_, e)| e.is_symlink).map(|(i, e)| (i, e.path.clone())).collect();
        if !targets.is_empty() {
            let session = self.session.clone();
            let mut lookups = futures::stream::iter(targets.into_iter().map(move |(i, target)| {
                let session = session.clone();
                async move { (i, session.metadata(target).await.ok()) }
            }))
            .buffer_unordered(LINK_LOOKUPS_AT_ONCE);
            let deadline = tokio::time::Instant::now() + LINK_LOOKUP_BUDGET;
            while let Ok(Some((i, md))) = tokio::time::timeout_at(deadline, lookups.next()).await {
                if let Some(md) = md {
                    entries[i].kind = kind_of(&md);
                    entries[i].size = md.size.unwrap_or(entries[i].size);
                }
            }
        }
        Ok(entries)
    }

    pub async fn stat(&self, path: &str) -> Result<RemoteEntry> {
        let md = self.guard(self.session.metadata(path).await)?;
        Ok(entry(&parent_remote(path), file_name_remote(path).to_string(), &md))
    }

    pub async fn exists(&self, path: &str) -> Result<bool> {
        self.guard(self.session.try_exists(path).await)
    }

    pub async fn mkdir(&self, path: &str) -> Result<()> {
        self.guard(self.session.create_dir(path).await)
    }

    /// `mkdir -p`
    pub async fn mkdir_all(&self, path: &str) -> Result<()> {
        let mut built = String::new();
        if path.starts_with('/') {
            built.push('/');
        }
        for part in path.split('/').filter(|p| !p.is_empty()) {
            built = join_remote(&built, part);
            if !self.exists(&built).await? {
                self.guard(self.session.create_dir(built.clone()).await)?;
            }
        }
        Ok(())
    }

    pub async fn rename(&self, from: &str, to: &str) -> Result<()> {
        self.guard(self.session.rename(from, to).await)
    }

    pub async fn remove_file(&self, path: &str) -> Result<()> {
        self.guard(self.session.remove_file(path).await)
    }

    pub async fn remove_dir(&self, path: &str) -> Result<()> {
        self.guard(self.session.remove_dir(path).await)
    }

    /// Deletes a file or a whole directory tree. Symlinks are removed, never followed.
    pub async fn remove_recursive(&self, path: &str) -> Result<()> {
        let md = self.guard(self.session.symlink_metadata(path).await)?;
        if md.file_type() != FileType::Dir {
            return self.remove_file(path).await;
        }
        let mut stack = vec![(path.to_string(), false)];
        while let Some((dir, visited)) = stack.pop() {
            if visited {
                self.remove_dir(&dir).await?;
                continue;
            }
            stack.push((dir.clone(), true));
            for e in self.guard(self.session.read_dir(dir.clone()).await)? {
                let child = e.path();
                if e.file_type() == FileType::Dir {
                    stack.push((child, false));
                } else {
                    self.remove_file(&child).await?;
                }
            }
        }
        Ok(())
    }

    /// Sets permission bits (`chmod`). `mode` is the low 12 bits, e.g. `0o755`.
    pub async fn chmod(&self, path: &str, mode: u32) -> Result<()> {
        let attrs = FileAttributes { permissions: Some(mode & 0o7777), ..Default::default() };
        self.guard(self.session.set_metadata(path, attrs).await)
    }

    pub async fn chmod_recursive(&self, path: &str, mode: u32) -> Result<()> {
        let mut stack = vec![path.to_string()];
        while let Some(p) = stack.pop() {
            let md = self.guard(self.session.symlink_metadata(p.clone()).await)?;
            if md.file_type() == FileType::Symlink {
                continue;
            }
            self.chmod(&p, mode).await?;
            if md.file_type() == FileType::Dir {
                for e in self.guard(self.session.read_dir(p).await)? {
                    stack.push(e.path());
                }
            }
        }
        Ok(())
    }

    pub async fn symlink(&self, target: &str, link_path: &str) -> Result<()> {
        // SFTP's argument order is famously inverted between servers; russh-sftp follows OpenSSH.
        self.guard(self.session.symlink(link_path, target).await)
    }

    pub async fn read_link(&self, path: &str) -> Result<String> {
        self.guard(self.session.read_link(path).await)
    }

    /// Free/total space via `statvfs@openssh.com`, when the server supports it.
    pub async fn disk_usage(&self, path: &str) -> Result<Option<DiskUsage>> {
        Ok(self.guard(self.session.fs_info(path).await)?.map(|s| DiskUsage {
            total: s.blocks.saturating_mul(s.fragment_size),
            free: s.blocks_avail.saturating_mul(s.fragment_size),
        }))
    }

    /// Reads up to `limit` bytes of a file (for previews/editing).
    pub async fn read_prefix(&self, path: &str, limit: usize) -> Result<(Vec<u8>, bool)> {
        let mut file = self.guard(self.session.open(path).await)?;
        let mut buf = vec![0u8; limit + 1];
        let mut filled = 0;
        while filled < buf.len() {
            let n = file.read(&mut buf[filled..]).await?;
            if n == 0 {
                break;
            }
            filled += n;
        }
        let truncated = filled > limit;
        buf.truncate(filled.min(limit));
        Ok((buf, truncated))
    }

    /// Replaces a file's contents *in place* — truncating it first. For files someone is going to read back as they
    /// were, use [`SftpClient::replace_file`].
    pub async fn write_file(&self, path: &str, data: &[u8]) -> Result<()> {
        let mut file = self.guard(self.session.create(path).await)?;
        file.write_all(data).await?;
        file.shutdown().await?;
        Ok(())
    }

    /// Replaces a file's contents so that a failure at any point leaves the old file as it was, and so that the file
    /// keeps its owner, group and permissions.
    ///
    /// The new contents are written next to it under a temporary name, given the old file's owner and permissions, and
    /// only then swapped in. The swap goes through a second temporary name, because `SSH_FXP_RENAME` refuses to replace an
    /// existing file on common servers (OpenSSH's included); whatever a failed or unanswered rename left behind is found
    /// and put right, so the original is back in its place or the new contents are.
    ///
    /// Not every file can be saved that way. A swap needs a folder you may add to, and it makes a *new* file, which
    /// belongs to you — so where that would change the owner (someone else's file you may write, a group you're not in)
    /// or the folder is closed to you, the contents are written in place instead, which leaves the owner alone.
    ///
    /// `path` is resolved first, so a symlink to the file is edited through rather than replaced by a regular file.
    pub async fn replace_file(&self, path: &str, data: &[u8]) -> Result<()> {
        let target = self.canonicalize(path).await.unwrap_or_else(|_| path.to_string());
        let old = self.guard(self.session.metadata(target.clone()).await).ok();
        if old.as_ref().is_some_and(|o| o.file_type() == FileType::Dir) {
            return Err(SshError::Invalid(format!("{target} is a folder, not a file.")));
        }
        match self.swap_in(&target, data, old.as_ref()).await? {
            Swap::Done => Ok(()),
            Swap::Unsuitable => self.write_file(&target, data).await,
        }
    }

    /// Writes `data` beside `target` and swaps it in; see [`SftpClient::replace_file`].
    async fn swap_in(&self, target: &str, data: &[u8], old: Option<&FileAttributes>) -> Result<Swap> {
        let dir = parent_remote(target);
        let name = file_name_remote(target).to_string();
        let tag = uuid::Uuid::new_v4().simple().to_string();
        let temp = join_remote(&dir, &format!(".{name}.aether-{}.tmp", &tag[..12]));
        let backup = join_remote(&dir, &format!(".{name}.aether-{}.old", &tag[..12]));

        // The new file gets the old one's permissions the moment it exists, not after the data is in: a secret saved
        // over would otherwise sit readable under the server's default mode for as long as the upload takes.
        let created_with =
            FileAttributes { permissions: old.and_then(|o| o.permissions).map(|m| m & 0o777), ..Default::default() };
        let mut file = match self
            .session
            .open_with_flags_and_attributes(
                temp.clone(),
                OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE,
                created_with,
            )
            .await
        {
            Ok(file) => file,
            // A folder you can't add to: whatever you may do to the file itself has to happen in place.
            Err(SftpError::Status(s)) if old.is_some() && s.status_code == StatusCode::PermissionDenied => {
                return Ok(Swap::Unsuitable);
            }
            Err(e) => return Err(self.note(e)),
        };
        let prepared = async {
            file.write_all(data).await?;
            file.shutdown().await?;
            let Some(old) = old else { return Ok::<bool, SshError>(true) };
            // The owner first: changing it afterwards can drop permission bits such as setuid. Only root may give a
            // file away, so this is a request; whether it took is what decides.
            if let (Some(uid), Some(gid)) = (old.uid, old.gid) {
                let owner = FileAttributes { uid: Some(uid), gid: Some(gid), ..Default::default() };
                let _ = self.session.set_metadata(temp.clone(), owner).await;
                let now = self.guard(self.session.metadata(temp.clone()).await)?;
                if (now.uid, now.gid) != (old.uid, old.gid) {
                    return Ok(false);
                }
            }
            if let Some(mode) = old.permissions {
                let attrs = FileAttributes { permissions: Some(mode & 0o7777), ..Default::default() };
                match self.session.set_metadata(temp.clone(), attrs).await {
                    Ok(()) => {}
                    // A server that won't change permissions (a gateway without setstat) can't give the new file the
                    // old one's, so the old one is written over instead.
                    Err(SftpError::Status(_)) => return Ok(false),
                    Err(e) => return Err(self.note(e)),
                }
            }
            Ok(true)
        }
        .await;
        match prepared {
            Ok(true) => {}
            Ok(false) => {
                let _ = self.session.remove_file(temp).await;
                return Ok(Swap::Unsuitable);
            }
            Err(e) => {
                let _ = self.session.remove_file(temp).await;
                return Err(e);
            }
        }

        if old.is_none() {
            // A new file: there is nothing to put aside.
            if let Err(e) = self.session.rename(temp.clone(), target.to_string()).await {
                let _ = self.session.remove_file(temp).await;
                return Err(self.note(e));
            }
            return Ok(Swap::Done);
        }
        if let Err(e) = self.session.rename(target.to_string(), backup.clone()).await {
            // The server may have done it and lost the answer; look before reporting.
            let refused = matches!(&e, SftpError::Status(_));
            return match self.settle(target, &backup, &temp).await {
                // The server said no and nothing moved: this file can't be swapped (a sticky folder, a mount point, a
                // busy file...). It can still be written, which is what an editor does when it can't rename.
                Settled::Original if refused => Ok(Swap::Unsuitable),
                Settled::Unknown => Err(Self::interrupted(target, &backup, &temp)),
                Settled::Original | Settled::NewIsIn => Err(self.note(e)),
            };
        }
        if let Err(e) = self.session.rename(temp.clone(), target.to_string()).await {
            return match self.settle(target, &backup, &temp).await {
                // The new file was moved in after all: only the answer was lost.
                Settled::NewIsIn => Ok(Swap::Done),
                Settled::Original => Err(self.note(e)),
                Settled::Unknown => Err(Self::interrupted(target, &backup, &temp)),
            };
        }
        if let Err(e) = self.session.remove_file(backup.clone()).await {
            tracing::warn!(error = %e, file = %backup, "the old copy of a saved file couldn't be removed");
        }
        Ok(Swap::Done)
    }

    /// After a rename of a save failed or went unanswered: finds out what is where, puts the original back if it was put
    /// aside, and removes what is left over.
    async fn settle(&self, target: &str, backup: &str, temp: &str) -> Settled {
        let (Ok(temp_there), Ok(target_there)) =
            (self.session.try_exists(temp.to_string()).await, self.session.try_exists(target.to_string()).await)
        else {
            // When the server can't be asked nothing can be put right either.
            return Settled::Unknown;
        };
        if !temp_there && target_there {
            let _ = self.session.remove_file(backup.to_string()).await;
            return Settled::NewIsIn;
        }
        if !target_there {
            let _ = self.session.rename(backup.to_string(), target.to_string()).await;
        }
        let _ = self.session.remove_file(temp.to_string()).await;
        Settled::Original
    }

    /// What to tell when a save was cut off at a moment that leaves the file's place in doubt.
    fn interrupted(target: &str, backup: &str, temp: &str) -> SshError {
        SshError::SaveInterrupted { target: target.to_string(), original: backup.to_string(), new: temp.to_string() }
    }

    // ── single-file transfers ────────────────────────────────────────────────

    /// Downloads one file. Data is written to `local` directly; callers that want
    /// atomicity download to a temporary name and rename. When `resume_from > 0` the
    /// existing local file is appended to from that offset. Returns bytes written.
    pub async fn download_file(
        &self,
        remote: &str,
        local: &Path,
        resume_from: u64,
        progress: ByteProgress<'_>,
        cancel: &CancellationToken,
    ) -> Result<u64> {
        let mut src = self.guard(self.session.open(remote).await)?;
        let mut dst = if resume_from > 0 {
            src.seek(std::io::SeekFrom::Start(resume_from)).await?;
            tokio::fs::OpenOptions::new().append(true).open(local).await?
        } else {
            tokio::fs::File::create(local).await?
        };

        let mut buf = vec![0u8; CHUNK];
        let mut total = 0u64;
        loop {
            if cancel.is_cancelled() {
                dst.flush().await?;
                return Err(SshError::Cancelled);
            }
            let n = src.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            dst.write_all(&buf[..n]).await?;
            total += n as u64;
            progress(n as u64);
        }
        dst.flush().await?;
        dst.sync_all().await.ok();
        Ok(total)
    }

    /// Uploads one file. With `resume_from > 0` the remote file is continued from that offset.
    pub async fn upload_file(
        &self,
        local: &Path,
        remote: &str,
        resume_from: u64,
        progress: ByteProgress<'_>,
        cancel: &CancellationToken,
    ) -> Result<u64> {
        let mut src = tokio::fs::File::open(local).await?;
        let mut dst = if resume_from > 0 {
            src.seek(std::io::SeekFrom::Start(resume_from)).await?;
            let mut f = self.guard(self.session.open_with_flags(remote, OpenFlags::WRITE).await)?;
            f.seek(std::io::SeekFrom::Start(resume_from)).await?;
            f
        } else {
            self.guard(self.session.create(remote).await)?
        };

        let mut buf = vec![0u8; CHUNK];
        let mut total = 0u64;
        loop {
            if cancel.is_cancelled() {
                let _ = dst.shutdown().await;
                return Err(SshError::Cancelled);
            }
            let n = src.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            dst.write_all(&buf[..n]).await?;
            total += n as u64;
            progress(n as u64);
        }
        dst.shutdown().await?;
        Ok(total)
    }

    /// Sets the remote modification time (seconds since the epoch).
    pub async fn set_mtime(&self, path: &str, mtime_secs: u32) -> Result<()> {
        let attrs = FileAttributes { mtime: Some(mtime_secs), atime: Some(mtime_secs), ..Default::default() };
        self.guard(self.session.set_metadata(path, attrs).await)
    }

    pub async fn close(&self) {
        let _ = self.session.close().await;
    }
}

/// Seconds since the epoch for a local file time, if representable.
pub(crate) fn unix_secs(t: std::time::SystemTime) -> Option<u32> {
    t.duration_since(UNIX_EPOCH).ok().and_then(|d| u32::try_from(d.as_secs()).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_path_helpers() {
        assert_eq!(join_remote("/home/me", "file.txt"), "/home/me/file.txt");
        assert_eq!(join_remote("/", "etc"), "/etc");
        assert_eq!(join_remote("/a/", "b"), "/a/b");
        assert_eq!(join_remote(".", "x"), "x");
        assert_eq!(join_remote("/a", "/abs"), "/abs");
        assert_eq!(parent_remote("/a/b/c"), "/a/b");
        assert_eq!(parent_remote("/a"), "/");
        assert_eq!(parent_remote("/"), "/");
        assert_eq!(parent_remote("rel/x"), "rel");
        assert_eq!(parent_remote("x"), ".");
        assert_eq!(file_name_remote("/a/b/c.txt"), "c.txt");
        assert_eq!(file_name_remote("/a/b/"), "b");
    }

    #[test]
    fn attributes_map_to_entries() {
        let mut attrs = FileAttributes {
            size: Some(1234),
            permissions: Some(0o100_644),
            mtime: Some(1_700_000_000),
            user: Some("me".into()),
            gid: Some(100),
            ..Default::default()
        };
        attrs.set_regular(true);
        let e = entry("/home/me", "notes.txt".into(), &attrs);
        assert_eq!(e.path, "/home/me/notes.txt");
        assert_eq!(e.kind, EntryKind::File);
        assert_eq!(e.mode, Some(0o644));
        assert_eq!(e.modified, Some(1_700_000_000_000));
        assert_eq!((e.owner.as_deref(), e.group.as_deref()), (Some("me"), Some("100")));
    }
}
