# 多客户端配置

中转页的「客户端配置」用于 Claude Code 和 OpenCode。Codex 继续使用上方的路由注入入口。

1. 创建供应商，设置 API 密钥、协议和默认模型，然后启用供应商。
2. 在客户端卡片中选择「配置」，选择已有中转卡片。地址、凭据和可用模型自动带入；新配置默认原生直连。
3. 预览待写入的字段，确认后应用。预览不返回本地路由令牌或供应商密钥；首次注入会列出将被覆盖的现有字段（网关地址、凭据、模型菜单等），管理级 `managed-settings.json` 与本次注入冲突时也会提示。
4. 重新打开客户端会话。原生直连无需保持 AgentHarbor 运行；本地路由模式需要保持运行，AgentHarbor 正常退出时会撤回该模式的客户端配置。
5. 使用「恢复原配置」撤回配置；原有官方供应商、权限和其他设置会保留。

| 客户端 | 支持协议 | 写入内容 |
| --- | --- | --- |
| Claude Code 原生直连 | Anthropic Messages | 将中转地址及凭据写入 `settings.json`；按角色自动匹配 Sonnet、Opus、Fable、Haiku（分段匹配、取数值最新版本），缺少对应模型时使用所选默认模型 |
| Claude Code 本地路由 | Anthropic Messages、Responses、Chat Completions | 写入所选供应商的本地入口；非 Messages 上游由路由转换请求及响应 |
| OpenCode | Responses、Chat Completions、Anthropic Messages | 可多选中转卡片，加入所选卡片的兼容模型，并设置一个默认模型；原生直连和本地路由均可选 |

Claude Code 的角色映射与子代理模型可在模型映射表中调整，认证字段位于高级设置。认证只写入 `ANTHROPIC_AUTH_TOKEN` 或 `ANTHROPIC_API_KEY` 之一，另一字段会删除并纳入恢复记录，避免旧凭据覆盖新配置。
角色自动匹配按分段边界识别角色并取数值上最新的版本（`claude-opus-4-10` 优于 `claude-opus-4`），启动默认模型命中角色时优先，结果与供应商模型存储顺序无关。子代理映射留空时不写入 `CLAUDE_CODE_SUBAGENT_MODEL`——强制所有子代理使用主模型会覆盖它们按 tier（如 Haiku）请求的模型；显式映射照常写入。`CLAUDE_CODE_USE_BEDROCK/VERTEX/FOUNDRY` 既不创建也不改写：未设置或为假值时接入本就由 `ANTHROPIC_BASE_URL` 决定，已是真值则视为用户自己的云接入配置，注入会明确拒绝而不是静默关闭。遗留的 `ANTHROPIC_SMALL_FAST_MODEL`（旧版后台小任务变量）存在时会被改写为 Haiku 角色同值，不创建新键。
Claude Code 选择中转卡片后默认勾选其中的全部兼容模型，也可只保留需要的模型。同一系列的不同版本分别保留，例如两个 Opus 版本与一个 Fable 版本可以同时配置。启动默认模型只决定新会话的初始选择；Sonnet、Opus、Fable、Haiku 和子代理的角色映射直接展示，可分别调整。取消选择默认模型时会改用仍选中的模型，指向被移除模型的角色映射会恢复为自动匹配。[1m] 标记由解析出的上下文窗口决定：已知不足 1M 的模型不标记（对话框中对应勾选禁用并提示），未知上下文以勾选为准。

多个模型通过用户配置中的 `modelPicker.options` 写入 `/model` 菜单，需要 Claude Code **2.1.242 或更新版本**。AgentHarbor 会检测本机 `claude` 版本（每进程一次），版本过低或无法检测时提示，注入本身不受影响。每次应用会把写入的模型条目记入所有权记录，重新应用时精准撤回这些条目后按当前选择重新生成；用户手写的条目及其显示名称、描述和能力配置永不改动或删除（唯一例外：文件既无会话记录也无所有权记录时的首次接管，会一次性清理旧版本遗留的纯 `model+label` 行），应用之间新增的条目会被保留而不是报冲突。已有 `availableModels` 限制同样按元素补入，并设置 `replaceBuiltInOptions` 使具体版本分别显示。恢复操作还原原始菜单设置并清除所有权记录。

