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

这不等于 Claude Desktop 使用 `/provider/v1/messages` 后自动获得搜索：Desktop 的 WebSearch 属于推理侧托管工具；本项目没有把它桥接到 Command 的独立搜索 REST API。Messages 原生透传取决于上游是否支持该托管工具；Messages 转换到 Chat/Responses 目前只支持普通 function tools，不会将原生 `web_search_*` 类型伪装成普通函数。

因此本次修复覆盖截图中确定存在的网页白名单拒绝，不能承诺它同时修复所有上游的原生搜索。Command 独立搜索桥接需要单独设计工具请求、结果、流式与计费契约。不要通过切换模型协议或删除工具字段来伪造支持。

## 验证边界

本次自动化覆盖直连与路由 profile 写入、出口字段预览、原有受限出口基线恢复和配置回滚。`pnpm test:rust` 通过 789 个 lib、11 个 Desktop 集成与 9 个本地工作流用例，21 个真机/辅助用例仍 ignored；Desktop 前端契约检查、余额界面回归与 `pnpm build` 通过。未修改真实配置，未使用真实推理密钥执行付费搜索；真实 Desktop 的网页访问和各 provider 的搜索结果仍需在重新应用后观察。

参考源码：`src-tauri/src/core/claude_desktop/profile.rs`、`transaction.rs`；参考项目 `cc-switch/src-tauri/src/claude_desktop_config.rs`、`dsh-commandcode-provider/src/web-search.ts`。
