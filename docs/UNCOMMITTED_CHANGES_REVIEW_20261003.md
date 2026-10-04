# 未提交改动代码审查报告(2026-10-03)

> 范围:当前工作区全部未提交改动(80 个已跟踪文件,+4767/-876,含 10 个新文件),按 5 个功能组分组审查。
> 快照:2026-10-03 18:30–18:40 逐条现场复核。此前审查文档中 4 个已知发现(2×P1、2×P2)已在 17:20–17:45 的并发修复中全部解决并附回归测试,本文不再展开。
> **总体结论:COMMENT —— 无 P0、无 P1。属实发现 28 项:P2 × 8、P3 × 20,全部经源码逐条复核确认存在。**

## 复核方法与可信度

- 分组审查:5 个功能组各由独立审查通过,覆盖 `git diff` 全部改动与新文件全文;审查中实跑 67 个 `client_config` 测试、36 个能力目录单测,全部通过。
- 逐条复核:全部发现按 file:line 回到当前源码用 `sed`/`grep` 核对,非转述子代理结论。
- 修正记录:
  - 行号漂移 4 处(并发编辑所致):`account_monitor.rs` 108/118→138/148、`lib.rs` 171→175、`codex-access-card.tsx` 147→142、`test-frontend-regressions.mjs` 452/465→462。
  - 误报修正 1 处:OFFICIAL_CATALOGS 死条目由 5 个修正为 4 个——`baichuan` 实际存在于嵌入快照,初报有误。
- 标注「待验证」的发现共 4 处(P2-6、P2-8、P3-9、P3-18),均因依赖真实外部服务或需实际复现,代码层面的条件已确认存在。

---

## P2 —— 建议交付前修复(8 项)

### P2-1 `-max`/`-ultra` 被当思考档位剥离,与真实产品型号冲突

- **位置**:`src-tauri/src/core/model_metadata.rs:290`(`KNOWN_EFFORTS` 含 `max`、`ultra`)、`:407-413`(剥离逻辑)
- **证据**:`if mode.eq_ignore_ascii_case("thinking") || is_known_effort(mode) { push("reasoningSuffix", base); }`
- **影响**:嵌入快照中 `openai/gpt-5.1-codex-max` 与 `gpt-5.1-codex`、`google/imagen-4-ultra` 与 `imagen-4` 等是并存的真实不同型号。当前原厂目录恰好两者都缺或精确命中(exact 优先)所以未出错;一旦某原厂目录收录了 base 而漏掉 `-max` 变体,能力会被静默归一到错误型号(context/output/reasoning)并标为 Normalized 自动填入,与「已识别原厂但缺型号保持未匹配」的声明矛盾。
- **建议**:将 `max`/`ultra` 移出跨型号剥离集,或归约命中后做存在性校验(base 与变体同存时才允许归一)。

### P2-2 `nova-` 前缀产出白名单外的 `amazon-bedrock` scope

- **位置**:`src-tauri/src/core/router.rs:2448`(family 表 `("nova-", "amazon-bedrock")`)、`:2294-2326`(`OFFICIAL_CATALOGS` 不含它)
- **影响**:同一 provider 三条识别路径(家族前缀 / 主机匹配 / 全局引用 allowlist)信任边界不一致。方向是 fail-closed 的(`nova-lite-v1`、`amazon.nova-pro-v1:0` 等真实形态不可命中),但与文档「仅限 OFFICIAL_CATALOGS」不符;bedrock 条目的 canonical 是 `amazon/nova-lite`,仅 `nova-lite` 可经 canonical 命中。
- **建议**:family 表 owner 统一过 `official_catalog_namespace` 校验,并把 amazon-bedrock(或 amazon)显式纳入或移除白名单。

### P2-3 账户快照 reasoning 档位绕过 `ReasoningLevel::valid()`

- **位置**:`src-tauri/src/core/router.rs:2540-2551`
- **证据**:`if effort.is_empty() || effort.len() > 32 { return None; }` —— 仅限长度,不限字符集;description 无长度上限、级数无上限。
- **影响**:用户覆盖与 upstream 声明都经过 `valid()`(effort 仅字母数字下划线、description ≤1024、级数 ≤32),OFFICIAL 路径是同类输入两套校验。来源是 TLS 拉取的账户快照,风险有限(显示膨胀/配置写入放大),但结果会被序列化进客户端配置与 catalog JSON。
- **建议**:official 路径复用 `ReasoningLevel::valid()` 与 32 级上限。

