# 2026-10-05 路由、Desktop 进程控制与 Skills 整改验收

基线：`main / 38ee957`。本轮依据外部审查逐条复核，并参考本地 CC Switch 搜索转换源码；实现后再次自审、修复并执行回归。

## 协议边界

Claude Desktop 始终使用本地 Anthropic Messages 接口。链路是：

```text
Desktop Messages 请求
    → 按已配置模型的协议转换请求
    → 上游 Messages / Responses / Chat Completions
    → 转回 Messages 响应及 SSE
    → Desktop
```

没有更改已保存的模型协议，没有将 Desktop 改成 OpenAI 客户端。Responses 同时是项目内部既有的转换中间表示；它不要求 Chat 中转额外提供 `/responses`。

## 2026-10-06 聚合与协议解耦增量

本轮在上述历史修复之上增加了独立的模型聚合入口。入口仍由客户端路径
决定：Responses、Messages 和 Desktop Messages 分别进入自己的协议边界；聚合
调度只选择成员，不把模型名称当成协议。`ModelTargetRef`、共享 resolver 和
`AgentConnectionExport` 让 Codex、Claude Code、Claude Desktop、OpenCode 及
其它已支持的 agent 消费同一个公开目标描述，而不复制中转地址、密钥或模型能力。

协议层新增 `WireAdapter` 的 Inbound/Outbound 边界，并让普通 provider 与聚合
attempt 共用 `forward` 的请求准备、鉴权、history/tool、stream 和 usage 处理。
同协议尽量保留原始 JSON/SSE；跨协议只在已验证的转换矩阵内编码，未知字段和事件
不会被无提示删除。聚合响应在确认上游身份后只投影协议定义的 `model` 字段为
`publicModelId`，诊断和账本仍保存真实 served model 与实际成员价格快照。

当前基础交付支持 Responses/Messages 聚合入口和 Responses/Chat/Messages 三种
成员出口，并明确处理资源归属、`count_tokens`、Responses `compact`、1M 容量准入、
有限 429/连接前重试及有界诊断。通用 `/aggregate/v1/chat/completions` 入口、
Chat→Responses/Messages 的公共 canonical 事件层、分帧性能基线和完整托管工具矩阵
属于 P7，当前保持 pending；只支持 Chat 的外部 agent 不应被导出为可消费的混合池模板。

本增量没有运行真实中转凭据、付费搜索或真实 Desktop/其它 agent 的全组合验收。
mock/隔离测试只能证明本地契约，不能替代上游协议能力结论。

## 已修复与实现

| 范围 | 修复及验收依据 |
| --- | --- |
| WebSearch 的本地 400 | Messages → Responses 识别托管搜索，不再把合法搜索声明误判为自定义工具缺少 schema。映射原生 `web_search`、强制选择、域名白名单、近似位置、次数上限和来源 include。 |
| 搜索响应 | 转回成对的 `server_tool_use` / `web_search_tool_result`，保留调用名称、实际次数、真实 URL、失败及超限原因。文本引用转换为链接，不伪造 Anthropic 加密索引。 |
| CC Switch 的晚到引用处理 | 来源优先取 `action.sources`，引用补齐标题，未归属 URL 归到最后一次成功且未超限的搜索。搜索正文暂存至结束帧，期间发送启始事件及 ping；8 MiB 正文上限、帧上限及缺失终态错误仍生效。普通对话保持增量输出。 |
| 后续搜索对话 | 搜索块纳入可见签名；有界缓存保留准确上游输出，作用域继续包含渠道、凭据及会话标识。缓存缺失时重建搜索查询和来源；失败且无 action 的搜索历史可回放。 |
| Haiku 子代理别名 | 先匹配保存的别名，再将受验证的官方版本/日期 ID 对应到显式配置的同档位路由；没有 Haiku 时不猜测切换到 Sonnet。 |
| 工具历史 | reasoning 排在 function call 后时归入原助手轮次，不插入隔断工具结果的消息。Messages → Chat 在本地拒绝缺失、重复、孤立或不连续的工具结果，不编造成功回复。 |
| 长思考与流式失败 | 读取静默超时改为 900 秒；超时及传输异常输出对应协议的错误事件，不发送成功结束帧。上游错误 SSE 提取 JSON 消息。 |
| 本地鉴权 | 网关令牌及相关鉴权比较使用 `subtle` 常量时间比较。 |
| Desktop 停止 | 单 PID 的优雅退出或强制终止失败不跳过其余 PID；优雅窗口内重试暂未建窗的进程，强制操作前重新枚举，最终验证实际停止状态。枚举失败继续阻止配置写入。 |
| Desktop 启动及发现 | 区分发现失败和未安装；失败作为可重试原因呈现。重启在停止之后重新发现安装路径，避免自动更新更换可执行文件导致旧路径匹配超时。Unix 已退出 PID 的信号竞态与未知状态分开处理。 |
| Windows 辅助进程 | 及时读取 ToolHelp 错误码；超时子进程的 capture 文件在回收后重试删除。删除未调用的 profile 辅助函数。 |
| Skills 扫描 | 目录、条目或读取失败显式报告，不伪装成空目录。区分缺失目录和错误；备份目录读取同样处理。 |
| Skills 移动及删除 | 在发布前解析 summary；通过原子重命名隐藏待删除目录，再重试清理。跨卷移动回滚保留原错误、回滚错误与恢复路径，避免半删除可见目录。恢复使用备份中的技能名。 |
| Skills 双份冲突 | UI 可以保留启用或禁用副本；另一份完整备份后撤下。所有变更在成功或失败后等待两份目录查询刷新；删除备份也参与忙碌锁。 |
| Skills 清理 | 仅清理工具拥有的 UUID stage/retired 目录；保留可能是唯一恢复源的 previous 目录，不遍历链接或删除其他隐藏目录。 |

