//! The image workbench's image materials: pictures the user keeps in groups to use
//! as image-to-image references, each with a name and note the polishing model reads.
//! The pictures are copied into one folder beside an index of the groups: the app data
//! folder unless the user picks another, so the folder alone holds the whole library.

use super::image::{
    decode_data_url, gallery_dir, paths, same_folder, thumbnail_data_url, MAX_IMAGE_BYTES,
    THUMBNAIL_EDGE, THUMBNAIL_JOBS,
};
use crate::core::{repository::Repository, storage};
use crate::platform::paths::CodexPaths;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::State;

const DIRECTORY: &str = "image-materials";
/// The folder the user picked, kept in the app data folder as a JSON string.
const DIRECTORY_SETTING: &str = "image-materials-directory.json";
const INDEX_FILE: &str = "library.json";
const LIBRARY_VERSION: u32 = 1;
const MAX_GROUPS: usize = 100;
const MAX_ITEMS: usize = 1000;
/// Images added in one go; a larger pick is refused rather than half added.
const MAX_ADD: usize = 50;
const MAX_GROUP_NAME_CHARS: usize = 40;
const MAX_NAME_CHARS: usize = 80;
const MAX_NOTE_CHARS: usize = 200;
const MAX_ID_CHARS: usize = 64;
const UNNAMED_GROUP: &str = "未命名分组";
const UNNAMED_ITEM: &str = "未命名图片";
const EXTENSIONS: [&str; 4] = ["png", "jpg", "webp", "gif"];

/// The index and the files beside it change one at a time.
static LIBRARY_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageMaterial {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub note: String,
    /// The picture's file name in the materials folder; set by the backend only.
    #[serde(default)]
    pub file: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageMaterialGroup {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub items: Vec<ImageMaterial>,
}

/// Array order is display order.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageMaterialLibrary {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub groups: Vec<ImageMaterialGroup>,
    /// Where the pictures are kept, for the user to open; ignored when saving.
    #[serde(default)]
    pub directory: String,
}

/// One picture to add: what to call it, and the image as a data URL.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewImageMaterial {
    #[serde(default)]
    pub name: String,
    pub data_url: String,
}

pub(super) fn directory(paths: &CodexPaths) -> PathBuf {
    std::fs::read(paths.app_data_dir.join(DIRECTORY_SETTING))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<String>(&bytes).ok())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| paths.app_data_dir.join(DIRECTORY))
}

fn clipped(value: &str, limit: usize) -> String {
    value.trim().chars().take(limit).collect()
}

fn usable_id(id: &str) -> bool {
    !id.trim().is_empty() && id.chars().count() <= MAX_ID_CHARS
}

/// A file name the backend wrote: `<id>.<ext>`, nothing that could leave the folder.
fn valid_file(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(stem, extension)| {
        !stem.is_empty()
            && stem.len() <= MAX_ID_CHARS
            && stem
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            && EXTENSIONS.contains(&extension)
    })
}

fn with_directory(paths: &CodexPaths, groups: Vec<ImageMaterialGroup>) -> ImageMaterialLibrary {
    ImageMaterialLibrary {
        version: LIBRARY_VERSION,
        groups,
        directory: directory(paths).to_string_lossy().into_owned(),
    }
}

/// Trim and bound names, and drop what cannot be kept: duplicate ids, items past
/// the limits, and items whose file is not one the backend could have written.
/// Returns the clean groups and how many items were dropped.
fn sanitize(groups: Vec<ImageMaterialGroup>) -> (Vec<ImageMaterialGroup>, usize) {
    let mut seen = HashSet::new();
    let mut skipped = 0;
    let mut total = 0;
    let mut clean = Vec::with_capacity(groups.len().min(MAX_GROUPS));
    for group in groups {
        if clean.len() >= MAX_GROUPS || !usable_id(&group.id) || !seen.insert(group.id.clone()) {
            skipped += group.items.len().max(1);
            continue;
        }
        let mut items = Vec::with_capacity(group.items.len());
        for item in group.items {
            if total >= MAX_ITEMS
                || !usable_id(&item.id)
                || !valid_file(&item.file)
                || !seen.insert(item.id.clone())
            {
                skipped += 1;
                continue;
            }
            items.push(ImageMaterial {
                name: match clipped(&item.name, MAX_NAME_CHARS) {
                    name if name.is_empty() => UNNAMED_ITEM.to_owned(),
                    name => name,
                },
                note: clipped(&item.note, MAX_NOTE_CHARS),
                ..item
            });
            total += 1;
        }
        clean.push(ImageMaterialGroup {
            name: match clipped(&group.name, MAX_GROUP_NAME_CHARS) {
                name if name.is_empty() => UNNAMED_GROUP.to_owned(),
                name => name,
            },
            items,
            ..group
        });
    }
    (clean, skipped)
}

