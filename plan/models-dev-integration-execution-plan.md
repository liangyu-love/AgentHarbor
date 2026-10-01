# models.dev 统一模型元数据与费用统计执行计划

日期：2026-10-01\
状态：待实施；本文件是实施依据，不表示功能已经完成。\
项目：AiMaMi\
参考：本地 `D:/workspace/temp/magpie`，对应 `yetone/magpie` 提交 `7a3337dd25bd759b1a73a051cdefeab6033b63f6`。

## 1. 目标与交付结果

接入 models.dev，建立统一、可追溯、能够区分供应商的模型元数据解析层，提供模型名称、上下文总容量、输入上限、输出上限、思考能力和价格。用户可以覆盖默认能力值和压缩设置。所有能力展示、匹配预览、客户端配置注入和费用估算消费同一份解析结果，逐步删除散落在 Rust 和前端中的按模型名猜测数值的规则。

交付后应达到以下结果：

1. 常见模型从 models.dev 获得默认能力值；应用离线时能够使用随版本发布的快照。
2. 带供应商前缀、渠道标记、已知选项后缀的模型，能够经过可解释的匹配找到元数据，原始上游请求 ID 保持完整。
3. 默认自动压缩比例为 90%，用户可以修改全局比例、供应商或模型级策略，并保留原有固定 token 阈值。
4. 能力目录展示原始数据、最终生效值、用户覆盖、匹配过程、来源和更新时间；预览与实际注入一致。
5. 请求级路由诊断继续解释真实路由和尝试过程，并可以关联该次请求使用的元数据版本与费用来源。
6. 用量界面展示今日、近七日、累计预估费用，以及按供应商、模型和日期的费用分布。
7. 删除旧的模型族名窗口表、硬编码思考档位表，以及前端重复的族名默认值规则。

models.dev 是默认数据来源，不能被描述成对任意中转实际配置的保证。用户覆盖、供应商明确上报和无法匹配的情况必须在结果中表达出来。

## 2. 必须保持的现有行为

本次改造以以下约束作为交付条件：

| 行为 | 约束 |
| --- | --- |
| 路由选择 | 保持 slug、active provider、启用状态和现有故障转移规则；元数据与价格不参与选路 |
| 故障转移身份 | 保持现有“同模型”判定与候选顺序；新能力匹配结果不扩大可故障转移集合 |
| 请求模型名 | 上游请求继续使用原始配置 ID；能力解析用的去前后缀结果不得写回请求体 |
| 协议与认证 | 保持现有 Responses / Chat Completions / Anthropic Messages 选择、转换、认证和端点逻辑 |
| 客户端注入 | 保留 catalog slug、模型列表和别名、必要的 instructions/tools 字段，以及现有直连和路由模式 |
| 配置管理 | 保留备份、配置所有权、漂移检测、恢复和撤销功能；用户在外部修改的字段不被后台刷新强制覆盖 |
| 用户配置 | 保留模型显示名、是否启用、手动能力值、协议选择和已有压缩阈值 |
| 诊断 | 仍记录真实请求和每次尝试；预览请求不触发上游调用、不改变路由状态 |
| 请求延迟 | 请求热路径不下载 models.dev，也不逐请求解析完整目录 |

“能力值发生合理变化”与“路由或注入机制退化”分开验收。旧手写值可能不准确，不能把所有数值必须与旧值相等作为通过条件。

本期不引入按价格选路、不实现新的上下文摘要引擎、不增加 wire_id 改写功能。供应商全量自动预设不作为本期交付前置条件。

### 2.1 四条客户端路径的注入边界（代码复核补充）

以下边界按当前仓库实现制定，实施前必须以对应适配器为准，不把所有客户端当成同一条注入链。

| 路径 | 元数据允许影响的内容 | 必须保持的内容 |
| --- | --- | --- |
| Codex 路由模式 | 自建 relay catalog 条目的能力、名称和逐模型压缩阈值；请求诊断附加来源 | 官方 catalog 条目完整保留；relay slug 仍为 provider::model；路由地址、认证、转换、候选集合和请求 ID 保持；不新增全局窗口/压缩覆盖 |
| Codex 原生直连 | 当前供应商自建 catalog 的能力，以及其已有全局窗口/压缩设置通路 | 直连上游地址、env 凭据、wire_api、Responses 模型筛选、模型原始 ID、配置所有权和恢复保持；不因目录宣称支持某协议而放宽可注入模型 |
| Claude Code | 自动名称、已有可用模型的目录补充及独立的能力预览 | ANTHROPIC_BASE_URL、认证字段选择、角色映射、用户模型子集、picker、availableModels、[1m] 选项和自定义 picker 行保留；不写入 Codex 专用 limit/effort/压缩键 |
| OpenCode | 已有选中模型的 name、limit.context/output 默认值及能力预览 | provider key 与协议分组、npm、baseURL、apiKey、当前 model、模型子集以及用户 headers/variants/custom models 保留；继续只管理生成的叶子字段 |

额外约束：

- models.dev 的 npm、provider.shape 和协议提示不得写入当前 endpointTypes 并触发 supportsProtocol 过滤；这些提示只能供参考，接入协议仍由现有配置决定。
- 完整 models.dev 模型列表是参考目录，不能直接替换某账号实际可用或用户已选中的模型列表。目录缺少一个模型也不能把它从客户端配置删除。
- Claude Code 的 [1m] 是现有用户选项。能力查找可识别该标记，但不能因此修改角色变量中的标记或用户选择；目录值与用户选项不同时，在预览解释。
- 当前 client_config::prepare 对所有客户端都校验 context_window/max_output_tokens，包含并不消费这些值的 Claude Code。迁移为可选能力字段时，要按目标适配器校验，保证无上下文元数据的 Claude Code 模型仍能按现有选择注入，不能简单去掉所有客户端的校验。
- OpenCode 当前选中模型使用 ConfigRequest 的数值，其他模型使用元数据。新方案应区分自动默认与用户显式输入：用户值不能被目录刷新覆盖，自动值也不能因为曾被保存就永久冻结。
- Codex 自建 catalog 保留 relay_catalog_entry 生成的必需 instructions/tools 等字段。禁止用一条 models.dev 原始记录直接替换完整 Codex catalog 条目，否则可能使 config/read 失败。
- 原生直连流量不经过当前 relay usage 观察器；费用功能不能为了补统计而把原生直连偷偷切换为代理。

