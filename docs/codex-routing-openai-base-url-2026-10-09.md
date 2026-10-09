# Codex 路由注入重构：内置 provider + openai_base_url（2026-10-09）

取代 `codex-routing-close-and-terminal-proxy-fixes-2026-10-09.md` 中的“关闭接入时迁回会话”方案。

## 问题与根因

- Codex 恢复线程时，provider 和 model 取自 `state_5.sqlite` 的 `threads` 行；`thread/list`
  默认只列出当前 `model_provider` 的线程。
- 旧方案把 `model_provider` 换成 `agentharbor_relay`。旧官方线程只有迁移后才能用中转；
  关闭时又要把线程迁回。迁回需要停 Codex、改写 rollout/SQLite、写 journal，任何一步失败
  都会阻断关闭或退出，表现为 Codex 反复重启、提示“部分会话未能迁回”。
- 线程如果仍是 `openai` provider 却选了 `uuid::model`，请求会直连 ChatGPT，对话和
  remote compact 都会报 `model is not supported when using Codex with a ChatGPT account`。
- 已提交的路由在遇到 `encrypted_content` 或 `compaction` 历史时整单拒绝协议转换，
  所以官方切到中转几乎必定失败。

## 新方案

账号模式写入：

```toml
openai_base_url = "http://127.0.0.1:{port}/openai/{token}/v1"
model_catalog_json = "<AgentHarbor 目录>"
```

`model_provider` 保持内置 `openai`。若原值是其它 provider，会改成 `"openai"`，关闭时恢复原值。
所有线程的 provider 永远不变：

- 开启或关闭路由只改上面的配置，不迁移线程，也不阻断关闭或退出。后端**不会自动重启 Codex**：
  Codex 正在运行时，界面弹窗提示“立即重启 / 稍后重启”。选择稍后，接入卡片和拓扑图上会保留
  “Codex 待重启”提示和快捷重启按钮。待重启期间本地路由保持监听，旧连接继续可用。
  只有退出 AgentHarbor 时（路由即将消失）才会在 Codex 运行时重启它一次，失败也不阻断退出。
- 内置 provider 不能附加自定义请求头，所以本地 token 放在 URL 路径里，由路由校验。
- 无账号模式没有 ChatGPT 登录，继续使用 `agentharbor_relay`。Native 模式不变，
  切到 Native 时会移除 `openai_base_url`。
- 选中聚合时也走内置 provider：路由按聚合的公开模型 ID 分发。
- 旧版本留下的 `agentharbor_relay` / `agentharbor_aggregate` 线程行，在 Codex 停止时
  （开关路由、重启 Codex、应用启动、退出）会通过一条 SQLite UPDATE 归一为 `openai`。
  路由关闭时，`openai` 线程中残留的 `uuid::model` 会清空为 NULL，Codex 改用配置中的默认模型。
  会话文件从不改写。

## 路由层（`router_proxy/codex_compat.rs`）

| Codex 内置 provider 行为 | 路由处理 |
| --- | --- |
| 先尝试 Responses WebSocket | 返回 426，Codex 在本会话内回落到 HTTP |
| `Content-Encoding: zstd` 请求体 | 解压并移除该头，限制解压后大小 |
| 历史含官方加密 reasoning | 中转请求丢弃 reasoning，并移除 `include: reasoning.encrypted_content`；官方请求保留 |
| 官方返回 400 拒绝重放的 reasoning（如账号切换后） | 去掉 reasoning 后自动重试一次 |
| `internal_chat_message_metadata_passthrough`、`encrypted_function_args`、`configuration_update` 等第一方字段 | 只从中转请求中移除 |
| remote compaction V2（`compaction_trigger`） | 官方模型原样转发；中转模型改写成 Codex 自己的摘要提示词，再把返回的摘要封装成一个 compaction 项 |
| 本路由生成的 compaction 项 | 发往任何目标前展开成可读摘要消息 |
| 官方加密 compaction 发往中转 | 先用同一账号把它转换成可读摘要（进程内缓存）；失败时改为提示文本，不把密文发给中转 |
| `x-codex-turn-state` | 只发给官方 |

