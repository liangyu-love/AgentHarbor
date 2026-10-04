//! Claude Desktop 直连：预览 ticket。
//!
//! 预览与提交之间需要一道「输入没变」的闸门：预览时算出的 revision 会被固化成
//! 单次消费的 ticket，提交时先消费 ticket、再重算 revision 比对。这样旧标签页、
//! 迟到的异步响应和后端重启都不会让一次过期预览落盘。
//!
//! ticket 只保存**重放所需的最小参数**（供应商 ID、模型、路径、接管选项），
//! 不含密钥；文件本身仍走 `secret_file` 写入。

use std::path::Path;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::core::secret_file;

use super::constants::{MAX_PENDING_TICKETS, TICKET_TTL_SECONDS};
use super::transaction;
use super::types::{error_code, ActionRequest, DesktopAction, DesktopError, DesktopResult};

/// 一张已固化的预览。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ticket {
    pub revision: String,
    pub action: DesktopAction,
    pub paths_identity: String,
    pub created_at: i64,
    pub request: ActionRequest,
    #[serde(default)]
    pub process_epoch: String,
}

/// 命令层每次创建 Manager，进程 epoch 必须在整个应用进程内共享。
pub fn process_epoch() -> &'static str {
    static EPOCH: OnceLock<String> = OnceLock::new();
    EPOCH.get_or_init(|| uuid::Uuid::new_v4().to_string())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TicketStore {
    #[serde(default)]
    tickets: Vec<TicketEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TicketEntry {
    id: String,
    ticket: Ticket,
}

/// ticket 集合；每次操作都读写同一个文件，因此所有方法都接收路径。
pub struct Tickets<'a> {
    path: &'a Path,
}

impl<'a> Tickets<'a> {
    pub fn new(path: &'a Path) -> Self {
        Self { path }
    }

    /// 是否存在未消费的预览（用于状态提示）。
    pub fn has_open(&self) -> bool {
        self.load()
            .map(|mut store| {
                prune(&mut store);
                !store.tickets.is_empty()
            })
            .unwrap_or(false)
    }

    /// 记录一张预览。超过上限时返回可重试错误而不是静默丢弃。
    pub fn issue(&self, ticket: Ticket) -> DesktopResult<String> {
        let mut store = self.load()?;
        prune(&mut store);
        let open = store
            .tickets
            .iter()
            .filter(|entry| entry.ticket.paths_identity == ticket.paths_identity)
            .count();
        if open >= MAX_PENDING_TICKETS {
            return Err(Box::new(
                DesktopError::new(
                    error_code::TOO_MANY_PREVIEWS,
                    "claudeDesktop.errors.tooManyPreviews",
                )
                .retryable(),
            ));
        }
        let id = uuid::Uuid::new_v4().to_string();
        store.tickets.push(TicketEntry {
            id: id.clone(),
            ticket,
        });
        self.save(&store)?;
        Ok(id)
    }

    /// 消费一张预览。过期、已消费或不存在的 revision 一律报「预览已失效」，
    /// 而不是被后续的 revision 比较说成「输入已变化」。
    pub fn take(&self, revision: &str) -> DesktopResult<Ticket> {
        let mut store = self.load()?;
        prune(&mut store);
        let index = store
            .tickets
            .iter()
            .position(|entry| entry.id == revision)
            .ok_or_else(|| {
                Box::new(
                    DesktopError::new(
                        error_code::PREVIEW_EXPIRED,
                        "claudeDesktop.errors.previewExpired",
                    )
                    .retryable()
                    .action("repreview"),
                )
            })?;
        let entry = store.tickets.remove(index);
        self.save(&store)?;
        Ok(entry.ticket)
    }

    /// 一次提交后作废同一目标的所有预览，避免旧表单继续用旧 ticket。
    pub fn consume_all(&self, paths_identity: &str) -> DesktopResult<()> {
        let mut store = self.load()?;
        store
            .tickets
            .retain(|entry| entry.ticket.paths_identity != paths_identity);
        self.save(&store)
    }

    /// 启动时清理过期项。
    pub fn prune_expired(&self) -> DesktopResult<()> {
        let mut store = self.load()?;
        prune(&mut store);
        self.save(&store)
    }

    fn load(&self) -> DesktopResult<TicketStore> {
        let Some(bytes) = secret_file::read(self.path).map_err(|_| transaction::storage_error())?
        else {
            return Ok(TicketStore::default());
        };
        if bytes.is_empty() {
            return Ok(TicketStore::default());
        }
        serde_json::from_slice(&bytes).map_err(|_| transaction::storage_error())
    }

    fn save(&self, store: &TicketStore) -> DesktopResult<()> {
        let bytes = serde_json::to_vec(store).map_err(|_| transaction::storage_error())?;
        secret_file::write(self.path, &bytes).map_err(|_| transaction::storage_error())
    }
}

fn prune(store: &mut TicketStore) {
    let now = chrono::Utc::now().timestamp();
    store
        .tickets
        .retain(|entry| entry.ticket.process_epoch == process_epoch()
            && now.checked_sub(entry.ticket.created_at)
                .is_some_and(|age| (0..=TICKET_TTL_SECONDS).contains(&age)));
}


#[cfg(test)]
mod tests {
    use super::*;

    fn ticket() -> Ticket {
        Ticket {
            revision: "identical-private-input-revision".into(), action: DesktopAction::ApplyDirect,
            paths_identity: "temporary-roots".into(), created_at: chrono::Utc::now().timestamp(),
            request: ActionRequest { provider_id: None, models: vec![], paths: Default::default(), rebaseline: false },
            process_epoch: process_epoch().to_owned(),
        }
    }

    #[test]
    fn tickets_from_a_previous_process_epoch_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("previews.dat");
        let store = Tickets::new(&path);
        let mut old = ticket();
        old.process_epoch = "terminated-process-epoch".into();
        let id = store.issue(old).unwrap();
        assert_eq!(store.take(&id).unwrap_err().code, error_code::PREVIEW_EXPIRED);
        assert!(!store.has_open());
        store.prune_expired().unwrap();
        assert!(store.load().unwrap().tickets.is_empty());
    }

    #[test]
    fn identical_new_previews_do_not_revive_a_consumed_ticket() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("previews.dat");
        let store = Tickets::new(&path);
        let first = store.issue(ticket()).unwrap();
        store.take(&first).unwrap();
        let second = Tickets::new(&path).issue(ticket()).unwrap();
        assert_ne!(first, second);
        assert_eq!(store.take(&first).unwrap_err().code, error_code::PREVIEW_EXPIRED);
        assert_eq!(store.take(&second).unwrap().revision, "identical-private-input-revision");
    }

    #[test]
    fn extreme_and_future_ticket_times_expire_without_overflow() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("previews.dat");
        let store = Tickets::new(&path);
        for created_at in [i64::MIN, i64::MAX, chrono::Utc::now().timestamp() + TICKET_TTL_SECONDS] {
            let mut invalid = ticket();
            invalid.created_at = created_at;
            let id = store.issue(invalid).unwrap();
            assert!(!store.has_open());
            assert_eq!(store.take(&id).unwrap_err().code, error_code::PREVIEW_EXPIRED);
            store.prune_expired().unwrap();
            assert!(store.load().unwrap().tickets.is_empty());
        }
        let id = store.issue(ticket()).unwrap();
        assert!(store.has_open());
        store.take(&id).unwrap();
    }
}
