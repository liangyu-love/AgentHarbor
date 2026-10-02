# Claude Code 注入问题修复计划

日期：2026-10-02  
状态：已核实待评审，未开始实施（2026-10-02 修订：并入 magpie 对照结论）  
范围：`src-tauri/src/core/client_config/`、`src-tauri/src/core/models_dev`（只读）、`src/components/newapi/client-config-*`、`docs/client-configuration.md`  
核实基线：工作区 `feat/models-dev-integration` 分支当前代码；本机安装的 Claude Code 2.1.285（二进制字符串与解析逻辑实测）；本机同时装有 CC Switch  
参考源码：本地 `D:/workspace/temp/magpie`，提交 `7a3337d`（`internal/agent/claude.go`、`internal/agent/applied.go`）——借鉴点在对应修复项中标注出处

## 1. 背景与核实方式

针对外部评审提出的 14 项 Claude Code 注入问题，逐条与当前代码比对，并对存疑的协议事实做了实测验证：

1. 全部行号引用均按当前工作区代码重新核对，不是转抄评审原文。
2. 环境变量名与设置键名在本机 `claude.exe`（2.1.285，scoop nodejs shim 指向 `node_modules/@anthropic-ai/claude-code/bin/claude.exe`）中逐一 grep 确认。
3. 从二进制中提取了 Claude Code 的环境变量布尔解析函数，确认 `"0"` 按 falsy 处理（只认 `1/true/yes/on`，大小写不敏感、trim 后比较），因此写 `"0"` 的危害方向是「关闭」，不存在「误开启」。
4. 与本地 magpie（提交 `7a3337d`）的 Claude Code 注入与漂移检测逐点对照：新增缺陷线索一项（#15），充实方案四处（F2、F4、F6、F7），补充两项可选增强；对照中确认 magpie 的 stash/unstash 恢复粒度不如本项目的字段级事务（见第 4 节），不做借鉴。

## 2. 核实结论总表