Codex 路由不再做会话绑定，因为 HTTP 请求每次都携带完整历史。其它客户端的 `/p`、`/v1` 路由不变。

## 删除的内容

- 关闭、切 Native、退出时的线程迁回：`thread_recovery/transition.rs`、`codex_native::prepare_provider`，
  以及 `force` 参数和“仍然关闭 / 仍然退出”。
- 会话页的路由状态徽标、“批量接入路由”及对应命令（`codex_thread_route_overview`、`preview/apply_codex_thread_batch`）。
- 单线程“检查并修复会话”保留；无账号模式下仍可接入路由。

## 验证

| 检查 | 结果 |
| --- | --- |
| Rust 库测试（MSVC） | 1137 passed / 0 failed / 19 ignored |
| `local_workflows`、`claude_desktop_workflows` | 20 + 14 passed |
| 真实 Codex 回环（库内 4 个，Desktop 自带 0.162.0-alpha.2 与 CLI 0.159.2 各跑一遍） | 4/4 passed |
| `codex_router_auth` 真实 Codex 隔离用例（0.162.0-alpha.2） | 9/9 passed |
| 前端完整回归（Playwright + Chrome） | 195 组 PASS，EXIT 0 |
| `tsc`、`vite build`、`cargo check --all-targets`、`git diff --check` | 通过，无新增警告 |

真实 Codex 用例（`--ignored`，需要 `AGENTHARBOR_TEST_CODEX_BIN`）：

- `real_codex_switches_an_existing_official_thread_to_relays_without_adoption`：线程先在官方端点创建，
  切中转时复现 “not supported when using Codex with a ChatGPT account”；开启路由后，同一线程无需接管，
  provider 仍为 `openai`，即可在两个中转间切换，且历史完整。
- `real_codex_same_free_thread_switches_official_and_relays_repeatedly`：同一线程官方 → A → B →
  官方 → A → B → 官方，冷启动恢复后 provider 仍为 `openai`。官方夹具对中转 ID 返回与 ChatGPT
  相同的 “not supported” 错误。
- `real_codex_compacts_on_relays_and_official_and_keeps_switching`：中转压缩 → 官方读取摘要 →
  官方原生压缩 → 中转读取由账号转换出的摘要。
- 子代理、无账号目录用例经新配置同样通过。

上游都是回环夹具，没有发送真实账号或付费请求。真实 ChatGPT 后端的 426 回落、zstd 和 V2 压缩已通过
Codex 客户端验证；后端本身的行为（例如它是否接受转换摘要的请求）还需在真实账号上复测。

日志：`.runtime-test/routing-ui-rust-1.log`、`.runtime-test/routing-ui-frontend-final2.log`。
重构前未提交的改动备份在 `.runtime-test/pre-routing-refactor-20261009-215828/`。

## 中转注入页面重设计（同日追加）

- 顶部新增“路由拓扑”主视觉：左侧为 Codex、Claude Code、Claude Desktop、OpenCode、Pi，中间为
  AgentHarbor 路由核心，右侧为 OpenAI 官方、中转、聚合。经路由的链路显示渐变光线、流动虚线和粒子；
  直连中转绕开核心并用绿色表示，未接入为灰色虚线。开启路由时连线绘制、核心点亮并出现光环。
  悬停或聚焦节点只高亮相关链路；点击客户端或聚合节点跳到对应配置，点击中转节点打开中转模型页。
  窄屏改为上下布局，节点改为图标在上、名称在下。
- 页面按“客户端接入”（Codex、客户端配置、Claude Desktop）和“路由能力”（模型聚合、路由诊断与压缩、
  模型能力目录）分组，各区块统一卡片圆角、标题与图标样式，并带入场动效。
- Codex 接入卡片拆成“连接方式”“当前链路”“子代理默认配置”三块，新增待重启提示条。
- 所有动效遵守 `prefers-reduced-motion`。
