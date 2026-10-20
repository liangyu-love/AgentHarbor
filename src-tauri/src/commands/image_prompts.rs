//! The image workbench's prompt library: groups of prompt templates the user keeps,
//! and beside it the materials (outfits, poses, angles) prompt polishing draws on.

use super::image::{paths, MAX_PROMPT_CHARS};
use crate::core::{repository::Repository, storage};
use crate::platform::paths::CodexPaths;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::Path, sync::Mutex};
use tauri::State;

const LIBRARY_FILE: &str = "image-prompt-library.json";
const MATERIALS_FILE: &str = "image-prompt-materials.json";
/// The materials a new install starts with. Their ids are fixed so that what the
/// polishing panel ticks still matches before the user first saves the materials.
const MATERIALS_SEED: &str = include_str!("image_materials_seed.json");
const LIBRARY_VERSION: u32 = 1;
const MAX_GROUPS: usize = 100;
const MAX_TEMPLATES: usize = 2000;
const MAX_GROUP_NAME_CHARS: usize = 40;
const MAX_TITLE_CHARS: usize = 80;
const MAX_NOTE_CHARS: usize = 200;
const MAX_ID_CHARS: usize = 64;
const MAX_IMPORT_BYTES: u64 = 16 * 1024 * 1024;
const UNNAMED_GROUP: &str = "未命名分组";
/// Characters of the prompt that title a template saved without one.
const DERIVED_TITLE_CHARS: usize = 16;

/// Read-modify-write of the library file (saves and imports) happens one at a time.
static LIBRARY_LOCK: Mutex<()> = Mutex::new(());

/// Which book of the 画谱: whole prompts to fill in, or materials to polish with.
/// Both share one shape: a material is a template, its category a group.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LibraryKind {
    #[default]
    Prompts,
    Materials,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptTemplate {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptGroup {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub templates: Vec<PromptTemplate>,
}

/// Array order is display order; pinned templates lead their group.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptLibrary {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub groups: Vec<PromptGroup>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptImportResult {
    pub library: PromptLibrary,
    pub groups: usize,
    pub templates: usize,
    pub skipped: usize,
}

/// An export file is the library itself or a bare list of its groups.
#[derive(Deserialize)]
#[serde(untagged)]
enum ImportFile {
    Library(PromptLibrary),
    Groups(Vec<PromptGroup>),
}

fn library_path(paths: &CodexPaths, kind: LibraryKind) -> std::path::PathBuf {
    paths.app_data_dir.join(match kind {
        LibraryKind::Prompts => LIBRARY_FILE,
        LibraryKind::Materials => MATERIALS_FILE,
    })
}

fn seed_groups(kind: LibraryKind) -> Vec<PromptGroup> {
    match kind {
        LibraryKind::Prompts => Vec::new(),
        LibraryKind::Materials => serde_json::from_str(MATERIALS_SEED).unwrap_or_default(),
    }
}

fn clipped(value: &str, limit: usize) -> String {
    value.trim().chars().take(limit).collect()
}

fn derived_title(prompt: &str) -> String {
    let line = prompt.trim().lines().next().unwrap_or_default();
    clipped(line, DERIVED_TITLE_CHARS)
}

fn fresh_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Keep an id that is usable and unseen; otherwise mint one.
fn claim_id(id: String, seen: &mut HashSet<String>) -> String {
    let id = if id.trim().is_empty() || id.chars().count() > MAX_ID_CHARS || seen.contains(&id) {
        fresh_id()
    } else {
        id
    };
    seen.insert(id.clone());
    id
}

/// Trim and bound every field, repair ids, put pinned templates first, and drop
/// what cannot be kept. Returns the clean groups and how many items were dropped.
fn sanitize(groups: Vec<PromptGroup>) -> (Vec<PromptGroup>, usize) {
    let mut seen = HashSet::new();
    let mut skipped = 0;
    let mut total = 0;
    let mut clean = Vec::with_capacity(groups.len().min(MAX_GROUPS));
    for group in groups {
        if clean.len() >= MAX_GROUPS {
            skipped += 1 + group.templates.len();
            continue;
        }
        let name = match clipped(&group.name, MAX_GROUP_NAME_CHARS) {
            name if name.is_empty() => UNNAMED_GROUP.to_owned(),
            name => name,
        };
        let mut templates = Vec::with_capacity(group.templates.len());
        for template in group.templates {
            let prompt = template.prompt.trim().to_owned();
            if prompt.is_empty()
                || prompt.chars().count() > MAX_PROMPT_CHARS
                || total >= MAX_TEMPLATES
            {
                skipped += 1;
                continue;
            }
            let title = match clipped(&template.title, MAX_TITLE_CHARS) {
                title if title.is_empty() => derived_title(&prompt),
                title => title,
            };
            templates.push(PromptTemplate {
                id: claim_id(template.id, &mut seen),
                title,
                note: clipped(&template.note, MAX_NOTE_CHARS),
                prompt,
                pinned: template.pinned,
            });
            total += 1;
        }
        templates.sort_by_key(|template| !template.pinned);
        clean.push(PromptGroup {
            id: claim_id(group.id, &mut seen),
            name,
            templates,
        });
    }
    (clean, skipped)
}

