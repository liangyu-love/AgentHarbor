# 会话线程恢复实施计划

## 2026-09-30 Magpie 借鉴：以可复现测试决定优化

**Status:** complete

- 范围：优先验证思考强度转换和保留内置 provider 的接入方式，不叠加第二个网关，不改真实用户会话或 Codex 配置。
- [x] 记录当前基线；核实协议文档与可用测试条件。
- [x] 使用安装的 Codex、临时 home 和隔离端点，比较固定 Router provider 与 `openai_base_url`，检查同 ID/历史、热切换和模型菜单。
- [x] 用合成内容对已配置服务做小规模真实请求；直连探针凭据只在内存，端到端 Router 测试仅使用临时加密副本；不输出密钥，不改源凭据/配置。
- [x] 仅针对已复现的问题做最小优化，新增反例与回归测试；不能以本地 mock 代替真实上游验证。
- [x] 审查新增代码的安全、历史保留、协议边界和冗余，输出测试证据与暂不采纳的方案。

结果详见 `docs/MAGPIE_VALIDATION.md`。真实官方往返、加密压缩、1M容量仍未验证，不列入本轮通过项。

## 目标

建立可靠的线程目录和单线程恢复闭环，优先解决切换供应商、Router/Native 接入方式或官方账号后历史会话不可用。

## Phase 1：权威目录与数据模型

**Status:** complete

- 使用 state SQLite threads 作为权威目录，并合并 rollout-only 记录以暴露缺目录场景。
- schema 自适应读取可选列，过滤 thread_spawn_edges 与 agent_job_items 子线程。
- 校验 rollout 路径和 thread ID，标记 available / missing。
- 数据库不可用时回退文件扫描并返回 warning。
- 前后端展示 provider、model、tokens、目录来源和 rollout 状态。

## Phase 2：统一 rollout 事件解析

**Status:** complete

- 支持 item_completed、response_item/message 和直接消息事件。
- 按角色、规范化文本、事件格式和时间窗口做一对一跨格式去重。
- 同格式不同时间的真实重复消息仍保留。

## Phase 3：恢复预览与 provider 目标解析

**Status:** complete

- 恢复目标使用当前 config.toml 的精确 model_provider key。
- openai 作为内置 provider；其他 provider 必须存在对应定义，否则 fail-closed。
- 预览列出 SQLite 与 rollout 的 provider、状态、路径和拟修改项。
- 冲突、缺文件、无效元数据和无效 provider 均禁止应用。

## Phase 4：备份、写入和回滚闭环

**Status:** complete

- Windows 上检测 Codex Desktop、CLI、app-server 和 ChatGPT 写进程。
- 使用拒绝其他写入、删除、重命名和路径替换的锁定句柄覆盖完整恢复生命周期。
- 创建完整 rollout + SQLite provider 备份和 durable journal。
- journal 阶段为 prepared → rolloutWritten → committed，失败或崩溃后可协调补偿。
- 未知文件哈希、provider 冲突和进程枚举失败均 fail-closed。
- 非 Windows 平台当前只允许预览，不开放写入恢复。

## Phase 5：前端显式恢复入口

**Status:** complete

- 所有带 thread ID 的单线程卡片都提供“检查并修复会话”入口，使 missingRollout 等后端诊断可达。
- 弹窗展示状态、目标 provider、具体修改项，以及 missingCatalog 将写入的完整冻结目录字段。
- 确认按钮只以服务端 canApply=true 为准，避免前端状态白名单与后端能力脱节。
- 缺 rollout 文案明确不承诺找回原始会话内容。

## Phase 6：综合验证与代码审查

**Status:** complete

- Rust 格式、全部 target 编译和测试二进制编译已通过。
- 前端 TypeScript/Vite 生产构建已通过。
- 多轮安全审查发现的并发覆盖、journal、provider 映射、冲突授权和去重问题已修复。
- 2026-09-25 复审发现的短写补偿、配置 generation、preview revision、全局 journal 授权、错误 rollout 回退和恢复按钮布局问题已修复。
- Rust 全目标检查、MSVC 全量 lib 测试、前端生产构建和 diff 检查重新通过。

## Phase 7：缺目录线程补建

**Status:** complete

### 7.1 安全预览分析器

**Status:** complete

- 用 `PRAGMA table_xinfo(threads)` 冻结列顺序、affinity、NOT NULL、默认值、PK 和 hidden/generated 标志。
- 只允许固定白名单列参与动态 INSERT；未知 `NOT NULL` 且无默认值的列 fail-closed。
- 请求绑定用户所见 rollout 路径；路径越界、ID 不匹配或同 ID 多候选时禁止应用。
- 从 session_meta、turn_context 和统一消息解析器构建 catalog metadata，缺必填元数据时返回明确状态。

### 7.2 Codex 字段规范映射

**Status:** complete

- 按 Codex 0.156.1 的 typed serde 结构规范化 `sandbox_policy`，包括 split filesystem、legacy workspace-write 和稳定字段顺序。
- 将 `approval_policy` 映射到 `approval_mode`，支持 granular 默认值并对未知值 fail-closed。
- 识别 parent/fork/sub-agent、guardian review、memory consolidation 和 SQLite 子线程关系证据。
- 后续非法 settings 记录不会静默回退到较早有效值，任何映射错误都会阻断目录补建。
- 复审无 P1 阻断项；路径等价与旧异常 cwd 的边界兼容保留为后续优化。

