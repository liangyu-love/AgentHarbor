# UI 重构第三轮：去“AI 感”、注入页重排、侧边栏与其余页面（2026-10-10）

用户反馈：首页最近会话长标题撑破区域；“本地数据目录”只有 Codex；背景艺术字太挤；中转配置页配色与图标有 AI 感；
中转注入下方仍是旧排版、卡片套卡片不好看（可把 Codex 与 Claude 放在一起）；首页路由圈再调整；
剩余页面和侧边栏也要重构，不要被原有 UI 固化。

## 方向

- 去 AI 感：去掉紫 / 青渐变、发光、彩虹头像；只用强调色 + 中性色，靠排版、细线、留白和展示字体建立层次。
- 少套娃：外层是页面分区（标题 + 细线），只有真正的实体是卡片；卡片内部用浅底色分区而不是再画边框。

## 步骤

1. [x] 小修：最近会话标题截断；水印字放宽字距与字宽、降低存在感；本地数据目录覆盖 Codex / Claude Code / Claude Desktop / OpenCode / Pi / AgentHarbor。
2. [x] 中转页去 AI 感：站点头像改为浅色底 + 展示字体字母；去掉彩虹流光条与紫青渐变；概览、筛选、弹窗头部同步收敛配色与图标。
3. [x] 中转注入页重排：拓扑之后改为“官方客户端”（Codex 与 Claude Desktop 并排）+“命令行客户端”（Claude Code / OpenCode / Pi 三列）+“路由能力”；
   去掉外层卡片包内层带框面板的写法。
4. [x] 首页路由圈再调整：客户端节点下加名称，中心显示“经路由客户端数 / 总数”，连线改为单一强调色。
5. [x] 侧边栏重构：品牌区、分组导航（概览 / 连接 / 会话与创作 / 扩展 / 系统）、新的选中态与折叠态。
6. [x] 其余页面：共享 `PageHero`（海报标题 + 英文小标 + 水印 + 说明 + 操作区），用于账号、生图、自定义指令、MCP、Skills、维护、设置；
   统一 BentoCard / AuroraCard / GlowCard 等底层卡片外观。
7. [x] 回归：tsc、vite build、前端回归、截图自查（浅色 / 深色 / 窄屏）。

## 结果

- 新增后端字段 `AppPathState.appDataDir / claudeHome / claudeProjectsExists`，首页“本地数据目录”列出六个客户端目录（复制 / 打开）。
- 去 AI 感：移除紫 / 青色（`262 83% 66%`、`190 95% 50%`）与 indigo / purple / cyan 类；站点头像、叠卡、流光条、协议码、进度条改为强调色或中性色。
- 容器：`.harbor-card`（细线实体卡）与 `.harbor-well`（无边框浅底分区）替换注入页的卡中卡；BentoCard / BentoInnerPanel / AuroraCard 同步。
- 注入页：`.inject-grid` 让 Codex 与 Claude Desktop 并排、命令行客户端三列（DOM 顺序不变，满足既有测试）。
- 侧边栏：Home / Connect / Work / Extend / System 分组、凸起选中态、底部路由状态；夹具新增 `sidebar` 预览模式。
- 其余页面：共享 `layout/page-hero.tsx`，用于账号、MCP、Skills、自定义指令、维护、设置、生图。
- 回归：前端 197 项（含 Desktop 配置检查）、cargo test lib 1140 + 集成 14/20 全部通过；测试改用 `[data-page-scroll-container]`、`[data-skill-card]` 等稳定钩子。
