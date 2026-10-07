# 模型聚合与协议解耦验收记录

日期：2026-10-06。本文记录 `plan/model-aggregation-and-protocol-decoupling-execution-todo.md` 的基础交付与 P7 补交付状态。实施计划提交为 `5fb0f1c`；基础聚合提交为 `9906616`，测试夹具修复为 `b9adcfb`。P7 公共事件、统一终态与通用 Chat 扩展已补齐，真实环境验收仍单列未完成。

## 交付范围

本轮完成 P0–P6 的基础实现，保留已有 provider 卡片、单渠道直连、Codex、Claude Code、Claude Desktop、OpenCode 的保存/reapply/restore/ownership 与恢复事务。旧的全局“同模型渠道故障切换”已从产品入口、命令、视图和自动候选中退役；历史字段只用于兼容读取和迁移，不自动创建聚合。

聚合是独立目标层：

- `aggregateId` 是内部身份，`publicModelId` 是客户端看到的稳定模型 ID，成员保存精确 `actualModelId` 和 provider 引用。
- 聚合成员只引用已有供应商和模型，不复制 endpoint、密钥、协议、能力覆盖或价格。
- 选择先做能力/协议准入，再按优先级、轮询或平滑加权轮询调度；真实目标共享冷却和并发 lease。
- 最多两次重试仅适用于连接建立前或 429、请求必须是可移植的新请求且尚未向下游提交内容；工具、搜索、私有历史、资源续接、超时和副作用请求不盲目重放。
- Responses/Messages/Chat 聚合入口使用 `/aggregate/v1`；成员可使用 Responses、Chat Completions 或 Anthropic Messages 出口，入口协议保持稳定。Chat 两个新增转换方向在 P7 补交付中验收。
- 聚合 Responses/Messages/ Desktop 响应的协议 `model` 字段投影为 `publicModelId`。普通单渠道 Desktop 继续使用角色 alias；诊断和账本保留 alias、实际成员、wire model、有效协议和价格快照。
- Claude Code 与 Desktop 聚合的 1M 请求按真实容量准入，可跨三种出口筛选；只有 Messages 出口添加 Claude beta，Chat/Responses 不编造 1M 参数。直连客户端原有协议边界不随本轮改变。

## 使用流程

1. 在供应商页的模型聚合入口新建一个公开模型，选择或明确填写官方模型 ID 和展示名。
2. 添加已配置中转卡中的精确模型为成员，设置优先级、权重、轮询策略和是否保留会话亲和；不需要重复填写中转地址或密钥。
3. 保存后在 Codex、Claude Code、Claude Desktop 或 OpenCode 的配置选择器中选择该聚合目标，预览后应用。原有单渠道配置、保存卡和恢复入口继续可用。
4. 其它 agent 使用聚合导出入口，按客户端需要选择 Responses、Messages 或 Chat 以及对应模板。预览脱敏；显式复制配置时才解析本地 Router token，导出不包含上游密钥。
5. 仅供外部 agent 使用时启用“保持聚合服务运行”。轮转/权重修改无需重新写入客户端文件；公开身份、所选目标或本地 token 等实际配置内容改变后，再预览或重新导出。

公开模型 ID 创建后保持固定，关联建议不会改写实际 wire ID。Chat 入口通过新增请求/响应/SSE 转换消费混合池，并非仅把 Chat 出口成员包装成一个模板。OpenCode 的新增 Chat 分组是 `agentharbor_aggregate_chat`，使用 `@ai-sdk/openai-compatible`；既有 Responses/Messages 分组保持原名。

### 公开名称与容量配置

公开模型 ID 候选复用所有卡片的显式目录引用与能力预览匹配结果，按 ID 去重。当前编辑的聚合成员对应候选排在前面，关联使用卡片 ID 与精确真实模型 ID；其它卡片候选仍保留，输入框允许手填名称。

公开名称没有目录匹配不会导致容量丢失：聚合始终按真实成员解析能力，复用模型级手填覆盖、供应商设置、上游声明与目录数据。可用成员的上下文、输入预算和输出限制取保守最小值；推理等级取交集。聚合有独立压缩策略，目前没有独立上下文或最大输出覆盖字段。需要补能力时，在原卡片的模型配置中填写真实上下文、输入上限或最大输出，再让聚合重新投影。

真实成员未匹配且没有其他能力来源时，客户端数值回退为上下文 `128,000`、最大输出 `8,192`。回退来源保留为 `fallback`，不能把它当作上游能力声明，也不能据此宣称支持 1M。输出回退参与成员准入：客户端请求输出超过该值时，该成员会被跳过。补齐真实成员能力后，回退标记自动消失；禁用成员不会参与汇总。

