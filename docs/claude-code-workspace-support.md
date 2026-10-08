# Claude Code：会话、全局指令和 MCP 管理

线程页、自定义指令页和 MCP 控制台都可以选择 Codex 或 Claude Code。每个页面记住自己的来源，两端的配置、草稿、预设和查询缓存分别管理。

## 会话删除

Claude Code 会话支持单条和批量删除；详情、普通列表和虚拟列表共用同一入口。确认后将主日志及同名 sidecar 目录移入项目根目录中的备份批次。整批先校验，移动失败则回滚，完成的批次最多保留 10 份。详情见 [本地会话说明](claude-code-sessions.md)。

## 全局指令

- Codex 继续使用原有 `~/.codex/AGENTS.md` 和历史目录。
- Claude Code 使用配置根目录下的 `CLAUDE.md`，默认是 `~/.claude/CLAUDE.md`。历史记录单独存放在应用数据目录 `custom-instructions/claude-code/history/`。
- 编辑、预览、应用、清除和历史回滚只操作 AgentHarbor 管理的指令内容，保留文件中其他文字。异常或重复标记会进入保护状态。
- 页面分为编辑和预设两个视图。预设可按开发、审查、文档分类或搜索，包含日常开发、测试回归、渐进重构等 8 项中英文模板。载入模板后可以修改，再预览应用。
- 可以把编辑内容保存为自己的预设，每个客户端最多 40 项。草稿和个人预设保存在本机应用 WebView 的存储中，独立于已经写入的指令文件。
- 预览展示当前指令、待应用指令和完整文件结果。提交检查当前指令是否仍与预览一致；写入前再次检查文件，发现外部修改则要求刷新。

模板依据 [Claude Code 官方最佳实践](https://code.claude.com/docs/en/best-practices) 中关于具体指令、项目约定与验证的建议编写，内容为项目自行整理。文件层级参考 [官方记忆说明](https://code.claude.com/docs/en/memory)。模板不会随下载自动应用。

## 用户级 MCP

Claude Code MCP 读写默认 `~/.claude.json` 的顶层 `mcpServers`；支持 stdio、HTTP 和 SSE，以及参数、环境变量和请求头。启停使用 `disabledMcpServers`，保留服务定义。Codex 继续使用原有 TOML 适配器。

页面管理用户级配置。项目的 `.mcp.json`、`projects` 内的本地配置和插件提供的服务不合并进此列表，也不被写入操作修改。开关表示配置是否启用，不表示服务正在运行或已经成功连接。

每次写入基于最新 JSON，只更新选中服务的相关字段，保留账号、项目、其他服务和未知字段。写入前检查外部变更，并生成加密的上一版本备份 `*.agentharbor-backup.dat`。损坏 JSON 会报错并保留原文件。保存、删除或切换状态期间锁定来源和冲突操作，命令面板也按选定客户端读取服务缓存。

路径规则与 cc-switch 的 Claude 配置解析一致：

| 配置根目录 | 全局指令 | 用户 MCP 文件 |
|---|---|---|
| 默认 `~/.claude` | `~/.claude/CLAUDE.md` | `~/.claude.json` |
| 自定义 `CLAUDE_CONFIG_DIR` | `<目录>/CLAUDE.md` | `<目录>/.claude.json` |
| 隔离 `AGENT_HARBOR_CLIENT_HOME` | `<隔离用户根>/.claude/CLAUDE.md` | `<隔离用户根>/.claude.json` |

自定义目录即使叫 `.claude`，也按完整路径处理，避免误写其父目录。协议与作用域参考 [Claude Code 官方 MCP 文档](https://code.claude.com/docs/en/mcp)；配置保留和会话 sidecar 处理也对照了本地 cc-switch 源码。

## 2026-10-08 验证

- 完整 Rust 回归：1098 个库测试、14 个 Desktop 集成测试、20 个本地工作流测试通过；16 个库测试和 11 个 Desktop live 测试按原配置忽略。
- `pnpm test:frontend` 和 `pnpm build` 通过。
- 新增前端回归覆盖双客户端隔离、模板、个人预设、草稿、文件预览、应用和清除、MCP 保存和启停、操作锁及命令面板缓存；Claude 会话回归覆盖删除确认、取消与来源隔离。
- 后端新增覆盖会话整批校验、sidecar 备份、保留上限、Windows 占用失败回滚，指令历史隔离、回滚保留用户文字和外部变更，MCP JSON 保留、异常输入与配置路径。
- 已核对桌面浅色和移动端深色截图。测试只使用临时文件和模拟 API，未删除真实会话，未修改真实客户端配置，未启动真实 MCP 服务。

日志保存在本机忽略目录 `.runtime-test/claude-support-*.log`。整库测试同时修正了一条旧 Desktop 断言：原生 `count_tokens` 请求与实际模型推理请求分别计数。
