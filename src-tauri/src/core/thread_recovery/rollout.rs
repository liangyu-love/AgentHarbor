use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RolloutLookup {
    Indexed,
    Refresh,
}

pub(super) fn select_rollout(
    paths: &CodexPaths,
    database: Option<&DatabaseThread>,
    thread_id: &str,
    lookup: RolloutLookup,
) -> Result<Option<(PathBuf, RolloutSnapshot)>, CoreError> {
    if lookup == RolloutLookup::Refresh {
        return find_rollout(paths, thread_id);
    }
    if let Some(catalog_path) = database.and_then(|entry| entry.rollout_path.as_deref()) {
        if catalog_path.is_file() {
            let path = validate_rollout_path(paths, catalog_path)?;
            let snapshot = read_rollout(&path, thread_id)?;
            if snapshot.has_session_meta {
                return Ok(Some((path, snapshot)));
            }
        }
    }
    if let Some(paths_by_id) = super::super::threads::indexed_rollout_paths(
        &paths.sessions_dir,
        &paths.archived_sessions_dir,
        thread_id,
    ) {
        if !paths_by_id.is_empty() {
            match read_rollout_candidates(paths, thread_id, paths_by_id) {
                Ok(Some(found)) => return Ok(Some(found)),
                Ok(None) | Err(_) => {}
            }
        }
    }
    find_rollout(paths, thread_id)
}

fn read_rollout_candidates(
    paths: &CodexPaths,
    thread_id: &str,
    candidates: Vec<PathBuf>,
) -> Result<Option<(PathBuf, RolloutSnapshot)>, CoreError> {
    let mut matches = Vec::new();
    for candidate in candidates {
        if !candidate.is_file() {
            return Ok(None);
        }
        let path = validate_rollout_path(paths, &candidate)?;
        let snapshot = read_rollout(&path, thread_id)?;
        if snapshot.has_session_meta {
            matches.push((path, snapshot));
        }
    }
    select_unique_rollout(matches, thread_id)
}

pub(super) fn find_rollout(
    paths: &CodexPaths,
    thread_id: &str,
) -> Result<Option<(PathBuf, RolloutSnapshot)>, CoreError> {
    let mut matches = Vec::new();
    for root in [&paths.sessions_dir, &paths.archived_sessions_dir] {
        if !root.is_dir() {
            continue;
        }
        let mut stack = vec![root.clone()];
        while let Some(directory) = stack.pop() {
            for entry in std::fs::read_dir(directory)? {
                let entry = entry?;
                let file_type = entry.file_type()?;
                if file_type.is_symlink() {
                    continue;
                }
                if file_type.is_dir() {
                    stack.push(entry.path());
                    continue;
                }
                let path = entry.path();
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("");
                if !file_type.is_file()
                    || !name.starts_with("rollout-")
                    || !name.ends_with(".jsonl")
                    || !name.contains(thread_id)
                {
                    continue;
                }
                let path = validate_rollout_path(paths, &path)?;
                let snapshot = read_rollout(&path, thread_id)?;
                if snapshot.has_session_meta {
                    matches.push((path, snapshot));
                }
            }
        }
    }
    select_unique_rollout(matches, thread_id)
}

fn select_unique_rollout(
    mut matches: Vec<(PathBuf, RolloutSnapshot)>,
    thread_id: &str,
) -> Result<Option<(PathBuf, RolloutSnapshot)>, CoreError> {
    match matches.len() {
        0 => Ok(None),
        1 => Ok(matches.pop()),
        count => Err(CoreError::OperationFailed(format!(
            "THREAD_RECOVERY_MULTIPLE_ROLLOUTS: found {count} rollout files for thread {thread_id}"
        ))),
    }
}

pub(super) fn validate_rollout_path(paths: &CodexPaths, path: &Path) -> Result<PathBuf, CoreError> {
    let resolved = std::fs::canonicalize(path)?;
    let allowed = [&paths.sessions_dir, &paths.archived_sessions_dir]
        .iter()
        .filter_map(|root| std::fs::canonicalize(root).ok())
        .any(|root| resolved.starts_with(root));
    if !allowed {
        return Err(CoreError::InvalidData(format!(
            "Thread rollout is outside the Codex session directories: {}",
            path.display()
        )));
    }
    Ok(resolved)
}

pub(super) fn read_rollout(path: &Path, thread_id: &str) -> Result<RolloutSnapshot, CoreError> {
    let bytes = std::fs::read(path)?;
    parse_rollout_bytes(bytes, thread_id)
}

