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

    let repository = Repository::from_paths((*shared_paths).clone());
    let app = builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .manage(Mutex::new(repository))
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
                    let reconciliation_paths = paths.clone();
                    tauri::async_runtime::spawn(async move {
                        let _guard = core::routing_runtime::mutation_guard().await;
                        if let Err(error) =
                            core::routing_runtime::reconcile_on_startup(&reconciliation_paths)
                                .await
                        {
                            eprintln!("[AgentHarbor] local router startup failed: {error}");
                        }
                    });
                    // Refresh the models.dev catalog when it is stale, then keep
                    // checking hourly. Failures keep the last valid snapshot.
                    let handle = app.handle().clone();
                    let catalog_paths = paths.clone();
                    tauri::async_runtime::spawn(async move {
                        loop {
                            refresh_models_dev_if_due(&handle, &catalog_paths).await;
                            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                        }
                    });
                }
            }
            // SIGTERM (logout, `kill`), SIGHUP and Ctrl+C would otherwise end the
            // process without RunEvent::Exit and leave clients aimed at a dead router.
            #[cfg(unix)]
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    use tokio::signal::unix::{signal, SignalKind};
                    let (Ok(mut term), Ok(mut hangup), Ok(mut interrupt)) = (
                        signal(SignalKind::terminate()),
                        signal(SignalKind::hangup()),
                        signal(SignalKind::interrupt()),
                    ) else {
                        log::warn!("could not install termination signal handlers");
                        return;
                    };
                    tokio::select! {
                        _ = term.recv() => {}
                        _ = hangup.recv() => {}
                        _ = interrupt.recv() => {}
                    }
                    handle.exit(0);
                });
            }
            // Release builds have no console; this covers Ctrl+C in a dev terminal.
            // Shutdown/logoff arrives as WM_ENDSESSION and already ends in RunEvent::Exit.
            #[cfg(windows)]
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    if let Ok(mut ctrl_c) = tokio::signal::windows::ctrl_c() {
                        ctrl_c.recv().await;
                        handle.exit(0);
                    }
                });
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
            commands::auto_switch::spawn(app.handle().clone());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::client_config::list_client_configs,
            commands::client_config::preview_client_config,
            commands::client_config::apply_client_config,
            commands::client_config::restore_client_config,
            commands::client_config::release_client_config,
            commands::client_config::reapply_client_config,
            commands::accounts::list_codex_accounts,
            commands::usage::get_token_usage,
            commands::accounts::refresh_codex_accounts,
            commands::accounts::login_codex_account,
            commands::accounts::switch_codex_account,
            commands::accounts::delete_codex_account,
            commands::newapi::load_newapi_state,
            commands::newapi::reveal_newapi_provider_api_key,
            commands::newapi::load_route_diagnostics,
            commands::newapi::load_model_capabilities,
            commands::newapi::preview_newapi_model_resolutions,
            commands::newapi::preview_newapi_capability_clear,
            commands::newapi::apply_newapi_capability_clear,
            commands::newapi::load_models_dev_status,
            commands::newapi::refresh_models_dev_catalog,
            commands::newapi::set_newapi_failover,
            commands::newapi::set_newapi_compression,
            commands::newapi::set_newapi_model_price,
            commands::newapi::save_newapi_provider,
            commands::newapi::delete_newapi_provider,
            commands::newapi::set_newapi_provider_enabled,
            commands::newapi::set_newapi_provider_network,
            commands::newapi::test_newapi_provider,
            commands::newapi::fetch_newapi_models,
            commands::newapi::query_newapi_provider_balance,
            commands::newapi::set_newapi_balance_credentials,
            commands::newapi::set_newapi_balance_script,
            commands::newapi::set_newapi_balance_sharing,
            commands::newapi::set_newapi_smart_routing,
            commands::newapi::set_newapi_access_mode,
            commands::newapi::set_newapi_accountless,
            commands::newapi::import_newapi_providers,
            commands::newapi::export_newapi_providers,
            commands::image::generate_images,
            commands::image::cancel_image_generation,
            commands::image::list_image_key_sites,
            commands::image::delete_image_key,
            commands::image::image_key_status,
            commands::image::save_image_key,
            commands::image::save_image_file,
            commands::image::save_images_to_gallery,
            commands::image::set_gallery_directory,
            commands::image::list_gallery_images,
            commands::image::load_gallery_image,
            commands::image::load_gallery_thumbnail,
            commands::image::delete_gallery_image,
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
            commands::system::diagnose,
            commands::system::graceful_restart_for_update,
            commands::system::check_update_installability,
            commands::system::open_path,
            commands::system::get_system_info,
            commands::targets::restart_target,
            commands::threads::list_conversations,
            commands::threads::get_conversation_details,
            commands::threads::get_conversation_messages,
            commands::threads::export_conversations,
            commands::threads::delete_conversations,
            commands::threads::cancel_pending_conversation_deletions,
            commands::thread_recovery::preview_codex_thread_recovery,
            commands::thread_recovery::apply_codex_thread_recovery,
            commands::thread_recovery::adopt_codex_thread_router,
        ])
        .build(context)
        .expect("error while building AgentHarbor");

    let activation_watcher_guard = platform::single_instance::start_activation_watcher({
        let handle = app.handle().clone();
        move || commands::hotspot::force_reveal_main_window(&handle)
    })
    .map_err(|error| {
        eprintln!("[AgentHarbor] failed to start single-instance activation watcher: {error}");
        error
    })
    .ok();
    let activation_watcher_guard = Rc::new(RefCell::new(activation_watcher_guard));
    let single_instance_guard_for_exit = Rc::clone(&single_instance_guard);
    let activation_watcher_guard_for_exit = Rc::clone(&activation_watcher_guard);

    app.run(move |_app_handle, event| {
        if matches!(event, RunEvent::Exit) {
            // The proxy lives in this process. Restore every client that points
            // to it before exit; direct upstream configurations remain durable.
            // Tray quit, Windows shutdown/logoff (WM_ENDSESSION) and the Unix
            // termination signals handled in setup all arrive here.
            core::routing_runtime::release_router_clients(&shared_paths);
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

/// Background models.dev refresh: only when the catalog is stale or a previous
/// attempt failed and is due for a retry. Runs on the blocking pool because the
/// download uses the blocking client, and never blocks startup.
async fn refresh_models_dev_if_due(app: &tauri::AppHandle, paths: &CodexPaths) {
    use tauri::Manager;
    if !core::models_dev::should_refresh(&paths.app_data_dir) {
        return;
    }
    let proxy = app
        .state::<Mutex<Repository>>()
        .lock()
        .map(|repository| repository.api_proxy_config())
        .unwrap_or_default();
    let app_data_dir = paths.app_data_dir.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        core::models_dev::refresh(&app_data_dir, &proxy)
    })
    .await;
    if let Err(error) = result.unwrap_or_else(|_| Err("refresh task failed".into())) {
        eprintln!("[AgentHarbor] models.dev refresh failed: {error}");
    }
}
