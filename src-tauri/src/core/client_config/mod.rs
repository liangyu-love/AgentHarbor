//! Client-specific config adapters share one reversible file transaction boundary.
//! This module depends on provider data, never on Codex paths, accounts or processes.
mod claude;
mod opencode;
mod options;
use options::{client_endpoint, validate_model};
pub use options::{ClaudeAuthField, ClaudeModels, ClaudeOneM, ClientAccessMode, ClientOptions};
#[cfg(test)]
mod tests;

use super::{
    router::{ModelProtocol, Provider, Store},
    secret_file, storage,
    targets::TargetId,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

static WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static CLAUDE: claude::ClaudeCode = claude::ClaudeCode;
static OPENCODE: opencode::OpenCode = opencode::OpenCode;

#[derive(Clone)]
pub struct ClientPaths {
    pub claude: PathBuf,
    pub opencode: PathBuf,
}

impl ClientPaths {
    pub fn discover() -> Self {
        if let Some(home) = env_path("AGENT_HARBOR_CLIENT_HOME") {
            return Self::from_home(&home);
        }
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let config = env_path("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
        Self {
            claude: env_path("CLAUDE_CONFIG_DIR")
                .unwrap_or_else(|| home.join(".claude"))
                .join("settings.json"),
            opencode: env_path("OPENCODE_CONFIG")
                .unwrap_or_else(|| opencode_path(&config.join("opencode"))),
        }
    }

    pub fn from_home(home: &Path) -> Self {
        Self {
            claude: home.join(".claude/settings.json"),
            opencode: opencode_path(&home.join(".config/opencode")),
        }
    }
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

fn opencode_path(dir: &Path) -> PathBuf {
    ["opencode.jsonc", "opencode.json", "config.json"]
        .iter()
        .map(|name| dir.join(name))
        .find(|path| path.exists())
        .unwrap_or_else(|| dir.join("opencode.json"))
}

trait ClientAdapter: Sync {
    fn id(&self) -> TargetId;
    fn name(&self) -> &'static str;
    fn protocols(&self) -> &'static [ModelProtocol];
    fn path<'a>(&self, paths: &'a ClientPaths) -> &'a Path;
    fn edits(
        &self,
        document: &Value,
        store: &Store,
        request: &ConfigRequest,
    ) -> Result<Vec<Edit>, String>;
}

fn adapter(id: TargetId) -> Result<&'static dyn ClientAdapter, String> {
    match id {
        TargetId::ClaudeCode => Ok(&CLAUDE),
        TargetId::OpenCode => Ok(&OPENCODE),
        TargetId::Codex => Err("clients.errors.unsupported".into()),
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigRequest {
    pub target_id: TargetId,
    pub provider_id: String,
    pub model: String,
    pub context_window: u64,
    pub max_output_tokens: u64,
    #[serde(flatten)]
    pub options: ClientOptions,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientStatus {
    pub target_id: TargetId,
    pub name: String,
    pub path: String,
    pub protocols: Vec<ModelProtocol>,
    pub configured: bool,
    pub conflict: bool,
    pub error: Option<String>,
    pub provider_id: Option<String>,
    pub model: Option<String>,
    pub request: Option<ConfigRequest>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigPreview {
    pub path: String,
    pub revision: String,
    /// Only the managed fields are returned; all credential values are redacted.
    pub content: String,
    pub removed_fields: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigApplied {
    pub path: String,
    pub backup_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

struct Edit {
    path: Vec<String>,
    value: Option<Value>,
}

fn edit(path: &[&str], value: Value) -> Edit {
    Edit {
        path: path.iter().map(|s| (*s).to_owned()).collect(),
        value: Some(value),
    }
}

fn remove(path: &[&str]) -> Edit {
    Edit {
        path: path.iter().map(|key| (*key).to_owned()).collect(),
        value: None,
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct FieldChange {
    path: Vec<String>,
    before: Option<Value>,
    after: Option<Value>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Session {
    path: PathBuf,
    original: Option<String>,
    installed: Value,
    changes: Vec<FieldChange>,
    request: ConfigRequest,
}

pub struct ConfigManager {
    data_dir: PathBuf,
    paths: ClientPaths,
}

impl ConfigManager {
    pub fn new(data_dir: &Path, paths: ClientPaths) -> Self {
        Self {
            data_dir: data_dir.join("client-config"),
            paths,
        }
    }

    pub fn list(&self) -> Vec<ClientStatus> {
        [TargetId::ClaudeCode, TargetId::OpenCode]
            .into_iter()
            .map(|id| {
                let adapter = adapter(id).expect("registered adapter");
                let mut status = ClientStatus {
                    target_id: adapter.id(),
                    name: adapter.name().into(),
                    path: adapter.path(&self.paths).to_string_lossy().into(),
                    protocols: adapter.protocols().to_vec(),
                    configured: false,
                    conflict: false,
                    error: None,
                    provider_id: None,
                    model: None,
                    request: None,
                };
                let result = (|| -> Result<(), String> {
                    let session = self.session(id)?;
                    if let Some(session) = &session {
                        status.configured = true;
                        status.path = session.path.to_string_lossy().into();
                        status.provider_id = Some(session.request.provider_id.clone());
                        status.model = Some(session.request.model.clone());
                        status.request = Some(session.request.clone());
                        let current = parse(read_text(&session.path)?.as_deref(), id)?;
                        status.conflict = verify_fields(&current, session).is_err();
                    } else {
                        parse(read_text(adapter.path(&self.paths))?.as_deref(), id)?;
                    }
                    Ok(())
                })();
                status.error = result.err();
                status
            })
            .collect()
    }

    fn session_path(&self, id: TargetId) -> PathBuf {
        self.data_dir.join(match id {
            TargetId::ClaudeCode => "claude-code.dat",
            TargetId::OpenCode => "opencode.dat",
            TargetId::Codex => "codex.dat",
        })
    }

    fn session(&self, id: TargetId) -> Result<Option<Session>, String> {
        secret_file::read(&self.session_path(id))?
            .map(|bytes| {
                let mut value: Value = serde_json::from_slice(&bytes)
                    .map_err(|_| "clients.errors.storage".to_string())?;
                // Existing installations used the gateway before access mode was explicit.
                if let Some(request) = value.get_mut("request").and_then(Value::as_object_mut) {
                    request
                        .entry("accessMode")
                        .or_insert_with(|| json!("router"));
                }
                serde_json::from_value(value).map_err(|_| "clients.errors.storage".into())
            })
            .transpose()
            .map(Option::flatten)
    }

    pub fn preview(&self, store: &Store, request: &ConfigRequest) -> Result<ConfigPreview, String> {
        let plan = self.prepare(store, request)?;
        let mut patch = json!({});
        for change in &plan.session.changes {
            set_field(&mut patch, &change.path, change.after.clone())?;
        }
        redact(&mut patch);
        Ok(ConfigPreview {
            path: plan.session.path.to_string_lossy().into(),
            revision: plan.revision,
            content: serde_json::to_string_pretty(&patch).map_err(|_| "clients.errors.storage")?,
            removed_fields: plan
                .session
                .changes
                .iter()
                .filter(|change| change.after.is_none())
                .map(|change| change.path.join("."))
                .collect(),
        })
    }

    pub fn apply(
        &self,
        store: &Store,
        request: &ConfigRequest,
        expected_revision: &str,
    ) -> Result<ConfigApplied, String> {
        let _guard = WRITE_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .map_err(|_| "clients.errors.storage")?;
        let plan = self.prepare(store, request)?;
        if plan.revision != expected_revision {
            return Err("clients.errors.changed".into());
        }
        let text = format!(
            "{}\n",
            serde_json::to_string_pretty(&plan.session.installed)
                .map_err(|_| "clients.errors.storage")?
        );
        self.commit(
            request.target_id,
            &plan.session.path,
            plan.current,
            Some(text),
            Some(&plan.session),
        )
    }

    pub fn restore(&self, id: TargetId) -> Result<ConfigApplied, String> {
        let _guard = WRITE_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .map_err(|_| "clients.errors.storage")?;
        adapter(id)?;
        let session = self.session(id)?.ok_or("clients.errors.notConfigured")?;
        let current = read_text(&session.path)?;
        let document = parse(current.as_deref(), id)?;
        verify_fields(&document, &session)?;
        let restored = if document == session.installed {
            // If there were no independent edits, restore exact bytes (including comments).
            session.original.clone()
        } else {
            let mut document = document;
            undo(&mut document, &session)?;
            Some(format!(
                "{}\n",
                serde_json::to_string_pretty(&document).map_err(|_| "clients.errors.storage")?
            ))
        };
        self.commit(id, &session.path, current, restored, None)
    }

    /// Stop managing a client without touching its config file. This is the way out
    /// of a conflict (the user edited managed fields) or an unreadable session record:
    /// the file keeps whatever the user has now, and a later apply starts from it.
    pub fn release(&self, id: TargetId) -> Result<(), String> {
        let _guard = WRITE_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .map_err(|_| "clients.errors.storage")?;
        adapter(id)?;
        let journal = self.session_path(id);
        if journal.exists() {
            storage::remove_file_durable(&journal).map_err(|_| "clients.errors.storage")?;
        }
        Ok(())
    }

    /// Local-router URLs stop working with this process. Direct client
    /// configurations are persistent and remain in place across app restarts.
    /// Attempt each client independently so one conflict cannot strand another.
    pub fn restore_router_configs_on_exit(&self) -> Vec<(TargetId, String)> {
        [TargetId::ClaudeCode, TargetId::OpenCode]
            .into_iter()
            .filter_map(|id| match self.session(id) {
                Ok(Some(session)) if session.request.needs_router() => {
                    self.restore(id).err().map(|error| (id, error))
                }
                Ok(_) => None,
                Err(error) => Some((id, error)),
            })
            .collect()
    }

    fn prepare(&self, store: &Store, request: &ConfigRequest) -> Result<Plan, String> {
        let adapter = adapter(request.target_id)?;
        let provider = selected_provider(store, request)?;
        if !request.accepts_model(provider, &request.model) {
            return Err("clients.errors.protocol".into());
        }
        validate_model(&request.model)?;
        if request.context_window < 1024
            || request.max_output_tokens == 0
            || request.max_output_tokens > request.context_window
        {
            return Err("clients.errors.limits".into());
        }
        if request.needs_router() && store.router.bearer_token.is_empty() {
            return Err("clients.errors.router".into());
        }
        let previous = self.session(request.target_id)?;
        let path = previous
            .as_ref()
            .map(|s| s.path.as_path())
            .unwrap_or_else(|| adapter.path(&self.paths))
            .to_path_buf();
        let current = read_text(&path)?;
        let mut document = parse(current.as_deref(), request.target_id)?;
        let independently_changed = previous.as_ref().is_some_and(|p| document != p.installed);
        if let Some(previous) = &previous {
            verify_fields(&document, previous)?;
            undo(&mut document, previous)?;
        }
        let original = if independently_changed {
            Some(format!(
                "{}\n",
                serde_json::to_string_pretty(&document).map_err(|_| "clients.errors.storage")?
            ))
        } else {
            previous
                .map(|s| s.original)
                .unwrap_or_else(|| current.clone())
        };
        let edits = adapter.edits(&document, store, request)?;
        let mut changes = Vec::new();
        for edit in edits {
            let before = field(&document, &edit.path).cloned();
            set_field(&mut document, &edit.path, edit.value.clone())?;
            changes.push(FieldChange {
                path: edit.path,
                before,
                after: edit.value,
            });
        }
        let session = Session {
            path,
            original,
            installed: document,
            changes,
            request: request.clone(),
        };
        let mut hasher = Sha256::new();
        hasher.update(
            serde_json::to_vec(&(&current, &session)).map_err(|_| "clients.errors.storage")?,
        );
        Ok(Plan {
            revision: format!("{:x}", hasher.finalize()),
            current,
            session,
        })
    }

    fn commit(
        &self,
        id: TargetId,
        path: &Path,
        current: Option<String>,
        next: Option<String>,
        session: Option<&Session>,
    ) -> Result<ConfigApplied, String> {
        if read_text(path)? != current {
            return Err("clients.errors.changed".into());
        }
        let backup_path = self
            .data_dir
            .join("backups")
            .join(format!("{}.dat", uuid::Uuid::new_v4()));
        secret_file::write(
            &backup_path,
            &serde_json::to_vec(&current).map_err(|_| "clients.errors.storage")?,
        )?;
        storage::prune_backups(&self.data_dir.join("backups"), 20, |path| {
            path.extension().is_some_and(|extension| extension == "dat")
        });
        let previous_session = self.session(id)?;
        let journal = self.session_path(id);
        secret_file::write(
            &journal,
            &serde_json::to_vec(&session).map_err(|_| "clients.errors.storage")?,
        )?;
        let write_result = (|| -> Result<(), String> {
            if read_text(path)? != current {
                return Err("clients.errors.changed".into());
            }
            if let Some(text) = next {
                storage::write_atomic(path, text.as_bytes())
                    .map_err(|_| "clients.errors.storage")?;
            } else if path.exists() {
                std::fs::remove_file(path).map_err(|_| "clients.errors.storage")?;
            }
            Ok(())
        })();
        if let Err(error) = write_result {
            secret_file::write(
                &journal,
                &serde_json::to_vec(&previous_session).map_err(|_| "clients.errors.storage")?,
            )?;
            return Err(error);
        }
        Ok(ConfigApplied {
            path: path.to_string_lossy().into(),
            backup_path: backup_path.to_string_lossy().into(),
            warning: None,
        })
    }
}

struct Plan {
    revision: String,
    current: Option<String>,
    session: Session,
}

pub fn has_router_configs(data_dir: &Path) -> Result<bool, String> {
    let manager = ConfigManager::new(data_dir, ClientPaths::from_home(Path::new(".")));
    for id in [TargetId::ClaudeCode, TargetId::OpenCode] {
        match manager.session(id) {
            Ok(Some(session)) if session.request.needs_router() => return Ok(true),
            Ok(_) => {}
            // One unreadable record must not keep Codex routing down; the client list
            // reports it and offers release.
            Err(error) => log::warn!("skipping unreadable {id:?} client session: {error}"),
        }
    }
    Ok(false)
}

fn selected_provider<'a>(
    store: &'a Store,
    request: &ConfigRequest,
) -> Result<&'a Provider, String> {
    store
        .providers
        .iter()
        .find(|p| p.id == request.provider_id && p.has_route_credentials())
        .ok_or_else(|| "clients.errors.provider".into())
}

fn read_text(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("clients.errors.read".into()),
    }
}

fn parse(text: Option<&str>, id: TargetId) -> Result<Value, String> {
    let Some(text) = text else {
        return Ok(json!({}));
    };
    let text = text.trim_start_matches('\u{feff}');
    let value: Value = if id == TargetId::OpenCode {
        json5::from_str(text).map_err(|_| "clients.errors.invalidConfig")?
    } else {
        serde_json::from_str(text).map_err(|_| "clients.errors.invalidConfig")?
    };
    if !value.is_object() {
        return Err("clients.errors.invalidConfig".into());
    }
    Ok(value)
}

fn field<'a>(document: &'a Value, path: &[String]) -> Option<&'a Value> {
    path.iter().try_fold(document, |value, key| value.get(key))
}

fn set_field(document: &mut Value, path: &[String], value: Option<Value>) -> Result<(), String> {
    let Some((key, rest)) = path.split_first() else {
        return Err("clients.errors.invalidConfig".into());
    };
    let object = document
        .as_object_mut()
        .ok_or("clients.errors.invalidConfig")?;
    if rest.is_empty() {
        if let Some(value) = value {
            object.insert(key.clone(), value);
        } else {
            object.remove(key);
        }
    } else if value.is_some() || object.contains_key(key) {
        let child = object.entry(key.clone()).or_insert_with(|| json!({}));
        set_field(child, rest, value)?;
    }
    Ok(())
}

fn verify_fields(document: &Value, session: &Session) -> Result<(), String> {
    if session
        .changes
        .iter()
        .any(|c| field(document, &c.path) != c.after.as_ref())
    {
        return Err("clients.errors.conflict".into());
    }
    Ok(())
}

fn undo(document: &mut Value, session: &Session) -> Result<(), String> {
    let original = parse(session.original.as_deref(), session.request.target_id)?;
    for change in session.changes.iter().rev() {
        set_field(document, &change.path, change.before.clone())?;
        // Remove containers introduced by our leaf edits only when still empty.
        // Existing empty objects and siblings added by the user must survive.
        for depth in (1..change.path.len()).rev() {
            let parent = &change.path[..depth];
            if field(&original, parent).is_none()
                && field(document, parent)
                    .and_then(Value::as_object)
                    .is_some_and(|object| object.is_empty())
            {
                set_field(document, parent, None)?;
            }
        }
    }
    Ok(())
}

fn redact(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                let normalized: String = key
                    .chars()
                    .filter(char::is_ascii_alphanumeric)
                    .map(|c| c.to_ascii_lowercase())
                    .collect();
                if normalized.ends_with("apikey")
                    || normalized.ends_with("token")
                    || normalized.ends_with("authorization")
                    || normalized.ends_with("cookie")
                    || normalized.contains("secret")
                    || normalized.contains("password")
                    || normalized.contains("credential")
                {
                    *value = json!("[managed locally]");
                } else {
                    redact(value);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(redact),
        _ => {}
    }
}
