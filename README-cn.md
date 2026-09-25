<p align="center">
  <img src="assets/app-icon-composed.png" alt="AgentHarbor" width="128" height="128" />
</p>

<h1 align="center">AgentHarbor</h1>

<p align="center">
  <strong>面向编程 Agent 的原生桌面工作台 —— 统一管理账号、路由、会话与本地配置。</strong>
</p>

<p align="center">
  <a href="./README.md">English</a> · 简体中文
</p>

---

## 当前重构版

AgentHarbor 已使用独立的应用标识、进程名、安装包名、单实例锁、本地数据目录、路由命名空间和默认端口，可以与本机已安装的 AiMaMi 并行运行。当前源码包含概览、中转管理、Codex 本地路由与原生接入、路由无账号模式、MCP、Skills、自定义指令、维护和设置等本地工作流。

开发启动：`pnpm install --frozen-lockfile` 后运行 `pnpm tauri dev`。Windows 需要 Rust MSVC、Visual Studio C++ Build Tools 和 WebView2。完整步骤、隔离测试方式及验证记录见 [修复版运行说明](docs/RUNNING.md)。

## 概述

Codex 的账号、会话、MCP、Skills、智能路由与中转配置分散在 `~/.codex` 下的多个文件里。多账号切换、额度耗尽、第三方模型接入与路由维护、会话清理和配置漂移，都会把日常操作变成手改 TOML / JSON / SQLite。

AgentHarbor 基于 **Tauri 2 + React + Rust**。Codex 是首个已接入的 Agent，产品身份与工具专属模块保持分离，后续可继续增加 Claude Code 等适配器。当前版本把 Codex 的高频操作 —— 含智能路由与中转管理 —— 收敛到一个桌面应用里，减少手工修改本地文件带来的风险。

---

## 核心能力

| 模块 | 当前源码提供的能力 |
| --- | --- |
| **账号管理** | 通过 Codex 登录添加账号、查看额度、手动切换和移除已保存账号 |
| **Codex 接入** | 独立选择本地路由 Router 或原生直连 Native，与模型协议和网络设置分开 |
| **中转管理** | 供应商默认协议与逐模型协议覆盖、连通性测试、导入导出和诊断 |
| **会话管理** | 列出、搜索、筛选 Codex 会话，将选中会话导出为 Markdown 或 JSONL |
| **MCP / Skills** | 图形化管理 MCP 条目与 Skills 生命周期，支持备份恢复 |
| **客户端配置** | 预览、应用和恢复 Claude Code / OpenCode 经本地路由使用的供应商配置 |
| **图片工作台** | 使用已配置的供应商生成或编辑图片，并保存到本地图册 |
| **自定义指令** | 仅管理 `~/.codex/AGENTS.md` 中的 AgentHarbor 受控区块，支持预览与回滚 |
| **系统维护** | 本地诊断、冗余备份清理、重建 registry 和重启 Codex |
| **设置与运行时** | 主题、语言、额度刷新、网络代理、可选更新检查和基础托盘菜单 |

**当前源码尚未实现：** 自动切换账号、账号导入导出、会话批量删除、插件管理。禁用的控件或历史截图不代表这些功能已经可用。维护页的清理仅处理冗余本地备份与过期索引，不会删除会话。额度热力图与 macOS 刘海额度展示也未提供；更新检查需要另行配置带签名的发布服务。

**模型协议：** 每个模型可分别使用 Responses、Chat Completions 或 Anthropic Messages。供应商协议只是未设置覆盖时的继承默认值，不会按协议筛掉模型；刷新列表会保留覆盖设置。上游协议、Codex 接入方式和网络出口相互独立。

**Codex 接入方式：** Router 经 AgentHarbor 本地路由转发，为 Codex 的 Responses 接口转换 Chat Completions / Anthropic Messages，可同时使用官方和已启用的中转模型，需要保持 AgentHarbor 运行。Native 直接连接当前供应商：默认模型必须使用 Responses，目录只包含该供应商已选中的 Responses 模型，并使用上游真实模型 ID。选择不兼容时会报错，不会静默更换模型或供应商；Chat / Anthropic 模型应使用 Router。

**原生鉴权：** AgentHarbor 将供应商 API Key 存入 Codex 主目录 `.env` 中的专属随机变量，再通过 `env_key` 引用。`.env` 及其备份是本地秘密文件，不是加密凭据库，应保护访问权限并避免提交到版本控制。密钥不写入 `auth.json`，也不出现在配置预览中。Native 设置 `requires_openai_auth=false`，因此 Codex 可能不显示 ChatGPT 账号，但原有 `auth.json` 会保留。如需官方与中转模型并存及官方账号显示，请切回 Router、关闭无账号模式并重启 Codex。

切换接入方式、修改模型配置或原生网络出口后，请重启 Codex；修改代理后应使用 AgentHarbor 的重启入口，让 Codex 进程加载相应设置。Native 模型请求不要求本地服务持续运行，但仍经本地路由的 Claude Code / OpenCode 配置需要该服务。

<p align="center">
  <img src="assets/console.png" alt="AgentHarbor" width="1200" height="812" />
</p>
## 平台支持

| 平台 | 说明 |
| --- | --- |
| macOS | Universal（Apple Silicon + Intel），macOS 12+ |
| Windows | x64，NSIS 安装包 |
| Linux | 部分能力为尽力支持 |

---

## 技术栈

Tauri 2 · React 18 · TypeScript · Vite 6 · Tailwind CSS · shadcn/ui · Rust

---

## 快速开始

**环境要求：** Node.js · pnpm · Rust · [Tauri 系统依赖](https://v2.tauri.app/start/prerequisites/)

```bash
cd AgentHarbor
pnpm install
pnpm tauri dev
```

```bash
pnpm build                                      # 前端构建检查
cargo check --manifest-path src-tauri/Cargo.toml  # Rust 检查
pnpm tauri build                                # 生产构建
```

---

## 项目结构

```text
src/           React 前端
src-tauri/     Tauri 壳与 Rust 后端
src/locales/   国际化（中 / 英）
scripts/       构建与发布脚本
assets/        品牌与文档素材
```

---

## 架构

```text
React UI ── invoke() ──▶ Tauri commands ──▶ core/
                                              ├── ~/.codex          (Codex 原生)
                                              └── ~/.codex/agentharbor/ (AgentHarbor 数据)
                         platform/            macOS / Windows 差异实现
```

---

## 参与贡献

欢迎提交 Issue 与 Pull Request。请先阅读 [CONTRIBUTING.md](CONTRIBUTING.md)，了解开发环境、提交前检查以及本地 Codex 数据安全注意事项。较大改动建议先开 Issue 讨论方案。

---

## 许可证

[Apache License 2.0](LICENSE)

---

## 免责声明

AgentHarbor 是独立的 Codex 本地工作流工具，与 OpenAI 无隶属、背书或赞助关系。使用第三方中转服务请自行评估风险并遵守相应条款。
