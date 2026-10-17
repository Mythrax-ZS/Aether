//! System tray: quick connect to favourites and recent hosts, show/quit.

use aether_core::HostId;
use aether_core::model::Host;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};
use tracing::warn;

use crate::dto::TrayConnectEvent;
use crate::state::AppState;
use crate::window;

const TRAY_ID: &str = "main";
const MAX_HOSTS: usize = 12;

fn quick_hosts(state: &AppState) -> Vec<Host> {
    let mut hosts = state.hosts.snapshot().hosts;
    // Favourites first, then most recently used.
    hosts.sort_by(|a, b| {
        b.favorite
            .cmp(&a.favorite)
            .then(b.last_connected_at.cmp(&a.last_connected_at))
            .then(a.label.to_lowercase().cmp(&b.label.to_lowercase()))
    });
    hosts.into_iter().take(MAX_HOSTS).collect()
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;
    menu.append(&MenuItem::with_id(app, "show", "Open Aether", true, None::<&str>)?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;

    if let Some(state) = app.try_state::<std::sync::Arc<AppState>>() {
        let hosts = quick_hosts(&state);
        if hosts.is_empty() {
            menu.append(&MenuItem::with_id(app, "none", "No saved hosts yet", false, None::<&str>)?)?;
        } else {
            let sub = Submenu::with_id(app, "quick", "Quick connect", true)?;
            for h in hosts {
                let star = if h.favorite { "★ " } else { "" };
                let text = format!("{star}{}  —  {}", h.display_name(), h.target());
                sub.append(&MenuItem::with_id(app, format!("host:{}", h.id), text, true, None::<&str>)?)?;
            }
            menu.append(&sub)?;
        }
    }

    menu.append(&MenuItem::with_id(app, "new-window", "New window", true, None::<&str>)?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(app, "quit", "Quit Aether", true, None::<&str>)?)?;
    Ok(menu)
}

pub fn init(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_menu(app)?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Aether")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();
            match id {
                "show" => window::show_main(app),
                "new-window" => window::open_blank(app),
                "quit" => app.exit(0),
                other => {
                    if let Some(host_id) = other.strip_prefix("host:").and_then(|s| s.parse::<HostId>().ok()) {
                        window::emit_to_main(app, "tray:connect", TrayConnectEvent { host_id });
                    }
                }
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                window::show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    // On Linux and the BSDs the tray library loads libayatana-appindicator while it builds the icon, and panics when
    // that isn't installed. The app is fine without a tray, so that must not take the whole start-up down with it.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| builder.build(app)))
        .map_err(|_| anyhow::anyhow!("the tray library could not be loaded (is libayatana-appindicator installed?)"))
        .map_err(tauri::Error::Anyhow)??;
    Ok(())
}

/// Rebuilds the quick-connect list after hosts change.
pub fn refresh(app: &AppHandle) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        match build_menu(app) {
            Ok(menu) => {
                let _ = tray.set_menu(Some(menu));
            }
            Err(e) => warn!(error = %e, "could not refresh the tray menu"),
        }
    }
}
