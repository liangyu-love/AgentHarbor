# AgentHarbor 代码审查清单

> 日期：2026-09-28。依据 [CODEBASE_MAP.md](CODEBASE_MAP.md) 分 6 块只读审查（IPC 契约、客户端配置、图片工作台、Router/协议、账号/系统/前端架构、会话归档）。
> 标注：**[确认]** 读代码确认；**[推测]** 需实测验证；**[改动中]** 位于当前未提交改动。
> 修复进度见 [FIX_TODO.md](FIX_TODO.md)。

## 1. 高优先级

| ID | 问题 | 位置 | 标注 |
| --- | --- | --- | --- |
| H1 | Router 路径未过滤 `..` / `%2e%2e`，`suffix` 原样拼进上游 URL，可能带着 ChatGPT token 或供应商 key 访问上游任意路径 | `core/router_proxy.rs` 路径切分与 suffix 拼接 | 推测 |
| H2 | `set_gallery_directory` 接受任意目录，`delete_gallery_image` 不校验扩展名，可删图库根下任意文件 | `commands/image.rs` | 确认 |
| H3 | 1M 逻辑错误：Claude Code 自己识别并剥离 `[1m]` 后缀，Native 即可用；现实现强制要求 Router，且全局写/删 `CLAUDE_CODE_MAX_CONTEXT_TOKENS`、`AUTO_COMPACT_WINDOW`（该变量对带 `[1m]` 的 ID 无效，反而作用于未开 1M 的网关模型；关闭时会删掉用户自设值） | `core/client_config/claude.rs:84-105,226-237`，`claude-model-config.tsx`，`client-config-dialog.tsx` | 确认、改动中 |
| H4 | 用户改过任一受管字段后进入 conflict，重新应用和恢复都被禁用，无出路 | `core/client_config/mod.rs` `verify_fields`，`client-configs.tsx` | 确认 |
| H5 | Responses→Anthropic 只要有 `reasoning.effort` 就开 thinking，但回放历史无签名 thinking block，工具调用第二轮起可能 400；`max_tokens` 固定 4096；thinking 时仍透传 `temperature` | `core/protocol/chat.rs:58,81-93` | 推测 |
| H6 | `restart_target` 为同步 command，在主线程执行并持有 repo 锁，最长约 13s | `commands/targets.rs` | 确认 |
| H7 | 删除队列 warning 被前端按 code 全部过滤，取消/失败原因不可见 | `threads-page.tsx:105` | 确认、改动中 |
| H8 | 删除时 `read_rollout_id` 遇坏 JSONL 行即失败（扫描会跳过坏行），此类会话删不掉，队列任务永久挂起 | `core/thread_delete.rs:658` vs `core/threads.rs:1080` | 确认 |
| H9 | 切号时 auth 备份 `let _ = copy(...)` 吞掉失败后照样覆盖 `auth.json`，原凭据可能丢失 | `core/accounts.rs:235-237` | 确认 |

## 2. 中优先级

### 边界与数据安全
- M1 备份无限增长且多含明文凭据：`config-backups/`、`accounts/backups/auth-*.json`、`skill-backups/`、`client-config/backups/`（从不读取）、`thread-delete-backups/`（位于 Codex home）。
- M2 `relay-usage.jsonl` 无轮转，每次全量读取且持锁；`map_while(Result::ok)` 遇坏行后续全丢（`core/relay_usage.rs:424`）。
- M3 `open_path` 不校验路径，可执行文件会被运行（`commands/system.rs:309`）。
- M4 `save_image_file` 接受任意路径、自动建目录、覆盖已有文件、不校验图片内容，相对路径未拒绝（`commands/image.rs:525-581`）。
- M5 会话删除先删 rollout 后 commit，无 journal，崩溃留下半删状态；非 Windows 不检测 Codex writer（`core/thread_delete.rs:82,551-590`）。
- M6 客户端配置只在 `RunEvent::Exit` 恢复，崩溃后仍指向 43127；任一 `.dat` 损坏会让 `reconcile` 整体失败（`lib.rs:201`，`routing_runtime.rs:27`）。
- M7 Router 上游只有 connect 超时，上游挂住时请求/SSE 永不结束（`router_proxy.rs:171`）。
- M8 `clean` 删除 backups 不按文件名过滤，遇子目录中断（`core/repository.rs:200-216`）。
- M9 登录流程 `load_auth_file` 失败时不清理临时目录（`core/account_monitor.rs:173`）。
- M10 auth.json 解析失败时整个账号页不可用（`core/accounts.rs:79`）。

