# AgentHarbor 运行说明

AgentHarbor 已从原始源码分离出独立的产品身份。应用名、进程、安装标识、单实例锁、本地数据目录和智能路由端口均与旧版隔离。

## 当前源码的功能范围

- 概览：读取真实的 MCP、Skills、自定义指令状态和本地目录，展示读取错误。
- 账号：通过 Codex 登录添加账号、查看和刷新额度、手动切换、移除已保存账号。
- 会话：列出、搜索、筛选 Codex 会话，选择后导出为 Markdown 或 JSONL。
- MCP：新增、编辑、启停、删除；支持 Windows 路径、多行参数、HTTP headers，保留其他服务器选项和非 MCP 配置。
- Skills：扫描、导入、替换、删除前备份、恢复和删除备份；拒绝越界路径和符号链接复制。
- 自定义指令：模板、编辑、预览、应用、清除、历史回滚；回滚保留受控区块之外的后续用户修改。
- 设置与维护：主题、语言、本地设置、代理保存与无凭据连通性测试、本地诊断和维护入口。
- 中转站：多个供应商的逐模型协议、上下文、连通性测试、余额和导入导出；独立选择 Codex Router / Native 接入，另有 Claude Code / OpenCode 配置预览、应用和恢复。
- 图片工作台：供应商图片生成与编辑、本地图册保存与管理。

自动切换账号、账号导入导出、会话批量删除及插件管理尚未实现。自动切换入口保持不可用；维护页的清理不包含会话删除。历史截图中的额度热力图、热力图颜色设置和 macOS 刘海额度展示不属于当前功能范围。

账号额度刷新会在应用内互斥写回，并核对请求前后的凭据与账号身份，丢弃登录、切换或删除后的迟到结果。这个锁不能约束外部 Codex 进程；若 Codex 恰好在最终核对与文件替换之间改写登录文件，仍存在跨进程竞态，因此尚不能保证两个程序同时进行账号写入时的原子一致性。

代理探测成功只表示网络可达，不代表账号鉴权或额度有效。本地路由默认监听 `127.0.0.1:43127`。Router 模式且无账号模式关闭时，已登录 Codex 使用独立的 `x-agentharbor-token` 本地凭据并保留官方认证；路由无账号模式及其他客户端可使用本地 Bearer Token。路由验证本地凭据后选择对应上游，供应商请求不会携带官方账号凭据。Native 的供应商密钥鉴权独立于路由无账号开关，详见下文。

网络出口分为系统代理、手动代理和直连。系统模式使用环境变量或平台静态系统代理；PAC 需要先探测并保存为手动代理。代理设置立即作用于 AgentHarbor 自身请求和路由模型转发；单个供应商选择直连时不使用全局代理。Codex 自身的登录、账号请求及 Native 模型请求由 Codex 进程发出，不能靠本地模型路由代为设置出口。修改全局代理或原生接入的网络出口后，请使用 AgentHarbor 的“重启 Codex”应用相应进程设置；已有 Codex 进程不会因保存设置而自动更新。接入方式与网络出口是两个独立选项。

图片生成结果中的下载链接沿用该供应商的有效网络出口，下载请求不携带供应商密钥。所有链接先检查 URL、公共 IP 和本机 DNS，拒绝跳转；直连时绑定已验证的 DNS 结果。HTTP / SOCKS5h 等代理可能自行重新解析目标域名，因此用户配置的代理必须可信，应用无法约束其远端 DNS 结果。本机 DNS 无法通过安全校验时仍会拒绝下载。

图册列表加载最长边 256 像素的 PNG 缩略图，小图不放大；PNG、JPEG、WebP、GIF 均可预览，动画显示首帧。原图仅用于详情或导出。预览文件上限为 20 MiB，单边不超过 8192 像素、总像素不超过 16 × 1024²，解码预算为 128 MiB，最多并行处理两张；超限或损坏文件会报错，不会完整加载到图册列表。

更新服务未配置公钥和端点，因此不加载更新插件，也不在启动时请求更新。后续发布时需要自行配置与签名。

## 更换应用头像

在“系统设置 → 外观 → 应用头像”点击“更换头像”，也可以直接点击左上角头像。支持 PNG、JPG、WebP，单张不超过 5 MB、任一边不超过 8192 像素。图片居中裁切为正方形并缩小到 256 × 256，设置页可查看预览，侧栏立即同步。

