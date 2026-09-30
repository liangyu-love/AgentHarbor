# Magpie 借鉴与实测（2026-09-30）

## 结论与范围

保留现有固定 Router provider、模型前缀和会话一次接入机制。本轮只落地已复现的思考参数转换问题；不引入第二个网关、不替换运行中的应用、不改真实 Codex 配置或会话。

参考快照：`D:/workspace/temp/magpie`，commit `7a3337dd25bd759b1a73a051cdefeab6033b63f6`。参考其“思考能力与协议参数分离”和预算分档思想，独立实现小型 Rust 适配函数，没有复制 Go 网关或新增运行时依赖。

## 实际验证结果

| 项目 | 实验方式 | 结果及能证明的范围 |
| --- | --- | --- |
| 旧思考转换 | 真实已配置 Claude Opus 4.6 中转，分别发送 none/low/high | 三者实际请求都为 `thinking.enabled + budget_tokens=2048`；全部 HTTP 200。复现了 none 误开启、档位丢失，而不是连通性问题 |
| 新思考转换 | 同一真实中转及模型 | none 不附加 thinking；low/high 为 adaptive + 对应 effort；三者 HTTP 200、end_turn、有 OK 文本 |
| 老模型预算转换 | 真实已配置 Claude Haiku 4.5 中转 | none 不开启；low 4096；high 受调用方 8192 输出上限约束为 7168；三者 HTTP 200、end_turn、有 OK 文本 |
| Astra none 错误 | 真实已配置 GPT-6-Astra 中转，直接 Responses 非流式请求 | 本次 none/low/high 均 HTTP 200、completed；没有复现用户之前的 none 错误，不能推广为所有供应商一律拒绝 none，也不据此扩大目录宣称 |
| 同会话跨真实中转 | 安装的 Codex 0.159.0 + 本项目真实 Router + 配置的 GPT-6-Astra / Claude Opus 4.6 中转 | 同一个临时 thread，GPT→Claude→GPT，三轮均 completed；后两轮正确返回首轮随机 UUID 标记。没有 fork、新建替代会话或修改 provider |
| Magpie 式入口 | 安装的 Codex 0.159.0 + 合成 ChatGPT 登录 + 两个本地模拟端点 | `openai_base_url` 路径能保留 openai provider、原 ID、历史及官方/带前缀模型菜单；重启后模型 A→B→A 成功 |
| 旧实例热切换反例 | 上述实验中只改配置、不重启原 app-server | prefixed 模型请求仍到旧端点并失败；抓到了实际请求，而不仅是读取元数据 |
| 入口协议差异 | 捕获上述内置 provider 请求 | 请求为 `Content-Encoding: zstd`；实验网关做了解压。当前产品 Router 未因此获得完整 Codex backend 入口支持，不能只改配置就宣称已接入 |

真实中转测试仅使用合成内容，没有发送用户真实会话、项目文件或代码。源供应商加密文件前后逐字节相等。端到端测试只在临时目录保存临时配置、合成官方登录和中转凭据的加密副本，退出后清理；不会把真实官方凭据交给中转。

**不能从这些结果得出的结论：**

- HTTP 200 不证明中转内部没有改写 effort，也不证明 high 的质量高于 low；本次验证的是发出的参数、响应完成和短历史延续。
- 尚未做真实官方 ChatGPT 上游与真实中转之间的同会话往返；内置 provider 实验的上游是模拟服务。
- 尚未验证 1M 实际输入容量、长会话压缩、跨账号加密 reasoning/compaction、真实工具执行续接、配额耗尽及桌面端热刷新。
- 不采用 Magpie 遇到部分错误就删除不可读历史项的策略；本项目仍拒绝有损跨协议转换。

## 落地代码及审查

