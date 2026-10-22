//! Codex on a remote host, configured direct: a Codex config file points at
//! the relay's Responses root, the key sits in `~/.codex/.env` (which Codex
//! loads) and the relay's models are described in a catalog beside them.
//! As the default the relay is written into `config.toml`; as a profile it
//! gets a file of its own, `<name>.config.toml`, which Codex layers over
//! `config.toml` for `codex -p <name>`, so plain `codex` and its official
//! sign-in stay as they were. Only the fields named in [`MANAGED`] are
//! touched; what they held before is journaled so a restore puts the file
//! back as it was.

use crate::core::{
    client_config::ClientFiles,
    models_dev,
    router::{self, Store},
    secret_file,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item};

pub const JOURNAL_FILE: &str = "codex.dat";
const PROVIDER_KEY: &str = "agentharbor_remote";
const ENV_NAME: &str = "AGENTHARBOR_REMOTE_CODEX_KEY";
const BEGIN: &str = "# --- AgentHarbor remote Codex key (managed, do not edit) ---";
const END: &str = "# --- End AgentHarbor remote Codex key ---";
const CATALOG_FILE: &str = "agentharbor-models.json";
const CONFIG_FILE: &str = "config.toml";

/// The configuration fields this app owns in the file it writes.
const MANAGED: [&[&str]; 7] = [
    &["model"],
    &["model_provider"],
    &["model_catalog_json"],
    &["model_context_window"],
    &["model_auto_compact_token_limit"],
    &["model_providers", PROVIDER_KEY],
    &["web_search"],
];

