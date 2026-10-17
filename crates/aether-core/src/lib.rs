//! # aether-core
//!
//! The UI-agnostic heart of Aether: the domain model, crash-safe persistence, OpenSSH
//! `ssh_config` / `known_hosts` handling and the error taxonomy. It performs no networking
//! and links no UI toolkit, so everything here is fast to compile and trivial to test.
//!
//! Types that cross the IPC boundary derive [`ts_rs::TS`]; running `cargo test` regenerates
//! the TypeScript bindings in `ui/src/lib/bindings/`.

pub mod error;
pub mod ids;
pub mod known_hosts;
pub mod model;
pub mod paths;
pub mod ssh_config;
pub mod store;
pub mod util;

pub use error::{CoreError, ErrorCode, ErrorInfo, Result, UserFacing};
pub use ids::{ConnectionId, GroupId, HostId, KeyId, PromptId, SnippetId, TerminalId, TransferId};
pub use paths::{AppPaths, StorageMode};
