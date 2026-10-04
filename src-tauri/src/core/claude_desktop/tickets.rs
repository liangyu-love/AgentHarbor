//! Claude Desktop 直连：预览 ticket。
//!
//! 预览与提交之间需要一道「输入没变」的闸门：预览时算出的 revision 会被固化成
//! 单次消费的 ticket，提交时先消费 ticket、再重算 revision 比对。这样旧标签页、
//! 迟到的异步响应和后端重启都不会让一次过期预览落盘。
//!
//! ticket 只保存**重放所需的最小参数**（供应商 ID、模型、路径、接管选项），
//! 不含密钥；文件本身仍走 `secret_file` 写入。

use std::path::Path;

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
    pub fn issue(&self, ticket: Ticket) -> DesktopResult<()> {
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
        store.tickets.push(TicketEntry {
            id: uuid::Uuid::new_v4().to_string(),
            ticket,
        });
        self.save(&store)
    }

    /// 消费一张预览。过期、已消费或不存在的 revision 一律报「预览已失效」，
    /// 而不是被后续的 revision 比较说成「输入已变化」。
    pub fn take(&self, revision: &str) -> DesktopResult<Ticket> {
        let mut store = self.load()?;
        prune(&mut store);
        let index = store
            .tickets
            .iter()
            .position(|entry| entry.ticket.revision == revision)
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
        .retain(|entry| now - entry.ticket.created_at <= TICKET_TTL_SECONDS);
}