OpenCode 分别使用 `@ai-sdk/openai`、`@ai-sdk/openai-compatible` 和 `@ai-sdk/anthropic`。
模型上下文及输出长度优先使用供应商返回的元数据，未知值的默认配置可以在应用前修改。
默认模型以外的模型使用各自的元数据或模型默认值，不会统一继承默认模型的上下文长度。
中转地址、凭据、模型列表发生变化后，原生模式需要重新应用配置。路由模式的凭据及网络出口实时读取中转配置；客户端中的模型列表和角色映射仍需重新应用才能更新。

## Claude Code 协议转换范围

本地路由沿用 `/p/{providerId}/v1/messages`，独立于 Codex 接入开关。支持文本、图片、普通函数工具调用及结果、流式参数增量、结束原因和上游实际返回的用量；中途断流、错误或不完整工具参数不会被标记为正常完成。

原生 Messages 同协议直通。转换模式不等同于原生 Anthropic 的全部能力：签名 thinking 不跨协议重放，原生服务器工具、文档块等无对应映射的内容明确报错；Responses 不支持的自定义停止序列也不会静默丢弃。`/messages/count_tokens` 等辅助端点只对原生 Messages 上游直通，不伪造 Token 计数。暂未实现 Gemini Native 协议。

## 路径与恢复

- Claude Code：`CLAUDE_CONFIG_DIR/settings.json`，默认 `~/.claude/settings.json`。
- OpenCode：优先 `OPENCODE_CONFIG`；否则使用 `$XDG_CONFIG_HOME/opencode`（默认 `~/.config/opencode`），选择已有的 `opencode.jsonc`、`opencode.json`、`config.json`。
- 隔离调试：设置 `AGENT_HARBOR_CLIENT_HOME` 后，这两个客户端都从指定目录构造配置路径，不读取正式客户端的目录覆盖变量。
- 备份和管理记录：AgentHarbor 数据目录下的 `client-config`（会话记录，以及记录每轮写入条目的所有权文件 `*.owned.dat`）。Windows 使用现有 DPAPI 加密存储，Unix 文件只允许当前用户读取。

配置采用字段级合并和原子替换。预览版本会校验配置是否在预览后变更。恢复只撤回管理字段；没有其他改动时恢复原始文件全文（含注释），原先不存在的文件会删除。带注释或尾逗号的文件（Claude 与 OpenCode 均适用）解析后应用会规范化为 JSON，原始带注释文本保存在备份中。

如果外部工具修改了管理字段，会报告冲突并指出范围（网关地址、凭据、模型或模型菜单），避免覆盖。此时有两个出口：「以当前文件为准重新接管」把现有值记录为新基线后按上次配置重新写入；「释放管理」停止管理且不改写文件，所有权记录会保留，之后的再次应用仍能精准撤回这些条目。没有冲突时也可一键「重新应用」按上次配置重写。项目级配置、环境配置及启动参数仍可能覆盖全局配置；页面显示的是写入状态，不代表已完成模型连通性测试。

正常退出时，Claude Code 和 OpenCode 的本地路由配置会逐个尝试恢复，防止它们继续指向已经停止的本地代理；原生直连配置像普通客户端设置一样保留。若 cc-switch 等工具已经修改了受管理字段，退出清理会保留外部改动并记录冲突，不会强行覆盖。强制结束进程无法执行退出清理，重新打开 AgentHarbor 会重新提供仍在配置中的本地路由。

OpenCode 的管理范围精确到供应商名称、SDK、端点、凭据及模型名称/长度字段。应用后新增或修改的 headers、模型 options/variants、手动模型会在重新应用、取消选择卡片和恢复时保留；修改受管理的端点或凭据仍会触发冲突。恢复只清理本次创建且仍为空的对象，原有空对象会保留。旧版整块 provider 管理记录仍可读取，成功重新应用后改用字段级记录。

预览只包含受管理字段，不复制无关自定义 headers；凭据脱敏兼容大小写、下划线、连字符及常见 token、secret、password、Authorization、Cookie 字段。

## 与 cc-switch 的取舍

对照本地 cc-switch 源码后，复用其保留用户配置、撤回注入字段、串行切换的思路。AgentHarbor 已使用字段恢复记录保留 hooks、权限及其他独立改动，因此无需再引入供应商配置 backfill 数据库和 common-config 模板。

