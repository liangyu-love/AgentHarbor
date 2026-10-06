# 模型聚合与协议解耦验收记录

日期：2026-10-06。本文记录 `plan/model-aggregation-and-protocol-decoupling-execution-todo.md` 的基础交付状态。实施计划提交为 `5fb0f1c`；基础聚合提交为 `9906616`，测试夹具修复为 `b9adcfb`。P7 与真实环境验收仍保持未完成。

## 交付范围

本轮完成 P0–P6 的基础实现，保留已有 provider 卡片、单渠道直连、Codex、Claude Code、Claude Desktop、OpenCode 的保存/reapply/restore/ownership 与恢复事务。旧的全局“同模型渠道故障切换”已从产品入口、命令、视图和自动候选中退役；历史字段只用于兼容读取和迁移，不自动创建聚合。

聚合是独立目标层：

- `aggregateId` 是内部身份，`publicModelId` 是客户端看到的稳定模型 ID，成员保存精确 `actualModelId` 和 provider 引用。
- 聚合成员只引用已有供应商和模型，不复制 endpoint、密钥、协议、能力覆盖或价格。
- 选择先做能力/协议准入，再按优先级、轮询或平滑加权轮询调度；真实目标共享冷却和并发 lease。
- 最多两次重试仅适用于连接建立前或 429、请求必须是可移植的新请求且尚未向下游提交内容；工具、搜索、私有历史、资源续接、超时和副作用请求不盲目重放。
- Responses/Messages 聚合入口使用 `/aggregate/v1`；成员可使用 Responses、Chat Completions 或 Anthropic Messages 出口，入口协议保持稳定。
- 聚合 Responses/Messages/ Desktop 响应的协议 `model` 字段投影为 `publicModelId`。普通单渠道 Desktop 继续使用角色 alias；诊断和账本保留 alias、实际成员、wire model、有效协议和价格快照。
- Claude Code 与 Desktop 聚合的 1M 请求按真实容量准入，可跨三种出口筛选；只有 Messages 出口添加 Claude beta，Chat/Responses 不编造 1M 参数。直连客户端原有协议边界不随本轮改变。

## 使用流程

1. 在供应商页的模型聚合入口新建一个公开模型，选择或明确填写官方模型 ID 和展示名。
2. 添加已配置中转卡中的精确模型为成员，设置优先级、权重、轮询策略和是否保留会话亲和；不需要重复填写中转地址或密钥。
3. 保存后在 Codex、Claude Code、Claude Desktop 或 OpenCode 的配置选择器中选择该聚合目标，预览后应用。原有单渠道配置、保存卡和恢复入口继续可用。
4. 其它 agent 使用聚合导出入口，按客户端需要选择 Responses 或 Messages 以及对应模板。预览脱敏；显式复制配置时才解析本地 Router token，导出不包含上游密钥。
5. 仅供外部 agent 使用时启用“保持聚合服务运行”。轮转/权重修改无需重新写入客户端文件；公开身份、所选目标或本地 token 等实际配置内容改变后，再预览或重新导出。

公开模型 ID 创建后保持固定，关联建议不会改写实际 wire ID。只支持 Chat 的 agent 当前没有通用混合池模板；不能用成员恰好包含 Chat 出口来推断 Chat 入口已开放。

## 实现位置

| 层 | 主要路径 | 验收关注点 |
| --- | --- | --- |
| 聚合配置 | `src-tauri/src/core/aggregation/{types,config,projection,selector,runtime,affinity,execution,tests,mod}.rs` | Store v9、引用完整性、保守能力投影、轮询/权重、冷却、lease、亲和、重试边界 |
| 共享目标 | `src-tauri/src/core/model_targets.rs`、`model_targets_export.rs` | Provider/Aggregate typed target、可用性/修订、无密钥 resolver、通用/Anthropic SDK/OpenCode 导出 |
| 协议边界 | `src-tauri/src/core/protocol/{types,adapters}.rs`、`router_proxy/forward.rs` | 同协议透传、跨协议适配、结构化错误，普通 provider 与聚合共用准备/传输/落账逻辑 |
| 聚合 HTTP | `src-tauri/src/core/router_proxy/{aggregate,aggregate_output,aggregate_tests}.rs` | `/aggregate/v1` 路径、目录投影、资源归属、`count_tokens`、Responses `compact`、SSE/JSON 投影、流取消和 lease 释放 |
| 路由诊断/账本 | `src-tauri/src/core/route_trace.rs`、`relay_usage.rs`、`router_proxy.rs` | request/attempt/member/route/catalog 修订、跳过原因、有界诊断、真实模型与实际价格记录 |
| 注入器 | Codex、Claude Code、Desktop、OpenCode 的 client_config/commands/UI | typed target、保存卡/预览/reapply/restore、原有 ownership 与多模型选择不缩水 |
| UI/回归 | `src/components/newapi/aggregation-manager.tsx`、`scripts/test-aggregation.mjs` | 聚合编辑、精确 wire ID、脱敏导出、不可用目标不隐式替换、移动宽度回归 |

