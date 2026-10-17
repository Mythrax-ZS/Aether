//! User-defined themes: one JSON file per theme in the `themes/` folder.

use std::fs;
use std::path::PathBuf;

use tracing::warn;

use super::json_file::JsonFile;
use crate::error::{CoreError, Result};
use crate::model::Theme;

pub struct ThemeStore {
    dir: PathBuf,
}

impl ThemeStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn file_for(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    /// All valid themes on disk, sorted by name. Invalid files are skipped (and logged) so
    /// one bad hand-edited theme can't break the picker.
    pub fn list(&self) -> Vec<Theme> {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut themes: Vec<Theme> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("json")))
            .filter_map(|path| match JsonFile::new(&path).read::<Theme>() {
                Ok(Some(mut theme)) => match theme.validate() {
                    Ok(()) => {
                        theme.builtin = false;
                        Some(theme)
                    }
                    Err(e) => {
                        warn!(file = %path.display(), error = %e, "ignoring invalid theme");
                        None
                    }
                },
                Ok(None) => None,
                Err(e) => {
                    warn!(file = %path.display(), error = %e, "ignoring unreadable theme");
                    None
                }
            })
            .collect();
        themes.sort_by_key(|t| t.name.to_lowercase());
        themes
    }

    pub fn save(&self, mut theme: Theme) -> Result<Theme> {
        theme.builtin = false;
        theme.validate()?;
        JsonFile::new(self.file_for(&theme.id)).save(&theme)?;
        Ok(theme)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        // Reuse the id validation so a crafted id can't escape the themes folder.
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(CoreError::invalid("theme id", "unknown theme"));
        }
        let path = self.file_for(id);
        fs::remove_file(&path).map_err(|e| CoreError::io(path, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{TerminalPalette, ThemeKind};
    use std::collections::BTreeMap;

    fn theme(id: &str, name: &str) -> Theme {
        let c = || "#123456".to_string();
        Theme {
            id: id.into(),
            name: name.into(),
            author: None,
            kind: ThemeKind::Dark,
            ui: BTreeMap::new(),
            terminal: TerminalPalette {
                background: c(),
                foreground: c(),
                cursor: c(),
                cursor_accent: c(),
                selection_background: c(),
                selection_foreground: None,
                black: c(),
                red: c(),
                green: c(),
                yellow: c(),
                blue: c(),
                magenta: c(),
                cyan: c(),
                white: c(),
                bright_black: c(),
                bright_red: c(),
                bright_green: c(),
                bright_yellow: c(),
                bright_blue: c(),
                bright_magenta: c(),
                bright_cyan: c(),
                bright_white: c(),
            },
            builtin: true,
        }
    }

    #[test]
    fn save_list_delete() {
        let dir = tempfile::tempdir().unwrap();
        let store = ThemeStore::new(dir.path());
        store.save(theme("zeta", "Zeta")).unwrap();
        store.save(theme("alpha", "Alpha")).unwrap();
        let names: Vec<_> = store.list().into_iter().map(|t| t.name).collect();
        assert_eq!(names, ["Alpha", "Zeta"]);
        assert!(store.list().iter().all(|t| !t.builtin));
        store.delete("alpha").unwrap();
        assert_eq!(store.list().len(), 1);
    }

    #[test]
    fn invalid_files_are_ignored_and_ids_cannot_escape() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("bad.json"), "{ nope").unwrap();
        let store = ThemeStore::new(dir.path());
        assert!(store.list().is_empty());
        assert!(store.delete("../../etc/passwd").is_err());
    }
}
