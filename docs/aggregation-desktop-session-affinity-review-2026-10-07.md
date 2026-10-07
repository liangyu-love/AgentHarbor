# 聚合注入 Claude Desktop：会话亲和缺失与轮转缺陷审查

日期：2026-10-07
范围：`src-tauri/src/core/aggregation/`、`src-tauri/src/core/router_proxy/aggregate.rs`
参考实现：`D:\workspace\temp\axonhub`（Go）

本文保留修复前的代码片段和位置，修复后的验收见文末。复核确认了缺少会话身份、工具续轮裸轮转、私有历史准入三项问题；没有据此推断真实 Desktop 必定不发会话头。

---

## 摘要

Claude Desktop 使用聚合模型时，**没有已建立的成员绑定、且续轮携带私有历史**的请求会失败，返回：

```
HTTP 409
{"error":{"message":"conversation member is unknown or expired; start a new conversation",
          "type":"agentharbor_local_router"}}
```

出站前拒绝，路由诊断显示 `尝试记录 (0)`，耗时 5–9 ms。

两个问题同源于一处：

1. **聚合的会话亲和只认 5 个 HTTP 请求头**，没有这些头时绑定不会建立，即使 body 带有可解析的会话身份也没有被利用。带 `thinking` 签名的续轮需要"同一成员"这一保证，拿不出保证就被守卫硬拒 → 409。
2. **同一原因让无会话头的请求退化为裸轮转** —— 每个请求推进一步，不受对话边界约束，失去渠道内缓存复用的保证。缓存损失的具体比例未测量。

问题 2 不修也会被问题 1 掩盖：**关掉 thinking 后 409 消失，但缓存问题依然存在**。二者是同一根因的两种表现。

单渠道（非聚合）的 Desktop 路径不受影响，见 §2.4。

---

## 一、问题 1：带私有历史的续轮被硬拒（409）

### 1.1 守卫位置

`src-tauri/src/core/router_proxy/aggregate.rs:188`

```rust
if bound.is_none()
    && (resource_id.is_some()
        || previous.is_some()
        || requirements.private_history                       // ← 触发点
        || (requirements.continuation && session_key.is_some()))
{
    return Err((
        StatusCode::CONFLICT,
        "conversation member is unknown or expired; start a new conversation",
    ));
}
```

该字符串全仓库仅此一处，可直接定位。

### 1.2 `private_history` 何时为真

`src-tauri/src/core/aggregation/execution.rs:57`

```rust
private_history |= matches!(kind, "thinking" | "redacted_thinking")
    || object.get("signature").is_some()
    || object.get("encrypted_content").is_some();
```

遍历范围限 `["input", "messages", "system"]`，工具 schema 不参与判定（`execution.rs:44`）。

因此：

| 请求内容 | `history` | `private_history` | 守卫 |
|---|---|---|---|
| 单条 user 消息（首轮） | false | false | 放行 |
| `tool_result` 回填 | **true** | false | 放行 |
| assistant 的 `thinking` 块（含 `signature`） | true | **true** | **409** |

### 1.3 为什么 Claude Desktop 永远拿不到绑定

**绑定 key 只来自 5 个请求头**（`aggregate.rs:629`）：

```rust
fn session(headers: &HeaderMap) -> Option<&str> {
    ["session_id", "x-session-id", "x-codex-session-id",
     "conversation_id", "x-opencode-session"]
    .iter()
    .find_map(...)          // 全部缺失 → None
}
```

`aggregate.rs:166` 用它构造 `session_key`；`aggregate.rs:334`：

```rust
let binding_reservation = session_key
    .as_deref()
    .map(|key| affinity::shared().reserve_binding(key))
    .transpose()?;          // session_key 为 None ⇒ Ok(None)
```

⇒ **不预留、不 commit**。下一轮 `affinity::shared().binding(key)` 仍为 `None`。

> **复核纠正**：`bound.is_none()` 不要求 `session_key == None`，绑定不存在或过期时也会出现。
> 因此该 409 不能证明真实 Desktop 不发这 5 个头。合成测试证明的是“无头请求”的缺陷；
> 真实客户端携带哪些头和 metadata，仍需脱敏抓包确认。

