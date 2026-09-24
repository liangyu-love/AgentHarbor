# 修复复审记录

日期：2026-09-21
基线审查提交：`b9a7d85`

## 结论

原审查中可直接修复的数据一致性、安全边界、协议建模、兼容接口和遗留代码问题已完成修正。最终复审未发现新增的 P0/P1 缺陷。

## 已完成

- 统一 Codex `config.toml` 的进程内读改写入口，增加备份、原子替换、UTF-8/TOML 校验、外部修改冲突检测及 Router/MCP 并发测试。
- 将 AgentHarbor 数据迁至系统应用数据目录；迁移使用临时目录完整复制后原子发布，旧目录保留。
- 引入 `TargetId`、按能力拆分的目标接口和通用目标/会话命令，保留旧命令兼容层。
- Provider Store 升级为显式 Responses、Chat Completions、Anthropic Messages 协议；按协议选择路径、认证头、连通测试和模型列表认证。
- 本地代理支持同协议的 `/v1/responses`、`/v1/chat/completions`、`/v1/messages`，替换本地认证凭据并透明传递流式响应。
- 修复旧路由迁移的提交顺序，避免配置清理失败后永久留下陈旧路由；未知旧协议会安全禁用。
- 启用生产 CSP，并为 Vite 开发服务器单独开放本机 HMR 连接；Capability 仅绑定主窗口。
- 准确区分 Windows DPAPI 与 macOS/Linux owner-only 文件存储语义。
- 删除不可达的旧 NewAPI 单连接、Hotspot 设置链和 Voice 残留；保留主窗口唤醒逻辑。
- 前端新增 `providers`、`routing`、`targets`、`conversations` 领域 API，旧前端/Tauri 接口继续兼容。

## 验证

- `pnpm build`
- `pnpm test:rust`：53 passed，1 个真实网络测试按设计 ignored；本地工作流 9 passed
- `node scripts/tauri.mjs build --debug --no-bundle`
- `cargo clippy --lib --tests`：成功；仍有 24 个既有风格类提示，无正确性错误
- `git diff --check`

## 保留边界

- 当前注册表仍只有 Codex 实现；Claude Code/OpenCode 的自动配置、进程与会话适配尚未实现。
- Chat/Anthropic 支持是同协议透明转发，不执行 Responses、Chat、Anthropic 之间的请求或流事件转换。
- `router.rs` 已拆出配置仓库、协议职责和目标骨架，但进一步模块化仍属于后续重构，而非本次缺陷修复。
