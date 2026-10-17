//! Remote file browsing and the transfer queue.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use aether_core::error::UserFacing;
use aether_core::{ConnectionId, ErrorCode, ErrorInfo, TransferId};
use aether_ssh::{
    ConflictPolicy, DiskUsage, EntryKind, FileVersion, RemoteEntry, RemoteText, SftpClient, TransferDirection,
    TransferInfo, TransferOptions, TransferRequest, decode_text, encode_text, plan_download, plan_upload,
};

use super::St;
use crate::dto::{PlanSummary, RemoteListing, WriteTextRequest};
use crate::state::{PendingPlan, Res, not_found};

async fn client(state: &St<'_>, connection_id: ConnectionId) -> Res<SftpClient> {
    state.session(connection_id)?.sftp().await.map_err(|e| e.info())
}

/// Resolves `~`, `.` and relative paths against the server's home directory.
async fn resolve(client: &SftpClient, path: &str) -> String {
    let p = path.trim();
    if p.is_empty() || p == "~" || p == "." {
        return client.home().to_string();
    }
    if let Some(rest) = p.strip_prefix("~/") {
        return aether_ssh::join_remote(client.home(), rest);
    }
    client.canonicalize(p).await.unwrap_or_else(|_| p.to_string())
}

#[tauri::command]
pub async fn sftp_list(state: St<'_>, connection_id: ConnectionId, path: String) -> Res<RemoteListing> {
    let client = client(&state, connection_id).await?;
    let path = resolve(&client, &path).await;
    let entries = client.list(&path).await.map_err(|e| e.info())?;
    Ok(RemoteListing { path, entries })
}

#[tauri::command]
pub async fn sftp_home(state: St<'_>, connection_id: ConnectionId) -> Res<String> {
    Ok(client(&state, connection_id).await?.home().to_string())
}

#[tauri::command]
pub async fn sftp_mkdir(state: St<'_>, connection_id: ConnectionId, path: String) -> Res<()> {
    client(&state, connection_id).await?.mkdir(&path).await.map_err(|e| e.info())
}

#[tauri::command]
pub async fn sftp_rename(state: St<'_>, connection_id: ConnectionId, from: String, to: String) -> Res<()> {
    client(&state, connection_id).await?.rename(&from, &to).await.map_err(|e| e.info())
}