### P2-4 每请求目录 scope 推断为 O(providers) 线性扫描 + 逐个 URL 解析,无缓存

- **位置**:`src-tauri/src/core/router.rs:2347-2368`
- **证据**:对 `catalog.snapshot.providers` 逐个执行 `official_catalog_namespace` + `reqwest::Url::parse` 后按 host 精确比对。
- **影响**:无家族前缀且无手动绑定的 provider 每个请求至少触发一次完整解析(`relay_pricing` 每次尝试必跑、Anthropic 翻译再跑一次、`catalog_entry_for_slug` 兼容检查再跑),每次对快照约 225 个 provider 逐个解析 URL。其余查询(scoped/canonical/global)均已索引,此处是唯一超线性热路径点。
- **建议**:catalog 构建索引时预生成 host→provider_id 映射,或对解析结果按 catalog revision 做 memo。

### P2-5 Command Code 套餐前缀匹配会把未来版本静默映射到旧额度

- **位置**:`src-tauri/src/core/router_balance.rs:376-398`(`command_code_plan_monthly`)
- **证据**:`starts_with("individual-pro-v1")` → 80.0,兜底 `starts_with("individual-pro")` → 30.0;`individual-goat`(70.0)必须排在 `individual-go`(10.0)之前。
- **影响**:供应商将来推出 `individual-pro-v2`/`individual-go-v2` 等新额度版本时,前缀兜底会命中并显示旧套餐总额(而非按「未知套餐不编造数值」原则返回 `None`);分支顺序即契约,靠隐式排列维持。
- **建议**:未知/无版本的分支改用精确相等匹配,或对带未知版本后缀的 id 返回 `None`;改为表驱动或给分支顺序加注释。

### P2-6(待验证) reset 时间戳一律按毫秒解释

- **位置**:`src-tauri/src/core/router_balance.rs:400-411`(`command_code_reset`)
- **证据**:`parse_number(...).filter(|time| *time > 0.0 && *time < i64::MAX as f64).and_then(|time| DateTime::from_timestamp_millis(time as i64))` —— RFC3339 之外的数字无量级判断。
- **影响**:若 API 实际返回秒级 epoch(如 `1900000000`),`resetAt`/`currentPeriodEnd` 会渲染成 1970 年的「重置于」文案而非被跳过。真实账户返回结构尚未核对,故标待验证。
- **建议**:加量级判断(`< 10^12` 视为秒或直接拒绝),或在真实账户核验时确认单位。

### P2-7 Claude `/model` 切换的冲突豁免提前返回,跳过 picker 兜底

- **位置**:`src-tauri/src/core/client_config/mod.rs:953-956`
- **证据**:`if let Some(selected) = &session.request.options.claude_model_ids { return selected.iter().any(...); }`
- **影响**:`claude_model_ids` 为 `Some` 时不再落到 installed picker 兜底:用户在 CLI 切到 picker 中存在(用户自增行)但不在存储的 `claude_model_ids` 里的模型,会被误判为冲突,阻断严格 restore/apply,迫使用户走替换/重接管;反向(id 在列表中但行已被删)则豁免过宽。触发面窄。
- **建议**:`claude_model_ids` 命中失败时继续落到 picker 兜底(取并集),并补一条回归。

### P2-8(待验证) 工具信封的 `$ref`-only items 不加信封;strict 分支追加 `additionalProperties:false`

- **位置**:`src-tauri/src/core/router_proxy/tool_schema.rs:31-36`(`needs_envelope`)、`:136-138`(strict)
- **证据**:items 对象只含 `$ref`(无 `type`)时 `!items.contains_key("type") && !items.contains_key("$ref")` 为 false,不触发信封;strict 时写入 `additionalProperties: false`。
- **影响**:若中转把 Chat/Responses 工具转换为 Gemini 受限 Schema,仅 `$ref` 的 items 无 type、`additionalProperties` 历史上不在受限 Schema 字段集内——两种形态可能「换个字段再报错」(与 P2-8 前身 items:{} 问题同型)。真实中转是否转发 `strict` 并做受限转换无法静态确定。
- **建议**:补充针对这两形态的受限契约测试(`valid_google_schema` 模型目前未含 `additionalProperties`),或确认目标通道后注释说明。

---

## P3 —— 择机跟进(20 项)

### 组 1:Claude 路由跨协议

