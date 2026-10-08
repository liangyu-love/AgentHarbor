# Magpie 请求诊断与能力预览补充计划

日期：2026-10-02
状态：独立后续计划，按用户后续指令实施；验收记录见 `docs/magpie-follow-up-implementation.md`
关联计划：`plan/models-dev-integration-execution-plan.md`（正在实施）
参考源码：本地 `D:/workspace/temp/magpie`，提交 `7a3337dd25bd759b1a73a051cdefeab6033b63f6`

## 1. 与正在实施计划的关系

这是 models.dev 主计划之外的一份补充计划。主计划继续按其现有范围和顺序实施；本文件不替换、不回写、不重新排序主计划。用户后续明确要求实施本计划，因此已在独立分支接入本文范围。

实施前先确认了主计划落地后的代码状态和文件所有权；本计划仍不授权修改路由行为或客户端配置注入策略。

## 2. 目标

在保持现有路由选择、故障转移、协议转换和客户端注入行为的前提下，补足请求级对账和能力匹配解释：

1. 同时记录 AiMaMi 发出的模型 ID 与上游响应自报的模型 ID，提示可观察到的应答身份差异；上游漏报、回显请求名或伪报时无法证明实际由哪个模型执行。
2. 把请求归因到当次实际发出的凭据；明确区分 AiMaMi 使用的凭据与中转商内部账号。
3. 解释 provider 选择和故障转移的真实原因，帮助用户排查路由。
4. 为已匹配的目录记录展示可选的 `canonical_model_id` 关联身份，并展示覆盖清除后的下一生效来源。
5. 保留 typed reasoning controls、`interleaved.field` 等元数据，只有经过对应客户端/协议契约确认后才用于适配。
6. 所有诊断只观察请求和响应，不修改 wire model、请求体、endpoint、认证、协议、候选集合或配置注入。

## 3. Magpie 调研基线与边界

结论按本地 Magpie 提交 `7a3337dd25bd759b1a73a051cdefeab6033b63f6` 核实：

- Magpie 的网关 trace 和 usage ledger 记录 requested/served model；`sniff.go` 从 JSON 与 SSE 响应中提取模型名，`served.go` 归一化一部分供应商前缀、区域和版本尾缀后判断是否换模。
- 账号/key 候选 trace 可记录身份类别、名称或掩码；key 指纹用于区分 key。但长期 usage record 没有 key/账号归因，也没有显式 `key not recorded` 标记。因此不能将“每条 Magpie 历史账目都含 key 指纹”当作事实。
- `stays=auto/session/turn/off` 会改变候选顺序；组内共同 effort、最小 context 和接近 95% 窗口后的扩容也会影响路由。这些是独立路由功能。
- 在该本地提交中未找到通用 `X-Magpie-Account` pin header；Codex 专用 `chatgpt-account-id` 流程不能代表通用 pin API。
- `MAGPIE_DEBUG=1` 只输出请求摘要，没有证据表明它记录完整请求和响应。
- provider 的模型探测使用实际暴露的模型 ID 与能服务该模型的 key/endpoint；没有通用 `wire_id` 配置字段。
- Magpie 本地 catalog/matcher 未消费 `canonical_model_id`，也未发现用 `interleaved.field` 选择响应推理字段的实现。
- Magpie 的多个客户端写入使用各自 adapter。Pi、Crush、Droid、Cline、Qoder、Zcode 等不是可由一个通用字段替代的配置路径。

models.dev 的当前公开 Model 类型列有 typed `reasoning_options`、`interleaved`、limits 和 cost，但没有列出 `canonical_model_id`。此前检查的在线 `api.json` 中约 63% 的模型含该字段。本轮检查仓库内置快照：8,339 条模型中有 5,274 条带 `canonicalModelId`；按 `{providerId}/{modelId}` 精确定位，4,043 条目标存在，1,231 条目标缺失；在目标存在的记录中，1,632 条的 `limit` 与来源渠道记录不同，85 条合法地指向自身。因此只能将该字段视为可选的**模型身份关联**，不能把目标记录的能力或价格无条件替换来源记录。快照更新后重新统计，不把上述数量写成长期不变的 schema 保证。