Codex catalog 和 OpenCode limits 消费投影的具体数值；Claude Code 与 Desktop 只消费各自支持的公开模型/角色和 1M 选项，不会将任意上下文、输出字段写进其配置。通用连接导出的 limits 当前仅包含数值，未携带来源字段。

2026-10-06 本次调整的验证：聚合浏览器回归与 TypeScript 检查通过；`aggregation::` 的 24 条和 `aggregate_tests` 的 30 条 Rust 回归通过。覆盖相关候选排序、同名模型跨卡片隔离、手填 ID、未匹配容量回退与来源、能力覆盖更新、禁用成员排除，以及既有注入/1M/三协议转发用例。未重复执行真实计费请求。

## 实现位置

| 层 | 主要路径 | 验收关注点 |
| --- | --- | --- |
| 聚合配置 | `src-tauri/src/core/aggregation/{types,config,projection,selector,runtime,affinity,execution,tests,mod}.rs` | Store v9、引用完整性、保守能力投影、轮询/权重、冷却、lease、亲和、重试边界 |
| 共享目标 | `src-tauri/src/core/model_targets.rs`、`model_targets_export.rs` | Provider/Aggregate typed target、可用性/修订、无密钥 resolver、通用/Anthropic SDK/OpenCode 导出 |
| 协议边界 | `src-tauri/src/core/protocol/{types,adapters,canonical,chat_ingress,chat_sse}.rs`、`router_proxy/{forward,native_sse}.rs` | 公共事件、统一终态、同协议保留、跨协议适配、结构化错误，普通 provider 与聚合共用准备/传输/落账逻辑 |
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

## 基础交付的自动化证据

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

## P7 补交付：公共事件、终态与 Chat

新增 `canonical.rs` 的消息/工具 borrowed views、usage、来源扩展与公共事件，兼容现有 JSON 历史。事件区分上游 `source`、内部 `payloadProtocol` 和是否原始 wire：解码后的 Responses 载荷不能冒充上游 Chat/Messages 回放。原生路径保留原始字段/事件；跨协议无法表达的 hosted/custom tool、私有签名、加密 reasoning 和非法工具历史明确拒绝。

Chat→Messages 的增量链路改为 Chat decoder→公共事件→Messages encoder，省去中间 Responses SSE 的序列化和再次解析。新增 Chat→Responses/Messages 请求及对应 JSON/SSE 返回转换，覆盖文本/图片、并行 function tools、参数增量、工具结果历史、tool choice、usage 与正常 incomplete。`/aggregate/v1/chat/completions` 可在三种出口成员间轮转，导出、共享目标与 OpenCode Chat SDK 分组已同步。

历史、请求诊断、trace、实际费用和聚合健康恢复共用终态语义：completed、合法 incomplete、failed、EOF truncated 分开处理。Responses `response.incomplete` 可展示，不能默认当失败；切断工具参数不进入成功历史。Chat 的正常完成要求 finish_reason 后收到 `[DONE]`，裸 `[DONE]` 不代替 Messages/Responses 终态。原生帧门控在转发前阻止非法 DONE，避免 SDK 提前结束而漏读后续错误；EOF 判断独立于诊断采样上限，故障帧有独立空行边界。half-open 渠道只在有效终止后恢复。

SSE 首帧 BOM 在门控、诊断、用量、工具适配和聚合身份投影中统一识别，非法 UTF-8 帧明确失败。合法原生 BOM 与未知事件保留原字节；只有本地 keepalive 已先行输出时，去掉首个上游帧的 BOM，避免它落到流中间变成字段名前缀。聚合 BOM 回归还验证 model 改写、CRLF 与资源 ID 登记；keepalive 顺序通过 Notify 控制，避免仅靠计时猜测。

没有启用用于重试的首段暂存：headers/keepalive 提交后仍不换成员。原生门控仅做有界 SSE 分帧，单帧 8 MiB；跨协议帧仍限 8 MiB、累计保留内容限 16 MiB，超限不发送成功终态。聚合公开身份投影仍限单事件 256 KiB、JSON 16 MiB；两个历史缓存各自保留全局 512 项/16 MiB 上限，Messages 缓存还有 1h TTL。不是按每个聚合各开一份无界历史。

### 分帧与性能基线

