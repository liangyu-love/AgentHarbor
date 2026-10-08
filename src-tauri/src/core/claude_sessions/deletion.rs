use super::*;
use crate::core::{storage, thread_delete::ThreadDeleteResult};

/// Stage whole sessions on the same volume before removing them from the
/// catalog. If any rename fails, restore every already staged path.
pub fn delete_threads(root: &Path, keys: &[String]) -> Result<ThreadDeleteResult, CoreError> {
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK
        .lock()
        .map_err(|e| CoreError::OperationFailed(e.to_string()))?;
    let unique: HashSet<_> = keys.iter().collect();
    if unique.is_empty() || unique.len() > MAX_FILES {
        return Err(CoreError::InvalidData(
            "Select between 1 and 10000 Claude sessions".into(),
        ));
    }
    // Resolve the entire batch before changing a file. This reuses the reader's
    // identity, traversal and linked-path checks.
    let mut files: Vec<_> = unique
        .into_iter()
        .map(|key| resolve(root, key, None))
        .collect::<Result<_, _>>()?;
    files.sort();
    let mut moves = Vec::new();
    for path in &files {
        if !fs::symlink_metadata(path)?.is_file() {
            return Err(CoreError::InvalidData(
                "Claude transcript is not a file".into(),
            ));
        }
        moves.push(path.clone());
        let sidecar = path.with_extension("");
        match fs::symlink_metadata(&sidecar) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(CoreError::InvalidData(
                    "Linked Claude sidecar is unsupported".into(),
                ))
            }
            Ok(_) => moves.push(sidecar),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    // Nested batches cannot be mistaken for projects/<project>/<UUID>.jsonl.
    let backups = root.join(".agentharbor-deleted");
    if fs::symlink_metadata(&backups).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err(CoreError::InvalidData(
            "Linked Claude backup root is unsupported".into(),
        ));
    }
    let backup = backups.join(format!(
        "{}-{}",
        crate::core::auth::current_timestamp(),
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&backup)?;
    storage::write_atomic(
        &backup.join("manifest.json"),
        &serde_json::to_vec_pretty(&serde_json::json!({
            "source": "claudeCode", "keys": keys,
        }))?,
    )?;
    let mut staged: Vec<(PathBuf, PathBuf)> = Vec::new();
    let result = (|| {
        for source in moves {
            let destination = backup.join(source.strip_prefix(root).unwrap());
            fs::create_dir_all(destination.parent().unwrap())?;
            fs::rename(&source, &destination)?;
            staged.push((source, destination));
        }
        storage::write_atomic(&backup.join("completed"), b"claudeCode")?;
        Ok::<_, CoreError>(())
    })();
    if let Err(error) = result {
        let mut failures = Vec::new();
        for (source, destination) in staged.iter().rev() {
            if let Err(restore) = fs::rename(destination, source) {
                failures.push(format!("{}: {restore}", destination.display()));
            }
        }
        if !failures.is_empty() {
            return Err(CoreError::OperationFailed(format!(
                "{error}; restore from {}: {}",
                backup.display(),
                failures.join("; ")
            )));
        }
        return Err(error);
    }
    if let Ok(mut cache) = summary_cache().lock() {
        for file in &files {
            cache.remove(file);
        }
    }
    if let Err(error) = prune_backups(&backups, &backup) {
        eprintln!("Claude session backup cleanup failed: {error}");
    }
    Ok(ThreadDeleteResult {
        deleted: files.len(),
        queued: 0,
        catalog_rows_deleted: 0,
        rollout_files_deleted: files.len(),
        backup_path: backup.display().to_string(),
    })
}

fn prune_backups(root: &Path, current: &Path) -> Result<(), CoreError> {
    let canonical = root.canonicalize()?;
    let mut completed = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let valid = name.split_once('-').is_some_and(|(time, id)| {
            time.parse::<i64>().is_ok() && uuid::Uuid::parse_str(id).is_ok()
        });
        if kind.is_dir()
            && !kind.is_symlink()
            && valid
            && path.join("completed").is_file()
            && path.canonicalize()?.starts_with(&canonical)
        {
            completed.push((entry.metadata()?.modified()?, path));
        }
    }
    completed.sort_by_key(|(modified, path)| {
        (
            std::cmp::Reverse(path == current),
            std::cmp::Reverse(*modified),
        )
    });
    for (_, path) in completed.into_iter().skip(10) {
        fs::remove_dir_all(path)?;
    }
    Ok(())
}
