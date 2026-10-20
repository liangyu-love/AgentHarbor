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

Codex、Claude Code、Claude Desktop、OpenCode、Pi、DeepSeek Harness 和 Cursor 各自把账号、模型设置、会话、MCP 和指令存放在不同的文件里。额度用完要切号、接入第三方中转、清理会话，往往都得手改 TOML、JSON 和 SQLite。

AgentHarbor 是基于 Tauri 2 + React + Rust 的桌面应用，替你完成这些事：
- 运行一个本地模型路由（默认 `127.0.0.1:43127`，可在系统设置中更改）。
- 对每个客户端的配置做可撤回的逐字段修改，改之前先备份。
- 在一个仪表盘上展示各客户端实际的连接状态。

Codex 是主要集成对象，其他客户端共用同一套中转与路由。

## 功能

| 模块 | 能做什么 |
| --- | --- |
| **仪表盘** | 各客户端的路由拓扑、今日 Token 概览、最近会话，以及可折叠的本地数据目录：列出各客户端的目录和关键文件 |
| **中转模型** | 管理多个供应商；逐模型协议（Responses、Chat Completions、Anthropic Messages）、连通性测试、New API 余额、导入导出；能力目录与逐中转的匹配预览（上下文、输出上限、价格）来自内置的 [models.dev](https://github.com/anomalyco/models.dev) 快照 |
| **模型注入** | 把中转模型和官方账号的模型接到下列客户端：预览、应用、恢复配置，每次都有备份；路由模式下中转、官方账号、聚合模型三类可以任意组合<br>• Codex：本地路由或原生直连，支持无账号模式和子代理默认配置<br>• Claude Code、Claude Desktop、OpenCode、Pi、DeepSeek Harness<br>• Cursor IDE 与 cursor-agent：Agent、Tab 补全和提交信息都能用这些模型，见下文<br>• 每个客户端面板都能一键启动或重启对应客户端，见下文 |
| **本地路由** | 三种协议互相转换<br>模型聚合：按优先级分层、失败切换、首字超时切换、实时成员状态<br>请求级路由诊断 |
| **通用网关** | 一个本地地址加上命名 key，任何能填 Base URL 和 API Key 的 agent 都能使用全部开启的模型，见下文 |
| **账号** | Codex：ChatGPT OAuth 登录、额度窗口、手动切号或按阈值自动切号（正在运行的 Codex 会把任务做完，原账号额度用尽时才重启）<br>账号来源：ChatGPT（Sign in with ChatGPT）、WorkBuddy、WorkBuddy AI、GitHub Copilot、Gemini CLI、Antigravity、Kiro、Grok、Cursor、Qoder、Qoder CN、Trae CN、Trae 国际版、MiniMax Code（国内 / 国际）、ZCode、OpenCode Zen、OpenCode Go、Command Code，登录后可以提供给模型注入使用；有签到奖励的来源支持每日自动签到，见下文 |
| **用量** | 所有客户端的 Token 与费用合成一本账：本地 Codex / Claude Code 会话与经路由的请求对齐去重，按客户端、渠道、模型和项目拆分；删除会话不影响已记录的用量 |
| **线程管理** | 浏览、搜索、导出、删除 Codex 与 Claude Code 会话；恢复 Codex 索引里丢失的线程 |
| **MCP / Skills / 自定义指令** | MCP 一份清单统一管理 Codex、Claude Code、OpenCode、Pi、Cursor、DeepSeek Harness 和 Claude Desktop，按 Agent 单独开关，并按各自的格式写入配置；可从模板、粘贴的 JSON / TOML 或一行命令添加，也能导入各 Agent 已有的服务器。Skills 每个只存一份，通过目录联接放进各 Agent 的 skills 目录（Windows 用 junction，无需管理员权限），带备份，可把各 Agent 已有的 Skill 纳入管理。以及 Codex 和 Claude Code 的 `AGENTS.md` / `CLAUDE.md` 中由 AgentHarbor 管理的区块 |
| **生图工作台** | 通过已配置的供应商生成或编辑图片，保存到本地图册，每张图都记下生成时的提示词和参数，可按瀑布流完整浏览或以可滑动的图集轮播观看；按自建分组管理自己的提示词模版，另有服装、动作、视角、睡衣居家、COS 角色等素材库（自带一套整理好的素材），都可导入导出 JSON，也能一键添加或找回自带的预置内容；可用中转、账号或聚合里的任意模型润色提示词，勾选素材分类后由模型挑选合适的素材写进去，图生图时还能附上底本让识图模型对照着写改图指令；画布可用同样的请求一键重新生成 |
| **设置与维护** | 主题、语言、自定义壁纸（深色模式自动保证对比度）、应用头像、API 代理、Windows 终端代理、开机自启（启动到托盘）、Windows 内存优化（释放工作集与待机缓存）、诊断、备份清理和重启 Codex。托盘菜单显示本地路由状态和今日用量，可直达各主要页面，并能切换 Cursor 注入、开机自启、检查更新和一键优化内存 |

尚未实现：账号导入导出、应用内插件管理、额度热力图和 macOS 刘海显示。更新来自 GitHub Releases，校验签名后在应用内安装。

## Codex 的接入方式

接入方式、上游协议和网络出口是三个独立的设置。

| 方式 | 请求如何流转 | 需要保持 AgentHarbor 运行吗？ |
| --- | --- | --- |
| **Router（本地路由）** | Codex 连接本地 Responses 网关，网关按各模型的协议转发，并转换 Chat Completions / Anthropic Messages。官方模型和中转模型可以出现在同一个菜单里；不开无账号模式时，官方账号保持登录。 | 使用中转模型期间需要 |
| **Native（原生直连）** | Codex 直接连接所选供应商。只列出该供应商的 Responses 模型，使用真实模型 ID；默认模型不兼容时直接报错，不会被偷偷替换。 | 不需要 |

- **配置写入**：启用接入只修改 `config.toml` 中由 AgentHarbor 管理的字段，关闭时恢复原来的内容。切换方式、改模型或改代理后要重启 Codex，应用里有重启按钮。
- **模型来源**：Codex 卡片上的「中转模型 / 官方账号 / 聚合模型」三个开关可以任意组合，菜单是开启项的并集，关掉的类别也不能再被调用。打开聚合时可以选一个聚合作为启动模型。ChatGPT 官方模型仍由无账号模式控制。
- **思考档位**：中转模型的档位来自 models.dev 的声明，只要查得到就会显示。几个来源说法不一时，取它们都列出的档位；只按思考预算（budget）声明的模型（如 Claude）显示 low / medium / high / xhigh，路由会换算成预算；速度版本（如 `-fast`）沿用基础模型的思考档位；发布方完全没有收录的模型，参考 models.dev 里其他提供商的同名条目。models.dev 每天自动更新，新模型的档位会自动带出，无需手工维护。默认档位是 medium，没有 medium 时是 high。只能开关思考的模型（如 MiMo）显示「开启 / 关闭」两档；有具体档位又能关闭思考的模型多一个「关闭」档。选「关闭」时，路由按上游发对应参数：`thinking.type`，OpenRouter 用 `reasoning.enabled`，DashScope 用 `enable_thinking`。在中转模型的「覆盖字段 → 思考等级」里手填的档位会原样显示。
- **中转模型继承官方行为**：中转模型会沿用同名官方模型的 Codex 客户端行为，所以多代理 V2 和 **Ultra** 档位经中转也能用。Ultra 只是 Codex 客户端里的设置，上游实际收到的仍是 `xhigh`、`max` 这样的具体档位。
- **跨协议的提示缓存**：Anthropic Messages 不带标记就完全不缓存，所以路由翻译成 Messages 的请求（来自 Codex 或任何 Chat 客户端）会带上 Claude Code 自己发送的两个缓存标记：系统提示词末尾，以及对话的最后一个内容块。这样每轮共享的前缀都从缓存读取，不再全价计费。Codex 的 `prompt_cache_key` 会转发给官方 Chat 接口和 Responses 上游。报给客户端的输入 token 包含缓存读写，Codex 的上下文用量因此保持准确。
- **Chat 中转的推理内容**：Codex 会回传推理项，但中转永远收不到它们。如果某个 Chat 中转用 `reasoning_content` 返回推理（DeepSeek 在带工具的对话里缺了它会直接拒绝），路由会按同一中转、同一模型把这段文本保存在内存里，并放回它所属的那一轮。用其他字段返回推理的中转不会收到额外内容。AgentHarbor 重启后，之前的轮次不再带推理内容。
- **Native 鉴权**：使用供应商密钥。密钥存放在 Codex 的 `.env` 里一个随机命名的 AgentHarbor 变量中，通过 `env_key` 引用；不会写进 `auth.json`，也不会出现在预览里。`.env` 及其备份不加密，请当作机密文件保管。

## 启动客户端

注入页的每个客户端面板都能在改完配置后直接启动对应客户端：

- **Claude Code、OpenCode、Pi、DeepSeek Harness**：在新终端（装了 Windows Terminal 就用它，否则用 PowerShell）里运行命令行，工作目录第一次选好后按客户端记住，按钮菜单里还有最近用过的目录。Harness 按注入的 profile 决定：`desktop` 启动或重启 Harness 桌面版，`web` 运行 `dsh web`，一次性的 `headless`、`sdk`、`acp` profile 则提供命令供复制。
- **Codex、Claude Desktop、Cursor IDE**：没在运行时按钮显示「启动」，运行中显示「重启」；Codex 没在运行时直接启动，不再弹重启确认。
- AgentHarbor 拉起的程序不再继承它的套接字，所以 Cursor 或 Codex 还开着时退出 AgentHarbor，路由端口也会被释放。若路由没在运行而端口确实被别的程序占着，注入页会显示占用的程序名和 PID，确认后结束它并重新启动路由。被 Windows 为 Hyper-V、WSL 或 Docker 保留的端口会单独说明。两种情况都能当场或在系统设置中换一个端口：经路由接入的客户端和 Codex 会改写到新端口，新端口不可用时什么都不改。
- **经路由的客户端会自动接回**：Claude Code、OpenCode、Pi、Harness 的本地路由配置在 AgentHarbor 退出时会撤回（路由随它一起停止），下次启动、路由占到端口后自动重新应用，和 Cursor 的重连方式一样。等待接回期间（比如端口被别的程序占着），面板会提示，「配置」弹窗也会带上这份配置；手动恢复原配置、解除管理，或点「不再接回」，就不会再自动接回。

## 组合模型来源

Claude Code、OpenCode、Pi、DeepSeek Harness 的注入弹窗在路由模式下也有这三个开关：关掉一类会取消它已选的中转或聚合，并从列表里隐藏。Claude Code 只选一个中转、或只用聚合时和以前一样；同时选多个中转，或中转加聚合时，它经本地路由使用和网关相同的 `短名/模型` id，`/model` 菜单里会列出全部所选模型。

## DeepSeek Harness 的接入方式

DeepSeek Harness（`dsh`、Harness 桌面版）的自定义模型走 pi-ai 路由，写在 profile 的 `cordis.patch.yml` 里。注入弹窗只写入你选的一个 profile（`desktop`、`web` 或 `$DSH_HOME/profiles` 下的其他 profile；`$DSH_HOME` 默认为 `~/.dsh`）：

- **路由**：每个中转按协议各建一条 `agentharbor-*` provider（一条路由只用 Chat Completions、Responses、Anthropic Messages 中的一种），所选聚合共用一条，都加进该 profile 自己的 `llm-pi-ai` 条目，和你的 provider 并列。patch 条目的 `config` 会整块替换组合出的配置，所以 AgentHarbor 不会另加第二个 `llm-pi-ai` 条目。
- **密钥**：路由里只写凭据引用名，值存进 Harness 的 `$DSH_HOME/.credentials.yaml`（本地路由用 `AGENTHARBOR_ROUTER_TOKEN`，直连时每个中转一个 `AGENTHARBOR_RELAY_*`）。账号来源和聚合只能经本地路由使用，由路由持有并刷新它们的凭据。headless profile（Harness 0.2.1 起）不带凭据存储，所以密钥改写进 `$DSH_HOME/.env`：只写 AgentHarbor 自己的几行，恢复时同样撤回。启动 `dsh` 时导出的同名环境变量、或工作目录 `.env` 里的同名变量会优先。
- **默认模型**：可选写入 `agent-default-model`，让新会话用所选模型。之后在 Harness 里换成别的模型属于正常使用，不算冲突；恢复时也会保留你的选择。
- 注释、`!!js` 表达式和其他条目都原样保留；每次修改都会重新解析，结果和预期不完全一致就拒绝写入。恢复时，文件没被别的改动过就逐字节还原，否则逐字段撤回；patch 被清空时写回 `[]`（Harness 遇到空文件会启动失败）。退出 AgentHarbor 时也会这样撤回路由模式的配置，下次启动再自动接回。
- Harness 会在下一次请求时重新读取 profile patch。`$DSH_HOME/cordis.patch.yml` 中同 id 的条目优先级高于 profile，只会提示，不会被修改。

## Cursor 的接入方式

Cursor 自己决定模型和 Agent 在哪里运行，没有可以填写的第三方地址。AgentHarbor 在本机提供一个接入端口，替 Cursor 的服务端处理选中 AgentHarbor 模型时的请求：

```text
Cursor IDE ── http.proxy ──┐
cursor-agent ── CURSOR_API_ENDPOINT ─┤──▶ AgentHarbor 接入端口（127.0.0.1）
                                     ├ 模型列表、账号/订阅检查、Agent、Tab、提交信息 → 本地处理，经本地路由调用模型
                                     └ 其余请求、Cursor 自带的模型 → 原样发往 Cursor
```

- **Cursor IDE**：在注入页的 Cursor 面板打开开关后，AgentHarbor 会在 Cursor 的 `settings.json` 里写入 `http.proxy` 等四项设置。编辑时保留原有的注释和格式；关闭开关或退出 AgentHarbor 时，只把自己写过的项改回原值，你之后自己改过的项不动。重启 Cursor 后生效，面板上有重启按钮。
- **本地证书**：Cursor 通过 HTTPS 访问服务器，所以本地代理需要一张 Cursor 信任的证书。证书在本机生成，带名称约束，只能用于 `cursor.sh`、`cursor.com` 及其子域名。在 Windows 上点「安装并信任」会装进当前用户的根证书库，系统会先弹窗确认，不需要管理员权限；也可以一键移除。macOS 和 Linux 上，面板会给出安装命令。只覆盖 `cursor.sh` 的旧版证书会被替换，需要重新信任。其他网站原样走隧道；如果 Cursor 原来配置了代理，隧道会继续经过它。
- **cursor-agent CLI**：打开开关后，AgentHarbor 会为当前用户设置环境变量 `CURSOR_API_ENDPOINT`（Windows），之后新开的终端里 `cursor-agent` 就会接入；也可以点「打开已接入的终端」。CLI 不需要证书。其他系统上，面板会给出启动命令。
- **账号**：未登录的 IDE 会在 `globalStorage/state.vscdb` 中获得本地身份，关闭 IDE 接入或退出 AgentHarbor 时还原；已有官方登录令牌不动。本地账号和经官方确认的免费账号会收到模拟的有效 Ultra 订阅与用量响应，让客户端可以选择注入模型；付费账号保留官方响应。模拟状态不提供 Cursor 官方额度，注入模型通过你配置的渠道调用，Cursor 自带模型仍发往官方。开关 IDE 接入后需重启 Cursor。
- **Agent**：选 AgentHarbor 提供的模型后，Agent 循环由 AgentHarbor 在本地运行。工具（终端、读写和编辑文件、搜索、MCP、子 Agent）由 Cursor 执行，结果再交回模型；会话检查点保存在 `cursor/conversations.db`。WebSearch 使用客户端接入底部共用的「联网搜索」里选的搜索账号，WebFetch 由 AgentHarbor 抓取网页。提示词按 Cursor 的工具约定编写，模型能正常调用 Cursor 的工具。
- **模型来源**：面板上同样有「中转模型 / 官方账号 / 聚合模型」三个开关，可以任意组合。思考档位与 models.dev 的声明一致；Cursor 账号来源的模型不会再提供给 Cursor。
- **Tab 补全和提交信息**：默认保持 Cursor 官方，不做任何改动。选择一个模型后，Tab 补全在本地生成，代码不会发给 Cursor 的 Tab 服务；Git 提交信息也可以选模型和语言（中文或英文）。

Agent 部分的协议适配移植自 [cursor-byok](https://github.com/leookun/cursor-byok)（MIT），见 `LICENSES/THIRD_PARTY.md`。

## 跨协议搜索

Codex 与 Claude Code / Claude Desktop 保留各自原来的 WebSearch 用法，由本地路由按目标模型的协议转换搜索声明、结果、引用和历史。

- **原生优先**：Codex 的 `web_search` 可以在 Anthropic Messages 模型上执行，Claude 的搜索可以在 Responses 模型上执行。Chat 模型只有确认支持的格式才转换，目前是 OpenAI Chat 搜索和 MiMo 搜索：官方地址会自动识别，中转需要在供应商的“高级设置 → 跨协议搜索方式”里指定。Codex 默认的 cached 搜索模式在其它供应商上按实时搜索执行。
- **检测搜索能力**：在供应商的高级设置里，可以用已保存的密钥发一到两个小的计费请求，看这个模型是否真的执行了搜索。如果某种 Chat 格式能用，会给出建议，但只修改草稿，不会自动保存。
- **兜底**：上游明确拒绝可选的搜索声明时，只移除搜索声明，让消息照常发出，这不算搜索成功；同一目标 30 分钟内不会再试原生搜索。也可以在注入页客户端接入底部的「联网搜索」里指定一个 Responses / Messages 搜索模型（默认关闭，所有经路由的客户端共用），由它执行搜索，原来的推理模型继续回答。DeepSeek 官方地址的供应商还可以改选 DeepSeek Search API（`/v1/tools/search`，用同一个密钥，2026 年 11 月 1 日起按次计费），这样 DeepSeek Flash 这类自身不执行搜索的模型也能搜索；把这类模型的跨协议搜索方式设为“始终使用选定搜索账号”即可。
- **诊断**：请求级路由诊断里会标出这次请求实际用的方式：原生适配、指定账号搜索、单独执行搜索，还是省略了搜索声明。

## 通用网关

不想逐个注入时，可以把模型注入页的「通用网关」直接给 agent 用：

- **地址**（默认路由端口下）：OpenAI 兼容（Responses、Chat Completions）用 `http://127.0.0.1:43127/gateway/v1`，Anthropic Messages 用 `http://127.0.0.1:43127/gateway`。`/gateway/v1/models` 列出这个 key 能用的全部模型。
- **key**：可以为每个 agent 建一个命名 key，单独复制、重新生成或删除。它和注入时写进客户端配置的内部令牌是分开的，互不通用。
- **模型名**：
  - 中转模型和账号来源是 `短名/模型`，例如 `deepseek/deepseek-v4-pro`、`workbuddy/glm-5.3`。聚合模型是 `前缀/公开名`，默认 `pool/gpt-5.5`，不会和中转里的同名模型混淆；共用前缀在网关面板里改，单个聚合也可以设自己的前缀。仍然直接写聚合公开名的请求照常可用。
  - 短名默认取自中转名称，可以在中转编辑里修改；之后给中转改名不会改变短名。
- **提供哪些模型**：点网关示意图右侧的「中转模型 / 官方账号 / 聚合模型」可以单独关闭，关掉的类别不出现在模型列表里，请求也会被拒绝。
- **协议转换**：和注入一样自动进行，比如用 Responses 调用只支持 Chat Completions 的中转。
- **保持运行**：打开「保持网关运行」后，即使没有任何注入，本地路由也会常驻。
- **连接配置**：可以按通用、Anthropic SDK、OpenCode 模板复制。预览里的 key 是隐藏的，点复制时才会写入真实 key。

## 账号来源

账号来源是自带模型的订阅账号。在账号页登录后，打开「提供给模型注入」，它就会作为一个特殊的供应商出现在模型注入页和聚合组里。

- **只共享需要的模型**：来源默认共享全部模型；在账号页点模型即可取消，取消的模型不提供给模型注入，也不进 Codex 的模型菜单（Codex 桌面版的菜单只显示前 100 个模型，超出时 Codex 卡片会列出被截掉的模型）。全部取消会同时停止共享。
- **只经本地路由使用**：每次请求由路由取当前账号的令牌，快过期时自动刷新。客户端配置里不会写入任何账号令牌，所以直连模式和 Codex 原生直连都不能选账号来源。
- **多种登录方式**：支持多种登录方式（浏览器、厂商自己的应用或 CLI、粘贴密钥）的来源，登录按钮会展开成菜单，第一项是常用方式。需要粘贴的密钥或地址在对话框里填写，和其他凭据一样加密保存。
- **每日签到**：WorkBuddy、MiniMax Code、Qoder、Qoder CN、Trae CN 每天签到送积分。在来源面板打开「每日自动签到」后，AgentHarbor 每个北京时间日为它的每个账号签到一次（启动两分钟后先签，之后每 30 分钟检查；失败稍后重试）；「立即签到」马上执行，账号卡片上的开关可以让某个账号不参与。默认关闭。厂商只给自家应用发签到奖励或要求验证码时，卡片会提示，不会强行签到。
- **提示词与客户端身份照 [magpie](https://github.com/yetone/magpie) 处理**：只回应自家客户端的厂商，AgentHarbor 会像 magpie 及其社区插件那样发送该客户端的请求头、系统提示词开头和请求形状，并改写厂商校验会拒绝的那几句话。AgentHarbor 不会替你同意任何厂商条款。在厂商应用之外使用订阅可能违反其条款；厂商明确说明有风险的来源，登录前账号页会先提示。
- **ChatGPT**：使用 OpenAI 官方提供给本地应用的 Sign in with ChatGPT，请求走标准的 `api.openai.com/v1/responses`，提示词是客户端自己的。它和 Codex 账号互相独立，需要单独登录；额度与 ChatGPT 共用，剩余量只能在 chatgpt.com 查看；个别套餐没有资格，登录时会提示。Team、Business、Enterprise 工作区要管理员先允许「Sign in with ChatGPT」应用，或者在登录页切到个人工作区。
- **WorkBuddy / WorkBuddy AI**：腾讯 CodeBuddy 套餐的国内版和国际版，使用自己的登录，不读取也不改动 WorkBuddy 桌面端的会话。Codex 和 Claude Code 提示词里会被 WorkBuddy 渠道校验拒绝的那几句话会在转发时自动改写，所以各客户端都能用；遇到没收录的措辞仍可能被拒，放进聚合组时会切到下一个成员。企业账号会带上企业的请求头并显示企业用量。
- **GitHub Copilot**：用 GitHub 设备码登录，账号页会显示要输入的代码。每个模型走 Copilot 提供它的接口：Claude 系列走 Anthropic Messages，新 GPT 走 Responses，其余走 Chat Completions。只列出账号现在能用的模型；需要先同意条款的模型请到 GitHub 的 Copilot 设置里启用，AgentHarbor 不会代为同意。卡片显示本月高级请求。
- **Gemini CLI / Antigravity**：分别用两个应用自己的 Google 登录，由 Google Code Assist 提供模型。Gemini CLI 的登录只对 Gemini Code Assist 标准版 / 企业版开放，需要 Google Cloud 项目（在账号卡片里填写）。Antigravity 个人账号也能用，但 Google 可能封禁在 Antigravity 之外使用的账号，所以登录前会先确认。Antigravity 会对 Claude Code 和 Claude Agent SDK 的系统提示词返回 429，请在 OpenCode、Pi 中使用，或放进聚合组。
- **Kiro**：在 Kiro 官网登录（Google、GitHub、AWS Builder ID 或 IAM Identity Center），每个账号独立保存，不影响 Kiro IDE 和 kiro-cli 的登录。卡片显示本月积分。
- **Grok**（SuperGrok、X Premium+）：用 xAI 的 Grok CLI（Grok Build）登录，需要先安装。AgentHarbor 在自己的 Grok 目录里运行 `grok login`，不影响你终端里 grok 的登录；令牌由 CLI 保存并续期。请求走 CLI 用的 Responses 接口。Codex 的分组工具和自由格式工具会转成 Grok 能接受的形式，回复里再还原；工具参数里 Grok 写成 `2500.0` 的整数会改回 `2500`。
- **Cursor**：用 Cursor CLI（`cursor-agent`）的登录，需要先安装；CLI 在本机只保留一个登录，移除这里的账号不会退出 CLI。每次请求是一次走 HTTP/2 的 Cursor Agent 运行：对话以隐私模式发送，客户端的工具作为 Cursor 可回调的工具提供，Cursor 自己的命令（终端、改文件）AgentHarbor 一律不执行。
- **Qoder / Qoder CN**：照 Qoder 桌面客户端的方式登录（设备码；qoder.com 可用个人访问令牌；qoder.cn 可填企业 VPC 地址），模型走客户端自己的对话接口。模型写在正文里的工具调用会被提取成真正的工具调用。Qoder 的每日积分需要本机装有 Qoder 桌面版或 Qoder CLI（用它的 runtime-info 读取设备身份），或者设置 `QODER_RUNTIME_INFO` 指向它；Qoder CN 不需要。
- **Trae CN / Trae 国际版**（实验性）：照 IDE 的方式在浏览器登录；登录时为本机生成设备（ID 与 P-256 密钥），令牌绑定到这个设备续期。Trae 的接口只收文本，工具会写进系统提示词，模型写出的调用（标签、GLM 模板、DeepSeek DSML）会被提取成真正的工具调用。国际版没有签到。
- **MiniMax Code**（国内 / 国际）：用 MiniMax Code 自己的设备码登录，不读取也不改动 `~/.minimax`，模型走 Anthropic Messages。MiniMax 的积分只供 MiniMax Code 使用，在其他工具里用可能被限制账号，所以登录前会先确认。
- **ZCode**：可用 Z.ai 或智谱 BigModel 登录、导入 ZCode 应用的登录，或填密钥。使用账号的 GLM Coding Plan、团队席位或免费的 Start Plan（带 ZCode 自己的提示词和请求头）；赠送套餐的模型以 `-Trial` 列出。
- **OpenCode Zen / OpenCode Go**：导入 `opencode auth login` 保存的密钥（只读取 auth.json），或粘贴密钥；每个模型走 OpenCode 文档写明的接口。Zen 的免费模型不需要账号：请求按 OpenCode 1.19 的样子发送，始终流式，并以 OpenCode 的核心工具作为替身。Gemini 模型走 Google 自己的接口，不提供。
- **Command Code**：可在浏览器登录、读取 CLI 的 `~/.commandcode/auth.json`，或填密钥。Claude 走 Anthropic Messages，其余按 Command Code 给出的接口；Go 套餐没有 Provider API，会像 CLI 那样走 `/alpha/generate`。
- Google、Kiro、Cursor、Qoder、Trae 和 Command Code Go 的接口在路由内部经 Chat Completions 转换，所以各种客户端协议都能照常用上它们。
- **Claude Desktop** 暂不支持账号来源。

## 数据与安全

- **Codex 主目录**：依次取 `AGENT_HARBOR_CODEX_HOME`、`CODEX_HOME`，都没有时用 `~/.codex`。
- **AgentHarbor 数据目录**：存放中转、路由状态、用量账本和缓存。位于系统本地数据目录下的 `AgentHarbor`（例如 `%LOCALAPPDATA%\AgentHarbor`），也可以用 `AGENT_HARBOR_DATA_DIR` 指定。仪表盘的数据目录里能看到确切路径。
- **供应商凭据**：保存在 `newapi-providers.dat` 中。Windows 上用当前用户的 DPAPI 加密；其他平台只限文件所有者读写（`0600`），不加密。界面不会回读已保存的密钥，编辑时留空即保留原值。
- **账号来源凭据**：保存在 `account-sources.dat` 中，保护方式与供应商凭据相同；令牌只在路由转发请求时使用，界面里看不到。
- **DeepSeek Harness**：路由所需的密钥存在它自己的 `.credentials.yaml` 里，headless profile 则存在 `$DSH_HOME/.env`（macOS 和 Linux 上按 Harness 的要求只限所有者读写 `0600`，不加密）；profile patch 里只有引用名。
- **网关 key**：和供应商凭据一起加密保存。网关只监听 `127.0.0.1`，只接受网关 key；转发给上游时用的是各中转自己的凭据，网关 key 不会发出去。
- **本地路由**：需要本地令牌才能访问。发往中转的请求不会携带官方账号凭据。
- **客户端配置**：逐字段写入并先备份。被你或其他工具改过的字段会提示冲突，不会被悄悄覆盖。
- **Cursor**：本地证书和它的私钥在 AgentHarbor 数据目录的 `cursor` 子目录里，私钥只限文件所有者读写。证书受名称约束，只能用于 `cursor.sh` 和 `cursor.com`。退出 AgentHarbor 时会还原自己写入的本地身份、Cursor 的 `settings.json` 和 `CURSOR_API_ENDPOINT`；下次启动时重新接入。
- **代理探测**：只检测网络是否可达，不检查鉴权和额度。Codex 自身的登录和 Native 请求从 Codex 进程发出，改了它的代理后要重启 Codex。
- **直连卡片**：只有该中转不走代理。经本地路由时每张卡各走各的出口；Codex 原生直连、直连配置的客户端和从面板启动的客户端，其余流量（登录、MCP、插件）照常走代理。系统代理实时读取，开关代理软件或换端口后不必重启 AgentHarbor。TUN 模式下由代理软件的分流规则决定，任何应用都绕不开 TUN 网卡。

## 平台支持

| 平台 | 说明 |
| --- | --- |
| Windows | x64，NSIS 安装包（中文与英文）；主要的开发和测试平台 |
| macOS | 12 及以上，app 包 |
| Linux | 部分流程尽力支持 |

终端代理目前仅支持 Windows。

## 开发

**环境要求**：
- Node.js（本机使用 24）、pnpm 11.22、Rust stable。
- [Tauri 前置依赖](https://v2.tauri.app/start/prerequisites/)。在 Windows 上需要 MSVC 工具链、Visual Studio 2022 Build Tools（"使用 C++ 的桌面开发"及 Windows SDK）和 WebView2。

```bash
pnpm install --frozen-lockfile
pnpm tauri dev            # 桌面应用；`pnpm dev` 只是浏览器预览，没有 Rust 命令
```

在 Windows 上，`scripts/tauri.mjs` 默认使用 `stable-x86_64-pc-windows-msvc`；设置了 `RUSTUP_TOOLCHAIN` 时以它为准。全局的 Rust 默认工具链不会被改动。

**隔离运行**：把 AgentHarbor 指向一个临时的 Codex 主目录，真实目录不受影响。

```powershell
$env:AGENT_HARBOR_CODEX_HOME = Join-Path (Get-Location) '.runtime-test\codex'
pnpm tauri dev
```

这样也会分开单实例锁和 WebView 数据。`AGENT_HARBOR_CLIENT_HOME` 对 Claude Code、Claude Desktop、OpenCode、Pi、DeepSeek Harness（改用其下的 `.dsh`，不读 `DSH_HOME`）和 Cursor IDE 的路径做同样的隔离（并且不改动 `CURSOR_API_ENDPOINT`）。

**检查**：

```bash
pnpm build           # TypeScript + Vite 生产构建
pnpm test:rust       # Rust 单元与工作流测试
pnpm test:frontend   # Playwright 回归：真实组件 + 模拟命令（先执行 pnpm build）
pnpm release         # 签名发布构建：安装包、更新签名、latest.json 和 SHA256SUMS.txt，位于 src-tauri/target/release/bundle/nsis/
```

- **前端测试**：需要 Playwright 和 Chromium。不在默认搜索路径时，用 `PLAYWRIGHT_MODULE_PATH` 和 `PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH` 指定；用 `FRONTEND_QA_SCOPE` 只跑某一组。
- **真实 Codex 测试**：默认跳过。设置 `AGENTHARBOR_TEST_CODEX_BIN` 后运行 `cargo test --manifest-path src-tauri/Cargo.toml -- --ignored real_codex`。这些测试只使用临时目录、合成凭据和本地模拟上游。
- **真实 DeepSeek Harness 测试**：同样默认跳过。把 `AGENTHARBOR_TEST_DSH_BIN` 设为已安装的 `dsh`（或它的 `lib/bin.js`，用 Node 运行），再运行 `cargo test --manifest-path src-tauri/Cargo.toml -- --ignored real_harness`。它在临时 `DSH_HOME` 里初始化 profile，让每种路由经本地路由访问本地模拟上游。
- **更新签名**：发布构建会为应用内更新签名安装包。私钥依次从 `TAURI_SIGNING_PRIVATE_KEY`、`TAURI_SIGNING_PRIVATE_KEY_PATH` 或 `~/.tauri/agentharbor-updater.key` 读取，不进仓库；请备份好，已安装的版本只接受用它签名的更新。
- **发布**：升版本号（`package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`）并提交，再推送一个带注释的标签，标签说明就是发布说明：`git tag -a v1.0.2 -F notes.md && git push origin v1.0.2`。GitHub Actions 会先跑测试，再用仓库密钥 `TAURI_SIGNING_PRIVATE_KEY` 签名构建，并带上面四个文件创建草稿 Release。确认后把草稿发布为最新版本（不要标成预发布）：更新地址是 `releases/latest/download/latest.json`。手动运行 Release 工作流只构建文件（保存为构建产物），不建 Release。
- **持续集成**：每次推送到 `main` 和每个 PR 都会跑类型检查、Rust 测试和前端回归。
- **模型目录快照**：发布前用 `node scripts/build-models-dev-snapshot.mjs` 更新内置快照。运行时下载的缓存只要完整有效，就会取代内置快照。

## 项目结构

```text
src/                 React 界面（页面、组件、hooks、中英文案）
src-tauri/src/core/  Rust 领域逻辑：路由与代理、聚合、客户端配置、账号、会话、MCP、Skills
src-tauri/src/       Tauri 命令与平台代码
scripts/             构建辅助与前端回归测试
assets/、LICENSES/   品牌素材与第三方声明
```

## 参与贡献

欢迎提交 Issue 和 PR。动手前请先阅读 [CONTRIBUTING.md](CONTRIBUTING.md)：本应用会写入真实的 Codex 和其他客户端数据，所有改动都必须安全、可撤回。较大的改动建议先开 Issue 讨论。

## 许可证

[Apache License 2.0](LICENSE)。第三方组件和数据（包括 models.dev 快照）见 [LICENSES/THIRD_PARTY.md](LICENSES/THIRD_PARTY.md)。

## 免责声明

AgentHarbor 是独立工具，与 OpenAI、Anthropic 没有隶属、背书或赞助关系。使用第三方中转服务的风险由你自行承担，并请遵守其服务条款。
