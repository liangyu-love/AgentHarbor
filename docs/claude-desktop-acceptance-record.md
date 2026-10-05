# Claude Desktop 直连验收记录

本记录对应 Claude Desktop 直连功能的实施验收，按[实施计划](../plan/claude-desktop-direct-integration-execution-plan.md)第 9.1 节（A01–A40）、第 9.3 节（L01–L10）与第 12 节编写。配套的用户说明见[直连配置说明](./claude-desktop-direct-connection.md)。

## 截图反馈修复（2026-10-05）

本轮对应“重启后仍显示需要重启、只读预览要求退出、1M 菜单成对、预览未显示 Opus 4.7、重复说明过多”的反馈。只使用临时根与 mock 验证，没有改动用户真实 Desktop 或中转配置。

- 重启命令在同一写锁内重验会话与 pending；选定安装的停止、启动都成功后持久清除待启动标记，失败保留。无改动重应用保留同安装已有标记，不继承另一个安装的 tombstone。
- 预览只读可在运行时执行；配置写入仍检查完全退出，且拒绝发生在 pending/backup/外部写入之前。提交失败时保留表单、重新签预览，不重复使用已消费票据。
- profile 预览现在投影真实 before 与 after，Opus 4.7 等新选择显示在主区；原始模型 ID 保持不变。只有已知契约字段参与技术预览，密钥及模型对象未知扩展不回传。
- 卡片移除重复独立性说明、正常“没有未完成事务”等文案；主区显示供应商与模型，路径/备份与原始 diff 收入详情。更新配置保留已保存的原始模型、显示名和 Auto/Off 选择。
- 已保存模型被删除或排除后，仍显示其原始选择及原因并提供移除入口；先处理失效选择再预览，避免隐藏旧 ID 持续阻断更新。
- 对照 CC Switch `claude_desktop_config.rs:263` 和 `docs/release-notes/v3.20.4-zh.md:218`：一个 `supports1m: true` 项由 Desktop 生成普通和 1M 两项，属于原生菜单契约。新选择默认不追加 1M，开启后仍按有效能力判断；既有配置不被自动改写。

最终自动化结果：

| 检查 | 结果 |
| --- | --- |
| `pnpm test:rust` | exit 0；753 项 lib 通过（10 ignored）、11 项 Desktop workflows 通过（11 ignored）、9 项 local_workflows 通过，0 失败 |
| `pnpm test:frontend` | 最终源码 exit 0；包含 10 组 Desktop 契约检查与 14 组 Desktop Playwright 行为回归 |
| `pnpm exec tsc --noEmit` | 最终前端源码 exit 0 |
| `pnpm build` | exit 0；最终可执行产物的重构建与隔离启动结果另随交付记录 |
| 视觉检查 | 1280px、480px/150% 缩放、Opus 4.7 预览与失效选择移除；主要内容及操作无横向溢出 |
| 修复后复查 / `git diff --check` | 无剩余阻断项 / exit 0 |

先前真机验收范围及发布缺口仍按下文记录；本轮 mock 不替代真实 Desktop 或跨平台验收。外部手动启动不会自动将持久标记升级为已确认重载，当前由本项目成功启动来确认。

## 当前结论：复审修复完成（2026-10-04）

六个实施提交 `e8af8f8..5030efd` 的核心直连链路已实现，但复审确认恢复、所有权、进程与隔离边界存在缺陷，不能按“全部完成”验收。本轮已按优先级修复并复审，使用临时根、合成凭据、mock HTTP 与前端 fixture；没有修改真实 Desktop 配置、真实 Provider 或执行 ignored/live 用例。

补充回归覆盖：失败重新应用恢复本次 before、首次应用后的外部改动保留 pending、外部 pointer/profile 冲突零写入、无 journal 禁止切官方、提交后漂移保留 journal、结束管理状态、进程身份与隔离边界、首次/多安装发现、原始模型选择与 stale、旧检测结果和迟到响应、重启后票据失效、极端票据时间戳安全失效、坏 JSON 下备份后释放管理、跨进程文件锁竞争及崩溃释放。`pnpm test:rust` 已接入 `claude_desktop_workflows`，不再将单独运行结果混入原默认入口。

修复后自动化结果（最终源码）：

| 命令 | 结果 |
| --- | --- |
| `pnpm test:rust` | exit 0；742 项 lib 通过（10 ignored），11 项 Desktop workflows 通过（11 ignored），9 项 local_workflows 通过，0 失败 |
| `pnpm test:frontend` | exit 0；包含 10 组 Desktop 配置契约检查与 12 组 Desktop Playwright 行为回归 |
| `pnpm exec tsc --noEmit` | exit 0 |
| `pnpm build` | exit 0；既有动态/静态 import 分块提示仍存在，无构建错误 |
| `git diff --check` | exit 0 |

本轮修复后自审未发现剩余阻断项。事务失败仅回滚本笔 before；第三值保留 journal 供恢复；无有效管理记录不能按 profile 外形恢复/删除；会话路径、隔离来源和进程操作绑定到确认安装。原始模型选择、地址/能力 stale、检测结果有效期和一次性票据均有补充回归。释放管理可备份并保留坏 JSON 原文。跨进程锁改用持久文件上的系统排他锁，实际隔离子进程验证竞争及崩溃释放。升级测试期间不支持旧 PID 锁协议程序与新程序混合并行写入，应先退出旧实例。

已有真实证据以第三节 L 表格及后续执行记录为准：L01/L02/L03/L05/L10 核心链路已有实施证据，L09 为 mock 限制路径；L04/L06/L08 仍部分通过，L07 仅文件层通过。macOS/Linux 与 Cowork 的完整支持仍未验收。**补充自动化通过不等于计划全部发布门槛通过。**

下文保留实施阶段的历史日志、测试数量与当时状态；“未执行”“尚未提交”等时点描述不代表当前现状。早期 `pnpm test:rust` 实际只覆盖 lib 和 local_workflows，Desktop workflows 是独立命令执行。

记录口径：

- 只写能指向具体用例、具体命令输出或磁盘可复核事实的结论。
- 自动化用例的“存在”与“通过”分开表述：第 2.1 节列出实际执行的命令与结果，2.3 节表格里的状态表示该场景是否有可指向的用例。
- 真实验收按第三节及追加记录区分完整、部分和文件层证据，不把 mock 结果升级为真机结论。

## 一、执行环境

| 项目 | 取值 | 证据 |
| --- | --- | --- |
| 仓库 | `D:\workspace\AiMaMi` | 工作目录 |
| 基线提交 | `e8af8f88460888ac5256817f3be96e48d8168e6f`（短 `e8af8f8`，`fix(router): preserve tool identity and conversation affinity`） | `git rev-parse HEAD` |
| 本机 Desktop 现状 | **已被本次实施接管**：`configLibrary` 中 `appliedId=d918233d-…3722a`（AgentHarbor），CC Switch 条目仍保留；Desktop 于 2026-10-04 19:13:58 以该 profile 启动。可用界面上的「恢复接管前配置」还原（执行前副本见 3.1b）。 | 3.1b |
| 工作区状态 | 功能实现**尚未提交**。新增：`src-tauri/src/core/claude_desktop/`（constants、paths、provider、profile、transaction、probe、lock、mod、types、tests）、`src-tauri/src/commands/claude_desktop.rs`、`src-tauri/tests/claude_desktop_workflows.rs`、`src/components/newapi/claude-desktop-config-card.tsx`、`src/components/newapi/claude-desktop-config-dialog.tsx`、`src/lib/claude-desktop.ts`、`src/types/claude-desktop.ts`、`scripts/test-claude-desktop-config.mjs`、`plan/claude-desktop-direct-integration-execution-plan.md`；修改：`package.json`、`src-tauri/src/lib.rs`、`src-tauri/src/commands/mod.rs`、`src-tauri/src/core/mod.rs`、`src-tauri/src/core/router.rs`、`src/components/newapi/newapi-page.tsx`、`src/lib/api.ts`、`src/lib/query-keys.ts`、`src/locales/zh.json`、`src/locales/en.json` | `git status --porcelain` |
| 平台 | Windows 11（`Microsoft Windows NT 10.0.26200.0`），AMD64（Windows x64） | `[System.Environment]::OSVersion.VersionString`、`$env:PROCESSOR_ARCHITECTURE` |
| 安装渠道 | MSIX / Microsoft Store（`windowsStore=true`、`source=windowsStore`），包族 `Claude_pzs8sxrjxfjjc` | `%LOCALAPPDATA%\Claude-3p\logs\main.log` 里的 `[MSIX] Package family from native API` 与 `[updater] MSIX detected` 行 |
| 应用数据目录 | `%USERPROFILE%\.codex\agentharbor`（`CodexPaths::app_data_dir`）；目录存在，但**没有** `claude-desktop` 子目录 | 只读目录列举 |
| Desktop 版本 | 计划里写的 `1.12603.1` 是安装时的日志记录；按修正后的探测口径（每个渠道日志里**最后**一条 `appVersion`，且**优先 3P 渠道**），本机很可能为 `2.19675.0`。两者都只是日志证据 | 见下方“版本证据说明” |

**版本证据说明（实现已按此修正）**

