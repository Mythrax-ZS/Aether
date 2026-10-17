//! File transfer jobs: planning, a queue with bounded parallelism, pause/resume/cancel,
//! conflict handling and throttled progress events.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use aether_core::error::{ErrorCode, ErrorInfo, UserFacing};
use aether_core::util::now_ms;
use aether_core::{ConnectionId, TransferId};
use dashmap::DashMap;
use futures::StreamExt;
use parking_lot::Mutex;
use serde::Serialize;
use tokio::runtime::Handle;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};
use ts_rs::TS;

use crate::error::{Result, SshError};
use crate::sftp::{SftpClient, file_name_remote, join_remote, unix_secs};

const PART_SUFFIX: &str = ".aetherpart";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum TransferDirection {
    Upload,
    Download,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum TransferState {
    Queued,
    Running,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

impl TransferState {
    pub fn is_finished(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

/// What to do when the destination already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum ConflictPolicy {
    Overwrite,
    Skip,
    /// Keep both: the new file becomes `name (1).ext`.
    KeepBoth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ItemKind {
    Dir,
    File,
}

#[derive(Debug, Clone)]
pub struct PlanItem {
    pub local: PathBuf,
    pub remote: String,
    kind: ItemKind,
    pub size: u64,
    /// Source modification time (seconds since the epoch).
    pub mtime: Option<u32>,
}

impl PlanItem {
    pub fn is_dir(&self) -> bool {
        self.kind == ItemKind::Dir
    }
}

/// What a transfer will do, computed before anything moves so the UI can ask about conflicts.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub items: Vec<PlanItem>,
    pub total_bytes: u64,
    pub file_count: u32,
    /// Destination paths that already exist (files only; existing folders merge).
    pub conflicts: Vec<String>,
    /// Things that were skipped while planning (special files, unreadable folders).
    pub warnings: Vec<String>,
    /// Names of the top-level items the user selected (used to title the job).
    pub roots: Vec<String>,
    /// For downloads: the folder every local path must stay inside. Checked again before anything is written.
    pub local_root: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct TransferOptions {
    /// Files transferred at the same time within one job.
    pub concurrency: usize,
    pub preserve_times: bool,
}

impl Default for TransferOptions {
    fn default() -> Self {
        Self { concurrency: 3, preserve_times: true }
    }
}

fn mtime_of(md: &std::fs::Metadata) -> Option<u32> {
    md.modified().ok().and_then(unix_secs)
}

// ── names from the server ────────────────────────────────────────────────────

/// Longest name Aether will create, in UTF-16 units (Windows allows 255; room is left for ` (1)` and `.aetherpart`).
const MAX_NAME_UNITS: usize = 200;
/// A plan that grows past this stops there, with a warning: nothing sane downloads this many items in one go.
const MAX_PLAN_ITEMS: usize = 500_000;
const MAX_PLAN_DEPTH: usize = 64;
/// How long a single remote folder may take to list while planning.
const LIST_TIMEOUT: Duration = Duration::from_secs(90);

/// Turns a name the **server** supplied into one that is safe to create on this PC.
///
/// A server is not trusted. Its listing can contain `..\..\Startup\x.bat` (traversal), `C:\x` or `\\host\share\x`
/// (an absolute path, or a UNC path that makes Windows authenticate to someone else's machine), `file:stream` (an NTFS
/// alternate data stream), `NUL` and friends (devices), or names that Windows silently rewrites (trailing dots and
/// spaces). So: separators and every character Windows forbids become `_`, trailing dots/spaces become `_`, reserved
/// device names get a leading `_`, and the length is capped. The result is never empty, `.` or `..`, and is always a
/// single path component.
pub fn safe_local_name(name: &str) -> String {
    let mut s: String =
        name.chars()
            .map(|c| {
                if c.is_control() || matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                    '_'
                } else {
                    c
                }
            })
            .collect();

    // Windows drops trailing dots and spaces, so "a." and "a" would be the same file (and ".." the parent folder).
    let kept = s.trim_end_matches(['.', ' ']).len();
    let dropped = s.len() - kept;
    s.truncate(kept);
    s.push_str(&"_".repeat(dropped));
    if s.is_empty() {
        s.push('_');
    }

    // Reserved device names, with or without an extension ("aux.c" is a device on Windows).
    let stem = s.split('.').next().unwrap_or("");
    let upper = stem.trim_end().to_ascii_uppercase();
    let reserved = matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.as_bytes()[3].is_ascii_digit()
            && upper.as_bytes()[3] != b'0');
    if reserved {
        s.insert(0, '_');
    }

    // Cap the length without splitting a character.
    let mut units = 0;
    let mut end = s.len();
    for (i, c) in s.char_indices() {
        units += c.len_utf16();
        if units > MAX_NAME_UNITS {
            end = i;
            break;
        }
    }
    s.truncate(end);
    s
}

/// Is `path` strictly inside `root`, reached only through ordinary names (no `..`, no drive or UNC prefix)?
fn is_inside(root: &Path, path: &Path) -> bool {
    path.strip_prefix(root).is_ok_and(|rel| {
        rel.components().next().is_some() && rel.components().all(|c| matches!(c, std::path::Component::Normal(_)))
    })
}

/// Tracks the local paths a download plan has claimed, case-insensitively (NTFS is), so two server names that
/// collapse to the same file here ("Readme", "README", or two names that sanitise alike) never share one.
#[derive(Default)]
struct Claims(HashSet<String>);

impl Claims {
    /// `dir/name`, or `dir/name (n)` when that is already taken within this plan.
    fn claim(&mut self, dir: &Path, name: &str) -> PathBuf {
        let name = safe_local_name(name);
        let first = dir.join(&name);
        if self.0.insert(first.to_string_lossy().to_lowercase()) {
            return first;
        }
        let (stem, ext) = match name.rfind('.') {
            Some(i) if i > 0 => (&name[..i], &name[i..]),
            _ => (name.as_str(), ""),
        };
        for n in 1.. {
            let candidate = dir.join(format!("{stem} ({n}){ext}"));
            if self.0.insert(candidate.to_string_lossy().to_lowercase()) {
                return candidate;
            }
        }
        unreachable!("the counter is unbounded")
    }
}

/// Walks local files/folders to be uploaded into `remote_dir`.
pub async fn plan_upload(client: &SftpClient, local_paths: &[PathBuf], remote_dir: &str) -> Result<Plan> {
    let paths = local_paths.to_vec();
    let remote_dir = remote_dir.to_string();
    let mut plan = tokio::task::spawn_blocking(move || -> Result<Plan> {
        let mut plan = Plan::default();
        for root in &paths {
            let name = root
                .file_name()
                .ok_or_else(|| SshError::Invalid(format!("{} has no name", root.display())))?
                .to_string_lossy()
                .into_owned();
            plan.roots.push(name.clone());
            let mut stack = vec![(root.clone(), join_remote(&remote_dir, &name))];
            while let Some((local, remote)) = stack.pop() {
                let md = match std::fs::metadata(&local) {
                    Ok(m) => m,
                    Err(e) => {
                        plan.warnings.push(format!("{}: {e}", local.display()));
                        continue;
                    }
                };
                if md.is_dir() {
                    plan.items.push(PlanItem {
                        local: local.clone(),
                        remote: remote.clone(),
                        kind: ItemKind::Dir,
                        size: 0,
                        mtime: mtime_of(&md),
                    });
                    match std::fs::read_dir(&local) {
                        Ok(rd) => {
                            let mut children: Vec<_> = rd.filter_map(|e| e.ok()).collect();
                            children.sort_by_key(|e| e.file_name());
                            for child in children.into_iter().rev() {
                                // Don't follow directory symlinks/junctions: avoids loops.
                                if child.file_type().is_ok_and(|t| t.is_symlink()) && child.path().is_dir() {
                                    plan.warnings.push(format!("{}: folder link skipped", child.path().display()));
                                    continue;
                                }
                                let child_remote = join_remote(&remote, &child.file_name().to_string_lossy());
                                stack.push((child.path(), child_remote));
                            }
                        }
                        Err(e) => plan.warnings.push(format!("{}: {e}", local.display())),
                    }
                } else if md.is_file() {
                    plan.total_bytes += md.len();
                    plan.file_count += 1;
                    plan.items.push(PlanItem {
                        local,
                        remote,
                        kind: ItemKind::File,
                        size: md.len(),
                        mtime: mtime_of(&md),
                    });
                } else {
                    plan.warnings.push(format!("{}: not a regular file", local.display()));
                }
            }
        }
        Ok(plan)
    })
    .await
    .map_err(|e| SshError::Internal(e.to_string()))??;

    // Which destinations already exist? Check in parallel batches.
    let files: Vec<String> = plan.items.iter().filter(|i| !i.is_dir()).map(|i| i.remote.clone()).collect();
    for batch in files.chunks(24) {
        let checks = futures::future::join_all(
            batch.iter().map(|p| async move { (p, client.exists(p).await.unwrap_or(false)) }),
        )
        .await;
        plan.conflicts.extend(checks.into_iter().filter(|(_, e)| *e).map(|(p, _)| p.clone()));
    }
    Ok(plan)
}

/// Walks remote files/folders to be downloaded into `local_dir`.
///
/// Every name comes from the server and is made safe first (see [`safe_local_name`]); planning is bounded in depth,
/// size and time so a hostile or endless tree can't hang or exhaust the app.
pub async fn plan_download(client: &SftpClient, remote_paths: &[String], local_dir: &Path) -> Result<Plan> {
    let mut plan = Plan { local_root: Some(local_dir.to_path_buf()), ..Plan::default() };
    let mut claims = Claims::default();
    'roots: for root in remote_paths {
        let name = file_name_remote(root).to_string();
        plan.roots.push(name.clone());
        let mut stack = vec![(root.clone(), claims.claim(local_dir, &name), 0usize)];
        while let Some((remote, local, depth)) = stack.pop() {
            if plan.items.len() >= MAX_PLAN_ITEMS {
                plan.warnings.push(format!("Stopped planning after {MAX_PLAN_ITEMS} items; select fewer folders."));
                break 'roots;
            }
            let st = match client.stat(&remote).await {
                Ok(s) => s,
                Err(e) => {
                    plan.warnings.push(format!("{remote}: {e}"));
                    continue;
                }
            };
            let mtime = st.modified.map(|ms| (ms / 1000) as u32);
            match st.kind {
                crate::sftp::EntryKind::Dir => {
                    plan.items.push(PlanItem {
                        local: local.clone(),
                        remote: remote.clone(),
                        kind: ItemKind::Dir,
                        size: 0,
                        mtime,
                    });
                    if depth >= MAX_PLAN_DEPTH {
                        plan.warnings
                            .push(format!("{remote}: folders nested more than {MAX_PLAN_DEPTH} deep were skipped"));
                        continue;
                    }
                    match tokio::time::timeout(LIST_TIMEOUT, client.list_up_to(&remote, MAX_PLAN_ITEMS)).await {
                        Ok(Ok(mut children)) => {
                            children.sort_by(|a, b| a.name.cmp(&b.name));
                            let mut next = Vec::with_capacity(children.len());
                            for c in children {
                                if c.is_symlink && c.kind == crate::sftp::EntryKind::Dir {
                                    plan.warnings.push(format!("{}: folder link skipped", c.path));
                                    continue;
                                }
                                next.push((c.path.clone(), claims.claim(&local, &c.name), depth + 1));
                            }
                            // Popped from the back, so push in reverse to visit in name order.
                            stack.extend(next.into_iter().rev());
                        }
                        Ok(Err(e)) => plan.warnings.push(format!("{remote}: {e}")),
                        Err(_) => plan.warnings.push(format!("{remote}: took too long to list and was skipped")),
                    }
                }
                crate::sftp::EntryKind::File | crate::sftp::EntryKind::Symlink => {
                    plan.total_bytes += st.size;
                    plan.file_count += 1;
                    plan.items.push(PlanItem { local, remote, kind: ItemKind::File, size: st.size, mtime });
                }
                crate::sftp::EntryKind::Other => plan.warnings.push(format!("{remote}: not a regular file")),
            }
        }
    }
    plan.conflicts =
        plan.items.iter().filter(|i| !i.is_dir() && i.local.exists()).map(|i| i.local.display().to_string()).collect();
    Ok(plan)
}

// ── job state ────────────────────────────────────────────────────────────────

/// A snapshot of a job for the UI.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TransferInfo {
    pub id: TransferId,
    pub direction: TransferDirection,
    /// "report.pdf" or "12 items".
    pub title: String,
    pub state: TransferState,
    #[ts(type = "number")]
    pub bytes_done: u64,
    #[ts(type = "number")]
    pub bytes_total: u64,
    pub files_done: u32,
    pub files_total: u32,
    pub skipped: u32,
    pub current: Option<String>,
    #[ts(type = "number")]
    pub speed_bps: u64,
    pub error: Option<ErrorInfo>,
    #[ts(type = "number")]
    pub started_at: i64,
    #[ts(type = "number | null")]
    pub finished_at: Option<i64>,
    /// Folder to reveal when the user clicks "show" (local destination for downloads).
    pub reveal: Option<String>,
}

