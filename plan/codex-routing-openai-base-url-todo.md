# Codex 路由注入重构：openai_base_url（2026-10-09）

## 目标

- 官方账号模型与中转模型同时出现在 Codex 菜单，任意线程（新/旧）可自由切换。
- 非 Responses 中转继续由路由转换协议。
- 开启路由后官方/中转模型都能压缩上下文。
- 开关路由、退出应用不再迁移线程，不再阻断关闭/退出。

## 根因（已核实）

- Codex 恢复线程时 provider 取自 `state_5.sqlite` 的 `threads.model_provider`；`thread/list`
  默认只列当前 `model_provider` 的线程。旧方案把 provider 换成 `agentharbor_relay`，
  只能双向迁移线程，失败即阻断关闭/退出。
- 本机 3 个 `openai` 线程带 `uuid::model` 中转 ID，直连 ChatGPT →
  `model is not supported when using Codex with a ChatGPT account`（对话与 remote compact 同因）。
- 已提交代码遇到 `encrypted_content` / `compaction` 历史即拒绝转换（400），官方→中转切换必挂。

## 新架构

- 账号模式：`openai_base_url = http://127.0.0.1:{port}/openai/{token}/v1`，不改 `model_provider`
  （移除我们写入的值，回到内置 `openai`），继续注入 `model_catalog_json`。
- 内置 provider 不能加自定义请求头 → 鉴权 token 放在 URL 路径。
- 无账号模式、Native 模式维持原自定义 provider（无官方线程）。
- 路由对内置 provider 请求：
  - websocket 升级 → 426，Codex 自动回落 HTTP；
  - `Content-Encoding: zstd` 解压；
  - 按目标渠道清洗历史：中转不发加密 reasoning / 内部字段，官方遇解密失败自动去 reasoning 重试；
  - `compaction_trigger`（remote compaction V2）：中转由路由生成可读摘要并封装成 compaction 项；
    后续请求把本路由的 compaction 项展开成摘要文本；官方原生 compaction 项发往中转时先请官方生成可读摘要（缓存）。
  - Codex 路由不再做会话绑定（每次请求都是完整历史）。

## TODO

- [x] 1. 路由入口：`/openai/{token}/v1/...` 鉴权与改写、426、zstd 解压
- [x] 2. 历史清洗（替代 validate_translated_input 的整单拒绝），官方解密失败重试
- [x] 3. 压缩代理：compaction_trigger 摘要生成、compaction 项展开、旧官方 compaction 转换
- [x] 4. 配置注入：账号模式写 openai_base_url；快照所有权；intact/matches 校验
- [x] 5. 去掉线程迁移：关闭/切 Native/退出只改配置 + 必要时重启 Codex；旧路由线程 provider 一次性归一为 openai
  - Rust lib 1137 passed / 0 failed（迁移相关旧测试已按新语义重写）
- [x] 6. 前端：删除迁移确认/强制关闭/批量接入，开关提示重启 Codex
- [x] 7. 测试：新增路由单测，删改迁移相关测试；cargo test、pnpm build、前端回归
- [x] 8. 真实 Codex CLI 隔离验证（openai_base_url + 回环上游）与文档：docs/codex-routing-openai-base-url-2026-10-09.md
- [x] 9. 开关路由只写配置，Codex 待重启提示 + 快捷重启；待重启期间路由保持监听
- [x] 10. 中转注入页面 UI 重设计（路由拓扑动效、分组重排、卡片统一）
