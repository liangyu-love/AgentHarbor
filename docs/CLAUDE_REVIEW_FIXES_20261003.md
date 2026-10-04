# Claude 路由复审问题修复（2026-10-03）

本轮处理 [最近两轮代码审查](CLAUDE_LAST_TWO_FIXES_REVIEW_20261003.md) 中的四项发现。源码修复和回归完成后，按用户后续要求生成新交付包；没有使用真实密钥调用计费上游。

## 修复

### P1：Gemini 不限类型数组的工具参数

- 删除补 `items: {}` 的旧方案。缺少 items、空 items 或没有可用元素类型的数组，无法直接表示为 Google 的受限 Schema。
- 新增 `router_proxy/tool_schema.rs`。仅对包含上述数组的 Gemini 工具，将完整工具参数对象序列化为内部 JSON 字符串；出站 schema 为有明确类型的 object/string，原始工具 schema 放在字符串字段说明中。没有凭空将数组元素限定为字符串或对象。
- Responses、Chat、Messages 的非流式和流式返回都会解码该内部表示，客户端仍收到原始工具参数对象，混合类型、null、嵌套数组和对象均保留。已有明确类型且不需要适配的工具沿用原 schema 和参数。
- 续轮中的历史工具调用重新编码为同一种上游表示；原工具 ID、消息 ID、思考字段和工具结果保持原值。
- 流式只暂存受影响工具的参数，完整校验后交给客户端；普通文本继续流式交付。无效 JSON、非对象、额外内部字段、截断或超过缓存限制的参数不会生成成功的工具终止。
- 回归检查实际发送的 object/string schema，明确拒绝旧版空 items 的测试条件；覆盖三协议的流式/非流式和工具续轮、逐字节分块、CRLF、混合工具以及 Chat chunk 元数据。

### P1：max_tokens 文本回复的思考历史

- `message_history::cacheable_response` 统一判断可回放的输出：除了合法 completed 响应，也允许经过校验、因 max_output_tokens 终止的普通文本。
- Messages 流式转换器保留这类已交付输出，Router 在终止事件交给客户端前记录它；非流式走同一判断。
- 失败响应、内容过滤和不完整工具调用仍不回放。Chat/Responses 均覆盖“普通文本达到 token 上限后继续调用工具”的思考恢复路径。

### P2：缺会话标识的缓存隔离

- 新增显式 session 参数，由 Router 传入已有会话头生成的 affinity_key；metadata.user_id 和 session 都为空时，顶层 system 和消息中的 system/developer 参与身份摘要。
- cache_control 标记不参与上下文身份；有会话标识的动态 system 兼容逻辑保留。
- 两份不同原始输出若仍落在完全相同的可见对话键上，缓存记录歧义标记并停止回放，避免覆盖另一会话的思考或原始 ID。相同输出的重复记录仍可正常命中。歧义标记同样受容量和闲置过期限制。
- 覆盖不同 system/developer、相同可见回复、缺失/空 user_id 的隔离，显式会话头兼容动态 system，以及重复输出/歧义/过期的缓存边界。

### P2：“跟随默认”文案

- 计算菜单文案前将单模型 protocol 清为 null，展示实际继承结果，保留既有 Claude 自动协议规则。
- 前端断言菜单展示的是提供商默认值，随后清除覆盖、改变默认值并保存，验证最终 protocol=null。

## 验收

| 检查 | 结果 |
| --- | --- |
| Rust 全库串行 | 562 passed、0 failed、8 ignored |
| local_workflows | 9 passed、0 failed |
| 前端交互回归 | 64 PASS |
| TypeScript | tsc --noEmit 通过 |
| 已安装 Claude Code 专项 | 4 passed；52 次模型请求、18 次实际 Read |
| Gemini 本地 HTTP 专项（包含于 Rust 全库） | 三协议 × 流式/非流式 × 工具首轮/续轮，共 12 次请求通过 |
| 新增边界回归 | token 截断、无会话隔离、缓存歧义、逐字节 SSE、错误/截断 JSON、typed 工具保留、CRLF 与 chunk 元数据均通过 |

本轮曾出现一个新增测试夹具把 Chat `[DONE]` 当 JSON 解析的失败，已修正夹具；上述为最终源码的回归结果。没有使用真实计费上游。日志：

- `.runtime-test/claude-review-fixes-final-rust.log`
- `.runtime-test/claude-review-fixes-local-workflows.log`
- `.runtime-test/claude-review-fixes-cli.log`
- `.runtime-test/claude-review-fixes-frontend.log`
- `.runtime-test/claude-review-fixes-tsc.log`

## 复查与实际边界

- 已复查参数编码/解码对称性、工具历史重放、流式终止、原 ID 保留、错误拒绝、缓存隔离和继承显示。本轮未发现新的阻断项。
- 所有上游验收均为 localhost 模拟服务。新的模拟检查不再把空 items 当作成功条件，但仍不能替代用户实际中转的线上验收。
- 此适配只在 AgentHarbor 路由执行，Claude Code 直接连接提供商时不会经过它。受影响的工具参数会增加一次 JSON 编解码，参数增量在完成前暂存；普通文本仍保持流式。
- 思考缓存仍只在内存中，受容量和过期限制。重启、过期或身份无法区分时不能伪造缺失的上游思考，需要带可靠会话标识或使用新会话。
- 先前生成的 `deepseek-history` / `protocol-schema` 包不含本轮修复；请使用下方新交付包。

## 打包交付

- 目录：`.runtime-test/deliverables/AgentHarbor-20261003-review-fixes/`。
- `AgentHarbor.exe` 为直接运行版；`AgentHarbor-Setup.exe` 为 Windows x64 安装包。
- 使用当前工作区构建，前端和 Rust release、NSIS 打包成功；构建期间 370 个源文件哈希未变化。记录见 `source-manifest.json`，未把未提交修改当成旧 HEAD 打包。
- 交付 EXE 与 release EXE 的 SHA256 相同；PE 子系统为 Windows GUI。
- 在全新隔离应用数据和 Codex 目录启动 8 秒，进程存活、主窗口句柄非零、stderr 为 0。检查结束后已关闭测试实例，未修改用户现有配置。
- EXE SHA256：`19979D5D0AF85CDCDB7BAF2DCC41BE452857EB8233F47E8D981C5E73B745B421`。
- 本次没有运行安装升级程序；实际中转服务仍需线上验收。
