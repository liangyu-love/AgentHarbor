# Claude Code 注入修复总结

日期：2026-10-02  
分支：`fix/claude-code-injection`（合入 `feat/models-dev-integration`）  
计划依据：`plan/claude-code-injection-fix-plan.md`（含核实方式与逐项证据）  
实施提交：`9f54df9`、`e386d72`、`4e56b12`、`aa8b5ff`（主分支）＋ `7531770..0c3335f`（修复分支）

## 1. 目的

外部评审并对本机 Claude Code 2.1.285 二进制与本地 magpie 源码实测核实后，确认注入路径存在 13 项需修复的问题（另有 1 项证伪、1 项仅剩后端语义），集中在四类风险：

1. **破坏用户环境**：无条件写 `CLAUDE_CODE_USE_BEDROCK/VERTEX/FOUNDRY = "0"` 且原生模式退出不恢复，会永久关闭用户自己的云接入；首次注入无接管检测，静默顶掉 cc-switch 等工具写入的地址与凭据。
2. **能力值错误**：`[1m]` 六个槽位默认全开，不看模型真实上下文；角色自动匹配取首个子串命中，结果随模型存储顺序漂移。
3. **误删/漏清用户数据**：`modelPicker.options` 靠形状猜测清理，两头都不可靠；数组整块比对让用户在应用后的任何改动都变成硬冲突，违背文档承诺。
4. **行为缺陷与出路缺失**：`CLAUDE_CODE_SUBAGENT_MODEL` 被无条件写成主模型，覆盖 subagent 按 tier 请求模型的行为；冲突态只有「释放管理」一条路。

修复约束：不改变「字段级合并 + 可恢复事务」的既有模型；每项修复独立提交、可单独回滚；与并行的 Magpie 诊断线路（`0c3bad6`）零耦合。

## 2. 做了什么

| 项 | 修复 | 提交 |
| --- | --- | --- |
| F1 | 云模式开关不再创建或改写；真值（1/true/yes/on，与 Claude Code 解析对齐）直接以 `clients.errors.cloudMode` 拒绝注入 | `9f54df9` |
| F6 | 遗留 `ANTHROPIC_SMALL_FAST_MODEL` 改写为 Haiku 角色同值，不创建新键 | `e386d72` |
| F11 | subagent 映射为空时移除 `CLAUDE_CODE_SUBAGENT_MODEL`，让 subagent 跟随各角色 tier | `4e56b12` |
| F2 | 首次注入接管警告（地址/凭据/模型菜单被覆盖时在预览与结果中列出）＋ 管理级 `managed-settings.json` 冲突检测（三平台固定路径，可注入测试） | `aa8b5ff` |
| F3 | 新增所有权记录 `client-config/<target>.owned.dat`：`modelPicker.options` 与 `availableModels` 改为元素级管理——精准撤回自己写入的条目、采纳用户新增/改动/删除、release 后仍可撤回、restore 清除；形状启发式降级为无记录时的一次性迁移 | `7531770` |
| F4 | `[1m]` 由共享 resolver 决定：已知上下文 <1M 强制不标记（前端同步禁用勾选并提示），未知尊重显式勾选；缺省值翻转为安全的「不标记」 | `1af8d39`、`83bff8c` |
| F5 | 角色自动匹配改为分段边界匹配、启动模型优先、数字感知取最新版本（前后端同一算法），与存储顺序无关 | `af1f696` |
| F7 | 新增 `reapply_client_config`：一键按上次配置重注入；`rebaseline` 把当前文件值记录为新基线，是冲突的显式出路（与 Codex 接管语义对齐）；冲突提示按地址/凭据/模型/菜单分型 | `3b2f9b6` |
| F8 | Claude Code 版本探测（`claude --version`，2 秒超时，Windows cmd 回退，每进程缓存），卡片显示版本、`/model` 提示按版本分级 | `3de159d` |
| F9 | Claude `settings.json` 带注释/尾逗号时 json5 回退解析，写回规范化 JSON，原文备份保留 | `3de159d` |
| F10 | 对话框逐个列出已失效的所选模型，替代泛化报错 | `3de159d` |
| 审查跟进 | subagent 自动标签与 F11 行为对齐、`effectiveContext` 优先级对齐 resolver（override > provider 级 > 上游）、测试加固 | `0c3335f` |

## 3. 产出

- **代码**：`core/client_config/` 新增 `owned.rs`（所有权记录）、`takeover.rs`（接管与管理级配置警告）、`version.rs`（版本探测）；`claude.rs`/`mod.rs`/`opencode.rs` 适配；命令层新增 `reapply_client_config`；前端 `client-configs.tsx`、`client-config-dialog.tsx`、`claude-model-config.tsx`、`client-config.ts`、类型与 26 个新 i18n 键（zh/en 对齐，后端 56 个错误键全量对账无缺失）。
- **测试**：新增 18 个针对性测试（云开关三态、SMALL_FAST、subagent 跟随、F2 警告、所有权记录 8 场景、`[1m]` 三态、角色匹配顺序无关、rebaseline 全链路、json5 注释往返、数组键删除语义、版本解析）；回归脚本按数据驱动 `[1m]` 新规范改写。合并后全量：**Rust 499 通过、tsc 0 错误、vite 构建通过、33 条 Playwright 回归通过**，clippy 对本范围零警告。
- **文档**：`docs/client-configuration.md`（接管警告、元素级数组、重新接管、云开关、版本提示、jsonc 策略）、`docs/CODEBASE_MAP.md`（新命令与新模块）、计划文档归档并标注实施状态。
- **回滚**：每项一个独立 commit，`git revert <hash>` 可单独撤销；`owned.dat` 缺失或损坏时自动退化为会话种子推导，不会误删用户数据。
