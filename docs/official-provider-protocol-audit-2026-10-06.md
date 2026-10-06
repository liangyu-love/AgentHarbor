# 官方提供商协议核查与修复

核对日期：2026-10-06。范围为 `src/lib/provider-presets.ts` 的全部预设，以及共享路由、连接测试、Claude Desktop / Claude Code / OpenCode / Codex 原生配置的端点消费。先使用 Context7 检索官方文档，索引不足或旧版本信息再回到官方当前页面；只访问公开文档和无鉴权模型列表，未使用真实密钥调用推理、搜索或收费接口。

## 是否固定协议

预设中的协议是默认值，模型行允许单独选择 Messages、Chat Completions、Responses。模型显式选择优先于卡片默认；未单独设置的 Claude 名称模型优先使用模型目录声明的协议，缺声明时使用 Messages，其他模型继承卡片默认。OpenCode Go 的种子模型有模型级选择，修改卡片默认不会替换它们，用户应修改模型行。目录刷新保留用户选择。

三种协议的转换器已实现；上游提供商是否接受该模型、请求格式或工具，由其自身契约决定。不能把“网关有三种接口”当作“每个模型都能使用三种接口”。

## 官方接口范围

“未证实”表示此次未找到该提供商官方支持证据，不代表通过猜测地址或 GET 404 判定其不支持。

