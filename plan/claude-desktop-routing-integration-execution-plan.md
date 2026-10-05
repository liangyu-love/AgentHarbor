# Claude Desktop 路由模式：参考核对与实施计划

日期：2026-10-05。状态：**设计与源码核对完成，路由模式尚未实现**。

本文接续 [直连接入计划](claude-desktop-direct-integration-execution-plan.md) 第 10 节。第 1–2 节描述 cc-switch 当前源码；第 3–10 节是 AgentHarbor 的实施规格，不是现有功能清单。直连的实现、四文件事务与验收缺口仍以原计划和 [验收记录](../docs/claude-desktop-acceptance-record.md) 为准。

参考根目录：`D:/workspace/temp/cc-switch`，提交 `1ee2fdc3a791f1e73476c631c7ab7ce8fac0638f`。本项目设计基线：`70da81f`。本轮只读研究源码，没有执行真实 Desktop 接管、进程控制、登录或计费请求。

## 1. cc-switch 的路由模式究竟做什么

直连和路由都写同一种 Desktop 3P profile。区别在于 profile 指向的地址、凭据和模型名：

| 内容 | cc-switch 直连 | cc-switch 模型映射模式 |
| --- | --- | --- |
| 地址 | 供应商根地址 | `http://<本地监听地址>:<端口>/claude-desktop` |
| profile 的 Key | 上游推理 Key | 本机持久 gateway token |
| profile 的模型 | Desktop 接受的真实 Claude ID | Desktop 接受的角色 alias |
| 真实目标模型 | 请求原模型 | 供应商配置内的 `upstream_model` |
| 格式转换 | 上游必须兼容 Messages | 共用 Claude adapter / proxy 转换链 |
| 运行依赖 | Desktop 自己请求上游 | cc-switch 代理进程必须运行 |

请求路径为：

```text
Desktop profile 中的安全 alias + 本地 token
  → GET  /claude-desktop/v1/models
  → POST /claude-desktop/v1/messages
  → Desktop token 校验
  → 选择 claude-desktop namespace 的供应商
  → alias 映射成真实模型
  → 共享鉴权替换、协议转换、转发、SSE 和用量记录
  → 上游真实地址 + 上游真实凭据
```

### 1.1 profile 和两个 HTTP 端点

- `proxy/server.rs:299` 注册上述两个端点；当前没有 Desktop `count_tokens` 入口。
- `proxy/handlers.rs:134` 使用 `AppType::ClaudeDesktop`，把入站路径的 `/claude-desktop` 前缀去掉后交给共享转发器。它不从 Claude Code 当前供应商取路由。
- `claude_desktop_config.rs:989` 构造网关 profile；模型列表写安全 alias，可附 `labelOverride` 和 `supports1m`。真实上游模型保留在供应商的 routes 中。
- `claude_desktop_config.rs:650` 的 `/models` 数据项包含 `type: "model"`、`id`、`created_at`、可选 `supports1m`，外层有 `has_more: false`、`first_id`、`last_id`。此响应不带 `labelOverride`，菜单展示名来自 profile。
- `claude_desktop_config.rs:920` 把 wildcard IPv4 转为 `127.0.0.1`、wildcard IPv6 转为 `::1`，IPv6 URL 使用方括号。profile 生成会拒绝 `port=0`；`services/proxy.rs:974` 在服务启动后把实际绑定端口保存到 DB。因此“支持动态绑定”和“可以把 0 写进 profile”是两件事。

### 1.2 本地鉴权和上游鉴权

`claude_desktop_config.rs:278` 读取 DB setting `claude_desktop_gateway_token`，非空则复用，否则生成 `ccs-<UUID simple>`。此基线未发现轮换、过期或撤销流程。

`proxy/handlers.rs:271` 只从 Authorization 读取 Bearer（接受 `Bearer ` / `bearer `），与本地 token 比较；错误或缺失返回 401。`/models` 和 `/messages` 都校验。当前请求校验也调用此 token getter，所以首次请求可能触发生成；本项目不照搬这一副作用。

`proxy/forwarder.rs:2082` 在出站时用供应商 adapter 生成的鉴权替换 authorization / x-api-key / x-goog-api-key。本地 token 不应作为上游 Key，也不是余额 token 或账号 session。

### 1.3 模型映射的六步算法

源码：`claude_desktop_config.rs:686`，未知模型错误映射到 HTTP 400（`proxy/error.rs:153`）。

