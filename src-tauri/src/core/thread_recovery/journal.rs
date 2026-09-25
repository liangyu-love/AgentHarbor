use super::*;

pub(super) fn backup_directory(paths: &CodexPaths) -> PathBuf {
    paths.app_data_dir.join(BACKUP_DIR)
}

pub(super) fn journal_path(paths: &CodexPaths) -> PathBuf {
    paths.app_data_dir.join(JOURNAL_FILE)
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(super) fn write_backup(
    paths: &CodexPaths,
    thread_id: &str,
    database_provider: &str,
    rollout_path: &Path,
    rollout: &RolloutSnapshot,
    target_provider: &str,
    config_generation: &str,
    updated_sha256: &str,
    updated_length: u64,
) -> Result<PathBuf, CoreError> {
    write_backup_with_mutation(
        paths,
        thread_id,
        database_provider,
        rollout_path,
        rollout,
        target_provider,
        config_generation,
        updated_sha256,
        updated_length,
        CatalogMutation::UpdateProvider {
            original_provider: database_provider.to_owned(),
            target_provider: target_provider.to_owned(),
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn write_backup_with_mutation(
    paths: &CodexPaths,
    thread_id: &str,
    database_provider: &str,
    rollout_path: &Path,
    rollout: &RolloutSnapshot,
    target_provider: &str,
    config_generation: &str,
    updated_sha256: &str,
    updated_length: u64,
    mutation: CatalogMutation,
) -> Result<PathBuf, CoreError> {
    let backup = ThreadRecoveryBackup {
        version: 2,
        created_at: Utc::now().timestamp_millis(),
        thread_id: thread_id.to_owned(),
        database_path: paths.codex_state_db_path.display().to_string(),
        database_provider: database_provider.to_owned(),
        rollout_path: rollout_path.display().to_string(),
        rollout_provider: rollout.provider.clone(),
        target_provider: target_provider.to_owned(),
        config_generation: config_generation.to_owned(),
        original_rollout_sha256: rollout.sha256.clone(),
        updated_rollout_sha256: updated_sha256.to_owned(),
        original_rollout_length: rollout.length,
        updated_rollout_length: updated_length,
        rollout_base64: BASE64.encode(&rollout.bytes),
        mutation: Some(mutation),
    };
    let path = backup_directory(paths).join(format!(
        "{}-{}.json",
        Utc::now().timestamp_millis(),
        uuid::Uuid::new_v4()
    ));
    storage::write_atomic(&path, &serde_json::to_vec_pretty(&backup)?)?;
    Ok(path)
}

pub(super) fn write_journal(
    paths: &CodexPaths,
    journal: &RecoveryJournal,
) -> Result<(), CoreError> {
    storage::write_atomic(&journal_path(paths), &serde_json::to_vec_pretty(journal)?)
}

pub(super) fn decode_pending_journal(bytes: &[u8]) -> Result<RecoveryJournal, CoreError> {
    let journal: RecoveryJournal = serde_json::from_slice(bytes)?;
    if journal.version != 1 && journal.version != 2 {
        return Err(CoreError::InvalidData(
            "Unsupported thread recovery journal version".into(),
        ));
    }
    validate_thread_id(&journal.thread_id)?;
    Ok(journal)
}

pub(super) fn inspect_pending_journal(paths: &CodexPaths) -> PendingJournalInspection {
    let path = journal_path(paths);
    let directory_path = path
        .parent()
        .unwrap_or(&paths.app_data_dir)
        .display()
        .to_string();
    match std::fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => {
            return PendingJournalInspection::Blocked(ThreadRecoveryCoordination {
                journal_path: path.display().to_string(),
                directory_path,
                backup_path: None,
                detail: "The recovery journal path is not a regular file".into(),
            });
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return PendingJournalInspection::None;
        }
        Err(error) => {
            return PendingJournalInspection::Blocked(ThreadRecoveryCoordination {
                journal_path: path.display().to_string(),
                directory_path,
                backup_path: None,
                detail: format!("Cannot inspect the recovery journal: {error}"),
            });
        }
    }
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return PendingJournalInspection::Blocked(ThreadRecoveryCoordination {
                journal_path: path.display().to_string(),
                directory_path,
                backup_path: None,
                detail: format!("Cannot read the recovery journal: {error}"),
            });
        }
    };
    let backup_path = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|value| {
            value
                .get("backupPath")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        });
    match decode_pending_journal(&bytes) {
        Ok(journal) => PendingJournalInspection::Ready(journal),
        Err(error) => PendingJournalInspection::Blocked(ThreadRecoveryCoordination {
            journal_path: path.display().to_string(),
            directory_path,
            backup_path,
            detail: error.to_string(),
        }),
    }
}

