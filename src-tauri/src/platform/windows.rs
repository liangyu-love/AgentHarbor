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
