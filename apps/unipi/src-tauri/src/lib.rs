//! UniPi App — Tauri core entry point.
//!
//! This is the mobile (and desktop) app core described in
//! `unipi-app/DESIGN.md` §5.2: it hosts the connection manager and
//! exposes typed commands to the SolidJS UI. For M0 it only exposes
//! `app_info`, used by the UI to show version/platform and check
//! protocol compatibility.

#[cfg(desktop)]
mod app_update;
#[cfg(desktop)]
pub mod setup;
/// Mobile stand-in for the PATH helpers `host_supervisor`/`setup` use
/// (they compile everywhere but only run on desktop; setup needs reqwest).
#[cfg(mobile)]
pub mod setup {
    pub fn merged_path() -> String {
        String::new()
    }
    pub async fn merged_path_here(_home: &std::path::Path) -> String {
        String::new()
    }
}
#[cfg(desktop)]
use setup::install;
mod blob;
mod board;
mod browser;
mod chat;
mod commands;
mod cron;
mod desktop;
mod forward;
mod fs;
mod host_store;
mod host_supervisor;
mod lifecycle;
mod hosts_commands;
mod local_files;
mod local_host;
#[cfg(target_os = "ios")]
mod ios_webview;
mod net;
mod op_stream;
mod pair_phone;
mod path_kind;
mod push;
mod retry;
mod sharing;
mod term;