`performance_tests.rs` 正常执行 6 项，覆盖逐字节 Unicode、每个双段切分位置、交错工具、历史与事件差分、单帧超限和多块累计超限。测量基准单独执行 1 项，MSVC debug、每项预热并采样 9 次，取总耗时中位数。模拟旧桥接用当前 decoder/encoder 重建字符串路径；差分只归一化随机 ID，没有对旧版独立 EXE 作比较。

| fixture / 指标 | 公共事件 | 模拟旧字符串桥接 |
| --- | ---: | ---: |
| 2,011 B 工具流总耗时 | 2,328 µs | 6,878 µs |
| 工具流首文本编码耗时 | 221 µs | 814 µs |
| 工具流分配次数 | 2,147 | 2,747 |
| 155,867 B 文本＋工具流总耗时 | 10,772 µs | 47,056 µs |
| 文本＋工具流中间 JSON 编码/解析 | 0 / 0 | 154 / 154 |
| 文本＋工具流累计申请分配字节 | 5,108,617 B | 10,674,669 B |

1,549,546 B / 545 条消息历史：现有 tools-only context 构建中位数 89,191 µs、21,408 次分配；模拟再做一次全历史转换为 120,206 µs、38,693 次分配。context 仍解析入站 JSON 一次，避免的是额外完整转换。累计申请分配量不是峰值内存，首文本编码耗时不含网络与推理，不能外推固定性能倍数；真实长流/并发的进程峰值和网络首 token 留在 P8。

复现命令：

```powershell
node scripts/tauri.mjs --test-rust performance_tests -- --nocapture --test-threads=1
node scripts/tauri.mjs --test-rust p7_measure_stream_and_large_history_baselines -- --ignored --nocapture --test-threads=1
```

### 本轮自审修正

- 新 Chat 路径不再忽略 Messages hosted/未知内容与私有扩展。
- 原生 EOF 输出可解析的错误帧；非法 DONE 在普通和聚合路径都先拦截。
- Chat 故障探活不会在仅 finish_reason、未 DONE 时提前清除冷却。
- 事件来源与载荷格式分开，生成的内部 Responses JSON 不能作原始 Chat 回放。
- 持续小帧/多块输出共享累计限制，超限释放保留内容；原生分隔符从新增字节附近继续扫描。
- 大于诊断采样预算的原生事件仍独立检查 EOF 终态；普通/工具适配两路径覆盖三种协议的大事件有/无结束帧。
- 非 UTF-8 和首帧 BOM 不能隐藏非法 DONE；合法 BOM 的投影、工具参数、费用及 keepalive 顺序均有回归覆盖。
- 聚合注入的协议选择仅向 OpenCode 开放；Codex 继续使用 Responses，Claude Code 继续使用 Messages。
- 旧“反向 Chat 转换不支持”断言更新为实际生成请求的验证，原有 Responses→Chat 行为断言保留。

A44（Chat 新入口）与 A45（接口/事件差分）已有独立隔离回归。托管搜索的真实服务能力、旧/新独立版本的网络性能和全部私有扩展组合仍不由这些 fixtures 代替。

### 本轮最终自动化结果

| 命令/检查 | 结果 | 范围 |
| --- | --- | --- |
| `pnpm test:rust`（`RUST_TEST_THREADS=1`） | 通过 | 库测试 `972 passed, 0 failed, 12 ignored`；Desktop 工作流 `11 passed, 11 ignored`；本地工作流 `20 passed`。包含最终原生终态门控、BOM/keepalive 及既有四类注入回归。 |
| `pnpm test:frontend` | 通过 | Desktop 配置契约、聚合管理与既有前端回归。 |
| `FRONTEND_QA_SCOPE=aggregation pnpm test:frontend` | 通过 | 最后一次 UI 调整后复跑聚合编辑/导出、四种注入、保存卡、协议选择和移动布局；Desktop 配置契约检查同时通过。 |
| `pnpm build` | 通过 | TypeScript 与 Vite 生产构建。 |
| Chat 入口定向测试 | 通过 | `14 passed`；完整 Rust 回归再次覆盖。 |
| 分帧/性能普通测试 | 通过 | `6 passed, 1 ignored`；完整 Rust 回归再次覆盖。 |
| 单独性能基准 | 通过 | `1 passed`；结果与限制见上表。 |
| `git diff --check` | 通过 | 本轮源码、测试、文档没有空白错误。 |

第一轮完整 Rust 唯一失败是旧测试仍断言 Chat→Responses 不支持；修正为验证真实请求转换并保留原有 Responses→Chat 断言后，最终完整回归全部通过。未运行需要真实凭据、计费请求或真实客户端文件修改的 ignored 用例。

