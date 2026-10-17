//! Th local side of the SFTP dual-pane browser.

use std::path::{Path, PathBuf};

use aether_core::ErrorInfo;
use aether_core::error::ErrorCode;

use super::blocking;
use crate::dto::{LocalEntry, LocalEntryKind, LocalListing, LocalRoot, LocalRootKind};
use crate::state::Res;

fn io_error(what: &str, path: &Path, e: std::io::Error) -> ErrorInfo {
    let (code, title) = match e.kind() {
        std::io::ErrorKind::PermissionDenied => (ErrorCode::PermissionDenied, "Access denied"),
        std::io::ErrorKind::NotFound => (ErrorCode::NotFound, "Not found"),
        std::io::ErrorKind::AlreadyExists => (ErrorCode::Conflict, "Already exists"),
        _ => (ErrorCode::Io, "File error"),
    };
    ErrorInfo::new(code, title, format!("Couldn't {what} {}.", path.display())).detail(e.to_string())
}

#[cfg(windows)]
fn is_hidden_attr(md: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    md.file_attributes() & (HIDDEN | SYSTEM) != 0
}

#[cfg(not(windows))]
fn is_hidden_attr(_: &std::fs::Metadata) -> bool {
    false
}

fn modified_ms(md: &std::fs::Metadata) -> Option<i64> {
    md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64)
}

/// Drives and well-known folders for the sidebar of the local pane.
#[tauri::command]
pub fn local_roots() -> Vec<LocalRoot> {
    let mut roots = Vec::new();
    let mut add = |path: Option<PathBuf>, label: &str, kind: LocalRootKind| {
        if let Some(p) = path.filter(|p| p.is_dir()) {
            roots.push(LocalRoot { path: p.display().to_string(), label: label.into(), kind });
        }
    };
    add(dirs::home_dir(), "Home", LocalRootKind::Home);
    add(dirs::desktop_dir(), "Desktop", LocalRootKind::Desktop);
    add(dirs::document_dir(), "Documents", LocalRootKind::Documents);
    add(dirs::download_dir(), "Downloads", LocalRootKind::Downloads);
    roots.extend(drives());
    roots
}

#[cfg(windows)]
fn drives() -> Vec<LocalRoot> {
    use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW};
    use windows::core::PCWSTR;

    // SAFETY: plain Win32 queries with valid, NUL-terminated buffers.
    let mask = unsafe { GetLogicalDrives() };
    let mut out = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        let root: Vec<u16> = format!("{letter}:\\").encode_utf16().chain(std::iter::once(0)).collect();
        // 2 = removable, 3 = fixed, 4 = network, 5 = CD-ROM, 6 = RAM disk.
        let drive_type = unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) };
        if drive_type < 2 {
            continue;
        }
        let mut name = [0u16; 261];
        let has_label =
            unsafe { GetVolumeInformationW(PCWSTR(root.as_ptr()), Some(&mut name), None, None, None, None).is_ok() };
        let label = if has_label {
            let end = name.iter().position(|c| *c == 0).unwrap_or(0);
            String::from_utf16_lossy(&name[..end])
        } else {
            String::new()
        };
        let label = if label.is_empty() { format!("Local Disk ({letter}:)") } else { format!("{label} ({letter}:)") };
        out.push(LocalRoot { path: format!("{letter}:\\"), label, kind: LocalRootKind::Drive });
    }
    out
}

#[cfg(not(windows))]
fn drives() -> Vec<LocalRoot> {
    vec![LocalRoot { path: "/".into(), label: "Root".into(), kind: LocalRootKind::Drive }]
}

#[tauri::command]
pub async fn local_list(path: String) -> Res<LocalListing> {
    blocking(move || {
        let dir = if path.trim().is_empty() {
            dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
        } else {
            PathBuf::from(&path)
        };
        let rd = std::fs::read_dir(&dir).map_err(|e| io_error("open", &dir, e))?;
        let mut entries = Vec::new();
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let p = entry.path();
            let lmd = entry.metadata().ok();
            let is_symlink = entry.file_type().map(|t| t.is_symlink()).unwrap_or(false);
            // Follow links for the kind so a link to a folder opens like a folder.
            let md = if is_symlink { std::fs::metadata(&p).ok().or(lmd.clone()) } else { lmd.clone() };
            let (kind, size, modified) = match &md {
                Some(m) if m.is_dir() => (LocalEntryKind::Dir, 0, modified_ms(m)),
                Some(m) if m.is_file() => (LocalEntryKind::File, m.len(), modified_ms(m)),
                Some(m) => (LocalEntryKind::Other, m.len(), modified_ms(m)),
                None => (LocalEntryKind::Other, 0, None),
            };
            let hidden = name.starts_with('.') || lmd.as_ref().is_some_and(is_hidden_attr);
            entries.push(LocalEntry { name, path: p.display().to_string(), kind, is_symlink, hidden, size, modified });
        }
        Ok(LocalListing {
            parent: dir.parent().map(|p| p.display().to_string()).filter(|p| !p.is_empty()),
            path: dir.display().to_string(),
            entries,
        })
    })
    .await
}

