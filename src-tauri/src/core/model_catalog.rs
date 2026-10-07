//! Shared official model metadata, separate from channel IDs and request bodies.
//! Validate before publishing; a failed/partial refresh retains the last good copy.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    io::Read,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Instant, SystemTime},
};

const MAX_BYTES: u64 = 16 * 1024 * 1024;
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub models: Vec<Value>,
    pub updated_at: Option<i64>,
    #[serde(default)]
    pub stale: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub model_updated_at: HashMap<String, Option<i64>>,
}
#[derive(Clone)]
struct Cached {
    stamp: Option<(u64, SystemTime, Option<SystemTime>)>,
    backup: Option<PathBuf>,
    checked: Instant,
    snapshot: Snapshot,
    /// A read-only preview must not prevent the next ordinary read from
    /// persisting the fresh last-good backup.
    write_backup: bool,
}
fn cache() -> &'static Mutex<HashMap<PathBuf, Cached>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Cached>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}
fn parse(bytes: &[u8]) -> Option<Vec<Value>> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    let entries = value["models"].as_array()?;
    if entries.is_empty() || entries.len() > 10_000 {
        return None;
    }
    let mut models = Vec::new();
    for entry in entries {
        let slug = entry["slug"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 200 && !s.chars().any(char::is_control))?;
        if super::model_defaults::channel_entry(slug) {
            continue;
        }
        for key in ["context_window", "max_output_tokens"] {
            if entry
                .get(key)
                .is_some_and(|v| !v.is_null() && v.as_u64().is_none_or(|n| n == 0))
            {
                return None;
            }
        }
        if let Some(levels) = entry
            .get("supported_reasoning_levels")
            .filter(|v| !v.is_null())
        {
            if levels.as_array().is_none_or(|a| {
                a.iter().any(|v| {
                    v["effort"].as_str().is_none_or(str::is_empty)
                        || v["description"].as_str().is_none()
                })
            }) {
                return None;
            }
        }
        models.push(entry.clone());
    }
    (!models.is_empty()).then_some(models)
}
pub(crate) fn load(source: &Path, backup: Option<&Path>) -> Snapshot {
    load_with_backup_mode(source, backup, true)
}

fn load_with_backup_mode(source: &Path, backup: Option<&Path>, write_backup: bool) -> Snapshot {
    let mut cache = cache().lock().unwrap_or_else(|e| e.into_inner());
    let stamp = std::fs::metadata(source)
        .ok()
        .and_then(|m| Some((m.len(), m.modified().ok()?, m.created().ok())));
    if let Some(cached) = cache.get(source).filter(|c| {
        c.stamp == stamp
            && c.backup.as_deref() == backup
            && (!write_backup || c.write_backup)
            && (!c.snapshot.stale || c.checked.elapsed().as_secs() < 30)
    }) {
        return cached.snapshot.clone();
    }
    let fresh = stamp
        .filter(|(len, _, _)| *len <= MAX_BYTES)
        .and_then(|_| bounded_read(source))
        .and_then(|b| parse(&b));
    let snapshot = if let Some(models) = fresh {
        let updated_at = stamp
            .and_then(|(_, t, _)| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64);
        let model_updated_at = models
            .iter()
            .filter_map(|m| Some((m["slug"].as_str()?.to_owned(), updated_at)))
            .collect();
        let snapshot = Snapshot {
            models,
            updated_at,
            stale: false,
            error: None,
            model_updated_at,
        };
        if let Some(backup) = backup.filter(|_| write_backup) {
            let data = serde_json::json!({"source": source.to_string_lossy(), "models": snapshot.models, "updatedAt": snapshot.updated_at});
            if let Ok(bytes) = serde_json::to_vec(&data) {
                let _ = super::storage::write_atomic(backup, &bytes);
            }
        }
        snapshot
    } else {
        let previous = cache
            .get(source)
            .filter(|c| !c.snapshot.models.is_empty())
            .map(|c| c.snapshot.clone())
            .or_else(|| {
                let backup = backup?;
                if std::fs::metadata(backup).ok()?.len() > MAX_BYTES {
                    return None;
                }
                let bytes = bounded_read(backup)?;
                let v: Value = serde_json::from_slice(&bytes).ok()?;
                if v["source"].as_str()? != source.to_string_lossy() {
                    return None;
                }
                let models = parse(&bytes)?;
                let updated_at = v["updatedAt"].as_i64();
                let model_updated_at = models
                    .iter()
                    .filter_map(|m| Some((m["slug"].as_str()?.to_owned(), updated_at)))
                    .collect();
                Some(Snapshot {
                    models,
                    updated_at,
                    model_updated_at,
                    ..Default::default()
                })
            })
            .unwrap_or_default();
        Snapshot {
            stale: true,
            error: Some("catalog_unavailable_or_invalid".into()),
            ..previous
        }
    };
    if cache.len() >= 16 && !cache.contains_key(source) {
        if let Some(key) = cache.keys().next().cloned() {
            cache.remove(&key);
        }
    }
    cache.insert(
        source.to_path_buf(),
        Cached {
            stamp,
            backup: backup.map(Path::to_path_buf),
            checked: Instant::now(),
            snapshot: snapshot.clone(),
            write_backup,
        },
    );
    snapshot
}
pub(crate) fn official(codex_home: &Path, app_data: &Path) -> Snapshot {
    official_with_backup_mode(codex_home, app_data, true)
}