| # | 发现 | 位置 | 建议 |
|---|---|---|---|
| 1 | 空 output 的 incomplete 回复可入缓存;Responses 续轮 `splice` 会把该轮替换为零条目,上游看不到该轮 | `message_history.rs:377-384`(cacheable 不要求 output 非空)、`:362-364` | `cacheable_response` 要求 output 非空,或 restore 在 `output.is_empty()` 时跳过 splice |
| 2 | 全局 `Mutex` 临界区内做 JSON 反序列化与 MiB 级字节比较,并发续轮互相阻塞 | `message_history.rs:297-310`(restore 锁内 `from_slice`)、`:84-88`(record 锁内比较) | 锁内只取字节克隆,解析/比较移到锁外 |
| 3 | 同一前缀的合法重试(输出不同)会永久墓碑化该键,严格上游(DeepSeek)后续续轮将 400 | `message_history.rs:84-91` | 保留机制,但在诊断中区分「墓碑 miss」与「无缓存 miss」 |
| 4 | 空 reasoning 文本会注入 `reasoning_content:""`,严格上游会拒绝 | `message_history.rs:327-345` | `text.is_empty()` 时同样跳过 |
| 5 | 非交替历史(连续 assistant 消息)合并后缓存签名双失配,推理回放静默跳过(静态推导,触发罕见) | `anthropic.rs:186-189`、`chat.rs:292-303`、`message_history.rs:143-170` | 按 tool_call id 集合做次级匹配,或接受为边界并注释 |
| 6 | 同模型 failover 换供应商后首个续轮必无推理缓存(隔离设计的正确代价) | `router_proxy.rs:658-663`(scope 含 provider/credential) | 在诊断中可见即可 |

### 组 2:配置接管冲突流程

| # | 发现 | 位置 | 建议 |
|---|---|---|---|
| 7 | 冲突说明在预览生成前就引用「下方预览」 | `client-config-dialog.tsx:332-334` | 改为「按预览替换」,或仅在 `preview` 存在时显示该句 |
| 8 | 被替换的用户外部值仅存于无 UI 入口的加密备份文件 | `client_config/mod.rs:600-610、746-753` | 文档注明备份位置,或提供备份列表入口 |
| 9 | (待验证)restore 清空所有权记录后,下一次全新接管的 legacy 形状清理可能误删 `label==model` 用户行 | `client_config/mod.rs:681-683、803-810`、`claude.rs:243-254` | 收窄 legacy 清理,或 restore 时保留用户行记录;补「restore 后重新接管」测试 |
| 10 | `conflicting_fields` 以 `path.join(".")` 归并,含 `.` 的键可能碰撞(方向是多保留用户值,安全侧) | `client_config/mod.rs:933、591` | rebaseline 采纳改用 `Vec<String>` 路径比较 |
| 11 | zh/en `restoreConflictDescription` 信息量不对称(zh 多「无需放弃接管」) | `src/locales/zh.json` / `en.json` | en 补 "no release needed" 或 zh 删去 |

### 组 3:供应商 / 余额 / UI

| # | 发现 | 位置 | 建议 |
|---|---|---|---|
| 12 | `balance_from_entries` 取第一个 entry 当卡片 headline,缺 credits 时把速率窗口剩余当余额展示 | `router_balance.rs:714-728` | headline 优先 monthly/purchased/free 类 entry |
| 13 | relay 卡 Command Code caption 耦合 `entries[0]` 顺序 | `relay-card.tsx:52` | 改为 `entries.find(e => e.label?.startsWith("Command Code"))` |
| 14 | 死 CSS 类 `thread-session-card` 无任何定义 | `thread-card.tsx:100、366` | 删除或补上预期规则 |
| 15 | 待加入区「全选模型」会加入全部 pending(含与当前搜索不匹配的) | `provider-model-editor.tsx:138` | 仅勾选 `matchingPending`,或 tooltip 说明范围 |
| 16 | `selected === 0` 的「模型库为空」空态与待加入区并存时自相矛盾 | `provider-model-editor.tsx:152-155` | 无 pending 时才显示该占位 |
| 17 | 前端回归新增断言硬编码中文 UI 文本,对文案改动脆弱 | `test-frontend-regressions.mjs:431、462` | 与现有 `labels` 表模式统一 |

### 组 4:模型能力目录匹配

