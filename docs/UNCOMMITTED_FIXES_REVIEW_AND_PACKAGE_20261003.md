# 修复二次审查与 Windows 交付（2026-10-03）

## 结论

本轮对 `UNCOMMITTED_REVIEW_FIXES_20261003.md` 所述修复重新检查，发现并修正一项目录缓存淘汰问题。修正后未发现本批新增的阻塞问题，已完成 Windows 发布构建。

当前磁盘仍含此前的未提交改动；本次直接使用磁盘源码构建，没有新建提交。

## 发现与处理

**[P2，已修复] models_dev 目录缓存按 HashMap 随机顺序淘汰条目。**

并行回归首次运行时，`refresh_failure_never_replaces_valid_data` 失败：刚记录的 `network_error` 状态因新目录加入而被随机淘汰，重新读盘后丢失错误状态。目录文件和模型数据没有被损坏。

改为按最早检查时间淘汰，保留新近刷新结果与重试状态。新增使用独立局部缓存的 `path_cache_eviction_keeps_fresh_failure_status` 回归，验证淘汰规则，避免该断言依赖全局并行测试时序。

## 复查范围

- 推理缓存：Arc 快照、锁外签名和解析、固定长度指纹、内存与 TTL 边界、空 output/空白 reasoning、墓碑与凭据隔离、连续 assistant 工具边界。
- 配置接管：恢复后迁移标记、按字段路径比较冲突、安装 picker 的模型切换豁免、回滚点与用户数组行保留。
- 模型目录：max/ultra 产品型号、Nova 和 Amazon Bedrock 的品牌边界、聚合商 canonical 身份隔离、主机索引、账户 reasoning 校验及请求投影一致性。
- 前端与余额：按搜索匹配批量加入、待加入空态、窗口额度与余额区分、套餐说明、翻译命名空间。
- Windows 后台命令常量与发布程序 Windows subsystem。

## 本轮验证

| 项目 | 结果 |
|---|---|
| Rust 单元测试 | 585 通过，0 失败，10 忽略 |
| local_workflows | 9 通过 |
| 前端构建 | TypeScript / Vite 通过 |
| 前端回归 | 71 条 PASS |
| 发布构建 | MSVC Release + NSIS x64 成功 |
| 构建输入 | 355 个文件哈希，构建前后相同 |
| 成品 EXE 启动与窗口唤醒 | 5 项通过，stderr 0 字节 |
| EXE 校验 | 交付副本与 SHA256SUMS 一致 |

成品检查覆盖：创建主窗口与存活、重复启动恢复最小化窗口、唤醒期间关闭后保持隐藏、保留原有置顶状态、保留最大化尺寸。测试使用独立应用数据、Codex 和客户端目录，仅清理测试启动的进程。

首次 Rust 回归失败日志已由后续修正后的完整回归日志替换；失败详情记录在上面的发现中。最新日志在 `.runtime-test/review-repackage-*.log`。

## 交付

目录：`D:/workspace/AiMaMi/.runtime-test/deliverables/AgentHarbor-20261003-reviewed-203156`

- `AgentHarbor.exe`：直接运行。
- `AgentHarbor-Setup.exe`：Windows x64 安装程序。
- `SHA256SUMS.txt`、`build-manifest.json`：校验与构建记录。

## 保留限制

真实 DeepSeek/Gemini/Command Code 上游未联网验收；10 条外部环境测试未执行。安装包已成功生成，未在用户环境执行安装、升级或卸载。上一轮 Clippy `-D warnings` 报告的 45 项提示仍未整体清理，本轮没有宣称 Clippy 全绿。启动与唤醒测试不等于对所有后台命令瞬时控制台窗口的视觉验收。
