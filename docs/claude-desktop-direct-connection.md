# Claude Desktop 直连配置说明

本文说明 AgentHarbor 的 **Claude Desktop 直连**功能：把 Claude Desktop 桌面客户端直接接到一张已经保存在 AgentHarbor 里的中转卡片，不再经过 AgentHarbor 自身的转发。

- 界面与文案见 [claude-desktop-config-card.tsx](../src/components/newapi/claude-desktop-config-card.tsx)、[claude-desktop-config-dialog.tsx](../src/components/newapi/claude-desktop-config-dialog.tsx) 与 [zh.json](../src/locales/zh.json) 的 `claudeDesktop` 命名空间。
- 后端实现见 [src-tauri/src/core/claude_desktop](../src-tauri/src/core/claude_desktop)。

## 1. 这是什么 / 不是什么

**是什么**

- 从**已保存的中转卡片**直连：地址与推理密钥都由后端从卡片读取，配置对话框不询问地址、不询问密钥，也不询问余额 token（见 [claude-desktop-config-dialog.tsx](../src/components/newapi/claude-desktop-config-dialog.tsx) 的 `savedKeyHint` 文案与实现）。
- 直连的落地方式是写 Claude Desktop 自己的第三方配置（第四节四个文件），并让其中的 `configLibrary` 条目指向本工具生成的 profile。
- 卡片是否在路由池启用与直连资格无关：`enabled=false` 的卡片同样可以直连，而且直连**不会**把它启用，也不会改动它的默认模型、协议、`activeProviderId` 或全局路由开关。

**不是什么**

- **与 Claude Code 客户端完全独立**：Desktop 是独立的卡片、独立的接口、独立的四文件事务；界面文案明确"本项设置与 Claude Code 客户端相互独立，修改它不会影响 Claude Code"，组件也不引用任何 Claude Code 配置模块。
- **请求不经过本项目**：Claude Desktop 启动后直接向 profile 里的地址发请求。因此直连**没有**本项目的路由诊断（route trace、请求诊断）、**没有**费用账本与价格统计，也**不使用**本项目的全局代理设置。它也不要求 AgentHarbor 保持运行，不要求本地路由服务开启。
- **不是网关/中转再转发**：本工具只写配置，不做请求转发、不做协议转换、不做模型名映射。Desktop 发出的模型 ID 就是 profile 里写的 ID。

## 2. 兼容条件

四项必须同时满足：协议是 Anthropic Messages、模型名符合 Desktop 的安全命名、地址是可投影的站点根、卡片上已有可用的推理密钥。判定逻辑集中在 [provider.rs](../src-tauri/src/core/claude_desktop/provider.rs)，前端预览与后端提交使用同一份规则。

### 2.1 协议

- 逐模型判定顺序：**显式协议设置 > 端点声明（`endpoint_types`，只对名字含 `claude` 的条目生效）> 名称推断 > 卡片默认协议**。
- 只有判定结果是 Anthropic Messages 才能直连。显式设为 Responses 或 Chat Completions 的模型会被排除（原因码 `incompatible_protocol`），**不会**因为名字里含 `claude` 而放行。
- 端点声明不含 Messages 能力、且卡片默认协议也不是 Messages 时，排除原因是 `unsupported_model_endpoint`。
- 协议来源继续保存在候选数据中，模型列表不重复显示推断提示；需要调整时在中转卡片设置协议。

### 2.2 模型名

- 必须符合 `claude-{sonnet|opus|haiku}[-fable]-<后缀>`：允许 `anthropic/claude-` 前缀，后缀必须非空，模型 ID **原样保留**（不做归一化后再写入）。
- `fable` 只在已核实版本上开放：探测到的 Desktop 版本不低于 `1.12603.1`（[constants.rs](../src-tauri/src/core/claude_desktop/constants.rs) 的 `FABLE_MIN_VERSION`）；版本未知或更低时 `fable` 不在白名单里。`mythos` 不属于可用角色，即使名字像 Claude 也不放行。
- 以下都会被拒绝，并且**逐项报出具体原因、整次提交零写入**：任意品牌（`gpt-4o`、`kimi-k2`）、旧 Claude 布局（`claude-3-5-sonnet-20241022`）、空后缀（`claude-sonnet-`）、Claude Code 的 `[1m]` 后缀、含空白或控制字符、ID 超过 200 字符。
- 卡片中 `included=false` 的条目会显示排除原因，不能选择或提交；即使同时是默认模型也不能绕过。已删除或不属于该卡片候选集的模型会被拒绝；只有目录中没有条目时，精确的卡片默认模型可作为已有手填候选。

