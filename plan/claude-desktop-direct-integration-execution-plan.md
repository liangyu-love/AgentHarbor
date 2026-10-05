# Claude Desktop 直连接入实施计划

日期：2026-10-04

状态：**首期核心实现已落地，复审修复完成，完整发布验收仍未通过**；本文既是实施规格也是验收依据。

当前实施状态（替代早期阶段性结论）：

- 后端独立模块与命令层、供应商页卡片和对话框已落地，复用已有中转卡片；未接本地路由模式。
- 前端独立卡片与对话框 `src/components/newapi/claude-desktop-config-{card,dialog}.tsx`、`src/lib/claude-desktop.ts`、`src/types/claude-desktop.ts` 已接入 NewApiPage，复用既有中转卡片，不重新填写地址与密钥。
- 修复基线为六个实施提交的末端 `5030efd`。复审发现事务失败回滚、冲突门禁、进程身份与隔离来源等边界缺陷；已按优先级修复、补回归并完成修复自审。实际修复后测试结果见验收记录的“复审修复”节。
- 上游连通性已按用户指定的测试中转核对通过：卡片 `kiro-fucheers`（`https://www.fucheers.top/`）配 `claude-haiku-4-5-20251001`，`/v1/models` 与 `/v1/messages` 在**裸 Bearer**（Desktop 的鉴权形态、不带 `x-api-key`）下均为 200，且上游自报模型与请求一致。
- 已有 Windows + Desktop 2.19675.0 实施记录支持 L01/L02/L03/L05/L10 的核心链路，L09 由 loopback mock 验证限制路径；L07 仅文件层通过。复审修复不重新执行真实接管、真实进程控制或计费请求。
- 预览与连通性检测可在 Desktop 运行时执行；应用、恢复等配置写入前必须完全退出 Desktop（含托盘）。提交会重读与比较，文件变化时重新预览。成功完成精确安装的停止/启动后清除待重启标记；隔离或覆盖路径不提供真实进程控制。
- 原记录 A01–A40 为 22 项通过、18 项部分通过。补充回归不自动将整张验收表改为通过；发布仍按第 9.3 节门槛逐项核验。
- 尚缺 L04 的 Desktop 流式渲染观察、L06 真实恢复、L08“本地路由启动后退出”组合、L07 官方流程以及 macOS/Linux 真机验证；Cowork 首期不承诺。
- 实施后代码审查：删除 6 处冗余/死代码，把只读状态投影与预览票据从 1396 行的 `mod.rs` 抽出为 `status.rs` / `tickets.rs`，能力探测从 `probe.rs` 抽出为 `capability.rs`；对外行为不变。
- 实施中发现并修复的缺陷、逐条 A01–A40 映射与回滚路径见 `docs/claude-desktop-acceptance-record.md`；用户文档见 `docs/claude-desktop-direct-connection.md`。

项目基线：AiMaMi / AgentHarbor，`e8af8f8`（`fix(router): preserve tool identity and conversation affinity`）。

参考基线：本地 `D:/workspace/temp/cc-switch`，HEAD `1ee2fdc`；调研文档 `docs/claude-desktop-3p-injection-zh.md` 为该参考仓库的未跟踪文档，需与源码交叉核对。

首期交付：从已保存的中转卡片选择供应商，安全预览、应用、重新应用和恢复 Claude Desktop 的直连配置。后续再接本地路由。

## 1. 目标、范围与实施结论

本方案可行，但“已有中转测试通过”不等于“可以供 Claude Desktop 直连”。首期必须同时检查模型、协议、接口布局和 Bearer 鉴权，并把“文件已配置”与“真实对话已验证”分开显示。

用户的主要操作应是：

1. 打开现有中转页面的 **Claude Desktop** 卡片。
2. 选择一张已经保存的供应商卡片，无需重新输入地址、密钥或余额凭据。
3. 从该卡片的兼容模型中选取要展示的模型，可调整展示名称；支持 1M 的新选择默认勾选 1M，用户可自行取消。已保存的选择保持原有意图。
4. 查看配置文件的路径、操作和原值/新值，模型与地址包含在实际配置中；预览不要求退出 Desktop。应用前完全退出 Desktop（含托盘），文件已变化时重新预览。
5. 启动 Claude Desktop，在真实应用内确认模型菜单和对话。
6. 需要撤销时选择“恢复接管前配置”；确实需要官方模式时使用独立的“切换官方模式”。

首期与后续范围：

| 内容 | 首期直连 | 后续路由 |
| --- | --- | --- |
| 复用已保存的 Provider、模型协议和元数据 | 必须 | 必须 |
| 四文件事务、所有权记录、恢复、漂移检查 | 必须 | 复用 |
| Anthropic Messages + 上游接受 Bearer | 必须 | 由网关适配上游鉴权 |
| Desktop 合法 Claude 模型名原样发上游 | 必须 | 可以用安全角色别名映射真实模型 |
| Chat Completions / Responses 模型 | 不提供直连转换 | 复用已有协议转换后验收 |
| 项目自身退出后仍可使用 | 是，Desktop 自己连接上游 | 需要本地网关持续运行 |
| 复用项目全局代理和卡片网络出口 | Desktop 不会自动继承 | 经项目网关使用 |
| 项目路由诊断、费用账本 | 直连请求不经过项目，无法自动记录 | 接入既有管线 |
| 用户触发的 Desktop 重启 | 已实现，限可精确确认的安装 | 复用已验收的进程控制 |
| 自动进程接管、会话导入、桌面登录接管 | 不纳入 | 另行评估 |
| Cowork、任意网络访问策略 | 不承诺支持 | 独立验收后决定 |
| Gemini Native、托管 OAuth 等 CC Switch 专属能力 | 不纳入 | 本项目有适配器后再考虑 |

首期不扩展现有 `TargetId`，不改 `ClientAdapter` 的单文件契约，不增加一套 Desktop 供应商数据库。新增独立 Desktop 模块，只共享供应商、模型元数据和底层文件工具。

## 2. 参考事实与待验证行为

### 2.1 CC Switch 提供的实现证据

参考入口是 `src-tauri/src/claude_desktop_config.rs`：

- 两处 `claude_desktop_config.json` 写入 `deploymentMode: "3p"`。
- 在 `Claude-3p/configLibrary` 写入工具自己的 profile，并在 `_meta.json` 注册条目、设置 `appliedId`。
- profile 使用 `inferenceProvider: "gateway"`、`inferenceGatewayAuthScheme: "bearer"`、网关地址和推理凭据。
- 可选 `inferenceModels` 控制菜单；不写该键时，由 Desktop 请求 `<base>/v1/models`。
- 模型名受到 Desktop 内部校验器限制；一条非法模型可能导致整份 profile 拒收。
- 配置需要完全退出并重启 Desktop 才读取。
- CC Switch 的直连与路由共用 profile 外形，但直连保留模型原名，路由先将角色名映射到真实模型。

这些是本地源码及其版本观察的证据，**不是公开、稳定的 Claude Desktop API 保证**。P0 与最终验收必须记录实际 Desktop 版本、安装渠道、平台和行为。

### 2.2 不直接照搬的行为

| 参考行为 | 本项目决定 |
| --- | --- |
| 固定 CC Switch UUID / 名称 | 使用 AgentHarbor 自己的稳定 UUID，显示名 `AgentHarbor`，不得覆盖 CC Switch profile |
| 仅从 `ANTHROPIC_AUTH_TOKEN` 获取直连凭据 | 使用已有 `Provider.api_key`；字段名不决定上游支持的鉴权方式 |
| 某些非对象 JSON 当成空对象 | 已存在的共享文件根节点/关键结构不合法时阻止写入，保留原始内容 |
| 恢复官方时无条件删五个 `enterpriseConfig` 键 | 不删除外部企业策略；区分恢复原配置和切换官方模式 |
| 删除自己的 meta 条目后选择第一个外部 profile | 按 journal 恢复原 `appliedId`；不擅自选择外部条目 |
| 四文件内存快照与异常回滚 | 写前持久事务日志，覆盖进程中断及部分回滚失败 |
| 四档模型缺省 `supports1m=true` | 从有效能力和来源推导，未知不声明 1M |
| profile 总写 `coworkEgressAllowedHosts: ["*"]` | P0 先验证省略该键的最小推理 profile；不把省略视为已证实可行，不自动扩大权限 |
| Windows `Claude*` 候选排序选第一项 | 验证配置证据，多候选让用户选择，不误选 Claude Code/另一安装渠道 |

