//! Claude Desktop 直连：跨进程 writer 互斥。
//!
//! 对本项目的持久 writer.lock 取得操作系统文件锁。锁的有效期跟随文件句柄，
//! 进程退出或崩溃会自动释放；文件内的 pid 仅用于提示，不参与所有权判断。
//! 始终保留同一个锁文件，避免删除/重建让等待者锁住不同文件的竞争。

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use super::constants::DATA_DIR_NAME;
use super::types::{error_code, DesktopError, DesktopResult};

const LOCK_FILE: &str = "writer.lock";

fn lock_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(DATA_DIR_NAME).join(LOCK_FILE)
}

/// 持有句柄期间排他；关闭句柄释放锁，不删除锁文件。
#[derive(Debug)]
pub struct DesktopLock {
    _file: File,
}

fn busy_error(file: &mut File) -> Box<DesktopError> {
    let mut text = String::new();
    let _ = file.read_to_string(&mut text);
    let pid = text
        .lines()
        .find_map(|line| line.strip_prefix("pid="))
        .and_then(|pid| pid.parse::<u32>().ok())
        .map(|pid| pid.to_string())
        .unwrap_or_else(|| "?".into());
    Box::new(
        DesktopError::new(error_code::WRITER_BUSY, "claudeDesktop.errors.writerBusy")
            .param(pid)
            .retryable(),
    )
}

/// 已有持有者时立即失败；空、损坏或旧记录均不影响操作系统的互斥判断。
pub fn acquire(app_data_dir: &Path, label: &str) -> DesktopResult<DesktopLock> {
    let path = lock_path(app_data_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| super::transaction::storage_error())?;
    }
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|_| super::transaction::storage_error())?;
    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Err(busy_error(&mut file)),
        Err(TryLockError::Error(_)) => return Err(super::transaction::storage_error()),
    }

    // 只能在取得锁后更新诊断记录；失败会关闭句柄并释放锁。
    file.set_len(0)
        .and_then(|()| file.seek(SeekFrom::Start(0)).map(|_| ()))
        .and_then(|()| writeln!(file, "pid={}\nlabel={label}", std::process::id()))
        .and_then(|()| file.sync_all())
        .map_err(|_| super::transaction::storage_error())?;
    Ok(DesktopLock { _file: file })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

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
        acquire(&dir, "third").expect("lock released");
        assert!(
            lock_path(&dir).exists(),
            "never unlink the shared lock file"
        );
    }

    #[test]
    fn an_empty_record_during_initialization_cannot_be_reclaimed() {
        let dir = temp_dir("initializing");
        let path = lock_path(&dir);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.try_lock().unwrap();
        let error = acquire(&dir, "second").expect_err("empty active lock must block");
        assert_eq!(error.code, error_code::WRITER_BUSY);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
        drop(file);
        acquire(&dir, "after-drop").expect("released empty lock is reusable");
    }

    #[test]
    fn a_stale_or_corrupt_record_without_a_holder_is_reusable() {
        for record in ["", "garbage", "pid=4294967294\nlabel=ghost\n"] {
            let dir = temp_dir("stale");
            let path = lock_path(&dir);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, record).unwrap();
            acquire(&dir, "recover").expect("unlocked records do not block");
        }
    }

    // The parent starts this exact test in a separate test process with a private root.
    #[test]
    fn subprocess_lock_holder() {
        let Some(dir) = std::env::var_os("AGENTHARBOR_DESKTOP_LOCK_TEST_ROOT") else {
            return;
        };
        let dir = PathBuf::from(dir);
        let _guard = acquire(&dir, "child").unwrap();
        std::fs::write(dir.join("ready"), b"ready").unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn a_foreign_process_blocks_and_crash_releases_the_lock() {
        let dir = temp_dir("subprocess");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "core::claude_desktop::lock::tests::subprocess_lock_holder",
                "--nocapture",
            ])
            .env("AGENTHARBOR_DESKTOP_LOCK_TEST_ROOT", &dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !dir.join("ready").exists() && Instant::now() < deadline {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let ready = dir.join("ready").exists();
        if !ready {
            let _ = child.kill();
            let _ = child.wait();
            panic!("isolated child did not acquire its lock");
        }
        // Always stop/reap the synthetic child before asserting the outcome.
        let blocked = acquire(&dir, "parent");
        let _ = child.kill();
        child.wait().unwrap();
        assert_eq!(
            blocked.expect_err("another process holds the lock").code,
            error_code::WRITER_BUSY
        );
        acquire(&dir, "after-crash").expect("OS releases the terminated holder's lock");
    }
}