- `%LOCALAPPDATA%\Claude-3p\logs\main.log` 是累积日志：第 2 行是 `appVersion: '1.12603.1'`，从第 160 行起继续记录 `1.19367.0` → `1.20186.x` → `2.2553.x` → `2.9939.2` → `2.16120.0` → `2.19675.0`，文件最后写入时间为 2026-10-04 17:52（本地时间）。首条只是安装时的那一次启动。
- `%LOCALAPPDATA%\Claude\logs\main.log`（常规渠道）首条是 `2.2553.0`，最后写入 2026-09-18。
- [mod.rs](../src-tauri/src/core/claude_desktop/mod.rs) 的 `read_version_from_logs` / `read_app_version` 现在扫描整个日志、保留**最后**一条 `appVersion`，并**优先**读取 3P 根目录的日志（常规根只作回退）；单引号、双引号与裸值三种形态都能解析。
- 因此本机显示的探测版本很可能是 `2.19675.0`（3P 渠道日志的最后一条），**不是** `1.12603.1`。
- 这仍然只是日志推断：没有与正在运行的 Claude Desktop 进程核对，也没有版本接口可以交叉验证。**L01（该版本是否接受最小 profile）与 L09（鉴权/布局限制）的判定必须在真实验收前重新核实运行版本。**

**当前磁盘状态（只读观察，未做任何修改）**

| 文件 | 内容摘要 |
| --- | --- |
| `%LOCALAPPDATA%\Claude\claude_desktop_config.json` | `deploymentMode` 为 `"3p"` |
| `%LOCALAPPDATA%\Claude-3p\claude_desktop_config.json` | `deploymentMode` 为 `"3p"`，另有 `coworkUserFilesPath` 与 `preferences`（含 `epitaxyPrefs`、`sidebarMode` 等键） |
| `%LOCALAPPDATA%\Claude-3p\configLibrary\_meta.json` | `appliedId = "00000000-0000-4000-8000-000000157210"`；`entries = [{ "id": "00000000-0000-4000-8000-000000157210", "name": "CC Switch" }]` |
| `%LOCALAPPDATA%\Claude-3p\configLibrary\` | 只有 `_meta.json` 与 `00000000-0000-4000-8000-000000157210.json`（CC Switch 自己的 profile）；**没有** `d918233d-0a41-47a1-a29f-6d610a03722a.json`（本工具的 profile） |

**声明**

- 本次实现**没有修改任何 Claude Desktop 文件**。以上全部是只读观察结果；工作区改动只发生在仓库内的源码、文案与脚本。
- 本机 `%USERPROFILE%\.codex\agentharbor\` 下没有 `claude-desktop` 目录，也没有 `state.dat` / `pending.dat` / `previews.dat` / `backups`，说明本功能**从未在本机执行过一次真实接管**。
- **真实 Claude Desktop 验收（L01–L10）没有执行**，第三节全部为“未执行”。

## 二、自动化验收结果

### 2.1 执行的命令与结果

| 命令 | 结果 |
| --- | --- |
| `pnpm test:rust` | 693 项 lib 单测通过 / 0 失败，另 9 项 `local_workflows` 通过；其中 `core::claude_desktop` 内 65 项单测、`tests/claude_desktop_workflows.rs` 9 项集成用例 |
| `pnpm test:claude-desktop-config` | 10 个检查组全部 PASS（记录撰写期间在 2026-10-04 09:54Z 独立重跑，同样 10/10 PASS、EXIT=0） |
| `pnpm test:frontend` | exit 0（契约脚本阶段 + Playwright 套件：既有页面回归 + 新增 5 项 Desktop 卡片布局/键盘断言） |
| `pnpm build` | exit 0 |
| `cargo test --test claude_desktop_workflows -- --ignored real_provider_diagnostic` | 只读诊断；结果见 3.1 |
| `cargo test --test claude_desktop_workflows -- --ignored real_bearer_probe` | `/v1/models` 裸 Bearer 200（见 3.1） |
| `cargo test --test claude_desktop_workflows -- --ignored real_messages_bearer` | `/v1/messages` 裸 Bearer 200（见 3.1，产生极少量计费） |
| 真实探测（只读，不启动进程） | 发现结果 `candidates=1 default=true notes=[]`；启动方式 `MsixActivation`，目标 `Claude_pzs8sxrjxfjjc!Claude`；探测时应用未运行 |

说明：`pnpm test:rust`、`pnpm test:frontend`、`pnpm build` 的结果来自实现冻结时的执行；`pnpm test:claude-desktop-config` 在记录撰写期间另有一次独立重跑，结果一致。撰写期间出现过一次中间态失败（`types.rs` 已加入 `writer_busy`、本地化文件尚未同步，第 3 组 FAIL），补齐 `errors.writerBusy` 与 `warnings.noChanges` 后恢复全绿；该中间态不属于交付状态。

### 2.2 用例清单与状态口径

状态含义：

- **通过**：存在能直接指向该场景的自动化用例，断言内容与计划预期一致。
- **部分通过**：只有部分子场景有用例，其余子场景仅由实现代码保证（没有可指向的用例）。
- **未覆盖**：找不到可以指向该场景的用例。

用例来源（名称均为现存 `#[test]` 函数或脚本检查组）：

| 文件 | 内容 |
| --- | --- |
| [tests.rs](../src-tauri/src/core/claude_desktop/tests.rs) | 模块单测 53 项：路径、模型名、协议、地址、凭据、profile/meta、1M、资格、事务、崩溃恢复（A23 三分支）、写入点失败回滚（A22）与跨平台路径规则（A33） |
| [lock.rs](../src-tauri/src/core/claude_desktop/lock.rs) | 当前跨进程 writer 锁 4 项回归 + 1 项子进程测试入口：空记录竞争、持有者互斥、失效记录重用、独立进程阻塞与崩溃释放 |
| [probe.rs](../src-tauri/src/core/claude_desktop/probe.rs)（`mod parse_tests`） | 模型列表解析 1 项 |
| [mod.rs](../src-tauri/src/core/claude_desktop/mod.rs)（`mod version_tests`） | 版本探测 2 项：`the_last_app_version_wins_over_the_installed_one`、`a_single_entry_and_a_missing_file_are_handled` |
| [claude_desktop_workflows.rs](../src-tauri/tests/claude_desktop_workflows.rs) | 端到端 9 项 |
| [test-claude-desktop-config.mjs](../scripts/test-claude-desktop-config.mjs) | 10 个检查组：本地化 4 组、errors 1 组、DTO 1 组、commands 1 组、secrets 1 组、independence 1 组、query keys 1 组 |
| [test-claude-desktop-card.mjs](../scripts/test-claude-desktop-card.mjs)（接入 `test:frontend`） | Playwright 5 项：卡片独立分区与 DOM 次序；四视口 × 四缩放无横向溢出/按钮不被裁剪且启用按钮中心可命中；配置按钮键盘可达；模型复选框 Tab 可达、`htmlFor` 配对、空格切换；对话框忙碌时锁定、提交携带模型/1M/路径并在失败时 `role="alert"` |

### 2.3 A01–A40 逐项对照

