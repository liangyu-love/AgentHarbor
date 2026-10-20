//! The tray menu: what AgentHarbor is doing, quick ways into the window,
//! Cursor's injection and the app's own switches. Texts follow the interface
//! language. The status lines and check marks are refreshed in place when the
//! pointer reaches the icon, so the menu is current by the time it opens;
//! today's usage is counted off the main thread and at most once a minute.
use crate::core::{autostart, cursor_inject, relay_usage, repository::Repository, router_proxy, routing_runtime};
use crate::platform::paths::CodexPaths;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::menu::{CheckMenuItem, Menu, MenuBuilder, MenuItem, SubmenuBuilder};
use tauri::{AppHandle, Emitter, Manager, Wry};
use tauri_plugin_notification::NotificationExt;

const TRAY_ID: &str = "main";
const HEADER_ID: &str = "tray_header";
const ROUTER_ID: &str = "tray_router";
const USAGE_ID: &str = "tray_usage";
const OPEN_MAIN_ID: &str = "tray_open_main";
const NAVIGATE_PREFIX: &str = "tray_navigate:";
const CURSOR_IDE_ID: &str = "tray_cursor_ide";
const CURSOR_CLI_ID: &str = "tray_cursor_cli";
const AUTOSTART_ID: &str = "tray_autostart";
const CHECK_UPDATE_ID: &str = "tray_check_update";
const OPEN_DATA_ID: &str = "tray_open_data";
const OPTIMIZE_MEMORY_ID: &str = "tray_optimize_memory";
const QUIT_ID: &str = "tray_quit";

/// Asks the window to show a page; the payload is the route.
const NAVIGATE_EVENT: &str = "tray-navigate";
/// Asks the window to check for an update and show the result.
const CHECK_UPDATE_EVENT: &str = "tray-check-update";
/// A switch flipped from the tray, for pages showing its state.
const SWITCHED_EVENT: &str = "tray-switched";

#[derive(Clone, serde::Serialize)]
struct Switched {
    /// `"cursor"` or `"autostart"`.
    switch: &'static str,
    /// The i18n key of a switch that did not change.
    error: Option<String>,
}

fn switched(app: &AppHandle, switch: &'static str, result: Result<(), String>) {
    let error = result.err();
    if error.is_some() {
        // The page explains what is missing, such as the certificate's trust.
        navigate(app, if switch == "cursor" { CURSOR_PAGE } else { "settings" });
    }
    let _ = app.emit(SWITCHED_EVENT, Switched { switch, error });
}

/// The pages the tray opens, in the sidebar's order.
const PAGES: [(&str, &str, &str); 6] = [
    ("overview", "仪表盘", "Dashboard"),
    ("usage", "用量", "Usage"),
    ("accounts", "账号", "Accounts"),
    ("newapi", "模型注入", "Model inject"),
    ("relays", "中转模型", "Relay models"),
    ("settings", "系统设置", "Settings"),
];
/// Where Cursor's injection is set up, and its errors explained.
const CURSOR_PAGE: &str = "newapi";
const USAGE_TTL: Duration = Duration::from_secs(60);

fn english() -> bool {
    super::system::ui_language_is_english()
}

fn pick(zh: &'static str, en: &'static str) -> &'static str {
    if english() {
        en
    } else {
        zh
    }
}

/// The menu items whose text or check mark changes while the menu exists.
#[derive(Clone)]
struct Items {
    router: MenuItem<Wry>,
    usage: MenuItem<Wry>,
    ide: CheckMenuItem<Wry>,
    cli: Option<CheckMenuItem<Wry>>,
    autostart: Option<CheckMenuItem<Wry>>,
}

static ITEMS: Mutex<Option<Items>> = Mutex::new(None);

/// Today's request count and cost, with when it was counted.
static USAGE: Mutex<Option<(Instant, u64, u64)>> = Mutex::new(None);
static COUNTING: AtomicBool = AtomicBool::new(false);

fn paths(app: &AppHandle) -> Option<CodexPaths> {
    let state = app.state::<Mutex<Repository>>();
    let repo = state.lock().ok()?;
    Some(repo.paths().clone())
}

fn router_text() -> String {
    match router_proxy::running_port() {
        Some(port) => format!("{} · {port}", pick("● 本地路由运行中", "● Local router running")),
        None => pick("○ 本地路由未运行", "○ Local router stopped").to_owned(),
    }
}