1. 要求 `body.model` 是非空字符串，trim，再剥离末尾大小写不敏感的 `[1m]`。
2. 精确匹配解析后的安全 `route_id`。
3. 兼容 `claude-opus-5` 与 `claude-opus-4-8` 的双向灰度别名。
4. 查旧版原始 routes 的 key，兼容以前保存的非安全 alias。
5. 请求本身为安全 Claude ID 时，按 Opus / Haiku / Fable / Sonnet 角色寻找同档；**仅 Fable 缺档时允许回落 Opus**。
6. 仍无目标则返回 400；命中后把 `body.model` 换成 `upstream_model`，特定供应商兼容处理见第 1.4 节，随后进入共享转发和格式转换。

这里没有“未知模型随便走默认模型”，也没有“运行时把 Haiku 自动降级 Sonnet”。UI 中另一种回填容易造成混淆：

- `ClaudeDesktopProviderForm.tsx:192,695` 固定 Sonnet / Opus / Fable / Haiku 四档，至少填一档，空档继承第一条已填档，Sonnet 优先；展示名和 1M 选择也继承。它发生在**保存配置时**。
- `claude_desktop_config.rs:560` 会修复旧配置的不安全 alias，使用安全角色名避碰，必要时加 `-r2`；空 route/model 跳过，全空报错。
- 从 Claude Code 导入时，`commands/provider.rs:430` 使用 `ANTHROPIC_DEFAULT_*_MODEL`，全空才使用 `ANTHROPIC_MODEL`。普通供应商导入默认 1M=true、OAuth=false（`:346`），属于参考项目产品策略，不是模型能力证明。

此提交中的默认 ID 已是 `claude-sonnet-5`、`claude-opus-5`、`claude-haiku-4-5`、`claude-fable-5`（`claude_desktop_config.rs:35`）。旧调研文档的 4.7/4.8 常量不能作为当前值。常量本身也不能证明所有 Desktop 版本均接受它们，实际客户端仍需单独验收。

### 1.4 协议、流式、工具和计费身份

参考实现允许 anthropic / openai_chat / openai_responses / gemini_native，托管 OAuth 可以没有静态 Key（`claude_desktop_config.rs:397`）。Claude Desktop 和 Claude Code 共用 ClaudeAdapter（`proxy/providers/mod.rs:261`）；Desktop 映射在格式转换之前执行，失败不会进入普通 Claude 默认模型映射（`proxy/forwarder.rs:1245,1618`）。

原生 Messages 和转换后的非流/SSE 使用共享 response pipeline（`proxy/handlers.rs:237,385`）。工具能力来自共享转换链，不等于已经完成真实 Desktop 工具验收。Codex OAuth 的强制 SSE 聚合、Gemini 的 tool schema hints、Responses 的托管 web search、MiMo 的历史 thinking 修补都是具体路径，不能作为本项目已支持这些供应商的证据。

用量保留三份模型身份：客户端 `request_model`、改写后的 `outbound_model`、上游回显 `model`，原始 `app_type` 为 `claude-desktop`（`proxy/handlers.rs:314,458`；`proxy/response_processor.rs:479`）。供应商价格覆盖按 Desktop namespace 查询；全局计费默认继承 Claude（`proxy/usage/logger.rs:349`）。统计 UI 可以把 Desktop 折入 Claude 展示，但原日志未丢来源（`services/usage_stats.rs:242`）。

## 2. 参考实现的启停和恢复限制

这是本次补充的关键结论：**停止 cc-switch 的本地代理不会替 Desktop 撤回网关 profile。**

| 源码行为 | 实际含义 |
| --- | --- |
| `ClaudeDesktopRouteToggle.tsx:16` 启停共享服务 | 不是 Desktop 独立 takeover 开关 |
| `app_config.rs:442` 的 `supports_local_proxy()` 不包含 Desktop | Desktop 供应商切换走 `switch_normal`（`services/provider/mod.rs:5735`） |
| `commands/proxy.rs:28` 停止服务 | Desktop profile 保留，本地地址可能暂时不可用 |
| `services/proxy.rs:2279` 恢复列表只有 Claude/Codex/Gemini/GrokBuild | 全局 stop_with_restore / 程序退出不负责恢复 Desktop |
| `useProviderActions.ts:195,341` 提示重启 | 没有自动停止/重启 Desktop 的实现 |
| `claude_desktop_config.rs:1013` 切换官方 | 写两处 1p、删除工具 profile/条目及指定 enterprise keys；不是精确恢复接管前四文件 |
| `claude_desktop_config.rs:955` 的写入快照 | 仅当次内存回滚，没有本项目已有的持久 journal / 首次基线 / 一次性票据 |