- `src-tauri/src/core/protocol/reasoning.rs`：思考参数适配，无网络/配置 I/O；none 不开启思考，已知新版 Claude 使用 adaptive，预算式模型按档位映射并尊重输出 cap。
- `protocol/chat.rs`：沿用历史与工具回合保护，调用适配函数；不在 Router 转发主流程堆模型特例。
- 新增 5 项单元回归，先在旧实现观察到 2 项失败，再确认修改后通过。覆盖无思考、档位差异、型号边界、预算下限/输出上限、强制工具选择及未知档位。
- 复用现有隔离 app-server 测试辅助代码新增真实中转往返测试，未建立第二套生产代理。
- 真实网络实验默认 ignored；需要显式选择已启用供应商和已配置模型。认证/限额错误或网络失败后不继续刷请求。诊断探针完成不等于其所有上游请求成功，应检查每行 status/stop_reason。
- 报告输出使用 create_new，防止环境变量误指向现有配置或凭据文件造成覆盖。错误日志脱敏；普通 CI 不读取真实凭据、不产生模型费用。
- 本轮未加全局“最近档位向上取整”。不同中转对同名模型的接受能力不一致，不能凭一次响应就替用户提高思考成本。

## 回归

- `cargo test --lib --quiet`：400 passed，1 ignored。
- 安装版 Codex 隔离回归：6 passed。
- 真实 Codex + Router + 中转往返：1 passed，3 轮 completed，后两轮 marker_retained=true。
- `npm run build`、`npm run test:frontend`：通过。
- Clippy：无错误；仍有既有警告，未为此批量改无关文件。
- 改动文件 rustfmt、`git diff --check`：通过。

本地原始证据位于 `.runtime-test/magpie-proof/`（git 忽略）：`claude-before.json`、`claude-after.json`、`haiku-after.json`、`astra-before.json`、`live-codex-routing.json`、`builtin-entrypoint.json`、`thinking-red.log`、各回归日志。内置入口隔离实验脚本为同目录 `probe_entrypoints.py`，依赖 Python 3.14 的标准库 zstd，不属于产品代码。

## 复现命令

无真实凭据的安装版 Codex 回归：

```powershell
$env:AGENTHARBOR_TEST_CODEX_BIN = '安装的 codex.exe 绝对路径'
cargo test --manifest-path src-tauri/Cargo.toml --test codex_router_auth real_codex -- --ignored --nocapture
```

真实网络实验会消耗供应商额度，只用合成内容。先选择具体供应商和模型；不要将密钥写入命令行：

```powershell
$env:AGENTHARBOR_PROBE_STORE = '现有 newapi-providers.dat 绝对路径'
$env:AGENTHARBOR_PROBE_PROVIDER = '已启用供应商 ID'
$env:AGENTHARBOR_PROBE_MODEL = '该供应商已配置的模型 ID'
$env:AGENTHARBOR_PROBE_CASES = 'current_none,current_low,current_high'
# 供应商不是 direct 时，可显式设置 AGENTHARBOR_PROBE_PROXY。
# 可设置 AGENTHARBOR_PROBE_REPORT 为一个尚不存在的 JSON 文件路径。
cargo test --manifest-path src-tauri/Cargo.toml --test router_live_probe -- --ignored --nocapture

$env:AGENTHARBOR_LIVE_ROUTING = '1'
$env:AGENTHARBOR_LIVE_MODELS = '[["供应商A ID","模型A ID"],["供应商B ID","模型B ID"]]'
cargo test --manifest-path src-tauri/Cargo.toml --test codex_router_auth live_codex_switches_configured_relays -- --ignored --nocapture
```

协议依据通过 Context7 核对官方 Codex `core/src/config/mod.rs`、Anthropic SDK 的 thinking/output_config 参数定义与 Claude extended-thinking 文档；兼容性结论以以上运行记录为准，不以 Magpie 注释或模型名称单独充当实测证明。

## 后续：中转 Claude 默认 1M（2026-09-30，用户指定策略）

### 设置与兼容性