## 3. 如何直接使用现有中转

### 3.1 供应商与凭据唯一来源

前端复用已经加载的 `NewApiProvider[]`，仅提交 `providerId`、模型 ID 和 Desktop 展示设置。后端重新读取 `router::load_store()`，根据 ID 解析最新 `Provider`。

- 推理凭据来自 `Provider.model_credential().api_key`。
- `Provider.access_token` / `user_id` 是余额或后台管理凭据，禁止写入 Desktop profile。
- 不调用前端 `revealApiKey`，不把密钥放入请求参数、组件状态、localStorage、错误或日志。
- Desktop 设置保存供应商引用，不复制卡片，不分叉 API Key，不导入一份独立的 Claude Code 供应商。
- 不修改 `active_provider_id`、`codex_access_mode`、Provider 默认模型/协议、`enabled`、Claude Code 或 OpenCode 配置。

`Provider.enabled` 表示该卡片是否参与当前路由池。**Desktop 直连可以选择 `enabled=false` 的卡片**，只要凭据、协议、模型和端点满足条件；选择它不会自动启用路由池。

不要复用要求 `enabled` 的 `has_route_credentials()`、`can_route_model()`、`client_config::selected_provider()` 或前端 `exclusionOf()` 来判 Desktop 直连资格，也不要改变这些函数原有语义。新增 Desktop resolver。

### 3.2 模型选择规则

候选集来自该卡片已包含的模型：

1. 使用 `models` 中 `included=true` 的条目。
2. 卡片默认 `provider.model` 在模型目录中没有条目时，可作为已有手填模型候选。
3. 已有条目明确 `included=false` 时，不因其同时是默认模型而绕过排除设置。
4. 每个候选按 `Provider::protocol_for_model(modelId)` 与 `supports_model(modelId)` 判断；不能仅按卡片级 `protocol` 筛选。
5. `protocol_for_model()` 包含 Claude 名称推断，`supports_model()` 对未知模型也可能返回允许。因此这只是配置资格，不能冒充上游实测成功。候选返回 `protocolSource`，区分模型显式设置、端点声明、名称推断和卡片默认；推断来源显示未验证提示。显式模型协议为 Responses/Chat 时，即使名字符合 Desktop 白名单也拒绝直连。
6. 再按 Desktop 版本白名单、基础 ID 校验和地址布局过滤。
7. 前端显示可选项与排除原因，后端 preview/commit 重新验证，不能信任前端过滤结果。

同一卡片默认模型是 Responses、另外一个已包含模型是 Anthropic Messages 时，应允许单独选择后者，保持卡片默认模型不变。

首期只提供 **显式模型列表**，至少选择一个合法模型。卡片没有已知模型时，先利用现有卡片的获取/配置模型功能补齐，再回来选择。上游自动列举作为后续增强：整个 `/v1/models` 列表可能含非法 ID，不能在首期用省略 `inferenceModels` 绕开 fail-all 校验。

直连没有角色映射。`labelOverride` 只改变显示名，不能把 `kimi-k2`、`gpt-*`、`providerId::modelId` 或旧式 Claude 名伪装成可直连模型。

### 3.3 资格与检测分开建模

后端候选返回结构化原因，至少包含：

| 原因码 | 行为 |
| --- | --- |
| `provider_missing` | 无法应用；原配置仍允许恢复 |
| `no_inference_key` / `invalid_inference_key` | 无法应用；提示到原卡片补推理密钥 |
| `incompatible_protocol` / `unsupported_model_endpoint` | 排除该模型，不替它修改卡片协议 |
| `model_not_included` / `model_removed` | 重新选择，不默默回退另一模型 |
| `unsafe_model_name` / `unsupported_desktop_role` | 排除该模型，保留原始 ID 便于定位 |
| `endpoint_layout_unsupported` | 无法直连，提示后续网关可解决 |
| `bearer_unverified` | 提示未验证；允许显式应用配置后实测 |
| `bearer_check_rejected` | 明确显示检测被拒；重新检测/修正卡片，不标记兼容成功 |

检测被拒只说明当次 `/models` 检查返回拒绝，不充分证明 `/messages` 永远不支持 Bearer。若用户仍选择应用，预览必须明确保留此状态，应用后也不把它改成已连接。

### 3.4 地址投影与接口布局

已有 URL helper 可复用基础校验、去末尾 `/v1`、保留自定义前缀以及 DeepSeek 的协议根路径处理，但 Desktop 不执行本项目的端点特例。

新增 `resolve_direct_endpoints(provider)`：

1. 校验 HTTP(S)、主机、无 userinfo/query/fragment；复用现有基础校验。
2. 拒绝把 `/v1/messages`、`/v1/responses`、`/v1/chat/completions` 等完整接口地址当 root；不把已有错误地址偷偷改成另一种含义。
3. 按 Anthropic Messages 调用 `protocol_base_url()` 得到有末尾 `/` 的标准根，用于拼接。
4. 构造 Desktop 将实际访问的 `root + v1/messages`、`root + v1/models`；profile 中保存没有末尾多余 `/` 的根地址，具体拼接行为需 P0 验证。
5. 将 Desktop 的 Messages 地址与 `newapi::provider_endpoint(provider.base_url, "v1/messages")` 比较。若不一致，返回 `endpoint_layout_unsupported`。
6. 不做字符串全局替换，不改 Provider 原地址，不将 models.dev 的标准化 ID 当请求 ID。

| 已有卡片地址示例 | Desktop 根 / 实际请求 | 处理 |
| --- | --- | --- |
| `https://relay.example`、`https://relay.example/v1` | 根 `https://relay.example`，请求 `/v1/messages` | 正常投影，避免 `/v1/v1` |
| `https://relay.example/custom/v1` | 根 `/custom`，请求 `/custom/v1/messages` | 保留自定义前缀 |
| `https://api.deepseek.com` | 协议根 `/anthropic`，请求 `/anthropic/v1/messages` | 可投影地址；模型白名单另检，地址可投影不等于整体可用 |
| `api.novita.ai/v3/openai` | 项目端点会去掉 `v1`，Desktop 会追加 `v1` | 布局不一致，首期拒绝 |
| `open.bigmodel.cn` / `api.z.ai` 的 `/api/{coding/}paas/v4` | 同样存在 non-v1 特例 | 首期拒绝，不猜测另一 URL |

HTTP 地址保持与已有卡片能力一致，但预览说明非 TLS 连接会发送明文 Bearer；不会静默升级 HTTPS，也不会降低 TLS 验证。

### 3.5 Bearer 检测与真实可用性

现有 `provider_probe` 和 `newapi::anthropic_models` 使用 `x-api-key + anthropic-version`。其成功结果不能复制为 Desktop Bearer 通过结果。

新增独立、用户点击触发的只读检测：

- 对 **Desktop 实际 `/v1/models` 地址** 请求，使用 `Authorization: Bearer <Provider.api_key>`，不以 `x-api-key` 成功代替 Bearer 成功。
- 密钥可复用现有 header 校验与前缀清理规则；profile 只存原始 token 值，不把 `Bearer ` 前缀写进 key 导致双前缀。
- 不自动发送计费 Messages 请求；不改 Provider 的原 `test_status`。
- 保留现有连接超时、响应大小上限、禁止自动重定向等边界；返回状态/结构化错误，不返回响应原文中的密钥或 HTML。
- 区分 `notChecked / modelsAccessible / authRejected / endpointUnavailable / networkFailed / invalidResponse`。
- 401/403 标记检查被拒；404 表示列表端点不可用；429 表示限流；200 HTML/错误格式不算通过。
- 验证模型列表格式和选中 ID 是否可见，给出提示，不用列表结果静默替换已选择模型。
- 检测使用项目自身的网络设置时，结果注明检测出口；Desktop 使用自身网络，两者出口不同，检测成功仍不等于 Desktop 连通。
- 结果绑定供应商连接/凭据指纹、选中模型及时间；相关输入变化后失效。

