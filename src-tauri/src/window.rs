//! Window creation and native styling.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tracing::{debug, warn};

use crate::commands::app::NewWindowRequest;

static COUNTER: AtomicU32 = AtomicU32::new(1);

/// How long the main page gets to ask for its window before the window is shown without being asked.
pub const INTERFACE_PATIENCE: Duration = Duration::from_secs(10);

/// Shows and focuses a window.
pub fn reveal(window: &WebviewWindow) {
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
}

/// Calls `show` if the page has still not asked for its window after `patience`.
///
/// The window starts hidden and the page asks for it as soon as it has painted. If that never happens — the web view
/// could not start, a script fails before the end of start-up, or the interface bundled into this build is older than
/// the code that expects it — the window would stay hidden for good and the program would look hung. A window that is
/// there but blank can at least be seen, closed and reported. `hidden_on_purpose` is for a start at sign-in, which is
/// meant to show nothing.
pub async fn show_if_page_is_silent(
    patience: Duration,
    asked_for_window: impl Fn() -> bool,
    hidden_on_purpose: impl Fn() -> bool,
    show: impl FnOnce(),
) {
    tokio::time::sleep(patience).await;
    if asked_for_window() || hidden_on_purpose() {
        return;
    }
    warn!(
        seconds = patience.as_secs(),
        "the page has not asked for its window; showing it anyway. If it stays blank, the web view may have failed to start \
         or the interface bundled into this build may be out of date (rebuild ui/ before building the app)"
    );
    show();
}

/// Brings the main window forward, recreating it if it was closed.
///
/// Safe to call from the main thread (tray and menu handlers): a missing window is rebuilt elsewhere.
pub fn show_main(app: &AppHandle) {
    match app.get_webview_window("main") {
        Some(w) => reveal(&w),
        None => {
            let app = app.clone();
            tauri::async_runtime::spawn_blocking(move || {
                if let Err(e) = create_with_label(&app, "main", &NewWindowRequest::default()) {
                    warn!(error = %e, "could not recreate the main window");
                }
            });
        }
    }
}

/// Brings the main window forward and sends it an event. The main window — not every window — acts on
/// requests that come from outside (a second launch with arguments, a tray menu entry), so one request
/// opens one connection. If the main window had to be recreated, its page needs a moment before it listens.
pub fn emit_to_main<S>(app: &AppHandle, event: &'static str, payload: S)
where
    S: serde::Serialize + Clone + Send + 'static,
{
    let existed = app.get_webview_window("main").is_some();
    show_main(app);
    if existed {
        let _ = app.emit_to("main", event, payload);
    } else {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
            let _ = app.emit_to("main", event, payload);
        });
    }
}

/// Creates another top-level window running the same UI.
///
/// A webview window can only be finished by the main thread's message loop, so building one *on* that thread
/// (from a synchronous command, or a tray/menu handler) deadlocks on Windows. This always builds elsewhere.
pub async fn create(app: &AppHandle, request: NewWindowRequest) -> tauri::Result<String> {
    let label = format!("win-{}", COUNTER.fetch_add(1, Ordering::SeqCst));
    let (app, name) = (app.clone(), label.clone());
    tauri::async_runtime::spawn_blocking(move || create_with_label(&app, &name, &request)).await??;
    Ok(label)
}

/// An empty new window, without waiting for it (for callers that run on the main thread).
pub fn open_blank(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = create(&app, NewWindowRequest::default()).await {
            warn!(error = %e, "could not open a new window");
        }
    });
}

fn create_with_label(app: &AppHandle, label: &str, request: &NewWindowRequest) -> tauri::Result<()> {
    let mut url = String::from("index.html");
    if let (Some(c), Some(t)) = (&request.adopt_connection, &request.adopt_terminal) {
        url.push_str(&format!("?adopt={c}&terminal={t}"));
        if let Some(h) = &request.adopt_host {
            url.push_str(&format!("&host={h}"));
        }
    }
    let window = WebviewWindowBuilder::new(app, label, WebviewUrl::App(url.into()))
        .title(request.title.as_deref().unwrap_or("Aether"))
        .inner_size(1180.0, 760.0)
        .min_inner_size(780.0, 500.0)
        .decorations(false)
        .shadow(true)
        .visible(false)
        .center()
        .build()?;
    style_native(&window);
    debug!(label, "window created");
    Ok(())
}

/// Native touches that make a frameless window feel like a Windows 11 app.
pub fn style_native(window: &WebviewWindow) {
    #[cfg(windows)]
    {
        use std::ffi::c_void;
        use windows::Win32::Foundation::HWND;
        use windows::Win32::Graphics::Dwm::{
            DWM_WINDOW_CORNER_PREFERENCE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
        };
        if let Ok(hwnd) = window.hwnd() {
            let hwnd = HWND(hwnd.0);
            let pref: DWM_WINDOW_CORNER_PREFERENCE = DWMWCP_ROUND;
            // SAFETY: `pref` outlives the call and the size matches the attribute type.
            let result = unsafe {
                DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_WINDOW_CORNER_PREFERENCE,
                    &pref as *const _ as *const c_void,
                    std::mem::size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
                )
            };
            if let Err(e) = result {
                // Older Windows versions don't know this attribute; that's fine.
                debug!(error = %e, "rounded corners unavailable");
            }
        }
    }
    #[cfg(not(windows))]
    let _ = window;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    use super::show_if_page_is_silent;

    const PATIENCE: Duration = Duration::from_millis(40);

    /// Runs the watch to the end and says how many times it showed the window.
    async fn shown(asked: &Arc<AtomicBool>, hidden_on_purpose: bool) -> usize {
        let count = Arc::new(AtomicUsize::new(0));
        let (asked, shown) = (asked.clone(), count.clone());
        show_if_page_is_silent(
            PATIENCE,
            move || asked.load(Ordering::SeqCst),
            move || hidden_on_purpose,
            move || {
                shown.fetch_add(1, Ordering::SeqCst);
            },
        )
        .await;
        count.load(Ordering::SeqCst)
    }

    #[tokio::test]
    async fn a_page_that_never_asks_gets_its_window_shown() {
        assert_eq!(shown(&Arc::new(AtomicBool::new(false)), false).await, 1);
    }

    #[tokio::test]
    async fn a_page_that_has_asked_is_left_alone() {
        assert_eq!(shown(&Arc::new(AtomicBool::new(true)), false).await, 0);
    }

    #[tokio::test]
    async fn a_page_that_asks_while_the_watch_is_waiting_is_left_alone() {
        let asked = Arc::new(AtomicBool::new(false));
        let late = asked.clone();
        tokio::spawn(async move {
            tokio::time::sleep(PATIENCE / 4).await;
            late.store(true, Ordering::SeqCst);
        });
        assert_eq!(shown(&asked, false).await, 0);
    }

    #[tokio::test]
    async fn a_start_that_is_meant_to_stay_hidden_stays_hidden() {
        assert_eq!(shown(&Arc::new(AtomicBool::new(false)), true).await, 0);
    }
}