fn read_groups(paths: &CodexPaths) -> Result<Vec<ImageMaterialGroup>, String> {
    read_index(&directory(paths))
}

fn read_index(folder: &Path) -> Result<Vec<ImageMaterialGroup>, String> {
    let bytes = match std::fs::read(folder.join(INDEX_FILE)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err("image.errors.imageMaterials".into()),
    };
    let stored: ImageMaterialLibrary =
        serde_json::from_slice(&bytes).map_err(|_| "image.errors.imageMaterials".to_owned())?;
    Ok(sanitize(stored.groups).0)
}

fn write_groups(paths: &CodexPaths, groups: &[ImageMaterialGroup]) -> Result<(), String> {
    write_index(&directory(paths), groups)
}

fn write_index(folder: &Path, groups: &[ImageMaterialGroup]) -> Result<(), String> {
    let index = ImageMaterialLibrary {
        version: LIBRARY_VERSION,
        groups: groups.to_vec(),
        directory: String::new(),
    };
    let contents =
        serde_json::to_vec_pretty(&index).map_err(|_| "image.errors.imageMaterials".to_owned())?;
    storage::write_atomic(&folder.join(INDEX_FILE), &contents)
        .map_err(|_| "image.errors.imageMaterials".to_owned())
}

fn load(paths: &CodexPaths) -> Result<ImageMaterialLibrary, String> {
    Ok(with_directory(paths, read_groups(paths)?))
}

/// Save names, notes, order and groups as the UI holds them. Pictures cannot be
/// created here, only kept, moved or removed: an item is matched to the saved one
/// by id and keeps that one's file, and the files of removed items are deleted.
fn save(paths: &CodexPaths, library: ImageMaterialLibrary) -> Result<ImageMaterialLibrary, String> {
    let _guard = LIBRARY_LOCK
        .lock()
        .map_err(|_| "image.errors.imageMaterials".to_owned())?;
    let current = read_groups(paths)?;
    let files: HashMap<&str, &str> = current
        .iter()
        .flat_map(|group| &group.items)
        .map(|item| (item.id.as_str(), item.file.as_str()))
        .collect();
    let mut incoming = library.groups;
    for item in incoming.iter_mut().flat_map(|group| &mut group.items) {
        item.file = files
            .get(item.id.as_str())
            .ok_or("image.errors.imageMaterialsInvalid")?
            .to_string();
    }
    let (groups, skipped) = sanitize(incoming);
    if skipped > 0 {
        return Err("image.errors.imageMaterialsInvalid".into());
    }
    write_groups(paths, &groups)?;
    let kept: HashSet<&str> = groups
        .iter()
        .flat_map(|group| &group.items)
        .map(|item| item.file.as_str())
        .collect();
    for file in files.values().filter(|file| !kept.contains(*file)) {
        // A file left behind only takes space; the index no longer names it.
        if let Err(error) = std::fs::remove_file(directory(paths).join(file)) {
            log::warn!("image material {file} not deleted: {error}");
        }
    }
    Ok(with_directory(paths, groups))
}