最终可用性只由真实 Desktop 消息、菜单、多轮/流式验收确认。生产 UI 首期显示“配置已应用，请重启验证”，不凭写文件成功显示“已连接”。

### 3.6 1M 与现有模型能力的消费

复用 `router::resolve_model_metadata()` / `model_metadata` 的有效 context 与来源，不新增模型族名推断表。

- `supports1m` 自动值只在非 fallback/unknown 的有效上下文达到 1,000,000 时开启。
- 未知、产品默认兜底、只有模型名称猜测时，默认不声明 1M。
- 支持 1M 的新选择默认勾选 1M，用户可自行取消；已有 Auto/Off 意图保持不变。`supports1m: true` 会让 Desktop 同时显示普通与 1M 选项，这不是重复写入。需要声明更大能力时先在已有能力面板配置有来源的覆盖，再重新预览。
- 该设置使用单击复选框；没有有效 1M 能力时禁止新开启，但允许取消原有 Auto。它控制额外菜单变体，不设置上游模型容量；Desktop 普通项的本地预算需由其版本/SDK识别与真实对话另行核验。
- Desktop 使用对象键 `supports1m`；**不写 Claude Code 的 `[1m]` 后缀**。
- label 默认用已有模型展示名，空白回退原 ID；限制长度、去除控制字符，不改变 model ID。
- 不把 output、reasoning、价格、90% 压缩阈值写入未证实支持的 Desktop 字段。首期能力投影不借用 Codex 官方快照兜底。

## 4. 模块与现有结构的兼容边界

### 4.1 推荐文件布局

```text
src-tauri/src/core/claude_desktop/
  mod.rs                 # Manager / 状态 / 操作编排
  types.rs               # request、view、session、transaction 内部类型
  paths.rs               # 平台发现、目录选择、隔离测试路径
  provider.rs            # Provider 引用、模型资格、URL/能力投影
  profile.rs             # 纯函数：profile、meta、deploymentMode 补丁
  transaction.rs         # 四文件 revision、持久 journal、恢复
  probe.rs               # 可选 Bearer /models 检测
  tests.rs               # 同模块单测；规模扩大再按职责拆
src-tauri/src/commands/claude_desktop.rs
src/types/claude-desktop.ts
src/lib/claude-desktop.ts
src/components/newapi/claude-desktop-config-card.tsx
src/components/newapi/claude-desktop-config-dialog.tsx
scripts/test-claude-desktop-config.mjs
```

文件按真实职责拆分，不引入通用插件框架或第二个供应商服务。小型纯函数可以留在对应文件，不为每个字段另建层。

### 4.2 现有文件修改清单

| 位置 | 最小修改 | 不应顺带改动 |
| --- | --- | --- |
| `core/mod.rs`、`commands/mod.rs`、`lib.rs` | 注册 Desktop 模块、命令及启动恢复入口 | 既有命令签名 |
| `src/lib/api.ts` | 新增 `api.claudeDesktop` | `api.targets`、路由 API 的现有请求形态 |
| `src/lib/query-keys.ts` | 新增 `claudeDesktopConfig` | 已有客户端 key |
| `newapi-page.tsx` | 在现有客户端区旁加入独立 Desktop 卡片，传已有 providers | 卡片树、路由选择、诊断布局 |
| `use-provider-state.ts` 或页面更新事件 | Provider 变更后使 Desktop 状态失效 | 自动写 Desktop 原生文件 |
| `locales/zh.json`、`en.json` | 独立 `claudeDesktop.*` 文案、原因码 | 复用 Claude Code 名称造成混淆 |
| `storage.rs` / `secret_file.rs` | 直接复用；确有必要时只补共用文件权限能力 | 重写现有事务、平台存储 |
| `router.rs` / `model_metadata` | 调用已有查询函数；必要时窄化共享只读 helper | Store version、Provider schema、价格解析与默认路由语义 |
| 前端 fixture / `local_workflows` | 增加隔离场景与真实断言 | 把真实本机配置当 fixture |

现有 `TargetId` 同时关联会话源、进程重启与配置，只有 Codex 有完整会话/进程能力。首期独立配置能力不扩该枚举，避免 Desktop 误走 Codex 分支。UI 可以位于同一区域，API 与数据模型保持独立。

现有 `ClientAdapter.path()`、`Session.path/original/installed`、`ConfigPreview.path/content` 都面向单文件。Desktop 的四文件事务独立实现；共享低层 `storage::write_atomic()`、`remove_file_durable()`、`secret_file::{read,write}`，不先改全项目的适配器抽象。

## 5. 路径、版本和 profile 契约

### 5.1 路径发现

| 平台 | normal root | 3P root | 首期支持声明 |
| --- | --- | --- | --- |
| Windows | `%LOCALAPPDATA%/Claude` | `%LOCALAPPDATA%/Claude-3p` | 本机优先真实验收 |
| macOS | `~/Library/Application Support/Claude` | 同目录 `Claude-3p` | 解析单测；实测后声明支持 |
| Linux | 绝对 `$XDG_CONFIG_HOME`，否则 `~/.config` 下 `Claude` | 同根 `Claude-3p` | 按具体发行版/安装渠道验收，不因代码分支存在声明官方可用 |
| Flatpak 等沙箱 | 需显式确认宿主/沙箱配置目录 | 与确认的 normal root 配对 | 默认不按 CC Switch 宿主路径策略直接宣称支持 |

规则：

- 精确目录名优先，但仍要有安装/配置证据，不能因本工具创建了目录就宣称已安装。
- 多渠道/候选目录时返回候选列表，用户明确选择两根匹配的路径；不按字典序挑 `Claude*`。
- 后端验证 normal/3P 根是同一安装渠道的两个不同目录，规范化后的四个目标路径不能指向同一文件；路径选择 ID 不能替代这一校验，异常时零写入。
- 3P 目录缺失但 normal root 已确认时允许创建；normal root 未确认时返回 `notDetected`，不凭空创建“已配置”的假安装。
- 版本探测失败用 `unknown`，不编造版本；普通角色仍可候选，显示需要实际验收。
- preview 固化路径；apply 检查路径身份；reapply/restore 使用 session 路径，不重新发现到另一安装。
- `AGENT_HARBOR_CLIENT_HOME` 生效时，所有 Desktop 查找、版本证据与四文件读写落入隔离根，禁止再扫描生产 `%LOCALAPPDATA%`/用户目录。
- 支持显式 `DesktopPaths::from_roots()` 的测试依赖注入。目标文件名由后端固定，前端不能提交任意文件路径。
- 路径重定向、链接/reparse identity 在预览后变化时作 revision 变化处理；不跨到未知目标继续写。

### 5.2 四个外部文件

```text
<normal>/claude_desktop_config.json
<threep>/claude_desktop_config.json
<threep>/configLibrary/<AGENT_HARBOR_PROFILE_ID>.json
<threep>/configLibrary/_meta.json
```

产品 profile 使用不同于 CC Switch 的固定 UUID；P0 生成并锁定常量，后续升级不得变更。同一产品反复应用只保留一个自身条目，不创建多份 profile。

共享 JSON 的处理：缺失文件可按对象新建；已有语法错误、非对象根、meta.entries 非数组、无效 appliedId 类型等返回具体文件与原因，不把它们当 `{}` 丢弃。无关 JSON 键和外部 entry 原样保留；应用 AgentHarbor 自己的完整 profile 时也必须先识别现有文件和基线，不能仅凭同名推定可删。

### 5.3 最小 profile 候选

```json
{
  "disableDeploymentModeChooser": true,
  "inferenceProvider": "gateway",
  "inferenceGatewayAuthScheme": "bearer",
  "inferenceGatewayBaseUrl": "https://relay.example/custom",
  "inferenceGatewayApiKey": "<后端写入真实推理凭据，预览固定脱敏>",
  "inferenceModels": [
    "claude-sonnet-4-6",
    { "name": "claude-opus-4-6", "labelOverride": "已有中转 · Opus", "supports1m": true }
  ]
}
```

