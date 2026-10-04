use super::{models::CoreError, storage, threads};
use chrono::Utc;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;

/// Each backup holds a full catalog database copy plus the deleted rollouts.
const DELETE_BACKUP_LIMIT: usize = 10;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadDeleteResult {
    pub deleted: usize,
    pub queued: usize,
    pub catalog_rows_deleted: usize,
    pub rollout_files_deleted: usize,
    pub backup_path: String,
}

#[derive(Debug, Default)]
pub struct PendingDeleteStatus {
    pub keys: Vec<String>,
    pub warnings: Vec<String>,
    pub recovery_required: bool,
}

#[derive(Debug, Default)]
struct PendingJobScan {
    jobs: Vec<(PathBuf, PendingDelete)>,
    invalid_paths: Vec<PathBuf>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PendingDelete {
    schema_version: u32,
    entries: Vec<PendingDeleteEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PendingDeleteEntry {
    key: String,
    thread_id: String,
    rollout_sha256: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeleteManifest {
    schema_version: u32,
    created_at: String,
    database_backup: Option<String>,
    entries: Vec<DeleteManifestEntry>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeleteManifestEntry {
    key: String,
    thread_id: String,
    catalog_row: bool,
    original_rollout_path: Option<String>,
    backup_rollout_path: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct RolloutBackup {
    original: PathBuf,
    backup: PathBuf,
}

/// Written before any rollout is removed and deleted once the deletion settles, so a
/// crash between removing rollouts and committing the catalog can be rolled back.
#[derive(Debug, Serialize, Deserialize)]
struct DeleteJournal {
    schema_version: u32,
    catalog_ids: Vec<String>,
    rollouts: Vec<RolloutBackup>,
}

pub fn delete_threads(
    state_database_path: &Path,
    sessions_dir: &Path,
    archived_dir: &Path,
    keys: &[String],
) -> Result<ThreadDeleteResult, CoreError> {
    let _guard = super::thread_recovery::thread_mutation_guard()?;
    // Settle an interrupted deletion before its backup can be pruned by this one.
    for warning in recover_journals(state_database_path) {
        log::warn!("{warning}");
    }
    if let Err(error) = crate::platform::process::ensure_no_codex_writer_processes() {
        if error.to_string().contains("CODEX_WRITER_RUNNING") {
            return queue_delete(state_database_path, sessions_dir, archived_dir, keys);
        }
        return Err(error);
    }
    delete_threads_inner(state_database_path, sessions_dir, archived_dir, keys)
}

fn pending_dir(state_database_path: &Path) -> PathBuf {
    state_database_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("thread-delete-queue")
}

fn journal_dir(state_database_path: &Path) -> PathBuf {
    state_database_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("thread-delete-journal")
}

/// Rolls back or finalizes deletions that were interrupted by a crash. Returns warnings
/// for journals that could not be settled; those are kept and retried next time.
fn recover_journals(state_database_path: &Path) -> Vec<String> {
    let mut warnings = Vec::new();
    let Ok(entries) = std::fs::read_dir(journal_dir(state_database_path)) else {
        return warnings;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "json") || !path.is_file() {
            continue;
        }
        let result = std::fs::read(&path)
            .map_err(CoreError::from)
            .and_then(|bytes| {
                serde_json::from_slice::<DeleteJournal>(&bytes)
                    .map_err(|error| CoreError::InvalidData(error.to_string()))
            })
            .and_then(|journal| settle_journal(state_database_path, &path, &journal));
        if let Err(error) = result {
            warnings.push(format!(
                "interrupted thread deletion could not be recovered ({}): {error}",
                path.display()
            ));
        }
    }
    warnings
}

fn has_journals(state_database_path: &Path) -> bool {
    std::fs::read_dir(journal_dir(state_database_path)).is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "json")
        })
    })
}

fn settle_journal(
    state_database_path: &Path,
    path: &Path,
    journal: &DeleteJournal,
) -> Result<(), CoreError> {
    if journal.schema_version != 1 {
        return Err(CoreError::InvalidData("unsupported journal version".into()));
    }
    // The catalog rows are deleted in one transaction, so their presence tells whether the
    // deletion committed. Without catalog rows only a complete file removal counts as done.
    let committed = if journal.catalog_ids.is_empty() {
        journal.rollouts.iter().all(|item| !item.original.exists())
    } else {
        !catalog_has_any(state_database_path, &journal.catalog_ids)?
    };
    if !committed {
        for item in &journal.rollouts {
            if item.original.exists() {
                continue;
            }
            if let Some(parent) = item.original.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&item.backup, &item.original)?;
        }
    }
    storage::remove_file_durable(path)
}

