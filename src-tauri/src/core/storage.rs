use crate::core::models::CoreError;
use std::io::Write;
use std::path::Path;

/// Replace a file only after its complete contents have been written.
pub fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), CoreError> {
    let parent = path.parent().ok_or_else(|| CoreError::InvalidData("Missing parent directory".into()))?;
    std::fs::create_dir_all(parent)?;
    let staged = parent.join(format!(".aimami-{}.tmp", uuid::Uuid::new_v4()));
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
        std::fs::rename(&staged, path)?;
        Ok(())
    })();
    if result.is_err() { let _ = std::fs::remove_file(staged); }
    result
}