本地 Magpie 提交是本计划已核实的源码基线。尝试读取 GitHub 当前 HEAD 时连接被重置；本文不对该提交之后的远端变化作结论。

## 4. 本计划包含的工作

### 4.1 请求级应答模型对账

每次 attempt 分开保存：

- `upstream_model`：AiMaMi 实际发给上游的模型 ID。
- `served_model`：上游响应明确自报的模型 ID，可为空。
- `model_match_status`：`same`、`declared_different` 或 `unknown`；`declared_different` 表示上游**声明**与出站 ID 不同，不等于已证实实际换模。
- `model_match_basis`：原始精确相同，或通过已审计、受限的别名规则判定同一身份；另存 `unknown_reason`，例如未声明、响应失败、互相冲突、观测超限或流提前结束。

采集要求：

- 仅对实际收到的成功上游响应观察：JSON 顶层 `model`、Responses 的 `response.model`、Anthropic `message_start.message.model`，以及对应协议明确列出的 SSE 事件位置；不递归搜索任意名为 `model` 的字段，也不读取转换后的客户端响应来冒充上游声明。失败响应即使带 `model`，对账状态仍为 `unknown`。
- SSE 按事件边界拼接多条 `data:` 行，处理 CRLF、跨 chunk、注释、`[DONE]` 和终止事件；同一 attempt 多次声明不一致时保留受限的冲突摘要并标 `unknown`，不能采用最后一条覆盖前值。提前断流或缺少协议要求的完成标记时，即使先前见到模型名，也标注未完成，不能展示为已确认 `same`。
- 对流式响应沿用有界增量观察；现有 `route_trace` 对非流 JSON 的总缓冲、对 SSE 的单行缓冲上限均为 256 KiB，新事件解析必须保持等量级的上界。超限、非 UTF-8、非合法 JSON、缺失字段均返回带原因的 `unknown`；不为诊断缓冲或持久化完整请求/响应，也不改变原 observer 的失败和首 token 判定。
- 保留原始 `served_model`。归一化只用于比较，不覆盖模型字段、不删除未知后缀、不把推测结果伪装成上游原值。
- 原始 ID 需限制长度并去控制字符；截断后不能再作 `same` 或 `declared_different` 判断。比较规则只接受精确相等和有明确来源的已知别名；不能通用地剥离地区、日期、版本或渠道后缀，也不能仅凭 `canonical_model_id` 宣称两次实际执行相同。
- 上游没有自报模型时是 `unknown`，不得推断未换模或发生换模；自报与真实执行不一致也无法被本功能发现。
- 每次重试独立记录 model pair，不能只在整条 route 上记最后一次结果。
- 每次 attempt 的原始上游字节只产生一份有界的 `ServedModelObservation`，在正常 EOF、上游错误、客户端提前断开和对象 drop 时幂等定稿；trace 与已有 provider usage 明细消费同一份结果。当前 `route_trace::Observation` 与 `relay_usage::RelayUsageObserver` 是两条观察路径，实施时应在原始响应分叉前确定共享结果，不能让二者分别猜一次，也不能以转换生成的模型字段填充上游观察值。
- 保持现有响应字节、流式时序、状态处理、失败阶段和重试资格不变。

费用仍使用主计划中请求开始时固定的价格快照。`served_model` 初期只用于诊断；如果要按 served model 另估价格或展示价差，应单独评审，不能重写已落账费用或历史价格快照。

### 4.2 当次实际凭据归因

在出站 header 已构造、即将发送时，形成不可变的 attempt 凭据快照；此前构造失败的 attempt 标为 `not_sent`：

- 凭据类型，例如 provider API key、Codex 本地凭据或未知。
- provider ID/name，与凭据身份分开存储。
- 凭据显式标签（仅配置中确实存在时）。
- 可选的凭据版本指纹与可选的账号身份分别记录，不把 token 版本误称为稳定账号；不持久化原始 key/token。
- `recorded` / `not_recorded` 状态及缺失原因（如凭据由系统密钥库提供且无法安全计算指纹）。