fn catalog_has_any(state_database_path: &Path, thread_ids: &[String]) -> Result<bool, CoreError> {
    if !state_database_path.is_file() {
        // Cannot prove the commit happened; restoring the rollouts keeps the data.
        return Ok(true);
    }
    let connection = Connection::open_with_flags(
        state_database_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(sql_error)?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(sql_error)?;
    for thread_id in thread_ids {
        let found = connection
            .query_row(
                "SELECT 1 FROM threads WHERE id = ?1",
                params![thread_id],
                |_| Ok(()),
            )
            .optional()
            .map_err(sql_error)?;
        if found.is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn pending_jobs(state_database_path: &Path) -> Result<PendingJobScan, CoreError> {
    let directory = pending_dir(state_database_path);
    if !directory.is_dir() {
        return Ok(PendingJobScan::default());
    }
    let mut scan = PendingJobScan::default();
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "json") || !path.is_file() {
            continue;
        }
        let parsed = (|| -> Result<PendingDelete, String> {
            if entry.metadata().map_err(|error| error.to_string())?.len() > 1024 * 1024 {
                return Err("entry is too large".into());
            }
            let job = serde_json::from_slice::<PendingDelete>(
                &std::fs::read(&path).map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            if job.schema_version != 1 || job.entries.is_empty() {
                return Err("unsupported or empty entry".into());
            }
            Ok(job)
        })();
        match parsed {
            Ok(job) => scan.jobs.push((path, job)),
            // Surfaced through `recovery_required`; the UI shows a dedicated recovery banner.
            Err(_) => scan.invalid_paths.push(path),
        }
    }
    scan.jobs.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(scan)
}

fn file_sha256(path: &Path) -> Result<String, CoreError> {
    let before = std::fs::metadata(path)?;
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let after = std::fs::metadata(path)?;
    if before.len() != after.len() || before.modified()? != after.modified()? {
        return Err(CoreError::OperationFailed(
            "THREAD_DELETE_CHANGED: rollout changed while preparing deletion".into(),
        ));
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn queue_delete(
    state_database_path: &Path,
    sessions_dir: &Path,
    archived_dir: &Path,
    keys: &[String],
) -> Result<ThreadDeleteResult, CoreError> {
    if keys.is_empty() || keys.iter().collect::<HashSet<_>>().len() != keys.len() {
        return Err(CoreError::InvalidData(
            "select distinct threads to delete".into(),
        ));
    }
    let jobs = pending_jobs(state_database_path)?.jobs;
    let existing: HashSet<&str> = jobs
        .iter()
        .flat_map(|(_, job)| job.entries.iter().map(|entry| entry.key.as_str()))
        .collect();
    if keys.iter().any(|key| existing.contains(key.as_str())) {
        return Err(CoreError::InvalidData(
            "THREAD_DELETE_ALREADY_QUEUED: one or more selected threads are already queued".into(),
        ));
    }
    let scan = threads::scan_threads_from_catalog_with_warnings(
        state_database_path,
        sessions_dir,
        archived_dir,
    )?;
    let mut rollout_counts: HashMap<String, usize> = HashMap::new();
    for item in threads::scan_threads(sessions_dir, archived_dir)? {
        *rollout_counts.entry(item.id).or_default() += 1;
    }
    let mut entries = Vec::with_capacity(keys.len());
    let mut seen_ids = HashSet::new();
    for key in keys {
        let matches: Vec<_> = scan.items.iter().filter(|item| item.key == *key).collect();
        if matches.len() != 1 || !seen_ids.insert(matches[0].id.as_str()) {
            return Err(CoreError::NotFound(
                "selected threads changed; refresh and try again".into(),
            ));
        }
        let item = matches[0];
        if rollout_counts.get(&item.id).copied().unwrap_or_default() > 1 {
            return Err(CoreError::InvalidData(format!(
                "thread {} has multiple rollout files; remove the duplicate before deleting",
                item.id
            )));
        }
        let rollout_sha256 = if item.rollout_status == "available" {
            let path = validate_rollout(item, sessions_dir, archived_dir)?;
            Some(file_sha256(&path)?)
        } else {
            None
        };
        entries.push(PendingDeleteEntry {
            key: key.clone(),
            thread_id: item.id.clone(),
            rollout_sha256,
        });
    }
    let job = PendingDelete {
        schema_version: 1,
        entries,
    };
    let path = pending_dir(state_database_path).join(format!("{}.json", Uuid::new_v4()));
    storage::write_atomic(
        &path,
        &serde_json::to_vec(&job).map_err(|error| CoreError::OperationFailed(error.to_string()))?,
    )?;
    Ok(ThreadDeleteResult {
        deleted: 0,
        queued: keys.len(),
        catalog_rows_deleted: 0,
        rollout_files_deleted: 0,
        backup_path: String::new(),
    })
}

fn pending_matches(
    job: &PendingDelete,
    state_database_path: &Path,
    sessions_dir: &Path,
    archived_dir: &Path,
) -> Result<(Vec<String>, Vec<String>), CoreError> {
    let scan = threads::scan_threads_from_catalog_with_warnings(
        state_database_path,
        sessions_dir,
        archived_dir,
    )?;
    let (mut unchanged, mut changed) = (Vec::new(), Vec::new());
    for entry in &job.entries {
        let matches = match scan.items.iter().find(|item| item.key == entry.key) {
            Some(item) if item.id == entry.thread_id => match &entry.rollout_sha256 {
                Some(expected) if item.rollout_status == "available" => {
                    let path = validate_rollout(item, sessions_dir, archived_dir)?;
                    &file_sha256(&path)? == expected
                }
                None => item.rollout_status == "missing",
                _ => false,
            },
            _ => false,
        };
        if matches {
            unchanged.push(entry.key.clone());
        } else {
            changed.push(entry.key.clone());
        }
    }
    Ok((unchanged, changed))
}

fn process_pending_deletes_inner(
    state_database_path: &Path,
    sessions_dir: &Path,
    archived_dir: &Path,
    jobs: Vec<(PathBuf, PendingDelete)>,
    can_delete: bool,
) -> PendingDeleteStatus {
    let mut status = PendingDeleteStatus::default();
    for (path, job) in jobs {
        if can_delete {
            match pending_matches(&job, state_database_path, sessions_dir, archived_dir) {
                Ok((unchanged, changed)) => {
                    // A thread that changed since it was queued is kept; the rest of
                    // the same request still goes ahead.
                    if !changed.is_empty() {
                        status.warnings.push(format!(
                            "{} queued thread(s) changed and were kept: {}",
                            changed.len(),
                            changed.join(", ")
                        ));
                    }
                    let result = if unchanged.is_empty() {
                        Ok(())
                    } else {
                        delete_threads_inner(
                            state_database_path,
                            sessions_dir,
                            archived_dir,
                            &unchanged,
                        )
                        .map(|_| ())
                    };
                    match result {
                        Ok(()) => {
                            if let Err(error) = std::fs::remove_file(&path) {
                                status.warnings.push(error.to_string());
                                status.keys.extend(unchanged);
                            }
                            continue;
                        }
                        Err(error) => status.warnings.push(error.to_string()),
                    }
                }
                Err(error) => status.warnings.push(error.to_string()),
            }
        }
        status
            .keys
            .extend(job.entries.into_iter().map(|entry| entry.key));
    }
    status
}

pub fn process_pending_deletes(
    state_database_path: &Path,
    sessions_dir: &Path,
    archived_dir: &Path,
) -> PendingDeleteStatus {
    let _guard = match super::thread_recovery::thread_mutation_guard() {
        Ok(guard) => guard,
        Err(error) => {
            return PendingDeleteStatus {
                warnings: vec![error.to_string()],
                ..Default::default()
            }
        }
    };
    let journal_warnings = recover_journals(state_database_path);
    let scan = match pending_jobs(state_database_path) {
        Ok(scan) => scan,
        Err(error) => {
            let mut warnings = journal_warnings;
            warnings.push(error.to_string());
            return PendingDeleteStatus {
                warnings,
                recovery_required: true,
                ..Default::default()
            };
        }
    };
    if scan.jobs.is_empty() {
        return PendingDeleteStatus {
            warnings: journal_warnings,
            recovery_required: !scan.invalid_paths.is_empty(),
            ..Default::default()
        };
    }
    let writer_check = crate::platform::process::ensure_no_codex_writer_processes();
    let warning = writer_check.as_ref().err().and_then(|error| {
        (!error.to_string().contains("CODEX_WRITER_RUNNING")).then(|| error.to_string())
    });
    let mut status = process_pending_deletes_inner(
        state_database_path,
        sessions_dir,
        archived_dir,
        scan.jobs,
        writer_check.is_ok(),
    );
    status.recovery_required = !scan.invalid_paths.is_empty();
    status.warnings.splice(0..0, journal_warnings);
    if let Some(warning) = warning {
        status.warnings.push(warning);
    }
    status
}

pub fn cancel_pending_deletes(state_database_path: &Path) -> Result<usize, CoreError> {
    let _guard = super::thread_recovery::thread_mutation_guard()?;
    let directory = pending_dir(state_database_path);
    if !directory.is_dir() {
        return Ok(0);
    }
    let mut cancelled = 0;
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
            && path.is_file()
        {
            std::fs::remove_file(path)?;
            cancelled += 1;
        }
    }
    Ok(cancelled)
}

fn delete_threads_inner(
    state_database_path: &Path,
    sessions_dir: &Path,
    archived_dir: &Path,
    keys: &[String],
) -> Result<ThreadDeleteResult, CoreError> {
    let requested: HashSet<&str> = keys.iter().map(String::as_str).collect();
    if requested.is_empty() {
        return Err(CoreError::InvalidData("no threads selected".into()));
    }
    if requested.len() != keys.len() {
        return Err(CoreError::InvalidData(
            "duplicate thread keys are not allowed".into(),
        ));
    }

    let scan = threads::scan_threads_from_catalog_with_warnings(
        state_database_path,
        sessions_dir,
        archived_dir,
    )?;
    let mut matches: HashMap<&str, Vec<&threads::ThreadSummary>> = HashMap::new();
    for item in &scan.items {
        if requested.contains(item.key.as_str()) {
            matches.entry(item.key.as_str()).or_default().push(item);
        }
    }
    if matches.len() != requested.len() || matches.values().any(|items| items.len() != 1) {
        return Err(CoreError::NotFound(
            "one or more selected threads changed; refresh and try again".into(),
        ));
    }

    let mut selected = Vec::with_capacity(keys.len());
    let mut selected_ids = HashSet::new();
    for key in keys {
        let item = matches
            .get(key.as_str())
            .and_then(|items| items.first())
            .copied()
            .ok_or_else(|| {
                CoreError::NotFound(
                    "one or more selected threads changed; refresh and try again".into(),
                )
            })?;
        if !selected_ids.insert(item.id.as_str()) {
            return Err(CoreError::InvalidData(
                "multiple selected entries share a thread ID; refresh and delete them separately"
                    .into(),
            ));
        }
        selected.push(item);
    }

    let mut rollout_counts: HashMap<String, usize> = HashMap::new();
    for item in threads::scan_threads(sessions_dir, archived_dir)? {
        *rollout_counts.entry(item.id).or_default() += 1;
    }
    for item in &selected {
        if rollout_counts.get(&item.id).copied().unwrap_or_default() > 1 {
            return Err(CoreError::InvalidData(format!(
                "thread {} has multiple rollout files; remove the duplicate before deleting",
                item.id
            )));
        }
    }

    let backup_root = state_database_path
        .parent()
        .or_else(|| sessions_dir.parent())
        .unwrap_or(sessions_dir)
        .join("thread-delete-backups");
    let backup_dir = backup_root.join(format!(
        "{}-{}",
        Utc::now().format("%Y%m%d-%H%M%S"),
        Uuid::new_v4()
    ));
    std::fs::create_dir_all(&backup_dir)?;
    // Every deletion snapshots the whole catalog database; keep the latest few only.
    // An unsettled journal still needs its backup, so pruning waits until it is recovered.
    if !has_journals(state_database_path) {
        storage::prune_backups(&backup_root, DELETE_BACKUP_LIMIT, |path| path.is_dir());
    }

    let mut manifest_entries = Vec::with_capacity(selected.len());
    let mut rollout_backups = Vec::new();
    let mut catalog_ids = Vec::new();
    for (index, item) in selected.iter().enumerate() {
        let catalog_row = item.thread_source == "sqlite";
        if catalog_row {
            catalog_ids.push(item.id.clone());
        }
        let (original_rollout_path, backup_rollout_path) = if item.rollout_status == "available" {
            let original = validate_rollout(item, sessions_dir, archived_dir)?;
            let file_name = original
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("rollout.jsonl");
            let backup = backup_dir.join(format!("rollout-{index}-{file_name}"));
            std::fs::copy(&original, &backup)?;
            rollout_backups.push(RolloutBackup {
                original: original.clone(),
                backup: backup.clone(),
            });
            (
                Some(original.display().to_string()),
                Some(backup.display().to_string()),
            )
        } else {
            (None, None)
        };
        manifest_entries.push(DeleteManifestEntry {
            key: item.key.clone(),
            thread_id: item.id.clone(),
            catalog_row,
            original_rollout_path,
            backup_rollout_path,
        });
    }

    let mut connection = if catalog_ids.is_empty() {
        None
    } else {
        if !state_database_path.is_file() {
            return Err(CoreError::NotFound(format!(
                "thread catalog does not exist: {}",
                state_database_path.display()
            )));
        }
        let connection = Connection::open_with_flags(
            state_database_path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(sql_error)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(sql_error)?;
        connection
            .execute_batch("PRAGMA foreign_keys=ON")
            .map_err(sql_error)?;
        Some(connection)
    };

    let database_backup = if let Some(connection) = connection.as_ref() {
        let path = backup_dir.join("state.sqlite");
        let escaped = path.to_string_lossy().replace('\'', "''");
        connection
            .execute_batch(&format!("VACUUM INTO '{escaped}'"))
            .map_err(sql_error)?;
        Some(path.display().to_string())
    } else {
        None
    };

    let manifest = DeleteManifest {
        schema_version: 1,
        created_at: Utc::now().to_rfc3339(),
        database_backup,
        entries: manifest_entries,
    };
    std::fs::write(
        backup_dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)
            .map_err(|error| CoreError::OperationFailed(error.to_string()))?,
    )?;

    let journal = DeleteJournal {
        schema_version: 1,
        catalog_ids: catalog_ids.clone(),
        rollouts: rollout_backups,
    };
    let journal_path = journal_dir(state_database_path).join(format!("{}.json", Uuid::new_v4()));
    storage::write_atomic(
        &journal_path,
        &serde_json::to_vec(&journal)
            .map_err(|error| CoreError::OperationFailed(error.to_string()))?,
    )?;
    let outcome = remove_and_commit(connection.as_mut(), &catalog_ids, &journal.rollouts);
    // On failure the rollouts were already put back; settling re-checks that from disk and
    // keeps the journal for the next launch if anything is still missing.
    let settled = match &outcome {
        Ok(_) => storage::remove_file_durable(&journal_path),
        Err(_) => settle_journal(state_database_path, &journal_path, &journal),
    };
    if let Err(error) = settled {
        log::warn!(
            "thread deletion journal was left for recovery ({}): {error}",
            journal_path.display()
        );
    }
    let removed_files = outcome?;

    let _ = threads::scan_threads_with_warnings(sessions_dir, archived_dir);
    Ok(ThreadDeleteResult {
        deleted: selected.len(),
        queued: 0,
        catalog_rows_deleted: catalog_ids.len(),
        rollout_files_deleted: removed_files.len(),
        backup_path: backup_dir.display().to_string(),
    })
}

fn remove_and_commit(
    connection: Option<&mut Connection>,
    catalog_ids: &[String],
    rollout_backups: &[RolloutBackup],
) -> Result<Vec<usize>, CoreError> {
    if let Some(connection) = connection {
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        validate_catalog_rows(&transaction, catalog_ids)?;
        let removed = remove_rollouts(rollout_backups)?;
        let mutation = (|| -> Result<usize, CoreError> {
            let mut deleted = 0;
            for thread_id in catalog_ids {
                let changed = transaction
                    .execute("DELETE FROM threads WHERE id = ?1", params![thread_id])
                    .map_err(sql_error)?;
                if changed != 1 {
                    return Err(CoreError::OperationFailed(format!(
                        "thread {thread_id} changed during deletion"
                    )));
                }
                deleted += changed;
            }
            Ok(deleted)
        })();
        let catalog_deleted = match mutation {
            Ok(value) => value,
            Err(error) => {
                restore_rollouts(rollout_backups, &removed);
                return Err(error);
            }
        };
        if let Err(error) = transaction.commit() {
            restore_rollouts(rollout_backups, &removed);
            return Err(sql_error(error));
        }
        debug_assert_eq!(catalog_deleted, catalog_ids.len());
        Ok(removed)
    } else {
        remove_rollouts(rollout_backups)
    }
}

fn validate_rollout(
    item: &threads::ThreadSummary,
    sessions_dir: &Path,
    archived_dir: &Path,
) -> Result<PathBuf, CoreError> {
    let original = PathBuf::from(&item.file_path);
    let metadata = std::fs::symlink_metadata(&original)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(CoreError::InvalidData(format!(
            "rollout is not a regular file: {}",
            original.display()
        )));
    }
    let canonical = std::fs::canonicalize(&original)?;
    let mut expected_key = None;
    for (scope, root) in [("active", sessions_dir), ("archived", archived_dir)] {
        let Ok(root) = std::fs::canonicalize(root) else {
            continue;
        };
        if let Ok(relative) = canonical.strip_prefix(&root) {
            expected_key = Some(format!("{scope}:{}", normalize_relative_path(relative)));
            break;
        }
    }
    if expected_key.as_deref() != Some(item.key.as_str()) {
        return Err(CoreError::InvalidData(
            "rollout path no longer matches the selected thread key".into(),
        ));
    }
    let file_name = canonical
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if !file_name.starts_with("rollout-") || !file_name.ends_with(".jsonl") {
        return Err(CoreError::InvalidData(
            "selected file is not a rollout".into(),
        ));
    }
    let rollout_id = read_rollout_id(&canonical)?;
    if rollout_id != item.id {
        return Err(CoreError::InvalidData(format!(
            "rollout thread ID changed from {} to {rollout_id}",
            item.id
        )));
    }
    Ok(canonical)
}

fn read_rollout_id(path: &Path) -> Result<String, CoreError> {
    let reader = BufReader::new(File::open(path)?);
    let mut session_id = None;
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        // Match the scanner: a truncated or corrupt line must not make a listed thread undeletable.
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if value.get("type").and_then(|value| value.as_str()) != Some("session_meta") {
            continue;
        }
        let id = value
            .get("payload")
            .and_then(|payload| payload.get("id"))
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| CoreError::InvalidData("session_meta has no thread ID".into()))?;
        if session_id.as_deref().is_some_and(|current| current != id) {
            return Err(CoreError::InvalidData(
                "rollout contains conflicting session IDs".into(),
            ));
        }
        session_id = Some(id.to_owned());
    }
    session_id.ok_or_else(|| CoreError::InvalidData("rollout has no session metadata".into()))
}