上面的模型仅为合法形态示例，不预置为用户可用模型。没有 label/1M 选项时使用字符串；有选项时使用对象。false 的 1M 默认省略，保持参考输出形态。

CC Switch 始终额外写 `coworkEgressAllowedHosts: ["*"]`，没有省略成功的源码证据。**省略该字段是 P0 待验证的最小候选**：必须验证启动、模型菜单、普通真实请求、多轮、重启和恢复。失败则阻断该候选交付，记录版本与失败表现，再评审有依据的最小策略；不得失败后自动写入 `*`。首期不承诺 Cowork 可用。

`disableDeploymentModeChooser=true` 与参考一致，预览说明会隐藏应用内模式选择；项目必须提供恢复路径。Desktop 可读取的 profile 必须为普通 JSON，不能用 `secret_file` 把客户端文件加密。

### 5.4 模型名与版本策略

- 先做非空、长度、控制字符、空白、`[1m]` 检查；原 ID 不能因校验而转成另一上游 ID。
- 首期采用保守的 `claude-{sonnet|opus|haiku}-<非空后缀>` 及 `anthropic/claude-...` 形式；保留原值，非规范大小写/空白 ID 不能静默归一后写入。
- 旧式 `claude-3-5-sonnet-*`、空后缀、任意其他品牌、路由复合 ID 都排除。
- CC Switch 注释提到 1.12603.1+ 内部白名单包含 Fable，旧版观察只含 Sonnet/Opus/Haiku。这只是版本证据；Fable 仅对本项目真实验收过的版本/渠道开启，unknown 默认关闭。
- 不开放 Mythos 等未纳入参考可用菜单的角色，不由 substring 自动放行。
- 所有选中模型逐条校验，任何一条失败都阻止该配置提交，返回具体 ID/原因。已选非法项不能被静默删除后把缩水列表算作成功。
- 单独展示 `labelOverride`，它不能参与 ID 校验/协议选择。

## 6. 四文件事务、所有权与恢复

### 6.1 管理状态与存储

Desktop 内部状态独立放在 `<app_data_dir>/claude-desktop/`：

```text
state.dat                    # schemaVersion、路径、请求引用、接管基线、owned patches
pending.dat                  # 未完成事务，包含旧/新 state 与四文件 before/after
backups/<transactionId>.dat   # 人工恢复备份；参考既有保留策略，最多最近 20 份
```

所有包含文件快照或凭据的内部记录通过 `secret_file` 写入：Windows 使用当前用户 DPAPI；Unix 使用权限 0600，**现有工具在 Unix 不提供内容加密，不能文案声称全平台加密**。

Desktop 的外部 profile 需要保存可读取的推理凭据，这是直连格式本身的限制。预览明确说明这一点；profile 写入复用 Unix 0600，Windows 验证配置目录的当前用户 ACL 继承，不扩大共享目录权限。不创建无保护的明文备份或日志副本。

逻辑设置保存 `{providerId, models[{modelId,labelOverride,oneMMode}], pathsIdentity}`。只有事务/恢复快照可以包含已经写入或原有的秘密；不把密钥复制进一份新的逻辑供应商配置。

状态保留首次接管基线、最近安装值/指纹、每个受管字段和自身 meta entry。普通 reapply 更换供应商时保持最初恢复点；只有显式 rebaseline 才采用当前配置作为新恢复点，旧备份仍按保留策略可查。

### 6.2 预览与 revision

预览读取四个文件、当前管理状态、Provider 和所选模型的有效投影，形成不可变 `PreparedChange`。revision 覆盖：

- 操作种类、接管/rebaseline 选项与请求参数。
- 两根路径及身份、四文件是否存在、原始内容、管理状态版本。
- 当前 Provider 的推理密钥和投影后地址、所选模型是否仍被包含、有效协议与影响 profile 的 label/context 来源。
- Desktop 能力/版本策略及最终 profile 输出。

余额、价格刷新、原路由开关等无关字段变化不应无故使直连预览失效；影响实际 profile 的变更必须失效。

对前端返回不含密钥摘要的 opaque preview ticket；后端保存相应 snapshot fingerprint，绑定操作与路径，提交时重读比较。ticket 单次使用、应用重启后失效，首期采用 5 分钟有效期、每个配置目标最多 8 个未消费 ticket。不要让前端提交真实 profile JSON/补丁，不返回可独立用于猜测密钥的 fingerprint。

预览 DTO 只展示**受管变更**：共享文件不回传完整 JSON，避免泄露未知扩展字段中的凭据；profile 的 key 前后值都固定替换为 `<redacted>`。`_meta` 展示自身条目/应用指针及“保留外部条目数”，不输出外部 profile 原文。

### 6.3 锁与一致性

所有 Desktop 写操作沿用命令层 `routing_runtime::mutation_guard()`，与已有 Provider 保存/删除串行，**这不表示直连要启动代理**。再进入 Desktop 事务锁，短时加载 Store/状态并提交；不在持有同步 mutex 时等待网络。

固定顺序：取 Repository 路径后释放 Repository mutex → await mutation guard → Desktop 事务锁 → 短时 Store 读锁。Store 读锁在进行四文件 I/O 前释放。preview/probe 读取快照后释放锁；网络检测不占全局 mutation guard，完成后检查 fingerprint 才记录结果。

合作 writer 的 pending/state 用文件锁或项目已有单实例约束保护；不能只靠进程内锁允许两个 AgentHarbor 实例同时管理同一目录。检查现有单实例机制后选一个最小可靠方案，P2 必须覆盖两个独立 writer 的竞争场景。

Desktop/CC Switch 不遵守本项目锁，必须在写前重读与比较，能检测已经发生的变更；重读与原子替换之间仍有外部写入窗口，现有文件工具不提供跨工具 CAS 保证。应用/恢复时要求完全退出 Desktop 并暂停其它配置管理器，不承诺任意并发外部写入都绝不覆盖。所有权回读和异常 journal 用于发现/恢复可辨识冲突，不能取代这项操作条件。

### 6.4 提交流程

```text
prepare：解析 Provider、模型、四文件；检查 drift/policy；生成脱敏预览与 ticket
commit：取得写锁；重读全部输入；不一致则 changed，要求重新预览
  -> 写入持久 pending（before/after/previousState/nextState/operation）
  -> 写入受保护 backup；此步失败则不改外部文件
  -> 按顺序逐文件 write_atomic/remove_file_durable，写前再次校验该文件
  -> 写完逐项读回，确认内容和结构
  -> 持久写入 nextState（含 transactionId/generation；这是提交点）
  -> 删除 pending；清理失败时仍是已提交，返回清理警告并禁止下一笔写入直到确认清理
提交点之前失败 -> 按 before 快照/owned patch 逆向恢复已改项
  -> 全部恢复成功：还原 previousState，移除 pending，返回原失败
  -> 部分恢复失败：保留 pending，返回 recoveryRequired 与失败文件
```

应用顺序选择：自己的 profile → 注册/选中 meta → 3P config → normal config。准备 profile 后再切 normal mode，减少指向尚不存在 profile 的时间。恢复优先撤回自己的模式/指针，再清理已不再引用的自有 profile；确切顺序按操作生成，不让 meta 临时指向被删文件。

这仍是四个单文件原子替换的协调事务，**不是四文件同时可见的系统事务**。预览保持只读且允许 Desktop 运行；应用前必须完全退出 Desktop。配置提交不自动终止进程，显式重启按钮只停止并启动已确认的安装。真实验收需要验证应用过程和重启时的行为。

未完成 journal 不清理、损坏 journal 不当作“没有管理记录”。启动在提供写操作前扫描 Desktop pending，并按持久提交点判定：

- state 的 transactionId/generation 与 pending.nextState 一致，且四文件仍等于 after：事务已提交，只读回核实并完成 pending 清理，不回滚。
- state 仍等于 previousState：事务未提交，按 before/after 与 owned patch 安全恢复；不能因为某文件已写完就视为整体完成。
- state 已提交但外部文件又有受管漂移，或 state 不匹配前后任一版本：保留证据，报告 recovery conflict，不自动覆盖/回滚。