### 逻辑
- M11 `serde_json` 未开 `preserve_order`，写回客户端配置时 key 重排，`.jsonc` 注释丢失。
- M12 流式工具名分片被拼接，重复发全名的中转会得到 `lookuplookup`（`protocol/sse.rs:212`）。
- M13 `/responses/compact` 等子路径不翻译不记 usage，转 Chat 时路径改错；上游错误体不做协议转换（`router_proxy.rs:263-309`）。
- M14 `[1M]` 剥离只认大写（`router_proxy.rs:209`）。
- M15 MCP 新建同名会静默覆盖已有 server，名称未 trim（`mcp-page.tsx`，`core/mcp.rs:29`）。
- M16 AGENTS.md 为 CRLF 时管理区块残留 `\r`（`core/custom_instructions.rs:292`）。
- M17 会话筛选后选中项保留，批量删除会删到不可见会话（`threads-page.tsx:158`）。
- M18 图片旧版明文 key 在 `baseUrl` 为空时永不迁移（`image-workbench-page.tsx:427-467`）；图片凭据单槽位；生成不可取消。（已处理：见 FIX_TODO 阶段 6 G1 与阶段 10。）
- M19 账号额度串行刷新；任一账号出错整批丢弃（`core/accounts.rs:274`，`commands/accounts.rs:31`）。
- M20 加载 Claude profile 时直接 preview 后立即 apply，跳过确认（`client-config-dialog.tsx:280`）。
- M21 同一 job 内一个会话变化，整批删除被丢弃（`thread_delete.rs:276`）。
- M22 `stop()` 后立即在同端口重启可能误报 `portBusy`（`router_proxy.rs:59-112`）。

## 3. 死代码

- D1 自动切号整条链路：`set_auto_switch`、`configure_auto_switch`、`run_daemon_once`、`platform/daemon.rs` 安装逻辑、设置页阈值 mutation；`load_snapshot` 在 Windows 上每次仍起 PowerShell 查计划任务。（已处理：改为在应用内实现自动切号，见 FIX_TODO 阶段 9。）
- D2 前端无调用的已注册命令：`list_codex_threads`、`export_codex_threads`、`restart_codex`（代理逻辑已与 `restart_target` 分叉）、`load_bootstrap_state`、IPC 形式的 `focus_main_window`。
- D3 约 20 个无组件调用的 facade 方法；alias `fetchNewApiModels` 写死 `responses` 协议。
- D4 `CoreEnvelope::error` 无调用；`routable()`、`infer_protocol()`；`set_usage_refresh_interval` 的 `_interval_seconds`。
- D5 `targets.rs` 6 个 capability trait 只用到 2 个。（已处理：见 FIX_TODO 阶段 12。）
- D6 en.json 约 100+ 未引用 key；`settings.bgUploadEmpty` 缺 en，`settings.bgPresetSelected` 缺 zh。

## 4. 解耦

- C1 大组件：`image-workbench-page.tsx` 1620 行、`newapi-page.tsx` 1000、`threads-page.tsx` 979、`mcp-page.tsx` 856、`accounts-page.tsx` 819。
- C2 `router_proxy.rs` `handle()` 约 590 行。
- C3 query key、localStorage key 为散落字面量。（已处理：见 FIX_TODO 阶段 13。）
- C4 错误格式两套（i18n key / 原文），前端兜底不一致。（已处理：前端统一，见 FIX_TODO 阶段 13。）
- C5 重复实现：`table_columns` ×3、协议可翻译判断 ×2、两个客户端 adapter 的 base URL/凭据选择。（已处理：见 FIX_TODO 阶段 13。）
- C6 重 IO 同步 command 可能阻塞 UI（图库、diagnose、clean）。（已处理：见 FIX_TODO 阶段 13。）

## 5. 代码地图勘误

1. §4.5：设置页通过 `useAutoRefresh` 调用了 `get/set_usage_refresh_interval`；问题在于后端保存的值无人读取。
2. §4.3：图库无分页，仅截取前 200 条。
3. §5：三个前缀下任意子路径、任意 method 都会转发，并非"未匹配返回 404"。
4. `restart_codex` 已完全不用；`load_bootstrap_state` 的 `snapshotProgressive` 从不写入。
5. 25817 迁移会强制关闭 Router，老用户需手动重新启用。
