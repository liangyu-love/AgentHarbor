# Pi 接入与中转注入快捷入口

日期：2026-10-09。范围：模型/中转/聚合配置；不接管 Pi 登录与默认模型设置。

## 实施与验收

- [x] 注册 Pi 客户端，定位 `~/.pi/agent/models.json`，支持 `PI_CODING_AGENT_DIR`、隔离根。
- [x] 多中转、多模型按实际协议生成独立供应商，复用现有上下文、输出、推理和价格解析；未知价格不冒充免费。
- [x] 支持 Chat、Responses、Messages 原生直连、来源独立的本地路由和聚合公共模型。
- [x] 配置预览脱敏、一次性版本校验、保存后重新应用、恢复基线、退出时撤回路由配置；保留外部供应商与未受管字段。
- [x] 密钥按 Pi 字面量规则转义，不执行导入的环境或 shell 表达式；不修改 auth.json/settings.json。
- [x] 前端添加 Pi 客户端配置、模型子集和聚合入口；路由诊断区分 Pi 来源。
- [x] 顶部图标快捷栏：Codex、Claude Code、Claude Desktop、OpenCode、Pi、聚合；展开、滚动定位并转移键盘焦点，兼容暗色和窄屏。
- [x] 临时根回归：三协议 URL/鉴权、模型子集、聚合、冲突/迟到预览、恢复、外部数据保留、退出消费者。
- [x] 本机 Pi CLI 只读目录加载与 localhost 模拟推理，验证实际配置可消费；不使用真实凭据或付费调用。
- [x] 自审、Rust/前端回归与构建，通过后提交，工作区干净。

## 设计约束

Pi 使用稳定的 `agentharbor-relay-<id>-<api>` 和 `agentharbor-aggregation-<api>` 供应商标识。按协议分组使认证方式和根地址明确，修改卡片默认协议不会移动其余模型。模型 ID 保持原始值；聚合使用公共模型 ID。Pi 通过 `/model` 选择和刷新模型，原生默认选择仍由 Pi 管理。

配置操作沿用 client_config 的字段归属、加密恢复记录与原子写入。供应商节点首次遇到外部同名内容时拒绝覆盖；修改受管模型数组按现有冲突流程处理，其他供应商保持独立。Pi 的 cost 仅为静态参考，实际渠道账本仍为权威；无法表达的条件价格和聚合可变价格不写成固定单价。

参考：[调研](../docs/pi-agent-integration-research-2026-10-09.md)、[Pi 模型配置](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/models.md)、cc-switch `pi_config` 的增量写入原则。

## 完成记录

Rust 全量 1147 + Desktop 14 + 工作流 20 通过；最后的手动档位修正另跑客户端配置 98 项通过。前端全量 199 条 PASS（包含 Desktop 静态 10 组），构建通过。安装的 Pi CLI 0.84.2 在临时根完成 7 组协议/模式用例和一次工具续轮，共 8 次 localhost 请求。Impeccable 机械检查无发现；亮暗/窄屏、键盘、持久折叠已检查。自审修正了未匹配模型的手动思考档位，以及两条旧生图夹具的站点与选择隔离问题。未执行真实渠道付费请求，未打包。详见 [使用与验证记录](../docs/pi-agent-integration.md)。
