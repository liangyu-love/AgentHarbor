# 实施进度

## 2026-09-25

- 完成 Codex++ 报告与当前仓库实现对照，并建立文件化 TODO。
- 接入 SQLite 权威目录、rollout-only 展示、两类子 Agent 过滤和路径/thread ID 校验。
- 统一三类 rollout 消息事件，并实现跨格式一对一去重。
- 完成 provider/model/token/目录来源/rollout 状态展示。
- 完成单线程恢复预览、应用 API、确认弹窗和中英文文案。
- 恢复目标改为当前配置的精确 provider key，未知定义 fail-closed。
- 增加完整备份、durable journal、崩溃协调、补偿校验和备份裁剪。
- Windows 恢复全程持有拒绝外部修改的 rollout 句柄；非 Windows 保持 preview-only。
- 多轮审查发现的并发覆盖、journal、provider 映射、冲突授权和去重问题已修复。
- cargo check --all-targets、cargo test --lib --no-run、npm run build 和 git diff --check 已通过。
- 实际 Rust 测试执行仍被本机 STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139) 阻断。

## 2026-09-25 第一阶段复审与 Phase 7.1

- 完成第一阶段 Rust 后端、前端与接口契约复审。
- 修复短写补偿、配置漂移、预览 revision、跨线程 pending journal、错误 rollout 回退、重复 session_meta 和恢复按钮布局。
- 完成 Phase 7.1 只读 schema/metadata 安全分析器；没有开放 missingCatalog 写入。
- 验证通过：cargo fmt --check、cargo check --all-targets --locked、cargo test --lib --no-run --locked、npm run build、git diff --check。
- 实际 Rust 测试运行仍受本机 STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139) 阻断。

## 2026-09-25 Phase 7.3、7.4 与会话恢复契约修复

- 完成 CatalogMutation::InsertRow 的完整冻结行、schema signature、行指纹、durable journal、事务插入与精确补偿闭环。
- 修复 missingCatalog 后端 canApply=true、前端却拒绝确认的契约脱节；确认按钮现在直接遵循服务端 canApply。
- preview 新增 catalogFields，弹窗展示完整待插入 threads 行，并为目录补建提供独立确认和成功文案。
- 所有带 thread ID 的会话都可打开恢复预览，SQLite 行存在但 rollout 缺失时也能看到 missingRollout 诊断。
- 更新中英文恢复入口、说明、目录补建状态和成功提示，去除“写入尚未开放”的过时文案。
- GNU 测试二进制仍受 STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139) 影响；改用已安装的 Windows MSVC Rust 工具链完成真实运行。
- 验证通过：cargo fmt --all、cargo check --all-targets --locked、MSVC cargo test --lib --locked（259 passed、1 ignored）、pnpm build、locale JSON 解析和 git diff --check。

## 2026-09-25 P2 journal 人工协调

- 新增 `JournalBlocked` 专用状态，将损坏、不可读、未知版本、非法线程 ID 和非普通文件 journal 转换为可展示的恢复预览。
- preview 新增 coordination 信息，包含 journal 路径、恢复目录、可解析的备份提示和底层诊断；前端新增人工协调卡片与“打开恢复目录”入口。
- 无法验证的 journal 保持原样，自动恢复不会删除或改写；正常 journal 继续进入 `RecoveryPending` 并沿用严格补偿流程。
- 新增 malformed、unsupported version 和 non-file journal fixture；MSVC 全量串行结果为 262 passed、0 failed、1 ignored。
- 验证通过：cargo fmt --all -- --check、cargo check --all-targets --locked、pnpm build、locale JSON 解析和 git diff --check。

## 2026-09-25 会话恢复收尾优化

- 新增按 sessions/archived roots 隔离的 thread-ID rollout 索引，会话列表扫描后直接刷新索引；缓存路径失效时恢复流程自动回退递归查找。
- apply 阶段不信任预览索引，强制刷新同 ID rollout 候选；新增预览后出现重复文件的 fail-closed 回归。
- 将 thread_recovery.rs 从 5,714 行拆为 3,843 行父模块、836 行 policy、283 行 rollout 和 887 行 journal 子模块。
- 新增 OfficialAccountUnverified 状态，明确官方 openai 本地元数据正常但无法验证服务端 ChatGPT 账号归属，禁止自动应用。
- 补充 missingCatalog 完整冻结字段与确认、JournalBlocked 打开恢复目录、官方账号只读提示的浏览器交互回归。
- bundled Playwright + Chrome 回归全部通过；MSVC 串行结果为 265 passed、0 failed、1 ignored，另有 9 个 local_workflows 集成测试通过。

