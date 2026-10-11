//! One cancellable OAuth attempt per application. No credential material is logged.
use super::{account_monitor, auth, models::ApiProxyConfigPayload};
use crate::platform::paths::CodexPaths;
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    path::PathBuf,
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{ChildStdin, ChildStdout, Command},
    sync::watch,
};

const CANCELLED: &str = "accounts.loginCancelled";
const FAILED: &str = "accounts.loginFailed";
const TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Default, Clone)]
pub struct LoginManager(Arc<Mutex<LoginState>>);
#[derive(Default)]
struct LoginState {
    active: Option<ActiveLogin>,
    // Bounded tombstones also cover cancel arriving before start over IPC.
    recent: VecDeque<(String, bool, Instant)>,
}
struct ActiveLogin {
    id: String,
    cancel: watch::Sender<bool>,
    done: watch::Receiver<bool>,
    committed: bool,
}
pub struct LoginAttempt {
    manager: LoginManager,
    id: String,
    cancel: watch::Receiver<bool>,
    done: watch::Sender<bool>,
}
impl LoginState {
    fn remember(&mut self, id: String, committed: bool) {
        self.recent.retain(|(_, _, at)| at.elapsed() < TIMEOUT);
        self.recent.push_back((id, committed, Instant::now()));
        while self.recent.len() > 64 {
            self.recent.pop_front();
        }
    }
}
impl LoginManager {
    pub fn begin(&self, id: String) -> Result<LoginAttempt, String> {
        validate_id(&id)?;
        let mut state = self.0.lock().map_err(|_| FAILED)?;
        state.recent.retain(|(_, _, at)| at.elapsed() < TIMEOUT);
        if state.recent.iter().any(|(old, _, _)| old == &id) {
            return Err(CANCELLED.into());
        }
        if state.active.is_some() {
            return Err("accounts.loginBusy".into());
        }
        let (cancel_tx, cancel) = watch::channel(false);
        let (done, done_rx) = watch::channel(false);
        state.active = Some(ActiveLogin {
            id: id.clone(),
            cancel: cancel_tx,
            done: done_rx,
            committed: false,
        });
        Ok(LoginAttempt {
            manager: self.clone(),
            id,
            cancel,
            done,
        })
    }
    pub async fn cancel(&self, id: String) -> Result<bool, String> {
        validate_id(&id)?;
        let mut done = {
            let mut state = self.0.lock().map_err(|_| FAILED)?;
            if let Some(active) = state.active.as_ref().filter(|active| active.id == id) {
                if active.committed {
                    return Ok(false);
                }
                let _ = active.cancel.send(true);
                active.done.clone()
            } else {
                if let Some((_, committed, _)) = state.recent.iter().find(|(old, _, _)| old == &id)
                {
                    return Ok(!committed);
                }
                state.remember(id, false);
                return Ok(true);
            }
        };
        tokio::time::timeout(Duration::from_secs(10), async {
            while !*done.borrow_and_update() {
                done.changed().await.map_err(|_| FAILED)?;
            }
            Ok::<_, &str>(())
        })
        .await
        .map_err(|_| "accounts.cancelFailed")?
        .map_err(str::to_string)?;
        Ok(true)
    }
    pub fn shutdown(&self) {
        if let Ok(state) = self.0.lock() {
            if let Some(active) = &state.active {
                let _ = active.cancel.send(true);
            }
        }
    }
}
fn validate_id(id: &str) -> Result<(), String> {
    uuid::Uuid::parse_str(id)
        .map(|_| ())
        .map_err(|_| FAILED.into())
}
impl LoginAttempt {
    pub fn cancellation(&self) -> watch::Receiver<bool> {
        self.cancel.clone()
    }
    /// Cancellation and account registration share the same short critical section.
    pub fn commit<T>(&self, register: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        let mut state = self.manager.0.lock().map_err(|_| FAILED)?;
        let active = state
            .active
            .as_mut()
            .filter(|active| active.id == self.id)
            .ok_or(FAILED)?;
        if *active.cancel.borrow() {
            return Err(CANCELLED.into());
        }
        let result = register();
        active.committed = result.is_ok();
        result
    }
}
impl Drop for LoginAttempt {
    fn drop(&mut self) {
        if let Ok(mut state) = self.manager.0.lock() {
            if state
                .active
                .as_ref()
                .is_some_and(|active| active.id == self.id)
            {
                let active = state.active.take().unwrap();
                state.remember(self.id.clone(), active.committed);
            }
        }
        let _ = self.done.send(true);
    }
}