| # | 评审问题 | 结论 | 证据 |
| --- | --- | --- | --- |
| 1 | 无条件写 `CLAUDE_CODE_USE_BEDROCK/VERTEX/FOUNDRY = "0"`；原生模式退出不恢复 | 属实 | `claude.rs:202-209` 无条件写入，键不存在也会凭空创建（`mod.rs:598-616` 的 `set_field` 语义）；`mod.rs:370-381` 退出恢复只过滤 `needs_router()`。三个变量在 2.1.285 二进制中均存在且生效 |
| 2 | 首次注入零接管检测，静默顶掉 cc-switch 等工具 | 属实 | `mod.rs:242-244` 无 session 时只验证可解析；`ConfigPreview` 只返回管理字段补丁，无 before 值、无警告。本机装有 CC Switch，风险现实 |
| 3 | `[1m]` 六个槽位默认全开；catalog 收到但未使用 | 属实 | `options.rs:41-52` 六个 `true`；前端 `client-config.ts:9-13` 缺省同样全 `true`；`claude.rs:30` 参数名为 `_catalog` 未使用 |
| 4 | 角色自动匹配 = 首个子串命中，随存储顺序 | 属实 | `claude.rs:127-131`；候选顺序来自 `options.rs:89-103`（启动模型在前，models 按存储序） |
| 5 | `stale_generated` 形状判据两头不可靠 | 属实，漏清面更大 | `claude.rs:176-178` 判据 `len==2 && label==model`。补充：我们生成的行 label 取 `display_name`（`claude.rs:191-198`），通常不等于 model id，所以连自己生成的两键行都匹配不上——release 后重配时残留比评审描述的更宽 |
| 6 | `availableModels` 缓慢累积脏 id | 属实 | `claude.rs:210-227` 只收缩 stale 命中的 id。有 journal 的正常重配靠 `undo` 整块还原是干净的；release/日志丢失后重配才累积 |
| 7 | 数组整块比对 → 用户后加一行即硬冲突，违背文档承诺 | 属实 | `mod.rs:618-627` 整值比对；`docs/client-configuration.md` 承诺「保留已有的手动条目及其显示名称、描述和能力配置」，该承诺只对首次应用前已存在的条目成立 |
| 8 | 冲突态下「恢复原配置」必然失败 + UI 看似可用 | 后端属实，前端已修 | 后端 `mod.rs:336` 先 `verify_fields` 再恢复，冲突即报错；但前端 `client-configs.tsx:120` 冲突态已禁用恢复按钮，`:123-126` 同时展示「释放管理」入口——评审描述的 UX 陷阱在当前代码不存在 |
| 9 | Codex 有「重新接管」路径，Claude/OpenCode 没有 | 属实 | `router.rs:2474-2483`「以 cc-switch 现状为新基线」+ `reject_foreign_managed_provider_entries`（`router.rs:1899, 2488`）；`client_config/mod.rs` 全文无对应机制 |
| 10 | 2.1.242 门槛只有静态文案，无版本检测 | 属实 | `locales/en.json:102`、`zh.json:102`；`targets.rs:15-16` 注明 Claude Code/OpenCode 为 configuration-only，全仓库无二进制/版本探测 |
| 11 | `ANTHROPIC_DEFAULT_FABLE_MODEL` 与 `*_NAME` 系列变量名存疑 | 不成立 | 2.1.285 二进制中 `ANTHROPIC_DEFAULT_FABLE_MODEL`、`ANTHROPIC_DEFAULT_SONNET_MODEL_NAME` 等全部真实存在，无需修改 |
| 12 | `ANTHROPIC_SMALL_FAST_MODEL` 未管理 | 属实 | 仓库无任何处理；该变量在 2.1.285 二进制中仍有 18 处引用（未彻底退役），旧网关遗留值会继续生效 |
| 13 | `claude_model_ids` 报通用错误，不指明哪个模型 | 属实 | `claude.rs:71`；locale 文案为泛化提示 |
| 14 | Claude 走严格 JSON，带注释的 settings.json 无法注入 | 属实 | `mod.rs:583-587`：OpenCode 走 json5，其余 `serde_json` 严格解析 |
| 15 | `CLAUDE_CODE_SUBAGENT_MODEL` 无条件写主模型，覆盖 subagent 的 tier 请求（magpie 对照新增） | 属实 | `claude.rs:117-131`：subagent 映射为空时，自动匹配被 `!role.is_empty()` 谓词跳过，落到 `unwrap_or(&request.model)`，总是写主模型。magpie `claude.go:125-137` 明确避免此行为：「one model for every subagent would override the tiers they ask for」 |

结论：15 项中 13 项需要修复（#1–#7、#9–#10、#12–#15），#11 关闭，#8 仅剩后端语义随 #9 一并处理（前端已正确）。#15 为与 magpie 对照后新增。

## 3. 修复方案

### 批次一：防破坏（可独立合入）

#### F1 云模式开关不再无条件写入（修 #1）

问题：`claude.rs:202-209` 对三个云开关无条件写 `"0"`，字段不存在也会创建（污染 settings.json）；原生直连模式退出时不恢复（`mod.rs:370-381`），Bedrock/Vertex/Foundry 用户被永久关闭云接入。

方案——从「替用户关掉」改为「绝不碰」：

- 字段不存在：不写、不创建。
- 字段存在且解析为假值（`"0"/"false"/"no"/"off"/""` 等不区分大小写、trim，与 Claude Code 的 `Ne()` 解析对齐）：不重写，现状已生效。
- 字段存在且解析为真值：拒绝应用，返回新错误键 `clients.errors.cloudMode`，指明变量名与处理建议（该值代表用户真实的云接入配置，必须由用户自行决定去留）。preview 阶段报同样错误，让用户在应用前就知道。
- 删除现有无条件写入循环；`claude.rs:202` 的注释同步改写。

兼容性：老 journal 中已记录的字段 before/after 照常回滚，`restore` 不受影响；已写过 `"0"` 的用户升级后字段为假值、不再被触碰，想清除只能手动或恢复。

