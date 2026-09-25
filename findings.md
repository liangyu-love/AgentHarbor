# 会话线程恢复调查记录

## 当前实现

- SQLite threads 已作为权威目录，并合并 rollout-only 记录以暴露缺目录场景。
- 已同时过滤 thread_spawn_edges.child_thread_id 与 agent_job_items.assigned_thread_id。
- catalog 路径会核对 rollout 内 thread ID；不一致时按 ID 重新发现并返回 warning。
- Windows 上普通 providerMismatch 已具备预览、备份、journal、锁定写入和补偿闭环。

## 本机只读样本

- 后续复核时 SQLite threads 为 107 行，provider 主要为 custom，另有 3 条 openai。
- rollout 为 122 个，除 custom / openai 外还发现 aimai1。
- 第三方 provider key 不会普遍归一化为 custom。
- 多数真实 rollout 同时记录 response_item/message 和 item_completed，需要跨格式一对一去重。
- 存在 rollout 缺失、catalog 缺行、旧路径移出当前 sessions 根目录和同 ID 多副本等历史状态。

## Provider 结论

- 认证方式、上游协议、接入方式和线程持久化 provider 是不同维度。
- 恢复目标使用当前 config.toml 的精确 model_provider key。
- openai 是内置 provider；其他 provider 必须存在对应 model_providers 定义。
- 官方、Router、Native 和用户自定义线程可以共存，不在账号或供应商切换时自动批量改写。
- 只修复用户显式选择且预览判定为可应用的单个线程。

## 安全边界

- Windows 恢复全程持有拒绝其他写入、删除、重命名和路径替换的 rollout 句柄。
- SQLite 与 rollout 通过完整备份和 durable journal 协调崩溃补偿。
- 补偿只接受 original 或 updated 两个冻结哈希；未知内容保留 journal/backup 并停止覆盖。
- ProviderConflict、MissingRollout、InvalidRollout、InvalidTargetProvider 和非 Windows 写入均 fail-closed。
- missingCatalog 在 Windows 安全分析通过后可补建 SQLite 行；preview 会展示实际冻结的完整字段，确认后仍执行 schema、配置 generation、rollout 哈希和行不存在复核。

## 实施约束

- 未写入或删除真实用户 ~/.codex 数据。
- 写入测试只使用临时 Codex home 和隔离 SQLite/JSONL fixture。
- 未跟踪的 .playwright-cli 不纳入本次改动。

## 2026-09-25 会话恢复复审发现

- 原地写 rollout 的崩溃安全不能只按完整原始/目标哈希判断；本次写入产生的空文件或目标前缀可以从完整备份识别并恢复，其他未知内容必须继续拒绝覆盖。
- provider 恢复必须同时冻结 preview revision 与 config generation，避免用户确认后配置切换导致写入未确认目标。
- pending journal 是全局协调状态，但授权必须绑定 journal.thread_id，不能让一次确认修改两个线程。
- 当前 Codex threads schema 的 sandbox_policy 与 approval_mode 缺少可靠 rollout 到数据库规范映射，因此 Phase 7 只能先做预览分析，不能猜值补行。
- 不同官方账号都表现为 openai provider；账号服务端归属问题不能通过 provider 元数据修复来判断。

## 2026-09-25 目录补建放行复审

- 后端已经为 missingCatalog 返回可应用计划，前端原有状态白名单却只接受 providerMismatch/recoveryPending，导致功能不可达；现已改为单一 canApply 契约。
- 目录补建写入完整 threads 行，不能只展示 provider from/to；现已把冻结列名和值加入 preview 并在确认弹窗完整展示。
- 恢复入口原先依赖 rollout 存在，使 SQLite-only 的 missingRollout 诊断不可达；现只要求有效 thread ID。
- Windows GNU 测试二进制的 DLL 入口点故障不是断言结果；使用 MSVC 工具链完成真实运行后，260 个 lib 测试中 259 通过、1 个显式网络测试忽略。
- P2 journal 协调已完成：损坏、不可读、未知版本、非法线程 ID 或非普通文件 journal 会返回 `JournalBlocked`，保留原文件并提供恢复目录与备份提示。
- 后续 P2：rollout 按 ID 索引，以及在保持现有测试覆盖的前提下拆分过大的 thread_recovery.rs。

## 2026-09-25 收尾调查结论

- rollout 全量列表扫描现在维护按 thread ID 分组的路径索引；恢复预览复用已有索引，索引缺失、路径删除或内容失效时回退现有递归发现逻辑。
- 索引只优化预览定位，不降低写入安全性：apply 会重新扫描 sessions 与 archived_sessions，并拒绝预览后新增的同 ID 文件。
- 官方账号的认证快照可以生成账号散列，但 threads 行和 rollout session_meta 不保存可验证的账号标识，无法建立本地线程 ID 到 ChatGPT 账号的可信关系。
- 因此官方 openai 线程分为两部分：本地 rollout/SQLite 可正常读取；切换账号后能否继续服务端线程取决于官方服务端权限，当前应用既无法验证也不会伪造归属。
- 自定义 provider、Router 与 Native 的本地恢复闭环已通过完整 Rust 与浏览器回归；官方账号边界现以 OfficialAccountUnverified 明确展示并保持只读。