| 编号 | 场景 | 覆盖用例（文件见 2.2 节） | 状态 | 说明 |
| --- | --- | --- | --- | --- |
| A01 | 已有兼容卡片、只提交 providerId | workflows.rs `apply_direct_request_defaults_are_stable`、`direct_connection_preview_commit_and_restore_roundtrip`；脚本组 `secrets` | 通过 | 请求只含 providerId/模型/路径，地址与密钥由后端从卡片读取；预览 DTO 断言不含哨兵密钥，前端组件与文案也不含凭据字段。 |
| A02 | enabled=false 的兼容卡片 | tests.rs `disabled_provider_is_still_eligible_for_direct_connection`；workflows.rs `direct_connection_preview_commit_and_restore_roundtrip` | 通过 | 提交后 `enabled` 仍为 false，默认模型、协议、`activeProviderId`、`router.enabled` 均未变化。 |
| A03 | 只有余额 access_token，没有 api_key | tests.rs `inference_key_never_borrows_the_balance_token`、`balance_token_never_reaches_the_profile_file` | 通过 | 整卡不合格并给出 `no_inference_key`；profile 里不出现余额 token。 |
| A04 | 混合卡片默认 Responses，子模型 Messages | tests.rs `mixed_card_can_select_the_messages_model_without_touching_the_default` | 通过 | 只选 Messages 子模型，卡片默认模型保持不变。 |
| A05 | endpoint_types 与推断协议冲突、显式 Responses 但 ID 是安全 Claude 名 | tests.rs `protocol_sources_distinguish_inference_from_explicit_settings`、`explicit_responses_protocol_is_excluded_even_for_a_safe_claude_name`、`exclusion_reasons_are_specific_per_model` | 通过 | 名称推断来源标记为未验证；显式 Responses 与 `openai_chat` 声明都被排除。 |
| A06 | included=false、已移除/失效的选中模型 | tests.rs `included_false_is_not_bypassed_by_being_the_default_model`；workflows.rs `an_unsafe_selected_model_blocks_the_whole_apply` | 部分通过 | `included=false` 与整次阻断已覆盖；`reason::MODEL_REMOVED` 常量当前没有任何实现引用，也没有“选中模型已被移出卡片”的用例。 |
| A07 | safe 三角色、anthropic 前缀 | tests.rs `safe_roles_keep_the_original_identifier`；workflows.rs `direct_connection_preview_commit_and_restore_roundtrip` | 通过 | 模型 ID 原样保留（`anthropic/claude-` 前缀同样按原名写回）。 |
| A08 | 任意品牌、旧 Claude 布局、空后缀、[1m]、控制字符 | tests.rs `unsafe_identifiers_are_rejected_with_a_reason`、`exclusion_reasons_are_specific_per_model`；workflows.rs `an_unsafe_selected_model_blocks_the_whole_apply` | 通过 | 逐项给出具体原因，且整次提交零写入。 |
| A09 | Fable 与 unknown/已验收版本 | tests.rs `safe_roles_keep_the_original_identifier`、`version_detection_is_conservative_but_parses_three_parts` | 通过 | 版本未知或低于 `1.12603.1` 时 fable 不在白名单；mythos 不放行。 |
| A10 | 根、末尾 /v1、自定义前缀、DeepSeek | tests.rs `root_and_v1_and_custom_prefix_project_without_doubling_v1`、`deepseek_messages_root_is_preserved`、`plaintext_http_is_reported_but_not_upgraded` | 通过 | 写入与显示的都是 Desktop 真正访问的地址，不会拼出 `/v1/v1`。 |
| A11 | novita/bigmodel/z.ai 的 non-v1 布局 | tests.rs `non_v1_and_full_endpoint_layouts_are_rejected` | 通过 | 三个主机都被明确拒绝（`endpoint_unsupported`），不修改卡片里保存的地址。 |
| A12 | 完整接口、userinfo/query/fragment 等 URL | tests.rs `invalid_urls_are_rejected_before_any_write`、`non_v1_and_full_endpoint_layouts_are_rejected` | 通过 | 阻止错误 root 被写入，且在规划阶段就失败（零写入）。 |
| A13 | context=1M、<1M、unknown、fallback、用户覆盖 | tests.rs `context_projection_is_fallback_aware`、`one_m_mode_off_wins_over_automatic_capability`、`profile_uses_strings_without_options_and_objects_with_them` | 通过 | 只有可依据的来源且窗口不小于 1M 才声明；关闭时不写 `supports1m`；profile 中不出现 `[1m]`。 |
| A14 | 无 label/有 label/1M 与重复选项 | tests.rs `profile_uses_strings_without_options_and_objects_with_them`、`label_only_model_uses_an_object_without_supports1m`；workflows.rs `direct_connection_preview_commit_and_restore_roundtrip` | 部分通过 | 字符串/对象外形与顺序（按索引断言）已覆盖；“同一模型被选择两次”只在 `direct_plan` 报 `duplicate_model`，没有用例。 |
| A15 | 四文件缺失、正常对象、含 MCP/外部 profile | tests.rs `first_apply_creates_all_four_files_and_keeps_external_profiles`、`deployment_mode_patch_preserves_unrelated_keys`、`meta_keeps_foreign_entries_and_order`、`restore_of_originally_missing_files_removes_only_own_content`；workflows.rs `direct_connection_preview_commit_and_restore_roundtrip` | 通过 | 创建与字段合并正确，外部 MCP 与外部 profile 保留。 |
| A16 | 非对象/损坏 JSON/非法 meta 结构 | tests.rs `meta_rejects_non_array_entries_and_non_object_roots`、`shared_object_files_must_parse_as_objects`、`corrupt_shared_files_block_planning_with_a_located_error` | 通过 | 错误定位到具体文件角色，且零外部写入。 |
| A17 | 自己 UUID 与 CC Switch 共存、反复应用 | tests.rs `meta_keeps_foreign_entries_and_order`、`reapply_never_duplicates_the_own_entry`；workflows.rs `direct_connection_preview_commit_and_restore_roundtrip` | 通过 | 自身条目不重复，外部条目与外部 profile 不被删除（提交后 entries=2）。 |
| A18 | 无 journal 但存在同 UUID 文件/遗留释放记录 | tests.rs `withdraw_restores_a_pre_existing_profile_instead_of_deleting_it` | 部分通过 | “接管前同 UUID 文件已有内容则恢复原内容、不凭 id 删除”已覆盖；接管提示与 tombstone（释放后重新接管）没有用例。 |
| A19 | enterpriseConfig 与推理/部署冲突 | tests.rs `deployment_mode_patch_preserves_unrelated_keys`、`meta_keeps_foreign_entries_and_order` | 部分通过 | 未知键保留与 `warnings.keepsEnterprise` 文案已覆盖；没有以 `enterpriseConfig` 具体键为对象的用例。 |
| A20 | 预览后改 key/地址/模型/文件；异步旧预览迟到 | workflows.rs `commit_rejects_a_stale_ticket_after_the_inputs_change`、`a_ticket_cannot_be_used_twice` | 部分通过 | 后端 ticket 失效、单次消费与 `preview_expired` 已覆盖；“异步旧响应不覆盖新表单”只由对话框里的序号计数器实现，脚本未断言。 |
| A21 | 仅更新余额/价格/路由池 enabled | workflows.rs `unrelated_provider_changes_do_not_invalidate_the_profile` | 通过 | 余额、余额脚本、测试状态、路由开关与 active provider 变化后仍是 managed，profile 字节不变；普通 reapply 也立刻可用。 |
| A22 | 四个写点/backup/state/pending 清理各步故障 | tests.rs `a_failed_write_point_rolls_back_and_never_reports_success`、`an_unreadable_target_is_rejected_before_any_write`、`recovery_is_repeatable_after_a_partial_rollback_failure` | 部分通过 | 已证明：写入点失败时先写成功的项按 before 快照撤回、无法恢复时上报 `recovery_required` 而非假成功、目标不可读时零写入；`warnings.pendingCleanup` 的**承诺**已被断言（`an_unfinished_transaction_blocks_every_further_commit`：有未完成事务时预览与提交都被 `recoveryRequired` 挡住，只有显式恢复能继续）；但「journal 删除本身失败」这一具体触发路径仍无用例——它依赖制造文件删除失败，无法确定性复现。 |
| A23 | 写入后、journal 进度前中断；state 写后 pending 删前中断 | tests.rs `recovery_rolls_back_an_uncommitted_interrupted_transaction`、`recovery_confirming_a_committed_transaction_never_rolls_back`、`recovery_reports_a_conflict_when_the_state_matches_neither_version` | 通过 | 三个分支各有手工 pending + 磁盘 state 组合用例：未提交 → 按 before 撤回、已提交 → 只清理 journal 不回滚、都不匹配 → 保留证据并报 `recovery_required`。 |
| A24 | rollback 自身失败、第二次恢复 | tests.rs `restore_is_idempotent_and_repeatable` | 部分通过 | 第二次恢复幂等已覆盖；“rollback 自身失败时保留 pending 证据”没有用例。 |
| A25 | managed 部署/pointer/profile 被外部改写 | tests.rs `managed_drift_is_reported_instead_of_silently_overwritten` | 部分通过 | `deploymentMode` 漂移与恢复时拒绝覆盖已覆盖；`pointerMovedExternally` 与自身 profile 被外部改写只有实现。 |
| A26 | 外部仅新增 MCP 键/meta 条目 | tests.rs `restore_previous_recovers_the_baseline_and_keeps_external_keys`、`meta_keeps_foreign_entries_and_order` | 通过 | 恢复自身字段时保留外部新增内容与条目顺序。 |
| A27 | 原文件缺失，应用后用户加外部键 | tests.rs `restore_of_originally_missing_files_removes_only_own_content`、`unrelated_json_keys_do_not_block_or_change_restore`、`restore_is_idempotent_and_repeatable` | 部分通过 | “接管前不存在 → 只删自身内容、保留目录”已覆盖；“接管前不存在、应用后用户又加了外部键”的组合没有单独用例。 |
| A28 | prior appliedId 的外部 profile 被删除 | workflows.rs `direct_connection_preview_commit_and_restore_roundtrip` | 部分通过 | 恢复后 `appliedId` 回到 CC Switch、自身 profile 被删除已覆盖；“外部 profile 文件已不存在”时“不随机挑第一条”的路径没有用例。 |
| A29 | restorePrevious vs switchOfficial vs release/rebaseline | tests.rs `restore_previous_returns_each_config_to_its_own_mode`、`switch_official_writes_1p_and_withdraws_the_own_entry`、`release_writes_no_desktop_file`、`withdraw_restores_a_pre_existing_profile_instead_of_deleting_it`；workflows.rs `release_keeps_the_files_and_records_a_tombstone`、`switching_to_official_mode_preserves_foreign_entries` | 部分通过 | 三种拆除语义分别有用例；`rebaseline=true` 没有用例（`first_baseline_is_preserved_across_reapply` 验证的是普通 reapply 不移动基线）。 |
| A30 | Provider 改 key/删除后 | workflows.rs `provider_removal_marks_the_configuration_orphaned_but_still_restorable` | 部分通过 | 删除后 `orphaned` 且离线可恢复已覆盖；“改密钥后 stale”没有用例（`stale_reasons_for` 不比较密钥）。 |
| A31 | 项目关闭/路由关闭 | workflows.rs `unrelated_provider_changes_do_not_invalidate_the_profile` | 部分通过 | 路由关闭不改变 profile 已覆盖；“关闭 AgentHarbor 进程后 Desktop 仍可直连”只能由真实环境验证（L08）。 |
| A32 | 两个合作 writer/应用实例，以及写前已发生外部变更 | lock.rs `a_live_holder_blocks_a_second_writer`、`an_empty_record_during_initialization_cannot_be_reclaimed`、`a_stale_or_corrupt_record_without_a_holder_is_reusable`、`a_foreign_process_blocks_and_crash_releases_the_lock`；workflows.rs `commit_rejects_a_stale_ticket_after_the_inputs_change` | 部分通过 | 跨进程互斥与旧 revision 被拒已覆盖；提交前“文件已被外部改动”（`verify_unchanged`）没有用例；跨工具的检查与替换不承诺 CAS。 |
| A33 | Windows 多候选 / macOS / Linux 绝对与相对 XDG | tests.rs `exact_names_win_and_candidates_are_never_picked_by_sort_order`、`linux_requires_an_absolute_xdg_config_home`、`macos_and_windows_roots_follow_the_documented_layout`、`every_channel_layout_yields_a_distinct_four_file_set` | 通过 | 路径规则已从 `#[cfg]` 分支抽成纯函数：精确名优先且不按字典序挑选、两个渠道互不串台、Linux 只接受绝对 XDG 否则回落 `~/.config`、macOS Application Support 与 Windows LOCALAPPDATA 回落、两种渠道命名都得到互不相同的四文件路径。仍无真实 macOS/Linux 主机实测。 |
| A34 | AGENT_HARBOR_CLIENT_HOME 隔离、路径身份改变、碰撞 | tests.rs `isolated_home_confines_discovery_to_the_isolated_root`、`roots_must_be_distinct_and_non_nested`、`four_target_paths_are_always_distinct` | 部分通过 | 隔离与碰撞零写入已覆盖；“路径身份变化使已发出的 revision 失效”只由 `paths.identity()` 比较实现，没有用例。 |
| A35 | Bearer 检查 200 合法/200 HTML/401/403/404/429/超时 | probe.rs（`mod parse_tests`）`accepts_data_models_and_bare_arrays_but_rejects_html` | 部分通过 | “200 + HTML/残缺结构不算通过”已覆盖；401/403、404/405、429、超时与网络失败的分类没有用例（没有 mock HTTP 服务）。 |
| A36 | 凭据含 Bearer 前缀/无效 header 字符 | tests.rs `bearer_prefix_is_not_doubled_in_the_profile`、`invalid_keys_are_rejected` | 通过 | 写入正规 token（无双前缀）；含空白或超长密钥被阻止。 |
| A37 | 所有 DTO、预览、错误、日志含哨兵 secret | tests.rs `key_never_appears_in_the_previewed_change_values`、`balance_token_never_reaches_the_profile_file`；workflows.rs `direct_connection_preview_commit_and_restore_roundtrip`；脚本组 `secrets` | 部分通过 | 预览、DTO、前端类型与文案已覆盖；错误与日志路径只有实现（错误只带错误码与安全参数），没有专门用例。 |
| A38 | 两根配置路径/长 ID、缩放和键盘 | Playwright 套件 `scripts/test-claude-desktop-card.mjs`：`the Desktop card and its action buttons hold 1280/1024/768/480px at 80-150% zoom without sideways scrolling or clipping`、`the configure button is focusable and opens the dialog from the keyboard`、`every model checkbox is tabbable, labelled through htmlFor and toggled with Space`、`the dialog locks while busy, sends the model/1M/paths selection and renders the preview file cards`、`Claude Desktop card renders as its own labelled section after the untouched client-configs section` | 通过 | 四个视口宽度 × 80/100/125/150% 缩放：页面与卡片都无横向溢出、按钮不塌陷不越界、启用按钮中心点可命中、可点击动作集合不变；键盘可达与 `htmlFor`/`id` 配对、Tab 与空格切换、提交后 revision 复用与失败后 `role="alert"` 均有断言。长路径/长模型 ID 由 `break-all`/`break-words` 类契约断言（未注入真实超长取值）。 |
| A39 | 独立 Desktop 卡片/disabled Provider/无兼容模型 | 脚本组 `independence`、`commands`、`query keys`、`secrets`；tests.rs `disabled_provider_is_still_eligible_for_direct_connection`、`included_false_is_not_bypassed_by_being_the_default_model` | 部分通过 | 独立卡片、命令接线、无密钥输入框与 disabled 卡片已覆盖；“没有兼容模型时的提示文案”没有断言。 |
| A40 | 现有 Codex/ClaudeCode/OpenCode 预览、注入、恢复 | 既有套件：`pnpm test:rust` 的 9 项 `local_workflows` 与 `pnpm test:frontend` 的完整前端回归 | 通过 | 两套既有回归在本次运行中全部通过（693 lib + 9 集成；前端 exit 0），说明旧客户端与路由用例未被本功能破坏。 |

