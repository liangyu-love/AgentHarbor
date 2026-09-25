# 会话线程恢复代码审查

## 结论

2026-09-25 复审最初结论为 `REQUEST_CHANGES`。本轮 P1 阻断项已经修复，结论更新为 `APPROVE`。Windows 已开放通过安全分析的 missingCatalog 目录补建，非 Windows 继续只开放预览。

## 本轮阻断项

- 原地截断写在崩溃或短写后可能留下既非原始哈希也非目标哈希的 rollout；必须识别本次部分写入并从完整备份恢复，同时继续拒绝其他未知内容。
- 恢复计划未绑定 config generation 和用户确认的 preview revision；配置切换后可能写入弹窗未展示的 provider。
- catalog rollout_path 指向错误线程时恢复不会正确回退；同 ID 多候选必须 fail-closed。
- 全局 pending journal 不能让任意所选线程进入可应用状态，也不能一次确认顺带修改两个线程。
- compact 卡片重复显示恢复按钮，grid 卡片缺少恢复入口。
- Rust 的 unsupportedPlatform 状态未同步到 TypeScript 和中英文文案。

以上阻断项均已修复。apply 现在必须携带 preview revision，config generation 在关键写入点重复校验；pending journal 按线程隔离，短写前缀可从完整备份补偿，其他未知内容继续 fail-closed。

## Phase 7 进展

- 已使用 `PRAGMA table_xinfo(threads)` 生成包含列顺序、affinity、NOT NULL、默认值、PK 和 hidden 信息的 schema signature。
- 未知必填无默认值列返回 `SchemaUnsupported`；缺少已验证字段返回 `InsufficientMetadata`。
- preview 提供 catalogAnalysis、完整冻结 catalogFields、schema signature、缺失字段和不支持列。
- missingCatalog 使用参数化 INSERT、完整行指纹、durable journal 和事务内复核补建 SQLite 行。
- 前端确认只遵循后端 canApply，并展示真实待写字段；目录补建和 provider 修复使用独立确认、成功文案。
- 损坏、不可读、未知版本或非普通文件 journal 返回 `JournalBlocked`；UI 展示 journal/backup 诊断并可打开恢复目录，原文件保持不变。

## 产品边界

当前恢复修复的是本地 rollout、SQLite 目录和 provider 元数据。不同官方账号都使用 `openai` provider；如果不可用来自服务端账号归属，现有 provider 比较无法识别，不能宣称已自动恢复官方账号间的服务端线程。

本轮新增 OfficialAccountUnverified，把该边界变成明确的只读产品状态。用户仍可读取本地记录，但应用不会把本地 provider 一致误报成服务端账号可继续。

## 已解决的阻断问题

- SQLite 权威目录缺失、子 Agent 混入和 catalog 路径指向错误线程。
- 三种消息事件的重复计数与重复导出。
- 把所有第三方 provider 错误折叠为 custom。
- rollout 检查与写入之间的 TOCTOU 覆盖风险。
- SQLite 与 rollout 跨资源提交的崩溃不一致窗口。
- journal 无 UI 协调入口、补偿覆盖未知内容、进程枚举失败放行。
- ProviderConflict 后端与前端授权不一致。
- 前后端恢复状态类型不同步。

## 本轮结构与性能复审

- thread ID 索引由完整会话扫描原子替换，按 roots 隔离并限制缓存组数；删除和重复文件均有 fixture。
- 恢复预览可以使用索引，apply 始终全量刷新，保留同 ID 多候选 fail-closed 语义。
- policy、rollout、journal 已拆为子模块，父模块保留公开 API、计划编排与测试，未改变序列化契约。
- 浏览器 fixture 已覆盖目录补建、人工 journal 协调和官方账号边界。

## 保留问题

- 官方 API/服务端未提供可由本地安全验证的线程账号 provenance，跨官方账号继续线程仍不能自动修复。
- GNU linker 仍有本机 export ordinal too large，但 MSVC 工具链已完成真实全量测试。
- Linux preview-only/fail-closed 分支仍应进入 CI。

## 验证

- cargo fmt --all -- --check
- cargo check --all-targets --locked
- node scripts/tauri.mjs --test-rust -- --test-threads=1
- pnpm build
- pnpm test:frontend（bundled Playwright + Chrome）
- git diff --check

以上检查均通过；MSVC 全量 lib 测试串行实际运行结果为 265 passed、0 failed、1 ignored，9 个 local_workflows 集成测试全部通过。
