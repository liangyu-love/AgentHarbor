# Claude 路由修复与复审（2026-10-03）

> 正式复审发现的 2 项 P1、2 项 P2 已落实后续源码修复。当前实现和验收见 [复审问题修复](CLAUDE_REVIEW_FIXES_20261003.md)，原发现见 [最近两轮代码审查](CLAUDE_LAST_TWO_FIXES_REVIEW_20261003.md)。以下保留当时修改与验收记录；此前构建包不含后续修复，暂不交付。

## 已完成的三项修复

### 1. 工具续轮保留上游思考数据

- 新增 `protocol/message_history.rs`，在本地内存保留每一成功 assistant 回复的完整上游 output，包括普通聊天、工具回复与最终回复；Chat 回传原 `reasoning_content`，Responses 回传完整原始 output（包括 reasoning.content 中的 `reasoning_text`、原始 message/function_call/reasoning ID、encrypted_content 和 summary）。不向 Claude 伪造签名思考块。
- 隔离依据包括本地存储位置、提供商、接口地址、凭据摘要、模型、协议、会话标识、用户/助手历史前缀、可见助手文本和工具 ID/名称/参数。
- 真实 Claude Code 会在续轮重写动态 system 消息、移动 cache_control 标记。历史匹配忽略这些非身份信息，实际发往上游的消息仍保留。JSON 字段顺序、文本字符串/文本块表示差异不会破坏身份。
- 前缀摘要按请求线性计算，避免每一历史轮重复哈希整个对话；内存最多 512 条、16 MiB，闲置 1 小时过期，命中刷新存活时间。缓存不落盘，不记录凭据明文。没有原厂思考内容时不会凭空给 Chat 增加 reasoning_content 字段。
- 流式分支在成功终止事件交给客户端前保存历史，避免下一轮先到达；失败或不完整工具响应不进入缓存。

### 2. 交错工具与文本生成合法 Chat 消息

- Messages 的同一 assistant 轮，先收集文本和全部工具调用，统一生成一条 Chat assistant 消息；紧接完整工具结果组。
- 覆盖 tool/text/tool、tool/tool/text、text/tool/text/tool/text，不跨用户或工具结果轮次随意归并。Responses 继续保留原内容次序。

### 3. 转换错误参与诊断和用量成功判断

- 转换器暴露明确的终止/失败状态；Router 在转换之后完成观察器，诊断记录 translation 阶段错误。
- 畸形工具参数、截断 SSE、上游错误、非法非流式回复和中途断开不再仅凭 HTTP 200 计为成功。
- 已报告的 token 和计费数据保留，原上游 HTTP 状态也保留；成功状态独立反映交付结果。
- 正常协议终止后及时结束响应，避免客户端收到 message_stop 后主动断开被误计失败。

## 实测结果

| 验证 | 结果 |
| --- | --- |
| Rust 全库串行 | 549 passed、0 failed、7 ignored |
| local_workflows | 9 passed |
| 前端交互回归 | 63 PASS |
| 真实 Claude Code 普通文本专项 | 四模型 × Chat/Responses，8 次请求通过 |
| 真实 Claude Code 工具专项 | 四模型 × Chat/Responses，8 次 Read 工具执行、16 次请求通过 |
| 真实 Claude Code 先聊天再工具专项 | 四模型 × Chat/Responses，8 次 Read 工具执行、24 次请求通过 |
| 本地 HTTP 路由专项 | 两协议 × 流式/非流式；直接工具、先聊天再工具，以及 4 类错误，共 24 次请求通过 |
| 中断、身份隔离、动态 system、无思考字段、多工具轮、缓存容量/过期 | 回归通过 |

真实 CLI 专项使用已安装 Claude Code，工具只允许 Read，读取新建临时目录内的测试文件；后续请求必须实际带回文件内容和对应原厂思考数据。DeepSeek Flash、GPT-6 Luna、GLM-5.3 Flash 和 Gemini-3.8 Flash 在两种协议下均完成该链路。测试中的上游均为 localhost 模拟服务，未使用真实密钥调用外部计费服务。

日志：

- `.runtime-test/claude-deepseek-history-final-rust.log`
- `.runtime-test/claude-deepseek-history-final-cli.log`
- `.runtime-test/claude-deepseek-history-frontend.log`
- `.runtime-test/claude-deepseek-history-http.log`
- `.runtime-test/claude-deepseek-history-release.log`

## 修复后复审

原三项发现均有修复及回归覆盖。用户新反馈揭示旧版仅保存工具回复的缺口：DeepSeek Responses 要求带 tools 的后续请求回传此前所有 assistant 轮的 reasoning_text；先聊天再调用工具时，普通聊天的思考历史也不能丢失。已扩展为所有成功 assistant 回复的完整输出保存，并增加不含 encrypted_content、仅含真实 reasoning_text 结构的严格模拟验收。复查了完整输出与原始 ID 恢复、缓存隔离与边界、动态消息身份、同轮工具归并、流式终止竞争、失败时用量保留和非流式错误路径，未发现新增阻断问题。保留此前配置接管冲突修复。

## 无账号开关与后台窗口

- 无账号开关原来被 `!routingEnabled` 禁用；移除此限制，允许在开启注入前保存无账号选择。后端仍按当前连接状态决定是否写入客户端配置。前端覆盖关闭路由时开/关，后端覆盖先选择、再注入、最后恢复原配置。
- Claude 版本检测和 Codex 后台 CLI（直接 EXE 和 cmd shim）统一增加 Windows `CREATE_NO_WINDOW`。主程序 release 已使用 Windows GUI 子系统。启动存活和 stderr 检查不等同于监测全部瞬时窗口，黑框最终消失仍需用户环境复测。
- 官方格式参考：<https://api-docs.deepseek.com/guides/responses_api>、<https://api-docs.deepseek.com/api/create-response>、<https://api-docs.deepseek.com/guides/thinking_mode>。通过 Context7 核对。

验证边界：

- 通过本地协议模拟与真实客户端验证，不能代替每家真实上游的服务验收。
- 上游思考缓存仅在当前进程内有效。重启应用、超过闲置时限或容量淘汰后，旧会话的这部分数据无法凭空恢复；依赖这些数据的旧工具续轮需要新开会话。切换提供商/凭据/模型不会挪用旧上游的私有思考数据。
- 未实施无关架构重构、全库 lint 清理或依赖升级。

## 交付

最新目录：`.runtime-test/deliverables/AgentHarbor-20261003-deepseek-history/`。此前 `router-fixed` 包不包含本轮普通聊天 reasoning_text、无账号开关和后台窗口修复。

包含直接运行的 `AgentHarbor.exe`、可选安装包 `AgentHarbor-Setup.exe`、许可证、使用说明、验收说明与 SHA256。启动验证记录保存为 `startup-check.json`。

Windows release/NSIS 构建成功。交付 EXE 与 release SHA256 一致；隔离数据、Codex 与客户端目录启动 8 秒存活，窗口句柄非零，stderr 为零。验证后仅结束本次启动的测试进程。

EXE SHA256：`99527F73C3D7776EBABE449840A91C2E4CB60991408C2123F8F559FD4BE3FD9E`。退出旧版并运行新 EXE 后，新建 Claude Code 会话再测；继续使用已选的 Responses 协议。应用重启或缓存失效后同样可能需要新会话。
