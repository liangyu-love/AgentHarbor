use crate::core::{
    models_dev,
    repository::Repository,
    router,
    usage_ledger::{self, LedgerPaths, UsagePayload, UsageRange},
};
use crate::platform::paths::CodexPaths;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::State;

/// Where the local transcripts and the usage ledger live.
pub(crate) struct LocalUsageDirs {
    paths: CodexPaths,
    claude_home: PathBuf,
}

impl LocalUsageDirs {
    pub(crate) fn of(repo: &Mutex<Repository>) -> Result<Self, String> {
        let paths = repo
            .lock()
            .map_err(|error| error.to_string())?
            .paths()
            .clone();
        let claude_home = if let Some(path) = std::env::var_os("AGENT_HARBOR_CLAUDE_HOME")
            .or_else(|| std::env::var_os("CLAUDE_CONFIG_DIR"))
        {
            PathBuf::from(path)
        } else {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".claude")
        };
        Ok(Self { paths, claude_home })
    }

    fn ledger(&self) -> LedgerPaths<'_> {
        LedgerPaths {
            codex_home: &self.paths.codex_home,
            claude_home: &self.claude_home,
            data_dir: &self.paths.app_data_dir,
        }
    }

    /// Counts the transcripts into the ledger while they still exist, so the
    /// sessions about to be deleted keep their usage. Deletion goes on regardless.
    pub(crate) fn record_before_delete(&self) {
        if let Err(error) = usage_ledger::record(&self.ledger()) {
            log::warn!("usage ledger sync before deletion failed: {error}");
        }
    }
}

#[tauri::command]
pub async fn get_token_usage(
    repo: State<'_, Mutex<Repository>>,
    range: Option<UsageRange>,
) -> Result<UsagePayload, String> {
    let dirs = LocalUsageDirs::of(&repo)?;
    tauri::async_runtime::spawn_blocking(move || {
        // Without a readable router store, calls are still counted; relay
        // turns just lose their provider names and rates.
        let store = router::load_store(&dirs.paths).ok();
        let catalog = models_dev::load(&dirs.paths.app_data_dir);
        usage_ledger::load(
            &dirs.ledger(),
            store.as_ref(),
            &catalog,
            range.unwrap_or_default(),
        )
    })
    .await
    .map_err(|error| error.to_string())?
}
