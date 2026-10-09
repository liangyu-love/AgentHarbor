//! Read-only view of the provider recorded for each thread in Codex's catalog.
use super::*;

/// Threads whose catalog provider equals `provider`.
pub fn threads_with_provider(paths: &CodexPaths, provider: &str) -> Result<Vec<String>, CoreError> {
    Ok(database_providers(&paths.codex_state_db_path)?
        .into_iter()
        .filter(|(_, stored)| stored == provider)
        .map(|(thread_id, _)| thread_id)
        .collect())
}

fn database_providers(database_path: &Path) -> Result<Vec<(String, String)>, CoreError> {
    match std::fs::metadata(database_path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => {
            return Err(CoreError::InvalidData(
                "Codex thread catalog is not a regular file".into(),
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    }
    let connection = open_database(database_path, true)?;
    let columns = crate::core::threads::table_columns(&connection, "threads")?;
    if columns.is_empty() {
        return Ok(Vec::new());
    }
    if !columns.contains("id") || !columns.contains("model_provider") {
        return Err(CoreError::InvalidData(
            "Codex thread catalog has an unsupported provider schema".into(),
        ));
    }
    let mut statement = connection
        .prepare("SELECT id, COALESCE(model_provider, '') FROM threads")
        .map_err(sqlite_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(sqlite_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(sqlite_error)
}