### 1.4 为什么单渠道 Desktop 反而正常

`src-tauri/src/core/router_proxy.rs:892`

```rust
let provider_route = match segments.as_slice() {
    ["codex", "v1", ..] => requested_model.as_deref().is_some_and(|m| m.contains("::")),
    ["p", _, "v1", ..] | ["v1", ..] => true,
    _ => false,                       // ← /claude-desktop/v1 落这里
};
```

`/claude-desktop/v1/*` 判为 `false` ⇒ `affinity_key = None` ⇒ **压根不要求绑定**，目标由 role 映射（`gateway::resolve_message`）直接定死。

**只有聚合这条路径把"身份未知"升级成了硬错误。**

### 1.5 为什么此前未暴露

- 验收记录（`docs/model-aggregation-acceptance-record.md`）中 Desktop 多轮只验证了 **`tool_result` 回填** —— 按 §1.2，它只置 `history`，不置 `private_history`，可溜过守卫。这正是该 bug 长期潜伏的原因，也与计划文档「工具上下文未闭合…不无条件迁移」的自定规则相矛盾。
- `router_proxy/aggregate_tests.rs:1521` `desktop_aggregate_projects_public_identity_and_keeps_one_m_wire_contract_across_egresses` 只发送**单条 user 消息**，未覆盖续轮。
- `thinking.type: adaptive` 是新 Claude 模型的默认项（见故障截图请求参数），因此该路径现在**每段对话必走**，从偶发变为必现。

---

## 二、问题 2：无会话身份时退化为裸轮转

### 2.1 算法本体

调用点 `aggregate.rs:404`：

```rust
agg::runtime::shared().select(&aggregate.id, aggregate.strategy, &candidates, &excluded)
```

`src-tauri/src/core/aggregation/runtime.rs:150` `select`：

1. **排序**（`runtime.rs:191`）：`priority` 升序，其次 `member_id` 字典序（即 UUID 序，**不是 UI 添加顺序**）。
2. **物理去重**（`runtime.rs:196`）：`target_key` 相同者合并 —— 同 endpoint + 同凭据 + 同 wire model + 同协议视为同一物理目标。两个不同供应商的成员 `target_key` 不同，正常计为两个候选。
3. **过滤**：排除本次请求已试过的、冷却中的、并发满 16 的、探活中的。
4. **锁优先级带**（`runtime.rs:232`）：
   ```rust
   let priority = ordered[0].priority;
   ordered.retain(|candidate| candidate.priority == priority);
   ```
5. **带内选择**：
   - `RoundRobin`（`runtime.rs:249`）：取第一个 `member_id > rotation.last_member`，无则回绕到第 0 个。
   - `WeightedRoundRobin`（`runtime.rs:400`）：平滑加权，credits 记账。
6. **游标 key**（`runtime.rs:243`）：`(scope, priority)`，其中 `scope` = **`aggregate.id`**。

### 2.2 状态只有游标，没有粘性

唯一跨请求状态是 `rotation.last_member`。**游标 key 不含 client** —— Codex / Claude Code / Desktop 共用一个聚合时共用同一游标。

`Affinity::Session`（默认值，`aggregation/types.rs:20`）本意就是粘性层，但它的实现依赖 `binding`，而 `binding` 依赖请求头。**Desktop 拿不到头 ⇒ 粘性层空转 ⇒ 退化为裸轮转。**

### 2.3 三种场景的实际后果

| 场景 | 守卫 | 实际行为 | 用户感知 |
|---|---|---|---|
| 开 thinking（新默认） | **拒绝** | 第 2 轮出站前 409，**不消耗轮转游标** | ❌ 对话每段从第 2 轮死掉 |
| 关 thinking，带工具 | 放行 | 每轮**静默换成员** | ⚠️ 不报错；新成员收到它未产出的 `tool_use`/`tool_result` 历史；缓存每轮全冷 |
| 纯文本多轮 | 放行 | 每轮换成员 | ⚠️ 缓存每轮冷启动，慢且贵 |

