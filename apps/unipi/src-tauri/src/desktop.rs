//! Desktop-app native bits (UNI-117 part 2): system tray, OS notifications,
//! window title. Everything that touches a desktop-only plugin or the tray
//! is `#[cfg(desktop)]`; the commands still exist on Android (so the
//! handler list is the same everywhere) and answer `E_UNSUPPORTED` / no-op.
//!
//! - Tray: the app icon labelled **UniPi (oino)** (host.md §5.1: the owner's old
//!   unipi has a tray icon too), with "Open Unipi" (first row), "Hide Unipi" and "Quit",
//!   and a status line + tooltip — "2 phones · 3 sessions running · 1 needs
//!   you" (§5.3) — fed by `desktop_status` (needs-you/running counts) and
//!   `host_runtime` (paired phones).
//! - `desktop_notify { title, body }`: a native notification
//!   (tauri-plugin-notification). The UI decides when (needs you / finished).
//! - `desktop_set_title { title }`: the window title follows the screen/chat.
//! - Window size/position are remembered by tauri-plugin-window-state
//!   (registered in lib.rs; nothing to call).

use unipi_proto::{AppErrorCode, AppErrorDto};

#[cfg(desktop)]
pub const TRAY_ID: &str = "main";

/// The tray label: the app's visible name "Unipi" (owner decision 2026-11; the
/// old UniPi 0.5.3 app shows "UniPi"). The tooltip adds the status line.
pub const TRAY_TITLE: &str = "Unipi";

/// The event the UI listens for to show the "Quit UniPi?" sheet (UNI-237):
/// `{message, paired}`, sent only when quitting really needs asking.
pub const QUIT_CONFIRM_EVENT: &str = "unipi-quit-confirm";

/// Pure: the tray's §5.3 status line — paired phones, running sessions and the
/// needs-you count ("2 phones · 3 sessions running · 1 needs you").
pub fn tray_line(phones: usize, running: u32, needs: u32) -> String {
    let phones = match phones {
        0 => "No phones".to_string(),
        1 => "1 phone".to_string(),
        n => format!("{n} phones"),
    };
    let running = match running {
        0 => "No sessions running".to_string(),
        1 => "1 session running".to_string(),
        n => format!("{n} sessions running"),
    };
    let needs = match needs {
        0 => "Nobody needs you".to_string(),
        1 => "1 needs you".to_string(),
        n => format!("{n} need you"),
    };
    format!("{phones} · {running} · {needs}")
}

/// Pure: the tray's status line for `needs` waiting agents and `running` working ones.
pub fn status_line(needs: u32, running: u32) -> String {
    match (needs, running) {
        (0, 0) => "All quiet".to_string(),
        (0, r) => format!("{r} {} working", if r == 1 { "agent" } else { "agents" }),
        (n, 0) => format!(
            "{n} {} you",
            if n == 1 { "agent needs" } else { "agents need" }
        ),
        (n, r) => format!(
            "{n} {} you · {r} working",
            if n == 1 { "agent needs" } else { "agents need" }
        ),
    }
}

/// Pure: the tray tooltip (the label first, then the status line).
#[cfg_attr(not(desktop), allow(dead_code))]
pub fn tooltip(status: &str) -> String {
    format!("{TRAY_TITLE} — {status}")
}

