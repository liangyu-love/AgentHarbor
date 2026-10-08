//! Claude Desktop 直连的端到端验收：预览 → 提交 → 漂移 → 恢复。
//!
//! 全部在临时根目录内完成，不读取也不写入真实的 `%LOCALAPPDATA%`。
//! 每个用例同时证明**供应商全局状态没有被牵动**：直连不写 Store、不改
//! `enabled` / `activeProviderId` / 路由开关。

use agentharbor_lib::core::claude_desktop::paths::DesktopPaths;
use agentharbor_lib::core::claude_desktop::types::{
    ActionRequest, ApplyDirectRequest, DesktopAction, OneMMode, PathsChoice,
};
use agentharbor_lib::core::claude_desktop::Manager;
use agentharbor_lib::core::newapi;
use agentharbor_lib::core::router::{self, ModelProtocol, ProviderInput, Store};
use agentharbor_lib::platform::paths::CodexPaths;
use std::fs;

const SENTINEL_KEY: &str = "sk-E2E-SENTINEL-DO-NOT-LEAK";

/// 构造一张测试卡片。
///
/// `Provider` 有 `pub(crate)` 字段，集成测试无法直接写字面量；借助 serde 默认值
/// 从 JSON 构造，既不落盘也不依赖内部构造器。
fn card(id: &str, base_url: &str, api_key: &str) -> agentharbor_lib::core::router::Provider {
    let value = serde_json::json!({
        "id": id,
        "name": id,
        "baseUrl": base_url,
        "apiKey": api_key,
        "protocol": "anthropicMessages",
        "model": "claude-haiku-4-5-20251001",
        "enabled": true,
        // 直连出口：测试必须绕开机器的全局代理，否则回环请求会被代理影响。
        "networkMode": "direct",
    });
    serde_json::from_value(value).expect("构造测试卡片")
}

// ---------------------------------------------------------------------------
// 只读诊断：针对本机真实卡片，永不写文件、永不打印密钥
/// 真实只读 Bearer 检测：对选定卡片 Desktop 实际会访问的 `/v1/models` 发起
/// 一次 GET。不发 Messages、不计费、不写任何文件。
///
/// `cargo test --test claude_desktop_workflows -- --ignored --nocapture real_bearer_probe`
#[test]
#[ignore = "联网只读检测；只在显式要求时运行"]
fn real_bearer_probe() {
    let wanted = std::env::var("AGENT_HARBOR_PROBE_PROVIDER").unwrap_or_default();
    let codex = CodexPaths::new();
    let store = router::load_store(&codex).expect("read real store");
    let settings = crate::load_settings(&codex);
    let provider = store
        .providers
        .iter()
        .find(|provider| provider.id == wanted || provider.name == wanted)
        .expect("provider not found by id or name");
    let models: Vec<String> = provider
        .models
        .iter()
        .filter(|model| model.included)
        .map(|model| model.id.clone())
        .collect();
    println!("provider={} baseUrl={}", provider.name, provider.base_url);
    println!("networkMode={}", provider.network_mode());
    let report = agentharbor_lib::core::claude_desktop::probe::run(
        provider,
        &models,
        &settings,
        "test-harness",
    )
    .expect("probe ran");
    println!("url={}", report.url);
    println!("status={}", report.status.as_str());
    println!("httpStatus={:?}", report.http_status);
    println!("listedModels={:?}", report.listed_models);
    println!("modelsVisible={:?}", report.models_visible);
    println!("modelsMissing={:?}", report.models_missing);
    println!("note={:?}", report.note);
}

/// 决定性兼容性检查：Desktop 在 `/v1/messages` 上**只发** `Authorization: Bearer`，
/// 决定性兼容性检查：Desktop 在 `/v1/messages` 上**只发** `Authorization: Bearer`。
/// 走 `probe::run_messages_check`，即产品里同一个受限实现。
///
/// 会产生极少量计费。只在显式要求时运行。
#[test]
#[ignore = "会发起一次最小计费 Messages 请求"]
fn real_messages_bearer() {
    let wanted = std::env::var("AGENT_HARBOR_PROBE_PROVIDER").unwrap_or_default();
    let model = std::env::var("AGENT_HARBOR_PROBE_MODEL").unwrap_or_default();
    let codex = CodexPaths::new();
    let store = router::load_store(&codex).expect("read real store");
    let provider = store
        .providers
        .iter()
        .find(|provider| provider.id == wanted || provider.name == wanted)
        .expect("provider not found");
    assert!(!model.is_empty(), "set AGENT_HARBOR_PROBE_MODEL");
    let proxy = load_settings(&codex);
    let report = agentharbor_lib::core::claude_desktop::probe::run_messages_check(
        provider, &model, &proxy,
    )
    .expect("check ran");
    println!("url={}", report.url);
    println!("model={}", report.model);
    println!("status={}", report.status.as_str());
    println!("httpStatus={:?}", report.http_status);
    println!("responseModel={:?}", report.response_model);
    println!("stopReason={:?}", report.stop_reason);
    println!("note={:?}", report.note);
}


// ---------------------------------------------------------------------------
// 桌面应用进程控制（只读/隔离）
// ---------------------------------------------------------------------------

/// 启动方式判定必须只接受已核实的包族名与 `!Claude` 应用 id，并且必须把
/// Claude Code CLI 的路径排除在外——否则重启会去杀 Desktop 的工具子进程。
#[test]
fn restart_never_targets_the_claude_code_cli() {
    use agentharbor_lib::core::claude_desktop::desktop_app::{
        is_cli_path, is_desktop_process_name, is_msix_app_id,
    };
    assert!(is_msix_app_id("Claude_pzs8sxrjxfjjc!Claude"));
    assert!(!is_msix_app_id("Claude_pzs8sxrjxfjjc!SshProxy"));
    assert!(is_desktop_process_name("Claude"));
    assert!(!is_desktop_process_name("claude-code"));
    assert!(is_cli_path(
        r"C:\Users\u\AppData\Local\Claude-3p\claude-code\2.1.286\claude.exe"
    ));
}

/// 路径覆盖下没有确认到启动方式时，状态必须如实报告「不可重启」，而不是猜一个目标。
#[test]
fn an_unknown_installation_reports_no_restart_capability() {
    let fixture = fixture("restartCapability");
    // 用临时根覆盖的双根 + 明确注入的版本：既不扫描平台目录，也不读取生产日志。
    let status = fixture.manager.status(&Store::default());
    assert!(status.launch_method.is_none(), "覆盖根下不得解析出启动方式");
    assert!(!status.restart_capable);
    // 属性名与 TS DTO 一致（camelCase 序列化）。
    let rendered = serde_json::to_value(&status).unwrap();
    assert!(rendered.get("restartCapable").is_some());
    assert!(rendered.get("appRunning").is_some());
}

// ---------------------------------------------------------------------------
// 真实接管（用户显式授权；会改动本机四文件并重启应用）
// ---------------------------------------------------------------------------

/// `cargo test --test claude_desktop_workflows -- --ignored --nocapture real_apply_direct`
///
/// 先决条件：由使用者自己备份四文件。这里完全走产品的 prepare → ticket → commit
/// 路径，然后重启 Claude Desktop，使新配置立即生效。
#[test]
#[ignore = "会改动本机 Claude Desktop 配置并重启应用"]
fn real_apply_direct() {
    let codex = CodexPaths::new();
    let store = router::load_store(&codex).expect("read real store");
    let manager = Manager::new(&codex.app_data_dir);
    let paths = manager
        .resolve_paths(&PathsChoice::default())
        .expect("resolved desktop paths");
    println!("normalRoot={}", paths.normal_root.display());
    println!("thirdPartyRoot={}", paths.third_party_root.display());

    // 应用前状态。
    println!("--- before ---");
    for path in [
        &paths.normal_config,
        &paths.third_party_config,
        &paths.meta_path,
        &paths.profile_path,
    ] {
        println!(
            "{}: {}",
            path.display(),
            std::fs::read_to_string(path).unwrap_or_else(|_| "<absent>".to_owned())
        );
    }

    let provider_id = std::env::var("AGENT_HARBOR_PROBE_PROVIDER").unwrap_or_default();
    let model = std::env::var("AGENT_HARBOR_PROBE_MODEL").unwrap_or_default();
    let provider = store
        .providers
        .iter()
        .find(|entry| entry.id == provider_id || entry.name == provider_id)
        .expect("provider not found");
    println!("provider={} model={}", provider.name, model);
    assert!(!model.is_empty(), "set AGENT_HARBOR_PROBE_MODEL");

    // 支持多个模型：`AGENT_HARBOR_PROBE_MODEL="a,b,c"`，可带 `id=显示名`。
    let models: Vec<agentharbor_lib::core::claude_desktop::types::DesktopModelChoice> = model
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let (id, label) = match entry.split_once('=') {
                Some((id, label)) => (id.trim().to_owned(), Some(label.trim().to_owned())),
                None => (entry.to_owned(), None),
            };
            agentharbor_lib::core::claude_desktop::types::DesktopModelChoice {
                model_id: id,
                label_override: label,
                one_m_mode: OneMMode::Auto,
            }
        })
        .collect();
    println!("models to expose={}", models.len());
    let rebaseline = std::env::var("AGENT_HARBOR_PROBE_REBASELINE")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    println!("rebaseline={rebaseline}");
    let request = || ActionRequest {
        provider_id: Some(provider.id.clone()),
        models: models.clone(),
        paths: PathsChoice::default(),
        rebaseline,
        routes: Vec::new(),
        rotate_gateway_token: false,
    };

    let prepared = manager
        .prepare(&store, DesktopAction::ApplyDirect, &request())
        .expect("prepare");
    println!("revision={}", prepared.revision);
    println!("messagesUrl={:?}", prepared.messages_url);
    println!("warnings={:?}", prepared.warnings);
    println!("conflicts={:?}", prepared.conflicts);
    for plan in &prepared.plans {
        let existed = prepared
            .previous_state
            .as_ref()
            .and_then(|state| state.edition(&plan.role))
            .map(|edition| edition.existed_before)
            .unwrap_or(false);
        println!(
            "plan {} {} -> {}",
            plan.role,
            plan.kind(existed),
            plan.path.display()
        );
    }
    let preview = manager.issue_ticket(&prepared).expect("issue ticket");

    let applied = manager
        .commit(
            &store,
            DesktopAction::ApplyDirect,
            &preview.revision,
            &request(),
        )
        .expect("commit");
    println!("--- applied ---");
    println!("transactionId={}", applied.transaction_id);
    println!("backupId={:?}", applied.backup_id);
    println!("requiresRestart={}", applied.requires_restart);
    println!("changedPaths={:?}", applied.changed_paths);
    println!("warnings={:?}", applied.warnings);

    println!("--- after ---");
    for path in [
        &paths.normal_config,
        &paths.third_party_config,
        &paths.meta_path,
        &paths.profile_path,
    ] {
        println!(
            "{}: {}",
            path.display(),
            std::fs::read_to_string(path).unwrap_or_else(|_| "<absent>".to_owned())
        );
    }

    let status = manager.status(&store);
    println!("--- status ---");
    println!(
        "state={} mode={} restartRequired={} appRunning={} restartCapable={} launchMethod={:?}",
        status.management_state,
        status.mode,
        status.restart_required,
        status.app_running,
        status.restart_capable,
        status.launch_method
    );
    assert_eq!(status.management_state, "managed");

    // 重启应用：先完全退出，再启动。
    let method = manager.restart_app(&paths).expect("restart app");
    println!("--- restarted via {method} ---");
    let after = manager.app_state(&paths);
    println!("running={} count={}", after.running, after.process_count);
    assert!(after.running, "重启后应探测到 Desktop 进程");
}