pub trait TransferEvents: Send + Sync + 'static {
    fn changed(&self, info: &TransferInfo);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Intent {
    None,
    Pause,
    Cancel,
}

struct Shared {
    state: Mutex<TransferState>,
    bytes_done: AtomicU64,
    files_done: AtomicU32,
    skipped: AtomicU32,
    speed: AtomicU64,
    current: Mutex<Option<String>>,
    errors: Mutex<Vec<(String, ErrorInfo)>>,
    done_items: Mutex<HashSet<usize>>,
    /// Items whose data this job has started writing. Only those may be continued after a pause: a partial file
    /// that this job did not write is somebody else's data and must not be appended to.
    started: Mutex<HashSet<usize>>,
    /// The `.aetherpart` file each running download is writing, so cancelling removes exactly those.
    parts: Mutex<HashMap<usize, PathBuf>>,
    finished_at: Mutex<Option<i64>>,
}

struct Control {
    token: CancellationToken,
    intent: Intent,
}

struct Job {
    id: TransferId,
    direction: TransferDirection,
    title: String,
    /// The connection the files travel over — and, so that a transfer stopped by a dropped connection can be resumed
    /// over the new one, the SFTP channel to use, replaced on resume.
    connection: ConnectionId,
    client: Mutex<SftpClient>,
    plan: Plan,
    policy: ConflictPolicy,
    options: TransferOptions,
    started_at: i64,
    reveal: Option<String>,
    shared: Shared,
    control: Mutex<Control>,
    /// Where this job's tasks run, so pause/resume can be called from any thread.
    runtime: Handle,
}