- 中转 Claude（含 Haiku、旧型号、带供应商装饰的 ID）默认上下文为 `1_000_000`，不再按版本白名单限制；即使上游返回旧值，也使用本产品默认值。官方 Codex 模型条目不经过此策略。
- 现代版本供应商存储在正常加载时也会更新旧默认值，不再依赖旧 schema 迁移。`metadataSource=manual` 的上下文和输出值保持不变；扩大上下文不会联动放大已保存的输出上限。
- Claude Code 的启动模型与五个角色默认勾选 1M，补上 Haiku 开关。取消只移除强制标记，跟随客户端/模型，不代表一律降成 200K。Codex/OpenCode 使用的模型上下文可在模型列表中手动填写。
- 旧配置缺失的 1M 字段采用新默认；已有显式 `false` 保留。旧版本曾省略“全部 false”，无法从缺失字段推断当时是否主动取消；升级后需按需取消一次，新版本会完整保存关闭状态。
- 保存、重新打开、恢复 profile、切换模型/模式和缩减模型列表都不重新开启已经取消的开关。未改写真实用户配置，已有客户端配置仍需预览并应用。
- 模型接口补读 Anthropic `max_input_tokens`、`max_tokens`；保留原字段优先级，排除零/负数，并用最终上下文限制异常输出 cap。

### 实测证据

1. 新增回归先在旧实现失败：Haiku 200K 未更新、缺省配置未启用 1M；原始结果为 `claude-one-m/red.log`。
2. Rust lib：**405 passed，1 ignored**。覆盖现代 store 读取/重复读取、手动覆盖、接口字段优先级、旧 profile 缺字段、全部关闭后真实配置文件保存/重开。
3. 真实安装的 Codex 0.159.0 + 生产 Router + loopback 模拟 Messages 服务：Haiku 默认 1M 和手动 200K 均完成流式响应。`thread/tokenUsage/updated` 实际报告分别为 **950000** / **190000**，验证 95% 可用上下文计算跟随配置；非 Claude 对照为 128000 / 121600。6 项隔离 Codex 回归通过，官方账号展示、模型列表与同线程切换仍通过。
4. 浏览器回归通过：六个开关默认开启；全部取消、切换、应用/重开、profile 恢复均保留 false；目录刷新不覆盖手动 200K。桌面/窄窗口、浅色/深色截图已检查。
5. `npm run build`、前端全量回归、Clippy、改动 Rust 文件格式检查、`git diff --check` 通过；Clippy 仍有 37 条既有警告，构建仍有既有 chunk-size 提示。

这不是百万 token 容量测试，也不是对所有官方 Claude 型号容量的事实声明。本轮没有真实上游计费请求，没有启动真实用户 home 的 app-server，没有重启桌面或替换运行中的应用。

### 审查与简化

审查了未提交改动中的默认值/序列化、目录与官方认证边界、Router 转发/历史保护、一次接入与恢复弹窗，以及新增测试。修复了旧数据读取遗漏、全 false 被省略后重开、Haiku 开关缺位；删除了前端重复型号白名单、三处切换时重置 1M 的分支和废弃 `is_empty` 判断。复用既有默认值与 app-server 测试辅助函数，无新依赖、无第二套网关。

未发现本轮剩余的阻塞项；真实官方↔中转往返、加密压缩跨协议、实际 1M 输入容量仍是未验证边界，不包含在通过结论中。当前轮证据位于 `.runtime-test/claude-one-m/`（git 忽略）。

## 下一步优化优先级

1. **真实官方↔中转同线程验收**：保留同一 ID、历史和模型列表，抓取脱敏路由结果；覆盖工具调用后切换、应用重启后的恢复。现有真实中转往返不能替代真实官方上游验证。
2. **长历史与压缩**：对跨协议不可翻译的加密 reasoning/compaction 提供更明确的界面诊断和可恢复路径，不以删除历史换取成功。实际 1M 输入测试需另设成本与超时上限，不能通过短请求推断。
3. **减少模型能力猜测**：逐步采用供应商返回的能力信息，并保留来源与手动覆盖；1M 继续遵循用户指定策略，不能让刷新覆盖用户取消选择。
4. **工程维护**：统一 Rust 最低版本声明与实际使用 API 的要求，分类清理 37 条既有 Clippy 警告；按测量结果处理前端主包大小提示，避免为消除警告而大规模无关重构。
5. **发布验收**：Windows 安装/升级/卸载及 macOS/Linux 平台回归需要独立环境；生成安装包不代表已经验证真实安装升级。
