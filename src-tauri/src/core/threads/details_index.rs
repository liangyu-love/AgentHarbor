//! Offset-only transcript index. Never caches plaintext or skips identity checks.
use super::*;
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    io::{Seek, SeekFrom},
    sync::{Arc, Mutex, OnceLock},
    time::SystemTime,
};

const CACHE_BYTES: usize = 32 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Stamp {
    len: u64,
    modified: SystemTime,
    created: Option<SystemTime>,
    identity: (u64, u64),
    changed: (i64, i64),
}
pub(super) fn stamp(file: &File) -> Result<Stamp, CoreError> {
    let metadata = file.metadata()?;
    #[cfg(unix)]
    let (identity, changed) = {
        use std::os::unix::fs::MetadataExt;
        (
            (metadata.dev(), metadata.ino()),
            (metadata.ctime(), metadata.ctime_nsec()),
        )
    };
    #[cfg(windows)]
    let (identity, changed) = {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            FileBasicInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
            BY_HANDLE_FILE_INFORMATION, FILE_BASIC_INFO,
        };
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        let mut basic: FILE_BASIC_INFO = unsafe { std::mem::zeroed() };
        // Both buffers have the documented layout and remain valid for the call;
        // the borrowed OS handle remains open for the entire snapshot read.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0
            || unsafe {
                GetFileInformationByHandleEx(
                    file.as_raw_handle(),
                    FileBasicInfo,
                    &mut basic as *mut _ as *mut _,
                    std::mem::size_of::<FILE_BASIC_INFO>() as u32,
                )
            } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        (
            (
                info.dwVolumeSerialNumber as u64,
                ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
            ),
            (basic.ChangeTime, 0),
        )
    };
    #[cfg(not(any(unix, windows)))]
    let (identity, changed) = ((0, 0), (0, 0));
    Ok(Stamp {
        len: metadata.len(),
        modified: metadata.modified()?,
        created: metadata.created().ok(),
        identity,
        changed,
    })
}
pub(super) fn token(stamp: &Stamp) -> String {
    format!("{:x}", Sha256::digest(format!("{stamp:?}").as_bytes()))
}
pub(super) fn ensure_unchanged(
    path: &Path,
    file: &File,
    expected: &Stamp,
) -> Result<(), CoreError> {
    if &stamp(file)? != expected || stamp(&File::open(path)?)? != *expected {
        cache()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|(p, _)| p != path);
        return Err(CoreError::InvalidData(
            "rollout changed; refresh the thread details".into(),
        ));
    }
    Ok(())
}
pub(super) struct Index {
    stamp: Stamp,
    snapshot_bytes: u64,
    id: String,
    positions: Vec<u64>,
    diagnostics: Diagnostics,
}
type Cache = VecDeque<(PathBuf, Arc<Index>)>;
fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(VecDeque::new()))
}
pub(super) fn remember(
    path: &Path,
    stamp: Stamp,
    snapshot_bytes: u64,
    id: &str,
    positions: Vec<u64>,
    diagnostics: Diagnostics,
) -> Arc<Index> {
    let entry = Arc::new(Index {
        stamp,
        snapshot_bytes,
        id: id.into(),
        positions,
        diagnostics,
    });
    let mut cache = cache().lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|(p, _)| p != path);
    let size = entry.positions.len() * std::mem::size_of::<u64>();
    if size <= CACHE_BYTES {
        while cache.len() >= 8
            || cache
                .iter()
                .map(|(_, i)| i.positions.len() * 8)
                .sum::<usize>()
                + size
                > CACHE_BYTES
        {
            cache.pop_front();
        }
        cache.push_back((path.into(), entry.clone()));
    }
    entry
}
pub(super) fn page(
    path: &Path,
    id: &str,
    snapshot_bytes: u64,
    offset: usize,
    snapshot_id: Option<&str>,
) -> Result<MessagePage, CoreError> {
    let (mut reader, _) = open_snapshot(path, Some(snapshot_bytes))?;
    let fingerprint = stamp(reader.get_ref().get_ref())?;
    if snapshot_id.is_some_and(|t| t != token(&fingerprint)) {
        cache()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|(p, _)| p != path);
        return Err(CoreError::InvalidData(
            "rollout changed; refresh the thread details".into(),
        ));
    }
    let cached = {
        let mut cache = cache().lock().unwrap_or_else(|e| e.into_inner());
        cache.retain(|(p, entry)| p != path || entry.stamp == fingerprint);
        cache
            .iter()
            .find(|(p, entry)| {
                p == path
                    && entry.stamp == fingerprint
                    && entry.id == id
                    && entry.snapshot_bytes == snapshot_bytes
            })
            .map(|(_, i)| i.clone())
    };
    let index = if let Some(index) = cached {
        index
    } else {
        let mut diagnostics = Diagnostics::default();
        let mut parser = MessageParser::default();
        let mut positions = Vec::new();
        visit(&mut reader, id, &mut diagnostics, |value, _, position| {
            if parser.candidate(value).is_some() {
                positions.push(position);
            }
            true
        })?;
        ensure_unchanged(path, reader.get_ref().get_ref(), &fingerprint)?;
        remember(
            path,
            fingerprint.clone(),
            snapshot_bytes,
            id,
            positions,
            diagnostics,
        )
    };
    let file = reader.into_inner().into_inner();
    let mut reader = BufReader::new(file);
    let end = offset.saturating_add(PAGE_SIZE).min(index.positions.len());
    let mut messages = Vec::new();
    let mut line = Vec::new();
    for position in index.positions.get(offset..end).unwrap_or_default() {
        reader.seek(SeekFrom::Start(*position))?;
        let (read, oversized, _) = read_line(
            &mut (&mut reader).take(snapshot_bytes - position),
            &mut line,
        )?;
        if !read || oversized {
            return Err(CoreError::InvalidData(
                "indexed rollout changed; refresh details".into(),
            ));
        }
        let value: Value = serde_json::from_slice(&line)?;
        let (role, text) = MessageParser::default().candidate(&value).ok_or_else(|| {
            CoreError::InvalidData("indexed message changed; refresh details".into())
        })?;
        messages.push(Message {
            role,
            timestamp: timestamp(&value).into(),
            truncated: text.chars().nth(MAX_TEXT_CHARS).is_some(),
            text: text.chars().take(MAX_TEXT_CHARS).collect(),
        });
    }
    ensure_unchanged(path, reader.get_ref(), &fingerprint)?;
    Ok(MessagePage {
        messages,
        next_offset: (end < index.positions.len()).then_some(end),
        diagnostics: index.diagnostics.clone(),
    })
}
