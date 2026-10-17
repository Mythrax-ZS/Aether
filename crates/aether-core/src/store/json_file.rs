//! Crash-safe JSON persistence.
//!
//! Writes go to a temporary sibling file that is flushed to disk and then renamed over the
//! target, so a crash or power loss can never leave a half-written file. The previous
//! version is kept as `<name>.bak` — but only if it was sound: a file that doesn't parse,
//! or is empty, never replaces a good backup.
//!
//! Loading tells apart three situations:
//!
//! * the content is bad (it doesn't parse): it is moved aside to `<name>.corrupt-<timestamp>`
//!   and the backup, if any, is restored — the app always starts, and nothing is destroyed;
//! * the file is empty (the usual sign of a write that was cut short by something that isn't
//!   us): the backup is restored if there is one, otherwise it counts as "no data yet";
//! * the file can't be *read* (locked by a virus scanner, permissions): that says nothing
//!   about its content, so it is neither moved nor replaced — the app starts without the
//!   data, and saving is refused so the file that is probably fine is not overwritten with
//!   an empty one.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;
use tracing::{debug, warn};

use crate::error::{CoreError, Result};
use crate::util::now_ms;

#[derive(Debug, Clone)]
pub struct JsonFile {
    path: PathBuf,
    /// Set when the file exists but could not be read at load time. Saving over it would replace data that may be
    /// perfectly fine with whatever the app started with, so saves are refused instead.
    unreadable: Arc<AtomicBool>,
}

/// Result of loading a file, noting whether recovery was needed.
#[derive(Debug)]
pub struct Loaded<T> {
    pub value: T,
    /// Set when the primary file was unreadable and something else was used instead.
    pub recovery: Option<CoreError>,
}

/// What a file held, before it is interpreted.
enum Content {
    Missing,
    /// Present but nothing in it (or only whitespace / a byte-order mark).
    Blank,
    Text(Vec<u8>),
}

/// How often and how long to wait for a file that is momentarily locked (a scanner opening it) before giving up.
const READ_ATTEMPTS: u32 = 4;
const READ_RETRY_DELAY: Duration = Duration::from_millis(75);

