# 用户反馈修复与验收（2026-10-02—2026-10-05）

## Command Code 账户总览补充（2026-10-05）

- 最新只读参考为用户指定的 `D:/workspace/temp/dsh-commandcode-provider`，基线 `7152f63cbe80ee2c0ec5ec95fd347971f20e24fc`。核对 `src/adapter.ts` 的 `getUsage()`、`src/client/section.tsx` 和 `src/login.ts`，确认登录最终保存的是模型 API Key，查询无需额外 cookie、session 或登录模块。研究仓库克隆使用用户指定的 `http://127.0.0.1:10808`，未改动应用已保存的代理设置。
- credits 为必需请求；成功后读取 `/alpha/whoami`、`/alpha/billing/subscriptions?orgId=...` 和 `/alpha/usage/summary`。四接口复用同一 `Authorization: Bearer <模型 API Key>`，组织 ID 仅用于订阅参数。新增 usage 请求与身份/订阅读取重叠，不增加一整段串行超时。
- 新增可选 `balance.commandCode` 投影账户名称/用户名、套餐及账期、总请求/完成/失败计数、成功率、费用、credits 消耗和输入/输出 Token。仅保存明确的展示字段，不传完整账户响应、密钥或未知字段；旧余额 DTO 保持兼容。
- 统计为官方账户用量，独立于本项目 Router 账本。`totalCount` 显示为“总请求”；`successRate` 已是 0–100 百分值；`totalCost` 显示 USD；额度、窗口和 `totalCredits` 继续显示 credits，不照抄参考项目将额度标为美元的格式器。
- 仅内置 Command Code 查询使用紧凑总览：账户/套餐、四项统计、月度/已购/赠送额度、5 小时/周窗口已用进度、重置/账期/更新时间。旧报告、共享余额与自定义脚本继续沿用原契约，脚本结果仍用通用界面。
- HTTP/JSON/显式 `success:false` 辅助失败保留可用额度；缺失或非法统计显示未知，真实零保留。零已购/赠送额度不再误报账户不可用，真实窗口耗尽仍有短提示；未知未来额度标签不冒充月额度。计数所有解析路径统一限制在 JavaScript 安全整数范围。
- 布局检查定位并修复余额弹窗装饰背景的横向溢出；标题和总览允许收缩。已审阅 1200px/390px 深浅主题，窄窗口统计自动变为两列，内容可纵向滚动。截图均为合成数据。

| 本轮检查 | 结果与证据 |
| --- | --- |
| `pnpm build` | TypeScript/Vite 生产构建成功；`.runtime-test/command-code-build.log` |
| `RUST_TEST_THREADS=1 pnpm test:rust` | 757 单元、11 Desktop 集成、9 本地流程通过；21 ignored 不计为通过；`.runtime-test/command-code-rust-final.log` |
| `pnpm test:frontend` | 10 组配置检查 + 115 条浏览器回归，共 125 PASS，exit 0；`.runtime-test/command-code-frontend-final.log` |
| HTTP/解析回归 | 四接口同 Key、组织参数、辅助 404/503/坏 JSON/空响应/`success:false`、必需接口失败、旧 DTO、零/缺失/非法值与安全整数边界均通过 |
| 前端回归 | 官方统计、币种、零额度、已用比例、失败保留旧报告、旧 DTO、部分字段、真实窗口限制、未知标签、仅窗口结果及自定义脚本回退均通过 |
| 独立终审 | 安全整数、月额度识别、窗口耗尽提示三项修复后无新增阻断项 |
| EXE/安装包 | 交付目录的 `build.log`、`build-manifest.json`、`SHA256SUMS.txt` 与 `startup-check.json` 单独记录本轮构建及隔离启动结果 |

本轮未查询用户真实账户、读取真实认证文件或向计费接口发请求。接口契约与本地 mock 已验证，真实账户返回及安装升级流程尚未验收。

以下保留此前各轮记录；其中测试计数和交付目录属于各自日期的版本。

## 范围与结果

当前工作区代码保留原有 Claude Code 注入修复。本轮未提交、推送或发布远程 Release。