/// The current account's catalog is authoritative for official picker entries.
/// Historical router catalogs provide capability metadata, not account access:
/// merging them here would revive removed models or override `visibility`.
pub(crate) fn account_catalog(codex_home: &Path, app_data: &Path) -> Snapshot {
    account_catalog_with_backup_mode(codex_home, app_data, true)
}

/// Resolve the same official sources and stale fallback without creating or
/// refreshing a last-good file while an unsaved provider draft is being edited.
pub(crate) fn official_read_only(codex_home: &Path, app_data: &Path) -> Snapshot {
    official_with_backup_mode(codex_home, app_data, false)
}

fn official_with_backup_mode(codex_home: &Path, app_data: &Path, write_backup: bool) -> Snapshot {
    let mut snapshot = account_catalog_with_backup_mode(codex_home, app_data, write_backup);
    let previous = load(&app_data.join("previous-router-catalog.json"), None);
    let mut slugs: HashSet<String> = snapshot
        .models
        .iter()
        .filter_map(|m| m["slug"].as_str().map(str::to_owned))
        .collect();
    for entry in previous.models {
        if let Some(slug) = entry["slug"].as_str() {
            if slugs.insert(slug.into()) {
                snapshot
                    .model_updated_at
                    .insert(slug.into(), previous.updated_at);
                snapshot.models.push(entry);
            }
        }
    }
    snapshot
}

fn account_catalog_with_backup_mode(
    codex_home: &Path,
    app_data: &Path,
    write_backup: bool,
) -> Snapshot {
    load_with_backup_mode(
        &codex_home.join("models_cache.json"),
        Some(&app_data.join("model-capabilities-last-good.json")),
        write_backup,
    )
}

fn bounded_read(path: &Path) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= MAX_BYTES).then_some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn historical_metadata_cannot_expand_the_current_account_catalog() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let app_data = dir.path().join("app");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&app_data).unwrap();
        let current = serde_json::json!([
            {"slug":"current-model","visibility":"list"},
            {"slug":"hidden-model","visibility":"hide"},
            {"slug":"relay::relay-model","visibility":"list"}
        ]);
        std::fs::write(
            home.join("models_cache.json"),
            serde_json::json!({"models":current}).to_string(),
        )
        .unwrap();
        std::fs::write(
            app_data.join("previous-router-catalog.json"),
            serde_json::json!({"models":[
                {"slug":"retired-model","visibility":"list"},
                {"slug":"hidden-model","visibility":"list"}
            ]})
            .to_string(),
        )
        .unwrap();
        let menu = account_catalog(&home, &app_data);
        assert_eq!(menu.models, current.as_array().unwrap()[..2]);
        let metadata = official(&home, &app_data);
        assert!(metadata.models.iter().any(|entry| entry["slug"] == "retired-model"));
        assert_eq!(metadata.models[1]["visibility"], "hide");

        // A bad refresh may keep this account's last good catalog, but never
        // promote the unrelated historical metadata into available models.
        std::fs::write(home.join("models_cache.json"), b"{broken").unwrap();
        let stale = account_catalog(&home, &app_data);
        assert!(stale.stale);
        assert_eq!(stale.models, menu.models);
    }

    #[test]
    fn read_only_official_preserves_fallback_and_defers_backup_writes() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let app_data = dir.path().join("app");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&app_data).unwrap();
        let source = home.join("models_cache.json");
        let backup = app_data.join("model-capabilities-last-good.json");
        std::fs::write(
            &source,
            r#"{"models":[{"slug":"gpt-6-luna","context_window":400000}]}"#,
        )
        .unwrap();
        let preview = official_read_only(&home, &app_data);
        assert_eq!(preview.models[0]["context_window"], 400_000);
        assert!(!backup.exists());
        let ordinary = official(&home, &app_data);
        assert_eq!(preview.models, ordinary.models);
        assert!(backup.exists());
        let original_backup = std::fs::read(&backup).unwrap();
        std::fs::write(&source, b"{broken").unwrap();
        let stale = official_read_only(&home, &app_data);
        assert!(stale.stale);
        assert_eq!(stale.models, ordinary.models);
        assert_eq!(std::fs::read(&backup).unwrap(), original_backup);
    }

    #[test]
    fn malformed_empty_and_missing_refresh_keep_last_valid_data() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.json");
        let backup = dir.path().join("backup.json");
        std::fs::write(
            &source,
            r#"{"models":[{"slug":"gpt-6-astra","context_window":400000}]}"#,
        )
        .unwrap();
        let good = load(&source, Some(&backup));
        assert!(!good.stale);
        for bad in [
            "{",
            r#"{"models":[]}"#,
            r#"{"models":[{"slug":"bad","context_window":"huge"}]}"#,
        ] {
            std::fs::write(&source, bad).unwrap();
            let stale = load(&source, Some(&backup));
            assert!(stale.stale);
            assert_eq!(stale.models, good.models);
            assert_eq!(stale.updated_at, good.updated_at);
        }
        std::fs::remove_file(&source).unwrap();
        cache().lock().unwrap().remove(&source);
        assert_eq!(load(&source, Some(&backup)).models, good.models);
    }

    #[test]
    fn enabling_backup_after_a_memory_read_still_persists_it() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.json");
        let backup = dir.path().join("backup.json");
        std::fs::write(
            &source,
            r#"{"models":[{"slug":"model","context_window":1000}]}"#,
        )
        .unwrap();
        load(&source, None);
        load(&source, Some(&backup));
        assert!(backup.exists());
    }
}
