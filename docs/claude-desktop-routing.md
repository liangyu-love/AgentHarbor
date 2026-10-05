# Claude Desktop 路由模式

日期：2026-10-05。基础路由提交：`ea7cb17`，实施起点：`596c856`。Windows 安装包和自动化验证已提供；用户截图提供了 Chat 普通多轮的部分真机证据，其余真实 Desktop 路由验收见 [验收记录](claude-desktop-routing-acceptance-record.md)。本轮 Fast/1M 更新的 Rust、前端和构建均已通过，产物与隔离启动结果随交付目录保存。

## 使用已有中转

1. 打开供应商页的 Claude Desktop 配置，选择「路由」。
2. 给需要的 Sonnet、Opus、Haiku 角色选择已有中转卡片及其中一个模型。不同角色可以使用不同卡片，空角色保持未配置。
3. 模型有据的生效上下文达到 1,000,000 token 时，可单击勾选「额外 1M 选项」；初始关闭。Messages、Chat、Responses 路由使用同一能力判断。
4. 查看下方配置文件预览。预览可在 Desktop 运行时执行；实际应用前需完全退出 Desktop。
5. 应用后启动或重启 Desktop。配置菜单保存角色 alias，网关在请求时映射为实际模型。

无需重新填写中转地址或 Key。路由只引用卡片；不会启用原本禁用的卡片，也不会修改其默认模型、协议、全局默认供应商或 Codex 总开关。卡片必须启用，有有效推理 Key，目标模型必须存在且已纳入模型列表。手工默认模型仅允许精确 ID，不按模型族名猜测。

## 请求路径和模型身份

```text
Desktop 角色 alias
  → 127.0.0.1:<已绑定端口>/claude-desktop/v1/messages
  → 本地 Desktop token
  → 所选 Provider + 实际模型
  → Messages / Chat / Responses 上游
```

`/claude-desktop/v1/models` 和 `/claude-desktop/v1/messages` 使用独立 Desktop token。普通路由 token、Desktop token 不能跨入口使用。Desktop profile 不保存上游推理 Key；新请求读取已有卡片的当前推理 Key 和 URL。修改卡片凭据后无需为路由重新写 Desktop 配置。

首版 Sonnet / Opus / Haiku 使用固定安全 alias。空档、未知 alias、目标模型不可用会明确拒绝，不回落到另一档，也不触发现有 Codex 自动故障切换。Fable 仅在版本策略认可时提供，尚无真机验收证据。

勾选额外 1M 后，profile 仍是一条模型配置，但 Desktop 会生成“普通”和“1M context window”两项，并使用同一个展示名。这是 `supports1m: true` 的菜单契约，直连和路由相同。`prefer1m` 只改变默认选择，`[1m]` 配置简写也会归一化为该字段；当前核对的 Desktop 2.19675.0 没有“仅显示 1M 项”的配置字段。只需要一项时关闭额外 1M；这不改变上游模型能力，但不能据此保证 Desktop 普通项的本地上下文预算为 1M。

关闭后，旧缓存 `[1m]` 后缀及 1M beta header 都不能绕过路由开关。用户截图已确认路由开启时会生成成对菜单项；实际百万 token 请求和客户端本地预算仍待真机核对。

1M 判断复用模型能力解析结果，用户覆盖、卡片设置、上游声明及适用目录引用均保留来源；无据的默认上下文不能启用。已保存的 1M 选择失去能力后仍显示，需关闭该选项或重新选模型后再预览。Chat/Responses 转换只保留实际模型、消息与现有请求参数，不编造 1M 参数，也不把 Claude beta header 发给这两类上游。普通客户端路由原有的 1M 协议边界不随 Desktop 规则改变。

Fast 型号优先使用完整型号的目录记录或显式 `catalogRef`。完整匹配缺失时，末尾 `-fast`、`_fast`、`@fast`、`:fast` 可参考同版本基础型号的上下文上限；请求型号与展示名保持原样，不借用基础型号的价格、输入/输出上限或思考字段。详见 [模型能力解析说明](models-dev.md)。

诊断和用量明细可按 Claude Desktop 来源筛选，显示「请求 alias → 真实型号」。费用按真实 Provider/model 的价格快照计算，零价、未知价格、缓存读取和写入分别保留。旧用量记录没有来源时显示「未记录」。

## 预览、恢复和退出

直连和路由 profile 均包含 `coworkEgressAllowedHosts: ["*"]`，允许工具访问公网。旧版省略这个字段时，路由模式的工具白名单只有推理主机 `127.0.0.1`，WebFetch 会报 `cowork-egress-blocked`。升级后重新应用配置并重启 Desktop；该设置随 profile 事务回滚和恢复，不改外部配置或管理员策略。WebSearch 属于另一条上游服务链路，Command 的 dsh 搜索适配器不会自动接入 Desktop；见 [网页访问与搜索](claude-desktop-web-access.md)。

路由预览先取得实际监听端口，保留短期引用。取消、关闭配置窗口或票据过期会释放引用；其他客户端仍需要路由时，共享服务继续运行。监听重新绑定、后端重启或关键选项变化后需要重新预览。

配置应用沿用既有四文件事务。普通重新应用和直连/路由切换保留首次接管基线；只有显式重新接管才替换还原点。路由模式不能直接「释放管理」，应先恢复接管前配置、切换官方或切换直连。

关闭 AgentHarbor 窗口继续留在托盘。完整退出或更新重启时，若 Desktop 正在运行，可选择保持后台，或停止 Desktop、恢复接管前配置后继续退出/重启。停止或恢复失败会保留程序和网关，供重试。更新安装后选择保持后台，稍后重启不会重复下载或安装。

崩溃后重新启动会先检查事务，再恢复固定端口和本地 token。若端口被占，卡片显示「网关未就绪」，保存的配置仍在；释放原端口后可恢复。此时不会改写运行中的 Desktop。

## 验证范围

上一轮已用隔离文件、本地 HTTP 服务和 Playwright 验证三协议非流/SSE、工具回传、多轮、鉴权隔离、模型映射、历史隔离、费用身份、退出协调和窄窗显示。本轮新增 Fast 上下文引用与三协议 1M 规则，最终自动化结果以验收记录为准。实现会话没有修改真实 Desktop 文件、停止真实客户端或发送真实计费请求。

用户截图中，Desktop/Cowork 的 Command Code 目标完成普通至少两轮；请求诊断显示 `claude-sonnet-4-6` → `deepseek/deepseek-v4.1-flash`、`chatCompletions`、HTTP 200，上游回显与出站一致。完整版本/安装渠道和逐次抓包尚未补齐。角色菜单/profile、Messages/Responses 对话、逐段 SSE、工具、三协议 1M 和完整退出恢复仍需分别记录。Cowork 界面中的普通回答不代表其托管工具通过；macOS/Linux、Fable、Cowork 工具、托管工具和百万 token 压力测试未完成真机验收。