### 2.3 地址

- 必须是**站点根**（可以带自定义路径前缀）。Claude Desktop 会在这个根地址后追加 `v1/messages` 与 `v1/models`，因此写入 profile 的根会去掉末尾 `/` 与末尾 `/v1`：

| 卡片里保存的地址 | 写入 profile 的根 | Desktop 实际访问 |
| --- | --- | --- |
| `https://relay.example` | `https://relay.example` | `https://relay.example/v1/messages` |
| `https://relay.example/v1` | `https://relay.example` | `https://relay.example/v1/messages`（不会拼成 `/v1/v1`） |
| `https://relay.example/custom/v1` | `https://relay.example/custom` | `https://relay.example/custom/v1/messages` |
| `https://api.deepseek.com` | `https://api.deepseek.com/anthropic` | `https://api.deepseek.com/anthropic/v1/messages` |

- 会被拒绝的形态：
  - **完整接口地址**：以 `/v1/messages`、`/v1/responses`、`/v1/chat/completions`、`/v1/models`、`/messages`、`/chat/completions` 结尾的地址（不会把它悄悄改成另一种含义，请先在卡片里改成站点根）。
  - **non-v1 布局**：`api.novita.ai/v3/openai/`、`open.bigmodel.cn/api/paas/v4/`（含 `/api/coding/paas/v4/`）、`api.z.ai/api/coding/paas/v4/`。这些站点本项目会去掉 `v1/` 前缀，而 Desktop 只会追加 `v1/`，两者布局不一致，因此首期直接拒绝直连。后续 Desktop 网关模式计划适配这类布局，目前尚未接入。
  - **无法解析的地址**：非 `http(s)`、含 userinfo（`user:pass@`）、含 query 或 fragment 的 URL。
  - 与项目自身端点投影不一致的地址（布局不匹配）同样被拒绝。
- `http://` 明文地址会被接受但会在预览里标记为明文，不会自动升级成 https。

### 2.4 凭据

- 必须使用卡片上的**推理密钥**（`api_key`）。**余额 token（`access_token`）不能代替推理密钥**：只有余额凭据的卡片整卡不合格（原因码 `no_inference_key`）。
- 密钥不能包含空白或控制字符，长度上限 8192 字节；把 `Bearer ` 前缀一起粘贴进来会被去掉，profile 里不会出现双前缀。

## 3. 操作步骤

1. 打开中转页里的 **Claude Desktop** 卡片，点"配置"（已托管时按钮显示"切换 / 更新"）。选择、检测和预览均可在 Desktop 运行时使用。
2. 在对话框里按三步操作：
   - **选择卡片**：选择已有的合格卡片；不合格的卡片会显示具体原因。检测到多个安装渠道时，先选择"配置位置"。
   - **选择模型与展示名 / 1M**：勾选要开放的模型；可以为每个模型填写显示名称（可选，最长 64 字符，控制字符会被清理）；“额外 1M 选项”使用单击复选框，新选择默认关闭。有效能力未声明 1M 时不能新开启；已有开启选择仍可取消。这里还有两个连通性核对按钮：
     - **检测 Bearer 支持**：一次只读的 `GET /v1/models`，确认该地址用这张卡片的推理密钥能读到模型列表。**不发送任何计费请求**，也不改动卡片的测试状态。
     - **核对 Messages 兼容性**：一次 `POST /v1/messages`、`max_tokens=1` 的真实请求，用来确认该地址接受 Claude Desktop 实际使用的**裸 Bearer**（Desktop 在 `/messages` 上不带 `x-api-key`）。这一步**会产生极少量计费**，所以必须由你点击才会发生；默认从不自动发送。结果里会显示上游自报的模型名，便于发现中转是否改写了模型名。
   - **预览**：直接按文件显示角色、路径、操作及原值/新值，模型配置在 `inferenceModels` 中查看。密钥值固定显示为 `<redacted>`，外部配置只报告数量。预览 ticket 有效期 300 秒，表单任何变化都会让旧预览失效。