/// 一次性验证网关上中转：把 Desktop 指向本地回环 mock，由它记录收到的请求
/// （路径、鉴权头有无、模型名），再转发给真实上游，最后把答案回给 Desktop。
///
/// 这样就能**测到**「Desktop 发的是裸 Bearer，模型名原样出站」这条最关键的结论，
/// 而不是从日志推断。
///
/// `cargo test --test claude_desktop_workflows -- --ignored --nocapture verify_relay`
#[test]
#[ignore = "会临时改写 Desktop 配置、起本地回环 mock 并转发真实请求"]
fn verify_relay() {
    use std::io::Write;
    use std::net::TcpListener;

    let codex = CodexPaths::new();
    let store = router::load_store(&codex).expect("read real store");
    let manager = Manager::new(&codex.app_data_dir);
    let paths = manager
        .resolve_paths(&PathsChoice::default())
        .expect("resolved paths");
    let provider_id = std::env::var("AGENT_HARBOR_PROBE_PROVIDER").unwrap_or_default();
    let _model = std::env::var("AGENT_HARBOR_PROBE_MODEL").unwrap_or_default();
    let provider = store
        .providers
        .iter()
        .find(|entry| entry.id == provider_id || entry.name == provider_id)
        .expect("provider not found");
    let upstream = provider.base_url.trim_end_matches('/').to_owned();
    let upstream = upstream.strip_suffix("/v1").unwrap_or(&upstream).to_owned();
    let real_key = provider.api_key.trim().to_owned();

    // 本地 mock：只接受 127.0.0.1 上的连接。
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().unwrap().port();
    println!("MOCK=http://127.0.0.1:{port}");
    println!("UPSTREAM={upstream}");

    let mock = std::thread::spawn(move || {
        let mut captured: Vec<String> = Vec::new();
        listener
            .set_nonblocking(true)
            .expect("non-blocking listener");
        // 只在有窗口期内工作；期间接受**不限数量**的连接，绝不主动断开，
        // 避免把「正在使用的 Desktop」置于无法连通的中间态。
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
        while std::time::Instant::now() < deadline {
            let (mut socket, _) = match listener.accept() {
                Ok(pair) => pair,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    continue;
                }
                Err(_) => break,
            };
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(120)))
                .ok();
            // 按 Content-Length 读**完整**请求体：只读一次会截断大请求
            // （系统提示可达数万字符），转发上去必被上游拒绝。
            let Some(request) = read_full_request(&mut socket) else {
                continue;
            };
            let head = request.split("\r\n\r\n").next().unwrap_or("").to_owned();
            let path = head.lines().next().unwrap_or("").to_owned();
            let auth = head
                .lines()
                .find(|line| line.to_ascii_lowercase().starts_with("authorization:"))
                .map(|line| line.trim().to_owned());
            let has_api_key = head
                .lines()
                .any(|line| line.to_ascii_lowercase().starts_with("x-api-key:"));
            let body = request.split("\r\n\r\n").nth(1).unwrap_or("").to_owned();
            let sent_model = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|value| value.get("model").and_then(|m| m.as_str()).map(str::to_owned));
            let version = head
                .lines()
                .find(|line| line.to_ascii_lowercase().starts_with("anthropic-version:"))
                .map(|line| line.trim().to_owned());
            let stream = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|value| value.get("stream").and_then(|s| s.as_bool()))
                .unwrap_or(false);
            captured.push(format!(
                "MOCK_CAPTURE path={path} authPresent={} authPrefix={:?} xApiKeyPresent={has_api_key} model={sent_model:?} version={version:?} stream={stream}",
                auth.is_some(),
                auth.as_deref().map(|a| a.split_whitespace().next().unwrap_or("")),
            ));

            // 转发给真实上游，并原样回给 Desktop。
            let url = format!("{upstream}{}", path.split_whitespace().nth(1).unwrap_or("/"));
            let reply = forward(&url, &head, &body, &real_key);
            let _ = socket.write_all(&reply);
            let _ = socket.flush();
            // 每条记录后立刻打印，便于在窗口期内观察。
            println!("{}", captured.last().unwrap());
        }
        captured
    });

    // 把 Desktop 指向 mock：这是验证手段，**无论成功、失败还是 panic 都必须还原**，
    // 否则会把正在使用的 Desktop 留在连不通的中间态。
    let profile = std::fs::read_to_string(&paths.profile_path).expect("read profile");
    let restore = ProfileRestore {
        path: paths.profile_path.clone(),
        original: profile.clone(),
    };
    let mut value: serde_json::Value = serde_json::from_str(&profile).expect("parse profile");
    value["inferenceGatewayBaseUrl"] = serde_json::json!(format!("http://127.0.0.1:{port}"));
    std::fs::write(
        &paths.profile_path,
        serde_json::to_string_pretty(&value).unwrap(),
    )
    .expect("write temp profile");
    println!("PROFILE_POINTED_AT_MOCK");

    // 重启 Desktop，让它读取指向 mock 的 profile。
    manager.restart_app(&paths).expect("restart via mock");
    println!("DESKTOP_RESTARTED; waiting for the user to send a message...");

    // 等用户发消息；mock 线程会记录并转发。
    let captured = mock.join().unwrap_or_default();
    println!("--- captures ---");
    for line in &captured {
        println!("{line}");
    }

    // 还原 profile 并重启回真实上游（drop 也会兜底）。
    restore.restore();
    println!("PROFILE_RESTORED");
    let _ = manager.restart_app(&paths);
    println!("DESKTOP_RESTARTED_BACK_TO_REAL_RELAY");
    assert!(!captured.is_empty(), "没有捕获到任何请求");
}

/// 验证期间临时指向 mock 的 profile 必须在离开作用域时写回。
struct ProfileRestore {
    path: std::path::PathBuf,
    original: String,
}

impl ProfileRestore {
    fn restore(&self) {
        let _ = std::fs::write(&self.path, &self.original);
    }
}

impl Drop for ProfileRestore {
    fn drop(&mut self) {
        self.restore();
    }
}

/// 读一条完整的 HTTP 请求：先读头部，再按 `Content-Length` 读满请求体。
///
/// 只调用一次 `read()` 会把大请求截断（Desktop 的系统提示可达数万字符），
/// 截断的 JSON 转发给上游必然被拒——这会污染正在使用的会话。
fn read_full_request(socket: &mut std::net::TcpStream) -> Option<String> {
    use std::io::Read;
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 16384];
    let head_end = loop {
        let read = socket.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = find_head_end(&buffer) {
            break position;
        }
        if buffer.len() > 32 * 1024 * 1024 {
            return None;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..head_end]).to_string();
    let content_length = head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse::<usize>().ok())
            .flatten()
    });
    let body_start = head_end + 4;
    if let Some(length) = content_length {
        let target = body_start + length;
        while buffer.len() < target {
            let read = socket.read(&mut chunk).ok()?;
            if read == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..read]);
        }
        if buffer.len() < target {
            return None;
        }
    }
    String::from_utf8(buffer).ok()
}

/// 头部结束（`\r\n\r\n`）的位置。
fn find_head_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

