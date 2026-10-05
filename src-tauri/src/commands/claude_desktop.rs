//! Claude Desktop 直连与本地网关配置命令。
//!
//! 与既有客户端命令一样：返回 `Result<T, String>`，错误是稳定错误码或本地化
//! key，绝不携带密钥。路由预览先绑定共享监听器，提交后协调服务需求；
//! 不改动 Provider 的 enabled / 默认模型 / 协议 / 全局 active 状态。

use crate::core::claude_desktop::{self, paths_from_state, types as desktop, Manager};
use crate::core::newapi;
use crate::core::repository::Repository;
use crate::core::router;
use crate::core::routing_runtime;
use crate::platform::paths::CodexPaths;
use std::sync::Mutex;
use tauri::{AppHandle, Manager as _, State};

/// Desktop 版本提示：来自应用自身包信息时不可靠，因此只在显式提供时使用。
fn paths(repo: &State<'_, Mutex<Repository>>) -> Result<CodexPaths, String> {
    Ok(repo
        .lock()
        .map_err(|_| "claudeDesktop.errors.storage")?
        .paths()
        .clone())
}

fn manager(app_data_dir: &std::path::Path) -> Manager {
    Manager::new(app_data_dir)
}

/// 把结构化错误序列化为前端可消费的 JSON 字符串。
fn encode(error: Box<desktop::DesktopError>) -> String {
    serde_json::to_string(&*error)
        .unwrap_or_else(|_| "{\"code\":\"storage\",\"message\":\"claudeDesktop.errors.storage\",\"retryable\":false,\"params\":[]}".to_owned())
}

async fn action_lease(
    paths: &CodexPaths,
    action: desktop::DesktopAction,
) -> Result<Option<desktop::ListenerLease>, String> {
    let needs_listener = if action == desktop::DesktopAction::Reapply {
        let data = paths.app_data_dir.clone();
        super::blocking(move || {
            Ok(Manager::new(&data)
                .load_state()
                .map_err(encode)?
                .is_some_and(|state| {
                    !state.tombstone
                        && state.data.applied_mode
                            == claude_desktop::transaction::AppliedMode::Router
                }))
        })
        .await?
    } else {
        action == desktop::DesktopAction::ApplyRouter
    };
    if needs_listener {
        routing_runtime::ensure_listener(paths).await.map(Some)
    } else {
        Ok(None)
    }
}

#[tauri::command]
pub async fn get_claude_desktop_status(
    repo: State<'_, Mutex<Repository>>,
) -> Result<desktop::DesktopStatus, String> {
    let paths = paths(&repo)?;
    tauri::async_runtime::spawn_blocking(move || {
        let store = router::load_store(&paths)?;
        let manager = manager(&paths.app_data_dir);
        Ok(manager.status(&store))
    })
    .await
    .map_err(|_| "claudeDesktop.errors.storage".to_owned())?
}

#[tauri::command]
pub async fn list_claude_desktop_candidates(
    repo: State<'_, Mutex<Repository>>,
    paths_choice: Option<desktop::PathsChoice>,
) -> Result<desktop::DesktopCandidates, String> {
    let paths = paths(&repo)?;
    let choice = paths_choice.unwrap_or_default();
    tauri::async_runtime::spawn_blocking(move || {
        let store = router::load_store(&paths)?;
        let manager = manager(&paths.app_data_dir);
        manager.candidates(&store, &choice).map_err(encode)
    })
    .await
    .map_err(|_| "claudeDesktop.errors.storage".to_owned())?
}

