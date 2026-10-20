//! Directory links from an agent's skills folder to the library's copy. On
//! Windows a junction (a mount-point reparse point) needs neither
//! administrator rights nor developer mode, unlike a directory symlink; on
//! other systems a symlink. Removing a link never touches what it points to.
use std::path::Path;

/// Whether `path` itself is a link (a symlink or a Windows junction).
pub fn is_link(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| super::is_link(&metadata))
}

/// Whether `path` is a link that resolves to `target`.
pub fn links_to(path: &Path, target: &Path) -> bool {
    is_link(path)
        && match (std::fs::canonicalize(path), std::fs::canonicalize(target)) {
            (Ok(resolved), Ok(target)) => resolved == target,
            _ => false,
        }
}

/// Removes the link at `path`; the directory it points to stays.
pub fn remove(path: &Path) -> std::io::Result<()> {
    if !is_link(path) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "not a link",
        ));
    }
    // A junction and a directory symlink are removed as directories on Windows.
    std::fs::remove_dir(path).or_else(|_| std::fs::remove_file(path))
}

#[cfg(unix)]
pub fn create(link: &Path, target: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
pub fn create(link: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Ioctl::FSCTL_SET_REPARSE_POINT;
    use windows_sys::Win32::System::IO::DeviceIoControl;
    const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;

    // A junction names an absolute local path; `\\?\` paths become `\??\`.
    let target = std::fs::canonicalize(target)?;
    let text = target.to_string_lossy();
    let plain = text.strip_prefix(r"\\?\").unwrap_or(&text);
    if plain.starts_with(r"UNC\") || plain.starts_with(r"\\") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "a junction cannot point to a network path",
        ));
    }
    let substitute: Vec<u16> = format!(r"\??\{plain}").encode_utf16().collect();
    let print: Vec<u16> = plain.encode_utf16().collect();
    let substitute_bytes = substitute.len() * 2;
    let print_bytes = print.len() * 2;
    // The four name fields, then both names with their terminators.
    let data_length = 8 + substitute_bytes + 2 + print_bytes + 2;
    let field = |value: usize| {
        u16::try_from(value).map(u16::to_le_bytes).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "path too long for a junction",
            )
        })
    };
    let mut buffer = Vec::with_capacity(8 + data_length);
    buffer.extend(IO_REPARSE_TAG_MOUNT_POINT.to_le_bytes());
    buffer.extend(field(data_length)?);
    buffer.extend([0, 0]);
    buffer.extend(field(0)?);
    buffer.extend(field(substitute_bytes)?);
    buffer.extend(field(substitute_bytes + 2)?);
    buffer.extend(field(print_bytes)?);
    for unit in substitute.iter().chain(&[0]).chain(&print).chain(&[0]) {
        buffer.extend(unit.to_le_bytes());
    }

    std::fs::create_dir(link)?;
    let wide: Vec<u16> = link.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: `wide` is a NUL-terminated path; the handle is closed below.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        let error = std::io::Error::last_os_error();
        let _ = std::fs::remove_dir(link);
        return Err(error);
    }
    let mut returned = 0u32;
    // SAFETY: the buffer outlives the call and its length is passed with it.
    let done = unsafe {
        DeviceIoControl(
            handle,
            FSCTL_SET_REPARSE_POINT,
            buffer.as_ptr().cast(),
            buffer.len() as u32,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    let error = (done == 0).then(std::io::Error::last_os_error);
    // SAFETY: `handle` came from CreateFileW above and is closed once.
    unsafe { CloseHandle(handle) };
    if let Some(error) = error {
        let _ = std::fs::remove_dir(link);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_reads_through_and_removing_it_keeps_the_target() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("library dir & more/skill");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("SKILL.md"), "# Skill").unwrap();
        let link = temp.path().join("agent/skills/skill");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();

        create(&link, &target).unwrap();
        assert!(is_link(&link));
        assert!(links_to(&link, &target));
        assert_eq!(
            std::fs::read_to_string(link.join("SKILL.md")).unwrap(),
            "# Skill"
        );
        assert!(!links_to(&link, temp.path()));

        remove(&link).unwrap();
        assert!(!link.exists());
        assert!(target.join("SKILL.md").is_file(), "the target stays");
        assert!(
            remove(&target).is_err(),
            "a real directory is not removed as a link"
        );
        // An existing name is never replaced.
        std::fs::create_dir(&link).unwrap();
        assert!(create(&link, &target).is_err());
    }
}