pub(super) fn read_pending_journal(
    paths: &CodexPaths,
) -> Result<Option<RecoveryJournal>, CoreError> {
    let path = journal_path(paths);
    if !path.is_file() {
        return Ok(None);
    }
    Ok(Some(decode_pending_journal(&std::fs::read(path)?)?))
}

pub(super) fn remove_journal(paths: &CodexPaths) -> Result<(), CoreError> {
    storage::remove_file_durable(&journal_path(paths))
}

pub(super) fn load_backup(
    paths: &CodexPaths,
    path: &Path,
) -> Result<ThreadRecoveryBackup, CoreError> {
    let backup_root = backup_directory(paths);
    let parent = path
        .parent()
        .ok_or_else(|| CoreError::InvalidData("Recovery backup has no parent".into()))?;
    if parent != backup_root {
        return Err(CoreError::InvalidData(
            "Recovery backup is outside the AgentHarbor backup directory".into(),
        ));
    }
    let mut backup: ThreadRecoveryBackup = serde_json::from_slice(&std::fs::read(path)?)?;
    if !matches!(backup.version, 1 | 2)
        || Path::new(&backup.database_path) != paths.codex_state_db_path
    {
        return Err(CoreError::InvalidData(
            "Recovery backup belongs to another Codex home".into(),
        ));
    }
    validate_thread_id(&backup.thread_id)?;
    validate_rollout_path(paths, Path::new(&backup.rollout_path))?;
    if backup.mutation.is_none() {
        if backup.version != 1 {
            return Err(CoreError::InvalidData(
                "Recovery backup lacks its catalog mutation".into(),
            ));
        }
        backup.mutation = Some(CatalogMutation::UpdateProvider {
            original_provider: backup.database_provider.clone(),
            target_provider: backup.target_provider.clone(),
        });
    }
    Ok(backup)
}

pub(super) fn decode_original_rollout(backup: &ThreadRecoveryBackup) -> Result<Vec<u8>, CoreError> {
    let bytes = BASE64
        .decode(&backup.rollout_base64)
        .map_err(|_| CoreError::InvalidData("Recovery backup contains invalid base64".into()))?;
    if bytes.len() as u64 != backup.original_rollout_length
        || sha256_hex(&bytes) != backup.original_rollout_sha256
    {
        return Err(CoreError::InvalidData(
            "Recovery backup rollout checksum mismatch".into(),
        ));
    }
    Ok(bytes)
}

pub(super) fn decode_updated_rollout(backup: &ThreadRecoveryBackup) -> Result<Vec<u8>, CoreError> {
    let original = decode_original_rollout(backup)?;
    let updated = rewrite_rollout_provider(&original, &backup.thread_id, &backup.target_provider)?;
    if updated.len() as u64 != backup.updated_rollout_length
        || sha256_hex(&updated) != backup.updated_rollout_sha256
    {
        return Err(CoreError::InvalidData(
            "Recovery backup target rollout checksum mismatch".into(),
        ));
    }
    Ok(updated)
}

pub(super) fn backup_mutation(
    backup: &ThreadRecoveryBackup,
) -> Result<&CatalogMutation, CoreError> {
    backup.mutation.as_ref().ok_or_else(|| {
        CoreError::InvalidData("Recovery backup lacks its normalized catalog mutation".into())
    })
}

pub(super) fn validate_journal_mutation(
    journal: &RecoveryJournal,
    backup: &ThreadRecoveryBackup,
) -> Result<(), CoreError> {
    let backup_mutation = backup_mutation(backup)?;
    match (&journal.mutation, journal.version) {
        (Some(journal_mutation), 2) if journal_mutation == backup_mutation => Ok(()),
        (None, 1) if matches!(backup_mutation, CatalogMutation::UpdateProvider { .. }) => Ok(()),
        _ => Err(CoreError::InvalidData(
            "Recovery journal and backup catalog mutations differ".into(),
        )),
    }
}