#[tauri::command]
pub async fn preview_claude_desktop_change(
    repo: State<'_, Mutex<Repository>>,
    action: desktop::DesktopAction,
    request: desktop::ActionRequest,
) -> Result<desktop::DesktopPreview, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    let lease = action_lease(&paths, action).await?;
    let reservation = lease
        .as_ref()
        .map(crate::core::router_proxy::reserve_listener)
        .transpose()?;
    let preview_paths = paths.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let store = router::load_store(&preview_paths)?;
        let mut manager = manager(&preview_paths.app_data_dir);
        if let Some(lease) = lease {
            manager = manager.with_listener_lease(lease);
        }
        let prepared = manager.prepare(&store, action, &request).map_err(encode)?;
        manager.issue_ticket(&prepared).map_err(encode)
    })
    .await
    .unwrap_or_else(|_| Err("claudeDesktop.errors.storage".to_owned()));
    if let Some(reservation) = reservation {
        match &result {
            Ok(preview) => {
                routing_runtime::retain_preview_listener(preview.revision.clone(), reservation)
            }
            Err(_) => crate::core::router_proxy::release_listener_reservation(&reservation),
        }
    }
    if result.is_err() {
        let _ = routing_runtime::reconcile(&paths).await;
    }
    result
}

#[tauri::command]
pub async fn commit_claude_desktop_change(
    repo: State<'_, Mutex<Repository>>,
    action: desktop::DesktopAction,
    revision: String,
    request: desktop::ActionRequest,
) -> Result<desktop::DesktopApplied, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    let lease = match action_lease(&paths, action).await {
        Ok(lease) => lease,
        Err(error) => {
            routing_runtime::release_preview_listener(&revision);
            let _ = routing_runtime::reconcile(&paths).await;
            return Err(error);
        }
    };
    let ticket_revision = revision.clone();
    let commit_paths = paths.clone();
    let mut result = tauri::async_runtime::spawn_blocking(move || {
        let store = router::load_store(&commit_paths)?;
        let mut manager = manager(&commit_paths.app_data_dir);
        if let Some(lease) = lease {
            manager = manager.with_listener_lease(lease);
        }
        manager
            .commit(&store, action, &revision, &request)
            .map_err(encode)
    })
    .await
    .unwrap_or_else(|_| Err("claudeDesktop.errors.storage".to_owned()));
    if result.is_ok() {
        routing_runtime::release_all_preview_listeners();
    } else {
        routing_runtime::release_preview_listener(&ticket_revision);
    }
    if let Err(error) = routing_runtime::reconcile(&paths).await {
        // The State commit is durable. A subsequent listener failure cannot
        // turn that generation into an uncommitted operation in the UI.
        if let Ok(applied) = &mut result {
            applied.warnings.push(error);
        }
    }
    result
}

#[tauri::command]
pub async fn cancel_claude_desktop_preview(
    repo: State<'_, Mutex<Repository>>,
    revision: String,
) -> Result<(), String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    let data = paths.app_data_dir.clone();
    let ticket_revision = revision.clone();
    super::blocking(move || {
        Manager::new(&data)
            .cancel_preview(&revision)
            .map_err(encode)
    })
    .await?;
    routing_runtime::release_preview_listener(&ticket_revision);
    routing_runtime::reconcile(&paths).await
}

/// 检测结果：列表检测始终执行；Messages 核对只在显式请求时执行。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeOutcome {
    pub bearer: claude_desktop::probe::BearerReport,
    /// `None` 表示本次没有请求 Messages 核对。
    pub messages: Option<claude_desktop::probe::MessagesReport>,
}

/// 只读检测。默认只发一次 `/v1/models` GET；只有 `checkMessages` 为真时才
/// 额外发一次 `max_tokens: 1` 的 Messages 请求。两种情况都不改动 Provider 的
/// 测试状态，也不写任何 Desktop 文件。
#[tauri::command]
pub async fn probe_claude_desktop_provider(
    repo: State<'_, Mutex<Repository>>,
    request: desktop::ProbeRequest,
) -> Result<ProbeOutcome, String> {
    let (paths, proxy) = {
        let repo = repo.lock().map_err(|_| "claudeDesktop.errors.storage")?;
        (repo.paths().clone(), repo.api_proxy_config())
    };
    tauri::async_runtime::spawn_blocking(move || {
        let store = router::load_store(&paths)?;
        let manager = manager(&paths.app_data_dir);
        let bearer = manager
            .probe(&store, &request.provider_id, &request.model_ids, &proxy)
            .map_err(encode)?;
        let messages = if request.check_messages {
            let model = request.model_ids.first().cloned().unwrap_or_default();
            let provider = store
                .providers
                .iter()
                .find(|entry| entry.id == request.provider_id)
                .ok_or_else(|| {
                    encode(Box::new(desktop::DesktopError::new(
                        desktop::error_code::PROVIDER_MISSING,
                        "claudeDesktop.errors.providerMissing",
                    )))
                })?;
            Some(
                claude_desktop::probe::run_messages_check(provider, &model, &proxy)
                    .map_err(encode)?,
            )
        } else {
            None
        };
        Ok(ProbeOutcome { bearer, messages })
    })
    .await
    .map_err(|_| "claudeDesktop.errors.storage".to_owned())?
}

