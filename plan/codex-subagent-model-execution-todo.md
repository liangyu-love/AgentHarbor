# Codex 子代理模型配置实施 TODO

日期：2026-10-07。范围：Codex 接入卡片中的子代理默认模型与思考强度。

## 契约与边界

- 复用官方 `[agents].default_subagent_model`、
  `[agents].default_subagent_reasoning_effort`，不新增推理档位，不修改 Ultra。
- 默认不接管用户已有的子代理配置。保存“继承”明确删除对应默认键；
  “恢复原配置”撤销本功能的接管，保留外部后来修改的值。
- 保存配置；Codex 接入启用时应用，未启用时仅保存，下一次启用再应用。
- 模型候选复用当前接入模式的模型目录：
  智能路由使用官方裸 ID 或中转命名空间 ID；Native 使用当前中转的原始 ID；
  聚合模式使用聚合入口公开 ID。不可用目标拒绝应用，不回落官方。
- 设置只影响之后创建的子代理；不修改旧子线程、主线程模型、provider、
  自定义角色、其它 agents 配置、认证与会话文件。
- 显式 spawn 和自定义角色的优先级遵循官方实现。
  旧子代理自动跟随主通道、服务端加密协作与 Ultra 兼容不在本轮范围。

## TODO

- [x] P0：核对官方字段、优先级、生命周期；记录既有工作区改动边界。
- [x] P1：独立配置模块、持久化设置、按接入模式投影模型候选及思考档位。
- [x] P2：复用配置仓库与现有 journal，扩展两项字段的可逆接管；
  覆盖首次基线、重复应用、清除、恢复、失败与外部修改。
- [x] P3：命令、类型、API 与 Codex 接入卡片中的配置入口；
  继承、保存、恢复原配置、不可用选项、加载/失败/操作锁、中英文。
- [x] P4：后端回归：普通路由、Native、聚合、模式变化、
  删除/停用模型、档位不支持、非表 agents、旧 journal、配置修复与关闭还原。
- [x] P5：前端回归：保存/重开/继承/还原、长模型名、深浅主题、窄屏。
- [x] P6：真实 Codex CLI + 隔离目录 + 本地假上游：
  识别默认字段，新建子代理使用指定模型与思考强度；
  主线程配置不变、已有子线程不被改写。禁止真实计费请求。
- [x] P7：代码自查、修复发现的问题、必要测试与构建、同步文档、提交本轮文件。

## 验收

1. 普通用户不配置该功能，原有接入行为与用户 agents 配置保持不变。
2. 固定中转子模型的请求进入该中转，出站 ID 与凭据正确，不走官方账号。
3. 思考强度以目标模型能力为准，未设置使用 Codex 的继承/默认规则。
4. 继承、恢复原配置、关闭接入各自行为明确；首次恢复点不随普通重应用移动。
5. 配置更新使用已有原子写入、备份、锁与 journal，不另建写配置通道。
6. 不存在的模型或跨模式不兼容的设置有明确反馈，不偷偷选替代模型。
7. 不读取真实 token，不改真实配置、不消耗额度；真实客户端测试仍使用隔离夹具。

## 当前工作区边界

开始前 `src-tauri/src/core/router_proxy/aggregate_tests.rs` 已有未提交的聚合测试改动。
本轮不修改或提交该文件。
结束时另有并行会话新建的
`docs/aggregation-desktop-session-affinity-review-2026-10-07.md`，同样保留并排除出本轮提交。

## 验收结果

2026-10-07：

- `pnpm test:rust`：1029 lib + 14 Desktop 集成 + 20 工作流通过；
  常规测试中的真实环境用例保持 ignored。
- 显式运行 `real_codex_spawns_saved_relay_child_without_changing_parent`：
  真实 CLI `0.162.0-alpha.2`，分别验证免登录中转主线程和合成 Free
  官方主线程，子代理均进入指定中转、带 xhigh 与正确凭据。
  修改默认值后冷恢复旧子线程仍保留其原模型与思考强度。
  假上游只生成一次 `spawn_agent` 及固定文本，推理请求全部本地回环；
  官方账号/服务发现的非本地连接被拒绝代理阻断。未读取真实账号凭据。
- `pnpm test:frontend`、子代理配置专项、`pnpm build`、`git diff --check` 通过。
  深浅主题和 420px 窄屏检查通过，长模型名截断并可悬停查看。
- 自查修复 Router/Native 的恢复短路、外部 provider 接管后的残留别名、
  继承模式的档位校验、保存后读回失败、已有默认值无改动时的保存入口。
  旧 journal 和失败/中断写入均有回归。
- 新建子代理配置与旧子线程恢复已验证；自定义角色/显式 spawn 仍遵循
  官方优先级。`multi_agent_v2`、Ultra 与旧子代理自动跟随不在此交付中。

使用说明：[codex-subagent-model-settings.md](../docs/codex-subagent-model-settings.md)。

## 来源

- [官方子代理说明](https://learn.chatgpt.com/docs/agent-configuration/subagents)。
- [官方配置参考](https://learn.chatgpt.com/docs/config-file/config-reference)。
- [前期核查](../docs/codex-routing-and-context-review-2026-10-07.md)。