pub(super) fn known_catalog_references(
    connection: &Connection,
    thread_id: &str,
) -> Result<Vec<String>, CoreError> {
    let mut references = Vec::new();
    let mut statement = connection
        .prepare(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name <> 'threads' ORDER BY name",
        )
        .map_err(sqlite_error)?;
    let tables = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sqlite_error)?;
    drop(statement);
    for table in tables {
        let columns = table_columns(connection, &table)?;
        let mut candidates = columns
            .iter()
            .filter(|column| {
                let normalized = column.to_ascii_lowercase();
                normalized == "thread_id"
                    || normalized == "session_id"
                    || (normalized.contains("thread") && normalized.ends_with("_id"))
            })
            .cloned()
            .collect::<HashSet<_>>();
        let pragma = format!("PRAGMA foreign_key_list({})", quote_identifier(&table));
        let mut foreign_keys = connection.prepare(&pragma).map_err(sqlite_error)?;
        let rows = foreign_keys
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })
            .map_err(sqlite_error)?;
        for row in rows {
            let (target_table, from_column, target_column) = row.map_err(sqlite_error)?;
            if target_table.eq_ignore_ascii_case("threads")
                && target_column
                    .as_deref()
                    .is_none_or(|column| column.eq_ignore_ascii_case("id"))
            {
                candidates.insert(from_column);
            }
        }
        drop(foreign_keys);
        for column in candidates {
            let sql = format!(
                "SELECT 1 FROM {} WHERE {} = ?1 LIMIT 1",
                quote_identifier(&table),
                quote_identifier(&column)
            );
            if connection
                .query_row(&sql, [thread_id], |_| Ok(()))
                .optional()
                .map_err(sqlite_error)?
                .is_some()
            {
                references.push(format!("{table}.{column}"));
            }
        }
    }
    references.sort();
    references.dedup();
    Ok(references)
}

pub(super) fn delete_inserted_catalog_row(
    paths: &CodexPaths,
    thread_id: &str,
    schema_signature: &str,
    columns: &[CatalogColumnValue],
    expected_fingerprint: &str,
) -> Result<(), CoreError> {
    let mut connection = open_database(&paths.codex_state_db_path, false)?;
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(sqlite_error)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sqlite_error)?;
    let Some((_, actual_fingerprint)) =
        read_catalog_row(&transaction, thread_id, schema_signature, columns)?
    else {
        transaction.commit().map_err(sqlite_error)?;
        return Ok(());
    };
    if actual_fingerprint != expected_fingerprint {
        return Err(CoreError::OperationFailed(
            "THREAD_RECOVERY_COMPENSATION_CONFLICT: inserted catalog row was externally modified"
                .into(),
        ));
    }
    let triggers = thread_trigger_names(&transaction)?;
    if !triggers.is_empty() {
        return Err(CoreError::OperationFailed(format!(
            "THREAD_RECOVERY_COMPENSATION_CONFLICT: threads triggers prevent safe deletion ({})",
            triggers.join(", ")
        )));
    }
    let references = known_catalog_references(&transaction, thread_id)?;
    if !references.is_empty() {
        return Err(CoreError::OperationFailed(format!(
            "THREAD_RECOVERY_COMPENSATION_CONFLICT: inserted catalog row is referenced by {}",
            references.join(", ")
        )));
    }
    let changed = transaction
        .execute("DELETE FROM threads WHERE id = ?1", [thread_id])
        .map_err(sqlite_error)?;
    if changed != 1 {
        return Err(CoreError::OperationFailed(
            "THREAD_RECOVERY_COMPENSATION_CONFLICT: expected one inserted row to be deleted".into(),
        ));
    }
    if read_catalog_row(&transaction, thread_id, schema_signature, columns)?.is_some() {
        return Err(CoreError::OperationFailed(
            "THREAD_RECOVERY_COMPENSATION_CONFLICT: inserted catalog row remains after deletion"
                .into(),
        ));
    }
    transaction.commit().map_err(sqlite_error)
}