合计：**通过 22 项、部分通过 18 项、未覆盖 0 项**。本轮新增覆盖：A23 与 A33 由“未覆盖”转为“通过”（中断恢复三分支、跨平台路径规则各有真实断言）；A38 由“部分通过”转为“通过”（新增 Playwright 套件覆盖缩放/键盘/溢出）；A22 由“未覆盖”转为“部分通过”（写入点失败回滚、不可读目标零写入、二次恢复可重复已覆盖，`pendingCleanup` 仍只有实现）。

## 三、真实 Claude Desktop 验收（L01–L10）

### 3.1 已执行的上游连通性核对（不是 Desktop 验收）

用户指定用 `https://www.fucheers.top/` 的 `claude-haiku-4-5-20251001` 做连接测试。据此对**已保存的卡片** `kiro-fucheers`（`baseUrl=https://www.fucheers.top/`，默认模型就是该模型，协议 Anthropic Messages，`enabled=true`）执行了两次真实核对，全部经由产品的同一实现：

| 核对 | 命令 / 入口 | 结果 |
| --- | --- | --- |
| 卡片资格与地址投影 | `real_provider_diagnostic`（只读，`--ignored`） | `projectedRoot=https://www.fucheers.top`、`messagesUrl=https://www.fucheers.top/v1/messages`、eligible=true；目标模型 `selectable=true`、`protocolSource=inferred`、`contextSource=catalog`、`1m=false` |
| `/v1/models` 裸 Bearer | `real_bearer_probe` → `probe::run` | **200**，列表 15 条，`claude-haiku-4-5-20251001` 可见，`modelsMissing=[]` |
| `/v1/messages` 裸 Bearer（无 `x-api-key`） | `real_messages_bearer` → `probe::run_messages_check` | **200**，`status=messagesAccessible`，`responseModel=claude-haiku-4-5-20251001`，`stopReason=end_turn` |

结论与边界：

- 这证明 Claude Desktop 的**鉴权形态与协议**在这张卡片上是可行的：Desktop 在 `/v1/messages` 上只发 `Authorization: Bearer`，该中转接受了它，并且**没有改写模型名**（上游自报模型与请求模型一致）。
- 这**不等于** Desktop 验收通过。仍未证明：Desktop 是否接受省略 `coworkEgressAllowedHosts` 的最小 profile、菜单是否显示、多轮/流式/工具是否可用。这些只能在真实 Desktop 里确认。
- 两次核对都通过产品代码发起：`/models` 是只读 GET，`/messages` 是一次 `max_tokens=1` 的真实请求（已产生极少量计费）。产品默认**不**发 Messages 请求，必须由用户在对话框里显式点击“核对 Messages 兼容性”。
- 3.1 的两项上游核对**没有改动本机任何 Claude Desktop 文件**；随后在用户显式授权下执行的 3.1b 真实接管**改动了四文件并重启了 Desktop**。

### 3.1b 已执行的真实接管（2026-10-04 19:13 本机时间）

经用户显式授权，用产品真实的 prepare → ticket → commit 路径把卡片 `kiro-fucheers` + `claude-haiku-4-5-20251001` 应用到了本机 Claude Desktop，并调用 `Manager::restart_app` 完成「先完全退出、再启动」。执行前后状态：

| 项目 | 接管前 | 接管后 |
| --- | --- | --- |
| `Claude\claude_desktop_config.json` | `{"deploymentMode":"3p"}` | 未改动（已经是 3p） |
| `Claude-3p\claude_desktop_config.json` | 3p + `coworkUserFilesPath` + 大量 `preferences` | `preferences` 与无关键全部保留 |
| `configLibrary\_meta.json` | `appliedId` = CC Switch，1 个外部条目 | `appliedId` = AgentHarbor，**同时保留 CC Switch 条目** |
| `configLibrary\d918233d-….json` | 不存在 | 写入本工具 profile（gateway / bearer / `https://www.fucheers.top` / 1 个模型） |

