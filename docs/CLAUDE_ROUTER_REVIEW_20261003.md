# Claude 路由审查（2026-10-03）

## 结论

**初审：REQUEST_CHANGES，确认 3 项问题（P1 两项、P2 一项）。后续修复已完成，并通过定向复现、全库回归和真实 Claude Code 工具续轮验收。**

下文保留初审时的复现与代码位置，属于修复前记录。当前修复说明与验证边界见 `docs/CLAUDE_ROUTER_FIXES_20261003.md`。最新交付为 `AgentHarbor-20261003-router-fixed`，此前 `config-conflict` 包不含这 3 项路由修复。

本轮已修复配置恢复、重新应用与冲突更新必须先放弃接管的问题。该修复与以下路由审查结论分开验收。当前 EXE 可用于验收配置修复，不能据此认定跨协议工具续轮已经完整可用。

范围：12 个核心文件——protocol 下 anthropic、anthropic_sse、chat、responses、sse、reasoning、mod、history，以及 router_proxy、session_compat、route_trace、relay_usage；并检查相关回归。工作区有大量历史未提交修改，本报告审查当前磁盘代码，不把所有差异归因于本轮。

## 1. P1：工具续轮没有保留上游思考内容

- 位置：`src-tauri/src/core/protocol/anthropic.rs:313`、`anthropic_sse.rs:272`、`protocol/mod.rs:167`。
- Chat 回复的 `reasoning_content` 先被转换为 canonical reasoning，随后在生成 Messages 回复时被丢弃。流式分支同样忽略 reasoning。Messages 下一请求只从客户端传回的消息重建；现有 history 仅供 Responses 调用方使用，不能恢复这里丢失的信息。
- 复现：模拟 Chat 回复同时含 `reasoning_content` 和 tool_call；将生成的 Messages assistant 内容与 tool_result 组成下一请求。转换结果 `assistantReasoningPreserved=false`，但 `continuedEffort=high`，仍是思考请求。
- 影响：要求回传思考内容的 DeepSeek 工具续轮缺少必需字段，可能在第二轮返回 400。这里实证的是字段丢失，没有调用真实 DeepSeek 验证 HTTP 错误。参考官方说明：<https://api-docs.deepseek.com/guides/thinking_mode>。
- 建议：为 Messages 跨协议续轮保存并恢复原始上游思考数据，按提供商、凭据、模型和会话隔离，并设置容量与过期边界；不能伪造 Anthropic 签名。覆盖流式/非流式、多工具续轮和模型切换。Responses 的 opaque reasoning 也应单独验收，不能以摘要代替其身份数据。

## 2. P1：交错的工具与文本被拆成不合法的 Chat 工具历史

- 位置：`src-tauri/src/core/protocol/chat.rs:215`，关联 `anthropic.rs:158`。
- 合法 Messages assistant 内容为 `tool_use(c1), text, tool_use(c2)`，随后 user 返回两个结果。canonical 转换在文本处 flush；Chat 转换因此生成两条 assistant 消息，c1 的工具结果被隔到了后一条 assistant 之后。
- 实际结果：`assistant(tool_calls=[c1]) → assistant(text, tool_calls=[c2]) → tool(c1) → tool(c2)`。
- 影响：要求每个 assistant 工具调用组紧接相应工具结果的 Chat 上游可拒绝该历史。即便第一次工具调用成功，后续轮仍有失败风险。
- 建议：保留原 assistant 轮次边界，把同轮文本和全部 tool_calls 合并到一条 Chat assistant 消息，后接整组结果；不能跨真实轮次随意合并。覆盖 tool/text/tool、tool/text、text/tool/text 及多结果顺序。

## 3. P2：客户端收到流式转换错误，诊断与账本仍显示成功

- 位置：`src-tauri/src/core/router_proxy.rs:1459`、`relay_usage.rs:579`。
- `observed_stream` 在转换前观察原始上游字节。转换器发出的错误没有传回 observation，账本 success 仅由 HTTP 状态码决定。
- 本地完整 HTTP 链路复现：上游返回 200 SSE，tool arguments 为不完整的 `{`，随后 tool_calls 结束、usage、DONE。Claude 侧收到 `event: error`，但 `traceDone=true`、`traceError=null`、`traceFailureStage=null`，`ledgerSuccessCount=1`。
- 影响：用户在请求级诊断中无法定位本地转换失败，成功统计被抬高。
- 建议：让 translator 的完成/失败状态参与最终诊断与账本定稿，区分上游 HTTP 成功和客户端交付成功。保留已发生的 token 用量，不因失败清零；覆盖异常参数、截断 SSE、上游错误和客户端断开。

## 复现与验证边界

- 证据：`.runtime-test/claude-router-review-probe.log`。
- 复现源归档：`.runtime-test/claude_router_review_probe.rs`。如需重跑，放回 `src-tauri/examples/claude_router_review_probe.rs`，使用 MSVC 工具链运行对应 cargo example；其相对 include 依赖此位置。它只使用临时目录和 localhost 模拟上游。
- 本轮配置修复：Rust 单元测试 538 通过、5 ignored；local_workflows 9 通过；前端交互回归 63 通过；TypeScript/Vite 与 Windows release/NSIS 构建通过。
- 既有四模型 × Chat/Responses 的 8 次真实 CLI → 本地 mock 测试只覆盖普通文本回复，没有覆盖真实工具执行续轮。不能将这些通过数当成上述缺陷不存在的证明。
- 未使用真实密钥、未调用计费上游；没有验证所有提供商的私有扩展。未发现需要本轮实施的大型重构或无关删除项。

## 后续修复顺序

先解决思考内容的续轮恢复，再修正同轮工具消息归并，最后统一流式失败诊断和账本状态；补齐本报告中的复现场景后重新审查。
