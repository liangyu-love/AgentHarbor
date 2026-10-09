# Codex 关闭接入与终端代理修复验收

## 范围

修复关闭 Codex 路由、切换直连、退出 AgentHarbor 时的会话迁回顺序，以及终端代理保存后状态被旧查询覆盖、过长 NO_PROXY 导致取消失败的问题。

## Codex 接入

- 关闭接入由一个后端命令协调：停止 Codex 写入 → 重新枚举 → 校验目标与所有会话 → 迁回 → 撤销配置。原 Router 在迁回期间保留。
- 枚举覆盖普通 Router、聚合、Native 与历史接入 provider。初始没有已索引会话也不能跳过停止阶段，以便捕获退出时才写入的首条会话。
- Router 切换直连先准备 Native provider 与凭据，保持原 Router 选中，迁回后再切换。部分迁回后若刷新原 Router，会保留已被会话引用的 Native provider。
- 迁回复用既有单会话备份、journal、文件锁与 revision 校验。整批预检查只保留 ID 与 revision；发生错误时保留原路由，已完成的会话迁回不撤回。
- 失败后可以重试、取消，或明确选择“仍然关闭 / 仍然切换 / 仍然退出”。强制入口只跳过会话迁回，不删除会话及其备份；未迁回的会话可能需要重新接入或手动修复。
- 前端会话数量查询仅用于确认提示。即使 SQLite 不可读或不支持现有 schema，关闭操作仍交给后端判断并显示强制关闭入口，不会停在前端查询失败处。
- 正常退出使用同一迁回流程。异常退出兜底遇到受管会话、未完成恢复日志或仍在运行的 Codex writer 时保留原配置与 generation。
- 每次接入配置变更刷新会话路由概览；恢复成功但重新打开 Codex 失败时保留可见警告。

## 终端代理

- 启用时对补全 localhost、127.0.0.1、::1 之后的 NO_PROXY 检查 4096 字节上限，非法值在配置文件与环境变量写入前拒绝。
- 兼容旧版最多 4120 字节的所有权日志，恢复阶段不重新校验用户表单，因此旧日志及损坏的当前输入不会挡住取消。
- 取消按所有权恢复 HTTP_PROXY、HTTPS_PROXY、ALL_PROXY、NO_PROXY 的原值或原不存在状态；已被外部修改的变量保留，并返回保留清单。
- 保存成功先取消未完成的状态查询，再写入最新结果，避免旧查询把启用与恢复按钮覆盖掉。

## 验证

本轮仅使用临时会话、SQLite、配置目录与环境夹具；不停止真实 Codex、不修改用户代理变量、不向付费模型端点发请求。

- Rust 完整回归：1133 个库用例、14 个 Desktop 集成用例、20 个本地工作流用例通过；17 个库用例与 11 个 Desktop 真机用例按原设置忽略。
- 新增 13 个协调迁回用例：覆盖关闭成功、停止失败、全量预检查、有效/损坏 pending journal、显式强制关闭、Native/聚合/旧 provider、重新打开失败、首会话退出落盘、数据库目录/不支持 schema、异常退出 writer 保护及 Native 定义保留。
- Native 准备阶段新增 5 个回归：保留 Router 与接管基线、复用凭据、最终选择与还原、拒绝不合法目标/外部接管、写入失败清理。
- 终端代理覆盖补全后长度边界、原值恢复、旧版超长日志三种状态、外部修改保留；前端覆盖启用、地址更新、恢复后迟到查询不能覆盖保存结果。
- `pnpm test:frontend` 完整通过（192 组 PASS 记录），覆盖关闭与 Native 切换单命令、迁回失败、概览查询失败、强制操作取消/确认、路由概览缓存刷新、退出/更新器取消恢复。最后调整的退出标题、重试及“仍然重启”另跑 Desktop 定向回归通过。
- `pnpm build`、TypeScript 检查与 `git diff --check` 通过。构建仅有既有大于 500 kB chunk 提示。

本地日志：

- `.runtime-test/routing-close-proxy-rust-final.log`
- `.runtime-test/routing-close-proxy-frontend-final.log`
- `.runtime-test/routing-close-proxy-desktop-final.log`
- `.runtime-test/routing-close-proxy-build-final.log`

本轮没有打包安装程序，也未验证真实 Codex 桌面进程停止、重开或 Explorer 对新终端的环境广播；这些行为使用假进程或环境夹具验证。