测试（`native_tests.rs`）：无字段 → 不创建；`"0"` → 保持原值；`"1"`/`"true"`/`" yes "` → 应用报 `cloudMode`；含三个变量组合的场景。

#### F2 首次注入接管警告（修 #2）

问题：无 session 的首次 `apply`/`preview` 不检查现状，直接覆盖 `env.ANTHROPIC_BASE_URL` 与凭据，会静默顶掉 cc-switch 等工具写入的值。

方案——警告不阻断，但必须显式可见：

- `prepare` 在 `previous == None` 时检查现状文档：`env.ANTHROPIC_BASE_URL` 非空且不等于即将写入的值（强警告）；`env.ANTHROPIC_AUTH_TOKEN`/`ANTHROPIC_API_KEY` 非空（提示，凭据将被替换）；`model`、`modelPicker.options`、`availableModels` 已存在（提示）。
- `ConfigPreview` 新增 `warnings: Vec<String>`（i18n 键列表）；`ConfigApplied.warning` 字段已存在（`mod.rs:148-155`，当前恒为 `None`），apply 时带入合并后的警告。
- 前端：`src/types/clients.ts:48` 的 `ClientConfigPreview` 增加对应字段；`client-config-dialog.tsx` 预览页以 destructive 样式列出，措辞说明「以下字段将被覆盖，可能属于其他工具」。
- 有 session 的重配路径不加（`verify_fields` 已兜底外部改动）。

补充检测源——管理级配置（借鉴 magpie `claude.go:269-279`、`:238-241`）：

- Claude Code 的管理级 `managed-settings.json` 优先级高于用户级 `~/.claude/settings.json`，其 `env` 会整体盖掉我们的注入，且从用户文件里看不出来：
  - Windows：`C:\ProgramData\ClaudeCode\managed-settings.json`
  - macOS：`/Library/Application Support/ClaudeCode/managed-settings.json`
  - Linux：`/etc/claude-code/managed-settings.json`
- preview/apply 时读取该文件（存在才读），若其 `env` 定义了 `ANTHROPIC_BASE_URL`、认证或模型角色变量，warnings 中以最高级别提示「管理级配置将覆盖本次注入」并列出冲突键；文件不存在时零开销。
- 路径解析抽为可注入函数（对齐 magpie 的 `claudeManaged` 变量模式），便于单元测试与隔离环境。

测试：`tests.rs` 增补无 session + 既有 base URL/凭据的 preview 断言；管理级配置经注入路径单测（有/无冲突键两态）；手测与 CC Switch 共存场景。

#### F6 `ANTHROPIC_SMALL_FAST_MODEL` 纳入管理（修 #12）

问题：旧版 haiku 角色变量不在管理范围，旧网关遗留值会继续生效（2.1.285 二进制仍引用该变量，未彻底退役——仍读取它的版本上，后台小任务会打向遗留值指向的模型）。

方案：将该变量纳入管理集合——已存在则覆盖为 haiku 角色同值（与 `ANTHROPIC_DEFAULT_HAIKU_MODEL` 一致），不存在则不创建（与 F1 的「不凭空创建」原则一致）。覆盖写入顺带清掉旧网关遗留值；恢复按 before/after 记录回滚。magpie 的做法是无条件写入（`claude.go:122`，把 haiku tier 写进该变量，兼容只认旧变量的版本，还在网关侧把它作为 haiku 兜底读取，`claude.go:308-312`）；其无条件创建对旧版兼容性更好，但会在新版用户的文件里留下无效键，与 F1 的原则冲突。待 F8 版本检测就绪后，再评估是否对确认仍读取该变量的旧版本改为主动写入。

测试：存在遗留值 → 覆盖为 haiku 同值且 restore 还原；不存在 → 无操作、不创建。

#### F11 `CLAUDE_CODE_SUBAGENT_MODEL` 不再无条件写主模型（修 #15）

