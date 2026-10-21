<p align="center">
  <img src="assets/app-icon.png" alt="AgentHarbor" width="128" height="128" />
</p>

<h1 align="center">AgentHarbor</h1>

<p align="center">
  <strong>编程智能体的桌面港湾：账号、中转、本地模型路由、会话与本地配置，一处管理。</strong>
</p>

<p align="center">
  <a href="./README.md">English</a> · 简体中文
</p>

---

## 这是什么

Codex、Claude Code、Claude Desktop、OpenCode、Pi、DeepSeek Harness 和 Cursor 各自把账号、模型、会话、MCP 和指令存在不同的地方。AgentHarbor 把它们集中到一个桌面应用里管理：切换账号、接入第三方中转、组合模型、清理会话，都不用再手改配置文件。所有对客户端配置的修改都会先备份，可以随时恢复。

## 功能

### 模型与路由
- **中转管理**：管理多个模型供应商，支持 Responses、Chat Completions、Anthropic Messages 三种协议，可测试连通性、查看余额、导入导出，并提供模型能力目录（上下文、价格等）
- **模型注入**：一键把中转模型和账号模型接入 Codex、Claude Code、Claude Desktop、OpenCode、Pi、DeepSeek Harness 和 Cursor，可预览、应用、恢复
- **本地路由**：三种协议自动互相转换；支持模型聚合，按优先级失败切换
- **通用网关**：一个本地地址加上 API Key，任何支持自定义 Base URL 的 agent 都能用上全部模型
- **跨协议联网搜索**：各客户端自带的联网搜索在其他协议的模型上照样可用
- **Cursor 接入**：在 Cursor 的 Agent、Tab 补全和提交信息中使用自己配置的模型
- **一键启动**：改完配置后直接启动或重启对应客户端

### 账号
- **Codex 多账号**：ChatGPT 登录、额度查看、手动或按阈值自动切号
- **账号来源**：ChatGPT、WorkBuddy、GitHub Copilot、Gemini CLI、Antigravity、Kiro、Grok、Cursor、Qoder、Trae、MiniMax Code、ZCode、OpenCode Zen / Go、Command Code 等订阅账号登录后，可以作为模型供应商使用
- **每日签到**：支持签到奖励的账号可自动签到

### 会话与配置
- **用量统计**：汇总所有客户端的 Token 与费用，按客户端、渠道、模型、项目拆分
- **会话管理**：浏览、搜索、导出、删除 Codex 与 Claude Code 会话，找回丢失的线程
- **MCP 管理**：一份清单统一管理各 Agent 的 MCP 服务器，按 Agent 单独开关
- **Skills 管理**：每个 Skill 只存一份，按需分发到各 Agent
- **自定义指令**：管理 `AGENTS.md` / `CLAUDE.md`

### 生图工作台
- 文生图，或最多用 16 张参考图做图生图
- 本地图册，记录每张图的提示词和参数
- 提示词模版、服装 / 动作 / 视角等素材库和参考图库
- 用任意模型润色提示词

### 其他
- 仪表盘：路由状态、今日用量、最近会话一目了然
- 主题、语言、自定义壁纸、代理设置、开机自启、内存优化
- 托盘菜单快捷操作
- 应用内自动更新

## 支持平台

| 平台 | 说明 |
| --- | --- |
| Windows | x64，主要支持平台 |
| macOS | 12 及以上 |
| Linux | 部分功能可用 |

从 [Releases](https://github.com/liangyu-love/AgentHarbor/releases/latest) 下载安装包。

## 从源码运行

需要 Node.js、pnpm、Rust 和 [Tauri 前置依赖](https://v2.tauri.app/start/prerequisites/)。

```bash
pnpm install --frozen-lockfile
pnpm tauri dev
```

欢迎提交 Issue 和 PR，动手前请先阅读 [CONTRIBUTING.md](CONTRIBUTING.md)。

## 许可证

[Apache License 2.0](LICENSE)。第三方组件和数据（包括 models.dev 快照、移植自 [cursor-byok](https://github.com/leookun/cursor-byok) 的代码）见 [LICENSES/THIRD_PARTY.md](LICENSES/THIRD_PARTY.md)。

## 免责声明

AgentHarbor 是独立工具，与 OpenAI、Anthropic 及其他厂商没有隶属、背书或赞助关系。在厂商官方应用之外使用订阅账号可能违反其服务条款；使用第三方中转服务和账号来源的风险由你自行承担。