> 注意第 1 行：守卫位于 `select` 调用（`aggregate.rs:404`）**之前**，因此 409 不推进游标 ——
> 现象是"压根没轮到任何成员"，而非"轮到某成员失败"。这正是诊断里 `尝试记录 (0)` 的来源。
>
> 也因此：**优先级、单成员化等任何调度侧调整都治不了 409。**

### 2.4 缓存的代价

prompt cache 按**上游账号**隔离（Anthropic `cache_control` 绑定 API key；OpenAI 前缀缓存按账号；中转后端各自一套，互不共享）。聚合两个同类模型 = **两套独立缓存**。每轮换成员：

```
轮1 → A   A 未见此前缀 → 全量冷启动，全额计费，最慢
轮2 → B   B 亦未见      → 又一次全量冷启动
轮3 → A   A 仅轮1段是热的，轮2回复 A 未见过 → 大量 cache write
```

命中率趋近 0，成本与首字延迟随对话长度**线性增长**，不被缓存摊薄。

### 2.5 优先级可作手动止血（治不了 409）

§2.1 步骤 4 的优先级带裁剪发生在**冷却过滤之后**，因此：

| 成员状态 | 结果 |
|---|---|
| A 健康 | 仅 A 进入候选（B 不参与） |
| A 429 冷却 / 并发满 / 被排除 | A 已被滤掉，`ordered[0]` 变为 B → 自动降级 |

把成员设为不同 `priority`（如 A=0、B=1）即可获得"优先固定、故障降级"的效果，缓存随之转热。

⚠️ 但如 §2.3 所述，**守卫在调度之前，优先级无法解决 409**。此项仅为临时规避，不是最终方案。

---

## 三、复现

### 3.1 用例

加入 `src-tauri/src/core/router_proxy/aggregate_tests.rs`：

```rust
/// Claude Desktop 不发会话身份头，且会原样回填含 thinking 签名的 assistant 轮次。
/// 该对话的第二轮必须能继续被服务。
#[test]
fn desktop_aggregate_continuation_with_private_history_is_not_rejected() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let _stop = Stop;
        let upstream = mock(router::ModelProtocol::AnthropicMessages, 200, false).await;
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().into());
        let store = fixture(&[(
            &upstream,
            "platform/real-wire-fast",
            router::ModelProtocol::AnthropicMessages,
        )]);
        let (client, base) = start(&paths, &store).await;
        let alias = desktop_aggregate_state(&paths, running_port().unwrap(), &store.aggregates[0]);

        // 第一轮：单条 user 消息，无私有历史
        let fresh = client
            .post(format!("{base}/claude-desktop/v1/messages"))
            .bearer_auth("desktop-fixture")
            .json(&json!({"model":alias,"max_tokens":128,
                "messages":[{"role":"user","content":"hello"}]}))
            .send()
            .await
            .unwrap();
        assert_eq!(fresh.status(), StatusCode::OK);
        fresh.bytes().await.unwrap();

        // 第二轮：客户端原样回填上一轮 assistant 的 thinking 块（含签名）
        let continuation = client
            .post(format!("{base}/claude-desktop/v1/messages"))
            .bearer_auth("desktop-fixture")
            .json(&json!({"model":alias,"max_tokens":128,"messages":[
                {"role":"user","content":"hello"},
                {"role":"assistant","content":[
                    {"type":"thinking","thinking":"plan","signature":"c2lnbmF0dXJl"},
                    {"type":"text","text":"hi"}]},
                {"role":"user","content":"again"}]}))
            .send()
            .await
            .unwrap();
        let status = continuation.status();
        assert_eq!(status, StatusCode::OK, "{}", continuation.text().await.unwrap());
    });
}
```

### 3.2 执行

```powershell
node scripts/tauri.mjs --test-rust desktop_aggregate_continuation_with_private_history_is_not_rejected -- --nocapture --test-threads=1
```

### 3.3 实测结果（2026-10-07）