fn usage_text() -> String {
    let counted = USAGE.lock().ok().and_then(|usage| *usage);
    match counted {
        Some((_, requests, cost_micros)) => {
            let cost = relay_usage::micros_to_usd(cost_micros);
            if english() {
                format!("Today: {requests} requests · ${cost:.2}")
            } else {
                format!("今日 {requests} 次请求 · ${cost:.2}")
            }
        }
        None => pick("今日用量统计中…", "Counting today's usage…").to_owned(),
    }
}

fn cursor_flags(app: &AppHandle) -> (bool, bool) {
    paths(app)
        .map(|paths| cursor_inject::load(&paths.app_data_dir))
        .map_or((false, false), |settings| (settings.ide, settings.cli))
}

pub fn create_tray_menu(app: &AppHandle) -> Result<Menu<Wry>, String> {
    build(app).map_err(|error| error.to_string())
}

fn build(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let version = app.package_info().version.to_string();
    let header = MenuItem::with_id(app, HEADER_ID, format!("AgentHarbor v{version}"), false, None::<&str>)?;
    let router = MenuItem::with_id(app, ROUTER_ID, router_text(), false, None::<&str>)?;
    let usage = MenuItem::with_id(app, USAGE_ID, usage_text(), false, None::<&str>)?;
    let open = MenuItem::with_id(app, OPEN_MAIN_ID, pick("打开 AgentHarbor", "Open AgentHarbor"), true, None::<&str>)?;
    let mut pages = SubmenuBuilder::new(app, pick("前往页面", "Go to"));
    for (route, zh, en) in PAGES {
        let label = if english() { en } else { zh };
        pages = pages.item(&MenuItem::with_id(app, format!("{NAVIGATE_PREFIX}{route}"), label, true, None::<&str>)?);
    }
    let pages = pages.build()?;

    let (ide_on, cli_on) = cursor_flags(app);
    let ide = CheckMenuItem::with_id(app, CURSOR_IDE_ID, pick("Cursor IDE 注入", "Cursor IDE injection"), true, ide_on, None::<&str>)?;
    let cli = cursor_inject::cli::supported()
        .then(|| CheckMenuItem::with_id(app, CURSOR_CLI_ID, pick("cursor-agent CLI 注入", "cursor-agent CLI injection"), true, cli_on, None::<&str>))
        .transpose()?;
    let startup = autostart::status();
    let autostart = startup
        .supported
        .then(|| CheckMenuItem::with_id(app, AUTOSTART_ID, pick("开机自启", "Start at sign-in"), true, startup.enabled, None::<&str>))
        .transpose()?;
    let check_update = MenuItem::with_id(app, CHECK_UPDATE_ID, pick("检查更新", "Check for updates"), true, None::<&str>)?;
    let open_data = MenuItem::with_id(app, OPEN_DATA_ID, pick("打开数据目录", "Open data folder"), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, QUIT_ID, pick("退出", "Quit"), true, None::<&str>)?;

    let mut menu = MenuBuilder::new(app)
        .item(&header)
        .item(&router)
        .item(&usage)
        .separator()
        .item(&open)
        .item(&pages)
        .separator()
        .item(&ide);
    if let Some(cli) = &cli {
        menu = menu.item(cli);
    }
    menu = menu.separator();
    if let Some(autostart) = &autostart {
        menu = menu.item(autostart);
    }
    menu = menu.item(&check_update).item(&open_data);
    // Releasing memory needs Windows' memory lists; elsewhere the menu has no item.
    if cfg!(windows) {
        menu = menu.item(&MenuItem::with_id(app, OPTIMIZE_MEMORY_ID, pick("优化内存", "Optimize memory"), true, None::<&str>)?);
    }
    let menu = menu.separator().item(&quit).build()?;
    if let Ok(mut items) = ITEMS.lock() {
        *items = Some(Items { router, usage, ide, cli, autostart });
    }
    Ok(menu)
}

/// Builds the menu again, for a new interface language.
pub fn rebuild_tray_menu(app: &AppHandle) {
    let Ok(menu) = create_tray_menu(app) else {
        return;
    };
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_menu(Some(menu));
    }
}

/// Brings the status lines and check marks up to date in place, and counts
/// today's usage again once the last count is a minute old.
pub fn refresh_tray_menu(app: &AppHandle) {
    let stale = USAGE
        .lock()
        .map(|usage| usage.is_none_or(|(at, _, _)| at.elapsed() >= USAGE_TTL))
        .unwrap_or(false);
    if stale && !COUNTING.swap(true, Ordering::AcqRel) {
        let app = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let state = app.state::<Mutex<Repository>>();
            match super::usage::today_totals(&state) {
                Ok(today) => {
                    if let Ok(mut usage) = USAGE.lock() {
                        *usage = Some((Instant::now(), today.request_count, today.cost_micros));
                    }
                }
                Err(error) => log::warn!("tray usage count failed: {error}"),
            }
            COUNTING.store(false, Ordering::Release);
            update_items(&app);
        });
    }
    update_items(app);
}

