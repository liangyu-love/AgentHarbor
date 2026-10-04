# OAuth 登录取消与窗口唤醒修复（2026-10-03）

## 问题与实现

- 原登录只有 `codex login` 的成功、失败和五分钟超时，没有取消入口。现改为官方 app-server 的 `account/login/start`、`account/login/completed`、`account/login/cancel`，并由 AgentHarbor 打开官方授权 URL。
- 每次登录使用独立 UUID。后端只允许一个活动登录；取消可先于启动到达，旧取消不会影响新登录。取消等待进程与临时目录的清理；注册账号与取消共用短临界区，账号注册已完成时，迟到的取消返回 false，保留成功结果。
- 临时 CODEX_HOME 强制使用 file credential store，移除继承的 API key 环境变量，不覆盖用户原来的 auth.json 或 keyring。成功、取消、失败、超时均清理临时 home；正常取消不弹错误。
- 前端新增等待状态条、取消按钮、取消中提示，支持取消失败重试、取消后重新登录、旧成功/错误回调过滤和组件卸载清理。桌面与 390px 窄窗口均已检查；沿用现有视觉样式和共享按钮。
- Windows OAuth 优先运行原生 Codex CLI，支持新版 npm 平台包和旧版 vendor 目录。不存在的相对 `codex.exe` 不再遮住 PATH 的安装。OAuth 不通过 CMD shim 启动；仅有无法定位原生文件的非标准 shim 时，提示安装/更新 CLI。
- Windows 使用 ShellExecuteW 打开授权浏览器；后台 CLI 使用 CREATE_NO_WINDOW。CLI 置于 Job Object 中，终止时等待其自身及子进程退出，浏览器由父应用打开，不在被终止的 Job 中。
- 窗口唤醒记录首次置顶基线及递增 generation，仅最新延迟回调恢复基线；延迟回调不再 show/focus。用户在 180ms 脉冲期间隐藏窗口，窗口保持隐藏；原本持久置顶的窗口保持置顶。
- Windows 最终置顶恢复使用原生 SetWindowPos，仅修改 Z 顺序，避免 Tao 窗口标志更新重新 ShowWindow；基线也读取原生 WS_EX_TOPMOST。唤醒时仅对原生 IsIconic 为真的窗口使用 SW_RESTORE，避免窗口库状态采样竞争，保留原本最大化的尺寸。
- 启动延迟聚焦尊重用户提前隐藏或最小化的操作；macOS 延迟显示 Dock 也检查窗口可见性。
- 单实例激活标记改为原子写入，读取完整 UUID，并使用构建应用前捕获的基线跳过旧标记，同时保留初始化期间的新激活请求。

## 代码复审与修复

复审范围：新增 OAuth 状态与 worker、Tauri 命令、前端 hook/API/登录 UI、CLI 查找、窗口置顶脉冲、单实例标记、相关回归和 Windows 启动脚本。沿用 code-review-expert 的生命周期、共享状态、错误处理、凭据、进程资源检查。

| 发现 | 修复 |
| --- | --- |
| 结束 Windows Job 是异步动作，立即删除临时凭据目录可能与子进程退出竞争 | 显式终止 Job，查询 ActiveProcesses，等待清零后等待根进程，再清理目录；新增持有端口的子进程回归 |
| 启动延迟聚焦的可见性检查与 show 分属两个主线程回调，期间用户隐藏窗口仍可能被重开 | 在同一主线程回调中检查 visible/minimized 并显示窗口 |
| 授权启动响应返回后，取消可能已经到达，浏览器仍可能打开 | 打开浏览器前再次检查取消信号；最终账号注册前再次检查取消 |
| CMD/npm launcher 可在 Job 绑定之前产生后代进程 | OAuth 仅直接运行解析到的原生 CLI；识别新旧 npm native 布局，保留额度查询原有 shim fallback |
| 打包后的 Windows 测试发现最小化恢复有时序竞争，Tao 更新置顶标志可能根据缓存 VISIBLE 再次 ShowWindow | 唤醒时检查原生最小化状态，原生恢复最小化窗口；置顶脉冲使用不改变可见性和尺寸的原生 SetWindowPos，新增真实窗口关闭消息及最大化尺寸检查 |

