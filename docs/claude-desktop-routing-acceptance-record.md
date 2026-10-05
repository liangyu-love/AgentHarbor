# Claude Desktop 路由模式实施验收

日期：2026-10-05。设计计划：[路由接入实施计划](../plan/claude-desktop-routing-integration-execution-plan.md)。实施起点：`596c856`；最终提交与产物 SHA256 见交付目录 `build-manifest.json`。

**结论：R1–R3 已实现，主要自动化回归通过；R4 的真实 Desktop 验收未完成。** 安装包是本地验收产物，三协议模拟测试不代表真实客户端行为已全部通过。

## 实现与结构

- 新 `routes.rs` 管理角色引用和当前卡片资格，`gateway.rs` 只提供已提交网关快照。没有复制 Provider 或推理凭据数据库。
- 受保护的 State 是 token/routes/generation 唯一权威，原子替换就是发布点。Pending 保存整代 before/next；未提交恢复旧代，已提交核验并清理新代。
- HTTP namespace 分派在鉴权之前，Desktop 与普通路由 token 互不越权，请求不会创建或轮换 token。
- Desktop 请求复用既有三协议转换、SSE、工具和账本，冻结同一份 Catalog Arc 供资格、转换能力和价格使用。
- 新 `ClientScope` 区分客户端、配置、角色、路由版本，保留原客户端旧 scope 行为；Desktop 不写 Codex seen/binding，不继承全局 failover。
- 用量及诊断增加可选身份字段，JSONL 旧记录兼容，费用继续按真实出站 model 聚合。
- 共享 listener 的 lease/pin 分别约束预览与已提交配置，不与 Codex 开关绑定。未监听提示与 stale 状态分开。
- 完整退出、托盘退出、信号和更新重启共用前置协调；正常关窗仍隐藏。macOS 延迟 helper 只在恢复成功后创建；Windows 重启在前置完成后调用 `request_restart`。

## 自动化结果

最终执行日志：

| 命令 | 结果 | 日志 |
| --- | --- | --- |
| `pnpm test:rust` | 779 lib + 11 Desktop 集成 + 9 本地集成通过；21 个 ignored（含真机、手工及辅助用例） | `.runtime-test/desktop-router-rust-verified.log` |
| `pnpm build` | TypeScript 与 Vite 通过 | `.runtime-test/desktop-router-build-verified.log` |
| `pnpm test:frontend` | 静态契约及全量 Playwright 通过 | `.runtime-test/desktop-router-frontend-verified.log` |
| 诊断/用量定向回归 | 来源筛选、alias/真实型号、零价通过 | `.runtime-test/desktop-router-diagnostics-verified.log` |
| `pnpm tauri build --bundles nsis` | x64 GUI EXE 与 NSIS；隔离启动结果见产物 manifest | `.runtime-test/desktop-router-release.log` |

所有配置和 Key 均为隔离根合成数据，HTTP 上游为本地 mock；浏览器测试拦截外部网络。忽略的真机用例没有计为通过。截图在 `.runtime-test/desktop-router-ui/`，交付目录附相应合成截图及完整日志。

## 计划逐项对应

“已验证”指表中自动化证据通过；“部分”表示剩余组合没有专用测试，并不等于实现已知失败。

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
| R-A19 | 已验证 | native Messages 1M 能力/菜单、off/跨协议/旧 marker/beta、UI 单击 checkbox；profile 每个 alias 只有一条记录。 |
| R-A20 | 已验证 | HTTP 真实 fixture-model 计价，零价/未知价/缓存 read/write 分拆断言；trace/ledger 保留 requested/resolved。 |
| R-A21 | 已验证 | 旧 JSONL 身份缺省、compaction 保留可选身份并按实际 model 汇总；来源未知不补造。 |
| R-A22 | 部分 | 路由表单 390/1200 深浅主题、键盘、失败重试；Desktop 卡片 80–150% 缩放。未对路由表单单独穷举全部缩放矩阵。 |

## 真机缺口

R-L01–R-L09 **本轮全部未执行**。没有修改真实 Desktop 配置、控制真实进程或发送真实计费请求。

后续需记录 Windows Desktop 的版本/安装渠道和明确选择的卡片，逐项验收：角色菜单与请求 alias、三协议普通/多轮对话、工具执行回传、逐段 SSE/截断/取消、1M marker、其他客户端开关与托盘后台、模式切换与完整退出恢复、崩溃/端口冲突、实际价格身份。不得把现有直连 L01–L10 结果迁移为路由真机结果。

macOS/Linux、Fable、Cowork、托管工具、跨协议 1M 和百万 token 压力测试仍无新增实测证据。

## 自审修正

本轮自审修正了上游 `Bearer` 前缀重复拼接、请求目录跨版本、localhost 直连误判 Router、监听健康混入 stale、过期预览 pin 临时阻挡操作、恢复已提交 cleanup 后再次要求停 Desktop、更新保持后台后的重复安装，以及本地 token 的 Debug 输出风险。

前端诊断筛选测试补了渲染等待，用量来源筛选补明确的可访问名称；退出恢复测试按 JSON 内容核对共享配置，仍对不可写/停止失败路径逐字核对零修改。

回归中一次既有 Sub2API 本地 mock 出现 `newapi.errors.network`，完整重跑结果以最终日志为准；没有将失败记录改写成通过，也没有为本功能修改余额协议。