fn update_items(app: &AppHandle) {
    let (ide_on, cli_on) = cursor_flags(app);
    let startup = autostart::status();
    // Off the main thread each setter waits for the main thread to run it, and
    // the main thread takes this lock to build or refresh the menu: holding it
    // across the setters would deadlock the two.
    let Some(items) = ITEMS.lock().ok().and_then(|items| items.clone()) else {
        return;
    };
    let _ = items.router.set_text(router_text());
    let _ = items.usage.set_text(usage_text());
    let _ = items.ide.set_checked(ide_on);
    if let Some(cli) = &items.cli {
        let _ = cli.set_checked(cli_on);
    }
    if let Some(autostart) = &items.autostart {
        let _ = autostart.set_checked(startup.enabled);
    }
}

fn navigate(app: &AppHandle, route: &str) {
    let _ = super::hotspot::focus_main_window(app.clone());
    let _ = app.emit(NAVIGATE_EVENT, route);
}

fn notify(app: &AppHandle, body: &str) {
    if let Err(error) = app.notification().builder().title("AgentHarbor").body(body).show() {
        log::warn!("tray notification failed: {error}");
    }
}

/// Turns Cursor's IDE or CLI injection over from its saved state; the item
/// shows the result once it is settled.
fn switch_cursor(app: &AppHandle, ide: bool) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Some(paths) = paths(&app) else {
            return;
        };
        let (on, result) = {
            let _guard = routing_runtime::mutation_guard().await;
            let settings = cursor_inject::load(&paths.app_data_dir);
            let on = !if ide { settings.ide } else { settings.cli };
            let result = if ide {
                cursor_inject::set_ide(&paths, on).await
            } else {
                cursor_inject::set_cli(&paths, on).await
            };
            (on, result)
        };
        if result.is_ok() {
            notify(
                &app,
                match (ide, on) {
                    (true, true) => pick("已写入 Cursor 设置，重启 Cursor 后生效", "Cursor's settings are written; restart Cursor to use them"),
                    (true, false) => pick("已还原 Cursor 设置", "Cursor's settings are restored"),
                    (false, true) => pick("已设置 CURSOR_API_ENDPOINT，新开的终端生效", "CURSOR_API_ENDPOINT is set for new terminals"),
                    (false, false) => pick("已还原 CURSOR_API_ENDPOINT", "CURSOR_API_ENDPOINT is restored"),
                },
            );
        }
        switched(&app, "cursor", result);
        update_items(&app);
    });
}

fn switch_autostart(app: &AppHandle) {
    let enabled = !autostart::status().enabled;
    switched(app, "autostart", autostart::set(enabled).map(drop));
    update_items(app);
}

pub fn handle_tray_menu_event(app: &AppHandle, event_id: &str) {
    if let Some(route) = event_id.strip_prefix(NAVIGATE_PREFIX) {
        navigate(app, route);
        return;
    }
    match event_id {
        OPEN_MAIN_ID => {
            let _ = super::hotspot::focus_main_window(app.clone());
        }
        CURSOR_IDE_ID => switch_cursor(app, true),
        CURSOR_CLI_ID => switch_cursor(app, false),
        AUTOSTART_ID => switch_autostart(app),
        CHECK_UPDATE_ID => {
            let _ = super::hotspot::focus_main_window(app.clone());
            let _ = app.emit(CHECK_UPDATE_EVENT, ());
        }
        OPEN_DATA_ID => {
            if let Some(paths) = paths(app) {
                let folder = paths.app_data_dir.to_string_lossy().into_owned();
                if let Err(error) = super::system::open_path(folder) {
                    log::warn!("opening the data folder failed: {error}");
                }
            }
        }
        OPTIMIZE_MEMORY_ID => crate::commands::memory::optimize_from_tray(app),
        QUIT_ID => crate::commands::desktop_exit::request_exit(app, crate::commands::desktop_exit::ExitIntent::Quit),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tray_page_is_a_route_the_window_knows() {
        let routes = include_str!("../../../src/types/navigation.ts");
        for (route, _, _) in PAGES {
            assert!(routes.contains(&format!("\"{route}\"")), "{route}");
        }
        assert!(PAGES.iter().any(|(route, _, _)| *route == CURSOR_PAGE));
    }
}
