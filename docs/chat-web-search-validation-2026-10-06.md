# Chat 路由 WebSearch 修复与验收

## 问题与最终行为

用户在 Command Code 的 Chat 模型路由上收到 `400 this target protocol has no equivalent hosted tool`。错误来自本项目协议转换器：入站 Messages 的 WebSearch 声明没有对应的 Chat 托管工具，旧实现只支持原生 Messages 和 Responses 搜索，没有执行 Command 的独立搜索 REST API。

本次增加通用的 Messages → Chat 搜索执行器。模型按卡片既有协议、端点、凭据和网络模式请求；搜索服务独立选择，首个后端采用 Command 的 `POST /alpha/web-search`。其他 Chat 模型也能复用搜索账户，模型品牌不参与执行器分流。原生 Messages 与 Responses 保持原有搜索路径。

执行器把搜索声明替换为内部普通函数，执行模型生成的查询，把真实 URL、标题和摘要交回同一模型，最终对 Desktop 输出 Messages 搜索块与回答。搜索结果不是模型臆造；REST 调用不伪造推理用量，每轮模型调用独立落账。

搜索服务设置在“请求级路由诊断”中，可选“自动”或具体已配置卡片。自动先用当前 Command 卡片，否则使用唯一搜索卡片；多张候选必须明确选择。显式选中的卡片失效时保留设置并报不可用，防止改扣其他账户。

## 审查与修复

- 核对了 Command 插件的 REST 路径、请求体、Bearer 鉴权及版本头，并把搜索凭据和模型凭据分开。
- 域名白名单同时发送给搜索服务、按主机/子域边界过滤结果；禁止无等价支持的参数静默丢失。
- 修复草稿中搜索结果前后的消息顺序、混合客户端工具配对以及多轮私有推理回放；缓存按渠道、凭据、会话及 Desktop 配置作用域隔离。
- 修复后续关闭 WebSearch 后无法转换旧搜索历史的问题；历史转换不要求新的搜索账户，也不启用搜索工具。
- 修复上游 `choices[0].message` 为字符串/数组/数字时的 panic；畸形响应现在返回 502，并保留已实际发生的用量记录。
- 保留工具 schema 的原始出站编码，在回放缓存中保存解码后的消息，避免下一轮重复包裹 Gemini 工具参数。
- 共享输出预算、限制模型轮次与缓冲大小；搜索失败或次数用尽后移除内部搜索函数，避免强制搜索循环。
- 核对取消、错误及账本路径；断开流不会继续发起下一轮计费模型请求，错误响应正文不对外展示。

## 自动化结果

| 检查 | 结果 | 证据 |
| --- | --- | --- |
| 完整 Rust：lib + Desktop + 本地工作流 | 846 + 11 + 20 通过；22 ignored | `.runtime-test/hosted-chat-rust-full.log` |
| 最终搜索专项（含新增账户选择用例） | 37 通过 | `.runtime-test/hosted-chat-search-final.log` |
| Chat 执行器回归 | 14 通过：两个真实本地入口、非流式/SSE、摘要/来源、账本、连续对话与关闭搜索等 | `.runtime-test/hosted-chat-targeted.log` |
| 完整前端回归 | exit 0，含搜索服务选择、自动模式及失效显式选择 | `.runtime-test/hosted-chat-frontend.log` |
| TypeScript/Vite 生产构建 | exit 0 | `pnpm build`；发布构建日志随交付包保留 |
| diff 空白检查 | 通过 | `git diff --check` |

新增的搜索专项只使用 localhost mock、临时配置与测试凭据。生产搜索端点固定为 Command 官方 HTTPS 地址；localhost 替换只存在于 `cfg(test)` 构建。

## 实际复测与边界

运行新 EXE，保留当前模型卡片的协议设置。当前卡片为 Command 时可直接使用自动搜索；其他 Chat 模型通过“WebSearch 搜索服务”选择已配置的 Command 卡片。搜索设置不修改 Desktop 注入文件，无需为了这个设置重新应用模型档位。

自动化不等同于真实 Command 服务的付费搜索成功。尚未用真实凭据调用搜索、观察 Desktop 最终界面或验证安装升级。Chat 搜索内部采用非流式模型轮次，最终正文集中输出，期间有保活；原生 Messages 与 Responses 的搜索支持仍取决于上游。程序化搜索、近似位置等缺少等价后端能力的参数会明确拒绝。

本次未关闭用户运行中的旧版、未写入真实 Desktop 或客户端配置。新包的 SHA256、构建日志和隔离启动检查以交付目录中的 manifest 为准。
