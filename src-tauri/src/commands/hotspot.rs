use std::sync::Mutex;
use tauri::{AppHandle, Manager};

#[derive(Default)]
pub struct WindowRevealState(Mutex<RevealPulse>);
#[derive(Default)]
struct RevealPulse {
    generation: u64,
    baseline: Option<bool>,
}
impl RevealPulse {
    fn begin(&mut self, current: bool) -> u64 {
        self.baseline.get_or_insert(current);
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }
    fn finish(&mut self, generation: u64) -> Option<bool> {
        if generation != self.generation {
            return None;
        }
        self.baseline.take()
    }
}

/// Used by the tray menu; not exposed over IPC.
pub fn focus_main_window(app: AppHandle) -> Result<(), String> {
    force_reveal_main_window(&app);
    Ok(())
}

/// Show the main window without changing its persistent always-on-top state.
pub fn reveal_main_window(app: &AppHandle) {
    reveal_main_window_inner(app, false);
}

/// Startup's delayed focus must respect an early user close/minimize.
pub fn reveal_visible_main_window(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(win) = handle.get_webview_window("main") {
            if win.is_visible().unwrap_or(false) && !win.is_minimized().unwrap_or(false) {
                bring_main_window_forward(&win);
            }
        }
    });
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
                let reveal_state = handle.state::<WindowRevealState>();
                let Ok(mut pulse) = reveal_state.0.lock() else {
                    return;
                };
                let generation = pulse.begin(window_is_always_on_top(&win));
                drop(pulse);
                bring_main_window_force_forward(&win);

                let handle = handle.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(180));
                    let window_handle = handle.clone();
                    let _ = handle.run_on_main_thread(move || {
                        if let Some(win) = window_handle.get_webview_window("main") {
                            if let Ok(mut pulse) =
                                window_handle.state::<WindowRevealState>().0.lock()
                            {
                                if let Some(baseline) = pulse.finish(generation) {
                                    // Never reopen a window that was hidden during the pulse.
                                    set_window_topmost(&win, baseline);
                                }
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
                let visibility_handle = dock_handle.clone();
                let _ = dock_handle.run_on_main_thread(move || {
                    if visibility_handle
                        .get_webview_window("main")
                        .is_some_and(|win| win.is_visible().unwrap_or(false))
                    {
                        crate::platform::dock::set_dock_visible(true);
                    }
                });
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overlapping_reveals_restore_original_baseline() {
        let mut pulse = RevealPulse::default();
        let first = pulse.begin(false);
        let second = pulse.begin(true);
        assert_eq!(pulse.finish(first), None);
        assert_eq!(pulse.finish(second), Some(false));
        assert_eq!(pulse.finish(second), None);
    }
    #[test]
    fn persistent_topmost_survives_repeated_reveals() {
        let mut pulse = RevealPulse::default();
        pulse.begin(true);
        let latest = pulse.begin(true);
        assert_eq!(pulse.finish(latest), Some(true));
        let next = pulse.begin(false);
        assert_eq!(pulse.finish(next), Some(false));
    }
}

fn bring_main_window_forward(win: &tauri::WebviewWindow) {
    #[cfg(target_os = "macos")]
    {
        set_window_alpha(win, 1.0);
        crate::platform::dock::set_dock_visible(true);
        activate_macos_app();
    }
    let _ = win.show();
    #[cfg(windows)]
    if let Ok(hwnd) = win.hwnd() {
        // Tao samples minimized state before dispatch. A native restore also
        // handles activation arriving while the minimize animation is pending.
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindowVisible, ShowWindow, SW_RESTORE, SW_SHOW};
            if IsIconic(hwnd.0 as _) != 0 { ShowWindow(hwnd.0 as _, SW_RESTORE); }
            else if IsWindowVisible(hwnd.0 as _) == 0 { ShowWindow(hwnd.0 as _, SW_SHOW); }
        }
    }
    let _ = win.unminimize();
    let _ = win.set_focusable(true);
    let _ = win.set_focus();
}

fn bring_main_window_force_forward(win: &tauri::WebviewWindow) {
    bring_main_window_forward(win);
    set_window_topmost(win, true);
    let _ = win.set_focus();
}

#[cfg(windows)]
fn window_is_always_on_top(win: &tauri::WebviewWindow) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowLongW, GWL_EXSTYLE, WS_EX_TOPMOST};
    win.hwnd()
        .map(|hwnd| unsafe { GetWindowLongW(hwnd.0 as _, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST != 0 })
        .unwrap_or(false)
}
#[cfg(not(windows))]
fn window_is_always_on_top(win: &tauri::WebviewWindow) -> bool {
    win.is_always_on_top().unwrap_or(false)
}

#[cfg(windows)]
fn set_window_topmost(win: &tauri::WebviewWindow, topmost: bool) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    };
    if let Ok(hwnd) = win.hwnd() {
        // Tao's flag diff calls ShowWindow for a cached VISIBLE flag. Only
        // change the native z-order here, preserving visibility/minimize state.
        unsafe {
            SetWindowPos(
                hwnd.0 as _,
                if topmost {
                    HWND_TOPMOST
                } else {
                    HWND_NOTOPMOST
                },
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }
}
#[cfg(not(windows))]
fn set_window_topmost(win: &tauri::WebviewWindow, topmost: bool) {
    let _ = win.set_always_on_top(topmost);
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