#[cfg(desktop)]
fn build_menu<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    visible: bool,
) -> tauri::Result<tauri::menu::Menu<R>> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
    // "Open Unipi" is the first row on purpose: Ubuntu's AppIndicator
    // extension (and libappindicator) turn a single left click into "show
    // the menu" — only a double click reaches `Activate` — so the first row
    // must be the one-click way in. It always shows + unminimizes + focuses
    // (creating the window again when it is gone); it never hides.
    let open = MenuItem::with_id(app, MENU_OPEN, "Open Unipi", true, None::<&str>)?;
    let hide = MenuItem::with_id(app, MENU_HIDE, "Hide Unipi", visible, None::<&str>)?;
    let status_item = MenuItem::with_id(app, "status", current_line(), false, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let mascot = tauri::menu::CheckMenuItem::with_id(
        app,
        "mascot",
        "Mascot window",
        true,
        mascot_visible(app),
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Quit Unipi", true, None::<&str>)?;
    Menu::with_items(
        app,
        &[&open, &hide, &sep, &status_item, &mascot, &sep2, &quit],
    )
}

/// Tray menu row ids (the first row is always "Open Unipi").
pub const MENU_OPEN: &str = "open";
pub const MENU_HIDE: &str = "hide";

#[cfg(desktop)]
fn main_visible<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> bool {
    use tauri::Manager;
    app.get_webview_window("main")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(true)
}

/// The main window, created again from the config when it doesn't exist (it
/// was destroyed, or never built). `None` only when building it fails.
#[cfg(desktop)]
fn ensure_main<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Option<tauri::WebviewWindow<R>> {
    use tauri::Manager;
    if let Some(window) = app.get_webview_window("main") {
        return Some(window);
    }
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == "main")?
        .clone();
    match tauri::WebviewWindowBuilder::from_config(app, &config).and_then(|b| b.build()) {
        Ok(window) => {
            log::info!("main window was gone: created it again");
            Some(window)
        }
        Err(error) => {
            log::warn!("main window: could not create it again: {error}");
            None
        }
    }
}

// The three numbers behind the tray's status line (atomics: the tray menu is
// rebuilt from any thread and a poisoned lock must never break the tray).
#[cfg(desktop)]
static NEEDS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
#[cfg(desktop)]
static RUNNING: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
#[cfg(desktop)]
static PHONES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The tray's §5.3 line right now (status item + tooltip).
#[cfg(desktop)]
fn current_line() -> String {
    use std::sync::atomic::Ordering::Relaxed;
    tray_line(
        PHONES.load(Relaxed),
        RUNNING.load(Relaxed),
        NEEDS.load(Relaxed),
    )
}

/// Sets how many phones are paired (from `host_runtime`, §5.3).
#[cfg(desktop)]
pub fn set_phone_count<R: tauri::Runtime>(app: &tauri::AppHandle<R>, paired: usize) {
    PHONES.store(paired, std::sync::atomic::Ordering::Relaxed);
    refresh_menu(app);
}

/// Shows (`true`) or hides the main window — the close dialog's "Minimize to
/// tray" and the tray's "Hide Unipi" share it (§5.4).
#[cfg(desktop)]
pub fn hide_main<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    use tauri::Manager;
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let _ = window.hide();
    refresh_menu_as(app, Some(false));
}

#[cfg(desktop)]
fn refresh_menu<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    refresh_menu_as(app, None);
}

/// Last known window visibility set by the tray rows (GTK's own answer lags).
#[cfg(desktop)]
static VISIBLE: std::sync::Mutex<Option<bool>> = std::sync::Mutex::new(None);

#[cfg(desktop)]
fn refresh_menu_as<R: tauri::Runtime>(app: &tauri::AppHandle<R>, visible: Option<bool>) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let line = current_line();
    let visible = match visible {
        Some(v) => {
            if let Ok(mut known) = VISIBLE.lock() {
                *known = Some(v);
            }
            v
        }
        None => VISIBLE
            .lock()
            .ok()
            .and_then(|k| *k)
            .unwrap_or_else(|| main_visible(app)),
    };
    if let Ok(menu) = build_menu(app, visible) {
        let _ = tray.set_menu(Some(menu));
    }
    // §5.1/§5.3: the label and the line, so this tray is distinguishable from
    // the owner's old unipi one.
    let _ = tray.set_title(Some(TRAY_TITLE));
    let _ = tray.set_tooltip(Some(tooltip(&line)));
}

