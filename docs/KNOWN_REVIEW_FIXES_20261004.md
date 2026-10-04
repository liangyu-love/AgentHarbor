# 外部审查问题修复与验证

日期：2026-10-04。基线：本地 `main` 的 `aeec15c`。本轮针对外部 23 项审查清单修复已确认的行为缺陷，保留需要协议原生支持或进一步实测的限制。

## 已实施

| 审查项 | 当前行为 | 主要实现 |
| --- | --- | --- |
| 7、8：工具定义和历史静默丢失 | Hosted 搜索、custom/freeform 工具、不能重建的历史项明确拒绝；不再丢工具或将 freeform 伪装为空 schema 的 function。普通 function 的 `strict` 和强制工具选择保留。 | `protocol/chat.rs` |
| 真实客户端补充：namespace 工具 | Codex 0.159.2 默认会发送 namespace function。转译时按 namespace/name 生成长度受限的临时函数名，返回最终工具项和缓存续轮时恢复原身份；保留 schema、strict、组描述和原参数，拒绝身份冲突及不支持的子工具。 | `protocol/tool_namespaces.rs`、`protocol/mod.rs`、`router_proxy.rs` |
| 9：结构化输出 | Responses `text.format.json_schema` 映射到 Chat `response_format.json_schema` 或 Anthropic `output_config.format`，后者与推理 effort 合并；Chat 支持 `json_object`，Anthropic 对无法等价表示的模式明确拒绝。 | `protocol/chat.rs` |
| 10：确定的格式失败被反复重试 | 无效流帧、无效工具参数、缺失/不支持的结束原因使用 Codex 可识别的 `invalid_prompt` 不可重试类；原诊断码保存在 `error.reason` 和诊断记录中。网络读取失败及连接截断保持可重试，上游真实错误码保持原值。 | `protocol/responses.rs`、`protocol/sse.rs` |
| 4 的展示问题、5：Native 全局限制 | 有供应商窗口覆盖时取所选模型已解析窗口的保守最小值；`clientDefault` 的 applied 为未判定，不再宣称已写入压缩阈值。 | `codex_native.rs` |
| 12：手工请求头被删除 | 同一端点重新应用时保留 `http_headers`、`env_http_headers`；端点改变或类型不合法时明确拒绝并保留原文件。手工 `experimental_bearer_token`、`auth`、`aws` 认证覆盖也明确拒绝，避免整表重写时悄悄删除。配置更新锁内重新读取头字段。 | `codex_native.rs`、中英文文案 |
| 13：非 `/v1` 原生端点错误 | Native 和 Router 共用端点规则，覆盖 Novita、z.ai、BigModel、DeepSeek 等根路径。 | `codex_native.rs` |
| 14：Native journal 丢失 | 有专属凭据/目录证据和唯一匹配备份时恢复原基线，可重新应用、移除或切回 Router；缺失/歧义基线继续拒绝，用户漂移字段保留。 | `router_config_snapshot.rs`、`codex_native.rs` |
| 16：`.env` 无法诊断 | 保留文件类型、大小、语法和所有权保护，新增文件类型、超限、读取、语法、并发修改及写入失败的独立提示；不输出 dotenv 内容或凭据。 | `codex_native.rs`、中英文文案 |
| 17：清理顺序和孤儿目录 | 先撤回未引用的专属密钥块，再清理目录；扫描保留命名空间下的规范 UUID 目录文件，可再次回收孤儿目录，跳过目录/符号链接，保留当前目录及外部凭据引用。 | `codex_native.rs` |
| 18 的元数据刷新问题 | 同端点、同 API key 的目录/模型元数据更新复用现有 env_key；目录单独产生新一代并在提交后回收旧目录，失败时保留旧配置。 | `codex_native.rs` |
| 19：本地 token 的端点权限过宽 | 加入 method/path 白名单，覆盖模型读取、推理及已支持的 Responses 子资源；不再转发 files、管理或计费端点。既有路径穿越防护继续生效。 | `router_proxy.rs` |
| 20：failover 无过期/开关闭联动 | 绑定空闲 24 小时过期，活跃访问续期；关闭后，新无状态请求释放旧绑定并按当前路由运行，仍绑定的历史/工具续轮返回明确 409。开启时未知或过期的续轮也返回 409。 | `router_proxy/failover.rs`、`router_proxy.rs` |
| 21：尝试数被显示为逻辑请求数 | 保留每供应商实际上游尝试的费用账本和 attempt_id，面板文案明确为“上游尝试”；失败尝试的零 token 记录不冒充额外 token/费用。 | 中英文 `usage.relayRequests` |