问题：`claude.rs:117-131` 中 subagent 映射为空（用户未配置子代理映射）时，自动匹配被 `!role.is_empty()` 谓词跳过，直接落到 `unwrap_or(&request.model)`——该变量总被写成启动主模型。Claude Code 语义是：设置了它，所有 subagent 强制使用它，覆盖 subagent 按各自 tier（`ANTHROPIC_DEFAULT_HAIKU_MODEL` 等）请求模型的行为；而其余四个角色变量又与主模型不同，结果是小任务跑主模型（往往更贵），用户也无法通过 tier 控制 subagent。

方案（判据来自 magpie `claude.go:125-137`）：

- `mapping.subagent` 非空（显式配置）：照写，现状不变。
- `mapping.subagent` 为空：`remove(&["env", "CLAUDE_CODE_SUBAGENT_MODEL"])`，纳入恢复记录，不再写主模型。
- 「为空即删」无需 magpie 的 tier 等值条件分支：当四个角色映射恰好都等于主模型时，删除与写主模型等价（角色变量已把所有 tier 解析到同一模型）；角色映射不同时，删除才能保留 tier 行为。严格不劣于现状。
- `docs/client-configuration.md` 角色映射描述同步改写：「缺少对应模型时使用所选默认模型」不再适用于子代理——子代理在未显式映射时跟随各角色 tier。

测试（`native_tests.rs`）：显式映射 → 照写；空映射且此前写过 → 移除且 restore 还原；空映射且本无该键 → 无操作。

### 批次二：元素级数组管理（核心语义，一次修三个问题）

#### F3 `modelPicker.options` / `availableModels` 按元素管理（修 #5、#6、#7）

根因：数组被当作不透明整块——应用时靠形状猜测清理（`stale_generated`），校验时整块比对（`verify_fields`）。形状猜测导致「误删用户行」与「漏清自己的行」双向失败；整块比对导致用户后续任何数组改动都变成硬冲突，与文档承诺矛盾。

方案——引入应用侧所有权记录 + 元素级重建：

1. 新增所有权记录 `client-config/<target>.owned.json`（与 journal 同目录、同等加密存储）：每次成功 apply 后记录本轮写入的 picker 行 model id 集合与 `availableModels` 值集合。`release` 不删除（记住我们写过什么，供下次清理）；`restore` 删除会话时一并删除。
2. `prepare`/`edits` 重建逻辑：无论有无 session，先按 owned 记录从当前数组移除「此前由我们写入」的元素（picker 按 `model` id 键，`availableModels` 按字符串值），再合并本次选择。效果：
   - 我们生成的条目精准可撤，不依赖形状（修 #5 漏清、#6 累积）；
   - 用户手写条目永不触碰，包括 `{"model","label"}` 极简行（修 #5 误删）；
   - 用户在应用后新增/修改的行自动被采纳为新基线，不再硬冲突（修 #7）；
   - 用户删过我们的行则视为放弃，本轮按当前选择重新生成。
3. `stale_generated` 形状启发式（`claude.rs:168-183`）退役，仅在「无 session 且无 owned 记录」（老版本残留或首次接管）时保留收窄版作为一次性迁移：只删 `label == model` 且不在当前选择中的两键行，并把结果写入 owned 记录，此后不再走启发式。
4. `verify_fields`（`mod.rs:618-627`）对这两个数组路径跳过整块比对（所有权语义已由 owned + 重建保证），标量路径维持硬校验。`undo` 仍按记录的 before 整块还原，恢复语义不变。
5. 升级种子迁移：已有老 session 的用户首次 apply 时，从其 `changes` 中数组字段的 after-before 差集（按键）生成 owned 记录。

场景验收表（每条一个测试）：

| 场景 | 期望 |
| --- | --- |
| 用户手写极简行 `{"model":"x","label":"x"}`，x 不在本次选择 | 保留 |
| 我们生成的行被用户补了 description | 本轮按选择重建（model id 在 owned 中，移除后重加），不残留 |
| 用户在应用后新增一行 | 保留，不报冲突 |
| 用户删除我们生成的某行 | 尊重删除；owned 中该 id 仍记录，下轮按当前选择重建 |
| 换中转重配 | 上一轮条目全部撤回，新选择重新生成，零残留 |
| release 后重配 | 按 owned 记录精准撤回，不依赖形状 |
| restore | 数组还原到本轮 apply 前的整块状态；会话与 owned 一并清除 |

