# 请求诊断与能力预览实施基线

本变更按 `plan/magpie-follow-up-diagnostics-and-capability-plan.md` 独立实施。起点为 models.dev 主计划后的 `feat/models-dev-integration` 代码；路由策略、协议转换和客户端写回逻辑保持原有决定点。诊断字段不参与选路、重试、价格结算或请求转换。

## F0 基线

- `route_trace` 在进程内保留最近 60 条路由，原先按最后一次 attempt 更新状态；`relay_usage` 只为已识别协议的 provider 请求写 JSONL，最近列表最多 200 条，旧明细在归档压缩后仅保留按 provider/model 的汇总。Codex official 和路由前失败没有 provider 用量明细。
- 既有故障转移只针对明确的新请求，在 HTTP 429 或连接建立前失败时尝试另一家；5xx、超时和已经开始响应的流不触发重试。已有会话绑定并非用户显式 pin。
- provider 推理请求使用 `model_credential().api_key` 构造最终认证 header；余额用的 `access_token` 不参与推理。Codex official 可取请求内凭据或本地 `auth.json`，账号 ID 只在本次最终出站 header 确实存在时记录。
- Claude Code、OpenCode、Codex 的既有配置注入和恢复路径不接受本次诊断字段作为输入。探测弹窗把所选 provider 模型的原始 `id` 传给 `test_newapi_provider`，`provider_probe` 用它构造请求体，未以 models.dev canonical ID 替换。

内置 models.dev 快照刷新后，revision 为 `0be6fdc12df0bb20ec424d59083b97f6792f9d1abc62a0d93f064ce87604dd4a`：225 个 provider、8,359 条模型；5,288 条包含 canonical 关联，其中 4,049 条目标在快照内，1,239 条缺失；已收录的目标中 1,644 条的 `limit` 与来源不同，89 条指向自身。`interleaved` 有 103 条布尔形态和 1,114 条 `{field}` 形态。6,155 条显式声明 `reasoning_options`，2,204 条未声明。这些数字是本次快照事实，不作为长期 schema 保证。

## Schema 与生命周期

- 逐 attempt 创建 ID，并与同一路由的 request ID 一起进入 trace 和已有 provider usage 记录。账本明细 schema 从 1 升到 2，新增 `credential` 和 `modelObservation`；旧记录通过默认值读取，界面显示未知或未记录，不从当前配置回填。
- 原始上游字节在协议转换前进入同一个有界观察器。该观察器只保存首个受支持的模型声明及一个冲突摘要；正常 EOF、上游错误和对象提前销毁均幂等定稿，然后把同一结果交给 trace 与账本。非流 JSON 总上限和 SSE 未完成事件上限均为 256 KiB。
- `same` 仅表示上游声明与出站模型 ID 精确相同；`declared_different` 仅表示声明不同。缺失、HTTP 失败、冲突、解析错误、超限、提前断流均为带原因的 `unknown`。未登记任何会删改日期、地区或版本后缀的别名规则。
- 凭据版本指纹用保存在应用数据目录的受保护随机密钥执行 HMAC-SHA256，输入为最终出站认证 header 及 provider/endpoint 范围。密钥无法读取时显示 `not_recorded`；Codex 账号 ID 另行做 HMAC，不把 OAuth token 版本解释为账号变化。原始凭据不进入 trace 或账本。
- 原有价格快照继续在请求开始时固定，served model 仅用于诊断。汇总归档不新增按凭据或 served model 的虚假分组。

## 能力预览契约

canonical 关联只从 resolver 已确定的目录来源记录读取，目标存在性按当前快照验证；来源记录的限制、reasoning 和价格不会继承目标条目。匹配涉及多条来源时逐条展示关联。`reasoning_options` 的声明状态和 `interleaved` 两种形态只供展示。旧快照无法恢复声明状态时标为未知。

覆盖清除 dry-run 在同一份 store、官方目录和 models.dev revision 上分别调用实际能力预览 resolver。应用时比较不含原文的配置版本令牌、目录 revision 和官方目录版本；发生变化返回新的预览，要求重新检查。预览本身不保存、探测、注入或刷新目录。

## F4 验收记录

- `pnpm test:frontend` 全部通过；夹具覆盖请求诊断、目录关联与 typed reasoning/interleaved 展示、旧账本缺失字段和窄屏布局。
- `pnpm build` 通过；`cargo test --manifest-path src-tauri/Cargo.toml` 为 497 通过、4 个默认忽略，普通集成测试 9 通过。真实本机 Codex 的 7 个本地合成凭据测试也通过；未运行需要真实渠道与计费凭据的 live 测试。
- 本地 HTTP fixture 核对了出站 model 与请求体、客户端收到的原始 JSON、HTTP 状态、429 故障切换与 5xx/不完整流不重试；同一 request 的 attempt ID、模型观察和凭据版本在 trace 与账本中一一对应。真实 Codex 夹具还覆盖 Chat/Anthropic 转换流、Router/Native 配置恢复与认证 header 隔离。
- 凭据原文与提示词不会进入 trace/账本；旧明细以未知字段读取，归档保持聚合。观察器对 JSON/SSE、跨 chunk、多行事件、冲突、超限、错误和提前结束有定向测试。

真实 Codex 夹具中三处旧预期已按此前主计划的实际行为更新：Haiku 4.5 目录 context 为 200,000，未确定的 reasoning 不再按模型家族补档，显式只声明 `low` 的模型只显示 `low`。本变更没有改客户端注入或路由策略。当前验收使用确定性行为断言和本地端到端夹具，未建立对旧二进制逐字节比较所有 stream 分帧的独立双版本 harness；现有转换和流式回归仍覆盖主要协议路径。