| 反馈 | 已实现内容 | 验证情况 |
| --- | --- | --- |
| 默认模型无法删除、不能清空 | 可取消默认、移除默认模型、清空选择及待加入列表；保存后不会复活默认 | 前端交互和 Rust 保存/重载测试通过。正在使用的 Native 配置仍保留有效性检查 |
| 拉取模型全部加入并默认选中 | 新模型进入待加入区，搜索后按需勾选；默认先显示 50 个，可继续展开；刷新保留原有选择与覆盖 | 前端验证拉取、加入、取消、保存和异步响应隔离 |
| API Key 眼睛无效 | 显式点击后读取已保存密钥；支持隐藏、替换、失败重试；关闭或切换提供商后忽略旧响应 | 前端显示/隐藏、替换、跨窗口响应、失败重试通过 |
| 能力目录和冲突来源难懂 | 增加匹配说明，列出完整 provider/model 引用和冲突候选来源 | 目录与诊断回归、壁纸背景兼容检查通过 |
| Router 与用量记录关系不清楚 | 页面说明只有经过本地 Router 的请求进入请求诊断及中转账本；Claude Code 直连日志走本地用量统计 | 说明与现有统计路径核对，诊断回归通过；没有新增直连流量的 Router 采集 |
| 全局默认、百分比预算含义不清楚 | 改为全局自动压缩、按上下文百分比，说明与账户资金无关 | 构建和既有压缩策略回归通过 |
| 线程总览间距、会话卡片不统一 | 收紧总览、页头和分组间距，统一网格/列表卡片边框、背景、圆角与选择状态 | 桌面/窄窗口、深浅主题截图审阅；虚拟列表、选择、详情返回滚动及焦点恢复回归通过 |
| MiMo 官方入口与 Token Plan | 新增两个独立预设；保留原余额、自定义脚本与共享余额行为 | 预设、旧余额保留、无接口时不自动查询、自定义脚本及共享脚本回归通过；自动余额查询尚未接入 |
| Command Code 套餐额度 | 新增官方预设；credits、套餐和额度窗口接入原有余额卡片与详情；失败保留上次结果 | Rust 解析及本地 HTTP mock、前端卡片/详情回归通过；真实账户未联网验收 |

## Command Code 接入细节

- 参考用户指定的只读项目 `D:/workspace/temp/command/dsh-commandcode-provider`。
- 模型端点为 `/provider/v1`；余额、身份与订阅查询使用根路径 `/alpha/billing/credits`、`/alpha/whoami`、`/alpha/billing/subscriptions`，组织 ID 用于订阅查询。
- 查询使用模型 API Key；不会将 NewAPI 账户 token 当作模型密钥发送。
- 展示月度 credits、已购/免费 credits、5 小时及周窗口，以及可解析的重置时间。额度保留 credits 单位，不转换为美元。
- 已知套餐总量来自参考项目；未知套餐、缺失窗口、无限额窗口不编造数值。供应商以后调整套餐时仍需更新适配。

## 验证记录

| 命令/检查 | 结果 |
| --- | --- |
| `npm run build` | TypeScript 与 Vite 生产构建成功 |
| `npm run test:rust` | 515 个单元测试、9 个集成测试通过；4 个原有测试忽略，不计为通过 |
| `npm run test:frontend` | 61 组 PASS，无未预期浏览器错误 |
| `FRONTEND_QA_SCOPE=provider-feedback npm run test:frontend` | 套餐 fixture 对齐后，4 组定向回归再次通过 |
| `git diff --check` | 通过 |
| `npm run tauri -- build` | Windows x64 release EXE 和 NSIS 安装包生成成功 |
| EXE 隔离启动 | 新数据/客户端目录下启动 8 秒存活并创建窗口，stderr 为空；不等同于真实账户全流程验收 |

测试日志位于 `.runtime-test/feedback-{build,rust,frontend,providers,release}.log`。截图位于 `.runtime-test-visual/feedback/`。浏览器回归使用合成 API 数据；Rust HTTP 测试使用本地 mock，不消耗真实账户额度。

## 本地交付

目录：`D:/workspace/AiMaMi/.runtime-test/deliverables/AgentHarbor-20261002-202337/`

- `AgentHarbor.exe`：直接运行版本。
- `AgentHarbor-Setup.exe`：Windows x64 安装包。
- `SHA256SUMS.txt`：文件校验值。
- `startup-check.json`：隔离启动结果。

构建时旧 release EXE 被运行进程占用，停止该进程后重建成功。没有安装新包或覆盖用户安装目录；安装、升级、卸载流程未在本轮执行。

## 尚未完成的验收