```
第一轮（单条 user）        → 200 OK
第二轮（含 thinking 签名） → 409 CONFLICT
  {"error":{"message":"conversation member is unknown or expired; start a new conversation",
            "type":"agentharbor_local_router"}}
```

与故障截图（HTTP 409、0 attempts、5–9 ms）逐字一致。

> 该用例当前为**红**，作为回归锚点保留至修复落地。

---

## 四、参考实现：axonhub

### 4.1 优先级分组 + requiredCount 早停

`internal/server/orchestrator/candidates.go:909` `sortCandidates`：

- 按 `c.Priority` 分组，**值小者为高优先级**，升序处理。
- 从最高优先级组开始取，累计到 `requiredCount` 即 **break，不再查看更低优先级组**。

默认 `internal/server/biz/system_default.go:80`：

```go
MaxChannelRetries: 3,        // ⇒ requiredCount = 1 + 3 = 4
```

⇒ 高优先级组只要有 ≥4 个渠道，低优先级组**永不参与**。仅在最高组数量不足时才向下补位。

### 4.2 sticky 层（关键差异）

`internal/server/biz/system_default.go:84` 默认：

```go
TraceStickyMode: TraceStickyPreferPreviousChannel,
```

`candidates.go:734` `LoadBalancedSelector.Select`：命中 previous channel 时，`pinStickyCandidate` 将其**钉到队首**并标记 `TraceSticky = true`，其余候选仅作 retry fallback：

```go
result := append([]*ChannelModelsCandidate{stickyCandidate}, fallbackCandidates...)
```

previous channel 查 trace → 回退 thread（`candidates.go:812`）。缓存写于 `internal/server/biz/request.go:1757`，**TTL = 30 分钟**（trace 与 thread 两个 key 各自 30 min，`request.go:1783`）。

⇒ **识别到会话并命中 sticky 时优先使用原渠道**；缓存过期或渠道不在候选中时回到负载均衡。若 body 也没有可解析的会话 ID，这条提取路径同样不能凭空建立 sticky。

> 轮转**粒度**两边其实一致（axonhub 也是每请求 `TrackSelection` 一次，
> `load_balancer.go:178`，且只对 `result[0]` 计数）。差异全在 sticky 这一层。

### 4.3 会话身份：header 优先 + body 兜底

`internal/server/middleware/trace.go:182` `tryExtractTraceIDFromClaudeCodeRequest`：

```go
userID := gjson.GetBytes(bodyBytes, "metadata.user_id").String()
uid := claudecode.ParseUserID(userID)
traceID := uid.SessionID
```

即 **从请求体 `metadata.user_id` 解析会话 UUID**，不依赖任何自定义头。OpenCode 走 `X-Opencode-Session` / `X-Session-Id` 等头；Codex 走 `Session_id` 头。

### 4.4 签名不匹配时过滤而非拒绝

- `llm/transformer/shared/signature.go:33` `GuessSignatureProvider` 对 blob 分类（`gAAAA*`→OpenAI；含 `claude`/`anthropic` 的 base64→Anthropic；protobuf 形态→Gemini；其余→unknown）。
- `llm/transformer/anthropic/outbound_convert.go:103` `prepareAnthropicReasoning`：解码失败时返回 `(nil, nil)`，即**连内容带签名一并剥离**，而非报错。
- Gemini 侧同理（`gemini/outbound_convert.go:456`），无有效签名时以默认签名兜底。

全仓库**不存在**任何类似 409 的路由拒绝。`.llm/transformer/shared/README.md` 记录了取舍理由：直接转发会导致上游硬失败（`encrypted content could not be verified`），故选择过滤。

---

## 五、对照与结论