| # | 发现 | 位置 | 建议 |
|---|---|---|---|
| 18 | (待验证)聚合商记录的 canonical 声明可参与自动 publisher 判定(仅身份提示;能力/价格数据不泄漏) | `router.rs:2390-2391`(`catalog.global(&id)` 未过滤) | global 结果按 `OFFICIAL_CATALOGS` 过滤,或注释说明属有意设计 |
| 19 | `OFFICIAL_CATALOGS` 含 4 个快照中不存在的死条目:`01-ai`、`internlm`、`bytedance`、`baidu` | `router.rs:2309-2312` 附近(全表 `:2294-2326`) | 与快照生成脚本联动校验,或注释标注「预留」 |
| 20 | `MAX_CANDIDATES=16` 可能截断多层装饰 ID 的深度归约 | `model_metadata.rs:36、337` | 按归约深度优先、提高上限,或在 trace 中标注「候选被截断」 |
| 21 | 请求翻译热路径解析 metadata 时传 `official=None`,与预览/注入输入不一致(仅官方端点 + Messages 翻译的罕见组合) | `router_proxy.rs:675-677` | 与注入路径同样传入 official |
| 22 | `docs/models-dev.md` 未覆盖 `nova- → amazon-bedrock` 例外与 `MatchKind::Invalid` | `docs/models-dev.md`(grep 计 0) | 补文档 |

### 组 5:其他改动

| # | 发现 | 位置 | 建议 |
|---|---|---|---|
| 23 | `compression_state` 中 plan 在获取 guard 之前计算,与 `intact()` 顺序不一致(仅影响 UI 瞬时展示) | `codex_native.rs:371-373` | 把 `configuration_plan` 挪到 guard 之后 |
| 24 | `config_active` 与 `applied_threshold` 对 config.toml 两次独立读取 | `codex_native.rs:375` | 第一次读到的 document 克隆复用 |
| 25 | `CREATE_NO_WINDOW` 三处裸写 `0x0800_0000`,未复用 `platform::windows` 已有常量(覆盖面经全仓清点无遗漏) | `account_monitor.rs:138、148`、`client_config/version.rs:43`;现成常量在 `platform/windows.rs:5、16` | 至少共享一个 const |
| 26 | `model_ids` 将 `provider.model` 不经协议检查并入 native catalog/共享压缩计划(HEAD 已有,非本次引入;可达性受前端 `nativeCompatible` 限制) | `codex_native.rs:257-259` | push 前补 `protocol_for_model == Responses` 过滤 |
| 27 | 无账号开关在注入前保存的 toast 文案为「已应用」,实际仅持久化未应用(前后端双重校验本身成立) | `codex-access-card.tsx:142` | 按 `change.state.router.enabled` 区分文案 |
| 28 | `reveal_newapi_provider_api_key` 进入 IPC 面:明文 key 返回 webview,仅靠前端显式点击约束,webview 内 XSS 可枚举拉取 | `lib.rs:175`(注册)、`commands/newapi.rs:199-215`(实现) | 后续加会话级确认或节流 |

---

## 修复优先级建议

1. **最可能变成用户可见错误,先修**:P2-1(`-max`/`-ultra` 剥离)、P2-5(套餐前缀匹配)。
2. **一致性收紧,可批量**:P2-2、P2-3、P2-7(白名单/校验/豁免统一),P2-4(热路径缓存,性能)。
3. **随联网验收确认**:P2-6(reset 时间单位)、P2-8(strict/$ref 契约)、P3-18(聚合商 canonical 数据质量)。
4. **P3 中值得优先**:P3-1(空缓存条目)、P3-2(锁内大比较)、P3-9(legacy 清理误删,需一条测试定性)、P3-25(常量共享)。

## 覆盖范围与残余风险

- **已覆盖**:5 个功能组全部 diff 与新文件;全部发现逐条现场复核;67 个 client_config 测试 + 36 个能力目录单测实跑通过。
- **未覆盖**:全量 `cargo test` / 前端回归未重跑(文档自述 553 Rust / 9 workflows / 64 前端通过为历史结果);真实上游未联网核对(Command Code / MiMo 账户、DeepSeek `/anthropic`、真实 Gemini 中转受限 Schema 转换);`claude_cli_tests.rs` 4 个 `#[ignore]` 真机测试未执行;并发/多进程竞态仅静态确认。
- **时效**:工作区在审查期间被并发会话修改过两轮(17:20–17:45 修复旧 4 发现;18:27–18:31 accounts/hotspot 一带,与本文全部发现位置零重叠)。若并发会话继续提交改动,P2 结论需按届时快照重验。
