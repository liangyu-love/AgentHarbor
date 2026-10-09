# Pi Agent 接入调研

日期：2026-10-09。AgentHarbor 基线：`f5738b2`。本轮完成调研和隔离配置验证，未实现 Pi 注入，也未改动用户的 Pi 配置。

## 结论

Pi 可以接入现有中转模型和聚合网关。建议新增原生 `models.json` 配置适配器，复用模型选择、配置预览/备份/恢复、协议转换及请求落账，不另建协议转换服务。

它的提供商配置是累加的：多张中转卡片可以同时出现在 `/model`，聚合则作为独立提供商展示。启用配置和选择会话模型是两件事，不应把默认模型当成实际使用的渠道。cc-switch 的 Pi 模块采用这个方式；它没有为 Pi 实现路由或故障切换。

## 1. 核查范围与证据

- 本地 cc-switch：`D:/workspace/temp/cc-switch`，提交 `1ee2fdc`，文档对应 v3.20.4。
- 本机 Pi：`@earendil-works/pi-coding-agent`，安装版本 **0.84.2**。只读取安装包文档/源码和运行隔离目录的模型列表命令。
- 当前官方文档通过 Context7 核对，并与安装包交叉比较。上游 main 会变动，实施时仍需对当时安装版本进行验收。
- 临时目录：`D:/workspace/AiMaMi/.runtime-test/pi-agent-research`。使用假密钥及虚构模型，不发推理请求、不查询余额、不接触真实 `auth.json`。

隔离验证中，三个提供商均被 `--list-models agentharbor` 正确加载：Chat、Responses、Messages；配置的 272,000 上下文、8,192 输出分别显示为 272K、8.2K。日志见临时目录 `catalog-smoke.log`。这证明配置能被解析，不代表工具续轮、流式或真实渠道已经通过验收。

## 2. 原生契约

用户目录默认为 `~/.pi/agent`，可由 `PI_CODING_AGENT_DIR` 改变。模型接入写 `models.json`；默认选择在 `settings.json`；登录凭据在 `auth.json`。全局与项目配置分别有作用范围，不能把全局默认项视为活动会话状态。[官方配置文档](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/configuration.md)

`providers.<providerId>` 可以定义地址、协议、认证和模型数组。打开 `/model` 会重新读取目录；Pi 支持同 ID 模型覆盖。原生认证具有自己的优先级，已有登录凭据可能优先于配置中的密钥；因此 AgentHarbor 应使用自己的提供商 ID。表达式密钥应保留，不能执行导入文件中的命令。[官方模型文档](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/models.md)

与现有协议的对应关系如下。地址后缀同时核对了本机 Pi 使用的 SDK 实现。

| AgentHarbor 协议 | Pi `api` | 当前聚合接入的 `baseUrl` | 实际端点 |
|---|---|---|---|
| Chat Completions | `openai-completions` | `http://127.0.0.1:<port>/aggregate/v1` | `/aggregate/v1/chat/completions` |
| Responses | `openai-responses` | `http://127.0.0.1:<port>/aggregate/v1` | `/aggregate/v1/responses` |
| Anthropic Messages | `anthropic-messages` | `http://127.0.0.1:<port>/aggregate` | `/aggregate/v1/messages` |

协议是客户端入口协议，与实际成员的出口协议分开。模型数组可逐项指定协议和地址；不同协议不能只改 `api` 而沿用错误的根地址。本机安装包 `docs/models.md` 的 Supported APIs、Model Configuration 与 SDK 的 `api/anthropic-messages.js` 是此表的直接核查依据。

## 3. cc-switch 值得借鉴的实现

| 实现位置（相对于参考仓库） | 可复用的行为 |
|---|---|
| `src-tauri/src/pi_config/mod.rs` | 路径解析；有界读取；JSON5；进程内串行写；原子替换；revision 检查 |
| `src-tauri/src/services/provider/pi.rs` | 按提供商节点新增/更新/移除；数据库写失败时回滚；禁用保留档案 |
| `docs/pi-native-contract-zh.md` | 显式节点决定启用状态；保留未知字段；不复制运行时内置目录 |
| `src-tauri/src/services/pi_state.rs` | 原生状态投影，不用其他客户端的“当前提供商”逻辑猜测 |
| `src-tauri/src/session_manager/providers/pi.rs` | 独立解析 Pi 会话；删除前核对身份与根目录 |

cc-switch 刷新列表时同步外部修改；保存只操作目标节点，保留其他提供商和顶层字段。其 revision 比较能检测读改写期间的外部变化，但检查与文件替换之间仍不是跨进程的原子 CAS。AgentHarbor 应沿用现有事务、预览失效、恢复冲突机制，并明确并发写入边界。

参考文档明确不管理 Pi 登录和默认模型、不实现 Pi 网关、故障切换。因此不应直接照搬其“单卡配置就是整个接入”的数据模型，也不应声称它已经完成我们的聚合接入。

## 4. 适合 AgentHarbor 的结构

### 4.1 模型选择和协议

新增 `TargetId::Pi` 与 `client_config/pi.rs` 适配器。选择仍保存 `ModelTargetRef::Provider` / `Aggregate`，不复制上游地址或密钥到新业务数据库。