#### F4 `[1m]` 数据驱动（修 #3）

问题：六个槽位默认全开（`options.rs:41-52`，前端同），对 200k 模型也打 1M 标记；`claude.rs` 收到 catalog 却未使用（`:30`）。

方案——后端兜底 + 前端预填，显式数据优先：

- 后端：`claude.rs` 在 `marked_model` 前用 `model_metadata::resolve`（`model_metadata.rs:655`，字段优先级 user > provider > upstream > official > catalog，见 `:800-803`）取该模型已知 context；已知且 < 1,000,000 → 强制不加 `[1m]`（覆盖勾选），并计入 preview warnings（新 i18n 键，说明已忽略哪个角色的 `[1m]` 勾选）；catalog 无数据 → 尊重勾选，不做猜测。
- 前端：`client-config-dialog.tsx` 打开时按当前映射模型的解析 context 预填六个勾选（已知 ≥1M → 勾选；已知 <1M → 不勾并禁用勾选框 + 提示；未知 → 不勾，可手动勾选）。`claudeOneMDefaults`（`client-config.ts:9-13`）仅作反序列化兜底。
- 不改 `ClaudeOneM` 的 serde 默认值，避免破坏老配置反序列化。

测试：catalog fixture 三态（200k / 1M / 无数据）× 勾选开关，断言最终写入的模型串；preview warnings 断言。

与 magpie 一致性：其 `claudeViaMagpie`（`claude.go:252-267`）仅对 context ≥ 1,000,000 的模型追加 `[1m]`，无数据一律不加，并注明依据（未标记按 200K 规划，1M 模型不标记会过早压缩）——本项的前端默认与后端判定均与该行为对齐。

#### F5 角色自动匹配确定性（修 #4）

问题：`claude.rs:127-131` 取首个包含子串的候选，结果随存储顺序漂移，`claude-opus-4` 与 `claude-opus-4-6` 并存时可能选中旧版。

方案——分段匹配 + 数字感知取新：

1. 候选集：把模型 id 按 `-`/`_`/`.` 分词后匹配角色 token（边界匹配，排除裸子串误配）。
2. 唯一候选直接采用；启动默认模型（`provider.model`）命中角色时优先。
3. 多候选：按数字感知的版本序取最新——拆尾部数字段逐段数值比较（`4-6` > `4`，`4-10` > `4-6`），避免字典序把 `4-10` 排到 `4-6` 之前；仍并列时按现有列表顺序。
4. 映射结果在 preview 补丁中可见（现状已如此），无需额外展示。

测试：`opus-4`/`opus-4-6` 两种存储序得出同一结果；`haiku-3-5`/`haiku-4-5`；带同名前缀的干扰模型不误配。

### 批次三：补齐与打磨

#### F7 显式「重新接管」基线（修 #9，连带 #8 的后端出路）

问题：Codex 有「外部接管 → 以现状为新基线」（`router.rs:2474-2483`）与外来条目防护，Claude/OpenCode 只能报冲突；冲突态下标量字段无应用内出路（`restore` 先 `verify_fields`，`mod.rs:336`），唯一出口 `release` 不还原字段。

方案：

- `apply` 命令（`commands/client_config.rs`）与 `ConfigRequest` 增加可选 `rebaseline: bool`。为 true 且存在冲突 session 时：跳过 `verify_fields` 的标量比对，把当前值记录为新 before（以现状为基线）后照常应用；数组路径经 F3 本就兼容。
- 前端冲突文案（`client-configs.tsx:110`）旁提供「以当前文件为准重新接管」入口 + 二次确认对话框，明确提示将覆盖现有值。
- 配套「重新应用上次配置」：session 存在时，客户端卡片提供一键按 `session.request` 重新注入（不重开配置对话框），用于用户手动改坏文件或重装客户端后快速恢复。语义对齐 magpie 的 `Reapply`（`applied.go:226-252`）。
- 冲突信息分型：`verify_fields` 内部已知哪些字段不一致，把冲突字段路径按「地址 / 凭据 / 模型 / 菜单」分类随 `ClientStatus` 暴露，前端冲突提示从泛化文案变为指出具体被改字段。借鉴 magpie 的 drift 分型与 `wiringOff` 三种措辞（值消失 / 指向别处 / 被改动，`applied.go:315-333`）。
- 语义与 Codex 路径对齐：都是用户在知情状态下的显式动作，不是静默采纳；「重新接管」对应 magpie 的 `Keep`（接受现状、忘掉旧记录，`applied.go:256-265`）。