完整套件包含既有余额用例；P7 本轮未调整余额产品逻辑，也未重复查询真实余额。基础交付的 `b9adcfb` 只修复全量回归暴露的本地回环测试隔离与等待上限。

## P8.2 追加实测：Command + DeepSeek Flash

2026-10-06 按用户授权复用两张已保存卡片。用户确认 DeepSeek 官方的 `deepseek-flash` 对应本次测试的 DeepSeek 4.1 Flash；公开模型为 `deepseek-v4.1-flash`，成员 wire ID 保持精确，不使用 Command 的 `-fast` 变体。

实测基线 `32e4191`；协议修复、隔离 live harness 和回归提交为 `70a8ca0`。

| 成员 | 实际出站模型 | 已保存的有效出口 | 最终请求数 |
| --- | --- | --- | --- |
| Command Code | `deepseek/deepseek-v4.1-flash` | Chat Completions | 8 |
| DeepSeek 官方 | `deepseek-flash` | Anthropic Messages | 8 |

测试入口为 `router_proxy/aggregate_live_tests.rs::command_and_deepseek_flash_live_aggregation`，平时标记 `ignored`。它只读解密生产卡片，不走迁移或保存；在临时目录保存两成员和一个临时聚合，开启独立测试进程内的随机 localhost 监听，沿用已保存的 `10808` 代理。测试客户端绕过代理连接 localhost。测试结束销毁 runtime、监听、临时密钥和账本，生产卡片文件逐字节核对未变；未调用真实客户端的注入、停进程、保存或恢复入口，也未查询真实余额。

### 最终真实请求结果

| 入站 | JSON：Command / DeepSeek | SSE：Command / DeepSeek | 覆盖 |
| --- | --- | --- | --- |
| Responses | 通过 / 通过 | 通过 / 通过 | 混合两出口、公开 `response.model`、成功终态及实际成员用量 |
| Messages | 通过 / 通过 | 通过 / 通过 | 混合两出口、公开 `message.model` / `message_start.message.model`、`end_turn` |
| Chat Completions | 通过 / 通过 | 通过 / 通过 | 原生 Chat 和 Chat→Messages、公开 `model`、finish reason 与 `[DONE]` |
| Messages 普通函数工具 | 两成员各通过两轮 | 本次未测工具流 | 首轮 `tool_use` 与参数、客户端回填 `tool_result`、同 session 保持原成员 |

最终 **16/16 HTTP 200**，每请求恰好一个实际 attempt。前 12 个无 session 的新请求按 `Command → DeepSeek` 交替；后 4 个分属两组独立会话，结果回填使用 `aggregate_session_binding`，没有把工具历史交给另一成员。测试将重试上限设为 1，不能用备用成功掩盖某个成员失败。

每条记录都核对实际 provider、member、wire model、配置/有效协议、上游自报 model、公开 model 和实际成员账本；不只检查回复文字。Messages 的 `max_tokens` 截断即使包含 `OK` 也不能被此验收判为完整通过，普通回归另锁定了这一判据。

最终账本 **16 条、16 条成功、16 条上报 usage**：输入 1,234 tokens，输出 333 tokens（其中 reasoning 198），总计 1,567。该组模型在本机没有可用价格绑定，16 条费用保持 `unpriced / costUsd: null`；不能把 totals 的数值 0 当成免费或真实扣费。首次 token 样本：Command 1,598–2,168 ms；DeepSeek 454–762 ms，仅为这 6 条流式短请求的网络实测，不是性能 SLA 或长流/并发基准。

### 实测发现并修复

首轮发出 12 个短请求，Responses/Messages 的 8 个通过，Chat 的 4 个失败。随后用 3 个合成请求只记录上游错误字段及 block/delta 形态，确认：

- 测试给原生 Chat 加了 `reasoning_effort: none`，Command 返回 400 并指出该字段；这是测试额外参数不兼容，最终普通 Chat 用例移除该提示，未更改保存的模型协议。
- DeepSeek Messages 不传 `thinking` 时默认返回非空签名的 thinking block；流先发空签名占位，随后发真正的 signature delta。既有 Chat 对私有签名的拒绝生效，但 Chat→Messages 的普通请求准备没有阻止这种默认输出。
- 修复显式 `none` 为 `thinking: {"type":"disabled"}`，包含工具续轮；新 Chat→Messages 在未指定/null effort 时也显式禁用思考。Responses 未指定 effort、原生 Messages、其它 effort 和未知值的已有行为保持。仍拒绝跨协议私有签名，不通过删除签名让测试变绿。