/// 把请求转发给真实上游并返回完整的 HTTP 响应。
fn forward(url: &str, head: &str, body: &str, real_key: &str) -> Vec<u8> {
    use std::io::Read;
    use std::time::Duration;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("client");
    let stream = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value.get("stream").and_then(|s| s.as_bool()))
        .unwrap_or(false);
    let mut request = client
        .post(url)
        .header("content-type", "application/json")
        // 用真实上游凭据替换 mock 收到的那个（可能是 Desktop 的任意值）。
        .header("Authorization", format!("Bearer {real_key}"))
        .body(body.to_owned());
    if head
        .lines()
        .any(|line| line.to_ascii_lowercase().starts_with("anthropic-version:"))
    {
        request = request.header("anthropic-version", "2023-06-01");
    }
    if stream {
        request = request.header("accept", "text/event-stream");
    }
    match request.send() {
        Ok(response) => {
            let status = response.status();
            let content_type = response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok())
                .unwrap_or("application/json")
                .to_owned();
            let mut payload = Vec::new();
            let _ = response.take(8 * 1024 * 1024).read_to_end(&mut payload);
            let mut out = format!(
                "HTTP/1.1 {} {}\r\ncontent-type: {}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                status.as_u16(),
                status.canonical_reason().unwrap_or("OK"),
                content_type,
                payload.len()
            )
            .into_bytes();
            out.extend_from_slice(&payload);
            out
        }
        Err(error) => format!(
            "HTTP/1.1 502 Bad Gateway\r\ncontent-type: application/json\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{{}}",
        )
        .replace("{}", &format!("{{\"error\":\"{}\"}}", error.to_string().replace('"', "")))
        .into_bytes(),
    }
}

// ---------------------------------------------------------------------------
// 上游能力逐项核对（只读探测，不改任何配置）
// ---------------------------------------------------------------------------
/// 逐项核对上游能力。实现放在 probe 模块内（那里能复用项目的受限 HTTP 客户端），
/// 这里只负责选卡片、打印结果。
///
/// `cargo test --test claude_desktop_workflows -- --ignored --nocapture probe_upstream_capabilities`
#[test]
#[ignore = "会向真实上游发起少量计费请求"]
fn probe_upstream_capabilities() {
    let codex = CodexPaths::new();
    let store = router::load_store(&codex).expect("read real store");
    let provider_id = std::env::var("AGENT_HARBOR_PROBE_PROVIDER").unwrap_or_default();
    let model = std::env::var("AGENT_HARBOR_PROBE_MODEL").unwrap_or_default();
    let provider = store
        .providers
        .iter()
        .find(|entry| entry.id == provider_id || entry.name == provider_id)
        .expect("provider not found");
    assert!(!model.is_empty(), "set AGENT_HARBOR_PROBE_MODEL");
    let proxy = load_settings(&codex);
    let report = agentharbor_lib::core::claude_desktop::capability::probe_capabilities(
        provider,
        &model,
        &proxy,
        &codex.app_data_dir,
    )
    .expect("capability probe ran");

    println!("UPSTREAM_MODELS={}", report.available_models.join(","));
    println!("DESKTOP_SELECTABLE={}", report.selectable_models.join(","));
    println!("STREAM status={:?} contentType={:?}", report.stream_status, report.stream_content_type);
    println!("STREAM events={}", report.stream_events.join(","));
    println!(
        "STREAM messageStart={} contentDelta={} messageStop={} error={}",
        report.stream_message_start,
        report.stream_content_delta,
        report.stream_message_stop,
        report.stream_error
    );
    println!("TOOL status={:?} stopReason={:?}", report.tool_status, report.tool_stop_reason);
    println!("TOOL blockTypes={}", report.tool_block_types.join(","));
    println!("TOOL hasToolUse={}", report.tool_has_use);
    println!("TOOL errorType={:?}", report.tool_error_type);
}

// ---------------------------------------------------------------------------
// L10：密钥变更 → stale → 重新应用 → 恢复（真实机器）
// ---------------------------------------------------------------------------