因此 Desktop 路由态下退出参考工具后，需要重新启网关，或显式切到直连/官方。不能把“代理停止成功”写成“Desktop 已恢复”。

同样不能把它概括为“Desktop 专属多供应商自动 failover 已完成”：`database/schema.rs:129` 的 proxy_config CHECK 没有 Desktop，`database/dao/proxy.rs:252` 无记录时返回默认配置，auto_failover 默认 false；`/models` 使用所选首个供应商。共享 provider router 可泛化选择，不等于完整的 Desktop 多供应商策略。

## 3. 本项目首版范围和兼容决策

采用**共享 listener、Desktop 独立路由表、受限鉴权、既有转发/诊断/账本**。继续从已有中转卡片选模型，不再填地址/Key，不复制供应商数据库，也不把 Desktop 塞进现有单文件 `TargetId` 注入管理器。

| 项目 | 首版决定 |
| --- | --- |
| 客户端入口 | 两个 `/claude-desktop/v1/*` 专属端点 |
| 本地监听 | 复用现有 localhost listener；固定非零端口 |
| 角色目标 | 各角色可选不同的已保存 Provider/model，精确映射 |
| 默认角色 | 默认提供 Sonnet/Opus/Haiku 的已验证安全 alias；Fable 受版本策略限制 |
| 空档 | 明确未配置；不复制 cc-switch 自动回填，不隐藏增加请求目标 |
| 缺失的子任务角色 | 明确报缺档；UI 可显式把该档选成同一模型，不运行时降级 |
| 真实模型限制 | 不套用直连 Claude 名白名单；支持既有 Messages/Chat/Responses 目标 |
| Provider.enabled | 路由按现有资格要求 true；用户显式启用，提交不偷偷改开关 |
| Codex 总开关/默认卡片 | 不改 router.enabled、active_provider_id 或其他客户端选择 |
| failover | Desktop 首版关闭，不继承全局 Codex failover/亲和 |
| Gemini | 经已有 OpenAI 兼容协议可选；不声称 Gemini Native 已实现 |
| OAuth / Cowork / 托管工具 | 不以 cc-switch 的能力代替本项目实现与验收 |

角色 alias 只用于满足 Desktop 的菜单协议，**不表示实际上游一定是 Claude**。菜单展示名由所选卡片与真实模型生成，保留用户自定义 label。示例“Sonnet · 中转 A / deepseek-chat”应能直接说明实际目标。

默认 alias 应集中在已有 Desktop 版本策略旁，由真机菜单/请求验收锁定；不能在请求处理中猜未来型号。未知客户端 alias 返回 400，不匹配任意安全前缀，不支持历史 alias，直到真实证据要求添加精确兼容表。

## 4. 数据、解析和 profile 契约

### 4.1 只保存引用，单次请求冻结解析结果

建议在 Desktop 状态中新增可选 router 配置，保存：

```text
DesktopRouteChoice {
  role, roleAlias,
  providerId, modelId,
  labelOverride?, oneMMode
}
DesktopRouterConfig {
  routes[], routeRevision,
  gatewayTokenVersion, boundPort
}
```

这是建议的内部类型，不是现成 DTO。`AppliedMode` 增加 Router，`DesktopAction` 增加 ApplyRouter；旧状态缺省仍为 Direct。既有单 provider 字段只服务直连兼容，不再代表全部路由目标。路由状态/DTO 提供 `routeViews`，不得把上游密钥、gateway token、四文件原文带到前端。

每次解析从 `router::load_store()` 获取当前 Provider，以 `model_credential()` 获取真实 Key；地址、协议、能力、价格继续使用现有 resolver。形成一次不可变 resolved route snapshot：

- Provider 存在、enabled、具有推理凭据；余额 access_token 不代替推理 Key。
- 目录模型必须存在且 included=true；可沿用当前明确例外：目录没有该项、且恰为 Provider 已保存的手填默认模型。
- 不能单靠 `Provider::supports_model()`：现有方法找不到模型也可能返回 true，并且不负责 included 判定。
- 协议必须由该模型的有效配置确定；只有客户端 alias 使用 Desktop 安全名校验。
- profile、`GET models`、提交资格和 `POST messages` 使用同一解析规则。运行时卡片被禁用/删除或模型被排除后，该路由不可用，不选择其他供应商。
- 卡片 Key 轮换后，下一个网关请求读取新 Key；不要求把 Key 再写入 Desktop。URL/协议也在请求开始解析，实际模型与计费快照在该请求内冻结。影响菜单 label/1M 的变更使预览失效、配置标 stale，更新后重启 Desktop。
- 余额、价格刷新及无关卡片变更不应使 Desktop 文件预览无故失效；一次请求的价格仍按实际目标当时快照落账。