| 机制 | AgentHarbor 的处理 |
| --- | --- |
| Claude 单供应商、OpenCode 多供应商 | 两个适配器分别生成配置；OpenCode 按所选卡片共存，其他供应商保留 |
| 切换前剥离旧注入 | 根据 before/after 记录撤回旧字段，再生成新配置；用户修改管理字段时报告冲突 |
| 代理认证占位符 | 使用随机本地路由凭据，保留本地访问校验；上游真实凭据由转发层读取，不能直接换成固定 `PROXY_MANAGED` |
| 模型别名 | 保留真实模型 ID 和现有角色映射；当前路由没有稳定角色别名解析，不引入硬编码别名 |
| 请求级注入 | 已支持协议转换及 Chat Completions 流式 usage；自动缓存断点、会话缓存键、平台 thinking 方言应按上游能力单独实现和验证 |

原分析中两处与本地源码不符：`opencode_config.rs` 的 `OPENCODE_DB` / `XDG_DATA_HOME` 用于数据库/数据目录，并非配置文件路径；`services/proxy.rs` 的托管 Copilot 默认使用 `AUTH_TOKEN`，仅显式选择 API Key 字段时改用 `API_KEY`。另外，cc-switch 普通第三方的保留策略可能替换多个已有认证键，单键约束主要用于其托管账号分支。cc-switch 不为 OpenCode 提供代理接管是其产品实现范围，AgentHarbor 已支持 OpenCode 本地路由，无需移除此能力。

## 模块边界

- `core/client_config/claude.rs`、`opencode.rs`：客户端专属字段生成。
- `core/client_config/owned.rs`、`takeover.rs`、`version.rs`：`modelPicker.options`/`availableModels` 的所有权记录、首次接管与管理级配置警告、Claude Code 版本探测。
- `core/client_config/mod.rs`：路径发现、协议验证、预览、备份、合并和恢复。只接收应用数据目录、客户端路径和供应商数据，不依赖 Codex 账号或进程。
- `commands/client_config.rs` / `api.targets`：统一配置接口。
- `core/routing_runtime.rs`：共享路由生命周期。Codex、Claude Code、OpenCode 的路由模式各自独立保持服务；只有原生客户端配置时不会启动或占用本地路由。
- `commands/usage.rs` / `api.usage`：客户端无关的中转用量接口。

原生模式把供应商凭据写入对应客户端本机配置；路由模式只写入本地路由凭据。预览均脱敏。路由模式复用现有用量观察器，Chat Completions 流式请求会请求 `include_usage`；上游没有返回用量时仍标记为未报告，不估算 Token 数。原生直连的请求不进入 AgentHarbor 中转统计，已有客户端本地日志统计独立工作。

旧版已安装配置在读取时保留原有路由模式，不会自动改成直连。重新打开配置弹窗会还原此前保存的卡片、接入模式和高级选项。

本次扩展的是模型供应商配置能力。账号登录、会话归档、MCP 和 Skills 当前仍按现有 Codex 能力提供，不会把其他客户端请求错误地转到 Codex。

## 验证

`pnpm build`；`pnpm test:rust`；`pnpm test:frontend`（需要本地 Playwright）。

测试使用临时客户端目录和本地模拟上游，覆盖配置保留、恢复、并发变更检测、协议匹配、从生成配置发送请求、SSE 用量记录，以及独立启停和重启后的路由恢复。不修改正式客户端配置，不调用付费上游。

参考：[Claude Code 网关](https://code.claude.com/docs/en/llm-gateway-connect)、[模型配置](https://code.claude.com/docs/en/model-config)、[OpenCode 供应商](https://opencode.ai/docs/providers/)、[配置](https://opencode.ai/docs/config/)。

## 2026-10-03 恢复与备份说明

恢复原配置后保留空的元素所有权记录，用于标记旧版 picker 迁移已经完成。之后再次接管不会重新按 `label == model` 的形状清理用户行。`/model` 切换到接管时已经存在的用户 picker 行，同样视为正常客户端操作。

冲突替换前的客户端文件保存在客户端配置管理器数据目录的 `backups/*.dat` 中，由 `secret_file` 加密存储，最多保留 20 份。它不是可以直接复制回客户端的明文 JSON；当前没有备份列表恢复界面。所有权记录属于辅助持久化数据，写入失败会记录警告，当前没有磁盘故障下的恢复验收；不要手工删除记录后期待迁移标记仍然存在。
