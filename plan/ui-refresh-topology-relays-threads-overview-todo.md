# UI 重构：路由拓扑 / 中转模型 / 线程管理 / 首页（2026-10-10）

承接 `codex-routing-openai-base-url-todo.md`。用户反馈：拓扑连线逻辑不对、OpenCode/Pi 图标换官方、中转卡片“当前路由”只标一个、
同 URL 叠卡翻页继续优化且与单卡等高、中转页残留旧风格；线程管理与首页整体重构，“抛弃原有 UI 固化”。

## 步骤

1. [x] 拓扑连线按实际使用点亮
   - `use-routing-links.ts` 计算每个客户端的实际去向（拓扑和首页共用）：Codex 中转注入 → 官方（非免账号）+ 中转；
     选了聚合 → 只走聚合；Claude Code / OpenCode / Pi 按 `modelSelection` 的 provider/aggregate 目标；Claude Desktop 按 `routeViews`。
   - Hub → 目的地只点亮被路由客户端用到的；悬停客户端只高亮它自己的整条链路；客户端副标题显示“路由 → 官方 + 中转”。
2. [x] OpenCode、Pi 换官方图标（opencode.ai favicon、pi.dev favicon），更新 `src/assets/client-marks/README.md` 署名。
3. [x] 中转卡片状态（`use-relay-consumers.ts`）
   - 路由模式下所有已启用、有密钥和模型的中转都标“已注入 Codex”，不再只给 `activeProviderId` 一张标“当前路由”；
     Codex 选了聚合时不标注入；原生直连模式只标唯一的“Codex 直连”。其他客户端用到的中转显示对应客户端图标。
4. [x] 叠卡翻页重做：每个卡位都预留顶部 12px，叠层只在这块区域里露出，叠卡与单卡同高；
   成员切换在卡头（左右按钮 + 成员名 + 分页点），翻页动画改为“从叠层升起 / 侧滑离开”。
5. [x] 中转模型页重构：顶部概览（站点数、已启用环形比例、Codex 注入数、连通性条、正在使用的客户端）、
   搜索框 + 站点筛选、新卡片（站点色字母头像、注入流光条、余额/模型双格），去掉 AuroraCard 金色等旧样式。
6. [x] 线程标题与修复
   - 后端：Codex 标题优先 `threads.name` → `session_index.jsonl` 的 `thread_name` → 清洗后的首条消息（去掉附件清单和注入上下文）；
     Claude Code 优先 `custom-title` → `ai-title` → `summary` → 首条消息；新增必填 `preview`（首条消息摘要，旧摘要缓存因此自动重建）。
   - 列表载荷带 `configuredProvider`；只有存储在其他 provider 下（Codex 中不可见）的线程显示“其他 provider”徽标与修复按钮，
     页面顶部提示数量并可“只看这些会话”。
7. [x] 线程管理 UI 重构：检索台（标题、来源切换、`/` 聚焦的大搜索框、筛选/排序/视图一行、统计条）、
   客户端头像 + 状态灯的会话行与网格卡，去掉输入/输出比例条；详情面板显示“首条消息”摘要；窄行隐藏次要操作。
8. [x] 首页（仪表盘）重构：渐变问候 + 路由轨道图（客户端环绕 AgentHarbor，经路由的连线流动）、今日 Token 折线、
   当前账号额度环、中转启用比例、会话数；最近会话、核心能力、模块入口、本地数据健康；动效尊重 reduced-motion。
9. [x] 回归：tsc、vite build、前端回归 197 项（新增 `scripts/test-ui-refresh.mjs`）、cargo test（lib 1140 + 集成 14/20）。