3. 保存 Desktop 中的工作，完全退出客户端（含托盘），再点"应用"提交。提交失败会保留输入并清除已消费的预览，重新预览后可再提交。提交前会重新读取全部输入并逐文件确认没有被外部改动；四个文件写入后逐项回读确认，再保存内部状态。任一步失败都会回滚；回滚不完整时会保留事务日志并报"恢复未完成"，不会出现"看起来成功"的假成功。
4. 让配置生效，两种方式任选：
   - **点卡片上的「重启 Claude Desktop」**：在已确认的安装上启动客户端；若提交后又打开了客户端，则先停止该安装再启动。
   - **手动启动 Desktop**。预览可在运行时查看；应用前完全退出 Desktop，客户端启动后读取新文件。
   如果四个文件本来就已是目标状态，不产生新的重启要求；已有待重启状态会保留，直到本项目确认启动成功。重启按钮只在「已确认可自动启动」且「配置确实待生效或应用正在运行」时出现；没有确认到启动方式时请手动启动。
5. 重启后在模型菜单里查看。如果菜单里没有出现预期模型，先回到卡片确认状态与警告，再按第 7 节排查冲突与未完成事务。

重新应用、恢复接管前配置、切换官方模式和释放管理都先预览再提交，并有确认对话框；恢复未完成事务直接核对已有 journal 的 before/after 与提交点，需单独确认，不能另签一笔配置预览。

## 4. 四个文件的确切路径与作用

四个文件由 [paths.rs](../src-tauri/src/core/claude_desktop/paths.rs) 从"常规根目录"与"3P 根目录"两个根推导；Windows 默认分别是 `%LOCALAPPDATA%\Claude` 与 `%LOCALAPPDATA%\Claude-3p`。

| 角色 | 路径 | 直连时写入什么 |
| --- | --- | --- |
| normalConfig | `<常规根>/claude_desktop_config.json` | **只写 `deploymentMode`**：直连写成 `"3p"`；同文件里的 MCP、偏好等其它键原样保留（受管键只有 `deploymentMode`，见 [constants.rs](../src-tauri/src/core/claude_desktop/constants.rs) 的 `MANAGED_CONFIG_KEYS`） |
| thirdPartyConfig | `<3P 根>/claude_desktop_config.json` | 同样只写 `deploymentMode`（`"3p"`）；`mcpServers`、`preferences`、`coworkUserFilesPath` 等键原样保留 |
| profile | `<3P 根>/configLibrary/d918233d-0a41-47a1-a29f-6d610a03722a.json` | 本工具**自己的完整 profile**，整套受管 |
| meta | `<3P 根>/configLibrary/_meta.json` | 注册自己的条目并把 `appliedId` 指向自己的 UUID；外部条目与未知键原样保留 |

**profile 的内容**（[profile.rs](../src-tauri/src/core/claude_desktop/profile.rs) 的 `profile_json`）：

```json
{
  "disableDeploymentModeChooser": true,
  "inferenceProvider": "gateway",
  "inferenceGatewayAuthScheme": "bearer",
  "inferenceGatewayBaseUrl": "https://relay.example",
  "inferenceGatewayApiKey": "<明文推理密钥>",
  "inferenceModels": ["claude-sonnet-4-6", { "name": "claude-opus-4-6", "labelOverride": "中转 A · Opus", "supports1m": true }]
}
```