实现时区分两种投影：**文件预览投影**只绑定 mode、角色引用、菜单 label/1M、提交相关目标资格/有效协议、token version、lease 和四文件输入；**请求执行快照**在请求开始解析并冻结真实 Key、出站地址、能力与价格。票据不能哈希整个请求执行快照，尤其不能把价格、余额或无关 Provider 塞进文件预览 revision。网关 Key 更新只影响下一次出站；若更新使目标资格失效，则提交前重验必须拒绝。

### 4.2 profile 形态

仍复用本产品 UUID 和四文件事务；网关示意（43210 仅示例，不是硬编码默认端口）：

```json
{
  "disableDeploymentModeChooser": true,
  "inferenceProvider": "gateway",
  "inferenceGatewayAuthScheme": "bearer",
  "inferenceGatewayBaseUrl": "http://127.0.0.1:43210/claude-desktop",
  "inferenceGatewayApiKey": "<本产品独立本地 token>",
  "inferenceModels": [
    {"name": "claude-sonnet-4-6", "labelOverride": "中转 A · deepseek-chat"},
    {"name": "claude-opus-4-7", "labelOverride": "中转 B · claude-opus-4-7"}
  ]
}
```

外部 profile 不写真实非 Claude 模型 ID、Provider 复合 slug、上游 Key 或余额凭据。示例 alias 最终必须通过目标 Desktop 版本验收。

`GET models` 从当前有效 routes 返回合法 alias，一项一个 ID，附已有协议所需时间和分页字段。名称从 profile label 来，不把目录中所有模型直接暴露给 Desktop；无有效路由时返回明确不可用状态，不伪造默认项。

### 4.3 1M 与请求改写

- 继续使用单击“额外 1M 选项”复选框，新选择默认关闭。`supports1m=true` 只给 Desktop 生成菜单变体，不再增加一条模型，也不把 `[1m]` 写入 route/modelId。
- 不只看 context ≥ 1M：当前共享管线拒绝 1M 请求转 Chat/Responses（`router_proxy.rs:618`）。首版仅对已具备能力的原生 Messages 目标开放；跨协议 1M 后续单独验收。
- 客户端请求末尾 `[1m]` 只在 alias 层剥离；保存的真实 modelId 保持原样。不凭 alias 创建另一种上游型号。
- 额外 1M 关闭或能力不满足时，旧缓存的 1M 请求明确拒绝；不能静默降成普通请求。
- 映射发生在协议转换前；客户端原始 alias 交给诊断，转换器收到真实 Provider/model。映射不改消息、工具或用户正文。

## 5. 鉴权、监听 lease 和会话隔离

### 5.1 按精确路径校验独立 token

先识别 namespace，再按各自 scope 鉴权。Desktop token 只授权 `GET models` 与 `POST messages`；现有 router token 的授权规则不变，不能用“任一 token 都可”的判断开放 `/v1`、`/p/*` 或 `/codex/v1`。

token 在显式网关配置操作中创建/复用，内部通过 `secret_file` 保存，DTO 只给存在状态/version。请求校验只读，不生成、不轮换；网关管理态未启用时不服务有效旧 token。未知方法/路径返回明确错误，不任意转发。

轮换必须是可预览的配置操作：Desktop 停止后提交新 token/profile；成功后旧 token 失效，提交点前失败恢复旧 token/state。候选 token 按 version 分开受保护保存，持久 pending 记录 before/next 的 token 引用与状态；State 持久提交是唯一提交点。提交点后的发布/清理失败保留已提交 nextState 待恢复，不反向启用旧 token，详见第 6.1 节。票据绑定 token version，不允许轮换后旧票据继续写入。上游收到的鉴权始终来自 resolved Provider。

### 5.2 listener 与票据

当前 `ensure_running()` 返回实际 `u16` 端口，`ensure_available()` 丢弃返回值；现有 `ProxyHandle` 没有 lease。这些必须新增，不能假定已有。