规则：

- provider 模型请求实际使用 `provider.model_credential().api_key`，余额用的 `access_token` 不是这次推理凭据。指纹必须对应最终出站认证 header 的凭据，并与本次 provider、endpoint 一起冻结；不从当前配置回推旧记录。key 轮换后新请求得到新凭据版本，旧请求记录保持不变。
- Codex 原生凭据可能来自内存、keyring 或 `auth.json`，且 OAuth access token 会刷新。token 指纹至多标识一个凭据版本；只有出站路径明确提供可信账号 ID 时才能另外记录账号归因，无法确认时标 `account_not_recorded`。不得用旧 `auth.json` 或 provider 名推断当次账号。
- provider 显示名不能冒充 key 名；AiMaMi 出站 key 不能被描述为中转商最终计费账号。
- 历史记录没有该字段时显示 `not_recorded`，不自动迁移出虚假的身份。
- 指纹方案需用持久、受保护的应用本地密钥计算 HMAC，或用有明确轮换语义的随机凭据实例 ID；普通无密钥散列不合格。密钥不可用、丢失或无法对应最终 header 时显示 `not_recorded`，不能静默生成一个看似可跨重启比较的值。
- 不在常规日志、错误消息、前端调试信息中输出凭据或指纹原文以外的秘密材料。

### 4.3 路由原因说明

在既有选路分支作出决定时生成只读原因码和必要的引用，供每条 route/attempt 展示：

- 请求模型的来源和原始 ID。
- provider 是由 Codex slug、`/p/{id}`、已有会话绑定、active provider，还是当前 `effective_active` 规则选中；区分 `rate_limited` 与建立连接前失败触发的 failover，并明确 5xx/超时没有触发自动重试。
- 实际候选尝试顺序以及是否发生 failover；未尝试的候选不能伪装成实际出站 attempt。
- 对应尝试的 wire model、协议与本次计价快照。

原因码只能在现有决定点附带采集，不能事后再执行一遍选路或用能力元数据推断原因；诊断采集失败不得使原请求失败。会话绑定是现有状态，不标为用户显式 pin。此计划不新增请求 pin header。若未来另行引入显式 pin，未知、禁用或不可用的目标必须明确失败，不得静默回退；那项请求契约单独设计。

### 4.4 能力匹配与覆盖接管预览

沿用主计划落地后的后端统一 resolver 优先级，不插入会抢占原结果的新候选。`canonical_model_id` 只在**唯一确定一条 models.dev 来源记录后**，作为该记录的可选关联身份输出；共识或冲突匹配涉及多条记录时，逐候选列出关联，不能任选一个代表全部。能力仍取来源渠道记录及既有用户覆盖。价格规则已于 2026-10-08 调整为自动渠道价格、模型参考价和自定义价格，见主计划 §10.2；本节关于独立绑定的早期约束由新规则取代。它不提供从一个任意 wire ID 反推出目录来源记录的可靠索引，不能替代显式映射。

约束：

- 将 canonical 值按首个 `/` 拆成 `{providerId, modelId}`，校验格式和目标是否在当前快照存在。指向自身是有效关联；若为展示而沿链追踪，需设置循环/深度上限。多个渠道指向同一规范身份是正常情况，不合并其能力；若用户查看聚合能力，逐字段标出各来源差异。缺失、无效或未收录目标只标注关联不可验证，继续使用已匹配来源记录的能力值，不跳到另一个目录条目。
- 目标存在也不能继承目标的 context、input、output、reasoning、status 或成本；渠道限制可能比原厂更窄。若未来要用目标字段补缺，须另作有来源标注的逐字段规则和保守性评审，本期不做。
- canonical 关联不改变实际出站 provider；价格遵循主计划 §10.2 的渠道价格与模型参考价规则，保留完整结构冲突检查。
- 完整原始模型 ID 和 wire ID 始终保留；规范匹配只服务元数据解析和预览。
- 用户清除模型/供应商的某个覆盖字段或显式目录引用时，提供只读 dry-run：在同一份配置快照、目录 revision 和官方数据快照上模拟只清除选定字段，列出清除前后每个受影响字段的值、来源、冲突以及建议阈值与已应用值的差异；包含供应商级覆盖仍接管的情况。预览本身不保存、不注入、不探测、不触发目录刷新。
- dry-run 与实际能力预览必须调用同一个后端 resolver，禁止前端复制优先级或压缩公式。真正应用清除前比较配置版本及目录 revision；若已变化，重新计算并展示新结果，不能按过期预览直接提交。应用沿用现有配置所有权和同步路径，不改无关字段。