## 复核修正与能力边界

- 两条原始 400 来自 AiMaMi 本地。错误文字不能证明第三方上游拒绝搜索；原 Messages → Responses 缺口已经修复。
- CC Switch 的 Responses 路径实施了托管搜索转换；其 Chat 路径把声明映射为 function，没有替 Desktop 执行 Command 搜索 REST 的桥接。这里采用前者的核心流程，不能靠后者保证实际搜索。
- 原生 Messages 透传仍要求上游有托管搜索能力。Chat Completions 无标准托管搜索等价物，继续返回准确的能力错误。Command `/alpha/web-search` 是独立服务，本轮没有引入其服务器端工具执行循环，不能宣称所有 DeepSeek/Command 渠道已经能搜索。
- 新版本程序化调用、非空域名黑名单、`response_inclusion` 等无法保持语义的设置明确拒绝；不删除调用者约束后假装支持。
- `max_tool_calls` 将上限传给 Responses 上游；响应侧将超限结果标记为错误。对不遵守次数上限的上游，响应转换无法撤销已经执行的调用或费用。
- 外部报告关于 cache 锁内计算签名的发现不成立：原代码已经在锁外完成。退出时 Background 行为是保留服务的既有产品语义，本轮不改。
- 外部报告中的 P0 级别有所夸大：停止失败会阻止后续写配置；Unix 竞态仅在进程于两次操作之间退出时发生，不是每次必现。行为缺陷仍按优先级修复。

## 自动化验证

- `pnpm test:rust`：MSVC，814 lib + 11 Desktop 工作流 + 18 本地工作流通过；22 个真机/辅助用例 ignored。
- 本轮较早的完整 `pnpm test:frontend` 通过；最后的 Skills 刷新调整后，以 `FRONTEND_QA_SCOPE=skills` 再运行 `pnpm test:frontend`，Desktop 配置契约十组与 Skills 浏览器回归通过。
- `pnpm build`：TypeScript 与生产构建通过；已有 chunk 大小提示仍存在。
- `git diff --check`：无空白错误。

新增重点用例：实际 localhost 网关的两种 Messages 路径、流式/非流式托管搜索、每一个字节分割点下的 SSE、晚到来源与仅引用来源的上游、强制选择与自定义搜索名称、搜索历史作用域、超限/失败/非法选项、危险 URL 与重复工具名、真实 HTTP 静默超时、停止信号失败及更新后重发现、Skills 双份选择/备份/扫描失败/暂存清理。

## 实测边界

本轮不触碰真实 Desktop 配置、进程和供应商密钥，未执行付费 live 搜索。Windows 进程控制的系统路径与可注入算法回归已运行；macOS/Linux 的新分支没有真机验证。模拟上游通过只能证明网关契约，不保证每个中转提供搜索服务。没有制作新 exe，旧构建不包含本轮修改。
