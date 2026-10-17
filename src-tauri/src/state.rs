//! Process-wide application state, shared with every command.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use aether_core::error::{ErrorCode, UserFacing};
use aether_core::known_hosts::KnownHosts;
use aether_core::model::Settings;
use aether_core::paths::StorageMode;
use aether_core::store::{HostStore, KeyStore, RecentsStore, SettingsStore, ThemeStore};
use aether_core::{AppPaths, ConnectionId, ErrorInfo};
use aether_ssh::{Plan, Session, SessionManager, TransferDirection, TransferManager};
use aether_vault::SecretStore;
use dashmap::DashMap;
use parking_lot::Mutex;
use tauri::AppHandle;
use tracing_appender::non_blocking::WorkerGuard;

use crate::connect::Resolver;
use crate::events::AppEvents;
use crate::login_entry::LoginEntry;
use crate::prompts::PromptBroker;

pub type Res<T> = Result<T, ErrorInfo>;

/// A transfer that has been planned and is waiting for the user to resolve conflicts.
pub struct PendingPlan {
    pub connection: ConnectionId,
    pub direction: TransferDirection,
    pub plan: Plan,
    pub reveal: Option<String>,
    pub created: Instant,
}

pub struct AppState {
    app: AppHandle,
    /// The last value of "launch at sign-in" that was applied to the system, so settings edits that have nothing to do
    /// with it (a font size slider) don't touch the registry or the autostart folder.
    applied_launch_at_login: Mutex<Option<bool>>,
    /// Started by the sign-in entry (`--background`) with a tray icon to come back to: the first window stays hidden.
    background_start: AtomicBool,
    /// The main page has said it is ready (and asked for its window).
    interface_ready: AtomicBool,
    pub paths: AppPaths,
    pub hosts: Arc<HostStore>,
    pub settings: Arc<SettingsStore>,
    pub themes: ThemeStore,
    pub recents: RecentsStore,
    pub keys: KeyStore,
    pub known_hosts: Arc<KnownHosts>,
    pub vault: Arc<dyn SecretStore>,
    pub sessions: Arc<SessionManager>,
    pub transfers: Arc<TransferManager>,
    pub prompts: Arc<PromptBroker>,
    pub plans: DashMap<String, PendingPlan>,
    pub startup_warnings: Vec<ErrorInfo>,
    _log_guard: Option<WorkerGuard>,
}

impl AppState {
    pub fn new(app: AppHandle, paths: AppPaths, log_guard: Option<WorkerGuard>) -> Self {
        let mut warnings = Vec::new();
        let mut note = |e: Option<aether_core::CoreError>| {
            if let Some(e) = e {
                tracing::warn!(error = %e, "recovered a damaged data file");
                warnings.push(e.info());
            }
        };

        let (hosts, e) = HostStore::open(paths.hosts_file());
        note(e);
        let (settings, e) = SettingsStore::open(paths.settings_file());
        note(e);
        let (keys, e) = KeyStore::open_reporting(paths.keys_file());
        note(e);
        let (recents, e) = RecentsStore::open_reporting(paths.recents_file());
        note(e);
        let initial = settings.get();

        let user_known_hosts = AppPaths::user_ssh_dir().map(|d| d.join("known_hosts"));
        let known_hosts = Arc::new(KnownHosts::new(paths.known_hosts_file(), user_known_hosts));
        known_hosts.set_read_user_file(initial.ssh.read_user_known_hosts);

        let vault = aether_vault::open_default(&paths.secrets_dir(), paths.mode != StorageMode::Installed);
        tracing::info!(backend = %vault.backend(), mode = ?paths.mode, "storage ready");

        let events = Arc::new(AppEvents { app: app.clone() });
        let sessions = SessionManager::new(known_hosts.clone(), events.clone());
        let transfers = Arc::new(TransferManager::new(events, 3));

        Self {
            themes: ThemeStore::new(paths.themes_dir()),
            recents,
            keys,
            app: app.clone(),
            applied_launch_at_login: Mutex::new(None),
            background_start: AtomicBool::new(false),
            interface_ready: AtomicBool::new(false),
            prompts: Arc::new(PromptBroker::new(app.clone())),
            hosts: Arc::new(hosts),
            settings: Arc::new(settings),
            known_hosts,
            vault,
            sessions,
            transfers,
            plans: DashMap::new(),
            startup_warnings: warnings,
            paths,
            _log_guard: log_guard,
        }
    }

    /// Applies the side effects of settings that other components cache.
    pub fn apply_settings(&self, s: &Settings) {
        self.known_hosts.set_read_user_file(s.ssh.read_user_known_hosts);
        self.apply_launch_at_login(s.general.launch_at_login);
    }

    /// Makes the system start (or stop starting) Aether at sign-in, as the setting says.
    fn apply_launch_at_login(&self, wanted: bool) {
        let mut applied = self.applied_launch_at_login.lock();
        if *applied == Some(wanted) {
            return;
        }
        // A portable copy lives wherever it was unpacked and can be moved or removed; a sign-in entry pointing at it
        // would break, or keep pointing at a USB stick. Only the installed app registers itself.
        if self.paths.mode != StorageMode::Installed {
            if wanted {
                tracing::warn!("\"launch at sign-in\" is only available in the installed version; not registering it");
            }
            *applied = Some(wanted);
            return;
        }
        let launcher = match LoginEntry::open(&self.app) {
            Ok(launcher) => launcher,
            Err(e) => {
                tracing::warn!(error = %e, "no way to start at sign-in on this system");
                return;
            }
        };
        let done = match launcher.is_enabled() {
            Ok(current) if current == wanted => true,
            Ok(_) => {
                let result = if wanted { launcher.enable() } else { launcher.disable() };
                match result {
                    Ok(()) => {
                        tracing::info!(wanted, "updated the sign-in entry");
                        true
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, wanted, "could not update the sign-in entry");
                        false
                    }
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "could not read the sign-in entry");
                false
            }
        };
        // An attempt that failed stays open: the next change of settings tries again, instead of the setting claiming
        // something that Windows was never told.
        if done {
            *applied = Some(wanted);
        }
    }

    /// Remember that this run was started by the sign-in entry and should stay out of sight until asked for.
    pub fn set_background_start(&self, value: bool) {
        self.background_start.store(value, Ordering::SeqCst);
    }

    /// True once, for the first window of a background start: it is not revealed. Later windows are shown normally.
    pub fn take_background_start(&self) -> bool {
        self.background_start.swap(false, Ordering::SeqCst)
    }

    /// Whether the first window is still being kept out of sight on purpose (see [`Self::set_background_start`]).
    pub fn is_background_start(&self) -> bool {
        self.background_start.load(Ordering::SeqCst)
    }

    /// Remember that the main page has started and asked for its window.
    pub fn set_interface_ready(&self) {
        self.interface_ready.store(true, Ordering::SeqCst);
    }

    pub fn is_interface_ready(&self) -> bool {
        self.interface_ready.load(Ordering::SeqCst)
    }

    pub fn resolver<'a>(&'a self, settings: &'a Settings) -> Resolver<'a> {
        Resolver::new(&self.hosts, &self.keys, settings)
    }

    pub fn session(&self, id: ConnectionId) -> Res<Arc<Session>> {
        self.sessions.get(id).ok_or_else(|| {
            ErrorInfo::new(ErrorCode::NotFound, "Session not found", "That connection was already closed.")
        })
    }
}

pub fn not_found(what: &str) -> ErrorInfo {
    ErrorInfo::new(ErrorCode::NotFound, "Not found", format!("{what} no longer exists."))
}