在未提交恢复过程中只撤回能证明属于事务的值；清理确认失败或存在冲突时阻止新的应用覆盖证据。restore/release 等结束管理的操作同样写入带 transactionId/generation 的 tombstone，不能直接删除 state 丢失提交判据。

### 6.5 漂移与安全恢复

恢复不依赖 Provider 仍存在，也不需要连接上游。基线包括文件原先不存在的情况。

- 共享 config 只管理 `deploymentMode`；meta 只管理自己的 entry 和接管期间的 `appliedId`。entries 用稳定 profileId 管元素级 before/after，保留外部条目顺序与未知字段，不按数组下标恢复。
- 当前受管值仍等于 installed 值：可以撤回；外部新增 MCP、其他键、其他 meta entry 保留。
- 当前受管值已被外部修改：返回 `conflict`，提供基于最新现状的重新接管预览或释放管理，不默认 force 覆盖。
- 自己 profile 是整体受管；内容被外部更改时冲突，不凭 UUID 删除修改后的文件。
- 无漂移时可还原原字节；存在无关修改时用字段/自身条目级 undo，不整体还原旧文件抹掉新修改。
- 原本不存在的共享文件，在删除自己的字段后仍有外部内容则保留；确实只含本次生成内容时可删除。不得递归删除整个 Claude/Claude-3p/configLibrary 目录。
- meta 恢复 prior `appliedId` 时验证原 entry/profile 仍存在；原先无指针则移除自己的指针。原 profile 被外部删除时，返回可解释的冲突，不随机选第一条。
- 崩溃恢复根据每项 before/after 与当前受管值判定“未写/已写/冲突”，不能只信 progress 计数：文件写入与 journal 更新之间也可能中断。
- 回滚再次中断或失败可重复执行，只有当前与预期吻合才写，恢复必须幂等。

### 6.6 操作语义

| 操作 | 文件变化 | 管理记录 |
| --- | --- | --- |
| 应用直连 | 合并两处 3p，写自身 profile，注册/选中自身 entry | 保存首次基线及本次安装值 |
| 普通重新应用 | 从最新 Provider 重新构造 profile；校验受管值未冲突 | 保留首次基线，更新最近安装值 |
| 以现状重新接管 | 最新预览明确显示将覆盖受管冲突；不动外部无关字段 | 当前现状成为新基线，保留旧备份 |
| 恢复接管前配置 | 撤回 own patches，恢复 prior pointer/profile；可能恢复另一 3P 工具 | 结束当前管理会话 |
| 切换官方模式 | 两处模式明确为 1p；清除仍指向自身的 appliedId，撤回自身 entry/profile；保留外部条目且不选另一外部 profile | 首期限完整受管会话；结束管理，执行前保存可恢复备份 |
| 释放管理 | 不写任何 Desktop 文件 | 结束恢复责任，保留非秘密 ownership tombstone/旧备份供识别；下次不能默默删除遗留 profile |
| 恢复未完成事务 | 安全还原 before，或展示受管冲突 | pending 成功恢复后才清除 |

恢复前配置与切换官方模式不能共用模糊的“恢复官方”按钮。检测到 `enterpriseConfig` 中推理/部署相关字段时保留，并提示可能覆盖本次 profile；已明确冲突则阻止声称接入完成。切官方同样不自动删除企业键，不能保证企业环境下的官方登录行为，状态给出原因。

`switchOfficial` 不使用普通 restore 的 appliedId undo：当前 pointer 仍为 AgentHarbor 时移除自身指针，不恢复 prior foreign pointer，不选择外部 entries；当前 pointer 已被外部切换则冲突，不能清掉那条外部指针。没有完整 journal 时首期不提供此管理动作，也不猜测哪些 profile 可以删除。自身 ID 在接管前已有内容时，只撤回可证明由本次管理安装的内容，原有 profile/entry 留存于备份并按所有权恢复规则保留，不凭同 ID 清除未知配置。

释放管理会使“恢复接管前配置”失去当前会话入口，预览说明该影响；已有 profile 仍包含凭据。再次应用需要新的明确接管预览，不按名字把遗留内容当垃圾清除。

### 6.7 供应商后续变更

原生 profile 是凭据和连接设置的快照，不会自动跟随中央 Provider：

| 事件 | Desktop 状态/行为 |
| --- | --- |
| 改 URL、推理 key、所选模型协议/包含状态、有效 1M 能力 | `stale`，说明待重新应用；不偷偷重写 |
| 改卡片名称/所选展示名，影响 profile label | `stale` 或明确可刷新展示；以是否改变输出为准 |
| 删 Provider | `orphaned`，仍可恢复；不从备份 key 自动当活跃 Provider 继续应用 |
| 仅关闭 Provider 路由池 enabled | 直连继续有效，不自动启用/恢复/撤销 |
| 修改余额 token、账本价格、全局路由默认值 | 不改变 Desktop 直连 profile |
| 项目正常退出或关闭现有路由 | 直连保持；不纳入 router client 退出恢复 |

禁用/删除卡片或释放管理不等于撤销已经落在 Desktop profile 中的上游密钥。UI 应在相关状态变化中准确说明快照仍在，用户通过恢复/切换与上游密钥轮换完成撤销。

## 7. API、状态与 UI 契约

### 7.1 独立 API

建议保持五类命令，避免一套相同事务重复实现多次：

```text
get_claude_desktop_status()
list_claude_desktop_candidates(pathsChoice?)
preview_claude_desktop_change(action)
commit_claude_desktop_change(action, revision)
probe_claude_desktop_provider(providerId, modelIds, pathsChoice?)
```

`action` 为独立 tagged enum：`applyDirect / reapply / restorePrevious / switchOfficial / release / recoverRollback`。重新接管通过 `applyDirect` 的显式 `rebaseline` 选项表达；所有会产生文件/管理记录变化的操作先预览再提交。UI API 可提供便捷方法，后端共用 prepare/transaction，不另走无 revision 的恢复捷径。

`applyDirect` 只接受供应商 ID、模型设置、后端给出的路径选择 ID、接管选项。首期不接受 `router` mode、不接收 base URL/API Key；为后续增加 mode 可以正常版本化扩展，无须现在发布未实现字段。

### 7.2 DTO 必须能被前端直接消费

| DTO | 必要字段 |
| --- | --- |
| `DesktopStatus` | detection/version/platform、paths摘要、mode、managementState、isManaged、providerMissing、providerId/name、model摘要、staleReasons、conflicts、recoveryState、warnings、lastAppliedAt、restartRequired、actions |
| `DesktopCandidates` | providerId/name、资格原因、每模型 ID/label/协议/protocolSource/1M 来源/排除原因、端点摘要、Bearer 检测状态 |
| `DesktopPreview` | revision、action、provider摘要、实际 Messages/Models URL、files[]、warnings、conflicts、requiresRestart |
| `FileChangeView` | role、path、create/update/delete/noop、仅受管 changes[]；秘密值固定脱敏 |
| `DesktopApplied` | transactionId、changedPaths、backupId、managementState、requiresRestart、warnings |
| `DesktopError` | 稳定 code、fileRole/模型 ID 等安全 params、retryable、建议 action；无原始密钥/body |

`DesktopStatus.managementState` 是卡片的主显示状态，优先级为 `recoveryRequired > conflict > orphaned > stale > managed/unmanaged`；不能作为全部事实的唯一来源。`isManaged`、`providerMissing`、`staleReasons[]`、`conflicts[]`、`recoveryState` 独立保留，所以密钥过期与文件冲突可以同时显示。`mode` 单独为 `unknown / official / foreignThirdParty / direct`；actions 从真实条件计算，不能只按主状态枚举猜操作资格。

`restartRequired` 表示配置仍待重新加载；写文件后为 true，无改动应用不清除原有待启动标记。显式重启按钮成功完成选定安装的停止与启动后持久置 false，失败则保持原值。该标记不代表真实请求验证；外部手动启动目前不自动升级为已确认重载。首期不增加无法自动证明的 connected 字段。

