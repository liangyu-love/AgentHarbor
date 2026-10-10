# UI 重构第二轮：中转表单 / 下拉框 / 模型信息 / 海报感字体与动效（2026-10-10）

承接 `ui-refresh-topology-relays-threads-overview-todo.md`。用户反馈：中转模型的填写表单还没优化（尤其下拉框）、
模型信息展示不清楚；动效继续美化，并加入“艺术字体 / 海报”的感觉。

会话修复：保留。开关路由不再改会话；接管其他工具（如 cc-switch 的自定义 provider）创建的会话仍存储在别的 provider 下，
在 Codex 中不可见，这正是修复入口出现的地方。

## 步骤

1. [x] 下拉框统一升级：Radix Select 触发器/弹层/选项重做（圆角、悬停、勾选、说明文字、动画）；
   原生 `<select>` 统一外观（自绘箭头），涉及模型压缩、测试对话框、用量面板等。
2. [x] 中转表单重构：
   - 新建时的预设改为可搜索的预设画廊（分类标签 + 卡片），替换原生下拉和硬编码中文分组。
   - 协议选择改为三张说明卡片（radiogroup），连接信息与协议并排。
   - 弹窗头部海报化：大标题 + 当前站点 / 协议标签。
3. [x] 模型信息展示：模型表改为卡片行，每个模型一眼可见上下文、输出上限、目录匹配（精确/推断/未匹配）、推理档位、协议；
   可编辑项保持原有字段与顺序（测试依赖）。
4. [x] 海报感字体：本地打包 Bricolage Grotesque（OFL，可变字体），提供 `font-display`；
   首页 / 中转 / 线程 / 拓扑的主视觉使用超大描边英文水印字、渐变流光标题、展示字体数字。
5. [x] 动效：卡片跟随指针的聚光（spotlight）、标题流光、水印缓慢漂移、首页轨道外圈旋转环形文字；全部尊重 reduced-motion。
6. [x] 回归：tsc、vite build、前端回归（更新预设 / 协议相关断言）、截图自查（浅色 / 深色 / 窄屏）。

## 结果

- 下拉框：`ui/select.tsx` 全局重做（圆角弹层、左侧勾选、`description` 说明行且不改变选项的可访问名称）；原生 `<select>` 统一用 `.native-select`。
- 中转表单：预设画廊（`provider-preset-gallery.tsx`，分类 + 搜索 + 站点色头像）、协议三选卡（`#newapi-provider-protocol` radiogroup，方向键切换）、
  海报式弹窗头部和固定底栏；模型卡显示协议 / 上下文 / 输出 / 目录匹配 / 推理档位，默认模型用星标。
- 字体：`src/assets/fonts/bricolage-grotesque-latin-variable.woff2`（OFL，许可证同目录），Tailwind `font-display`；中文回退系统字体。
- 海报元素：`poster-watermark` 描边水印（HARBOR / RELAYS / THREADS / ROUTING / RELAY）、`poster-title` 流光标题、`poster-index` 描边序号、
  `poster-kicker` 英文小标、展示字体数字；首页轨道外圈旋转环形文字。
- 动效：`lib/spotlight.ts` 单一 pointermove 监听驱动 `[data-spotlight]` 聚光；图标悬停微转；全部尊重 reduced-motion。
- 回归：tsc、vite build、前端回归 187 项 + Desktop 配置检查通过；更新了预设 / 协议 / 窄卡标题断言。