- `inferenceModels` 的每一项：没有显示名也没有 1M 时是字符串；有显示名或 1M 时是对象 `{ name, labelOverride?, supports1m? }`，且 `supports1m` **只在为 true 时出现**。
- 不写 Claude Code 的 `[1m]` 后缀；首期也不写 `coworkEgressAllowedHosts`（见第 8 节）。
- 1M 只在模型的有效上下文窗口不小于 1,000,000 且来源是可依据的来源（手动设置、供应商设置、上游、官方、目录及其别名/通用目录）时才声明；未知、兜底或纯名称推断一律不声明。开启后 Desktop 同时提供普通与 1M 项，模型 ID 只写一次；直连新选择在支持 1M 时默认勾选，用户可取消，已有选择不自动改变。
- 本机 Desktop 2.19675.0 的 `supports1m` 契约是“追加 1M 变体”，`prefer1m` 只控制首项默认选择，均不能只显示 1M 单项。关闭该选项只省略这个菜单标记，不修改中转模型能力；Desktop 实际窗口还依赖其 SDK 对模型、原生 1M 和供应商的识别，不能凭中转上下文值承诺普通项的本地预算。

**meta 条目**：`{"id": "<UUID>", "name": "AgentHarbor"}`，`appliedId` 指向同一个 UUID。自身条目的插入位置沿用它在文件里原来的位置（原来不存在时插到最前），外部条目的顺序与未知字段保持不变。

**UUID 固定不变**：`PROFILE_ID = d918233d-0a41-47a1-a29f-6d610a03722a`，常量注释写明"这些取值属于本产品的磁盘契约，一经发布不得变更"。它和 CC Switch 的 `00000000-0000-4000-8000-000000157210` 不同，两个工具可以同时存在于同一个 `configLibrary` 而互不覆盖。

**写入顺序**：自己的 profile → meta 注册/选中 → 3P config → normal config。每个文件都是原子替换；整体是"四个单文件原子替换的协调事务"，不是系统级同时可见的事务，因此中途崩溃由持久事务日志判定实际状态（见第 7 节）。

## 5. 恢复语义

| 操作 | 改什么 | 不改什么 |
| --- | --- | --- |
| **恢复接管前配置** | 两个 config **各自**回到自己接管前的 `deploymentMode`；接管前没有该键时删除它。`_meta.json` 恢复接管前记录的自身条目与 `appliedId`，包括当时已有的外部选择。profile 接管前已有内容则写回，否则删除；成功后结束本次管理 | 外部条目、未知键、MCP、偏好与企业策略保留；只恢复有记录的指针，不猜测其它外部条目 |
| **切换官方模式** | 两个 config 的 `deploymentMode` 都写成 `1p`；移除自身 meta 条目；移除 `appliedId`（不会指向其它外部 profile）；自己的 profile 同上（接管前有内容则写回，否则删除） | 外部条目与未知键保留；企业策略键保留。当前生效的第三方指针已被别的工具切换时会拒绝执行 |
| **释放管理** | 不写任何 Desktop 文件（返回的改动路径为空）；把内部状态转成 tombstone（清除文件快照与模型列表、清空供应商引用）并写一份本地备份 | 磁盘上的四个文件完全不变；释放后卡片回到"未配置"，同时**失去"恢复接管前配置"的入口**，磁盘上的 profile 仍含凭据 |
| **以现状重新接管** | 勾选"按当前状态重新接管"后提交（`rebaseline`）：忽略当前受管漂移冲突，把磁盘现状作为**新的还原点**，替换原来的还原点 | 外部条目与未知键仍保留；只是"可恢复到的历史状态"被当前状态取代，界面会提示"当前配置将成为新的还原点" |

**企业策略键不会被删除。** 以上每个操作都只处理本工具自己写入的值：两个 config 里的 `deploymentMode`、自己的 profile 文件、`_meta.json` 里的自身条目与 `appliedId`。其它键——包括企业推理与部署策略——一律保留；界面警告明确"配置中的企业推理与部署策略会被保留，并可能覆盖本次配置"。

## 6. 密钥与凭据

