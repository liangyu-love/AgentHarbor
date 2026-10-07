# Codex 路由与上下文复查（2026-10-07）

本轮核对 Codex 官方内置模型登记、当前账号目录、models.dev 快照，
以及官方／中转在同一会话中切换时的配置与请求边界。
按最终决定，保留现有推理档位解析，不新增 Ultra 兼容层。

## 官方模型与上下文

来源：

- [Codex 官方模型登记](https://github.com/openai/codex/blob/main/codex-rs/models-manager/models.json)
  （2026-10-07 读取，源内容 SHA-256：
  `943ca7fe1d19ed019054158303f0dbbd80b3bef43b3aa425a71aa7cb3fb52c2b`）。
- [官方模型与 Ultra 说明](https://learn.chatgpt.com/docs/models)。
- [Sol API 规格](https://developers.openai.com/api/docs/models/gpt-6.1-sol)。
- 项目已提交的 `src-tauri/assets/models-dev.snapshot.json`。

官方 Codex 登记与 API 规格描述不同的值，不能相互覆盖：

| 模型 | 官方 Codex 内置上下文 | 官方 Codex 内置菜单最高选项 | 项目 models.dev 最大上下文 |
| --- | ---: | --- | ---: |
| GPT-6 Astra、GPT-6.1 Sol | 272,000 | Ultra | 1,050,000 |
| GPT-6 Sol、GPT-5.6 Sol、GPT-5.6 Terra | 272,000 | Ultra | 1,050,000 |
| GPT-6 Luna、GPT-5.6 Luna | 272,000 | Max，无 Ultra | 1,050,000 |
| GPT-5.5 | 272,000 | xhigh | 1,050,000 |
| codex-auto-review | 272,000 | Max，无 Ultra | 未据此推导 |
| gpt-daybreak-blue-latest、gpt-daybreak-red-latest | 272,000／372,000 | Ultra | 未据此推导 |

最后两行仅用于核对完整登记文件，不作为账号模型可见性或可用性的依据。
未列于当前 Codex 内置登记的历史/API 模型继续使用现有解析来源：
项目快照中 GPT-5.4/Pro 为 1,050,000，GPT-5.3-Codex、
GPT-5.4 Mini/Nano 等为 400,000，Spark 为 128,000；
没有把所有 OpenAI 模型统一改成 1M 或 272K。

Ultra 是 Codex 的多代理模式，不是额外的 API `reasoning.effort=ultra`。
有 Max 不等于有 Ultra。CLI 返回的模型选项、桌面端菜单显示、
上游实际支持，是三个需要分别验证的条件；本轮不修改桌面端套餐控制。

## 272K 快捷覆盖

中转模型编辑器的上下文输入旁增加 `272K` 按钮，复用
`models[].overrides.contextWindow`，仅将当前模型设为 `272000`。
仍需保存；不改输入/输出覆盖、模型 ID、协议、其它模型或压缩策略。
`自动` 清除此字段，重新读取原有解析来源，包括目录中的 1M/1.05M。

原有预算裁剪与百分比压缩继续生效：在有效预算为 272,000 且使用
全局 90% 时，压缩阈值为 244,800。Codex 若再预留 5% 输入余量，
客户端显示的可用窗口为 258,400；这是原有客户端行为。

272K 同时是当前 Sol API 的长上下文计价分界，但本功能不保证中转价格；
中转计费仍按所选渠道的价格配置与实际用量记录。

## 路由修复边界

- 官方菜单只读取当前账号目录及其 last-good 回退，保留原始隐藏标记；
  历史路由目录仅供能力参考，不把旧官方模型重新加入账号菜单。
- 智能路由刷新、修复配置时保留仍然有效的官方／中转模型选择。
  被移除的模型才回到配置默认值。
- 显式选择聚合目标仍更新默认模型，避免保留旧选择覆盖用户操作。
- 请求由本次模型 ID 决定出口；每次替换对应渠道凭据并保留会话历史。
  已采用路由的会话不因模型切换改变其 provider。
- 旧官方会话仍需原有的、带预览与备份的路由接管流程。
  单独修改全局配置不能改变旧会话记录的 provider。

## 验证方法与范围

使用本机 Codex CLI `0.162.0-alpha.2`，账号凭据、会话和上游均为隔离夹具。
真实客户端进程验证 Free 账号可见性，以及同一会话内官方 → A → B →
官方 → A → B → 官方的请求出口、身份隔离、xhigh/Max 和冷启动恢复。
另一用例复现旧官方会话将中转 ID 发往旧出口，并验证接管后连续切换。
官方出口夹具根据真实客户端的路由元数据处理 zstd 请求，避免将压缩正文
误当空 JSON、导致本应拒绝的请求被假上游放行。

前端覆盖 272K 的保存、重新打开、清除后恢复自动值，以及宽屏／窄屏布局；
后端覆盖 Router/Native 两种投影、压缩重算和其它模型/覆盖字段保持不变。
这些测试不向真实计费端点发请求，也不验证桌面端订阅菜单或实际多代理任务。

最终保留代码通过 `pnpm test:rust`（1019 lib、14 Desktop 集成、
20 本地工作流）、9 条显式真实 Codex CLI 回环用例，以及生产 Router
上的 Free 同线程切换用例。`pnpm test:frontend`、`pnpm build` 和
`git diff --check` 通过。

## 子代理路由记录复查

用户提供的原始记录为
[`codex-subagent-routing-findings.md`](../codex-subagent-routing-findings.md)。
本轮核查源码与配置契约，没有重新统计记录中的 49 个线程，也没有消费真实额度。

“主会话切到中转，已有子代理仍使用旧官方模型”符合当前实现：

- Codex 在创建子代理时复制当时的模型与配置；普通消息/后续任务继续使用
  已有子线程，不会因主线程下一轮改模型而同步修改子线程。
- 项目 `router_proxy.rs` 的 `/codex/v1` 出口以本次模型 ID 分流：
  `provider::model` 进入对应中转，裸 ID 进入官方。
  当前没有把子代理模型自动改写为父线程通道的专门逻辑。
- Codex 的 agent controller 使用主线程 ID 作为会话身份，子线程有独立
  thread ID；`collab_spawn` 用于标识协作子代理。请求还包含父线程元数据。
  主线程 HTTP `session-id` 可以被 prompt cache key 覆盖，且旧版本身份
  行为不同；后续若实现跟随策略，不能仅凭该 HTTP 头猜测父子关系。

因此这不是“先耗尽官方额度，再自动切到中转”的故障切换。
模型切换修复与 272K 快捷覆盖没有改变上述子代理生命周期。

### 已实现的子代理配置入口

官方已有这两个键，可直接复用，不需要新增协议或推理档位：

```toml
[agents]
default_subagent_model = "provider-id::gpt-6-luna"
default_subagent_reasoning_effort = "xhigh"
```

此处模型 ID 为示意，实际应从当前接入模式的模型目录选择。
Codex 接入卡片已增加“子代理默认配置”，包含模型与思考强度、保存、
恢复原配置。未配置时不接管已有字段；接入关闭时只保存，
启用时通过原有配置仓库和 journal 应用。实现保持以下约束：

1. 仅修改上述受管键，保留 `agents` 中其它配置、自定义角色、主线程模型
   与 provider；选择“继承”清除对应默认键，恢复原配置及关闭接入
   还原首次接管值，并保留外部后来改动的字段。
2. 复用当前模型菜单与能力解析；固定中转保留命名空间，聚合使用聚合入口
   与公开模型 ID。被删除或停用的目标不能静默回落到官方。
3. 遵守 Codex 优先级：显式 spawn 参数覆盖全局默认；自定义角色文件若
   设了模型/思考强度，会覆盖此前解析值。默认配置不能承诺强制所有子代理。
4. 单独验证新建、已存在、冷恢复的子代理；配置默认值只影响之后创建的
   子代理，已有子代理需主动切换或关闭重建。

如需旧子代理也自动跟随主会话通道，应另设可选路由策略，分别处理会话
隔离、主线程连续切换、显式子模型、模型可用性和实际出口诊断。
这不能通过一个默认模型键实现，也不能无条件把所有裸模型请求改成中转。

原始笔记末尾引用的 `openai/codex#37858`、`#37859` 本轮查询返回 404，
不能据此确认“Ultra 服务端多代理只支持 ChatGPT 后端”的完整结论。
本地子代理模型配置与服务端协作消息/Ultra 支持需要分别验证。
本轮新增的 Ultra 专用投影、登记快照与生成脚本均已撤回，
原有 `model_metadata`、推理档位解析与会话兼容逻辑未改动。

实现与验收见 [实施 TODO](../plan/codex-subagent-model-execution-todo.md)、
[使用说明](codex-subagent-model-settings.md)。本轮新增真实 Codex CLI
回环用例，脚本化假上游仅返回 `spawn_agent` 和固定文本，
覆盖指定中转子模型、思考强度、主线程保持及默认变更后的旧子线程恢复。
该用例关闭 `multi_agent_v2`，验证本地子代理契约，不把结果扩展为
服务端多代理或 Ultra 的支持声明。

参考：

- [官方子代理说明](https://learn.chatgpt.com/docs/agent-configuration/subagents)。
- [官方配置参考](https://learn.chatgpt.com/docs/config-file/config-reference)。
- [子代理创建配置](https://github.com/openai/codex/blob/main/codex-rs/core/src/agent/child_config.rs)。
- [会话身份与恢复](https://github.com/openai/codex/blob/main/codex-rs/core/src/session/session.rs)。
- [请求会话身份投影](https://github.com/openai/codex/blob/main/codex-rs/core/src/client.rs)。
