# AiMaMi 项目摸底与优化起点

> 本文记录修复前的静态调查结果。后续已补齐构建依赖与本地配置管理链路；当前运行方法、功能边界和验证结果以 [修复版运行说明](RUNNING.md) 为准。

记录日期：2026-09-18。分析对象：公开仓库 `borawong/AiMaMi` 的 `main`，提交 `add37271e29ba81ee17f15f444a24925af50bb87`。

本次完成源码阅读、接口对照和 GitHub 分支／发布信息核对；没有安装项目依赖、执行构建或启动桌面程序。因此以下区分静态确认的问题与后续需要运行验证的事项。

## 核心判断

AiMaMi 的定位是 Codex 本地管理工具：用桌面界面管理原本分散在本地文件中的配置、账号及会话。现有 UI 和部分配置管理逻辑可以继续使用，但当前公开源码存在缺失模块和接口断点，尚不能作为已经可构建的完整产品基线。

README 中的账号管理、自动切换、智能路由、中转服务、会话管理，不应直接等同于当前源码已实现、可使用的能力。

GitHub 当前只返回 `main` 一个分支；最新发布为 `v1.2.6`，包含 Windows 和 macOS 安装包。该标签的 `src-tauri/src/core` 同样只有 `custom_instructions`、`mcp`、`models`、`repository`、`skills` 及 `mod.rs`，未找到下文提到的缺失模块。公开源码的三个版本配置仍为 `1.0.0`。这些信息说明发布物与源码的对应关系需要进一步核实，不能据此推断安装包本身也有同样问题。

## 架构与模块入口

```text
React 页面与组件
  → src/lib/api.ts：前端接口封装
  → src-tauri/src/lib.rs：Tauri 命令注册
  → src-tauri/src/commands/：参数和调用边界
  → src-tauri/src/core/：配置读写、备份、业务逻辑
  → src-tauri/src/platform/：路径、进程、代理和系统差异
  → CODEX_HOME，默认 ~/.codex
```

| 层次 | 当前实现 |
| --- | --- |
| 桌面运行环境 | Tauri 2 + Rust |
| 前端 | React 18、TypeScript、Vite 6 |
| UI | Tailwind CSS 3、Radix / shadcn 风格组件、Lucide 图标 |
| 数据状态 | TanStack Query；页面切换用本地状态管理 |
| 国际化 | i18next，中英文资源 |
| 数据存储 | Codex 原生文件与 `codexmate` 应用目录；部分账号数据位于 `accounts` |
| 授权 | 仓库声明 Apache-2.0；再分发时还应保留相关许可证文件 |

优先阅读：

- `src/main-app.tsx`：页面入口、懒加载、主题和全局更新提示。
- `src/lib/api.ts`、`src/types/index.ts`：前端调用与数据类型。
- `src/components/mcp/`：MCP 列表和编辑弹窗。
- `src/components/skills/`：Skills 导入、删除和备份恢复。
- `src/components/custom-instructions/`：指令模板、预览、应用和历史记录。
- `src-tauri/src/core/mcp.rs`：MCP 配置解析与局部文本替换。
- `src-tauri/src/core/skills.rs`：Skills 扫描、复制和备份。
- `src-tauri/src/core/custom_instructions.rs`：AGENTS.md 受控区块与历史快照。
- `src-tauri/src/platform/paths.rs`：数据目录约定，支持通过环境变量解析 Codex 主目录。

## 静态确认的问题

### P0：源码完整性与构建断点

1. **缺少 Rust 核心模块。** `core/repository.rs:1` 起引用 `auth`、`bootstrap_cache`、`quota_store`，其他代码还引用 `api_client`；对应文件不存在，`core/mod.rs` 也未声明它们。需要恢复来源明确的实现，或围绕明确的产品范围解耦依赖。
2. **入口库名不一致。** `Cargo.toml:13` 声明 `aimami_lib`，`src/main.rs:5` 和 `:12` 使用 `codexmate_lib`；还调用了当前 `lib.rs` 中没有实现的 `run_daemon_once_cli`。
3. **TypeScript 调用签名不一致。** `mcp-page.tsx:349` 传入一个对象，但 `api.ts:87` 定义为两个参数；指令预览、应用和回滚页面也与 API 封装的参数数量不一致。这些需要在依赖安装后通过类型检查取得完整错误清单。