fn normalize_relative_path(path: &Path) -> String {
    path.components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn validate_catalog_rows(connection: &Connection, thread_ids: &[String]) -> Result<(), CoreError> {
    let columns = threads::table_columns(connection, "threads")?;
    if !columns.contains("id") {
        return Err(CoreError::InvalidData(
            "Codex thread catalog has no threads.id column".into(),
        ));
    }
    let mut references = declared_thread_references(connection)?;
    references.extend([
        (
            "thread_spawn_edges".to_owned(),
            "parent_thread_id".to_owned(),
        ),
        (
            "thread_spawn_edges".to_owned(),
            "child_thread_id".to_owned(),
        ),
        ("agent_job_items".to_owned(), "thread_id".to_owned()),
        (
            "agent_job_items".to_owned(),
            "assigned_thread_id".to_owned(),
        ),
    ]);
    references.sort();
    references.dedup();
    for thread_id in thread_ids {
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM threads WHERE id = ?1",
                params![thread_id],
                |row| row.get(0),
            )
            .map_err(sql_error)?;
        if count != 1 {
            return Err(CoreError::InvalidData(format!(
                "thread {thread_id} has {count} catalog rows; refresh and resolve the duplicate"
            )));
        }
        for (table, column) in &references {
            if table_has_value(connection, table, column, thread_id)? {
                return Err(CoreError::InvalidData(format!(
                    "thread {thread_id} is still referenced by {table}.{column}"
                )));
            }
        }
    }
    Ok(())
}