| 维度 | AiMaMi | axonhub |
|---|---|---|
| 轮转粒度 | 每请求一格 | 每请求一格（相同） |
| 粘性层 | 有（`Affinity::Session`，默认开），但 key 依赖请求头 | 有（trace/thread sticky，默认开，TTL 30 min） |
| header-less 客户端的粘性 | **空转** → 裸轮转 | body 带可解析会话身份时生效 |
| 会话身份来源 | 仅 5 个请求头 | header 优先 + **body 兜底**（`metadata.user_id` 等） |
| 私有历史归属未知时 | **硬拒 409** | 不拒绝；签名不匹配则**过滤剥离** |
| 优先级语义 | 仅保留最高可用优先级带 | 按优先级收集候选，满足 `requiredCount` 后早停；高优先级数量不足时会收集较低优先级候选，并非完全相同 |

**结论：两个问题是同一个根因 —— 缺少可用于 header-less 客户端的会话身份。**

该根因同时导致：
1. 私有历史续轮无主 → 409；
2. 粘性层空转 → 每轮换成员 → 缓存逐轮失效。

因此**修复应聚焦"补身份"，而非调整轮转算法或优先级**。一旦绑定可建立，两个症状同时消失。

另需注意：`src-tauri/src/core/protocol/message_history.rs:404` **已经在读 `metadata.user_id`**，只有聚合亲和未复用该路径 —— 与 axonhub 的做法存在现成的对齐点。

---

## 六、待确认事项

尚未确证的是：**真实 Claude Desktop 是否携带任何可用于会话身份的标识**（如 `metadata.user_id`）。修复同时覆盖两种情况：

- 若携带 → 直接接入 body 解析，改动最小、语义最干净。
- 若不携带 → 用已交付完整历史的摘要找回所属会话。首条 user 消息或 alias 单独作为 key 会混淆不同对话，因此未采用这两种方案。

诊断面板只保留「客户端请求参数」，不记录入站 header（`src-tauri/src/core/request_diagnostics.rs`），**无法从 UI 判定**，需临时埋点或抓包。

---

## 七、次生问题（建议一并复核）

### 7.1 `private_history` 与跨协议出口互斥

`src-tauri/src/core/aggregation/execution.rs:129`

```rust
if self.private_history && self.ingress != effective {
    return Err("private_history_protocol");
}
```

Desktop 入站为 Messages，故带 thinking 的续轮**只能落到 AnthropicMessages 出口的成员**。若聚合内含 Chat/Responses 出口成员，它们会被逐个跳过（`aggregate.rs:240`）；若一个 Messages 出口成员都没有，则报 400 `private history requires its original protocol`。

该规则会拦截携带 Anthropic 私有块的跨协议续轮。但是 `message_history` 整体并非不可达：Chat/Responses 私有输出不会被伪造成 Anthropic 签名，普通可见续轮原本就能进入缓存恢复路径。修复应保留该恢复机制，并要求续轮保持原连接与有效协议。

### 7.2 工具续轮静默换成员

如 §2.3 第 2 行：`tool_result` 只置 `history`，守卫放行，于是续轮被静默转发给未产出该历史的成员。不报错，但与计划文档第 362 行「工具上下文未闭合…不无条件迁移」的规则相冲突。修复 §1 的会话身份后，此问题一并消失。

---

## 附录：代码位置索引

| 位置 | 内容 |
|---|---|
| `router_proxy/aggregate.rs:188` | 409 守卫 |
| `router_proxy/aggregate.rs:166` | `session_key` 构造 |
| `router_proxy/aggregate.rs:183` | `bound` 判定 |
| `router_proxy/aggregate.rs:334` | `binding_reservation`（None 即不 commit） |
| `router_proxy/aggregate.rs:404` | 轮转调用（守卫之后） |
| `router_proxy/aggregate.rs:629` | `session()` —— 5 个请求头 |
| `router_proxy.rs:892` | `provider_route`（Desktop 判为 false） |
| `aggregation/execution.rs:44` | `Requirements::parse` 遍历范围 |
| `aggregation/execution.rs:57` | `private_history` 判定 |
| `aggregation/execution.rs:129` | `private_history_protocol` 准入 |
| `aggregation/runtime.rs:150` | `select` 主体 |
| `aggregation/runtime.rs:191` | 排序 |
| `aggregation/runtime.rs:232` | 优先级带裁剪 |
| `aggregation/runtime.rs:243` | 游标 key |
| `aggregation/types.rs:20` | `Affinity` 枚举 |
| `protocol/message_history.rs:404` | 已有的 `metadata/user_id` 读取 |
| `protocol/message_history.rs:331` | `prefix_hashes` |
| `router_proxy/forward.rs:95` | 跨协议历史的构造条件 |

