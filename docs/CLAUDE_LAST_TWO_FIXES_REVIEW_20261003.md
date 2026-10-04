# 最近两轮 Claude 修复代码审查（2026-10-03）

> 本报告记录修复前的四项发现。用户授权后，四项已落实源码修复；当前处理说明和验收见 [复审问题修复](CLAUDE_REVIEW_FIXES_20261003.md)。以下保留原审查证据。

## 原审查结论

**REQUEST_CHANGES：2 项 P1、2 项 P2。暂不交付本轮构建包。**

范围为无账号开关、后台进程隐藏窗口、Messages 思考历史缓存、DeepSeek 官方协议地址、Gemini 工具 schema 适配和单模型协议继承菜单，以及相关路由调用和回归测试。当前工作区还有其他历史修改，本报告不代表全库审计。

本次进行静态代码审查及官方 API 契约核对，没有新增或重新运行测试，没有修改生产代码，也没有调用真实计费上游。此前验收记录中的 553 个 Rust 测试、9 个 local_workflows、64 个前端回归和 4 个 installed CLI 专项通过，仍是已有测试的结果；这些测试使用本地模拟上游，不能证明线上 Gemini 转换器接受补齐后的 schema。

本报告取代两份修复文档此前“未发现新增阻断问题”的复审结论。

## P1

### 1. Gemini 数组补空 items 仍不满足受限 Schema 契约

- 位置：`src-tauri/src/core/router_proxy/session_compat.rs:77–78`。
- 当前把缺少 items 的数组补成 `items: {}`。这在完整 JSON Schema 中表示不限元素类型；如果用户的中转把工具转换为 Google `FunctionDeclaration.parameters`，items 对应 Google 的受限 `Schema`，其 type 为必填字段，空对象仍不满足契约。因此原来的工具数组问题尚不能视为解决，报错可能从缺 items 变成缺 type。
- 官方证据：Google discovery 文档 `https://generativelanguage.googleapis.com/$discovery/rest?version=v1beta` 中，`Schema.properties.type` 说明为 `Required. Data type.`，TYPE_UNSPECIFIED 标注不应使用，items 引用 Schema。`FunctionDeclaration.parametersJsonSchema` 是另一种可选通道，不能将两种通道的支持范围混为一谈。
- 测试缺口：`claude_continuation_tests.rs:33` 直接把 `items == {}` 设为通过条件，没有验证最终 Google FunctionDeclaration 契约。这能证明 AgentHarbor 补了字段，不能证明 Google 接受它。
- 建议：核对实际工具定义和中转最终 payload；已知元素类型时保留或补真实类型，不限类型时应使用可支持它的 JSON Schema 通道或明确报告不支持，不能随意猜成字符串。补充会拒绝无 type 的最终 Google 契约测试。
- 边界：不能据此断言所有 Gemini 中转都会拒绝 `{}`；问题限定于使用受限 parameters 通道的转换路径。

### 2. 成功交付的 max_tokens 文本回复仍丢失思考历史

- 位置：`src-tauri/src/core/protocol/message_history.rs:210–212`，以及 `anthropic_sse.rs:397–399`。
- 非流式缓存和流式完成输出均只接受 status=completed。与此同时，`anthropic.rs:290–333` 接受正常的 incomplete 文本，把 max_output_tokens 映射为 max_tokens；`anthropic_tests.rs:398` 的测试也确认客户端会收到 message_stop。
- 触发条件：DeepSeek 返回带 reasoning 的普通文本，在 token 上限处终止；该可见 assistant 回复已进入客户端历史，随后继续对话或调用工具。缓存没有对应 reasoning，Chat/Responses 续轮无法恢复它，可能再次出现 reasoning_text 必须回传的 400。
- 建议：缓存经响应校验、已成功交付的 incomplete 普通文本及其原始思考输出，并覆盖流式和非流式的“max_tokens 后继续工具”场景。失败响应、无效输出和不完整工具调用应继续拒绝。

## P2

### 3. 缺少会话标识时，不同 system 的普通回复缓存可能碰撞

- 位置：`src-tauri/src/core/protocol/message_history.rs:190–201`，相关前缀计算为 `149–180`。
- scope 的 metadata.user_id 可以缺失；Router 的 affinity_key 在没有规定会话头时也为空。顶层 system 不参与身份，消息内的 system/developer 被前缀过滤。
- 可静态推导的触发条件：同一存储、提供商、密钥、模型和协议下，两个合法无会话标识请求分别带不同 system，但 user 均为 Hello；两次 assistant 可见回复也均为 Hello。它们的缓存键完全一致，第二次写入替换第一次的 reasoning 和原始 output ID。第一会话续轮会恢复第二会话的数据。
- 影响：对要求原始思考或 output ID 的上游回放错误历史。当前 Claude Code 专项带 metadata.user_id，这条不表示其常规路径必然混用。
- 建议：无可靠会话标识时，使用稳定 system/context 区分请求，或者停止会发生歧义的普通回复缓存恢复；补充不同 system、相同可见对话、缺会话标识的隔离回归。保留有可靠会话标识时对动态 system 的容忍。

### 4. “跟随默认”的说明展示了现有覆盖协议

- 位置：`src/components/newapi/provider-model-editor.tsx:197`。
- 该选项用 `effectiveModelProtocol(model, protocol)` 生成说明，而此函数先取 model.protocol。提供商默认 Responses、模型覆盖 Chat 时，菜单会显示“跟随默认：Chat”，但点选后 protocol 清为 null，实际变成 Responses。
- 影响：在用户排查协议不兼容时误导选择；保存 null 的逻辑本身正确。
- 建议：用清除 protocol 后的模型计算继承结果，保留既有 Claude 自动协议规则；前端回归断言菜单所展示的实际继承目标，而不只匹配“跟随默认”的文字前缀。

## 其他核对结果与边界

- DeepSeek 官方地址 helper 的精确域名限制、标准根路径转换和自定义路径保留，本次未发现新增阻断。
- 无账号开关解除前端禁用，以及后台版本探测/监控进程添加 Windows 隐藏窗口 flags，本次未发现新增阻断。静态审查不能证明所有黑框来源均已消除。
- Gemini schema 适配只在 AgentHarbor 路由执行；直接连接上游的 Claude 请求不经过它。
- 未验证 DeepSeek 官方 Messages 的模型列表接口和真实中转的完整 Schema 转换实现，不把未经证实的兼容性猜测列为确定漏洞。
- 建议先修复两项 P1，并补上两项 P2 的回归，再进行构建交付。此轮只整理审查结果。
