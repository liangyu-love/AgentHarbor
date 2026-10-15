//! DeepSeek Harness (`dsh`) injection.
//!
//! ```text
//! $DSH_HOME/profiles/<profile>/cordis.patch.yml
//!   - id: llm-pi-ai              config.providers.agentharbor-*   (one route per protocol)
//!   - id: agent-default-model    config.provider / config.model   (optional)
//! $DSH_HOME/.credentials.yaml    refs.AGENTHARBOR_*               (keys the routes name)
//! ```
//! A patch row's `config` replaces the composed row's whole config, so the
//! routes go into the user's own `llm-pi-ai` row next to their providers
//! rather than into a second row. Only the fields listed above belong to a
//! session: restore puts back exactly those, the files byte for byte when
//! nothing else changed since. Harness reloads the profile patch on change;
//! the home-level patch outranks it, which is reported, never edited.
mod routes;
mod yaml;

pub(super) use routes::PROTOCOLS;

use super::*;

/// The local router's token, as the routes reference it.
pub(super) const ROUTER_CREDENTIAL: &str = "AGENTHARBOR_ROUTER_TOKEN";
pub(super) const AGGREGATION_NAME: &str = "AgentHarbor Aggregation";
const PROVIDERS_ROW: &str = "llm-pi-ai";
const DEFAULT_ROW: &str = "agent-default-model";
const PATCH_FILE: &str = "cordis.patch.yml";
const CREDENTIALS_FILE: &str = ".credentials.yaml";

/// `$DSH_HOME` when set (a blank value is unset, as Harness reads it), else `~/.dsh`.
pub(super) fn home(user_home: &Path, configured: Option<std::ffi::OsString>) -> PathBuf {
    match configured
        .as_deref()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some("~") => user_home.to_owned(),
        Some(value) if value.starts_with("~/") || value.starts_with("~\\") => {
            user_home.join(&value[2..])
        }
        Some(value) => PathBuf::from(value),
        None => user_home.join(".dsh"),
    }
}

fn valid_profile(name: &str) -> bool {
    name.chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphanumeric())
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && name != "node_modules"
}

/// The profiles initialized under `home`, Desktop's and the web profile first.
pub(super) fn profiles(home: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(home.join("profiles"))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| valid_profile(name))
        .filter(|name| {
            let dir = home.join("profiles").join(name);
            dir.join(PATCH_FILE).is_file() || dir.join("package.json").is_file()
        })
        .collect();
    names.sort_by_key(|name| {
        (
            !matches!(name.as_str(), "desktop" | "web"),
            name.as_str() != "desktop",
            name.clone(),
        )
    });
    names
}

fn patch_path(home: &Path, profile: &str) -> PathBuf {
    home.join("profiles").join(profile).join(PATCH_FILE)
}

/// What the client list shows about Harness on this machine.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessInfo {
    pub home: String,
    pub profiles: Vec<HarnessProfile>,
    pub credentials: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessProfile {
    pub name: String,
    pub path: String,
}

pub(super) fn info(home: &Path) -> HarnessInfo {
    HarnessInfo {
        home: home.to_string_lossy().into(),
        profiles: profiles(home)
            .into_iter()
            .map(|name| HarnessProfile {
                path: patch_path(home, &name).to_string_lossy().into(),
                name,
            })
            .collect(),
        credentials: home.join(CREDENTIALS_FILE).to_string_lossy().into(),
    }
}

// ---- the session ---------------------------------------------------------------

/// A field this app owns in one of the two files.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "file", rename_all = "camelCase")]
enum Field {
    /// A value inside the `config` of the patch row `row`.
    Patch { row: String, path: Vec<String> },
    /// `refs.<name>` in the credential store.
    Credential { name: String },
}

impl Field {
    fn patch(row: &str, path: &[&str]) -> Self {
        Field::Patch {
            row: row.into(),
            path: path.iter().map(|key| (*key).to_owned()).collect(),
        }
    }

