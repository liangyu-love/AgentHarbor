//! Memory optimization from the maintenance page and the tray.
use crate::core::system_memory::{self, MemoryArea, MemoryOptimizeOutcome, MemoryStatus};
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;

/// Carries the outcome of a run the tray started, for an open maintenance page.
const OPTIMIZED_EVENT: &str = "memory-optimized";

#[tauri::command]
pub async fn get_memory_status() -> Result<MemoryStatus, String> {
    super::blocking(|| Ok(system_memory::status())).await
}

#[tauri::command]
pub async fn get_memory_areas(app: AppHandle) -> Result<Vec<MemoryArea>, String> {
    super::with_repo(app, |repo| Ok(repo.memory_areas())).await
}

#[tauri::command]
pub async fn set_memory_areas(
    app: AppHandle,
    areas: Vec<MemoryArea>,
) -> Result<Vec<MemoryArea>, String> {
    super::with_repo(app, move |repo| {
        repo.set_memory_areas(&areas)
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
pub async fn optimize_memory(areas: Vec<MemoryArea>) -> Result<MemoryOptimizeOutcome, String> {
    super::blocking(move || system_memory::optimize(&areas)).await
}

/// The tray item: the saved areas, the result as a notification.
pub fn optimize_from_tray(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let outcome = match super::with_repo(app.clone(), |repo| Ok(repo.memory_areas())).await {
            Ok(areas) => super::blocking(move || system_memory::optimize(&areas)).await,
            Err(error) => Err(error),
        };
        let (title, body) = match &outcome {
            Ok(outcome) => {
                let _ = app.emit(OPTIMIZED_EVENT, outcome);
                text::optimized(outcome)
            }
            // Declining the consent prompt is an answer, not a failure.
            Err(error) if error == "maintenance.memory.errors.cancelled" => return,
            Err(error) => text::failed(error),
        };
        if let Err(error) = app.notification().builder().title(title).body(body).show() {
            log::warn!("memory optimization notification failed: {error}");
        }
    });
}

/// Notification text in the app's language.
mod text {
    use super::MemoryOptimizeOutcome;

    fn pick(zh: String, en: String) -> String {
        if crate::commands::system::ui_language_is_english() {
            en
        } else {
            zh
        }
    }

    fn bytes(value: u64) -> String {
        let gib = value as f64 / f64::from(1u32 << 30);
        if gib >= 1.0 {
            format!("{gib:.1} GB")
        } else {
            format!("{} MB", value >> 20)
        }
    }

    pub fn optimized(outcome: &MemoryOptimizeOutcome) -> (String, String) {
        let (before, after) = (&outcome.before, &outcome.after);
        let (released, freed) = (after.released_since(before), after.freed_since(before));
        let available = bytes(after.standby + after.free);
        let body = if released >> 20 == 0 && freed >> 20 == 0 {
            pick(
                format!("内存已经很干净了，可用 {available}。"),
                format!("Memory was already lean; {available} available."),
            )
        } else {
            pick(
                format!(
                    "释放占用 {}，空闲内存增加 {}，可用 {available}。",
                    bytes(released),
                    bytes(freed)
                ),
                format!(
                    "Released {} in use and freed {}; {available} available.",
                    bytes(released),
                    bytes(freed)
                ),
            )
        };
        (pick("内存优化完成".into(), "Memory optimized".into()), body)
    }

    pub fn failed(error: &str) -> (String, String) {
        let body = match error {
            "maintenance.memory.errors.busy" => pick(
                "上一次内存优化还没结束。".into(),
                "The previous memory optimization is still running.".into(),
            ),
            "maintenance.memory.errors.elevate" => pick(
                "无法以管理员身份运行内存优化。".into(),
                "Memory optimization could not run as administrator.".into(),
            ),
            _ => pick(
                "内存优化没有完成。".into(),
                "Memory optimization did not finish.".into(),
            ),
        };
        (
            pick("内存优化失败".into(), "Memory optimization failed".into()),
            body,
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn sizes_read_in_gigabytes_from_one_gigabyte_up() {
            assert_eq!(bytes(1536 << 20), "1.5 GB");
            assert_eq!(bytes(300 << 20), "300 MB");
            assert_eq!(bytes(0), "0 MB");
        }
    }
}