### 4.5 typed reasoning 与 interleaved

- 保存并展示 `reasoning_options` 原始类型和值：`effort`、`toggle`、`budget_tokens` 不互相折算；空列表、未声明和明确 `reasoning=false` 保持不同语义。当前 `models_dev.rs` 已解析三种选项，但 `Vec` 默认值会合并“字段未声明”和“显式空列表”，实施时需在快照和类型中补存在性信息，或对无法恢复的旧快照明确标 `unknown`。
- 只有 Codex、Claude Code、OpenCode 某一适配器明确支持对应参数且已验证版本契约时，才输出该适配器的字段。
- `interleaved` 的 `true` 与 `{ field: ... }` 两种形态作为响应格式提示保存；当前 `models_dev.rs` 尚未保留该字段，实施时先补快照构建、反序列化与预览的完整链路。不能单凭它更换 endpoint、协议或请求转换。
- 若该字段用于诊断或协议解析，必须限于已确认使用它的模型和响应协议，并保留原始转发内容。

### 4.6 真实 wire model 探测

所有连通性探测和以后新增的适配器回归使用该 provider 配置实际会发送的模型 ID、endpoint 和可用凭据。当前 `router::test_provider` 接收用户指定的 `test_model`，实施时核对从 UI 到 `provider_probe` 的实际请求体。models.dev canonical ID、别名候选和规范化模型名不得替换探测请求的 wire model；探测结果只说明这次请求的可用性，不能证明目录能力或长期价格准确。

### 4.7 诊断、用量与数据生命周期

- `route_trace` 目前只在内存中保存最近 60 条 route；`relay_usage` 的 JSONL 明细另行持久化，当前最近列表最多显示 200 条，超过 35 天的明细会在文件达到压缩阈值时折叠成按 provider/model 的汇总。新 UI 只能承诺在**仍存在的明细**上显示 served/凭据身份，汇总归档没有逐请求身份，也不能从当前配置回填。
- 当前 usage 记录仅为具备 `required_protocol` 的 provider 推理请求创建；Codex official 原生目的地和路由解析前失败的请求没有该账本明细。`route_trace` 可显示这些请求的实时诊断，但不得承诺它们出现在用量历史中。若要扩大持久化范围，应另立范围和计费规则。
- 现有账本已含 `request_id`、`attempt_id`，但 trace attempt 没有 `attempt_id`。在每次真实 attempt 创建时生成一次 ID，并传给 trace 与账本；同一 route 的 request ID 相同，重试各有独立 attempt ID。即使没有用量记录，trace 的 ID 仍唯一；构造失败或未出站状态要与已发送失败区分。
- 明细中新字段采用可选、带版本的向后兼容读取；历史缺失统一显示 `not_recorded` / `unknown`。聚合归档结构不添加伪造的 key 或 served model 分组，也不为了查询诊断阻止现有压缩；UI 要明示超过明细保留范围后仅有汇总。
- 凭据指纹与账号标识属于敏感元数据：只保留完成归因所需的最短值、限定展示权限和导出范围；普通日志、错误文本与完整响应不写入这些字段。清除用量数据时一并清除相应持久归因，内存 trace 按现有生命周期清除。

## 5. 明确排除的工作

以下项目不属于本补充计划，不与当前主计划捆绑：