/// Copy pictures into a group, each validated as an image first; all or none are added.
fn add(
    paths: &CodexPaths,
    group_id: &str,
    images: Vec<NewImageMaterial>,
) -> Result<ImageMaterialLibrary, String> {
    if images.is_empty() || images.len() > MAX_ADD {
        return Err("image.errors.imageMaterialsAdd".into());
    }
    let decoded = images
        .iter()
        .map(|image| decode_data_url(&image.data_url).map_err(|_| "image.errors.input".to_owned()))
        .collect::<Result<Vec<_>, _>>()?;
    let _guard = LIBRARY_LOCK
        .lock()
        .map_err(|_| "image.errors.imageMaterials".to_owned())?;
    let mut groups = read_groups(paths)?;
    let total: usize = groups.iter().map(|group| group.items.len()).sum();
    if total + images.len() > MAX_ITEMS {
        return Err("image.errors.imageMaterialsFull".into());
    }
    let group = groups
        .iter_mut()
        .find(|group| group.id == group_id)
        .ok_or("image.errors.imageMaterialsInvalid")?;
    let folder = directory(paths);
    std::fs::create_dir_all(&folder).map_err(|_| "image.errors.imageMaterials".to_owned())?;
    let mut written = Vec::with_capacity(images.len());
    for (image, (bytes, extension)) in images.iter().zip(decoded) {
        let id = uuid::Uuid::new_v4().to_string();
        let file = format!("{id}.{extension}");
        if std::fs::write(folder.join(&file), bytes).is_err() {
            for file in &written {
                let _ = std::fs::remove_file(folder.join(file));
            }
            return Err("image.errors.imageMaterials".into());
        }
        written.push(file.clone());
        group.items.push(ImageMaterial {
            id,
            name: clipped(&image.name, MAX_NAME_CHARS),
            note: String::new(),
            file,
        });
    }
    let (groups, _) = sanitize(groups);
    if let Err(error) = write_groups(paths, &groups) {
        for file in &written {
            let _ = std::fs::remove_file(folder.join(file));
        }
        return Err(error);
    }
    Ok(with_directory(paths, groups))
}

/// What a folder import added and what it left out.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderImport {
    pub library: ImageMaterialLibrary,
    pub added: usize,
    /// Groups made for subfolders that had no same-named group yet.
    pub groups_created: usize,
    /// Pictures whose name the group already has, as when a folder is imported again.
    pub duplicates: usize,
    /// Files that are not a picture we keep, too large, unreadable, or past the limits.
    pub skipped: usize,
}

/// How deep a subfolder's own subfolders are read; deeper ones are left out.
const MAX_FOLDER_DEPTH: usize = 8;

/// The kept extension of a picture, judged by its bytes rather than its name.
fn picture_extension(bytes: &[u8]) -> Option<&'static str> {
    match image::guess_format(bytes).ok()? {
        image::ImageFormat::Png => Some("png"),
        image::ImageFormat::Jpeg => Some("jpg"),
        image::ImageFormat::WebP => Some("webp"),
        image::ImageFormat::Gif => Some("gif"),
        _ => None,
    }
}

fn looks_like_picture(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "webp" | "gif"
            )
        })
}

/// A folder's picture files and subfolders, each by name. Links are neither, so
/// a folder cannot lead the import outside itself.
fn folder_entries(folder: &Path) -> Option<(Vec<PathBuf>, Vec<PathBuf>)> {
    let mut entries: Vec<_> = std::fs::read_dir(folder).ok()?.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());
    let (mut files, mut folders) = (Vec::new(), Vec::new());
    for entry in entries {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_file() && looks_like_picture(&path) {
            files.push(path);
        } else if kind.is_dir() {
            folders.push(path);
        }
    }
    Some((files, folders))
}

/// The pictures in `folder`, then those of its subfolders down to `deep` levels.
fn picture_files(folder: &Path, deep: usize, materials: &Path) -> Vec<PathBuf> {
    let Some((mut files, folders)) = folder_entries(folder) else {
        return Vec::new();
    };
    if deep > 0 {
        for path in folders.iter().filter(|path| !same_folder(path, materials)) {
            files.extend(picture_files(path, deep - 1, materials));
        }
    }
    files
}