pub(crate) fn read_library(paths: &CodexPaths, kind: LibraryKind) -> Result<PromptLibrary, String> {
    let bytes = match std::fs::read(library_path(paths, kind)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // Never saved: the seed, until the first save writes the file.
            return Ok(PromptLibrary {
                version: LIBRARY_VERSION,
                groups: sanitize(seed_groups(kind)).0,
            });
        }
        Err(_) => return Err("image.errors.promptLibrary".into()),
    };
    if bytes.is_empty() {
        return Ok(PromptLibrary {
            version: LIBRARY_VERSION,
            groups: Vec::new(),
        });
    }
    let stored: PromptLibrary =
        serde_json::from_slice(&bytes).map_err(|_| "image.errors.promptLibrary".to_owned())?;
    // A hand-edited file is repaired on read; the repair is written on the next save.
    let (groups, _) = sanitize(stored.groups);
    Ok(PromptLibrary {
        version: LIBRARY_VERSION,
        groups,
    })
}

fn write_library(
    paths: &CodexPaths,
    kind: LibraryKind,
    groups: Vec<PromptGroup>,
) -> Result<PromptLibrary, String> {
    let library = PromptLibrary {
        version: LIBRARY_VERSION,
        groups,
    };
    let contents =
        serde_json::to_vec_pretty(&library).map_err(|_| "image.errors.promptLibrary".to_owned())?;
    storage::write_atomic(&library_path(paths, kind), &contents)
        .map_err(|_| "image.errors.promptLibrary".to_owned())?;
    Ok(library)
}

/// Save the whole library as the UI holds it. Nothing is dropped silently: a
/// library that does not fit the limits is refused instead of trimmed.
fn save_library(
    paths: &CodexPaths,
    kind: LibraryKind,
    library: PromptLibrary,
) -> Result<PromptLibrary, String> {
    let (groups, skipped) = sanitize(library.groups);
    if skipped > 0 {
        return Err("image.errors.promptInvalid".into());
    }
    let _guard = LIBRARY_LOCK
        .lock()
        .map_err(|_| "image.errors.promptLibrary".to_owned())?;
    write_library(paths, kind, groups)
}

/// Merge imported groups into the library: same-named groups are joined, a
/// template already in its group (same title and prompt) is skipped, and the
/// limits stop the import rather than evict what the user already has.
fn merge(
    mut groups: Vec<PromptGroup>,
    incoming: Vec<PromptGroup>,
) -> (Vec<PromptGroup>, PromptImportResult) {
    let (incoming, mut skipped) = sanitize(incoming);
    let mut total: usize = groups.iter().map(|group| group.templates.len()).sum();
    let mut new_groups = 0;
    let mut imported = 0;
    for group in incoming {
        let index = match groups
            .iter()
            .position(|existing| existing.name == group.name)
        {
            Some(index) => index,
            None if groups.len() < MAX_GROUPS => {
                groups.push(PromptGroup {
                    id: fresh_id(),
                    name: group.name,
                    templates: Vec::new(),
                });
                new_groups += 1;
                groups.len() - 1
            }
            None => {
                skipped += group.templates.len();
                continue;
            }
        };
        let target = &mut groups[index];
        for template in group.templates {
            let duplicate = target.templates.iter().any(|existing| {
                existing.title == template.title && existing.prompt == template.prompt
            });
            if duplicate || total >= MAX_TEMPLATES {
                skipped += 1;
                continue;
            }
            target.templates.push(PromptTemplate {
                id: fresh_id(),
                ..template
            });
            total += 1;
            imported += 1;
        }
    }
    let (groups, _) = sanitize(groups);
    let result = PromptImportResult {
        library: PromptLibrary::default(),
        groups: new_groups,
        templates: imported,
        skipped,
    };
    (groups, result)
}