/// 让 Claude Desktop 立即使用刚应用的配置：先完全退出，再启动。
///
/// 顺序由 `Manager::restart_app` 保证（Desktop 退出时会回写状态）。写操作与
/// 进程操作共用同一把跨进程锁，避免与另一个实例的配置提交交错。
///
/// 只有当配置已经提交时才应调用；不改 Desktop 配置，成功后清除内部待启动标记。
#[tauri::command]
pub async fn restart_claude_desktop(
    repo: State<'_, Mutex<Repository>>,
) -> Result<desktop::DesktopRestartResult, String> {
    let paths = paths(&repo)?;
    tauri::async_runtime::spawn_blocking(move || {
        let manager = manager(&paths.app_data_dir);
        // Stored roots are references, not process-control authorization. Recheck
        // their current candidate membership and isolation before using them.
        let desktop_paths = match manager.load_state().map_err(encode)? {
            Some(state) => {
                let saved = paths_from_state(&state);
                claude_desktop::paths::validate_session_paths(&saved).map_err(encode)?;
                if saved.isolated || saved.overridden {
                    return Err(encode(Box::new(
                        desktop::DesktopError::new(
                            desktop::error_code::PROCESS,
                            "claudeDesktop.errors.launchUnavailable",
                        )
                        .action("openManually"),
                    )));
                }
                manager
                    .resolve_paths(&desktop::PathsChoice {
                        normal_root: Some(claude_desktop::paths::display(&saved.normal_root)),
                        third_party_root: Some(claude_desktop::paths::display(
                            &saved.third_party_root,
                        )),
                    })
                    .map_err(encode)?
            }
            None => manager
                .resolve_paths(&desktop::PathsChoice::default())
                .map_err(encode)?,
        };
        let launch_method = manager.restart_app(&desktop_paths).map_err(encode)?;
        let app = manager.app_state(&desktop_paths);
        Ok(desktop::DesktopRestartResult {
            launch_method,
            running: app.running,
            restart_capable: app.restart_capable(),
        })
    })
    .await
    .map_err(|_| "claudeDesktop.errors.storage".to_owned())?
}

/// 启动恢复入口：清理过期预览 ticket，并在存在未完成事务时提示。
pub fn startup_recovery(app_data_dir: &std::path::Path) {
    let manager = Manager::new(app_data_dir);
    if let Err(error) = manager.cleanup_previews() {
        eprintln!(
            "[AgentHarbor] claude desktop preview cleanup failed: {}",
            error.message
        );
    }
    if manager.has_pending() {
        eprintln!(
            "[AgentHarbor] claude desktop has an unfinished transaction; open the card to recover"
        );
    }
}

/// `AppHandle` 版本提示入口，保留给后续把 Desktop 版本写进状态使用。
pub fn app_data_dir(app: &AppHandle) -> Option<std::path::PathBuf> {
    let state = app.state::<Mutex<Repository>>();
    state
        .lock()
        .ok()
        .map(|repo| repo.paths().app_data_dir.clone())
}

/// 让 `newapi` 的代理配置类型在签名里可见（避免未使用导入告警）。
pub fn _proxy_type_hint() -> Option<newapi::AppliedConfig> {
    None
}