头像副本保存在当前 AgentHarbor 界面的本地存储中，不上传到网络；移动或删除原图不影响显示，正常退出和重启后保留。点击“恢复默认”可以回到内置头像。此设置修改应用内头像，安装包和系统托盘图标使用固定的打包图标；本次已按用户提供的图片同时更新默认头像、程序、安装包和托盘图标。清除应用的 WebView 数据会同时清除头像、主题等界面偏好。

## New API 中转站

### 终端代理

“设置 → 网络与代理 → 终端代理”可以为当前 Windows 用户保存 HTTP/HTTPS 代理。它写入 `HTTP_PROXY`、`HTTPS_PROXY`、`ALL_PROXY` 和 `NO_PROXY`，与 AgentHarbor 自身的 API 代理设置独立。Claude Code 不支持 SOCKS 代理，请填写代理软件的 HTTP 或混合端口，例如 `http://127.0.0.1:7890`。

启用后关闭并重新打开终端应用；已运行的 Windows Terminal、VS Code 等进程新建标签页可能继续沿用旧环境。设置中的“打开 PowerShell”会立即打开带有已保存代理的终端。本机 `localhost`、`127.0.0.1`、`::1` 始终加入直连名单，保留本地路由与登录回调。

“关闭并恢复”按字段恢复启用前的注册表原始类型与内容；其他程序后来修改过的字段会保留。操作前保存恢复日志，部分失败可再次恢复。修改过程中不启动 shell 辅助程序；只有主动点击“打开 PowerShell”才显示终端窗口。该持久终端代理设置当前仅支持 Windows。

“测试连接”仅验证 Anthropic/OpenAI API 地址的无凭据网络连通性。它不验证模型权限，也不保证 Claude Code WebFetch 能访问所有网站。

### 同 URL 卡片堆叠

站点 URL 相同的供应商显示为一个卡片堆叠，忽略域名大小写、默认端口和末尾斜杠；不同路径仍为不同组。最多显示五层，每层位移 7px、旋转 1.7°。点击卡片空白区域或左右箭头翻页；聚焦堆叠后支持方向键、Enter 和空格。修改、查询余额、开关和菜单操作只针对当前显示的供应商。

展开后可以直接选择或操作各成员；每个成员的配置、凭据和余额仍独立。堆叠不自动轮播，并尊重系统的减少动画设置。

侧栏“中转站”支持管理多个供应商。为供应商填写名称、站点地址、模型 API Key、默认协议和默认模型；需要查询 New API 账户余额时再填写数字用户 ID 与用户访问令牌。两种令牌用途不同：

- 用户访问令牌：站点个人设置／安全设置生成。通过 `Authorization: Bearer …` 和 `New-Api-User` 查询 `/api/user/self`。
- 模型 API Key：站点“令牌”页生成的 `sk-` 密钥，通过 `/v1/models` 拉取该密钥可见的模型。

余额使用站点 `/api/status` 返回的 `quota_per_unit` 和显示单位换算；状态接口不可用或规则缺失时显示原始 quota，不假定固定汇率。结果包含累计使用额度、请求数和查询时间。刷新失败保留旧结果并标记，不将失败当作余额为零。请求沿用“系统设置”的代理，禁止自动跟随跳转以免凭据发往其他地址。

拉取模型后可选择默认模型、设置上下文长度与自动压缩阈值。每个模型的上游协议可选“跟随默认”、Responses、Chat Completions 或 Anthropic Messages；未设置或为 `null` 时继承供应商默认协议。默认协议不会筛掉其他协议的模型，同一站点可以维护混合协议目录。刷新列表会保留模型的协议覆盖、勾选状态及手动设置；默认模型、数量、连通性测试和客户端兼容性按模型的实际协议处理。模型元数据取决于供应商，列表中出现模型不代表已经通过请求测试。

“Codex 接入”的方式独立于上游协议和网络出口：

| 接入方式 | 模型与请求路径 | 运行要求 |
| --- | --- | --- |
| Router（本地路由） | Codex 连接本地 Responses 网关；网关按逐模型协议转发，并对 Chat Completions / Anthropic Messages 做协议转换。关闭无账号模式时可保留官方模型和官方账号显示。 | 使用路由模型期间保持 AgentHarbor 运行。 |
| Native（原生提供方） | Codex 直接连接当前供应商，默认模型必须使用 Responses；目录仅包含当前供应商已选中的 Responses 模型，使用上游真实模型 ID，不使用本地路由别名。 | Codex 模型请求无需本地服务持续运行；其他使用本地路由的客户端仍需要它。 |

