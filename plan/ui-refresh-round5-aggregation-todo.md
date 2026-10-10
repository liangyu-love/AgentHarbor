# Round 5 — 注入页重排、聚合重做与 magpie 借鉴

## 用户诉求
- 除路由拓扑的 ROUTING 浮雕水印外，其余水印全部去掉。
- 中转注入页去掉 01 / 02 序号 UI。
- 全屏下 Codex 接入和 Claude 卡片尺寸不协调 → 改成「左栏切换 + 右侧详情」。
- 模型聚合展示太敷衍（一大坨）→ 重做。
- 参考 D:\workspace\temp\magpie（MIT）的路由组，用户选定四项全部借鉴：
  同名模型自动发现、主力/备用分层排序、首字超时切换、实时命中/冷却状态。

## 现状结论
- 我们的调度：优先级分层（数字小者优先）+ 层内轮询/加权，命中后粘住，失败达阈值才切换并冷却；
  层内顺序对选择无影响（按 member_id 轮转），所以「排序」的本质是分层。
- 请求内换成员仅在 `Attempts.safe`（portable_fresh && !bound）且连接失败/429 时发生。
- magpie 的 smart/pace/usage 依赖订阅额度窗口，嵌套组/意图路由过重，不做。

## 步骤
1. [x] 调研 magpie 与现有聚合、注入页、测试影响面。
2. [x] 水印清理：保留 ROUTING；移除 HARBOR / THREADS / RELAYS / RELAY 与 PageHero 的 watermark 参数。
3. [x] 注入页：SectionGroup 去掉序号；新增 ClientConsole（左栏 5 个客户端 + 状态，右侧单一详情面板，
       面板保持挂载、记住上次选择）；拓扑点击联动切换；Codex/Desktop/单客户端面板去掉卡片外壳；
       ClientConfigs 支持只渲染一个客户端，单客户端面板重新排版适配宽屏。
4. [x] 后端：`AggregateModel.first_token_timeout_secs`（0 关闭，上限 300）；流式创建请求在可换成员时
       先等首个内容增量，超时则放弃该成员、计一次失败、换下一个；不可换时照常等待。补 Rust 测试。
5. [x] 后端：`load_aggregation_live` 运行时快照（当前粘住成员、各成员冷却剩余、连续失败、在途数）。补测试。
6. [x] 前端聚合重做：
       - 列表：路由条（身份 + 分层车道 + 实时状态点 + 策略摘要 + 图标操作）。
       - 编辑器：分层车道（主力层 / 备用层，上下移层、拖拽换层），分段控件选策略与亲和，
         故障切换区（失败阈值、重试次数、首字超时）；保存时把层号写回 priority。
       - 同名模型发现：移植 magpie `sameModel` 归一化，≥2 个中转提供同一模型时给出一键建聚合建议。
       - 实时状态轮询（页面可见时 4s）。
7. [x] 文案（zh/en）、前端回归测试更新与新增断言。
8. [x] 构建、Rust 测试、前端回归、1920/1440/768/390 深浅色截图核对。

## 结果
- 水印只剩路由拓扑的 ROUTING；PageHero 去掉 watermark 参数；注入页去掉 01/02 序号。
- 注入页客户端区改为 `ClientConsole`：左栏 5 个客户端（状态与拓扑一致），右侧一次一个面板，
  面板常驻（切换不丢草稿）、记住上次选择、方向键/Home/End 切换；拓扑节点点击会切换面板并聚焦。
  Codex/Desktop 面板去掉折叠和卡片外壳；Claude Code/OpenCode/Pi 面板改为「接入方式/来源/默认模型」三格事实 + 配置路径。
- 聚合改为路由看板：每个聚合一行，左侧身份与策略摘要，右侧按层（主力/备用）排列成员芯片，
  芯片显示命中/待命/冷却 Ns/待探测/已停用/不可用，每 4 秒刷新运行时快照。
- 编辑器：分层车道（↑↓ 换层、拖拽换层、拖到底部新建备用层），轮询/加权与会话策略用分段控件，
  故障切换区新增首字超时；保存时 priority 归一为层号 0、1、2…，权重只在加权时显示。
- 同名模型发现：`src/lib/same-model.ts` 移植 magpie `sameModel`（MIT），≥2 个可用中转提供同一模型且未被聚合覆盖时给出一键建聚合建议。
- 后端：`first_token_timeout_secs`（0 或 5–300 秒）+ `aggregation/first_token.rs` 扫描器；
  仅对未绑定、无加密推理、非存储续写、仍有尝试次数且有备选成员的流式请求生效，超时计一次失败并换成员；
  `load_aggregation_live` 只读快照。
- 验证：Rust lib 1147 + 集成 14/20 全过（含新网关测试 `a_silent_stream_moves_to_the_next_member_before_anything_is_relayed`）；
  前端回归 200 PASS / EXIT 0（新增 `test-aggregation-board.mjs` 3 组）。