/// Copy the pictures of a folder in: those at its top level into `group_id`, and each
/// subfolder's (its own subfolders included) into the group of that name, made when
/// missing. A name the group already has is skipped, so importing again adds only
/// what is new. Whatever fits is added; the rest is counted as skipped.
fn import_folder(
    paths: &CodexPaths,
    group_id: &str,
    source: &Path,
) -> Result<FolderImport, String> {
    let source = source
        .canonicalize()
        .ok()
        .filter(|source| source.is_dir())
        .ok_or("image.errors.imageMaterials")?;
    let _guard = LIBRARY_LOCK
        .lock()
        .map_err(|_| "image.errors.imageMaterials".to_owned())?;
    let folder = directory(paths);
    if same_folder(&source, &folder) {
        return Err("image.errors.imageMaterialsInvalid".into());
    }
    let mut groups = read_groups(paths)?;
    if !groups.iter().any(|group| group.id == group_id) {
        return Err("image.errors.imageMaterialsInvalid".into());
    }
    // (group name, its files); the first entry is the chosen group.
    let (top, subfolders) = folder_entries(&source).ok_or("image.errors.imageMaterials")?;
    let mut batches = vec![(None, top)];
    for path in subfolders.iter().filter(|path| !same_folder(path, &folder)) {
        let files = picture_files(path, MAX_FOLDER_DEPTH, &folder);
        if !files.is_empty() {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            batches.push((Some(clipped(&name, MAX_GROUP_NAME_CHARS)), files));
        }
    }

    std::fs::create_dir_all(&folder).map_err(|_| "image.errors.imageMaterials".to_owned())?;
    let mut total: usize = groups.iter().map(|group| group.items.len()).sum();
    let mut written = Vec::new();
    let (mut groups_created, mut duplicates, mut skipped) = (0, 0, 0);
    for (name, files) in batches {
        let index = match &name {
            None => groups.iter().position(|group| group.id == group_id),
            Some(name) => groups.iter().position(|group| &group.name == name),
        };
        let index = match index {
            Some(index) => index,
            None if groups.len() < MAX_GROUPS => {
                groups.push(ImageMaterialGroup {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: name.clone().unwrap_or_default(),
                    items: Vec::new(),
                });
                groups_created += 1;
                groups.len() - 1
            }
            None => {
                skipped += files.len();
                continue;
            }
        };
        for path in files {
            let name = clipped(
                &path.file_stem().unwrap_or_default().to_string_lossy(),
                MAX_NAME_CHARS,
            );
            if groups[index].items.iter().any(|item| item.name == name) {
                duplicates += 1;
                continue;
            }
            let picture = std::fs::metadata(&path)
                .ok()
                .filter(|meta| meta.len() <= MAX_IMAGE_BYTES as u64)
                .and_then(|_| std::fs::read(&path).ok())
                .and_then(|bytes| picture_extension(&bytes).map(|extension| (bytes, extension)));
            let Some((bytes, extension)) = picture.filter(|_| total < MAX_ITEMS) else {
                skipped += 1;
                continue;
            };
            let id = uuid::Uuid::new_v4().to_string();
            let file = format!("{id}.{extension}");
            if std::fs::write(folder.join(&file), bytes).is_err() {
                skipped += 1;
                continue;
            }
            written.push(file.clone());
            groups[index].items.push(ImageMaterial {
                id,
                name,
                note: String::new(),
                file,
            });
            total += 1;
        }
    }
    let (groups, _) = sanitize(groups);
    if let Err(error) = write_groups(paths, &groups) {
        for file in &written {
            let _ = std::fs::remove_file(folder.join(file));
        }
        return Err(error);
    }
    Ok(FolderImport {
        library: with_directory(paths, groups),
        added: written.len(),
        groups_created,
        duplicates,
        skipped,
    })
}