| axonhub | 内容 |
|---|---|
| `internal/server/biz/system_default.go:80,84` | `MaxChannelRetries=3`、sticky 默认开启 |
| `internal/server/orchestrator/candidates.go:734` | `Select`，sticky 钉队首 |
| `internal/server/orchestrator/candidates.go:812` | `resolveStickyChannelID` |
| `internal/server/orchestrator/candidates.go:909` | `sortCandidates`，优先级早停 |
| `internal/server/biz/request.go:1783` | previous-channel 缓存，TTL 30 min |
| `internal/server/middleware/trace.go:182` | 从 `metadata.user_id` 提取会话 |
| `llm/transformer/shared/signature.go:33` | `GuessSignatureProvider` |
| `llm/transformer/anthropic/outbound_convert.go:103` | 签名不匹配则剥离 |

---

## 2026-10-07：修复与验收

### 最终行为

1. 聚合首先读取明确的会话身份：保留原有请求头，补充 `session-id`、`x-opencode-session-id`、Codex turn metadata，以及 body/metadata 的 session 字段。Claude Code 的 `metadata.user_id` 支持旧版 `_session_<uuid>` 与新版 JSON 格式。账号或设备 ID 本身不会作为会话 ID。
2. 无明确 ID 的 Messages 首轮生成独立会话标识，仍由原轮转算法选择成员。两个相同首句的新对话不会被首句 hash 强行合并。
3. JSON 回复或 SSE 合法终态交付后，登记“完整入站历史 + 完整 assistant 回复”的 SHA-256 摘要与会话标识。下一轮按最后一个 assistant 前缀找回原会话，再使用现有锁和成员绑定。签名、工具 ID、工具输入参与摘要，缓存提示不参与；只做字符串/文本块等价归一，不按关键词猜会话。
4. 无 ID 的工具回填、签名续轮和普通文本续轮保持原成员。历史未知、过期、两个会话的完整历史无法区分、原成员失效或连接身份变化时仍在出站前拒绝，不把私有或工具历史交给另一成员。
5. 同一 Messages 会话保持原有效协议。Command 的 Responses 配置如果因搜索使用 Chat，后续不再声明搜索时也保留 Chat。原生 Messages 签名逐字段保留；Messages→Chat/Responses 复用已有私有历史缓存，分别恢复原 reasoning 和原 output IDs/encrypted content，不伪造 Anthropic 签名。其他协议的 opaque state 仍禁止跨协议迁移。
6. `count_tokens` 可以处理带历史的请求，仍仅选择原生 Messages 成员，不建立推理绑定、不落推理用量账。首轮错误或流取消会释放未发布的合成会话绑定；失败续轮保留已有绑定。

新增 `router_proxy/aggregate_session.rs` 只负责会话提取、摘要与当前回复观察。选择器、优先级、权重、凭据、公开模型 ID、四类注入接口、诊断与账本格式均沿用原实现。

### 内存边界

所有新状态只在内存中：最多 4,096 条固定大小的历史摘要索引，空闲 TTL 为 24 小时。沿用最多 1,024 个会话绑定、1,024 个 Responses 资源归属；达到容量在派发前返回容量错误，不驱逐活跃归属后悄悄换成员。

摘要索引不保存提示词、回复或签名正文。流式登记只暂存当前回复，限制为 16 MiB，并复用现有 256 KiB 单帧限制；结束或取消即释放。跨协议私有输出仍使用既有 16 MiB / 512 轮 / 1 小时缓存。未增加磁盘对话日志或诊断正文留存。

进程退出后这些状态会消失。旧历史不能确定成员时仍需开启新对话；压缩、改写或截断完整历史后若失去匹配前缀，也不会猜测成员。明确的会话 ID 能避免依赖正文摘要，但不能恢复进程重启后未知的旧绑定。