actions 给出每项是否可用及禁用原因，后端仍检查。Provider 不存在时允许 restorePrevious；pending 时只允许恢复/查看；官方/外部 profile 不能被误标成“AgentHarbor 已管理”。

### 7.3 UI 流程

- NewApiPage 在现有客户端区旁显示独立 Desktop 卡片，沿用当前卡片、边框、间距与 Dialog 组件，不把整个页面改成新的视觉体系。
- 标题明确 `Claude Desktop`，不重复显示独立性说明。直连模式首期固定显示，不提供可点击但未实现的路由开关。
- 配置对话框按“选择已有卡片 → 模型/能力 → 预览”推进；候选灰项给具体原因，关闭路由池的卡片不灰掉。
- 展示已经使用的地址和“已保存推理密钥”，不显示密钥值，也不索要余额 token。
- 模型列表支持子集选择和 label，额外 1M 选项用复选框；协议来源保留在后端，不重复展示推断徽标。不复用 Claude Code 的 Sonnet/Opus/Fable/Haiku/Subagent 环境变量表单，不强制补满四档。
- 预览按四个文件分组，显示路径、修改键、create/delete 等动作与脱敏差异，说明外部 profile/MCP 保留以及原选择将被切换。
- 有外部 3P 时明确显示接管前选择及恢复方向；用户的“应用”操作确认这份预览，不另加无依据的重复确认。
- `changed` 返回保留表单、刷新预览，不能直接 retry 旧 ticket。
- 任意表单输入变化或相关 Provider 刷新立即清除旧预览；用 request snapshot/递增序号忽略迟到的异步预览/检测结果。提交使用当前界面仍在展示、且对应当前表单的固化归一化 action + revision，不能用修改后的表单配旧 ticket，也不能在B表单提交存下的A请求。提交期间冻结相关输入；关闭对话框/更换供应商使未完成预览失效。
- 应用前检查完整退出（包括托盘进程），应用成功后提供已确认安装的 Desktop 启动/重启入口，不调用 Codex restart。
- 操作成功只刷新 Desktop 查询；供应商列表改变时刷新 Desktop 候选/状态。不要为直连刷新或启动路由进程。
- 在 80%/100%/125%/150% 缩放与窄窗口检查四文件预览、长模型名/路径，保证不遮挡确认、恢复按钮。

## 8. 按依赖顺序实施与提交

各批次结束更新本计划状态和验收记录。本文不要求把真实凭据、截图中的密钥或整份本机配置提交 Git。

| 批次 | 工作 | 依赖 | 交付/完成标准 | 建议提交 |
| --- | --- | --- | --- | --- |
| P0：契约确认 | 最小 profile、Desktop 版本/路径证据、Bearer/URL行为、Cowork 字段候选验证；锁定独立 UUID、DTO | 无 | 当前 Windows Desktop 普通对话链路证据；不满足时记录阻断，不能跳过 | `docs(claude-desktop): verify direct profile contract` |
| P1：纯逻辑 | Provider resolver、候选/协议、地址投影、ID 白名单、1M来源、profile/meta补丁、路径解析 | P0 契约 | 完整 fixture 单测；不读取生产目录；卡片 Store 无写入 | `feat(claude-desktop): resolve saved relays and direct profiles` |
| P2：事务 | 独立 state/pending/backup、preview ticket、四文件提交、字段/entry所有权恢复、崩溃/竞争 | P1 | 每一步故障注入、断点重启与冲突保留通过 | `feat(claude-desktop): add recoverable multi-file configuration` |
| P3：命令/状态 | 注册 API、候选/状态/preview/commit、Bearer只读检测、启动pending恢复 | P2 | 无密钥 DTO；Provider变化失效；不触碰路由生命周期 | `feat(claude-desktop): expose direct configuration commands` |
| P4：UI | 独立卡片/表单/多文件预览、恢复/官方/接管语义、i18n、响应式 | P3 | 前端 fixture 断言、键盘与缩放验收；复用卡片无需重填 key | `feat(ui): configure Claude Desktop from saved providers` |
| P5：联调验收 | 新旧客户端回归、真实 Desktop 请求、外部工具共存、恢复、用户文档 | P4 | 第9节首期必测全部通过；保留限制声明与结果 | `test(claude-desktop): validate direct configuration workflows` |
| R0：后续设计 | 第10节路由映射/鉴权/生命周期契约，单独修订实施任务 | 首期稳定后 | 明确入口、provider引用、价格身份与listener lease | 单独计划，不混入首期提交 |

P0 的真实操作在实施阶段使用用户选择的测试卡片和可恢复配置完成；本次只编写文档，不进行真实注入或发起计费请求。若 P0 没有可用 Desktop/凭据，可以继续纯函数与 fixture 工作，但首期不能判定验收完成或宣称可运行。

P0 不依赖尚未开发的正式事务/API/UI。先做 P0a 只读记录版本、安装目录和当前配置结构；P0b 用限定实验 harness 或受控手工操作验证最小 profile：完整受保护四文件备份、独立测试 UUID、一个 loopback mock gateway（记录 URL/Bearer、返回合法 models 与 Messages/SSE）、完全退出 Desktop 后应用，结束立即恢复备份并重新启动核验。这样先证明 Desktop 的文件/拼接/鉴权/省略字段行为，且不会因 mock 请求产生上游费用。供应商实际 Bearer 可用性由选定测试中转补验，正式 resolver/事务和外部工具共存恢复留在 P5 的 L02/L06/L10 验收。实验 harness 不作为生产模块交付，也不能把手工恢复成功当作 P2 事务通过。

P2/P3 每个命令必须证明：没有调用 `set_active`/`set_enabled`/`ensure_available`，没有修改旧 ClientConfigRequest/TargetId。依赖新的共享 helper 时，证明其旧调用输出一致。

## 9. 验收矩阵与执行方法

### 9.1 自动测试