/// Creates the tray icon. Called from `setup` on desktop; a failure (no
/// status-notifier host on this desktop) is logged, never fatal.
#[cfg(desktop)]
pub fn setup_tray<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> tauri::Result<()> {
    use tauri::tray::TrayIconBuilder;
    let menu = build_menu(app, true)?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .title(TRAY_TITLE)
        .tooltip(tooltip(&current_line()))
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            MENU_OPEN => show_main(app),
            MENU_HIDE => hide_main(app),
            "mascot" => {
                let show = !mascot_visible(app);
                let _ = set_mascot_window(
                    app,
                    show,
                    MASCOT_CRAB.load(std::sync::atomic::Ordering::Relaxed),
                );
                use tauri::Emitter;
                let _ = app.emit_to("main", "mascot-window-changed", show);
                refresh_menu(app);
            }
            "quit" => crate::lifecycle::request_quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // A left click (or double click) opens the window: shows it when
            // it is hidden or minimized, creates it again when it is gone,
            // and focuses it. It never hides it (the menu's "Hide" does).
            // Windows/macOS deliver the click directly
            // (`show_menu_on_left_click(false)`, right click = menu). Linux
            // uses the KSNI backend (Cargo.toml): GNOME's AppIndicator
            // extension sends `Activate` on a double click (a single click
            // opens the menu, whose first row is "Open Unipi"); KDE and most
            // other SNI hosts send it on a single click.
            if opens_main(&event) {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// Pure: does this tray icon event open the main window?
#[cfg(desktop)]
fn opens_main(event: &tauri::tray::TrayIconEvent) -> bool {
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
    matches!(
        event,
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } | TrayIconEvent::DoubleClick {
            button: MouseButton::Left,
            ..
        }
    )
}

/// The crab (mascot window) was clicked: open the main window from Rust.
/// The main page's own `mascot-open` listener still opens the chat, but a
/// hidden webview can be too throttled to bring its window back, so showing
/// it must not depend on the page.
#[cfg(desktop)]
pub fn watch_mascot_open<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    use tauri::Listener;
    let handle = app.clone();
    app.listen_any("mascot-open", move |_| show_main(&handle));
}

// ── Mascot window (UNI-200) ────────────────────────────────────────────
// A small transparent, frameless, always-on-top window (label `mascot`,
// mascot.html) with the animated Unicrab + reply bubbles. Off by default;
// Settings → Unicrab or the tray's "Mascot window" turns it on. The main
// window feeds it over Tauri events (`mascot-feed`).

#[cfg_attr(not(desktop), allow(dead_code))]
pub const MASCOT_LABEL: &str = "mascot";
/// UNI-255: crab widths the window accepts (Settings → Unicrab → size), default 96.
#[cfg_attr(not(desktop), allow(dead_code))]
pub const MASCOT_CRAB_SIZES: [u32; 3] = [72, 96, 128];
#[cfg_attr(not(desktop), allow(dead_code))]
pub const MASCOT_CRAB_DEFAULT: u32 = 96;
/// The last size the page asked for (the tray toggle reuses it).
#[cfg_attr(not(desktop), allow(dead_code))]
static MASCOT_CRAB: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(MASCOT_CRAB_DEFAULT);
/// Room for the reply bubble to the crab's left.
const MASCOT_BUBBLE_ROOM: f64 = 276.0;

/// Pure (UNI-255): the window's logical size for a crab width — the crab
/// (24×20 cells) plus a small margin, and room for a bubble to its left.
/// Everything else is transparent.
#[cfg_attr(not(desktop), allow(dead_code))]
pub fn mascot_size(crab: u32) -> (f64, f64) {
    let crab = if MASCOT_CRAB_SIZES.contains(&crab) {
        crab
    } else {
        MASCOT_CRAB_DEFAULT
    } as f64;
    let crab_h = (crab * 20.0 / 24.0).round();
    (crab + 16.0 + MASCOT_BUBBLE_ROOM, (crab_h + 16.0).max(104.0))
}

/// Pure: where the mascot window goes on a monitor (bottom-right, above a taskbar margin).
#[cfg_attr(not(desktop), allow(dead_code))]
pub fn mascot_position(monitor: (f64, f64), origin: (f64, f64), size: (f64, f64)) -> (f64, f64) {
    let margin = 24.0;
    let x = origin.0 + (monitor.0 - size.0 - margin).max(0.0);
    let y = origin.1 + (monitor.1 - size.1 - margin * 3.0).max(0.0);
    (x, y)
}

#[cfg(desktop)]
fn mascot_visible<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> bool {
    use tauri::Manager;
    app.get_webview_window(MASCOT_LABEL)
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false)
}

