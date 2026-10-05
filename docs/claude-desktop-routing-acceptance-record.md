# Claude Desktop 路由模式实施验收

日期：2026-10-05。设计计划：[路由接入实施计划](../plan/claude-desktop-routing-integration-execution-plan.md)。实施起点：`596c856`，基础路由提交：`ea7cb17`；本轮最终提交、tree 与产物 SHA256 见交付目录 `build-manifest.json`。

**结论：R1–R3 已实现，本轮 Fast 上下文引用与三协议 1M 的完整 Rust、前端和构建回归通过。R4 仅有用户截图提供的 Chat 普通多轮部分证据，完整真机验收未完成。** 安装包是本地验收产物，三协议模拟测试不代表真实客户端行为已全部通过。

## 实现与结构

- 新 `routes.rs` 管理角色引用和当前卡片资格，`gateway.rs` 只提供已提交网关快照。没有复制 Provider 或推理凭据数据库。
- 受保护的 State 是 token/routes/generation 唯一权威，原子替换就是发布点。Pending 保存整代 before/next；未提交恢复旧代，已提交核验并清理新代。
- HTTP namespace 分派在鉴权之前，Desktop 与普通路由 token 互不越权，请求不会创建或轮换 token。
- Desktop 请求复用既有三协议转换、SSE、工具和账本，冻结同一份 Catalog Arc 供资格、转换能力和价格使用。
- 新 `ClientScope` 区分客户端、配置、角色、路由版本，保留原客户端旧 scope 行为；Desktop 不写 Codex seen/binding，不继承全局 failover。
- 用量及诊断增加可选身份字段，JSONL 旧记录兼容，费用继续按真实出站 model 聚合。
- 共享 listener 的 lease/pin 分别约束预览与已提交配置，不与 Codex 开关绑定。未监听提示与 stale 状态分开。
- 完整退出、托盘退出、信号和更新重启共用前置协调；正常关窗仍隐藏。macOS 延迟 helper 只在恢复成功后创建；Windows 重启在前置完成后调用 `request_restart`。
- 本轮 Fast 匹配优先完整型号或显式目录引用；完整匹配缺失时，末尾 `-fast` / `_fast` / `@fast` / `:fast` 仅参考同版本基础型号的 context。真实 ID、展示名、输入/输出、思考字段和独立价格身份不随该引用变更。
- Desktop 路由的 1M 资格改为有据的生效 context ≥ 1,000,000；Messages/Chat/Responses 共用此资格。Chat/Responses 不添加虚构 1M 参数，不向上游传 Claude beta header；普通客户端边界保留。

## 自动化结果

下列记录为本轮新增规则的最终执行结果，打包和隔离启动证据随交付目录保存：

| 命令 | 结果 | 日志 |
| --- | --- | --- |
| `pnpm test:rust` | 789 lib + 11 Desktop 集成 + 9 本地集成通过；21 个 ignored（含真机、手工及辅助用例），未计为通过 | `.runtime-test/desktop-fast-1m-rust.log` |
| `pnpm build` | TypeScript 与 Vite 通过 | `.runtime-test/desktop-fast-1m-build.log` |
| `pnpm test:frontend` | 静态契约及全量 Playwright 通过；三协议 1M 单击选择、容量不足、保存后失去能力均覆盖 | `.runtime-test/desktop-fast-1m-frontend.log` |
| 诊断/用量前端回归 | 来源筛选、Desktop 映射原因与 Fast 来源文案通过；并入全量前端 | `.runtime-test/desktop-fast-1m-frontend.log` |
| `pnpm tauri build --bundles nsis` | EXE、NSIS、SHA256 和隔离启动最终结果见交付目录 manifest | `.runtime-test/desktop-fast-1m-release.log`、交付目录 `build-manifest.json` / `startup-check.json` |

所有配置和 Key 均为隔离根合成数据，HTTP 上游为本地 mock；浏览器测试拦截外部网络。忽略的真机用例没有计为通过。本轮截图目录为 `.runtime-test/desktop-fast-1m-ui/`，交付目录附相应合成截图及完整日志。

## 计划逐项对应

“已验证”指本轮完整回归中自动化证据通过；“部分”表示剩余组合没有专用测试，并不等于实现已知失败。R-A19 的本地 HTTP 矩阵覆盖三协议 × 普通/1M × 非流/SSE × 两轮工具往返，共 24 次成功请求；开关关闭及当前容量下降时，旧 marker/beta 被拒绝且不触发上游。

