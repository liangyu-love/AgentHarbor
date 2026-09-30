# 官方重置卡与总体线程树（2026-09-30）

## 实现

- 用户已澄清“重置卡”指官方 ChatGPT/Codex 的可用重置次数，放在官方账号卡片，不接入中转余额。
- 借鉴 Magpie 的 wham 读取接口与失败降级思想，未复制 Go 实现。复用现有官方账号认证、身份校验、代理和刷新；仅发送 GET，不增加 consume/购买/兑换操作。
- `wham/usage` 的 `rate_limit_reset_credits.available_count` 提供次数；`wham/rate-limit-reset-credits` 的 credits 提供状态、获得时间、到期时间。详情失败保留已知次数，不将账号标记为登录失效；未知与 0 次分开。
- 卡片徽章支持悬停、点击固定、键盘打开、Escape/关闭/点外部关闭；浮层 Portal 不受账号卡片裁切。近 30 天获得记录仅来自详情实际返回的条目，不宣称完整历史。
- 项目 → 线程导航按完整 cwd 分组，同名工程展示路径；未知工程单独分组。全局/项目计数展示筛选命中数与总数，搜索与归档状态联动；线程可单独定位、返回项目/全部。
- 大项目首次仅展示 30 条子项，按需加载；右侧仍复用原虚拟列表。项目或线程刷新后消失时清理定位，批量选择不保留被隐藏的线程。
- 网格列数按实际容器宽度估算，不再错误地按整个窗口计算；窄窗口线程页按钮可换行。

## 实测

- **真实官方接口**：忽略测试 `live_official_reset_credits_read_only` 用显式指定的本机认证快照读取官方服务，成功返回 **2 次可用重置**、两张 available 卡的 `granted_at` 与 `expires_at`。详情字段包含 `credits`, `available_count`, `total_earned_count`, `immediate_reset_purchase_eligible`, `history_enabled`。不保存/输出 Token、邮箱、账号 ID、卡片 ID；验证调用前后认证文件字节完全一致。
- **Rust**：409 项单元通过、2 项默认忽略（含此只读 live 测试），9 项 local_workflows 集成通过；增加缺字段/0/非法 count、日期、非 Codex 卡过滤、解析原子性、缓存保存/重建/清理、GET 认证头回归。
- **浏览器**：真实 React 组件 + mock IPC，无外网。验证重置卡的数量/到期/近 30 天记录、未知/0/详情失败、悬停跨间隙、键盘焦点/关闭/刷新；100 条线程的分组/同名工程/按需展开/搜索/归档/定位/刷新/选择清理/虚拟化，1440/960/390 宽度和亮暗主题截图。
- 前端生产构建通过；Clippy 无错误，仍为 37 条既有告警。Impeccable 新组件扫描无主问题；审查清理了重复项目选择、重复详情状态字段、无关格式化改动，补齐线程跨项目移动后定位清理；没有新增 Router 或大型框架。

证据目录：`.runtime-test/account-thread-tree/`（忽略目录，不包含账号认证或原始官方响应）。

## 复现

```powershell
pnpm build
pnpm test:frontend
# 只运行本轮浏览器业务：
$env:FRONTEND_QA_SCOPE='account-thread-navigation'
pnpm test:frontend
Remove-Item Env:FRONTEND_QA_SCOPE

# 显式允许一次真实官方只读查询；不会刷新/改写凭据：
$env:AGENTHARBOR_TEST_ACCOUNT_AUTH_PATH='C:/path/to/auth.json'
cargo +stable-x86_64-pc-windows-msvc test --manifest-path src-tauri/Cargo.toml --lib live_official_reset_credits_read_only --locked -- --ignored --nocapture
```

## 边界

- wham 是官方客户端使用的私有接口，不是公开、稳定的 SDK 合同；未来字段变更按未知/详情不可用降级。
- 本次只验证当前账号的真实 available 卡；真实 redeemed/expired 状态与没有权限的账号通过 fixture 覆盖，未消耗卡片制造这些状态。
- 本轮不重做官方 ↔ 中转长对话切换、百万 token 压测，不修改真实会话/Router 配置；实现阶段未提交、打包或安装，后续按用户要求与线程详情统一提交。