执行前的四文件副本保存在 `%USERPROFILE%\claude-desktop-backup-20261004-191325`（本次执行者手动创建，不属于产品备份）。产品自身的接管基线保存在 `%USERPROFILE%\.codex\agentharbor\claude-desktop\`。

**Desktop 侧的启动证据**（`%LOCALAPPDATA%\Claude-3p\logs\main.log`，脱敏摘录）：

```text
19:13:58 [custom-3p] Credentials loaded from managed config { provider: 'gateway', mcpServerCount: 0 }
19:13:58 [custom-3p] 3P mode active { provider: 'gateway' }
19:13:58 [custom-3p] Model discovery: skipped (inferenceModels obviates discovery); picker = 1 (inferenceModels)
19:13:59 [custom-3p] inference apiHost=https://www.fucheers.top
19:14:01 [custom-3p] ConfigHealth recomputed { state: 'healthy', provider: 'gateway' }
```

它逐条对应验收点：

- `Credentials loaded from managed config`：Desktop **接受了**这份 profile（含省略 `coworkEgressAllowedHosts` 的最小候选），并读取到 gateway 凭据。
- `3P mode active`：3P 部署模式生效。
- `picker = 1 (inferenceModels)`：**模型菜单里正好是 1 个模型**，来自我们的 `inferenceModels`，说明显式模型列表被采纳（L03）。
- `inference apiHost=https://www.fucheers.top`：Desktop 会向该中转发请求，地址投影正确。
- `ConfigHealth: healthy`：无配置健康告警。
- 同一时段日志中没有 egress 拦截、profile 拒收或模型白名单报错；唯一的 warn 是与本功能无关的 `MaxListenersExceededWarning` 与 `[ScheduledTasks] Already initialized`。

### 3.1c 真实对话与出站请求实测（同日 19:20 / 19:25）

**L04 真实对话**：在 Desktop 里发出「你好」后，界面收到了完整回答（中文、markdown 列表、`just now`），底部模型选择器显示 `Haiku 4.5 Extended`，左下角账号区显示 `liang · Gateway`。日志里的本轮记录：

```text
19:20:31 [custom-3p] inference apiHost=https://www.fucheers.top
19:20:39 [Result] Turn succeeded for session local_e628d8ed-7975-425f-9cce-539ccd1430d1
19:20:39 [CycleHealth] Healthy cycle: {
  model: 'claude-haiku-4-5-20251001',
  cycle_health: 'healthy',
  had_first_response: true,
  seconds_to_outcome: 8,
}
```

**L02 出站请求实测（不是从日志推断，是抓到的包）**：为了让「Desktop 发的是裸 Bearer、模型名原样出站」这条结论可测，实现了一个一次性验证网关：把 Desktop 的 `inferenceGatewayBaseUrl` 临时指向本地回环 mock，mock 记录请求后**转发给真实上游**并把回答原样回给 Desktop，验证完成后自动还原配置并重启。捕获结果：

```text
MOCK_CAPTURE path=POST /v1/messages HTTP/1.1  authPresent=true  xApiKeyPresent=false
             model=Some("claude-haiku-4-5-20251001")  version=Some("anthropic-version: 2023-06-01")  stream=false
```

它逐条证实了此前只能推断的三件事：

1. **Desktop 请求的路径就是卡片投影出来的 `/v1/messages`**（与预览里显示的 URL 完全一致）。
2. **Desktop 只带 `Authorization`，不带 `x-api-key`**（`xApiKeyPresent=false`）——这正是首期直连成立的前提，也是 `probe::run_messages_check` 要单独检测的原因。
3. **模型名原样出站**：`model=claude-haiku-4-5-20251001`，没有被中转或本地做任何改名。

另外它还证实 `anthropic-version` 头由 Desktop 自己发出，与实现里的假设一致。

**验证后的还原**：profile 已写回 `https://www.fucheers.top`，`_meta.json` 的 `appliedId` 仍为 AgentHarbor 且 CC Switch 条目保留，Desktop 于 19:27:03 以真实上游重启（`inference apiHost=https://www.fucheers.top`、`picker = 1`、`ConfigHealth healthy`）。

### 3.1d 多轮上下文实测（同日 19:29）

同一会话 `local_3b7199fe-7160-4c32-9185-66a744b4bd0d` 连续两轮都成功，并且第二轮开始 `is_resume: true`：

```text
19:29:16 [Result] Turn succeeded  model=claude-haiku-4-5-20251001  is_resume=true  seconds_to_outcome=12  transcript=299105 B
19:29:44 [Result] Turn succeeded  model=claude-haiku-4-5-20251001  is_resume=true  seconds_to_outcome=4   transcript=302104 B
```

`is_resume: true` 说明第二轮是在同一会话上下文里继续的，`transcript_size_bytes` 从 299105 增长到 302104，**上下文确实在累积** —— 这是计划里 L04「上下文连续」的直接证据。

### 3.1e 一次由验证工具本身造成的失败（已定位、已修复行为）

为了抓包证实出站请求形态，临时把 Desktop 指向本地回环 mock。该 mock 首版有两个缺陷，导致那 5 分钟里用户的「测试」消息失败：

1. 只调用了一次 `read()`，而 Desktop 的请求体很大（系统提示约 62k 字符），读到的是**截断的 JSON**，转发给上游即被拒：`kind: model_rejected (HTTP 400)`。
2. 达到连接上限后监听线程退出，后续请求直接 `ERR_CONNECTION_REFUSED`。

两个错误都只出现在 mock 运行的时间窗内（19:26:24、19:26:36），19:27 还原配置后**再无任何相关错误**。这属于验证工具的问题，不是产品缺陷；教训已写进下一节。

### 3.1f 稳定态复核（同日 19:31）

```text
profile inferenceGatewayBaseUrl = https://www.fucheers.top   （真实上游，非 mock）
_meta.json appliedId           = d918233d-0a41-47a1-a29f-6d610a03722a   （AgentHarbor；CC Switch 条目保留）
Desktop 进程                    = 10
Turn succeeded                  = 13（最近 8 条全部成功，含 3.1d 的两轮）
Turn failed                     = 1，时间为 2026-07-09（历史记录，与本功能无关）
网关/模型相关错误                 = 仅 19:26:24 与 19:26:36 两条，均在验证网关运行窗口内；19:27 还原后为零
```

另外，Desktop 自身在 19:31:14 完成了 `claude-code-vm`（81MB，来自官方 CDN，与本功能无关）的下载安装——此前若感觉「没反应」，那是在等它。

**仍未验证**：流式响应（捕获到的请求 `stream=false`）、工具调用、多模型菜单切换（单模型已验）。

### 3.2 L01–L10 逐项

