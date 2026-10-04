use crate::core::models::CoreError;
use std::io::Write;
use std::path::Path;

#[cfg(target_os = "windows")]
fn replace_staged_file(staged: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let staged: Vec<u16> = staged.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    let result = unsafe {
        MoveFileExW(
            staged.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
fn replace_staged_file(staged: &Path, target: &Path) -> std::io::Result<()> {
    std::fs::rename(staged, target)
}

/// Replace a file only after its complete contents have been written.
pub fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), CoreError> {
    let parent = path
        .parent()
        .ok_or_else(|| CoreError::InvalidData("Missing parent directory".into()))?;
    std::fs::create_dir_all(parent)?;
    let staged = parent.join(format!(".agentharbor-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<(), CoreError> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&staged)?;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        replace_staged_file(&staged, path)?;
        #[cfg(unix)]
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(staged);
    }
    result
}

pub fn remove_file_durable(path: &Path) -> Result<(), CoreError> {
    match std::fs::remove_file(path) {
        Ok(()) => {
            #[cfg(unix)]
            if let Some(parent) = path.parent() {
                std::fs::File::open(parent)?.sync_all()?;
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Keep only the `keep` most recently modified entries of `dir` that match `owned`,
/// removing older files or directories. Best effort: a failure to prune never fails
/// the operation that just created a new backup.
pub fn prune_backups(dir: &Path, keep: usize, owned: impl Fn(&Path) -> bool) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut backups: Vec<(std::time::SystemTime, std::path::PathBuf)> = entries
        .filter_map(Result::ok)
        .filter(|entry| owned(&entry.path()))
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            Some((metadata.modified().ok()?, entry.path()))
        })
        .collect();
    if backups.len() <= keep {
        return;
    }
    // Newest first; names break mtime ties (timestamped names sort chronologically).
    backups.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
    for (_, path) in backups.into_iter().skip(keep) {
        let result = if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        if let Err(error) = result {
            log::warn!("could not prune backup {}: {error}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_keeps_newest_owned_entries_only() {
        let temp = tempfile::tempdir().unwrap();
        for index in 0..5 {
            let path = temp.path().join(format!("backup-{index}.dat"));
            std::fs::write(&path, b"x").unwrap();
            let time =
                std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000 + index);
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(time)
                .unwrap();
        }
        std::fs::write(temp.path().join("keep-me.txt"), b"x").unwrap();
        prune_backups(temp.path(), 2, |path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("backup-"))
        });
        let mut left: Vec<String> = std::fs::read_dir(temp.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["backup-3.dat", "backup-4.dat", "keep-me.txt"]);
    }

    #[test]
    fn write_atomic_replaces_an_existing_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state.json");
        std::fs::write(&path, b"old").unwrap();
        write_atomic(&path, b"new").unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"new");
    }

    #[test]
    fn remove_file_durable_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("journal.json");
        std::fs::write(&path, b"pending").unwrap();
        remove_file_durable(&path).unwrap();
        remove_file_durable(&path).unwrap();
        assert!(!path.exists());
    }
}
