//! Window creation and native styling.

use std::sync::atomic::{AtomicU32, Ordering};

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tracing::{debug, warn};

use crate::commands::app::NewWindowRequest;

static COUNTER: AtomicU32 = AtomicU32::new(1);

/// Shows and focuses a window.
pub fn reveal(window: &WebviewWindow) {
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
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