| 预设 / 服务 | Chat | Responses | Messages | 核对结论与官方来源 |
| --- | --- | --- | --- | --- |
| DeepSeek | 有 | 有 | 有 | Chat/Responses 使用普通根，Messages 使用 `/anthropic`。[Messages](https://api-docs.deepseek.com/guides/anthropic_api/)、[Responses](https://api-docs.deepseek.com/guides/responses_api/)；Responses 内置 `web_search` 等工具仍被忽略，协议支持不等于搜索支持。 |
| OpenCode Go | 按模型 | 按模型 | 按模型 | 三种根均为 `/zen/go/v1`，按[官方模型表](https://opencode.ai/docs/go/)选择；公开 `/models` 缺少协议字段。 |
| OpenCode Zen | 按模型 | 按模型 | 按模型 | 三种根均为 `/zen/v1`，按[官方模型表](https://opencode.ai/docs/zen/)选择；Go/Zen 同名模型的协议可能不同，不能按族名统一。Google / System One 等额外原生协议不属于本项目三协议适配范围。 |
| OpenRouter | 有 | 有 | 有 | 共用 `/api/v1`，Messages 采用 Bearer。[API](https://openrouter.ai/docs/api_reference/overview)、[Responses](https://openrouter.ai/docs/api_reference/responses/overview)、[Messages](https://openrouter.ai/docs/api/api-reference/anthropic-messages/create-a-message)。 |
| SiliconFlow CN | 有 | 不提供原生 | 有 | 共用 `/v1`，Messages 使用 Bearer。[Messages](https://docs.siliconflow.cn/docs/api/messages-post)、[Codex 指南](https://docs.siliconflow.cn/docs/usercases/use-siliconcloud-in-codex)明确需要将 Responses 转换为 Chat。 |
| SiliconFlow Global | 有 | 未证实 | 有 | 共用 `/v1`，Messages 使用 Bearer。[官方 Messages](https://docs.siliconflow.com/en/api-reference/chat-completions/messages)；不根据旧域名迁移公告切换用户的 CN/Global 账号。 |
| Kimi Coding | 有 | 未证实 | 有 | 已证实两协议共用 `/coding/v1`；不能把 CLI 可接入其他厂商的 Responses 当作 Kimi 服务支持。[官方 API](https://www.kimi.com/code/docs/en/)。 |
| MiniMax CN / Global | 有 | 有 | 有 | Chat/Responses 使用 `/v1`，Messages 使用 `/anthropic/v1`；[CN Chat](https://platform.minimax.cn/docs/api-reference/text-chat-openai)、[CN Messages](https://platform.minimax.cn/docs/api-reference/text-chat-anthropic)、[CN Responses](https://platform.minimax.cn/docs/api-reference/responses-create)、[Global Messages](https://platform.minimax.io/docs/api-reference/text-chat-anthropic)、[Global Responses](https://platform.minimax.io/docs/api-reference/responses-create)。当前 CN 文档使用 `api.minimax.cn`；未确认旧 `api.minimaxi.com` 已失效，因此不迁移既有卡片的域名或凭据。 |
| GLM / Z.AI Coding Plan | 有 | 有 | 有 | 分别使用 `/api/coding/paas/v4`、`/api/v1`、`/api/anthropic/v1`；[GLM 配置表](https://docs.bigmodel.cn/cn/coding-plan/tool/others)、[Z.AI 配置表](https://docs.z.ai/devpack/tool/others)。 |
| GLM / Z.AI 普通 API | 有 | 未证实 | 未证实 | 普通 Chat 为 `/api/paas/v4`。此次未找到普通 API 使用上述 Coding 路径的官方依据，不能自动改计费渠道。[GLM](https://docs.bigmodel.cn/cn/api/introduction)、[Z.AI](https://docs.z.ai/api-reference/introduction)。 |
| MiMo API / Token Plan | 有 | 有 | 有 | 各自保留主机与密钥：OpenAI 两协议使用 `/v1`，Messages 使用 `/anthropic/v1`，Messages 支持 `api-key` 或 Bearer。[API Messages](https://mimo.mi.com/docs/en-US/api/chat/anthropic-api)、[API Responses](https://mimo.mi.com/docs/en-US/api/chat/responses)、[Token Plan Claude](https://mimo.mi.com/docs/en-US/tokenplan/integration/claudecode)、[Token Plan Codex](https://mimo.mi.com/docs/en-US/tokenplan/integration/codex-configuration)。 |
| Command Code | 按模型 | 按模型 | 按模型 | 共用 `/provider/v1`，Claude 仅 Messages，DeepSeek 等按模型目录 `supported_endpoints`。[官方文档](https://commandcode.ai/docs/provider)；该字段解析已在前一提交补齐。 |
| StepFun | 有 | 限定模型 | 有 | 共用 `/v1`；[Responses](https://platform.stepfun.com/docs/zh/api-reference/responses/responses-create)当前列出 `step-5-preview`、`step-3.7-flash`，工具仅 `function`。[Messages](https://platform.stepfun.com/docs/zh/api-reference/chat/messages-create)。Step Plan 是独立 `/step_plan/v1` 产品根，保留原路径。 |
| Novita AI | 有 | 官方不支持 | 限定模型 | Chat 推荐 `/openai/v1`，Messages 为 `/anthropic/v1` 并使用 Bearer。[Messages 契约](https://novita.ai/docs/guides/llm-anthropic-compatibility)、[Responses 边界](https://novita.ai/docs/guides/openai-agents-sdk)。旧 `/v3/openai` Chat 地址保持兼容。 |
| OpenAI | 有 | 有 | 未提供官方兼容入口 | Chat 和 Responses 共用 `/v1`；[OpenAI Docs](https://developers.openai.com/api/docs/guides/migrate-to-responses)。客户端入站 Messages 通过本项目路由转换，不能发送到官方 `/v1/messages`。 |
| Anthropic | 未提供官方兼容入口 | 未提供官方兼容入口 | 有 | 官方模型接口为 `/v1/messages`；[API 总览](https://platform.claude.com/docs/en/api/overview)。客户端入站 OpenAI 协议通过本项目适配，不能当作官方端点存在。 |

## 本次修复

1. 共享端点投影识别 MiniMax、MiMo 的规范根与 `/anthropic`，按协议切换路径；识别 GLM/Z.AI 原始 Coding 根，分别生成三个正式端点。修复 Chat 与 Responses 共用同一个根类型参数的旧实现。
2. Novita 的 Messages 投影到 `/anthropic/v1/messages`；新预设采用官方推荐 `/openai/v1`，旧 `/v3/openai` Chat 不改写，保存的域名、凭据及卡片原始地址不变。
3. 路由、模型连接测试、Messages 模型列表请求共用鉴权策略；已证实的 OpenRouter、SiliconFlow、Novita、MiMo 与 GLM/Z.AI Coding Messages 入口使用 Bearer。Anthropic 和自定义中转继续保持 `x-api-key`。
4. Desktop 先投影实际 Messages 根，再判断其路径布局，避免从正确 Coding / OpenAI 根切到 Messages 后仍被原地址拦住；保留完整接口地址禁止充当根地址、最终客户端与项目端点一致性核对。
5. OpenCode 直连配置按每个 SDK 分组的协议构造根地址，修复所有组误用当前选中模型协议的问题；Messages 官方 Bearer 网关使用 SDK 的 `authToken`，撤回旧认证选项并保留用户的其他请求头。路由模式继续共用本地入口和本地密钥。

根投影及官方鉴权规则只识别 HTTPS 标准端口、精确官方主机和已证明的标准路径。保留自定义前缀、非标准端口、本地地址以及相似主机名，不切换 CN/Global 或按量/套餐凭据。

GLM 的原始 `/api/coding/paas/v4` 保存了产品范围，可以安全正向投影。若用户只保存 `/api/anthropic` 或 `/api/v1`，无法判断 Chat 应使用普通 API 还是 Coding Plan，因此不会猜反向路径；需要用户明确填写对应的 Chat 产品根。模型列表地址独立于推理地址，不能因为 Messages 端点可用就宣称该根下必然也支持 `/models`。

## 验证边界

本次验证结果：

- `pnpm test:rust`：858 项库测试、11 项 Desktop 集成测试、20 项本地工作流测试通过，共 889 项通过，22 项真机用例忽略。
- `pnpm test:frontend`：通过。
- `pnpm build`：TypeScript 与 Vite 生产构建通过。
- OpenCode 配置额外使用真实 `@ai-sdk/anthropic@3.0.111`、模拟 fetch 发起请求：直连仅发送 Bearer，路由仅发送本地 `x-api-key`，保留自定义非鉴权头；未调用线上服务。
- `git diff --check`：通过。

回归覆盖协议切换、官方鉴权头、旧路径兼容、自定义路径保留、Desktop 根地址校验、混合 SDK 分组。所有线上推理权限、真实套餐计费、托管搜索以及客户端实际服务端能力仍未用真实密钥验证。

已有直连客户端配置需要重新应用，才会写入修复后的协议根和鉴权选项；路由使用共享端点逻辑，不修改既有卡片的原始地址与密钥。
