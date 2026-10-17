//! Which program holds a local port: when the router cannot take its port,
//! the window names the program listening there and can end it.
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortOwner {
    pub pid: u32,
    /// The executable's file name, such as `Cursor.exe`.
    pub name: String,
}

/// The other process listening on `port` over IPv4 loopback or every
/// interface; none when the port is free or this process holds it.
pub fn listener(port: u16) -> Option<PortOwner> {
    let pid = platform::listening_pid(port)?;
    (pid != std::process::id() && pid != 0).then(|| PortOwner {
        pid,
        name: platform::process_name(pid).unwrap_or_else(|| format!("PID {pid}")),
    })
}

/// Ends the process holding `port`, only if it is still `pid`.
pub fn end(port: u16, pid: u32) -> Result<(), String> {
    if listener(port).map(|owner| owner.pid) != Some(pid) {
        return Err("newapi.errors.portOwnerChanged".into());
    }
    platform::terminate(pid)?;
    // The port frees once the process and its handles are gone.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if platform::listening_pid(port).is_none() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Err("newapi.errors.portBusy".into())
}

#[cfg(windows)]
mod platform {
    use std::net::Ipv4Addr;
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_INSUFFICIENT_BUFFER, INVALID_HANDLE_VALUE, NO_ERROR};
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCPROW_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };

    /// `AF_INET`, without pulling in the WinSock bindings for one constant.
    const AF_INET: u32 = 2;

    pub fn listening_pid(port: u16) -> Option<u32> {
        let mut size = 0u32;
        for _ in 0..4 {
            // u32 words keep the table's rows aligned.
            let mut table = vec![0u32; (size as usize).div_ceil(4).max(1)];
            let status = unsafe {
                GetExtendedTcpTable(table.as_mut_ptr().cast(), &mut size, 0, AF_INET, TCP_TABLE_OWNER_PID_LISTENER, 0)
            };
            if status == ERROR_INSUFFICIENT_BUFFER {
                continue;
            }
            if status != NO_ERROR {
                return None;
            }
            let count = table[0] as usize;
            let rows = unsafe {
                std::slice::from_raw_parts(table.as_ptr().add(1).cast::<MIB_TCPROW_OWNER_PID>(), count)
            };
            return rows
                .iter()
                .find(|row| {
                    let address = Ipv4Addr::from(u32::from_be(row.dwLocalAddr));
                    u16::from_be(row.dwLocalPort as u16) == port && (address.is_loopback() || address.is_unspecified())
                })
                .map(|row| row.dwOwningPid);
        }
        None
    }

    pub fn process_name(pid: u32) -> Option<String> {
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return None;
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut name = None;
            let mut more = Process32FirstW(snapshot, &mut entry) != 0;
            while more {
                if entry.th32ProcessID == pid {
                    let length = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                    name = Some(String::from_utf16_lossy(&entry.szExeFile[..length]));
                    break;
                }
                more = Process32NextW(snapshot, &mut entry) != 0;
            }
            CloseHandle(snapshot);
            name
        }
    }

    pub fn terminate(pid: u32) -> Result<(), String> {
        let status = crate::platform::windows::background_command("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .status()
            .map_err(|_| "newapi.errors.portOwnerEnd")?;
        status.success().then_some(()).ok_or_else(|| "newapi.errors.portOwnerEnd".into())
    }
}

#[cfg(not(windows))]
mod platform {
    /// `lsof` names the listener on macOS and most Linux systems.
    pub fn listening_pid(port: u16) -> Option<u32> {
        let output = std::process::Command::new("lsof")
            .args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-t"])
            .output()
            .ok()?;
        String::from_utf8_lossy(&output.stdout).lines().next()?.trim().parse().ok()
    }

    pub fn process_name(pid: u32) -> Option<String> {
        let output = std::process::Command::new("ps").args(["-p", &pid.to_string(), "-o", "comm="]).output().ok()?;
        let name = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        (!name.is_empty()).then(|| name.rsplit('/').next().unwrap_or(&name).to_owned())
    }

    pub fn terminate(pid: u32) -> Result<(), String> {
        let status = std::process::Command::new("kill")
            .arg(pid.to_string())
            .status()
            .map_err(|_| "newapi.errors.portOwnerEnd")?;
        status.success().then_some(()).ok_or_else(|| "newapi.errors.portOwnerEnd".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_port_this_process_listens_on_has_no_other_owner() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert_eq!(platform::listening_pid(port), Some(std::process::id()));
        assert_eq!(super::listener(port), None);
        assert_eq!(end(port, 4242).unwrap_err(), "newapi.errors.portOwnerChanged");
        drop(listener);
        assert_eq!(platform::listening_pid(port), None);
    }

    /// Apps AgentHarbor starts (Cursor, Codex, a terminal) must not inherit its
    /// sockets: an inherited router listener outlives AgentHarbor and keeps the
    /// port busy until that app exits. mio before 1.2.2 made them inheritable.
    #[cfg(windows)]
    #[tokio::test]
    async fn tokio_sockets_are_not_inherited_by_spawned_apps() {
        use std::os::windows::io::AsRawSocket;
        use windows_sys::Win32::Foundation::{GetHandleInformation, HANDLE_FLAG_INHERIT};
        let inheritable = |socket: &dyn AsRawSocket| {
            let mut flags = 0u32;
            assert_ne!(unsafe { GetHandleInformation(socket.as_raw_socket() as _, &mut flags) }, 0);
            flags & HANDLE_FLAG_INHERIT != 0
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = tokio::net::TcpStream::connect(listener.local_addr().unwrap()).await.unwrap();
        let (accepted, _) = listener.accept().await.unwrap();
        assert!(!inheritable(&listener));
        assert!(!inheritable(&accepted));
        assert!(!inheritable(&client));
    }
}
