use super::{models::CoreError, threads};
use chrono::Utc;
use rusqlite::{params, Connection, OpenFlags, TransactionBehavior};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadDeleteResult {
    pub deleted: usize,
    pub catalog_rows_deleted: usize,
    pub rollout_files_deleted: usize,
    pub backup_path: String,
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

#[derive(Debug)]
struct RolloutBackup {
    original: PathBuf,
    backup: PathBuf,
}

pub fn delete_threads(
    state_database_path: &Path,
    sessions_dir: &Path,
    archived_dir: &Path,
    keys: &[String],
) -> Result<ThreadDeleteResult, CoreError> {
    let _guard = super::thread_recovery::thread_mutation_guard()?;
    #[cfg(target_os = "windows")]
    crate::platform::process::ensure_no_codex_writer_processes()?;
    delete_threads_inner(state_database_path, sessions_dir, archived_dir, keys)
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

    let removed_files = if let Some(connection) = connection.as_mut() {
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        validate_catalog_rows(&transaction, &catalog_ids)?;
        let removed = match remove_rollouts(&rollout_backups) {
            Ok(removed) => removed,
            Err(error) => return Err(error),
        };
        let mutation = (|| -> Result<usize, CoreError> {
            let mut deleted = 0;
            for thread_id in &catalog_ids {
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
                restore_rollouts(&rollout_backups, &removed);
                return Err(error);
            }
        };
        if let Err(error) = transaction.commit() {
            restore_rollouts(&rollout_backups, &removed);
            return Err(sql_error(error));
        }
        debug_assert_eq!(catalog_deleted, catalog_ids.len());
        removed
    } else {
        remove_rollouts(&rollout_backups)?
    };

    let _ = threads::scan_threads_with_warnings(sessions_dir, archived_dir);
    Ok(ThreadDeleteResult {
        deleted: selected.len(),
        catalog_rows_deleted: catalog_ids.len(),
        rollout_files_deleted: removed_files.len(),
        backup_path: backup_dir.display().to_string(),
    })
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
        let value: serde_json::Value = serde_json::from_str(&line).map_err(|error| {
            CoreError::InvalidData(format!("invalid rollout {}: {error}", path.display()))
        })?;
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
    let columns = table_columns(connection, "threads")?;
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

fn table_columns(connection: &Connection, table: &str) -> Result<HashSet<String>, CoreError> {
    let escaped = table.replace('"', "\"\"");
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info(\"{escaped}\")"))
        .map_err(sql_error)?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(sql_error)?;
    rows.collect::<Result<HashSet<_>, _>>().map_err(sql_error)
}

fn table_has_value(
    connection: &Connection,
    table: &str,
    column: &str,
    value: &str,
) -> Result<bool, CoreError> {
    if !table_columns(connection, table)?.contains(column) {
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