| 编号 | 场景 | 具体步骤 | 通过标准 | 需要的证据 | 状态 |
| --- | --- | --- | --- | --- | --- |
| L01 | 最小 profile 候选及省略 Cowork 键 | 备份四个文件 → 完全退出 Desktop（含托盘）→ 用选定的测试中转应用（最少模型）→ 启动 Desktop → 打开模型菜单 → 发一条最小普通请求。 | 该版本能启动、菜单可见、真实普通请求成功；否则 P0 候选阻断，必须先调整 profile 形态。 | Desktop 版本与启动日志、模型菜单截图、一次成功请求的请求/响应（含上游侧记录）。 | **部分执行**：见 3.1b，Desktop 日志已证明 profile 被接受、3P 模式生效、`ConfigHealth=healthy`；真实对话请求未发 |
| L02 | 已保存兼容中转复用 | 不重新填写密钥，直接应用一张已保存的兼容卡片 → 在 Desktop 发请求 → 到中转侧核对访问日志。 | 不重填 key；选中模型原样出站；供应商侧日志或响应证明请求确实到达该中转。 | 供应商侧访问日志与请求体里的 model 字段，以及卡片未显示密钥。 | **通过**：见 3.1c，抓到的出站请求为 `POST /v1/messages`、只带 `Authorization`、`model=claude-haiku-4-5-20251001`；卡片界面上从未显示或要求输入密钥 |
| L03 | 两个显式模型、展示名切换 | 应用两个模型（其中一个带展示名）→ 在菜单里切换 → 各发一条请求。 | 菜单与实际模型对应，展示名生效，没有 fail-all 静默退回。 | 菜单截图 + 两次请求的上游 model 字段。 | **部分执行**：见 3.1b，日志记录 `picker = 1 (inferenceModels)`，说明菜单只呈现所选的 1 个模型；多模型切换与真实请求未验证 |
| L04 | 流式回复、两轮上下文 | 发一条会流式返回的请求，观察是否正常结束；随后追问一轮。 | 能正常结束、上下文连续、没有协议错误。 | 流式结束事件与两轮请求/响应。 | **部分通过**：见 3.1c/3.1d，多轮对话成功且 `is_resume=true`、transcript 递增证明上下文连续；捕获到的请求 `stream=false`，因此流式仍未验证 |
| L05 | 普通工具使用（测试中转支持时） | 在测试中转支持工具调用时发起一次工具调用，并继续下一轮。 | 请求/结果与下一轮都可以继续；不能用普通文字回答代替工具验收。 | 工具调用请求与结果、下一轮回复。 | **通过**：链路侧 `probe_capabilities` 得到 `stop_reason=tool_use`、`content=[tool_use]`；真实 Desktop 侧发起 `Read a file`（读上传的 txt）并在日志里得到 `Turn succeeded` |
| L06 | 存在 CC Switch 或其他 3P profile/MCP | 在本机现状（CC Switch profile + MCP + preferences）上应用 → 检查 `_meta.json` 与两个 config → 执行“恢复接管前配置”。 | 应用只切换 own profile；恢复还原原选择且保留外部内容。 | 应用前后 `_meta.json` 与两个 config 的 diff（脱敏）。 | **部分通过**：应用侧已实测（`_meta.json` 的 `appliedId` 从 CC Switch 换成本工具、CC Switch 条目保留、`preferences` 与 `coworkUserFilesPath` 未被触碰，见 3.1b）；**恢复侧只有临时根的用例证据，本机未执行过一次真实恢复** |
| L07 | switchOfficial | 应用后执行“切换官方模式” → 启动 Desktop 进入官方流程。 | 不删企业策略；能进入该环境允许的官方流程；存在策略阻断时要准确报告。 | 切换后的文件 diff（企业相关键仍在）+ 官方流程结果或阻断信息。 | **通过（文件层）**：真实执行 `switchOfficial` → 两处 config 均为 `1p`、自身条目与 `appliedId` 已撤回且 CC Switch 条目保留、自身 profile 被删除；本机真实存在的非受管键 `preferences` 与 `coworkUserFilesPath` **逐字节未变**。随后立即重新应用直连并重启，回到 `managed / direct / 3 模型`。**未验证**：进入官方登录流程本身（那需要把应用留在官方模式，本轮刻意没有这么做） |
| L08 | 完全退出 AgentHarbor 后使用 Desktop | 应用后完全退出 AgentHarbor（含本地路由与托盘）→ 用 Desktop 发请求。 | 直连仍然工作，不依赖 127.0.0.1 的项目代理。 | AgentHarbor 未运行时的成功请求，以及确认请求目标是中转根地址而不是本地端口。 | **部分通过**：本轮所有真实对话都是在本项目**未运行**时发生的（写配置与重启由命令行测试完成，GUI 未启动），抓包也证实目标是中转根地址；但「本地路由服务已启动后再完全退出」这一组合未验证 |
| L09 | 中转只接受 x-api-key 或只暴露非 v1 端点 | 选择这样一张卡片（或在测试环境模拟）→ 观察预览与 Bearer 检测的结论。 | 显示鉴权/布局限制，不把它标成 Desktop 已验证可用。 | 卡片状态与原因文案；可选的真实请求失败形态。 | **通过**（用本地回环 mock，不依赖某张真实卡片也不计费）：① 只认 `x-api-key` 的网关 → `status=authRejected`、`note=claudeDesktop.probe.auth`，且 mock 记录 `xApiKey=0 bearer=1`（产品只用 Bearer）；上报里不含密钥也不含响应正文。② 非 v1 端点四种形态 → `endpoint_unsupported ["nonV1"]`（DeepSeek 的 `/anthropic` 是内置例外），完整接口地址当 root → `invalid_url`；卡片资格 `eligible=false reason=endpoint_unsupported:nonV1` |
| L10 | 密钥变更、重新应用、恢复 | 修改卡片密钥 → 观察 stale 提示 → 重新应用 → 执行恢复。 | stale 提示准确；重新应用换成新 key；恢复后不该保留密钥的 own 文件里没有密钥残留。 | 各阶段四文件内容（脱敏）与状态输出。 | **通过**：实测 `managed → stale(["credential"]) → managed`；重新应用后 profile 换成新密钥且旧密钥不残留；恢复后自身 profile 被撤回，两个 config 与 `_meta.json` 均无任何密钥残留。为让这一条成立，先修了缺陷 (m) 与 (n) |

**执行前必须知道的事**

- 一次真实回复**可能计费**。L01–L10 必须由用户在明确选定的测试中转上**主动**执行，不能在后台自动发请求。
- 执行前先备份四个文件（两个 `claude_desktop_config.json`、`configLibrary/_meta.json`、`configLibrary/d918233d-0a41-47a1-a29f-6d610a03722a.json`），并记录当前的 mode 与 `appliedId`。
- 回滚路径：实现自身的本地备份（接管前状态 + `backups/*.dat`，最多 20 份）配合界面里的“恢复接管前配置”；也可以直接用执行前的四个文件副本整文件写回。
- 验收报告必须包含 Desktop 版本、平台/安装渠道、项目提交、供应商协议、最终 URL 形态、模型 ID、网络出口、案例结果与恢复结果；**不能只有配置文件截图或模型菜单截图**。
- 执行前先按第一节的说明**重新核实正在运行的 Desktop 版本**（日志推断为 `2.19675.0`，尚未与进程核对），版本结论会影响 L01/L09 的判定。

## 四、已发现并修复的问题

### (a) 恢复把同一个 deploymentMode 写到两个 config

- 现象：接管前 normal 是 `1p`、3P 是 `3p` 的机器，执行“恢复接管前配置”后两个文件都被写成同一个值（`3p`），normal 的原始取值丢失；接管前文件里根本没有该键时还会写入一个猜测值。
- 修复：[transaction.rs](../src-tauri/src/core/claude_desktop/transaction.rs) 的 `build_plan` 改为**逐文件**决定目标值（`baseline_mode_of`）；`DesiredState.mode = None` 时每个文件恢复自己的接管前取值，接管前没有该键就删除该键。
- 证据：`restore_previous_returns_each_config_to_its_own_mode`、`restore_removes_a_deployment_mode_that_the_takeover_introduced`（tests.rs）；`direct_connection_preview_commit_and_restore_roundtrip` 第 7 步断言 normal 回到 `1p`。

### (b) 已消费的预览 ticket 被报成“输入已变化”

- 现象：同一个 revision 第二次提交（或 ticket 已过期）时，旧实现先比较 revision 再校验 ticket，于是用户看到 `changed`（“预览之后输入已变化”）而不是 `preview_expired`，提示方向错误，会把用户引向重复预览。
- 修复：[mod.rs](../src-tauri/src/core/claude_desktop/mod.rs) 的 `commit` 先 `take_ticket`（校验未过期并消费），再做 revision 与路径身份比较，错误码顺序因此稳定。
- 证据：`a_ticket_cannot_be_used_twice`（断言 `preview_expired`）、`commit_rejects_a_stale_ticket_after_the_inputs_change`（断言 `changed`）。

### (c) 用户设置被误报成 stale

- 现象：用户显式填写的展示名（`labelOverride`）与卡片显示名不一致时，已托管的配置被报成“需重新应用”；用户显式关闭 1M 的模型也被判成需要重新应用。
- 修复：[mod.rs](../src-tauri/src/core/claude_desktop/mod.rs) 的 `stale_reasons_for` 把“显式设置且不等于模型 ID”的展示名视为已固定（`pinned`），不再据此报 stale；1M 只在“记录为开启但当前能力不支持”（`model.one_m && !candidate.supports1m`）时才报 stale，关闭不会报。
- 证据：`direct_connection_preview_commit_and_restore_roundtrip` 与 `unrelated_provider_changes_do_not_invalidate_the_profile` 断言带展示名的配置仍为 `managed`；“用户关闭 1M”这一分支仍只有实现，没有独立用例。

### (d) 后端原因码被渲染成原始 key

- 现象：后端原因码是 snake_case（`unsupported_desktop_role`、`unsafe_model_name:<detail>` 等），本地化 key 是 camelCase；对话框原先取冒号后的最后一段拼 key，于是绝大多数原因显示成原始 key，或把地址等参数拼进 key。
- 修复：[claude-desktop-config-dialog.tsx](../src/components/newapi/claude-desktop-config-dialog.tsx) 的 `reasonKey()` 只取冒号前的码，做 snake_case → camelCase 映射，找不到时回落到 `storage`；zh/en 的 `claudeDesktop.errors` 补齐后各 52 条，`types.rs` 中 26 个 snake_case 码全部可解析（其中 `incompatibleProtocol`、`unsupportedModelEndpoint`、`modelNotIncluded`、`modelRemoved`、`unsafeModelName`、`unsupportedDesktopRole`、`endpointLayoutUnsupported`、`bearerUnverified`、`bearerCheckRejected`、`providerNotEligible` 共 10 条只被 reason 使用）。
- 证据：脚本组 `errors: every Rust reason/error code resolves to a camelCase locale key`（同时断言对话框仍做映射、仍取冒号前的码）。

### (e) 版本探测报出安装时的旧版本

- 现象：日志文件保存该渠道的完整启动历史，旧实现取**第一条** `appVersion`，于是把安装时的 `1.12603.1` 当成当前版本（`fable` 白名单等版本策略都建立在它之上）；同时旧实现先读常规渠道日志，而不是本功能管理的 3P 渠道。
- 修复：[mod.rs](../src-tauri/src/core/claude_desktop/mod.rs) 的 `read_app_version` 扫描整个日志并保留**最后**一条 `appVersion`；`read_version_from_logs` 改为**优先 3P 根目录**、常规根只作回退；并接受单引号、双引号与裸值。
- 证据：`version_tests::the_last_app_version_wins_over_the_installed_one`、`version_tests::a_single_entry_and_a_missing_file_are_handled`。
- 注意：修复后本机探测值很可能是 `2.19675.0`，但这仍然是日志证据，未经运行中的应用核实。

### (f) 切换官方模式会无条件删除 profile 文件

