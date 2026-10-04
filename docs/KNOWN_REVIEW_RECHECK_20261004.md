# 修复提交复查与补修

日期：2026-10-04。审查基线：本地 `main` 的 `5a6c5af`（`fix(router): harden translation and native configuration`）。本记录补充前一轮 `KNOWN_REVIEW_FIXES_20261004.md`，不改写其历史验证结果。

## 确认并修复的问题

| 级别 | 问题 | 修复后的行为 |
| --- | --- | --- |
| P1 | 默认 `functions` 命名空间的历史和强制工具选择被改为未定义的哈希名称 | 顶层 function 与 `functions` 组共享普通函数身份；定义、历史和工具选择一致，重复定义明确拒绝。 |
| P2 | namespace 工具的哈希名称丢失语义，缺少描述时工具难以区分 | 转译描述始终保留 `Original tool: namespace.name`，并保留组和工具描述；非法描述类型明确拒绝。 |
| P2 | Native 密钥轮换清理掉 `env_http_headers` 仍引用的凭据 | 引用扫描同时识别 `env_key` 和请求头变量，覆盖 Native 和外部 provider；最后一个引用移除后才回收专属凭据。 |
| P2 | Windows 等价路径被误判为孤儿目录文件，当前模型目录被删除 | 按路径对象及规范化文件身份保护当前目录，兼容斜杠和大小写差异。 |
| P1 | 绑定过期且关闭 failover 后，历史续轮可能发送到原渠道 | 当前 store 的进程内 failover 使用标记在监听器重启后保留；未知或过期的已识别续轮持续返回 409。关闭后的新普通会话建立普通绑定，仍可正常续轮；始终关闭 failover 的普通用法保持兼容。 |
| P2 | 被拒绝的新请求或无 body 子资源清除、创建会话绑定 | 路径、请求体及工具校验成功后才更新推理绑定；读取、删除、取消等资源操作沿用可定位的既有渠道，不创建或改写绑定，模型目录独立选路。子资源 body 原样转发，避免空取消请求被当作推理 JSON 重写。`responses/compact` 仅读取既有渠道绑定，关闭 failover 时仍拒绝需 failover 的压缩续轮。 |

实现涉及 `codex_native.rs`、`protocol/tool_namespaces.rs`、`router_proxy.rs`、`router_proxy/failover.rs`；HTTP 回归位于 `router_proxy/failover_tests.rs`。未修改前端和文案。

## 验证结果

- 完整 Rust 回归：**628 通过，10 个 opt-in 测试跳过**。
- `local_workflows`：**9 通过**。
- 已安装的 **Codex CLI 0.159.2**：**9 项真实 CLI 隔离测试通过**。覆盖 Router 鉴权、官方账户保留、Native 凭据与模式切换、线程续接、Chat/Anthropic 流式、结构化输出字段及确定的格式错误不重试。
- 新增回归覆盖默认 namespace 的 Chat/Anthropic 定义、历史和工具选择一致性；Native 两项清理问题均在旧实现上实际复现失败。
- HTTP 回归覆盖关闭开关、非法请求、过期记录已被清理后的重复续轮、同 store 监听器重启及 compact 渠道选择；从原 A 路径读取、删除、取消、列举响应输入项实际抵达绑定的 B，且取消 body 保持原样、模型目录不随绑定切换。
- `git diff --check` 通过；最终交叉复查未发现新增阻断问题。

真实 CLI 测试使用临时 Codex home、合成凭据及 localhost mock 上游，没有修改真实用户配置，也没有调用付费上游。完整 Rust 与真实 CLI 日志分别保存在忽略目录 `.runtime-test/recheck-fixes-rust.log`、`.runtime-test/recheck-fixes-real-codex.log`。

## 仍保留的边界

- failover 绑定和使用标记仍是进程内状态，不跨应用进程重启保存；同 store 的监听器停止/启动与应用重启是不同场景。
- 缺少可识别会话标识的请求仍不能保证渠道绑定。绑定空闲 24 小时过期后拒绝旧续轮，不承诺恢复旧渠道。
- 本轮子资源修复恢复 Native `/p/{id}/v1` 和 `/v1` 在空 model 会话 key 下的已有绑定查询，没有新增 response ID 到渠道的索引；`/codex/v1` 无 model 子资源及创建请求带 model 而后续资源无 model 的定位限制保持原状。
- 真实 CLI 测试使用本地 mock，不证明付费上游的计费、实际 schema 遵循或 hosted 搜索兼容；未运行 `live_codex_switches_configured_relays`。
- 远端压缩、加密推理历史、自动压缩触发实验及新旧版本 stream 分帧逐字节 harness 等前轮列出的限制保持原状。