- **profile 里必然包含明文推理凭据**（`inferenceGatewayApiKey`）。这是 Desktop 直连格式的限制：Desktop 需要一个可以直接使用的 token，所以无法只写引用或句柄。
- 预览、接口返回值与卡片只显示 `<redacted>`；密钥不会出现在错误消息与日志里（有专门的用例断言哨兵密钥不出现在预览与其它返回值中）。
- 内部记录（`state.dat`、`pending.dat`、`previews.dat` 与 `backups/*.dat`）用 `secret_file` 写入：**Windows 为当前用户 DPAPI 加密；Unix 只设置 0600 权限、不做内容加密**。现有工具在 Unix 不提供内容加密，因此本文不声称"全平台加密"。
- **释放管理不会撤销已经落在 profile 里的密钥**：释放不改动任何 Desktop 文件，profile 与其中的凭据原样留在磁盘上。**删除中转卡片同样不会撤销它**——配置文件里的副本不受 Store 影响。
- 要真正撤销：执行"恢复接管前配置"或"切换官方模式"（两者都会把 profile 写回接管前内容或删除它），或者在上游轮换密钥使其失效。界面在释放时会提示"释放管理不会删除磁盘上已有的配置，其中仍包含中转凭据；请在上游轮换密钥使其失效"。

## 7. 冲突与恢复

- **外部改写受管字段**（两个 config 的 `deploymentMode`、`_meta.json` 的自身条目或 `appliedId`、自己 profile 的内容）会在状态里显示为冲突（`drift:<文件角色>`），**默认不覆盖**。提交前还会逐文件校验"当前内容仍等于事务开始时的快照"，不一致时报"预览之后文件已被改动"并且零写入，需要重新预览。
- **未完成事务**：启动后旧进程的预览 ticket 失效；卡片显示"有未完成的事务"并提供恢复入口，在此之前其它写入操作会被拒绝。恢复时比对提交点与实际受管字段：
  - state 提交点匹配且受管字段符合事务 **after** → 清理事务日志；
  - 尚未提交、当前受管值等于 **before** 或 **after** → 撤回本次事务，回到本次 **before**，保留外部新增的无关键；
  - 与两者都不一致 → 保留证据并报冲突，不自动覆盖。
- **预览与 revision**：前端收到的是不透明一次性 ticket；后端绑定操作、原始选择、路径身份、四文件基线、写入计划与状态版本。ticket 有效期 300 秒，且只在签发它的后端进程中有效；每个配置目标最多保留 8 个未消费 ticket。重复使用会报过期；输入变化后必须重新预览。
- **跨进程写入**：另一个 AgentHarbor 实例或会话正在管理同一份配置时，写入会立即失败（`writer_busy`，不排队、不阻塞）；锁文件在应用数据目录的 `claude-desktop/writer.lock`，**不写进 Claude Desktop 目录**，使用操作系统排他文件锁，持有句柄关闭或进程崩溃后自动可用；锁文件保留，pid 仅用于提示。升级后先退出旧版实例，旧 PID 锁协议不支持与新实例混合并行写入。注意这把锁只约束 AgentHarbor 自己：Claude Desktop 与其它配置管理工具（例如 CC Switch）都不遵守它，因此操作前仍应完全退出 Claude Desktop，并暂停其它配置工具。
- **本地备份**：每次提交与释放前都会写一份受保护的恢复备份（`backups/<id>.dat`），**最多保留 20 份**；卡片显示当前备份数量。

## 8. 限制与未验证项

**已有 Windows + Claude Desktop 2.19675.0 的核心链路实施证据，完整发布验收尚未通过。** 当前证据与缺口见[验收记录](./claude-desktop-acceptance-record.md)：

