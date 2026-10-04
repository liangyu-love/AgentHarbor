//! Reversible ownership of the router's six nonsecret root settings.
//! The complete config (including provider credentials) stays in the existing
//! config repository's backups, never in this sidecar.

use super::{
    catalog_path, model_provider_is_ours, remove_managed_provider_entries, Store,
    CATALOG_PROVIDER_KEY, LOCAL_AUTH_HEADER, PREVIOUS_CATALOG_PROVIDER_KEY,
};
use crate::{
    core::{models::CoreError, storage},
    platform::paths::CodexPaths,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Mutex, MutexGuard, OnceLock},
};
use toml_edit::{DocumentMut, Item, Value};

const KEYS: [&str; 6] = [
    "model",
    "model_provider",
    "model_catalog_json",
    "model_context_window",
    "model_auto_compact_token_limit",
    "preferred_auth_method",
];
const VERSION: u32 = 1;
static WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub(super) fn guard() -> Result<MutexGuard<'static, ()>, CoreError> {
    WRITE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| snapshot_error())
}

pub(super) fn path(paths: &CodexPaths) -> PathBuf {
    paths.app_data_dir.join("codex-router-overrides.json")
}

fn snapshot_error() -> CoreError {
    CoreError::OperationFailed("newapi.errors.routerSnapshot".into())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum Scalar {
    Text(String),
    Integer(i64),
}

type Fields = BTreeMap<String, Option<Scalar>>;

fn fields(document: &DocumentMut) -> Result<Fields, CoreError> {
    KEYS.into_iter()
        .map(|key| {
            let scalar = document
                .get(key)
                .map(|item| {
                    if matches!(
                        key,
                        "model_context_window" | "model_auto_compact_token_limit"
                    ) {
                        item.as_integer().map(Scalar::Integer)
                    } else {
                        item.as_str().map(|value| Scalar::Text(value.to_owned()))
                    }
                    .ok_or_else(snapshot_error)
                })
                .transpose()?;
            Ok((key.to_owned(), scalar))
        })
        .collect()
}

fn validate_fields(values: &Fields) -> Result<(), CoreError> {
    if values.keys().any(|key| !KEYS.contains(&key.as_str())) {
        return Err(snapshot_error());
    }
    for key in KEYS {
        match values.get(key) {
            None | Some(None) => {}
            Some(Some(Scalar::Integer(_)))
                if matches!(
                    key,
                    "model_context_window" | "model_auto_compact_token_limit"
                ) => {}
            Some(Some(Scalar::Text(_)))
                if !matches!(
                    key,
                    "model_context_window" | "model_auto_compact_token_limit"
                ) => {}
            _ => return Err(snapshot_error()),
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Action {
    Apply,
    Remove,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    action: Action,
    before_hash: String,
    after_hash: String,
    applied: Fields,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Snapshot {
    version: u32,
    config_path: PathBuf,
    // A projection of just KEYS, retaining their original comments/formatting.
    original: String,
    applied: Option<Fields>,
    pending: Option<Pending>,
    #[serde(skip)]
    auth_owned: bool,
}

fn hash(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn parse_original(snapshot: &Snapshot) -> Result<DocumentMut, CoreError> {
    let document = snapshot
        .original
        .parse::<DocumentMut>()
        .map_err(|_| snapshot_error())?;
    if document.iter().any(|(key, _)| !KEYS.contains(&key)) {
        return Err(snapshot_error());
    }
    fields(&document)?;
    Ok(document)
}

pub(super) fn load(paths: &CodexPaths, current: &str) -> Result<Option<Snapshot>, CoreError> {
    let bytes = match std::fs::read(path(paths)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut snapshot: Snapshot = serde_json::from_slice(&bytes).map_err(|_| snapshot_error())?;
    if snapshot.version != VERSION || snapshot.config_path != paths.config_path {
        return Err(snapshot_error());
    }
    parse_original(&snapshot)?;
    if let Some(applied) = &snapshot.applied {
        validate_fields(applied)?;
    }
    if let Some(pending) = snapshot.pending.take() {
        validate_fields(&pending.applied)?;
        let current_hash = hash(current);
        if current_hash == pending.after_hash {
            match pending.action {
                Action::Apply => snapshot.applied = Some(pending.applied),
                // A completed removal with a failed sidecar cleanup must not
                // become the baseline of a later, unrelated router session.
                Action::Remove => {
                    discard(paths)?;
                    return Ok(None);
                }
            }
        } else if current_hash != pending.before_hash {
            // An external edit after an interrupted write makes its outcome
            // ambiguous. Keep both files for an explicit recovery.
            return Err(snapshot_error());
        }
    }
    // A failed first install never acquired any settings. A retry captures
    // the still-unmodified config afresh.
    Ok(snapshot.applied.is_some().then_some(snapshot))
}

impl Snapshot {
    pub(super) fn selected_provider_is_ours(&self, document: &DocumentMut) -> bool {
        self.applied.as_ref().is_some_and(|applied| {
            document
                .get("model_provider")
                .and_then(Item::as_str)
                .is_some_and(|current| {
                    applied.get("model_provider") == Some(&Some(Scalar::Text(current.into())))
                })
        })
    }

    pub(super) fn capture(paths: &CodexPaths, document: &DocumentMut) -> Result<Self, CoreError> {
        fields(document)?;
        let mut original = DocumentMut::new();
        for key in KEYS {
            if let Some((formatted, value)) = document.as_table().get_key_value(key) {
                original
                    .as_table_mut()
                    .insert_formatted(formatted, value.clone());
            }
        }
        Ok(Self {
            version: VERSION,
            config_path: paths.config_path.clone(),
            original: original.to_string(),
            applied: None,
            pending: None,
            auth_owned: false,
        })
    }

    pub(super) fn apply_auth(
        &mut self,
        document: &mut DocumentMut,
        accountless: bool,
    ) -> Result<(), CoreError> {
        if accountless {
            set_value(
                document,
                "preferred_auth_method",
                Some(Value::from("apikey")),
            );
            self.auth_owned = true;
        } else if let Some(applied) = &self.applied {
            let current = fields(document)?;
            if current.get("preferred_auth_method") == applied.get("preferred_auth_method") {
                restore_field(document, &parse_original(self)?, "preferred_auth_method");
                self.auth_owned = true;
            }
        }
        Ok(())
    }

    pub(super) fn restore(
        &self,
        paths: &CodexPaths,
        document: &mut DocumentMut,
    ) -> Result<(), CoreError> {
        let Some(applied) = &self.applied else {
            return Ok(());
        };
        // A provider switch is an external takeover of the whole route. The
        // six root settings form one choice; restoring individual old values
        // here would mix our former model/catalog/auth with the new provider.
        if !self.selected_provider_is_ours(document) {
            return Ok(());
        }
        let current = fields(document)?;
        let original = parse_original(self)?;
        let router_alias = is_installed_router_alias(paths, document);
        for key in KEYS {
            // Ownership is value-based: comments can be edited independently,
            // and a user-changed or user-deleted setting remains untouched.
            if current.get(key) == applied.get(key) || (key == "model" && router_alias) {
                restore_field(document, &original, key);
            }
        }
        Ok(())
    }

    pub(super) fn prepare(
        &mut self,
        paths: &CodexPaths,
        before: &str,
        after: &DocumentMut,
        action: Action,
    ) -> Result<(), CoreError> {
        let mut after_text = after.to_string();
        if !after_text.is_empty() && !after_text.ends_with('\n') {
            after_text.push('\n');
        }
        let mut applied = fields(after)?;
        if matches!(action, Action::Apply) && !self.auth_owned {
            // Preserving a user's auth preference is not ownership of it.
            applied.remove("preferred_auth_method");
        }
        self.pending = Some(Pending {
            action,
            before_hash: hash(before),
            after_hash: hash(&after_text),
            applied,
        });
        // This durable intent precedes the repository's backup and atomic
        // replace, so a failed config write cannot lose the original values.
        storage::write_atomic(&path(paths), &serde_json::to_vec(self)?)
    }

    pub(super) fn finish(mut self, paths: &CodexPaths) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        let result = match pending.action {
            Action::Apply => {
                self.applied = Some(pending.applied);
                serde_json::to_vec(&self)
                    .map_err(CoreError::from)
                    .and_then(|bytes| storage::write_atomic(&path(paths), &bytes))
            }
            Action::Remove => discard(paths),
        };
        if result.is_err() {
            // The already-durable pending record can settle this on the next
            // operation. Do not report the successful config write as failed.
            log::warn!("Router config changed; snapshot finalization will retry on next use");
        }
    }
}

pub(super) fn discard(paths: &CodexPaths) -> Result<(), CoreError> {
    match std::fs::remove_file(path(paths)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn set_value(document: &mut DocumentMut, key: &str, value: Option<Value>) {
    if let Some(mut value) = value {
        if let Some(current) = document.get(key).and_then(Item::as_value) {
            *value.decor_mut() = current.decor().clone();
        }
        document[key] = Item::Value(value);
    } else {
        document.remove(key);
    }
}

fn restore_field(document: &mut DocumentMut, original: &DocumentMut, key: &str) {
    if let Some((formatted, item)) = original.as_table().get_key_value(key) {
        if document.get(key).is_some() {
            set_value(document, key, item.as_value().cloned());
        } else {
            document
                .as_table_mut()
                .insert_formatted(formatted, item.clone());
        }
    } else {
        document.remove(key);
    }
}

fn unrelated_config(document: &DocumentMut) -> Result<toml::Value, CoreError> {
    let mut projected = document.clone();
    for key in KEYS {
        projected.remove(key);
    }
    remove_managed_provider_entries(&mut projected);
    if projected
        .get("model_providers")
        .and_then(Item::as_table_like)
        .is_some_and(|table| table.iter().next().is_none())
    {
        projected.remove("model_providers");
    }
    projected.to_string().parse().map_err(CoreError::from)
}

fn is_installed_router_alias(paths: &CodexPaths, document: &DocumentMut) -> bool {
    if super::native_config::installed_catalog_model(paths, document) {
        return true;
    }
    if !model_provider_is_ours(document)
        || document.get("model_catalog_json").and_then(Item::as_str)
            != Some(catalog_path(paths).to_string_lossy().as_ref())
    {
        return false;
    }
    let Some(model) = document.get("model").and_then(Item::as_str) else {
        return false;
    };
    if !model
        .split_once("::")
        .is_some_and(|(id, model)| !id.is_empty() && !model.is_empty())
    {
        return false;
    }
    std::fs::read(catalog_path(paths))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|catalog| {
            catalog
                .get("models")
                .and_then(serde_json::Value::as_array)
                .map(|models| {
                    models.iter().any(|entry| {
                        entry.get("slug").and_then(serde_json::Value::as_str) == Some(model)
                    })
                })
        })
        .unwrap_or(false)
}

/// Older builds did not record field ownership. Only accept an unambiguous
/// pre-router backup with the same unrelated settings and provider definitions.
/// A missing/ambiguous baseline stops the operation without guessing defaults.
pub(super) fn recover_legacy(
    paths: &CodexPaths,
    current: &DocumentMut,
    store: &Store,
) -> Result<Snapshot, CoreError> {
    let provider_key = current
        .get("model_provider")
        .and_then(Item::as_str)
        .filter(|key| {
            matches!(
                *key,
                CATALOG_PROVIDER_KEY
                    | PREVIOUS_CATALOG_PROVIDER_KEY
                    | super::native_config::PROVIDER_KEY
            )
        })
        .ok_or_else(snapshot_error)?;
    let provider = current
        .get("model_providers")
        .and_then(|providers| providers.get(provider_key))
        .ok_or_else(snapshot_error)?;
    let is_loopback_route = provider
        .get("base_url")
        .and_then(Item::as_str)
        .and_then(|url| url.strip_prefix("http://127.0.0.1:"))
        .and_then(|url| url.strip_suffix("/codex/v1"))
        .is_some_and(|port| port.parse::<u16>().is_ok_and(|port| port != 0));
    let local_token = provider
        .get("http_headers")
        .and_then(|headers| headers.get(LOCAL_AUTH_HEADER))
        .and_then(Item::as_str)
        .or_else(|| {
            provider
                .get("experimental_bearer_token")
                .and_then(Item::as_str)
        });
    let native = provider_key == super::native_config::PROVIDER_KEY;
    if if native {
        !super::native_config::proven_installation(paths, current)
    } else {
        !is_loopback_route
            || !matches!(
                provider.get("wire_api").and_then(Item::as_str),
                Some("responses" | "chat")
            )
            || local_token != Some(store.router.bearer_token.as_str())
            || store.router.bearer_token.is_empty()
    } {
        return Err(snapshot_error());
    }
    let unrelated = unrelated_config(current)?;
    let entries = std::fs::read_dir(paths.app_data_dir.join("config-backups"))
        .map_err(|_| snapshot_error())?;
    let mut baseline: Option<DocumentMut> = None;
    for entry in entries {
        let entry = entry.map_err(|_| snapshot_error())?;
        let backup_path = entry.path();
        if !entry.file_type().map_err(|_| snapshot_error())?.is_file()
            || backup_path.extension().and_then(|value| value.to_str()) != Some("toml")
            || !backup_path
                .file_stem()
                .and_then(|value| value.to_str())
                .is_some_and(|value| {
                    uuid::Uuid::parse_str(value).is_ok()
                        || (native
                            && value
                                .strip_prefix("agentharbor-")
                                .and_then(|value| value.get(value.len().checked_sub(36)?..))
                                .is_some_and(|value| uuid::Uuid::parse_str(value).is_ok()))
                })
        {
            continue;
        }
        let backup = std::fs::read_to_string(&backup_path)
            .map_err(|_| snapshot_error())?
            .parse::<DocumentMut>()
            .map_err(|_| snapshot_error())?;
        if model_provider_is_ours(&backup)
            || backup.get("model_catalog_json").and_then(Item::as_str)
                == Some(catalog_path(paths).to_string_lossy().as_ref())
            || unrelated_config(&backup)? != unrelated
        {
            continue;
        }
        if let Some(previous) = &baseline {
            if fields(previous)? != fields(&backup)? {
                return Err(snapshot_error());
            }
        } else {
            baseline = Some(backup);
        }
    }
    let baseline = baseline.ok_or_else(snapshot_error)?;
    let mut snapshot = Snapshot::capture(paths, &baseline)?;
    if native {
        let mut applied = Fields::new();
        let current_fields = fields(current)?;
        for key in ["model_provider", "model_catalog_json"] {
            applied.insert(key.into(), current_fields[key].clone());
        }
        if super::native_config::installed_catalog_model(paths, current) {
            applied.insert("model".into(), current_fields["model"].clone());
        }
        // Infer limits only from a provider matching the installed endpoint and
        // owned credential. Snapshot::restore preserves drifted user fields.
        if let Some(previous) = store
            .providers
            .iter()
            .find(|previous| super::native_config::matches_installation(paths, current, previous))
        {
            let (context, threshold) = super::native_config::native_limits(paths, store, previous);
            applied.insert(
                "model_context_window".into(),
                context.map(|v| Scalar::Integer(v as i64)),
            );
            applied.insert(
                "model_auto_compact_token_limit".into(),
                threshold.map(|v| Scalar::Integer(v as i64)),
            );
        }
        applied.insert(
            "preferred_auth_method".into(),
            Some(Scalar::Text("apikey".into())),
        );
        snapshot.applied = Some(applied);
        return Ok(snapshot);
    }
    let current_model = current.get("model").and_then(Item::as_str);
    let previous_provider = current_model
        .and_then(|model| model.split_once("::"))
        .and_then(|(id, _)| store.providers.iter().find(|provider| provider.id == id));
    let model_is_catalog_alias = is_installed_router_alias(paths, current);
    let mut applied = Fields::new();
    applied.insert(
        "model_provider".into(),
        Some(Scalar::Text(provider_key.into())),
    );
    applied.insert(
        "model_catalog_json".into(),
        Some(Scalar::Text(
            catalog_path(paths).to_string_lossy().to_string(),
        )),
    );
    // The caller may already have selected/deleted a provider. An alias in the
    // still-installed catalog identifies the old route without relying on the
    // current active provider or requiring it to remain routable.
    if model_is_catalog_alias
        || previous_provider.is_some_and(|provider| {
            current_model == Some(format!("{}::{}", provider.id, provider.model).as_str())
        })
    {
        applied.insert(
            "model".into(),
            current_model.map(|value| Scalar::Text(value.into())),
        );
    }
    if let Some(previous) = previous_provider {
        // Record the values the native path actually applies (the provider
        // setting plus the derived conservative threshold), not the legacy
        // fixed field, so drift detection and restore stay truthful.
        let (context, threshold) = super::native_config::native_limits(paths, store, previous);
        applied.insert(
            "model_context_window".into(),
            context.map(|value| Scalar::Integer(value as i64)),
        );
        applied.insert(
            "model_auto_compact_token_limit".into(),
            threshold.map(|value| Scalar::Integer(value as i64)),
        );
    }
    // With no old provider metadata, context overrides cannot be distinguished
    // from user edits. Leave those fields alone instead of claiming ownership.
    applied.insert(
        "preferred_auth_method".into(),
        fields(&baseline)?["preferred_auth_method"].clone(),
    );
    if store.router.accountless
        && current.get("preferred_auth_method").and_then(Item::as_str) == Some("apikey")
        && provider.get("requires_openai_auth").and_then(Item::as_bool) == Some(false)
    {
        // Old signed-in routes also set requires_openai_auth=false. Only the
        // persisted accountless choice proves this auth preference was ours.
        applied.insert(
            "preferred_auth_method".into(),
            Some(Scalar::Text("apikey".into())),
        );
    }
    snapshot.applied = Some(applied);
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::router::{
        apply_routing_config, remove_routing_config, Provider, RouterSettings,
    };

    const ORIGINAL: &str = "# user config\nmodel = 'original-model' # chosen model\nmodel_provider = 'custom'\nmodel_catalog_json = 'custom-models.json'\n# context note\nmodel_context_window = 90000 # original context\nmodel_auto_compact_token_limit = 80000\npreferred_auth_method = 'chatgpt' # login preference\ntheme = 'dark'\n[model_providers.custom]\nname = 'Custom'\nbase_url = 'https://fixture.invalid'\nexperimental_bearer_token = 'fixture-original-secret'\n[mcp_servers.tools]\ncommand = 'node' # MCP comment\n";

    /// Builds before the sidecar existed wrote bare `<uuid>.toml` backups. Simulate
    /// such an install by applying with this build, then giving the new backup the
    /// legacy name and dropping the sidecar.
    fn apply_as_legacy_install(paths: &CodexPaths, store: &Store) -> PathBuf {
        let applied = apply_routing_config(paths, store).unwrap();
        let backup = applied.backup_path.map(PathBuf::from).unwrap();
        let legacy = backup.with_file_name(format!("{}.toml", uuid::Uuid::new_v4()));
        std::fs::rename(&backup, &legacy).unwrap();
        std::fs::remove_file(path(paths)).unwrap();
        legacy
    }

    fn fixture(original: &str) -> (tempfile::TempDir, CodexPaths, Store) {
        let directory = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(directory.path().join("codex"));
        std::fs::create_dir_all(&paths.codex_home).unwrap();
        std::fs::write(&paths.config_path, original).unwrap();
        let store = Store {
            providers: vec![Provider {
                id: "fixture-relay".into(),
                name: "Fixture".into(),
                base_url: "https://fixture.invalid".into(),
                api_key: "fixture-relay-secret".into(),
                model: "relay-model".into(),
                enabled: true,
                ..Default::default()
            }],
            router: RouterSettings {
                enabled: true,
                bearer_token: "fixture-local-secret".into(),
                ..Default::default()
            },
            active_provider_id: Some("fixture-relay".into()),
            ..Default::default()
        };
        (directory, paths, store)
    }

    fn config(paths: &CodexPaths) -> DocumentMut {
        std::fs::read_to_string(&paths.config_path)
            .unwrap()
            .parse()
            .unwrap()
    }

    fn parsed(text: &str) -> toml::Value {
        text.parse().unwrap()
    }

    fn write_config(paths: &CodexPaths, document: &DocumentMut) {
        std::fs::write(&paths.config_path, document.to_string()).unwrap();
    }

    fn assert_restored(paths: &CodexPaths, original: &str) {
        let restored = std::fs::read_to_string(&paths.config_path).unwrap();
        assert_eq!(parsed(&restored), parsed(original));
        assert!(!path(paths).exists());
        assert!(!catalog_path(paths).exists());
    }

    #[test]
    fn round_trip_restores_all_six_fields_comments_and_foreign_tables() {
        let (_directory, paths, mut store) = fixture(ORIGINAL);
        store.router.accountless = true;
        let applied = apply_routing_config(&paths, &store).unwrap();
        assert_eq!(
            std::fs::read_to_string(applied.backup_path.unwrap()).unwrap(),
            ORIGINAL
        );
        let snapshot = std::fs::read_to_string(path(&paths)).unwrap();
        for secret in [
            "fixture-original-secret",
            "fixture-relay-secret",
            "fixture-local-secret",
        ] {
            assert!(!snapshot.contains(secret));
        }
        remove_routing_config(&paths, &store).unwrap();
        assert_restored(&paths, ORIGINAL);
        let restored = std::fs::read_to_string(&paths.config_path).unwrap();
        for comment in [
            "# user config",
            "# chosen model",
            "# context note",
            "# original context",
            "# login preference",
            "# MCP comment",
        ] {
            assert!(restored.contains(comment), "lost {comment}");
        }
    }

    #[test]
    fn repeated_apply_and_accountless_changes_keep_first_baseline() {
        let (_directory, paths, mut store) = fixture(ORIGINAL);
        apply_routing_config(&paths, &store).unwrap();
        let first: Snapshot =
            serde_json::from_slice(&std::fs::read(path(&paths)).unwrap()).unwrap();
        store.providers[0].model = "next-model".into();
        store.providers[0].context_window = Some(140000);
        store.providers[0].auto_compact_limit = Some(120000);
        store.router.accountless = true;
        apply_routing_config(&paths, &store).unwrap();
        apply_routing_config(&paths, &store).unwrap();
        assert_eq!(
            config(&paths)["preferred_auth_method"].as_str(),
            Some("apikey")
        );
        store.router.accountless = false;
        apply_routing_config(&paths, &store).unwrap();
        assert_eq!(
            config(&paths)["preferred_auth_method"].as_str(),
            Some("chatgpt")
        );
        let last: Snapshot = serde_json::from_slice(&std::fs::read(path(&paths)).unwrap()).unwrap();
        assert_eq!(last.original, first.original);
        remove_routing_config(&paths, &store).unwrap();
        assert_restored(&paths, ORIGINAL);
    }

    #[test]
    fn removal_preserves_each_user_changed_or_deleted_field() {
        let (_directory, paths, mut store) = fixture(ORIGINAL);
        store.router.accountless = true;
        store.providers[0].context_window = Some(140000);
        store.providers[0].auto_compact_limit = Some(120000);
        apply_routing_config(&paths, &store).unwrap();
        let mut changed = config(&paths);
        set_value(&mut changed, "model", Some("user-model".into()));
        set_value(&mut changed, "model_provider", Some("user-provider".into()));
        set_value(
            &mut changed,
            "model_catalog_json",
            Some("user-catalog.json".into()),
        );
        set_value(
            &mut changed,
            "model_context_window",
            Some(230000_i64.into()),
        );
        changed.remove("model_auto_compact_token_limit");
        set_value(
            &mut changed,
            "preferred_auth_method",
            Some("user-auth".into()),
        );
        write_config(&paths, &changed);
        let expected = fields(&changed).unwrap();
        remove_routing_config(&paths, &store).unwrap();
        assert_eq!(fields(&config(&paths)).unwrap(), expected);
        assert_eq!(
            config(&paths)["mcp_servers"]["tools"]["command"].as_str(),
            Some("node")
        );
    }

    #[test]
    fn normal_reapply_does_not_claim_a_user_auth_preference() {
        let (_directory, paths, store) = fixture(ORIGINAL);
        apply_routing_config(&paths, &store).unwrap();
        let mut changed = config(&paths);
        set_value(
            &mut changed,
            "preferred_auth_method",
            Some("user-auth".into()),
        );
        write_config(&paths, &changed);
        apply_routing_config(&paths, &store).unwrap();
        remove_routing_config(&paths, &store).unwrap();
        assert_eq!(
            config(&paths)["preferred_auth_method"].as_str(),
            Some("user-auth")
        );
        assert_eq!(config(&paths)["model"].as_str(), Some("original-model"));
    }

    #[test]
    fn absent_original_fields_are_removed_without_inventing_defaults() {
        let original = "theme = 'dark'\n[mcp_servers.tools]\ncommand = 'node'\n";
        let (_directory, paths, mut store) = fixture(original);
        store.router.accountless = true;
        apply_routing_config(&paths, &store).unwrap();
        remove_routing_config(&paths, &store).unwrap();
        assert_restored(&paths, original);
        remove_routing_config(&paths, &store).unwrap();
        assert_restored(&paths, original);
    }

    #[test]
    fn disabling_without_an_owned_snapshot_never_rewrites_cc_switch_config() {
        let foreign = "model = 'cc-model'\nmodel_provider = 'cc-switch'\n[model_providers.cc-switch]\nbase_url = 'https://cc.invalid'";
        let (_directory, paths, store) = fixture(foreign);
        let before = std::fs::read(&paths.config_path).unwrap();
        assert!(remove_routing_config(&paths, &store).unwrap().is_none());
        assert_eq!(std::fs::read(&paths.config_path).unwrap(), before);
        assert!(!path(&paths).exists());
    }

    #[test]
    fn cc_switch_before_and_during_injection_is_preserved_on_cleanup() {
        let foreign = "model = 'cc-model'\nmodel_provider = 'cc-switch'\nmodel_catalog_json = 'cc-models.json'\npreferred_auth_method = 'chatgpt'\n[model_providers.cc-switch]\nbase_url = 'https://cc.invalid'\n";
        let (_directory, paths, store) = fixture(foreign);
        apply_routing_config(&paths, &store).unwrap();
        remove_routing_config(&paths, &store).unwrap();
        assert_eq!(
            parsed(&std::fs::read_to_string(&paths.config_path).unwrap()),
            parsed(foreign)
        );

        apply_routing_config(&paths, &store).unwrap();
        let mut takeover = config(&paths);
        set_value(&mut takeover, "model", Some("cc-new-model".into()));
        set_value(&mut takeover, "model_provider", Some("cc-switch".into()));
        set_value(
            &mut takeover,
            "model_catalog_json",
            Some("cc-new-models.json".into()),
        );
        set_value(
            &mut takeover,
            "preferred_auth_method",
            Some("cc-auth".into()),
        );
        takeover["model_providers"]["cc-switch"]["base_url"] =
            toml_edit::value("https://cc-new.invalid");
        write_config(&paths, &takeover);
        remove_routing_config(&paths, &store).unwrap();
        let cleaned = config(&paths);
        assert_eq!(fields(&cleaned).unwrap(), fields(&takeover).unwrap());
        assert_eq!(
            cleaned["model_providers"]["cc-switch"]["base_url"].as_str(),
            Some("https://cc-new.invalid")
        );
        assert!(cleaned["model_providers"]
            .get(CATALOG_PROVIDER_KEY)
            .is_none());
    }

    #[test]
    fn cc_switch_inline_provider_map_round_trips_without_shape_loss() {
        let foreign = "model = 'cc-model'\nmodel_provider = 'cc-switch'\nmodel_providers = { cc-switch = { base_url = 'https://cc.invalid', wire_api = 'responses' } }\n";
        let (_directory, paths, store) = fixture(foreign);
        apply_routing_config(&paths, &store).unwrap();
        let injected = config(&paths);
        assert!(injected["model_providers"]
            .as_value()
            .is_some_and(toml_edit::Value::is_inline_table));
        assert_eq!(
            injected["model_providers"]["cc-switch"]["base_url"].as_str(),
            Some("https://cc.invalid")
        );
        remove_routing_config(&paths, &store).unwrap();
        assert_eq!(
            parsed(&std::fs::read_to_string(&paths.config_path).unwrap()),
            parsed(foreign)
        );
        assert!(config(&paths)["model_providers"]
            .as_value()
            .is_some_and(toml_edit::Value::is_inline_table));
    }

    #[test]
    fn explicit_reenable_after_cc_switch_takeover_restores_new_baseline() {
        let (_directory, paths, store) = fixture(ORIGINAL);
        apply_routing_config(&paths, &store).unwrap();
        let mut takeover = config(&paths);
        set_value(&mut takeover, "model", Some("cc-model".into()));
        set_value(&mut takeover, "model_provider", Some("cc-switch".into()));
        set_value(
            &mut takeover,
            "model_catalog_json",
            Some("cc-catalog.json".into()),
        );
        takeover["model_providers"]["cc-switch"] = Item::Table(toml_edit::Table::new());
        takeover["model_providers"]["cc-switch"]["base_url"] =
            toml_edit::value("https://cc.invalid");
        write_config(&paths, &takeover);
        apply_routing_config(&paths, &store).unwrap();
        remove_routing_config(&paths, &store).unwrap();
        let restored = config(&paths);
        assert_eq!(fields(&restored).unwrap(), fields(&takeover).unwrap());
        assert_eq!(
            restored["model_providers"]["cc-switch"]["base_url"].as_str(),
            Some("https://cc.invalid")
        );
    }

    #[test]
    fn malformed_or_foreign_snapshot_fails_without_changing_config() {
        let (_directory, paths, store) = fixture(ORIGINAL);
        apply_routing_config(&paths, &store).unwrap();
        let installed = std::fs::read(&paths.config_path).unwrap();
        let valid = std::fs::read(path(&paths)).unwrap();
        for corrupt in [b"broken JSON".to_vec(), {
            let mut snapshot: serde_json::Value = serde_json::from_slice(&valid).unwrap();
            snapshot["config_path"] = "another-config.toml".into();
            serde_json::to_vec(&snapshot).unwrap()
        }] {
            std::fs::write(path(&paths), corrupt).unwrap();
            assert_eq!(
                apply_routing_config(&paths, &store).unwrap_err(),
                "newapi.errors.routerSnapshot"
            );
            assert_eq!(
                remove_routing_config(&paths, &store).unwrap_err(),
                "newapi.errors.routerSnapshot"
            );
            assert_eq!(std::fs::read(&paths.config_path).unwrap(), installed);
        }
    }

    #[test]
    fn snapshot_and_config_write_failures_preserve_original_and_allow_retry() {
        let (_directory, paths, store) = fixture(ORIGINAL);
        std::fs::create_dir_all(path(&paths)).unwrap();
        assert!(apply_routing_config(&paths, &store).is_err());
        assert_eq!(
            std::fs::read_to_string(&paths.config_path).unwrap(),
            ORIGINAL
        );
        std::fs::remove_dir(path(&paths)).unwrap();
        let backup_directory = paths.app_data_dir.join("config-backups");
        std::fs::write(&backup_directory, "block creating a backup directory").unwrap();
        assert!(apply_routing_config(&paths, &store).is_err());
        assert_eq!(
            std::fs::read_to_string(&paths.config_path).unwrap(),
            ORIGINAL
        );
        assert!(path(&paths).is_file());
        std::fs::remove_file(backup_directory).unwrap();
        apply_routing_config(&paths, &store).unwrap();
        remove_routing_config(&paths, &store).unwrap();
        assert_restored(&paths, ORIGINAL);
    }

    #[test]
    fn committed_config_with_pending_journal_can_still_restore() {
        let (_directory, paths, store) = fixture(ORIGINAL);
        apply_routing_config(&paths, &store).unwrap();
        let installed = config(&paths);
        let mut interrupted = Snapshot::capture(&paths, &ORIGINAL.parse().unwrap()).unwrap();
        interrupted
            .prepare(&paths, ORIGINAL, &installed, Action::Apply)
            .unwrap();
        // Simulate exiting after the atomic config replacement but before finish().
        remove_routing_config(&paths, &store).unwrap();
        assert_restored(&paths, ORIGINAL);
    }

    #[test]
    fn pending_write_with_an_external_edit_fails_closed() {
        let (_directory, paths, store) = fixture(ORIGINAL);
        apply_routing_config(&paths, &store).unwrap();
        let installed = config(&paths);
        let mut interrupted = Snapshot::capture(&paths, &ORIGINAL.parse().unwrap()).unwrap();
        interrupted
            .prepare(&paths, ORIGINAL, &installed, Action::Apply)
            .unwrap();
        let mut changed = installed;
        changed["theme"] = toml_edit::value("external-edit");
        write_config(&paths, &changed);
        let before = std::fs::read(&paths.config_path).unwrap();
        assert_eq!(
            remove_routing_config(&paths, &store).unwrap_err(),
            "newapi.errors.routerSnapshot"
        );
        assert_eq!(std::fs::read(&paths.config_path).unwrap(), before);
    }

    #[test]
    fn legacy_route_uses_a_unique_matching_backup_and_preserves_manual_values() {
        let (_directory, paths, store) = fixture(ORIGINAL);
        apply_as_legacy_install(&paths, &store);
        let mut changed = config(&paths);
        set_value(
            &mut changed,
            "model_context_window",
            Some(333000_i64.into()),
        );
        write_config(&paths, &changed);
        remove_routing_config(&paths, &store).unwrap();
        let restored = config(&paths);
        assert_eq!(restored["model"].as_str(), Some("original-model"));
        assert_eq!(restored["model_provider"].as_str(), Some("custom"));
        assert_eq!(
            restored["model_catalog_json"].as_str(),
            Some("custom-models.json")
        );
        assert_eq!(restored["model_context_window"].as_integer(), Some(333000));
        assert_eq!(
            restored["model_auto_compact_token_limit"].as_integer(),
            Some(80000)
        );
    }

    #[test]
    fn legacy_route_removes_old_alias_after_provider_switch_deletion_or_model_change() {
        for change in ["switched", "deleted", "model-changed", "disabled"] {
            let (_directory, paths, mut store) = fixture(ORIGINAL);
            apply_as_legacy_install(&paths, &store);
            match change {
                "switched" => {
                    let mut next = store.providers[0].clone();
                    next.id = "next-relay".into();
                    next.model = "next-model".into();
                    store.active_provider_id = Some(next.id.clone());
                    store.providers.push(next);
                }
                "deleted" => {
                    store.providers.clear();
                    store.active_provider_id = None;
                }
                "model-changed" => store.providers[0].model = "updated-model".into(),
                _ => store.providers[0].enabled = false,
            }
            remove_routing_config(&paths, &store).unwrap();
            let restored = config(&paths);
            assert_eq!(
                restored["model"].as_str(),
                Some("original-model"),
                "{change}"
            );
            assert_eq!(
                restored["model_provider"].as_str(),
                Some("custom"),
                "{change}"
            );
        }
    }

    #[test]
    fn legacy_route_keeps_an_unknown_user_selected_model() {
        let (_directory, paths, store) = fixture(ORIGINAL);
        apply_as_legacy_install(&paths, &store);
        let mut changed = config(&paths);
        set_value(&mut changed, "model", Some("user-model".into()));
        write_config(&paths, &changed);
        remove_routing_config(&paths, &store).unwrap();
        assert_eq!(config(&paths)["model"].as_str(), Some("user-model"));
    }

    #[test]
    fn removal_restores_catalog_selected_alias_but_keeps_external_models() {
        for (selected, external_provider, expected) in [
            ("second-relay::second-model", false, "original-model"),
            ("external-model", false, "external-model"),
            (
                "second-relay::second-model",
                true,
                "second-relay::second-model",
            ),
        ] {
            let (_directory, paths, mut store) = fixture(ORIGINAL);
            let mut second = store.providers[0].clone();
            second.id = "second-relay".into();
            second.model = "second-model".into();
            store.providers.push(second);
            apply_routing_config(&paths, &store).unwrap();
            let mut selected_config = config(&paths);
            set_value(&mut selected_config, "model", Some(selected.into()));
            if external_provider {
                set_value(
                    &mut selected_config,
                    "model_provider",
                    Some("external-provider".into()),
                );
            }
            write_config(&paths, &selected_config);
            remove_routing_config(&paths, &store).unwrap();
            assert_eq!(config(&paths)["model"].as_str(), Some(expected));
            assert!(!path(&paths).exists());
            assert_eq!(catalog_path(&paths).exists(), external_provider);
        }
    }

    #[test]
    fn completed_remove_journal_is_deleted_before_a_later_user_edit() {
        let (_directory, paths, store) = fixture(ORIGINAL);
        apply_routing_config(&paths, &store).unwrap();
        let installed = std::fs::read_to_string(&paths.config_path).unwrap();
        let mut snapshot = load(&paths, &installed).unwrap().unwrap();
        let mut restored = config(&paths);
        snapshot.restore(&paths, &mut restored).unwrap();
        remove_managed_provider_entries(&mut restored);
        snapshot
            .prepare(&paths, &installed, &restored, Action::Remove)
            .unwrap();
        write_config(&paths, &restored);
        assert!(path(&paths).is_file());
        // Retry after the config write succeeded but sidecar deletion did not.
        remove_routing_config(&paths, &store).unwrap();
        assert!(!path(&paths).exists());
        let mut changed = config(&paths);
        changed["theme"] = toml_edit::value("light");
        write_config(&paths, &changed);
        let expected = parsed(&changed.to_string());
        apply_routing_config(&paths, &store).unwrap();
        remove_routing_config(&paths, &store).unwrap();
        assert_eq!(parsed(&config(&paths).to_string()), expected);
    }

    #[test]
    fn legacy_auth_preference_requires_persisted_accountless_evidence() {
        for original in [
            ORIGINAL.to_owned(),
            ORIGINAL.replace("preferred_auth_method = 'chatgpt' # login preference\n", ""),
        ] {
            for accountless in [false, true] {
                let (_directory, paths, mut store) = fixture(&original);
                store.router.accountless = accountless;
                apply_as_legacy_install(&paths, &store);
                let mut legacy = config(&paths);
                set_value(&mut legacy, "preferred_auth_method", Some("apikey".into()));
                let provider = &mut legacy["model_providers"][CATALOG_PROVIDER_KEY];
                provider["requires_openai_auth"] = toml_edit::value(false);
                provider["experimental_bearer_token"] =
                    toml_edit::value(&store.router.bearer_token);
                provider.as_table_mut().unwrap().remove("http_headers");
                write_config(&paths, &legacy);
                remove_routing_config(&paths, &store).unwrap();
                let restored = config(&paths);
                let original = original.parse::<DocumentMut>().unwrap();
                let expected = if accountless {
                    original.get("preferred_auth_method").and_then(Item::as_str)
                } else {
                    Some("apikey")
                };
                assert_eq!(
                    restored.get("preferred_auth_method").and_then(Item::as_str),
                    expected
                );
                assert_eq!(restored["model"].as_str(), Some("original-model"));
            }
        }
    }

    #[test]
    fn missing_or_ambiguous_legacy_backup_cannot_overwrite_config() {
        for ambiguous in [false, true] {
            let (_directory, paths, store) = fixture(ORIGINAL);
            let legacy_backup = apply_as_legacy_install(&paths, &store);
            if ambiguous {
                std::fs::write(
                    paths
                        .app_data_dir
                        .join("config-backups")
                        .join(format!("{}.toml", uuid::Uuid::new_v4())),
                    ORIGINAL.replace("original-model", "a-different-original"),
                )
                .unwrap();
            } else {
                std::fs::remove_file(legacy_backup).unwrap();
            }
            let installed = std::fs::read(&paths.config_path).unwrap();
            assert_eq!(
                remove_routing_config(&paths, &store).unwrap_err(),
                "newapi.errors.routerSnapshot"
            );
            assert_eq!(
                apply_routing_config(&paths, &store).unwrap_err(),
                "newapi.errors.routerSnapshot"
            );
            assert_eq!(std::fs::read(&paths.config_path).unwrap(), installed);
        }
    }
}
