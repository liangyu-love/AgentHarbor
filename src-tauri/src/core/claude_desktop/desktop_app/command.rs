//! Bounded helper commands. A file sink avoids blocking on a full/inherited pipe.
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

struct Capture {
    path: std::path::PathBuf,
    file: Option<File>,
}
impl Drop for Capture {
    fn drop(&mut self) {
        self.file.take();
        let _ = std::fs::remove_file(&self.path);
    }
}

fn cleanup(mut child: Child, capture_path: &std::path::Path) {
    let _ = child.kill();
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // Keep reaping outside the writer if the OS has not completed termination.
    let capture_path = capture_path.to_path_buf();
    std::thread::spawn(move || {
        let _ = child.wait();
        let _ = std::fs::remove_file(capture_path);
    });
}

pub(super) fn output(command: &mut Command, timeout: Duration) -> std::io::Result<Output> {
    let path = std::env::temp_dir().join(format!(
        "agentharbor-desktop-process-{}.tmp",
        uuid::Uuid::new_v4()
    ));
    let file = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(&path)?;
    let mut capture = Capture {
        path,
        file: Some(file),
    };
    command
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::from(capture.file.as_ref().unwrap().try_clone()?));
    let deadline = Instant::now() + timeout;
    let mut child = command.spawn()?;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Err(error) => {
                cleanup(child, &capture.path);
                return Err(error);
            }
            Ok(None) => {}
        }
        if Instant::now() >= deadline {
            cleanup(child, &capture.path);
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "Desktop helper timed out",
            ));
        }
        let size = match capture.file.as_ref().unwrap().metadata() {
            Ok(metadata) => metadata.len(),
            Err(error) => {
                cleanup(child, &capture.path);
                return Err(error);
            }
        };
        if size > 1024 * 1024 {
            cleanup(child, &capture.path);
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Desktop helper output too large",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let file = capture.file.as_mut().unwrap();
    // Only installation/process metadata is expected, never response bodies.
    if file.metadata()?.len() > 1024 * 1024 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Desktop helper output too large",
        ));
    }
    file.seek(SeekFrom::Start(0))?;
    let mut stdout = Vec::new();
    file.take(1024 * 1024 + 1).read_to_end(&mut stdout)?;
    if stdout.len() > 1024 * 1024 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Desktop helper output too large",
        ));
    }
    Ok(Output {
        status,
        stdout,
        stderr: Vec::new(),
    })
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn helper_failure_exit_code_and_stdout_are_preserved() {
        let mut command = crate::platform::windows::background_command("cmd.exe");
        command.args(["/C", "echo desktop-helper & exit /b 7"]);
        let result = output(&mut command, Duration::from_secs(2)).unwrap();
        assert_eq!(result.status.code(), Some(7));
        assert!(String::from_utf8_lossy(&result.stdout).contains("desktop-helper"));
    }

    /// Not a test of its own: the hung helper the test below runs, as this
    /// test binary, which starts at once (PowerShell can take seconds).
    #[test]
    #[ignore]
    fn hung_helper_stand_in() {
        if let Ok(path) = std::env::var("DESKTOP_HELPER_PID") {
            std::fs::write(path, std::process::id().to_string()).unwrap();
            std::thread::sleep(Duration::from_secs(120));
        }
    }

    #[test]
    fn a_hung_helper_times_out_and_is_reaped_without_blocking_the_writer() {
        let temp = tempfile::tempdir().unwrap();
        let pid_path = temp.path().join("pid");
        let mut command = crate::platform::windows::background_command_path(&std::env::current_exe().unwrap());
        command.args(["--exact", "core::claude_desktop::desktop_app::command::tests::hung_helper_stand_in", "--ignored", "--nocapture"])
            .env("DESKTOP_HELPER_PID", &pid_path);
        let timeout = Duration::from_secs(5);
        let started = Instant::now();
        let error = output(&mut command, timeout).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(started.elapsed() < timeout + Duration::from_secs(4));
        let pid = std::fs::read_to_string(&pid_path)
            .unwrap()
            .parse::<u32>()
            .unwrap();
        assert!(matches!(
            super::super::process_image(pid),
            super::super::ProcessImage::Exited
        ));
    }
}