pub struct LoginArtifact {
    pub auth_path: PathBuf,
    _home: TemporaryHome,
}
struct TemporaryHome(PathBuf);
impl Drop for TemporaryHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
pub async fn login(
    paths: &CodexPaths,
    proxy: &ApiProxyConfigPayload,
    cancel: watch::Receiver<bool>,
) -> Result<LoginArtifact, String> {
    let home = TemporaryHome(
        paths
            .accounts_dir
            .join("login")
            .join(uuid::Uuid::new_v4().to_string()),
    );
    std::fs::create_dir_all(&home.0).map_err(|_| FAILED)?;
    let mut command =
        account_monitor::codex_login_command(&["-c", "cli_auth_credentials_store='file'", "app-server"])?;
    command.env("CODEX_HOME", &home.0).env_remove("OPENAI_API_KEY").env_remove("CODEX_API_KEY");
    account_monitor::configure_network(&mut command, proxy)?;
    run_login(command, home, cancel, TIMEOUT, open_browser).await
}

async fn cancelled(cancel: &mut watch::Receiver<bool>) {
    while !*cancel.borrow_and_update() {
        if cancel.changed().await.is_err() {
            return;
        }
    }
}
type Output = Lines<BufReader<ChildStdout>>;
async fn send(stdin: &mut ChildStdin, value: Value) -> Result<(), String> {
    stdin
        .write_all(format!("{value}\n").as_bytes())
        .await
        .map_err(|_| FAILED)?;
    stdin.flush().await.map_err(|_| FAILED.into())
}
async fn next(reader: &mut Output) -> Result<Value, String> {
    loop {
        let line = reader
            .next_line()
            .await
            .map_err(|_| FAILED)?
            .ok_or(FAILED)?;
        if let Ok(value) = serde_json::from_str(&line) {
            return Ok(value);
        }
    }
}
async fn request(
    stdin: &mut ChildStdin,
    reader: &mut Output,
    id: u64,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    send(stdin, json!({"id":id,"method":method,"params":params})).await?;
    loop {
        let value = next(reader).await?;
        if value["id"].as_u64() == Some(id) {
            if value.get("error").is_some() {
                return Err(FAILED.into());
            }
            return value.get("result").cloned().ok_or_else(|| FAILED.into());
        }
    }
}
async fn run_login(
    mut command: Command,
    home: TemporaryHome,
    mut cancel: watch::Receiver<bool>,
    timeout: Duration,
    opener: impl Fn(&str) -> Result<(), String>,
) -> Result<LoginArtifact, String> {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(crate::platform::windows::CREATE_NO_WINDOW);
    let mut child = command.spawn().map_err(|_| FAILED)?;
    // A job kills this worker and its launcher descendants; the browser is opened
    // by AgentHarbor, outside the job. Closing it never closes the user's browser.
    #[cfg(windows)]
    let job = match WorkerJob::attach(&child) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(error);
        }
    };
    let mut login_id = None;
    let opener_cancel = cancel.clone();
    let result = async {
        let mut stdin = child.stdin.take().ok_or(FAILED)?;
        let mut reader = BufReader::new(child.stdout.take().ok_or(FAILED)?).lines();
        let flow = async {
            request(&mut stdin, &mut reader, 1, "initialize", json!({"clientInfo":{"name":"agent-harbor","version":env!("CARGO_PKG_VERSION")}})).await?;
            send(&mut stdin, json!({"method":"initialized"})).await?;
            let start = request(&mut stdin, &mut reader, 2, "account/login/start", json!({"type":"chatgpt"})).await?;
            let id = start["loginId"].as_str().filter(|id| !id.is_empty()).ok_or(FAILED)?.to_string();
            login_id = Some(id.clone());
            let url = start["authUrl"].as_str().ok_or(FAILED)?;
            validate_auth_url(url)?;
            if *opener_cancel.borrow() { return Err(CANCELLED.into()); }
            opener(url)?;
            loop {
                let value = next(&mut reader).await?;
                if value["method"] == "account/login/completed" && value["params"]["loginId"] == id {
                    return if value["params"]["success"] == true { Ok::<(), String>(()) } else { Err(FAILED.into()) };
                }
            }
        };
        let outcome = tokio::select! {
            biased;
            _ = cancelled(&mut cancel) => Err(CANCELLED.into()),
            result = tokio::time::timeout(timeout, flow) => result.unwrap_or_else(|_| Err("accounts.loginTimeout".into())),
        };
        if outcome.is_err() {
            if let Some(id) = &login_id {
                let _ = tokio::time::timeout(Duration::from_secs(1), request(&mut stdin, &mut reader, 3, "account/login/cancel", json!({"loginId":id}))).await;
            }
        }
        outcome
    }.await;
    #[cfg(windows)]
    let cleanup = job.stop().await;
    let _ = child.kill().await;
    let _ = child.wait().await;
    #[cfg(windows)]
    cleanup?;
    result?;
    let auth_path = home.0.join("auth.json");
    let auth = auth::load_auth_file(&auth_path).map_err(|_| FAILED)?;
    if auth
        .tokens
        .access_token
        .as_deref()
        .is_none_or(|token| token.trim().is_empty())
    {
        return Err(FAILED.into());
    }
    auth::make_auth_snapshot(&auth, &auth_path).map_err(|_| FAILED)?;
    Ok(LoginArtifact {
        auth_path,
        _home: home,
    })
}

