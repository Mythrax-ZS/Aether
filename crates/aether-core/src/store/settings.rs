//! Persistent application settings with JSON merge-patch updates.

use std::path::PathBuf;

use parking_lot::RwLock;
use serde_json::Value;
use tokio::sync::broadcast;

use super::json_file::JsonFile;
use crate::error::{CoreError, Result};
use crate::model::Settings;

pub struct SettingsStore {
    file: JsonFile,
    data: RwLock<Settings>,
    changed: broadcast::Sender<()>,
}

impl SettingsStore {
    pub fn open(path: impl Into<PathBuf>) -> (Self, Option<CoreError>) {
        let file = JsonFile::new(path);
        let loaded = file.load_or_default::<Settings>();
        let mut settings = loaded.value;
        settings.sanitize();
        let (changed, _) = broadcast::channel(16);
        (Self { file, data: RwLock::new(settings), changed }, loaded.recovery)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.changed.subscribe()
    }

    pub fn get(&self) -> Settings {
        self.data.read().clone()
    }

    pub fn replace(&self, mut settings: Settings) -> Result<Settings> {
        settings.sanitize();
        self.file.save(&settings)?;
        *self.data.write() = settings.clone();
        let _ = self.changed.send(());
        Ok(settings)
    }

    /// Applies a partial update. Objects are merged key by key; scalars, arrays and the
    /// `keybindings` map replace the existing value; `null` clears optional fields.
    pub fn patch(&self, patch: Value) -> Result<Settings> {
        let current = self.get();
        let mut merged = serde_json::to_value(&current)
            .map_err(|e| CoreError::Parse { what: "settings".into(), message: e.to_string() })?;
        merge_patch(&mut merged, patch, true);
        let next: Settings = serde_json::from_value(merged)
            .map_err(|e| CoreError::Parse { what: "settings".into(), message: e.to_string() })?;
        self.replace(next)
    }
}

fn merge_patch(target: &mut Value, patch: Value, top_level: bool) {
    match (target, patch) {
        (Value::Object(t), Value::Object(p)) => {
            for (key, value) in p {
                if top_level && key == "keybindings" {
                    t.insert(key, value);
                } else if let (Some(existing), true) = (t.get_mut(&key), value.is_object()) {
                    merge_patch(existing, value, false);
                } else {
                    t.insert(key, value);
                }
            }
        }
        (target, patch) => *target = patch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn open() -> (tempfile::TempDir, SettingsStore) {
        let dir = tempfile::tempdir().unwrap();
        let (s, _) = SettingsStore::open(dir.path().join("settings.json"));
        (dir, s)
    }

    #[test]
    fn patch_merges_nested_objects() {
        let (_d, s) = open();
        let before = s.get();
        let after = s.patch(json!({ "terminal": { "fontSize": 17 } })).unwrap();
        assert_eq!(after.terminal.font_size, 17.0);
        assert_eq!(after.terminal.line_height, before.terminal.line_height, "siblings untouched");
        assert_eq!(after.ssh, before.ssh);
    }

    #[test]
    fn patch_can_clear_optional_fields() {
        let (_d, s) = open();
        s.patch(json!({ "terminal": { "theme": "dracula" } })).unwrap();
        assert_eq!(s.get().terminal.theme.as_deref(), Some("dracula"));
        s.patch(json!({ "terminal": { "theme": null } })).unwrap();
        assert_eq!(s.get().terminal.theme, None);
    }

    #[test]
    fn keybindings_are_replaced_not_merged() {
        let (_d, s) = open();
        s.patch(json!({ "keybindings": { "a": "Ctrl+A", "b": "Ctrl+B" } })).unwrap();
        s.patch(json!({ "keybindings": { "c": "Ctrl+C" } })).unwrap();
        let kb = s.get().keybindings;
        assert_eq!(kb.len(), 1);
        assert_eq!(kb.get("c").map(String::as_str), Some("Ctrl+C"));
    }

    #[test]
    fn invalid_patches_are_rejected_without_changing_state() {
        let (_d, s) = open();
        let before = s.get();
        assert!(s.patch(json!({ "terminal": { "fontSize": "huge" } })).is_err());
        assert_eq!(s.get(), before);
    }

    #[test]
    fn patch_is_persisted_and_clamped() {
        let (dir, s) = open();
        s.patch(json!({ "terminal": { "fontSize": 9999 } })).unwrap();
        let (reopened, _) = SettingsStore::open(dir.path().join("settings.json"));
        assert_eq!(reopened.get().terminal.font_size, 48.0);
    }
}
