//! Settings, themes and the persisted workspace layout.

use aether_core::ErrorInfo;
use aether_core::model::{Settings, Theme};
use aether_core::store::JsonFile;
use serde_json::Value;
use tauri::{AppHandle, Emitter};

use super::St;
use super::app::map_core;
use crate::dto::SettingsEvent;
use crate::state::Res;

#[tauri::command]
pub fn settings_get(state: St<'_>) -> Settings {
    state.settings.get()
}

/// Merges a partial settings object (see `SettingsStore::patch`) and tells every window.
#[tauri::command]
pub fn settings_patch(app: AppHandle, state: St<'_>, patch: Value) -> Res<Settings> {
    let next = state.settings.patch(patch).map_err(map_core)?;
    state.apply_settings(&next);
    let _ = app.emit("settings:changed", SettingsEvent { settings: next.clone() });
    Ok(next)
}

#[tauri::command]
pub fn settings_reset(app: AppHandle, state: St<'_>) -> Res<Settings> {
    let next = state.settings.replace(Settings::default()).map_err(map_core)?;
    state.apply_settings(&next);
    let _ = app.emit("settings:changed", SettingsEvent { settings: next.clone() });
    Ok(next)
}

#[tauri::command]
pub fn themes_custom(state: St<'_>) -> Vec<Theme> {
    state.themes.list()
}

#[tauri::command]
pub fn theme_save(state: St<'_>, theme: Theme) -> Res<Theme> {
    state.themes.save(theme).map_err(map_core)
}

#[tauri::command]
pub fn theme_delete(state: St<'_>, id: String) -> Res<()> {
    state.themes.delete(&id).map_err(map_core)
}

/// The tabs/splits layout last saved by the UI, as opaque JSON.
#[tauri::command]
pub fn workspace_load(state: St<'_>) -> Option<Value> {
    JsonFile::new(state.paths.workspace_file()).read::<Value>().ok().flatten()
}

#[tauri::command]
pub fn workspace_save(state: St<'_>, workspace: Value) -> Res<()> {
    JsonFile::new(state.paths.workspace_file()).save(&workspace).map_err(|e| -> ErrorInfo { map_core(e) })
}