```text
ListenerLease { boundPort, generation, processEpoch }
```

首版固定非零端口，不扩展动态端口配置。成功绑定后才生成本地 URL，票据绑定 lease、token version、第 4.1 节的文件预览投影和四文件基线；端口冲突零外部写入。预览不要求退出 Desktop，也不调用真实上游，只允许内部票据/凭据准备和临时监听引用。

提交在共享 mutation guard 与 Desktop writer lock 下重验票据。监听停止/重绑、后端重启、token/路由输入变化均使旧票据失效。临时预览引用随票据失效/过期释放，只触发 reconcile，不直接停止其他使用者的 listener。

活跃的 Desktop 网关配置固定该监听端口；其他客户端改端口时必须明确拒绝或进入所有相关配置协调更新，首版选择拒绝。不能静默重绑后留下 Desktop 指向旧地址。

### 5.3 会话、历史和 failover

复用传输与转换，不直接复用全部路由选择。首版从 alias 指定唯一目标，跳过 Codex 的全局 failover 与已有 binding。

当前 failover key/共享 seen 状态和 message_history scope 没有完整客户端 namespace。新入口必须把 `clientSource=claudeDesktop`、安装/配置身份、alias、routeRevision 加入缓存 scope，再叠加既有 Provider/凭据/协议/会话身份。不同角色即使映射到同一个真实模型，也不能串用工具/思考历史。

请求开始冻结 resolved route；更新配置不改正在转发请求的快照。stream 已开始或工具历史已建立后，不自动切模型/供应商。后续 failover 另设同模型候选、稳定会话标识和独立策略，不继承全局开关即算“支持”。

## 6. 事务、模式切换、启停和恢复

### 6.1 首次基线与单次回滚点

复用 `editions_before()`、pending journal、读回校验与备份。正常切换始终保持首次接管基线：

```text
未接管 → 直连 → 路由 → 直连 → 恢复接管前配置
                                  → 首次接管前的四文件/受管键
```

`Pending.before` 只撤回本次失败提交；不代替首次恢复点。仅显式 rebaseline 移动基线。Reapply 按保存的 mode 重建计划，不能继续一律 `direct_plan()`；`status::read_mode()` 不能把本产品 UUID 一律解释成 direct。

提交顺序：准备并验证 listener/候选token/路线 → 确认 Desktop 已停止 → 写持久 pending（before/nextState及token version引用）→ 四文件写入与回读 → 内部 State 持久提交 → 原子发布新的 routes/token/generation 整代快照 → 完成 pending 清理。旧 token 版本在事务完成前保留；候选版本未提交前不参与请求授权。

- **提交点前失败**：恢复本次文件、State与原 token，未提交候选可在确认无引用后清理；回滚不完整保留 pending，不开放新一代端点。
- **提交点后失败**：发布或 pending 清理失败不得撤回已提交 generation。保留 nextState 和候选 token 版本，报告恢复/网关未就绪，通过恢复流程完成发布与清理；不能把该操作当普通未提交失败重新消费票据。
- **启动恢复**：先按持久 State 的 transaction/generation 判 pending 已提交或未提交，核对四文件与 token 引用，完成相应恢复，再开放 Desktop 端点。不能让新文件搭配旧 token/routes 服务。
- **并发请求**：整代快照一次替换，已授权的在途请求保留原快照；不能逐字段更新 token 和 routes。

路由转直连/官方/恢复后，撤销 Desktop 的 listener 使用需求；是否停止共享服务由 reconcile 决定，不碰其他客户端。

### 6.2 共享服务需求

```text
needsServer =
  Codex 路由需求
  || Claude Code / OpenCode 等现有客户端路由需求
  || Desktop 网关已提交且仍受管
  || 未过期的网关预览监听引用
```

关闭 Codex 路由、切 Codex Native 或恢复另一个客户端不会停止 Desktop。Desktop 配置直连不增加服务需求。路由故障仍保留其需求和管理证据，不能因请求暂时不可用而自动释放管理。

### 6.3 释放与完整退出

当前直连 release 只留下 tombstone，适合直连地址；**路由态不能直接照搬**。首版要求先切直连、官方或恢复，再释放；不提供保留 localhost profile 却停止网关的普通 release。