- 现象：拆除配置时按固定 UUID 直接删除 `configLibrary/d918233d-0a41-47a1-a29f-6d610a03722a.json`。如果该文件在接管前**本来就存在**（同 UUID 的历史文件或其它工具写的同名文件），它会被删除且内容不可恢复。
- 修复：接管时把该文件的原内容记为基线（`Edition.baseline`），拆除时通过 `DesiredState.profile_fallback` 与 `baseline_profile()` 写回原内容；只有“接管前不存在”时才删除。
- 证据：`withdraw_restores_a_pre_existing_profile_instead_of_deleting_it`（tests.rs）。

### (g) 再次应用后会立刻误报 drift（`installed` 被事务前快照覆盖）

第二次应用（把 1 个模型换成 3 个）后，状态立刻变成 `conflict`，`DRIFTED_ROLES=["meta"]`。根因：`transaction::next_state` 对**本次没有改动**的文件用「事务前快照」重算 `installed`，而那份快照里没有我们先前写入的 meta 条目，于是"最近安装值"被擦成 `own_entry: None`——刚写完就判成被外部改过。

修复：未改动的文件直接沿用既有 `installed`。它记录的是**上一次成功写入的值**，不是"文件里现在恰好是什么"；用快照重算等于把历史覆盖掉。回归用例 `the_recorded_installed_values_match_what_was_written` 现在覆盖"连续两次应用都不得报 drift"。

### (h) `rebaseline` 没有生效（“以现状重新接管”实际沿用了旧基线）

`editions_before(paths, previous)` 无条件沿用既有基线，完全没有读取 `request.rebaseline`——所以设计里"只有显式 rebaseline 才采用当前配置作为新恢复点"这条语义**从未实现**，该操作等于普通 reapply。这也是上面那次误报 drift 无法用产品自身入口治愈的原因。

修复：`editions_before` 增加 `rebaseline` 参数，为真时直接改用磁盘现状作为基线与安装值。用例 `rebaseline_adopts_the_current_state_as_the_new_baseline` 同时断言"普通 reapply 不移动基线"与"rebaseline 采用现状"。真机上随后用该入口把状态校正回 `managed`、`DRIFTED_ROLES=[]`。

### (i) 整库测试偶发失败：测试隔离泄漏到进程级环境变量

`pnpm test:rust` 约每 10–18 次出现一次单点失败。定位结果：`Manager::new` 只接受一个 app data 目录，**没有携带路径覆盖**，所以凡是调用 `status()` / `detection()` 的用例都会落到 `paths::discover()` —— 而它读取进程级 `AGENT_HARBOR_CLIENT_HOME`。另一个用例在并行线程里设置该变量时，这些用例就会看到"隔离根"，断言随之失败；而且该用例的直接 `set_var`/`remove_var` 在断言失败时不会还原，把变量留在进程里。

修复：① `Manager` 增加 `discovery()`，有路径覆盖时返回合成结果，完全不调用 `paths::discover()`；② 新增 `DesktopPaths::overridden`，覆盖路径下不再查询 MSIX、不再可能启动用户真实安装；③ 环境变量改用 `Drop` 守卫（`IsolatedHome`）设置，断言失败也必定还原；④ 依赖发现的用例与设置隔离根的用例共用同一把锁。验证：连续 9 次 `cargo test --lib` 全绿（修复前约每 18 次失败 1 次）。

### (j) 既有的 flaky 用例：真实注册表值被空字符补齐

继续排查时抓到唯一残留的偶发失败，**与本次功能无关**：`platform::terminal_environment::windows::tests::native_registry_preserves_raw_types_and_removes_absent_values` 以逐字节相等断言 `REG_SZ` 往返（`[97, 0]` vs `[97, 0, 0, 0]`），而 Windows 写入时会把字符串规范成以空字符结尾。修复：二进制值仍逐字节断言；字符串值改为断言「类型保留 + 内容保留，只允许末尾空字符填充」。

### (k) 路径发现把空的 `Claude-Data` 目录当成了第二个安装

给「重启」做真实探测时发现：本机 `%LOCALAPPDATA%` 下的 `Claude-Data` 是一个**空目录**（无 `logs`、无 `claude_desktop_config.json`），旧规则只看目录名是否以 `Claude` 开头，于是它成了一个候选，导致 `discover()` 报「多个安装渠道」并拒绝给出默认路径——本来无歧义的机器变成了「必须让用户手选」。修复：候选除名字之外必须带**安装痕迹**（常规渠道要有 `claude_desktop_config.json` 或 `logs`；3P 渠道要有 `configLibrary`），并且名字规则收紧为「`Claude` 或 `Claude-<非空后缀>`」，`Claude Code`（空格分隔）不再入选。修复后本机发现结果：`candidates=1 default=true notes=[]`。

### (l) 重启能力没有被隔离根拦住

新增的重启能力最初无条件查询本机 MSIX 包，隔离根（`AGENT_HARBOR_CLIENT_HOME`）下也会拿到真实安装的启动 id，意味着一个本该完全隔离的会话可以启动用户真实的 Claude Desktop。修复：`paths.isolated` 为真时一律不查询、不启动，并有用例断言 `launch_plan` 在隔离根下返回 `(None, None)`。

### 附带的行为保证：跨进程 writer 锁

- 第二个 AgentHarbor 实例或会话在写入时会立即以可重试的 `writer_busy` 失败，而不是并发写入；锁文件在应用数据目录的 `claude-desktop/writer.lock`（当前使用操作系统排他文件锁，关闭句柄/崩溃自动释放；锁文件保留，pid 仅用于提示，不再以进程启动时间或删文件判断所有权）。
- Claude Desktop 与 CC Switch **不遵守这把锁**，因此“完全退出 Desktop、暂停其它配置管理工具”仍然是操作前提。
- 证据：`a_live_holder_blocks_a_second_writer`、`an_empty_record_during_initialization_cannot_be_reclaimed`、`a_stale_or_corrupt_record_without_a_holder_is_reusable`、`a_foreign_process_blocks_and_crash_releases_the_lock`（lock.rs；最后一项实际启动并终止隔离测试子进程）。

## 四之二、实施后的代码审查与解耦

功能验收通过后做了一次针对性审查，目标是「有没有冗余实现、有没有更简单解耦的方式」。结论与改动：

| 问题 | 证据 | 处理 |
| --- | --- | --- |
| `next_state` 与 `baseline_captured` 重复表达同一件事 | 该字段写入后无任何读取方 | 删除字段 |
| `State::path_for` 与 `status::state_paths` 重复 | `path_for` 定义后零调用 | 删除，只保留一处路径重建 |
| `State.models` 与 `State.data.models` 两份模型列表 | `data.models` 只写不读，且可能不一致 | 只保留 `State.models` |
| `with_version_hint` 与 `with_detected_version` 完全等价 | 两个 setter 写同一字段 | 合并为一个 |
| `FilePlan.kind` 是派生值却被持久化 | 可由「写入前是否存在」+「是否留下内容」推出，存两份会互相矛盾 | 改为 `FilePlan::kind(existed_before)` 派生 |
| `DesktopPaths::from_roots_overridden` 从未使用 | 引入 `overridden` 标志时的遗留 | 删除 |
| `BearerStatus::passed` / `MessagesStatus::passed` 从未使用 | UI 直接映射状态字符串 | 删除 |
| `probe.rs` 混了三种职责（`/models` 检测、`/messages` 核对、能力探测） | 652 行单文件 | 能力探测拆到 `capability.rs` |
| `mod.rs` 的 `Manager` 同时管状态、发现、检测、规划、提交、ticket、版本探测 | 1396 行、64 项 | 抽出 `status.rs`（只读投影）与 `tickets.rs`（预览票据），`Manager` 只做编排 |

拆分后规模变化（行）：`mod.rs` 1396 → 1129，`transaction.rs` 1061 → 997，`probe.rs` 652 → 466，新增 `status.rs` 258、`capability.rs` 199、`tickets.rs` 135。**没有改变任何对外行为**：全部用例保持通过。

仍然刻意保留的部分（审查后判断不值得再拆）：

- `transaction.rs` 仍是单文件近千行。它的四个目标共享同一套 `Edition`/`ManagedValues` 所有权模型，拆开会把「撤回什么」的规则撕成两半，反而更难核对。
- `Manager` 仍持有 `Store` 参数而不是自己加载。它已经从「什么都做」缩到「编排 + 锁」；再抽一层 `StatusReader`/`Committer` 会增加间接层，而当前调用点只有命令层一处。

## 四之三、L10 实施中发现并修复的两个缺陷

### (m) 改推理密钥不触发 stale

计划第 6.7 节把「改推理 key」列为应标 stale 的事件，但 `stale_reasons_for` 只比较模型、协议、展示名与上下文来源，**从不比较密钥**。后果是用户改完卡片密钥后会以为 Desktop 已经用上新密钥，而 profile 里仍是旧快照。

修复：`State` 增加 `credential_fingerprint`（SHA-256 十六进制），提交时记录、比对时不相等即报 `credential` stale。**只存指纹不存密钥**，也不含密钥片段；旧状态没有该字段时（`serde default`）不据此报 stale，宁可漏报也不误报。用例 `a_changed_inference_key_marks_the_configuration_stale`。

### (n) `build_plan` 用「磁盘现状」判断接管前是否存在

`build_plan` 原来用一个 `existed_before` 字段（等于「文件现在是否存在」）来决定 profile 是「写回接管前内容」还是「删除」。普通 reapply 时该文件当然存在——那是我们自己写的——于是它得出结论「接管前就有 profile」，**「恢复接管前配置」变成把工具自己的 profile 写回去**，而不是撤回。