#[cfg(target_os = "windows")]
pub(super) fn verify_insert_compensated_locked(
    paths: &CodexPaths,
    backup: &ThreadRecoveryBackup,
    schema_signature: &str,
    columns: &[CatalogColumnValue],
    locked_rollout: &mut LockedRollout,
) -> Result<(), CoreError> {
    let connection = open_database(&paths.codex_state_db_path, true)?;
    if read_catalog_row(&connection, &backup.thread_id, schema_signature, columns)?.is_some() {
        return Err(CoreError::OperationFailed(
            "Compensated catalog insertion still exists".into(),
        ));
    }
    let bytes = locked_rollout.read_bytes()?;
    if bytes.len() as u64 != backup.original_rollout_length
        || sha256_hex(&bytes) != backup.original_rollout_sha256
    {
        return Err(CoreError::OperationFailed(
            "Compensated rollout checksum does not match the backup".into(),
        ));
    }
    let snapshot = parse_rollout_bytes(bytes, &backup.thread_id)?;
    if snapshot.provider != backup.rollout_provider {
        return Err(CoreError::OperationFailed(
            "Compensated rollout provider does not match the backup".into(),
        ));
    }
    Ok(())
}

pub(super) fn recover_pending_journal(paths: &CodexPaths) -> Result<(), CoreError> {
    let path = journal_path(paths);
    if !path.is_file() {
        return Ok(());
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = paths;
        return Err(CoreError::OperationFailed(
            "THREAD_RECOVERY_UNSUPPORTED_PLATFORM: pending recovery requires Windows file locking"
                .into(),
        ));
    }

    #[cfg(target_os = "windows")]
    {
        recover_pending_journal_windows(paths)
    }
}

