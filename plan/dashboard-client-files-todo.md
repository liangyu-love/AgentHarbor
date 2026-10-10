# 仪表盘：各客户端关键文件并入数据地图

## 用户诉求
仪表盘「本机数据」只有「Codex 关键文件」，没有 Claude 等客户端的展示。用户选择「合并进数据地图」：
每个客户端那一行在目录下直接带一排文件状态标签，去掉单独的 Codex 关键文件区。

## 步骤
1. [x] 后端 `core/client_files.rs`：只做存在性检查（不读、不建），按客户端返回关键文件
       - Codex：config.toml、auth.json、registry.json、sessions/、AGENTS.md
       - Claude Code：settings.json、.claude.json、projects/、CLAUDE.md
       - Claude Desktop：常规配置、第三方配置、接管 profile、接管元数据（与 Desktop 状态同一组路径）
       - OpenCode：opencode.json(c)
       - Pi：models.json、auth.json、settings.json、sessions/
       命令 `list_client_files`；纯函数单测。
2. [x] 前端：类型、api、query key；数据地图每行加文件标签（存在 ✓ / 缺失 ○，悬停显示完整路径）；
       删除 Codex 关键文件区与 `HealthItem`、相关样式与文案。
3. [x] 文案 zh/en、fixture mock、回归测试断言。
4. [x] 构建、Rust/前端回归、亮/暗/手机截图核对。

## 结果
- 后端 `core/client_files.rs::collect` + 命令 `list_client_files`；Claude Desktop 路径用新增的
  `Manager::current_paths()`（与状态同源：已接管取保存路径，否则取发现的默认路径），定位不到时不列该客户端。
  单测覆盖存在/缺失/「目录位置是文件」与「收集过程不创建任何东西」。
- 数据地图：每行底部一排文件标签（存在为实线 + 绿勾，缺失为虚线 + 空心圈，悬停显示完整路径），
  宽屏与文字列对齐、手机占满整行；Desktop 两份同名配置按角色显示「常规配置 / 第三方配置」。
  有关键文件存在但其他来源没给出目录时，用首个文件所在目录兜底（复制/打开目录随之可用）。
- 删除 Codex 关键文件区、`HealthItem`、`overview-health-*` 样式与 9 个旧文案。
- 测试：Rust lib 1154 + 集成 14 + 20；前端 202 PASS（含新增数据地图断言）；亮/暗/390px 截图核对。