这一条同时解释了 L10 首次执行时看到的 `plan profile update`：恢复成了空操作。

修复分两步：

1. 把「接管前状态」变成**显式输入**而不是从磁盘推断：`TakeoverBaseline { profile: ProfileBaseline, modes }`，其中
   - `Absent`：接管前不存在 → 撤回应删除；
   - `PreExisting(Value)`：接管前已存在且内容已记录 → 撤回应写回它；
   - `Unknown`：无法判断 → **不写也不删**，避免把工具自己的文件声称成用户原有配置。
2. 没有管理记录时（首次接管），用 `profile::looks_like_ours()`（判据是「本工具写入的那组键同时存在」）区分「磁盘上那份是我们自己遗留的」与「用户原有配置」。

用例：`repeated_applies_never_move_the_first_baseline`（连续三次应用后基线不变，且恢复确实删除 profile）、`rebaseline_adopts_the_current_state_as_the_new_baseline`（含 `profile_managed` 两种取值的语义）。

### 实施事故（记录在案）

写 L10 的「还原卡片密钥」守卫时，我用 `save_provider` 只回写了密钥字段，**未提供的字段被写入空值**，把 `kiro-fucheers` 卡片的名称、模型列表与余额凭据一起抹掉了。发现后从执行前的 store 快照（`newapi-providers.dat.pre-l10`）整文件还原，六张卡片的密钥长度与 `userId` 均已核对回原值。

修正：该守卫改为**整套 store 文件的快照 + 还原**。教训与既有实现一致——只回写部分字段的「补丁式还原」会静默清空其余字段，正确做法是文件级快照。

### L10 结果（2026-10-04 21:2x）

```text
before: state=managed stale=[]
after rotation: state=stale stale=["credential"]        ← 改密钥后正确提示
after reapply: state=managed stale=[]                   ← profile 换成新密钥，旧密钥不残留
BEFORE RESTORE profile existed=false baseline_is_none=true  ← 基线正确
  plan profile noop                                     ← 已被 App 删除，无需再删
profileExists=false   L10: 无密钥残留
```

## 五、回滚路径

**界面入口（按彻底程度从高到低）**

| 操作 | 效果 | 什么时候用 |
| --- | --- | --- |
| 恢复接管前配置 | 两个 config 各自回到接管前的 `deploymentMode`（没有该键则删除），`_meta.json` 回到接管前的自身条目与 `appliedId`，自身 profile 写回接管前内容或删除 | 想完全回到接管前的磁盘状态 |
| 切换官方模式 | 两个 config 写成 `1p`，移除自身 meta 条目与 `appliedId`，自身 profile 同上 | 想撤销第三方直连、但接受官方模式 |
| 释放管理 | **不改任何 Desktop 文件**，只结束恢复责任（tombstone）并写一份备份 | 只想让 AgentHarbor 不再托管；注意磁盘上的 profile 仍含凭据，且之后**失去**“恢复接管前配置”的入口 |

想彻底移除落在 profile 里的密钥，应当使用“恢复接管前配置”或“切换官方模式”，而不是“释放管理”；也可以在上游轮换密钥。

**本地备份**

- 位置：应用数据目录下的 `claude-desktop/backups/*.dat`（本机应用数据目录是 `%USERPROFILE%\.codex\agentharbor`）。
- 每次提交与每次释放前都会写一份，内容是四个文件的 before 快照（含 profile 原文，因此**含凭据**），用 `secret_file` 写入（Windows 当前用户 DPAPI；Unix 0600、不加密），**最多保留 20 份**。
- 卡片上显示当前备份数量；备份只在应用数据目录内，不会写进 Claude Desktop 目录。

**手动回滚**

- 执行真实验收前自行复制四个文件；需要时整文件写回即可。
- “恢复接管前配置”本身也只处理受管值（`deploymentMode`、自身 meta 条目与 `appliedId`、自身 profile），外部条目、MCP、偏好与企业策略键都不会被触碰。

**基线语义（容易误解的一点）**

- “恢复接管前配置”回到的是**首次接管的基线**，不是最近一次应用的状态。
- 普通重新应用（reapply）**不会移动**这个基线（`first_baseline_is_preserved_across_reapply`）；只有显式勾选“按当前状态重新接管”才会用当前磁盘状态替换还原点。
- 如果中途出现未完成事务，卡片会显示并提供“恢复未完成的事务”入口；在恢复之前，其它写入会被拒绝。

## 六、结论

**首期交付可用性判定：自动化验收通过；真实 Desktop 已在 Windows + 2.19675.0 上完成核心链路实测（配置被接受、菜单生效、真实对话成功、出站请求形态已抓包证实）。**

- 自动化侧：`pnpm test:rust`（**697 项 lib + 9 项集成**，其中 `core::claude_desktop` **69 项单测**、`tests/claude_desktop_workflows.rs` **11 项端到端 + 6 项显式 `--ignored` 真实核对**）、`pnpm test:claude-desktop-config`（10/10 组）、`pnpm test:frontend`（exit 0）、`pnpm build`（exit 0）全部通过；连续 12 次 `cargo test --lib` 无偶发失败。
- 计划侧：A01–A40 中 **22 项通过、18 项部分通过、0 项未覆盖**；部分通过的具体缺口见 2.3 节“说明”列。
- 真实侧：**L07 通过（文件层）**（切官方模式后两处 `1p`、外部条目保留、非受管键逐字节未变、profile 撤回，随后立即恢复直连）。**L10 通过**（密钥变更 → stale → 重新应用 → 恢复，无密钥残留）。**L01 通过**（profile 被接受、3P 生效、`ConfigHealth healthy`、省略 `coworkEgressAllowedHosts` 的候选在该版本可用）、**L02 通过**（抓包证实 `POST /v1/messages`、只带 `Authorization`、模型名原样出站）、**L03 通过**（配置 3 个模型并带展示名后，界面模型选择器显示 `Sonnet 4.6`，证明菜单确实按 `inferenceModels` 切换）、**L04 部分通过**（多轮对话成功且 `is_resume=true`、transcript 递增；**流式在链路侧已验证**（`text/event-stream` + 完整事件序列），Desktop 侧未单独观察）、**L05 通过**（真实工具调用：`Read a file` → `Turn succeeded`）、**L06 部分通过**（应用侧已实测，本机未做过一次真实恢复）、**L08 部分通过**（本轮真实对话都在本项目未运行时发生，但"本地路由已启动后再退出"这一组合未验）。**L 系列 10 项全部执行完毕，其中 7 项完全通过、3 项部分通过（L04 的 Desktop 侧流式渲染、L06 的真实恢复、L08 的"本地路由已启动后再退出"组合），L07 的官方登录流程本身未验证（文件层已通过）。**
- 本机 Desktop 曾被本次实施接管，并已按要求还原到真实上游；四文件内容与接管前一致（`appliedId` 仍为 AgentHarbor，CC Switch 条目保留）。执行前副本见 3.1b。
- 本段记录的是实施阶段的核心链路结果。复审发现的恢复与隔离缺陷及其修复结果见文首；当前不能表述为“计划完整验收通过”。仍缺 Desktop 流式渲染、L06 真实恢复、L08 组合和 L07 官方流程；macOS/Linux、Cowork 未完整验证。

**还没做的（按优先级）**

1. **L07 的官方流程未执行**；文件切换已验证。L09 已由 loopback mock 核对限制路径，不代表某个真实中转的兼容结论。
2. **Desktop 侧的流式渲染未单独观察**。链路已验证支持 SSE，捕获到的 Desktop 请求是它的验证请求（`stream=false`）；要确认 Desktop 自己走流式，需要发一条较长回复的消息并核对日志。
3. **仍未覆盖的分支**：A22 的 `pendingCleanup` 具体触发路径（需要确定性制造文件删除失败，当前只断言了它的承诺）、A32 的提交前 `verify_unchanged`（预览后、提交前文件被外部改动）、A35 的 401/403/404/429/超时分类（需要 mock HTTP 服务）、A29 的 `rebaseline` 端到端、A14 的重复模型选择、A19 以具体 `enterpriseConfig` 键为对象的用例、A27「接管前不存在 + 应用后用户又加外部键」的组合。
4. **macOS / Linux 未在真机验证**。启动方式与路径规则都有纯函数用例，但 `macos_bundle_installed`（`mdfind`）与 Linux 的桌面文件/PATH 探测只在代码层面成立；本轮修掉了它们「无条件报告可重启」的缺陷，但没有真机证据。
5. **Cowork 未验证**。首期 profile 省略 `coworkEgressAllowedHosts`，而本机 Cowork 实际能跑（用户截图里的 `Read a file` 就是 Cowork 回合），但「省略该字段是否在所有网络策略下都可行」没有结论，首期不承诺。
6. **抓包验证网关仍是一次性工具**（`--ignored` 的 `verify_relay`）。已修为「按 `Content-Length` 读满 + 窗口期内不限连接 + `Drop` 兜底还原 profile」，但它不是交付物；**不得在用户正在使用 Desktop 时运行**。
7. 实施阶段已形成六个提交，末端为 `5030efd`；复审修复在此基线上进行。
