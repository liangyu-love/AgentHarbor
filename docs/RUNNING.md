# AgentHarbor 运行说明

AgentHarbor 已从原始源码分离出独立的产品身份。应用名、进程、安装标识、单实例锁、本地数据目录和智能路由端口均与旧版隔离。

## 已恢复的范围

- 概览：读取真实的 MCP、Skills、自定义指令状态和本地目录，展示读取错误。
- MCP：新增、编辑、启停、删除；支持 Windows 路径、多行参数、HTTP headers，保留其他服务器选项和非 MCP 配置。
- Skills：扫描、导入、替换、删除前备份、恢复和删除备份；拒绝越界路径和符号链接复制。
- 自定义指令：模板、编辑、预览、应用、清除、历史回滚；回滚保留受控区块之外的后续用户修改。
- 设置与维护：主题、语言、本地设置、代理保存与无凭据连通性测试、本地诊断和维护入口。
- 中转站：New API 账户余额、模型列表、上下文设置、Codex 配置预览与带备份的应用。

代理探测成功只表示网络可达，不代表账号鉴权或额度有效。智能路由默认监听 `127.0.0.1:43127`，只接受应用生成的本地 Bearer Token。

更新服务未配置公钥和端点，因此不加载更新插件，也不在启动时请求更新。后续发布时需要自行配置与签名。

## 更换应用头像

在“系统设置 → 外观 → 应用头像”点击“更换头像”，也可以直接点击左上角头像。支持 PNG、JPG、WebP，单张不超过 5 MB、任一边不超过 8192 像素。图片居中裁切为正方形并缩小到 256 × 256，设置页可查看预览，侧栏立即同步。

头像副本保存在当前 AgentHarbor 界面的本地存储中，不上传到网络；移动或删除原图不影响显示，正常退出和重启后保留。点击“恢复默认”可以回到内置头像。此设置修改应用内头像，安装包和系统托盘图标使用固定的打包图标；本次已按用户提供的图片同时更新默认头像、程序、安装包和托盘图标。清除应用的 WebView 数据会同时清除头像、主题等界面偏好。

## New API 中转站

侧栏“中转站”支持保存一个站点连接，填写站点地址、数字用户 ID、用户访问令牌和模型 API Key 后点击“保存设置”。两种令牌用途不同：

- 用户访问令牌：站点个人设置／安全设置生成。通过 `Authorization: Bearer …` 和 `New-Api-User` 查询 `/api/user/self`。
- 模型 API Key：站点“令牌”页生成的 `sk-` 密钥，通过 `/v1/models` 拉取该密钥可见的模型。

余额使用站点 `/api/status` 返回的 `quota_per_unit` 和显示单位换算；状态接口不可用或规则缺失时显示原始 quota，不假定固定汇率。结果包含累计使用额度、请求数和查询时间。刷新失败保留旧结果并标记，不将失败当作余额为零。请求沿用“系统设置”的代理，禁止自动跟随跳转以免凭据发往其他地址。

拉取模型后可选择默认模型、设置上下文长度与自动压缩阈值。未返回上下文元数据的模型明确显示“站点未提供”；留空时不写入覆盖值，由 Codex 决定默认值。选择新模型会清除上一个模型的长度覆盖值。Codex 配置使用 Responses 协议，模型出现在列表中并不保证它能用于 Codex。

“保存并预览配置”只保存 AgentHarbor 偏好；“应用到 Codex”经过界面确认后才修改当前数据目录下的 `config.toml`，并将原文件备份到 `agentharbor/config-backups/`。其他提供方、MCP 配置及注释保留；无效 TOML 或不支持的内联提供方表拒绝写入。不会修改 `auth.json`，也不会自动重启 Codex。

默认生成 `env_key = "AGENT_HARBOR_NEWAPI_KEY"`，需要在启动 Codex 的环境里设置该变量。勾选“将已保存的 API Key 明文写入 Codex 配置”可免去环境变量，但会使用 `experimental_bearer_token` 将密钥写进配置文件，后续备份也可能含有它。配置预览从不展示真实密钥。

AgentHarbor 连接保存在 `agentharbor/newapi-connection.dat`。Windows 使用当前用户的 DPAPI 加密，其他平台使用仅文件所有者可读的本地文件（0600，不承诺加密）。前端不读取已保存的明文凭据，密码框留空保留原值；更换站点时不会沿用旧站凭据。“移除连接”仅清除 AgentHarbor 的连接，已经应用的 Codex 配置保留。