#### F8 Claude Code 版本检测（修 #10）

问题：2.1.242 门槛只有静态文案（`locales/*.json:102`），`targets.rs:15-16` 明确 Claude Code 是 configuration-only，没有任何位置知道其安装与版本。

方案：

- 新增轻量探测：PATH 查找 `claude`（Windows `where`，Unix `which`）+ 常见安装位置（npm global、native installer），执行 `claude --version`（2 秒超时，失败容忍），解析版本号。结果按进程缓存，不在 `list()` 每次调用时反复 spawn。
- `ClientStatus` 增加 `client_version: Option<String>`。
- 配置对话框中 `modelPickerVersion` 文案从静态脚注升级为状态感知：检测到版本 < 2.1.242 → 醒目警示（`/model` 菜单不生效，其余注入不受影响）；检测不到 → 信息性提示。不做阻断——版本只影响菜单展示，不影响注入本身。

#### F9 Claude settings.json 注释容忍（修 #14）

问题：`mod.rs:583-587` 中 Claude 走严格 `serde_json`，被其他工具写入注释的 settings.json 直接 `invalidConfig`，无法注入。

方案：`parse` 对 ClaudeCode 增加 json5 回退（`serde_json` 失败后再试 `json5`，与 OpenCode 一致）；写入始终为规范化 JSON。这与文档既有政策一致（「JSONC 应用后规范化为 JSON，原始带注释文本保存在备份中」，原文针对 OpenCode，补充说明对 Claude 同样适用）。`restore` 的「无独立改动还原原始字节」路径本就保留注释（`mod.rs:337-339`），无需改动。

#### F10 前端模型选择校验（修 #13）

问题：`claude_model_ids` 含已失效模型时只报泛化的 `clients.errors.modelSelection`（`claude.rs:71`），不指明哪个。

方案：前端在提交前对照最新 `clientModels()`（`client-config.ts:17-25`）校验所选集合，对已失效 id 逐个列出并提示刷新；后端泛化错误保留为兜底（供应商模型在对话框打开后变化的窗口期仍可能触达）。

## 4. 核实后不修改的项

1. **#11 变量名**：`ANTHROPIC_DEFAULT_FABLE_MODEL`、`ANTHROPIC_DEFAULT_*_MODEL_NAME` 经 2.1.285 二进制核实为真实变量，关闭该项。
2. **#8 前端部分**：恢复按钮冲突态已禁用、释放入口已展示（`client-configs.tsx:120-126`），保持现状；后端「先校验后恢复」的语义保留，用户出路由 F7 的显式重新接管补齐。
3. **`"0"` 的取值**：实测 Claude Code 布尔解析只认 `1/true/yes/on`，`"0"` 为 falsy；F1 落地后本项目不再写这些字段，此事实仅作为 F1 真值判定函数的实现依据。
4. **恢复事务机制**：magpie 的 stash/unstash（`claude.go:71-110`）只按名记录 3 个字段、无备份轮转与并发修订校验；本项目的字段级 before/after 记录 + 备份 + revision 校验更强，保持现状，不做借鉴。

## 5. 可选增强（非缺陷，独立评审）