| ID | 状态 | 自动化证据和边界 |
| --- | --- | --- |
| R-A01 | 已验证 | 原 direct/official/reapply/tombstone 工作流保留；router 字段缺省兼容。旧安装真实升级未做。 |
| R-A02 | 已验证 | `router_each_role_resolves_its_card_and_invalid_current_cards_do_not_fallback`、profile 测试及 UI request/store 比对；角色使用不同卡片/同卡片模型，原默认配置未改。 |
| R-A03 | 已验证 | `router_resolver_requires_enabled_included_or_the_exact_manual_default`、HTTP 空推理 Key 拒绝；余额凭据不参与推理。 |
| R-A04 | 已验证 | `router_profile_exposes_only_safe_aliases_and_a_local_credential`、models/DTO 断言；真实型号和上游 Key 不写 profile。 |
| R-A05 | 已验证 | `router_roles_are_exact_unique_and_do_not_autofill`、未知/旧 alias HTTP 400；空档无隐式回填。 |
| R-A06 | 已验证 | `desktop_namespace_routes_three_protocols_tools_and_actual_model_accounting` 覆盖两端点、token 越界/轮换/x-api-key 拒绝和请求不创建凭据。 |
| R-A07 | 已验证 | 路由票据测试和 HTTP rotated-key；新推理 Key/URL 下一请求生效，余额变更不重写 Desktop。 |
| R-A08 | 部分 | `desktop_listener_leases_pin_preview_and_managed_ports`、票据 lease/epoch 测试、启动占端口保持文件测试；尚无真实 Desktop 四文件与实际重绑的整组端到端验证。 |
| R-A09 | 已验证 | `preview_reservations_expire_without_stopping_another_client`、UI 取消/迟到响应/切模式回归；保留其他 CLI 的服务需求。 |
| R-A10 | 部分 | 票据绑定文件/角色资格/协议/1M，Key/有效 URL/余额/无关卡片变化回归已覆盖；价格变化由投影排除，尚无单独价格变更测试。 |
| R-A11 | 已验证 | `router_switches_preserve_the_first_takeover_baseline_and_release_is_blocked` 及原 rebaseline/first-baseline 测试；正常切换保留首次基线。 |
| R-A12 | 部分 | `router_each_external_write_and_state_commit_interruption_restores_the_old_generation` 覆盖 4 次写前及 State 提交前 5 个中断点；另有 token 提交前回滚、提交后 running cleanup 及共享真实写失败测试。未注入所有 `apply_plan/save_state/pending unlink` I/O 故障组合。State 单权威消除了独立 active table 的发布失败路径。 |
| R-A13 | 已验证 | `managed_desktop_survives_codex_native_and_corrupt_preview_cleanup`、needs_server 和 lease pin；Codex Native/关闭保留 Desktop，端口不能暗改。 |
| R-A14 | 已验证 | `core::desktop_exit` 测试、退出 coordinator 及 UI 退出/更新重启；release 阻止、后台保留、停止失败重试、恢复后 cleanup 不重复停进程。真实托盘/更新/客户端仍待真机。 |
| R-A15 | 部分 | startup portBusy 组合保持 Desktop token/State/四文件，CLI 独立恢复，端口释放后原 token/端口恢复。这里是在同一进程模拟崩溃；独立进程/真实 Desktop 运行组合未验。 |
| R-A16 | 已验证 | HTTP 同 history/session/真实目标跨角色、路由版本、普通路由的私有输出隔离；seen/failover 状态无污染。 |
| R-A17 | 已验证 | Messages/Chat/Responses 各非流及 SSE 的工具定义、执行回传、多轮；6 种形态均用本地模拟上游。 |
| R-A18 | 部分 | HTTP bad-tool/truncated/upstream-error/bad-json/cancel 只落账一次并保留已报用量；共享 diagnostics incomplete 终止测试存在，Desktop 入口尚无专门 incomplete HTTP 案例。 |
| R-A19 | 已验证 | 三协议有据 context ≥ 1M 可选，能力不足/失去能力与 off/旧 marker/beta 拒绝，单击 checkbox 默认关闭；Chat/Responses 无虚构参数、无 Claude beta，profile 每个 alias 只有一条记录。这不表示 Desktop 菜单只有一项：开启后客户端生成普通与 1M 两项。普通路由原有协议限制单独保留。 |
| R-A20 | 已验证 | HTTP 真实 fixture-model 计价，零价/未知价/缓存 read/write 分拆断言；trace/ledger 保留 requested/resolved。 |
| R-A21 | 已验证 | 旧 JSONL 身份缺省、compaction 保留可选身份并按实际 model 汇总；来源未知不补造。 |
| R-A22 | 部分 | 路由表单 390/1200 深浅主题、键盘、失败重试；Desktop 卡片 80–150% 缩放。未对路由表单单独穷举全部缩放矩阵。 |

## 用户证据与真机缺口

