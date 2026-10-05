# 外部审查复核与修复验收（2026-10-05）

## 范围与结论

基线为本地 `main` 的 `abd9ace`，复核 Skills 恢复、Desktop 进程控制、Messages 路由与托管搜索。有效问题已修复；`max_uses` 的“会话累计预算”前提不成立，不能据此扣减历史调用。未配置档位仍保持用户的显式选择，增加提交前提示，不自动回填。

本次使用 `code-review-expert` 检查当前差异，并通过 Context7 核对 Anthropic / OpenAI 官方字段定义。没有修改真实 Desktop 配置、终止真实 Desktop 进程或调用付费推理端点。

## P1 逐条核对

| 外部条目 | 判定 | 修复与证据 |
| --- | --- | --- |
| 坏 `metadata.json` 阻塞所有备份 | 成立 | 扫描返回正常条目和逐目录警告；截断 JSON、目录形态、非 UTF-8 内容均跳过该备份，保留文件和其余恢复入口。临时目录集成测试验证健康备份仍能恢复。 |
| 非 UTF-8 `SKILL.md` 阻塞 catalog | 成立 | `InvalidData` 隔离到该技能；启用区与禁用区的警告都进入原有 `CoreEnvelope.warnings`，UI 可打开问题目录。根目录或目录权限错误仍整体失败。 |
| 无窗口残留等待完整 20 秒 | 成立；建议仅停止重发仍不足以降低延迟 | 无窗口进程给约 2 秒建窗机会；当前仍在运行的进程全部无窗口时进入强制阶段。已成功收到关闭请求的进程保留正常优雅期，投递错误单独重试，不等同于无窗口。补 stop 层测试，原 signal 测试改为准确名称。 |
| `max_uses` 为会话累计预算 | 不成立；超限反馈确有语义差异 | Anthropic 定义是单次 API 请求，Responses 是单次 response；保留本轮限额映射。不会扣历史次数或凭“达到限额”伪造工具错误。差异在使用文档中明确记录。 |
| `Context::new` 重做完整请求转换 | 成立 | 改为只扫描 `tools`，对搜索声明调用 `web_search::prepare`。测试缺少 `max_tokens` 的上下文仍能保存搜索名称和恢复原始搜索输出，证明不再依赖完整翻译。未声称测得 CPU 降幅。 |
| Responses 悬空客户端工具调用 | 成立 | Messages → Responses 在 canonical `input` 校验调用/结果配对、非空唯一 ID、孤儿或重复结果和末尾未回答调用。保留 Responses 的交错消息与并行结果顺序，不套用 Chat 的连续排列要求。原生 Responses 输入不走此校验。 |

### 搜索限额的官方依据