use commands::app_info;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .manage(net::NetState::default())
        .manage(term::TermRegistry::new())
        .manage(board::BoardStreams::default())
        .manage(chat::SessionStreams::default())
        .manage(forward::ForwardRegistry::default())
        .manage(browser::BrowserStreams::default())
        .manage(pair_phone::PairPhone::default())
        .manage(host_supervisor::Shared::default())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_log::Builder::new().level(log::LevelFilter::Info).level_for("iroh", log::LevelFilter::Warn).level_for("noq_proto", log::LevelFilter::Warn).build())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .invoke_handler(tauri::generate_handler![
            app_info,
            hosts_commands::pair_host,
            hosts_commands::list_hosts,
            hosts_commands::forget_host,
            hosts_commands::list_sessions,
            hosts_commands::mux_tree,
            hosts_commands::mux_preview,
            hosts_commands::mux_action,
            hosts_commands::shortcuts,
            term::term_open,
            term::term_input,
            term::term_resize,
            term::term_close,
            term::term_list,
            term::term_kill,
            hosts_commands::host_status,
            hosts_commands::set_relay,
            hosts_commands::pairing_preview,
            hosts_commands::notify_config,
            hosts_commands::notify_register,
            hosts_commands::notify_test,
            board::board_availability,
            board::board_request,
            board::board_events,
            board::board_events_close,
            blob::blob_put_file,
            blob::upload_stage_begin,
            blob::upload_stage_chunk,
            blob::upload_stage_drop,
            blob::blob_put_bytes,
            blob::blob_get_file,
            blob::blob_get_bytes,
            chat::sessions_list,
            chat::session_open,
            chat::session_new,
            chat::session_fork,
            chat::session_cmd,
            chat::session_stream_close,
            chat::session_archive,
            chat::session_rename,
            chat::session_close,
            chat::session_meta_set,
            chat::events_open,
            chat::events_close,
            chat::events_focus,
            fs::fs_roots,
            fs::fs_list,
            fs::fs_stat,
            fs::fs_mkdir,
            fs::fs_rename,
            fs::fs_delete,
            fs::fs_search,
            fs::fs_read,
            fs::fs_write,
            fs::fs_import,
            fs::git_op,
            fs::git_watch,
            cron::cron_list,
            cron::cron_show,
            cron::cron_runs,
            cron::cron_log,
            cron::cron_add,
            cron::cron_edit,
            cron::cron_rm,
            cron::cron_enable,
            cron::cron_disable,
            cron::cron_run,
            cron::cron_stop,
            cron::cron_pause_all,
            cron::cron_resume_all,
            cron::cron_status,
            cron::cron_parse,
            cron::cron_settings_get,
            cron::cron_settings_set,
            cron::cron_open_session,
            forward::ports_list,
            forward::forward_start,
            forward::forward_stop,
            forward::forward_list,
            browser::browser_status,
            browser::browser_open,
            browser::browser_cmd,
            browser::browser_stop,
            browser::browser_close,
            browser::browser_window,
            browser::browser_profiles,
            browser::browser_takeover,
            #[cfg(desktop)]
            setup::setup_detect,
            #[cfg(desktop)]
            setup::updates::setup_check_updates,
            #[cfg(desktop)]
            setup::install::setup_plan,
            #[cfg(desktop)]
            setup::install::setup_run,
            #[cfg(desktop)]
            setup::install::setup_cancel,
            #[cfg(desktop)]
            setup::install::install_component_here,
            host_supervisor::host_runtime,
            host_supervisor::host_log,
            host_supervisor::host_restart,
            lifecycle::close_action_state,
            lifecycle::close_action_set,
            lifecycle::close_answer,
            lifecycle::quit_now,
            lifecycle::autostart_state,
            lifecycle::autostart_set,
            push::push_list,
            push::push_fetch,
            push::fs_download,
            push::open_url,
            push::push_done,
            push::push_save_as,
            push::push_open,
            desktop::desktop_status,
            desktop::desktop_notify,
            desktop::desktop_set_title,
            desktop::mascot_window,
            desktop::mascot_focus_main,
            sharing::feature_request,
            sharing::feature_requests,
            sharing::local_host,
            sharing::sharing_status,
            sharing::local_requests,
            sharing::sharing_change,
            pair_phone::pair_phone_start,
            local_host::local_admin,
            pair_phone::pair_phone_cancel,
            pair_phone::local_host_start,
            local_files::local_files,
            local_files::local_file_bytes,
            local_files::clipboard_files,
            #[cfg(desktop)]
            app_update::app_update_check,
            #[cfg(desktop)]
            app_update::app_update_install
        ]);
    #[cfg(mobile)]
    let builder = builder
        .plugin(tauri_plugin_barcode_scanner::init())
        .plugin(tauri_plugin_haptics::init());
    // iOS: the page must cover the whole screen; CSS env() adds the safe
    // areas once (see ios_webview.rs — WKWebView otherwise insets twice).
    #[cfg(target_os = "ios")]
    let builder = builder.on_page_load(|webview, _payload| {
        if let Err(error) = webview.with_webview(|platform| {
            ios_webview::disable_safe_area_content_inset(platform.inner());
        }) {
            log::warn!("ios webview inset fix failed: {error}");
        }
    });
    // UNI-117: desktop-only plugins + the tray (DESIGN §5.7: windowState,
    // nativeNotifications, tray caps). Window size/position are saved on
    // close and restored when the main window is created.
    #[cfg(desktop)]
    let builder = builder
        // UNI-240: the setup engine's cancel flag (desktop only).
        .manage(install::CancelFlag::default())
        // UNI-237 (A6): "Start at login" (starts hidden in the tray).
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .plugin(tauri_plugin_notification::init())
        // UNI-263: self-update (prompted in the UI, never silent; app_update.rs).
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Position / maximized / fullscreen only: the size is saved by
        // desktop::save_window_size (the plugin's grows the window on every
        // launch under GTK client-side decorations), and VISIBLE is left out
        // so quitting from the tray while hidden can't start the app hidden.
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::POSITION
                        | tauri_plugin_window_state::StateFlags::MAXIMIZED
                        | tauri_plugin_window_state::StateFlags::FULLSCREEN
                        | tauri_plugin_window_state::StateFlags::DECORATIONS,
                )
                .build(),
        )
        .setup(|app| {
            desktop::restore_window_size(app.handle());
            if let Err(error) = desktop::setup_tray(app.handle()) {
                log::warn!("tray unavailable: {error}");
            }
            // v2 §5.1: this PC's host — adopt a running one or run our own child.
            #[cfg(desktop)]
            {
                use tauri::Manager;
                if let Some(dir) = local_host::host_dir() {
                    let shared = app.state::<host_supervisor::Shared>().inner().clone();
                    let bin = host_supervisor::host_bin(Some(app.handle()));
                    tauri::async_runtime::spawn(async move {
                        let (plan, supervisor) = host_supervisor::decide(&dir, bin.as_deref());
                        if let Some(s) = supervisor {
                            *shared.0.lock().await = Some(s);
                        }
                        log::info!("host supervisor: {plan:?} on {}", dir.display());
                    });
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // UNI-256: files dropped on the window may be read back by the UI
            // (local_file_bytes) — only these, never a path the page names.
            if let tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) = event {
                local_files::grant(paths.iter().cloned());
            }
            // v2 §5.4: the remembered `closeAction` decides — `ask` (the
            // default) shows the tray/quit dialog once, `tray` hides, `quit`
            // quits (with the phone warning when it applies, §5.5).
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    use tauri::Manager;
                    desktop::save_window_size(window.app_handle());
                    match lifecycle::close_decision(lifecycle::close_action()) {
                        lifecycle::CloseDecision::Hide => {
                            api.prevent_close();
                            let _ = window.hide();
                        }
                        lifecycle::CloseDecision::Ask => {
                            api.prevent_close();
                            use tauri::Emitter;
                            let _ = window.app_handle().emit_to(
                                "main",
                                lifecycle::CLOSE_ASK_EVENT,
                                serde_json::json!({ "message": lifecycle::CLOSE_ASK_MESSAGE }),
                            );
                        }
                        lifecycle::CloseDecision::Quit => {
                            api.prevent_close();
                            lifecycle::request_quit(window.app_handle());
                        }
                    }
                }
            }
        });
    // §5.6: a login start passes `--hidden` (tauri-plugin-autostart); the window
    // is created hidden so it never flashes, and the tray is the way in.
    let context = tauri::generate_context!();
    #[cfg(desktop)]
    let context = {
        let mut context = context;
        if lifecycle::start_hidden(std::env::args().skip(1)) {
            if let Some(window) = context.config_mut().app.windows.first_mut() {
                window.visible = false;
            }
            log::info!("started with --hidden: in the tray, no window");
        }
        context
    };
    #[cfg(desktop)]
    let result = builder.build(context).map(|app| {
        app.run(|handle, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                desktop::save_window_size(handle);
            }
        })
    });
    #[cfg(not(desktop))]
    let result = builder.run(context);

    if let Err(error) = result {
        eprintln!("fatal: tauri application failed to run: {error}");
        std::process::exit(1);
    }
}
