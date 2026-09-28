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

## 阶段 4：需确认后再做

- [-] H4 conflict 出路："强制覆盖"入口涉及交互设计。
- [-] H5 Anthropic thinking 回放：需要实测上游行为。
- [-] M1 备份保留策略：需定保留数量或天数。
- [-] M5 删除 journal、M6 崩溃恢复、M11 保序写回。
- [-] D1–D5 死代码清理：会删除已注册的 IPC 命令。
- [-] C1–C6 解耦重构。