复查后，当前范围没有待修复的 P0/P1 问题。移除旧 `codex login` 等待流程，避免双套登录路径；保留额度查询和已有配置能力。

## 验证证据

| 检查 | 结果与日志 |
| --- | --- |
| Rust 全库 | **574 passed / 0 failed / 10 ignored**；`.runtime-test/oauth-final-rust.log` |
| OAuth 模拟专项 | 8 passed，覆盖预取消、并发拒绝、旧取消隔离、取消等待清理、注册成功竞态、官方 URL 校验、成功/失败/超时/浏览器失败/无效凭据/无效 URL、取消后端口释放与重新登录、子进程回收；`.runtime-test/oauth-tests.log` |
| 已安装 Codex CLI | initialize + 对不存在 loginId 的 cancel 返回 notFound；强制 file store 被接受，未创建 auth.json、未启动授权；`.runtime-test/oauth-installed-cli.log` |
| 前端全库 | **69 PASS**；`.runtime-test/oauth-final-frontend.log` |
| 登录前端专项 | 5 PASS，覆盖取消等待、取消后再登录、旧成功/失败隔离、取消失败重试、超时恢复、成功与迟到取消、卸载取消、空账号入口；`.runtime-test/oauth-frontend.log` |
| 本地工作流 | **9 passed**；`.runtime-test/oauth-final-local-workflows.log` |
| TypeScript | `tsc --noEmit` exit 0；`.runtime-test/oauth-final-tsc.log` |
| 严格 lint | `cargo clippy --lib -- -D warnings` 被其他现有模块的 40 条 lint 阻止；本轮新增 OAuth/唤醒模块没有 lint 报告。保留完整 `.runtime-test/oauth-final-clippy.log`，未宣称全库 clippy 干净，也未批量改写无关代码 |
| UI | 两轮桌面/窄窗口截图，状态、文案、取消按钮、窄窗口无横向溢出；`.runtime-test/oauth-ui-round1`、`oauth-ui-round2`；Impeccable detector 为 `[]` |
| 新包 Windows 实机 | **5 PASS**：隔离启动存活/主窗口/空 stderr/非置顶，三次第二实例唤醒及清除临时置顶，脉冲期间关闭后保持隐藏，持久置顶保留，最大化尺寸保留；`.runtime-test/oauth-packaged-startup-wake-final.log`；只停止验收自己启动的进程 |

Rust 首轮并行全库遇到已有 models.dev 共享缓存测试的淘汰竞争：缓存上限 32，多个临时目录并发加载时可淘汰另一个测试的错误状态。该用例单独重跑通过；最终全库单线程通过。未修改无关目录匹配业务代码以掩盖测试竞争。

## 验证边界

- 未使用真实账号或凭据，未发起真实 OAuth；在线授权成功仍需要用户在浏览器完成一次实际登录。取消、临时凭据隔离与工具协议通过本地模拟和已安装 CLI 接口检查验证。
- Windows 新包的启动与多次单实例唤醒由 `scripts/test-windows-startup-wake.ps1` 在隔离目录验证，验收结果随包保存。
- 黑框排查确认后台命令使用隐藏标志，OAuth 去掉 CMD 与 CLI 自行打开浏览器的路径。没有在线启动真实授权浏览器，不能声称所有外部浏览器/第三方 launcher 行为都已实测。
- 不改已有未提交修复，不提交或回滚历史，不关闭用户运行中的旧软件与浏览器。

## 官方接口依据

- https://developers.openai.com/codex/app-server
- https://developers.openai.com/codex/auth
- Tauri 与 Tokio 文档通过 Context7 核对窗口主线程操作与子进程取消语义。