Native 不会为不兼容的默认模型自动换成其他模型或供应商，也不会把 Chat / Anthropic 直接当作 Responses 使用；请明确选择 Responses 模型或改用 Router。保存供应商、选择接入方式和启用接入是不同操作。启用后按所选方式写入当前 Codex `config.toml`，关闭接入时只撤回受管理字段、恢复此前配置。模式切换或模型配置修改后需要重启 Codex。

Native 使用供应商 API Key 鉴权：密钥写入 Codex 主目录 `.env` 中的 AgentHarbor 专属随机变量，`config.toml` 通过 `env_key` 引用变量名，不写入明文密钥。`.env` 及其备份属于本地秘密文件，应保护文件权限、避免共享或提交到版本控制；这里不提供加密凭据库的保证。已有 `.env` 中无关条目会保留，退出 Native 后清理不再使用且仍由 AgentHarbor 管理的条目。真实密钥不显示在配置预览中，也不写入 `auth.json`。

Native 明确设置 `requires_openai_auth=false`，由供应商密钥而非官方账号令牌为第三方请求鉴权。原有 `auth.json` 保留，Codex 可能因此不显示 ChatGPT 账号，并不表示被登出。需要官方模型、中转模型并存及官方账号显示时，请切回 Router、关闭无账号模式并重启 Codex。路由无账号开关不改变 Native 的鉴权方式。

Claude Code / OpenCode 仍使用各自的预览、应用和恢复入口：当前这些客户端配置写入本地路由地址与本地凭据，不会自动重启客户端；这不适用于上面直接使用供应商密钥的 Codex Native 模式。客户端按所选模型的有效协议判断兼容性，详见 [多客户端配置](client-configuration.md)。

供应商连接本身保存在 `agentharbor/newapi-providers.dat`，旧版单连接数据会迁移。Windows 对这个 AgentHarbor 数据文件使用当前用户的 DPAPI 加密，其他平台使用仅文件所有者可读的本地文件（0600，不承诺加密）；不要将这一说明误用于 Native 的 Codex `.env`。前端不读取已保存的明文凭据；编辑时留空保留原值，更换站点时不会沿用旧站凭据。导出供应商连接用于迁移时也应妥善保管。

接口与配置参考：[New API 认证](https://github.com/QuantumNous/new-api-docs-v1/blob/main/content/docs/en/api/management/auth.mdx)、[Codex 配置参考](https://learn.chatgpt.com/docs/config-file/config-reference)、[Codex 其他提供方鉴权](https://learn.chatgpt.com/docs/auth#alternative-model-providers)。

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

前端回归测试使用真实页面组件和模拟接口，覆盖会话滚动、扫描警告、图片连接选择、过期模型请求、图册缩略图缓存、逐模型协议继承与刷新保留、Codex 接入方式和模型兼容性；测试期间禁止访问外部网络。先执行 `pnpm build`，再执行 `pnpm test:frontend`。测试需要可用的 Playwright 或 playwright-core 包及 Chromium；如果不在项目依赖搜索路径中，可通过 `PLAYWRIGHT_MODULE_PATH` 或 `--playwright-module=<包的绝对路径>` 指定已安装的包，通过 `PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH` 或 `--browser-executable=<浏览器绝对路径>` 指定浏览器。

`src-tauri/tests/codex_router_auth.rs` 另提供真实 Codex CLI 的隔离兼容性测试，默认跳过。将 `AGENTHARBOR_TEST_CODEX_BIN` 指向已安装的 Codex CLI 可执行文件后，运行 `cargo +stable-x86_64-pc-windows-msvc test --manifest-path src-tauri/Cargo.toml --test codex_router_auth real_codex --offline -- --ignored --nocapture`。保留 `real_codex` 过滤：这些测试只使用临时目录、合成账号凭据和本地模拟上游，不读取或修改真实登录配置；同文件另有需要显式授权的真实计费上游测试，详见 `MAGPIE_VALIDATION.md`。

## 历史验证记录（2026-09-19）

以下记录对应当时的代码和测试环境，不代表后续源码已经重新打包、部署或完成真实账号与上游验证。当前改动需要重新执行上面的构建与检查；单元测试和模拟上游通过也不能代替真实桌面验证。

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