### 回归证据

修复前新增 `desktop_headerless_signed_history_stays_on_its_member_and_new_conversations_rotate`，实测第二轮返回 409，断言失败；修复后同一用例通过。

新增或扩展覆盖：

- 同首句的新会话轮转，两组带原生签名的续轮各自返回原成员；密钥变更和跨客户端复用历史均在出站前拒绝。
- 三种出口 × JSON/SSE 的 Desktop 工具续轮，SSE 按 7 字节拆帧；核对思考签名、工具 arguments/call ID、原 Chat reasoning、原 Responses reasoning ID/encrypted content 和 function item ID。
- body 中的 Claude Code 会话 ID 区分相同首句；未知工具历史拒绝派发；无 ID 的完全相同已交付历史建立歧义标记而非选择任一成员。
- Codex/OpenCode 请求头、两版 Claude metadata、账号 ID 排除、控制字符/长度限制；缓存提示归一、工具输入与签名不得合并。
- 并行工具 JSON、分段签名与 citation delta 重组；非法参数、过大回复、未知 semantic delta 不登记错误摘要。
- 摘要索引容量、取消预留、TTL、歧义标记；取消首轮释放绑定、失败续轮保留绑定；计数请求不提前绑定推理成员。
- 续轮诊断为 `aggregate_session_binding`，实际成员、出站模型及实际用量归属一致；拒绝请求为 0 attempts 且不增加账目。

完整命令：

```powershell
node scripts/tauri.mjs --test-rust -- --test-threads=1
```

结果：**1,039 库测试 + 14 Desktop 集成测试 + 20 工作流测试通过，0 failed**。证据保存在本机忽略目录 `.runtime-test/aggregation-session-fix-rust.log`。`git diff --check` 通过。

本轮未进行真实 Desktop 抓包、真实渠道推理或缓存命中率基准，不将这些项目标为已验证。没有停止客户端、写入真实配置或查询余额。

## 2026-10-07：渠道保持与故障阈值替代请求轮转

以上轮转复现及会话修复是 `6748449` 及其之前的历史行为。本轮按用户规则修改聚合调度：新会话和无 session 请求都持续使用聚合当前命中成员，同优先级也不再逐请求轮转。完整会话摘要及私有/工具/资源归属仍保留，两个相同首句的新对话有独立会话身份，可以使用同一渠道，不能合并它们的历史或签名。

数字越小优先级越高。初选从最高可用优先级开始；当前渠道持续成功就保持它。默认连续 3 次实际模型渠道故障才允许替换，完整合法成功响应清零。达到阈值后重新从所有可用成员的最高优先级组选择，因此支持高→低、同组替换及低→高；旧高优先级恢复不会抢占仍健康的低优先级命中渠道。顺序和权重只决定初选或必要替换，配置枚举保留兼容，界面不再称为逐请求轮询。

连接失败、超时、断流、明确的服务端渠道错误及 408/429/500/502/503/504/529 计入；模型/参数/协议校验、401/403/404 等、计数/资源操作、搜索服务失败和客户端取消不计入。阈值前有 Retry-After 时暂停当前渠道，本地暂停拒绝不累计次数。旧在途成功不能清除更新的故障；当前渠道满并发或未完成恢复试探不触发分流。已失败的试探解除选择阻挡，仍持有实际响应并发槽；旧试探的完成或释放不能影响新试探。

新鲜且可移植的请求在阈值后可以更新 session 绑定；工具回填、签名、加密历史和原生 response 资源仍须回原成员，不能因为故障阈值就转交另一渠道。5xx、已输出流或工具副作用不自动重放；只有既有安全边界内的建立连接前失败/429 可在阈值后尝试一次备用。

详细测试结果和证据追加在 [聚合验收记录](model-aggregation-acceptance-record.md) 的“命中渠道保持与连续故障切换”章节。本轮没有实测真实缓存命中率；这里只保证正常请求不再因逐请求轮转主动破坏渠道缓存。