### P1：操作链路与默认页面

1. **首页为空。** `main-app.tsx:69` 默认路由为 `overview`，`:106` 对应分支直接返回 `null`；README 仪表盘截图无法由该页面实现复现。
2. **IPC 参数不一致。** MCP 封装发送 `{ name, config }`，Rust 命令要求 `name`、`transport`、`enabled` 等顶层参数。Skills 导入发送 `sourcePath` 而后端要求 `path`；删除、恢复、删除备份发送 `name` 而后端要求 `id`；指令回滚需要 `historyId`，封装没有发送。
3. **遗留接口。** `api.ts` 封装 `load_snapshot`，当前命令注册表没有该命令，不能直接用于补建仪表盘。
4. **功能边界不清。** 当前路由只有概览、指令、MCP、Skills、维护、设置，没有账号、会话和路由管理页面。保留的账号数据结构和系统接口不代表这些功能完整可用。

接口判断参考 Tauri 官方文档：JavaScript `invoke` 的参数对象需要与 Rust 命令参数对应，默认使用 camelCase 键名，命令还必须注册到 `generate_handler!`。已通过 Context7 核对。

### P1：配置写入与回滚

- 指令应用和清除会保存历史；但 `core/custom_instructions.rs:170` 起的回滚逻辑恢复整个 `AGENTS.md` 快照。如果用户在快照之后编辑了受控区块之外的内容，也会一起被覆盖。建议回滚只影响受控区块，并检测外部修改。
- 指令写入直接使用 `std::fs::write`；MCP 写入已有临时文件再重命名的实现。可以统一写入策略，补充失败恢复和并发修改检查。
- Skills 替换与恢复已有备份，但流程先删除现有目录再复制。建议先完成暂存和校验，再替换，并验证复制失败时可恢复。
- Skills 备份 ID 和元数据路径进入目录拼接与删除流程，后续应测试路径边界、异常元数据和符号链接行为。

### P2：工程与体验

- 仓库未提交前端锁文件，没有发现 `.github` CI 配置，`package.json` 没有测试脚本。Rust 源码有少量内嵌单元测试，不能将此描述为完全没有测试。
- 页面已有懒加载、预热、查询缓存、主题和国际化，值得保留。性能问题尚未测量，暂不把框架升级或大规模重构列为首要事项。
- MCP 和 Skills 页面需要更清楚地区分加载中、读取失败、真正没有数据，并为保存失败提供可操作反馈。
- Tauri 配置默认打包目标为 `app`，更新插件配置为空。Windows 安装包与更新流程需要单独验证，不能假设 README 的命令已覆盖发布链路。

## 建议实施顺序

1. **确定可构建基线。** 找回缺失源码，或明确首版只覆盖 MCP、Skills、指令管理。修复入口、接口和资源问题，完成前端构建与 Rust 检查。
2. **打通最小操作闭环。** 建立有真实状态的首页，验证 MCP 增删改、Skills 导入及恢复、指令预览和回滚。用临时测试目录验证文件变化。
3. **保障配置修改可恢复。** 统一写入与备份流程、局部回滚、外部修改检测和失败反馈。
4. **按用户最常用场景优化。** 调整导航、表单、搜索、批量操作和诊断信息；如重点是账号或路由，则在来源和需求明确后作为独立模块实现。
5. **建立持续验证。** 固定依赖，加入关键操作与接口测试，再验证 Windows 打包。依据实测决定是否继续做启动速度、内存或列表性能优化。

第一阶段的验收应是“能构建、能启动、核心按钮完成真实操作”，之后再衡量界面和效率改善。

## 本次工作边界与环境备注

- 源码已克隆到 `D:\workspace\AiMaMi`，业务代码没有修改。
- 本次没有启动应用或操作用户的真实 Codex 配置。
- 没有运行 `pnpm build`、`cargo check` 或应用测试；构建断点来自源码静态核对。
- 用户提供本机代理端口 `127.0.0.1:10808`，后续联网操作可使用；未修改系统或 Git 全局代理配置。

参考：

- 仓库：https://github.com/borawong/AiMaMi
- 发布：https://github.com/borawong/AiMaMi/releases/tag/v1.2.6
- Tauri 命令调用：https://v2.tauri.app/develop/calling-rust/
