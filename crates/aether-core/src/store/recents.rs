//! Quick-connect history: ad-hoc targets typed into the palette that were never saved as hosts.

use std::path::PathBuf;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::json_file::JsonFile;
use crate::error::{CoreError, Result};
use crate::util::now_ms;

const MAX_RECENTS: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RecentTarget {
    /// `user@host[:port]`
    pub target: String,
    pub last_used: i64,
    pub count: u32,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct RecentsData {
    recents: Vec<RecentTarget>,
}

pub struct RecentsStore {
    file: JsonFile,
    data: Mutex<RecentsData>,
}

impl RecentsStore {
    pub fn open(path: impl Into<PathBuf>) -> Self {
        Self::open_reporting(path).0
    }

    /// Like [`RecentsStore::open`], also saying whether the file had to be recovered (or could not be read).
    pub fn open_reporting(path: impl Into<PathBuf>) -> (Self, Option<CoreError>) {
        let file = JsonFile::new(path);
        let loaded = file.load_or_default::<RecentsData>();
        (Self { file, data: Mutex::new(loaded.value) }, loaded.recovery)
    }

    pub fn list(&self) -> Vec<RecentTarget> {
        self.data.lock().recents.clone()
    }

    /// Records a use of `target`, moving it to the front.
    pub fn record(&self, target: &str) -> Result<()> {
        let target = target.trim();
        if target.is_empty() {
            return Ok(());
        }
        self.transact(|data| {
            let count = data
                .recents
                .iter()
                .position(|r| r.target.eq_ignore_ascii_case(target))
                .map(|i| data.recents.remove(i).count)
                .unwrap_or(0);
            data.recents.insert(
                0,
                RecentTarget { target: target.to_string(), last_used: now_ms(), count: count.saturating_add(1) },
            );
            data.recents.truncate(MAX_RECENTS);
        })
    }

    pub fn remove(&self, target: &str) -> Result<()> {
        self.transact(|data| data.recents.retain(|r| !r.target.eq_ignore_ascii_case(target)))
    }

    pub fn clear(&self) -> Result<()> {
        self.transact(|data| data.recents.clear())
    }

    /// Applies `change` to a copy and keeps it only once it has reached the disk.
    fn transact(&self, change: impl FnOnce(&mut RecentsData)) -> Result<()> {
        let mut data = self.data.lock();
        let mut next = RecentsData { recents: data.recents.clone() };
        change(&mut next);
        self.file.save(&next)?;
        *data = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn most_recent_first_with_counts() {
        let dir = tempfile::tempdir().unwrap();
        let store = RecentsStore::open(dir.path().join("recents.json"));
        store.record("a@one").unwrap();
        store.record("b@two").unwrap();
        store.record("A@ONE").unwrap();
        let list = store.list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].target, "A@ONE");
        assert_eq!(list[0].count, 2);
    }

    #[test]
    fn capped_and_persistent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recents.json");
        let store = RecentsStore::open(&path);
        for i in 0..(MAX_RECENTS + 10) {
            store.record(&format!("host{i}")).unwrap();
        }
        assert_eq!(store.list().len(), MAX_RECENTS);
        assert_eq!(RecentsStore::open(&path).list().len(), MAX_RECENTS);
        store.clear().unwrap();
        assert!(RecentsStore::open(&path).list().is_empty());
    }
}