#[cfg(desktop)]
fn set_mascot_window<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    show: bool,
    crab: u32,
) -> tauri::Result<bool> {
    use tauri::Manager;
    let size = mascot_size(crab);
    if let Some(window) = app.get_webview_window(MASCOT_LABEL) {
        if show {
            // A new crab size: grow/shrink keeping the bottom-right corner put.
            let scale = window.scale_factor().unwrap_or(1.0);
            if let (Ok(inner), Ok(pos)) = (window.inner_size(), window.outer_position()) {
                let inner = inner.to_logical::<f64>(scale);
                if (inner.width - size.0).abs() > 0.5 || (inner.height - size.1).abs() > 0.5 {
                    let pos = pos.to_logical::<f64>(scale);
                    let _ = window.set_size(tauri::LogicalSize::new(size.0, size.1));
                    let _ = window.set_position(tauri::LogicalPosition::new(
                        pos.x + inner.width - size.0,
                        pos.y + inner.height - size.1,
                    ));
                }
            }
            window.show()?;
        } else {
            window.close()?;
        }
        return Ok(show);
    }
    if !show {
        return Ok(false);
    }
    // UNI-255: fully transparent — no decorations, no shadow, transparent
    // webview + RGBA visual (tao picks the screen's RGBA visual for a
    // transparent window on Linux; needs a compositor, which GNOME always has).
    let mut builder = tauri::WebviewWindowBuilder::new(
        app,
        MASCOT_LABEL,
        tauri::WebviewUrl::App("mascot.html".into()),
    )
    .title("Unicrab")
    .inner_size(size.0, size.1)
    .resizable(false)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .shadow(false)
    .focused(false);
    #[cfg(not(target_os = "macos"))]
    {
        builder = builder
            .transparent(true)
            .background_color(tauri::window::Color(0, 0, 0, 0));
    }
    if let Some(monitor) = app.primary_monitor().ok().flatten() {
        let scale = monitor.scale_factor();
        let size = monitor.size().to_logical::<f64>(scale);
        let origin = monitor.position().to_logical::<f64>(scale);
        let (x, y) = mascot_position(
            (size.width, size.height),
            (origin.x, origin.y),
            mascot_size(crab),
        );
        builder = builder.position(x, y);
    }
    builder.build()?;
    Ok(true)
}

/// Shows (creates) or closes the always-on-top mascot window, sized for the
/// crab width `size` (UNI-255; default 96). Answers whether it's shown.
#[tauri::command]
pub fn mascot_window(
    app: tauri::AppHandle,
    show: bool,
    size: Option<u32>,
) -> Result<bool, AppErrorDto> {
    #[cfg(desktop)]
    {
        let crab = size.unwrap_or(MASCOT_CRAB_DEFAULT);
        MASCOT_CRAB.store(crab, std::sync::atomic::Ordering::Relaxed);
        let shown = set_mascot_window(&app, show, crab).map_err(|e| {
            AppErrorDto::new(
                AppErrorCode::E_INTERNAL,
                format!("mascot window: {e}"),
                true,
            )
        })?;
        refresh_menu(&app);
        Ok(shown)
    }
    #[cfg(not(desktop))]
    {
        let _ = (app, show, size);
        Err(unsupported("The mascot window"))
    }
}

/// A second launch of the app (fix5-host, tauri-plugin-single-instance): the
/// running instance shows, unminimizes and focuses its main window instead of
/// a second process starting (and spawning a second host that fights over
/// `host.lock`).
#[cfg(desktop)]
pub fn show_main<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(window) = ensure_main(app) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
    refresh_menu_as(app, Some(true));
}

/// The mascot window was clicked: bring the main window to the front.
#[tauri::command]
pub fn mascot_focus_main(app: tauri::AppHandle) -> Result<(), AppErrorDto> {
    #[cfg(desktop)]
    {
        show_main(&app);
        Ok(())
    }
    #[cfg(not(desktop))]
    {
        let _ = app;
        Err(unsupported("The mascot window"))
    }
}

