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
    catch_panic_quietly(|| builder.build(app)).map_err(|why| {
        let why = why.replace('\n', "; ");
        tauri::Error::Anyhow(anyhow::anyhow!(
            "the tray library could not be loaded (is libayatana-appindicator installed?): {why}"
        ))
    })??;
    Ok(())
}

/// Runs `f`, and turns a panic into its message without the usual panic report. For a panic that is a known, handled
/// condition (a missing library) rather than a bug: the report on stderr and in the log would read as a crash.
fn catch_panic_quietly<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    let reporting = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    std::panic::set_hook(reporting);
    outcome.map_err(|payload| {
        payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
            .unwrap_or_else(|| "unknown cause".to_string())
    })
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

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread::ThreadId;

    use super::catch_panic_quietly;

    /// The panic hook is process-wide, so the test counts only the reports that come from its own thread.
    static REPORTING_THREAD: Mutex<Option<ThreadId>> = Mutex::new(None);
    static REPORTS: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn a_handled_panic_becomes_its_message_and_the_earlier_hook_comes_back() {
        *REPORTING_THREAD.lock().unwrap() = Some(std::thread::current().id());
        let original = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {
            if *REPORTING_THREAD.lock().unwrap() == Some(std::thread::current().id()) {
                REPORTS.fetch_add(1, Ordering::SeqCst);
            }
        }));

        assert_eq!(catch_panic_quietly(|| 7), Ok(7));
        // A formatted message and a plain one arrive as different payload types.
        assert_eq!(catch_panic_quietly(|| panic!("{} is missing", "libfoo")), Err::<(), _>("libfoo is missing".into()));
        assert_eq!(catch_panic_quietly(|| panic!("plain words")), Err::<(), _>("plain words".into()));
        assert_eq!(REPORTS.load(Ordering::SeqCst), 0, "the earlier hook is not asked about a panic that was handled");

        // The hook that was in place is back, so a panic that is a bug still gets its report.
        let _ = std::panic::catch_unwind(|| panic!("a bug"));
        assert_eq!(REPORTS.load(Ordering::SeqCst), 1);

        std::panic::set_hook(original);
    }

    /// The default hook prints a report to stderr, which is what made a handled panic look like a crash. Whether
    /// anything is printed can only be seen from outside, so this runs itself again and looks at what that run wrote.
    #[test]
    fn a_handled_panic_prints_no_report() {
        const INNER: &str = "AETHER_TEST_HANDLED_PANIC";
        if std::env::var_os(INNER).is_some() {
            let _ = catch_panic_quietly(|| panic!("handled"));
            return;
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "tray::tests::a_handled_panic_prints_no_report", "--nocapture", "--test-threads=1"])
            .env(INNER, "1")
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "the inner run failed:\n{stderr}");
        assert!(!stderr.contains("panicked"), "a handled panic must print no report:\n{stderr}");
    }
}