/// `cargo test --test claude_desktop_workflows -- --ignored --nocapture real_l10_key_rotation`
///
/// 全程只打印密钥**指纹**与长度，不打印密钥本身。轮换用的是一次性假密钥，
/// 结束前一定还原卡片原密钥（用 `Drop` 兜底，断言失败也会还原）。
#[test]
#[ignore = "会临时改写卡片密钥与 Desktop 配置，随后还原"]
fn real_l10_key_rotation() {
    let codex = CodexPaths::new();
    let store = router::load_store(&codex).expect("read real store");
    let provider_id = std::env::var("AGENT_HARBOR_PROBE_PROVIDER").unwrap_or_default();
    let provider = store
        .providers
        .iter()
        .find(|entry| entry.id == provider_id || entry.name == provider_id)
        .expect("provider not found");
    let original_key = provider.api_key.clone();
    let original_base = provider.base_url.clone();
    assert!(!original_key.trim().is_empty(), "该卡片必须有推理密钥");
    println!("provider={} keyLen={}", provider.name, original_key.trim().len());

    let manager = Manager::new(&codex.app_data_dir);
    let paths = manager
        .resolve_paths(&PathsChoice::default())
        .expect("resolved paths");

    // 还原守卫：整套 provider store 文件级快照 + 还原。
    //
    // **不要**用 `save_provider` 只回写密钥：它对未提供的字段会写入空值，
    // 会把卡片的名称、模型列表、余额凭据一起抹掉（这个错误真发生过一次）。
    // 文件级快照是唯一能保证「原样还原」的做法。
    struct StoreSnapshot {
        live: std::path::PathBuf,
        bytes: Vec<u8>,
    }
    impl StoreSnapshot {
        fn take(codex: &CodexPaths) -> Self {
            let live = codex.app_data_dir.join("newapi-providers.dat");
            let bytes = fs::read(&live).expect("read provider store");
            Self { live, bytes }
        }
    }
    impl Drop for StoreSnapshot {
        fn drop(&mut self) {
            let _ = fs::write(&self.live, &self.bytes);
        }
    }
    let _restore = StoreSnapshot::take(&codex);

    // 1) 基线：当前应当是 managed，且已记录指纹。
    let before = manager.status(&store);
    println!("before: state={} stale={:?}", before.management_state, before.stale_reasons);
    assert_eq!(before.management_state, "managed", "先要有一份已应用的配置");
    let recorded = manager
        .load_state()
        .unwrap()
        .and_then(|state| state.credential_fingerprint)
        .expect("状态里应记录指纹");
    assert_eq!(recorded.len(), 64);

    // 2) 轮换密钥（一次性假值），观察 stale。
    let rotated_key = format!("sk-L10-ROTATION-{}", std::process::id());
    let mut rotated = provider.clone();
    rotated.api_key = rotated_key.clone();
    let rotated_store = {
        let mut copy = store.clone();
        if let Some(entry) = copy.providers.iter_mut().find(|entry| entry.id == provider.id) {
            entry.api_key = rotated_key.clone();
        }
        copy
    };
    let stale = manager.status(&rotated_store);
    println!(
        "after rotation: state={} stale={:?}",
        stale.management_state, stale.stale_reasons
    );
    assert_eq!(stale.management_state, "stale");
    assert!(
        stale.stale_reasons.contains(&"credential".to_owned()),
        "必须报 credential stale: {:?}",
        stale.stale_reasons
    );

    // 3) 把卡片真正改成新密钥，然后重新应用。
    router::save_provider(
        &codex,
        ProviderInput {
            id: Some(provider.id.clone()),
            name: provider.name.clone(),
            base_url: original_base.clone(),
            user_id: None,
            access_token: String::new(),
            api_key: rotated_key.clone(),
            protocol: ModelProtocol::AnthropicMessages,
            model: provider.model.clone(),
            models: provider.models.clone(),
            context_window: None,
            auto_compact_limit: None,
            catalog_providers: None,
            price_catalog_provider: None,
            price_multiplier: None,
            compression: None,
            network_mode: provider.network_mode().to_owned(),
        },
    )
    .expect("rotate key");
    let rotated_store = router::load_store(&codex).unwrap();
    let models: Vec<agentharbor_lib::core::claude_desktop::types::DesktopModelChoice> = manager
        .load_state()
        .unwrap()
        .map(|state| {
            state
                .models
                .iter()
                .map(|model| {
                    agentharbor_lib::core::claude_desktop::types::DesktopModelChoice {
                        model_id: model.model_id.clone(),
                        label_override: Some(model.label.clone()),
                        one_m_mode: OneMMode::Auto,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let request = ActionRequest {
        provider_id: Some(provider.id.clone()),
        models,
        paths: PathsChoice::default(),
        rebaseline: false,
        routes: Vec::new(),
        rotate_gateway_token: false,
    };
    let prepared = manager
        .prepare(&rotated_store, DesktopAction::Reapply, &request)
        .expect("prepare reapply");
    let preview = manager.issue_ticket(&prepared).expect("ticket");
    manager
        .commit(
            &rotated_store,
            DesktopAction::Reapply,
            &preview.revision,
            &request,
        )
        .expect("reapply with the new key");
    let profile = std::fs::read_to_string(&paths.profile_path).expect("profile");
    assert!(
        profile.contains(&rotated_key),
        "重新应用后 profile 必须换成新密钥"
    );
    assert!(!profile.contains(original_key.trim()), "旧密钥不得残留");
    let after = manager.status(&rotated_store);
    println!("after reapply: state={} stale={:?}", after.management_state, after.stale_reasons);
    assert_eq!(after.management_state, "managed");

    // 4) 恢复接管前配置：自身 profile 必须消失，密钥不残留。
    {
        let state = manager.load_state().unwrap().expect("state");
        for edition in &state.data.files {
            if edition.role == "profile" {
                println!(
                    "BEFORE RESTORE profile existed={} baseline_is_none={}",
                    edition.existed_before,
                    matches!(
                        &edition.baseline,
                        agentharbor_lib::core::claude_desktop::transaction::ManagedValues::Whole { value: None }
                    )
                );
            }
        }
        println!("BEFORE RESTORE provider_id={}", state.provider_id);
    }
    let restore = ActionRequest {
        provider_id: None,
        models: vec![],
        paths: PathsChoice::default(),
        rebaseline: false,
        routes: Vec::new(),
        rotate_gateway_token: false,
    };
    let prepared = manager
        .prepare(&rotated_store, DesktopAction::RestorePrevious, &restore)
        .expect("prepare restore");
    println!("restore plans={}", prepared.plans.len());
    for plan in &prepared.plans {
        let existed = prepared
            .previous_state
            .as_ref()
            .and_then(|state| state.edition(&plan.role))
            .map(|edition| edition.existed_before)
            .unwrap_or(false);
        println!(
            "  plan {} {} -> {}",
            plan.role,
            plan.kind(existed),
            plan.path.display()
        );
    }
    println!("restore warns={:?} conflicts={:?}", prepared.warnings, prepared.conflicts);
    let preview = manager.issue_ticket(&prepared).expect("ticket");
    manager
        .commit(
            &rotated_store,
            DesktopAction::RestorePrevious,
            &preview.revision,
            &restore,
        )
        .expect("restore");
    println!("profileExists={}", paths.profile_path.exists());
    assert!(!paths.profile_path.exists(), "恢复后自身 profile 必须被撤回");
    let meta = std::fs::read_to_string(&paths.meta_path).unwrap_or_default();
    assert!(!meta.contains(&rotated_key), "meta 不得残留密钥");
    assert!(!meta.contains(original_key.trim()), "meta 不得残留旧密钥");
    for path in [&paths.normal_config, &paths.third_party_config] {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        assert!(!text.contains(&rotated_key), "{} 不得残留密钥", path.display());
        assert!(!text.contains(original_key.trim()), "{} 不得残留旧密钥", path.display());
    }
    println!("L10: 无密钥残留");
}

// ---------------------------------------------------------------------------
// L09：中转的鉴权方案或端点布局与 Desktop 直连不兼容
// ---------------------------------------------------------------------------

#[test]
#[ignore = "本地回环连通性自检"]
fn real_l09_loopback_sanity() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    println!("binding on {port}");
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().expect("accept");
        let mut buf = [0u8; 2048];
        let n = socket.read(&mut buf).unwrap_or(0);
        println!("SERVER saw {} bytes", n);
        let body = "{}";
        let resp = format!("HTTP/1.1 401 Unauthorized\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
        let _ = socket.write_all(resp.as_bytes());
        let _ = socket.flush();
    });
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .no_proxy()
        .build()
        .expect("client");
    let started = std::time::Instant::now();
    let result = client
        .get(format!("http://127.0.0.1:{port}/v1/models"))
        .header("Authorization", "Bearer sk-x")
        .send();
    println!("CLIENT result={:?} in {:?}", result.map(|r| r.status().as_u16()), started.elapsed());
    let _ = server.join();
}

/// 中转只接受 `x-api-key`（真实形态：上游对 Bearer 返回 401/403）。
///
/// 通过标准是「如实报告鉴权被拒，而不是把它标成 Desktop 已验证可用」。
/// 这里用本地回环 mock，不依赖某张真实的卡片，也不计费。
///
/// `cargo test --test claude_desktop_workflows -- --ignored --nocapture real_l09_auth_scheme`
#[test]
#[ignore = "需要本地回环 mock；不需要真实上游"]
fn real_l09_auth_scheme_rejects_bearer() {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    // 只接受 x-api-key 的网关：无论收到什么一律 401，并记录用到的鉴权头。
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().unwrap().port();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let gateway_stop = stop.clone();
    let gateway = std::thread::spawn(move || {
        use std::sync::atomic::Ordering;
        let mut seen_x_api_key = 0usize;
        let mut seen_bearer = 0usize;
        // 非阻塞 + 停止标志：不能为「预期之外的第 N 个连接」一直等下去。
        listener.set_nonblocking(true).expect("non-blocking");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !gateway_stop.load(Ordering::Relaxed) && std::time::Instant::now() < deadline {
            let (mut socket, _) = match listener.accept() {
                Ok(pair) => pair,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    continue;
                }
                Err(_) => break,
            };
            let mut buffer = vec![0u8; 8192];
            let read = socket.read(&mut buffer).unwrap_or(0);
            let request = String::from_utf8_lossy(&buffer[..read]).to_ascii_lowercase();
            if request.contains("x-api-key:") {
                seen_x_api_key += 1;
            }
            if request.contains("authorization: bearer") {
                seen_bearer += 1;
            }
            // 无论怎样都拒绝：模拟「只认 x-api-key」的网关对 Bearer 的态度。
            let body = r#"{"error":{"type":"authentication_error"}}"#;
            let response = format!(
                "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = socket.write_all(response.as_bytes());
            let _ = socket.flush();
        }
        (seen_x_api_key, seen_bearer)
    });

    let card = card(
        "l09-auth",
        &format!("http://127.0.0.1:{port}"),
        "sk-l09-bearer-only",
    );
    let model = "claude-haiku-4-5-20251001".to_owned();

    // 地址本身是可投影的：卡片刻度上不会被静态规则挡住。
    let (root, messages, _models) =
        agentharbor_lib::core::claude_desktop::provider::resolve_direct_endpoints(&card)
            .expect("回环地址应当可以投影");
    assert!(root.starts_with("http://127.0.0.1"));
    assert!(messages.ends_with("/v1/messages"));

    let proxy = load_settings(&CodexPaths::new());
    let report = agentharbor_lib::core::claude_desktop::probe::run(
        &card,
        std::slice::from_ref(&model),
        &proxy,
        "127.0.0.1",
    )
    .expect("探测本身不应报错，只应给出状态");
    println!("bearer status={} note={:?}", report.status.as_str(), report.note);
    assert_eq!(
        report.status.as_str(),
        "authRejected",
        "只认 x-api-key 的网关必须被报告为鉴权被拒，而不是可用"
    );

    // 上报里不得出现密钥或响应正文。
    let rendered = serde_json::to_string(&report).unwrap();
    assert!(!rendered.contains("sk-l09-bearer-only"));
    assert!(!rendered.contains("authentication_error"));

    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let (seen_x_api_key, seen_bearer) = gateway.join().expect("mock thread");
    println!("mock saw xApiKey={seen_x_api_key} bearer={seen_bearer}");
    assert!(seen_bearer > 0, "产品必须使用 Bearer 发探测请求");
    assert_eq!(seen_x_api_key, 0, "产品不得使用 x-api-key");
}

/// 端点布局不兼容：Desktop 只会追加 `v1/...`，与项目自身的投影不一致。
///
/// 通过标准是「在预览阶段就报告布局限制」，而不是写下一份指向错误地址的 profile。
#[test]
#[ignore = "不需要外部依赖，验证静态投影规则"]
fn real_l09_endpoint_layout_is_reported() {
    use agentharbor_lib::core::claude_desktop::{provider, types::error_code};

    let card = |base: &str| card("l09-layout", base, "sk-l09");

    for base in [
        "https://api.z.ai/api/paas/v4",
        "https://open.bigmodel.cn/api/paas/v4",
        "https://api.novita.ai/v3/openai",
        "https://relay.example/v1/messages",
    ] {
        let error = provider::resolve_direct_endpoints(&card(base))
            .expect_err("不兼容的布局必须被拒绝");
        let expected = if base.ends_with("/v1/messages") {
            error_code::INVALID_URL
        } else {
            error_code::ENDPOINT_UNSUPPORTED
        };
        assert_eq!(error.code, expected, "{base}: {} {:?}", error.code, error.params);
        println!("{base} -> {} {:?}", error.code, error.params);
    }

    // 卡片刻度据此判定不可用，并带上可读原因。
    // 空目录：不联网、不含模型族推断表。
    let catalog = agentharbor_lib::core::models_dev::load(std::path::Path::new(
        "target/l09-empty-catalog-never-created",
    ));
    let policy = provider::DesktopVersionPolicy::conservative();
    let (eligible, reason, models) =
        provider::provider_qualification(&card("https://api.z.ai/api/paas/v4"), policy, &catalog);
    assert!(!eligible, "布局不兼容的卡片不得判定为可用");
    println!("eligible={eligible} reason={reason:?} models={}", models.len());
    assert!(reason.is_some_and(|text| text.contains(error_code::ENDPOINT_UNSUPPORTED)));
}

// ---------------------------------------------------------------------------
// L07：切换官方模式（真实机器，随后立即恢复直连）
// ---------------------------------------------------------------------------

/// `cargo test --test claude_desktop_workflows -- --ignored --nocapture real_l07_switch_official`
///
/// 真实执行「切换官方模式」→ 核对四文件 → **立即重新应用直连**。全程不重启 Desktop，
/// 因此不会把应用留在官方登录流程里。四个 Desktop 文件与内部状态都有文件级快照，
/// 断言失败也会原样还原。
#[test]
#[ignore = "会临时把 Desktop 切到官方模式，随后立即恢复并还原快照"]
fn real_l07_switch_official() {
    let codex = CodexPaths::new();
    let manager = Manager::new(&codex.app_data_dir);
    let paths = manager
        .resolve_paths(&PathsChoice::default())
        .expect("resolved paths");

    // 文件级快照：比回写字段安全，见 L10 里那次真实教训。
    struct Snapshot {
        files: Vec<(std::path::PathBuf, Option<Vec<u8>>)>,
    }
    impl Snapshot {
        fn take(paths: &DesktopPaths, app_data_dir: &std::path::Path) -> Self {
            let mut files = Vec::new();
            for path in [
                &paths.normal_config,
                &paths.third_party_config,
                &paths.meta_path,
                &paths.profile_path,
            ] {
                files.push((path.clone(), fs::read(path).ok()));
            }
            files.push((app_data_dir.join("claude-desktop/state.dat"), fs::read(app_data_dir.join("claude-desktop/state.dat")).ok()));
            files.push((app_data_dir.join("claude-desktop/pending.dat"), fs::read(app_data_dir.join("claude-desktop/pending.dat")).ok()));
            Self { files }
        }
    }
    impl Drop for Snapshot {
        fn drop(&mut self) {
            for (path, bytes) in &self.files {
                match bytes {
                    Some(bytes) => {
                        let _ = fs::write(path, bytes);
                    }
                    None => {
                        let _ = fs::remove_file(path);
                    }
                }
            }
        }
    }
    let _snapshot = Snapshot::take(&paths, &codex.app_data_dir);

    // 记录切换前的非受管键，用于核对它们不被触碰。
    let before: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&paths.third_party_config).expect("3p config"))
            .expect("3p config json");
    let foreign_before = before["preferences"].clone();
    let cowork_before = before["coworkUserFilesPath"].clone();
    assert!(foreign_before.is_object(), "切换前应有 preferences 作为非受管键");

    let store = router::load_store(&codex).expect("read real store");
    let models: Vec<agentharbor_lib::core::claude_desktop::types::DesktopModelChoice> = manager
        .load_state()
        .unwrap()
        .map(|state| {
            state
                .models
                .iter()
                .map(|model| {
                    agentharbor_lib::core::claude_desktop::types::DesktopModelChoice {
                        model_id: model.model_id.clone(),
                        label_override: Some(model.label.clone()),
                        one_m_mode: OneMMode::Auto,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let provider_id = manager
        .load_state()
        .unwrap()
        .map(|state| state.provider_id)
        .expect("先要有一份已应用的直连配置");

    // 1) 切换官方模式。
    let switch_request = ActionRequest {
        provider_id: None,
        models: vec![],
        paths: PathsChoice::default(),
        rebaseline: false,
        routes: Vec::new(),
        rotate_gateway_token: false,
    };
    let prepared = manager
        .prepare(&store, DesktopAction::SwitchOfficial, &switch_request)
        .expect("prepare switchOfficial");
    let preview = manager.issue_ticket(&prepared).expect("ticket");
    let applied = manager
        .commit(
            &store,
            DesktopAction::SwitchOfficial,
            &preview.revision,
            &switch_request,
        )
        .expect("switchOfficial");
    println!("switched: {:?}", applied);

    // 2) 核对：两处 config 都是 1p，自身条目与 appliedId 已撤回，profile 已删除。
    for path in [&paths.normal_config, &paths.third_party_config] {
        let value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(path).expect("config")).expect("json");
        assert_eq!(value["deploymentMode"], serde_json::json!("1p"), "{}", path.display());
    }
    let meta: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&paths.meta_path).expect("meta")).expect("json");
    assert!(meta.get("appliedId").is_none(), "不得选中任何 profile");
    let entries = meta["entries"].as_array().expect("entries");
    assert_eq!(entries.len(), 1, "只应剩下外部条目: {entries:?}");
    assert_eq!(entries[0]["name"], serde_json::json!("CC Switch"));
    assert!(!paths.profile_path.exists(), "自身 profile 必须被撤回");

    // 3) 非受管键必须原样保留（这里是本机真实存在的 preferences 与路径）。
    let after: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&paths.third_party_config).expect("3p config"))
            .expect("json");
    assert_eq!(after["preferences"], foreign_before, "preferences 不得被改写");
    assert_eq!(
        after["coworkUserFilesPath"], cowork_before,
        "coworkUserFilesPath 不得被改写"
    );
    println!("official mode: 两处 1p、外部条目保留、非受管键未动、profile 已撤回");

    // 4) 立即恢复直连，把应用留在可用状态。
    let reapply = ActionRequest {
        provider_id: Some(provider_id),
        models,
        paths: PathsChoice::default(),
        rebaseline: false,
        routes: Vec::new(),
        rotate_gateway_token: false,
    };
    let prepared = manager
        .prepare(&store, DesktopAction::ApplyDirect, &reapply)
        .expect("prepare reapply");
    let preview = manager.issue_ticket(&prepared).expect("ticket");
    manager
        .commit(
            &store,
            DesktopAction::ApplyDirect,
            &preview.revision,
            &reapply,
        )
        .expect("reapply direct");
    let status = manager.status(&store);
    println!(
        "restored: state={} mode={} models={} stale={:?}",
        status.management_state,
        status.mode,
        status.models.len(),
        status.stale_reasons
    );
    assert_eq!(status.management_state, "managed");
    assert_eq!(status.mode, "direct");
    assert!(paths.profile_path.exists(), "直连 profile 必须回来");
    // 提交后文件已变而 Desktop 尚不知道，因此**确实**需要重启；这里断言的是文件
    // 已经回到直连形态，而不是「不需要重启」。
    assert!(status.restart_required, "文件变了就必须提示重启");
    let restored: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&paths.third_party_config).expect("3p config"))
            .expect("json");
    assert_eq!(restored["deploymentMode"], serde_json::json!("3p"));
    let restored_meta: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&paths.meta_path).expect("meta")).expect("json");
    assert_eq!(
        restored_meta["appliedId"],
        serde_json::json!("d918233d-0a41-47a1-a29f-6d610a03722a"),
        "必须重新选中自己的 profile"
    );

    // 非受管键在往返之后依然完好。
    let final_config: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&paths.third_party_config).expect("3p config"))
            .expect("json");
    assert_eq!(final_config["preferences"], foreign_before);
    assert_eq!(final_config["coworkUserFilesPath"], cowork_before);
    println!("L07 完成：官方模式已核对，直连已恢复");
}

