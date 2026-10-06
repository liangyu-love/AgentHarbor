# Claude Desktop 网页访问与搜索

2026-10-05 排查用户提供的路由模式截图，区分两个独立问题。

## WebFetch 与网页访问

错误 `Host ... is not on the network allowlist (cowork-egress-blocked). Allowed: 127.0.0.1` 来自 Desktop 工具联网策略。不是 Command 凭据、DeepSeek 模型名或中转协议选择导致的拒绝。

核对本机 Desktop 2.19675.0 的 `app.asar` 配置定义：

- `coworkEgressAllowedHosts` 用于 Cowork 和 Desktop 内 Code 会话的工具请求。
- 字段省略时，工具只能访问推理端点。路由端点是本地网关，因此允许列表只有 `127.0.0.1`。
- 字段支持精确主机、子域通配符和 `*`；`*` 允许公网访问。WebFetch 仍阻止私网地址。
- 该字段不涵盖 WebSearch、推理请求或 MCP 流量；独立部署的 Claude Code managed-settings 仍可优先限制网络。

本次将共享 `profile_json` 改为写入 `coworkEgressAllowedHosts: ["*"]`，与 CC Switch 的 `build_gateway_profile` 一致，直连和路由同时修复。字段出现在配置文件预览中，沿用四文件事务、首次接管基线、回滚和恢复逻辑。恢复接管前配置会恢复原来的出口字段，而非保留工具新写入的 `*`。

已有配置需要在新版本内重新应用，退出并重启 Desktop，新会话才读取新配置。代码更新不会自动修改真实 Desktop 文件或其管理员策略。

## WebSearch 与 Command 搜索适配器

Command 源码 `src/web-search.ts` 中的 `CommandCodeSearchProvider` 是 dsh 插件的独立搜索实现：

- 使用同一 Command API key，以 Bearer 调用根地址下的 `POST /alpha/web-search`。
- 请求体是 `{ query, numResults }`，响应解析为 `{ results: [{ title, url, snippet }] }`。
- 插件在 dsh 的 `ctx.web` 注册搜索 provider，`webSearch` 默认开启。

Claude Desktop 的 WebSearch 属于推理侧托管工具。路由现在为 Chat Completions 增加独立的搜索执行链路：把搜索声明转换为普通函数，让模型生成查询，调用搜索服务，再把真实搜索结果送回原模型继续生成回答。模型与搜索服务分别使用各自卡片的凭据和网络出口；不会把其他模型供应商的密钥送到 Command。

2026-10-06 补齐 Chat 搜索执行后，Desktop 的入站接口仍是 Anthropic Messages；出站按卡片内该模型的协议配置决定，与 Codex 共享转换器：

| 目标协议 | WebSearch 行为 |
| --- | --- |
| Anthropic Messages | 原生透传，由上游执行搜索；仍要求上游实际支持托管工具。 |
| OpenAI Responses | 转换为原生 `web_search`，带域名白名单、近似位置、`max_tool_calls` 和来源字段；将响应和 SSE 转回 Messages 搜索块及引用链接。 |
| Chat Completions | 本地执行搜索函数，调用单独配置的搜索服务，再由原 Chat 模型消费结果；模型不必来自 Command。普通 function/MCP 工具转换继续支持。 |

这不会为搜索自动切换协议、供应商或模型。已配置为 Messages 的接口仍走 Messages；已配置为 Chat 的接口仍走 Chat。Responses 是可选的出站协议，不是 Desktop 的新入站接口。

### Command Code 的模型协议选择

