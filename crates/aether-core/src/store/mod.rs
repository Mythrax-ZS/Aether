//! File-backed stores. Each owns one file (or folder) and keeps an in-memory copy behind
//! a lock; every mutation is written through atomically.

mod hosts;
mod json_file;
mod keys;
mod recents;
mod settings;
mod themes;

pub use hosts::{GroupDeleteMode, HOSTS_VERSION, HostStore, HostsData, ImportSummary};
pub use json_file::{JsonFile, Loaded};
pub use keys::KeyStore;
pub use recents::{RecentTarget, RecentsStore};
pub use settings::SettingsStore;
pub use themes::ThemeStore;