实现会话没有执行真实 Desktop 接管、进程控制或计费请求。用户随后提供了真实 Desktop/Cowork 界面及请求诊断截图：Command Code 目标显示普通至少两轮；诊断记录请求 alias `claude-sonnet-4-6` → 实际型号 `deepseek/deepseek-v4.1-flash`，协议 `chatCompletions`，HTTP 200，上游回显与真实出站型号一致。这些是 Chat 通路和模型映射的部分真机证据。仍缺完整 Desktop 版本/安装渠道、受管文件核对、逐次独立抓包及工具/SSE/1M 验收，不能直接把 R-L02 标为全部通过。

| 项目 | 当前证据 |
| --- | --- |
| R-L02：Chat 普通多轮 | Desktop/Cowork 的 Command Code 目标至少两轮；诊断协议 `chatCompletions`、HTTP 200，仍缺逐次独立抓包与完整版本记录 |
| R-L02：Messages / Responses | 未完成真实客户端验收 |
| R-L03 / R-L04：工具 / SSE | 未完成真实客户端验收；截图中的回答不能证明工具执行或逐段流式 |
| R-L05：1M | 后续用户截图确认开启后生成普通与 1M 成对菜单项；实际请求标记、本地预算与百万 token 接受能力仍未验 |
| R-L01 / R-L09：alias 与型号身份 | 诊断显示 `claude-sonnet-4-6` → `deepseek/deepseek-v4.1-flash`，上游回显与出站一致；完整 profile/菜单读取及实际价格快照核对未完成 |
| R-L06–R-L08 | 未完成独立启停、退出恢复与故障恢复的完整真机验收 |

后续需记录 Windows Desktop 的版本/安装渠道和明确选择的卡片，逐项验收：角色菜单与请求 alias、三协议普通/多轮对话、工具执行回传、逐段 SSE/截断/取消、1M marker、其他客户端开关与托盘后台、模式切换与完整退出恢复、崩溃/端口冲突、实际价格身份。不得把现有直连 L01–L10 结果迁移为路由真机结果。

macOS/Linux、Fable、Cowork、托管工具、三协议 1M 和百万 token 压力测试仍无新增实测证据。

## 自审修正

2026-10-05 用户再次报告开启 1M 后同名菜单成对。核对发现这不是新增重复 profile 记录：`supports1m: true` 的客户端 picker 明确返回基础项及 `${id}[1m]` 项，二者继承同一个 `labelOverride`。旧直连修复只调整默认关闭和交互，并没有实现“开启后单项”。路由说明及此前交付描述把一条 profile 记录推导成单项菜单，结论错误，本次更正。`prefer1m` 只改默认选择，后缀 shorthand 同样产生两项；核对版本 2.19675.0 没有仅保留 1M 项的配置字段。静态证据沿用 `.runtime-test/desktop-onem-contract/evidence.json`，本次未改写真实 Desktop 配置或安装文件。

本次重新计算已安装 `app.asar` 的 SHA256，与上述静态证据完全一致；只读核对受管 profile 的模型名称及布尔标记，得到 4 条记录、4 个唯一 alias，四项 `supports1m` 均为 true，与截图中 4 组成对菜单一致。检查未输出本地 token 或上游凭据，也未停止客户端、执行 SDK 或发起推理请求。

本轮自审修正了上游 `Bearer` 前缀重复拼接、请求目录跨版本、localhost 直连误判 Router、监听健康混入 stale、过期预览 pin 临时阻挡操作、恢复已提交 cleanup 后再次要求停 Desktop、更新保持后台后的重复安装，以及本地 token 的 Debug 输出风险。

前端诊断筛选测试补了渲染等待，用量来源筛选补明确的可访问名称；退出恢复测试按 JSON 内容核对共享配置，仍对不可写/停止失败路径逐字核对零修改。

本轮还将 `desktop_role_mapping`、`speedVariantReference` 和 `catalogSpeedReference` 转为界面本地化文案，新增三协议 1M 可选/能力不足/已保存选择失去能力的回归断言。Fast 解析用例要求完整匹配优先、只参考同版本 context、原 ID/输出/思考/价格身份保留；同时修正目录提示消费，避免把基础型号的 canonical/思考选项/interleaved 误展示为 Fast 能力。最终通过计数见上方执行表。独立只读自审未发现新增阻断问题。

新增 Fast 文案断言曾因整行文字使用独立文字精确定位而超时，已改为按局部文案定位；针对性及全量前端重跑通过。

回归中一次既有 Sub2API 本地 mock 出现 `newapi.errors.network`，完整重跑结果以最终日志为准；没有将失败记录改写成通过，也没有为本功能修改余额协议。