## 2026-09-25 批量修复与会话删除

- 新增会话页“修复全部”入口，逐个使用最新 preview revision，只应用服务端明确允许的修复。
- 新增会话卡片删除和所选会话批量删除，界面仅保留简短确认及结果提示。
- 删除后端按稳定 key 重新解析目标，拒绝重复 thread ID、路径越界、ID 漂移及所有已声明或已知目录引用。
- 缺 rollout 会话只删除 SQLite 目录行；正常会话同时删除精确 rollout 文件和目录行。
- 每次删除预先生成一致的 SQLite 快照、rollout 副本和 manifest，数据库提交失败时恢复文件。
- 删除与恢复复用全局线程变更锁，避免并发写入互相覆盖。
- 验证通过：MSVC lib 269 passed、1 ignored；local_workflows 9 passed；前端构建及 bundled Playwright + Chrome 回归通过。

## 2026-09-30 Magpie 借鉴实测与最小优化

- 完成 Codex 0.159.0 的两种接入方式对照；内置 provider 入口重启后保持原 ID/历史/完整菜单，但旧实例只改配置仍发向旧地址，且此入口实际使用 zstd 请求。
- 真实中转基线复现 none/low/high 被转换成相同2048预算；新增2个先失败的回归，拆出 reasoning 适配并保留历史/工具保护。
- 真实 Claude Opus4.6、Haiku4.5 转换后各3个请求成功；GPT-6-Astra 本次 none/low/high 都被接受，没有复现历史报错，不做跨供应商泛化。
- 真实 Codex + 生产 Router + GPT→Claude→GPT 三轮同线程成功，后两轮返回首轮随机标记。
- 全量 Rust 400 passed/1 ignored、安装版隔离集成6 passed、前端构建和全部回归通过；Clippy无错误，未批量修改既有告警。
- 审查后报告改为仅创建新文件；复用现有 app-server 测试辅助代码，未叠加网关/依赖/生产重试，保持有损历史拒绝策略。
- 实验/工具错误记录：首个新增 live 测试 fixture 缺 accessToken，修正空字段后实测通过；初次 patch 上下文与 pub mod history 不匹配，未改文件，调整后应用；单元红灯为有意记录旧缺陷。
- 详细范围和复现命令：docs/MAGPIE_VALIDATION.md。未改真实用户配置/会话，未替换应用，未提交。

## 2026-09-30：中转 Claude 默认 1M 与可持久取消

- 所有中转 Claude 默认 1M，保留手动上下文和输出 cap；补现代 store 加载刷新、Anthropic 原生 token 字段解析。
- Claude Code 启动/角色 1M 默认开启，补 Haiku；关闭后切换模型/模式、应用/重开和 profile 恢复不复活。
- 审查并清理重复前端型号表、切换重置分支、旧序列化省略判断；复用原测试基础，无新依赖。
- 405 项 Rust lib 回归、6 项安装版 Codex 隔离回归、前端构建及浏览器全量回归通过；Clippy 无错误、37 既有警告。
- 实际 Codex 运行时确认 1M/200K 分别可用 950K/190K。详见 docs/MAGPIE_VALIDATION.md 与 .runtime-test/claude-one-m/。

## 2026-09-30：提交与 Windows 打包准备

- 提交前重新运行 MSVC Rust 全量测试：405 单元通过、9 集成通过；6 项真实安装 Codex 隔离回归通过；前端构建和浏览器回归通过。
- 汇总实现与后续优化到 docs/MAGPIE_VALIDATION.md。修正 docs/RUNNING.md 的 real_codex 测试过滤，区分隔离测试与显式授权的计费测试。
- 仅提交本次源码、回归和文档；不纳入本地截图、临时实验、brainstorm.md、test-checklist.md、update_probe.js。
- 使用现有 1.0.0 Windows x64 NSIS 配置打包；不推送远程、不执行安装、不重启真实客户端。打包日志与产物校验另存 .runtime-test/release-20260930-133636/。