## 3. Magpie 中可借鉴的实现

| 参考文件 | 已观察到的实现 | AiMaMi 的采用方式 |
| --- | --- | --- |
| `internal/catalog/catalog.go` | 统一 Model；读取 limit、reasoning_options、cost；精确查询与裸 ID 查询 | 借鉴统一数据结构；增加逐字段来源和供应商身份 |
| `internal/catalog/catalog.go` 的 window / ContextOf | 输入预算优先使用 limit.input，否则使用 limit.context | 保留原始 context/input/output 三项，另行计算提示词预算 |
| `internal/catalog/catalog.go` 的 bareID / cutAt / unprofiled | 去路径前缀、识别括号或冒号后缀、识别 Bedrock 区域 | 做成有顺序、有轨迹的候选生成；完整 ID 始终先查 |
| `internal/catalog/live.go` 的 Decorate | 完整 ID 优先，再尝试去路径前缀；补充上游列表缺失的元数据 | 元数据补充保持原始列表、请求 ID、用户选择和别名 |
| `internal/provider/provider.go` 的 Catalogs | 供应商可指定一组 models.dev provider ID | 提供能力目录映射；价格绑定另外记录，避免把原厂价格误认为中转价格 |
| `internal/catalog/catalog.go` 的 mostGiven / mostListed | 跨供应商按出现次数聚合；平票时保守选择 | 作为参考候选和冲突解释；本期不把多数票直接视为准确的供应商能力 |
| `internal/catalog/catalog.go` 的 PriceOf / PricedBy | 价格优先按供应商和模型查询 | 采用供应商范围约束；不跨供应商取均价或最低价 |
| `internal/catalog/fresh.go` | 24 小时 stale、后台定期检查、刷新去重、超时 | 采用后台刷新与只读缓存分离；增加原子写入和完整校验 |
| `internal/provider/models.go` 的 ContextOf | 模型级用户值优先，供应商通配值其次 | 借鉴覆盖范围，并兼容现有 provider.contextWindow |

在已检查的代码中没有发现可直接复用的“全局自动压缩比例 90%”设置。该比例是 AiMaMi 的产品默认值；Magpie 对 compaction_trigger 的摘要处理属于另一类功能，不纳入本期移植。

如果实际复制 Magpie 的代码或其他受许可保护的内容，应遵守其 MIT 许可并保留相应归属。只参考行为时也应在实现说明中记录参考提交。

## 4. 当前代码与改造位置

| 位置 | 当前职责与问题 | 计划改造 |
| --- | --- | --- |
| `src-tauri/src/core/model_defaults.rs` | model_key、Codex 快照读取、族名窗口表、精确 ID 思考表混在一起 | 分离身份候选、官方快照与默认策略，最终删除 named_defaults / reasoning |
| `src-tauri/src/core/newapi.rs` | with_local_defaults / with_catalog_defaults 把默认值填进 Model；一个 metadata_source 表示整条记录 | 保存原始声明与用户覆盖，通过统一解析器产生生效结果 |
| `src-tauri/src/core/router.rs` | catalog_value、resolved_model_metadata、reasoning 来源与 Claude 族名档位兜底 | 统一消费解析结果；删除 relay_reasoning_fallback 中剩余的族名档位规则 |
| `src-tauri/src/core/codex_native.rs` | 原生 catalog 和 context / auto-compact 注入、配置匹配检查 | 与 router 复用同一解析器及压缩策略计算 |
| `src-tauri/src/core/router_config_snapshot.rs` | 注入字段所有权、快照与恢复 | 将推导阈值纳入实际已应用值记录，不能继续仅用 provider 原始值推断 |
| `src-tauri/src/core/client_config/opencode.rs` | 部分模型仍调用 for_model 兜底 | 使用统一客户端投影，保持 selected model 的用户输入优先 |
| `src-tauri/src/core/client_config/claude.rs` | 模型列表、角色和别名配置 | 保持现有行为；不注入未确认支持的上下文或压缩参数 |
| `src-tauri/src/core/capability_preview.rs` | 重复推断字段来源；matched_model 目前是 model_key 输出 | 直接序列化解析器结果，提供真正匹配到的 provider/model 与候选轨迹 |
| `src-tauri/src/core/router_proxy.rs` | 请求转发、尝试、usage 观察 | 仅附加请求级元数据/价格快照引用与观测，不改选路及请求 ID |
| `src-tauri/src/core/relay_usage.rs` | token 计数、今日/累计汇总、账本归档；缺少缓存计数与费用字段 | 扩展计费计数、请求费用、汇总和归档兼容 |
| `src/lib/client-config.ts` | clientDefaults 中仍按 Claude 族名设 1M，否则 128K | 删除前端族名默认规则，读取后端解析结果 |
| `src/lib/provider-models.ts` | 刷新时保护别名、用户值和协议 | 适配逐字段覆盖和模型目录映射；刷新不能改变 override |
| `src/components/newapi/model-capabilities.tsx` | 能力目录与来源展示 | 增加完整匹配说明、压缩推导、价格和冲突信息 |
| `src/components/newapi/routing-diagnostics.tsx` | 请求级诊断 | 增加只读元数据快照/计价来源，保持现有尝试展示 |
| `src/components/newapi/provider-model-editor.tsx` | 模型编辑 | 增加目录映射、逐字段覆盖及恢复自动值入口 |
| `src/components/newapi/provider-dialog.tsx` | 供应商设置和已有固定压缩阈值 | 增加供应商策略与目录绑定，保留旧阈值 |
| `src/components/accounts/relay-usage-panel.tsx` | 用量展示，成本列占位 | 今日/累计预估费用、明细和计价覆盖情况 |
| `src/types/newapi.ts`、相关 usage 类型、`src/lib/api.ts` | 前后端契约 | 同步新增数据结构和命令 |
| `src/locales/zh.json` / `en.json` | 文案 | 增加来源、匹配、压缩策略和费用状态文案 |

