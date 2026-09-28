# 修复 TODO

> 对应 [CODE_REVIEW.md](CODE_REVIEW.md)。状态：`[ ]` 未开始，`[~]` 进行中，`[x]` 完成，`[-]` 暂缓（需确认）。

## 阶段 1：未提交改动收尾

- [x] T1 (H3/M14) 1M 重做：去掉 router 限制，Native 可用；只给勾选角色加 `[1m]` 后缀（`model` 与 `ANTHROPIC_MODEL` 一致）；不再写/删 `CLAUDE_CODE_MAX_CONTEXT_TOKENS` / `AUTO_COMPACT_WINDOW`；前端解除限制并更新文案；Router 兜底剥离改为大小写不敏感。
- [x] T2 (H7) 损坏的队列记录只走 `recoveryRequired` 标记，其余队列 warning（取消/失败原因）恢复显示。

## 阶段 2：安全

- [x] T3 (H1) Router 拒绝 `.`/`..`/`%2e` 点段及编码分隔符（`%2f`、`%5c`、`\`）。
- [x] T4 (H2) 图库删除只允许图片扩展名。
- [x] T5 (M4) `save_image_file` 拒绝相对路径，写入前校验图片格式。
- [x] T6 (M3) `open_path` 要求绝对且存在的路径，拒绝可执行扩展名。

## 阶段 3：数据不丢 / 逻辑

- [x] T7 (H9) 切号前 auth 备份失败即中止。
- [x] T8 (H8) 删除读 rollout id 时跳过坏行，与扫描一致。
- [x] T9 (M2) `relay_usage` 按字节切行，坏行（含截断 UTF-8）只跳过自身。
- [x] T10 (H6) `restart_target` 改为 async + `spawn_blocking`，不再持有 repo 锁。
- [x] T11 (M7) Router 上游加 300s 读空闲超时（每个 chunk 重置）。
- [x] T12 (M15) MCP 新建时拒绝同名；后端拒绝首尾空白的名称。
- [x] T13 (M16) AGENTS.md 管理区块兼容 CRLF，写回保持原换行风格。
- [x] T14 (M17) 会话选中项随筛选/搜索收窄到可见集合。
- [x] T15 (D6) en 补 `settings.bgUploadEmpty`，删除未使用的 `bgPresetSelected`。

### 验证（阶段 1–3）

- `pnpm build`：通过。
- `cargo test --lib --test local_workflows`：lib 335 通过 / 1 ignored，集成测试 9 通过。
- `pnpm test:frontend`：30 项 PASS。
- zh/en locale key 完全对齐。

## 阶段 4（按优先级）

- [x] P1 (H4) conflict / 损坏记录出路：新增 `release_client_config`（"放弃接管"），只删除管理记录、不改配置文件，之后可基于当前文件重新配置。
- [x] P2 (M1) 备份保留：客户端配置备份、auth 备份各保留最近 20 份，会话删除备份保留最近 10 份。
- [x] P3 (M6) 单个客户端 `.dat` 损坏不再让 `reconcile` 失败、拖垮 Codex Router。
- [x] P4 (M11) `serde_json` 开启 `preserve_order`，写回客户端配置保持用户 key 顺序。
- [x] P5 (D2–D4) 删除死代码：`list_codex_threads`、`export_codex_threads`、`restart_codex`、`run_daemon_once`、`load_bootstrap_state` 与整个只写不读的 bootstrap cache、IPC 形式的 `focus_main_window`、约 20 个无调用 facade、`routable`/`infer_protocol`/`usage_refresh_interval_seconds`；`load_snapshot` 不再每次探测计划任务。
- [x] P6 (H5) Anthropic：工具循环中（最近 assistant 轮含 tool_use）不开 thinking，避免缺签名 thinking block 导致 400；开 thinking 时不透传 temperature/top_p；`max_tokens` 默认 4096 → 32000。
- [-] 未做：M5 删除 journal、D1 自动切号剩余链路（`set_auto_switch`/`configure_auto_switch`/`platform/daemon.rs` 安装逻辑）、`config-backups`/`skill-backups` 保留策略（前者被旧版配置恢复逻辑读取、后者在 UI 中可见，需要单独设计）、C1–C6 大重构。

### 验证（阶段 4）

- `cargo test --lib --test local_workflows`：lib 340 通过 / 1 ignored，集成 9 通过；`cargo check --all-targets` 无 warning。
- `pnpm build` 通过；`pnpm test:frontend` 30 项 PASS。
- P6 仅按 Anthropic 文档语义处理，未对真实上游实测。