impl Job {
    fn client(&self) -> SftpClient {
        self.client.lock().clone()
    }

    fn info(&self) -> TransferInfo {
        let errors = self.shared.errors.lock();
        let error = match errors.len() {
            0 => None,
            1 => Some(errors[0].1.clone()),
            n => {
                let list: Vec<String> = errors.iter().take(5).map(|(p, e)| format!("{p}: {}", e.message)).collect();
                Some(
                    ErrorInfo::new(ErrorCode::Sftp, "Some items weren't transferred", format!("{n} items failed."))
                        .detail(list.join("\n")),
                )
            }
        };
        TransferInfo {
            id: self.id,
            direction: self.direction,
            title: self.title.clone(),
            state: *self.shared.state.lock(),
            bytes_done: self.shared.bytes_done.load(Ordering::Relaxed).min(self.plan.total_bytes.max(1) * 2),
            bytes_total: self.plan.total_bytes,
            files_done: self.shared.files_done.load(Ordering::Relaxed),
            files_total: self.plan.file_count,
            skipped: self.shared.skipped.load(Ordering::Relaxed),
            current: self.shared.current.lock().clone(),
            speed_bps: self.shared.speed.load(Ordering::Relaxed),
            error,
            started_at: self.started_at,
            finished_at: *self.shared.finished_at.lock(),
            reveal: self.reveal.clone(),
        }
    }
}