以上是启动清单，实施前仍需搜索全部 for_model、named_defaults、reasoning fallback 和族名数值分支，覆盖未列出的调用方。

## 5. 统一数据结构

建议采用三个职责明确的模块，不建立过多抽象层：

- `models_dev.rs`：上游结构、快照加载、索引、数据校验。
- `model_metadata.rs`：身份匹配、逐字段来源决策、覆盖和结果对象。
- `model_policy.rs`：通用未知值兜底、压缩比例计算和客户端投影所需的应用策略。

网络刷新放在现有命令或后台服务边界，核心解析函数不联网。模块名称可按仓库习惯微调，职责和唯一解析入口必须保持。

### 5.1 原始模型记录

至少保留：

- 目录 provider ID、模型 ID、name。
- limit.context、limit.input、limit.output。
- reasoning 及 reasoning_options，包括 effort、toggle、budget_tokens 原始类型。
- cost 的完整结构；包括缓存、推理和条件价格等实际存在的字段。
- modalities、模型类型或状态中用于筛选适用模型的信息。
- provider/model 的协议提示，例如 npm 或 provider.shape，供诊断参考，不自动覆盖当前协议。
- last_updated、必要的来源链接或目录来源标识。

不直接依赖未经验证的 canonical_model_id。P0 必须对实际 api.json 和当前上游 schema 核验其存在性、含义及覆盖率。字段不存在时使用显式目录引用和候选索引完成匹配。

### 5.2 用户配置

- 供应商配置：能力目录 provider 引用、可选价格目录 provider 绑定、供应商压缩策略。
- 模型配置：可选完整能力引用 `{ providerId, modelId }`、独立价格引用、逐字段覆盖、模型压缩策略。
- 继续保存原始 id、displayName、protocol、included，不因刷新目录替换这些用户配置。
- 来源字段是派生信息，不作为判断“用户是否修改了此字段”的唯一依据。
- 覆盖至少区分“继承自动值”“设置具体值”；思考档位另外保留“明确没有档位”的空列表语义。

### 5.3 解析结果

`ResolvedModelMetadata` 应包含：

- 原始 provider ID、原始上游模型 ID、目录匹配引用。
- matchKind：explicit / exact / normalized / consensus / ambiguous / unmatched。
- matchSteps、候选结果和冲突字段，便于预览解释。
- contextWindow、inputLimit、outputLimit、reasoning、defaultEffort、price。
- 每个字段的 value、source、sourceRef、catalogRevision、updatedAt。
- 用户覆盖及应用限制之后的生效值；原始目录值仍可查看。
- 缓存整体 fetchedAt、stale 和 lastRefreshError。

“未知”“明确不支持”“零价格”必须保持不同状态。价格和 token 字段均不能用缺失等于零的默认反序列化掩盖未知。

## 6. 匹配规则与来源顺序

### 6.1 先保留原始身份，再按顺序生成候选

完整模型 ID 永远先查。每一步规范化都生成新候选并记录依据，不直接覆盖原始 ID。

| 示例 | 候选处理 | 限制 |
| --- | --- | --- |
| `openai/GPT-6-Sol` | 完整精确查找之后尝试大小写规范化、供应商路径前缀 | 同一目录范围存在大小写冲突时停止自动匹配 |
| `relay::gpt-6-sol` | 按现有渠道标记规则生成裸 ID | 只用于能力候选；不改变路由 slug |
| `channel__gpt-6-sol` | 保留现有渠道前缀识别 | 不以任意下划线切分版本名 |
| `gpt-5.5(high)` | 括号内容是已知 effort 标记时生成基础模型候选 | 不任意删除括号中的未知版本说明 |
| `glm-5:free` / `model@fast` | 完整版本优先，再尝试已识别标签候选 | 价格可能与基础模型不同，不自动借基础价格 |
| `claude-opus-5-5-kiro` | 保留现有已知渠道后缀能力匹配 | 保留数字版本，不匹配成 claude-opus-5 |
| `apac.anthropic.claude-…` | 按 Bedrock 区域白名单尝试去区域 | 限定在相关目录/格式，不处理任意点分前缀 |
| `model-preview` / `model-20261001` | 默认保留版本、日期、preview 后缀 | 除非有明确映射，否则不与基础 ID 合并 |
| 任意不认识的前后缀 | 提供手动映射入口 | 不采用包含字符串、编辑距离或任意 startsWith 自动套值 |

### 6.2 匹配范围

1. 模型显式能力引用优先，引用失效时显示失效状态，不能悄悄匹配另一条。
2. 已知供应商目录内完整 ID 精确匹配。
3. 用户配置的一组能力目录内，先查所有完整 ID，再进行规范化候选匹配；同一优先级的冲突必须显式处理。
4. 无供应商映射时，允许通过全局规范化索引寻找候选，但只能作为通用能力参考。
5. 通用参考只有在相关字段一致时才自动填入该字段；上下文一致、思考档位冲突时，保留上下文并对思考显示冲突。
6. 多数票结果可以展示为建议，首期不作为自动注入的依据。第一方目录也不能仅凭名字相似证明未知中转的实际限制。

未知中转仍能利用 models.dev 的已知基础模型能力，但界面会显示“通用模型参考”，用户可以改成精确映射或手动覆盖。

### 6.3 字段级来源顺序

