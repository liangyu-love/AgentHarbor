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
- [-] 未做（M5 已在阶段 8、D1 已在阶段 9 完成）：`config-backups`/`skill-backups` 保留策略（前者被旧版配置恢复逻辑读取、后者在 UI 中可见，需要单独设计）、C1–C6 大重构。

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
- [x] G6 (M1) `config-backups/`：见阶段 11。
- [x] M18b/c 图片凭据多槽位、生成取消：见阶段 10。

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

## 阶段 8：会话删除 journal（M5）

- [x] J1 删除 rollout 前写 `thread-delete-journal/<uuid>.json`（catalog id + 原路径/备份路径），成功或已回滚后删除。
- [x] J2 列表加载和每次删除前先恢复遗留 journal：catalog 行仍在 → 未提交，从备份补回缺失 rollout；行已删除 → 已提交，只删 journal。无 catalog 行时文件全删视为完成，部分删除则补回。恢复失败的 journal 保留并以 `THREAD_DELETE_QUEUE` 警告显示；存在未恢复 journal 时暂停备份清理，避免删掉它需要的备份。
- [x] J3 macOS/Linux 删除与队列也检测 Codex writer（原来只有 Windows）。
- [x] J4 测试：未提交崩溃恢复、已提交崩溃、纯文件部分删除、备份丢失时 journal 保留，正常删除不留 journal。
- 未验证：macOS/Linux 的 writer 检测路径本机无法运行（仅 Windows 编译测试）；真实进程崩溃未模拟，测试通过手工构造 journal 覆盖崩溃点。

## 阶段 9：自动切号（D1 → 实现）

方案（已与用户确认）：应用内后台任务，不用系统计划任务；触发后切换 `auth.json`，Codex 桌面端在运行时重启它，并发系统通知。

- [x] A1 `core/auto_switch.rs` 纯决策：当前账号 5h 或每周剩余 ≤ 阈值时触发；候选账号需本轮刷新成功、各窗口都高于阈值，按最紧的窗口剩余取最大；切换后 10 分钟冷却；无候选时每轮额度耗尽只通知一次。
- [x] A2 后台循环：按"刷新频率"设置（最少 60s）刷新全部账号 → 决策 → 在同一把仓库锁内切换 → 锁外重启正在运行的 Codex → 通知 + `codex-accounts-changed` 事件。刷新逻辑与 `refresh_codex_accounts` 共用。
- [x] A3 开关：`set_auto_switch(true)` 放开；状态 `supported: true`、`enabled` 取配置。
- [x] A4 删除 `daemon-run-once` 入口、schtasks/launchd 安装与状态探测、`launch_agent_path`/`auto_switch_*` 路径、transient state 与 `serviceState`/`serviceLabel` 字段；`platform/daemon.rs` 只保留切换开关时清理旧版计划任务。
- [x] A5 前端：补齐缺失的 `settings.autoSwitch*` 文案，更新阈值说明；监听事件刷新账号列表和状态。
- [x] A6 测试：决策函数 7 个单测、检查间隔单测、开关与阈值往返单测。
- 新增依赖：`tauri-plugin-notification =2.3.3`（2.5.0 需要 tauri ^2.12，与当前 2.10 不兼容）；Cargo.lock 只新增包，不升级已有包。
- 附带修复：设置页"保存阈值"原来会把 `enabled` 乐观置为 true、失败无提示；现在以后端返回为准并提示错误，启用时先存阈值再开开关。
- 未验证：没有用真实多账号跑后台切换（需要真实 ChatGPT 账号和额度耗尽场景）；系统通知只在 Windows 编译，未在 macOS/Linux 上看过弹出效果；macOS 通知在开发构建中可能因未签名不显示。

## 阶段 10：图片凭据多槽位与取消生成（M18b/c）

- [x] I1 `image-workbench-key.dat` 改为按站点保存的列表（最多 20 个，最近使用在前），兼容读取旧的单对象格式；新增 `list_image_key_sites`、`delete_image_key`。
- [x] I2 生成改为 async reqwest（含下载与 DNS 校验），`generate_images` 带 `jobId`，新增 `cancel_image_generation`：取消时丢弃请求 future，连接立即关闭并释放并发名额。
- [x] I3 前端：站点输入框提供已保存站点下拉、"忘记密钥"按钮；生成中画布可取消。
- [x] I4 测试：多站点共存、旧格式兼容、删除、上限；取消会中断挂起的上游请求。
- 界面：站点地址输入框用 `<datalist>` 提示已保存的站点；有已存密钥时显示"忘记此站点的密钥"；生成中画布右上角有停止按钮，取消后显示"已取消"而不是"失败"。
- 未验证：没有对真实图片供应商测过取消（本地用一个只接受连接不回应的 socket 验证，取消 < 4 秒返回、名额释放）；取消只断开本地连接，供应商侧是否停止计费取决于对方实现。