fn validate_auth_url(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|_| FAILED)?;
    if parsed.scheme() != "https"
        || !matches!(
            parsed.host_str(),
            Some("auth.openai.com" | "chatgpt.com" | "auth0.openai.com")
        )
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.port_or_known_default() != Some(443)
    {
        return Err(FAILED.into());
    }
    Ok(())
}
#[cfg(windows)]
pub(crate) fn open_browser(url: &str) -> Result<(), String> {
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    let verb: Vec<u16> = "open\0".encode_utf16().collect();
    let url: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            url.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize <= 32 {
        Err("accounts.browserFailed".into())
    } else {
        Ok(())
    }
}
#[cfg(not(windows))]
pub(crate) fn open_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(not(target_os = "macos"))]
    let program = "xdg-open";
    std::process::Command::new(program)
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|_| "accounts.browserFailed".into())
}

#[cfg(windows)]
struct WorkerJob(usize);
#[cfg(windows)]
impl WorkerJob {
    async fn stop(self) -> Result<(), String> {
        use windows_sys::Win32::System::JobObjects::*;
        if unsafe { TerminateJobObject(self.0 as _, 1) } == 0 {
            return Err(FAILED.into());
        }
        // Termination is asynchronous. Wait for descendants before deleting credentials.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { std::mem::zeroed() };
            let ok = unsafe {
                QueryInformationJobObject(
                    self.0 as _,
                    JobObjectBasicAccountingInformation,
                    &mut info as *mut _ as *mut _,
                    std::mem::size_of_val(&info) as u32,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(FAILED.into());
            }
            if info.ActiveProcesses == 0 {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(FAILED.into());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    fn attach(child: &tokio::process::Child) -> Result<Self, String> {
        use windows_sys::Win32::System::JobObjects::*;
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(FAILED.into());
        }
        let job = Self(handle as usize);
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of_val(&info) as u32,
            ) != 0
                && AssignProcessToJobObject(handle, child.raw_handle().ok_or(FAILED)? as _) != 0
        };
        if !ok {
            return Err(FAILED.into());
        }
        Ok(job)
    }
}
#[cfg(windows)]
impl Drop for WorkerJob {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0 as _);
        }
    }
}

#[cfg(test)]
#[path = "account_login_tests.rs"]
mod tests;