1. `ANTHROPIC_DEFAULT_{SONNET,OPUS,FABLE,HAIKU}_MODEL_DESCRIPTION` 与 `_SUPPORTED_CAPABILITIES`：2.1.285 二进制中存在该系列。`modelPicker.options` 行已保留能力元数据，未来可将映射后的显示描述与能力以 env 形式补写，提升 `/model` 菜单信息完整度。实施前需单独确认行内元数据与 env 变量的优先级关系。
2. **bypassed 交叉检测**（借鉴 magpie `applied.go:144-161`、`claude.go:245-248`）：用 Claude Code 本地使用痕迹（`~/.claude/history.jsonl` 的最后使用时间，跳过斜杠命令行）与路由侧最后到达时间（route_trace/usage）交叉比对，发现「配置正确但请求没到」——agent 未重启、项目级/管理级配置或 shell 环境变量覆盖，这些从配置文件侧无法检测。仅本地路由模式适用（原生直连的流量不经过本应用）；只读时间戳，不采集内容。magpie 的两个细分（上次请求去了别处 → 未重启；请求被拒 → 当时网关未运行）依赖逐请求目的地记录，评估 route_trace 现有数据是否够用后再定范围。
3. **`CLAUDE_CODE_EFFORT_LEVEL` / `effortLevel` 推理档位管理**（magpie `claude.go:161-185` 已实现）：settings.json 的 `effortLevel`（low–xhigh）加 env 的 `CLAUDE_CODE_EFFORT_LEVEL`（max，仅会话级语义由 Claude Code 自己处理）。待 models.dev typed reasoning 数据就绪并确认客户端契约后纳入，与 magpie 后续计划的元数据使用原则一致。

## 6. 实施顺序与测试

顺序：F1 → F2 → F6 → F11（四项小而独立，先消除破坏性）→ F3 → F4 → F5 → F7 → F8 → F9 → F10。F3 改动面最大（会话语义 + `claude.rs` + owned 记录），与 F7 的冲突出口一并评审合入。

- 单元测试：`native_tests.rs`（Claude 注入/恢复断言，覆盖 F1/F4/F5/F6/F11）、`tests.rs`（会话与冲突流转，覆盖 F2 含管理级配置注入路径、F9）、`ownership_tests.rs`（F3 的 owned 生命周期与场景表逐条落测）。
- 回归：`scripts/test-frontend-regressions.mjs` 覆盖前端 warnings 展示、勾选预填与冲突分型提示。
- 手测：真实 Claude Code 2.1.285 与 CC Switch 共存（F2）；带注释的 settings.json（F9）；版本 < 2.1.242 的提示路径（F8，可用环境变量路径隔离测试）；subagent tier 行为（F11，观察小任务实际请求的模型）。
- 文档与文案：`docs/client-configuration.md` 同步云开关行为、接管警告、重新接管、owned 记录与 JSONC 策略适用范围；`locales/zh.json`/`en.json` 新增 `clients.errors.cloudMode`、接管警告、rebaseline 确认、`[1m]` 忽略提示等键。
- 兼容性：journal/backup 格式不变；owned 记录为新增文件，老用户首次 apply 走种子迁移；所有 serde 默认值保持向后兼容。

## 7. 风险

1. F3 改变数组冲突语义：从「硬冲突」放宽为「采纳合并」。这是产品语义变化，需在文档与 UI 中明示；对依赖冲突提示发现外部改动的用户，F7 的重新接管提供等价入口。
2. F4 后端强制剥离 `[1m]` 覆盖显式勾选：仅发生在解析 context 已知且 < 1M 时（数据事实优先），UI 同步禁用勾选框避免「勾了没用」的困惑。
3. F8 依赖外部进程探测：spawn 失败、非标准安装、便携版无版本输出等都要容忍降级为「未知」，不得阻塞注入流程。
4. F9 的 json5 回退会接受比 Claude Code 本身更宽松的语法：写入时规范化为 JSON 可能丢弃注释，需按文档既有政策在备份中保留原文（机制已存在）。
5. F11 是默认行为变化：依赖「子代理跟随主模型」的用户升级后会看到 subagent 开始按 tier 请求模型（成本与用量结构变化）。变更说明与文档需明示；需要旧行为的用户在角色映射表中显式配置子代理即可完全复原。
6. F2 的管理级配置检测在企业环境下可能常态命中：提示设计为警告而非阻断，避免把企业管控当成故障。
