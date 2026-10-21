//! The Skills library: one copy of each skill in AgentHarbor's data folder,
//! linked into the skills folder of every agent it is on. Whether a skill is on
//! for an agent is read from that folder — a link to the library's copy — so
//! there is no second record to drift. Skills an agent has that are not in
//! the library are listed for import; links another tool made are left alone.
use super::{
    backup_skill_directory, copy_dir_all, first_markdown_heading, first_skill_summary_line, link,
    load_skill_backups, resolve_skill_source, retire_directory, SkillBackupMetadata,
};
use crate::core::client_config::ClientPaths;
use crate::core::models::{CoreError, SkillBackupSummary};
use crate::platform::paths::CodexPaths;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Changes to the library and the agents' folders happen one at a time.
static LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SkillApp {
    Codex,
    ClaudeCode,
    OpenCode,
    Pi,
    Cursor,
    Harness,
}

impl SkillApp {
    pub const ALL: [SkillApp; 6] = [
        SkillApp::Codex,
        SkillApp::ClaudeCode,
        SkillApp::OpenCode,
        SkillApp::Pi,
        SkillApp::Cursor,
        SkillApp::Harness,
    ];
}

struct Target {
    app: SkillApp,
    dir: PathBuf,
    installed: bool,
}

/// Where the library, its backups and the agents' skills folders are.
pub struct SkillContext {
    library: PathBuf,
    backups: PathBuf,
    /// Backups and turned-off skills older versions kept per client.
    legacy_backups: Vec<(PathBuf, PathBuf)>,
    legacy_disabled: Vec<(SkillApp, PathBuf)>,
    targets: Vec<Target>,
}

impl SkillContext {
    pub fn new(codex: &CodexPaths, clients: &ClientPaths) -> Self {
        let parent_exists = |path: &Path| path.parent().is_some_and(Path::is_dir);
        let opencode_dir = clients
            .opencode
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let pi_dir = clients
            .pi
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let claude_clients = codex.app_data_dir.join("skill-clients/claude-code");
        Self {
            library: codex.app_data_dir.join("skills-library"),
            backups: codex.skill_backups_dir.clone(),
            legacy_backups: vec![(claude_clients.clone(), claude_clients.join("skill-backups"))],
            legacy_disabled: vec![
                (SkillApp::Codex, codex.app_data_dir.join("disabled-skills")),
                (SkillApp::ClaudeCode, claude_clients.join("disabled-skills")),
            ],
            targets: vec![
                Target {
                    app: SkillApp::Codex,
                    dir: codex.skills_dir.clone(),
                    installed: codex.codex_home.is_dir(),
                },
                Target {
                    app: SkillApp::ClaudeCode,
                    dir: clients.claude_root().join("skills"),
                    installed: clients.claude_root().is_dir(),
                },
                Target {
                    app: SkillApp::OpenCode,
                    installed: opencode_dir.is_dir(),
                    dir: opencode_dir.join("skills"),
                },
                Target {
                    app: SkillApp::Pi,
                    installed: pi_dir.is_dir(),
                    dir: pi_dir.join("skills"),
                },
                Target {
                    app: SkillApp::Cursor,
                    installed: clients.cursor_cli.is_dir(),
                    dir: clients.cursor_cli.join("skills"),
                },
                Target {
                    app: SkillApp::Harness,
                    installed: parent_exists(&clients.harness.join("skills")),
                    dir: clients.harness.join("skills"),
                },
            ],
        }
    }

    pub fn discover(codex: &CodexPaths) -> Self {
        Self::new(codex, &ClientPaths::discover())
    }

    fn target(&self, app: SkillApp) -> &Target {
        self.targets
            .iter()
            .find(|target| target.app == app)
            .expect("every app has a target")
    }

