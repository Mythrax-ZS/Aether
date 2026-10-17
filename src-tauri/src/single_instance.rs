//! One running copy per profile. A second launch quits at once, and the first copy is told what it was launched with.
//!
//! Windows, macOS and Linux use the Tauri plugin. The other Unix systems (the BSDs) are not covered by it, so there
//! `aether_core::instance` does the same with a lock file and a socket.

use std::path::PathBuf;

use tauri::{AppHandle, Builder, Wry};

/// Adds the guard to `builder`. It has to come before anything that opens a window.
///
/// `on_launch` runs in the first copy for every later launch, with that launch's command line (the program name comes
/// first, as in [`std::env::args`]). `folder` is where the guard may keep its files; the plugin doesn't need one.
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
pub fn install(
    builder: Builder<Wry>,
    _folder: PathBuf,
    on_launch: impl Fn(&AppHandle, Vec<String>) + Send + Sync + 'static,
) -> Builder<Wry> {
    builder.plugin(tauri_plugin_single_instance::init(move |app, args, _cwd| on_launch(app, args)))
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
pub fn install(
    builder: Builder<Wry>,
    folder: PathBuf,
    on_launch: impl Fn(&AppHandle, Vec<String>) + Send + Sync + 'static,
) -> Builder<Wry> {
    use aether_core::instance::{Claim, claim};
    use tauri::Manager;
    use tracing::warn;

    builder.plugin(
        tauri::plugin::Builder::<Wry>::new("aether-single-instance")
            .setup(move |app, _api| {
                let args: Vec<String> = std::env::args().collect();
                match claim(&folder, &args) {
                    Ok(Claim::First(instance)) => {
                        let handle = app.clone();
                        if let Err(e) = instance.serve(move |args| on_launch(&handle, args)) {
                            warn!(error = %e, "cannot listen for later launches");
                        }
                        // The lock lives as long as the claim does, which is as long as the app.
                        app.manage(instance);
                    }
                    Ok(Claim::AlreadyRunning) => {
                        app.cleanup_before_exit();
                        std::process::exit(0);
                    }
                    Err(e) => warn!(error = %e, "single-instance guard unavailable; carrying on without it"),
                }
                Ok(())
            })
            .build(),
    )
}