fn import_library(
    paths: &CodexPaths,
    kind: LibraryKind,
    source: &Path,
) -> Result<PromptImportResult, String> {
    let size = std::fs::metadata(source)
        .map_err(|_| "image.errors.promptImport".to_owned())?
        .len();
    if size > MAX_IMPORT_BYTES {
        return Err("image.errors.promptImport".into());
    }
    let bytes = std::fs::read(source).map_err(|_| "image.errors.promptImport".to_owned())?;
    let incoming = match serde_json::from_slice::<ImportFile>(&bytes) {
        Ok(ImportFile::Library(library)) => library.groups,
        Ok(ImportFile::Groups(groups)) => groups,
        Err(_) => return Err("image.errors.promptImport".into()),
    };
    if incoming.is_empty() {
        return Err("image.errors.promptImport".into());
    }
    let _guard = LIBRARY_LOCK
        .lock()
        .map_err(|_| "image.errors.promptLibrary".to_owned())?;
    let current = read_library(paths, kind)?;
    let (groups, mut result) = merge(current.groups, incoming);
    result.library = write_library(paths, kind, groups)?;
    Ok(result)
}

fn export_target(path: &str) -> Result<std::path::PathBuf, String> {
    let target = std::path::PathBuf::from(path.trim());
    let is_json = target
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("json"));
    let escapes = target
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir));
    if path.contains('\0') || !target.is_absolute() || !is_json || escapes {
        return Err("image.errors.promptExport".into());
    }
    Ok(target)
}

fn export_library(paths: &CodexPaths, kind: LibraryKind, path: &str) -> Result<(), String> {
    let target = export_target(path)?;
    let library = read_library(paths, kind)?;
    let contents =
        serde_json::to_vec_pretty(&library).map_err(|_| "image.errors.promptExport".to_owned())?;
    storage::write_atomic(&target, &contents).map_err(|_| "image.errors.promptExport".to_owned())
}

#[tauri::command]
pub async fn load_image_prompt_library(
    repo: State<'_, Mutex<Repository>>,
    kind: Option<LibraryKind>,
) -> Result<PromptLibrary, String> {
    let paths = paths(&repo)?;
    super::blocking(move || read_library(&paths, kind.unwrap_or_default())).await
}

#[tauri::command]
pub async fn save_image_prompt_library(
    repo: State<'_, Mutex<Repository>>,
    library: PromptLibrary,
    kind: Option<LibraryKind>,
) -> Result<PromptLibrary, String> {
    let paths = paths(&repo)?;
    super::blocking(move || save_library(&paths, kind.unwrap_or_default(), library)).await
}

#[tauri::command]
pub async fn import_image_prompt_library(
    repo: State<'_, Mutex<Repository>>,
    path: String,
    kind: Option<LibraryKind>,
) -> Result<PromptImportResult, String> {
    let paths = paths(&repo)?;
    let kind = kind.unwrap_or_default();
    super::blocking(move || import_library(&paths, kind, Path::new(&path))).await
}