    fn copy_of(&self, name: &str) -> PathBuf {
        self.library.join(name)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInfo {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SkillCellState {
    On,
    Off,
    /// The agent's folder has something else under this name.
    Occupied,
    NotInstalled,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillCell {
    pub app: SkillApp,
    pub on: bool,
    pub state: SkillCellState,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillView {
    #[serde(flatten)]
    pub info: SkillInfo,
    pub cells: Vec<SkillCell>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillAppView {
    pub app: SkillApp,
    pub dir: String,
    pub installed: bool,
}

/// A skill in an agent's folder that the library does not manage.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnmanagedSkill {
    #[serde(flatten)]
    pub info: SkillInfo,
    pub app: SkillApp,
    /// Turned off by an older AgentHarbor and kept aside.
    pub legacy_disabled: bool,
    /// A link another tool made; it is shown, not imported.
    pub external: bool,
    /// The library already has a skill by this name.
    pub in_library: bool,
    /// That skill has other contents, so importing this one would be refused.
    pub conflict: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillFailure {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<SkillApp>,
    pub error: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillLibraryView {
    pub library: String,
    pub skills: Vec<SkillView>,
    pub apps: Vec<SkillAppView>,
    pub unmanaged: Vec<UnmanagedSkill>,
    pub backups: Vec<SkillBackupSummary>,
    pub failures: Vec<SkillFailure>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillImport {
    pub name: String,
    pub app: SkillApp,
    #[serde(default)]
    pub legacy_disabled: bool,
}

/// A skill folder name: one plain, visible path component.
fn checked_name(name: &str) -> Result<&str, String> {
    let plain = !name.is_empty()
        && !name.starts_with('.')
        && name != ".."
        && !name.contains(['/', '\\', ':'])
        && Path::new(name).components().count() == 1;
    if plain {
        Ok(name)
    } else {
        Err("skills.errors.name".into())
    }
}

/// `key: value` pairs of a SKILL.md front matter; `>` and `|` blocks are joined.
fn front_matter(text: &str) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    let mut lines = text.trim_start_matches('\u{feff}').lines();
    if lines.next().map(str::trim) != Some("---") {
        return values;
    }
    let mut block: Option<(String, Vec<String>)> = None;
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        if let Some((key, parts)) = &mut block {
            if line.starts_with(' ') || line.starts_with('\t') || line.trim().is_empty() {
                parts.push(line.trim().to_owned());
                continue;
            }
            values.insert(key.clone(), parts.join(" ").trim().to_owned());
            block = None;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if key.starts_with(' ') || key.trim().is_empty() {
            continue;
        }
        let value = value.trim();
        if matches!(value, ">" | "|" | ">-" | "|-") {
            block = Some((key.trim().to_owned(), Vec::new()));
        } else {
            let value = value.trim_matches(|c| c == '"' || c == '\'');
            values.insert(key.trim().to_owned(), value.to_owned());
        }
    }
    if let Some((key, parts)) = block {
        values.insert(key, parts.join(" ").trim().to_owned());
    }
    values
}

/// What a skill folder says about itself, when it has a SKILL.md.
fn info(dir: &Path) -> Option<SkillInfo> {
    let file = dir.join("SKILL.md");
    let text = std::fs::read_to_string(&file).ok()?;
    let meta = front_matter(&text);
    let updated_at = std::fs::metadata(&file)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|age| age.as_secs() as i64);
    Some(SkillInfo {
        name: dir.file_name()?.to_string_lossy().into_owned(),
        title: meta
            .get("name")
            .filter(|name| !name.is_empty())
            .cloned()
            .or_else(|| first_markdown_heading(&text)),
        description: meta
            .get("description")
            .filter(|text| !text.is_empty())
            .cloned()
            .or_else(|| first_skill_summary_line(&text)),
        path: dir.display().to_string(),
        updated_at,
    })
}

/// The visible entries of a folder.
fn entries(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            !path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with('.'))
        })
        .collect();
    entries.sort();
    entries
}

/// Whether two skill folders hold the same files with the same contents.
fn same_tree(a: &Path, b: &Path) -> bool {
    let names = |dir: &Path| -> Option<Vec<std::ffi::OsString>> {
        let mut names: Vec<_> = std::fs::read_dir(dir)
            .ok()?
            .flatten()
            .map(|entry| entry.file_name())
            .collect();
        names.sort();
        Some(names)
    };
    match (names(a), names(b)) {
        (Some(left), Some(right)) if left == right => left.iter().all(|name| {
            let (x, y) = (a.join(name), b.join(name));
            if x.is_dir() {
                y.is_dir() && same_tree(&x, &y)
            } else {
                std::fs::read(&x).ok() == std::fs::read(&y).ok()
            }
        }),
        _ => false,
    }
}

fn library_skills(context: &SkillContext) -> Vec<SkillInfo> {
    entries(&context.library)
        .into_iter()
        .filter(|path| path.is_dir() && !link::is_link(path))
        .filter_map(|path| info(&path))
        .collect()
}

fn backups(context: &SkillContext) -> Vec<SkillBackupSummary> {
    let mut all = load_skill_backups(&context.backups).unwrap_or_default();
    for (_, dir) in &context.legacy_backups {
        all.extend(load_skill_backups(dir).unwrap_or_default());
    }
    all.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    all
}

fn view(context: &SkillContext, failures: Vec<SkillFailure>) -> SkillLibraryView {
    let skills = library_skills(context);
    let in_library = |name: &str| skills.iter().any(|skill| skill.name == name);
    // Only a copy with the same contents is imported; another is refused.
    let conflict = |name: &str, path: &Path| in_library(name) && !same_tree(path, &context.copy_of(name));
    let mut unmanaged = Vec::new();
    for target in context.targets.iter().filter(|target| target.installed) {
        for path in entries(&target.dir) {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            if link::links_to(&path, &context.copy_of(&name)) {
                continue;
            }
            let external = link::is_link(&path);
            if !external && !path.is_dir() {
                continue;
            }
            if let Some(info) = info(&path) {
                unmanaged.push(UnmanagedSkill {
                    in_library: in_library(&name),
                    conflict: !external && conflict(&name, &path),
                    info,
                    app: target.app,
                    legacy_disabled: false,
                    external,
                });
            }
        }
    }
    for (app, dir) in &context.legacy_disabled {
        for path in entries(dir)
            .into_iter()
            .filter(|path| path.is_dir() && !link::is_link(path))
        {
            if let Some(info) = info(&path) {
                unmanaged.push(UnmanagedSkill {
                    in_library: in_library(&info.name),
                    conflict: conflict(&info.name, &path),
                    info,
                    app: *app,
                    legacy_disabled: true,
                    external: false,
                });
            }
        }
    }
    let skills = skills
        .into_iter()
        .map(|info| {
            let copy = context.copy_of(&info.name);
            let cells = context
                .targets
                .iter()
                .map(|target| {
                    let place = target.dir.join(&info.name);
                    let state = if !target.installed {
                        SkillCellState::NotInstalled
                    } else if link::links_to(&place, &copy) {
                        SkillCellState::On
                    } else if std::fs::symlink_metadata(&place).is_ok() {
                        SkillCellState::Occupied
                    } else {
                        SkillCellState::Off
                    };
                    SkillCell {
                        app: target.app,
                        on: state == SkillCellState::On,
                        state,
                    }
                })
                .collect();
            SkillView { info, cells }
        })
        .collect();
    SkillLibraryView {
        library: context.library.display().to_string(),
        skills,
        apps: context
            .targets
            .iter()
            .map(|target| SkillAppView {
                app: target.app,
                dir: target.dir.display().to_string(),
                installed: target.installed,
            })
            .collect(),
        unmanaged,
        backups: backups(context),
        failures,
    }
}

pub fn load_skills(context: &SkillContext) -> Result<SkillLibraryView, String> {
    let _guard = LOCK.lock().map_err(|_| "skills.errors.library")?;
    Ok(view(context, Vec::new()))
}

/// Link the library's copy into the agent's folder, or take the link out.
fn switch(context: &SkillContext, name: &str, app: SkillApp, on: bool) -> Result<(), String> {
    let target = context.target(app);
    let copy = context.copy_of(name);
    let place = target.dir.join(name);
    if on {
        if !target.installed {
            return Err("skills.errors.notInstalled".into());
        }
        if link::links_to(&place, &copy) {
            return Ok(());
        }
        if std::fs::symlink_metadata(&place).is_ok() {
            return Err("skills.errors.occupied".into());
        }
        std::fs::create_dir_all(&target.dir).map_err(|_| "skills.errors.link")?;
        link::create(&place, &copy).map_err(|error| {
            log::warn!("linking skill {name} for {app:?} failed: {error}");
            "skills.errors.link".to_owned()
        })
    } else if link::links_to(&place, &copy) {
        link::remove(&place).map_err(|_| "skills.errors.unlink".into())
    } else {
        Ok(())
    }
}

/// Turn the named skills on or off for `app`, or for every installed agent.
pub fn set_skills(
    context: &SkillContext,
    names: &[String],
    app: Option<SkillApp>,
    on: bool,
) -> Result<SkillLibraryView, String> {
    let _guard = LOCK.lock().map_err(|_| "skills.errors.library")?;
    let mut failures = Vec::new();
    for name in names {
        let name = checked_name(name)?;
        if !context.copy_of(name).is_dir() {
            return Err("skills.errors.notFound".into());
        }
        let apps: Vec<SkillApp> = match app {
            Some(app) => vec![app],
            None => context
                .targets
                .iter()
                .filter(|target| target.installed)
                .map(|target| target.app)
                .collect(),
        };
        for app in apps {
            if let Err(error) = switch(context, name, app, on) {
                failures.push(SkillFailure {
                    name: name.to_owned(),
                    app: Some(app),
                    error,
                });
            }
        }
    }
    Ok(view(context, failures))
}

/// Copy `source` into the library under its folder name, through a staging
/// folder so a failed copy leaves nothing half written.
fn copy_in(context: &SkillContext, source: &Path, name: &str) -> Result<(), String> {
    std::fs::create_dir_all(&context.library).map_err(|_| "skills.errors.library")?;
    let staged = context
        .library
        .join(format!(".agentharbor-stage-{}", uuid::Uuid::new_v4()));
    let result = copy_dir_all(source, &staged)
        .map_err(|error| match error {
            CoreError::InvalidData(_) => "skills.errors.linkedInside".to_owned(),
            _ => "skills.errors.copy".to_owned(),
        })
        .and_then(|()| {
            std::fs::rename(&staged, context.copy_of(name)).map_err(|_| "skills.errors.copy".into())
        });
    if staged.exists() {
        let _ = std::fs::remove_dir_all(&staged);
    }
    result
}

/// Add a skill folder (or the folder of a SKILL.md) and turn it on for `apps`.
pub fn add_skill(
    context: &SkillContext,
    source: &str,
    apps: &[SkillApp],
) -> Result<SkillLibraryView, String> {
    let source =
        resolve_skill_source(Path::new(source)).map_err(|_| "skills.errors.source".to_owned())?;
    let name = source
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or("skills.errors.source")?;
    let name = checked_name(&name)?.to_owned();
    let _guard = LOCK.lock().map_err(|_| "skills.errors.library")?;
    if std::fs::symlink_metadata(context.copy_of(&name)).is_ok() {
        return Err("skills.errors.duplicate".into());
    }
    let inside_library = std::fs::canonicalize(&source).is_ok_and(|source| {
        std::fs::canonicalize(&context.library).is_ok_and(|library| source.starts_with(library))
    });
    if inside_library {
        return Err("skills.errors.source".into());
    }
    copy_in(context, &source, &name)?;
    let mut failures = Vec::new();
    for app in apps {
        if let Err(error) = switch(context, &name, *app, true) {
            failures.push(SkillFailure {
                name: name.clone(),
                app: Some(*app),
                error,
            });
        }
    }
    Ok(view(context, failures))
}

/// Bring skills an agent has into the library. The original is backed up,
/// copied in, and replaced by a link, so it stays on for that agent; one an
/// older version turned off joins the library off. A name the library has
/// with other contents is reported, not merged.
pub fn import_skills(
    context: &SkillContext,
    items: &[SkillImport],
) -> Result<SkillLibraryView, String> {
    let _guard = LOCK.lock().map_err(|_| "skills.errors.library")?;
    let mut failures = Vec::new();
    for item in items {
        let fail = |error: &str| SkillFailure {
            name: item.name.clone(),
            app: Some(item.app),
            error: error.to_owned(),
        };
        let name = match checked_name(&item.name) {
            Ok(name) => name,
            Err(error) => {
                failures.push(fail(&error));
                continue;
            }
        };
        let source = if item.legacy_disabled {
            context
                .legacy_disabled
                .iter()
                .find(|(app, _)| *app == item.app)
                .map(|(_, dir)| dir.join(name))
        } else {
            Some(context.target(item.app).dir.join(name))
        };
        let Some(source) = source.filter(|source| source.is_dir() && !link::is_link(source)) else {
            failures.push(fail("skills.errors.notFound"));
            continue;
        };
        let copy = context.copy_of(name);
        if copy.exists() {
            if !same_tree(&source, &copy) {
                failures.push(fail("skills.errors.importConflict"));
                continue;
            }
        } else if let Err(error) = copy_in(context, &source, name) {
            failures.push(fail(&error));
            continue;
        }
        let root = source.parent().unwrap_or(&source).to_path_buf();
        if let Err(error) = backup_skill_directory(&source, &root, &context.backups, "import") {
            log::warn!("backing up skill {name} before import failed: {error}");
            failures.push(fail("skills.errors.backup"));
            continue;
        }
        if let Err(error) = retire_directory(&source) {
            log::warn!("retiring imported skill {name} failed: {error}");
            failures.push(fail("skills.errors.copy"));
            continue;
        }
        if !item.legacy_disabled {
            if let Err(error) = switch(context, name, item.app, true) {
                // The original went to the backup; put the library's copy back in its place.
                let _ = copy_dir_all(&copy, &source);
                failures.push(fail(&error));
            }
        }
    }
    Ok(view(context, failures))
}

/// Take a skill out of every agent and out of the library, backed up first.
/// When an agent's link cannot be removed, the skill stays to be retried.
pub fn uninstall_skill(context: &SkillContext, name: &str) -> Result<SkillLibraryView, String> {
    let name = checked_name(name)?;
    let _guard = LOCK.lock().map_err(|_| "skills.errors.library")?;
    let copy = context.copy_of(name);
    if !copy.is_dir() {
        return Err("skills.errors.notFound".into());
    }
    let mut failures = Vec::new();
    for app in SkillApp::ALL {
        if let Err(error) = switch(context, name, app, false) {
            failures.push(SkillFailure {
                name: name.to_owned(),
                app: Some(app),
                error,
            });
        }
    }
    if failures.is_empty() {
        backup_skill_directory(&copy, &context.library, &context.backups, "remove")
            .map_err(|_| "skills.errors.backup")?;
        retire_directory(&copy).map_err(|_| "skills.errors.copy")?;
    }
    Ok(view(context, failures))
}

/// Where a backup lives: the library's backups, or a client's from before.
fn backup_root(context: &SkillContext, id: &str) -> Option<PathBuf> {
    std::iter::once(&context.backups)
        .chain(context.legacy_backups.iter().map(|(_, dir)| dir))
        .find(|dir| dir.join(id).join("metadata.json").is_file())
        .cloned()
}

/// Put a backed-up skill back into the library, off for every agent.
pub fn restore_skill(context: &SkillContext, id: &str) -> Result<SkillLibraryView, String> {
    let id = checked_name(id)?;
    let _guard = LOCK.lock().map_err(|_| "skills.errors.library")?;
    let root = backup_root(context, id).ok_or("skills.errors.notFound")?;
    let meta: SkillBackupMetadata = std::fs::read_to_string(root.join(id).join("metadata.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .ok_or("skills.errors.backupCorrupt")?;
    let name = Path::new(&meta.relative_path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or(meta.name);
    let name = checked_name(&name)?.to_owned();
    if std::fs::symlink_metadata(context.copy_of(&name)).is_ok() {
        return Err("skills.errors.duplicate".into());
    }
    copy_in(context, &root.join(id).join("skill"), &name)?;
    Ok(view(context, Vec::new()))
}

pub fn delete_backup(context: &SkillContext, id: &str) -> Result<SkillLibraryView, String> {
    let id = checked_name(id)?;
    let _guard = LOCK.lock().map_err(|_| "skills.errors.library")?;
    let root = backup_root(context, id).ok_or("skills.errors.notFound")?;
    retire_directory(&root.join(id)).map_err(|_| "skills.errors.copy")?;
    Ok(view(context, Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> (tempfile::TempDir, SkillContext, CodexPaths) {
        let temp = tempfile::tempdir().unwrap();
        let codex = CodexPaths::from_home(temp.path().join(".codex"));
        for dir in [
            codex.codex_home.clone(),
            codex.app_data_dir.clone(),
            temp.path().join(".claude"),
            temp.path().join(".cursor"),
        ] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let clients = ClientPaths::from_home(temp.path());
        let context = SkillContext::new(&codex, &clients);
        (temp, context, codex)
    }

    fn skill(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("SKILL.md"), body).unwrap();
        path
    }

    fn cell(view: &SkillLibraryView, name: &str, app: SkillApp) -> SkillCellState {
        view.skills
            .iter()
            .find(|skill| skill.info.name == name)
            .unwrap()
            .cells
            .iter()
            .find(|cell| cell.app == app)
            .unwrap()
            .state
    }

    #[test]
    fn front_matter_names_and_describes_a_skill() {
        let meta = front_matter("---\nname: pdf-tools\ndescription: >\n  Read and fill\n  PDF forms.\nlicense: \"MIT\"\n---\n# Heading\nBody");
        assert_eq!(meta["name"], "pdf-tools");
        assert_eq!(meta["description"], "Read and fill PDF forms.");
        assert_eq!(meta["license"], "MIT");
        assert!(front_matter("# No front matter").is_empty());
    }

    #[test]
    fn an_added_skill_is_linked_into_the_chosen_agents_and_switched_per_agent() {
        let (temp, context, codex) = context();
        let source = skill(
            &temp.path().join("downloads"),
            "pdf",
            "---\nname: PDF\ndescription: Fill forms\n---\n",
        );
        let view = add_skill(
            &context,
            &source.display().to_string(),
            &[SkillApp::Codex, SkillApp::ClaudeCode],
        )
        .unwrap();
        assert!(view.failures.is_empty(), "{:?}", view.failures);
        assert_eq!(view.skills[0].info.title.as_deref(), Some("PDF"));
        assert_eq!(
            view.skills[0].info.description.as_deref(),
            Some("Fill forms")
        );
        assert_eq!(cell(&view, "pdf", SkillApp::Codex), SkillCellState::On);
        assert_eq!(cell(&view, "pdf", SkillApp::Cursor), SkillCellState::Off);
        assert_eq!(
            cell(&view, "pdf", SkillApp::Pi),
            SkillCellState::NotInstalled
        );
        let linked = codex.skills_dir.join("pdf");
        assert!(link::is_link(&linked));
        assert!(linked.join("SKILL.md").is_file());

        // Off takes the link out; the library keeps its copy.
        let view = set_skills(&context, &["pdf".into()], Some(SkillApp::Codex), false).unwrap();
        assert_eq!(cell(&view, "pdf", SkillApp::Codex), SkillCellState::Off);
        assert!(!linked.exists());
        assert!(context.copy_of("pdf").join("SKILL.md").is_file());

        // A folder the agent already holds under the name is not replaced.
        skill(
            &temp.path().join(".cursor/skills"),
            "pdf",
            "# Someone else's",
        );
        let view = set_skills(&context, &["pdf".into()], Some(SkillApp::Cursor), true).unwrap();
        assert_eq!(view.failures[0].error, "skills.errors.occupied");
        assert_eq!(
            cell(&view, "pdf", SkillApp::Cursor),
            SkillCellState::Occupied
        );

        assert_eq!(
            add_skill(&context, &source.display().to_string(), &[]).unwrap_err(),
            "skills.errors.duplicate"
        );

        // Removing unlinks every agent, backs the copy up, and it can come back.
        let view = uninstall_skill(&context, "pdf").unwrap();
        assert!(view.skills.is_empty());
        assert!(!temp.path().join(".claude/skills/pdf").exists());
        assert!(
            temp.path().join(".cursor/skills/pdf/SKILL.md").is_file(),
            "the other folder stays"
        );
        let backup = view
            .backups
            .iter()
            .find(|backup| backup.name == "pdf")
            .unwrap()
            .id
            .clone();
        let view = restore_skill(&context, &backup).unwrap();
        assert_eq!(cell(&view, "pdf", SkillApp::Codex), SkillCellState::Off);
    }

    #[test]
    fn skills_an_agent_has_are_imported_and_stay_on_there() {
        let (temp, context, codex) = context();
        skill(&codex.skills_dir, "review", "# Review\nChecks diffs.");
        skill(
            &temp.path().join(".claude/skills"),
            "review",
            "# Review\nChecks diffs.",
        );
        skill(&temp.path().join(".claude/skills"), "notes", "# Notes");
        skill(&codex.app_data_dir.join("disabled-skills"), "old", "# Old");
        let elsewhere = skill(&temp.path().join("cc-switch"), "theirs", "# Theirs");
        link::create(&codex.skills_dir.join("theirs"), &elsewhere).unwrap();

        let view = load_skills(&context).unwrap();
        assert!(view.skills.is_empty());
        let names: Vec<_> = view
            .unmanaged
            .iter()
            .map(|skill| {
                (
                    skill.info.name.as_str(),
                    skill.app,
                    skill.legacy_disabled,
                    skill.external,
                )
            })
            .collect();
        assert!(names.contains(&("review", SkillApp::Codex, false, false)));
        assert!(names.contains(&("old", SkillApp::Codex, true, false)));
        assert!(names.contains(&("theirs", SkillApp::Codex, false, true)));

        let imports = [
            SkillImport {
                name: "review".into(),
                app: SkillApp::Codex,
                legacy_disabled: false,
            },
            SkillImport {
                name: "review".into(),
                app: SkillApp::ClaudeCode,
                legacy_disabled: false,
            },
            SkillImport {
                name: "old".into(),
                app: SkillApp::Codex,
                legacy_disabled: true,
            },
        ];
        let view = import_skills(&context, &imports).unwrap();
        assert!(view.failures.is_empty(), "{:?}", view.failures);
        assert_eq!(cell(&view, "review", SkillApp::Codex), SkillCellState::On);
        assert_eq!(
            cell(&view, "review", SkillApp::ClaudeCode),
            SkillCellState::On,
            "the same skill joins once"
        );
        assert_eq!(cell(&view, "old", SkillApp::Codex), SkillCellState::Off);
        assert!(!codex.app_data_dir.join("disabled-skills/old").exists());
        assert!(view.backups.iter().any(|backup| backup.name == "review"));
        // The other tool's link is still listed and untouched.
        assert!(view
            .unmanaged
            .iter()
            .any(|skill| skill.info.name == "theirs" && skill.external));
        assert!(elsewhere.join("SKILL.md").is_file());

        // Different contents under a library name are not merged.
        skill(
            &temp.path().join(".cursor/skills"),
            "review",
            "# Another review",
        );
        // The list says so first: only a copy with the library's contents is offered.
        skill(&temp.path().join(".cursor/skills"), "old", "# Old");
        let view = load_skills(&context).unwrap();
        let listed = |name: &str| {
            let skill = view
                .unmanaged
                .iter()
                .find(|skill| skill.info.name == name && skill.app == SkillApp::Cursor)
                .unwrap();
            (skill.in_library, skill.conflict)
        };
        assert_eq!(listed("review"), (true, true));
        assert_eq!(listed("old"), (true, false));
        let view = import_skills(
            &context,
            &[SkillImport {
                name: "review".into(),
                app: SkillApp::Cursor,
                legacy_disabled: false,
            }],
        )
        .unwrap();
        assert_eq!(view.failures[0].error, "skills.errors.importConflict");
        assert!(!link::is_link(&temp.path().join(".cursor/skills/review")));
    }

    #[test]
    fn imports_keep_nested_and_hidden_files_and_refuse_links_inside() {
        let (temp, context, codex) = context();
        let source = skill(&codex.skills_dir, "deep", "# Deep");
        std::fs::create_dir_all(source.join("scripts/.cache")).unwrap();
        std::fs::write(source.join("scripts/.cache/data"), "kept").unwrap();
        let view = import_skills(
            &context,
            &[SkillImport {
                name: "deep".into(),
                app: SkillApp::Codex,
                legacy_disabled: false,
            }],
        )
        .unwrap();
        assert!(view.failures.is_empty(), "{:?}", view.failures);
        assert_eq!(
            std::fs::read_to_string(context.copy_of("deep").join("scripts/.cache/data")).unwrap(),
            "kept"
        );
        assert_eq!(
            std::fs::read_to_string(source.join("scripts/.cache/data")).unwrap(),
            "kept",
            "read through the link"
        );

        // A link inside a skill could reach anywhere: it is not copied in.
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let linked = skill(&temp.path().join(".claude/skills"), "linked", "# Linked");
        link::create(&linked.join("escape"), &outside).unwrap();
        let view = import_skills(
            &context,
            &[SkillImport {
                name: "linked".into(),
                app: SkillApp::ClaudeCode,
                legacy_disabled: false,
            }],
        )
        .unwrap();
        assert_eq!(view.failures[0].error, "skills.errors.linkedInside");
        assert!(linked.join("SKILL.md").is_file(), "the original stays");
        assert!(!context.copy_of("linked").exists());
    }

    #[test]
    fn damaged_skills_and_backups_do_not_hide_the_healthy_ones() {
        let (_temp, context, codex) = context();
        let bad = codex.skills_dir.join("bad");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join("SKILL.md"), [0xd6, 0xd0]).unwrap();
        skill(&codex.skills_dir, "good", "# Good");
        std::fs::create_dir_all(context.backups.join("broken")).unwrap();
        std::fs::write(
            context.backups.join("broken/metadata.json"),
            "{\"backupID\":",
        )
        .unwrap();
        let view = load_skills(&context).unwrap();
        let names: Vec<_> = view
            .unmanaged
            .iter()
            .map(|skill| skill.info.name.as_str())
            .collect();
        assert_eq!(names, ["good"]);
        assert!(view.backups.is_empty());
        for id in ["../escape", "broken/../..", ""] {
            assert!(restore_skill(&context, id).is_err(), "{id}");
            assert!(delete_backup(&context, id).is_err(), "{id}");
        }
        assert!(
            context.backups.join("broken").exists(),
            "a damaged backup is kept for recovery"
        );
    }

    #[test]
    fn names_that_leave_the_folder_are_refused() {
        let (_temp, context, _) = context();
        for name in ["", "..", ".hidden", "a/b", "a\\b", "C:x"] {
            assert_eq!(
                set_skills(&context, &[name.into()], None, true).unwrap_err(),
                "skills.errors.name",
                "{name}"
            );
        }
    }
}