#[cfg(not(desktop))]
fn unsupported(what: &str) -> AppErrorDto {
    AppErrorDto::new(
        AppErrorCode::E_UNSUPPORTED,
        format!("{what} is desktop only"),
        false,
    )
}

/// The UI's status for the tray: how many agents need you / are working.
#[tauri::command]
pub fn desktop_status(
    app: tauri::AppHandle,
    needs: u32,
    running: u32,
) -> Result<String, AppErrorDto> {
    let line = status_line(needs, running);
    #[cfg(desktop)]
    {
        NEEDS.store(needs, std::sync::atomic::Ordering::Relaxed);
        RUNNING.store(running, std::sync::atomic::Ordering::Relaxed);
        refresh_menu(&app);
    }
    #[cfg(not(desktop))]
    let _ = app;
    Ok(line)
}

/// A native OS notification (desktop app only).
#[tauri::command]
pub fn desktop_notify(
    app: tauri::AppHandle,
    title: String,
    body: String,
) -> Result<(), AppErrorDto> {
    #[cfg(desktop)]
    {
        use tauri_plugin_notification::NotificationExt;
        app.notification()
            .builder()
            .title(title)
            .body(body)
            .show()
            .map_err(|e| {
                AppErrorDto::new(
                    AppErrorCode::E_INTERNAL,
                    format!("notification failed: {e}"),
                    true,
                )
            })
    }
    #[cfg(not(desktop))]
    {
        let _ = (app, title, body);
        Err(unsupported("Native notifications"))
    }
}

/// The window title (follows the active screen / chat).
#[tauri::command]
pub fn desktop_set_title(app: tauri::AppHandle, title: String) -> Result<(), AppErrorDto> {
    #[cfg(desktop)]
    {
        use tauri::Manager;
        if let Some(window) = app.get_webview_window("main") {
            window
                .set_title(&title)
                .map_err(|e| AppErrorDto::new(AppErrorCode::E_INTERNAL, e.to_string(), false))?;
        }
        Ok(())
    }
    #[cfg(not(desktop))]
    {
        let _ = (app, title);
        Err(unsupported("Window titles"))
    }
}

// ── Window size (UNI-117 QA) ───────────────────────────────────────────
// tauri-plugin-window-state saves `inner_size()` and restores it with
// `set_size()`. On GTK with client-side decorations (GNOME/Wayland) the
// two differ by the decoration extent (tauri plugins-workspace #3553), so
// the window grew by ~52×89 px on EVERY launch. The plugin keeps position /
// maximized / fullscreen; the size is ours: we save the webview's size (the
// content area, which is exactly what `set_size` sets) and restore it.

pub const SIZE_FILE: &str = "window-size.json";
/// Never restore a window smaller than this (the shell's 900px breakpoint stays reachable).
pub const MIN_RESTORE: (u32, u32) = (480, 360);

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SavedSize {
    pub width: u32,
    pub height: u32,
}

/// Pure: the size to restore from the saved file contents (None = keep the config default).
pub fn restore_size(saved: Option<&str>) -> Option<SavedSize> {
    let size: SavedSize = serde_json::from_str(saved?).ok()?;
    (size.width >= MIN_RESTORE.0
        && size.height >= MIN_RESTORE.1
        && size.width <= 16384
        && size.height <= 16384)
        .then_some(size)
}

#[cfg(desktop)]
fn size_path<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Option<std::path::PathBuf> {
    use tauri::Manager;
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join(SIZE_FILE))
}

/// Restores the main window's content size (call from `setup`).
#[cfg(desktop)]
pub fn restore_window_size<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    use tauri::Manager;
    let Some(path) = size_path(app) else { return };
    let text = std::fs::read_to_string(&path).ok();
    let Some(size) = restore_size(text.as_deref()) else {
        return;
    };
    if let Some(window) = app.get_webview_window("main") {
        let maximized = window.is_maximized().unwrap_or(false);
        if !maximized {
            let _ = window.set_size(tauri::PhysicalSize::new(size.width, size.height));
        }
    }
}

