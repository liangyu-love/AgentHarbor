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

## 阶段 5：小项

- [x] S1 (M8) `clean` 只删除 AgentHarbor 写入的 `auth-*.json` / `registry-*.json`，跳过其他文件与子目录。
- [x] S2 (M9) 登录流程 `load_auth_file` 失败时也清理临时目录。
- [x] S3 (M10) `auth.json` 损坏时账号列表、切换、删除仍可用。
- [x] S4 (M12) 中转每个 chunk 重复发送完整工具头（同 id + 同名）时不再拼接名字；无 id 的名字分片仍按原逻辑拼接。
- [x] S5 (M20) 加载 profile 只生成预览，由用户点"应用"确认。
- [x] S6 (M22) Router 重启时对 `AddrInUse` 短暂重试（约 0.5s），端口被其他进程持有时仍报 `portBusy`。
- [x] S7 (D4) 删除未使用的 `CoreEnvelope::error`。
- [x] S8 (D6) 删除 105 个静态扫描确认未引用的 locale key（排除模板拼接前缀和复数后缀），中英文仍完全对齐。
- [x] S9 更新 `CODEBASE_MAP.md`：删除已移除的命令，补充 `release_client_config`、备份保留、路径校验，并入 5 处勘误。

## 阶段 6：需要设计的项（按优先级）

- [x] G1 (M18a) 图片旧版明文 key：无论迁移成功、失败，还是 `baseUrl` 为空，加载后都从 localStorage 清除明文。`baseUrl` 为空时 key 只留在本次输入框里。
- [x] G2 (M19) 账号额度：单个账号凭据回写失败只把该账号标为 `error`，其余结果照常保存；刷新改为最多 3 个账号并发（每个 RPC 使用独立的 `CODEX_HOME`）。
- [x] G3 (M21) 删除队列：只保留发生变化的会话并在 warning 里列出 key，同批其余会话照常删除。
- [x] G4 (M2) `relay-usage.jsonl`：超过 16MB 时把 35 天前的记录折叠成按供应商/模型的汇总行，all-time 与分供应商/模型统计保持精确；读取时只在读文件期间持锁，解析和聚合不持锁。
- [x] G5 (M13) 上游非 2xx 错误体转换为调用方协议的错误格式（Anthropic `{"type":"error",...}` / OpenAI `{"error":{...}}`），Content-Type 统一为 JSON；需要翻译时，`/responses/*` 子路径（如 `compact`）返回 501，不再改写成不存在的 Chat 端点。
- [-] G6 (M1) `config-backups/`：暂不清理。`router_config_snapshot::recover_legacy` 会扫描整个目录，为旧版安装寻找唯一的路由前基线，多个候选互相冲突时会故意报错。删除任意备份都可能删掉基线，或把“有歧义”变成“看似唯一”，导致恢复写入错误的值。需要先给基线加标记，再做保留策略。
- [-] M18b/c 图片凭据多槽位、生成取消：涉及交互设计，留到图片页重构（C1）一起做。

## 阶段 7：崩溃 / 关机后的客户端配置（M6）

- [x] R1 现状（读 tao 0.34.8 / tauri-runtime-wry 2.10.1 源码确认）：
  - 托盘退出：`app.exit(0)` 会触发 `RunEvent::Exit`，已清理。
  - Windows 关机/注销：tao 收到 `WM_ENDSESSION` 后调用 `loop_destroyed()`，最终触发 `RunEvent::Exit`，已清理。
  - Unix SIGTERM/SIGHUP/SIGINT：没有处理，进程直接退出，不清理。
  - 强杀或崩溃：不清理。下次启动时 `reconcile` 会重新拉起 Router，客户端恢复可用；但端口被其他进程占用时，客户端会一直指向那个进程，并把本地 token 发给它。
- [x] R2 修复：
  - Unix 下监听 SIGTERM/SIGHUP/SIGINT，Windows 下监听 Ctrl+C，收到后都调用 `app.exit(0)` 走正常清理。
  - 启动时如果端口被占（`portBusy`），把指向本地路由的客户端恢复原样，并撤销 Codex 的路由注入。
  - 退出清理的逻辑抽成 `routing_runtime::release_router_clients`。
- [x] R3 测试与文档：
  - 新增集成测试：客户端以 router 模式应用后，端口被其他 listener 占用，启动协调应把 `settings.json` 恢复成原始字节。
  - 新增 `global_router_test_lock`，让操作全局 Router 单例的测试串行执行，修复了新测试和代理测试并行时的竞态。
  - `CODEBASE_MAP.md` 启动链一节已更新。
- 未验证：Unix 信号处理的代码无法在本机编译检查（交叉编译 Linux 需要 C 工具链，本机没有）。只对照 tokio 1.50 源码核对了 API 签名，需要在 macOS/Linux 上构建确认。
- 仍未覆盖：`kill -9`、任务管理器结束进程、崩溃后端口空闲的情况。这些情况下下次启动会恢复可用，但在重新启动之前，客户端会一直连接失败。

### 验证（阶段 6）

- `cargo test --lib --test local_workflows`：lib 348 通过 / 1 ignored，集成 9 通过；`cargo check --all-targets` 无 warning。
- `pnpm build` 通过；`pnpm test:frontend` 30 项 PASS。
- 新增测试：多账号其中一个凭据回写被拒、同批删除部分会话变化、usage 账本压缩前后统计一致且可重复压缩、三种上游错误体转换。
- G1（前端迁移逻辑）没有专门的自动化测试，只通过了构建与现有回归。

### 验证（阶段 5）

- `cargo test --lib --test local_workflows`：lib 344 通过 / 1 ignored，集成 9 通过。
- `pnpm build` 通过；`pnpm test:frontend` 30 项 PASS（profile 用例已改为"先预览、再应用"）。

### 验证（阶段 4）

- `cargo test --lib --test local_workflows`：lib 340 通过 / 1 ignored，集成 9 通过；`cargo check --all-targets` 无 warning。
- `pnpm build` 通过；`pnpm test:frontend` 30 项 PASS。
- P6 仅按 Anthropic 文档语义处理，未对真实上游实测。