#[tauri::command]
pub async fn sftp_delete(state: St<'_>, connection_id: ConnectionId, paths: Vec<String>) -> Res<()> {
    let client = client(&state, connection_id).await?;
    for p in paths {
        client.remove_recursive(&p).await.map_err(|e| e.info())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn sftp_chmod(
    state: St<'_>,
    connection_id: ConnectionId,
    path: String,
    mode: u32,
    recursive: bool,
) -> Res<()> {
    let client = client(&state, connection_id).await?;
    if recursive { client.chmod_recursive(&path, mode).await } else { client.chmod(&path, mode).await }
        .map_err(|e| e.info())
}

#[tauri::command]
pub async fn sftp_disk_usage(state: St<'_>, connection_id: ConnectionId, path: String) -> Res<Option<DiskUsage>> {
    client(&state, connection_id).await?.disk_usage(&path).await.map_err(|e| e.info())
}

fn version_of(entry: &RemoteEntry) -> FileVersion {
    FileVersion { size: entry.size, modified: entry.modified }
}

/// Reads the start of a text file for the quick editor/preview (1 MiB at most). What can't be carried through a text
/// box and back — binary data, a legacy code page, UTF-16 — is refused rather than quietly altered.
#[tauri::command]
pub async fn sftp_read_text(state: St<'_>, connection_id: ConnectionId, path: String) -> Res<RemoteText> {
    let client = client(&state, connection_id).await?;
    let stat = client.stat(&path).await.map_err(|e| e.info())?;
    if stat.kind == EntryKind::Dir {
        return Err(ErrorInfo::new(ErrorCode::Unsupported, "That's a folder", "Only files can be opened as text."));
    }
    let (bytes, truncated) = client.read_prefix(&path, 1024 * 1024).await.map_err(|e| e.info())?;
    decode_text(&bytes, truncated, version_of(&stat))
        .map_err(|why| ErrorInfo::new(ErrorCode::Unsupported, "Can't open this file as text", why.explain()))
}

/// Saves edited text: the file's line ending and BOM are put back, a file that changed on the server in the meantime
/// is not overwritten without a `conflict`, and the swap is atomic — a failed save leaves the old file as it was.
#[tauri::command]
pub async fn sftp_write_text(
    state: St<'_>,
    connection_id: ConnectionId,
    request: WriteTextRequest,
) -> Res<FileVersion> {
    let client = client(&state, connection_id).await?;
    // The file itself, not a link to it: editing through a symlink must not replace the link.
    let real = client.canonicalize(&request.path).await.unwrap_or_else(|_| request.path.clone());
    if let Some(expected) = request.expected
        && let Ok(now) = client.stat(&real).await
        && version_of(&now) != expected
    {
        return Err(ErrorInfo::new(
            ErrorCode::Conflict,
            "The file changed on the server",
            "It was modified after you opened it, so saving now would overwrite that change.",
        )
        .hint("Save anyway to replace it, or close the editor and open the file again to see the newer version."));
    }
    let bytes = encode_text(&request.text, request.line_ending, request.bom);
    client.replace_file(&real, &bytes).await.map_err(|e| e.info())?;
    let now = client.stat(&real).await.map_err(|e| e.info())?;
    Ok(version_of(&now))
}

// ── transfers ────────────────────────────────────────────────────────────────

fn summarize(id: String, plan: &aether_ssh::Plan) -> PlanSummary {
    PlanSummary {
        plan_id: id,
        file_count: plan.file_count,
        total_bytes: plan.total_bytes,
        conflicts: plan.conflicts.clone(),
        warnings: plan.warnings.clone(),
    }
}

/// Figures out what an upload involves (and which files would be overwritten).
#[tauri::command]
pub async fn transfer_plan_upload(
    state: St<'_>,
    connection_id: ConnectionId,
    local_paths: Vec<String>,
    remote_dir: String,
) -> Res<PlanSummary> {
    let client = client(&state, connection_id).await?;
    let remote_dir = resolve(&client, &remote_dir).await;
    let paths: Vec<PathBuf> = local_paths.into_iter().map(PathBuf::from).collect();
    let plan = plan_upload(&client, &paths, &remote_dir).await.map_err(|e| e.info())?;
    let id = uuid::Uuid::new_v4().to_string();
    let summary = summarize(id.clone(), &plan);
    state.plans.insert(
        id,
        PendingPlan {
            connection: connection_id,
            direction: TransferDirection::Upload,
            plan,
            reveal: None,
            created: Instant::now(),
        },
    );
    prune_plans(&state);
    Ok(summary)
}

#[tauri::command]
pub async fn transfer_plan_download(
    state: St<'_>,
    connection_id: ConnectionId,
    remote_paths: Vec<String>,
    local_dir: String,
) -> Res<PlanSummary> {
    let client = client(&state, connection_id).await?;
    let dir = PathBuf::from(&local_dir);
    std::fs::create_dir_all(&dir).map_err(|e| {
        ErrorInfo::new(aether_core::ErrorCode::Io, "Can't use that folder", format!("{local_dir} couldn't be created."))
            .detail(e.to_string())
    })?;
    let plan = plan_download(&client, &remote_paths, &dir).await.map_err(|e| e.info())?;
    let id = uuid::Uuid::new_v4().to_string();
    let summary = summarize(id.clone(), &plan);
    state.plans.insert(
        id,
        PendingPlan {
            connection: connection_id,
            direction: TransferDirection::Download,
            plan,
            reveal: Some(local_dir),
            created: Instant::now(),
        },
    );
    prune_plans(&state);
    Ok(summary)
}

fn prune_plans(state: &St<'_>) {
    state.plans.retain(|_, p| p.created.elapsed() < Duration::from_secs(15 * 60));
}

#[tauri::command]
pub async fn transfer_start(state: St<'_>, plan_id: String, policy: ConflictPolicy) -> Res<TransferId> {
    let (_, pending) = state.plans.remove(&plan_id).ok_or_else(|| not_found("This transfer"))?;
    let client = client(&state, pending.connection).await?;
    let settings = state.settings.get();
    let options = TransferOptions {
        concurrency: settings.sftp.concurrent_transfers.into(),
        preserve_times: settings.sftp.preserve_times,
    };
    Ok(state.transfers.start(TransferRequest {
        connection: pending.connection,
        client,
        direction: pending.direction,
        plan: pending.plan,
        policy,
        options,
        reveal: pending.reveal,
    }))
}

#[tauri::command]
pub fn transfer_discard_plan(state: St<'_>, plan_id: String) {
    state.plans.remove(&plan_id);
}

#[tauri::command]
pub fn transfers_list(state: St<'_>) -> Vec<TransferInfo> {
    state.transfers.list()
}

#[tauri::command]
pub fn transfer_pause(state: St<'_>, id: TransferId) {
    state.transfers.pause(id);
}

#[tauri::command]
pub async fn transfer_resume(state: St<'_>, id: TransferId) -> Res<()> {
    // If the connection dropped and came back since the transfer stopped, the channel it started on is gone: carry on
    // over the connection's current one.
    let fresh = match state.transfers.connection_of(id) {
        Some(connection) => Some(client(&state, connection).await?),
        None => return Ok(()),
    };
    state.transfers.resume(id, fresh);
    Ok(())
}

#[tauri::command]
pub fn transfer_cancel(state: St<'_>, id: TransferId) {
    state.transfers.cancel(id);
}

#[tauri::command]
pub fn transfer_dismiss(state: St<'_>, id: TransferId) {
    state.transfers.dismiss(id);
}

#[tauri::command]
pub fn transfers_clear_finished(state: St<'_>) {
    state.transfers.clear_finished();
}