/// Keep the materials in `target` from now on. A folder that already holds a library
/// is switched to as it is; otherwise the current pictures and index move there,
/// copied first and removed from the old folder only once the new one is in use.
/// The gallery's folder is refused unless `allow_shared`: the gallery would list the
/// pictures, and deleting one there deletes the material.
fn relocate(
    paths: &CodexPaths,
    target: &Path,
    allow_shared: bool,
) -> Result<ImageMaterialLibrary, String> {
    let _guard = LIBRARY_LOCK
        .lock()
        .map_err(|_| "image.errors.imageMaterials".to_owned())?;
    let target = target
        .canonicalize()
        .ok()
        .filter(|target| target.is_dir())
        .ok_or("image.errors.imageMaterials")?;
    let current = directory(paths);
    if current
        .canonicalize()
        .is_ok_and(|current| current == target)
    {
        return load(paths);
    }
    if !allow_shared && same_folder(&gallery_dir(paths), &target) {
        return Err("image.errors.imageMaterialsShared".into());
    }
    let moving = !target.join(INDEX_FILE).exists();
    let groups = if moving {
        read_groups(paths)?
    } else {
        read_index(&target)?
    };
    let files: Vec<&str> = groups
        .iter()
        .flat_map(|group| &group.items)
        .map(|item| item.file.as_str())
        .collect();
    let undo = |copied: &[&str]| {
        for file in copied.iter().chain([&INDEX_FILE]) {
            let _ = std::fs::remove_file(target.join(file));
        }
    };
    let mut copied = Vec::new();
    if moving {
        for &file in &files {
            match std::fs::copy(current.join(file), target.join(file)) {
                Ok(_) => copied.push(file),
                // A picture already gone from the old folder is no more lost than before.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => {
                    undo(&copied);
                    return Err("image.errors.imageMaterials".into());
                }
            }
        }
        if let Err(error) = write_index(&target, &groups) {
            undo(&copied);
            return Err(error);
        }
    }
    let setting = serde_json::to_vec(&target.to_string_lossy())
        .map_err(|_| "image.errors.imageMaterials".to_owned())?;
    if storage::write_atomic(&paths.app_data_dir.join(DIRECTORY_SETTING), &setting).is_err() {
        if moving {
            undo(&copied);
        }
        return Err("image.errors.imageMaterials".into());
    }
    if moving {
        for file in files.iter().chain([&INDEX_FILE]) {
            // The new folder is in use; a copy left behind only takes space.
            match std::fs::remove_file(current.join(file)) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    log::warn!("old image material {file} not deleted: {error}");
                }
                _ => {}
            }
        }
    }
    Ok(with_directory(paths, groups))
}