- **中转直连**：每张卡片一个稳定的 `agentharbor-relay-<id>` 节点；真实模型 ID 原样保留。每模型协议不同则分别生成正确的地址和 `api`。
- **路由中转**：消费现有单卡路由，使用后端解析出来的客户端模型 ID；不能把仅供 Codex 使用的模型身份随意套到另一入口。
- **聚合网关**：独立 `agentharbor-aggregate` 节点，输出聚合的公开模型 ID，使用本地网关令牌，隐藏实际成员密钥。

聚合先选一个已验收的入口协议，其他两种作为可选入口。Pi 自身发送标准工具调用，AgentHarbor 继续负责转换和渠道调度。任何入口都不会凭空增加上游搜索、视觉或推理能力。

现有通用聚合导出已经提供连接地址、模型 ID 和限制，可作为手工接入基础；完整 Pi 导出仍需新增 consumer profile。自动注入必须经过配置管理器，不能把手工 JSON 导出当成带恢复能力的注入。

### 4.2 能力、上下文与费用

统一消费有效模型投影：手动模型配置、目录匹配和已保存的限制继续遵守现有优先级。聚合使用覆盖全部可选成员的有效限制，不能取某个成员的最大上下文冒充整体能力。

Pi 的模型配置包含 `contextWindow`、`maxTokens`、`reasoning`、`input`、`cost`。官方实现会给缺省限制和费用补默认值；未知不能因此被解释为已确认能力或免费。[官方提供商合成实现](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/src/core/provider-composer.ts)

当前 `AgentConnectionExport` 尚未完整输出推理、视觉、思考档位与费用。应在共享投影中补足必要字段，再由 Pi 适配器消费。上下文无法可靠确定时沿用现有明确配置流程；不要另写一份模型族名猜测表。

Pi 显示的费用只是客户端估算。聚合实际费用仍按命中的成员、价格快照和缓存用量入账；成员价格不同不能靠一组固定 `cost` 得到准确账单。直连可输出有效自定义价格/倍率后的价格，未知值不回写成零价到 AgentHarbor 账本。

### 4.3 配置所有权与恢复

1. 确定绝对 agent 目录，只读取目标文件。
2. 预览管理节点、原值、有效模型和连接条件，密钥脱敏。
3. 提交前复核 revision，备份并原子更新管理节点。
4. 不修改 `auth.json`、用户其他提供商、项目默认项或活动会话。
5. 禁用时仅撤回本工具节点；恢复只还原接管前受管路径，外部冲突保留给用户处理。
6. 网关生命周期纳入现有 `routing_runtime`：有 Pi 消费者时保留监听；停用不能留下仍指向已关闭网关的托管配置。

清除接入相当于恢复 Pi 原生提供商选择，并不等于退出官方账号。`/login` 和 OAuth 刷新继续交给 Pi；无需复制 Claude Code 的登录恢复流程。

写入本工具生成的字面密钥或 Header 时，要转义 Pi 表达式中的 `$` 和前导 `!`，避免把真实凭据当成环境插值或命令。导入已有表达式则保持原样，配置管理器不能执行它们。

### 4.4 请求诊断和粘性

现有 `/clients/` 来源标签只识别 Codex、Claude Code、OpenCode。手工使用通用聚合入口目前会显示通用来源；正式实现需增加 Pi 来源标签、筛选文案和兼容旧诊断数据的测试。

Pi 的兼容选项可发送会话粘性头，本机文档已核查对应字段。必须验证它与现有会话提取规则一致，不能用固定 `headers` 把所有会话绑成同一个。渠道保持和连续故障阈值由聚合运行时处理，Pi 不另做每请求轮转。

## 5. 后续实施 TODO（尚未执行）

- [ ] P0：增加 Pi 路径、目标和来源标签；不破坏现有目标枚举及配置兼容。
- [ ] P1：原生多提供商适配器；未知字段保留、节点级备份/撤回、陈旧预览与外部冲突处理。
- [ ] P2：中转直连和聚合网关消费；补共享能力输出与 Pi export，支持三种入口的正确地址。
- [ ] P3：一键应用已保存选择、连接状态、打开文件、恢复配置；实际会话模型不冒充“默认模型”。
- [ ] P4：隔离 Pi CLI + 本地 mock 验收流式、普通工具多轮、推理/图片、取消、错误、费用和渠道粘性。
- [ ] P5：用户需要时再扩展 Pi 指令、Skills、MCP 与会话。它们各有独立原生格式和版本约束，不属于模型注入的前置条件。

最低验收还应包括：同名模型不同中转同时可见；登录凭据不覆盖托管提供商；开启/关闭不会改动第三方节点；重复应用幂等；坏文件不被覆盖；默认选项与会话切换相互独立；路由停止前配置正确撤回；所有三种入口都能完成工具续轮。

## 6. 本轮未验证的边界

- 真实模型请求、缓存命中、搜索和付费费用未测试。
- Pi 0.84.2 的 CLI 验证仅覆盖配置加载，不覆盖最新版所有兼容选项。
- 自动接入 UI、Pi 来源诊断及生命周期协调尚未实现。
- 跨协议续轮中的私有推理/资源归属仍遵循既有路由保护；不能为了“所有协议都能选”移除这些保护。