fn declared_thread_references(connection: &Connection) -> Result<Vec<(String, String)>, CoreError> {
    let mut statement = connection
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .map_err(sql_error)?;
    let tables = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(sql_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql_error)?;
    let mut references = Vec::new();
    for table in tables {
        let escaped = table.replace('"', "\"\"");
        let mut foreign_keys = connection
            .prepare(&format!("PRAGMA foreign_key_list(\"{escaped}\")"))
            .map_err(sql_error)?;
        let rows = foreign_keys
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })
            .map_err(sql_error)?;
        for row in rows {
            let (target_table, source_column, target_column) = row.map_err(sql_error)?;
            if target_table == "threads"
                && target_column.as_deref().is_none_or(|column| column == "id")
            {
                references.push((table.clone(), source_column));
            }
        }
    }
    Ok(references)
}

fn table_has_value(
    connection: &Connection,
    table: &str,
    column: &str,
    value: &str,
) -> Result<bool, CoreError> {
    if !threads::table_columns(connection, table)?.contains(column) {
        return Ok(false);
    }
    let sql = format!(
        "SELECT EXISTS(SELECT 1 FROM \"{}\" WHERE \"{}\" = ?1 LIMIT 1)",
        table.replace('"', "\"\""),
        column.replace('"', "\"\"")
    );
    connection
        .query_row(&sql, params![value], |row| row.get::<_, i64>(0))
        .map(|value| value != 0)
        .map_err(sql_error)
}