/// The picture's bytes and media type, found by item id in the index.
fn read_picture(paths: &CodexPaths, id: &str) -> Result<(Vec<u8>, &'static str), String> {
    let file = read_groups(paths)?
        .into_iter()
        .flat_map(|group| group.items)
        .find(|item| item.id == id)
        .map(|item| item.file)
        .ok_or("image.errors.notFound")?;
    let path = directory(paths).join(&file);
    let size = std::fs::metadata(&path)
        .map_err(|_| "image.errors.notFound")?
        .len();
    if size > MAX_IMAGE_BYTES as u64 {
        return Err("image.errors.inputTooLarge".into());
    }
    let bytes = std::fs::read(&path).map_err(|_| "image.errors.notFound".to_owned())?;
    let mime = match file.rsplit_once('.').map(|(_, extension)| extension) {
        Some("jpg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        _ => "image/png",
    };
    Ok((bytes, mime))
}

#[tauri::command]
pub async fn load_image_materials(
    repo: State<'_, Mutex<Repository>>,
) -> Result<ImageMaterialLibrary, String> {
    let paths = paths(&repo)?;
    super::blocking(move || load(&paths)).await
}

#[tauri::command]
pub async fn save_image_materials(
    repo: State<'_, Mutex<Repository>>,
    library: ImageMaterialLibrary,
) -> Result<ImageMaterialLibrary, String> {
    let paths = paths(&repo)?;
    super::blocking(move || save(&paths, library)).await
}

#[tauri::command]
pub async fn add_image_materials(
    repo: State<'_, Mutex<Repository>>,
    group_id: String,
    images: Vec<NewImageMaterial>,
) -> Result<ImageMaterialLibrary, String> {
    let paths = paths(&repo)?;
    super::blocking(move || add(&paths, &group_id, images)).await
}

/// Copy a folder's pictures in, its subfolders into groups of their names.
#[tauri::command]
pub async fn import_image_materials_folder(
    repo: State<'_, Mutex<Repository>>,
    group_id: String,
    directory: String,
) -> Result<FolderImport, String> {
    let paths = paths(&repo)?;
    super::blocking(move || import_folder(&paths, &group_id, Path::new(&directory))).await
}

/// Keep the materials in another folder, moving them there unless it holds a library.
#[tauri::command]
pub async fn set_image_materials_directory(
    repo: State<'_, Mutex<Repository>>,
    directory: String,
    allow_shared: Option<bool>,
) -> Result<ImageMaterialLibrary, String> {
    let paths = paths(&repo)?;
    let allow_shared = allow_shared.unwrap_or(false);
    super::blocking(move || relocate(&paths, Path::new(&directory), allow_shared)).await
}

/// A picture as a data URL: the original, or a copy no larger than `edge` for the grid.
#[tauri::command]
pub async fn load_image_material(
    repo: State<'_, Mutex<Repository>>,
    id: String,
    edge: Option<u32>,
) -> Result<String, String> {
    let paths = paths(&repo)?;
    let Some(edge) = edge else {
        return super::blocking(move || {
            let (bytes, mime) = read_picture(&paths, &id)?;
            Ok(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
        })
        .await;
    };
    let permit = THUMBNAIL_JOBS
        .acquire()
        .await
        .map_err(|_| "image.errors.response")?;
    super::blocking(move || {
        let _permit = permit;
        let (bytes, _) = read_picture(&paths, &id)?;
        thumbnail_data_url(&bytes, edge.min(THUMBNAIL_EDGE))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_paths() -> (tempfile::TempDir, CodexPaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join("codex"));
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        (dir, paths)
    }

    fn png() -> String {
        let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            3,
            2,
            image::Rgb([200, 40, 40]),
        ));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        format!(
            "data:image/png;base64,{}",
            STANDARD.encode(bytes.into_inner())
        )
    }

    fn group(id: &str, name: &str) -> ImageMaterialGroup {
        ImageMaterialGroup {
            id: id.into(),
            name: name.into(),
            items: Vec::new(),
        }
    }

    fn new_image(name: &str, data_url: String) -> NewImageMaterial {
        NewImageMaterial {
            name: name.into(),
            data_url,
        }
    }

    fn library(groups: Vec<ImageMaterialGroup>) -> ImageMaterialLibrary {
        ImageMaterialLibrary {
            version: 1,
            groups,
            directory: String::new(),
        }
    }

    #[test]
    fn pictures_are_copied_in_and_read_back_by_id() {
        let (_dir, paths) = temp_paths();
        assert!(load(&paths).unwrap().groups.is_empty());
        save(&paths, library(vec![group("g1", " 服装参考 ")])).unwrap();
        let added = add(
            &paths,
            "g1",
            vec![new_image(" 碎花裙 ", png()), new_image("", png())],
        )
        .unwrap();
        let items = &added.groups[0].items;
        assert_eq!(added.groups[0].name, "服装参考");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name, "碎花裙");
        assert_eq!(items[1].name, UNNAMED_ITEM);
        assert!(items.iter().all(|item| item.file.ends_with(".png")));
        assert!(directory(&paths).join(&items[0].file).is_file());
        assert_eq!(load(&paths).unwrap(), added);

        let original = read_picture(&paths, &items[0].id).unwrap();
        assert_eq!(original.1, "image/png");
        assert_eq!(
            format!("data:image/png;base64,{}", STANDARD.encode(&original.0)),
            png()
        );
        assert_eq!(
            read_picture(&paths, "missing").unwrap_err(),
            "image.errors.notFound"
        );
    }

    #[test]
    fn adding_checks_every_picture_and_the_group_first() {
        let (_dir, paths) = temp_paths();
        save(&paths, library(vec![group("g1", "人物")])).unwrap();
        assert_eq!(
            add(
                &paths,
                "g1",
                vec![
                    new_image("好", png()),
                    new_image("坏", "data:text/html;base64,aGVsbG8=".into())
                ]
            )
            .unwrap_err(),
            "image.errors.input"
        );
        assert_eq!(
            add(&paths, "gone", vec![new_image("好", png())]).unwrap_err(),
            "image.errors.imageMaterialsInvalid"
        );
        assert_eq!(
            add(&paths, "g1", Vec::new()).unwrap_err(),
            "image.errors.imageMaterialsAdd"
        );
        assert!(load(&paths).unwrap().groups[0].items.is_empty());
        let stray = std::fs::read_dir(directory(&paths))
            .unwrap()
            .filter(|entry| entry.as_ref().unwrap().file_name() != INDEX_FILE)
            .count();
        assert_eq!(stray, 0, "nothing is written for a refused add");
    }

    #[test]
    fn saving_keeps_files_by_id_and_deletes_removed_pictures() {
        let (_dir, paths) = temp_paths();
        save(
            &paths,
            library(vec![group("g1", "人物"), group("g2", "背景")]),
        )
        .unwrap();
        let added = add(
            &paths,
            "g1",
            vec![new_image("一", png()), new_image("二", png())],
        )
        .unwrap();
        let (first, second) = (
            added.groups[0].items[0].clone(),
            added.groups[0].items[1].clone(),
        );

        // Renamed, moved to the other group with a forged file name, the other removed.
        let mut edited = added.clone();
        edited.groups[0].items.clear();
        edited.groups[1].items.push(ImageMaterial {
            name: "改名".into(),
            note: "备注".into(),
            file: "../../escape.png".into(),
            ..first.clone()
        });
        let saved = save(&paths, edited).unwrap();
        assert_eq!(
            saved.groups[1].items[0].file, first.file,
            "the file is the backend's"
        );
        assert_eq!(saved.groups[1].items[0].name, "改名");
        assert!(directory(&paths).join(&first.file).is_file());
        assert!(!directory(&paths).join(&second.file).exists());

        // A picture cannot be made up by a save.
        let mut forged = saved.clone();
        forged.groups[0].items.push(ImageMaterial {
            id: "made-up".into(),
            name: "假的".into(),
            note: String::new(),
            file: first.file.clone(),
        });
        assert_eq!(
            save(&paths, forged).unwrap_err(),
            "image.errors.imageMaterialsInvalid"
        );

        // Deleting the group deletes its pictures.
        save(&paths, library(vec![group("g1", "人物")])).unwrap();
        assert!(!directory(&paths).join(&first.file).exists());
    }

    #[test]
    fn moving_the_folder_takes_the_pictures_along() {
        let (dir, paths) = temp_paths();
        save(&paths, library(vec![group("g1", "人物")])).unwrap();
        let added = add(&paths, "g1", vec![new_image("一", png())]).unwrap();
        let file = added.groups[0].items[0].file.clone();
        let old = directory(&paths);
        let custom = dir.path().join("my-materials");
        std::fs::create_dir_all(&custom).unwrap();

        let moved = relocate(&paths, &custom, false).unwrap();
        let canonical = custom.canonicalize().unwrap();
        assert_eq!(directory(&paths), canonical);
        assert_eq!(moved.groups, added.groups);
        assert_eq!(moved.directory, canonical.to_string_lossy());
        assert!(canonical.join(&file).is_file());
        assert!(canonical.join(INDEX_FILE).is_file());
        assert!(!old.join(&file).exists());
        assert!(!old.join(INDEX_FILE).exists());
        assert!(read_picture(&paths, &added.groups[0].items[0].id).is_ok());

        // Choosing the same folder again changes nothing.
        assert_eq!(
            relocate(&paths, &custom, false).unwrap().groups,
            added.groups
        );
        assert!(canonical.join(&file).is_file());
    }

    #[test]
    fn a_folder_with_a_library_is_switched_to_as_it_is() {
        let (dir, paths) = temp_paths();
        save(&paths, library(vec![group("g1", "人物")])).unwrap();
        add(&paths, "g1", vec![new_image("一", png())]).unwrap();
        let first = directory(&paths);
        let second = dir.path().join("second");
        std::fs::create_dir_all(&second).unwrap();
        write_index(&second, &[group("g2", "背景")]).unwrap();

        let switched = relocate(&paths, &second, false).unwrap();
        assert_eq!(switched.groups, vec![group("g2", "背景")]);
        assert!(
            first.join(INDEX_FILE).is_file(),
            "the other library stays where it was"
        );
        assert_eq!(
            relocate(&paths, &first, false).unwrap().groups[0]
                .items
                .len(),
            1
        );
        assert_eq!(
            relocate(&paths, &dir.path().join("missing"), false).unwrap_err(),
            "image.errors.imageMaterials"
        );
    }

    #[test]
    fn sharing_the_gallery_folder_needs_the_users_consent() {
        let (_dir, paths) = temp_paths();
        save(&paths, library(vec![group("g1", "人物")])).unwrap();
        let gallery = gallery_dir(&paths);
        std::fs::create_dir_all(&gallery).unwrap();
        assert_eq!(
            relocate(&paths, &gallery, false).unwrap_err(),
            "image.errors.imageMaterialsShared"
        );
        assert_ne!(directory(&paths), gallery.canonicalize().unwrap());
        relocate(&paths, &gallery, true).unwrap();
        assert_eq!(directory(&paths), gallery.canonicalize().unwrap());
    }

    fn png_bytes() -> Vec<u8> {
        STANDARD
            .decode(png().trim_start_matches("data:image/png;base64,"))
            .unwrap()
    }

    #[test]
    fn a_folder_import_puts_subfolders_in_groups_of_their_names() {
        let (dir, paths) = temp_paths();
        save(
            &paths,
            library(vec![group("g1", "人物"), group("g2", "背景")]),
        )
        .unwrap();
        let source = dir.path().join("素材");
        std::fs::create_dir_all(source.join("背景/夜景")).unwrap();
        std::fs::create_dir_all(source.join("服装")).unwrap();
        std::fs::create_dir_all(source.join("空的")).unwrap();
        std::fs::write(source.join("b.PNG"), png_bytes()).unwrap();
        std::fs::write(source.join("a.png"), png_bytes()).unwrap();
        std::fs::write(source.join("note.txt"), b"not a picture").unwrap();
        std::fs::write(source.join("fake.png"), b"not a picture").unwrap();
        std::fs::write(source.join("背景/街道.png"), png_bytes()).unwrap();
        std::fs::write(source.join("背景/夜景/霓虹.png"), png_bytes()).unwrap();
        std::fs::write(source.join("服装/裙子.png"), png_bytes()).unwrap();

        let imported = import_folder(&paths, "g1", &source).unwrap();
        assert_eq!(
            (
                imported.added,
                imported.groups_created,
                imported.duplicates,
                imported.skipped
            ),
            (5, 1, 0, 1)
        );
        let names = |group: &ImageMaterialGroup| -> Vec<String> {
            group.items.iter().map(|item| item.name.clone()).collect()
        };
        let groups = &imported.library.groups;
        assert_eq!(groups.len(), 3, "an empty subfolder makes no group");
        assert_eq!(names(&groups[0]), ["a", "b"]);
        assert_eq!(
            names(&groups[1]),
            ["街道", "霓虹"],
            "nested pictures join their subfolder's group"
        );
        assert_eq!(
            (groups[2].name.as_str(), names(&groups[2])),
            ("服装", vec!["裙子".to_owned()])
        );
        assert!(groups
            .iter()
            .flat_map(|group| &group.items)
            .all(
                |item| item.file.ends_with(".png") && directory(&paths).join(&item.file).is_file()
            ));
        assert_eq!(load(&paths).unwrap(), imported.library);

        // Importing again adds only what is new.
        std::fs::write(source.join("服装/外套.png"), png_bytes()).unwrap();
        let again = import_folder(&paths, "g1", &source).unwrap();
        assert_eq!(
            (again.added, again.groups_created, again.duplicates),
            (1, 0, 5)
        );
        assert_eq!(names(&again.library.groups[2]), ["裙子", "外套"]);

        assert_eq!(
            import_folder(&paths, "gone", &source).unwrap_err(),
            "image.errors.imageMaterialsInvalid"
        );
        assert_eq!(
            import_folder(&paths, "g1", &directory(&paths)).unwrap_err(),
            "image.errors.imageMaterialsInvalid"
        );
    }

    #[test]
    fn only_backend_file_names_are_kept() {
        for name in ["a1b2-c3.png", "x.jpg", "x.webp", "x.gif"] {
            assert!(valid_file(name), "{name}");
        }
        for name in [
            "", ".png", "x.exe", "../x.png", "a/b.png", "a b.png", "x.PNG", "x",
        ] {
            assert!(!valid_file(name), "{name}");
        }
    }
}
