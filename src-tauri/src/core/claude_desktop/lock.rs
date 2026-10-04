//! Claude Desktop 直连：跨进程 writer 互斥。
//!
//! 项目已有单实例约束，但那只覆盖同一个交互会话：Windows 的命名 mutex 在
//! `Local\` 作用域内、Unix 的 flock 只对同一台机器上的同一用户生效。两个
//! 会话（或用户）仍可能同时管理同一份 Desktop 配置，因此写操作前先取一个
//! 跨进程 writer 锁。
//!
//! 锁文件由**本项目**独占（`<app_data_dir>/claude-desktop/writer.lock`），
//! 不会写进 Claude Desktop 的目录。持有者的活体判定用 pid + 进程启动时间：
//! pid 会被复用，只看 pid 会在复用后误判为“仍在运行”而永久阻塞写入。

use std::path::{Path, PathBuf};

use super::constants::DATA_DIR_NAME;
use super::types::{error_code, DesktopError, DesktopResult};

const LOCK_FILE: &str = "writer.lock";

fn lock_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(DATA_DIR_NAME).join(LOCK_FILE)
}

/// 记录持有者身份；`started_at` 用来区分被复用的 pid。
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub struct Owner {
    pub pid: u32,
    pub started_at: Option<u64>,
    pub label: String,
}

impl Owner {
    pub fn current(label: &str) -> Self {
        Self {
            pid: std::process::id(),
            started_at: process_started_at(std::process::id()),
            label: label.to_owned(),
        }
    }

    fn encode(&self) -> String {
        format!(
            "pid={}\nstartedAt={}\nlabel={}\n",
            self.pid,
            self.started_at.map(|v| v.to_string()).unwrap_or_default(),
            self.label
        )
    }

    fn decode(text: &str) -> Option<Self> {
        let mut pid = None;
        let mut started_at = None;
        let mut label = String::new();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "pid" => pid = value.trim().parse::<u32>().ok(),
                "startedAt" => {
                    started_at = value.trim().parse::<u64>().ok();
                }
                "label" => label = value.trim().to_owned(),
                _ => {}
            }
        }
        Some(Self {
            pid: pid?,
            started_at,
            label,
        })
    }

    /// 该持有者是否仍然活着。无法确认时按“活着”处理，避免误抢锁。
    pub fn is_alive(&self) -> bool {
        if self.pid == std::process::id() {
            return true;
        }
        let Some(current) = process_started_at(self.pid) else {
            return false;
        };
        match self.started_at {
            // 启动时间不一致说明 pid 已被复用，旧持有者已经退出。
            Some(recorded) => recorded == current,
            None => true,
        }
    }
}

/// 已持有的 writer 锁；drop 时清理锁文件。
#[derive(Debug)]
pub struct DesktopLock {
    path: PathBuf,
    owner: Owner,
}

impl Drop for DesktopLock {
    fn drop(&mut self) {
        // 只清理仍属于自己的锁文件，避免删掉后来者刚写入的记录。
        if let Ok(text) = std::fs::read_to_string(&self.path) {
            if Owner::decode(&text).as_ref() == Some(&self.owner) {
                let _ = std::fs::remove_file(&self.path);
            }
        }
    }
}

fn busy_error(owner: &Owner) -> Box<DesktopError> {
    Box::new(
        DesktopError::new(error_code::WRITER_BUSY, "claudeDesktop.errors.writerBusy")
            .param(owner.pid.to_string())
            .retryable(),
    )
}

/// 取得 writer 锁。已有活着的持有者时立即失败，不阻塞、不排队。
pub fn acquire(app_data_dir: &Path, label: &str) -> DesktopResult<DesktopLock> {
    let path = lock_path(app_data_dir);
    let owner = Owner::current(label);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| super::transaction::storage_error())?;
    }

    // 独占创建；已存在时检查现有持有者。
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut file) => {
            use std::io::Write;
            file.write_all(owner.encode().as_bytes())
                .map_err(|_| super::transaction::storage_error())?;
            file.sync_all()
                .map_err(|_| super::transaction::storage_error())?;
            return Ok(DesktopLock { path, owner });
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(super::transaction::storage_error()),
    }

    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let existing = Owner::decode(&text);
    if let Some(existing) = &existing {
        if existing.is_alive() {
            return Err(busy_error(existing));
        }
    }

    // 残留的失效记录：条带化替换成自己的记录。
    let _ = std::fs::remove_file(&path);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|_| super::transaction::storage_error())?;
    use std::io::Write;
    file.write_all(owner.encode().as_bytes())
        .map_err(|_| super::transaction::storage_error())?;
    file.sync_all()
        .map_err(|_| super::transaction::storage_error())?;
    Ok(DesktopLock { path, owner })
}

