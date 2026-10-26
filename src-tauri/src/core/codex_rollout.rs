//! Codex compresses cold rollouts in place to `<name>.jsonl.zst` and turns them
//! back into plain `.jsonl` before appending. Its catalog keeps the plain path,
//! so readers accept either representation and writers materialize first.
use super::{models::CoreError, storage};
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};

const COMPRESSED_SUFFIX: &str = ".zst";
/// Bound for decoded rollouts, matching the largest plain rollout we open.
pub const MAX_DECODED_BYTES: u64 = 512 * 1024 * 1024;

/// `rollout-*.jsonl` or its compressed `rollout-*.jsonl.zst`.
pub fn is_rollout_name(name: &str) -> bool {
    name.starts_with("rollout-") && (name.ends_with(".jsonl") || name.ends_with(".jsonl.zst"))
}

pub fn is_rollout_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(is_rollout_name)
}

pub fn is_compressed(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".jsonl.zst"))
}

/// The plain `.jsonl` path a rollout is known by, whichever form is on disk.
pub fn plain_path(path: &Path) -> PathBuf {
    if is_compressed(path) {
        path.with_extension("")
    } else {
        path.to_path_buf()
    }
}

pub fn compressed_path(path: &Path) -> PathBuf {
    let mut name = plain_path(path).into_os_string();
    name.push(COMPRESSED_SUFFIX);
    PathBuf::from(name)
}

/// A compressed rollout whose plain sibling exists is stale: Codex reads the
/// plain file and overwrites the compressed one on its next compression pass.
pub fn is_shadowed(path: &Path) -> bool {
    is_compressed(path) && plain_path(path).is_file()
}

/// The representation on disk, preferring plain like Codex does.
pub fn existing(path: &Path) -> Option<PathBuf> {
    [plain_path(path), compressed_path(path)]
        .into_iter()
        .find(|candidate| candidate.is_file())
}

/// Decoded JSONL bytes of the file at `path` (no representation lookup).
pub fn reader(path: &Path) -> io::Result<Box<dyn Read + Send>> {
    let file = File::open(path)?;
    decoded(file, is_compressed(path))
}

pub fn decoded(file: File, compressed: bool) -> io::Result<Box<dyn Read + Send>> {
    Ok(if compressed {
        Box::new(zstd::stream::read::Decoder::new(file)?.take(MAX_DECODED_BYTES + 1))
    } else {
        Box::new(file)
    })
}

pub fn buffered(path: &Path) -> io::Result<BufReader<Box<dyn Read + Send>>> {
    Ok(BufReader::new(reader(path)?))
}

pub fn read(path: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader(path)?.read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_DECODED_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "decoded rollout is too large",
        ));
    }
    Ok(bytes)
}

/// Make the plain `.jsonl` the only representation before writing to it, the
/// way Codex does before an append. Returns the plain path.
pub fn materialize(path: &Path) -> Result<PathBuf, CoreError> {
    let plain = plain_path(path);
    let compressed = compressed_path(path);
    if plain.is_file() {
        return Ok(plain);
    }
    if !compressed.is_file() {
        return Err(CoreError::NotFound(format!(
            "Thread rollout {}",
            plain.display()
        )));
    }
    let bytes = read(&compressed)?;
    storage::write_atomic(&plain, &bytes)?;
    storage::remove_file_durable(&compressed)?;
    Ok(plain)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn compress(path: &Path) -> PathBuf {
        let target = compressed_path(path);
        let bytes = std::fs::read(path).unwrap();
        std::fs::write(&target, zstd::stream::encode_all(&bytes[..], 3).unwrap()).unwrap();
        std::fs::remove_file(path).unwrap();
        target
    }

    #[test]
    fn names_and_paths_map_between_representations() {
        assert!(is_rollout_name("rollout-a.jsonl"));
        assert!(is_rollout_name("rollout-a.jsonl.zst"));
        assert!(!is_rollout_name("rollout-a.zst"));
        assert!(!is_rollout_name("other.jsonl.zst"));
        let plain = Path::new("s/rollout-a.jsonl");
        assert_eq!(compressed_path(plain), Path::new("s/rollout-a.jsonl.zst"));
        assert_eq!(plain_path(&compressed_path(plain)), plain);
        assert_eq!(plain_path(plain), plain);
    }

    #[test]
    fn reads_and_materializes_a_compressed_rollout() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("rollout-a.jsonl");
        std::fs::write(&plain, b"{\"type\":\"session_meta\"}\n").unwrap();
        let compressed = compress(&plain);
        assert_eq!(existing(&plain), Some(compressed.clone()));
        assert_eq!(read(&compressed).unwrap(), b"{\"type\":\"session_meta\"}\n");
        assert_eq!(materialize(&compressed).unwrap(), plain);
        assert!(!compressed.exists());
        assert_eq!(
            std::fs::read(&plain).unwrap(),
            b"{\"type\":\"session_meta\"}\n"
        );
        assert_eq!(existing(&compressed), Some(plain));
    }
}

#[cfg(test)]
pub(crate) use tests::compress;