fn unique_local(path: &Path) -> PathBuf {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (1..10_000)
        .map(|n| path.with_file_name(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .unwrap_or_else(|| path.to_path_buf())
}

async fn unique_remote(client: &SftpClient, remote: &str) -> Result<String> {
    let name = file_name_remote(remote);
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let dir = crate::sftp::parent_remote(remote);
    for n in 1..10_000 {
        let candidate = join_remote(&dir, &format!("{stem} ({n}){ext}"));
        if !client.exists(&candidate).await? {
            return Ok(candidate);
        }
    }
    Ok(remote.to_string())
}

/// Downloads never write outside the folder the user chose, whatever the plan says (defence in depth: names are
/// already sanitised when the plan is made).
fn ensure_local_inside(job: &Job, path: &Path) -> Result<()> {
    match &job.plan.local_root {
        Some(root) if is_inside(root, path) => Ok(()),
        Some(_) => Err(SshError::Invalid(format!("{} is outside the download folder", path.display()))),
        None => Ok(()),
    }
}

fn part_path(target: &Path) -> PathBuf {
    let mut s = target.as_os_str().to_os_string();
    s.push(PART_SUFFIX);
    PathBuf::from(s)
}

// ── manager ──────────────────────────────────────────────────────────────────

/// Everything a transfer needs to begin.
pub struct TransferRequest {
    /// The connection the files travel over (so a stopped transfer can be given a new channel on it).
    pub connection: ConnectionId,
    /// The SFTP channel to use first.
    pub client: SftpClient,
    pub direction: TransferDirection,
    pub plan: Plan,
    pub policy: ConflictPolicy,
    pub options: TransferOptions,
    /// A local folder the UI can open afterwards.
    pub reveal: Option<String>,
}

pub struct TransferManager {
    jobs: DashMap<TransferId, Arc<Job>>,
    events: Arc<dyn TransferEvents>,
    /// How many jobs run at once; the rest wait in the queue.
    slots: Arc<Semaphore>,
}

impl TransferManager {
    pub fn new(events: Arc<dyn TransferEvents>, parallel_jobs: usize) -> Self {
        Self { jobs: DashMap::new(), events, slots: Arc::new(Semaphore::new(parallel_jobs.max(1))) }
    }

    pub fn list(&self) -> Vec<TransferInfo> {
        let mut v: Vec<TransferInfo> = self.jobs.iter().map(|j| j.info()).collect();
        v.sort_by_key(|i| i.started_at);
        v
    }

    /// Queues a planned transfer.
    pub fn start(&self, request: TransferRequest) -> TransferId {
        let TransferRequest { connection, client, direction, plan, policy, options, reveal } = request;
        let title = match plan.roots.as_slice() {
            [only] => only.clone(),
            [] => "Transfer".to_string(),
            many => format!("{} items", many.len()),
        };
        let id = TransferId::new();
        let job = Arc::new(Job {
            id,
            direction,
            title,
            connection,
            client: Mutex::new(client),
            plan,
            policy,
            options,
            started_at: now_ms(),
            reveal,
            shared: Shared {
                state: Mutex::new(TransferState::Queued),
                bytes_done: AtomicU64::new(0),
                files_done: AtomicU32::new(0),
                skipped: AtomicU32::new(0),
                speed: AtomicU64::new(0),
                current: Mutex::new(None),
                errors: Mutex::new(Vec::new()),
                done_items: Mutex::new(HashSet::new()),
                started: Mutex::new(HashSet::new()),
                parts: Mutex::new(HashMap::new()),
                finished_at: Mutex::new(None),
            },
            control: Mutex::new(Control { token: CancellationToken::new(), intent: Intent::None }),
            runtime: Handle::current(),
        });
        self.jobs.insert(id, job.clone());
        self.events.changed(&job.info());
        self.spawn_run(job, false);
        id
    }

    fn spawn_run(&self, job: Arc<Job>, resume: bool) {
        let events = self.events.clone();
        let slots = self.slots.clone();
        let token = {
            let mut c = job.control.lock();
            c.token = CancellationToken::new();
            c.intent = Intent::None;
            c.token.clone()
        };
        *job.shared.state.lock() = TransferState::Queued;
        *job.shared.finished_at.lock() = None;
        let runtime = job.runtime.clone();
        runtime.spawn(async move {
            // Wait for a free slot, unless cancelled/paused while queued.
            let permit = tokio::select! {
                p = slots.acquire_owned() => p.ok(),
                _ = token.cancelled() => None,
            };
            let Some(_permit) = permit else {
                finish(&job, &events, Err(SshError::Cancelled));
                return;
            };
            *job.shared.state.lock() = TransferState::Running;
            events.changed(&job.info());

            let ticker = {
                let job = job.clone();
                let events = events.clone();
                tokio::spawn(async move {
                    let mut last = job.shared.bytes_done.load(Ordering::Relaxed);
                    let mut speed = 0f64;
                    loop {
                        tokio::time::sleep(Duration::from_millis(250)).await;
                        let now = job.shared.bytes_done.load(Ordering::Relaxed);
                        let instant = (now.saturating_sub(last)) as f64 / 0.25;
                        speed = if speed == 0.0 { instant } else { speed * 0.7 + instant * 0.3 };
                        last = now;
                        job.shared.speed.store(speed as u64, Ordering::Relaxed);
                        events.changed(&job.info());
                    }
                })
            };
            let result = execute(&job, &token, resume).await;
            ticker.abort();
            finish(&job, &events, result);
        });
    }

    pub fn pause(&self, id: TransferId) {
        if let Some(job) = self.jobs.get(&id) {
            let mut c = job.control.lock();
            if matches!(*job.shared.state.lock(), TransferState::Running | TransferState::Queued) {
                c.intent = Intent::Pause;
                c.token.cancel();
            }
        }
    }

    /// The connection a transfer runs over, so a resume can be given that connection's current SFTP channel.
    pub fn connection_of(&self, id: TransferId) -> Option<ConnectionId> {
        self.jobs.get(&id).map(|j| j.connection)
    }

    /// Continues a paused or failed transfer. After the connection dropped and came back, the channel the job was
    /// started on is gone: pass the connection's current one as `fresh` and the job carries on over that.
    pub fn resume(&self, id: TransferId, fresh: Option<SftpClient>) {
        if let Some(job) = self.jobs.get(&id).map(|j| j.clone())
            && (*job.shared.state.lock() == TransferState::Paused || *job.shared.state.lock() == TransferState::Failed)
        {
            if let Some(client) = fresh {
                *job.client.lock() = client;
            }
            job.shared.errors.lock().clear();
            self.spawn_run(job, true);
        }
    }

    pub fn cancel(&self, id: TransferId) {
        if let Some(job) = self.jobs.get(&id).map(|j| j.clone()) {
            let state = *job.shared.state.lock();
            if state.is_finished() {
                return;
            }
            {
                let mut c = job.control.lock();
                c.intent = Intent::Cancel;
                c.token.cancel();
            }
            if state == TransferState::Paused {
                // Nothing is running to notice the cancellation.
                finish(&job, &self.events, Err(SshError::Cancelled));
            }
        }
    }

    /// Removes a finished job from the list.
    pub fn dismiss(&self, id: TransferId) {
        if self.jobs.get(&id).is_some_and(|j| j.shared.state.lock().is_finished()) {
            self.jobs.remove(&id);
        }
    }

    pub fn clear_finished(&self) {
        self.jobs.retain(|_, j| !j.shared.state.lock().is_finished());
    }
}

fn finish(job: &Arc<Job>, events: &Arc<dyn TransferEvents>, result: Result<()>) {
    let intent = job.control.lock().intent;
    let has_errors = !job.shared.errors.lock().is_empty();
    let state = match (&result, intent) {
        (_, Intent::Cancel) => TransferState::Cancelled,
        (Err(SshError::Cancelled), Intent::Pause) => TransferState::Paused,
        (Err(SshError::Cancelled), Intent::None) => TransferState::Cancelled,
        (Err(e), _) => {
            job.shared.errors.lock().push((job.title.clone(), e.info()));
            TransferState::Failed
        }
        (Ok(()), _) if has_errors => TransferState::Failed,
        (Ok(()), _) => TransferState::Completed,
    };
    if state == TransferState::Cancelled {
        cleanup_partials(job);
    }
    *job.shared.current.lock() = None;
    job.shared.speed.store(0, Ordering::Relaxed);
    if state.is_finished() {
        *job.shared.finished_at.lock() = Some(now_ms());
    }
    *job.shared.state.lock() = state;
    events.changed(&job.info());
}

/// Cancelled downloads leave no half-written files behind.
fn cleanup_partials(job: &Job) {
    for (_, part) in job.shared.parts.lock().drain() {
        let _ = std::fs::remove_file(part);
    }
}

async fn execute(job: &Arc<Job>, token: &CancellationToken, resume: bool) -> Result<()> {
    // Progress is rebuilt from what is finished: after a pause the partial progress of the items that were running is
    // counted again as they continue, so carrying the old total over would count those bytes twice.
    let finished: u64 = job.shared.done_items.lock().iter().map(|&i| job.plan.items[i].size).sum();
    job.shared.bytes_done.store(finished, Ordering::Relaxed);

    // Folders first, in order, so their children have somewhere to land.
    for (i, item) in job.plan.items.iter().enumerate() {
        if !item.is_dir() || job.shared.done_items.lock().contains(&i) {
            continue;
        }
        if token.is_cancelled() {
            return Err(SshError::Cancelled);
        }
        match job.direction {
            TransferDirection::Upload => job.client().mkdir_all(&item.remote).await?,
            TransferDirection::Download => {
                ensure_local_inside(job, &item.local)?;
                tokio::fs::create_dir_all(&item.local).await?
            }
        }
        job.shared.done_items.lock().insert(i);
    }

    let pending: Vec<usize> = job
        .plan
        .items
        .iter()
        .enumerate()
        .filter(|(i, it)| !it.is_dir() && !job.shared.done_items.lock().contains(i))
        .map(|(i, _)| i)
        .collect();

    let concurrency = job.options.concurrency.max(1);
    let mut stream = futures::stream::iter(pending)
        .map(|i| {
            let job = job.clone();
            let token = token.clone();
            async move { (i, transfer_one(&job, i, &token, resume).await) }
        })
        .buffer_unordered(concurrency);

    while let Some((i, outcome)) = stream.next().await {
        match outcome {
            Ok(()) => {
                job.shared.done_items.lock().insert(i);
                job.shared.files_done.fetch_add(1, Ordering::Relaxed);
            }
            Err(SshError::Cancelled) => return Err(SshError::Cancelled),
            Err(e) => {
                let name = job.plan.items[i].remote.clone();
                warn!(item = %name, error = %e, "transfer failed");
                // Lost connections affect every remaining file; stop instead of failing each one. The job records this
                // one failure itself (see `finish`): adding it here too would report "2 items failed" for one drop.
                if e.is_transient() {
                    return Err(e);
                }
                job.shared.errors.lock().push((name, e.info()));
            }
        }
    }
    Ok(())
}

async fn transfer_one(job: &Arc<Job>, index: usize, token: &CancellationToken, resume: bool) -> Result<()> {
    let item = &job.plan.items[index];
    let shared = &job.shared;
    *shared.current.lock() = Some(match job.direction {
        TransferDirection::Upload => {
            item.local.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        }
        TransferDirection::Download => file_name_remote(&item.remote).to_string(),
    });
    let add = |n: u64| {
        shared.bytes_done.fetch_add(n, Ordering::Relaxed);
    };

    match job.direction {
        TransferDirection::Download => {
            ensure_local_inside(job, &item.local)?;
            let exists = tokio::fs::try_exists(&item.local).await?;
            let target = match (exists, job.policy) {
                (true, ConflictPolicy::Skip) => {
                    shared.skipped.fetch_add(1, Ordering::Relaxed);
                    add(item.size);
                    return Ok(());
                }
                (true, ConflictPolicy::KeepBoth) => unique_local(&item.local),
                _ => item.local.clone(),
            };
            ensure_local_inside(job, &target)?;
            if let Some(parent) = target.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            let part = part_path(&target);
            // Continue a partial file only if this job was the one writing it.
            let resumable = resume && shared.started.lock().contains(&index);
            let mut offset = 0;
            if resumable
                && let Ok(md) = tokio::fs::metadata(&part).await
                && md.len() < item.size
            {
                offset = md.len();
                add(offset);
            }
            shared.started.lock().insert(index);
            shared.parts.lock().insert(index, part.clone());
            job.client().download_file(&item.remote, &part, offset, &add, token).await?;
            tokio::fs::rename(&part, &target).await?;
            shared.parts.lock().remove(&index);
            if job.options.preserve_times
                && let Some(secs) = item.mtime
            {
                let when = std::time::UNIX_EPOCH + Duration::from_secs(u64::from(secs));
                if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&target) {
                    let _ = f.set_modified(when);
                }
            }
        }
        TransferDirection::Upload => {
            let exists = job.client().exists(&item.remote).await?;
            let target = match (exists, job.policy) {
                (true, ConflictPolicy::Skip) => {
                    shared.skipped.fetch_add(1, Ordering::Relaxed);
                    add(item.size);
                    return Ok(());
                }
                (true, ConflictPolicy::KeepBoth) => unique_remote(&job.client(), &item.remote).await?,
                _ => item.remote.clone(),
            };
            // Continue a partial upload only if this job was the one writing it; otherwise "resume" would graft the
            // new file's tail onto whatever happened to be on the server.
            let resumable = resume && shared.started.lock().contains(&index);
            let mut offset = 0;
            if resumable
                && let Ok(st) = job.client().stat(&target).await
                && st.size < item.size
            {
                offset = st.size;
                add(offset);
            }
            shared.started.lock().insert(index);
            job.client().upload_file(&item.local, &target, offset, &add, token).await?;
            if job.options.preserve_times
                && let Some(secs) = item.mtime
            {
                let _ = job.client().set_mtime(&target, secs).await;
            }
        }
    }
    debug!(item = %item.remote, "transferred");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_names_become_single_safe_components() {
        // Traversal and absolute paths lose their separators; the rest of the text is kept so the file stays recognisable.
        assert_eq!(safe_local_name(r"..\..\Startup\x.bat"), ".._.._Startup_x.bat");
        assert_eq!(safe_local_name("../x"), ".._x");
        assert_eq!(safe_local_name(r"C:\Users\Public\x"), "C__Users_Public_x");
        assert_eq!(safe_local_name(r"\\evil\share\x"), "__evil_share_x");
        // NTFS alternate data streams and characters Windows forbids.
        assert_eq!(safe_local_name("file.txt:stream"), "file.txt_stream");
        assert_eq!(safe_local_name("a<b>c|d?e*f\"g"), "a_b_c_d_e_f_g");
        assert_eq!(safe_local_name("tab\there\u{7}"), "tab_here_");
        // Windows drops trailing dots and spaces, so they can't be allowed to alias another file or the parent folder.
        assert_eq!(safe_local_name("name. "), "name__");
        assert_eq!(safe_local_name(".."), "__");
        assert_eq!(safe_local_name("."), "_");
        assert_eq!(safe_local_name(""), "_");
        // Dot-files are fine.
        assert_eq!(safe_local_name(".bashrc"), ".bashrc");
        // Device names are reserved, with or without an extension, in any case.
        for device in ["CON", "nul", "Aux.c", "com1", "LPT9.txt", "prn", "CONIN$"] {
            assert!(safe_local_name(device).starts_with('_'), "{device}");
        }
        // ...but near-misses are ordinary names.
        for fine in ["COM0", "COM10", "console", "auxiliary.txt", "nullable", "lpt"] {
            assert_eq!(safe_local_name(fine), fine, "{fine}");
        }
        // Ordinary and non-Latin names pass unchanged.
        for fine in ["report (final).pdf", "データ.csv", "naïve café.md", "a b.txt"] {
            assert_eq!(safe_local_name(fine), fine);
        }
    }

    #[test]
    fn long_names_are_capped_without_splitting_characters() {
        let long = "é".repeat(500);
        let safe = safe_local_name(&long);
        assert!(safe.encode_utf16().count() <= MAX_NAME_UNITS);
        assert!(safe.chars().all(|c| c == 'é'));
        let astral = "😀".repeat(300); // two UTF-16 units each
        assert!(safe_local_name(&astral).encode_utf16().count() <= MAX_NAME_UNITS);
    }

    #[test]
    fn names_that_collide_here_get_distinct_files() {
        let dir = Path::new("out");
        let mut claims = Claims::default();
        let a = claims.claim(dir, "Readme");
        let b = claims.claim(dir, "README");
        let c = claims.claim(dir, "readme");
        let d = claims.claim(dir, "notes.txt");
        let e = claims.claim(dir, "NOTES.TXT");
        let all = [&a, &b, &c, &d, &e];
        let lower: HashSet<String> = all.iter().map(|p| p.to_string_lossy().to_lowercase()).collect();
        assert_eq!(lower.len(), all.len(), "{all:?}");
        assert_eq!(a, dir.join("Readme"));
        assert_eq!(e, dir.join("NOTES (1).TXT"), "the number goes before the extension");
        // The same name in another folder is not a collision.
        assert_eq!(claims.claim(&dir.join("sub"), "Readme"), dir.join("sub").join("Readme"));
    }

    #[test]
    fn containment_check_rejects_anything_but_plain_descendants() {
        let root = Path::new("C:/dl");
        assert!(is_inside(root, Path::new("C:/dl/a/b.txt")));
        assert!(!is_inside(root, Path::new("C:/dl/../x")));
        assert!(!is_inside(root, Path::new("C:/other/x")));
        assert!(!is_inside(root, root), "the folder itself is not a file inside it");
    }
}
