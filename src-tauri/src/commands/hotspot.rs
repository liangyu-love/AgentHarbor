use tauri::{AppHandle, Manager};

/// Used by the tray menu; not exposed over IPC.
pub fn focus_main_window(app: AppHandle) -> Result<(), String> {
    force_reveal_main_window(&app);
    Ok(())
}

/// Show the main window without changing its persistent always-on-top state.
pub fn reveal_main_window(app: &AppHandle) {
    reveal_main_window_inner(app, false);
}

/// Bring the main window forward with a short always-on-top pulse.
pub fn force_reveal_main_window(app: &AppHandle) {
    reveal_main_window_inner(app, true);
}

fn reveal_main_window_inner(app: &AppHandle, force_front: bool) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(win) = handle.get_webview_window("main") {
            if force_front {
                let was_always_on_top = win.is_always_on_top().unwrap_or(false);
                bring_main_window_force_forward(&win);

                let handle = handle.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(180));
                    let window_handle = handle.clone();
                    let _ = handle.run_on_main_thread(move || {
                        if let Some(win) = window_handle.get_webview_window("main") {
                            bring_main_window_force_forward(&win);
                            if !was_always_on_top {
                                let _ = win.set_always_on_top(false);
                            }
                        }
                    });
                });
            } else {
                bring_main_window_forward(&win);
            }
        }

        #[cfg(target_os = "macos")]
        {
            let dock_handle = handle.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(50));
                let _ = dock_handle.run_on_main_thread(move || {
                    crate::platform::dock::set_dock_visible(true);
                });
            });
        }
    });
}

fn bring_main_window_forward(win: &tauri::WebviewWindow) {
    #[cfg(target_os = "macos")]
    {
        set_window_alpha(win, 1.0);
        crate::platform::dock::set_dock_visible(true);
        activate_macos_app();
    }
    let _ = win.show();
    let _ = win.unminimize();
    let _ = win.set_focusable(true);
    let _ = win.set_focus();
}

fn bring_main_window_force_forward(win: &tauri::WebviewWindow) {
    bring_main_window_forward(win);
    let _ = win.set_always_on_top(true);
    let _ = win.set_focus();
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
fn activate_macos_app() {
    use objc2_app_kit::{NSApplication, NSRunningApplication};
    use objc2_foundation::MainThreadMarker;

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    app.activateIgnoringOtherApps(true);
    let running = NSRunningApplication::currentApplication();
    let _ = running.activateWithOptions(
        objc2_app_kit::NSApplicationActivationOptions::ActivateAllWindows
            | objc2_app_kit::NSApplicationActivationOptions::ActivateIgnoringOtherApps,
    );
}

#[cfg(target_os = "macos")]
fn set_window_alpha(win: &tauri::WebviewWindow, alpha: f64) {
    use objc2::rc::Retained;
    use objc2_app_kit::NSWindow;

    let ns_window_ptr = match win.ns_window() {
        Ok(ptr) => ptr,
        Err(_) => return,
    };
    let Some(ns_window) = (unsafe { Retained::retain(ns_window_ptr as *mut NSWindow) }) else {
        return;
    };
    ns_window.setAlphaValue(alpha);
}