#[cfg(target_os = "windows")]
pub(super) fn recover_pending_journal_windows(paths: &CodexPaths) -> Result<(), CoreError> {
    let journal = read_pending_journal(paths)?.ok_or_else(|| {
        CoreError::NotFound("Thread recovery journal disappeared during coordination".into())
    })?;
    let backup_path = PathBuf::from(&journal.backup_path);
    let backup = load_backup(paths, &backup_path)?;
    if backup.thread_id != journal.thread_id {
        return Err(CoreError::InvalidData(
            "Recovery journal and backup thread IDs differ".into(),
        ));
    }
    if backup.config_generation != journal.config_generation {
        return Err(CoreError::InvalidData(
            "Recovery journal and backup config generations differ".into(),
        ));
    }
    validate_journal_mutation(&journal, &backup)?;
    ensure_config_generation(paths, &backup.config_generation)?;
    let rollout_path = validate_rollout_path(paths, Path::new(&backup.rollout_path))?;
    let mut locked_rollout = LockedRollout::open(&rollout_path)?;
    let current_bytes = locked_rollout.read_bytes()?;
    let current_hash = sha256_hex(&current_bytes);
    let rollout_is_original = current_bytes.len() as u64 == backup.original_rollout_length
        && current_hash == backup.original_rollout_sha256;
    let rollout_is_updated = current_bytes.len() as u64 == backup.updated_rollout_length
        && current_hash == backup.updated_rollout_sha256;
    let updated = decode_updated_rollout(&backup)?;
    let rollout_is_partial_update =
        current_bytes.len() < updated.len() && updated.starts_with(&current_bytes);

    if let CatalogMutation::InsertRow {
        schema_signature,
        columns,
        row_fingerprint,
    } = backup_mutation(&backup)?
    {
        let connection = open_database(&paths.codex_state_db_path, true)?;
        let row = read_catalog_row(&connection, &backup.thread_id, schema_signature, columns)?;
        drop(connection);
        let row_is_absent = row.is_none();
        let row_matches = row
            .as_ref()
            .is_some_and(|(_, fingerprint)| fingerprint == row_fingerprint);
        if row.is_some() && !row_matches {
            return Err(CoreError::OperationFailed(format!(
                "THREAD_RECOVERY_JOURNAL_CONFLICT: inserted row fingerprint changed; backupPath={}",
                backup_path.display()
            )));
        }
        if row_matches
            && rollout_is_updated
            && matches!(
                journal.stage,
                RecoveryStage::RolloutWritten | RecoveryStage::Committed
            )
        {
            verify_mutation_frozen_result_locked(
                paths,
                &backup.thread_id,
                backup_mutation(&backup)?,
                &backup.config_generation,
                &backup.updated_rollout_sha256,
                backup.updated_rollout_length,
                &mut locked_rollout,
            )?;
            remove_journal(paths)?;
            prune_backups(paths, Some(&backup_path))?;
            return Ok(());
        }
        if row_is_absent
            && rollout_is_original
            && matches!(
                journal.stage,
                RecoveryStage::Prepared | RecoveryStage::RolloutWritten
            )
        {
            verify_insert_compensated_locked(
                paths,
                &backup,
                schema_signature,
                columns,
                &mut locked_rollout,
            )?;
            remove_journal(paths)?;
            return Ok(());
        }
        if row_is_absent
            && (rollout_is_updated || rollout_is_partial_update)
            && matches!(
                journal.stage,
                RecoveryStage::Prepared | RecoveryStage::RolloutWritten
            )
        {
            locked_rollout.write_bytes(&decode_original_rollout(&backup)?)?;
            verify_insert_compensated_locked(
                paths,
                &backup,
                schema_signature,
                columns,
                &mut locked_rollout,
            )?;
            remove_journal(paths)?;
            return Ok(());
        }
        return Err(CoreError::OperationFailed(format!(
            "THREAD_RECOVERY_JOURNAL_CONFLICT: stage={:?}; backupPath={}",
            journal.stage,
            backup_path.display()
        )));
    }

    let database = load_database_thread(&paths.codex_state_db_path, &backup.thread_id)?
        .ok_or_else(|| CoreError::NotFound("Recovery journal thread catalog row".into()))?;
    let database_is_original = database.provider == backup.database_provider;
    let database_is_updated = database.provider == backup.target_provider;

    if database_is_updated && rollout_is_updated {
        verify_frozen_result_locked(
            paths,
            &backup.thread_id,
            &backup.target_provider,
            &backup.updated_rollout_sha256,
            backup.updated_rollout_length,
            &mut locked_rollout,
        )?;
        remove_journal(paths)?;
        prune_backups(paths, Some(&backup_path))?;
        return Ok(());
    }
    if database_is_original && rollout_is_original {
        verify_compensated_result_locked(paths, &backup, &mut locked_rollout)?;
        remove_journal(paths)?;
        return Ok(());
    }
    if database_is_original && rollout_is_updated {
        let original = decode_original_rollout(&backup)?;
        locked_rollout.write_bytes(&original)?;
        verify_compensated_result_locked(paths, &backup, &mut locked_rollout)?;
        remove_journal(paths)?;
        return Ok(());
    }
    if journal.stage == RecoveryStage::Prepared && database_is_original && rollout_is_partial_update
    {
        let original = decode_original_rollout(&backup)?;
        locked_rollout.write_bytes(&original)?;
        verify_compensated_result_locked(paths, &backup, &mut locked_rollout)?;
        remove_journal(paths)?;
        return Ok(());
    }
    if database_is_updated && rollout_is_original {
        update_database_provider(
            paths,
            &backup.thread_id,
            &backup.target_provider,
            &backup.database_provider,
        )?;
        verify_compensated_result_locked(paths, &backup, &mut locked_rollout)?;
        remove_journal(paths)?;
        return Ok(());
    }
    Err(CoreError::OperationFailed(format!(
        "THREAD_RECOVERY_JOURNAL_CONFLICT: stage={:?}; backupPath={}",
        journal.stage,
        backup_path.display()
    )))
}