/// 读取本机设置里的代理配置（与命令层同一来源）。
fn load_settings(codex: &CodexPaths) -> agentharbor_lib::core::models::ApiProxyConfigPayload {
    agentharbor_lib::core::newapi::proxy_from_settings(&codex.settings_path)
}

// ---------------------------------------------------------------------------

/// `cargo test --test claude_desktop_workflows -- --ignored --nocapture real_provider_diagnostic`
#[test]
#[ignore = "只在本机手动运行；读取真实的 provider 卡片并做只读判定"]
fn real_provider_diagnostic() {
    let codex = CodexPaths::new();
    let store = router::load_store(&codex).expect("read real store");
    let catalog = agentharbor_lib::core::models_dev::load(&codex.app_data_dir);
    let policy = agentharbor_lib::core::claude_desktop::provider::DesktopVersionPolicy::from_version(
        Some("2.19675.0"),
    );
    // 受管漂移：哪个文件、期望值 vs 现状。
    {
        use agentharbor_lib::core::claude_desktop::Manager as M;
        let m = M::new(&codex.app_data_dir);
        match (m.load_state().ok().flatten(), m.resolve_paths(&PathsChoice::default()).ok()) {
            (Some(state), Some(dpaths)) => {
                println!(
                    "STATE tx={} gen={} fingerprint={:?}",
                    state.transaction_id,
                    state.generation,
                    state.credential_fingerprint.as_deref().map(|value| &value[..8])
                );
                match agentharbor_lib::core::claude_desktop::transaction::drifted_roles(
                    &state,
                    &dpaths,
                ) {
                    Ok(roles) => println!("DRIFTED_ROLES={roles:?}"),
                    Err(error) => println!("DRIFT_ERROR={}", error.code),
                }
                for edition in &state.data.files {
                    let describe = |values: &agentharbor_lib::core::claude_desktop::transaction::ManagedValues| {
                        match values {
                            agentharbor_lib::core::claude_desktop::transaction::ManagedValues::Whole { value } => {
                                format!("Whole({})", if value.is_some() { "some" } else { "none" })
                            }
                            agentharbor_lib::core::claude_desktop::transaction::ManagedValues::Fields { values } => {
                                format!("Fields({:?})", values.keys().collect::<Vec<_>>())
                            }
                            agentharbor_lib::core::claude_desktop::transaction::ManagedValues::Meta { own_entry, applied_id } => {
                                format!(
                                    "Meta(own={} applied={})",
                                    own_entry.is_some(),
                                    applied_id.is_some()
                                )
                            }
                        }
                    };
                    println!(
                        "  edition {} existed={} baseline={} installed={}",
                        edition.role,
                        edition.existed_before,
                        describe(&edition.baseline),
                        describe(&edition.installed)
                    );
                }
            }
            (state, paths) => println!("no state ({}) or paths ({})", state.is_some(), paths.is_some()),
        }
    }
    // 重启能力：确认探测到的启动方式，不实际启动任何东西。
    {
        use agentharbor_lib::core::claude_desktop::desktop_app;
        let discovery = agentharbor_lib::core::claude_desktop::paths::discover();
        println!(
            "discovery: candidates={} default={:?} notes={:?}",
            discovery.candidates.len(),
            discovery.default.is_some(),
            discovery.notes
        );
        for candidate in &discovery.candidates {
            println!(
                "  candidate normal={} thirdParty={} evidence={}",
                candidate.paths.normal_root.display(),
                candidate.paths.third_party_root.display(),
                candidate.install_evidence
            );
        }
        let paths = agentharbor_lib::core::claude_desktop::paths::resolve(
            &PathsChoice::default(),
        );
        match paths {
            Ok(paths) => {
                println!(
                    "desktop roots: normal={} thirdParty={} isolated={}",
                    paths.normal_root.display(),
                    paths.third_party_root.display(),
                    paths.isolated
                );
                let app = desktop_app::state(&paths);
                println!(
                    "app: running={} count={} launchMethod={:?} target={:?}",
                    app.running,
                    app.process_count,
                    app.launch_method,
                    app.launch_target
                );
            }
            Err(error) => println!("desktop paths unresolved: {}", error.code),
        }
    }
    println!("providers={}", store.providers.len());
    for provider in &store.providers {
        println!("----");
        println!("id={}", provider.id);
        println!("name={}", provider.name);
        println!("baseUrl={}", provider.base_url);
        println!("enabled={}", provider.enabled);
        println!("networkMode={}", provider.network_mode());
        println!("defaultModel={}", provider.model);
        println!("defaultProtocol={:?}", provider.protocol);
        // 只报长度，永不打印密钥本身。
        println!(
            "hasApiKey={} apiKeyLen={} hasAccessToken={} userId={:?}",
            !provider.api_key.is_empty(),
            provider.api_key.len(),
            !provider.access_token.is_empty(),
            provider.user_id
        );
        match agentharbor_lib::core::claude_desktop::provider::resolve_direct_endpoints(provider) {
            Ok((root, messages, models)) => {
                println!("projectedRoot={root}");
                println!("messagesUrl={messages}");
                println!("modelsUrl={models}");
            }
            Err(error) => println!("endpointError={} params={:?}", error.code, error.params),
        }
        for model in &provider.models {
            let candidate = agentharbor_lib::core::claude_desktop::provider::model_candidate(
                provider, &model.id, &catalog, policy,
            );
            println!(
                "  model id={} included={} protocol={} source={} verified={} selectable={} reason={:?} ctx={:?} ctxSource={} 1m={}",
                model.id,
                model.included,
                candidate.protocol,
                candidate.protocol_source,
                candidate.protocol_verified,
                candidate.selectable,
                candidate.exclude_reason,
                candidate.context_window,
                candidate.context_source,
                candidate.supports1m,
            );
        }
        let (eligible, reason, models) = agentharbor_lib::core::claude_desktop::provider::provider_qualification(
            provider, policy, &catalog,
        );
        println!(
            "  qualifier eligible={eligible} reason={:?} candidates={}",
            reason,
            models.len()
        );
    }
}