关闭主窗口继续沿既有托盘行为保持网关；托盘退出、更新重启与显式完整退出必须走同一前置协调流程，覆盖 `commands/tray_menu.rs:63` 的 app.exit 和 `commands/system.rs:275` 的更新 restart/exit，再衔接 ExitRequested/最终 Exit。不能只在最终 `RunEvent::Exit` 临时塞一次四文件恢复。macOS 更新重启的延迟启动 helper 必须在退出获准后创建；用户保持后台时不能已经安排第二次启动。

| 场景 | 首版处理 |
| --- | --- |
| Desktop 已完全停止 | 按事务恢复首次基线，成功后释放 Desktop 服务需求并退出 |
| Desktop 仍运行，用户选择保持后台 | 保持网关与当前配置 |
| Desktop 仍运行，用户明确选择停止并恢复退出 | 仅停止已确认安装，待退出写回完成后恢复；失败保持网关和可重试状态 |
| 未能确认安装/停止权限 | 不自动杀进程或强行写盘，保留后台运行和手动恢复入口 |
| 系统关机/强制结束/崩溃 | 无法保证交互式停止恢复；持久管理态/pending 保留，不声称已经撤回 |

因此完整退出是网关新增的实际产品决策点，不增加常驻说明墙。正常窗口关闭不弹警告；只有退出会让服务中断时提供对应动作。

下次启动先恢复持久 token/route state，并尝试原固定端口。绑定失败且 Desktop 正运行时，报告网关不可用，等待明确停止/恢复操作；Desktop 已停止时才允许事务撤回。不能套用现有 CLI 端口占用时的直接写盘恢复。不同进程 epoch 不复活旧票据；持久路由状态不依赖旧 lease 存活。

## 7. 诊断、账本与输出消费

| 身份 | 来源 | 用途 |
| --- | --- | --- |
| clientSource / requestedModel | Desktop 入口及原 alias（含原始 1M 意图） | 入口筛选、用户选择与路径诊断 |
| resolvedModel / routeRevision | Desktop 路由解析快照 | 展示 alias 对应的卡片和真实型号 |
| attempt outbound model | 最终真实出站 body | 模型对账、真实请求证据 |
| 上游自报 model | 原始非流/SSE 响应 | served-model observation；缺失仍 unknown |
| providerId + model + pricingSnapshot | 真实出站目标 | 费用、缓存价格、用户价格覆盖与汇总 |

兼容新增可选 `clientSource/requestedModel/resolvedModel/routeRevision` 到诊断/账本视图与记录；旧 JSONL 没字段仍能读。既有 `RelayUsageRecord.model` 保持真实出站计价身份，不能换成 Opus 菜单 alias，也不按上游回显重新定价。

UI 继续消费已有诊断和用量页：可筛 Desktop 来源，同行看 alias → 实际 Provider/model。汇总仍按真实模型，历史 compaction 保留原费用和覆盖状态；首版不承诺按来源拆分旧归档数据，来源缺失显示未知。

响应默认使用原生透传/既有协议转换，不额外改回 alias。若真机证据要求客户端响应 alias，只在输出给 Desktop 的最后一层映射，保留原始响应观察与计费快照。SSE 转换失败、上游失败、客户端取消进入已有失败诊断；已产生用量仍保留，不能报成功或重复落账。

真实 Desktop 若调用 `count_tokens`，先记录实际请求与需求，再增加精确受限端点；原生 Messages 可考虑透传，跨协议未实现精确计数时明确 501，不编造结果。此项不是现在两个入口已支持的功能。

## 8. 文件职责与 UI

| 文件/模块 | 计划改动 |
| --- | --- |
| `claude_desktop/routes.rs`（新增） | 引用解析、严格候选、alias/1M、resolved views；纯逻辑可单测 |
| `claude_desktop/gateway.rs`（新增） | 独立受保护 token、受限入口数据/授权、listener 投影；不复制转发器 |
| `claude_desktop/types.rs`、`transaction.rs` | ApplyRouter/Router 状态、旧状态兼容、首次基线与模式分派 |
| `claude_desktop/mod.rs`、`tickets.rs`、`status.rs` | router plan、lease 票据、mode 状态、重应用/释放/恢复 |
| `router_proxy.rs` | 精确 Desktop namespace 分派、scope auth、唯一目标入口、实际模型和协议/SSE复用 |
| `routing_runtime.rs` | 返回 lease、Desktop/预览服务需求、端口 pin、退出协调 |
| `protocol/message_history.rs`、`router_proxy/failover.rs` | 增加显式 scope；原客户端默认行为保持兼容 |
| `route_trace.rs`、`request_diagnostics.rs`、`relay_usage.rs` | 可选入口/alias身份，真实目标价格与旧记录兼容 |
| `commands/claude_desktop.rs`、`lib.rs`、`commands/tray_menu.rs`、`commands/system.rs` | 路由 API、启动恢复；托盘退出/更新重启统一前置协调，保持既有窗口关闭隐藏行为 |
| Desktop card/dialog、`lib/claude-desktop.ts`、`types/claude-desktop.ts` | 直连/路由选择、角色映射、状态/动作与契约 |