/// A Codex profile name: what `codex -p` takes and a file name can carry.
fn valid_profile(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexRemoteRequest {
    pub provider_id: String,
    pub model: String,
    /// Whether Codex offers its `web_search` tool. A direct connection has no
    /// router to drop the tool for a relay that refuses it, so it is off
    /// (`web_search = "disabled"`) unless the user knows the relay takes it.
    #[serde(default)]
    pub web_search: bool,
    /// A profile name writes the relay into `<name>.config.toml` (run with
    /// `codex -p <name>`) and leaves `config.toml` as it was; none makes the
    /// relay the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
}

impl CodexRemoteRequest {
    /// The file under `~/.codex` the relay is written to.
    fn file(&self) -> String {
        match &self.profile {
            Some(name) => format!("{name}.config.toml"),
            None => CONFIG_FILE.into(),
        }
    }
}

fn config_file() -> String {
    CONFIG_FILE.into()
}

#[derive(Clone, Serialize, Deserialize)]
struct Journal {
    request: CodexRemoteRequest,
    /// The file under `~/.codex` that was written.
    #[serde(default = "config_file")]
    file: String,
    /// The file before the first apply; `None` when it did not exist.
    original: Option<String>,
    /// The file as last written.
    installed: String,
    /// Each managed field before the first apply, by dotted path.
    before: BTreeMap<String, Option<Value>>,
    /// Each managed field as last written.
    after: BTreeMap<String, Option<Value>>,
    /// The catalog's models: picking one of them in Codex is using it.
    #[serde(default)]
    models: Vec<String>,
    /// The relay's address, key and catalog as written.
    connection: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexRemoteStatus {
    pub path: String,
    pub configured: bool,
    pub request: Option<CodexRemoteRequest>,
    pub conflict: bool,
    pub conflict_fields: Vec<String>,
    /// The relay card changed (or is gone) since the configuration was written.
    pub stale: bool,
    pub error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexRemotePreview {
    pub path: String,
    pub revision: String,
    /// The managed fields as they will be written, and the key's line redacted.
    pub content: String,
    pub conflict_fields: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexRemoteApplied {
    pub path: String,
}

/// Codex's files on one remote host, and where its journal is kept here.
pub struct CodexRemote<'a> {
    files: &'a dyn ClientFiles,
    home: String,
    journal: PathBuf,
    app_data_dir: PathBuf,
    official: Vec<Value>,
}

/// A file put back the way a journal says, as read and as it will be.
struct Undo {
    path: PathBuf,
    current: Option<String>,
    next: Option<String>,
}

struct Plan {
    path: PathBuf,
    current: Option<String>,
    next: String,
    env_current: Option<String>,
    env_next: String,
    catalog: Vec<u8>,
    journal: Journal,
    conflicts: Vec<String>,
    /// The file a previous apply wrote in the other mode, put back first.
    switched: Option<Undo>,
    revision: String,
}

fn dotted(path: &[&str]) -> String {
    path.join(".")
}

/// A field of the document as plain data, for comparing and journaling.
fn field(document: &DocumentMut, path: &[&str]) -> Option<Value> {
    let value: toml::Value = toml::from_str(&document.to_string()).ok()?;
    let mut node = &value;
    for key in path {
        node = node.get(key)?;
    }
    serde_json::to_value(node).ok()
}

fn fields(document: &DocumentMut) -> BTreeMap<String, Option<Value>> {
    MANAGED
        .iter()
        .map(|path| (dotted(path), field(document, path)))
        .collect()
}

fn item(value: &Value) -> Result<Item, String> {
    Ok(match value {
        Value::Object(entries) => {
            let mut table = toml_edit::Table::new();
            for (key, value) in entries {
                table.insert(key, item(value)?);
            }
            Item::Table(table)
        }
        other => Item::Value(toml_value(other)?),
    })
}

fn toml_value(value: &Value) -> Result<toml_edit::Value, String> {
    Ok(match value {
        Value::String(text) => text.as_str().into(),
        Value::Bool(flag) => (*flag).into(),
        Value::Number(number) => match number.as_i64() {
            Some(integer) => integer.into(),
            None => number.as_f64().ok_or("remote.errors.codexConfig")?.into(),
        },
        Value::Array(items) => {
            let mut array = toml_edit::Array::new();
            for value in items {
                array.push(toml_value(value)?);
            }
            array.into()
        }
        Value::Object(entries) => {
            let mut table = toml_edit::InlineTable::new();
            for (key, value) in entries {
                table.insert(key, toml_value(value)?);
            }
            table.into()
        }
        Value::Null => return Err("remote.errors.codexConfig".into()),
    })
}

fn set(document: &mut DocumentMut, path: &[&str], value: Option<&Value>) -> Result<(), String> {
    match path {
        [key] => {
            match value {
                Some(value) => document[*key] = item(value)?,
                None => {
                    document.remove(key);
                }
            }
            Ok(())
        }
        [table, key] => {
            if document.get(table).is_none() {
                if value.is_none() {
                    return Ok(());
                }
                let mut created = toml_edit::Table::new();
                created.set_implicit(true);
                document[*table] = Item::Table(created);
            }
            let created = match &mut document[*table] {
                Item::Table(parent) => {
                    match value {
                        Some(value) => {
                            parent.insert(key, item(value)?);
                        }
                        None => {
                            parent.remove(key);
                        }
                    }
                    parent.is_implicit() && parent.is_empty()
                }
                Item::Value(toml_edit::Value::InlineTable(parent)) => {
                    match value {
                        Some(value) => {
                            parent.insert(*key, toml_value(value)?);
                        }
                        None => {
                            parent.remove(key);
                        }
                    }
                    false
                }
                _ => return Err("remote.errors.codexConfig".into()),
            };
            // A table that only held this entry goes with it.
            if created {
                document.remove(table);
            }
            Ok(())
        }
        _ => Err("remote.errors.codexConfig".into()),
    }
}

fn parse(text: Option<&str>) -> Result<DocumentMut, String> {
    text.unwrap_or_default()
        .trim_start_matches('\u{feff}')
        .parse()
        .map_err(|_| "remote.errors.codexConfig".into())
}

/// A dotenv value Codex reads back exactly.
fn quote(key: &str) -> String {
    let mut value = String::from("\"");
    for character in key.chars() {
        if matches!(character, '\\' | '"' | '$') {
            value.push('\\');
        }
        value.push(character);
    }
    value.push('"');
    value
}

/// `.env` without this app's block.
fn strip_env(text: &str) -> Result<String, String> {
    let Some(start) = text.find(BEGIN) else {
        if text.contains(ENV_NAME) {
            return Err("remote.errors.codexEnv".into());
        }
        return Ok(text.to_owned());
    };
    let end = text[start..]
        .find(END)
        .map(|at| start + at + END.len())
        .ok_or("remote.errors.codexEnv")?;
    let end = if text[end..].starts_with("\r\n") {
        end + 2
    } else if text[end..].starts_with('\n') {
        end + 1
    } else {
        end
    };
    let stripped = format!("{}{}", &text[..start], &text[end..]);
    if stripped.contains(ENV_NAME) {
        return Err("remote.errors.codexEnv".into());
    }
    Ok(stripped)
}

fn with_env(text: &str, key: &str) -> Result<String, String> {
    if key.chars().any(|c| c.is_control()) {
        return Err("newapi.errors.token".into());
    }
    let mut next = strip_env(text)?;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(&format!("{BEGIN}\n{ENV_NAME}={}\n{END}\n", quote(key)));
    // Read it back as Codex will: exactly one value, the key.
    let values: Vec<(String, String)> = dotenvy::from_read_iter(next.as_bytes())
        .collect::<Result<_, _>>()
        .map_err(|_| "remote.errors.codexEnv")?;
    let ours: Vec<_> = values.iter().filter(|(name, _)| name == ENV_NAME).collect();
    if ours.len() != 1 || ours[0].1 != key {
        return Err("remote.errors.codexEnv".into());
    }
    Ok(next)
}

fn fingerprint(endpoint: &str, key: &str, catalog: &[u8]) -> String {
    let mut hasher = Sha256::new();
    for part in [endpoint.as_bytes(), key.as_bytes(), catalog] {
        hasher.update(part);
        hasher.update([0]);
    }
    format!("{:x}", hasher.finalize())
}

impl<'a> CodexRemote<'a> {
    pub fn new(
        files: &'a dyn ClientFiles,
        home: &str,
        journal_dir: &Path,
        app_data_dir: &Path,
        official: Vec<Value>,
    ) -> Self {
        Self {
            files,
            home: home.trim_end_matches('/').to_owned(),
            journal: journal_dir.join(JOURNAL_FILE),
            app_data_dir: app_data_dir.to_path_buf(),
            official,
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        PathBuf::from(self.display(name))
    }

    fn display(&self, name: &str) -> String {
        format!("{}/.codex/{name}", self.home)
    }

    fn load_journal(&self) -> Result<Option<Journal>, String> {
        secret_file::read(&self.journal)?
            .map(|bytes| {
                serde_json::from_slice(&bytes).map_err(|_| "remote.errors.storage".to_owned())
            })
            .transpose()
    }

    fn save_journal(&self, journal: Option<&Journal>) -> Result<(), String> {
        match journal {
            Some(journal) => secret_file::write(
                &self.journal,
                &serde_json::to_vec(journal).map_err(|_| "remote.errors.storage")?,
            ),
            None => super::remove_journal(&self.journal),
        }
    }

    /// Managed fields changed on the host since they were written, other
    /// than back to what they were. Picking another of the catalog's models
    /// is using Codex, not a conflict.
    fn conflicts(document: &DocumentMut, journal: &Journal) -> Vec<String> {
        MANAGED
            .iter()
            .filter(|path| {
                let key = dotted(path);
                let current = field(document, path);
                if key == "model"
                    && current
                        .as_ref()
                        .and_then(Value::as_str)
                        .is_some_and(|model| journal.models.iter().any(|known| known == model))
                {
                    return false;
                }
                current != journal.after.get(&key).cloned().flatten()
                    && current != journal.before.get(&key).cloned().flatten()
            })
            .map(|path| dotted(path))
            .collect()
    }

    /// What `journal`'s file goes back to: its exact original when nothing
    /// else changed it, else its managed fields put back. A profile file
    /// left with nothing else in it goes.
    fn undo(&self, journal: &Journal, replace_conflicts: bool) -> Result<Undo, String> {
        let path = self.path(&journal.file);
        let current = self.files.read(&path)?;
        let next = if current.as_deref() == Some(journal.installed.as_str()) {
            journal.original.clone()
        } else {
            let mut document = parse(current.as_deref())?;
            if !Self::conflicts(&document, journal).is_empty() && !replace_conflicts {
                return Err("clients.errors.conflict".into());
            }
            for path in MANAGED {
                set(
                    &mut document,
                    path,
                    journal
                        .before
                        .get(&dotted(path))
                        .cloned()
                        .flatten()
                        .as_ref(),
                )?;
            }
            let text = document.to_string();
            (journal.file == CONFIG_FILE || journal.original.is_some() || !text.trim().is_empty())
                .then_some(text)
        };
        Ok(Undo {
            path,
            current,
            next,
        })
    }

    fn write_undo(&self, undo: &Undo) -> Result<(), String> {
        if self.files.read(&undo.path)? != undo.current {
            return Err("clients.errors.changed".into());
        }
        match &undo.next {
            Some(text) => self.files.write(&undo.path, text.as_bytes()),
            None => self.files.remove(&undo.path),
        }
    }

    fn plan(
        &self,
        store: &Store,
        request: &CodexRemoteRequest,
        replace_conflicts: bool,
    ) -> Result<Plan, String> {
        let provider = store
            .providers
            .iter()
            .find(|provider| provider.id == request.provider_id && provider.has_route_credentials())
            .ok_or("clients.errors.provider")?;
        let catalog = models_dev::load(&self.app_data_dir);
        let remote = router::remote_plan(store, provider, &catalog, &self.official)?;
        if !remote.models.contains(&request.model) {
            return Err("clients.errors.model".into());
        }
        if request
            .profile
            .as_deref()
            .is_some_and(|name| !valid_profile(name))
        {
            return Err("remote.errors.codexProfile".into());
        }
        let file = request.file();
        // Moving to the other file puts the previous one back first; the
        // new file then starts from how it is now.
        let (switched, previous) = match self.load_journal()? {
            Some(journal) if journal.file != file => {
                (Some(self.undo(&journal, replace_conflicts)?), None)
            }
            previous => (None, previous),
        };
        let path = self.path(&file);
        let current = self.files.read(&path)?;
        let mut document = parse(current.as_deref())?;
        if document
            .get("model_providers")
            .is_some_and(|item| !item.is_table_like())
        {
            return Err("remote.errors.codexConfig".into());
        }
        let conflicts = previous
            .as_ref()
            .map(|journal| Self::conflicts(&document, journal))
            .unwrap_or_default();
        let (original, before) = match &previous {
            Some(journal) => {
                for path in MANAGED {
                    set(
                        &mut document,
                        path,
                        journal
                            .before
                            .get(&dotted(path))
                            .cloned()
                            .flatten()
                            .as_ref(),
                    )?;
                }
                (journal.original.clone(), journal.before.clone())
            }
            None => (current.clone(), fields(&document)),
        };
        let catalog_path = self.display(CATALOG_FILE);
        let provider_entry = serde_json::json!({
            "name": provider.name,
            "base_url": remote.endpoint,
            "wire_api": "responses",
            "env_key": ENV_NAME,
            // Never let a third-party endpoint pick up official sign-in tokens.
            "requires_openai_auth": false,
        });
        let writes: [(&[&str], Option<Value>); 7] = [
            (&["model"], Some(request.model.clone().into())),
            (&["model_provider"], Some(PROVIDER_KEY.into())),
            (&["model_catalog_json"], Some(catalog_path.into())),
            (&["model_context_window"], remote.context.map(Value::from)),
            (
                &["model_auto_compact_token_limit"],
                remote.threshold.map(Value::from),
            ),
            (&["model_providers", PROVIDER_KEY], Some(provider_entry)),
            (
                &["web_search"],
                (!request.web_search).then(|| "disabled".into()),
            ),
        ];
        for (path, value) in &writes {
            set(&mut document, path, value.as_ref())?;
        }
        let next = document.to_string();
        parse(Some(&next))?;
        let env_current = self.files.read(&self.path(".env"))?;
        let env_next = with_env(env_current.as_deref().unwrap_or_default(), &remote.key)?;
        let catalog_bytes =
            serde_json::to_vec_pretty(&remote.catalog).map_err(|_| "remote.errors.storage")?;
        let journal = Journal {
            request: request.clone(),
            file,
            original,
            installed: next.clone(),
            before,
            after: fields(&document),
            models: remote.models.clone(),
            connection: fingerprint(&remote.endpoint, &remote.key, &catalog_bytes),
        };
        let mut hasher = Sha256::new();
        hasher.update(
            serde_json::to_vec(&(
                &current,
                &next,
                &env_current,
                &env_next,
                &catalog_bytes,
                switched.as_ref().map(|undo| (&undo.current, &undo.next)),
                replace_conflicts,
            ))
            .map_err(|_| "remote.errors.storage")?,
        );
        Ok(Plan {
            path,
            current,
            next,
            env_current,
            env_next,
            catalog: catalog_bytes,
            journal,
            conflicts,
            switched,
            revision: format!("{:x}", hasher.finalize()),
        })
    }

    pub fn status(&self, store: Option<&Store>) -> CodexRemoteStatus {
        let mut status = CodexRemoteStatus {
            path: self.display(CONFIG_FILE),
            configured: false,
            request: None,
            conflict: false,
            conflict_fields: Vec::new(),
            stale: false,
            error: None,
        };
        let result = (|| -> Result<(), String> {
            let journal = self.load_journal()?;
            let file = journal
                .as_ref()
                .map_or(CONFIG_FILE, |journal| journal.file.as_str());
            status.path = self.display(file);
            let document = parse(self.files.read(&self.path(file))?.as_deref())?;
            let Some(journal) = journal else {
                return Ok(());
            };
            status.configured = true;
            status.conflict_fields = Self::conflicts(&document, &journal);
            status.conflict = !status.conflict_fields.is_empty();
            if let Some(store) = store {
                let written = store
                    .providers
                    .iter()
                    .find(|provider| provider.id == journal.request.provider_id)
                    .and_then(|provider| {
                        let remote = router::remote_plan(
                            store,
                            provider,
                            &models_dev::load(&self.app_data_dir),
                            &self.official,
                        )
                        .ok()?;
                        let catalog = serde_json::to_vec_pretty(&remote.catalog).ok()?;
                        Some(fingerprint(&remote.endpoint, &remote.key, &catalog))
                    });
                status.stale = written.as_deref() != Some(journal.connection.as_str());
            }
            status.request = Some(journal.request);
            Ok(())
        })();
        status.error = result.err();
        status
    }

    pub fn preview(
        &self,
        store: &Store,
        request: &CodexRemoteRequest,
        replace_conflicts: bool,
    ) -> Result<CodexRemotePreview, String> {
        let plan = self.plan(store, request, replace_conflicts)?;
        let mut shown = DocumentMut::new();
        let document = parse(Some(&plan.next))?;
        for path in MANAGED {
            set(&mut shown, path, field(&document, path).as_ref())?;
        }
        let path = self.display(&plan.journal.file);
        let mut content = format!(
            "# {path}\n{}\n# {}\n{ENV_NAME}=\"[managed locally]\"\n",
            shown.to_string().trim_end(),
            self.display(".env"),
        );
        if let Some(undo) = &plan.switched {
            content.push_str(&format!(
                "# {}: {}\n",
                undo.path.to_string_lossy(),
                if undo.next.is_some() {
                    "put back as it was"
                } else {
                    "removed"
                }
            ));
        }
        Ok(CodexRemotePreview {
            path,
            revision: plan.revision,
            content,
            conflict_fields: plan.conflicts,
        })
    }

    pub fn apply(
        &self,
        store: &Store,
        request: &CodexRemoteRequest,
        revision: &str,
        replace_conflicts: bool,
    ) -> Result<CodexRemoteApplied, String> {
        let plan = self.plan(store, request, replace_conflicts)?;
        if plan.revision != revision {
            return Err("clients.errors.changed".into());
        }
        if !plan.conflicts.is_empty() && !replace_conflicts {
            return Err("clients.errors.conflict".into());
        }
        let previous = self.load_journal()?;
        // The previous file goes back first: if that fails, nothing of the
        // new configuration is written and the old one is still journaled.
        if let Some(undo) = &plan.switched {
            self.write_undo(undo)?;
        }
        self.save_journal(Some(&plan.journal))?;
        let written = (|| -> Result<(), String> {
            if self.files.read(&self.path(".env"))? != plan.env_current {
                return Err("clients.errors.changed".into());
            }
            self.files
                .write(&self.path(".env"), plan.env_next.as_bytes())?;
            self.files.write(&self.path(CATALOG_FILE), &plan.catalog)?;
            if self.files.read(&plan.path)? != plan.current {
                return Err("clients.errors.changed".into());
            }
            self.files.write(&plan.path, plan.next.as_bytes())
        })();
        if let Err(error) = written {
            // A switch already put the old file back, so its journal no
            // longer applies: take the key and catalog away too.
            if plan.switched.is_some() {
                let _ = self.clear_key_and_catalog();
                self.save_journal(None)?;
            } else {
                self.save_journal(previous.as_ref())?;
            }
            return Err(error);
        }
        Ok(CodexRemoteApplied {
            path: self.display(&plan.journal.file),
        })
    }

    fn clear_key_and_catalog(&self) -> Result<(), String> {
        if let Some(env) = self.files.read(&self.path(".env"))? {
            let stripped = strip_env(&env)?;
            if stripped.trim().is_empty() {
                self.files.remove(&self.path(".env"))?;
            } else if stripped != env {
                self.files.write(&self.path(".env"), stripped.as_bytes())?;
            }
        }
        self.files.remove(&self.path(CATALOG_FILE))
    }

    /// Puts the managed fields back, removes the key and the catalog, and
    /// forgets the configuration. Fields changed on the host since refuse
    /// the restore unless `replace_conflicts`.
    pub fn restore(&self, replace_conflicts: bool) -> Result<CodexRemoteApplied, String> {
        let journal = self.load_journal()?.ok_or("clients.errors.notConfigured")?;
        let undo = self.undo(&journal, replace_conflicts)?;
        self.write_undo(&undo)?;
        self.clear_key_and_catalog()?;
        self.save_journal(None)?;
        Ok(CodexRemoteApplied {
            path: self.display(&journal.file),
        })
    }

    /// Stops managing Codex on the host and leaves its files as they are.
    pub fn release(&self) -> Result<(), String> {
        self.save_journal(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        newapi,
        router::{ModelProtocol, Provider},
    };
    use std::sync::Mutex;

    /// Files kept in memory, standing in for a remote host.
    #[derive(Default)]
    struct Memory(Mutex<BTreeMap<String, String>>);

    impl ClientFiles for Memory {
        fn read(&self, path: &Path) -> Result<Option<String>, String> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .get(&path.to_string_lossy().replace('\\', "/"))
                .cloned())
        }
        fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), String> {
            self.0.lock().unwrap().insert(
                path.to_string_lossy().replace('\\', "/"),
                String::from_utf8(bytes.to_vec()).unwrap(),
            );
            Ok(())
        }
        fn remove(&self, path: &Path) -> Result<(), String> {
            self.0
                .lock()
                .unwrap()
                .remove(&path.to_string_lossy().replace('\\', "/"));
            Ok(())
        }
    }

    const ORIGINAL: &str = "# my settings\nmodel = \"gpt-5\"\napproval_policy = \"never\"\n\n[model_providers.mine]\nname = \"Mine\"\nbase_url = \"https://example.com/v1\"\n";

    fn store() -> Store {
        let model = |id: &str, protocol| newapi::Model {
            id: id.into(),
            protocol: Some(protocol),
            ..Default::default()
        };
        Store {
            providers: vec![Provider {
                id: "p1".into(),
                name: "Relay".into(),
                base_url: "https://relay.example.com".into(),
                api_key: "sk-test-\"$key".into(),
                enabled: true,
                models: vec![
                    model("gpt-5.5", ModelProtocol::Responses),
                    model("gpt-5.5-mini", ModelProtocol::Responses),
                    model("chat-only", ModelProtocol::ChatCompletions),
                ],
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn setup() -> (Memory, PathBuf, PathBuf) {
        let files = Memory::default();
        files
            .write(Path::new("/home/u/.codex/config.toml"), ORIGINAL.as_bytes())
            .unwrap();
        files
            .write(Path::new("/home/u/.codex/.env"), b"OTHER=1\n")
            .unwrap();
        let journal =
            std::env::temp_dir().join(format!("agentharbor-remote-codex-{}", uuid::Uuid::new_v4()));
        let data = journal.join("data");
        (files, journal, data)
    }

    fn request() -> CodexRemoteRequest {
        CodexRemoteRequest {
            provider_id: "p1".into(),
            model: "gpt-5.5".into(),
            web_search: false,
            profile: None,
        }
    }

    #[test]
    fn apply_and_restore_round_trip() {
        let (files, journal, data) = setup();
        let codex = CodexRemote::new(&files, "/home/u", &journal, &data, Vec::new());
        let store = store();
        let preview = codex.preview(&store, &request(), false).unwrap();
        assert!(!preview.content.contains("sk-test"));
        codex
            .apply(&store, &request(), &preview.revision, false)
            .unwrap();
        let config = files
            .read(Path::new("/home/u/.codex/config.toml"))
            .unwrap()
            .unwrap();
        assert!(config.contains("# my settings") && config.contains("approval_policy = \"never\""));
        assert!(config.contains("model_provider = \"agentharbor_remote\""));
        assert!(config.contains("base_url = \"https://example.com/v1\""));
        let document = parse(Some(&config)).unwrap();
        assert_eq!(
            field(&document, &["model_providers", PROVIDER_KEY, "base_url"]),
            Some("https://relay.example.com/v1".into())
        );
        assert_eq!(field(&document, &["model"]), Some("gpt-5.5".into()));
        assert_eq!(
            field(&document, &["web_search"]),
            Some("disabled".into()),
            "the tool a direct relay may refuse is off"
        );
        let env = files
            .read(Path::new("/home/u/.codex/.env"))
            .unwrap()
            .unwrap();
        assert!(env.starts_with("OTHER=1\n"));
        let values: Vec<(String, String)> = dotenvy::from_read_iter(env.as_bytes())
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(values.contains(&(ENV_NAME.into(), "sk-test-\"$key".into())));
        assert!(files
            .read(Path::new("/home/u/.codex/agentharbor-models.json"))
            .unwrap()
            .is_some());
        let status = codex.status(Some(&store));
        assert!(status.configured && !status.conflict && !status.stale);

        codex.restore(false).unwrap();
        assert_eq!(
            files
                .read(Path::new("/home/u/.codex/config.toml"))
                .unwrap()
                .as_deref(),
            Some(ORIGINAL)
        );
        assert_eq!(
            files
                .read(Path::new("/home/u/.codex/.env"))
                .unwrap()
                .as_deref(),
            Some("OTHER=1\n")
        );
        assert!(files
            .read(Path::new("/home/u/.codex/agentharbor-models.json"))
            .unwrap()
            .is_none());
        assert!(!codex.status(None).configured);
        let _ = std::fs::remove_dir_all(journal);
    }

    #[test]
    fn user_edits_are_kept_and_conflicts_refuse() {
        let (files, journal, data) = setup();
        let codex = CodexRemote::new(&files, "/home/u", &journal, &data, Vec::new());
        let store = store();
        let preview = codex.preview(&store, &request(), false).unwrap();
        codex
            .apply(&store, &request(), &preview.revision, false)
            .unwrap();
        // The user adds a setting of their own and points Codex elsewhere.
        let config = files
            .read(Path::new("/home/u/.codex/config.toml"))
            .unwrap()
            .unwrap();
        let edited = "hide_agent_reasoning = true\n".to_owned()
            + &config.replace(
                "model_provider = \"agentharbor_remote\"",
                "model_provider = \"mine\"",
            );
        files
            .write(Path::new("/home/u/.codex/config.toml"), edited.as_bytes())
            .unwrap();
        let status = codex.status(Some(&store));
        assert_eq!(status.conflict_fields, vec!["model_provider".to_string()]);
        assert_eq!(
            codex.restore(false).err().as_deref(),
            Some("clients.errors.conflict")
        );
        codex.restore(true).unwrap();
        let restored = files
            .read(Path::new("/home/u/.codex/config.toml"))
            .unwrap()
            .unwrap();
        assert!(restored.contains("hide_agent_reasoning = true"));
        assert!(!restored.contains(PROVIDER_KEY));
        let document = parse(Some(&restored)).unwrap();
        assert_eq!(field(&document, &["model"]), Some("gpt-5".into()));
        assert_eq!(field(&document, &["model_provider"]), None);
        let _ = std::fs::remove_dir_all(journal);
    }

    #[test]
    fn a_missing_config_is_created_and_removed() {
        let files = Memory::default();
        let journal =
            std::env::temp_dir().join(format!("agentharbor-remote-codex-{}", uuid::Uuid::new_v4()));
        let codex = CodexRemote::new(&files, "/root", &journal, &journal, Vec::new());
        let store = store();
        let preview = codex.preview(&store, &request(), false).unwrap();
        codex
            .apply(&store, &request(), &preview.revision, false)
            .unwrap();
        assert!(files
            .read(Path::new("/root/.codex/config.toml"))
            .unwrap()
            .is_some());
        codex.restore(false).unwrap();
        assert!(files.0.lock().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(journal);
    }

    #[test]
    fn a_profile_is_a_file_of_its_own_and_switching_modes_restores_exactly() {
        let (files, journal, data) = setup();
        let codex = CodexRemote::new(&files, "/home/u", &journal, &data, Vec::new());
        let store = store();
        let read = |name: &str| {
            files
                .read(Path::new(&format!("/home/u/.codex/{name}")))
                .unwrap()
        };
        let profiled = CodexRemoteRequest {
            profile: Some("relay".into()),
            web_search: true,
            ..request()
        };
        let preview = codex.preview(&store, &profiled, false).unwrap();
        assert_eq!(preview.path, "/home/u/.codex/relay.config.toml");
        codex
            .apply(&store, &profiled, &preview.revision, false)
            .unwrap();
        assert_eq!(
            read("config.toml").as_deref(),
            Some(ORIGINAL),
            "Codex's own config is untouched"
        );
        let profile = parse(read("relay.config.toml").as_deref()).unwrap();
        assert_eq!(
            field(&profile, &["model_provider"]),
            Some(PROVIDER_KEY.into())
        );
        assert_eq!(field(&profile, &["model"]), Some("gpt-5.5".into()));
        assert_eq!(field(&profile, &["web_search"]), None, "search left on");
        let status = codex.status(Some(&store));
        assert_eq!(status.path, "/home/u/.codex/relay.config.toml");
        assert!(status.configured && !status.conflict);
        // Codex's /model keeps the profile's choice in its file: not a conflict.
        let text = read("relay.config.toml").unwrap();
        files
            .write(
                Path::new("/home/u/.codex/relay.config.toml"),
                text.replace("model = \"gpt-5.5\"", "model = \"gpt-5.5-mini\"")
                    .as_bytes(),
            )
            .unwrap();
        assert!(!codex.status(Some(&store)).conflict);

        // Switching to the default puts the profile file away first.
        let preview = codex.preview(&store, &request(), false).unwrap();
        assert!(preview.content.contains("relay.config.toml: removed"));
        codex
            .apply(&store, &request(), &preview.revision, false)
            .unwrap();
        assert!(read("relay.config.toml").is_none());
        let config = parse(read("config.toml").as_deref()).unwrap();
        assert_eq!(
            field(&config, &["model_provider"]),
            Some(PROVIDER_KEY.into())
        );
        assert!(!codex.status(Some(&store)).conflict);

        // And back to a profile puts config.toml back byte for byte.
        let preview = codex.preview(&store, &profiled, false).unwrap();
        codex
            .apply(&store, &profiled, &preview.revision, false)
            .unwrap();
        assert_eq!(read("config.toml").as_deref(), Some(ORIGINAL));
        codex.restore(false).unwrap();
        assert!(read("relay.config.toml").is_none());
        assert_eq!(read("config.toml").as_deref(), Some(ORIGINAL));
        assert_eq!(
            read(".env").as_deref(),
            Some(
                "OTHER=1
"
            )
        );
        assert!(read(CATALOG_FILE).is_none());
        let _ = std::fs::remove_dir_all(journal);
    }

    #[test]
    fn a_profile_name_must_be_one_codex_can_take() {
        let (files, journal, data) = setup();
        let codex = CodexRemote::new(&files, "/home/u", &journal, &data, Vec::new());
        let bad = CodexRemoteRequest {
            profile: Some("my relay".into()),
            ..request()
        };
        assert_eq!(
            codex.preview(&store(), &bad, false).err().as_deref(),
            Some("remote.errors.codexProfile")
        );
        let _ = std::fs::remove_dir_all(journal);
    }

    #[test]
    fn env_blocks_are_owned_exactly() {
        assert_eq!(strip_env("A=1\n").unwrap(), "A=1\n");
        let with = with_env("A=1", "k").unwrap();
        assert_eq!(strip_env(&with).unwrap(), "A=1\n");
        assert!(strip_env(&format!("{ENV_NAME}=x\n")).is_err());
    }
}
