# 多客户端配置

中转页的「客户端配置」用于 Claude Code 和 OpenCode。Codex 继续使用上方的路由注入入口。

1. 创建供应商，设置 API 密钥、协议和默认模型，然后启用供应商。
2. 在客户端卡片中选择「配置」，选择供应商和模型。
3. 预览待写入的字段，确认后应用。预览不返回本地路由令牌或供应商密钥。
4. 保持 AgentHarbor 运行，重新打开客户端会话。
5. 使用「恢复原配置」撤回配置；原有官方供应商、权限和其他设置会保留。

| 客户端 | 支持协议 | 写入内容 |
| --- | --- | --- |
| Claude Code | Anthropic Messages | `settings.json` 的网关环境变量和默认模型；Opus、Sonnet、Haiku 别名均固定到所选模型 |
| OpenCode | Responses、Chat Completions、Anthropic Messages | 添加全部已启用供应商的兼容模型，并设置所选默认模型。已有供应商保留，可用 `/models` 切换 |

这里没有协议转换。Claude Code 不能直接使用仅支持 Chat Completions 的供应商。
OpenCode 分别使用 `@ai-sdk/openai`、`@ai-sdk/openai-compatible` 和 `@ai-sdk/anthropic`。
模型上下文及输出长度优先使用供应商返回的元数据，未知值的默认配置可以在应用前修改。
新增供应商或修改协议、模型列表后，需再次应用 OpenCode 配置以更新模型列表；密钥和网络出口由路由实时读取。

## 路径与恢复

- Claude Code：`CLAUDE_CONFIG_DIR/settings.json`，默认 `~/.claude/settings.json`。
- OpenCode：优先 `OPENCODE_CONFIG`；否则使用 `$XDG_CONFIG_HOME/opencode`（默认 `~/.config/opencode`），选择已有的 `opencode.jsonc`、`opencode.json`、`config.json`。
- 隔离调试：设置 `AGENT_HARBOR_CLIENT_HOME` 后，这两个客户端都从指定目录构造配置路径，不读取正式客户端的目录覆盖变量。
- 备份和管理记录：AgentHarbor 数据目录下的 `client-config`。Windows 使用现有 DPAPI 加密存储，Unix 文件只允许当前用户读取。

配置采用字段级合并和原子替换。预览版本会校验配置是否在预览后变更。恢复只撤回管理字段；没有其他改动时恢复原始文件全文（含注释），原先不存在的文件会删除。JSONC 应用后规范化为 JSON，原始带注释文本保存在备份中。

如果外部工具修改了管理字段，会报告冲突，避免覆盖。项目级配置、环境配置及启动参数仍可能覆盖全局配置；页面显示的是写入状态，不代表已完成模型连通性测试。

## 模块边界

- `core/client_config/claude.rs`、`opencode.rs`：客户端专属字段生成。
- `core/client_config/mod.rs`：路径发现、协议验证、预览、备份、合并和恢复。只接收应用数据目录、客户端路径和供应商数据，不依赖 Codex 账号或进程。
- `commands/client_config.rs` / `api.targets`：统一配置接口。
- `core/routing_runtime.rs`：共享路由生命周期。Codex 注入、Claude Code 配置、OpenCode 配置都可独立保持路由运行。
- `commands/usage.rs` / `api.usage`：客户端无关的中转用量接口。

供应商密钥只存储在 AgentHarbor；客户端写入本地路由凭据。三种协议均复用已有路由和用量观察器。Chat Completions 流式请求会请求 `include_usage`；上游没有返回用量时仍标记为未报告，不估算 Token 数。

本次扩展的是模型供应商配置能力。账号登录、会话归档、MCP 和 Skills 当前仍按现有 Codex 能力提供，不会把其他客户端请求错误地转到 Codex。

## 验证

`pnpm build`；`pnpm test:rust`；`pnpm tauri build --debug --no-bundle`。

测试使用临时客户端目录和本地模拟上游，覆盖配置保留、恢复、并发变更检测、协议匹配、从生成配置发送请求、SSE 用量记录，以及独立启停和重启后的路由恢复。不修改正式客户端配置，不调用付费上游。

参考：[Claude Code 网关](https://code.claude.com/docs/en/llm-gateway-connect)、[模型配置](https://code.claude.com/docs/en/model-config)、[OpenCode 供应商](https://opencode.ai/docs/providers/)、[配置](https://opencode.ai/docs/config/)。