UI 在现有 Desktop 卡片中加模式选择，不新建供应商页面。路由下按角色选已有卡片和模型，展示实际协议/目标，保留一个可直接点击的额外 1M 框；空角色保持可见、明确未配置。

预览继续只在下方列实际四文件变更，Key 脱敏；不再叠一份重复“模型预览”。预览不关闭 Desktop；写入前仍必须完全退出。直连和路由共用重启按钮的既有成功判定，不制造永久“需要重启”。

卡片只显示当前模式、健康/待重启状态及可用操作；运行依赖、错误处理等信息只在相关状态出现，不把文档的技术检查逐条搬到 UI。

## 9. 实施批次与停止条件

| 批次 | 交付内容 | 验收门槛 |
| --- | --- | --- |
| R0（本轮） | 源码事实、设计、测试矩阵及兼容决策 | 文档区分已核对/拟实现/待真机验证 |
| R1 | routes/token/状态兼容、实际 listener lease 与 pin | 旧 direct 不变；候选严格；token越界拒绝；lease失效零写入 |
| R2 | Desktop 两端点、唯一目标转发、scope隔离、诊断/账本 | 三协议 mock 非流/SSE/工具/用量；实际型号与价格身份准确 |
| R3 | 四文件路由事务、模式切换、重应用、启动/退出/释放、UI | 首次基线不移动；其他客户端独立；退出失败可恢复 |
| R4 | 全库回归、真实 Desktop 三协议验收、文档与打包 | 以真实行为限定支持范围，不把 mock 计为真机通过 |

R1–R3 各批必须自审后独立提交；未达状态/恢复门槛时不开放正式路由切换。除类型和共享入口的必要变动外，不重构全局 Provider schema、不重写协议转换器、不引入第二套账本。失败时优先回滚本批新增状态/模式，不能要求重建已有中转配置。

## 10. 验收矩阵

### 10.1 自动化：必须在隔离根和本地 mock 完成

| ID | 场景 | 必须证明 |
| --- | --- | --- |
| R-A01 | 旧 direct/official 状态载入、重应用 | 不误解释成 router，不改原 profile |
| R-A02 | 各角色不同卡片/同卡片不同模型 | 不改全局 activeProvider/default/protocol/enabled |
| R-A03 | 模型缺失、排除、禁用、只有余额 Key | 候选/提交/请求统一拒绝，手填例外限定精确默认模型 |
| R-A04 | profile 和 models | 只有合法 alias，非 Claude真实模型/真实Key不进入profile/DTO |
| R-A05 | 空角色/未知alias/旧安全前缀 | 400 或明确未配置，不模糊回落/偷偷回填 |
| R-A06 | token缺失/错误/旧版本、跨namespace | 两端点401；Desktop token不能调用Codex/p/v1；请求不生成Key |
| R-A07 | Key轮换、辅助余额变更 | 新请求用新推理Key；不影响direct状态/不借余额凭据 |
| R-A08 | 端口占用/0/监听停止重绑/进程epoch变化 | 拒绝profile写0；旧票据失效；四文件零写入 |
| R-A09 | 预览取消/过期、另一客户端在运行 | 释放临时引用不误停共享服务 |
| R-A10 | 预览后供应商/协议/模型/1M变化 | 拒绝旧计划；余额/无关卡片变更不误失效 |
| R-A11 | direct→router→direct→restore / rebaseline | 正常切换保留首次基线；显式重接管才更换 |
| R-A12 | 四文件逐步失败、state提交失败、提交后发布/清理失败、中断恢复 | 提交前统一回滚；提交后完成nextState，token/routes整代发布；启动先恢复后开放端点 |
| R-A13 | 关闭Codex/ClaudeCode/OpenCode、改端口 | Desktop网关仍存活；活跃端口不被偷偷改写 |
| R-A14 | router release / 托盘退出 / 更新重启 / Desktop运行中 | 不留无服务localhost；保持后台不触发延迟重启helper；不强行覆盖Desktop运行配置 |
| R-A15 | 崩溃重启、被其他进程占端口 | 稳定token/固定端口恢复；失败不直接写运行中Desktop |
| R-A16 | 两客户端/角色同session和真实模型 | 亲和、seen、工具/思考历史隔离；不继承全局failover |
| R-A17 | Messages/Chat/Responses，非流和SSE | 文本、多轮、工具定义/执行/回传与真实模型改写正确 |
| R-A18 | 流式错误/incomplete/断连/取消 | 终止分类与诊断准确，用量保留且仅落账一次 |
| R-A19 | 1M开/关、旧缓存marker、跨协议目标 | 一条菜单ID；off拒绝marker；Chat/Responses首版不宣称1M |
| R-A20 | alias像Opus但真实模型为DeepSeek等 | 按真实Provider/model价格绑定，零价/未知/缓存拆分保留 |
| R-A21 | 旧JSONL、compaction、同型号不同入口 | 兼容旧记录；汇总不以alias合并费用或伪造历史来源 |
| R-A22 | 浅深主题、缩放、窄窗、键盘、失败重试 | 卡片/映射/配置预览可操作，无重复说明/横向溢出 |