## Mock 与隔离覆盖

下表把计划中的 A01–A43 对应到实现与已有隔离回归入口，不表示每个组合已全部通过。组合覆盖与完整命令的最终退出码仍以交付前复跑为准；没有度量证据的性能项单列待验。

| 矩阵 | 代码/测试路径 | 覆盖内容 |
| --- | --- | --- |
| A01–A05 | `aggregation/tests.rs`、`model_targets.rs`、`router.rs` | v8→v9 迁移、旧 failover 丢弃、原子保存、public identity 唯一且不可悄悄改写、前缀/后缀/fast 关联只作建议、actual ID 精确出站 |
| A06–A10 | `aggregation/tests.rs`、`projection.rs`、`execution.rs` | 混合协议成员、保守上下文/输出投影、缺密钥/禁用/删除/排除成员、工具/图片/搜索/1M 能力准入 |
| A07–A14 | `aggregation/runtime.rs`、`affinity.rs`、`model_targets.rs` | 优先级、round robin、平滑加权、共享物理目标冷却、并发容量、会话/客户端/角色 scope、route/catalog/file 修订 |
| A15–A21 | `router_proxy/aggregate_tests.rs`、`aggregation/affinity.rs` | previous response/resource 归属、cancel/delete/input_items、未知/冲突资源、两次安全重试、Retry-After、撤销凭据、提交后不换成员 |
| A22–A29 | `router_proxy/aggregate_tests.rs`、`aggregate_output.rs`、`claude_desktop/aggregate_tests.rs` | 128K/1M/未知容量筛选、三出口 1M、JSON/SSE `model` 投影、未知事件保留、SSE 帧/JSON 上限、终态与 lease、原生 Responses compact、Messages count_tokens |
| A30–A34 | `client_config/aggregate_tests.rs`、`claude_desktop/aggregate_tests.rs`、`scripts/test-aggregation.mjs` | Codex/Claude Code/Desktop/OpenCode 的聚合 target、保存/应用/reapply/restore、四文件事务、listener 共享、不可用目标不 fallback |
| A35–A38 | `router_proxy/aggregate_tests.rs`、`relay_usage.rs`、`route_trace.rs` | 每 attempt 真实成员/价格快照、零价/未知费用、诊断有界、普通 relay/official/native/直连路径保持独立 |
| A39–A43 | `client_config/aggregate_tests.rs`、`model_targets_export.rs`、`routing_runtime.rs`、`scripts/test-aggregation.mjs` | OpenCode 三 SDK/多 provider/子集/headers/variants/custom models、版本化 export、SDK URL/目录格式、外部使用需求与 listener 生命周期 |

尚不能把下列验收场景标为完整通过：A24 的全部 namespace/native 工具组合、A25 的真实 Command search/history-only 三轮、A26–A28 的所有私有扩展跨协议差分、A34/A43 的真实多消费者退出/token 轮换，以及 A37 的连续长流/大历史峰值内存与 CPU 测量。现有普通协议/搜索/客户端基线用例继续保留；它们不替代新增聚合组合的实测。

费用边界：推理 create/stream（包括失败响应中可靠的 usage）按实际成员和模型价格快照落账；`messages/count_tokens` 只做能力准入和计数，不落账；Responses resource 的 `HEAD`、`GET`、`cancel`、`delete` 等资源操作不新生成推理用量，也不因查询到旧 usage 重复落账；`responses/compact` 是实际上游推理操作，按实际成员/模型快照落账。未知 usage 或未知价格保持 unknown/partial，不写成零费用。