- [Anthropic `WebSearchTool20250305Param.max_uses`](https://github.com/anthropics/anthropic-sdk-python/blob/main/src/anthropic/types/web_search_tool_20250305_param.py)：`Maximum number of times the tool can be used in the API request.`
- [OpenAI Responses 创建参数](https://github.com/openai/openai-python/blob/main/src/openai/types/responses/response_create_params.py)：`max_tool_calls` 限制一轮 response 的所有内置工具调用，超过部分被忽略。

本转换当前只接入搜索这一种内置工具，普通客户端 function 不争用该内置限额。因此“历史已用 18 次，本轮仍允许 20 次”本身符合按请求计数的定义。Responses 忽略额外调用与 Anthropic 返回 `max_uses_exceeded` 工具错误的表现不能完全等同；本地仅处理实际收到的结果，不编造上游未返回的调用或错误。

## 其余条目

| 外部条目 | 判定与处理 |
| --- | --- |
| 冲突禁用副本路径不可见 | 成立；新增 `conflictDirectoryPath`，两份目录都可打开核对，解冲突后清除此字段。 |
| 后续失败留下无法关联的备份 | 成立；源文件摘要校验提前；备份完成后的覆盖、移除、恢复和解冲突失败携带备份 ID / 路径，失败后也刷新备份列表。失败注入测试确认原技能保留且错误可关联备份。 |
| 确认键无 busy 门 | 成立；移除、删备份、解冲突的确认键均受技能操作状态约束，后续刷新完成前不重新放行。 |
| 扫描中条目消失导致整体失败 | 成立；跳过目录项及子目录的 `NotFound` 竞态。根目录和其他扫描错误仍显式失败。 |
| `open_page` / `find_in_page` URL 漏消毒 | 成立；复用 HTTP(S)、主机和控制字符校验，`find_in_page.pattern` 必须为字符串；覆盖 `javascript:`、`file:` 和控制字符回归。 |
| 终态帧两次 `enrich_output` | 成立；共享一次丰富后的输出供响应体和 SSE 生成使用。 |
| 两条转换流超时分支未覆盖 | 成立；三种上游协议分别驱动 observed、adapted-native、translated 路径，检查超时错误与不得出现成功终止帧。客户端 Responses 的失败事件是 `response.failed`。 |
| 非门控流无 keepalive | 成立；三条路径每静默 15 秒发 SSE 注释。上游响应头返回后生效，不进入用量观察，不重置 900 秒上游读取超时；EOF / 错误 / 取消保留原终止行为。覆盖保活、源数据顺序、EOF 和读取超时。 |
| 安装发现超时让重启按钮消失 | 成立；Windows MSIX 发现预算增加到 10 秒，发现失败保留重试入口。明确未安装、隔离或覆盖路径仍不可重启。 |
| 强制阶段枚举失败缺少手动退出指引 | 成立；统一返回 `processStopTimeout`，附原错误信息及重试/手动操作提示，查询失败不授权写配置。 |
| `GetLastError` 时序缺注释 | 成立；补充必须紧邻失败调用读取的注释；新增 signal 诊断同样先捕获错误码，再检查窗口或退出状态。 |
| signal 失败诊断被吞掉 | 成立；按 PID 记录首次优雅投递失败，记录强制阶段失败；Windows 错误携带具体操作名与 Win32 错误码。不妨碍其余已核实进程的清理。 |
| Skills 文档仍描述手动解冲突 | 成立；文档与两份目录入口、确认操作、备份和警告同步。 |
| 解冲突按钮立即执行且开关误导 | 成立；选定保留副本后确认；冲突卡片隐藏有歧义的启用开关。 |
| 未配置子 agent 档位缺提示 | 成立；路由表单显示缺少的角色和子 agent 请求影响，允许部分配置，不替用户猜目标。前端测试覆盖缺少 haiku 的提示。 |

## 验收

- Rust 以 `--test-threads=1` 运行：821 项 lib、11 项 Desktop 集成、20 项本地工作流通过，22 项显式运行用例 ignored。
- `pnpm test:frontend` 全套通过；新增缺档位断言后，Claude Desktop card 定向回归也通过。
- `pnpm build` 通过；仍有既有 Vite 大于 500 kB 的 chunk 提示。
- `git diff --check` 通过。

最终自审未发现新增阻断项。重点检查了单文件警告与根目录失败的边界、备份先完成再变更的顺序、前端确认门、转换专用工具配对、保活与上游超时的独立性、进程身份核验和强制阶段拒绝写入的条件。

初次并行 Rust 运行在既有余额辅助测试等待，改为串行完成验收；未修改余额实现。期间纠正了两条新增测试断言：客户端 Responses 的超时终态事件名，以及 Windows 等价路径的分隔符比较。最终代码和断言一并通过上述验证。

## 验证边界

损坏编码和 metadata 不需要真实环境，临时目录测试已复现并验证恢复。无窗口 stop 的分支用可控枚举/信号测试覆盖，Windows signal 另有隔离子进程测试；未计时真实 Desktop 的关闭耗时。

未向真实 OpenAI Responses 端点发送孤儿工具调用，未做真实 Desktop / macOS / Linux 验收。工具配对规则由转换契约和正反向测试验证，不宣称已复现真实端点的 400。

保活不能缩短尚未返回响应头的等待，也不会改变既定 900 秒读取超时。Command 独立 REST 搜索桥接仍未接入；此修复不会让不提供 hosted search 的 Chat 上游自动获得搜索服务。

相关说明：[Skills 启停](skills-activation.md)、[Desktop 进程控制](claude-desktop-process-control.md)、[Desktop 网页与搜索](claude-desktop-web-access.md)。
