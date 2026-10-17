//! # Aether application shell
//!
//! A deliberately thin layer: it owns the window, the tray and the IPC surface, and wires
//! the UI-agnostic crates ([`aether_core`], [`aether_vault`], [`aether_ssh`]) to the webview.
//! No SSH protocol logic lives here.

mod commands;
mod connect;
mod delegate;
mod dto;
mod events;
mod logging;
mod login_entry;
mod prompts;
mod single_instance;
mod state;
mod tray;
mod window;

use std::sync::Arc;

use aether_core::AppPaths;
use aether_core::paths::StorageMode;
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent};
use tauri_plugin_window_state::StateFlags;
use tracing::{info, warn};

use crate::dto::ArgsEvent;
use crate::state::AppState;

/// Closes every connection a window owns and forgets what was pending for them.
fn close_window_sessions(app: &AppHandle, label: &str) {
    let Some(state) = app.try_state::<Arc<AppState>>() else { return };
    let state = state.inner().clone();
    let label = label.to_string();
    tauri::async_runtime::spawn(async move {
        for id in state.prompts.connections_of(&label) {
            state.prompts.clear_connection(id);
            state.plans.retain(|_, p| p.connection != id);
            state.sessions.close(id).await;
        }
    });
}

/// Where the web view keeps its own profile (cache, local storage, shader caches). An installed copy leaves that to
/// Windows' usual place for an app's data. A portable copy — or one pointed at a folder with `AETHER_HOME` — keeps it
/// with everything else, so that "nothing is left behind on this PC" is true of the web view too. WebView2 reads
/// this variable when it creates its environment, so it has to be set before the first window; one the person set
/// themselves is respected.
fn keep_web_view_data_with_the_app(paths: &AppPaths) {
    if paths.mode == StorageMode::Installed || std::env::var_os("WEBVIEW2_USER_DATA_FOLDER").is_some() {
        return;
    }
    let folder = paths.cache_dir.join("webview");
    match std::fs::create_dir_all(&folder) {
        // SAFETY: this runs first thing in `run`, before the log writer, the async runtime and the web view exist, so
        // no other thread can be reading the environment.
        Ok(()) => unsafe { std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", &folder) },
        Err(e) => eprintln!("could not create the web view's data folder: {e}"),
    }
}

pub fn run() {
    let paths = AppPaths::resolve();
    if let Err(e) = paths.ensure_dirs() {
        eprintln!("could not create data folders: {e}");
    }
    keep_web_view_data_with_the_app(&paths);
    let log_guard = logging::init(&paths);
    info!(version = env!("CARGO_PKG_VERSION"), mode = ?paths.mode, "Aether starting");

    // The window's size and position live with the rest of the settings, not in a folder of the framework's choosing.
    let window_state_file = paths.config_dir.join("window-state.json").to_string_lossy().into_owned();
    let instance_folder = paths.cache_dir.clone();
    let mut pending = Some((paths, log_guard));

    let builder = single_instance::install(tauri::Builder::default(), instance_folder, |app, args| {
        // A second launch focuses the running instance and hands over its arguments.
        let args: Vec<String> = args.into_iter().skip(1).collect();
        if args.is_empty() {
            window::show_main(app);
        } else {
            window::emit_to_main(app, "app:args", ArgsEvent { args });
        }
    });
    let app = login_entry::install(builder)
        .plugin(
            tauri_plugin_window_state::Builder::default()
                // An absolute path: the plugin joins it onto its folder, and joining an absolute path gives that path.
                .with_filename(window_state_file)
                // Position and size only: visibility is decided by the UI's first paint.
                .with_state_flags(StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED)
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_process::init())
        .setup(move |app| {
            let (paths, guard) = pending.take().expect("setup runs once");
            let handle = app.handle().clone();
            let state = Arc::new(AppState::new(handle.clone(), paths, guard));
            state.apply_settings(&state.settings.get());
            app.manage(state.clone());

            if let Some(main) = app.get_webview_window("main") {
                window::style_native(&main);
            }
            match tray::init(&handle) {
                // Started by the sign-in entry: sit in the tray until asked for. Without a tray there would be no way
                // back to a hidden window, so then the window shows as usual.
                Ok(()) if std::env::args().any(|a| a == login_entry::BACKGROUND_FLAG) => {
                    state.set_background_start(true)
                }
                Ok(()) => {}
                Err(e) => warn!(error = %e, "system tray unavailable"),
            }

            // A page that never starts must not leave the program looking hung, with no window and nothing to look at.
            let (watched, watcher) = (state.clone(), handle.clone());
            tauri::async_runtime::spawn(window::show_if_page_is_silent(
                window::INTERFACE_PATIENCE,
                {
                    let state = watched.clone();
                    move || state.is_interface_ready()
                },
                move || watched.is_background_start(),
                move || {
                    if let Some(main) = watcher.get_webview_window("main") {
                        window::reveal(&main);
                    }
                },
            ));

            // Keep every window (and the tray) in sync when hosts change.
            let mut changes = state.hosts.subscribe();
            tauri::async_runtime::spawn(async move {
                use tokio::sync::broadcast::error::RecvError;
                while let Ok(()) | Err(RecvError::Lagged(_)) = changes.recv().await {
                    tray::refresh(&handle);
                    let _ = handle.emit("hosts:changed", ());
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } if window.label() == "main" => {
                let to_tray = window
                    .app_handle()
                    .try_state::<Arc<AppState>>()
                    .is_some_and(|s| s.settings.get().general.close_to_tray);
                if to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
            // A closed window takes its sessions with it, unless a tear-off adopted them.
            WindowEvent::Destroyed => close_window_sessions(window.app_handle(), window.label()),
            _ => {}
        })
        // The page starting to load again (a reload, or recovery after the webview crashed) means every
        // tab it showed is gone — and with it the only way to see or close the connections behind them.
        // A window's first load owns nothing yet, and a tear-off adopts its session only after loading.
        .on_page_load(|webview, payload| {
            // Whether a page ever finished loading is the first thing to know when a window stays empty.
            info!(window = webview.label(), event = ?payload.event(), url = %payload.url(), "page load");
            if payload.event() == PageLoadEvent::Started {
                close_window_sessions(webview.app_handle(), webview.label());
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::app::app_info,
            commands::app::app_ready,
            commands::app::app_quit,
            commands::app::app_restart,
            commands::app::window_new,
            commands::app::list_fonts,
            commands::app::open_path,
            commands::app::open_logs_dir,
            commands::app::open_config_dir,
            commands::app::custom_css_get,
            commands::hosts::hosts_snapshot,
            commands::hosts::host_save,
            commands::hosts::hosts_delete,
            commands::hosts::host_duplicate,
            commands::hosts::host_favorite,
            commands::hosts::hosts_move,
            commands::hosts::hosts_reorder,
            commands::hosts::group_save,
            commands::hosts::group_delete,
            commands::hosts::group_collapse,
            commands::hosts::recents_list,
            commands::hosts::recents_clear,
            commands::hosts::host_password_exists,
            commands::hosts::host_password_set,
            commands::hosts::host_password_clear,
            commands::hosts::ssh_config_preview,
            commands::hosts::ssh_config_import,
            commands::keys::keys_list,
            commands::keys::key_inspect,
            commands::keys::key_import,
            commands::keys::key_generate,
            commands::keys::key_rename,
            commands::keys::key_delete,
            commands::keys::key_public,
            commands::keys::key_export_public,
            commands::keys::key_install,
            commands::keys::known_hosts_list,
            commands::keys::known_host_remove,
            commands::settings::settings_get,
            commands::settings::settings_patch,
            commands::settings::settings_reset,
            commands::settings::themes_custom,
            commands::settings::theme_save,
            commands::settings::theme_delete,
            commands::settings::workspace_load,
            commands::settings::workspace_save,
            commands::sessions::session_open,
            commands::sessions::session_quick_connect,
            commands::sessions::session_adopt,
            commands::sessions::session_close,
            commands::sessions::session_reconnect,
            commands::sessions::sessions_list,
            commands::sessions::session_state,
            commands::sessions::terminal_new,
            commands::sessions::terminal_attach,
            commands::sessions::terminal_detach,
            commands::sessions::terminal_write,
            commands::sessions::terminal_write_binary,
            commands::sessions::terminal_resize,
            commands::sessions::terminal_ack,
            commands::sessions::terminal_close,
            commands::sessions::terminal_restart,
            commands::sessions::forward_start,
            commands::sessions::forward_stop,
            commands::sessions::forwards_list,
            commands::sessions::prompt_respond,
            commands::sftp::sftp_list,
            commands::sftp::sftp_home,
            commands::sftp::sftp_mkdir,
            commands::sftp::sftp_rename,
            commands::sftp::sftp_delete,
            commands::sftp::sftp_chmod,
            commands::sftp::sftp_disk_usage,
            commands::sftp::sftp_read_text,
            commands::sftp::sftp_write_text,
            commands::sftp::transfer_plan_upload,
            commands::sftp::transfer_plan_download,
            commands::sftp::transfer_start,
            commands::sftp::transfer_discard_plan,
            commands::sftp::transfers_list,
            commands::sftp::transfer_pause,
            commands::sftp::transfer_resume,
            commands::sftp::transfer_cancel,
            commands::sftp::transfer_dismiss,
            commands::sftp::transfers_clear_finished,
            commands::local::local_roots,
            commands::local::local_list,
            commands::local::local_mkdir,
            commands::local::local_rename,
            commands::local::local_delete,
            commands::local::local_exists,
            commands::local::local_default_download_dir,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build the Aether application");

    app.run(|handle, event| match event {
        RunEvent::ExitRequested { .. } => {
            // Say goodbye to servers politely instead of just dropping the sockets.
            if let Some(state) = handle.try_state::<Arc<AppState>>() {
                let sessions = state.sessions.clone();
                tauri::async_runtime::block_on(async move {
                    let _ = tokio::time::timeout(std::time::Duration::from_secs(3), sessions.close_all()).await;
                });
            }
        }
        RunEvent::Exit => {
            // The framework makes empty folders named after the app in the usual per-user places even when everything
            // was told to live elsewhere (the web view's profile, the window-state file's folder). They are taken away
            // again; `remove_dir` only ever removes an empty folder, so nothing can be lost by this — an installed copy's
            // web view profile, which isn't empty, stays.
            for dir in [handle.path().app_config_dir(), handle.path().app_local_data_dir()].into_iter().flatten() {
                let _ = std::fs::remove_dir(dir);
            }
        }
        _ => {}
    });
}