2026-10-06 核对 [官方 Provider API 文档](https://commandcode.ai/docs/provider) 与公开 `GET https://api.commandcode.ai/provider/v1/models`：网关同时提供 Chat Completions、Responses、Messages，但每个模型支持的端点不同。Claude 模型仅支持 `/messages`；公开目录中的 `deepseek/deepseek-v4.1-flash` 支持 `/chat/completions` 和 `/responses`，不支持 `/messages`；`deepseek/deepseek-v4-flash-fast` 当日仅声明 `/chat/completions`。具体以刷新后的模型目录为准，不能由模型基础能力推断后缀版本也支持相同协议。

Command 预设的 Chat 是默认值。中转编辑窗口的模型行可单独选择三种协议；模型显式选择优先于卡片默认，未单独选择的 Claude 模型优先使用目录声明协议，否则默认 Messages，其他模型继承卡片默认。模型列表解析同时接受 `supported_endpoint_types` 和 Command 的 `supported_endpoints`，供现有协议可用性校验消费；不会把目录刷新变成用户协议覆盖。

本项目直接构建 HTTP URL，填写 `https://api.commandcode.ai/provider` 或 `https://api.commandcode.ai/provider/v1` 都会得到正确的 `/provider/v1/{messages|chat/completions|responses}`。外部 SDK 的 base URL 追加规则无需套用到这里。DeepSeek 官方自己的 Messages 兼容入口与 Command 所支持的端点范围分别由各服务决定，不能因为官方可用 Messages 就把 Command 的 DeepSeek 模型也改为 Messages。

Responses 声明支持该模型，仅代表推理端点可用。Command 文档说明内置 `web_search` 会继续交给上游接受或拒绝，因此切换到 Responses 本身不能保证搜索可用；Chat 的独立搜索执行链路仍有必要。本次仅访问公开模型列表，未用真实密钥验证线上推理或托管搜索。

搜索转换接受已知版本的直接调用。新版搜索的程序化调用、非空域名黑名单、`response_inclusion` 等无等价设置会明确拒绝，不静默删掉约束。Chat 的独立搜索服务不支持近似位置，收到 `user_location` 会明确报错。结果保留真实来源 URL，引用以链接显示；不伪造 Anthropic 的加密引用索引。完整 Responses 搜索输出在有界、按会话和渠道隔离的缓存中保留以供回放。缓存不可用时从已交付的搜索块重建查询及来源。

参照 CC Switch，来源优先取每次搜索的 `action.sources`，终态文本引用补齐标题；未归属的引用补到最后一次成功且未超限的搜索。带托管搜索的流式正文暂存至结束帧，以避免提前发布不完整的搜索结果；期间转发 Messages 启始事件并输出 ping。正文暂存上限 8 MiB，缺少结束帧、超限或传输失败会输出错误事件。普通对话和普通工具调用仍增量输出。

网页白名单修复与托管搜索执行是两项独立能力。首个独立搜索后端为 Command 的 REST 搜索服务，其身份识别只决定搜索请求的地址，不决定模型路由协议。

“请求级路由诊断”中的 **WebSearch 搜索服务** 可选择已配置的 Command 卡片，或保留“自动”：

- 当前推理卡片也能调用 Command 搜索时，优先使用该卡片。
- 其他 Chat 模型可复用唯一已配置的 Command 搜索卡片；多张候选卡片需要显式选择。
- 显式选择后，删除该卡片或移除其凭据会显示服务不可用，不会悄悄切换另一个计费账户。重新选择服务或“自动”即可更新。
- 搜索卡片支持根地址、`/provider`、`/provider/v1` 的 `https://api.commandcode.ai`，需要推理 API key；其模型协议、当前选中模型、Codex 是否启用该卡片均不限制搜索服务使用。

原生 Messages 和 Responses 继续由上游执行搜索，不使用这个独立后端。配置搜索服务无需重新注入 Desktop，也不会更改其模型档位。

Chat 搜索结果中的域名白名单会传给搜索 REST API，并在本地按主机及子域边界再次过滤；`example.com` 不会匹配 `notexample.com`。模型读取实际返回的摘要，Desktop 收到真实 URL、标题及搜索结果块。内部多轮 Chat 消息和私有推理只留在有界的会话缓存里；缓存失效仍能从公开搜索块恢复查询、来源及工具配对。后续关闭搜索不会丢失既往搜索历史。

每次内部模型调用单独记录同一请求下的不同 attempt；搜索 REST 不被记成推理 token 或伪造费用。模型轮次共享调用方的输出 token 预算，`max_uses` 在本次请求中计数，用完或搜索服务失败后撤下搜索函数，让模型处理结果。混合客户端工具会交回 Desktop，避免在客户端结果尚未返回时继续请求模型。

Chat 搜索采用非流式内部模型调用，收到最终回答后按 Messages SSE 输出，等待期间立即返回 ping 并每 15 秒保活；取消会释放正在进行的请求，停止后续模型调用。总执行时间上限 900 秒、模型轮次上限 32、单个上游正文和累计输出上限 8 MiB，完整 Chat 请求沿用 64 MiB 限制。该执行流程固定本次模型及搜索账户，不在搜索过程中自动切换渠道。

`max_uses` 是单次 API 请求内的上限，不能扣减会话历史次数（[Anthropic 官方字段定义](https://github.com/anthropics/anthropic-sdk-python/blob/main/src/anthropic/types/web_search_tool_20250305_param.py)）。Responses `max_tool_calls` 对单次 response 的全部内置调用限额生效，本转换当前只映射搜索这一种内置工具，因此不与其他内置工具争用预算。两端超限表现不同：Responses 忽略额外调用，Anthropic 可以返回 `max_uses_exceeded` 工具错误；本地响应侧只对实际上游返回的越限调用报错，不会凭调用数达到上限就编造一个错误。

三种路由流式路径在上游响应头已返回后，每 15 秒静默输出一条 SSE 保活注释；它不计入模型响应/用量，也不重置上游 900 秒读取超时。断流或超时仍输出明确错误并结束。

## 验证边界

Command 端点声明修复通过 19 项模型接口测试、7 项模型协议测试、1 项 Desktop 三协议路由/工具/用量集成测试及完整前端回归。新增覆盖官方端点字段解析、旧字段优先级、异常字段容错、模型实际协议限制与两种 base URL 的路径拼接；未使用真实密钥调用线上推理端点。

2026-10-06 验证：整库 Rust 通过 846 个 lib、11 个 Desktop 集成与 20 个本地工作流用例，22 个真机/辅助用例仍 ignored；补充搜索账户选择用例后，37 项搜索专项全部通过。完整前端回归与 TypeScript/Vite 生产构建通过。Chat 的自动化覆盖两个本地入口、独立凭据、真实格式的来源与摘要、连续对话、缓存失效、关闭搜索、次数/输出预算、域名约束、流式取消、上游错误和畸形消息。未修改真实 Desktop 配置，未使用真实推理密钥执行付费搜索；线上 Command REST 搜索及 Desktop 实际渲染仍需使用新版本复测。

参考源码：`src-tauri/src/core/claude_desktop/profile.rs`、`transaction.rs`；参考项目 `cc-switch/src-tauri/src/claude_desktop_config.rs`、`dsh-commandcode-provider/src/web-search.ts`。
