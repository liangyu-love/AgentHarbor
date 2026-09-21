pub mod commands;
pub mod core;
pub mod platform;

use core::repository::Repository;
use image::ImageReader;
use platform::paths::CodexPaths;
use std::cell::RefCell;
use std::io::Cursor;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use tauri::image::Image;
use tauri::tray::TrayIconBuilder;
use tauri::{Manager, RunEvent};

pub fn run() {
    let shared_paths = Arc::new(CodexPaths::new());
    let mut context = tauri::generate_context!();
    if std::env::var_os("AGENT_HARBOR_CODEX_HOME").is_some_and(|value| !value.is_empty()) {
        for window in &mut context.config_mut().app.windows {
            window.data_directory = Some(shared_paths.app_data_dir.join("webview"));
        }
    }

    let single_instance_guard = match platform::single_instance::acquire(&shared_paths) {
        Ok(guard) => guard,
        Err(error) => {
            eprintln!("[AgentHarbor] another instance is already running; exiting: {error}");
            let activated = platform::single_instance::request_existing_instance_activation();
            if !activated {
                eprintln!("[AgentHarbor] failed to activate the running instance");
            }
            return;
        }
    };
    let single_instance_guard = Rc::new(RefCell::new(Some(single_instance_guard)));

    #[cfg(target_os = "windows")]
    let updater_plugin_builder = {
        let builder = tauri_plugin_updater::Builder::new();
        if let Some(arg) = platform::update::windows_current_install_dir_arg() {
            builder.installer_arg(arg)
        } else {
            builder
        }
    };
    #[cfg(not(target_os = "windows"))]
    let updater_plugin_builder = tauri_plugin_updater::Builder::new();

    let builder = tauri::Builder::default();
    let builder = if commands::system::updater_configured(context.config()) {
        builder.plugin(updater_plugin_builder.build())
    } else {
        builder
    };

    let app = builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_shell::init())
        .manage(Mutex::new(Repository::new()))
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                let win = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = win.hide();
                        #[cfg(target_os = "macos")]
                        platform::dock::set_dock_visible(false);
                    }
                });
            }

            let repo_state: tauri::State<'_, Mutex<Repository>> = app.state();
            {
                let paths = repo_state.lock().map(|r| r.paths().clone());
                if let Ok(paths) = paths {
                    if let Ok(store) = core::router::load_store(&paths) {
                        if store.router.enabled {
                            match core::router::ensure_routing_config(&paths, &store) {
                                Ok(_) => {
                                    let port = store.router.port;
                                    tauri::async_runtime::spawn(async move {
                                        let proxy = core::newapi::proxy_from_settings(&paths.settings_path);
                                        if let Err(e) = core::router_proxy::ensure_running(port, paths.clone(), proxy).await {
                                            eprintln!("[AgentHarbor] router proxy startup failed: {e}");
                                        }
                                    });
                                }
                                Err(e) => eprintln!("[AgentHarbor] router config repair failed: {e}"),
                            }
                        }
                    }
                }
            }
            let hotspot_enabled = repo_state
                .lock()
                .map(|r| r.get_hotspot_enabled())
                .unwrap_or(false);
            eprintln!("[AgentHarbor] startup: hotspot_enabled={hotspot_enabled}");
            commands::hotspot::register_hotspot_relayout_observers(app.handle());
            if hotspot_enabled && platform::screen::has_notch_screen() {
                if let Err(e) = commands::hotspot::create_hotspot_window(app.handle()) {
                    eprintln!("[AgentHarbor] failed to create hotspot window at startup: {e}");
                }
            }

            let tray_menu = commands::tray_menu::create_bootstrap_tray_menu(app.handle())
                .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            let tray_icon = load_tray_template_icon()
                .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;

            TrayIconBuilder::with_id("main")
                .icon(tray_icon)
                .icon_as_template(false)
                .tooltip("AgentHarbor")
                .menu(&tray_menu)
                .on_menu_event(|app, event| {
                    commands::tray_menu::handle_tray_menu_event(app, &event.id.0);
                })
                .show_menu_on_left_click(true)
                .build(app)?;

            schedule_startup_main_window_reveal(app.handle());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::newapi::load_newapi_state,
            commands::newapi::save_newapi_provider,
            commands::newapi::delete_newapi_provider,
            commands::newapi::set_newapi_provider_enabled,
            commands::newapi::set_newapi_provider_network,
            commands::newapi::test_newapi_provider,
            commands::newapi::fetch_newapi_models,
            commands::newapi::query_newapi_provider_balance,
            commands::newapi::set_newapi_smart_routing,
            commands::newapi::set_newapi_accountless,
            commands::newapi::import_newapi_providers,
            commands::newapi::export_newapi_providers,
            commands::system::load_snapshot,
            commands::system::is_updater_configured,
            commands::mcp::load_mcp_servers,
            commands::mcp::upsert_mcp_server,
            commands::mcp::set_mcp_server_enabled,
            commands::mcp::remove_mcp_server,
            commands::skills::load_installed_skills,
            commands::skills::load_skill_backups,
            commands::skills::import_skill,
            commands::skills::remove_skill,
            commands::skills::restore_skill_backup,
            commands::skills::delete_skill_backup,
            commands::custom_instructions::load_custom_instruction_state,
            commands::custom_instructions::preview_custom_instruction_apply,
            commands::custom_instructions::apply_custom_instruction,
            commands::custom_instructions::clear_custom_instruction_block,
            commands::custom_instructions::rollback_custom_instruction,
            commands::system::clean,
            commands::system::rebuild_registry,
            commands::system::set_auto_switch,
            commands::system::configure_auto_switch,
            commands::system::set_api_proxy_config,
            commands::system::test_api_proxy_config,
            commands::system::detect_api_proxy_config,
            commands::system::get_usage_refresh_interval,
            commands::system::set_usage_refresh_interval,
            commands::system::run_daemon_once,
            commands::system::diagnose,
            commands::system::restart_codex,
            commands::system::graceful_restart_for_update,
            commands::system::check_update_installability,
            commands::system::load_bootstrap_state,
            commands::system::open_path,
            commands::system::get_system_info,
            commands::hotspot::has_notch,
            commands::hotspot::get_hotspot_enabled,
            commands::hotspot::set_hotspot_enabled,
            commands::hotspot::focus_main_window,
            commands::hotspot::hotspot_ready,
            commands::threads::list_codex_threads,
            commands::threads::export_codex_threads,
        ])
        .build(context)
        .expect("error while building AgentHarbor");

    let activation_watcher_guard = platform::single_instance::start_activation_watcher({
        let handle = app.handle().clone();
        move || commands::hotspot::force_reveal_main_window(&handle)
    })
    .map_err(|error| {
        eprintln!(
            "[AgentHarbor] failed to start single-instance activation watcher: {error}"
        );
        error
    })
    .ok();
    let activation_watcher_guard = Rc::new(RefCell::new(activation_watcher_guard));
    let single_instance_guard_for_exit = Rc::clone(&single_instance_guard);
    let activation_watcher_guard_for_exit = Rc::clone(&activation_watcher_guard);

    app.run(move |_app_handle, event| {
        if matches!(event, RunEvent::Exit) {
            let _ = activation_watcher_guard_for_exit.borrow_mut().take();
            let _ = single_instance_guard_for_exit.borrow_mut().take();
        }

        #[cfg(target_os = "macos")]
        if let RunEvent::Reopen { .. } = event {
            commands::hotspot::force_reveal_main_window(_app_handle);
        }
    });
}

fn load_tray_template_icon() -> Result<Image<'static>, String> {
    let reader = ImageReader::new(Cursor::new(include_bytes!("../icons/32x32.png")))
        .with_guessed_format()
        .map_err(|e| format!("failed to guess tray icon format: {e}"))?;
    let decoded = reader
        .decode()
        .map_err(|e| format!("failed to decode tray icon png: {e}"))?
        .to_rgba8();
    let (width, height) = decoded.dimensions();
    Ok(Image::new_owned(decoded.into_raw(), width, height))
}

fn schedule_startup_main_window_reveal(app: &tauri::AppHandle) {
    let handle = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(180));
        commands::hotspot::reveal_main_window(&handle);
    });
}