1. **MiMo 自动余额/Token Plan 额度查询**：已有 API 与 Token Plan 预设，但本轮未取得可验证的余额接口与鉴权约定，因此没有实现自动余额适配。可继续使用自定义脚本；未知余额不会伪装为零。
2. **Command Code 真实账户验证**：当前依据参考项目及 mock 验证，尚未用用户真实账户核对返回结构、套餐与重置时间。
3. **真实客户端联调**：自动测试覆盖注入与路由逻辑；本轮未向外部模型发送真实计费请求，也未执行安装升级验收。

因此本轮可确认清单中的已实现功能通过上述测试，不能声称全部外部服务和所有真实场景均已验证。

## 模型原厂目录匹配纠正（2026-10-02）

- 原问题：未手动绑定目录时跨 models.dev 全目录合并同名模型；302.AI 等聚合商条目参与能力共识和来源选择，原厂值可能被冲突掩盖。
- 自动规则：安全规范化模型 ID 后识别原厂；常见系列涵盖 GPT、Claude、Gemini/Gemma、DeepSeek、MiMo、Qwen、Kimi、GLM、Grok、MiniMax、Mistral、StepFun 等。无明显前缀的型号可通过 canonical publisher 确定原厂；其次使用精确主机名匹配官方端点。
- 自动通用候选仅包含维护的原厂目录集合。302.AI、OpenRouter 等聚合目录不会自动进入来源、字段共识和冲突候选。新型号可复用原厂归属规则；新增原厂需要加入可信目录集合。
- 已识别原厂而原厂目录缺型号时保持未匹配；不把其他站点的数据冒充原厂。用户明确保存的目录绑定、单模型引用和能力覆盖仍优先；账单价格继续依赖独立绑定，不随能力目录自动推断。
- 能力预览、Codex Native/Router 和 Claude Code 投影共用解析结果。上游明确声明优先于原厂目录；上游明确空思考列表仍保持空。原厂目录可以补未声明能力，但不会引入官方账号缓存中的账号专属字段。
- 本次检查：Router 相关 43 项回归通过；Rust 全库 521 项通过、4 项忽略、0 失败；定向 Rust 格式与差异检查通过。回归包含十余个原厂型号、渠道前缀、未收录型号、聚合商同名污染、手动绑定及 Native 注入。
- 最新交付目录：`D:/workspace/AiMaMi/.runtime-test/deliverables/AgentHarbor-20261002-official-catalogs/`，包含直接运行 EXE、NSIS 安装包、SHA256 校验、models.dev 许可证和此验收说明。前一 `AgentHarbor-20261002-202337` 目录为旧版。
- 此次 `npm run tauri -- build --bundles nsis` 完成 TypeScript、Vite 及 Rust release 构建。新 EXE 在隔离数据、Codex 和客户端目录中启动 8 秒存活并创建窗口，stderr 为零；没有覆盖用户安装目录或关闭运行中的旧版。
- 构建日志：`.runtime-test/official-catalog-release.log`；交付目录中的 `startup-check.json` 保存启动证据。未使用真实 API Key 发起计费请求。

## 模型后缀与 canonical 身份匹配（2026-10-02）

- 修复 `claude-opus-4-8-thinking`、`gemini-3.8-flash-high` 等能力别名；支持 `thinking` 和思考档位的 `-`、`_`、`:`、`@` 形式，以及思考档位括号和渠道前缀组合。完整型号优先，版本号、日期、`preview`、`vision` 等实际型号差异不会被裁掉。
- 原厂 canonical 身份建立独立索引，支持带供应商和不带供应商的型号名。`deepseek-v4.1-flash` 根据 DeepSeek 官方条目声明的 `deepseek/deepseek-v4.1-flash` 对应到 `deepseek-flash`，没有硬编码版本替换。
- canonical 匹配限定已选定的原厂范围；同一身份有活跃条目时不混入已废弃条目，多个活跃条目冲突仍列出候选并保留字段冲突。聚合商命名空间不能改变已识别的模型原厂。
- 原始请求模型 ID 保留。后缀与 canonical 匹配只补能力参考；不会因此给别名套用基础模型价格。若完整别名有独立价格条目，仍优先使用完整条目。
- 预览增加中文/英文别名步骤说明，原厂目录自身声明的 canonical 别名显示为「本目录声明的模型别名」，避免误报成目标未收录。
- Rust 全库 525 项通过、4 项忽略、0 失败。回归包含截图中的型号、复合后缀、完整型号优先、canonical 原厂隔离、废弃别名、活跃冲突、版本保留、价格隔离和原始请求 ID 保留。
- 最新交付目录：`D:/workspace/AiMaMi/.runtime-test/deliverables/AgentHarbor-20261002-model-aliases/`，含直接运行 EXE、NSIS 安装包、许可证、SHA256 校验与验收说明；此前交付目录为旧版。
- `npm run tauri -- build --bundles nsis` 完成 TypeScript、Vite 和 Rust release 构建，日志为 `.runtime-test/model-alias-release.log`。
- 新交付 EXE 与本次 release 的 SHA256 完全一致。隔离数据、Codex 和客户端目录中启动 8 秒存活，创建窗口，stderr 为零；结果保存在 `startup-check.json`。未关闭用户旧版进程，未执行安装升级或真实计费请求。