接口与配置参考：[New API 认证](https://github.com/QuantumNous/new-api-docs-v1/blob/main/content/docs/en/api/management/auth.mdx)、[Codex 配置参考](https://developers.openai.com/codex/config-reference/)。

## Windows 开发环境

需要 Node.js（本机验证使用 Node 24）、pnpm 11.22.0、Rust stable MSVC、Visual Studio 2022 Build Tools 的“使用 C++ 的桌面开发”组件及 Windows SDK、WebView2 Runtime。

```powershell
pnpm install --frozen-lockfile
pnpm tauri dev
```

项目的 `scripts/tauri.mjs` 在 Windows 子进程中默认选择 `stable-x86_64-pc-windows-msvc`，不修改全局 Rust 默认工具链。若已显式设置 `RUSTUP_TOOLCHAIN`，则使用该值。前端依赖存储目录为项目内的 `.pnpm-store`。

AgentHarbor 使用 `dev.agentharbor.desktop` 和独立的单实例锁，可与旧应用并行运行。开发调试如需隔离 Codex 配置，可使用下面的环境变量。

`pnpm dev` 只启动网页预览；文件管理需要桌面程序的 Rust 命令，不能只开浏览器来验证。

如下载依赖需要本地代理，可在当前 PowerShell 会话设置：

```powershell
$env:HTTP_PROXY = 'http://127.0.0.1:10808'
$env:HTTPS_PROXY = 'http://127.0.0.1:10808'
```

## 隔离测试

```powershell
$env:AGENT_HARBOR_CODEX_HOME = Join-Path (Get-Location) '.runtime-test\codex'
New-Item -ItemType Directory -Force $env:AGENT_HARBOR_CODEX_HOME | Out-Null
pnpm tauri dev
```

`AGENT_HARBOR_CODEX_HOME` 只影响 AgentHarbor，优先于 `CODEX_HOME`，无需修改 Codex 的环境。隔离模式同时分开单实例锁、唤醒请求和 WebView 数据目录。退出当前会话或移除此变量即可恢复正常目录选择；未覆盖时读取已有 `CODEX_HOME`，否则使用用户目录下的 `.codex`。

测试过程中只使用临时 fixtures；未对真实 Codex 配置执行清理、账号同步、守护任务安装或重启。正式使用维护功能时，应先查看界面的操作说明。关闭窗口会收起到托盘，彻底退出请使用托盘菜单的 `Quit`。

## 构建与检查

```powershell
pnpm build
pnpm test:rust
pnpm tauri build --debug --no-bundle
pnpm tauri build
```

- 调试可执行文件：`src-tauri/target/debug/AgentHarbor.exe`。
- 发布可执行文件：`src-tauri/target/release/AgentHarbor.exe`。
- Windows 安装包：`src-tauri/target/release/bundle/nsis/`。
- 打包命令自动合并 `tauri.windows.conf.json`，生成中英文 NSIS 安装包。

Rust 测试覆盖 Skills 导入替换备份恢复、路径边界、指令回滚和异常标记、MCP CRUD/Windows 参数/HTTP headers/未知字段保留、无效 TOML 拒绝写入、本地身份解析、代理参数和设置持久化。Mac 的系统集成功能尚未在本次 Windows 环境验证。

## 本次验证记录（2026-09-19）

- `pnpm install --frozen-lockfile`：通过。
- `pnpm build`：TypeScript 检查和 Vite 生产构建通过。
- `pnpm test:rust`：23 项通过（14 个单元测试、9 个本地工作流测试）；真实站点测试默认跳过，已另行显式运行通过。
- `pnpm tauri build --debug --no-bundle`：Windows MSVC 桌面构建通过。
- `pnpm tauri build`：发布程序及中英文 NSIS 安装包生成成功。
- 真实桌面窗口：首页、MCP 列表与新增保存、Skills 扫描、自定义指令预览与应用、维护诊断和设置页均已检查；MCP 和指令写入结果也从测试文件核对。
- 最终发布程序再次启动成功，正确读取此前保存的 MCP 和指令；`http://127.0.0.1:10808` 代理连通性测试通过，代理保存结果已核对；手动检查更新正常提示未配置服务。
- 头像功能：在隔离桌面程序中实际选择 PNG 图片，确认侧栏与设置页同步更新；重启后仍显示已保存头像；“恢复默认”还原内置头像成功。图片格式、大小和读取失败均有校验与错误提示，异常文件分支尚未通过桌面文件选择器逐项实测。
- New API：真实站点余额和模型查询通过，返回 9 个可用模型；桌面窗口核对余额、查询时间及模型列表。`gpt-5.6-luna` 的最小流式 Responses 请求返回 HTTP 200 和 `response.completed`。未对每个模型或模型的最大上下文做压力测试。
- New API 回归测试覆盖站点换算、缺失元数据、凭据保存与跨站隔离、重定向拒绝、用户 ID 校验、上下文参数、配置预览与新文件创建、配置备份与未知配置保留。真实凭据未写入源码、测试断言或日志。
- 安装包尚未执行安装，以免覆盖本机已安装版本。现有代码仍有未使用函数等编译警告，不影响上述构建与测试。

内联表形式的 MCP 配置暂不支持直接编辑，会明确报错并要求先展开为 TOML 表；复杂格式发生解析或验证错误时保留原文件。
