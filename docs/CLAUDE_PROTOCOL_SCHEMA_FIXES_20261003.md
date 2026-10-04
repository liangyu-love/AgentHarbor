# Claude Code 协议地址与 Gemini 工具输入修复（2026-10-03）

> 正式复审发现的 2 项 P1、2 项 P2 已落实后续源码修复，尤其 Gemini 不再补空 items。当前实现和验收见 [复审问题修复](CLAUDE_REVIEW_FIXES_20261003.md)，原发现见 [最近两轮代码审查](CLAUDE_LAST_TWO_FIXES_REVIEW_20261003.md)。以下保留当时修改与验收记录；此前构建包不含后续修复，暂不交付。

## 本轮反馈与定位

1. DeepSeek Flash 在 Anthropic Messages 协议下，直接接入和路由接入均出现模型不可用提示；Chat 兼容协议能聊天。
2. Gemini Flash High 出现 `tools...properties[query].properties[where]...items: missing field`，Messages 和 Responses 均可能触发。
3. 用户实测 GPT-6 Luna 与 GLM-5.3 Flash 的 Responses 聊天与工具调用已能运行。

Gemini 的报错位置是工具参数 schema，与普通用户文本、思考档位或单一外层协议没有直接对应关系。Anthropic/JSON Schema 允许 `{"type":"array"}` 表示不限元素类型的数组，但 Gemini 的函数结构要求数组包含 items。

## 修改

### DeepSeek 官方协议地址

- 新增共享 `newapi::protocol_base_url`。仅针对精确官方主机 `api.deepseek.com` 和标准根目录：Messages 使用 `/anthropic`，Chat/Responses 使用普通根目录。
- Router、连通性测试和客户端直接注入复用此逻辑。消息请求实际使用 `/anthropic/v1/messages`，count_tokens 沿用同一前缀；Claude 的 ANTHROPIC_BASE_URL 为 `https://api.deepseek.com/anthropic`。
- 不猜测自定义网关、不重写第三方域名或用户自定义路径、不删除 `[1m]` 的客户端能力标记。真实客户端测试确认 `[1m]` 不进入上游模型 ID。
- 官方参考（Context7）：<https://api-docs.deepseek.com/quick_start/agent_integrations/claude_code>、<https://api-docs.deepseek.com/guides/anthropic_api>。官方支持 DeepSeek 的 Messages 接口和 `deepseek-flash[1m]`，不能把用户选择 Messages 视为错误。

### Gemini 工具数组 schema

- 路由实际出站前，对 Gemini 模型的 Responses、Chat、Messages 三种工具形状统一处理。使用已有模型名称规范化识别渠道/供应商前缀和思考后缀。
- 仅在 schema 的数组类型缺少 items 时补 `items: {}`，保留原不限元素类型的 JSON Schema 含义；不猜成字符串数组或对象数组。
- 递归处理 properties、items、组合 schema、定义和条件等 schema 位置；已有 items、required、限制、说明以及真实工具参数不改写。default、examples、enum 的数据不会被当成 schema 修改。
- 非 Gemini 请求字节保持原样；已经适配的 Gemini 请求再次执行不会重复修改。出站诊断读取处理后的实际请求。
- 官方函数工具/数组结构参考（Context7）：<https://ai.google.dev/gemini-api/docs/function-calling>、<https://ai.google.dev/gemini-api/docs/generate-content/structured-output>。

### 协议覆盖可取消

- 单模型协议优先于提供商默认协议，这是既有规则。只修改默认协议不会覆盖用户已保存的单模型选择。
- 单模型下拉菜单增加“跟随默认”，将 protocol 保存为 null。随后修改提供商默认值即可正常继承；明确选择的协议仍保留。
- 没有修改用户已保存的提供商、密钥、单模型协议或 Claude 配置文件；直接注入需要用户在新应用中重新应用配置，才能写入修正后的地址。

## 验证

| 检查 | 结果 |
| --- | --- |
| Rust 全库串行 | 553 passed、0 failed、8 ignored |
| local_workflows | 9 passed |
| 前端交互回归 | 64 PASS |
| 四项 installed Claude Code 专项 | 4 passed、0 failed；52 次模型请求、18 次实际 Read |
| 新增原生 Messages 专项 | 直接/路由各执行一次 Read 与续轮；4 次请求，模型 ID 无 [1m] |
| Gemini HTTP 专项 | Chat/Responses/Messages 各两轮，6 次请求；query.where 显式 items、既有 integer 数组保留、混合 JSON 参数原样交付 |
| DeepSeek 地址回归 | 普通根、v1、anthropic、anthropic/v1；Messages/count_tokens/Chat/Responses、直接注入、第三方和自定义路径 |
| schema 回归 | 嵌套/union/nullable/definitions、非 schema 数据保留、非 Gemini 不变、幂等 |
| 协议继承 UI | 取消单模型覆盖、修改默认、保存后 null 和默认协议正确 |

四项 CLI 专项上游全部为 localhost 模拟服务。原 48 次跨协议请求与 16 次 Read 继续通过，新增原生 Messages 4 次请求、2 次 Read。隔离临时配置，只开放 Read 工具，不使用用户 MCP、密钥或真实计费上游。可选 hello 探测不计为模型请求；最终日志无 worker panic。

日志：

- `.runtime-test/claude-model-schema-final-rust.log`
- `.runtime-test/claude-model-schema-final-cli.log`
- `.runtime-test/claude-model-schema-frontend.log`
- `.runtime-test/claude-model-schema-gemini.log`
- `.runtime-test/claude-model-schema-focused.log`
- `.runtime-test/claude-model-schema-release.log`

## 复审与边界

复查官方域名/路径边界、每模型协议优先级、真实 CLI 的直接/路由路径、schema 递归位置、参数保留、非 Gemini 字节保留和此前思考缓存路径。上述回归未发现新增阻断问题。

- 测试确认路由送出的缺失 items 已补齐，不能据此声称用户所用线上中转已验收。中转自身可能使用不同的 Gemini schema 转换实现；完整 JSON Schema 支持范围仍由该服务决定。
- Gemini 的 schema 适配位于 AgentHarbor 路由。Claude Code 直接连接上游时，请求不经过此代码；需要使用路由或由上游修正工具转换。
- 保留前一版完整 reasoning_text 历史修复和无账号/后台窗口修复。新版本测试请新建 Claude 会话；旧版丢失的思考历史不能补回，进程重启/缓存失效后同样可能需要新会话。
- 未执行安装升级或全库 lint 清理；未改变用户数据与当前运行中的旧版。

## 交付

最新目录：`.runtime-test/deliverables/AgentHarbor-20261003-protocol-schema/`。此前 `deepseek-history` 包不包含本轮地址、Gemini schema 与协议继承菜单修复。

- TypeScript/Vite、Windows release 与 NSIS 构建成功。
- 交付 EXE 与 release SHA256 一致：`38C05F66EDC3E2B0813DA5184D591E7AC43EF93067C34AA4EB92371945B26395`。
- 隔离数据、Codex 和客户端目录启动 8 秒存活，窗口句柄非零、stderr 为零；检查后只结束本轮测试进程。记录为随包 `startup-check.json`。
- 退出旧版并运行新包。协议选择请核对单模型行；需要继承提供商默认时选择“跟随默认”。DeepSeek 直接接入请重新应用 Claude 配置。Gemini schema 适配请使用 AgentHarbor 路由，并新建 Claude 会话测试。