定向协议回归 **23/23** 通过，覆盖请求参数、工具续轮、DeepSeek 文本 JSON/SSE、分帧、usage 与终态。最终 16 请求复验全部通过。首轮、形态诊断和最终复验合计 31 个合成短请求；没有发送用户历史、文件或搜索任务。

修复后完整 `node scripts/tauri.mjs --test-rust -- --test-threads=1`：库测试 **975 passed / 13 ignored**、Desktop 工作流 **11 passed / 11 ignored**、本地工作流 **20 passed**，全部 0 failed；live 用例只在上面的命名运行中执行。`git diff --check` 通过。

证据保存在本机忽略目录：

- `.runtime-test/aggregation-command-deepseek-20261006-first.{log,json}`：首轮失败证据。
- `.runtime-test/aggregation-command-deepseek-shapes-20261006.json`：脱敏上游形态，不含签名内容或正文。
- `.runtime-test/aggregation-command-deepseek-20261006-final.{log,json}`：最终 16 条 attempt 与账本对账。
- `.runtime-test/aggregation-live-rust-regression-20261006.log`：修复后完整 Rust 回归。

重复实测需显式设置 `AGENTHARBOR_AGGREGATION_LIVE_STORE` 为已有加密卡片文件、`AGENTHARBOR_AGGREGATION_LIVE_REPORT` 为一个全新的脱敏报告路径，然后只运行命名用例：

```powershell
node scripts/tauri.mjs --test-rust command_and_deepseek_flash_live_aggregation -- --ignored --nocapture --test-threads=1
```

这次只覆盖两个已保存出口（Chat、Messages）。Responses 作为出口、真实四类客户端注入/SDK、托管 WebSearch、1M 长上下文、真实故障/冷却和长流并发仍未验证，不将 P8.2 整体打勾。

## 尚未完成或未运行

### P8：真实环境与交付

- P8.2：已用 Command/DeepSeek 的真实凭据验证两成员、Chat/Messages 混合出口、三种入站 JSON/SSE 及 Messages 两轮函数工具；四类真实注入/SDK、WebSearch、1M、Responses 出口和外部 agent 导出消费仍未验证。
- P8.3：未完成单渠道/Native/官方与聚合并行的真实暂停、退出、重启、恢复组合；未修改真实 Desktop、Codex、Claude Code 或 OpenCode 配置。
- P8.4：本记录、计划和相关说明文档已补齐，自动化及两卡真实测试证据已补录；未覆盖的真机和外部消费者项仍未验证。
- P8.5：已完成完整工作区自审、`git diff --check`、费用边界回归修复和最终提交；本轮未执行发布包打包。

真实凭据、付费请求、外部搜索服务和真机结果不能由 localhost mock 通过替代。基础实现未改变任何真实客户端文件或上游配置。

## 2026-10-07：账号和用量页记账复查

聚合执行复用 `forward` 的用量观察器，在公开模型名投影之前读取实际出口的 usage，并持久化到应用数据目录的 `relay-usage.jsonl`。账号和用量页通过 `get_token_usage` 加载同一账本，展示中转趋势、按模型/供应商统计与请求明细。

- 统计归属实际命中的供应商卡片及出站模型；客户端请求的公开模型或 Desktop 档位保存在请求身份字段，不能再次作为费用记录叠加。
- JSON 与 SSE 完成后记录一次实际推理 attempt；计数、读取/取消/删除响应资源不会重复记录原响应的用量，派发前失败也不会产生推理账目。
- 发生 429 后重试时，两次实际 attempt 各有独立记录。未上报 usage 的失败记录保留状态，不估算 token 或费用；成功成员使用其自己的价格快照。价格缺失显示未定价，不将其当作免费。
- 中转用量不关联 Codex 官方账号的订阅额度。界面刷新后可见保存的统计；余额仍来自供应商余额查询，不由估算费用反推。

扩充已有回归断言，直接核对账号页消费的账本 payload：Responses 入站轮转、Chat 入站的三种出口 JSON/SSE、Desktop Messages 的三种出口 JSON/SSE，均校验总请求数、输入/输出 token、按实际供应商/模型分组、attempt 唯一性及拒绝请求不增加记录。相关聚合 HTTP 回归 **20/20 通过**，包含重试价格快照与资源操作测试。证据：`.runtime-test/balance-aggregation-usage-rust.log`。未额外发起真实余额或付费模型请求。