## Gemini 省略 preview 的能力参考（2026-10-02）

- 截图中的 `gemini-3.1-pro-high` 在去除 `high` 后为 `gemini-3.1-pro`；内置 Google 目录只收录 `gemini-3.1-pro-preview`，且未声明对应 canonical 别名，因此此前仍未匹配。
- 新规则在所有常规完整/规范化/canonical 候选都没有命中后，只在 Google 范围中查找 Gemini 候选的同版本 `<型号>-preview` 活跃条目。规则没有写死 3.1 型号；省略 preview 的其它 Gemini 名称与渠道/思考后缀组合复用同一路径。
- 这是能力参考，不把它称为 Google 声明的身份别名。匹配过程新增中英文 `previewReference` 说明，保留具体官方来源；原始请求模型 ID 保持不变，计费不借用预览型号价格。
- 完整型号与常规基础型号优先。不同版本、日期、vision、自定义工具、未知后缀不猜测；不反向删除 preview，不选择 deprecated 预览条目，不引用聚合商。多个活跃条目有冲突时保留候选，冲突字段不填入。
- Rust 全库 527 项通过、4 项忽略、0 失败。回归覆盖用户截图、复合前后缀、大小写、完整型号优先、版本/变体保留、废弃条目排除、第三方隔离、显式绑定失效与价格隔离。定向 Rust 格式检查、`git diff --check` 通过。
- 测试日志：`.runtime-test/gemini-preview-rust.log`；构建日志：`.runtime-test/gemini-preview-release.log`。
- 此次 TypeScript、Vite、Rust release 和 NSIS 安装包构建成功。最新交付目录：`D:/workspace/AiMaMi/.runtime-test/deliverables/AgentHarbor-20261002-gemini-preview/`。
- 交付 EXE 与 release SHA256 一致；隔离数据/客户端目录启动 8 秒存活并创建窗口，stderr 为零，证据为 `startup-check.json`。用户运行中的旧版未关闭或覆盖；未执行真实计费请求或安装升级验收。

## 移除匹配预览说明面板（2026-10-02）

- 按用户要求删除「匹配预览怎么看」整块折叠说明及其专用中英文文案。
- 本次变更仅涉及展示组件和文案，模型匹配算法未修改。
- 构建日志：`.runtime-test/remove-match-guide-release.log`。
- 本次 TypeScript、Vite、Windows release EXE 和 NSIS 安装包构建成功。最新交付目录：`D:/workspace/AiMaMi/.runtime-test/deliverables/AgentHarbor-20261002-clean-preview/`，包含此前的 Gemini preview 能力参考及其余修复。
- 此次未新增或重跑功能测试；此前测试记录对应各自修复版本。本次交付包含文件 SHA256 校验。

## Claude Router 请求结构与思考参数（2026-10-02）

### 确认原因与修改