## 验证

- 完整 Rust：**620 通过，10 个 opt-in 测试跳过**；`local_workflows`：**9 通过**。
- 已安装的 **Codex CLI 0.159.2** 隔离测试：**9 通过**。覆盖官方账户保留、Router 鉴权替换、Native `.env`/无官方回退、线程续接与渠道切换、推理菜单、Chat/Anthropic 流式、结构化输出字段，以及确定的格式失败不重试。
- Namespace HTTP 回归覆盖 Chat/Anthropic × 流式/非流式及缓存续轮，另有非法子工具和普通函数身份冲突断言。
- `pnpm build`（TypeScript + Vite）及前端完整回归通过。
- `git diff --check` 通过。只格式化编辑的 Rust 文件；仓库既有的其他 rustfmt 差异不在本轮处理范围。
- 一次完整回归曾触发既有 Windows 注册表 raw-string 测试的偶发终止符差异；单独复跑与后续完整回归均通过，未修改该测试或生产注册表实现。

真实 CLI 测试使用临时 Codex home、合成凭据、localhost mock 上游，并将外网流量导向本地拒绝代理。没有运行需要真实渠道/计费凭据的 `live_codex_switches_configured_relays`。Chat/Anthropic 转译测试显式设置 `web_search = 'disabled'`；这证明函数工具命名空间和转译链路兼容，不证明 hosted 搜索可跨协议执行。结构化输出测试验证字段抵达上游，不证明 mock 模型实际遵循 schema。

## 保留限制与后续验证

- **1、2**：远端 Responses 压缩和已有 compaction/加密推理历史仍需要兼容的原生 Responses 上游。跨协议时继续明确拒绝；不剥离加密历史来伪装为无损续接。
- **3**：公开的 Codex 0.159.2 源码确认存在每模型 `auto_compact_token_limit`。本轮未运行低阈值自动压缩触发实验，不能把“写入正确”当作客户端触发点已验证。
- **4 的共享 unknown veto**：Native 全局压缩计划继续对未知模型保守处理，避免把不确定的窗口当作可靠上限。
- **6**：托管路由的每模型目录继续优先于用户原有全局限制/自有目录；恢复通过所有权快照实现。本轮没有改变该产品规则。
- **11、15、22**：没有把尚未实测的普通 MCP 名称约束、旧认证偏好键兼容问题或缺失的可选 capability 字段直接视为已解决；没有凭猜测补能力声明。
- **18 的密钥轮换**：API key 或端点改变仍会轮换凭据；已启动的 Codex 可能需要重启以重新读取 `.env`。本轮不提供客户端自动重启。
- **20 的持久性**：failover 绑定仍是有界内存状态，不跨应用重启保存。绑定过期后不承诺恢复旧渠道；应在开启 failover 的状态下开始新会话。
- **21**：账本仍统计上游尝试，不是新增了按 request_id 去重的逻辑请求指标。
- **23**：未新增通用供应商 header/query 配置；Native 仍只支持 Responses。可保留的 Native 手工请求头不等于通用认证适配。
- 本轮没有建立计划 F4 的新旧版本 stream 分帧逐字节双版本 harness，也未做真实付费上游测试。

Codex 错误分类依据：`openai/codex` 的 `rust-v0.159.2` 标签下 `codex-rs/codex-api/src/sse/responses_error.rs`，并通过真实 CLI 的不可重试断言验证行为。
