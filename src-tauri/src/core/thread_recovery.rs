use super::{models::CoreError, storage};
use crate::platform::{paths::CodexPaths, process};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use chrono::{DateTime, Utc};
use rusqlite::{
    params_from_iter,
    types::{Value as SqlValue, ValueRef},
    Connection, OpenFlags, OptionalExtension, TransactionBehavior,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
#[cfg(target_os = "windows")]
use std::fs::{File, OpenOptions};
#[cfg(target_os = "windows")]
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, SystemTime};

mod journal;
mod policy;
mod rollout;

use journal::*;
use policy::*;
use rollout::*;

const BACKUP_LIMIT: usize = 20;
const BACKUP_MAX_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const JOURNAL_FILE: &str = "thread-recovery-journal.json";
const BACKUP_DIR: &str = "thread-recovery-backups";

static RECOVERY_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub(crate) fn thread_mutation_guard() -> Result<MutexGuard<'static, ()>, CoreError> {
    RECOVERY_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| CoreError::OperationFailed("Thread mutation lock is poisoned".into()))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ThreadRecoveryStatus {
    Compatible,
    OfficialAccountUnverified,
    ProviderMismatch,
    ProviderConflict,
    MissingCatalog,
    SchemaUnsupported,
    InsufficientMetadata,
    MissingRollout,
    InvalidRollout,
    InvalidTargetProvider,
    JournalBlocked,
    RecoveryPending,
    UnsupportedPlatform,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadCatalogAnalysis {
    pub schema_signature: Option<String>,
    pub ready: bool,
    pub missing_fields: Vec<String>,
    pub unsupported_columns: Vec<String>,
    pub relation: ThreadCatalogRelation,
    pub mapping_errors: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ThreadCatalogRelation {
    TopLevel,
    UserFork,
    Child,
    Ambiguous,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRecoveryChange {
    pub target: String,
    pub path: String,
    pub from_provider: Option<String>,
    pub to_provider: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRecoveryCatalogField {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRecoveryCoordination {
    pub journal_path: String,
    pub directory_path: String,
    pub backup_path: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRecoveryPreview {
    pub thread_id: String,
    pub configured_provider: String,
    pub target_provider: String,
    pub database_provider: Option<String>,
    pub rollout_provider: Option<String>,
    pub rollout_path: Option<String>,
    pub revision: String,
    pub catalog_analysis: Option<ThreadCatalogAnalysis>,
    pub catalog_fields: Vec<ThreadRecoveryCatalogField>,
    pub coordination: Option<ThreadRecoveryCoordination>,
    pub status: ThreadRecoveryStatus,
    pub can_apply: bool,
    pub reason: String,
    pub changes: Vec<ThreadRecoveryChange>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum SqliteAffinity {
    Integer,
    Text,
    Blob,
    Real,
    Numeric,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ThreadSchemaColumn {
    ordinal: i64,
    name: String,
    declared_type: String,
    affinity: SqliteAffinity,
    not_null: bool,
    default_value: Option<String>,
    primary_key: i64,
    hidden: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRecoveryResult {
    pub changed: bool,
    pub backup_path: Option<String>,
    pub preview: ThreadRecoveryPreview,
}

#[derive(Debug, Clone)]
struct ProviderTarget {
    configured: String,
    generation: String,
    valid: bool,
    invalid_reason: Option<String>,
}

#[derive(Debug, Clone)]
struct DatabaseThread {
    provider: String,
    rollout_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct RolloutSnapshot {
    provider: Option<String>,
    has_session_meta: bool,
    bytes: Vec<u8>,
    sha256: String,
    length: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct CatalogMetadataAnalysis {
    fields: HashSet<String>,
    values: BTreeMap<String, CatalogSqlValue>,
    mapping_errors: Vec<String>,
    relation: Option<ThreadCatalogRelation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
enum CatalogSqlValue {
    Null,
    Integer(i64),
    Text(String),
    TextBytesBase64(String),
    RealBits(u64),
    BlobBase64(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct CatalogColumnValue {
    ordinal: i64,
    name: String,
    affinity: SqliteAffinity,
    value: CatalogSqlValue,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
enum CatalogMutation {
    UpdateProvider {
        original_provider: String,
        target_provider: String,
    },
    InsertRow {
        schema_signature: String,
        columns: Vec<CatalogColumnValue>,
        row_fingerprint: String,
    },
}

#[derive(Debug, Clone)]
struct CatalogInsertPlan {
    mutation: CatalogMutation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CatalogPolicyFields {
    sandbox_policy: Option<String>,
    approval_mode: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LegacySandboxEnforcement {
    Managed,
    Disabled,
    External,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum CatalogNetworkPolicy {
    Restricted,
    Enabled,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
enum CatalogPermissionProfile {
    Managed {
        file_system: CatalogManagedFileSystem,
        network: CatalogNetworkPolicy,
    },
    Disabled,
    External {
        network: CatalogNetworkPolicy,
    },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
enum CatalogManagedFileSystem {
    Restricted {
        entries: Vec<CatalogFileSystemEntry>,
        #[serde(skip_serializing_if = "Option::is_none")]
        glob_scan_max_depth: Option<u64>,
    },
    Unrestricted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CatalogFileSystemPolicy {
    Managed(CatalogManagedFileSystem),
    ExternalSandbox,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct CatalogFileSystemEntry {
    path: CatalogFileSystemPath,
    access: CatalogFileSystemAccess,
    #[serde(skip_serializing_if = "Option::is_none")]
    missing_path_behavior: Option<CatalogMissingPathBehavior>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum CatalogFileSystemAccess {
    Read,
    Write,
    Deny,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum CatalogMissingPathBehavior {
    Skip,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
enum CatalogFileSystemPath {
    Path { path: String },
    GlobPattern { pattern: String },
    Special { value: CatalogSpecialPath },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum CatalogSpecialPath {
    Root,
    Minimal,
    ProjectRoots {
        #[serde(skip_serializing_if = "Option::is_none")]
        subpath: Option<String>,
    },
    Tmpdir,
    SlashTmp,
    Unknown {
        path: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        subpath: Option<String>,
    },
}

#[cfg(target_os = "windows")]
struct LockedRollout {
    file: File,
    path: PathBuf,
}

#[cfg(target_os = "windows")]
impl LockedRollout {
    fn open(path: &Path) -> Result<Self, CoreError> {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(FILE_SHARE_READ)
            .open(path)
            .map_err(|error| {
                CoreError::OperationFailed(format!(
                    "THREAD_RECOVERY_LOCK_FAILED: {}: {error}",
                    path.display()
                ))
            })?;
        Ok(Self {
            file,
            path: path.to_path_buf(),
        })
    }

    fn read_bytes(&mut self) -> Result<Vec<u8>, CoreError> {
        self.file.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        self.file.read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    fn snapshot(&mut self, thread_id: &str) -> Result<RolloutSnapshot, CoreError> {
        parse_rollout_bytes(self.read_bytes()?, thread_id)
    }

    fn write_bytes(&mut self, contents: &[u8]) -> Result<(), CoreError> {
        self.file.seek(SeekFrom::Start(0))?;
        self.file.set_len(0)?;
        self.file.write_all(contents)?;
        self.file.sync_all()?;
        Ok(())
    }
}

#[derive(Debug)]
struct RecoveryPlan {
    preview: ThreadRecoveryPreview,
    config_generation: String,
    rollout_path: Option<PathBuf>,
    rollout: Option<RolloutSnapshot>,
    pending_journal: Option<RecoveryJournal>,
    mutation: Option<CatalogMutation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadRecoveryBackup {
    version: u32,
    created_at: i64,
    thread_id: String,
    database_path: String,
    database_provider: String,
    rollout_path: String,
    rollout_provider: Option<String>,
    target_provider: String,
    #[serde(default)]
    config_generation: String,
    original_rollout_sha256: String,
    updated_rollout_sha256: String,
    original_rollout_length: u64,
    updated_rollout_length: u64,
    rollout_base64: String,
    #[serde(default)]
    mutation: Option<CatalogMutation>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum RecoveryStage {
    Prepared,
    RolloutWritten,
    Committed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecoveryJournal {
    version: u32,
    thread_id: String,
    backup_path: String,
    #[serde(default)]
    config_generation: String,
    stage: RecoveryStage,
    #[serde(default)]
    mutation: Option<CatalogMutation>,
}

#[derive(Debug)]
enum PendingJournalInspection {
    None,
    Ready(RecoveryJournal),
    Blocked(ThreadRecoveryCoordination),
}

pub fn preview_thread_recovery(
    paths: &CodexPaths,
    thread_id: &str,
) -> Result<ThreadRecoveryPreview, CoreError> {
    Ok(build_recovery_plan(paths, thread_id)?.preview)
}

pub fn apply_thread_recovery(
    paths: &CodexPaths,
    thread_id: &str,
    expected_revision: &str,
) -> Result<ThreadRecoveryResult, CoreError> {
    apply_thread_recovery_inner(paths, thread_id, expected_revision, true)
}

fn apply_thread_recovery_inner(
    paths: &CodexPaths,
    thread_id: &str,
    expected_revision: &str,
    check_writer_processes: bool,
) -> Result<ThreadRecoveryResult, CoreError> {
    let _guard = thread_mutation_guard()?;
    if check_writer_processes {
        process::ensure_no_codex_writer_processes()?;
    }
    let plan = build_recovery_plan_with_lookup(paths, thread_id, RolloutLookup::Refresh)?;
    if expected_revision.is_empty() || plan.preview.revision != expected_revision {
        return Err(CoreError::OperationFailed(
            "THREAD_RECOVERY_REVISION_CHANGED: preview state changed; refresh and confirm again"
                .into(),
        ));
    }
    if let Some(journal) = plan.pending_journal.as_ref() {
        if journal.thread_id != thread_id {
            return Err(CoreError::OperationFailed(format!(
                "THREAD_RECOVERY_PENDING_OTHER_THREAD: unfinished recovery belongs to {}",
                journal.thread_id
            )));
        }
        let backup_path = journal.backup_path.clone();
        recover_pending_journal(paths)?;
        prune_backups(paths, None)?;
        return Ok(ThreadRecoveryResult {
            changed: false,
            backup_path: Some(backup_path),
            preview: build_recovery_plan(paths, thread_id)?.preview,
        });
    }
    prune_backups(paths, None)?;

    if plan.preview.status == ThreadRecoveryStatus::Compatible {
        return Ok(ThreadRecoveryResult {
            changed: false,
            backup_path: None,
            preview: plan.preview,
        });
    }
    if !plan.preview.can_apply {
        return Err(CoreError::OperationFailed(format!(
            "THREAD_RECOVERY_NOT_APPLICABLE: {}",
            plan.preview.reason
        )));
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (paths, thread_id, check_writer_processes, plan);
        return Err(CoreError::OperationFailed(
            "THREAD_RECOVERY_UNSUPPORTED_PLATFORM: safe apply requires Windows file locking".into(),
        ));
    }

    #[cfg(target_os = "windows")]
    {
        apply_thread_recovery_windows(paths, thread_id, check_writer_processes, plan)
    }
}

#[cfg(target_os = "windows")]
fn apply_thread_recovery_windows(
    paths: &CodexPaths,
    thread_id: &str,
    check_writer_processes: bool,
    plan: RecoveryPlan,
) -> Result<ThreadRecoveryResult, CoreError> {
    let target_provider = plan.preview.target_provider.clone();
    let config_generation = plan.config_generation.clone();
    let mutation = plan.mutation.clone().ok_or_else(|| {
        CoreError::OperationFailed(
            "THREAD_RECOVERY_NOT_APPLICABLE: no frozen catalog mutation".into(),
        )
    })?;
    let rollout_path = plan
        .rollout_path
        .as_ref()
        .ok_or_else(|| CoreError::NotFound("Thread rollout file".into()))?
        .clone();
    let planned_rollout = plan
        .rollout
        .as_ref()
        .ok_or_else(|| CoreError::NotFound("Thread rollout snapshot".into()))?;
    let mut locked_rollout = LockedRollout::open(&rollout_path)?;
    let rollout = locked_rollout.snapshot(thread_id)?;
    if rollout.length != planned_rollout.length || rollout.sha256 != planned_rollout.sha256 {
        return Err(CoreError::OperationFailed(
            "THREAD_RECOVERY_CHANGED: rollout changed before the exclusive lock was acquired"
                .into(),
        ));
    }
    ensure_config_generation(paths, &config_generation)?;

    let updated_rollout = if rollout.provider.as_deref() == Some(target_provider.as_str()) {
        rollout.bytes.clone()
    } else {
        rewrite_rollout_provider(&rollout.bytes, thread_id, &target_provider)?
    };
    let updated_sha256 = sha256_hex(&updated_rollout);
    let updated_length = updated_rollout.len() as u64;

    let database_provider = match &mutation {
        CatalogMutation::UpdateProvider {
            original_provider, ..
        } => original_provider.as_str(),
        CatalogMutation::InsertRow { .. } => "",
    };
    let backup_path = write_backup_with_mutation(
        paths,
        thread_id,
        database_provider,
        &rollout_path,
        &rollout,
        &target_provider,
        &config_generation,
        &updated_sha256,
        updated_length,
        mutation.clone(),
    )?;
    let mut journal = RecoveryJournal {
        version: 2,
        thread_id: thread_id.to_owned(),
        backup_path: backup_path.display().to_string(),
        config_generation: config_generation.clone(),
        stage: RecoveryStage::Prepared,
        mutation: Some(mutation.clone()),
    };
    write_journal(paths, &journal)?;

    let operation = (|| -> Result<(), CoreError> {
        let mut connection = open_database(&paths.codex_state_db_path, false)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite_error)?;
        match &mutation {
            CatalogMutation::UpdateProvider {
                original_provider, ..
            } => {
                let current_database_provider = query_database_provider(&transaction, thread_id)?;
                if current_database_provider.as_deref() != Some(original_provider.as_str()) {
                    return Err(CoreError::OperationFailed(
                        "THREAD_RECOVERY_CHANGED: SQLite provider changed after preview".into(),
                    ));
                }
            }
            CatalogMutation::InsertRow {
                schema_signature,
                columns,
                ..
            } => {
                if read_catalog_row(&transaction, thread_id, schema_signature, columns)?.is_some() {
                    return Err(CoreError::OperationFailed(
                        "THREAD_RECOVERY_CHANGED: a catalog row appeared after preview".into(),
                    ));
                }
                let relation = catalog_metadata_analysis(
                    paths,
                    thread_id,
                    &ProviderTarget {
                        configured: target_provider.clone(),
                        generation: config_generation.clone(),
                        valid: true,
                        invalid_reason: None,
                    },
                    Some(&rollout_path),
                    Some(&rollout),
                    &transaction,
                )?;
                if !relation.mapping_errors.is_empty()
                    || !matches!(
                        relation.relation,
                        Some(ThreadCatalogRelation::TopLevel | ThreadCatalogRelation::UserFork)
                    )
                {
                    return Err(CoreError::OperationFailed(
                        "THREAD_RECOVERY_RELATION_CHANGED: thread is no longer a verified top-level or user-fork thread"
                            .into(),
                    ));
                }
            }
        }

        if check_writer_processes {
            process::ensure_no_codex_writer_processes()?;
        }
        ensure_config_generation(paths, &config_generation)?;
        let locked_snapshot = locked_rollout.snapshot(thread_id)?;
        if locked_snapshot.length != rollout.length || locked_snapshot.sha256 != rollout.sha256 {
            return Err(CoreError::OperationFailed(
                "THREAD_RECOVERY_CHANGED: locked rollout changed during recovery".into(),
            ));
        }
        if updated_sha256 != rollout.sha256 || updated_length != rollout.length {
            locked_rollout
                .write_bytes(&updated_rollout)
                .map_err(|error| {
                    CoreError::OperationFailed(format!(
                        "THREAD_RECOVERY_ROLLOUT_WRITE_FAILED: {error}; backupPath={}",
                        backup_path.display()
                    ))
                })?;
        }
        journal.stage = RecoveryStage::RolloutWritten;
        write_journal(paths, &journal)?;

        ensure_config_generation(paths, &config_generation)?;
        match &mutation {
            CatalogMutation::UpdateProvider {
                original_provider,
                target_provider,
            } => {
                let changed = transaction
                    .execute(
                        "UPDATE threads SET model_provider = ?1 WHERE id = ?2 AND model_provider = ?3",
                        (target_provider, thread_id, original_provider),
                    )
                    .map_err(sqlite_error)?;
                if changed != 1 {
                    return Err(CoreError::OperationFailed(
                        "THREAD_RECOVERY_CHANGED: SQLite row changed during recovery".into(),
                    ));
                }
            }
            CatalogMutation::InsertRow {
                schema_signature,
                columns,
                row_fingerprint,
            } => insert_catalog_row(
                &transaction,
                thread_id,
                schema_signature,
                columns,
                row_fingerprint,
            )?,
        }
        transaction.commit().map_err(sqlite_error)?;
        ensure_config_generation(paths, &config_generation)?;
        journal.stage = RecoveryStage::Committed;
        write_journal(paths, &journal)?;
        Ok(())
    })();

    if let Err(error) = operation {
        let compensation =
            compensate_from_backup_locked(paths, &backup_path, journal.stage, &mut locked_rollout);
        return match compensation {
            Ok(()) => Err(CoreError::OperationFailed(format!(
                "{error}; changes were compensated; backupPath={}",
                backup_path.display()
            ))),
            Err(compensation_error) => Err(CoreError::OperationFailed(format!(
                "{error}; compensation failed: {compensation_error}; backupPath={}",
                backup_path.display()
            ))),
        };
    }

    if let Err(error) = verify_mutation_frozen_result_locked(
        paths,
        thread_id,
        &mutation,
        &config_generation,
        &updated_sha256,
        updated_length,
        &mut locked_rollout,
    ) {
        let compensation =
            compensate_from_backup_locked(paths, &backup_path, journal.stage, &mut locked_rollout);
        return match compensation {
            Ok(()) => Err(CoreError::OperationFailed(format!(
                "THREAD_RECOVERY_VERIFY_FAILED: {error}; changes were compensated; backupPath={}",
                backup_path.display()
            ))),
            Err(compensation_error) => Err(CoreError::OperationFailed(format!(
                "THREAD_RECOVERY_VERIFY_FAILED: {error}; compensation failed: {compensation_error}; backupPath={}",
                backup_path.display()
            ))),
        };
    }

    remove_journal(paths)?;
    prune_backups(paths, Some(&backup_path))?;
    let mut preview = plan.preview;
    preview.database_provider = Some(target_provider.clone());
    preview.rollout_provider = Some(target_provider);
    preview.status = ThreadRecoveryStatus::Compatible;
    preview.can_apply = false;
    preview.reason = "The thread now matches the frozen recovery target".into();
    preview.changes.clear();
    Ok(ThreadRecoveryResult {
        changed: true,
        backup_path: Some(backup_path.display().to_string()),
        preview,
    })
}

fn build_recovery_plan(paths: &CodexPaths, thread_id: &str) -> Result<RecoveryPlan, CoreError> {
    build_recovery_plan_with_lookup(paths, thread_id, RolloutLookup::Indexed)
}

fn build_recovery_plan_with_lookup(
    paths: &CodexPaths,
    thread_id: &str,
    rollout_lookup: RolloutLookup,
) -> Result<RecoveryPlan, CoreError> {
    validate_thread_id(thread_id)?;
    let target = resolve_provider_target(&paths.config_path)?;
    let (pending_journal, coordination) = match inspect_pending_journal(paths) {
        PendingJournalInspection::None => (None, None),
        PendingJournalInspection::Ready(journal) => (Some(journal), None),
        PendingJournalInspection::Blocked(coordination) => (None, Some(coordination)),
    };
    let database = load_database_thread(&paths.codex_state_db_path, thread_id)?;
    let selected_rollout = if let Some(journal) = pending_journal.as_ref() {
        if journal.thread_id == thread_id {
            Some(read_pending_rollout_snapshot(paths, journal)?)
        } else {
            None
        }
    } else {
        select_rollout(paths, database.as_ref(), thread_id, rollout_lookup)?
    };
    let (rollout_path, rollout) = selected_rollout
        .map(|(path, snapshot)| (Some(path), Some(snapshot)))
        .unwrap_or((None, None));
    let database_provider = database.as_ref().map(|entry| entry.provider.clone());
    let rollout_provider = rollout.as_ref().and_then(|entry| entry.provider.clone());
    let (catalog_analysis, catalog_insert_plan) = if database.is_none() {
        let analysis = analyze_thread_catalog(
            paths,
            thread_id,
            &target,
            rollout_path.as_deref(),
            rollout.as_ref(),
        )?;
        let insert_plan = build_catalog_insert_plan(
            paths,
            thread_id,
            &target,
            rollout_path.as_deref(),
            rollout.as_ref(),
            &analysis,
        )?;
        (Some(analysis), insert_plan)
    } else {
        (None, None)
    };
    let mutation = if let Some(database) = database.as_ref() {
        Some(CatalogMutation::UpdateProvider {
            original_provider: database.provider.clone(),
            target_provider: target.configured.clone(),
        })
    } else {
        catalog_insert_plan.map(|plan| plan.mutation)
    };

    let mut changes = Vec::new();
    if target.valid {
        if let Some(provider) = &database_provider {
            if provider != &target.configured {
                changes.push(ThreadRecoveryChange {
                    target: "database".into(),
                    path: paths.codex_state_db_path.display().to_string(),
                    from_provider: Some(provider.clone()),
                    to_provider: target.configured.clone(),
                });
            }
        }
        if database.is_none() && mutation.is_some() {
            changes.push(ThreadRecoveryChange {
                target: "database".into(),
                path: paths.codex_state_db_path.display().to_string(),
                from_provider: None,
                to_provider: target.configured.clone(),
            });
        }
        if let (Some(path), Some(snapshot)) = (&rollout_path, &rollout) {
            if snapshot.provider.as_deref() != Some(target.configured.as_str()) {
                changes.push(ThreadRecoveryChange {
                    target: "rollout".into(),
                    path: path.display().to_string(),
                    from_provider: snapshot.provider.clone(),
                    to_provider: target.configured.clone(),
                });
            }
        }
    }

    let (status, can_apply, reason) = if let Some(coordination) = coordination.as_ref() {
        (
            ThreadRecoveryStatus::JournalBlocked,
            false,
            format!(
                "The recovery journal requires manual coordination: {}",
                coordination.detail
            ),
        )
    } else if let Some(journal) = pending_journal.as_ref() {
        let same_thread = journal.thread_id == thread_id;
        (
            ThreadRecoveryStatus::RecoveryPending,
            cfg!(target_os = "windows") && same_thread,
            if same_thread {
                "This thread has an unfinished recovery that must be reconciled first".into()
            } else {
                format!(
                    "An unfinished recovery belongs to thread {}; finish it before recovering another thread",
                    journal.thread_id
                )
            },
        )
    } else if !target.valid {
        (
            ThreadRecoveryStatus::InvalidTargetProvider,
            false,
            target
                .invalid_reason
                .clone()
                .unwrap_or_else(|| "The active provider is invalid".into()),
        )
    } else if database.is_none()
        && catalog_analysis
            .as_ref()
            .is_some_and(|analysis| !analysis.unsupported_columns.is_empty())
    {
        (
            ThreadRecoveryStatus::SchemaUnsupported,
            false,
            "The Codex threads schema cannot be reconstructed safely".into(),
        )
    } else if database.is_none()
        && catalog_analysis
            .as_ref()
            .is_some_and(|analysis| !analysis.missing_fields.is_empty())
    {
        (
            ThreadRecoveryStatus::InsufficientMetadata,
            false,
            "The rollout lacks verified values required by the Codex threads schema".into(),
        )
    } else if database.is_none()
        && catalog_analysis.as_ref().is_some_and(|analysis| {
            !analysis.mapping_errors.is_empty()
                || matches!(
                    analysis.relation,
                    ThreadCatalogRelation::Child | ThreadCatalogRelation::Ambiguous
                )
        })
    {
        (
            ThreadRecoveryStatus::InsufficientMetadata,
            false,
            "The rollout metadata cannot be mapped safely to a top-level Codex thread".into(),
        )
    } else if database.is_none() {
        (
            ThreadRecoveryStatus::MissingCatalog,
            cfg!(target_os = "windows") && mutation.is_some(),
            if cfg!(target_os = "windows") && mutation.is_some() {
                "The thread is absent from the Codex SQLite catalog and can be reconstructed from frozen metadata"
                    .into()
            } else {
                "The thread is absent from the Codex SQLite catalog; safe catalog insertion is unavailable"
                    .into()
            },
        )
    } else if !cfg!(target_os = "windows") {
        (
            ThreadRecoveryStatus::UnsupportedPlatform,
            false,
            "Safe thread recovery apply is currently supported only on Windows".into(),
        )
    } else if rollout_path.is_none() || rollout.is_none() {
        (
            ThreadRecoveryStatus::MissingRollout,
            false,
            "The thread rollout file is missing".into(),
        )
    } else if !rollout.as_ref().is_some_and(|entry| entry.has_session_meta) {
        (
            ThreadRecoveryStatus::InvalidRollout,
            false,
            "The rollout has no matching session metadata".into(),
        )
    } else if target.configured.eq_ignore_ascii_case("openai")
        && database_provider.as_deref() == Some("openai")
        && rollout_provider.as_deref() == Some("openai")
    {
        (
            ThreadRecoveryStatus::OfficialAccountUnverified,
            false,
            "Continue with the ChatGPT account that owns this thread".into(),
        )
    } else if database_provider.as_deref() == Some(target.configured.as_str())
        && rollout_provider.as_deref() == Some(target.configured.as_str())
    {
        (
            ThreadRecoveryStatus::Compatible,
            false,
            "The thread already matches the active Codex provider".into(),
        )
    } else if database_provider == rollout_provider {
        (
            ThreadRecoveryStatus::ProviderMismatch,
            true,
            "The thread provider differs from the active Codex provider".into(),
        )
    } else {
        (
            ThreadRecoveryStatus::ProviderConflict,
            false,
            "The SQLite catalog and rollout provider metadata disagree; automatic recovery is disabled"
                .into(),
        )
    };

    let revision = recovery_revision(
        thread_id,
        &target,
        database.as_ref(),
        rollout_path.as_deref(),
        rollout.as_ref(),
        pending_journal.as_ref(),
        coordination.as_ref(),
        catalog_analysis.as_ref(),
        mutation.as_ref(),
    )?;
    let catalog_fields = catalog_preview_fields(
        pending_journal
            .as_ref()
            .and_then(|journal| journal.mutation.as_ref())
            .or(mutation.as_ref()),
    );

    Ok(RecoveryPlan {
        preview: ThreadRecoveryPreview {
            thread_id: thread_id.to_owned(),
            configured_provider: target.configured.clone(),
            target_provider: target.configured,
            database_provider,
            rollout_provider,
            rollout_path: rollout_path.as_ref().map(|path| path.display().to_string()),
            revision,
            catalog_analysis,
            catalog_fields,
            coordination,
            status,
            can_apply,
            reason,
            changes,
        },
        config_generation: target.generation,
        rollout_path,
        rollout,
        pending_journal,
        mutation,
    })
}

fn resolve_provider_target(config_path: &Path) -> Result<ProviderTarget, CoreError> {
    let (text, generation) = match std::fs::read(config_path) {
        Ok(bytes) => {
            let mut generation_input = b"present\0".to_vec();
            generation_input.extend_from_slice(&bytes);
            let generation = sha256_hex(&generation_input);
            let text = String::from_utf8(bytes).map_err(|_| {
                CoreError::InvalidData("Codex config.toml is not valid UTF-8".into())
            })?;
            (text, generation)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ProviderTarget {
                configured: "openai".into(),
                generation: sha256_hex(b"missing\0"),
                valid: true,
                invalid_reason: None,
            });
        }
        Err(error) => return Err(error.into()),
    };
    let document: toml::Value = text.parse()?;
    let profile_provider = document
        .get("profile")
        .and_then(toml::Value::as_str)
        .and_then(|profile| document.get("profiles")?.get(profile))
        .and_then(|profile| profile.get("model_provider"))
        .and_then(toml::Value::as_str);
    let configured = profile_provider
        .or_else(|| document.get("model_provider").and_then(toml::Value::as_str))
        .filter(|provider| !provider.trim().is_empty())
        .unwrap_or("openai")
        .to_owned();
    if configured.eq_ignore_ascii_case("openai") {
        return Ok(ProviderTarget {
            configured: "openai".into(),
            generation,
            valid: true,
            invalid_reason: None,
        });
    }
    let defined = document
        .get("model_providers")
        .and_then(toml::Value::as_table)
        .and_then(|providers| providers.get(&configured))
        .is_some_and(toml::Value::is_table);
    Ok(ProviderTarget {
        configured: configured.clone(),
        generation,
        valid: defined,
        invalid_reason: (!defined).then(|| {
            format!(
                "The active provider {configured} has no model_providers.{configured} definition"
            )
        }),
    })
}

fn ensure_config_generation(paths: &CodexPaths, expected: &str) -> Result<(), CoreError> {
    let current = resolve_provider_target(&paths.config_path)?;
    if current.generation != expected {
        return Err(CoreError::OperationFailed(
            "THREAD_RECOVERY_CONFIG_CHANGED: Codex provider configuration changed during recovery"
                .into(),
        ));
    }
    Ok(())
}

fn recovery_revision(
    thread_id: &str,
    target: &ProviderTarget,
    database: Option<&DatabaseThread>,
    rollout_path: Option<&Path>,
    rollout: Option<&RolloutSnapshot>,
    journal: Option<&RecoveryJournal>,
    coordination: Option<&ThreadRecoveryCoordination>,
    catalog_analysis: Option<&ThreadCatalogAnalysis>,
    mutation: Option<&CatalogMutation>,
) -> Result<String, CoreError> {
    let state = serde_json::json!({
        "version": 1,
        "threadId": thread_id,
        "configuredProvider": target.configured,
        "configGeneration": target.generation,
        "targetValid": target.valid,
        "databaseProvider": database.map(|entry| entry.provider.as_str()),
        "databaseRolloutPath": database
            .and_then(|entry| entry.rollout_path.as_deref())
            .map(|path| path.display().to_string()),
        "rolloutPath": rollout_path.map(|path| path.display().to_string()),
        "rolloutProvider": rollout.and_then(|entry| entry.provider.as_deref()),
        "rolloutSha256": rollout.map(|entry| entry.sha256.as_str()),
        "rolloutLength": rollout.map(|entry| entry.length),
        "rolloutHasSessionMeta": rollout.map(|entry| entry.has_session_meta),
        "catalogAnalysis": catalog_analysis,
        "catalogMutation": mutation,
        "journal": journal.map(|entry| serde_json::json!({
            "threadId": entry.thread_id,
            "backupPath": entry.backup_path,
            "configGeneration": entry.config_generation,
            "stage": entry.stage,
        })),
        "coordination": coordination,
    });
    Ok(sha256_hex(&serde_json::to_vec(&state)?))
}

fn validate_thread_id(thread_id: &str) -> Result<(), CoreError> {
    if thread_id.is_empty()
        || thread_id.len() > 128
        || !thread_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(CoreError::InvalidData("Invalid thread id".into()));
    }
    Ok(())
}

fn open_database(path: &Path, read_only: bool) -> Result<Connection, CoreError> {
    if !path.is_file() {
        return Err(CoreError::NotFound(format!(
            "Codex state database: {}",
            path.display()
        )));
    }
    let flags = if read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX
    };
    Connection::open_with_flags(path, flags).map_err(sqlite_error)
}

fn load_database_thread(
    database_path: &Path,
    thread_id: &str,
) -> Result<Option<DatabaseThread>, CoreError> {
    if !database_path.is_file() {
        return Ok(None);
    }
    let connection = open_database(database_path, true)?;
    let columns = table_columns(&connection, "threads")?;
    if columns.is_empty() || !columns.contains("id") || !columns.contains("model_provider") {
        return Ok(None);
    }
    let rollout_expression = if columns.contains("rollout_path") {
        "rollout_path"
    } else {
        "NULL"
    };
    let sql =
        format!("SELECT model_provider, {rollout_expression} FROM threads WHERE id = ?1 LIMIT 1");
    connection
        .query_row(&sql, [thread_id], |row| {
            let provider: Option<String> = row.get(0)?;
            let rollout_path: Option<String> = row.get(1)?;
            Ok(DatabaseThread {
                provider: provider.unwrap_or_default(),
                rollout_path: rollout_path
                    .filter(|path| !path.is_empty())
                    .map(PathBuf::from),
            })
        })
        .optional()
        .map_err(sqlite_error)
}

fn query_database_provider(
    connection: &Connection,
    thread_id: &str,
) -> Result<Option<String>, CoreError> {
    connection
        .query_row(
            "SELECT model_provider FROM threads WHERE id = ?1",
            [thread_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(sqlite_error)
}

fn table_columns(connection: &Connection, table: &str) -> Result<HashSet<String>, CoreError> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(sqlite_error)?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(sqlite_error)?;
    let mut columns = HashSet::new();
    for row in rows {
        columns.insert(row.map_err(sqlite_error)?);
    }
    Ok(columns)
}

fn analyze_thread_catalog(
    paths: &CodexPaths,
    thread_id: &str,
    target: &ProviderTarget,
    rollout_path: Option<&Path>,
    rollout: Option<&RolloutSnapshot>,
) -> Result<ThreadCatalogAnalysis, CoreError> {
    if !paths.codex_state_db_path.is_file() {
        return Ok(ThreadCatalogAnalysis {
            schema_signature: None,
            ready: false,
            missing_fields: Vec::new(),
            unsupported_columns: vec!["threads (database missing)".into()],
            relation: ThreadCatalogRelation::Ambiguous,
            mapping_errors: vec!["The Codex state database is unavailable".into()],
        });
    }

    let connection = open_database(&paths.codex_state_db_path, true)?;
    let columns = thread_schema_columns(&connection)?;
    let schema_signature = Some(thread_schema_signature(&columns)?);
    let mut unsupported_columns = Vec::new();
    let triggers = thread_trigger_names(&connection)?;
    if !triggers.is_empty() {
        unsupported_columns.push(format!(
            "threads triggers are not supported ({})",
            triggers.join(", ")
        ));
    }

    let id_column = columns.iter().find(|column| column.name == "id");
    let primary_key_columns = columns
        .iter()
        .filter(|column| column.primary_key > 0)
        .count();
    match id_column {
        Some(column)
            if column.affinity == SqliteAffinity::Text
                && column.primary_key == 1
                && primary_key_columns == 1
                && column.hidden == 0 => {}
        Some(_) => {
            unsupported_columns.push("id (must be the sole visible TEXT primary key)".into())
        }
        None => unsupported_columns.push("id (missing TEXT primary key)".into()),
    }
    match columns
        .iter()
        .find(|column| column.name == "model_provider")
    {
        Some(column) if column.affinity == SqliteAffinity::Text && column.hidden == 0 => {}
        Some(_) => {
            unsupported_columns.push("model_provider (must be a visible TEXT column)".into())
        }
        None => unsupported_columns.push("model_provider (missing TEXT column)".into()),
    }

    if columns.is_empty() {
        unsupported_columns.push("threads (table missing)".into());
    }

    for column in &columns {
        if column.hidden != 0 {
            unsupported_columns.push(format!(
                "{} (hidden/generated column is not supported)",
                column.name
            ));
            continue;
        }
        if column.default_value.is_some() {
            if let Err(error) = frozen_default_for_column(column) {
                unsupported_columns.push(format!("{} ({error})", column.name));
                continue;
            }
        }
        if let Some(expected) = expected_catalog_affinity(&column.name) {
            if column.affinity != expected {
                unsupported_columns.push(format!(
                    "{} (expected {} affinity, found {})",
                    column.name,
                    affinity_name(expected),
                    affinity_name(column.affinity)
                ));
            }
        } else if column.not_null && column.default_value.is_none() && column.primary_key == 0 {
            unsupported_columns.push(format!(
                "{} (unknown NOT NULL column without a default)",
                column.name
            ));
        }
    }

    let metadata =
        catalog_metadata_analysis(paths, thread_id, target, rollout_path, rollout, &connection)?;
    let mut missing_fields = columns
        .iter()
        .filter(|column| {
            expected_catalog_affinity(&column.name).is_some()
                && (column.primary_key > 0
                    || (column.not_null
                        && frozen_default_for_column(column).ok().flatten().is_none()))
                && !metadata.fields.contains(&column.name)
        })
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();

    unsupported_columns.sort();
    unsupported_columns.dedup();
    missing_fields.sort();
    missing_fields.dedup();
    let relation = metadata
        .relation
        .unwrap_or(ThreadCatalogRelation::Ambiguous);
    let ready = unsupported_columns.is_empty()
        && missing_fields.is_empty()
        && metadata.mapping_errors.is_empty()
        && matches!(
            relation,
            ThreadCatalogRelation::TopLevel | ThreadCatalogRelation::UserFork
        );
    Ok(ThreadCatalogAnalysis {
        schema_signature,
        ready,
        missing_fields,
        unsupported_columns,
        relation,
        mapping_errors: metadata.mapping_errors,
    })
}

fn thread_schema_columns(connection: &Connection) -> Result<Vec<ThreadSchemaColumn>, CoreError> {
    let mut statement = connection
        .prepare("PRAGMA table_xinfo(\"threads\")")
        .map_err(sqlite_error)?;
    let rows = statement
        .query_map([], |row| {
            let declared_type: String = row.get(2)?;
            Ok(ThreadSchemaColumn {
                ordinal: row.get(0)?,
                name: row.get(1)?,
                affinity: sqlite_affinity(&declared_type),
                declared_type: declared_type.trim().to_ascii_uppercase(),
                not_null: row.get::<_, i64>(3)? != 0,
                default_value: row.get(4)?,
                primary_key: row.get(5)?,
                hidden: row.get(6)?,
            })
        })
        .map_err(sqlite_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(sqlite_error)
}

fn thread_trigger_names(connection: &Connection) -> Result<Vec<String>, CoreError> {
    let mut statement = connection
        .prepare(
            "SELECT name FROM sqlite_master WHERE type = 'trigger' AND tbl_name = 'threads' ORDER BY name",
        )
        .map_err(sqlite_error)?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(sqlite_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(sqlite_error)
}

fn sqlite_affinity(declared_type: &str) -> SqliteAffinity {
    let declared_type = declared_type.trim().to_ascii_uppercase();
    if declared_type.contains("INT") {
        SqliteAffinity::Integer
    } else if declared_type.contains("CHAR")
        || declared_type.contains("CLOB")
        || declared_type.contains("TEXT")
    {
        SqliteAffinity::Text
    } else if declared_type.is_empty() || declared_type.contains("BLOB") {
        SqliteAffinity::Blob
    } else if declared_type.contains("REAL")
        || declared_type.contains("FLOA")
        || declared_type.contains("DOUB")
    {
        SqliteAffinity::Real
    } else {
        SqliteAffinity::Numeric
    }
}

fn affinity_name(affinity: SqliteAffinity) -> &'static str {
    match affinity {
        SqliteAffinity::Integer => "INTEGER",
        SqliteAffinity::Text => "TEXT",
        SqliteAffinity::Blob => "BLOB",
        SqliteAffinity::Real => "REAL",
        SqliteAffinity::Numeric => "NUMERIC",
    }
}

fn thread_schema_signature(columns: &[ThreadSchemaColumn]) -> Result<String, CoreError> {
    Ok(format!("v1:{}", sha256_hex(&serde_json::to_vec(columns)?)))
}

fn expected_catalog_affinity(name: &str) -> Option<SqliteAffinity> {
    match name {
        "id" | "rollout_path" | "source" | "model_provider" | "cwd" | "title"
        | "sandbox_policy" | "approval_mode" | "git_sha" | "git_branch" | "git_origin_url"
        | "cli_version" | "first_user_message" | "agent_nickname" | "agent_role"
        | "memory_mode" | "model" | "reasoning_effort" | "agent_path" | "thread_source"
        | "preview" | "history_mode" | "name" | "thread_section_id" | "project_id"
        | "originator" => Some(SqliteAffinity::Text),
        "created_at"
        | "updated_at"
        | "tokens_used"
        | "has_user_event"
        | "archived"
        | "archived_at"
        | "created_at_ms"
        | "updated_at_ms"
        | "recency_at"
        | "recency_at_ms"
        | "is_pinned"
        | "section_position"
        | "section_entered_at_ms" => Some(SqliteAffinity::Integer),
        "daybreak_enabled" => Some(SqliteAffinity::Numeric),
        _ => None,
    }
}

fn normalize_catalog_value_for_affinity(
    value: CatalogSqlValue,
    affinity: SqliteAffinity,
) -> Result<CatalogSqlValue, String> {
    match (value, affinity) {
        (CatalogSqlValue::Null, _) => Ok(CatalogSqlValue::Null),
        (CatalogSqlValue::Text(value), SqliteAffinity::Text) => Ok(CatalogSqlValue::Text(value)),
        (CatalogSqlValue::Integer(value), SqliteAffinity::Text) => {
            Ok(CatalogSqlValue::Text(value.to_string()))
        }
        (CatalogSqlValue::Integer(value), SqliteAffinity::Integer | SqliteAffinity::Numeric) => {
            Ok(CatalogSqlValue::Integer(value))
        }
        (CatalogSqlValue::Integer(value), SqliteAffinity::Real) => {
            Ok(CatalogSqlValue::RealBits((value as f64).to_bits()))
        }
        (CatalogSqlValue::RealBits(value), SqliteAffinity::Real | SqliteAffinity::Numeric) => {
            Ok(CatalogSqlValue::RealBits(value))
        }
        (CatalogSqlValue::BlobBase64(value), SqliteAffinity::Blob) => {
            Ok(CatalogSqlValue::BlobBase64(value))
        }
        (CatalogSqlValue::TextBytesBase64(_), _) => {
            Err("raw SQLite TEXT bytes cannot be planned for insertion".into())
        }
        (_, affinity) => Err(format!(
            "value storage class cannot be frozen safely for {} affinity",
            affinity_name(affinity)
        )),
    }
}

fn frozen_default_for_column(
    column: &ThreadSchemaColumn,
) -> Result<Option<CatalogSqlValue>, String> {
    let Some(default) = column.default_value.as_deref() else {
        return Ok(None);
    };
    let value = parse_frozen_default(default)?;
    if column.not_null && value == CatalogSqlValue::Null {
        return Err("NULL default cannot satisfy a NOT NULL column".into());
    }
    normalize_catalog_value_for_affinity(value, column.affinity).map(Some)
}

fn parse_frozen_default(value: &str) -> Result<CatalogSqlValue, String> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("NULL") {
        return Ok(CatalogSqlValue::Null);
    }
    if let Ok(integer) = value.parse::<i64>() {
        return Ok(CatalogSqlValue::Integer(integer));
    }
    if let Ok(real) = value.parse::<f64>() {
        if real.is_finite() {
            return Ok(CatalogSqlValue::RealBits(real.to_bits()));
        }
        return Err("non-finite numeric default".into());
    }
    if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        let inner = &value[1..value.len() - 1];
        let mut decoded = String::with_capacity(inner.len());
        let mut chars = inner.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\'' && chars.peek() == Some(&'\'') {
                chars.next();
                decoded.push('\'');
            } else if ch == '\'' {
                return Err("malformed single-quoted text default".into());
            } else {
                decoded.push(ch);
            }
        }
        return Ok(CatalogSqlValue::Text(decoded));
    }
    Err("dynamic or unsupported SQLite default".into())
}

fn catalog_sql_value_to_rusqlite(value: &CatalogSqlValue) -> Result<SqlValue, CoreError> {
    Ok(match value {
        CatalogSqlValue::Null => SqlValue::Null,
        CatalogSqlValue::Integer(value) => SqlValue::Integer(*value),
        CatalogSqlValue::Text(value) => SqlValue::Text(value.clone()),
        CatalogSqlValue::TextBytesBase64(_) => {
            return Err(CoreError::InvalidData(
                "Raw SQLite TEXT bytes cannot be used as an insertion value".into(),
            ));
        }
        CatalogSqlValue::RealBits(bits) => {
            let value = f64::from_bits(*bits);
            if !value.is_finite() {
                return Err(CoreError::InvalidData(
                    "Catalog value contains a non-finite real".into(),
                ));
            }
            SqlValue::Real(value)
        }
        CatalogSqlValue::BlobBase64(value) => {
            SqlValue::Blob(BASE64.decode(value).map_err(|_| {
                CoreError::InvalidData("Catalog value contains invalid base64".into())
            })?)
        }
    })
}

fn catalog_sql_value_from_ref(value: ValueRef<'_>) -> CatalogSqlValue {
    match value {
        ValueRef::Null => CatalogSqlValue::Null,
        ValueRef::Integer(value) => CatalogSqlValue::Integer(value),
        ValueRef::Real(value) => CatalogSqlValue::RealBits(value.to_bits()),
        ValueRef::Text(value) => match std::str::from_utf8(value) {
            Ok(value) => CatalogSqlValue::Text(value.to_owned()),
            Err(_) => CatalogSqlValue::TextBytesBase64(BASE64.encode(value)),
        },
        ValueRef::Blob(value) => CatalogSqlValue::BlobBase64(BASE64.encode(value)),
    }
}

fn catalog_preview_fields(mutation: Option<&CatalogMutation>) -> Vec<ThreadRecoveryCatalogField> {
    let Some(CatalogMutation::InsertRow { columns, .. }) = mutation else {
        return Vec::new();
    };
    let mut columns = columns.iter().collect::<Vec<_>>();
    columns.sort_by_key(|column| column.ordinal);
    columns
        .into_iter()
        .map(|column| ThreadRecoveryCatalogField {
            name: column.name.clone(),
            value: match &column.value {
                CatalogSqlValue::Null => "NULL".into(),
                CatalogSqlValue::Integer(value) => value.to_string(),
                CatalogSqlValue::Text(value) => value.clone(),
                CatalogSqlValue::TextBytesBase64(value) => format!("base64:{value}"),
                CatalogSqlValue::RealBits(value) => f64::from_bits(*value).to_string(),
                CatalogSqlValue::BlobBase64(value) => format!("base64:{value}"),
            },
        })
        .collect()
}

fn catalog_row_fingerprint(
    schema_signature: &str,
    columns: &[CatalogColumnValue],
) -> Result<String, CoreError> {
    let mut ordered = columns.to_vec();
    ordered.sort_by_key(|column| column.ordinal);
    Ok(sha256_hex(&serde_json::to_vec(&serde_json::json!({
        "version": 1,
        "schemaSignature": schema_signature,
        "columns": ordered,
    }))?))
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn build_catalog_insert_plan(
    paths: &CodexPaths,
    thread_id: &str,
    target: &ProviderTarget,
    rollout_path: Option<&Path>,
    rollout: Option<&RolloutSnapshot>,
    public_analysis: &ThreadCatalogAnalysis,
) -> Result<Option<CatalogInsertPlan>, CoreError> {
    if !public_analysis.ready {
        return Ok(None);
    }
    let connection = open_database(&paths.codex_state_db_path, true)?;
    let columns = thread_schema_columns(&connection)?;
    let schema_signature = thread_schema_signature(&columns)?;
    if public_analysis.schema_signature.as_deref() != Some(schema_signature.as_str()) {
        return Err(CoreError::OperationFailed(
            "THREAD_RECOVERY_SCHEMA_CHANGED: threads schema changed during preview".into(),
        ));
    }
    let metadata =
        catalog_metadata_analysis(paths, thread_id, target, rollout_path, rollout, &connection)?;
    if !metadata.mapping_errors.is_empty()
        || !matches!(
            metadata.relation,
            Some(ThreadCatalogRelation::TopLevel | ThreadCatalogRelation::UserFork)
        )
    {
        return Ok(None);
    }
    let mut frozen = Vec::with_capacity(columns.len());
    for column in columns {
        if column.hidden != 0 {
            return Ok(None);
        }
        let value = if let Some(value) = metadata.values.get(&column.name) {
            normalize_catalog_value_for_affinity(value.clone(), column.affinity).ok()
        } else if column.default_value.is_some() {
            match frozen_default_for_column(&column) {
                Ok(value) => value,
                _ => return Ok(None),
            }
        } else if !column.not_null && column.primary_key == 0 {
            Some(CatalogSqlValue::Null)
        } else {
            return Ok(None);
        };
        let Some(value) = value else {
            return Ok(None);
        };
        if column.not_null && value == CatalogSqlValue::Null {
            return Ok(None);
        }
        frozen.push(CatalogColumnValue {
            ordinal: column.ordinal,
            name: column.name,
            affinity: column.affinity,
            value,
        });
    }
    frozen.sort_by_key(|column| column.ordinal);
    let row_fingerprint = catalog_row_fingerprint(&schema_signature, &frozen)?;
    Ok(Some(CatalogInsertPlan {
        mutation: CatalogMutation::InsertRow {
            schema_signature,
            columns: frozen,
            row_fingerprint,
        },
    }))
}

fn read_catalog_row(
    connection: &Connection,
    thread_id: &str,
    schema_signature: &str,
    expected_columns: &[CatalogColumnValue],
) -> Result<Option<(Vec<CatalogColumnValue>, String)>, CoreError> {
    let columns = thread_schema_columns(connection)?;
    if thread_schema_signature(&columns)? != schema_signature {
        return Err(CoreError::OperationFailed(
            "THREAD_RECOVERY_SCHEMA_CHANGED: threads schema no longer matches the frozen plan"
                .into(),
        ));
    }
    if columns.len() != expected_columns.len()
        || columns
            .iter()
            .zip(expected_columns)
            .any(|(actual, expected)| {
                actual.ordinal != expected.ordinal
                    || actual.name != expected.name
                    || actual.affinity != expected.affinity
                    || actual.hidden != 0
            })
    {
        return Err(CoreError::OperationFailed(
            "THREAD_RECOVERY_SCHEMA_CHANGED: threads columns differ from the frozen plan".into(),
        ));
    }
    let select = expected_columns
        .iter()
        .map(|column| quote_identifier(&column.name))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("SELECT {select} FROM threads WHERE id = ?1 LIMIT 2");
    let mut statement = connection.prepare(&sql).map_err(sqlite_error)?;
    let mut rows = statement.query([thread_id]).map_err(sqlite_error)?;
    let Some(row) = rows.next().map_err(sqlite_error)? else {
        return Ok(None);
    };
    let mut values = Vec::with_capacity(expected_columns.len());
    for (index, column) in expected_columns.iter().enumerate() {
        values.push(CatalogColumnValue {
            ordinal: column.ordinal,
            name: column.name.clone(),
            affinity: column.affinity,
            value: catalog_sql_value_from_ref(row.get_ref(index).map_err(sqlite_error)?),
        });
    }
    if rows.next().map_err(sqlite_error)?.is_some() {
        return Err(CoreError::OperationFailed(
            "THREAD_RECOVERY_CATALOG_CONFLICT: duplicate thread IDs found".into(),
        ));
    }
    let fingerprint = catalog_row_fingerprint(schema_signature, &values)?;
    Ok(Some((values, fingerprint)))
}

fn insert_catalog_row(
    connection: &Connection,
    thread_id: &str,
    schema_signature: &str,
    columns: &[CatalogColumnValue],
    expected_fingerprint: &str,
) -> Result<(), CoreError> {
    let triggers = thread_trigger_names(connection)?;
    if !triggers.is_empty() {
        return Err(CoreError::OperationFailed(format!(
            "THREAD_RECOVERY_SCHEMA_CHANGED: threads triggers appeared after preview ({})",
            triggers.join(", ")
        )));
    }
    if query_database_provider(connection, thread_id)?.is_some() {
        return Err(CoreError::OperationFailed(
            "THREAD_RECOVERY_CHANGED: a catalog row appeared after preview".into(),
        ));
    }
    let names = columns
        .iter()
        .map(|column| quote_identifier(&column.name))
        .collect::<Vec<_>>()
        .join(", ");
    let placeholders = (1..=columns.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("INSERT INTO threads ({names}) VALUES ({placeholders})");
    let values = columns
        .iter()
        .map(|column| catalog_sql_value_to_rusqlite(&column.value))
        .collect::<Result<Vec<_>, _>>()?;
    let changed = connection
        .execute(&sql, params_from_iter(values.iter()))
        .map_err(sqlite_error)?;
    if changed != 1 {
        return Err(CoreError::OperationFailed(
            "THREAD_RECOVERY_CHANGED: catalog insertion did not affect one row".into(),
        ));
    }
    let (_, fingerprint) = read_catalog_row(connection, thread_id, schema_signature, columns)?
        .ok_or_else(|| CoreError::NotFound("Inserted thread catalog row".into()))?;
    if fingerprint != expected_fingerprint {
        return Err(CoreError::OperationFailed(
            "THREAD_RECOVERY_CATALOG_CONFLICT: inserted row differs from the frozen plan".into(),
        ));
    }
    Ok(())
}

fn catalog_metadata_analysis(
    paths: &CodexPaths,
    thread_id: &str,
    target: &ProviderTarget,
    rollout_path: Option<&Path>,
    rollout: Option<&RolloutSnapshot>,
    connection: &Connection,
) -> Result<CatalogMetadataAnalysis, CoreError> {
    let mut analysis = CatalogMetadataAnalysis::default();
    if target.valid {
        analysis.fields.insert("model_provider".into());
        analysis.values.insert(
            "model_provider".into(),
            CatalogSqlValue::Text(target.configured.clone()),
        );
    }
    if let Some(path) = rollout_path {
        analysis.fields.insert("rollout_path".into());
        analysis.fields.insert("archived".into());
        analysis.values.insert(
            "rollout_path".into(),
            CatalogSqlValue::Text(path.display().to_string()),
        );
        let is_archived = std::fs::canonicalize(&paths.archived_sessions_dir)
            .ok()
            .is_some_and(|root| path.starts_with(root));
        analysis.values.insert(
            "archived".into(),
            CatalogSqlValue::Integer(i64::from(is_archived)),
        );
    }
    let Some(rollout) = rollout else {
        analysis.relation = Some(ThreadCatalogRelation::Ambiguous);
        analysis
            .mapping_errors
            .push("The rollout is unavailable for catalog metadata mapping".into());
        return Ok(analysis);
    };
    if !rollout.has_session_meta {
        analysis.relation = Some(ThreadCatalogRelation::Ambiguous);
        analysis
            .mapping_errors
            .push("The rollout lacks matching session metadata".into());
        return Ok(analysis);
    }

    let text = std::str::from_utf8(&rollout.bytes)
        .map_err(|_| CoreError::InvalidData("Thread rollout is not valid UTF-8".into()))?;
    let mut first_timestamp_ms = None;
    let mut first_user_message = None;
    let mut policy_fields = CatalogPolicyFields {
        sandbox_policy: None,
        approval_mode: None,
    };
    let mut relation_evidence = Vec::new();
    let mut forked_from = false;
    let mut session_meta_seen = false;
    for (line_index, line) in text.lines().enumerate() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if let Some(timestamp) = value.get("timestamp").and_then(serde_json::Value::as_str) {
            if let Ok(timestamp) = DateTime::parse_from_rfc3339(timestamp) {
                first_timestamp_ms.get_or_insert(timestamp.timestamp_millis());
            }
        }
        if is_matching_session_meta(&value, thread_id) {
            session_meta_seen = true;
            analysis.fields.insert("id".into());
            analysis
                .values
                .insert("id".into(), CatalogSqlValue::Text(thread_id.to_owned()));
            if let Some(payload) = value.get("payload") {
                for (column, key) in [
                    ("cwd", "cwd"),
                    ("title", "title"),
                    ("cli_version", "cli_version"),
                    ("history_mode", "history_mode"),
                    ("thread_source", "thread_source"),
                    ("originator", "originator"),
                    ("agent_nickname", "agent_nickname"),
                    ("agent_role", "agent_role"),
                    ("agent_path", "agent_path"),
                    ("memory_mode", "memory_mode"),
                    ("model", "model"),
                    ("name", "name"),
                ] {
                    if let Some(value) = payload
                        .get(key)
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    {
                        analysis.fields.insert(column.into());
                        analysis
                            .values
                            .insert(column.into(), CatalogSqlValue::Text(value.to_owned()));
                    }
                }
                map_session_source(
                    payload,
                    &mut analysis.fields,
                    &mut analysis.values,
                    &mut analysis.mapping_errors,
                    &mut relation_evidence,
                );
                if let Some(thread_source) = payload
                    .get("thread_source")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    match thread_source {
                        "subagent"
                        | "sub_agent"
                        | "guardian_review"
                        | "memory_consolidation"
                        | "internal" => relation_evidence
                            .push(format!("session_meta.thread_source={thread_source}")),
                        "cli" | "vscode" | "exec" | "mcp" | "app_server" | "app-server"
                        | "desktop" | "remote_control" | "remote-control" | "user" | "fork"
                        | "forked" | "main" | "top_level" => {}
                        other => analysis.mapping_errors.push(format!(
                            "session_meta.thread_source uses an unsupported value: {other}"
                        )),
                    }
                }
                if non_empty_string(payload.get("parent_thread_id")).is_some() {
                    relation_evidence.push("session_meta.parent_thread_id".into());
                }
                forked_from = non_empty_string(payload.get("forked_from_id")).is_some();
                if let Some(git) = payload.get("git").and_then(serde_json::Value::as_object) {
                    for (column, key) in [
                        ("git_sha", "commit_hash"),
                        ("git_branch", "branch"),
                        ("git_origin_url", "repository_url"),
                    ] {
                        if let Some(value) = git
                            .get(key)
                            .and_then(serde_json::Value::as_str)
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                        {
                            analysis.fields.insert(column.into());
                            analysis
                                .values
                                .insert(column.into(), CatalogSqlValue::Text(value.to_owned()));
                        }
                    }
                }
            }
        }
        if first_user_message.is_none() {
            first_user_message = catalog_user_message(&value);
        }
        let record_type = value.get("type").and_then(serde_json::Value::as_str);
        let payload = value.get("payload");
        let settings = if record_type == Some("turn_context") {
            payload
        } else if record_type == Some("event_msg")
            && payload
                .and_then(|payload| payload.get("type"))
                .and_then(serde_json::Value::as_str)
                == Some("thread_settings_applied")
        {
            payload.and_then(|payload| payload.get("thread_settings"))
        } else {
            None
        };
        if let Some(payload) = settings {
            if let Some(value) = payload
                .get("model")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                analysis.fields.insert("model".into());
                analysis
                    .values
                    .insert("model".into(), CatalogSqlValue::Text(value.to_owned()));
            }
            if let Some(value) = payload
                .get("effort")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                analysis.fields.insert("reasoning_effort".into());
                analysis.values.insert(
                    "reasoning_effort".into(),
                    CatalogSqlValue::Text(value.to_owned()),
                );
            }
            match map_catalog_policy_fields(payload) {
                Ok(mapped) => policy_fields = mapped,
                Err(error) => analysis
                    .mapping_errors
                    .push(format!("line {}: {error}", line_index + 1)),
            }
        }
    }
    if policy_fields.sandbox_policy.is_some() {
        analysis.fields.insert("sandbox_policy".into());
        analysis.values.insert(
            "sandbox_policy".into(),
            CatalogSqlValue::Text(policy_fields.sandbox_policy.clone().unwrap()),
        );
    }
    if policy_fields.approval_mode.is_some() {
        analysis.fields.insert("approval_mode".into());
        analysis.values.insert(
            "approval_mode".into(),
            CatalogSqlValue::Text(policy_fields.approval_mode.clone().unwrap()),
        );
    }
    if let Some(timestamp_ms) = first_timestamp_ms {
        for (field, value) in [
            ("created_at", timestamp_ms / 1000),
            ("updated_at", timestamp_ms / 1000),
            ("created_at_ms", timestamp_ms),
            ("updated_at_ms", timestamp_ms),
            ("recency_at", timestamp_ms / 1000),
            ("recency_at_ms", timestamp_ms),
        ] {
            analysis.fields.insert(field.into());
            analysis
                .values
                .insert(field.into(), CatalogSqlValue::Integer(value));
        }
    }
    analysis.fields.insert("has_user_event".into());
    analysis.values.insert(
        "has_user_event".into(),
        CatalogSqlValue::Integer(i64::from(first_user_message.is_some())),
    );
    if let Some(first_user_message) = first_user_message {
        for field in ["title", "first_user_message", "preview"] {
            analysis.fields.insert(field.into());
            analysis.values.insert(
                field.into(),
                CatalogSqlValue::Text(first_user_message.clone()),
            );
        }
    }
    relation_evidence.extend(catalog_child_reference_evidence(connection, thread_id)?);
    let has_agent_metadata = ["agent_path", "agent_role", "agent_nickname"]
        .iter()
        .any(|field| analysis.fields.contains(*field));
    analysis.relation = Some(if !relation_evidence.is_empty() {
        analysis.mapping_errors.push(format!(
            "Sub-agent or child thread evidence prevents top-level catalog recovery: {}",
            relation_evidence.join(", ")
        ));
        ThreadCatalogRelation::Child
    } else if has_agent_metadata {
        analysis
            .mapping_errors
            .push("Agent metadata is present without a verified parent or top-level source".into());
        ThreadCatalogRelation::Ambiguous
    } else if forked_from {
        ThreadCatalogRelation::UserFork
    } else if session_meta_seen {
        ThreadCatalogRelation::TopLevel
    } else {
        analysis
            .mapping_errors
            .push("No matching session metadata was mapped".into());
        ThreadCatalogRelation::Ambiguous
    });
    analysis.mapping_errors.sort();
    analysis.mapping_errors.dedup();
    Ok(analysis)
}

fn non_empty_string(value: Option<&serde_json::Value>) -> Option<&str> {
    value
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn map_session_source(
    payload: &serde_json::Value,
    fields: &mut HashSet<String>,
    values: &mut BTreeMap<String, CatalogSqlValue>,
    mapping_errors: &mut Vec<String>,
    relation_evidence: &mut Vec<String>,
) {
    let Some(source) = payload.get("source") else {
        return;
    };
    match source {
        serde_json::Value::String(value) if !value.trim().is_empty() => {
            let normalized = value.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "subagent"
                | "sub_agent"
                | "guardian_review"
                | "memory_consolidation"
                | "internal" => {
                    fields.insert("source".into());
                    values.insert("source".into(), CatalogSqlValue::Text(value.trim().into()));
                    relation_evidence.push(format!("session_meta.source={value}"));
                }
                "cli" | "vscode" | "exec" | "mcp" | "app_server" | "app-server" | "desktop"
                | "remote_control" | "remote-control" | "user" | "fork" | "forked" => {
                    fields.insert("source".into());
                    values.insert("source".into(), CatalogSqlValue::Text(value.trim().into()));
                }
                _ => mapping_errors.push(format!(
                    "session_meta.source uses an unsupported value: {value}"
                )),
            }
        }
        serde_json::Value::Object(object) if object.len() == 1 => {
            let (kind, detail) = object.iter().next().expect("one source entry");
            match kind.as_str() {
                "subagent" | "sub_agent" | "internal" => {
                    if !detail.is_object() && !detail.is_string() {
                        mapping_errors.push(format!(
                            "session_meta.source.{kind} has an invalid representation"
                        ));
                        return;
                    }
                    fields.insert("source".into());
                    values.insert(
                        "source".into(),
                        CatalogSqlValue::Text(serde_json::to_string(source).unwrap_or_default()),
                    );
                    relation_evidence.push(format!("session_meta.source.{kind}"));
                }
                _ => mapping_errors.push(format!(
                    "session_meta.source uses an unsupported structured variant: {kind}"
                )),
            }
        }
        _ => mapping_errors.push("session_meta.source has an invalid representation".into()),
    }
}

fn catalog_child_reference_evidence(
    connection: &Connection,
    thread_id: &str,
) -> Result<Vec<String>, CoreError> {
    let mut evidence = Vec::new();
    let spawn_columns = table_columns(connection, "thread_spawn_edges")?;
    if spawn_columns.contains("child_thread_id") {
        let exists = connection
            .query_row(
                "SELECT 1 FROM thread_spawn_edges WHERE child_thread_id = ?1 LIMIT 1",
                [thread_id],
                |_| Ok(()),
            )
            .optional()
            .map_err(sqlite_error)?
            .is_some();
        if exists {
            evidence.push("thread_spawn_edges.child_thread_id".into());
        }
    }
    let job_columns = table_columns(connection, "agent_job_items")?;
    if job_columns.contains("assigned_thread_id") {
        let exists = connection
            .query_row(
                "SELECT 1 FROM agent_job_items WHERE assigned_thread_id = ?1 LIMIT 1",
                [thread_id],
                |_| Ok(()),
            )
            .optional()
            .map_err(sqlite_error)?
            .is_some();
        if exists {
            evidence.push("agent_job_items.assigned_thread_id".into());
        }
    }
    Ok(evidence)
}

fn sqlite_error(error: rusqlite::Error) -> CoreError {
    CoreError::OperationFailed(format!("SQLite error: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const THREAD_ID: &str = "01999999-1111-7222-8333-444444444444";

    fn config_for(provider: &str) -> String {
        if provider == "openai" {
            "model_provider='openai'\n".into()
        } else {
            format!(
                "model_provider='{provider}'\n[model_providers.{provider}]\nname='Fixture'\nbase_url='https://fixture.invalid'\n"
            )
        }
    }

    fn fixture(
        configured_provider: &str,
        stored_provider: &str,
    ) -> (tempfile::TempDir, CodexPaths) {
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().join("codex"));
        std::fs::create_dir_all(&paths.sessions_dir).unwrap();
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        std::fs::write(&paths.config_path, config_for(configured_provider)).unwrap();
        let rollout = paths
            .sessions_dir
            .join(format!("rollout-{THREAD_ID}.jsonl"));
        let meta = serde_json::json!({
            "timestamp": "2026-09-25T00:00:00Z",
            "type": "session_meta",
            "payload": {
                "id": THREAD_ID,
                "model_provider": stored_provider,
                "cwd": "C:/fixture",
                "title": "你好"
            }
        });
        std::fs::write(
            &rollout,
            format!("{}\n", serde_json::to_string(&meta).unwrap()),
        )
        .unwrap();
        let connection = Connection::open(&paths.codex_state_db_path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, model_provider TEXT, rollout_path TEXT);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO threads (id, model_provider, rollout_path) VALUES (?1, ?2, ?3)",
                (THREAD_ID, stored_provider, rollout.display().to_string()),
            )
            .unwrap();
        (temp, paths)
    }

    fn replace_threads_schema(paths: &CodexPaths, schema: &str) {
        let connection = Connection::open(&paths.codex_state_db_path).unwrap();
        connection.execute_batch("DROP TABLE threads;").unwrap();
        connection.execute_batch(schema).unwrap();
    }

    fn prepare_recovery_backup(
        paths: &CodexPaths,
    ) -> (PathBuf, RolloutSnapshot, Vec<u8>, PathBuf, String) {
        let plan = build_recovery_plan(paths, THREAD_ID).unwrap();
        let rollout_path = plan.rollout_path.unwrap();
        let rollout = plan.rollout.unwrap();
        let updated =
            rewrite_rollout_provider(&rollout.bytes, THREAD_ID, "agentharbor_relay").unwrap();
        let backup_path = write_backup(
            paths,
            THREAD_ID,
            "openai",
            &rollout_path,
            &rollout,
            "agentharbor_relay",
            &plan.config_generation,
            &sha256_hex(&updated),
            updated.len() as u64,
        )
        .unwrap();
        (
            rollout_path,
            rollout,
            updated,
            backup_path,
            plan.config_generation,
        )
    }

    #[cfg(target_os = "windows")]
    fn prepare_insert_recovery_backup(
        paths: &CodexPaths,
    ) -> (
        PathBuf,
        RolloutSnapshot,
        Vec<u8>,
        PathBuf,
        String,
        CatalogMutation,
    ) {
        Connection::open(&paths.codex_state_db_path)
            .unwrap()
            .execute("DELETE FROM threads", [])
            .unwrap();
        let plan = build_recovery_plan(paths, THREAD_ID).unwrap();
        let rollout_path = plan.rollout_path.unwrap();
        let rollout = plan.rollout.unwrap();
        let mutation = plan.mutation.unwrap();
        let updated =
            rewrite_rollout_provider(&rollout.bytes, THREAD_ID, &plan.preview.target_provider)
                .unwrap();
        let backup_path = write_backup_with_mutation(
            paths,
            THREAD_ID,
            "",
            &rollout_path,
            &rollout,
            &plan.preview.target_provider,
            &plan.config_generation,
            &sha256_hex(&updated),
            updated.len() as u64,
            mutation.clone(),
        )
        .unwrap();
        (
            rollout_path,
            rollout,
            updated,
            backup_path,
            plan.config_generation,
            mutation,
        )
    }

    #[test]
    fn exact_provider_key_is_the_recovery_target() {
        let (_temp, paths) = fixture("agentharbor_relay", "custom");
        let preview = preview_thread_recovery(&paths, THREAD_ID).unwrap();
        assert_eq!(preview.target_provider, "agentharbor_relay");
        assert_eq!(preview.status, ThreadRecoveryStatus::ProviderMismatch);
        assert!(preview.can_apply);
    }

    #[test]
    fn unknown_provider_definition_fails_closed() {
        let (_temp, paths) = fixture("openai", "openai");
        std::fs::write(&paths.config_path, "model_provider='missing'\n").unwrap();
        let preview = preview_thread_recovery(&paths, THREAD_ID).unwrap();
        assert_eq!(preview.status, ThreadRecoveryStatus::InvalidTargetProvider);
        assert!(!preview.can_apply);
    }

    #[test]
    fn active_profile_provider_overrides_root_provider() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        std::fs::write(
            &path,
            "model_provider='openai'\nprofile='work'\n[profiles.work]\nmodel_provider='agentharbor_relay'\n[model_providers.agentharbor_relay]\nname='Relay'\n",
        )
        .unwrap();
        let target = resolve_provider_target(&path).unwrap();
        assert!(target.valid);
        assert_eq!(target.configured, "agentharbor_relay");
    }

    #[test]
    fn preview_and_apply_repair_exact_provider_key() {
        let (_temp, paths) = fixture("agentharbor_native", "openai");
        let preview = preview_thread_recovery(&paths, THREAD_ID).unwrap();
        let result =
            apply_thread_recovery_inner(&paths, THREAD_ID, &preview.revision, false).unwrap();
        assert!(result.changed);
        assert!(result
            .backup_path
            .as_ref()
            .is_some_and(|path| Path::new(path).is_file()));
        assert_eq!(result.preview.status, ThreadRecoveryStatus::Compatible);
        assert_eq!(result.preview.target_provider, "agentharbor_native");
        let database = load_database_thread(&paths.codex_state_db_path, THREAD_ID)
            .unwrap()
            .unwrap();
        assert_eq!(database.provider, "agentharbor_native");
        let rollout = read_rollout(
            &PathBuf::from(result.preview.rollout_path.unwrap()),
            THREAD_ID,
        )
        .unwrap();
        assert_eq!(rollout.provider.as_deref(), Some("agentharbor_native"));
        assert!(!journal_path(&paths).exists());
    }

    #[test]
    fn official_thread_reports_unverifiable_server_account_ownership() {
        let (_temp, paths) = fixture("openai", "openai");
        let preview = preview_thread_recovery(&paths, THREAD_ID).unwrap();
        assert_eq!(
            preview.status,
            ThreadRecoveryStatus::OfficialAccountUnverified
        );
        assert!(!preview.can_apply);
        assert!(preview.reason.contains("ChatGPT account"));
        assert!(apply_thread_recovery_inner(&paths, THREAD_ID, &preview.revision, false).is_err());
    }

    #[test]
    fn compatible_custom_thread_is_an_idempotent_no_op() {
        let (_temp, paths) = fixture("agentharbor_native", "agentharbor_native");
        let preview = preview_thread_recovery(&paths, THREAD_ID).unwrap();
        assert_eq!(preview.status, ThreadRecoveryStatus::Compatible);
        let result =
            apply_thread_recovery_inner(&paths, THREAD_ID, &preview.revision, false).unwrap();
        assert!(!result.changed);
        assert!(result.backup_path.is_none());
    }

    #[test]
    fn apply_refresh_detects_rollout_added_after_preview() {
        let (_temp, paths) = fixture("agentharbor_native", "openai");
        let preview = preview_thread_recovery(&paths, THREAD_ID).unwrap();
        std::fs::create_dir_all(&paths.archived_sessions_dir).unwrap();
        let original = PathBuf::from(preview.rollout_path.as_ref().unwrap());
        std::fs::copy(
            original,
            paths
                .archived_sessions_dir
                .join(format!("rollout-duplicate-{THREAD_ID}.jsonl")),
        )
        .unwrap();

        let error =
            apply_thread_recovery_inner(&paths, THREAD_ID, &preview.revision, false).unwrap_err();
        assert!(error
            .to_string()
            .contains("THREAD_RECOVERY_MULTIPLE_ROLLOUTS"));
    }

    #[test]
    fn rollout_hash_change_is_rejected() {
        let (_temp, paths) = fixture("agentharbor_relay", "openai");
        let plan = build_recovery_plan(&paths, THREAD_ID).unwrap();
        let path = plan.rollout_path.unwrap();
        let snapshot = plan.rollout.unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{}\n")
            .unwrap();
        assert!(ensure_rollout_unchanged(&path, &snapshot).is_err());
    }

    #[test]
    fn stale_preview_revision_is_rejected_before_writing() {
        let (_temp, paths) = fixture("agentharbor_relay", "openai");
        let preview = preview_thread_recovery(&paths, THREAD_ID).unwrap();
        std::fs::write(&paths.config_path, config_for("agentharbor_native")).unwrap();

        let error = apply_thread_recovery_inner(&paths, THREAD_ID, &preview.revision, false)
            .unwrap_err()
            .to_string();

        assert!(error.contains("THREAD_RECOVERY_REVISION_CHANGED"));
        assert_eq!(
            load_database_thread(&paths.codex_state_db_path, THREAD_ID)
                .unwrap()
                .unwrap()
                .provider,
            "openai"
        );
    }

    #[test]
    fn config_generation_guard_detects_changes_after_lock() {
        let (_temp, paths) = fixture("agentharbor_relay", "openai");
        let generation = resolve_provider_target(&paths.config_path)
            .unwrap()
            .generation;
        std::fs::write(&paths.config_path, config_for("agentharbor_native")).unwrap();
        assert!(ensure_config_generation(&paths, &generation).is_err());
    }

    #[test]
    fn mismatched_catalog_rollout_path_falls_back_to_thread_id_search() {
        let (_temp, paths) = fixture("agentharbor_relay", "openai");
        let wrong_path = paths.sessions_dir.join("rollout-wrong.jsonl");
        std::fs::write(
            &wrong_path,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"other-thread\",\"model_provider\":\"openai\"}}\n",
        )
        .unwrap();
        Connection::open(&paths.codex_state_db_path)
            .unwrap()
            .execute(
                "UPDATE threads SET rollout_path = ?1 WHERE id = ?2",
                (wrong_path.display().to_string(), THREAD_ID),
            )
            .unwrap();

        let preview = preview_thread_recovery(&paths, THREAD_ID).unwrap();
        let selected = std::fs::canonicalize(preview.rollout_path.unwrap()).unwrap();
        assert_ne!(selected, std::fs::canonicalize(wrong_path).unwrap());
        assert_eq!(preview.status, ThreadRecoveryStatus::ProviderMismatch);
    }

    #[test]
    fn multiple_rollouts_for_one_thread_fail_closed() {
        let (_temp, paths) = fixture("agentharbor_relay", "openai");
        let original = load_database_thread(&paths.codex_state_db_path, THREAD_ID)
            .unwrap()
            .unwrap()
            .rollout_path
            .unwrap();
        std::fs::copy(
            &original,
            paths
                .sessions_dir
                .join(format!("rollout-copy-{THREAD_ID}.jsonl")),
        )
        .unwrap();
        Connection::open(&paths.codex_state_db_path)
            .unwrap()
            .execute(
                "UPDATE threads SET rollout_path = 'missing-rollout.jsonl' WHERE id = ?1",
                [THREAD_ID],
            )
            .unwrap();

        let error = preview_thread_recovery(&paths, THREAD_ID)
            .unwrap_err()
            .to_string();
        assert!(error.contains("THREAD_RECOVERY_MULTIPLE_ROLLOUTS"));
    }

    #[test]
    fn duplicate_session_metadata_fails_closed() {
        let (_temp, paths) = fixture("agentharbor_relay", "openai");
        let rollout_path = load_database_thread(&paths.codex_state_db_path, THREAD_ID)
            .unwrap()
            .unwrap()
            .rollout_path
            .unwrap();
        let original = std::fs::read(&rollout_path).unwrap();
        OpenOptions::new()
            .append(true)
            .open(&rollout_path)
            .unwrap()
            .write_all(&original)
            .unwrap();

        let error = preview_thread_recovery(&paths, THREAD_ID)
            .unwrap_err()
            .to_string();
        assert!(error.contains("duplicate session metadata"));
    }

    #[test]
    fn prepared_empty_rollout_is_restored_from_backup() {
        let (_temp, paths) = fixture("agentharbor_relay", "openai");
        let (rollout_path, rollout, _updated, backup_path, config_generation) =
            prepare_recovery_backup(&paths);
        write_journal(
            &paths,
            &RecoveryJournal {
                version: 1,
                thread_id: THREAD_ID.into(),
                backup_path: backup_path.display().to_string(),
                config_generation,
                stage: RecoveryStage::Prepared,
                mutation: None,
            },
        )
        .unwrap();
        std::fs::write(&rollout_path, []).unwrap();

        recover_pending_journal(&paths).unwrap();
        assert_eq!(std::fs::read(rollout_path).unwrap(), rollout.bytes);
        assert!(!journal_path(&paths).exists());
    }

    #[test]
    fn prepared_target_prefix_is_restored_from_backup() {
        let (_temp, paths) = fixture("agentharbor_relay", "openai");
        let (rollout_path, rollout, updated, backup_path, config_generation) =
            prepare_recovery_backup(&paths);
        write_journal(
            &paths,
            &RecoveryJournal {
                version: 1,
                thread_id: THREAD_ID.into(),
                backup_path: backup_path.display().to_string(),
                config_generation,
                stage: RecoveryStage::Prepared,
                mutation: None,
            },
        )
        .unwrap();
        std::fs::write(&rollout_path, &updated[..updated.len() / 2]).unwrap();

        recover_pending_journal(&paths).unwrap();
        assert_eq!(std::fs::read(rollout_path).unwrap(), rollout.bytes);
    }

    #[test]
    fn invalid_utf8_target_prefix_can_still_be_coordinated() {
        let (_temp, paths) = fixture("agentharbor_relay", "openai");
        let (rollout_path, rollout, updated, backup_path, config_generation) =
            prepare_recovery_backup(&paths);
        write_journal(
            &paths,
            &RecoveryJournal {
                version: 1,
                thread_id: THREAD_ID.into(),
                backup_path: backup_path.display().to_string(),
                config_generation,
                stage: RecoveryStage::Prepared,
                mutation: None,
            },
        )
        .unwrap();
        let multibyte_start = updated
            .windows("你".len())
            .position(|window| window == "你".as_bytes())
            .unwrap();
        std::fs::write(&rollout_path, &updated[..multibyte_start + 1]).unwrap();

        let preview = preview_thread_recovery(&paths, THREAD_ID).unwrap();
        assert_eq!(preview.status, ThreadRecoveryStatus::RecoveryPending);
        let result =
            apply_thread_recovery_inner(&paths, THREAD_ID, &preview.revision, false).unwrap();

        assert!(!result.changed);
        assert_eq!(std::fs::read(rollout_path).unwrap(), rollout.bytes);
        assert_eq!(
            result.preview.status,
            ThreadRecoveryStatus::ProviderMismatch
        );
    }

    #[test]
    fn prepared_unknown_rollout_content_stays_fail_closed() {
        let (_temp, paths) = fixture("agentharbor_relay", "openai");
        let (rollout_path, _rollout, _updated, backup_path, config_generation) =
            prepare_recovery_backup(&paths);
        write_journal(
            &paths,
            &RecoveryJournal {
                version: 1,
                thread_id: THREAD_ID.into(),
                backup_path: backup_path.display().to_string(),
                config_generation,
                stage: RecoveryStage::Prepared,
                mutation: None,
            },
        )
        .unwrap();
        std::fs::write(&rollout_path, b"external unknown contents\n").unwrap();

        let error = recover_pending_journal(&paths).unwrap_err().to_string();
        assert!(error.contains("THREAD_RECOVERY_JOURNAL_CONFLICT"));
        assert_eq!(
            std::fs::read(&rollout_path).unwrap(),
            b"external unknown contents\n"
        );
        assert!(journal_path(&paths).exists());
    }

    #[test]
    fn pending_other_thread_is_not_coordinated() {
        const OTHER_THREAD_ID: &str = "02999999-1111-7222-8333-444444444444";
        let (_temp, paths) = fixture("agentharbor_relay", "openai");
        let generation = resolve_provider_target(&paths.config_path)
            .unwrap()
            .generation;
        write_journal(
            &paths,
            &RecoveryJournal {
                version: 1,
                thread_id: THREAD_ID.into(),
                backup_path: "unused-for-other-thread".into(),
                config_generation: generation,
                stage: RecoveryStage::Prepared,
                mutation: None,
            },
        )
        .unwrap();

        let preview = preview_thread_recovery(&paths, OTHER_THREAD_ID).unwrap();
        assert_eq!(preview.status, ThreadRecoveryStatus::RecoveryPending);
        assert!(!preview.can_apply);
        let error = apply_thread_recovery_inner(&paths, OTHER_THREAD_ID, &preview.revision, false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("THREAD_RECOVERY_PENDING_OTHER_THREAD"));
        assert!(journal_path(&paths).exists());
    }

    #[test]
    fn coordinating_same_thread_does_not_start_another_recovery() {
        let (_temp, paths) = fixture("agentharbor_relay", "openai");
        let (_rollout_path, _rollout, _updated, backup_path, config_generation) =
            prepare_recovery_backup(&paths);
        write_journal(
            &paths,
            &RecoveryJournal {
                version: 1,
                thread_id: THREAD_ID.into(),
                backup_path: backup_path.display().to_string(),
                config_generation,
                stage: RecoveryStage::Prepared,
                mutation: None,
            },
        )
        .unwrap();
        let preview = preview_thread_recovery(&paths, THREAD_ID).unwrap();

        let result =
            apply_thread_recovery_inner(&paths, THREAD_ID, &preview.revision, false).unwrap();

        assert!(!result.changed);
        assert_eq!(
            result.preview.status,
            ThreadRecoveryStatus::ProviderMismatch
        );
        assert_eq!(
            load_database_thread(&paths.codex_state_db_path, THREAD_ID)
                .unwrap()
                .unwrap()
                .provider,
            "openai"
        );
        assert!(!journal_path(&paths).exists());
    }

    #[test]
    fn pending_rollout_write_is_compensated_from_journal() {
        let (_temp, paths) = fixture("agentharbor_relay", "openai");
        let plan = build_recovery_plan(&paths, THREAD_ID).unwrap();
        let rollout_path = plan.rollout_path.unwrap();
        let rollout = plan.rollout.unwrap();
        let updated =
            rewrite_rollout_provider(&rollout.bytes, THREAD_ID, "agentharbor_relay").unwrap();
        let updated_hash = sha256_hex(&updated);
        let config_generation = resolve_provider_target(&paths.config_path)
            .unwrap()
            .generation;
        let backup_path = write_backup(
            &paths,
            THREAD_ID,
            "openai",
            &rollout_path,
            &rollout,
            "agentharbor_relay",
            &config_generation,
            &updated_hash,
            updated.len() as u64,
        )
        .unwrap();
        storage::write_atomic(&rollout_path, &updated).unwrap();
        write_journal(
            &paths,
            &RecoveryJournal {
                version: 1,
                thread_id: THREAD_ID.into(),
                backup_path: backup_path.display().to_string(),
                config_generation,
                stage: RecoveryStage::RolloutWritten,
                mutation: None,
            },
        )
        .unwrap();
        recover_pending_journal(&paths).unwrap();
        assert_eq!(std::fs::read(&rollout_path).unwrap(), rollout.bytes);
        assert!(!journal_path(&paths).exists());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn insert_journal_prepared_absent_row_is_already_compensated() {
        let (_temp, paths) = fixture("agentharbor_native", "openai");
        let (_rollout_path, _rollout, _updated, backup_path, config_generation, mutation) =
            prepare_insert_recovery_backup(&paths);
        write_journal(
            &paths,
            &RecoveryJournal {
                version: 2,
                thread_id: THREAD_ID.into(),
                backup_path: backup_path.display().to_string(),
                config_generation,
                stage: RecoveryStage::Prepared,
                mutation: Some(mutation),
            },
        )
        .unwrap();

        recover_pending_journal(&paths).unwrap();
        assert!(load_database_thread(&paths.codex_state_db_path, THREAD_ID)
            .unwrap()
            .is_none());
        assert!(!journal_path(&paths).exists());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn insert_journal_rollout_written_restores_absent_row_state() {
        let (_temp, paths) = fixture("agentharbor_native", "openai");
        let (rollout_path, rollout, updated, backup_path, config_generation, mutation) =
            prepare_insert_recovery_backup(&paths);
        storage::write_atomic(&rollout_path, &updated).unwrap();
        write_journal(
            &paths,
            &RecoveryJournal {
                version: 2,
                thread_id: THREAD_ID.into(),
                backup_path: backup_path.display().to_string(),
                config_generation,
                stage: RecoveryStage::RolloutWritten,
                mutation: Some(mutation),
            },
        )
        .unwrap();

        recover_pending_journal(&paths).unwrap();
        assert_eq!(std::fs::read(rollout_path).unwrap(), rollout.bytes);
        assert!(load_database_thread(&paths.codex_state_db_path, THREAD_ID)
            .unwrap()
            .is_none());
        assert!(!journal_path(&paths).exists());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn insert_journal_committed_accepts_exact_frozen_row() {
        let (_temp, paths) = fixture("agentharbor_native", "openai");
        let (rollout_path, _rollout, updated, backup_path, config_generation, mutation) =
            prepare_insert_recovery_backup(&paths);
        let CatalogMutation::InsertRow {
            schema_signature,
            columns,
            row_fingerprint,
        } = &mutation
        else {
            panic!("expected insert mutation");
        };
        let connection = Connection::open(&paths.codex_state_db_path).unwrap();
        insert_catalog_row(
            &connection,
            THREAD_ID,
            schema_signature,
            columns,
            row_fingerprint,
        )
        .unwrap();
        drop(connection);
        storage::write_atomic(&rollout_path, &updated).unwrap();
        write_journal(
            &paths,
            &RecoveryJournal {
                version: 2,
                thread_id: THREAD_ID.into(),
                backup_path: backup_path.display().to_string(),
                config_generation,
                stage: RecoveryStage::Committed,
                mutation: Some(mutation),
            },
        )
        .unwrap();

        recover_pending_journal(&paths).unwrap();
        assert_eq!(
            load_database_thread(&paths.codex_state_db_path, THREAD_ID)
                .unwrap()
                .unwrap()
                .provider,
            "agentharbor_native"
        );
        assert!(!journal_path(&paths).exists());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn insert_journal_committed_rejects_impossible_absent_row_state() {
        let (_temp, paths) = fixture("agentharbor_native", "openai");
        let (_rollout_path, _rollout, _updated, backup_path, config_generation, mutation) =
            prepare_insert_recovery_backup(&paths);
        write_journal(
            &paths,
            &RecoveryJournal {
                version: 2,
                thread_id: THREAD_ID.into(),
                backup_path: backup_path.display().to_string(),
                config_generation,
                stage: RecoveryStage::Committed,
                mutation: Some(mutation),
            },
        )
        .unwrap();

        let error = recover_pending_journal(&paths).unwrap_err().to_string();
        assert!(error.contains("THREAD_RECOVERY_JOURNAL_CONFLICT"));
        assert!(load_database_thread(&paths.codex_state_db_path, THREAD_ID)
            .unwrap()
            .is_none());
        assert!(journal_path(&paths).is_file());
        assert!(backup_path.is_file());
    }

    #[test]
    fn version_one_backup_and_journal_are_normalized_to_provider_update() {
        let (_temp, paths) = fixture("agentharbor_relay", "openai");
        let (_rollout_path, _rollout, _updated, backup_path, config_generation) =
            prepare_recovery_backup(&paths);
        let mut backup: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&backup_path).unwrap()).unwrap();
        backup["version"] = serde_json::json!(1);
        backup.as_object_mut().unwrap().remove("mutation");
        storage::write_atomic(&backup_path, &serde_json::to_vec_pretty(&backup).unwrap()).unwrap();
        let loaded = load_backup(&paths, &backup_path).unwrap();
        assert!(matches!(
            backup_mutation(&loaded).unwrap(),
            CatalogMutation::UpdateProvider {
                original_provider,
                target_provider
            } if original_provider == "openai" && target_provider == "agentharbor_relay"
        ));
        let journal = RecoveryJournal {
            version: 1,
            thread_id: THREAD_ID.into(),
            backup_path: backup_path.display().to_string(),
            config_generation,
            stage: RecoveryStage::Prepared,
            mutation: None,
        };
        validate_journal_mutation(&journal, &loaded).unwrap();
    }

    #[test]
    fn malformed_journal_returns_manual_coordination_preview() {
        let (_temp, paths) = fixture("agentharbor_native", "openai");
        std::fs::write(journal_path(&paths), b"{broken").unwrap();

        let preview = preview_thread_recovery(&paths, THREAD_ID).unwrap();
        assert_eq!(preview.status, ThreadRecoveryStatus::JournalBlocked);
        assert!(!preview.can_apply);
        let coordination = preview.coordination.unwrap();
        assert_eq!(
            coordination.journal_path,
            journal_path(&paths).display().to_string()
        );
        assert_eq!(
            coordination.directory_path,
            paths.app_data_dir.display().to_string()
        );
        assert!(coordination.backup_path.is_none());
        assert!(!coordination.detail.is_empty());
    }

    #[test]
    fn non_file_journal_path_returns_manual_coordination_preview() {
        let (_temp, paths) = fixture("agentharbor_native", "openai");
        std::fs::create_dir_all(journal_path(&paths)).unwrap();

        let preview = preview_thread_recovery(&paths, THREAD_ID).unwrap();
        assert_eq!(preview.status, ThreadRecoveryStatus::JournalBlocked);
        assert!(!preview.can_apply);
        assert!(preview
            .coordination
            .as_ref()
            .unwrap()
            .detail
            .contains("not a regular file"));
        assert!(journal_path(&paths).is_dir());
    }

    #[test]
    fn unsupported_journal_version_preserves_backup_hint() {
        let (_temp, paths) = fixture("agentharbor_native", "openai");
        let backup_path = backup_directory(&paths).join("manual-review.json");
        storage::write_atomic(
            &journal_path(&paths),
            &serde_json::to_vec_pretty(&serde_json::json!({
                "version": 99,
                "threadId": THREAD_ID,
                "backupPath": backup_path.display().to_string(),
                "configGeneration": "generation",
                "stage": "prepared",
                "mutation": null
            }))
            .unwrap(),
        )
        .unwrap();

        let preview = preview_thread_recovery(&paths, THREAD_ID).unwrap();
        assert_eq!(preview.status, ThreadRecoveryStatus::JournalBlocked);
        assert!(!preview.can_apply);
        let coordination = preview.coordination.unwrap();
        let expected_backup_path = backup_path.display().to_string();
        assert_eq!(
            coordination.backup_path.as_deref(),
            Some(expected_backup_path.as_str())
        );
        assert!(coordination.detail.contains("Unsupported"));
    }

    #[test]
    fn missing_catalog_is_ready_for_safe_insertion() {
        let (_temp, paths) = fixture("agentharbor_native", "openai");
        replace_threads_schema(
            &paths,
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                title TEXT,
                cwd TEXT,
                source TEXT,
                model_provider TEXT,
                model TEXT,
                rollout_path TEXT,
                sandbox_policy TEXT,
                approval_mode TEXT,
                archived INTEGER,
                created_at INTEGER,
                updated_at INTEGER
            );",
        );
        let preview = preview_thread_recovery(&paths, THREAD_ID).unwrap();
        assert_eq!(preview.status, ThreadRecoveryStatus::MissingCatalog);
        assert_eq!(preview.can_apply, cfg!(target_os = "windows"));
        let field_names = preview
            .catalog_fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<HashSet<_>>();
        for expected in [
            "id",
            "title",
            "cwd",
            "source",
            "model_provider",
            "model",
            "sandbox_policy",
            "approval_mode",
            "archived",
            "created_at",
            "updated_at",
        ] {
            assert!(
                field_names.contains(expected),
                "missing preview field {expected}"
            );
        }
        assert!(preview
            .catalog_fields
            .iter()
            .any(|field| field.name == "id" && field.value == THREAD_ID));
        let analysis = preview.catalog_analysis.unwrap();
        assert!(analysis.ready);
        assert!(analysis
            .schema_signature
            .is_some_and(|signature| signature.starts_with("v1:")));
        assert!(analysis.missing_fields.is_empty());
        assert!(analysis.unsupported_columns.is_empty());
    }

    #[test]
    fn unknown_nullable_and_defaulted_columns_allow_catalog_analysis() {
        let (_temp, paths) = fixture("agentharbor_native", "openai");
        replace_threads_schema(
            &paths,
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                model_provider TEXT,
                rollout_path TEXT,
                future_nullable TEXT,
                future_default TEXT NOT NULL DEFAULT 'safe'
            );",
        );

        let preview = preview_thread_recovery(&paths, THREAD_ID).unwrap();
        assert_eq!(preview.status, ThreadRecoveryStatus::MissingCatalog);
        let analysis = preview.catalog_analysis.unwrap();
        assert!(analysis.ready);
        assert!(analysis.unsupported_columns.is_empty());
    }

    #[test]
    fn unknown_required_column_is_schema_unsupported() {
        let (_temp, paths) = fixture("agentharbor_native", "openai");
        replace_threads_schema(
            &paths,
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                model_provider TEXT,
                rollout_path TEXT,
                future_required TEXT NOT NULL
            );",
        );

        let preview = preview_thread_recovery(&paths, THREAD_ID).unwrap();
        assert_eq!(preview.status, ThreadRecoveryStatus::SchemaUnsupported);
        assert!(!preview.can_apply);
        let analysis = preview.catalog_analysis.unwrap();
        assert!(!analysis.ready);
        assert!(analysis
            .unsupported_columns
            .iter()
            .any(|column| column.starts_with("future_required ")));
    }

    #[test]
    fn unverified_policy_metadata_is_reported_as_insufficient() {
        let (_temp, paths) = fixture("agentharbor_native", "openai");
        replace_threads_schema(
            &paths,
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                model_provider TEXT,
                rollout_path TEXT,
                sandbox_policy TEXT NOT NULL,
                approval_mode TEXT NOT NULL
            );",
        );

        let preview = preview_thread_recovery(&paths, THREAD_ID).unwrap();
        assert_eq!(preview.status, ThreadRecoveryStatus::InsufficientMetadata);
        assert!(!preview.can_apply);
        let analysis = preview.catalog_analysis.unwrap();
        assert_eq!(
            analysis.missing_fields,
            vec!["approval_mode".to_owned(), "sandbox_policy".to_owned()]
        );
        assert!(analysis.unsupported_columns.is_empty());
    }

    #[test]
    fn wrong_id_affinity_is_schema_unsupported() {
        let (_temp, paths) = fixture("agentharbor_native", "openai");
        replace_threads_schema(
            &paths,
            "CREATE TABLE threads (
                id INTEGER PRIMARY KEY,
                model_provider TEXT,
                rollout_path TEXT
            );",
        );

        let preview = preview_thread_recovery(&paths, THREAD_ID).unwrap();
        assert_eq!(preview.status, ThreadRecoveryStatus::SchemaUnsupported);
        let analysis = preview.catalog_analysis.unwrap();
        assert!(analysis
            .unsupported_columns
            .iter()
            .any(|column| column.starts_with("id (")));
    }

    #[test]
    fn id_must_be_the_sole_primary_key() {
        let (_temp, paths) = fixture("agentharbor_native", "openai");
        replace_threads_schema(
            &paths,
            "CREATE TABLE threads (
                id TEXT,
                alternate_id TEXT PRIMARY KEY,
                model_provider TEXT,
                rollout_path TEXT
            );",
        );

        let preview = preview_thread_recovery(&paths, THREAD_ID).unwrap();
        assert_eq!(preview.status, ThreadRecoveryStatus::SchemaUnsupported);
        let analysis = preview.catalog_analysis.unwrap();
        assert!(analysis
            .unsupported_columns
            .iter()
            .any(|column| column.starts_with("id (")));
    }

    #[test]
    fn approval_policy_is_canonicalized_strictly() {
        for value in ["untrusted", "on-request", "never"] {
            assert_eq!(
                canonical_approval_mode(&serde_json::json!(value)).unwrap(),
                value
            );
        }
        assert_eq!(
            canonical_approval_mode(&serde_json::json!("on-failure")).unwrap(),
            "on-request"
        );

        let granular = canonical_approval_mode(&serde_json::json!({
            "granular": {
                "sandbox_approval": true,
                "rules": false,
                "mcp_elicitations": true
            }
        }))
        .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&granular).unwrap(),
            serde_json::json!({
                "granular": {
                    "sandbox_approval": true,
                    "rules": false,
                    "skill_approval": false,
                    "request_permissions": false,
                    "mcp_elicitations": true
                }
            })
        );

        assert!(canonical_approval_mode(&serde_json::json!("future-mode")).is_err());
        assert!(canonical_approval_mode(&serde_json::json!({
            "granular": {
                "sandbox_approval": true,
                "mcp_elicitations": true
            }
        }))
        .is_err());
        assert!(canonical_approval_mode(&serde_json::json!({
            "granular": {
                "sandbox_approval": true,
                "rules": false,
                "mcp_elicitations": true,
                "future": true
            }
        }))
        .is_err());
    }

    #[test]
    fn permission_profiles_and_filesystem_entries_are_canonicalized_strictly() {
        let disabled =
            canonical_permission_profile(&serde_json::json!({"type": "disabled"})).unwrap();
        assert_eq!(disabled, r#"{"type":"disabled"}"#);

        let managed = canonical_permission_profile(&serde_json::json!({
            "type": "managed",
            "file_system": {
                "type": "restricted",
                "entries": [
                    {
                        "path": {"type": "special", "value": {"kind": "minimal"}},
                        "access": "read"
                    },
                    {
                        "path": {"type": "glob_pattern", "pattern": "**/*.env"},
                        "access": "none"
                    }
                ],
                "glob_scan_max_depth": 2
            },
            "network": "enabled"
        }))
        .unwrap();
        assert_eq!(
            managed,
            r#"{"type":"managed","file_system":{"type":"restricted","entries":[{"path":{"type":"special","value":{"kind":"minimal"}},"access":"read"},{"path":{"type":"glob_pattern","pattern":"**/*.env"},"access":"deny"}],"glob_scan_max_depth":2},"network":"enabled"}"#
        );

        assert!(canonical_permission_profile(&serde_json::json!({
            "type": "managed",
            "file_system": {
                "type": "restricted",
                "entries": [{
                    "path": {"type": "path", "path": "C:/fixture"},
                    "access": "read",
                    "missing_path_behavior": "error"
                }]
            },
            "network": "restricted"
        }))
        .is_err());
        assert!(canonical_permission_profile(&serde_json::json!({
            "type": "managed",
            "file_system": {
                "type": "restricted",
                "entries": [{
                    "path": {
                        "type": "special",
                        "value": {"kind": "root", "subpath": ".git"}
                    },
                    "access": "read"
                }]
            },
            "network": "restricted"
        }))
        .is_err());
    }

    #[test]
    fn raw_split_filesystem_policy_uses_codex_kind_shape() {
        let legacy = serde_json::json!({
            "type": "workspace-write",
            "writable_roots": [],
            "network_access": true,
            "exclude_tmpdir_env_var": false,
            "exclude_slash_tmp": false
        });
        let split = serde_json::json!({
            "kind": "restricted",
            "entries": [{
                "path": {"type": "special", "value": {"kind": "project_roots"}},
                "access": "write"
            }]
        });
        let mapped = canonical_legacy_permission_profile(&legacy, Some(&split), None).unwrap();
        let mapped: serde_json::Value = serde_json::from_str(&mapped).unwrap();
        assert_eq!(mapped["type"], "managed");
        assert_eq!(mapped["network"], "enabled");
        assert_eq!(mapped["file_system"]["type"], "restricted");

        assert!(canonical_legacy_permission_profile(
            &legacy,
            Some(&serde_json::json!({"type": "restricted", "entries": []})),
            None,
        )
        .is_err());

        let external = canonical_legacy_permission_profile(
            &serde_json::json!({"type": "external-sandbox", "network_access": "enabled"}),
            Some(&serde_json::json!({"kind": "external-sandbox"})),
            None,
        )
        .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&external).unwrap(),
            serde_json::json!({"type": "external", "network": "enabled"})
        );
    }

    #[test]
    fn conflicting_legacy_sandbox_enforcement_fails_closed() {
        assert!(canonical_legacy_permission_profile(
            &serde_json::json!({"type": "danger-full-access"}),
            Some(&serde_json::json!({"kind": "restricted", "entries": []})),
            None,
        )
        .is_err());
        assert!(canonical_legacy_permission_profile(
            &serde_json::json!({"type": "external-sandbox"}),
            Some(&serde_json::json!({"kind": "unrestricted"})),
            None,
        )
        .is_err());
        assert!(canonical_legacy_permission_profile(
            &serde_json::json!({"type": "read-only"}),
            Some(&serde_json::json!({"kind": "external-sandbox"})),
            None,
        )
        .is_err());
    }

    #[test]
    fn frozen_defaults_reject_expressions_and_accept_constants() {
        assert_eq!(parse_frozen_default("NULL").unwrap(), CatalogSqlValue::Null);
        assert_eq!(
            parse_frozen_default("-42").unwrap(),
            CatalogSqlValue::Integer(-42)
        );
        assert_eq!(
            parse_frozen_default("'it''s safe'").unwrap(),
            CatalogSqlValue::Text("it's safe".into())
        );
        assert!(parse_frozen_default("CURRENT_TIMESTAMP").is_err());
        assert!(parse_frozen_default("(1 + 2)").is_err());
        assert!(parse_frozen_default("random()").is_err());
    }

    #[test]
    fn missing_threads_table_and_provider_column_are_schema_failures() {
        for schema in [
            "CREATE TABLE other_table (id TEXT PRIMARY KEY);",
            "CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT);",
        ] {
            let (_temp, paths) = fixture("agentharbor_native", "openai");
            let connection = Connection::open(&paths.codex_state_db_path).unwrap();
            connection.execute_batch("DROP TABLE threads;").unwrap();
            connection.execute_batch(schema).unwrap();
            let preview = preview_thread_recovery(&paths, THREAD_ID).unwrap();
            assert_eq!(preview.status, ThreadRecoveryStatus::SchemaUnsupported);
            assert!(!preview.can_apply);
            assert!(!preview
                .catalog_analysis
                .unwrap()
                .unsupported_columns
                .is_empty());
        }
    }

    #[test]
    fn hidden_or_generated_catalog_columns_are_rejected() {
        let (_temp, paths) = fixture("agentharbor_native", "openai");
        replace_threads_schema(
            &paths,
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                model_provider TEXT,
                rollout_path TEXT,
                derived TEXT GENERATED ALWAYS AS (id) STORED
            );",
        );
        let preview = preview_thread_recovery(&paths, THREAD_ID).unwrap();
        assert_eq!(preview.status, ThreadRecoveryStatus::SchemaUnsupported);
        assert!(preview
            .catalog_analysis
            .unwrap()
            .unsupported_columns
            .iter()
            .any(|column| column.contains("hidden/generated")));
    }

    #[test]
    fn legacy_workspace_write_without_split_policy_uses_cwd_and_protected_paths() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("workspace");
        let extra_root = temp.path().join("extra");
        std::fs::create_dir_all(cwd.join(".git")).unwrap();
        std::fs::create_dir_all(&extra_root).unwrap();
        let cwd_value = serde_json::json!(cwd.display().to_string());
        let legacy = serde_json::json!({
            "type": "workspace-write",
            "writable_roots": [extra_root.display().to_string()],
            "network_access": false,
            "exclude_tmpdir_env_var": true,
            "exclude_slash_tmp": true
        });

        let mapped = canonical_legacy_permission_profile(&legacy, None, Some(&cwd_value)).unwrap();
        let mapped: serde_json::Value = serde_json::from_str(&mapped).unwrap();
        let entries = mapped["file_system"]["entries"].as_array().unwrap();

        assert_eq!(mapped["type"], "managed");
        assert_eq!(mapped["network"], "restricted");
        assert!(entries.iter().any(|entry| {
            entry["path"]
                == serde_json::json!({
                    "type": "special",
                    "value": {"kind": "project_roots"}
                })
                && entry["access"] == "write"
        }));
        assert!(!entries.iter().any(|entry| {
            matches!(
                entry["path"]["value"]["kind"].as_str(),
                Some("tmpdir" | "slash_tmp")
            )
        }));
        assert!(entries.iter().any(|entry| {
            entry["path"]
                == serde_json::json!({
                    "type": "path",
                    "path": extra_root.display().to_string()
                })
                && entry["access"] == "write"
        }));
        assert!(entries.iter().any(|entry| {
            entry["path"]
                == serde_json::json!({
                    "type": "path",
                    "path": cwd.join(".git").display().to_string()
                })
                && entry["access"] == "read"
                && entry["missing_path_behavior"] == "skip"
        }));
    }

    #[test]
    fn invalid_settings_records_remain_fail_closed_regardless_of_order() {
        for invalid_last in [true, false] {
            let (_temp, paths) = fixture("agentharbor_native", "openai");
            let database = load_database_thread(&paths.codex_state_db_path, THREAD_ID)
                .unwrap()
                .unwrap();
            let rollout_path = database.rollout_path.unwrap();
            let valid = serde_json::json!({
                "type": "turn_context",
                "payload": {
                    "cwd": rollout_path.parent().unwrap().display().to_string(),
                    "approval_policy": "never",
                    "permission_profile": {"type": "disabled"}
                }
            });
            let invalid = serde_json::json!({
                "type": "turn_context",
                "payload": {
                    "cwd": rollout_path.parent().unwrap().display().to_string(),
                    "approval_policy": "never",
                    "permission_profile": {"type": "future-profile"}
                }
            });
            let records = if invalid_last {
                [valid, invalid]
            } else {
                [invalid, valid]
            };
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(&rollout_path)
                .unwrap();
            for record in records {
                writeln!(file, "{}", serde_json::to_string(&record).unwrap()).unwrap();
            }
            drop(file);

            let connection = Connection::open(&paths.codex_state_db_path).unwrap();
            connection.execute("DELETE FROM threads", []).unwrap();

            let preview = preview_thread_recovery(&paths, THREAD_ID).unwrap();
            assert_eq!(preview.status, ThreadRecoveryStatus::InsufficientMetadata);
            let analysis = preview.catalog_analysis.unwrap();
            assert!(!analysis.ready);
            assert!(
                analysis
                    .mapping_errors
                    .iter()
                    .any(|error| error
                        .contains("unsupported permission_profile type: future-profile"))
            );
        }
    }

    #[test]
    fn explicit_permission_profile_never_falls_back_to_legacy_policy() {
        let mapped = map_catalog_policy_fields(&serde_json::json!({
            "permission_profile": {"type": "disabled"},
            "sandbox_policy": {"type": "future-legacy"},
            "approval_policy": "never"
        }))
        .unwrap();
        assert_eq!(
            mapped.sandbox_policy.as_deref(),
            Some("{\"type\":\"disabled\"}")
        );
        assert_eq!(mapped.approval_mode.as_deref(), Some("never"));

        assert!(map_catalog_policy_fields(&serde_json::json!({
            "permission_profile": {"type": "future-profile"},
            "sandbox_policy": {"type": "danger-full-access"},
            "approval_policy": "never"
        }))
        .is_err());
    }

    #[test]
    fn child_thread_evidence_blocks_catalog_reconstruction() {
        let (_temp, paths) = fixture("agentharbor_native", "openai");
        let database = load_database_thread(&paths.codex_state_db_path, THREAD_ID)
            .unwrap()
            .unwrap();
        let rollout_path = database.rollout_path.unwrap();
        let mut record: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&rollout_path).unwrap()).unwrap();
        record["payload"]["source"] = serde_json::json!("subagent");
        std::fs::write(
            &rollout_path,
            format!("{}\n", serde_json::to_string(&record).unwrap()),
        )
        .unwrap();
        let connection = Connection::open(&paths.codex_state_db_path).unwrap();
        connection.execute("DELETE FROM threads", []).unwrap();

        let preview = preview_thread_recovery(&paths, THREAD_ID).unwrap();
        assert_eq!(preview.status, ThreadRecoveryStatus::InsufficientMetadata);
        let analysis = preview.catalog_analysis.unwrap();
        assert_eq!(analysis.relation, ThreadCatalogRelation::Child);
        assert!(!analysis.ready);
        assert!(!analysis.mapping_errors.is_empty());
    }

    #[test]
    fn sqlite_child_reference_blocks_catalog_reconstruction() {
        let (_temp, paths) = fixture("agentharbor_native", "openai");
        let connection = Connection::open(&paths.codex_state_db_path).unwrap();
        connection
            .execute_batch("CREATE TABLE thread_spawn_edges (child_thread_id TEXT NOT NULL);")
            .unwrap();
        connection
            .execute(
                "INSERT INTO thread_spawn_edges (child_thread_id) VALUES (?1)",
                [THREAD_ID],
            )
            .unwrap();
        connection.execute("DELETE FROM threads", []).unwrap();

        let preview = preview_thread_recovery(&paths, THREAD_ID).unwrap();
        assert_eq!(preview.status, ThreadRecoveryStatus::InsufficientMetadata);
        let analysis = preview.catalog_analysis.unwrap();
        assert_eq!(analysis.relation, ThreadCatalogRelation::Child);
        assert!(analysis
            .mapping_errors
            .iter()
            .any(|error| error.contains("thread_spawn_edges.child_thread_id")));
    }

    #[test]
    fn unknown_source_and_thread_source_values_fail_closed() {
        for (field, value) in [
            ("source", "future-client"),
            ("thread_source", "future-kind"),
        ] {
            let (_temp, paths) = fixture("agentharbor_native", "openai");
            let database = load_database_thread(&paths.codex_state_db_path, THREAD_ID)
                .unwrap()
                .unwrap();
            let rollout_path = database.rollout_path.unwrap();
            let mut record: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&rollout_path).unwrap()).unwrap();
            record["payload"][field] = serde_json::json!(value);
            std::fs::write(
                &rollout_path,
                format!("{}\n", serde_json::to_string(&record).unwrap()),
            )
            .unwrap();
            Connection::open(&paths.codex_state_db_path)
                .unwrap()
                .execute("DELETE FROM threads", [])
                .unwrap();

            let preview = preview_thread_recovery(&paths, THREAD_ID).unwrap();
            assert_eq!(preview.status, ThreadRecoveryStatus::InsufficientMetadata);
            let analysis = preview.catalog_analysis.unwrap();
            assert!(!analysis.ready);
            assert!(analysis
                .mapping_errors
                .iter()
                .any(|error| error.contains("unsupported")));
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn missing_catalog_apply_inserts_full_frozen_row() {
        let (_temp, paths) = fixture("agentharbor_native", "openai");
        Connection::open(&paths.codex_state_db_path)
            .unwrap()
            .execute("DELETE FROM threads", [])
            .unwrap();
        let preview = preview_thread_recovery(&paths, THREAD_ID).unwrap();
        let result =
            apply_thread_recovery_inner(&paths, THREAD_ID, &preview.revision, false).unwrap();
        assert!(result.changed);
        let inserted = load_database_thread(&paths.codex_state_db_path, THREAD_ID)
            .unwrap()
            .unwrap();
        assert_eq!(inserted.provider, "agentharbor_native");
        assert!(!journal_path(&paths).exists());
    }

    #[test]
    fn inserted_row_compensation_rejects_fingerprint_drift_and_references() {
        let (_temp, paths) = fixture("agentharbor_native", "openai");
        let connection = Connection::open(&paths.codex_state_db_path).unwrap();
        connection.execute("DELETE FROM threads", []).unwrap();
        drop(connection);
        let plan = build_recovery_plan(&paths, THREAD_ID).unwrap();
        let CatalogMutation::InsertRow {
            schema_signature,
            columns,
            row_fingerprint,
        } = plan.mutation.unwrap()
        else {
            panic!("expected insert mutation");
        };
        let connection = Connection::open(&paths.codex_state_db_path).unwrap();
        insert_catalog_row(
            &connection,
            THREAD_ID,
            &schema_signature,
            &columns,
            &row_fingerprint,
        )
        .unwrap();
        connection
            .execute(
                "UPDATE threads SET rollout_path = 'externally-modified' WHERE id = ?1",
                [THREAD_ID],
            )
            .unwrap();
        drop(connection);
        assert!(delete_inserted_catalog_row(
            &paths,
            THREAD_ID,
            &schema_signature,
            &columns,
            &row_fingerprint,
        )
        .is_err());

        let connection = Connection::open(&paths.codex_state_db_path).unwrap();
        connection.execute("DELETE FROM threads", []).unwrap();
        insert_catalog_row(
            &connection,
            THREAD_ID,
            &schema_signature,
            &columns,
            &row_fingerprint,
        )
        .unwrap();
        connection
            .execute_batch(
                "CREATE TABLE thread_spawn_edges (parent_thread_id TEXT, child_thread_id TEXT);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO thread_spawn_edges (parent_thread_id) VALUES (?1)",
                [THREAD_ID],
            )
            .unwrap();
        drop(connection);
        assert!(delete_inserted_catalog_row(
            &paths,
            THREAD_ID,
            &schema_signature,
            &columns,
            &row_fingerprint,
        )
        .is_err());
    }

    #[test]
    fn inserted_row_compensation_discovers_unknown_inbound_foreign_keys() {
        let (_temp, paths) = fixture("agentharbor_native", "openai");
        Connection::open(&paths.codex_state_db_path)
            .unwrap()
            .execute("DELETE FROM threads", [])
            .unwrap();
        let plan = build_recovery_plan(&paths, THREAD_ID).unwrap();
        let CatalogMutation::InsertRow {
            schema_signature,
            columns,
            row_fingerprint,
        } = plan.mutation.unwrap()
        else {
            panic!("expected insert mutation");
        };
        let connection = Connection::open(&paths.codex_state_db_path).unwrap();
        insert_catalog_row(
            &connection,
            THREAD_ID,
            &schema_signature,
            &columns,
            &row_fingerprint,
        )
        .unwrap();
        connection
            .execute_batch(
                "CREATE TABLE mystery_links (
                    opaque_ref TEXT NOT NULL REFERENCES threads(id)
                );",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO mystery_links (opaque_ref) VALUES (?1)",
                [THREAD_ID],
            )
            .unwrap();
        drop(connection);

        assert!(delete_inserted_catalog_row(
            &paths,
            THREAD_ID,
            &schema_signature,
            &columns,
            &row_fingerprint,
        )
        .is_err());
    }

    #[test]
    fn invalid_utf8_sqlite_text_bytes_have_distinct_fingerprints() {
        let (_temp, paths) = fixture("agentharbor_native", "openai");
        replace_threads_schema(
            &paths,
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                model_provider TEXT,
                rollout_path TEXT,
                note TEXT
            );",
        );
        let connection = Connection::open(&paths.codex_state_db_path).unwrap();
        connection
            .execute(
                "INSERT INTO threads (id, model_provider, rollout_path, note)
                 VALUES (?1, 'openai', 'fixture', CAST(X'80' AS TEXT))",
                [THREAD_ID],
            )
            .unwrap();
        let schema = thread_schema_columns(&connection).unwrap();
        let signature = thread_schema_signature(&schema).unwrap();
        let expected = schema
            .iter()
            .map(|column| CatalogColumnValue {
                ordinal: column.ordinal,
                name: column.name.clone(),
                affinity: column.affinity,
                value: CatalogSqlValue::Null,
            })
            .collect::<Vec<_>>();
        let (first, first_fingerprint) =
            read_catalog_row(&connection, THREAD_ID, &signature, &expected)
                .unwrap()
                .unwrap();
        assert!(first.iter().any(|column| {
            column.name == "note" && matches!(column.value, CatalogSqlValue::TextBytesBase64(_))
        }));
        connection
            .execute(
                "UPDATE threads SET note = CAST(X'81' AS TEXT) WHERE id = ?1",
                [THREAD_ID],
            )
            .unwrap();
        let (_, second_fingerprint) =
            read_catalog_row(&connection, THREAD_ID, &signature, &expected)
                .unwrap()
                .unwrap();
        assert_ne!(first_fingerprint, second_fingerprint);
    }

    #[test]
    fn default_affinity_is_frozen_as_the_sqlite_storage_value() {
        let (_temp, paths) = fixture("agentharbor_native", "openai");
        replace_threads_schema(
            &paths,
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                model_provider TEXT,
                rollout_path TEXT,
                future_default TEXT NOT NULL DEFAULT 0
            );",
        );
        let plan = build_recovery_plan(&paths, THREAD_ID).unwrap();
        let CatalogMutation::InsertRow { columns, .. } = plan.mutation.unwrap() else {
            panic!("expected insert mutation");
        };
        assert!(columns.iter().any(|column| {
            column.name == "future_default" && column.value == CatalogSqlValue::Text("0".into())
        }));

        replace_threads_schema(
            &paths,
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                model_provider TEXT,
                rollout_path TEXT,
                impossible TEXT NOT NULL DEFAULT NULL
            );",
        );
        let preview = preview_thread_recovery(&paths, THREAD_ID).unwrap();
        assert_eq!(preview.status, ThreadRecoveryStatus::SchemaUnsupported);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn threads_trigger_side_effects_are_rejected_before_insertion() {
        let (_temp, paths) = fixture("agentharbor_native", "openai");
        replace_threads_schema(
            &paths,
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                model_provider TEXT,
                rollout_path TEXT,
                future_nullable TEXT
            );
            CREATE TABLE recovery_audit (thread_id TEXT NOT NULL);
            CREATE TRIGGER mutate_recovered_thread AFTER INSERT ON threads
            BEGIN
                INSERT INTO recovery_audit (thread_id) VALUES (NEW.id);
            END;",
        );
        let original_rollout =
            std::fs::read(find_rollout(&paths, THREAD_ID).unwrap().unwrap().0).unwrap();
        let preview = preview_thread_recovery(&paths, THREAD_ID).unwrap();
        assert_eq!(preview.status, ThreadRecoveryStatus::SchemaUnsupported);
        assert!(apply_thread_recovery_inner(&paths, THREAD_ID, &preview.revision, false).is_err());
        assert!(load_database_thread(&paths.codex_state_db_path, THREAD_ID)
            .unwrap()
            .is_none());
        let rollout_path = find_rollout(&paths, THREAD_ID).unwrap().unwrap().0;
        assert_eq!(std::fs::read(rollout_path).unwrap(), original_rollout);
        let audit_rows: i64 = Connection::open(&paths.codex_state_db_path)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM recovery_audit", [], |row| row.get(0))
            .unwrap();
        assert_eq!(audit_rows, 0);
    }

    use std::io::Write;
}
