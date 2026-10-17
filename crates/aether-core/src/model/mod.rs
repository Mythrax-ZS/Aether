//! The serialisable domain model. Everything here is exported to TypeScript by `ts-rs`
//! (see `.cargo/config.toml`), so the UI and backend share one source of truth.

pub mod forward;
pub mod host;
pub mod key;
pub mod settings;
pub mod theme;

pub use forward::{ForwardKind, ForwardRule};
pub use host::{
    AuthConfig, AuthMethodKind, ConnectionOptions, EnvVar, Group, Host, HostOrigin, JumpRef, KeySource, StartupConfig,
    TerminalOverride,
};
pub use key::{KeyInfo, KeyOrigin, KeyRegistry};
pub use settings::*;
pub use theme::{TerminalPalette, Theme, ThemeKind};