| 字段 | 决策顺序 |
| --- | --- |
| 名称 | 用户别名 → 有效供应商显示名 → 匹配目录名称 → 原始 ID |
| 上下文/输入/输出 | 模型字段覆盖 → 现有供应商用户上下文设置（按兼容规则）→ 已保存的真实供应商明确声明 → 适用的官方账户快照 → models.dev 精确/显式映射 → 无歧义通用参考 → 通用兜底或未知 |
| 思考模式/档位 | 模型覆盖 → 适用供应商明确声明 → 适用的官方账户快照 → models.dev 精确/显式映射 → 无歧义通用参考 → 未知 |
| 默认 effort | 合法的用户选择 → 与最终档位来源相符的已声明默认值 → 明确的应用默认策略或客户端默认 |
| 价格 | 模型价格覆盖 → 供应商通配价格覆盖 → 当前供应商精确价格记录或用户明确绑定的价格记录 → 未知 |

“真实供应商声明”必须是采集时保留下来的原始字段。历史 `with_local_defaults` 已经填充的值不能重新冒充上游声明。models.dev 仍是缺少明确声明时的主要默认来源。

Codex 官方快照只有在确认是适用官方模型/账户路径时享有来源优先级。第三方同名模型不因 model_key 相等就自动使用官方账户限制。

现有 provider.contextWindow 在 `resolved_model_metadata` 中实际是覆盖赋值，不只是 min clamp。迁移时保留其原有语义；如另加“仅降低上限”的限制选项，应使用不同字段，不能无提示地改变已有用户设置。

### 6.4 思考能力处理

- effort 类型映射为档位列表，保留值，不为未知模型制造 low/medium/high。
- toggle 和 budget_tokens 保留原始能力并展示，不伪造离散 effort。
- 未声明、空列表、reasoning=false 分别处理，明确声明应阻止低优先级兜底重新添加档位。
- 默认 effort 必须属于最终档位集合；models.dev 没有默认值时不得声称某档是厂商默认。
- 客户端需要默认值时，可沿用显式的通用选择策略，并标记为应用策略；族名到档位的硬编码仍必须删除。
- 目录声明只影响能力菜单和适配器输出；本期不借此改变请求协议或偷偷将 toggle 改写为预算参数。

## 7. 上下文与默认 90% 自动压缩

### 7.1 策略配置

提供 `inherit`、`percent`、`tokens`、`clientDefault` 四种策略：

- 全局默认策略：percent = 90，用户可调整；第一版允许 1–99 的整数百分比。
- 模型级策略优先于供应商级，供应商级优先于全局默认。
- 同一层级只选择一种策略，不同时启用固定 token 与百分比。
- 原有 autoCompactLimit 非空的供应商迁移为 tokens 策略，保留其数值。
- clientDefault 表示委托客户端管理，不能被描述为关闭压缩。
- 提供“恢复继承默认值”的操作，恢复后继续跟随上下文和全局比例变化。

### 7.2 计算过程

```text
rawContext = models.dev 的 limit.context
rawInput   = models.dev 的 limit.input（可能缺失）
rawOutput  = models.dev 的 limit.output

basePromptBudget = rawInput 存在时使用 rawInput，否则使用 rawContext
effectiveBudget  = 对 basePromptBudget 应用有效来源和用户上下文设置后的值

percent 模式：compactAt = floor(effectiveBudget × percent / 100)
tokens  模式：compactAt = 用户固定阈值
clientDefault：不生成应用拥有的阈值
```

例：有效输入预算为 200,000，默认 90% 对应 180,000；用户改为 80% 后为 160,000。若 limit.context 为 400,000、limit.input 为 272,000，按输入预算计算的 90% 为 244,800，而不是 360,000。

必须同时保存原始总上下文、输入上限和输出上限。不能为了使用输入预算而把原始 context 字段重写成 input，也不能对已扣除输出空间的 input 再重复减去 output。

当只有总上下文信息时，90% 是压缩触发策略，不代表上游永远有足够的输出空间；实际请求已有输出预算限制继续生效。客户端的 token 估计、预留空间和字段含义由适配器契约校验，不在此处发明统一的请求裁剪公式。

### 7.3 校验与更新

- 验证 0 < compactAt < effectiveBudget，使用有界整数运算。
- 固定阈值超过新上下文时提示用户调整，阻止应用无效配置；不能悄悄修改用户输入或把它伪装成已生效。
- 上下文未知时，若只能使用通用兜底，则压缩来源明确为“根据兜底预算计算”；不显示 models.dev 标签。
- 目录刷新后百分比模式可以重新推导；固定阈值保持用户输入。
- 目录候选值、最终计划值、已应用值分别记录，确保预览能显示“待同步”和“已生效”的区别。

### 7.4 多模型配置限制


复核当前代码后，必须区分两个现有通路：

- **Codex 路由模式**：router::catalog_value 已将 provider.auto_compact_limit 写入每条 relay catalog 的 `auto_compact_token_limit`；配置注入主动移除全局 `model_context_window` 和 `model_auto_compact_token_limit`，相关断言已存在。
- **Codex 原生直连**：codex_native 目前通过全局 `model_context_window` 和 `model_auto_compact_token_limit` 应用供应商设置，自建 catalog 使用该供应商模型的原始 ID。

执行规则：

1. 路由模式沿用每条模型 catalog 的阈值位置，把原来的 provider 固定值扩展为统一策略计算结果；禁止为了 90% 默认值重新添加全局窗口/压缩覆盖，更不能以所有官方和中转模型的最小窗口限制整个菜单。
2. 对目标 Codex 版本验证 `auto_compact_token_limit` 的实际读取和生效。现有代码生成了此字段是实现事实，不等于所有客户端版本都已验证支持。若不支持，记录限制并保留安全兼容行为，不能擅自退回影响全部模型的全局阈值。
3. 原生直连沿用现有设置通路。单模型配置可按所选模型计算；若用户在同一个直连配置中切换多个模型而共享全局阈值，应明确说明作用范围，并在该直连供应商的已选模型集合内采用保守阈值。若已验证的客户端支持逐模型设置，可以单独迁移原生适配器及其配置所有权。
4. 共享全局配置下的模型级覆盖是计算输入，不能承诺切换模型时一定采用独立阈值；不能在每次请求时重写 config.toml 追踪模型切换。
5. 同模型故障转移涉及不同供应商预算时，只读取现有路由器认定的候选集。其保守预算只用于对应 relay 模型条目，不影响不相关模型，也不改变候选顺序或可用性。
6. 有候选预算未知时，显示无法确认全部候选的提示；缺失数据不导致候选被停用。

