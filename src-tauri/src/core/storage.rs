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

#[cfg(test)]
mod tests {
    use super::*;

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