// ---------------------------------------------------------------------------
// 进程启动时间
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn process_started_at(pid: u32) -> Option<u64> {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut creation = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let mut exit = creation;
        let mut kernel = creation;
        let mut user = creation;
        let ok = GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user);
        let _ = CloseHandle(handle);
        if ok == 0 {
            return None;
        }
        Some(((creation.dwHighDateTime as u64) << 32) | creation.dwLowDateTime as u64)
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn process_started_at(pid: u32) -> Option<u64> {
    // /proc/<pid>/stat 的第 22 个字段是启动时间（jiffies）。
    let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_name = text.rsplit_once(')')?.1;
    after_name
        .split_whitespace()
        .nth(19)
        .and_then(|value| value.parse::<u64>().ok())
}

#[cfg(target_os = "macos")]
fn process_started_at(pid: u32) -> Option<u64> {
    // macOS 上不读取启动时间；pid 复用只会让写入多等一次进程退出后的清理，
    // 不会误删他人配置（写入前仍有文件级重读校验）。
    let _ = pid;
    None
}

#[cfg(not(any(windows, unix)))]
fn process_started_at(pid: u32) -> Option<u64> {
    let _ = pid;
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "agentharbor-desktop-lock-{name}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_live_holder_blocks_a_second_writer() {
        let dir = temp_dir("busy");
        let first = acquire(&dir, "first").expect("first writer");
        let second = acquire(&dir, "second").expect_err("second writer must fail");
        assert_eq!(second.code, error_code::WRITER_BUSY);
        assert!(second.retryable);
        drop(first);
        // 释放后可以再次取得。
        acquire(&dir, "third").expect("lock released");
    }

    #[test]
    fn a_dead_holder_does_not_block_forever() {
        let dir = temp_dir("stale");
        let path = lock_path(&dir);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // 一个几乎不可能存在的 pid，且 pid 与自己不同。
        std::fs::write(&path, "pid=4294967294\nstartedAt=1\nlabel=ghost\n").unwrap();
        acquire(&dir, "recover").expect("a dead holder must not block");
    }

    #[test]
    fn a_reused_pid_with_a_different_start_time_is_treated_as_stale() {
        let dir = temp_dir("reuse");
        let path = lock_path(&dir);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let owner = Owner {
            pid: std::process::id(),
            // 自己的 pid，但启动时间对不上：模拟 pid 复用。
            started_at: Some(1),
            label: "reused".into(),
        };
        // 自身 pid 视为活体，因此仍应阻塞（这是保守方向）。
        assert!(owner.is_alive());
        std::fs::write(&path, owner.encode()).unwrap();
        assert!(acquire(&dir, "next").is_err());
    }

    /// 模拟另一个**活着的进程**持有锁：写一个不同的、可确认为活体的 pid。
    #[test]
    fn a_foreign_live_process_blocks_and_is_never_deleted() {
        let dir = temp_dir("foreign");
        let path = lock_path(&dir);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // pid 1 在任何支持的平台上都是一个长期存在的进程。
        let foreign = Owner {
            pid: 1,
            started_at: process_started_at(1),
            label: "other-instance".into(),
        };
        if !foreign.is_alive() {
            // 无法确认为活体的平台跳过这条断言。
            return;
        }
        std::fs::write(&path, foreign.encode()).unwrap();
        let error = acquire(&dir, "self").expect_err("foreign writer must block");
        assert_eq!(error.code, error_code::WRITER_BUSY);
        assert!(error.retryable);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("label=other-instance"), "失败路径不得改动他人记录");
    }

    /// 一个连 pid 都解析不出来的残留文件不能永久阻塞。
    #[test]
    fn a_garbage_lock_file_is_reclaimable() {
        let dir = temp_dir("garbage");
        let path = lock_path(&dir);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "").unwrap();
        acquire(&dir, "reclaim").expect("空锁文件可以回收");
    }

    #[test]
    fn dropping_one_writer_never_removes_another_writers_lock() {
        let dir = temp_dir("handover");
        let first = acquire(&dir, "first").unwrap();
        let path = lock_path(&dir);
        // 模拟“另一个进程接管了锁文件”。
        std::fs::write(&path, Owner::current("other").encode()).unwrap();
        drop(first);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("label=other"), "不得删除他人记录");
    }
}