fn remove_rollouts(backups: &[RolloutBackup]) -> Result<Vec<usize>, CoreError> {
    let mut removed = Vec::new();
    for (index, item) in backups.iter().enumerate() {
        if let Err(error) = std::fs::remove_file(&item.original) {
            restore_rollouts(backups, &removed);
            return Err(error.into());
        }
        removed.push(index);
    }
    Ok(removed)
}

fn restore_rollouts(backups: &[RolloutBackup], removed: &[usize]) {
    for index in removed.iter().rev() {
        let item = &backups[*index];
        if let Some(parent) = item.original.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if !item.original.exists() {
            let _ = std::fs::copy(&item.backup, &item.original);
        }
    }
}

fn sql_error(error: rusqlite::Error) -> CoreError {
    CoreError::OperationFailed(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rollout(id: &str) -> String {
        format!(
            "{{\"timestamp\":\"2026-09-25T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"C:/work\",\"source\":\"cli\",\"cli_version\":\"1\",\"model_provider\":\"openai\"}}}}\n"
        )
    }

    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        let archived = temp.path().join("archived_sessions");
        let database = temp.path().join("state.sqlite");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::create_dir_all(&archived).unwrap();
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT, title TEXT, cwd TEXT);",
            )
            .unwrap();
        (temp, sessions, archived, database)
    }

    #[test]
    fn deletes_catalog_row_and_rollout_after_backing_them_up() {
        let (_temp, sessions, archived, database) = fixture();
        let path = sessions.join("rollout-one.jsonl");
        std::fs::write(&path, rollout("one")).unwrap();
        Connection::open(&database)
            .unwrap()
            .execute(
                "INSERT INTO threads (id, rollout_path, title, cwd) VALUES (?1, ?2, 'One', 'C:/work')",
                params!["one", path.display().to_string()],
            )
            .unwrap();
        let result = delete_threads_inner(
            &database,
            &sessions,
            &archived,
            &["active:rollout-one.jsonl".into()],
        )
        .unwrap();
        assert_eq!(result.deleted, 1);
        assert_eq!(result.catalog_rows_deleted, 1);
        assert_eq!(result.rollout_files_deleted, 1);
        assert!(!path.exists());
        assert!(Path::new(&result.backup_path)
            .join("manifest.json")
            .is_file());
        let count: i64 = Connection::open(&database)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM threads", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
        assert!(
            !has_journals(&database),
            "a finished deletion leaves no journal"
        );
    }

    /// Simulates a crash after the rollout was removed: writes the journal the way
    /// `delete_threads_inner` does and removes the original file.
    fn crashed_journal(database: &Path, original: &Path, catalog_ids: Vec<String>) -> PathBuf {
        let backup = database.parent().unwrap().join("backup-rollout.jsonl");
        std::fs::copy(original, &backup).unwrap();
        std::fs::remove_file(original).unwrap();
        let journal = DeleteJournal {
            schema_version: 1,
            catalog_ids,
            rollouts: vec![RolloutBackup {
                original: original.to_path_buf(),
                backup,
            }],
        };
        let path = journal_dir(database).join("crash.json");
        storage::write_atomic(&path, &serde_json::to_vec(&journal).unwrap()).unwrap();
        path
    }

    fn insert_thread(database: &Path, id: &str, path: &Path) {
        Connection::open(database)
            .unwrap()
            .execute(
                "INSERT INTO threads (id, rollout_path, title, cwd) VALUES (?1, ?2, 'T', 'C:/work')",
                params![id, path.display().to_string()],
            )
            .unwrap();
    }

    #[test]
    fn uncommitted_crash_restores_the_rollout() {
        let (_temp, sessions, _archived, database) = fixture();
        let path = sessions.join("rollout-one.jsonl");
        std::fs::write(&path, rollout("one")).unwrap();
        insert_thread(&database, "one", &path);
        let journal = crashed_journal(&database, &path, vec!["one".into()]);

        assert!(recover_journals(&database).is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), rollout("one"));
        assert!(!journal.exists());
    }

    #[test]
    fn committed_crash_keeps_the_deletion() {
        let (_temp, sessions, _archived, database) = fixture();
        let path = sessions.join("rollout-one.jsonl");
        std::fs::write(&path, rollout("one")).unwrap();
        let journal = crashed_journal(&database, &path, vec!["one".into()]);

        assert!(recover_journals(&database).is_empty());
        assert!(!path.exists());
        assert!(!journal.exists());
    }

    #[test]
    fn partial_file_only_deletion_is_rolled_back() {
        let (_temp, sessions, _archived, database) = fixture();
        let removed = sessions.join("rollout-one.jsonl");
        let kept = sessions.join("rollout-two.jsonl");
        std::fs::write(&removed, rollout("one")).unwrap();
        std::fs::write(&kept, rollout("two")).unwrap();
        let journal = crashed_journal(&database, &removed, Vec::new());
        // Add the second rollout to the journal as not yet removed.
        let mut record: DeleteJournal =
            serde_json::from_slice(&std::fs::read(&journal).unwrap()).unwrap();
        record.rollouts.push(RolloutBackup {
            original: kept.clone(),
            backup: database.parent().unwrap().join("unused.jsonl"),
        });
        std::fs::write(&journal, serde_json::to_vec(&record).unwrap()).unwrap();

        assert!(recover_journals(&database).is_empty());
        assert!(removed.is_file());
        assert!(kept.is_file());
        assert!(!journal.exists());
    }

    #[test]
    fn unrecoverable_journal_is_kept_and_blocks_backup_pruning() {
        let (_temp, sessions, _archived, database) = fixture();
        let path = sessions.join("rollout-one.jsonl");
        std::fs::write(&path, rollout("one")).unwrap();
        insert_thread(&database, "one", &path);
        let journal = crashed_journal(&database, &path, vec!["one".into()]);
        std::fs::remove_file(database.parent().unwrap().join("backup-rollout.jsonl")).unwrap();

        assert_eq!(recover_journals(&database).len(), 1);
        assert!(journal.exists());
        assert!(has_journals(&database));
    }

    #[test]
    fn rollout_id_ignores_truncated_lines_like_the_scanner() {
        let (_temp, sessions, _archived, _database) = fixture();
        let path = sessions.join("rollout-one.jsonl");
        std::fs::write(
            &path,
            format!("{}{{\"type\":\"event\",\"payl", rollout("one")),
        )
        .unwrap();
        assert_eq!(read_rollout_id(&path).unwrap(), "one");
    }

    #[test]
    fn queued_deletion_waits_for_writers_then_deletes_the_same_rollout() {
        let (_temp, sessions, archived, database) = fixture();
        let path = sessions.join("rollout-one.jsonl");
        std::fs::write(&path, rollout("one")).unwrap();
        Connection::open(&database)
            .unwrap()
            .execute(
                "INSERT INTO threads (id, rollout_path, title, cwd) VALUES (?1, ?2, 'One', 'C:/work')",
                params!["one", path.display().to_string()],
            )
            .unwrap();
        let queued = queue_delete(
            &database,
            &sessions,
            &archived,
            &["active:rollout-one.jsonl".into()],
        )
        .unwrap();
        assert_eq!(queued.queued, 1);
        let waiting = process_pending_deletes_inner(
            &database,
            &sessions,
            &archived,
            pending_jobs(&database).unwrap().jobs,
            false,
        );
        assert_eq!(waiting.keys, vec!["active:rollout-one.jsonl"]);
        assert!(path.is_file());

        let completed = process_pending_deletes_inner(
            &database,
            &sessions,
            &archived,
            pending_jobs(&database).unwrap().jobs,
            true,
        );
        assert!(completed.keys.is_empty());
        assert!(pending_jobs(&database).unwrap().jobs.is_empty());
        assert!(!path.exists());
        let rows: i64 = Connection::open(&database)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM threads", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 0);
    }

    #[test]
    fn queued_deletion_is_cancelled_if_the_rollout_changes() {
        let (_temp, sessions, archived, database) = fixture();
        let path = sessions.join("rollout-one.jsonl");
        std::fs::write(&path, rollout("one")).unwrap();
        Connection::open(&database)
            .unwrap()
            .execute(
                "INSERT INTO threads (id, rollout_path, title, cwd) VALUES (?1, ?2, 'One', 'C:/work')",
                params!["one", path.display().to_string()],
            )
            .unwrap();
        queue_delete(
            &database,
            &sessions,
            &archived,
            &["active:rollout-one.jsonl".into()],
        )
        .unwrap();
        std::fs::write(&path, format!("{}{{}}\n", rollout("one"))).unwrap();
        let status = process_pending_deletes_inner(
            &database,
            &sessions,
            &archived,
            pending_jobs(&database).unwrap().jobs,
            true,
        );
        assert!(status
            .warnings
            .iter()
            .any(|warning| warning.contains("changed")));
        assert!(pending_jobs(&database).unwrap().jobs.is_empty());
        assert!(path.is_file());
        let rows: i64 = Connection::open(&database)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM threads", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[test]
    fn a_changed_thread_is_kept_while_the_rest_of_its_job_is_deleted() {
        let (_temp, sessions, archived, database) = fixture();
        let connection = Connection::open(&database).unwrap();
        for id in ["one", "two"] {
            let path = sessions.join(format!("rollout-{id}.jsonl"));
            std::fs::write(&path, rollout(id)).unwrap();
            connection
                .execute(
                    "INSERT INTO threads (id, rollout_path, title, cwd) VALUES (?1, ?2, ?1, 'C:/work')",
                    params![id, path.display().to_string()],
                )
                .unwrap();
        }
        drop(connection);
        queue_delete(
            &database,
            &sessions,
            &archived,
            &[
                "active:rollout-one.jsonl".into(),
                "active:rollout-two.jsonl".into(),
            ],
        )
        .unwrap();
        let changed = sessions.join("rollout-one.jsonl");
        std::fs::write(&changed, format!("{}{{}}\n", rollout("one"))).unwrap();
        let status = process_pending_deletes_inner(
            &database,
            &sessions,
            &archived,
            pending_jobs(&database).unwrap().jobs,
            true,
        );
        assert!(status
            .warnings
            .iter()
            .any(|w| w.contains("active:rollout-one.jsonl")));
        assert!(pending_jobs(&database).unwrap().jobs.is_empty());
        assert!(changed.is_file(), "the changed thread is kept");
        assert!(
            !sessions.join("rollout-two.jsonl").exists(),
            "the unchanged thread is deleted"
        );
        let remaining: String = Connection::open(&database)
            .unwrap()
            .query_row("SELECT id FROM threads", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, "one");
    }

    #[test]
    fn queued_deletion_can_be_cancelled_before_writers_exit() {
        let (_temp, sessions, archived, database) = fixture();
        let path = sessions.join("rollout-one.jsonl");
        std::fs::write(&path, rollout("one")).unwrap();
        queue_delete(
            &database,
            &sessions,
            &archived,
            &["active:rollout-one.jsonl".into()],
        )
        .unwrap();
        assert_eq!(cancel_pending_deletes(&database).unwrap(), 1);
        assert!(pending_jobs(&database).unwrap().jobs.is_empty());
        assert!(path.is_file());
    }

    #[test]
    fn corrupt_queue_record_does_not_hide_valid_records_and_can_be_cleared_safely() {
        let (_temp, sessions, archived, database) = fixture();
        let rollout_path = sessions.join("rollout-one.jsonl");
        std::fs::write(&rollout_path, rollout("one")).unwrap();
        Connection::open(&database)
            .unwrap()
            .execute(
                "INSERT INTO threads (id, rollout_path, title, cwd) VALUES (?1, ?2, 'One', 'C:/work')",
                params!["one", rollout_path.display().to_string()],
            )
            .unwrap();
        queue_delete(
            &database,
            &sessions,
            &archived,
            &["active:rollout-one.jsonl".into()],
        )
        .unwrap();
        let queue = pending_dir(&database);
        std::fs::write(queue.join("broken.json"), b"not json").unwrap();

        let scan = pending_jobs(&database).unwrap();
        assert_eq!(scan.jobs.len(), 1);
        assert_eq!(scan.invalid_paths.len(), 1);
        assert_eq!(cancel_pending_deletes(&database).unwrap(), 2);
        assert!(pending_jobs(&database).unwrap().jobs.is_empty());
        assert!(rollout_path.is_file());
        let rows: i64 = Connection::open(&database)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM threads", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[test]
    fn corrupt_queue_without_valid_jobs_requests_recovery() {
        let (_temp, sessions, archived, database) = fixture();
        let queue = pending_dir(&database);
        std::fs::create_dir_all(&queue).unwrap();
        std::fs::write(queue.join("broken.json"), b"not json").unwrap();

        let status = process_pending_deletes(&database, &sessions, &archived);
        assert!(status.recovery_required);
        assert!(status.keys.is_empty());
        assert!(
            status.warnings.is_empty(),
            "corrupt records use the recovery flag"
        );
        assert_eq!(cancel_pending_deletes(&database).unwrap(), 1);
        assert!(!process_pending_deletes(&database, &sessions, &archived).recovery_required);
    }

    #[test]
    fn deletes_missing_rollout_catalog_entry() {
        let (_temp, sessions, archived, database) = fixture();
        Connection::open(&database)
            .unwrap()
            .execute(
                "INSERT INTO threads (id, rollout_path, title, cwd) VALUES ('missing', 'C:/gone.jsonl', 'Missing', 'C:/work')",
                [],
            )
            .unwrap();
        let result =
            delete_threads_inner(&database, &sessions, &archived, &["catalog:missing".into()])
                .unwrap();
        assert_eq!(result.catalog_rows_deleted, 1);
        assert_eq!(result.rollout_files_deleted, 0);
    }

    #[test]
    fn refuses_ambiguous_duplicate_rollouts() {
        let (_temp, sessions, archived, database) = fixture();
        std::fs::write(sessions.join("rollout-one.jsonl"), rollout("same")).unwrap();
        std::fs::write(archived.join("rollout-two.jsonl"), rollout("same")).unwrap();
        let error = delete_threads_inner(
            &database,
            &sessions,
            &archived,
            &["active:rollout-one.jsonl".into()],
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("multiple rollout files"));
    }

    #[test]
    fn refuses_unknown_inbound_catalog_references() {
        let (_temp, sessions, archived, database) = fixture();
        let path = sessions.join("rollout-one.jsonl");
        std::fs::write(&path, rollout("one")).unwrap();
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "PRAGMA foreign_keys=ON;
                 CREATE TABLE notes (
                   thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE
                 );",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO threads (id, rollout_path, title, cwd) VALUES (?1, ?2, 'One', 'C:/work')",
                params!["one", path.display().to_string()],
            )
            .unwrap();
        connection
            .execute("INSERT INTO notes (thread_id) VALUES ('one')", [])
            .unwrap();
        drop(connection);

        let error = delete_threads_inner(
            &database,
            &sessions,
            &archived,
            &["active:rollout-one.jsonl".into()],
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("notes.thread_id"));
        assert!(path.is_file());
        let count: i64 = Connection::open(&database)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM threads", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }
}