- 引入 `stays=auto/session/turn/off` 或改动会话亲和期限。
- 新增多模型路由组、组内 effort 交集/最小 context 或 95% 窗口扩容选路。
- 新增 `X-Magpie-Account` 或其他 provider/account pin 协议。
- 用全量请求/响应日志替代当前有界诊断。
- 用 served model 重算或追溯改写账本费用。
- 一次性扩展 Pi、Crush、Droid、Cline、Qoder、Zcode 等所有客户端配置写回。
- 改动 Codex 原生配置所有权、Claude Code 角色/模型子集、OpenCode 用户 headers/variants/custom models。

这些项目如有需要，分别建立独立计划并以其自己的路由或客户端契约为依据。

## 6. 分阶段执行

### F0：等待主计划稳定并确认基线

- 主计划完成后核对实际合并状态，不假设工作树与主计划文档完全一致。
- 记录 `route_trace`、`relay_usage`、`router_proxy`、provider 凭据解析与三个现有客户端配置的当前行为。
- 建立 JSON/SSE、重试、多协议转换、key 轮换、未知模型及目录冲突的本地 fixture。
- 锁定数据字段、`request_id`/`attempt_id` 贯通方式、旧记录读取规则、诊断保留期限和凭据指纹设计；明确 official 与 pre-dispatch 失败没有 usage 明细。
- 对内置快照重新统计 canonical 目标可解析率及来源/目标能力差异，固定本次评审基线。

交付：独立变更说明、路由行为基线和 schema 迁移方案。未完成本阶段前不接入主计划正在处理的文件。

### F1：只读应答观察和凭据快照

- 在原始上游 response observer 中增加协议限定、有界的 served model 提取；覆盖 JSON、标准 SSE 事件组装、冲突与不完整流状态。将单份最终观察结果传给 trace 与现有 provider usage 记录，normal EOF 和提前 drop 均只定稿一次。
- 为每次 attempt 创建 upstream/served 对账记录和共享的 attempt ID；核对协议转换前的原始上游事件，而非转换后事件。
- 在 dispatch 凭据确定时生成安全身份快照；无法安全识别时显式记录 `not_recorded`。
- 仅为当前已有的 provider usage 明细扩展 schema，向后兼容旧账本与聚合归档；不重算旧账单、不回填历史 key，不改变 official 路径的统计范围。

### F2：规范身份关联和重置预览

- 验证并展示已匹配来源记录的可选 `canonical_model_id` 关联；不改变现有匹配候选顺序或来源渠道能力。
- 将身份关联与 provider 范围、显式能力引用和价格来源隔离，补齐 `interleaved.field` 的快照到预览链路。
- 增加基于同一快照的清除覆盖 resolver dry-run，并在实际提交前检查版本变化。
- 确认 model probe 使用实际 wire ID。

### F3：前端诊断与能力 UI

- 请求详情展示 model 对账、凭据身份状态与路由原因。
- 在仍存在的 provider 用量明细展示 served model 和身份是否记录；对旧记录显示未知，对聚合归档只显示汇总。
- 能力预览展示 canonical 关联身份及验证状态、冲突、字段类型和覆盖清除后的下一生效值。
- 控制信息密度；原始轨迹与来源放在按需展开区。

### F4：隔离验收与独立发布

- 在独立分支提交，review 时只包含本补充计划的代码与文档。
- 用确定性本地 fixture 比较旧版和新版请求的 route、wire model、headers、body、status、原始及转换后 stream 字节、重试资格与 client config diff；动态 ID/时间戳只在明确标注后剔除。
- 确认诊断开销有界、凭据不泄漏、旧 usage 文件可读。
- 先发布只读诊断字段，再启用能力预览改进；不迁移流量、不改现有路由策略。

## 7. 验收标准

### 请求诊断与账本

