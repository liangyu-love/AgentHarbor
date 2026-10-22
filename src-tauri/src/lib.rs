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
    // The elevated copy that optimizes memory does only that, before any setup.
    if let Some(code) = core::system_memory::run_from_args() {
        std::process::exit(code);
    }
    let shared_paths = Arc::new(CodexPaths::new());
    let mut context = tauri::generate_context!();
    if std::env::var_os("AGENT_HARBOR_CODEX_HOME").is_some_and(|value| !value.is_empty()) {
        for window in &mut context.config_mut().app.windows {
            window.data_directory = Some(shared_paths.app_data_dir.join("webview"));
        }
    }

    let activation_baseline = platform::single_instance::activation_baseline();
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
        .manage(core::account_login::LoginManager::default())
        .manage(commands::account_sources::SourceLoginManager::default())
        .manage(commands::hotspot::WindowRevealState::default())
        .manage(commands::desktop_exit::ExitCoordinator::default())
        .setup(|app| {
            core::autostart::refresh();
            if let Some(window) = app.get_webview_window("main") {
                // The window starts hidden; a sign-in start stays in the tray.
                if core::autostart::launched_at_sign_in() {
                    #[cfg(target_os = "macos")]
                    platform::dock::set_dock_visible(false);
                } else {
                    let _ = window.show();
                }
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
                    commands::newapi::spawn_balance_refresh(app.handle().clone(), paths.clone());
                    core::account_sources::checkin::spawn(paths.clone());
                    let reconciliation_paths = paths.clone();
                    tauri::async_runtime::spawn(async move {
                        {
                            let _guard = core::routing_runtime::mutation_guard().await;
                            if let Err(error) =
                                core::routing_runtime::reconcile_on_startup(&reconciliation_paths)
                                    .await
                            {
                                eprintln!("[AgentHarbor] local router startup failed: {error}");
                            }
                        }
                        loop {
                            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                            let _guard = core::routing_runtime::mutation_guard().await;
                            // Ticket reservations have a bounded lifetime even
                            // when the window closes before sending cancellation.
                            let _ = core::routing_runtime::reconcile(&reconciliation_paths).await;
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
                    commands::desktop_exit::request_exit(
                        &handle,
                        commands::desktop_exit::ExitIntent::Quit,
                    );
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
                        commands::desktop_exit::request_exit(
                            &handle,
                            commands::desktop_exit::ExitIntent::Quit,
                        );
                    }
                });
            }

            let tray_menu = commands::tray_menu::create_tray_menu(app.handle())
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
                // The pointer reaches the icon before a click opens the menu.
                .on_tray_icon_event(|tray, event| {
                    if matches!(event, tauri::tray::TrayIconEvent::Enter { .. }) {
                        commands::tray_menu::refresh_tray_menu(tray.app_handle());
                    }
                })
                .show_menu_on_left_click(true)
                .build(app)?;

            schedule_startup_main_window_reveal(app.handle());
            commands::auto_switch::spawn(app.handle().clone());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::aggregation::load_aggregation_state,
            commands::aggregation::load_aggregation_live,
            commands::aggregation::save_aggregate_model,
            commands::aggregation::delete_aggregate_model,
            commands::aggregation::select_codex_aggregate,
            commands::aggregation::set_codex_sources,
            commands::claude_desktop::get_claude_desktop_status,
            commands::claude_desktop::list_claude_desktop_candidates,
            commands::claude_desktop::preview_claude_desktop_change,
            commands::claude_desktop::commit_claude_desktop_change,
            commands::claude_desktop::cancel_claude_desktop_preview,
            commands::claude_desktop::probe_claude_desktop_provider,
            commands::claude_desktop::restart_claude_desktop,
            commands::desktop_exit::get_claude_desktop_exit_request,
            commands::desktop_exit::resolve_claude_desktop_exit,
            commands::client_config::list_client_configs,
            commands::client_config::list_client_files,
            commands::client_config::preview_client_config,
            commands::client_config::harness_reasoning_levels,
            commands::client_config::apply_client_config,
            commands::client_config::restore_client_config,
            commands::client_config::preview_claude_code_official_config,
            commands::client_config::switch_claude_code_official_config,
            commands::client_config::release_client_config,
            commands::client_config::discard_client_resume,
            commands::client_config::reapply_client_config,
            commands::remote::list_remote_hosts,
            commands::remote::ssh_config_aliases,
            commands::remote::save_remote_host,
            commands::remote::delete_remote_host,
            commands::remote::test_remote_host,
            commands::remote::remote_client_status,
            commands::remote::remote_codex_router_models,
            commands::remote::probe_remote_client,
            commands::remote::remote_search_providers,
            commands::remote::deploy_remote_search,
            commands::remote::remove_remote_search,
            commands::remote::test_remote_search,
            commands::remote::preview_remote_codex,
            commands::remote::apply_remote_codex,
            commands::remote::restore_remote_codex,
            commands::remote::release_remote_codex,
            commands::remote::preview_remote_client,
            commands::remote::apply_remote_client,
            commands::remote::restore_remote_client,
            commands::remote::release_remote_client,
            commands::accounts::list_codex_accounts,
            commands::usage::get_token_usage,
            commands::accounts::refresh_codex_accounts,
            commands::accounts::login_codex_account,
            commands::cursor::load_cursor_status,
            commands::cursor::set_cursor_ide,
            commands::cursor::set_cursor_cli,
            commands::cursor::set_cursor_sources,
            commands::cursor::set_cursor_tab_model,
            commands::cursor::set_cursor_commit_model,
            commands::cursor::trust_cursor_certificate,
            commands::cursor::distrust_cursor_certificate,
            commands::cursor::restart_cursor_ide,
            commands::cursor::open_cursor_terminal,
            commands::gateway::load_gateway_state,
            commands::gateway::create_gateway_key,
            commands::gateway::rename_gateway_key,
            commands::gateway::regenerate_gateway_key,
            commands::gateway::delete_gateway_key,
            commands::gateway::reveal_gateway_key,
            commands::gateway::set_gateway_keep_running,
            commands::gateway::set_gateway_aggregate_prefix,
            commands::gateway::set_gateway_sources,
            commands::gateway::export_gateway_connection,
            commands::account_sources::list_account_sources,
            commands::account_sources::sign_in_account_source,
            commands::account_sources::cancel_account_source_sign_in,
            commands::account_sources::activate_account_source,
            commands::account_sources::remove_account_source,
            commands::account_sources::refresh_account_source_quota,
            commands::account_sources::refresh_account_source_models,
            commands::account_sources::set_account_source_exposed,
            commands::account_sources::set_account_source_checkin,
            commands::account_sources::set_account_source_checkin_off,
            commands::account_sources::check_in_account_source,
            commands::account_sources::set_account_source_hidden_models,
            commands::account_sources::set_account_source_project,
            commands::accounts::cancel_codex_account_login,
            commands::accounts::switch_codex_account,
            commands::accounts::delete_codex_account,
            commands::newapi::load_newapi_state,
            commands::newapi::load_router_port_owner,
            commands::newapi::end_router_port_owner,
            commands::newapi::set_router_port,
            commands::newapi::load_codex_subagents,
            commands::newapi::save_codex_subagents,
            commands::newapi::reveal_newapi_provider_api_key,
            commands::newapi::load_route_diagnostics,
            commands::newapi::clear_route_diagnostics,
            commands::newapi::load_model_capabilities,
            commands::newapi::preview_newapi_model_resolutions,
            commands::newapi::preview_newapi_capability_clear,
            commands::newapi::apply_newapi_capability_clear,
            commands::newapi::load_models_dev_status,
            commands::newapi::refresh_models_dev_catalog,
            commands::newapi::set_router_web_search_provider,
            commands::newapi::set_newapi_compression,
            commands::newapi::set_newapi_model_price,
            commands::newapi::save_newapi_provider,
            commands::newapi::delete_newapi_provider,
            commands::newapi::set_newapi_provider_enabled,
            commands::newapi::set_newapi_provider_network,
            commands::newapi::test_newapi_provider,
            commands::newapi::probe_newapi_provider_search,
            commands::newapi::fetch_newapi_models,
            commands::newapi::query_newapi_provider_balance,
            commands::newapi::set_newapi_balance_refresh_interval,
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
            commands::image_prompts::load_image_prompt_library,
            commands::image_prompts::save_image_prompt_library,
            commands::image_prompts::import_image_prompt_library,
            commands::image_prompts::export_image_prompt_library,
            commands::image_prompts::add_image_prompt_presets,
            commands::image_prompt_assist::load_prompt_assist,
            commands::image_prompt_assist::save_prompt_assist_settings,
            commands::image_prompt_assist::optimize_image_prompt,
            commands::image_materials::load_image_materials,
            commands::image_materials::save_image_materials,
            commands::image_materials::add_image_materials,
            commands::image_materials::load_image_material,
            commands::system::load_snapshot,
            commands::system::is_updater_configured,
            commands::system::set_ui_language,
            commands::newapi::load_codex_menu_overflow,
            commands::system::updater_proxy,
            commands::system::get_autostart_status,
            commands::system::set_autostart,
            commands::mcp::load_mcp_library,
            commands::mcp::import_mcp_servers,
            commands::mcp::save_mcp_server,
            commands::mcp::set_mcp_servers,
            commands::mcp::remove_mcp_server,
            commands::mcp::resync_mcp_servers,
            commands::mcp::parse_mcp_paste,
            commands::skills::load_skill_library,
            commands::skills::set_skills,
            commands::skills::add_skill,
            commands::skills::import_skills,
            commands::skills::uninstall_skill,
            commands::skills::restore_skill,
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
            commands::system::load_terminal_proxy_config,
            commands::system::set_terminal_proxy_config,
            commands::system::test_terminal_proxy_config,
            commands::system::open_proxy_terminal,
            commands::system::get_usage_refresh_interval,
            commands::system::set_usage_refresh_interval,
            commands::system::diagnose,
            commands::system::graceful_restart_for_update,
            commands::system::check_update_installability,
            commands::system::open_path,
            commands::system::get_system_info,
            commands::memory::get_memory_status,
            commands::memory::get_memory_areas,
            commands::memory::set_memory_areas,
            commands::memory::optimize_memory,
            commands::targets::restart_target,
            commands::targets::load_client_launch,
            commands::targets::launch_client,
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

    let activation_watcher_guard =
        platform::single_instance::start_activation_watcher(activation_baseline, {
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
        if let RunEvent::ExitRequested { api, .. } = &event {
            if !_app_handle
                .state::<commands::desktop_exit::ExitCoordinator>()
                .approved()
            {
                api.prevent_exit();
                commands::desktop_exit::request_exit(
                    _app_handle,
                    commands::desktop_exit::ExitIntent::Quit,
                );
            }
        }
        if matches!(event, RunEvent::Exit) {
            _app_handle
                .state::<core::account_login::LoginManager>()
                .shutdown();
            _app_handle
                .state::<commands::account_sources::SourceLoginManager>()
                .0
                .shutdown();
            // The proxy lives in this process. Restore every client that points
            // to it before exit; direct upstream configurations remain durable.
            // Tray quit, Windows shutdown/logoff (WM_ENDSESSION) and the Unix
            // termination signals handled in setup all arrive here.
            core::routing_runtime::release_router_clients(&shared_paths);
            // Forced shutdown may bypass preflight. Preserve Desktop State and
            // its journal so the next process can recover the fixed gateway.
            core::router_proxy::stop_unchecked();
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
        commands::hotspot::reveal_visible_main_window(&handle);
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
