//! The entry that starts Aether when the person signs in.
//!
//! Windows, macOS and Linux go through the Tauri autostart plugin. The other Unix systems (the BSDs) are not covered by
//! it, so there the entry is a freedesktop autostart file, which the usual desktops on those systems honour.

use tauri::{AppHandle, Builder, Wry};

/// What the sign-in entry starts Aether with: it then stays in the tray until asked for.
pub const BACKGROUND_FLAG: &str = "--background";

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
mod native {
    use tauri_plugin_autostart::ManagerExt;

    use super::*;

    pub fn install(builder: Builder<Wry>) -> Builder<Wry> {
        builder.plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![BACKGROUND_FLAG]),
        ))
    }

    pub struct LoginEntry(AppHandle);

    impl LoginEntry {
        pub fn open(app: &AppHandle) -> anyhow::Result<Self> {
            Ok(Self(app.clone()))
        }

        pub fn is_enabled(&self) -> anyhow::Result<bool> {
            Ok(self.0.autolaunch().is_enabled()?)
        }

        pub fn enable(&self) -> anyhow::Result<()> {
            Ok(self.0.autolaunch().enable()?)
        }

        pub fn disable(&self) -> anyhow::Result<()> {
            Ok(self.0.autolaunch().disable()?)
        }
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
mod native {
    use aether_core::autostart::Autostart;
    use anyhow::Context;

    use super::*;

    pub fn install(builder: Builder<Wry>) -> Builder<Wry> {
        builder
    }

    pub struct LoginEntry(Autostart);

    impl LoginEntry {
        pub fn open(_app: &AppHandle) -> anyhow::Result<Self> {
            let config_home = dirs::config_dir().context("no configuration folder for this user")?;
            let exe = std::env::current_exe().context("cannot tell where Aether is installed")?;
            Ok(Self(Autostart::new(&config_home, &exe, &[BACKGROUND_FLAG])))
        }

        pub fn is_enabled(&self) -> anyhow::Result<bool> {
            Ok(self.0.is_enabled()?)
        }

        pub fn enable(&self) -> anyhow::Result<()> {
            Ok(self.0.enable()?)
        }

        pub fn disable(&self) -> anyhow::Result<()> {
            Ok(self.0.disable()?)
        }
    }
}

pub use native::LoginEntry;

/// Adds whatever the platform needs to `builder` before the app is built.
pub fn install(builder: Builder<Wry>) -> Builder<Wry> {
    native::install(builder)
}