#[cfg(target_os = "windows")]
pub(super) fn compensate_from_backup_locked(
    paths: &CodexPaths,
    backup_path: &Path,
    stage: RecoveryStage,
    locked_rollout: &mut LockedRollout,
) -> Result<(), CoreError> {
    let backup = load_backup(paths, backup_path)?;
    let rollout_path = validate_rollout_path(paths, Path::new(&backup.rollout_path))?;
    if locked_rollout.path != rollout_path {
        return Err(CoreError::InvalidData(
            "Recovery compensation received a lock for another rollout".into(),
        ));
    }
    let original = decode_original_rollout(&backup)?;
    let current_bytes = locked_rollout.read_bytes()?;
    let current_hash = sha256_hex(&current_bytes);
    let rollout_is_original = current_bytes.len() as u64 == backup.original_rollout_length
        && current_hash == backup.original_rollout_sha256;
    let rollout_is_updated = current_bytes.len() as u64 == backup.updated_rollout_length
        && current_hash == backup.updated_rollout_sha256;
    let updated = decode_updated_rollout(&backup)?;
    let rollout_is_partial_update = stage == RecoveryStage::Prepared
        && current_bytes.len() < updated.len()
        && updated.starts_with(&current_bytes);
    if !rollout_is_original && !rollout_is_updated && !rollout_is_partial_update {
        return Err(CoreError::OperationFailed(format!(
            "THREAD_RECOVERY_COMPENSATION_CONFLICT: rollout differs from both frozen states; backupPath={}",
            backup_path.display()
        )));
    }

    if let CatalogMutation::InsertRow {
        schema_signature,
        columns,
        row_fingerprint,
    } = backup_mutation(&backup)?
    {
        let connection = open_database(&paths.codex_state_db_path, true)?;
        let row = read_catalog_row(&connection, &backup.thread_id, schema_signature, columns)?;
        drop(connection);
        if let Some((_, actual_fingerprint)) = row {
            if actual_fingerprint != *row_fingerprint {
                return Err(CoreError::OperationFailed(format!(
                    "THREAD_RECOVERY_COMPENSATION_CONFLICT: inserted row fingerprint changed; backupPath={}",
                    backup_path.display()
                )));
            }
            delete_inserted_catalog_row(
                paths,
                &backup.thread_id,
                schema_signature,
                columns,
                row_fingerprint,
            )?;
        }
        if rollout_is_updated || rollout_is_partial_update {
            locked_rollout.write_bytes(&original)?;
        }
        verify_insert_compensated_locked(
            paths,
            &backup,
            schema_signature,
            columns,
            locked_rollout,
        )?;
        return remove_journal(paths);
    }

    let database = load_database_thread(&paths.codex_state_db_path, &backup.thread_id)?
        .ok_or_else(|| CoreError::NotFound("Recovery compensation catalog row".into()))?;
    if database.provider == backup.target_provider {
        update_database_provider(
            paths,
            &backup.thread_id,
            &backup.target_provider,
            &backup.database_provider,
        )?;
    } else if database.provider != backup.database_provider {
        return Err(CoreError::OperationFailed(
            "Recovery compensation found an unexpected SQLite provider".into(),
        ));
    }
    if rollout_is_updated || rollout_is_partial_update {
        locked_rollout.write_bytes(&original)?;
    }
    verify_compensated_result_locked(paths, &backup, locked_rollout)?;
    remove_journal(paths)
}

#[cfg(target_os = "windows")]
pub(super) fn verify_compensated_result_locked(
    paths: &CodexPaths,
    backup: &ThreadRecoveryBackup,
    locked_rollout: &mut LockedRollout,
) -> Result<(), CoreError> {
    let database = load_database_thread(&paths.codex_state_db_path, &backup.thread_id)?
        .ok_or_else(|| {
            CoreError::NotFound("Thread catalog row during compensation verification".into())
        })?;
    if database.provider != backup.database_provider {
        return Err(CoreError::OperationFailed(
            "Compensated SQLite provider does not match the backup".into(),
        ));
    }
    let bytes = locked_rollout.read_bytes()?;
    if bytes.len() as u64 != backup.original_rollout_length
        || sha256_hex(&bytes) != backup.original_rollout_sha256
    {
        return Err(CoreError::OperationFailed(
            "Compensated rollout checksum does not match the backup".into(),
        ));
    }
    let snapshot = parse_rollout_bytes(bytes, &backup.thread_id)?;
    if snapshot.provider != backup.rollout_provider {
        return Err(CoreError::OperationFailed(
            "Compensated rollout provider does not match the backup".into(),
        ));
    }
    Ok(())
}