pub(super) fn read_pending_rollout_snapshot(
    paths: &CodexPaths,
    journal: &RecoveryJournal,
) -> Result<(PathBuf, RolloutSnapshot), CoreError> {
    let backup_path = PathBuf::from(&journal.backup_path);
    let backup = load_backup(paths, &backup_path)?;
    if backup.thread_id != journal.thread_id
        || backup.config_generation != journal.config_generation
    {
        return Err(CoreError::InvalidData(
            "Recovery journal and backup metadata differ".into(),
        ));
    }
    validate_journal_mutation(journal, &backup)?;
    let path = validate_rollout_path(paths, Path::new(&backup.rollout_path))?;
    let bytes = std::fs::read(&path)?;
    let snapshot = match parse_rollout_bytes(bytes.clone(), &journal.thread_id) {
        Ok(snapshot) => snapshot,
        Err(error) if error.to_string().contains("duplicate session metadata") => {
            return Err(error);
        }
        Err(_) => RolloutSnapshot {
            provider: None,
            has_session_meta: false,
            sha256: sha256_hex(&bytes),
            length: bytes.len() as u64,
            bytes,
        },
    };
    Ok((path, snapshot))
}

pub(super) fn parse_rollout_bytes(
    bytes: Vec<u8>,
    thread_id: &str,
) -> Result<RolloutSnapshot, CoreError> {
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| CoreError::InvalidData("Thread rollout is not valid UTF-8".into()))?;
    let mut provider = None;
    let mut matching_session_meta = 0_u32;
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if is_matching_session_meta(&value, thread_id) {
            matching_session_meta += 1;
            if matching_session_meta > 1 {
                return Err(CoreError::InvalidData(format!(
                    "Thread rollout contains duplicate session metadata for {thread_id}"
                )));
            }
            provider = value
                .get("payload")
                .and_then(|payload| payload.get("model_provider"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
        }
    }
    let length = bytes.len() as u64;
    let sha256 = sha256_hex(&bytes);
    Ok(RolloutSnapshot {
        provider,
        has_session_meta: matching_session_meta == 1,
        bytes,
        sha256,
        length,
    })
}

#[cfg(test)]
pub(super) fn ensure_rollout_unchanged(
    path: &Path,
    expected: &RolloutSnapshot,
) -> Result<(), CoreError> {
    let bytes = std::fs::read(path)?;
    if bytes.len() as u64 != expected.length || sha256_hex(&bytes) != expected.sha256 {
        return Err(CoreError::OperationFailed(
            "THREAD_RECOVERY_CHANGED: Rollout changed after preview".into(),
        ));
    }
    Ok(())
}

pub(super) fn is_matching_session_meta(value: &serde_json::Value, thread_id: &str) -> bool {
    if value.get("type").and_then(serde_json::Value::as_str) != Some("session_meta") {
        return false;
    }
    value
        .get("payload")
        .and_then(|payload| payload.get("id").or_else(|| payload.get("session_id")))
        .and_then(serde_json::Value::as_str)
        == Some(thread_id)
}

pub(super) fn rewrite_rollout_provider(
    original: &[u8],
    thread_id: &str,
    target_provider: &str,
) -> Result<Vec<u8>, CoreError> {
    let text = std::str::from_utf8(original)
        .map_err(|_| CoreError::InvalidData("Thread rollout is not valid UTF-8".into()))?;
    let mut updated = String::with_capacity(text.len());
    let mut replaced = false;
    for segment in text.split_inclusive('\n') {
        let (line, ending) = segment
            .strip_suffix("\r\n")
            .map(|line| (line, "\r\n"))
            .or_else(|| segment.strip_suffix('\n').map(|line| (line, "\n")))
            .unwrap_or((segment, ""));
        if let Ok(mut value) = serde_json::from_str::<serde_json::Value>(line) {
            if is_matching_session_meta(&value, thread_id) {
                if replaced {
                    return Err(CoreError::InvalidData(format!(
                        "Thread rollout contains duplicate session metadata for {thread_id}"
                    )));
                }
                let payload = value
                    .get_mut("payload")
                    .and_then(serde_json::Value::as_object_mut)
                    .ok_or_else(|| {
                        CoreError::InvalidData("Invalid rollout session metadata".into())
                    })?;
                payload.insert(
                    "model_provider".into(),
                    serde_json::Value::String(target_provider.to_owned()),
                );
                updated.push_str(&serde_json::to_string(&value)?);
                updated.push_str(ending);
                replaced = true;
                continue;
            }
        }
        updated.push_str(line);
        updated.push_str(ending);
    }
    if !replaced {
        return Err(CoreError::InvalidData(
            "No matching session metadata in rollout".into(),
        ));
    }
    Ok(updated.into_bytes())
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