/// Saves the main window's content size (on close / exit; skipped while maximized or minimized).
#[cfg(desktop)]
pub fn save_window_size<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    use tauri::Manager;
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if window.is_maximized().unwrap_or(false) || window.is_minimized().unwrap_or(false) {
        return;
    }
    let webview: &tauri::Webview<R> = window.as_ref();
    let Ok(size) = webview.size() else { return };
    if size.width == 0 || size.height == 0 {
        return;
    }
    let Some(path) = size_path(app) else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let saved = SavedSize {
        width: size.width,
        height: size.height,
    };
    if let Ok(json) = serde_json::to_string(&saved) {
        let _ = std::fs::write(path, json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_lines() {
        assert_eq!(status_line(0, 0), "All quiet");
        assert_eq!(status_line(0, 1), "1 agent working");
        assert_eq!(status_line(0, 3), "3 agents working");
        assert_eq!(status_line(1, 0), "1 agent needs you");
        assert_eq!(status_line(2, 0), "2 agents need you");
        assert_eq!(status_line(2, 1), "2 agents need you · 1 working");
        assert_eq!(tooltip("All quiet"), "Unipi — All quiet");
    }

    #[test]
    fn the_tray_line_is_phones_sessions_and_needs_you() {
        // §5.3: "N phones · M sessions running · needs-you count".
        assert_eq!(
            tray_line(2, 3, 1),
            "2 phones · 3 sessions running · 1 needs you"
        );
        assert_eq!(
            tray_line(1, 1, 2),
            "1 phone · 1 session running · 2 need you"
        );
        assert_eq!(
            tray_line(0, 0, 0),
            "No phones · No sessions running · Nobody needs you"
        );
        // §5.1: the tray label differs from the owner's old unipi one.
        assert_eq!(TRAY_TITLE, "Unipi");
        assert!(tooltip(&tray_line(2, 3, 1)).starts_with("Unipi"));
    }

    #[test]
    fn restore_size_parses_and_bounds() {
        assert_eq!(
            restore_size(Some(r#"{"width":1280,"height":820}"#)),
            Some(SavedSize {
                width: 1280,
                height: 820
            })
        );
        assert_eq!(restore_size(None), None);
        assert_eq!(restore_size(Some("garbage")), None);
        assert_eq!(restore_size(Some(r#"{"width":10,"height":10}"#)), None);
        assert_eq!(restore_size(Some(r#"{"width":99999,"height":800}"#)), None);
        // Round trip: what we save is what we restore (no growth per launch).
        let saved = SavedSize {
            width: 1332,
            height: 909,
        };
        assert_eq!(
            restore_size(Some(&serde_json::to_string(&saved).unwrap())),
            Some(saved)
        );
    }

    #[test]
    fn mascot_sits_bottom_right_on_its_monitor() {
        let size = mascot_size(96);
        let (x, y) = mascot_position((1920.0, 1080.0), (0.0, 0.0), size);
        assert_eq!(x, 1920.0 - size.0 - 24.0);
        assert_eq!(y, 1080.0 - size.1 - 72.0);
        // A second monitor to the right: offset by its origin.
        let (x2, _) = mascot_position((1920.0, 1080.0), (1920.0, 0.0), size);
        assert_eq!(x2, x + 1920.0);
        // A tiny screen never goes negative.
        assert_eq!(
            mascot_position((100.0, 100.0), (0.0, 0.0), size),
            (0.0, 0.0)
        );
    }

    #[test]
    fn mascot_window_fits_the_crab_size() {
        // 24×20 cells: height follows the width; bubble room on the left.
        assert_eq!(mascot_size(128), (128.0 + 16.0 + 276.0, 107.0 + 16.0));
        assert_eq!(mascot_size(72).1, 104.0); // at least a bubble's height
        assert!(mascot_size(128).0 > mascot_size(96).0 && mascot_size(96).0 > mascot_size(72).0);
        // Anything else is the default.
        assert_eq!(mascot_size(5000), mascot_size(MASCOT_CRAB_DEFAULT));
    }

    #[test]
    fn tray_menu_opens_first() {
        assert_eq!(MENU_OPEN, "open");
        assert_ne!(MENU_OPEN, MENU_HIDE);
    }
}
