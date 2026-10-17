//! Where Aether keeps its files.
//!
//! * **Installed** (default): user-authored data in `%APPDATA%\Aether` (roams with the
//!   profile), machine-bound data (DPAPI blobs), logs and cache in `%LOCALAPPDATA%\Aether`.
//! * **Portable**: if a `portable.flag` file sits next to the executable, everything lives
//!   in `AetherData\` beside it — copy the folder to a USB stick and go.
//! * **Custom**: `AETHER_HOME=<dir>` forces a root (used by tests and development).

use std::path::{Path, PathBuf};

use serde::Serialize;
use ts_rs::TS;

use crate::error::{CoreError, Result};

pub const PORTABLE_MARKER: &str = "portable.flag";
pub const PORTABLE_DIR: &str = "AetherData";
pub const HOME_ENV: &str = "AETHER_HOME";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum StorageMode {
    Installed,
    Portable,
    Custom,
}

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub mode: StorageMode,
    /// Hosts, settings, keys metadata, themes, known_hosts.
    pub config_dir: PathBuf,
    /// DPAPI-sealed secrets. Never roamed because it can't be decrypted elsewhere.
    pub vault_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub cache_dir: PathBuf,
}

impl AppPaths {
    /// Picks the right layout for this process.
    pub fn resolve() -> Self {
        if let Some(root) = std::env::var_os(HOME_ENV).filter(|v| !v.is_empty()) {
            return Self::rooted_at(PathBuf::from(root), StorageMode::Custom);
        }
        if let Ok(exe) = std::env::current_exe()
            && let Some(dir) = exe.parent()
            && dir.join(PORTABLE_MARKER).exists()
        {
            return Self::rooted_at(dir.join(PORTABLE_DIR), StorageMode::Portable);
        }
        let roaming = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        let local = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
        let local_root = local.join("Aether");
        Self {
            mode: StorageMode::Installed,
            config_dir: roaming.join("Aether"),
            vault_dir: local_root.join("vault"),
            logs_dir: local_root.join("logs"),
            cache_dir: local_root.join("cache"),
        }
    }

    /// A self-contained layout under `root` (portable and test setups).
    pub fn rooted_at(root: impl Into<PathBuf>, mode: StorageMode) -> Self {
        let root = root.into();
        Self {
            mode,
            config_dir: root.join("config"),
            vault_dir: root.join("vault"),
            logs_dir: root.join("logs"),
            cache_dir: root.join("cache"),
        }
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        for dir in [&self.config_dir, &self.vault_dir, &self.logs_dir, &self.cache_dir, &self.themes_dir()] {
            std::fs::create_dir_all(dir).map_err(|e| CoreError::io(dir, e))?;
        }
        Ok(())
    }

    pub fn hosts_file(&self) -> PathBuf {
        self.config_dir.join("hosts.json")
    }
    pub fn settings_file(&self) -> PathBuf {
        self.config_dir.join("settings.json")
    }
    pub fn keys_file(&self) -> PathBuf {
        self.config_dir.join("keys.json")
    }
    pub fn known_hosts_file(&self) -> PathBuf {
        self.config_dir.join("known_hosts")
    }
    pub fn recents_file(&self) -> PathBuf {
        self.config_dir.join("recents.json")
    }
    pub fn workspace_file(&self) -> PathBuf {
        self.config_dir.join("workspace.json")
    }
    pub fn custom_css_file(&self) -> PathBuf {
        self.config_dir.join("custom.css")
    }
    pub fn themes_dir(&self) -> PathBuf {
        self.config_dir.join("themes")
    }
    pub fn secrets_dir(&self) -> PathBuf {
        self.vault_dir.join("secrets")
    }

    /// The user's own OpenSSH folder (`~/.ssh`), which Aether only ever reads.
    pub fn user_ssh_dir() -> Option<PathBuf> {
        crate::util::home_dir().map(|h| h.join(".ssh"))
    }
}

impl AsRef<Path> for AppPaths {
    fn as_ref(&self) -> &Path {
        &self.config_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rooted_layout_is_self_contained() {
        let p = AppPaths::rooted_at("X:/data", StorageMode::Portable);
        for d in [&p.config_dir, &p.vault_dir, &p.logs_dir, &p.cache_dir] {
            assert!(d.starts_with("X:/data"));
        }
        assert_eq!(p.hosts_file(), PathBuf::from("X:/data/config/hosts.json"));
    }

    #[test]
    fn ensure_dirs_creates_everything() {
        let tmp = tempfile::tempdir().unwrap();
        let p = AppPaths::rooted_at(tmp.path().join("root"), StorageMode::Custom);
        p.ensure_dirs().unwrap();
        assert!(p.themes_dir().is_dir());
        assert!(p.vault_dir.is_dir());
    }
}