impl JsonFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into(), unreadable: Arc::new(AtomicBool::new(false)) }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn sibling(&self, suffix: &str) -> PathBuf {
        let mut name = self.path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
        name.push(suffix);
        self.path.with_file_name(name)
    }

    fn backup_path(&self) -> PathBuf {
        self.sibling(".bak")
    }

    fn read_content(path: &Path) -> std::io::Result<Content> {
        let bytes = match fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Content::Missing),
            Err(e) => return Err(e),
        };
        // Tolerate a UTF-8 BOM (Notepad adds one).
        let slice = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(&bytes);
        if slice.iter().all(u8::is_ascii_whitespace) {
            return Ok(Content::Blank);
        }
        Ok(Content::Text(slice.to_vec()))
    }

    /// Like [`Self::read_content`], waiting a moment for a file that is only briefly locked.
    fn read_content_patiently(&self) -> std::io::Result<Content> {
        let mut last = None;
        for attempt in 0..READ_ATTEMPTS {
            if attempt > 0 {
                std::thread::sleep(READ_RETRY_DELAY);
            }
            match Self::read_content(&self.path) {
                Ok(content) => return Ok(content),
                Err(e) => last = Some(e),
            }
        }
        Err(last.expect("at least one attempt was made"))
    }

    /// Reads and parses the file. `Ok(None)` when it doesn't exist or is empty.
    pub fn read<T: DeserializeOwned>(&self) -> Result<Option<T>> {
        match Self::read_content(&self.path).map_err(|e| CoreError::io(&self.path, e))? {
            Content::Missing | Content::Blank => Ok(None),
            Content::Text(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| CoreError::Corrupt { path: self.path.clone(), message: e.to_string() }),
        }
    }

    /// The good copy kept from before the last save, if there is one worth restoring.
    fn read_backup<T: DeserializeOwned>(&self) -> Option<T> {
        JsonFile::new(self.backup_path()).read::<T>().ok().flatten()
    }

    /// Loads the file, falling back to its backup and then to `T::default()`.
    pub fn load_or_default<T: DeserializeOwned + Default>(&self) -> Loaded<T> {
        match self.read_content_patiently() {
            Ok(Content::Missing) => Loaded { value: T::default(), recovery: None },
            Ok(Content::Blank) => match self.read_backup::<T>() {
                Some(value) => {
                    warn!(path = %self.path.display(), "data file was empty; restored from backup");
                    let _ = fs::copy(self.backup_path(), &self.path);
                    let err = CoreError::Corrupt { path: self.path.clone(), message: "the file was empty".into() };
                    Loaded { value, recovery: Some(err) }
                }
                None => Loaded { value: T::default(), recovery: None },
            },
            Ok(Content::Text(bytes)) => match serde_json::from_slice::<T>(&bytes) {
                Ok(value) => Loaded { value, recovery: None },
                Err(e) => {
                    let err = CoreError::Corrupt { path: self.path.clone(), message: e.to_string() };
                    warn!(path = %self.path.display(), error = %err, "primary data file unreadable");
                    self.quarantine();
                    match self.read_backup::<T>() {
                        Some(value) => {
                            warn!(path = %self.path.display(), "restored from backup");
                            // Put the good copy back so the next load is clean.
                            let _ = fs::copy(self.backup_path(), &self.path);
                            Loaded { value, recovery: Some(err) }
                        }
                        None => Loaded { value: T::default(), recovery: Some(err) },
                    }
                }
            },
            Err(e) => {
                // Not a statement about the content, so the file is left exactly where it is, and nothing is saved over
                // it: starting empty is fine for a session, overwriting what may be a good file is not.
                let err = CoreError::io(&self.path, e);
                warn!(path = %self.path.display(), error = %err, "data file exists but could not be read; leaving it alone");
                self.unreadable.store(true, Ordering::SeqCst);
                Loaded { value: T::default(), recovery: Some(err) }
            }
        }
    }

    fn quarantine(&self) {
        let target = self.sibling(&format!(".corrupt-{}", now_ms()));
        match fs::rename(&self.path, &target) {
            Ok(()) => warn!(from = %self.path.display(), to = %target.display(), "moved corrupt file aside"),
            Err(e) => warn!(error = %e, "could not move corrupt file aside"),
        }
    }

    /// Atomically replaces the file with the serialised value.
    pub fn save<T: Serialize>(&self, value: &T) -> Result<()> {
        if self.unreadable.load(Ordering::SeqCst) {
            return Err(CoreError::io(
                &self.path,
                std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "this file could not be read when Aether started, so it is not being overwritten — fix its permissions \
                     (or close whatever is using it) and restart Aether",
                ),
            ));
        }
        let bytes = serde_json::to_vec_pretty(value)
            .map_err(|e| CoreError::Parse { what: self.path.display().to_string(), message: e.to_string() })?;
        // Only a file that is itself valid JSON may become the backup.
        self.write_with(&bytes, |existing| serde_json::from_slice::<serde::de::IgnoredAny>(existing).is_ok())
    }

    /// Atomically replaces the file with `bytes` (not necessarily JSON — `known_hosts` lives here too). The previous
    /// contents are kept as the backup unless they were empty.
    pub fn write_atomic(&self, bytes: &[u8]) -> Result<()> {
        self.write_with(bytes, |_| true)
    }

    /// `worth_keeping` decides whether what is in the file now deserves to become the backup.
    fn write_with(&self, bytes: &[u8], worth_keeping: impl Fn(&[u8]) -> bool) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| CoreError::io(parent, e))?;
        }
        let tmp = self.sibling(".tmp");
        {
            let mut f = File::create(&tmp).map_err(|e| CoreError::io(&tmp, e))?;
            f.write_all(bytes).map_err(|e| CoreError::io(&tmp, e))?;
            f.sync_all().map_err(|e| CoreError::io(&tmp, e))?;
        }
        self.refresh_backup(&worth_keeping);
        fs::rename(&tmp, &self.path).map_err(|e| {
            let _ = fs::remove_file(&tmp);
            CoreError::io(&self.path, e)
        })
    }

    /// Keeps what is in the file now as the backup — if it is worth keeping. A file that is empty or damaged would
    /// otherwise push the last good copy out, and the backup is exactly what is needed when the file is damaged.
    fn refresh_backup(&self, worth_keeping: &impl Fn(&[u8]) -> bool) {
        let Ok(existing) = fs::read(&self.path) else { return };
        let body = existing.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(&existing);
        if body.iter().all(u8::is_ascii_whitespace) || !worth_keeping(body) {
            debug!(path = %self.path.display(), "current file is empty or damaged; the backup is left as it is");
            return;
        }
        // Copied under a temporary name first, so a crash can't leave a half-written backup either.
        let staged = self.sibling(".bak.tmp");
        let result = fs::write(&staged, &existing).and_then(|()| fs::rename(&staged, self.backup_path()));
        if let Err(e) = result {
            // Best effort: a failed backup must not block saving.
            debug!(error = %e, "could not refresh backup");
            let _ = fs::remove_file(&staged);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
    struct Doc {
        n: u32,
        name: String,
    }

    fn doc(n: u32) -> Doc {
        Doc { n, name: format!("doc {n}") }
    }

    #[test]
    fn missing_file_yields_default() {
        let dir = tempfile::tempdir().unwrap();
        let f = JsonFile::new(dir.path().join("a.json"));
        let loaded = f.load_or_default::<Doc>();
        assert_eq!(loaded.value, Doc::default());
        assert!(loaded.recovery.is_none());
    }

    #[test]
    fn save_then_load_roundtrips_and_keeps_backup() {
        let dir = tempfile::tempdir().unwrap();
        let f = JsonFile::new(dir.path().join("a.json"));
        f.save(&Doc { n: 1, name: "one".into() }).unwrap();
        f.save(&Doc { n: 2, name: "two".into() }).unwrap();
        assert_eq!(f.load_or_default::<Doc>().value, Doc { n: 2, name: "two".into() });
        let bak: Doc = JsonFile::new(dir.path().join("a.json.bak")).read().unwrap().unwrap();
        assert_eq!(bak.n, 1);
        assert!(!dir.path().join("a.json.tmp").exists());
        assert!(!dir.path().join("a.json.bak.tmp").exists());
    }

    #[test]
    fn corrupt_file_is_quarantined_and_backup_restored() {
        let dir = tempfile::tempdir().unwrap();
        let f = JsonFile::new(dir.path().join("a.json"));
        f.save(&Doc { n: 1, name: "good".into() }).unwrap();
        f.save(&Doc { n: 2, name: "newer".into() }).unwrap();
        fs::write(f.path(), b"{ this is not json").unwrap();

        let loaded = f.load_or_default::<Doc>();
        assert_eq!(loaded.value.n, 1, "falls back to the backup");
        assert!(matches!(loaded.recovery, Some(CoreError::Corrupt { .. })));
        let quarantined = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().contains(".corrupt-"));
        assert!(quarantined, "corrupt file kept for inspection");
        // The restored file now loads cleanly.
        assert!(f.load_or_default::<Doc>().recovery.is_none());
    }

    #[test]
    fn corrupt_without_backup_starts_fresh() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("b.json");
        fs::write(&path, b"garbage").unwrap();
        let loaded = JsonFile::new(&path).load_or_default::<Doc>();
        assert_eq!(loaded.value, Doc::default());
        assert!(loaded.recovery.is_some());
    }

    #[test]
    fn tolerates_a_bom_and_a_hand_made_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.json");
        fs::write(&path, b"\xEF\xBB\xBF{\"n\":7,\"name\":\"bom\"}").unwrap();
        assert_eq!(JsonFile::new(&path).load_or_default::<Doc>().value.n, 7);
        fs::write(&path, b"  \n").unwrap();
        let loaded = JsonFile::new(&path).load_or_default::<Doc>();
        assert!(loaded.recovery.is_none(), "an empty file with nothing to restore is just 'no data yet'");
        assert_eq!(loaded.value, Doc::default());
    }

    /// A write cut short by something other than us leaves an empty file. Treating that as "no hosts" would show the
    /// person an empty app — and the next save would then push the good copy out of the backup too.
    #[test]
    fn an_emptied_file_is_restored_from_the_backup_not_trusted() {
        let dir = tempfile::tempdir().unwrap();
        let f = JsonFile::new(dir.path().join("a.json"));
        f.save(&doc(1)).unwrap();
        f.save(&doc(2)).unwrap(); // the backup now holds 1
        fs::write(f.path(), b"").unwrap();

        let loaded = f.load_or_default::<Doc>();
        assert_eq!(loaded.value, doc(1), "the last good copy comes back");
        assert!(loaded.recovery.is_some(), "and the person is told");
        assert!(fs::read_to_string(f.path()).unwrap().contains("doc 1"), "restored on disk too");
    }

    /// The backup is for the day the file is damaged. A damaged file must not become the backup.
    #[test]
    fn a_damaged_or_empty_file_never_replaces_the_backup() {
        let dir = tempfile::tempdir().unwrap();
        let f = JsonFile::new(dir.path().join("a.json"));
        f.save(&doc(1)).unwrap();
        f.save(&doc(2)).unwrap(); // bak = 1, file = 2

        // Something damages the file while the app is running; the app saves again.
        fs::write(f.path(), b"{ half a doc").unwrap();
        f.save(&doc(3)).unwrap();
        let bak: Doc = JsonFile::new(dir.path().join("a.json.bak")).read().unwrap().unwrap();
        assert_eq!(bak, doc(1), "still the last good one, not the damaged text");

        fs::write(f.path(), b"").unwrap();
        f.save(&doc(4)).unwrap();
        let bak: Doc = JsonFile::new(dir.path().join("a.json.bak")).read().unwrap().unwrap();
        assert_eq!(bak, doc(1), "an empty file doesn't push it out either");
        assert_eq!(f.load_or_default::<Doc>().value, doc(4));
    }

    /// "I couldn't read it" says nothing about what is in it. The file stays where it is and is not overwritten.
    #[test]
    fn a_file_that_cannot_be_read_is_left_alone_and_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("locked.json");
        // A directory where the file should be: reading it fails with something other than "not found".
        fs::create_dir(&path).unwrap();

        let f = JsonFile::new(&path);
        let loaded = f.load_or_default::<Doc>();
        assert_eq!(loaded.value, Doc::default());
        assert!(matches!(loaded.recovery, Some(CoreError::Io { .. })), "{:?}", loaded.recovery);
        let moved_aside =
            fs::read_dir(dir.path()).unwrap().flatten().any(|e| e.file_name().to_string_lossy().contains(".corrupt-"));
        assert!(!moved_aside, "an unreadable file is not a corrupt one");
        assert!(path.is_dir(), "still exactly where it was");

        let err = f.save(&doc(1)).unwrap_err();
        assert!(err.to_string().contains("not being overwritten"), "{err}");
        assert!(path.is_dir());
    }
}