本期首先支持当前已有的 Codex 阈值注入通路。Claude Code 和 OpenCode 不写入未经验证的压缩键；界面区分“建议阈值”和“此客户端已应用阈值”。

## 8. 缓存与内置快照

### 8.1 内置快照

- 增加 `scripts/build-models-dev-snapshot.mjs`，生成提交到仓库的 `src-tauri/assets/models-dev.snapshot.json`。
- 更新快照是显式开发/发版步骤，正常构建只读取已提交文件，不依赖联网。
- 快照包含 schemaVersion、fetchedAt、sourceUrl、内容 revision/hash 和精简后的 provider/model 数据。
- 保留模型类型、modalities、完整 cost 和 reasoning_options；不沿用原计划“直接丢弃 modalities”的做法。
- logo、长描述和 benchmark 等与本期无关的内容可以裁剪。
- 记录许可，分发时保留实际要求的许可文本；sources/fetchedAt 不替代许可文本。

### 8.2 运行时缓存

- 建议文件：应用数据目录下 `models-dev.json`，使用版本化结构。
- 启动优先加载完整有效的运行时快照，否则使用内置快照。
- 同一请求或配置生成过程固定一个内存快照版本；不混用刷新前后的字段。
- 有效的新目录中缺少某条模型不自动用旧内置价格补回，避免复活已删除的错误记录。回退策略以整份有效快照为基础。
- 内存建立精确索引和规范化候选索引；查询不重复扫描整份 JSON。

### 8.3 刷新

- 默认有效期 24 小时；启动时过期则后台刷新，长时间运行每小时检查一次；另提供手动刷新。
- 同一时刻只进行一次刷新，设置超时和失败重试间隔，使用应用现有代理配置。
- P0 实测数据体积后确定下载/解压后字节上限；读取上限加一字节以检测超限，不只依赖 Content-Length。
- 先下载到临时文件，解析与校验通过后原子替换；失败保留最后一份有效缓存。
- 如服务支持 ETag / Last-Modified，可使用条件请求；304 只更新校验时间，不伪造模型 last_updated。
- 缓存状态分别记录 fetchedAt、lastCheckedAt、lastRefreshError；旧数据仍可用与刷新失败可以同时存在。
- 刷新只更换目录快照并通知订阅方；客户端配置同步继续走现有所有权和漂移保护流程。

## 9. 能力目录、匹配预览与请求诊断

### 9.1 能力目录与匹配预览

每条模型至少展示：

- 当前供应商、原始请求模型 ID、用户显示名。
- 匹配到的 models.dev provider/model、匹配类型和候选变换步骤。
- 原始 context/input/output 与最终客户端使用的值。
- 思考模式、可用档位、默认档位及其来源。
- 压缩策略、计算预算、比例或固定值、最终阈值和已应用状态。
- 价格、币种、每百万 token 单位、计价绑定和更新时间。
- 不匹配、字段冲突、显式引用失效和缓存过期状态。

提供按模型或供应商搜索、只看未匹配/冲突、设置目录映射和恢复自动值。用户不用阅读内部 Rust 类型；详细推导放在展开区域。

前端不得重复执行 model_key、能力来源优先级或压缩公式。后端返回与实际生成配置相同的投影结果。

### 9.2 请求级路由诊断

- 保留 requestedModel、尝试列表、真实 upstreamModel、protocol、状态、失败阶段和实际 sent 参数。
- 附加当时使用的 catalog revision、目录引用、matchKind 和计价状态。
- 记录的是请求发生时快照；稍后目录刷新不能改写已发生请求的诊断解释。
- 区分“能力声明”“客户端配置值”和“请求实际发送值”；不能因为目录支持 high 就声称该请求发送了 high。
- 每次重试或故障转移关联自己的供应商与计价引用。
- 不记录密钥、完整提示词或完整响应；沿用现有诊断开销及保留范围。

## 10. 费用计算与用量展示

### 10.1 UI 交付

用量界面增加今日预估费用、近七日预估费用、累计预估费用、每日趋势，以及按供应商/模型汇总和最近请求费用。

- 默认使用 USD，显示单位和有效的小额精度。
- 价格未知显示“未计价”；缺少用量显示“未返回用量”；真正的零价显示 0。
- 汇总附带已计价请求数和未计价请求数，不能把部分合计包装为全部支出。
- 本统计首先覆盖应用实际观测到的路由请求。原生直连未经过 usage observer 的请求不自动纳入，界面说明统计覆盖范围。
- 订阅套餐或中转站的实际扣费无法从原厂标价直接得出，费用展示名称使用“预估费用”。用户填写的价格覆盖仍是估价依据，不等同于账单回执。

### 10.2 价格匹配

- 用真实尝试供应商和原始上游模型关联价格。
- 能力映射与价格映射分开：借用 OpenAI 模型能力不自动证明某中转按 OpenAI 价格收费。
- 完整模型价格匹配先于别名；`model:free`、`model:batch`、区域或套餐差异不能因为去后缀而使用普通价格。
- 仅在当前计费供应商范围内无歧义且价格一致，或用户明确绑定时，允许价格别名回退。
- 不跨供应商投票、取均价、取最低价或自动借原厂报价。
- 保留上游实际 cost 结构。对长上下文分档等条件价格，能判断档位时使用对应价；条件无法判断时标记计价不完整或未知，不假装基础单价覆盖全部情况。

### 10.3 用量归一化与公式

当前 TokenCounts 没有 cache_read/cache_write，必须先扩展观察器。归一化到互不重叠的计费类别，再乘对应单价：

```text
costUSD = Σ(计费类别 token 数 × 该类别 USD/百万 token 单价) / 1,000,000
```