- 使用机器上已安装的 Claude Code，在隔离配置目录中捕获发往本地接收端的请求结构。普通请求包含 `messages[].role = system`、`thinking.type = adaptive`、`output_config.effort = high` 以及 22 个普通函数工具。旧转换器只接受 user/assistant，因而在发送上游之前返回 `400 cannot translate request`。这是一处已复现的实际原因，不能仅归因于思考强度。
- Messages 转换现在保留中途 system/developer 文本及其顺序。Chat 协议沿用现有 developer → system 的转换；非文本 system/developer、未知角色和不支持的内容仍明确拒绝。
- 思考意图优先采用显式 `output_config.effort`；保留 max，不再统一替换为 xhigh。只有没有显式档位时才根据 thinking budget 推导 low/medium/high；adaptive 的默认意图为 high。未知档位不传给上游。
- 实际发送前，通过共享模型能力解析器检查目标档位。支持的档位保留；不支持的档位选择不高于请求的已支持档位；明确不支持思考或明确空列表则移除 effort。未知能力保持原意图。消息、模型 ID 和其它 reasoning 字段保持原样。
- 请求诊断记录适配后的真实出站参数。错误提示细分未知角色、工具定义、工具参数、内容和结构错误。原生 Messages 请求沿用原协议，不进入此转换器。
- cc-switch 项目仅作只读参考。最新 DeepSeek 官方文档已提供 Chat 和 Responses 接口，不能依据旧版预设认定用户选择 Responses 错误；本轮未改写用户的协议选择。参考：<https://api-docs.deepseek.com/guides/responses_api>。

### 验证

- Rust 全库串行验收：534 项通过、5 项忽略、0 失败；local_workflows 9 项通过。日志：`.runtime-test/claude-router-final-rust-serial.log`。新增安装 CLI 回归属于需显式选择的 ignored 测试，已单独运行并通过。
- 全库一次并发复跑出现 `models_dev::refresh_failure_never_replaces_valid_data` 失败；该测试依赖进程共享且最多 32 项的目录缓存，其他并发测试加载临时目录可能淘汰它的条目。之前同批回归通过，串行复跑也通过；没有为掩盖问题修改缓存实现。原始日志保留在 `.runtime-test/claude-router-final-rust.log`。
- 安装 CLI → 本地 Router → 本地 mock 上游：四个型号各覆盖 Chat/Responses，8 次均收到正常文本回复；max 在 DeepSeek Flash、GPT-6 Luna、GLM-5.3 Flash 中保留，在 Gemini-3.8 Flash 中按目录降为 high。诊断中的已发送档位与 mock 实际收到的值一致；账本记录 8 次成功、160 输入 token、40 输出 token。日志：`.runtime-test/claude-router-cli-final.log`；测试：`router_proxy/claude_cli_tests.rs`。
- Router HTTP 回归另外覆盖上述实际消息结构的流式与非流式返回、凭据替换、客户端配置启动 Router，以及 Codex Router 关闭后 Claude Router 仍运行。思考回归覆盖显式档位、budget、未知能力、不支持思考、空列表、降档和消息保留。
- 前端交互回归 61 条通过，日志：`.runtime-test/claude-router-frontend.log`。包含模型全部取消、空选择阻止应用和重新选择；本轮模型选择截图位于 `.runtime-test/claude-router-ui/client-config/`，窄窗口截图已审阅。
- TypeScript、Vite、Windows release EXE 和 NSIS 构建成功，日志：`.runtime-test/claude-router-release.log`。交付目录：`D:/workspace/AiMaMi/.runtime-test/deliverables/AgentHarbor-20261002-claude-router/`。
- 交付 EXE 与本次 release 的 SHA256 一致；隔离数据、Codex 和客户端目录启动 8 秒存活并创建窗口，stderr 为零。检查后仅结束该测试进程，证据为交付目录中的 `startup-check.json`。
- 额外执行全库 `cargo clippy --lib --tests -- -D warnings` 未通过：当前工作区有 41 条 lint 报告，涉及既有 map_or、参数个数、引用、排序等代码；本轮新增的 Messages 转换、档位适配和 CLI 回归没有 lint 报告。本轮未扩展为全库样式清理，不声称 clippy 干净。日志：`.runtime-test/claude-router-clippy.log`。
- 上述上游回复和 token 数为本地测试数据，未用真实密钥发起外部计费请求；不能据此声称四家真实上游均完成验收。原厂特有的托管工具和签名思考内容不属于本轮跨协议转换支持范围。

### 随包保留的配置/UI修改

- 已恢复到接管前基线的受管字段不再误报冲突，可直接重新配置；显式确认恢复可处理冲突受管字段，退出时自动恢复仍遵循严格检查。模型数组撤回保留独立用户行。对应 Rust 回归已通过。
- 冲突时恢复按钮可用，并明确说明备份及恢复受管字段；模型区新增全部取消和选择数量。直接在冲突状态切换新配置的交互尚未完成，本轮按用户最新优先级先完成 Router，不能将该项标为全部解决。

## 配置冲突更新闭环与路由复审（2026-10-03）