- 最小 profile、模型菜单、真实 Bearer 请求、多轮和普通工具调用已有实施记录；Desktop 侧流式渲染未单独观察。
- 真实恢复、路由启动后完全退出的组合和官方登录流程仍未完成；本轮代码修复只运行隔离自动化，不重做真实接管。
- Cowork：未验证；首期 profile **省略** `coworkEgressAllowedHosts` 字段（有用例断言该字段不被写入），省略是否可行没有在真实 Desktop 上确认。
- macOS / Linux：路径发现按平台分支实现，但未在这两个平台运行与验收；Unix 的内部记录只有 0600 权限、不做内容加密。
- **检测成功不等于 Desktop 能连通**：检测使用 AgentHarbor 自身的网络设置与出口，Desktop 走自己的网络。
  - 列表检测只证明"该地址用这张卡片的推理密钥能读到 `/v1/models` 列表"；返回 401/403 也只说明本次校验被拒绝，不等于 `/messages` 永远不接受 Bearer。
  - Messages 核对是比列表检测更强的证据（它证明裸 Bearer 在该端点可用、且模型名被原样传递），但仍**不**覆盖 Desktop 自身的行为：profile 是否被接受、菜单是否显示、多轮/流式/工具是否可用，只能在真实 Desktop 里确认。
  - 实测记录：本机卡片 `kiro-fucheers`（`https://www.fucheers.top/`）配 `claude-haiku-4-5-20251001` 时，两项核对都是 200，上游自报模型与请求一致。这属于单张卡片的实测结果，不代表其它中转成立。
- 版本策略：`fable` 只在探测到的版本不低于 `1.12603.1` 时开放；探测失败时显示"未检测到版本"并保守关闭。版本探测读取每个渠道日志里**最后一条** `appVersion`，并且**优先使用 3P 渠道日志**（常规渠道只作回退），因为一个日志文件保存的是该渠道的完整启动历史（第一条往往是安装时的旧版本）。需要强调的是：这仍然只是日志证据，没有与正在运行的进程核对过，不能当作当前版本的保证。
- 生效时机：先选模型并查看预览，再完全退出 Desktop（含托盘）后应用，最后启动。预览允许运行；后端只在配置写入时拒绝仍运行的 Desktop，文件变化时需重新预览。成功使用重启按钮启动后，“需要重启”会清除；启动失败则保留。提交后的启动入口只操作已确认的安装。
- 重启能力的平台差异：Windows 对已确认安装使用 MSIX 激活或可执行文件；macOS/Linux 使用已识别的 Desktop 安装入口。没有确认身份时要求手动启动，不回退启动 Claude Code CLI。**检测不到启动方式时界面不显示重启按钮**。
- 隔离模式（`AGENT_HARBOR_CLIENT_HOME`）下**不提供重启**：那会去启动用户真实的 Desktop 安装。
- 进程识别按可执行路径和选定安装身份匹配；进程名只用来枚举候选，不作为停止权限。强制停止不结束进程树，避免误伤 CLI 子进程。

## 9. 环境变量 `AGENT_HARBOR_CLIENT_HOME`

设置该环境变量后，全部路径查找与读写都被限制在隔离根内：

- 只使用 `<隔离根>/Claude` 与 `<隔离根>/Claude-3p`，**不再扫描** `%LOCALAPPDATA%`（或 macOS/Unix 的默认位置），也不读取生产环境的日志。
- 状态会带上"隔离目录（AGENT_HARBOR_CLIENT_HOME）"的说明，并在警告里提示"当前设置了 AGENT_HARBOR_CLIENT_HOME，只读写隔离目录"。
- 路径身份包含隔离标记：在隔离与生产之间切换会让已经发出的预览 ticket 失效，需要重新预览。

## 如何验证

从项目根运行（命令来自 `package.json` 的 `scripts`）：

```powershell
pnpm test:rust
pnpm test:claude-desktop-config
pnpm test:frontend
pnpm build
```

- `pnpm test:rust`：Rust 单元测试与集成测试，包含本模块的单测（[tests.rs](../src-tauri/src/core/claude_desktop/tests.rs)）与端到端用例（[claude_desktop_workflows.rs](../src-tauri/tests/claude_desktop_workflows.rs)）。
- `pnpm test:claude-desktop-config`：不需要 DOM 的契约检查（[test-claude-desktop-config.mjs](../scripts/test-claude-desktop-config.mjs)）：本地化 key、DTO 字段、命令注册、密钥与独立性。
- `pnpm test:frontend`：上面的脚本加上既有前端回归套件。
- `pnpm build`：TypeScript 编译与前端产物构建。
