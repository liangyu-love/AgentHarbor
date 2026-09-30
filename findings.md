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
# 2026-09-30 Magpie 实测优化

- 基线：已安装 Codex 0.159.0，既有 6 个隔离 app-server/Router 集成测试全部通过（合成认证＋本地上游，不是服务商实测）。
- 真实 Claude Opus 4.6 中转：当前 Responses→Messages 转换把 none/low/high 全发成 thinking enabled、budget_tokens=2048；三者 HTTP 200。原生 adaptive+low/high 均 HTTP 200/end_turn。不能从这两个简单 OK 请求推断模型内部推理强度。
- 真实 GPT-6-Astra 中转：none/low/high 三个档位本次均 HTTP 200/completed；未复现用户之前的 none 错误，因此不能把某次错误推广到所有中转。
- 新增回归先失败：none 不应开启 thinking、档位应映射到不同参数；两项已在旧实现复现。
- Context7 官方源码确认 openai_base_url 设置进入内置 provider；Anthropic SDK 参数定义确认预算下限1024且小于max_tokens，adaptive/output_config 独立。但运行时支持必须用当前客户端及具体上游验证。
- 脱敏原始结果在 .runtime-test/magpie-proof/；未更改真实配置、凭据文件或会话。

## 2026-09-30 Claude 1M 复核（只读调查，尚未实施）

- Context7 获取的 Claude 官方 context-windows 文档：Opus4.6/4.7/4.8/5、Sonnet4.6/5、Fable5/5.1、Mythos5/5.1 等默认1M，无需beta；Haiku4.5、Sonnet4.5等仍200K。官方2026-04-30发布说明旧Sonnet4/4.5的1M beta已退役。
- 对当前配置的 Claude 中转 GET /v1/models 返回HTTP200，15个Claude模型条目均未报告context_window/context_length/max_input_tokens或输出限制；只确认可列出，不证明1M可用。
- 只读源加密store：这些Claude条目contextWindow均200000、metadataSource均local，包括Opus4.6/4.7/4.8/5、Sonnet4.6/5。不能把旧本地估计解释成上游限制。
- 发现值得回归验证的读取遗漏：router::load_store_unlocked 对local/catalog只调用with_catalog_defaults；该函数本身不调用with_local_defaults。现代store中的Claude旧200K估计可能绕过已实现的1M更新逻辑，需要优先写持久化旧store加载回归。
- parse_models 当前只读context_window/context_length/max_output_tokens，没有读取Anthropic ModelInfo的max_input_tokens/max_tokens。建议补字段并区分各字段来源，不覆盖manual或上游明确限制。
- 还应让客户端上下文/目录/压缩阈值一致，按实际模型能力而非角色名决定1M；新模型默认1M不应被表述为API必须手动开beta。此轮未改业务代码或真实配置，未做大上下文生成。

## 2026-09-30：中转 Claude 1M 默认策略与取消持久化

- 旧 store 的正常加载只刷新 Codex 缓存，没补 local defaults；现在当前 schema 的旧默认也会升级。手动 metadataSource=manual 不覆盖。
- ClaudeOneM 全 false 曾不序列化；默认改为 on 后会丢失取消意图。已移除省略，补 Haiku，缺字段默认 on、显式 false 保留。
- 实际安装 Codex + 生产 Router + loopback 流式服务：Haiku 1M → runtime effective_context=950000；手动 200K → 190000，非 Claude 128K → 121600。不是只检查生成的 JSON。
- 百万 token 容量、真实官方↔中转往返及加密压缩仍未验证；本轮仅实现用户指定默认策略，没有修改真实配置或计费请求。

## 2026-09-30：官方重置卡与总体线程树
- 重置卡不是中转数据。Magpie 的 wham/usage `rate_limit_reset_credits.available_count` 与 wham/rate-limit-reset-credits 详情 GET 在当前账号真实读取成功，2 张 available 卡，与用户截图概念一致。
- 官方字段属于私有客户端接口，未提供不代表 0；详情错误必须保留次数和健康用量，不触发过期登录判定。只展示，不实现 consume。
- 详情返回的 credits 可据 granted_at 筛近 30 天，但不将它宣称为完整独立历史；当前真实样本只有 available，redeemed/expired 仅用 fixture 测试。
- 线程树使用现有 cwd 项目分组，不推测父子/子 Agent 关系；同名项目按全路径区分。原网格虚拟化按 window.innerWidth 估算，加入侧栏后错误，应与容器 CSS 列宽统一。
- 详情状态可直接由 nullable credits 表达，删除重复字符串状态；线程跨工程移动/项目消失应清理隐性定位。实际审查及新增回归已覆盖。

## 2026-09-30：线程 JSONL 可视化与维护
- cc-switch 的角色/长消息折叠和 Magpie 的累计快照/模型变更记录可借鉴；不复制源码，不引入会话数据库。
- 真实旧日志同时有 id 与 session_id，初次只读探针揭示详情与列表优先级不一致；共用解析后 3 份历史通过，原文件字节不变。
- 新 IPC 使用受限稳定 key、最终 ID 校验和有界读取；正文按需 50 条、20,000 字符，未知能力/用量不伪装成 0。
- 本轮 419 lib / 9 workflows / 6 隔离安装版 Codex 回归，完整浏览器与生产入口 smoke；仍不能替代真实官方↔中转、工具续接与实际 1M 容量验证。
- Clippy 37→26；锁定 notify-rust 的 MSRV=1.89，修正声明（实测工具链 1.97.1）。主包 React 分块约 415kB，不宣称总下载同比减少。
- 详情证据与边界：docs/THREAD_DETAILS_VALIDATION.md；保留个人临时文件，未提交/打包。