- 上一节记录的“直接在冲突状态切换新配置尚未完成”现已完成。冲突不再禁用切换/更新；预览明确显示将替换冲突受管字段，应用时使用同一确认语义，无需放弃接管。当前文件先备份，原还原点保留；预览后文件再变化仍拒绝写入。
- Claude 在已配置列表内切换顶层 model（包含 [1m]/[1M]）不再误报外部冲突；密钥、网关、角色映射继续严格检查。
- 修正 rebaseline：只把实际外部冲突字段采用为新还原值，先撤回旧注入再重建，使精确快照恢复和字段合并恢复使用相同基线。更新后的 original 同时保留用户新增的模型数组行，避免精确恢复丢失该数据。
- 前端弹窗使用刷新后的 ClientStatus，冲突状态改变后清除旧预览。新增冲突更新、restore/rebaseline/reapply 后再次配置的完整交互，检查没有调用 release。
- client_config 定向测试 64 项通过；全库串行 538 项单元测试通过、5 ignored、0 失败；local_workflows 9 项通过。前端回归 63 条通过。日志：`.runtime-test/client-config-conflict-rust.log`、`client-config-conflict-final-rust.log`、`client-config-conflict-frontend.log`。
- TypeScript/Vite、Windows EXE 和 NSIS 安装包构建通过，日志：`.runtime-test/client-config-conflict-release.log`。新交付目录为 `.runtime-test/deliverables/AgentHarbor-20261003-config-conflict/`。
- 路由复审确认另外 3 项未修复问题：思考内容续轮丢失、交错工具/文本生成非法 Chat 历史、流式转换失败未反映到诊断及成功计数。详见 `docs/CLAUDE_ROUTER_REVIEW_20261003.md`；本次交付不宣称路由已完全验收。
- 新交付 EXE 的 SHA256 与 release 一致；隔离数据、Codex 和客户端目录启动 8 秒存活，窗口句柄非零、stderr 为零。仅停止本轮测试进程；证据保存于交付目录 `startup-check.json`。未测试安装升级或真实计费上游。

## Claude 路由三项审查问题修复（2026-10-03）

- 上节“3 项未修复问题”已全部完成修复：新增按路由/凭据/模型/会话历史隔离的思考续轮缓存；同一 assistant 轮的交错文本和工具合并成合法 Chat 工具组；转换失败与客户端中断参与诊断和账本成功判断，已发生的 token 用量保留。
- 真实 Claude Code 会重写动态 system 消息，最初基于原始前缀的匹配在真实 CLI 测试中失败；修正为规范化的用户/助手历史身份，动态消息仍完整转发，并补回归。
- 最终 Rust 全库串行 547 passed、0 failed、6 ignored；local_workflows 9 passed；前端回归 63 PASS。两项需显式启用的 installed CLI 测试另外运行通过：普通回复 8 次请求，真实 Read 工具执行与续轮 8 组、16 次请求，四模型均覆盖 Chat/Responses。
- 修复后复审未发现新增阻断问题。测试均为本地模拟上游，没有用真实密钥调用计费接口。思考缓存限当前进程、512 条/16 MiB、闲置 1 小时，重启或淘汰后的旧续轮不能保证恢复。详见 `docs/CLAUDE_ROUTER_FIXES_20261003.md`。
- 最新交付目录为 `.runtime-test/deliverables/AgentHarbor-20261003-router-fixed/`；之前 `config-conflict` 包不包含这三项路由修复。
- Windows release/NSIS 构建完成；新 EXE 与 release 哈希一致，隔离目录启动 8 秒存活并创建窗口、stderr 为零。验证后仅结束本轮测试进程，证据为交付目录 `startup-check.json`。

## DeepSeek Responses 完整历史、无账号开关与后台窗口（2026-10-03）