### 7.3 可回滚目录补建

**Status:** complete

- 将 backup/journal 扩展为 `CatalogMutation::InsertRow`，记录 schema signature、插入列值和完整行指纹。
- 在 `BEGIN IMMEDIATE` 内复核 schema、目标 provider、rollout 哈希和目录行仍不存在。
- 使用参数化 `INSERT` 补建 threads 行；禁止 `INSERT OR REPLACE`。
- 补偿删除必须匹配完整行指纹，外部修改后 fail-closed。
- preview 暴露实际冻结的完整目录字段，用户确认范围与真实 INSERT 范围一致。

### 7.4 测试与放行

**Status:** complete

- 覆盖当前/旧 schema、未知 nullable/default 列、未知必填列、路径冲突、schema 漂移和并发插入。
- 覆盖 Prepared、RolloutReady、CatalogCommitted 崩溃恢复和插入行外部修改冲突。
- missingCatalog 的冻结字段契约、完整行插入、指纹补偿、触发器和外键防护均有 Rust fixture 覆盖。
- Windows MSVC 实际运行 266 个 lib 测试：265 通过、1 个需显式本地连接的测试忽略；另有 9 个 local_workflows 集成测试通过。

### 7.5 损坏 journal 人工协调

**Status:** complete

- 损坏、不可读、未知版本、非法线程 ID 或非普通文件形式的 journal 返回 `JournalBlocked`，不再让预览 API 整体失败或误判为无 journal。
- preview 暴露 journal、恢复目录、可提取的备份提示和底层诊断；前端提供打开恢复目录的人工协调入口。
- 自动路径不会删除、改写或绕过无法验证的 journal，严格恢复路径继续 fail-closed。

### 7.6 官方账号切换边界

**Status:** complete

- 确认 SQLite threads、rollout session_meta 与当前认证快照之间没有可验证的线程账号归属关联。
- 新增 OfficialAccountUnverified 只读状态：本地 openai 元数据一致，但不能证明当前 ChatGPT 账号拥有服务端线程。
- 官方账号切换场景禁止自动改写；本地 rollout 仍可读取，服务端继续能力由官方账号权限决定。

### 7.7 Rollout 索引、模块拆分与浏览器回归

**Status:** complete

- 会话全量扫描会按 sessions/archived roots 建立 thread ID 到 rollout 路径的进程级索引；预览优先复用索引，失效时回退定向扫描。
- apply 始终刷新递归查找，确保预览后新增的同 ID 副本仍会触发 fail-closed。
- 将原 5,714 行 thread_recovery.rs 拆为父模块与 policy.rs、rollout.rs、journal.rs，原测试模块和覆盖保持不变。
- 使用 bundled Playwright 与本机 Chrome 增加 missingCatalog、JournalBlocked、OfficialAccountUnverified 三类浏览器交互回归。

## 后续优化

- 增加备份浏览、手动还原入口和 Windows 锁行为专项测试。
- 在 Linux CI 验证 preview-only/fail-closed 分支。

## Phase 8：批量修复与会话删除

**Status:** complete

- 会话页新增“修复全部”，串行预览并只应用服务端明确允许的修复。
- 新增单条和批量删除；缺 rollout 时只移除 SQLite 目录行，存在 rollout 时同步移除目录行和精确文件。
- 删除前检查 Codex 写进程、稳定 key、路径边界、thread ID、重复 rollout 和目录引用。
- 删除前生成 SQLite 快照、rollout 备份和 manifest；失败时恢复已经移除的文件。
- 删除与恢复共享线程变更锁，避免两类写操作并发竞争。
- 缺失原始记录提示缩减为单行状态，避免影响页面浏览。

## Errors Encountered

| Error | Attempt | Resolution |
|---|---:|---|
| GNU Rust 测试程序 STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139) | 多次定向测试 | 改用本机已安装的 MSVC Rust 工具链实际运行全量 lib 测试，262 通过、1 忽略；GNU DLL 环境问题仍保留为工具链维护项。 |
| Playwright 项目依赖中未配置 | 浏览器回归 | 复用 Codex bundled Playwright 和本机 Chrome 执行现有 fixture，新增的三类恢复交互回归均通过。 |
| GNU linker export ordinal too large | 定向 Rust 测试 | GNU 工具链链接 Tauri DLL 失败；使用项目脚本固定的 MSVC 工具链完成 266 个 lib 测试和 9 个集成测试。 |

## 2026-09-30：中转 Claude 默认 1M（用户覆盖策略）
- [x] 默认上下文与现代 store 迁移；手动值优先，输出上限不联动放大。
- [x] Claude Code 所有角色（含 Haiku）默认 1M；显式关闭持久化，切换/恢复不重开。
- [x] 模型字段解析、Rust/浏览器/安装版 Codex 隔离回归与未提交改动审查。
- 不修改真实用户配置或会话；不以短请求冒充百万 token 容量验证。