pub(super) fn update_database_provider(
    paths: &CodexPaths,
    thread_id: &str,
    expected: &str,
    replacement: &str,
) -> Result<(), CoreError> {
    let connection = open_database(&paths.codex_state_db_path, false)?;
    let changed = connection
        .execute(
            "UPDATE threads SET model_provider = ?1 WHERE id = ?2 AND model_provider = ?3",
            (replacement, thread_id, expected),
        )
        .map_err(sqlite_error)?;
    if changed != 1 {
        return Err(CoreError::OperationFailed(
            "Recovery compensation could not update the expected SQLite row".into(),
        ));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
pub(super) fn verify_mutation_frozen_result_locked(
    paths: &CodexPaths,
    thread_id: &str,
    mutation: &CatalogMutation,
    expected_config_generation: &str,
    expected_hash: &str,
    expected_length: u64,
    locked_rollout: &mut LockedRollout,
) -> Result<(), CoreError> {
    ensure_config_generation(paths, expected_config_generation)?;
    match mutation {
        CatalogMutation::UpdateProvider {
            target_provider, ..
        } => {
            let database = load_database_thread(&paths.codex_state_db_path, thread_id)?
                .ok_or_else(|| {
                    CoreError::NotFound("Thread catalog row during verification".into())
                })?;
            if database.provider != *target_provider {
                return Err(CoreError::OperationFailed(format!(
                    "SQLite provider is {}, expected {target_provider}",
                    database.provider
                )));
            }
        }
        CatalogMutation::InsertRow {
            schema_signature,
            columns,
            row_fingerprint,
        } => {
            let connection = open_database(&paths.codex_state_db_path, true)?;
            let (_, actual_fingerprint) =
                read_catalog_row(&connection, thread_id, schema_signature, columns)?.ok_or_else(
                    || {
                        CoreError::NotFound(
                            "Inserted thread catalog row during verification".into(),
                        )
                    },
                )?;
            if actual_fingerprint != *row_fingerprint {
                return Err(CoreError::OperationFailed(
                    "Inserted catalog row fingerprint differs from the frozen result".into(),
                ));
            }
        }
    }
    let bytes = locked_rollout.read_bytes()?;
    if bytes.len() as u64 != expected_length || sha256_hex(&bytes) != expected_hash {
        return Err(CoreError::OperationFailed(
            "Rollout checksum differs from the frozen recovery result".into(),
        ));
    }
    let snapshot = parse_rollout_bytes(bytes, thread_id)?;
    let expected_provider = match mutation {
        CatalogMutation::UpdateProvider {
            target_provider, ..
        } => target_provider,
        CatalogMutation::InsertRow { columns, .. } => columns
            .iter()
            .find(|column| column.name == "model_provider")
            .and_then(|column| match &column.value {
                CatalogSqlValue::Text(value) => Some(value),
                _ => None,
            })
            .ok_or_else(|| {
                CoreError::InvalidData("Frozen catalog row lacks a text model_provider".into())
            })?,
    };
    if snapshot.provider.as_deref() != Some(expected_provider.as_str()) {
        return Err(CoreError::OperationFailed(
            "Rollout provider differs from the frozen recovery target".into(),
        ));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
pub(super) fn verify_frozen_result_locked(
    paths: &CodexPaths,
    thread_id: &str,
    expected_provider: &str,
    expected_hash: &str,
    expected_length: u64,
    locked_rollout: &mut LockedRollout,
) -> Result<(), CoreError> {
    let database = load_database_thread(&paths.codex_state_db_path, thread_id)?
        .ok_or_else(|| CoreError::NotFound("Thread catalog row during verification".into()))?;
    if database.provider != expected_provider {
        return Err(CoreError::OperationFailed(format!(
            "SQLite provider is {}, expected {expected_provider}",
            database.provider
        )));
    }
    let bytes = locked_rollout.read_bytes()?;
    if bytes.len() as u64 != expected_length || sha256_hex(&bytes) != expected_hash {
        return Err(CoreError::OperationFailed(
            "Rollout checksum differs from the frozen recovery result".into(),
        ));
    }
    let snapshot = parse_rollout_bytes(bytes, thread_id)?;
    if snapshot.provider.as_deref() != Some(expected_provider) {
        return Err(CoreError::OperationFailed(
            "Rollout provider differs from the frozen recovery target".into(),
        ));
    }
    Ok(())
}

pub(super) fn prune_backups(paths: &CodexPaths, preserve: Option<&Path>) -> Result<(), CoreError> {
    let directory = backup_directory(paths);
    if !directory.is_dir() {
        return Ok(());
    }
    let mut files = std::fs::read_dir(&directory)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            if !file_type.is_file() || entry.path().extension()?.to_str()? != "json" {
                return None;
            }
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((entry.path(), modified))
        })
        .collect::<Vec<_>>();
    files.sort_by(|a, b| b.1.cmp(&a.1));
    let now = SystemTime::now();
    for (index, (path, modified)) in files.into_iter().enumerate() {
        if preserve.is_some_and(|preserve| preserve == path) {
            continue;
        }
        let expired = now
            .duration_since(modified)
            .is_ok_and(|age| age > BACKUP_MAX_AGE);
        if index >= BACKUP_LIMIT || (expired && index > 0) {
            let _ = std::fs::remove_file(path);
        }
    }
    Ok(())
}