## 阶段 11：`config-backups/` 基线标记与保留（G6）

- [x] B1 新写入的备份命名为 `agentharbor-<时间戳>-<uuid>.toml`，只保留最近 30 份。
- [x] B2 旧版写下的 `<uuid>.toml` 是 `recover_legacy` 寻找路由前基线的唯一输入：永不清理，也不再新增。这样新备份既不会删掉基线，也不会把"有歧义"变成"看似唯一"。
- [x] B3 `prune_backups` 在 mtime 相同时按文件名倒序，保证同一时刻写入的多份备份里留下最新的。
- [x] B4 测试：35 次写入后只剩 30 份新备份、最新一份保留、旧版 uuid 备份和无关文件不动；5 个 legacy 恢复测试改为先把新备份改名成旧格式来模拟旧安装。
- 行为变化：新版本自己的备份不再参与 legacy 恢复。只有从未写过 sidecar 的旧安装才会走这条路径，而它们的基线本来就只存在于旧文件中。

## 阶段 12：`targets.rs` trait 精简（D5）

- [x] T1 删除没有调用方的 `RoutingTarget`、`McpConfigStore`、`SkillStore`、`InstructionStore` 及对应 registry 查询；保留实际使用的 `ConversationSource`（会话页）和 `ProcessController`（重启 Codex、自动切号），并去掉其中未使用的 `target_id`/`is_running`。MCP、Skills、指令直接用 `CodexPaths`，与现状一致。
- 验证：lib 367 通过 / 1 ignored，集成 9 通过；`cargo check --all-targets` 无 warning。

## 阶段 13：解耦重构（C1–C6）

顺序（已与用户确认）：C5 → C6 → C3 → C4 → C2 → C1。

- [x] C5 合并重复实现：
  - `table_columns` 三份合一为 `threads::table_columns`（`thread_delete`、`thread_recovery`、`thread_recovery/journal` 共用）。原 `thread_recovery` 版本没有给表名加引号，现在统一转义；错误信息不再带 "SQLite error:" 前缀。
  - 协议可达判断统一为 `Translation::can_serve(from, to)`（同协议直通或受支持的翻译），替换 `router.rs`、`router_proxy.rs`、`client_config/options.rs` 三处各自的写法。
  - Claude Code / OpenCode adapter 的 base URL + 凭据选择合并为 `client_config::options::client_endpoint`。
  - 附带修复阶段 10 新增的取消测试偶发卡死（10 次约 2 次）：fixture 的 `accept()` 改为带 deadline 的轮询、固定走直连（不经系统代理），并等上游真正收到连接后再取消。连续 13 次全量 lib 测试通过。
- [x] C6 重 IO 同步 command 移出主线程：图库 6 个（保存/另存/设目录/列表/读原图/删除）、Skills 6 个（列表/备份列表/导入/删除/恢复/删除备份）、`clean`、`rebuild_registry`、`diagnose`、`export_newapi_providers`。新增 `commands::blocking`（spawn_blocking）与 `commands::with_repo`（仅在后台线程持有仓库锁）。前端调用签名不变。
  - 未改：MCP、自定义指令、账号列表/切换等只读写单个小文件的 command，仍是同步；`load_snapshot` 也保持同步（启动时就要，且只读 registry）。
- [x] C3 新增 `src/lib/query-keys.ts`（14 个 TanStack Query key）与 `src/lib/storage-keys.ts`（12 个 localStorage key），替换 23 个文件里的字面量。存储值保持不变，已保存的偏好照常读取；`index.html` 启动脚本读 `theme` 早于 bundle，仍用字面量（注释已说明）。
- [x] C4 统一前端错误处理（`src/lib/invoke-error.ts`）：
  - 约定：command 以字符串 reject，要么是 i18n key（`newapi.*`/`clients.*`/`image.*`），要么是原始文本（OS/SQLite/上游）。
  - `errorMessage` 取出文本（去掉 `Error: ` 前缀）；`errorKey(error, prefixes, fallback)` 供存 key 的表单使用；`formatInvokeError` 翻译已知 key、原样显示原始文本、未知 key 或空错误回退到通用文案。
  - 替换了 19 处 `String(error)` 和 5 个各页面自带的 `errorKey`/`errorMessage` 副本；之前 `String(new Error(key))` 会把未翻译的 `Error: newapi.errors.x` 直接显示给用户。
  - 后端没有改：把所有 `CoreError` 改成 key 改动面太大，且原始文本对排查有用；前端现在两种都能正确显示。
  - 测试：前端回归新增 10 个用例（key 翻译、原始文本保留、未知 key/空值回退、前缀过滤）。