代表性测试包括：

- `mixed_protocol_members_preserve_full_wire_identity_and_conservative_budget`
- `store_save_is_atomic_public_identity_immutable_and_dto_has_no_secrets`
- `v8_migration_preserves_credentials_and_first_protected_backup`
- `invalid_ranges_missing_refs_and_duplicate_members_fail_without_saving`
- `older_in_flight_success_cannot_clear_a_newer_rate_limit`
- `resource_operations_affinity_and_identity_revocation_do_not_reselect`
- `native_resource_head_cancel_delete_keep_empty_bodies_and_success_status`
- `aggregate_count_tokens_selects_only_native_messages_and_does_not_pin_inference`
- `standalone_compact_selects_native_responses_without_a_previous_resource`
- `aggregate_code_one_m_selects_qualified_capacity_across_egresses`
- `desktop_aggregate_projects_public_identity_and_keeps_one_m_wire_contract_across_egresses`
- `claude_aggregate_saved_request_reapplies_public_identity_and_restores_original`
- `opencode_aggregate_and_exact_provider_subset_survive_reapply_and_restore`

## 已取得的自动化证据

| 命令/检查 | 状态 | 说明 |
| --- | --- | --- |
| `cargo check --lib --no-default-features` | 通过 | 聚合、协议适配器和路由集成可编译。 |
| `pnpm test:frontend` | 通过 | 聚合 UI、既有前端回归和断言范围已调整。 |
| `pnpm build` | 通过 | Vite 生产构建通过，保留既有 chunk size 提示。 |
| `pnpm test:claude-desktop-config` | 通过 | Desktop 配置契约脚本覆盖路由/直连配置形状。 |
| `FRONTEND_QA_SCOPE=aggregation` | 通过 | 聚合管理器、导出脱敏、四种注入选择和窄窗布局。 |
| `pnpm test:rust`（串行） | 通过 | `927 passed, 0 failed, 11 ignored`。 |
| Desktop integration tests | 通过 | `11 passed, 11 ignored`。 |
| Local workflow tests | 通过 | `20 passed`。 |
| Aggregate targeted tests | 通过 | `16 passed`；另有普通 provider 辅助端点费用边界回归 `1 passed`。 |
| `git diff --check` | 通过 | 最终源码、测试和文档差分无空白错误。 |

## 尚未完成或未运行

### P7：协议 canonical/performance/通用 Chat

当前没有把 Chat decoder→公共 canonical event→Messages/Responses encoder 宣称为已交付。已有 `WireAdapter` 是行为保持的边界，仍复用现有 JSON/Responses 中间表示。以下内容继续 pending：

- 通用 `/aggregate/v1/chat/completions` 混合池入口及 Chat→Responses/Messages 两方向；当前入口明确返回 unsupported，不生成 Chat 导出模板。
- typed message/tool/usage/终态事件层、同协议未知字段差分验证、跨协议私有扩展约束。
- 大历史、分帧、首 token、峰值内存和重复转换的性能基线；首段缓冲/keepalive 的独立验收。
- Chat 模板/SDK group、托管工具/搜索在真实支持矩阵上的确认。

计划验收项 A44（Chat 新入口）和 A45（接口/事件优化前后差分）因此保持 pending。

### P8：真实环境与交付

- P8.2：未使用真实已配置中转凭据验证跨协议、双成员轮转、流式/工具/搜索/1M，也未用真实外部 agent 消费导出。
- P8.3：未完成单渠道/Native/官方与聚合并行的真实暂停、退出、重启、恢复组合；未修改真实 Desktop、Codex、Claude Code 或 OpenCode 配置。
- P8.4：本记录、计划和相关说明文档已补齐，测试证据已补录；真实凭据、真机和外部消费者项仍未验证。
- P8.5：已完成完整工作区自审、`git diff --check`、费用边界回归修复和最终提交；本轮未执行发布包打包。

真实凭据、付费请求、外部搜索服务和真机结果不能由 localhost mock 通过替代。基础实现未改变任何真实客户端文件或上游配置。
