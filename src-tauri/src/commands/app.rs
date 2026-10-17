//! App-level commands: info, window lifecycle, fonts, opening paths.

use std::sync::OnceLock;

use aether_core::ErrorInfo;
use aether_core::error::{ErrorCode, UserFacing};
use serde::Deserialize;
use tauri::{AppHandle, WebviewWindow};
use ts_rs::TS;

use super::{St, blocking};
use crate::dto::{AppInfo, FontInfo};
use crate::state::Res;
use crate::window;

#[tauri::command]
pub fn app_info(state: St<'_>) -> AppInfo {
    AppInfo {
        name: "Aether".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        debug: cfg!(debug_assertions),
        storage_mode: state.paths.mode,
        config_dir: state.paths.config_dir.display().to_string(),
        logs_dir: state.paths.logs_dir.display().to_string(),
        os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        vault_backend: state.vault.backend(),
        warnings: state.startup_warnings.clone(),
    }
}

/// The UI calls this once it has painted its first frame; revealing the window only then
/// avoids a white flash while the webview boots. When Windows started Aether at sign-in (and the tray icon is there to
/// bring it back), the first window stays hidden instead.
#[tauri::command]
pub fn app_ready(window: WebviewWindow, state: St<'_>) {
    if window.label() == "main" {
        state.set_interface_ready();
        tracing::info!("the interface is ready");
        if state.take_background_start() {
            return;
        }
    }
    window::reveal(&window);
}

#[tauri::command]
pub fn app_quit(app: AppHandle) {
    app.exit(0);
}

#[tauri::command]
pub fn app_restart(app: AppHandle) {
    app.restart();
}

#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NewWindowRequest {
    /// When set, the new window takes over this session's terminal (tab tear-off).
    pub adopt_connection: Option<String>,
    pub adopt_terminal: Option<String>,
    /// The saved host that session came from, so the new window can show it as that host.
    pub adopt_host: Option<String>,
    pub title: Option<String>,
}

/// Async on purpose: see [`window::create`] — building a window from a synchronous command deadlocks on Windows.
#[tauri::command]
pub async fn window_new(app: AppHandle, request: NewWindowRequest) -> Res<String> {
    window::create(&app, request).await.map_err(|e| ErrorInfo::internal(e.to_string()))
}

/// Installed font families, flagged when monospaced. Loaded once per process.
#[tauri::command]
pub async fn list_fonts() -> Res<Vec<FontInfo>> {
    static FONTS: OnceLock<Vec<FontInfo>> = OnceLock::new();
    if let Some(f) = FONTS.get() {
        return Ok(f.clone());
    }
    let fonts = tokio::task::spawn_blocking(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        let mut by_family: std::collections::BTreeMap<String, bool> = std::collections::BTreeMap::new();
        for face in db.faces() {
            if let Some((family, _)) = face.families.first() {
                let entry = by_family.entry(family.clone()).or_insert(false);
                *entry |= face.monospaced;
            }
        }
        by_family.into_iter().map(|(family, monospace)| FontInfo { family, monospace }).collect::<Vec<_>>()
    })
    .await
    .map_err(|e| ErrorInfo::internal(e.to_string()))?;
    let _ = FONTS.set(fonts.clone());
    Ok(fonts)
}

/// Opens an existing folder in Explorer, or reveals an existing file inside its folder — and nothing else. A path that
/// isn't there is refused rather than handed to the shell, which would try to *run* whatever it could make of it (a
/// `shell:` or `ms-settings:` address, a program name).
fn open_existing(path: &str) -> Res<()> {
    let p = std::path::PathBuf::from(path);
    let opened = if p.is_dir() {
        tauri_plugin_opener::open_path(&p, None::<&str>)
    } else if p.is_file() {
        tauri_plugin_opener::reveal_item_in_dir(&p)
    } else {
        return Err(ErrorInfo::new(ErrorCode::NotFound, "That location isn't there", format!("{path} doesn't exist.")));
    };
    opened.map_err(|e| {
        ErrorInfo::new(ErrorCode::Io, "Couldn't open that location", format!("{path} could not be opened."))
            .detail(e.to_string())
    })
}

/// Opens a folder in Explorer, or reveals a file inside its folder. Off the UI thread: a network path can take a while.
#[tauri::command]
pub async fn open_path(path: String) -> Res<()> {
    blocking(move || open_existing(&path)).await
}

#[tauri::command]
pub async fn open_logs_dir(state: St<'_>) -> Res<()> {
    let path = state.paths.logs_dir.display().to_string();
    blocking(move || open_existing(&path)).await
}

#[tauri::command]
pub async fn open_config_dir(state: St<'_>) -> Res<()> {
    let path = state.paths.config_dir.display().to_string();
    blocking(move || open_existing(&path)).await
}

/// Reads `custom.css` from the config folder (empty when absent, and when it is absurdly large).
#[tauri::command]
pub async fn custom_css_get(state: St<'_>) -> Res<String> {
    const MAX_BYTES: u64 = 2 * 1024 * 1024;
    let path = state.paths.custom_css_file();
    blocking(move || {
        let too_big = std::fs::metadata(&path).map(|m| m.len() > MAX_BYTES).unwrap_or(false);
        Ok(if too_big { String::new() } else { std::fs::read_to_string(&path).unwrap_or_default() })
    })
    .await
}

pub fn map_core(e: aether_core::CoreError) -> ErrorInfo {
    e.info()
}
