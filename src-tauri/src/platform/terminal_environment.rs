//! Native user environment access: no shell helper, no console flash.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EnvironmentValue {
    pub kind: u32,
    pub bytes: Vec<u8>,
}

impl EnvironmentValue {
    pub fn string(value: &str) -> Self {
        Self {
            kind: 1, // REG_SZ
            bytes: value
                .encode_utf16()
                .chain(Some(0))
                .flat_map(u16::to_le_bytes)
                .collect(),
        }
    }

    pub fn text(&self) -> Option<String> {
        if !matches!(self.kind, 1 | 2) || self.bytes.len() % 2 != 0 {
            return None;
        }
        let mut units: Vec<u16> = self
            .bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        while units.last() == Some(&0) {
            units.pop();
        }
        String::from_utf16(&units)
            .ok()
            .filter(|text| !text.contains('\0'))
    }
}

pub(crate) trait Environment {
    fn read(&mut self, name: &str) -> Result<Option<EnvironmentValue>, String>;
    fn write(&mut self, name: &str, value: Option<&EnvironmentValue>) -> Result<(), String>;
    fn notify(&self) -> bool;
}

#[cfg(windows)]
pub(crate) mod windows {
    use super::*;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_SUCCESS},
        System::Registry::*,
    };

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }
    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }

    pub struct UserEnvironment {
        key: Option<Key>,
        writable: bool,
    }
    impl UserEnvironment {
        pub fn open(writable: bool) -> Result<Self, String> {
            Self::open_at("Environment", writable)
        }

        // Tests use a temporary HKCU subkey, never the real user environment.
        pub(super) fn open_at(path: &str, writable: bool) -> Result<Self, String> {
            let name = wide(path);
            let mut key = null_mut();
            let access = KEY_QUERY_VALUE | if writable { KEY_SET_VALUE } else { 0 };
            let code =
                unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, name.as_ptr(), 0, access, &mut key) };
            if code == ERROR_FILE_NOT_FOUND {
                if !writable {
                    return Ok(Self {
                        key: None,
                        writable,
                    });
                }
                let code = unsafe {
                    RegCreateKeyExW(
                        HKEY_CURRENT_USER,
                        name.as_ptr(),
                        0,
                        null(),
                        REG_OPTION_NON_VOLATILE,
                        access,
                        null(),
                        &mut key,
                        null_mut(),
                    )
                };
                if code != ERROR_SUCCESS {
                    return Err("settings.terminalProxy.errors.environment".into());
                }
            } else if code != ERROR_SUCCESS {
                return Err("settings.terminalProxy.errors.environment".into());
            }
            Ok(Self {
                key: Some(Key(key)),
                writable,
            })
        }
    }

    impl Environment for UserEnvironment {
        fn read(&mut self, name: &str) -> Result<Option<EnvironmentValue>, String> {
            let Some(key) = &self.key else {
                return Ok(None);
            };
            let name = wide(name);
            // Retry boundedly if another application resizes a value mid-read.
            for _ in 0..3 {
                let mut kind = 0;
                let mut length = 0;
                let code = unsafe {
                    RegQueryValueExW(
                        key.0,
                        name.as_ptr(),
                        null(),
                        &mut kind,
                        null_mut(),
                        &mut length,
                    )
                };
                if code == ERROR_FILE_NOT_FOUND {
                    return Ok(None);
                }
                if code != ERROR_SUCCESS || length > 65_536 {
                    return Err("settings.terminalProxy.errors.environment".into());
                }
                let mut bytes = vec![0; length as usize];
                let code = unsafe {
                    RegQueryValueExW(
                        key.0,
                        name.as_ptr(),
                        null(),
                        &mut kind,
                        bytes.as_mut_ptr(),
                        &mut length,
                    )
                };
                if code == ERROR_MORE_DATA {
                    continue;
                }
                if code == ERROR_FILE_NOT_FOUND {
                    return Ok(None);
                }
                if code != ERROR_SUCCESS {
                    return Err("settings.terminalProxy.errors.environment".into());
                }
                bytes.truncate(length as usize);
                return Ok(Some(EnvironmentValue { kind, bytes }));
            }
            Err("settings.terminalProxy.errors.environment".into())
        }

        fn write(&mut self, name: &str, value: Option<&EnvironmentValue>) -> Result<(), String> {
            if !self.writable {
                return Err("settings.terminalProxy.errors.environment".into());
            }
            let key = self
                .key
                .as_ref()
                .ok_or("settings.terminalProxy.errors.environment")?;
            let name = wide(name);
            let code = unsafe {
                match value {
                    Some(value) => RegSetValueExW(
                        key.0,
                        name.as_ptr(),
                        0,
                        value.kind,
                        value.bytes.as_ptr(),
                        value.bytes.len() as u32,
                    ),
                    None => RegDeleteValueW(key.0, name.as_ptr()),
                }
            };
            if code == ERROR_SUCCESS || (value.is_none() && code == ERROR_FILE_NOT_FOUND) {
                Ok(())
            } else {
                Err("settings.terminalProxy.errors.environment".into())
            }
        }

        fn notify(&self) -> bool {
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
            };
            let message = wide("Environment");
            let mut result = 0;
            unsafe {
                SendMessageTimeoutW(
                    HWND_BROADCAST,
                    WM_SETTINGCHANGE,
                    0,
                    message.as_ptr() as isize,
                    SMTO_ABORTIFHUNG,
                    1_000,
                    &mut result,
                ) != 0
            }
        }
    }

    pub fn open_terminal(values: &[(String, String)]) -> Result<(), String> {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::CREATE_NEW_CONSOLE;
        // The user explicitly requested a visible interactive terminal.
        std::process::Command::new("powershell.exe")
            .args(["-NoLogo", "-NoProfile", "-NoExit"])
            .creation_flags(CREATE_NEW_CONSOLE)
            .envs(values.iter().cloned())
            .spawn()
            .map(|_| ())
            .map_err(|_| "settings.terminalProxy.errors.launch".to_owned())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn native_registry_preserves_raw_types_and_removes_absent_values() {
            let path = format!(r"Software\AgentHarbor-tests\{}", uuid::Uuid::new_v4());
            let mut env = UserEnvironment::open_at(&path, true).unwrap();
            let mut expanded = EnvironmentValue::string("http://%PROXY_HOST%:7890");
            expanded.kind = REG_EXPAND_SZ;
            env.write("HTTPS_PROXY", Some(&expanded)).unwrap();
            assert_eq!(env.read("https_proxy").unwrap(), Some(expanded));
            // 二进制值逐字节往返，不受空字符规范化影响。
            let binary = EnvironmentValue {
                kind: REG_BINARY,
                bytes: vec![0, 255, 17, 0, 85],
            };
            env.write("HTTP_PROXY", Some(&binary)).unwrap();
            assert_eq!(env.read("HTTP_PROXY").unwrap(), Some(binary));
            for raw in [
                // Windows 会在写入时把 `REG_SZ` 规范成以空字符结尾，因此这里
                // 断言的是「内容被保留」，而不是逐字节等于写入值——否则这条用例
                // 会依赖注册表实现细节而随机失败。
                EnvironmentValue {
                    kind: REG_SZ,
                    bytes: vec![b'a', 0],
                },
                EnvironmentValue {
                    kind: REG_SZ,
                    bytes: vec![],
                },
            ] {
                env.write("HTTP_PROXY", Some(&raw)).unwrap();
                let read = env.read("HTTP_PROXY").unwrap().expect("value present");
                assert_eq!(read.kind, raw.kind, "类型必须保留");
                // 空字符规范化只允许出现在末尾的填充。
                let mut expected = raw.bytes.clone();
                while expected.len() < read.bytes.len() {
                    expected.push(0);
                }
                assert_eq!(read.bytes, expected, "内容必须保留");
            }
            env.write("HTTP_PROXY", None).unwrap();
            env.write("HTTPS_PROXY", None).unwrap();
            env.write("HTTPS_PROXY", None).unwrap();
            assert_eq!(env.read("HTTPS_PROXY").unwrap(), None);
            let mut readonly = UserEnvironment::open_at(&path, false).unwrap();
            assert!(readonly.write("HTTPS_PROXY", None).is_err());
            drop(readonly);
            drop(env);
            let name = wide(&path);
            assert_eq!(
                unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, name.as_ptr()) },
                ERROR_SUCCESS
            );
        }
    }
}