    /// Choosing another model in Harness rewrites these: client activity, not a conflict.
    fn is_default_model(&self) -> bool {
        matches!(self, Field::Patch { row, .. } if row == DEFAULT_ROW)
    }

    fn label(&self) -> String {
        match self {
            Field::Patch { row, path } => std::iter::once(row.as_str())
                .chain(path.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join("."),
            Field::Credential { name } => format!("credentials.refs.{name}"),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Change {
    field: Field,
    before: Option<Value>,
    after: Option<Value>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileState {
    path: PathBuf,
    /// The rollback point, restored byte for byte while the file still holds
    /// exactly what was installed.
    original: Option<String>,
    installed: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Session {
    profile: String,
    patch: FileState,
    credentials: FileState,
    changes: Vec<Change>,
    /// Patch rows this session added; withdrawn once nothing of theirs is left.
    created_rows: Vec<String>,
    request: ConfigRequest,
    /// A direct configuration's cards as written, for the stale check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    connection: Option<String>,
}

impl Session {
    /// The route keys this session wrote.
    fn route_keys(&self) -> Vec<&str> {
        self.changes
            .iter()
            .filter(|change| change.after.is_some())
            .filter_map(|change| match &change.field {
                Field::Patch { row, path }
                    if row == PROVIDERS_ROW && path.len() == 2 && path[0] == "providers" =>
                {
                    Some(path[1].as_str())
                }
                _ => None,
            })
            .collect()
    }
}

/// The two files as text.
#[derive(Clone, PartialEq, Serialize)]
struct Texts {
    patch: Option<String>,
    credentials: Option<String>,
}

impl Texts {
    fn read(patch: &Path, credentials: &Path) -> Result<Self, String> {
        Ok(Self {
            patch: read_text(patch)?,
            credentials: read_text(credentials)?,
        })
    }

    fn value(&self, field: &Field) -> Result<Option<Value>, String> {
        Ok(match field {
            Field::Patch { row, path } => {
                yaml::patch_value(&yaml::read(self.patch.as_deref())?, row, path).cloned()
            }
            Field::Credential { name } => {
                yaml::credential(&yaml::read(self.credentials.as_deref())?, name).cloned()
            }
        })
    }

    fn set(&mut self, field: &Field, value: Option<&Value>) -> Result<(), String> {
        match field {
            Field::Patch { row, path } => {
                self.patch = yaml::set_patch(self.patch.as_deref(), row, path, value)?
            }
            Field::Credential { name } => {
                let value = match value {
                    Some(value) => Some(value.as_str().ok_or("clients.errors.invalidConfig")?),
                    None => None,
                };
                self.credentials = yaml::set_credential(self.credentials.as_deref(), name, value)?;
            }
        }
        Ok(())
    }
}

/// What a restore does with managed fields the user changed since the apply.
#[derive(Clone, Copy, PartialEq)]
enum Policy {
    /// Refuse the restore (an ordinary restore before confirmation).
    Refuse,
    /// Put the baseline back over them (a confirmed restore).
    Replace,
    /// Keep them and restore everything else (the restore on exit). A route
    /// still aimed at the local router is withdrawn even when edited: it
    /// cannot work once this process is gone.
    KeepEdits,
}

fn aims_at_router(value: Option<&Value>) -> bool {
    value
        .and_then(|value| value.get("baseURL"))
        .and_then(Value::as_str)
        .is_some_and(|base| base.starts_with("http://127.0.0.1:"))
}

fn conflicts(session: &Session, texts: &Texts) -> Result<Vec<Field>, String> {
    let mut fields = Vec::new();
    for change in &session.changes {
        if change.field.is_default_model() {
            continue;
        }
        let current = texts.value(&change.field)?;
        if current != change.after && current != change.before {
            fields.push(change.field.clone());
        }
    }
    Ok(fields)
}

fn is_empty_object(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_object)
        .is_some_and(|object| object.is_empty())
}

/// The session's fields put back over `texts`, except those in `kept`. The
/// default model goes back only while it still names one of this session's
/// routes: a model the user picked in Harness since is theirs. Containers and
/// rows this session made are withdrawn once empty, and a file it created
/// goes once nothing is left in it.
fn undo(session: &Session, texts: &Texts, kept: &[Field]) -> Result<Texts, String> {
    let mut result = texts.clone();
    let installed = |state: &FileState, current: &Option<String>| state.installed == *current;
    let patch_exact = installed(&session.patch, &texts.patch);
    let credentials_exact = installed(&session.credentials, &texts.credentials);
    if patch_exact {
        result.patch = session.patch.original.clone();
    }
    if credentials_exact {
        result.credentials = session.credentials.original.clone();
    }
    if patch_exact && credentials_exact {
        return Ok(result);
    }
    let ours = session.route_keys();
    let default_provider = texts.value(&Field::patch(DEFAULT_ROW, &["provider"]))?;
    let restore_default = default_provider
        .as_ref()
        .and_then(Value::as_str)
        .is_some_and(|provider| ours.contains(&provider));
    for change in session.changes.iter().rev() {
        let exact = match change.field {
            Field::Patch { .. } => patch_exact,
            Field::Credential { .. } => credentials_exact,
        };
        if exact
            || (change.field.is_default_model() && !restore_default)
            || kept.contains(&change.field)
        {
            continue;
        }
        result.set(&change.field, change.before.as_ref())?;
    }
    if !patch_exact {
        let original = yaml::read(session.patch.original.as_deref())?;
        for change in &session.changes {
            let Field::Patch { row, path } = &change.field else {
                continue;
            };
            for depth in (0..path.len()).rev() {
                let parent = &path[..depth];
                let current = yaml::read(result.patch.as_deref())?;
                if yaml::patch_value(&original, row, parent).is_none()
                    && is_empty_object(yaml::patch_value(&current, row, parent))
                {
                    result.patch = yaml::set_patch(result.patch.as_deref(), row, parent, None)?;
                }
            }
        }
        for row in &session.created_rows {
            let current = yaml::read(result.patch.as_deref())?;
            // Nothing but its address is left: the row was only ever ours.
            let leftover = yaml::row(&current, row)
                .and_then(Value::as_object)
                .is_some_and(|object| {
                    object.iter().all(|(key, value)| {
                        key == "id"
                            || key == "name"
                            || (key == "config" && is_empty_object(Some(value)))
                    })
                });
            if leftover {
                result.patch = yaml::remove_row(result.patch.as_deref(), row)?;
            }
        }
        if session.patch.original.is_none() && yaml::read(result.patch.as_deref())? == json!([]) {
            result.patch = None;
        }
    }
    if !credentials_exact {
        let original = yaml::read(session.credentials.original.as_deref())?;
        if original.get("refs").is_none()
            && is_empty_object(yaml::read(result.credentials.as_deref())?.get("refs"))
        {
            // `refs` came with this session and nothing of the user's is in it.
            result.credentials = yaml::remove_key(result.credentials.as_deref(), "refs")?;
        }
        if session.credentials.original.is_none()
            && yaml::read(result.credentials.as_deref())? == json!({"version": 1})
        {
            result.credentials = None;
        }
    }
    Ok(result)
}

impl ConfigManager {
    fn harness_home(&self) -> &Path {
        &self.paths.harness
    }

    fn harness_session(&self) -> Result<Option<Session>, String> {
        secret_file::read(&self.session_path(TargetId::Harness))?
            .map(|bytes| {
                serde_json::from_slice::<Option<Session>>(&bytes)
                    .map_err(|_| "clients.errors.storage".to_string())
            })
            .transpose()
            .map(Option::flatten)
    }

    pub(super) fn harness_stored_request(&self) -> Result<Option<ConfigRequest>, String> {
        Ok(self.harness_session()?.map(|session| session.request))
    }

    pub(super) fn harness_status(&self, store: Option<&Store>) -> ClientStatus {
        let home = self.harness_home();
        let info = info(home);
        let default_profile = info.profiles.first().map(|profile| profile.name.clone());
        let mut status = ClientStatus {
            target_id: TargetId::Harness,
            name: "DeepSeek Harness".into(),
            path: default_profile
                .as_deref()
                .map(|profile| patch_path(home, profile))
                .unwrap_or_else(|| home.join("profiles"))
                .to_string_lossy()
                .into(),
            protocols: PROTOCOLS.to_vec(),
            configured: false,
            conflict: false,
            error: None,
            provider_id: None,
            model: None,
            request: None,
            conflict_fields: Vec::new(),
            client_version: None,
            stale: false,
            harness: Some(info),
        };
        let result = (|| -> Result<(), String> {
            let Some(session) = self.harness_session()? else {
                if let Some(profile) = default_profile {
                    yaml::read(read_text(&patch_path(home, &profile))?.as_deref())?;
                }
                return Ok(());
            };
            status.configured = true;
            status.path = session.patch.path.to_string_lossy().into();
            status.provider_id = Some(session.request.provider_id.clone());
            status.model = Some(session.request.model.clone());
            status.request = Some(session.request.clone());
            status.stale = match (store, &session.connection) {
                (Some(store), Some(written)) => {
                    connection_fingerprint(store, &session.request, &DirectProxy::None) != *written
                }
                _ => false,
            };
            let texts = Texts::read(&session.patch.path, &session.credentials.path)?;
            let fields = conflicts(&session, &texts)?;
            status.conflict = !fields.is_empty();
            status.conflict_fields = fields.iter().map(Field::label).collect();
            Ok(())
        })();
        status.error = result.err();
        status
    }

    fn harness_prepare(
        &self,
        store: &Store,
        request: &ConfigRequest,
        rebaseline: bool,
        replace_conflicts: bool,
    ) -> Result<HarnessPlan, String> {
        let canonical = aggregate_models::canonical_request(store, request)?;
        let request = &canonical;
        if request.aggregate_target().is_none() {
            let provider = selected_provider(store, request)?;
            if !request.accepts_model(provider, &request.model) {
                return Err("clients.errors.protocol".into());
            }
        }
        validate_model(&request.model)?;
        if request
            .context_window
            .is_some_and(|value| !(1_024..=100_000_000).contains(&value))
            || request
                .max_output_tokens
                .is_some_and(|value| value == 0 || value > 100_000_000)
            || matches!((request.context_window, request.max_output_tokens), (Some(context), Some(output)) if output > context)
        {
            return Err("clients.errors.limits".into());
        }
        if request.needs_router() && store.router.bearer_token.is_empty() {
            return Err("clients.errors.router".into());
        }
        let home = self.harness_home();
        let profile = request
            .options
            .harness_profile
            .as_deref()
            .filter(|name| valid_profile(name))
            .ok_or("clients.errors.harnessProfile")?;
        if !profiles(home).iter().any(|name| name == profile) {
            return Err("clients.errors.harnessProfile".into());
        }
        let mut previous = self.harness_session()?;
        if previous
            .as_ref()
            .is_some_and(|session| session.profile != profile)
        {
            return Err("clients.errors.harnessProfileChanged".into());
        }
        let patch = patch_path(home, profile);
        let credentials = home.join(CREDENTIALS_FILE);
        let current = Texts::read(&patch, &credentials)?;
        let (baseline, original) = match previous.as_mut() {
            Some(session) => {
                let conflicted = conflicts(session, &current)?;
                if rebaseline {
                    // The user's values become the rollback point.
                    for change in &mut session.changes {
                        if conflicted.contains(&change.field) {
                            change.before = current.value(&change.field)?;
                        }
                    }
                } else if !replace_conflicts && !conflicted.is_empty() {
                    return Err("clients.errors.conflict".into());
                }
                let baseline = undo(session, &current, &[])?;
                let original = Texts {
                    patch: if current.patch == session.patch.installed && !rebaseline {
                        session.patch.original.clone()
                    } else {
                        baseline.patch.clone()
                    },
                    credentials: if current.credentials == session.credentials.installed
                        && !rebaseline
                    {
                        session.credentials.original.clone()
                    } else {
                        baseline.credentials.clone()
                    },
                };
                (baseline, original)
            }
            None => (current.clone(), current.clone()),
        };
        let built = {
            let catalog = models_dev::load(&self.app_data_dir);
            let official = self
                .official_home
                .as_ref()
                .map(|home| model_catalog::official(home, &self.app_data_dir))
                .unwrap_or_default();
            routes::build(store, request, &catalog, &official.models)?
        };
        let mut edits: Vec<(Field, Option<Value>)> = Vec::new();
        for route in &built.routes {
            let field = Field::patch(PROVIDERS_ROW, &["providers", &route.key]);
            // Left after withdrawing this app's routes: the user's own.
            if baseline.value(&field)?.is_some() {
                return Err("clients.errors.harnessProviderConflict".into());
            }
            edits.push((field, Some(route.value.clone())));
        }
        if let Some((provider, model)) = &built.default {
            edits.push((
                Field::patch(DEFAULT_ROW, &["provider"]),
                Some(json!(provider)),
            ));
            edits.push((Field::patch(DEFAULT_ROW, &["model"]), Some(json!(model))));
            // A saved effort belongs to the previous model.
            edits.push((Field::patch(DEFAULT_ROW, &["reasoningEffort"]), None));
        }
        for (name, value) in &built.credentials {
            edits.push((Field::Credential { name: name.clone() }, Some(json!(value))));
        }
        let rows_before = yaml::read(baseline.patch.as_deref())?;
        let mut texts = baseline.clone();
        let mut changes = Vec::new();
        for (field, value) in edits {
            let before = texts.value(&field)?;
            if before.is_none() && value.is_none() {
                continue;
            }
            texts.set(&field, value.as_ref())?;
            changes.push(Change {
                field,
                before,
                after: value,
            });
        }
        let rows_after = yaml::read(texts.patch.as_deref())?;
        let created_rows = [PROVIDERS_ROW, DEFAULT_ROW]
            .into_iter()
            .filter(|row| {
                yaml::row_index(&rows_before, row).ok().flatten().is_none()
                    && yaml::row_index(&rows_after, row).ok().flatten().is_some()
            })
            .map(str::to_owned)
            .collect();
        let warnings = self.harness_warnings(&rows_after, built.default.is_some())?;
        let session = Session {
            profile: profile.to_owned(),
            patch: FileState {
                path: patch,
                original: original.patch,
                installed: texts.patch.clone(),
            },
            credentials: FileState {
                path: credentials,
                original: original.credentials,
                installed: texts.credentials.clone(),
            },
            changes,
            created_rows,
            request: request.clone(),
            connection: (!request.needs_router())
                .then(|| connection_fingerprint(store, request, &DirectProxy::None)),
        };
        let mut hasher = Sha256::new();
        hasher.update(
            serde_json::to_vec(&(&current, &session, rebaseline, replace_conflicts))
                .map_err(|_| "clients.errors.storage")?,
        );
        Ok(HarnessPlan {
            revision: format!("{:x}", hasher.finalize()),
            current,
            next: texts,
            session,
            warnings,
        })
    }

    /// Notices about layers that decide over the profile patch.
    fn harness_warnings(&self, patch: &Value, default_model: bool) -> Result<Vec<String>, String> {
        let mut warnings = Vec::new();
        let home_patch = yaml::read(read_text(&self.harness_home().join(PATCH_FILE))?.as_deref())
            .unwrap_or(Value::Null);
        // A home-level row replaces the profile row's whole config.
        let overrides = |row: &str| {
            yaml::row(&home_patch, row).is_some_and(|row| {
                row.get("config").is_some() || row.get("disabled") == Some(&json!(true))
            })
        };
        if overrides(PROVIDERS_ROW) || (default_model && overrides(DEFAULT_ROW)) {
            warnings.push("clients.warnings.harnessHomeOverride".into());
        }
        if yaml::row(patch, PROVIDERS_ROW)
            .is_some_and(|row| row.get("disabled") == Some(&json!(true)))
        {
            warnings.push("clients.warnings.harnessDisabled".into());
        }
        let reload_off = |document: &Value| {
            yaml::row(document, "hmr").is_some_and(|row| row.get("disabled") == Some(&json!(true)))
        };
        if reload_off(patch) || reload_off(&home_patch) {
            warnings.push("clients.warnings.harnessRestart".into());
        }
        Ok(warnings)
    }

    pub(super) fn harness_preview(
        &self,
        store: &Store,
        request: &ConfigRequest,
        replace_conflicts: bool,
    ) -> Result<ConfigPreview, String> {
        let plan = self.harness_prepare(store, request, false, replace_conflicts)?;
        let mut patch = json!({});
        let mut credentials = json!({});
        for change in &plan.session.changes {
            match &change.field {
                Field::Patch { row, path } => {
                    let key: Vec<String> = [row.clone(), "config".into()]
                        .into_iter()
                        .chain(path.iter().cloned())
                        .collect();
                    set_field(&mut patch, &key, change.after.clone())?;
                }
                Field::Credential { name } => {
                    set_field(
                        &mut credentials,
                        &["refs".into(), name.clone()],
                        change.after.as_ref().map(|_| json!("[managed locally]")),
                    )?;
                }
            }
        }
        redact(&mut patch);
        let mut content = serde_json::Map::new();
        content.insert(plan.session.patch.path.to_string_lossy().into(), patch);
        if credentials
            .as_object()
            .is_some_and(|object| !object.is_empty())
        {
            content.insert(
                plan.session.credentials.path.to_string_lossy().into(),
                credentials,
            );
        }
        Ok(ConfigPreview {
            path: plan.session.patch.path.to_string_lossy().into(),
            revision: plan.revision,
            content: serde_json::to_string_pretty(&content)
                .map_err(|_| "clients.errors.storage")?,
            removed_fields: plan
                .session
                .changes
                .iter()
                .filter(|change| change.after.is_none() && change.before.is_some())
                .map(|change| change.field.label())
                .collect(),
            warnings: plan.warnings,
        })
    }

    pub(super) fn harness_apply(
        &self,
        store: &Store,
        request: &ConfigRequest,
        expected_revision: &str,
        replace_conflicts: bool,
    ) -> Result<ConfigApplied, String> {
        let plan = self.harness_prepare(store, request, false, replace_conflicts)?;
        if plan.revision != expected_revision {
            return Err("clients.errors.changed".into());
        }
        self.harness_commit(
            &plan.session.patch.path,
            &plan.session.credentials.path,
            &plan.current,
            &plan.next,
            Some(&plan.session),
            plan.warnings,
        )
    }

    pub(super) fn harness_reapply(
        &self,
        store: &Store,
        rebaseline: bool,
    ) -> Result<ConfigApplied, String> {
        let session = self
            .harness_session()?
            .ok_or("clients.errors.notConfigured")?;
        let plan = self.harness_prepare(store, &session.request, rebaseline, false)?;
        self.harness_commit(
            &plan.session.patch.path,
            &plan.session.credentials.path,
            &plan.current,
            &plan.next,
            Some(&plan.session),
            plan.warnings,
        )
    }

    pub(super) fn harness_restore(&self, replace_conflicts: bool) -> Result<ConfigApplied, String> {
        self.harness_restore_with(if replace_conflicts {
            Policy::Replace
        } else {
            Policy::Refuse
        })
    }

    fn harness_restore_with(&self, policy: Policy) -> Result<ConfigApplied, String> {
        let session = self
            .harness_session()?
            .ok_or("clients.errors.notConfigured")?;
        let current = Texts::read(&session.patch.path, &session.credentials.path)?;
        let conflicted = conflicts(&session, &current)?;
        let mut warnings = Vec::new();
        let mut kept = Vec::new();
        if !conflicted.is_empty() {
            match policy {
                Policy::Refuse => return Err("clients.errors.conflict".into()),
                Policy::Replace => {}
                Policy::KeepEdits => {
                    for field in conflicted {
                        if !aims_at_router(current.value(&field)?.as_ref()) {
                            kept.push(field);
                        }
                    }
                    warnings.push("clients.warnings.keptEdits".into());
                }
            }
        }
        let next = undo(&session, &current, &kept)?;
        self.harness_commit(
            &session.patch.path,
            &session.credentials.path,
            &current,
            &next,
            None,
            warnings,
        )
    }

    pub(super) fn harness_release(&self) -> Result<(), String> {
        let journal = self.session_path(TargetId::Harness);
        if journal.exists() {
            storage::remove_file_durable(&journal).map_err(|_| "clients.errors.storage")?;
        }
        Ok(())
    }

    /// The router's routes stop working with this process: withdraw them,
    /// keeping the user's own edits.
    pub(super) fn harness_restore_on_exit(&self) -> Option<Result<(), String>> {
        let _guard = match WRITE_LOCK.get_or_init(|| Mutex::new(())).lock() {
            Ok(guard) => guard,
            Err(_) => return Some(Err("clients.errors.storage".into())),
        };
        match self.harness_session() {
            Ok(Some(session)) if session.request.needs_router() => {
                Some(self.harness_restore_with(Policy::KeepEdits).map(|_| ()))
            }
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        }
    }

    pub(super) fn harness_uses_router(&self) -> Result<bool, String> {
        Ok(self
            .harness_session()?
            .is_some_and(|session| session.request.needs_router()))
    }

    fn harness_commit(
        &self,
        patch: &Path,
        credentials: &Path,
        current: &Texts,
        next: &Texts,
        session: Option<&Session>,
        warnings: Vec<String>,
    ) -> Result<ConfigApplied, String> {
        if Texts::read(patch, credentials)? != *current {
            return Err("clients.errors.changed".into());
        }
        let backup_path = self
            .data_dir
            .join("backups")
            .join(format!("{}.dat", uuid::Uuid::new_v4()));
        secret_file::write(
            &backup_path,
            &serde_json::to_vec(&json!({"harness": {"patch": [patch, &current.patch], "credentials": [credentials, &current.credentials]}}))
                .map_err(|_| "clients.errors.storage")?,
        )?;
        storage::prune_backups(&self.data_dir.join("backups"), 20, |path| {
            path.extension().is_some_and(|extension| extension == "dat")
        });
        let previous = self.harness_session()?;
        let journal = self.session_path(TargetId::Harness);
        secret_file::write(
            &journal,
            &serde_json::to_vec(&session).map_err(|_| "clients.errors.storage")?,
        )?;
        let write = |path: &Path, text: &Option<String>| -> Result<(), String> {
            match text {
                Some(text) => storage::write_atomic(path, text.as_bytes())
                    .map_err(|_| "clients.errors.storage".to_string()),
                None => storage::remove_file_durable(path)
                    .map_err(|_| "clients.errors.storage".to_string()),
            }
        };
        let result = (|| -> Result<(), String> {
            if Texts::read(patch, credentials)? != *current {
                return Err("clients.errors.changed".into());
            }
            if next.patch != current.patch {
                write(patch, &next.patch)?;
            }
            if next.credentials != current.credentials {
                if let Err(error) = write(credentials, &next.credentials) {
                    // Both files or neither: the patch must not name keys that are missing.
                    let _ = write(patch, &current.patch);
                    return Err(error);
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            secret_file::write(
                &journal,
                &serde_json::to_vec(&previous).map_err(|_| "clients.errors.storage")?,
            )?;
            return Err(error);
        }
        Ok(ConfigApplied {
            path: patch.to_string_lossy().into(),
            backup_path: backup_path.to_string_lossy().into(),
            warnings,
        })
    }
}

struct HarnessPlan {
    revision: String,
    current: Texts,
    next: Texts,
    session: Session,
    warnings: Vec<String>,
}

#[cfg(test)]
mod real_client_tests;
#[cfg(test)]
mod tests;