### 10.2 真机：实施阶段明确选择测试卡片后执行

记录项目提交、Desktop 版本/安装渠道、平台、真实协议、alias→Provider/model、网络出口与恢复结果；不记录 Key 或完整响应正文。

| ID | 真机案例 | 范围 |
| --- | --- | --- |
| R-L01 | 路由profile读取、角色菜单、实际请求alias | 不能仅用配置截图代替请求证据 |
| R-L02 | Messages / Chat / Responses 各一次普通及多轮对话 | 分协议验收，不把一条通路推广三协议 |
| R-L03 | 普通工具执行、回传和再回答 | 验证Desktop实际messages结构 |
| R-L04 | SSE逐段显示、终止/截断/取消 | 链路支持SSE不等于Desktop渲染通过 |
| R-L05 | 1M checkbox菜单变体与请求marker | 不承诺发送百万Token的完整上下文压力测试 |
| R-L06 | 关闭其他客户端路由、窗口隐藏托盘 | Desktop继续使用，同期其他配置正常 |
| R-L07 | 模式切换、完整退出前恢复、重新启动 | 验证文件与运行中客户端最终一致 |
| R-L08 | 原固定端口冲突、进程崩溃后启动恢复 | 明确可恢复路径，不把旧地址误报健康 |
| R-L09 | 真实身份与本地用量/价格快照核对 | 不用客户端alias或自报型号重定价 |

Windows 为第一验收平台。macOS/Linux、Fable、跨协议 1M、托管工具和 Cowork 都按各自证据另列，不因源码有分支就勾选通过。发布前 R-A01–R-A22 通过；R-L01–R-L09 中首版承诺的协议/功能必须有实测，其余标记缺口。

## 11. 本项目源码核对索引

| 入口 | 当前事实/复用点 |
| --- | --- |
| `claude_desktop/mod.rs:396,623,831` | 计划/事务、direct_plan、release当前仅tombstone |
| `claude_desktop/transaction.rs:215,452` | mode当前Direct/Official；首次editions基线 |
| `claude_desktop/profile.rs:253` | 四文件profile共同外形 |
| `claude_desktop/status.rs:181` | mode当前由own UUID推direct，须增加router识别 |
| `router_proxy.rs:40,141` | 当前ProxyHandle与ensure_running实际端口 |
| `router_proxy.rs:441,493,1948` | Provider目标、路径处理及现有全局鉴权 |
| `router_proxy.rs:618,739` | 1M跨协议限制、历史上下文scope |
| `router_proxy.rs:1165,1232,1344,1440` | 真实价格、URL/鉴权、模型改写、响应/SSE管线 |
| `routing_runtime.rs:14,26,59` | 丢弃实际端口、CLI退出恢复、needs_server无Desktop |
| `relay_usage.rs:257` | 当前记录真实model，新增可选来源/alias须兼容 |
| `lib.rs:69,293` | 窗口关闭与最终Exit；网关退出需更早协调 |
| `commands/tray_menu.rs:63`、`commands/system.rs:275` | 实际完整退出与更新重启入口，须共用协调且不提前spawn重启helper |

源码行号对应本文基线，实施后以函数与契约为准更新，不据旧行号推定实现仍相同。