- 每次真实 attempt 都能区分原始 upstream model、可选 served model 和 `same` / `declared_different` / `unknown`；响应未提供 model、失败、超限、冲突、截断或不完整时显示具体 unknown 原因。
- 能识别已覆盖的 JSON/SSE 响应形态，包括多行 `data:`、跨 chunk、CRLF、错误事件和提前断流；供应商明确回显实际出站 wire ID 时判 `same`，已登记的别名同一性要显示比较依据，未经登记的差异只判 `declared_different`。
- 原始 served ID 保留；归一化规则不会删除未知版本名或改变请求模型。
- 同一 `request_id` 下不同 `attempt_id` 的模型对账、出站凭据和计价快照一一对应；失败 attempt 不被最后一次成功结果覆盖，未出站 attempt 不伪造凭据或 served model。
- 旧 usage 数据可读取但身份显示为 `not_recorded`；key 轮换后旧记录不变。35 天前已压缩的归档只有汇总，无法按 attempt 展示身份；Codex official 与路由前失败也不承诺有 usage 明细。
- provider API key 版本、Codex OAuth token 版本和可确认的账号 ID 在 UI 中分开；OAuth 刷新不能被误报为账号切换。
- 原始凭据、完整提示词和完整响应不进入持久诊断或普通日志。
- 当前费用仍按请求开始时价格快照结算，served model 不追溯覆盖费用。

### 能力目录与路由不变性

- canonical 字段存在、缺失、无效、目标未收录或发生冲突均有可解释的关联状态；不存在的目标不自动匹配，来源渠道的能力不被目标覆盖。
- canonical 关联不改变原有匹配优先级、来源渠道 limit/reasoning、wire model、价格绑定、协议或模型列表。
- 清除覆盖的 dry-run 与实际生效 resolver 在相同配置及目录 revision 下一致；配置或目录已变化时重新计算，预览本身不保存、不注入、不改路由。
- `reasoning_options` 的三种类型与字段存在性、`interleaved` 的两种形态能保留并解释；没有已验证的客户端/协议契约时不输出新的请求参数或改解析方式。
- 连通性探测使用供应商真实 model ID。
- 相同请求的 provider、候选顺序、故障转移结果、endpoint、认证、协议转换和 client config 与基线一致。
- Codex 路由/原生、Claude Code 和 OpenCode 的原有注入及恢复边界不回退。

## 8. 风险与处理

| 风险 | 处理 |
| --- | --- |
| 上游不总返回 `model`、伪报模型或返回 alias | 只称“声明差异”；保留原值和比较依据，缺失/冲突/未完成为 `unknown`，不能宣称已证实换模 |
| SSE 多行、超限或响应中出现无关 `model` 字段 | 按协议白名单位置和事件边界读取；观测失败写 unknown 原因，不扩大缓冲或改输出 |
| trace 与 usage 生命周期不同 | 单份 attempt 观察结果经共同 ID 分发；内存淘汰、客户端断开、失败与归档压缩均按真实保留范围显示 |
| 用原始 key 做散列仍可能被猜测、OAuth token 会轮换 | 使用本地受保护密钥生成 HMAC 或凭据实例 ID；账号与凭据版本分开，无法实现时标 `not_recorded` |
| 当前 provider 配置被误用于旧请求 | 每个 dispatch 写不可变快照；迁移只赋未知状态 |
| canonical identity 被误当能力或价格关系 | 来源渠道能力优先；关联字段只作身份说明，价格引用完全独立；目标缺失不自动补值 |
| 覆盖预览与实际清除之间数据刷新 | 固定配置与目录 revision 做 dry-run；应用前比较版本，变化则重算并提示 |
| 诊断信息改变路由或注入 | F4 对比 wire 和配置；此计划禁止把这些字段作为选路或配置写回输入 |
| 当前主计划尚在改相同模块 | 等待主计划稳定，使用独立分支和单独评审，避免并发改动覆盖 |

## 9. 参考

- Magpie 本地源码：`D:/workspace/temp/magpie`，提交 `7a3337dd25bd759b1a73a051cdefeab6033b63f6`。
- Magpie GitHub：<https://github.com/yetone/magpie>
- models.dev API：<https://models.dev/api.json>
- models.dev 类型文档（Context7）：`/anomalyco/models.dev`。
- 原计划：`plan/models-dev-integration-execution-plan.md`，只作为依赖和兼容边界参考；本补充计划不修改它。