const CC_SWITCH_ID: &str = "00000000-0000-4000-8000-000000157210";

struct Fixture {
    _temp: tempfile::TempDir,
    codex: CodexPaths,
    paths: DesktopPaths,
    manager: Manager,
}

fn fixture(name: &str) -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let codex = CodexPaths::from_home(temp.path().join("codex"));
    codex.ensure_directories().unwrap();
    let normal = temp.path().join("Claude");
    let three_p = temp.path().join("Claude-3p");
    fs::create_dir_all(&normal).unwrap();
    let paths = DesktopPaths::from_roots(normal, three_p).unwrap();
    // `configLibrary` 由 Desktop/CC Switch 建立；测试显式准备它与真实环境一致。
    fs::create_dir_all(paths.meta_path.parent().unwrap()).unwrap();
    let manager = Manager::new(&codex.app_data_dir)
        .with_paths(paths.clone())
        .with_detected_version(Some("1.12603.1".to_owned()));
    let _ = name;
    Fixture {
        _temp: temp,
        codex,
        paths,
        manager,
    }
}

/// 卡片 id 由 Store 生成，测试在保存后读取，避免猜测 id 规则。
fn provider_id(store: &Store) -> String {
    store.providers[0].id.clone()
}

/// 保存一张关闭路由池的卡片，并返回保存后的 Store。
fn seed_provider(codex: &CodexPaths) -> Store {
    let models = vec![
        newapi::Model {
            id: "claude-sonnet-4-6".into(),
            display_name: "中转 · Sonnet".into(),
            protocol: Some(ModelProtocol::AnthropicMessages),
            included: true,
            ..Default::default()
        },
        newapi::Model {
            id: "claude-opus-4-6".into(),
            display_name: String::new(),
            protocol: Some(ModelProtocol::AnthropicMessages),
            included: true,
            ..Default::default()
        },
    ];
    router::save_provider(
        codex,
        ProviderInput {
            id: None,
            name: "中转 A".into(),
            base_url: "https://relay.example/v1".into(),
            user_id: Some(7),
            access_token: "balance-token-SENTINEL".into(),
            api_key: SENTINEL_KEY.into(),
            protocol: ModelProtocol::Responses,
            model: "claude-sonnet-4-6".into(),
            models,
            context_window: None,
            auto_compact_limit: None,
            catalog_providers: None,
            price_catalog_provider: None,
            price_multiplier: None,
            compression: None,
            network_mode: "direct".into(),
        },
    )
    .unwrap();
    let mut store = router::load_store(codex).unwrap();
    // enabled=false 是直连必须支持的情形。
    store.providers[0].enabled = false;
    store.active_provider_id = None;
    store
}

/// 基础请求：providerId 由调用方注入，模型与路径固定。
fn request() -> ActionRequest {
    ActionRequest {
        provider_id: None,
        models: vec![
            agentharbor_lib::core::claude_desktop::types::DesktopModelChoice {
                model_id: "claude-sonnet-4-6".into(),
                label_override: None,
                one_m_mode: OneMMode::Auto,
            },
            agentharbor_lib::core::claude_desktop::types::DesktopModelChoice {
                model_id: "claude-opus-4-6".into(),
                label_override: Some("中转 · Opus".into()),
                one_m_mode: OneMMode::Off,
            },
        ],
        paths: PathsChoice::default(),
        rebaseline: false,
        routes: Vec::new(),
        rotate_gateway_token: false,
    }
}

/// 统一的直连请求：两张模型都选自该卡片。
fn request_for(store: &Store) -> ActionRequest {
    let mut value = request();
    value.provider_id = Some(provider_id(store));
    value
}

fn read(path: &std::path::Path) -> Option<String> {
    fs::read_to_string(path).ok()
}