- [x] C2 `router_proxy::handle()` 从约 620 行拆到 123 行，只保留流程：`forwardable_headers`、`requested_model`、`codex_catalog_response`、`resolve_destination`、`upstream_suffix`、`relay_usage_request`、`provider_target`、`official_target`、`upstream_body`、`client_for`、`relay_response`、`translated_stream`。逻辑逐段搬移，未改行为；`relay_response` 的 if/else 链改成按 `(translation, status, stream)` 的 match，两条非翻译分支合并（observer 为 `Option`）。lib/集成测试各连续 3 次通过。
- [ ] C1 拆分大组件（每页单独提交，拆前后截图逐像素对比，不新增 DOM 层级）：
  - [x] V0 `scripts/visual-snapshot.mjs capture|compare`：9 个页面 × 亮/暗 18 张，禁用动画、固定时间与随机数，按解码后像素逐个比较。fixture 新增 `mcp`、`accounts` 模式。验证：无改动时两次截图完全一致；给 `.ink-scroll-frame` 加 1px 边距能检测到差异。
  - [x] P1 `image-workbench-page.tsx` 1684 → 94 行：3 个 hook（`use-image-connection` 连接与凭据、`use-image-gallery` 画册、`use-generation-queue` 生成队列）+ 7 个组件（`workbench-header`、`generation-form`、`prompt-library`、`ink-model-combobox`、`generation-canvas`、`gallery-panel`、`gallery-thumbnail`）+ `workbench-types`。JSX 原样搬移，未加层级；快照新增生成画布、画册、放大预览 3 个交互状态（基线取自拆分前代码），24 张全部逐像素一致，前端回归 32 项通过。
  - [x] P2 `newapi-page.tsx` 1000 → 154 行：`use-provider-state`（查询 + 单飞 mutation、余额、导入导出）、`use-relay-view`（筛选、别名、组内切换与展开）、`relay-groups`（分组与样式工具）+ `codex-access-card`、`relay-list`、`relay-card`、`group-alias-dialog`、`status-dot`。快照新增展开、切换成员、筛选、别名、网络菜单、删除确认、重启确认 7 个交互状态（基线取自拆分前代码），38 张逐像素一致，前端回归 32 项通过。
  - [x] P3 `threads-page.tsx` 984 → 118 行：`use-thread-list`（扫描查询、筛选排序、选择、折叠、批量修复/删除/导出）+ `threads-header`、`thread-notices`、`thread-metrics`、`thread-toolbar`、`selection-bar`、`thread-group-section`。快照新增网格、全选、组选、折叠、搜索无结果、排序菜单、队列恢复 7 个状态；同时修掉快照 harness 的遮罩层偶发像素抖动（软件光栅 + 去 backdrop-filter + 状态间清 localStorage）。52 张逐像素一致，前端回归 32 项通过。
  - [x] P4 `mcp-page.tsx` 868 → 165 行：`use-mcp-servers`（查询、开关/删除 mutation、搜索筛选分页）+ `mcp-header`、`mcp-stats`、`mcp-toolbar`、`mcp-server-card`、`mcp-editor-dialog`、`mcp-form-fields`。快照新增添加、编辑 HTTP、删除确认、筛选、搜索无结果、ENV 提示 6 个状态，64 张逐像素一致，前端回归 32 项通过。
  - [ ] P5 `accounts-page.tsx`（821 行）

### 验证（阶段 10）

- `cargo test --lib --test local_workflows`：lib 366 通过 / 1 ignored，集成 9 通过；`cargo check --all-targets` 无 warning。
- `pnpm build` 通过；`pnpm test:frontend` 31 项 PASS（新增运行中画布取消用例）。

### 验证（阶段 9）

- `cargo test --lib --test local_workflows`：lib 362 通过 / 1 ignored，集成 9 通过；`cargo check --all-targets` 无 warning。
- `pnpm build` 通过；`pnpm test:frontend` 30 项 PASS。

### 验证（阶段 8）

- `cargo test --lib --test local_workflows`：lib 353 通过 / 1 ignored，集成 9 通过；`cargo check --all-targets` 无 warning。

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
