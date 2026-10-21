#[cfg(target_os = "windows")]
pub(crate) const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(target_os = "windows")]
pub fn background_command(program: &str) -> std::process::Command {
    use std::os::windows::process::CommandExt;


    let mut command = std::process::Command::new(program);
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(target_os = "windows")]
pub fn background_command_path(program: &std::path::Path) -> std::process::Command {
    use std::os::windows::process::CommandExt;


    let mut command = std::process::Command::new(program);
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

/// The install folder an uninstall entry whose display name starts with
/// `prefix` records, for apps whose installer lets the user pick the folder.
#[cfg(target_os = "windows")]
pub fn uninstall_location(prefix: &str) -> Option<std::path::PathBuf> {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ,
        KEY_WOW64_32KEY, RRF_RT_REG_SZ,
    };
    let wide = |text: &str| text.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let read = |key: HKEY, subkey: &[u16], name: &str| -> Option<String> {
        let name = wide(name);
        let mut size = 0u32;
        let query = |data: *mut core::ffi::c_void, size: &mut u32| unsafe {
            RegGetValueW(key, subkey.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), data, size)
        };
        if query(std::ptr::null_mut(), &mut size) != 0 || size == 0 {
            return None;
        }
        let mut buffer = vec![0u16; size as usize / 2 + 1];
        if query(buffer.as_mut_ptr().cast(), &mut size) != 0 {
            return None;
        }
        let length = buffer.iter().position(|&unit| unit == 0).unwrap_or(buffer.len());
        Some(String::from_utf16_lossy(&buffer[..length]))
    };
    let root = wide(r"Software\Microsoft\Windows\CurrentVersion\Uninstall");
    for (hive, view) in [(HKEY_CURRENT_USER, 0), (HKEY_LOCAL_MACHINE, 0), (HKEY_LOCAL_MACHINE, KEY_WOW64_32KEY)] {
        let mut key: HKEY = std::ptr::null_mut();
        if unsafe { RegOpenKeyExW(hive, root.as_ptr(), 0, KEY_READ | view, &mut key) } != 0 {
            continue;
        }
        let mut found = None;
        for index in 0.. {
            let mut name = [0u16; 256];
            let mut length = name.len() as u32;
            let status = unsafe {
                RegEnumKeyExW(key, index, name.as_mut_ptr(), &mut length, std::ptr::null_mut(), std::ptr::null_mut(),
                    std::ptr::null_mut(), std::ptr::null_mut())
            };
            if status != 0 {
                break;
            }
            let subkey: Vec<u16> = name[..length as usize].iter().copied().chain(Some(0)).collect();
            if read(key, &subkey, "DisplayName").is_some_and(|display| display.starts_with(prefix)) {
                found = read(key, &subkey, "InstallLocation")
                    .map(|location| location.trim().trim_matches('"').to_owned())
                    .filter(|location| !location.is_empty())
                    .map(std::path::PathBuf::from);
                if found.is_some() {
                    break;
                }
            }
        }
        unsafe { RegCloseKey(key) };
        if found.is_some() {
            return found;
        }
    }
    None
}

/// A string value (an expandable one expanded), or `None` when the key or
/// value is missing.
#[cfg(target_os = "windows")]
fn registry_text(
    hive: windows_sys::Win32::System::Registry::HKEY,
    subkey: &str,
    name: &str,
) -> Option<String> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, RRF_RT_REG_SZ};
    let wide = |text: &str| text.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (subkey, name) = (wide(subkey), wide(name));
    let query = |data: *mut core::ffi::c_void, size: &mut u32| unsafe {
        RegGetValueW(hive, subkey.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), data, size)
    };
    let mut size = 0u32;
    if query(std::ptr::null_mut(), &mut size) != 0 || size == 0 {
        return None;
    }
    let mut buffer = vec![0u16; size as usize / 2 + 1];
    if query(buffer.as_mut_ptr().cast(), &mut size) != 0 {
        return None;
    }
    let length = buffer.iter().position(|&unit| unit == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..length]))
}

/// A current-user string value, or `None` when the key or value is missing.
#[cfg(target_os = "windows")]
pub(crate) fn user_registry_text(subkey: &str, name: &str) -> Option<String> {
    registry_text(windows_sys::Win32::System::Registry::HKEY_CURRENT_USER, subkey, name)
}

/// An environment variable as a process started now gets it: the user's
/// value, else the machine's. This app's own environment is the one it
/// started with.
#[cfg(target_os = "windows")]
pub(crate) fn current_environment_text(name: &str) -> Option<String> {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    registry_text(HKEY_CURRENT_USER, "Environment", name).or_else(|| {
        registry_text(
            HKEY_LOCAL_MACHINE,
            r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
            name,
        )
    })
}

/// PATH as a process started now gets it: the machine's entries, then the
/// user's. An installer that added a folder since this app started is in it.
#[cfg(target_os = "windows")]
pub(crate) fn current_path() -> Option<String> {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    let machine = registry_text(
        HKEY_LOCAL_MACHINE,
        r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
        "Path",
    );
    let user = registry_text(HKEY_CURRENT_USER, "Environment", "Path");
    let joined: Vec<String> = [machine, user]
        .into_iter()
        .flatten()
        .map(|part| part.trim_matches(';').to_owned())
        .filter(|part| !part.is_empty())
        .collect();
    (!joined.is_empty()).then(|| joined.join(";"))
}

/// A current-user DWORD value, or `None` when the key or value is missing.
#[cfg(target_os = "windows")]
pub(crate) fn user_registry_dword(subkey: &str, name: &str) -> Option<u32> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let wide = |text: &str| text.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (subkey, name) = (wide(subkey), wide(name));
    let mut value = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, subkey.as_ptr(), name.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(),
            (&mut value as *mut u32).cast(), &mut size)
    };
    (status == 0).then_some(value)
}
