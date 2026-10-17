//! Application settings. Every struct is `#[serde(default)]`, so settings files written by
//! older versions keep loading when fields are added.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum StartupBehavior {
    /// Reopen the tabs that were open when Aether last closed.
    Restore,
    /// Show the welcome screen.
    Welcome,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct GeneralSettings {
    pub startup: StartupBehavior,
    /// Keep running in the system tray when the last window closes.
    pub close_to_tray: bool,
    pub launch_at_login: bool,
    /// Ask before closing a tab or window that has a live session.
    pub confirm_close_active: bool,
    pub onboarding_completed: bool,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            startup: StartupBehavior::Welcome,
            close_to_tray: false,
            launch_at_login: false,
            confirm_close_active: true,
            onboarding_completed: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum MotionPreference {
    /// Follow the Windows "Show animations" setting.
    System,
    Full,
    Reduced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum Density {
    Comfortable,
    Compact,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct AppearanceSettings {
    /// `"system"`, a built-in theme id, or a custom theme id.
    pub theme: String,
    /// Theme used when `theme` is `"system"` and Windows is in light mode.
    pub light_theme: String,
    /// Theme used when `theme` is `"system"` and Windows is in dark mode.
    pub dark_theme: String,
    /// Overrides the theme's accent colour (`#rrggbb`).
    pub accent: Option<String>,
    pub ui_scale: f32,
    pub sidebar_width: u32,
    pub sidebar_collapsed: bool,
    pub motion: MotionPreference,
    pub density: Density,
    pub show_status_bar: bool,
    /// Load `custom.css` from the Aether data folder.
    pub custom_css: bool,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme: "aether-dark".into(),
            light_theme: "aether-light".into(),
            dark_theme: "aether-dark".into(),
            accent: None,
            ui_scale: 1.0,
            sidebar_width: 264,
            sidebar_collapsed: false,
            motion: MotionPreference::System,
            density: Density::Comfortable,
            show_status_bar: true,
            custom_css: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum CursorStyle {
    Block,
    Bar,
    Underline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum RightClickAction {
    /// Show a context menu (copy, paste, search, ...).
    Menu,
    /// Paste immediately; copy happens on selection. Classic PuTTY behaviour.
    Paste,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum BellStyle {
    None,
    Visual,
    Sound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum RendererPreference {
    /// WebGL when available, DOM otherwise.
    Auto,
    Webgl,
    Dom,
}

/// What a remote program may do through the OSC 52 clipboard escape sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum ClipboardAccess {
    Deny,
    /// Programs (vim, tmux) can copy to your clipboard but never read it.
    WriteOnly,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct TerminalSettings {
    /// CSS font-family list; first available wins.
    pub font_family: String,
    pub font_size: f32,
    pub line_height: f32,
    pub letter_spacing: f32,
    pub font_weight: u16,
    pub font_weight_bold: u16,
    pub ligatures: bool,
    pub cursor_style: CursorStyle,
    pub cursor_blink: bool,
    pub scrollback: u32,
    pub copy_on_select: bool,
    pub right_click: RightClickAction,
    pub bell: BellStyle,
    pub word_separators: String,
    pub osc52: ClipboardAccess,
    /// WCAG-style minimum contrast between text and background (1 = off).
    pub minimum_contrast: f32,
    pub smooth_scroll: bool,
    pub renderer: RendererPreference,
    pub scroll_on_output: bool,
    pub scroll_on_keypress: bool,
    /// Terminal colour scheme override; `None` follows the app theme.
    pub theme: Option<String>,
    /// Warn before pasting text that contains multiple lines.
    pub confirm_multiline_paste: bool,
    /// Render emoji and combining sequences as single cells.
    pub unicode_graphemes: bool,
    /// Sixel and iTerm2 inline images.
    pub inline_images: bool,
    pub screen_reader_mode: bool,
    /// Whether Ctrl+K opens the command palette even while the terminal has focus
    /// (otherwise it is sent to the shell for readline's kill-line).
    pub palette_in_terminal: bool,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            font_family: "'JetBrains Mono Variable', 'Cascadia Code', Consolas, monospace".into(),
            font_size: 13.5,
            line_height: 1.25,
            letter_spacing: 0.0,
            font_weight: 400,
            font_weight_bold: 700,
            ligatures: true,
            cursor_style: CursorStyle::Bar,
            cursor_blink: true,
            scrollback: 20_000,
            copy_on_select: true,
            right_click: RightClickAction::Menu,
            bell: BellStyle::Visual,
            word_separators: " ()[]{}'\"`,;|<>".into(),
            osc52: ClipboardAccess::WriteOnly,
            minimum_contrast: 1.0,
            smooth_scroll: true,
            renderer: RendererPreference::Auto,
            scroll_on_output: false,
            scroll_on_keypress: true,
            theme: None,
            confirm_multiline_paste: true,
            unicode_graphemes: true,
            inline_images: true,
            screen_reader_mode: false,
            palette_in_terminal: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum HostKeyPolicy {
    /// Ask before trusting a host Aether hasn't seen. Changed keys always block.
    Ask,
    /// Trust new hosts silently (like `StrictHostKeyChecking accept-new`). Changed keys block.
    AcceptNew,
    /// Refuse any host that isn't already in known_hosts.
    Strict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum AgentMode {
    /// Windows OpenSSH agent first, then Pageant.
    Auto,
    /// Only the Windows OpenSSH agent (`\\.\pipe\openssh-ssh-agent`).
    OpenSsh,
    Pageant,
    Off,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct SshSettings {
    /// Default username when a host doesn't set one. Empty = Windows user name.
    pub default_username: String,
    pub keepalive_interval_secs: u32,
    pub keepalive_max: u32,
    pub connect_timeout_secs: u32,
    pub auto_reconnect: bool,
    /// `0` = keep trying until the user stops it.
    pub reconnect_max_attempts: u32,
    pub host_key_policy: HostKeyPolicy,
    pub agent: AgentMode,
    /// Key files tried when a host has no explicit key. `~` is expanded.
    pub identity_files: Vec<String>,
    /// Also trust host keys found in `~/.ssh/known_hosts` (read-only).
    pub read_user_known_hosts: bool,
    /// Apply matching `~/.ssh/config` entries to quick-connect targets.
    pub use_ssh_config: bool,
    pub default_term: String,
    pub tcp_nodelay: bool,
    /// The X server on this PC that forwarded X11 programs should draw on (`localhost:0`
    /// for VcXsrv, X410, Xming, …).
    pub x11_display: String,
}

impl Default for SshSettings {
    fn default() -> Self {
        Self {
            default_username: String::new(),
            keepalive_interval_secs: 30,
            keepalive_max: 3,
            connect_timeout_secs: 15,
            auto_reconnect: true,
            reconnect_max_attempts: 12,
            host_key_policy: HostKeyPolicy::Ask,
            agent: AgentMode::Auto,
            identity_files: vec!["~/.ssh/id_ed25519".into(), "~/.ssh/id_ecdsa".into(), "~/.ssh/id_rsa".into()],
            read_user_known_hosts: true,
            use_ssh_config: true,
            default_term: "xterm-256color".into(),
            tcp_nodelay: true,
            x11_display: "localhost:0".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum OverwritePolicy {
    Ask,
    Overwrite,
    Skip,
    /// Keep both: the new file becomes `name (1).ext`.
    Rename,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct SftpSettings {
    pub show_hidden: bool,
    pub concurrent_transfers: u8,
    pub overwrite: OverwritePolicy,
    pub preserve_times: bool,
    /// Where downloads land when you don't pick a folder. `None` = Windows Downloads.
    pub download_dir: Option<String>,
}

impl Default for SftpSettings {
    fn default() -> Self {
        Self {
            show_hidden: false,
            concurrent_transfers: 3,
            overwrite: OverwritePolicy::Ask,
            preserve_times: true,
            download_dir: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct NotificationSettings {
    pub on_disconnect: bool,
    pub on_command_finished: bool,
    /// Only notify for commands that ran at least this long.
    pub min_command_secs: u32,
    /// Skip notifications while the Aether window is focused.
    pub only_when_unfocused: bool,
    /// Let programs on the server ask for a desktop notification (OSC 9 / OSC 777), shown with the session's name and
    /// limited to one every few seconds.
    pub from_server: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            on_disconnect: true,
            on_command_finished: true,
            min_command_secs: 10,
            only_when_unfocused: true,
            from_server: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct Settings {
    pub version: u32,
    pub general: GeneralSettings,
    pub appearance: AppearanceSettings,
    pub terminal: TerminalSettings,
    pub ssh: SshSettings,
    pub sftp: SftpSettings,
    pub notifications: NotificationSettings,
    /// Command id → accelerator (e.g. `"Ctrl+Shift+T"`). An empty string unbinds.
    pub keybindings: BTreeMap<String, String>,
}

pub const SETTINGS_VERSION: u32 = 1;

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            general: GeneralSettings::default(),
            appearance: AppearanceSettings::default(),
            terminal: TerminalSettings::default(),
            ssh: SshSettings::default(),
            sftp: SftpSettings::default(),
            notifications: NotificationSettings::default(),
            keybindings: BTreeMap::new(),
        }
    }
}

impl Settings {
    /// Clamps values into ranges the UI and renderer can cope with. Called after every
    /// load/update so a hand-edited file can't wedge the app.
    pub fn sanitize(&mut self) {
        let t = &mut self.terminal;
        t.font_size = t.font_size.clamp(6.0, 48.0);
        t.line_height = t.line_height.clamp(1.0, 2.5);
        t.letter_spacing = t.letter_spacing.clamp(-2.0, 8.0);
        t.font_weight = t.font_weight.clamp(100, 900);
        t.font_weight_bold = t.font_weight_bold.clamp(100, 900);
        t.scrollback = t.scrollback.clamp(100, 1_000_000);
        t.minimum_contrast = t.minimum_contrast.clamp(1.0, 21.0);
        let a = &mut self.appearance;
        a.ui_scale = a.ui_scale.clamp(0.75, 1.5);
        a.sidebar_width = a.sidebar_width.clamp(200, 520);
        let s = &mut self.ssh;
        s.keepalive_max = s.keepalive_max.clamp(1, 20);
        s.connect_timeout_secs = s.connect_timeout_secs.clamp(1, 300);
        if s.x11_display.trim().is_empty() {
            s.x11_display = SshSettings::default().x11_display;
        }
        self.sftp.concurrent_transfers = self.sftp.concurrent_transfers.clamp(1, 8);
        self.version = SETTINGS_VERSION;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_survive_a_json_roundtrip() {
        let s = Settings::default();
        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn partial_files_fill_in_defaults() {
        let s: Settings = serde_json::from_str(r#"{ "terminal": { "fontSize": 16 } }"#).unwrap();
        assert_eq!(s.terminal.font_size, 16.0);
        assert!(s.terminal.copy_on_select);
        assert_eq!(s.ssh.keepalive_interval_secs, 30);
    }

    #[test]
    fn sanitize_clamps_nonsense() {
        let mut s = Settings::default();
        s.terminal.font_size = 400.0;
        s.terminal.scrollback = 1;
        s.appearance.ui_scale = 0.0;
        s.sanitize();
        assert_eq!(s.terminal.font_size, 48.0);
        assert_eq!(s.terminal.scrollback, 100);
        assert_eq!(s.appearance.ui_scale, 0.75);
    }
}
