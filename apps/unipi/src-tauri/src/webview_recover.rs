//! E-13: a dead web process (OOM kill, GPU crash) used to leave a blank window
//! until the app was restarted. WebKitGTK reports it on the `WebView`
//! (`web-process-terminated`); we reload at once — WebKit spawns a fresh web
//! process for the reload — and remember why, so the UI can say so once it is
//! back (`take_webview_recovery`).

use std::sync::Mutex;

static PENDING: Mutex<Option<String>> = Mutex::new(None);

fn remember(reason: &str) {
    if let Ok(mut slot) = PENDING.lock() {
        *slot = Some(reason.to_string());
    }
}

/// The recovery notice, once: `Some(reason)` when the page was reloaded after
/// its web process died since the last call.
#[tauri::command]
pub fn take_webview_recovery() -> Option<String> {
    PENDING.lock().ok().and_then(|mut slot| slot.take())
}

/// Hooks the main window's web view. Linux only: WebKitGTK is the one engine
/// here that exposes the termination signal (WebView2/WKWebView recover alone).
#[cfg(target_os = "linux")]
pub fn watch<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    use tauri::Manager;
    use webkit2gtk::{WebProcessTerminationReason, WebViewExt};
    let Some(window) = app.get_webview_window("main") else { return };
    if let Err(error) = window.with_webview(|platform| {
        let view = platform.inner();
        view.connect_web_process_terminated(|view, reason| {
            let why = match reason {
                WebProcessTerminationReason::ExceededMemoryLimit => "it ran out of memory",
                WebProcessTerminationReason::Crashed => "it crashed",
                _ => "it stopped",
            };
            log::warn!("web process terminated ({reason:?}); reloading");
            remember(why);
            view.reload();
        });
    }) {
        log::warn!("webview recovery hook failed: {error}");
    }
}

#[cfg(not(target_os = "linux"))]
pub fn watch<R: tauri::Runtime>(_app: &tauri::AppHandle<R>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_notice_is_delivered_once() {
        // Serialised through the one static: drain first (other tests may share the process).
        let _ = take_webview_recovery();
        remember("it crashed");
        assert_eq!(take_webview_recovery().as_deref(), Some("it crashed"));
        assert_eq!(take_webview_recovery(), None);
    }
}