| 编号 | 场景 | 预期 |
| --- | --- | --- |
| A01 | 已有兼容卡片、只提交 providerId | 后端复用 base/key；前端请求/返回均无 key |
| A02 | `enabled=false` 的兼容卡片 | 可以直连；enabled/全局默认/路由配置完全不变 |
| A03 | 只有余额 access_token，没有 api_key | 阻止应用，不借余额 token |
| A04 | 混合卡片默认 Responses，子模型 Messages | 可以只选子模型，不改默认 |
| A05 | endpoint_types 与推断协议冲突、显式协议Responses但ID为安全Claude名 | 排除模型，不因含claude名绕过；推断来源保留在候选数据，不重复显示徽标 |
| A06 | included=false、已移除/失效的选中模型 | preview/commit具体报错，不选别的模型兜底 |
| A07 | safe三角色、anthropic前缀 | 保留原 ID、正确 profile |
| A08 | 任意品牌、旧Claude布局、空后缀、[1m]、控制字符 | 逐项报具体模型原因，整次不写 |
| A09 | Fable + unknown/已验收版本 | unknown排除；仅已验收策略允许；Mythos不放行 |
| A10 | 根、末尾/v1、自定义前缀、DeepSeek | 显示和写入正确 Desktop 实际地址 |
| A11 | novita/bigmodel/z.ai non-v1布局 | 明确拒绝直连，不修改保存地址 |
| A12 | 完整接口、userinfo/query/fragment等URL | 阻止错误root写入 |
| A13 | context=1M、<1M、unknown、fallback、用户覆盖 | 1M按有效来源输出，fallback不全开，无[1m] |
| A14 | 无label/有label/1M与重复选项 | 字符串/对象外形正确；模型ID去重，顺序稳定 |
| A15 | 四文件缺失、正常对象、含MCP/外部profile | 创建/字段合并正确；外部内容保留 |
| A16 | 非对象/损坏JSON/非法meta结构 | 文件定位错误；零外部写入 |
| A17 | 自己UUID与CC Switch共存，反复应用 | own entry不重复，外部entry/profile不删除 |
| A18 | 无journal但存在同UUID文件/遗留释放记录 | 提示接管并备份，不凭名称认领删除 |
| A19 | enterpriseConfig与推理/部署冲突 | 保留并给出原因，不清外部策略 |
| A20 | 预览后改key/地址/选中协议/context/文件；异步旧预览迟到 | ticket失效，旧提交零写入；旧响应不能覆盖新表单 |
| A21 | 仅更新余额/价格/路由池enabled | 不因无关变更使profile变化；直连不受enabled门槛影响 |
| A22 | 四个写点/backup/state/pending清理各步故障 | 完整回滚或recoveryRequired，无假成功 |
| A23 | 写入后、journal进度前中断；state写后pending删前中断 | 重启能判实际文件态，幂等恢复/提交确认 |
| A24 | rollback自身失败、第二次恢复 | 保留证据，可重试，不丢pending |
| A25 | managed部署/pointer/profile被外部改写 | conflict，默认不覆盖 |
| A26 | 外部仅新增MCP键/meta条目 | 恢复own字段时保留新增内容 |
| A27 | 原文件缺失，应用后用户加外部键 | 恢复后保留文件/键，不删目录 |
| A28 | prior appliedId的外部profile被删除 | 冲突可解释，不随机挑第一条 |
| A29 | restorePrevious vs switchOfficial vs release/rebaseline | 语义分别验证，备份/状态方向正确 |
| A30 | Provider改key/删除后 | stale/orphaned；仍可离线恢复；不偷偷重写 |
| A31 | 项目关闭/路由关闭 | Desktop直连四文件保持原样 |
| A32 | 两个合作writer/应用实例，以及写前已发生外部变更 | 合作writer互斥，旧revision的另一笔changed；外部变更可见时拒绝写入；不宣称检查与替换间的外部写入有跨工具CAS保证 |
| A33 | Windows多候选/macOS/Linux绝对XDG/相对XDG | 候选不误选；回落规则明确 |
| A34 | AGENT_HARBOR_CLIENT_HOME隔离、路径身份改变、两根/四文件碰撞 | 不扫描/写真实目录；变化使revision无效；碰撞零写入 |
| A35 | Bearer检查200合法/200HTML/401/403/404/429/超时 | 正确分类；不沿用x-api-key成功；不发Messages |
| A36 | 凭据含Bearer前缀/无效header字符 | 正规token写入，无双Bearer；无效值阻止 |
| A37 | 所有DTO、预览、错误、日志含哨兵secret | 所有前端/日志均不含；内部受保护记录可恢复 |
| A38 | 两根配置路径/长ID、缩放和键盘 | 预览与操作可达，无横向溢出/遮挡 |
| A39 | 独立Desktop卡片/disabled Provider/无兼容模型 | 用户流程和原因断言具体；无key输入/旧角色表单 |
| A40 | 现有Codex/ClaudeCode/OpenCode预览、注入、恢复 | 旧fixture输出和路由启停语义保持通过 |

故障与回归测试针对多文件恢复、凭据和旧客户端边界；无需为普通样式或实现内部每个 helper 写重复测试。使用 temporary roots、哨兵凭据和 mock HTTP 服务，测试必须能证明结果，不只匹配函数名或已有通用文本。

### 9.2 项目现有验证命令

从项目根运行，采用现有脚本的 Windows MSVC toolchain 处理：

```powershell
pnpm test:rust -- claude_desktop
pnpm test:rust
pnpm build
pnpm test:frontend
git diff --check
```

`test:rust` 当前执行 Rust library 和 `local_workflows` integration tests；新增场景接入这套入口。前端回归依赖 build 产物；新增 `test-claude-desktop-config.mjs` 接入现有 suite，使用后端调用记录断言 Provider 全局状态无变化。

文档编写阶段只检查文件、链接/源码对应和 diff，不把未运行的应用测试标记为已通过。

### 9.3 真实 Claude Desktop 必测

实际操作前记录四文件的安全备份和当前 mode/appliedId，完全退出 Desktop。使用明确选定的可用测试中转，敏感内容留在受保护的本地备份，验收报告只记录脱敏结果。

| 编号 | 真实场景 | 通过标准 |
| --- | --- | --- |
| L01 | 最小profile候选及省略Cowork键 | 该版本可启动、菜单可见、真实普通请求成功；否则P0候选阻断 |
| L02 | 已保存兼容中转复用 | 不重填key；选中模型原样出站；供应商侧日志/响应证明请求到达该中转 |
| L03 | 两个显式模型、展示名切换 | 菜单与实际模型对应，无fail-all静默退回 |
| L04 | 流式回复、两轮上下文 | 能正常结束、上下文连续、无协议错误 |
| L05 | 普通工具使用（测试中转支持时） | 请求/结果及下一轮可继续；不以普通文字回答代替工具验收 |
| L06 | 存在CC Switch或其他3P profile/MCP | 应用只切own profile；restorePrevious还原原选择且保留外部内容 |
| L07 | switchOfficial | 不删企业策略；可进入该环境允许的官方流程，存在策略阻断准确报告 |
| L08 | 完全退出AgentHarbor后使用Desktop | 直连仍工作，不依赖127.0.0.1项目代理 |
| L09 | 中转只接受x-api-key或只暴露非v1端点 | 显示鉴权/布局限制，不把它标为Desktop已验证可用 |
| L10 | 密钥变更、重新应用、恢复 | stale提示准确；重新应用换新key；恢复无密钥残留于不该保留的own文件 |

一次真实回复可能计费，留在实际实施/验收阶段，不能在后台自动发请求。验收报告包括 Desktop版本、平台/安装渠道、项目提交、供应商协议、最终URL形态、模型ID、网络出口、案例结果与恢复结果；不得只有配置文件截图或模型菜单截图。

发布首期的门槛：A01–A40 对应实现项通过；L01–L04、L06、L08、L10 在主支持平台通过；L05 按普通工具支持范围实测后声明，L07/L09验证限制与错误路径。未真实验收的平台/功能必须标明，不以mock结果推广。

## 10. 路由模式：CC Switch 的实现及本项目后续路线

2026-10-05 补充：完整源码核对、生命周期限制、数据/鉴权/lease/模式切换契约、R1–R4 批次与验收矩阵见 [Claude Desktop 路由详细实施计划](claude-desktop-routing-integration-execution-plan.md)。该文档完成的是设计，路由模式尚未实现。

### 10.1 已核实的 CC Switch 链路

```text
Claude Desktop安全角色ID
  -> /claude-desktop/v1/models 或 /claude-desktop/v1/messages
  -> 独立持久设备gateway token校验
  -> Desktop当前provider及routes
  -> 角色别名映射为真实模型（协议转换之前）
  -> 共享ClaudeAdapter / Messages转发、重试与协议转换
  -> 上游凭据 / 实际模型请求
  -> 用量记录分别保存request_model、outbound_model、上游响应声明模型
```

源码要点：

- `proxy/server.rs` 注册两个专属端点，不为未知端点任意兜底。
- `proxy/handlers.rs` 在 `/models`、`/messages` 都校验设备网关 Bearer token；格式 `ccs-<UUID>`，供应商真实密钥不写 Desktop 的网关 profile。
- `claude-desktop` 供应商 namespace 独立，转发器共享 Claude 适配和普通 Messages 管线，不依赖 Claude Code takeover。
- `proxy/forwarder.rs` 在转换前执行 `map_proxy_request_model()`，并在全部请求改写后抓真实出站模型。
- `/models` 返回合法角色ID、`type:model`、`created_at`、可选 `supports1m`，外层 `has_more/first_id/last_id`；真实上游ID保留在routes。
- 原生 Messages 响应 model 仍透传真实上游 ID；没有证据表明必须改回角色别名。

映射顺序为：剥离请求末尾 `[1m]` → 精确匹配 → 当前/旧 Opus 别名兼容 → 旧原始 route ID → 合法 Claude 名角色回落 → 无Fable配置时回落Opus → 明确未知模型错误。前端空档回填属于 CC Switch 产品选择，不能默认继承到本项目的首期直连。

`proxy_gateway_base_url_from_db()` 读取 DB listen_port 并拒绝 0；代理服务启动会把实际绑定端口保存到 DB。profile 可以消费启动后保存的端口，但没有本项目所需的监听 generation/lease 校验，不能将 0 直接写进 profile。