- 分别处理 Responses、Chat Completions 和 Anthropic 的输入、输出、缓存读写和 reasoning 计数语义。
- input 总数包含缓存时，需要先正确拆分；本来排除缓存时不能再减一次。
- reasoning 已包含在 output 时不能重复相加；若有独立 reasoning 单价，只有可以明确分离对应输出类别时才应用。
- 缺失缓存单价不能自动当成免费，缺失计数不能无条件推定为实际零用量。
- 流式增量、累计计数和最终事件合并按协议处理，避免相同 usage 重复计费。
- 金额使用统一精度和舍入规则累计，建议固定精度存储，展示时再格式化，避免逐请求四舍五入到美分后丢失小额费用。

### 10.4 账本记录和汇总

每个真实上游尝试保存版本化记录，包括已有时间/供应商/模型/协议/token 信息，以及：

- 请求与尝试关联标识，避免流结束、重试或断连造成重复记账。
- cache read/write 等新增计数及完整性状态。
- 请求开始时固定的价格快照引用和实际使用单价。
- costUsd、pricingStatus、pricingSource、catalogRevision。
- 有可靠 usage 的失败或中断请求也可计入费用，不以 HTTP 成功与否直接决定零费用。

RelayUsageTotals 及 ArchivedModel 同时保存费用与未计价数量；账本归档前后总费用必须一致。保留已落账费用，不随 models.dev 价格更新或用户以后修改价格而自动重算历史。

“今日”按应用统计时区的自然日统计，当前用户为 Asia/Shanghai；不等于滚动 24 小时。定义并保留统计时区/日期规则，避免跨时区部署使日期归属变化。

### 10.5 旧数据迁移

- 旧记录没有费用字段时默认为历史未计价，不填 0。
- 已归档旧记录只保留聚合计数，不能恢复缺失的缓存明细或请求时单价。
- 可提供显式“按当前价格估算历史”操作，结果标注 current-price-estimate，记录估算版本和不完整原因。
- 历史回算不伪称发生时实际价格；重新估算需要显式操作，不能因每次打开面板而改变累计金额。

## 11. 配置与数据迁移

1. 为新配置、目录缓存及用量记录建立独立 schemaVersion；读取旧版本时兼容缺失字段。
2. 历史 metadata_source=manual 的已有值保留为用户覆盖，清除覆盖使用独立操作。
3. 历史 local/catalog 自动填充值不固化为用户覆盖，交给新解析器重新解析。
4. 历史标成 upstream 的数据如果无法确认是否被 with_local_defaults 补过，标记为 legacy，不能获得真实上游声明优先级；后续刷新重新采集原始字段。
5. 显示名缺少“用户编辑过”标记时，保守保留已保存的非空别名；未来显式区分自动名称和用户别名。
6. 原有非空 autoCompactLimit 原值迁移为供应商 tokens 策略；无值者继承默认 90%，通过正常配置同步生成新阈值。
7. provider.contextWindow、模型手动能力值和已保存客户端 ConfigRequest 的人工输入保持原先范围，明确区分编辑表单值和已应用配置。
8. 新目录引用不修改 provider.id、model.id、slug、protocol 或 included。
9. 保存迁移前备份；迁移应幂等，失败恢复原数据，不留下半迁移状态。

## 12. 分阶段执行

### P0：确认数据契约和行为基线

- [ ] 读取实施时工作区指令和当前变更，避免覆盖用户正在修改的页面。
- [ ] 拉取真实 api.json，记录字段、体积、provider/model 数量、缺失字段比例和样本。
- [ ] 核实 canonical_model_id、完整 cost 条件结构及默认 API 是否包含所需模型类型，不凭原计划中的数量或 URL 参数假设实现。
- [ ] 检查三个客户端的目标版本与字段约束，重点确认 context/input 的投影、可省略字段和压缩阈值作用域。
- [ ] 列出所有旧能力表消费者、前端族名默认值和 router 内隐藏的 reasoning fallback。
- [ ] 记录现有路由候选、模型 ID、生成配置及所有权恢复行为，建立用于后续对比的固定样例。
- [ ] 选定实际配置字段、迁移版本、未知值策略和多模型压缩投影。

交付物：字段对照、固定小型目录 fixture、行为契约样例和待核验项结论。公共 schema 不支持的字段不得带着假设进入 P1。

### P1：目录数据、缓存与统一解析器

- [ ] 增加快照更新脚本和版本化内置数据。
- [ ] 实现只读加载、索引、解析校验、内存快照和 refresh/status 命令。
- [ ] 实现精确匹配、受限前后缀候选、冲突判定与字段来源。
- [ ] 实现能力映射/价格映射和用户字段覆盖结构，迁移旧数据。
- [ ] 建立 shadow 对比输出，新结果暂不替换正式注入。

完成条件：离线有数据、刷新失败保留旧缓存、未知字段可兼容、同一快照解析稳定；查询没有联网副作用。

### P2：能力目录与客户端接入

- [ ] 能力预览改为直接读取解析结果和客户端投影，不自行重算来源。
- [ ] 接入 newapi、router catalog、codex_native、OpenCode 及前端 clientDefaults。
- [ ] 保持 Claude Code 列表/别名与原有设置，仅补充适用的数据来源。
- [ ] 增加模型映射编辑与恢复自动值，保留用户协议/名称/启用状态。
- [ ] 为请求诊断记录实际元数据版本和匹配结果。
- [ ] 保留临时旧规则分支作为迁移对照，展示 legacyFallback 来源，禁止与 models.dev 来源混淆。

完成条件：预览与 router/native/OpenCode 的实际输出逐字段一致；路由候选、请求模型名和协议选择保持基线行为。

### P3：90% 自动压缩与用户覆盖

- [ ] 新增全局百分比设置，默认 90%。
- [ ] 实现模型/供应商继承和固定 token 模式，兼容已有 autoCompactLimit。
- [ ] 实现有效输入预算、共享配置阈值及范围校验。
- [ ] 统一配置生成、config matching、漂移检测、快照和恢复使用的已应用值。
- [ ] UI 展示自动计算过程、受哪个模型限制、待同步/已应用状态及客户端适用范围。