#[tauri::command]
pub async fn export_image_prompt_library(
    repo: State<'_, Mutex<Repository>>,
    path: String,
    kind: Option<LibraryKind>,
) -> Result<(), String> {
    let paths = paths(&repo)?;
    super::blocking(move || export_library(&paths, kind.unwrap_or_default(), &path)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: LibraryKind = LibraryKind::Prompts;

    fn template(id: &str, title: &str, prompt: &str, pinned: bool) -> PromptTemplate {
        PromptTemplate {
            id: id.into(),
            title: title.into(),
            note: String::new(),
            prompt: prompt.into(),
            pinned,
        }
    }

    fn group(id: &str, name: &str, templates: Vec<PromptTemplate>) -> PromptGroup {
        PromptGroup {
            id: id.into(),
            name: name.into(),
            templates,
        }
    }

    fn temp_paths() -> (tempfile::TempDir, CodexPaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join("codex"));
        (dir, paths)
    }

    #[test]
    fn a_missing_library_reads_as_empty_and_saves_round_trip() {
        let (_dir, paths) = temp_paths();
        assert!(read_library(&paths, P).unwrap().groups.is_empty());

        let library = PromptLibrary {
            version: 0,
            groups: vec![group(
                "g1",
                " 人像 ",
                vec![template("t1", "侧光", "  窗边侧光人像  ", false)],
            )],
        };
        let saved = save_library(&paths, P, library).unwrap();
        assert_eq!(saved.version, LIBRARY_VERSION);
        assert_eq!(saved.groups[0].name, "人像");
        assert_eq!(saved.groups[0].templates[0].prompt, "窗边侧光人像");
        assert_eq!(read_library(&paths, P).unwrap(), saved);
    }

    #[test]
    fn materials_start_from_the_seed_and_keep_their_own_file() {
        let (_dir, paths) = temp_paths();
        let seeded: Vec<PromptGroup> = serde_json::from_str(MATERIALS_SEED).unwrap();
        let (clean, skipped) = sanitize(seeded.clone());
        assert_eq!(skipped, 0);
        assert_eq!(clean, seeded, "the seed is already clean, so its ids hold");
        let first = read_library(&paths, LibraryKind::Materials).unwrap();
        let names: Vec<_> = first
            .groups
            .iter()
            .map(|group| group.name.as_str())
            .collect();
        assert_eq!(names[..3], ["服装", "动作", "视角"]);
        assert_eq!(read_library(&paths, LibraryKind::Materials).unwrap(), first);
        assert!(read_library(&paths, P).unwrap().groups.is_empty());

        // Emptied by the user, the materials stay empty.
        save_library(&paths, LibraryKind::Materials, PromptLibrary::default()).unwrap();
        assert!(read_library(&paths, LibraryKind::Materials)
            .unwrap()
            .groups
            .is_empty());
        assert!(!library_path(&paths, P).exists());
    }

    #[test]
    fn sanitize_repairs_ids_titles_and_pinned_order() {
        let (groups, skipped) = sanitize(vec![group(
            "g1",
            "",
            vec![
                template(
                    "same",
                    "",
                    "第一行很长很长很长很长很长很长很长的提示词\n第二行",
                    false,
                ),
                template("same", "置顶", "置顶提示词", true),
                template("t3", "空", "   ", false),
            ],
        )]);
        assert_eq!(skipped, 1);
        let group = &groups[0];
        assert_eq!(group.name, UNNAMED_GROUP);
        assert_eq!(group.templates[0].title, "置顶");
        assert!(group.templates[0].pinned);
        assert_eq!(
            group.templates[1].title.chars().count(),
            DERIVED_TITLE_CHARS
        );
        assert_ne!(group.templates[0].id, group.templates[1].id);
    }

    #[test]
    fn a_library_over_the_limits_is_refused_not_trimmed() {
        let (_dir, paths) = temp_paths();
        let groups = (0..=MAX_GROUPS)
            .map(|index| group(&format!("g{index}"), "组", Vec::new()))
            .collect();
        assert_eq!(
            save_library(&paths, P, PromptLibrary { version: 1, groups }).unwrap_err(),
            "image.errors.promptInvalid"
        );
        assert!(!library_path(&paths, P).exists());
    }

    #[test]
    fn import_joins_groups_by_name_and_skips_duplicates() {
        let (dir, paths) = temp_paths();
        save_library(
            &paths,
            P,
            PromptLibrary {
                version: 1,
                groups: vec![group(
                    "g1",
                    "海报",
                    vec![template("t1", "复古", "复古海报", false)],
                )],
            },
        )
        .unwrap();
        let source = dir.path().join("prompts.json");
        std::fs::write(
            &source,
            serde_json::json!([
                { "name": "海报", "templates": [
                    { "title": "复古", "prompt": "复古海报" },
                    { "title": "极简", "prompt": "极简海报", "pinned": true }
                ]},
                { "name": "人像", "templates": [{ "prompt": "胶片人像" }] }
            ])
            .to_string(),
        )
        .unwrap();

        let result = import_library(&paths, P, &source).unwrap();
        assert_eq!((result.groups, result.templates, result.skipped), (1, 2, 1));
        let posters = &result.library.groups[0];
        assert_eq!(posters.id, "g1");
        assert_eq!(posters.templates[0].title, "极简");
        assert_eq!(posters.templates[1].id, "t1");
        assert_eq!(result.library.groups[1].templates[0].title, "胶片人像");
        assert_eq!(read_library(&paths, P).unwrap(), result.library);

        std::fs::write(&source, b"not json").unwrap();
        assert_eq!(
            import_library(&paths, P, &source).unwrap_err(),
            "image.errors.promptImport"
        );
    }

    #[test]
    fn export_writes_only_absolute_json_paths() {
        let (dir, paths) = temp_paths();
        save_library(
            &paths,
            P,
            PromptLibrary {
                version: 1,
                groups: vec![group("g1", "海报", Vec::new())],
            },
        )
        .unwrap();
        let target = dir.path().join("out.json");
        export_library(&paths, P, target.to_str().unwrap()).unwrap();
        let exported: PromptLibrary =
            serde_json::from_slice(&std::fs::read(&target).unwrap()).unwrap();
        assert_eq!(exported.groups[0].name, "海报");

        for bad in [
            "relative.json",
            "",
            dir.path().join("out.txt").to_str().unwrap(),
        ] {
            assert_eq!(
                export_library(&paths, P, bad).unwrap_err(),
                "image.errors.promptExport"
            );
        }
    }
}
