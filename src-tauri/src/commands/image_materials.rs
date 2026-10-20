//! The image workbench's image materials: pictures the user keeps in groups to use
//! as image-to-image references, each with a name and note the polishing model reads.
//! The pictures are copied into the app data folder beside an index of the groups.

use super::image::{
    decode_data_url, paths, thumbnail_data_url, MAX_IMAGE_BYTES, THUMBNAIL_EDGE, THUMBNAIL_JOBS,
};
use crate::core::{repository::Repository, storage};
use crate::platform::paths::CodexPaths;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Mutex,
};
use tauri::State;

const DIRECTORY: &str = "image-materials";
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

fn directory(paths: &CodexPaths) -> PathBuf {
    paths.app_data_dir.join(DIRECTORY)
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
    let bytes = match std::fs::read(directory(paths).join(INDEX_FILE)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err("image.errors.imageMaterials".into()),
    };
    let stored: ImageMaterialLibrary =
        serde_json::from_slice(&bytes).map_err(|_| "image.errors.imageMaterials".to_owned())?;
    Ok(sanitize(stored.groups).0)
}

fn write_groups(paths: &CodexPaths, groups: &[ImageMaterialGroup]) -> Result<(), String> {
    let index = ImageMaterialLibrary {
        version: LIBRARY_VERSION,
        groups: groups.to_vec(),
        directory: String::new(),
    };
    let contents =
        serde_json::to_vec_pretty(&index).map_err(|_| "image.errors.imageMaterials".to_owned())?;
    storage::write_atomic(&directory(paths).join(INDEX_FILE), &contents)
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