### 10.2 本项目路由设计草案

保留 Provider 作为唯一供应商/凭据源，Desktop 独立记录角色路由：

```text
DesktopRoute {
  roleAlias,             # Desktop可接受的安全ID
  providerId, modelId,    # 已保存卡片与真实上游模型
  labelOverride,
  oneMMode
}
```

入口新增 `/claude-desktop/v1/models` 与 `/claude-desktop/v1/messages`。`roleAlias -> {providerId, modelId}` 后进入现有 `provider_destination`、鉴权替换、请求改写、协议转换、route_trace、request_diagnostics、价格快照与账本。不暴露现有 `providerId::modelId` slug，不改全局 active_provider_id。

后续实施要求：

1. **菜单与请求共用 resolved routes**：provider缺失、模型移除、协议不支持、路由池禁用等必须明确处理。直连允许enabled=false；未来首版网关按现有路由资格要求enabled=true，并在UI明确启用影响，不悄悄启用。
2. **鉴权边界**：优先 Desktop 专属持久随机 gateway token/scope，内部受保护保存；在路径分派后只授权Desktop的models/messages，不把全局 `authorized()` 改成“routerToken 或 desktopToken”进而开放 `/v1`、`/p/*`、Codex入口。两个端点一致校验，禁止转发该 token 到上游。token创建放在配置管理操作中；请求校验只读取，不惰性生成/轮换，缺失时要求重新应用。Desktop profile仅写本地token；轮换后必须重新应用profile。
3. **实际监听端口**：消费 `ensure_running()` 当前返回的实际绑定端口（`u16`），并新增/取得listener generation或lease；不是已有endpoint/lease返回契约。绑定成功后再预览/应用，commit校验该lease仍有效，不能写`:0`。若支持动态端口，调整 `routing_runtime::ensure_available()` 目前丢弃返回端口的契约；固定端口仍需绑定成功和lease校验。
4. **服务器生命周期**：Desktop网关配置纳入needs_server。关闭Codex路由或其他客户端不能误停Desktop；启动端口冲突、正常退出纳入已有恢复策略。
5. **Desktop缓存配置**：退出网关时恢复磁盘文件不等于正在运行的Desktop立即切换。明确要求重启；release管理不能遗留无提示的失效本地地址。需测试正常退出与崩溃、端口被其他进程占用。
6. **请求身份**：客户端roleAlias、映射后真实model、attempt出站model、上游自报model分别记录；价格按真实provider/model绑定，不能用菜单别名借官方价格。
7. **响应处理**：先保留真实响应供诊断/账本，默认沿既有转换输出；若实测Desktop要求响应别名，再在客户端视图层改，不能污染费用身份。
8. **协议范围**：首轮验证Messages→Messages / ChatCompletions / Responses及SSE、工具、多轮；现有翻译器存在不等于Desktop全部功能通过。
9. **隔离策略**：Desktop映射、会话亲和、failover范围独立于Codex/ClaudeCode；缺失映射明确报错。先精确映射，角色/旧版本回落只按实测需要添加，不能复制全部历史特例。
10. **端点完整性**：先提供已确认需要的models/messages；真实调用count_tokens后再定义契约。原生Messages可透传，跨协议无法精确支持时明确501，不伪造精确token计数。

Gemini经OpenAI兼容中转可走Chat路径，与Gemini Native适配不同。CC Switch已有托管Copilot/Codex/xAI OAuth与MiMo专项修补不属于本项目现成能力，不提前写“支持”。

后续路由分为R1 resolved routes/token/listener lease、R2入口与真实身份贯穿、R3生命周期/恢复、R4工具流式与价格验收四批，详细计划已补齐；不在首期先修改全局Store或开放半成品路由按钮。CC Switch 停服务/退出不会撤回 Desktop 网关 profile，且其默认单路并不等于 Desktop 专属多供应商 failover；本项目必须按新计划补齐这些契约。

## 11. 交付物与审查清单

实施最终应提供：

- 独立 Desktop 后端与前端模块，复用已有卡片，不要求重新填写凭据。
- 用户文档：兼容条件、直连/网关区别、重启要求、恢复原配置/官方/释放语义、网络出口与快照限制。
- 首期实际支持的Desktop版本/平台表与脱敏验收记录。
- 自动测试/真实测试结果，明确未覆盖范围；多文件故障和外部工具共存必须有证据。
- 本计划更新实施状态，记录实际提交与仍待验证项。

审查重点依次为：错误密钥来源/前端泄露 → 四文件部分失败恢复 → 外部profile与企业配置保护 → URL/协议/白名单 → Provider变更和全局路由隔离 → DTO状态与可用性声明 → UI可操作性。

验收不能只看“文件写入成功”。核心交付是：已有中转能被正确引用，真实Desktop确实使用选定中转，用户能安全撤销，而且已有Codex/ClaudeCode/OpenCode结构和行为没有被牵动。

## 12. 核实过的源码索引

行号按本文基线核对，仅作为定位；实施后以符号名为准。

| 文件/符号 | 用途 |
| --- | --- |
| [router.rs](/D:/workspace/AiMaMi/src-tauri/src/core/router.rs:75) `Provider`、`:171 ProviderView`、`:232 has_route_credentials`、`:240 protocol_for_model`、`:271 model_credential`、`:297 supports_model` | 供应商/秘密边界与逐模型资格 |
| [newapi.rs](/D:/workspace/AiMaMi/src-tauri/src/core/newapi.rs:25) `base_url`、`:165 provider_endpoint`、`:198 protocol_base_url`、`:583 anthropic_models` | 根URL、non-v1例外、现有x-api-key列表检测 |
| [provider_probe.rs](/D:/workspace/AiMaMi/src-tauri/src/core/provider_probe.rs:62) | 现有Anthropic测试鉴权边界 |
| [client_config/mod.rs](/D:/workspace/AiMaMi/src-tauri/src/core/client_config/mod.rs:89) `ClientAdapter`、`:159 ConfigPreview`、`:207 Session` | 单文件事务不适合直接承载Desktop四文件 |
| [targets.rs](/D:/workspace/AiMaMi/src-tauri/src/core/targets.rs:8) | 现有目标能力边界 |
| [storage.rs](/D:/workspace/AiMaMi/src-tauri/src/core/storage.rs:34)、[secret_file.rs](/D:/workspace/AiMaMi/src-tauri/src/core/secret_file.rs) | 单文件持久替换、DPAPI/Unix0600 |
| [routing_runtime.rs](/D:/workspace/AiMaMi/src-tauri/src/core/routing_runtime.rs:8) | mutation guard、server reconcile、退出恢复 |
| [newapi-page.tsx](/D:/workspace/AiMaMi/src/components/newapi/newapi-page.tsx:84)、[client-configs.tsx](/D:/workspace/AiMaMi/src/components/newapi/client-configs.tsx)、[provider-models.ts](/D:/workspace/AiMaMi/src/lib/provider-models.ts) | UI接入、候选预筛选 |
| [CC Switch调研文档](/D:/workspace/temp/cc-switch/docs/claude-desktop-3p-injection-zh.md) | 参考说明，本文第2节/10节纠正过度推广处 |
| [CC Switch claude_desktop_config.rs](/D:/workspace/temp/cc-switch/src-tauri/src/claude_desktop_config.rs:236) `is_claude_safe_model_id`、`:291 direct_gateway_credentials`、`:650 model_list_response`、`:686 map_proxy_request_model`、`:920 proxy_gateway_base_url_from_db`、`:936 apply_provider_to_paths`、`:1032 build_gateway_profile`、`:1148 write_meta` | 参考外部文件、鉴权、模型菜单/映射与恢复行为 |
| [CC Switch server.rs](/D:/workspace/temp/cc-switch/src-tauri/src/proxy/server.rs:300)、[handlers.rs](/D:/workspace/temp/cc-switch/src-tauri/src/proxy/handlers.rs:134)、[forwarder.rs](/D:/workspace/temp/cc-switch/src-tauri/src/proxy/forwarder.rs:1247) | Desktop专属端点、网关token、转换前映射与出站身份 |
