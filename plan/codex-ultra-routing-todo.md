# Codex Ultra 经路由注入失效

## 现象
选 Ultra 后路由诊断里看到的是 `xhigh`，也没有出现 Ultra 的"主动多代理"行为。

## 根因（对照 Codex 源码 aa38089 与本机会话）
- `ultra` 是 Codex 客户端别名，从不上线：`ModelInfo::resolve_reasoning_effort` 依次取
  `multi_agent_reasoning_effort` → `max` → 最高非 ultra 档 → `medium`。官方 gpt-6.1-sol 声明 `xhigh`，
  所以上游收到 `xhigh` 本身是正确的。
- Ultra 的真正差异是多代理 V2 的主动派发提示（`effective_multi_agent_mode` 仅在 V2 时生效）。
- Codex 按线程首次解析固定 `multi_agent_version`（`set_multi_agent_version_if_unset`）。
  中转目录项（`relay_catalog_entry`）不带 `multi_agent_version` → 走 features → V1，整条线程被钉死；
  之后切到官方模型 + Ultra 也只有 `xhigh`。中转项同时缺 `ultra` 档，选择器里不出现 Ultra。

## 步骤
1. [x] `adopt_codex_behavior`：同名官方条目的 `multi_agent_version`、`multi_agent_reasoning_effort`
       （仅当中转声明了该档）、`ultra` 档（官方 V2 且中转有推理档时）写入目录项；账号限额仍不继承。
2. [x] 应用到路由目录（`relay_menu_entry`，官方匹配不再受端点过滤，限额仍过滤）、原生目录、聚合目录（按公开模型名）。
3. [x] 代理兜底：`prepare_reasoning` 永不把字面量 `ultra` 当作上游支持的档位。
4. [x] 单测：纯函数表驱动、路由目录集成、原生目录、`prepare_reasoning` 的 ultra。
5. [x] Rust 全量测试 + 真实 Codex 回环（本机正在用的目录未改动，装新版后由应用自动重写）。

## 结果
- `router::adopt_codex_behavior`：同名官方条目 → `multi_agent_version`、`multi_agent_reasoning_effort`（中转声明了才写）、
  `ultra` 档（官方 V2 且中转有推理档时追加，描述沿用官方）；上下文 / 输出 / 推理档其余部分仍是中转自己的。
- 路由目录 `relay_menu_entry`：官方匹配对限额仍按端点过滤，对客户端行为不过滤；`catalog_entry_for_slug` 同源。
- 原生目录 `configuration_plan`、聚合目录 `aggregate_catalog_value`（新增 `codex_home` 参数，按公开模型名匹配）同样处理。
- `prepare_reasoning`：字面量 `ultra` 按 Codex 规则解析（多代理档 → max → 最高非 ultra 档），永不上线。
- 测试：纯函数表驱动、路由目录集成、原生目录、聚合目录、`prepare_reasoning`；
  真实 Codex 0.162 回环 `real_codex_relay_thread_keeps_ultra_and_multi_agent_v2`
  （菜单有 Ultra、上游收到 xhigh、线程不再是 multi_agent_v1）；关掉修复时该测试失败。
- Rust lib 1153 + 集成 14 + 20 全过；真实 Codex 回环 4 项全过。
- 限制：已经被钉在 V1 的旧线程不会变；从非 OpenAI 模型（claude、deepseek 等）起步的新线程仍按 Codex 特性默认（V1）。