#[tauri::command]
pub async fn local_mkdir(path: String) -> Res<()> {
    blocking(move || {
        let p = PathBuf::from(&path);
        std::fs::create_dir(&p).map_err(|e| io_error("create", &p, e))
    })
    .await
}

/// Renames `from` to `to` — only if nothing is called `to` already. The check and the rename are one operation, so a
/// file that appears in between is not silently replaced. (`std::fs::rename` replaces on Windows.)
fn rename_without_replacing(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::Storage::FileSystem::{MOVE_FILE_FLAGS, MoveFileExW};
        use windows::core::PCWSTR;

        let wide = |p: &Path| -> Vec<u16> { p.as_os_str().encode_wide().chain(std::iter::once(0)).collect() };
        let (f, t) = (wide(from), wide(to));
        // SAFETY: both paths are NUL-terminated and outlive the call. No flags: a move that would replace fails.
        // A rename that only changes the case of the name (`notes.txt` → `Notes.txt`) is the same file and works.
        unsafe { MoveFileExW(PCWSTR(f.as_ptr()), PCWSTR(t.as_ptr()), MOVE_FILE_FLAGS(0)) }
            // HRESULT_FROM_WIN32 keeps the Win32 code in the low 16 bits.
            .map_err(|e| std::io::Error::from_raw_os_error(i32::from((e.code().0 & 0xFFFF) as u16)))
    }
    #[cfg(not(windows))]
    {
        if to.symlink_metadata().is_ok() {
            return Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists));
        }
        std::fs::rename(from, to)
    }
}

#[tauri::command]
pub async fn local_rename(from: String, to: String) -> Res<()> {
    blocking(move || {
        let p = PathBuf::from(&from);
        rename_without_replacing(&p, Path::new(&to)).map_err(|e| match e.kind() {
            std::io::ErrorKind::AlreadyExists => {
                ErrorInfo::new(ErrorCode::Conflict, "Already exists", format!("{to} already exists."))
            }
            _ => io_error("rename", &p, e),
        })
    })
    .await
}

/// Moves items to the Recycle Bin (never a permanent delete).
#[tauri::command]
pub async fn local_delete(paths: Vec<String>) -> Res<()> {
    blocking(move || {
        trash::delete_all(&paths).map_err(|e| {
            ErrorInfo::new(ErrorCode::Io, "Couldn't move to the Recycle Bin", "Some items couldn't be deleted.")
                .detail(e.to_string())
        })
    })
    .await
}

#[tauri::command]
pub async fn local_exists(path: String) -> bool {
    tokio::task::spawn_blocking(move || Path::new(&path).exists()).await.unwrap_or(false)
}

/// The folder downloads go to by default.
#[tauri::command]
pub fn local_default_download_dir(state: super::St<'_>) -> String {
    state
        .settings
        .get()
        .sftp
        .download_dir
        .or_else(|| dirs::download_dir().map(|d| d.display().to_string()))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> =
            std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    #[test]
    fn a_rename_never_replaces_something_that_is_already_there() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a.txt"), dir.path().join("b.txt"));
        std::fs::write(&a, "from a").unwrap();
        std::fs::write(&b, "from b").unwrap();

        let err = rename_without_replacing(&a, &b).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists, "{err}");
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "from a");
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "from b", "the existing file is untouched");

        let c = dir.path().join("c.txt");
        rename_without_replacing(&a, &c).unwrap();
        assert_eq!(std::fs::read_to_string(&c).unwrap(), "from a");
        assert!(!a.exists());
        // Renaming something that isn't there is its own error.
        assert_eq!(
            rename_without_replacing(&a, &dir.path().join("d.txt")).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
    }

    /// `notes.txt` → `Notes.txt` is one file getting a new spelling, not a clash with itself — and a different file
    /// that merely differs by case *is* a clash on a case-insensitive disk.
    #[cfg(windows)]
    #[test]
    fn changing_only_the_case_of_a_name_works() {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("notes.txt");
        std::fs::write(&notes, "text").unwrap();

        rename_without_replacing(&notes, &dir.path().join("Notes.txt")).unwrap();
        assert_eq!(names(dir.path()), ["Notes.txt"], "the new spelling is what the directory shows");
        assert_eq!(std::fs::read_to_string(dir.path().join("Notes.txt")).unwrap(), "text");

        std::fs::write(dir.path().join("other.txt"), "x").unwrap();
        let err = rename_without_replacing(&dir.path().join("other.txt"), &dir.path().join("NOTES.TXT")).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists, "{err}");
    }

    #[tokio::test]
    async fn the_commands_report_conflicts_as_conflicts() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        std::fs::write(&a, "1").unwrap();
        std::fs::write(&b, "2").unwrap();
        let err = local_rename(a.display().to_string(), b.display().to_string()).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::Conflict);
        local_mkdir(dir.path().join("sub").display().to_string()).await.unwrap();
        assert!(local_exists(dir.path().join("sub").display().to_string()).await);
        let again = local_mkdir(dir.path().join("sub").display().to_string()).await.unwrap_err();
        assert_eq!(again.code, ErrorCode::Conflict);
        assert_eq!(names(dir.path()), ["a", "b", "sub"]);
    }
}