- 用户确认 DeepSeek Responses 在工具执行成功后返回 `The reasoning_text in the thinking mode must be passed back to the API`。上一版只保存工具回复的 reasoning，未覆盖此前普通聊天；原有工具专项也缺少这一场景。
- 现在保存所有成功 assistant 回复的完整原厂 output。Responses 按顺序恢复原始 reasoning/message/function_call item 及 ID，包含真实 `content[].reasoning_text`；Chat 从完整思考文本恢复 reasoning_content。缓存仍按本地存储、提供商、端点、凭据摘要、模型、协议、会话及历史隔离，不向 Claude 伪造签名思考块。
- 严格模拟服务会检查普通聊天和工具轮的完整思考历史及原始 ID，缺失则返回 400。真实安装 Claude Code 增加先聊天、resume 后 Read、再回传结果的测试。四型号各覆盖 Chat/Responses，新增 24 次请求与 8 次实际 Read；全部三个安装 CLI 专项累计 48 次请求、16 次实际 Read 通过。
- 无账号开关移除路由关闭时的禁用条件，支持在注入前选择。新增前端开/关测试及后端先选择、注入、恢复测试，不提前改写客户端文件。
- Windows Claude 版本探测和 Codex 后台 CLI（EXE/cmd shim）增加 CREATE_NO_WINDOW。release 主程序沿用 GUI 子系统。此源码修复针对后台子进程；启动日志正常不代表已监测所有瞬时黑框，需用户环境复测。
- 本轮最终 Rust 串行 549 passed、0 failed、7 ignored；local_workflows 9 passed；前端 63 PASS；三个 ignored 安装 CLI 专项另外通过。本地 HTTP 专项覆盖两协议与流式/非流式的直接工具、先聊天再工具及四类错误，共 24 次请求。
- 日志：`.runtime-test/claude-deepseek-history-final-rust.log`、`claude-deepseek-history-final-cli.log`、`claude-deepseek-history-frontend.log`、`claude-deepseek-history-http.log`。详细复审见 `docs/CLAUDE_ROUTER_FIXES_20261003.md`。
- 所有上游为本地模拟服务，未用真实密钥调用计费接口。新版本应新建 Claude 会话；旧版本丢失的思考数据无法补回，应用重启/缓存过期/容量淘汰后同样可能需要新会话。
- Windows TypeScript/Vite、release EXE 和 NSIS 构建成功，日志为 `.runtime-test/claude-deepseek-history-release.log`。最新交付目录：`.runtime-test/deliverables/AgentHarbor-20261003-deepseek-history/`，含直接 EXE、安装包、许可证、使用说明、验收记录和 SHA256。
- 交付 EXE 与本次 release 哈希一致：`99527F73C3D7776EBABE449840A91C2E4CB60991408C2123F8F559FD4BE3FD9E`。隔离数据、Codex 和客户端目录启动 8 秒存活，窗口句柄非零、stderr 为零。只停止本次测试进程；未覆盖用户安装目录或停止旧版。安装升级和线上计费接口未验收。

## DeepSeek Messages 地址与 Gemini 工具 schema（2026-10-03）

- 用户实测 GPT/GLM Responses 可聊天并调用工具；Gemini 的缺失 items 错误也会出现在 Messages 中。确认 Gemini 问题属于工具输入 schema，不能归因于单一外层协议。
- DeepSeek 官方 Messages 要求 `/anthropic` 基址，此前通用根地址造成直接/路由接入请求地址不正确。共享地址解析现限定官方精确主机和标准根，供注入、Router、连通性测试复用，第三方与自定义前缀保留。
- Gemini 路由出站对三种协议递归补齐缺少的数组 items（使用空 schema 保持元素不限类型），不改已有类型、default/examples 或实际工具参数。非 Gemini 请求保持原字节。直接接入绕过 Router 的 schema 适配。
- 单模型协议可以覆盖提供商默认值；新增“跟随默认”选项，允许取消覆盖并保存 null。没有自动改动用户现存协议或凭据。
- 最终 Rust 串行 553 passed、0 failed、8 ignored；local_workflows 9 passed；前端 64 PASS。四项安装 CLI 专项另行通过，共 52 次模型请求、18 次实际 Read。新增 DeepSeek Messages 的直接/路由 Read 续轮，以及 Gemini 三种协议下共 6 次 HTTP 请求，均通过。
- 详细实现、复审和边界见 `docs/CLAUDE_PROTOCOL_SCHEMA_FIXES_20261003.md`；日志前缀 `.runtime-test/claude-model-schema-`。所有上游为本地模拟，未验收用户线上中转的 Gemini 转换实现。
- Windows TypeScript/Vite、release EXE 与 NSIS 构建成功。最新交付目录 `.runtime-test/deliverables/AgentHarbor-20261003-protocol-schema/`，EXE SHA256 为 `38C05F66EDC3E2B0813DA5184D591E7AC43EF93067C34AA4EB92371945B26395`，与 release 一致。隔离目录启动 8 秒存活、窗口句柄非零、stderr 为零；只停止本轮测试进程。