完成条件：例如 200K 的 90% 为 180K，用户覆盖优先；新目录改变上下文后自动策略重新计算，固定值不被静默覆盖；切换注入模式与撤销仍能恢复用户原配置。

### P4：费用落账与用量统计

- [ ] 扩展协议 usage 解析，保留缓存和推理计数语义。
- [ ] 实现价格解析、独立计价绑定、用户价格覆盖及未知状态。
- [ ] 请求开始固定价格快照，结束时计算费用并持久化。
- [ ] 扩展今日/近七日/累计/供应商/模型汇总和归档格式。
- [ ] 增加费用 UI 与计价覆盖率，保持旧记录兼容。
- [ ] 如实现历史回算，单独标记并保持显式触发。

完成条件：按实际供应商计价；流式、失败和重试不重复；价格刷新不改已落账费用；归档后累计金额不丢失。

### P5：删除旧表、整理与交付

- [ ] 解释 shadow 对比中的差异，纠正原有错误默认值不视为失败。
- [ ] 删除 named_defaults、精确 ID reasoning 表、router 族名档位规则和前端族名数值规则。
- [ ] 保留身份工具中仍被路由使用的行为，新能力匹配器不反向修改这些规则。
- [ ] 对未知模型，核心解析器返回未知；客户端必须有数值时使用一个集中、清楚标记的通用策略。可以暂保留 128,000 / 8,192 作为兼容兜底，但不得称为 models.dev 数据，也不保留族名分支。
- [ ] 删除临时 shadow 命令或把它收敛成必要诊断工具，不长期维护两套正式解析链。
- [ ] 更新相关说明、迁移说明和引用许可。

完成条件：除用户配置、内置数据和单一通用策略外，没有按模型名猜上下文或思考档位的生产规则。

依赖顺序：P0 → P1 → P2 → P3/P4 → P5。P3 与 P4 技术上独立，但都依赖 P2 的统一解析结果。

## 13. 验收与验证清单

### 13.1 匹配与来源

- [ ] 精确供应商匹配、显式引用、缺失引用、大小写冲突、完整标签模型与基础模型同时存在。
- [ ] `/`、`::`、`__`、`:free`、`@fast`、`(high)`、已知 Kiro 后缀、Bedrock 区域前缀。
- [ ] 日期、数字版本、preview 和未知后缀不会被误删。
- [ ] 同名模型跨供应商有不同窗口/思考档位时，各字段按规则解析且展示冲突。
- [ ] 能力可借用通用模型数据，价格仍保持供应商范围。
- [ ] 缺失、明确空档位、明确不支持、价格为零不会被互相替代。
- [ ] 原始请求 ID、路由 slug、模型列表用户选择保持原样。

### 13.2 客户端与路由保护

- [ ] Codex native/router 模式同一模型的能力投影一致，必需 catalog 字段仍完整。
- [ ] 官方模型和第三方同名模型不无条件共享官方账户能力。
- [ ] OpenCode 所选模型及其他模型均读取统一结果；Claude Code 原有角色与别名保持。
- [ ] Claude Code 在目录无 context/output、含 [1m]、用户自定义 picker 行的情况下仍可注入；公共校验不误阻止无关适配器。
- [ ] Codex 路由只更新 relay catalog 条目，不写全局窗口/压缩设置；原生直连使用独立的字段所有权和配置恢复断言。
- [ ] 官方 catalog 条目的 instructions、tools、账户能力等不被 models.dev 原始记录替换。
- [ ] models.dev 刷新不改协议分组、认证、endpointTypes、provider key 或用户模型子集。
- [ ] OpenCode 保存和恢复仅操作受管理叶子字段，用户 headers/variants/custom models 不丢失。
- [ ] 显式路由、active provider、同模型故障转移、认证及协议转换行为保持基线。
- [ ] 目录不可用或字段冲突不会使本来可调用的供应商被自动排除。
- [ ] 来源预览、计划注入值和已应用值可区分，外部编辑受漂移保护。

### 13.3 压缩

- [ ] 90% 默认、自定义百分比、固定 token、继承、clientDefault。
- [ ] input 与 context 不同、数据缺失、用户上下文覆盖、非法阈值、整数边界。
- [ ] 多模型共享阈值和不同上下文故障转移候选的保守计算。
- [ ] 目录变化后百分比重算，固定值保留，失效覆盖有提示。
- [ ] 启用、切换、更新、撤销注入和恢复配置均覆盖推导阈值所有权。

### 13.4 费用与账本

- [ ] 不同协议输入/输出/缓存/推理 token 的重叠关系正确处理。
- [ ] 流式 usage、重复最终事件、客户端断开、失败且有 usage、重试分别核算。
- [ ] 零价、缺价、缺 usage、部分价格、条件价格和用户覆盖可区分。
- [ ] 同模型不同供应商价格不同；free/tag 版本不误套基础价。
- [ ] 单笔、小额累计、今日边界、七日和累计汇总正确。
- [ ] 账本归档、重启、旧版记录读取不改变已计价总额。
- [ ] models.dev 或手动价格更新后，旧账单金额保持稳定。

### 13.5 实施时执行的仓库检查

采用仓库已有命令，不把此前提到的 444 Rust / 50 前端数量作为固定事实；实施时记录实际基线和结果：

```text
pnpm test:rust
pnpm test:frontend
pnpm build
```

先执行改动相关的有意义验证，再在最终阶段运行上述必要检查。客户端配置与路由使用本地 fixture / mock 上游验证，不要求调用付费真实模型。页面验收覆盖空目录、离线、冲突、未计价和用户覆盖状态。

本文件创建阶段只整理计划，不代表已经运行这些验证，也不代表已经拉取本期真实 API 数据。

## 14. 发布、回退与完成标准

目录解析先进入 shadow 阶段，再切换消费者，最后删除旧表。每个阶段使用独立可审查的提交；实现前确认工作区现有改动，避免将其他任务修改混入本改造。