#[test]
fn direct_connection_preview_commit_and_restore_roundtrip() {
    let fixture = fixture("e2e");
    let store = seed_provider(&fixture.codex);
    let provider = store.providers.iter().find(|p| p.id == provider_id(&store)).unwrap();
    assert!(!provider.enabled, "用例前提：卡片未启用");

    // 既有外部 3P 工具与 MCP 内容。
    fs::write(
        &fixture.paths.normal_config,
        r#"{"deploymentMode":"1p","mcpServers":{"keep":1}}"#,
    )
    .unwrap();
    fs::write(
        &fixture.paths.third_party_config,
        r#"{"deploymentMode":"3p","mcpServers":{"keep":1}}"#,
    )
    .unwrap();
    fs::write(
        &fixture.paths.meta_path,
        &format!(r#"{{"appliedId":"{CC_SWITCH_ID}","entries":[{{"id":"{CC_SWITCH_ID}","name":"CC Switch"}}]}}"#),
    )
    .unwrap();

    // 1) 候选：关闭路由池的卡片仍可直连。
    let candidates = fixture
        .manager
        .candidates(&store, &PathsChoice::default())
        .unwrap();
    let candidate = candidates
        .providers
        .iter()
        .find(|entry| entry.provider_id == provider_id(&store))
        .unwrap();
    assert!(candidate.eligible, "reason: {:?}", candidate.reason);
    assert!(!candidate.enabled);
    let endpoint = candidate.endpoint.as_ref().unwrap();
    assert_eq!(endpoint.profile_base_url, "https://relay.example");
    assert_eq!(endpoint.messages_url, "https://relay.example/v1/messages");
    assert!(candidate.models.iter().all(|model| model.selectable));

    // 2) 预览 → ticket；预览 DTO 不含密钥，外部条目只报数量。
    let prepared = fixture
        .manager
        .prepare(&store, DesktopAction::ApplyDirect, &request_for(&store))
        .unwrap();
    let preview = fixture.manager.issue_ticket(&prepared).unwrap();
    let rendered = serde_json::to_string(&preview).unwrap();
    assert!(!rendered.contains(SENTINEL_KEY), "预览 DTO 泄露了推理密钥");
    assert!(!rendered.contains("balance-token-SENTINEL"));
    assert_eq!(preview.external_profile_count, 1);
    assert_eq!(preview.current_applied_id.as_deref(), Some(CC_SWITCH_ID));
    assert!(preview.requires_restart);
    assert!(preview.files.iter().any(|file| file.role == "profile"));

    // 3) 提交。
    let applied = fixture
        .manager
        .commit(
            &store,
            DesktopAction::ApplyDirect,
            &preview.revision,
            &request_for(&store),
        )
        .unwrap();
    assert!(applied.requires_restart);
    assert!(applied.backup_id.is_some());
    assert!(applied.changed_paths.len() >= 3);

    // 4) 四文件结果。
    let profile: serde_json::Value =
        serde_json::from_str(&read(&fixture.paths.profile_path).unwrap()).unwrap();
    assert_eq!(profile["inferenceGatewayApiKey"], serde_json::json!(SENTINEL_KEY));
    assert_eq!(profile["inferenceGatewayBaseUrl"], serde_json::json!("https://relay.example"));
    // 第一张模型带卡片展示名与自动 1M：写成对象且保留原 ID。
    assert_eq!(profile["inferenceModels"][0]["name"], serde_json::json!("claude-sonnet-4-6"));
    assert_eq!(
        profile["inferenceModels"][0]["labelOverride"],
        serde_json::json!("中转 · Sonnet")
    );
    assert_eq!(profile["inferenceModels"][0]["supports1m"], serde_json::json!(true));
    // 第二张显式关闭 1M：对象里不得出现 supports1m。
    assert_eq!(profile["inferenceModels"][1]["name"], serde_json::json!("claude-opus-4-6"));
    assert_eq!(profile["inferenceModels"][1]["labelOverride"], serde_json::json!("中转 · Opus"));
    assert!(profile["inferenceModels"][1].get("supports1m").is_none());
    assert!(!profile.to_string().contains("[1m]"));
    assert_eq!(profile["coworkEgressAllowedHosts"], serde_json::json!(["*"]));
    let threep: serde_json::Value =
        serde_json::from_str(&read(&fixture.paths.third_party_config).unwrap()).unwrap();
    assert_eq!(threep["mcpServers"]["keep"], serde_json::json!(1));
    let meta: serde_json::Value =
        serde_json::from_str(&read(&fixture.paths.meta_path).unwrap()).unwrap();
    assert_eq!(
        meta["appliedId"],
        serde_json::json!(agentharbor_lib::core::claude_desktop::constants::PROFILE_ID)
    );
    assert_eq!(meta["entries"].as_array().unwrap().len(), 2);

    // 5) 供应商全局状态完全未变。
    let after = router::load_store(&fixture.codex).unwrap();
    let after_provider = after.providers.iter().find(|p| p.id == provider_id(&store)).unwrap();
    assert!(!after_provider.enabled, "直连不得启用路由池");
    assert_eq!(after_provider.model, "claude-sonnet-4-6");
    assert_eq!(after_provider.protocol, ModelProtocol::Responses);
    assert!(after.active_provider_id.is_none());
    assert!(!after.router.enabled);

    // 6) 状态：已管理 + 需要重启，且没有冲突。
    let status = fixture.manager.status(&store);
    assert_eq!(status.management_state, "managed");
    assert!(status.is_managed);
    assert!(status.restart_required);
    assert!(status.conflicts.is_empty(), "{:?}", status.conflicts);
    assert_eq!(status.mode, "direct");
    assert_eq!(status.models.len(), 2);

    // 7) 恢复接管前配置：撤回受管内容，外部 MCP 与外部 profile 保留。
    let restore_request = ActionRequest {
        provider_id: None,
        models: vec![],
        paths: PathsChoice::default(),
        rebaseline: false,
        routes: Vec::new(),
        rotate_gateway_token: false,
    };
    let restore_prepared = fixture
        .manager
        .prepare(&store, DesktopAction::RestorePrevious, &restore_request)
        .unwrap();
    let restore_preview = fixture.manager.issue_ticket(&restore_prepared).unwrap();
    fixture
        .manager
        .commit(
            &store,
            DesktopAction::RestorePrevious,
            &restore_preview.revision,
            &restore_request,
        )
        .unwrap();
    let normal: serde_json::Value =
        serde_json::from_str(&read(&fixture.paths.normal_config).unwrap()).unwrap();
    assert_eq!(normal["deploymentMode"], serde_json::json!("1p"));
    assert_eq!(normal["mcpServers"]["keep"], serde_json::json!(1));
    let meta: serde_json::Value =
        serde_json::from_str(&read(&fixture.paths.meta_path).unwrap()).unwrap();
    assert_eq!(meta["appliedId"], serde_json::json!(CC_SWITCH_ID));
    assert_eq!(meta["entries"].as_array().unwrap().len(), 1);
    assert!(!fixture.paths.profile_path.exists());

    // 8) 恢复后 Store 仍然未变。
    let final_store = router::load_store(&fixture.codex).unwrap();
    assert_eq!(final_store.providers.len(), after.providers.len());
    assert!(!final_store.providers[0].enabled);
}

#[test]
fn commit_rejects_a_stale_ticket_after_the_inputs_change() {
    let fixture = fixture("stale");
    let store = seed_provider(&fixture.codex);

    let prepared = fixture
        .manager
        .prepare(&store, DesktopAction::ApplyDirect, &request_for(&store))
        .unwrap();
    let preview = fixture.manager.issue_ticket(&prepared).unwrap();

    // 表单变化：少选一个模型。
    let mut changed = request_for(&store);
    changed.models.pop();
    let error = fixture
        .manager
        .commit(
            &store,
            DesktopAction::ApplyDirect,
            &preview.revision,
            &changed,
        )
        .expect_err("旧 ticket 不得提交");
    assert_eq!(error.code, "changed");
    // 零外部写入。
    assert!(!fixture.paths.profile_path.exists());
    assert!(!fixture.paths.normal_config.exists());
    assert!(!fixture.paths.third_party_config.exists());
    assert!(!fixture.paths.meta_path.exists());
}

#[test]
fn a_ticket_cannot_be_used_twice() {
    let fixture = fixture("replay");
    let store = seed_provider(&fixture.codex);
    let prepared = fixture
        .manager
        .prepare(&store, DesktopAction::ApplyDirect, &request_for(&store))
        .unwrap();
    let preview = fixture.manager.issue_ticket(&prepared).unwrap();
    fixture
        .manager
        .commit(&store, DesktopAction::ApplyDirect, &preview.revision, &request_for(&store))
        .unwrap();
    let error = fixture
        .manager
        .commit(&store, DesktopAction::ApplyDirect, &preview.revision, &request_for(&store))
        .expect_err("ticket 只能消费一次");
    assert_eq!(error.code, "preview_expired");
}

#[test]
fn an_unsafe_selected_model_blocks_the_whole_apply() {
    let fixture = fixture("unsafe");
    let store = seed_provider(&fixture.codex);
    let mut request = request_for(&store);
    request.models[0].model_id = "gpt-4o".into();
    let error = fixture
        .manager
        .prepare(&store, DesktopAction::ApplyDirect, &request)
        .expect_err("不安全的模型名必须阻止提交");
    assert_eq!(error.code, "model_rejected");
    assert_eq!(error.model_id.as_deref(), Some("gpt-4o"));
    // 零写入，且不静默改成另一个模型。
    assert!(!fixture.paths.profile_path.exists());
    assert!(!fixture.paths.normal_config.exists());
}

#[test]
fn release_keeps_the_files_and_records_a_tombstone() {
    let fixture = fixture("release");
    let store = seed_provider(&fixture.codex);
    let prepared = fixture
        .manager
        .prepare(&store, DesktopAction::ApplyDirect, &request_for(&store))
        .unwrap();
    let preview = fixture.manager.issue_ticket(&prepared).unwrap();
    fixture
        .manager
        .commit(&store, DesktopAction::ApplyDirect, &preview.revision, &request_for(&store))
        .unwrap();

    let before = [
        read(&fixture.paths.normal_config),
        read(&fixture.paths.third_party_config),
        read(&fixture.paths.profile_path),
        read(&fixture.paths.meta_path),
    ];
    let release_request = ActionRequest {
        provider_id: None,
        models: vec![],
        paths: PathsChoice::default(),
        rebaseline: false,
        routes: Vec::new(),
        rotate_gateway_token: false,
    };
    let release_prepared = fixture
        .manager
        .prepare(&store, DesktopAction::Release, &release_request)
        .unwrap();
    let release_preview = fixture.manager.issue_ticket(&release_prepared).unwrap();
    let applied = fixture
        .manager
        .commit(
            &store,
            DesktopAction::Release,
            &release_preview.revision,
            &release_request,
        )
        .unwrap();
    assert!(!applied.requires_restart);
    assert!(applied.changed_paths.is_empty());
    let after = [
        read(&fixture.paths.normal_config),
        read(&fixture.paths.third_party_config),
        read(&fixture.paths.profile_path),
        read(&fixture.paths.meta_path),
    ];
    assert_eq!(before, after, "释放管理不得改动 Desktop 文件");
    let status = fixture.manager.status(&store);
    assert_eq!(status.management_state, "unmanaged");
    assert!(!status.is_managed);
    // 恢复到未受管状态后不能再“恢复接管前配置”。
    let action = status
        .actions
        .iter()
        .find(|view| view.action == DesktopAction::RestorePrevious)
        .unwrap();
    assert!(!action.enabled);
}

#[test]
fn unrelated_provider_changes_do_not_invalidate_the_profile() {
    let fixture = fixture("unrelated");
    let store = seed_provider(&fixture.codex);
    let prepared = fixture
        .manager
        .prepare(&store, DesktopAction::ApplyDirect, &request_for(&store))
        .unwrap();
    let preview = fixture.manager.issue_ticket(&prepared).unwrap();
    fixture
        .manager
        .commit(&store, DesktopAction::ApplyDirect, &preview.revision, &request_for(&store))
        .unwrap();
    let profile_before = read(&fixture.paths.profile_path);

    // 余额、账本、测试状态与路由开关都不进入直连 profile。
    let mut touched = router::load_store(&fixture.codex).unwrap();
    touched.providers[0].balance = None;
    touched.providers[0].balance_script = "return 1;".into();
    touched.providers[0].test_status = "ok".into();
    touched.router.enabled = false;
    touched.active_provider_id = Some(provider_id(&store));
    let status = fixture.manager.status(&touched);
    assert_eq!(status.management_state, "managed", "stale: {:?}", status.stale_reasons);
    assert!(status.conflicts.is_empty(), "{:?}", status.conflicts);
    assert_eq!(read(&fixture.paths.profile_path), profile_before);

    // 普通 reapply 立刻可用，且不带任何冲突。
    let refreshed = fixture
        .manager
        .prepare(&touched, DesktopAction::Reapply, &request_for(&touched))
        .unwrap();
    assert!(refreshed.conflicts.is_empty());
    assert_eq!(
        read(&fixture.paths.profile_path),
        profile_before,
        "无关变更不得重写 profile"
    );
}

#[test]
fn provider_removal_marks_the_configuration_orphaned_but_still_restorable() {
    let fixture = fixture("orphan");
    let store = seed_provider(&fixture.codex);
    let prepared = fixture
        .manager
        .prepare(&store, DesktopAction::ApplyDirect, &request_for(&store))
        .unwrap();
    let preview = fixture.manager.issue_ticket(&prepared).unwrap();
    fixture
        .manager
        .commit(&store, DesktopAction::ApplyDirect, &preview.revision, &request_for(&store))
        .unwrap();

    router::delete_provider(&fixture.codex, &provider_id(&store)).unwrap();
    let empty = router::load_store(&fixture.codex).unwrap();
    let status = fixture.manager.status(&empty);
    assert_eq!(status.management_state, "orphaned");
    assert!(status.provider_missing);
    let restore = status
        .actions
        .iter()
        .find(|view| view.action == DesktopAction::RestorePrevious)
        .unwrap();
    assert!(restore.enabled, "供应商被删除后仍必须能恢复");

    // 离线恢复：不需要 Provider，也不需要联网。
    let restore_request = ActionRequest {
        provider_id: None,
        models: vec![],
        paths: PathsChoice::default(),
        rebaseline: false,
        routes: Vec::new(),
        rotate_gateway_token: false,
    };
    let restore_prepared = fixture
        .manager
        .prepare(&empty, DesktopAction::RestorePrevious, &restore_request)
        .unwrap();
    let restore_preview = fixture.manager.issue_ticket(&restore_prepared).unwrap();
    fixture
        .manager
        .commit(
            &empty,
            DesktopAction::RestorePrevious,
            &restore_preview.revision,
            &restore_request,
        )
        .unwrap();
    assert!(!fixture.paths.profile_path.exists());
}

#[test]
fn switching_to_official_mode_preserves_foreign_entries() {
    let fixture = fixture("official");
    let store = seed_provider(&fixture.codex);
    fs::write(
        &fixture.paths.meta_path,
        &format!(r#"{{"appliedId":"{CC_SWITCH_ID}","entries":[{{"id":"{CC_SWITCH_ID}","name":"CC Switch"}}]}}"#),
    )
    .unwrap();
    let prepared = fixture
        .manager
        .prepare(&store, DesktopAction::ApplyDirect, &request_for(&store))
        .unwrap();
    let preview = fixture.manager.issue_ticket(&prepared).unwrap();
    fixture
        .manager
        .commit(&store, DesktopAction::ApplyDirect, &preview.revision, &request_for(&store))
        .unwrap();

    let switch_request = ActionRequest {
        provider_id: None,
        models: vec![],
        paths: PathsChoice::default(),
        rebaseline: false,
        routes: Vec::new(),
        rotate_gateway_token: false,
    };
    let switch_prepared = fixture
        .manager
        .prepare(&store, DesktopAction::SwitchOfficial, &switch_request)
        .unwrap();
    let switch_preview = fixture.manager.issue_ticket(&switch_prepared).unwrap();
    fixture
        .manager
        .commit(
            &store,
            DesktopAction::SwitchOfficial,
            &switch_preview.revision,
            &switch_request,
        )
        .unwrap();
    let normal: serde_json::Value =
        serde_json::from_str(&read(&fixture.paths.normal_config).unwrap()).unwrap();
    assert_eq!(normal["deploymentMode"], serde_json::json!("1p"));
    let meta: serde_json::Value =
        serde_json::from_str(&read(&fixture.paths.meta_path).unwrap()).unwrap();
    assert!(meta.get("appliedId").is_none(), "不得选中任何外部 profile");
    let entries = meta["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["id"], serde_json::json!(CC_SWITCH_ID));
    assert!(!fixture.paths.profile_path.exists());
    let status = fixture.manager.status(&store);
    assert!(!status.is_managed);
    assert_eq!(status.management_state, "unmanaged");
    assert!(status.restart_required, "切回官方模式后仍需提供启动入口");
    assert!(!status.app_running);
}

#[test]
fn apply_direct_request_defaults_are_stable() {
    // 前端只提交 providerId / 模型 / 路径；地址与密钥永不来自前端。
    let request: ApplyDirectRequest = serde_json::from_str(
        r#"{"providerId":"p1","models":[{"modelId":"claude-sonnet-4-6","oneMMode":"auto"}]}"#,
    )
    .unwrap();
    assert!(request.paths.normal_root.is_none());
    assert!(!request.rebaseline);
    assert!(request.models[0].label_override.is_none());
}

fn official_request() -> ActionRequest {
    ActionRequest {
        provider_id: None,
        models: vec![],
        paths: PathsChoice::default(),
        rebaseline: false,
        routes: vec![],
        rotate_gateway_token: false,
    }
}

#[test]
fn unmanaged_official_mode_preserves_external_profiles_without_taking_ownership() {
    let fixture = fixture("unmanagedOfficial");
    let store = Store::default();
    let meta = format!(
        "{{\n  \"entries\": [{{\"id\":\"{CC_SWITCH_ID}\",\"name\":\"CC Switch\",\"extra\":1}}],\n  \"appliedId\": \"{CC_SWITCH_ID}\", \"unrelated\": true\n}}"
    );
    let profile = "{ \"custom\": \"external-profile\" }\n";
    let external_profile_path = fixture
        .paths
        .meta_path
        .parent()
        .unwrap()
        .join(format!("{CC_SWITCH_ID}.json"));
    fs::write(&fixture.paths.meta_path, &meta).unwrap();
    fs::write(&fixture.paths.profile_path, profile).unwrap();
    fs::write(&external_profile_path, profile).unwrap();
    fs::write(
        &fixture.paths.normal_config,
        r#"{"deploymentMode":"3p","mcpServers":{"existing":{}}}"#,
    )
    .unwrap();
    fs::write(
        &fixture.paths.third_party_config,
        r#"{"deploymentMode":"3p","preferences":{"theme":"dark"}}"#,
    )
    .unwrap();
    let status = fixture.manager.status(&store);
    assert!(!status.is_managed);
    assert!(status
        .actions
        .iter()
        .any(|action| action.action == DesktopAction::SwitchOfficial && action.enabled));
    assert!(status
        .actions
        .iter()
        .any(|action| action.action == DesktopAction::RestorePrevious && !action.enabled));

    let request = official_request();
    let prepared = fixture
        .manager
        .prepare(&store, DesktopAction::SwitchOfficial, &request)
        .unwrap();
    assert_eq!(prepared.plans.len(), 2);
    assert_eq!(prepared.external_profile_count, 1);
    assert!(prepared.next_state.tombstone);
    assert!(
        prepared.next_state.data.files.is_empty(),
        "不得建立接管前配置的假恢复点"
    );
    let preview = fixture.manager.issue_ticket(&prepared).unwrap();
    fixture
        .manager
        .commit(
            &store,
            DesktopAction::SwitchOfficial,
            &preview.revision,
            &request,
        )
        .unwrap();
    let normal: serde_json::Value =
        serde_json::from_str(&read(&fixture.paths.normal_config).unwrap()).unwrap();
    let threep: serde_json::Value =
        serde_json::from_str(&read(&fixture.paths.third_party_config).unwrap()).unwrap();
    assert_eq!(normal["deploymentMode"], "1p");
    assert_eq!(normal["mcpServers"], serde_json::json!({"existing":{}}));
    assert_eq!(threep["deploymentMode"], "1p");
    assert_eq!(threep["preferences"], serde_json::json!({"theme":"dark"}));
    assert_eq!(read(&fixture.paths.meta_path).unwrap(), meta);
    assert_eq!(read(&fixture.paths.profile_path).unwrap(), profile);
    assert_eq!(read(&external_profile_path).unwrap(), profile);
    let status = fixture.manager.status(&store);
    assert_eq!(status.mode, "official");
    assert!(!status.is_managed);
    assert!(status.restart_required);
    assert!(fixture
        .manager
        .prepare(&store, DesktopAction::RestorePrevious, &request)
        .is_err());
    let second = fixture
        .manager
        .prepare(&store, DesktopAction::SwitchOfficial, &request)
        .unwrap();
    assert!(second.plans.is_empty());
    assert!(second.requires_restart, "重复切回官方不得丢失待重启标志");
}

#[test]
fn unmanaged_official_mode_does_not_parse_or_replace_foreign_metadata() {
    let fixture = fixture("unmanagedOfficialInvalidMetadata");
    let store = Store::default();
    let damaged = "{ unfinished external file\n";
    fs::write(&fixture.paths.meta_path, damaged).unwrap();
    fs::write(&fixture.paths.profile_path, damaged).unwrap();
    let request = official_request();
    let prepared = fixture
        .manager
        .prepare(&store, DesktopAction::SwitchOfficial, &request)
        .unwrap();
    let preview = fixture.manager.issue_ticket(&prepared).unwrap();
    fixture
        .manager
        .commit(
            &store,
            DesktopAction::SwitchOfficial,
            &preview.revision,
            &request,
        )
        .unwrap();
    assert_eq!(read(&fixture.paths.meta_path).unwrap(), damaged);
    assert_eq!(read(&fixture.paths.profile_path).unwrap(), damaged);
    assert_eq!(fixture.manager.status(&store).mode, "official");
}

#[test]
fn unmanaged_official_mode_rejects_stale_preview_before_any_write() {
    let fixture = fixture("unmanagedOfficialStalePreview");
    let store = Store::default();
    let request = official_request();
    let prepared = fixture
        .manager
        .prepare(&store, DesktopAction::SwitchOfficial, &request)
        .unwrap();
    let preview = fixture.manager.issue_ticket(&prepared).unwrap();
    fs::write(
        &fixture.paths.meta_path,
        r#"{"entries":[],"outsideUpdate":true}"#,
    )
    .unwrap();
    let result = fixture.manager.commit(
        &store,
        DesktopAction::SwitchOfficial,
        &preview.revision,
        &request,
    );
    assert_eq!(result.unwrap_err().code, "changed");
    assert!(!fixture.paths.normal_config.exists());
    assert!(!fixture.paths.third_party_config.exists());
    assert!(!fixture.paths.profile_path.exists());
    assert!(!fixture
        .codex
        .app_data_dir
        .join("claude-desktop/pending.dat")
        .exists());
    assert!(!fixture
        .codex
        .app_data_dir
        .join("claude-desktop/backups")
        .exists());
    assert!(!fixture.manager.status(&store).is_managed);
}