发布前保留迁移备份与配置恢复路径。发生目录下载故障时使用最后有效快照；发生新适配器问题时可回退到上一版本的客户端投影和配置备份。新增数据字段应兼容读取，回退不能清除用户原始 ID、协议、别名或覆盖。

本任务完成的定义：models.dev 已成为自动模型能力的主要默认来源；用户能够设置压缩比例和能力覆盖；带已知前后缀的模型匹配可解释；费用能够按真实观测范围累计；预览与注入共用解析结果；路由行为通过基线保护；旧族名数值表及前端重复规则已经删除。

## 15. 后续独立事项

- 由 models.dev provider 目录提供供应商搜索建议，但完整可用预设仍需端点、协议、认证和套餐适配。
- wire_id 与上游请求名称改写独立设计，不能借元数据规范化隐式实现。
- 其他客户端的压缩设置适配，在确认对应版本官方配置契约后增加。
- 原生直连客户端的用量导入、实际供应商账单对账与汇率换算另行实施。
- 多数票能力自动采用、更广泛别名库和复杂条件价格支持，根据未匹配统计再扩展。

## 16. 参考资料

- models.dev API：<https://models.dev/api.json>
- models.dev 仓库：<https://github.com/anomalyco/models.dev>
- Magpie：<https://github.com/yetone/magpie>
- 本地 Magpie：`D:/workspace/temp/magpie`，参考提交见文件开头。
- 本会话已通过 Context7 查询 models.dev 公开类型；实施 P0 仍需以届时实际响应与上游 schema 核验字段，不能把摘要中的缺省项当成完整契约。

## 17. 实施状态（分支 `feat/models-dev-integration`）

本节记录本分支已经落地的范围，供回退与后续迭代参考。

已实施：

- **P0**：拉取真实 `api.json`（5.28 MB / 225 供应商 / 8339 模型）核验字段；`limit.context|output` 全覆盖、`limit.input` 17%、`reasoning_options`（effort/toggle/budget_tokens）74%、`cost` 95%（含 `tiers`、`context_over_200k`）、`canonical_model_id` 63%；发现并兼容真实的 `budget_tokens.min = -1` 约定。
- **P1**：`scripts/build-models-dev-snapshot.mjs` + 提交的 `src-tauri/assets/models-dev.snapshot.json`；`models_dev.rs`（校验、索引、运行时缓存、原子刷新、ETag、失败保留旧数据）；`model_metadata.rs`（有序候选、显式/精确/规范化/通用/冲突、逐字段来源）；`model_policy.rs`（压缩策略与客户端投影）。
- **P2**：router catalog、native catalog、OpenCode 与能力预览共用 `model_projection`；官方账户快照只对官方端点生效；`catalogProviders` 映射与 `priceCatalogProvider` 绑定；能力面板展示匹配类型、候选轨迹、冲突、价格与来源；请求诊断记录目录版本、匹配类型与计价状态；`refresh_models_dev_catalog` / `load_models_dev_status` 命令与每小时后台检查。
- **P3**：全局默认 90%（可改为固定 token 或交由客户端）；模型 → 供应商 → 全局继承；预算优先使用 `limit.input`；无效阈值不注入；native 快照所有权记录推导值；供应商与模型级 UI。
- **P4**：usage 观察器拆分非重叠的输入/缓存读写计数；请求开始时固定价格快照并落账（含档位与 >200k 条件价）；账本与归档增加 micro-USD 费用与已计价/未计价数量；用量面板展示今日/近七日/累计预估费用、按供应商与请求的费用、每日与按模型的费用。
- **P5**：删除族名窗口表、精确 ID 思考表、`for_model`/`named_defaults`、`with_local_defaults`/`with_catalog_defaults` 以及前端族名数值规则；`docs/models-dev.md` 记录匹配规则、来源顺序、P0 字段覆盖、迁移与上游归属。
- **自查后补齐**：价格真正按 `priceCatalogProvider` 作用域取值（同一作用域价格冲突保持未知，不取均值；完整 ID 记 catalog，缩减候选记 catalogAlias）；压缩阈值显示“已生效/待同步/未检测”；逐模型目录映射与逐模型压缩策略编辑入口（含引用清洗）；用户价格覆盖在能力面板内联编辑并可一键恢复目录价格；近七日/每日/按模型费用；目录为空提示；P0 字段覆盖表。
- **分层与热路径**：目录查找改为 provider/model 哈希索引（请求热路径不再线性扫描）；请求兼容检查只解析被请求的那一条目录记录（与完整菜单共用同一个条目构造器）；`ReasoningLevel` 归入元数据层并由配置层重导出，去掉元数据→配置的依赖；诊断不再 import 用量账本的类型；legacy `autoCompactLimit` 不再出现在 ProviderView 契约中（仅保留在存储记录，供迁移与降级读取）。零价与缓存计数在界面分别显示为 `$0` 与“缓存 +N”。

仍属后续独立事项：

- 历史账单的显式“按当前价格估算”（§10.5）。
- 真实 Codex 客户端对 `auto_compact_token_limit` 的版本核验（§7.4.2）；当前按既有代码事实保留安全兼容行为。
- 固定阈值越界时只标记无效并拒绝注入，不阻止保存（避免在模型集合变化时误拦截）。
- 模型级 `priceRef`（独立价格引用）后端已支持，界面目前只提供供应商级价格绑定与模型级价格覆盖。
- `/codex/v1/models` 端点仍按请求重建整份菜单（该端点必须返回完整列表；单条查询已优化）。
- 多数票自动采纳、别名库扩展与复杂条件价格的自动档位推断（§15）。
- 原生直连的用量导入与真实账单对账（§15）。
- 供应商预设来自 models.dev 目录（§15）。

验证（本分支实际执行）：`pnpm test:rust`（463 个单元测试 + 9 个集成测试）、`pnpm test:frontend`（含 Playwright 回归与能力面板/价格覆盖/压缩策略/用量费用断言）、`pnpm build` 全部通过。
