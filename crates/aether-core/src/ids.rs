//! Strongly typed identifiers. All are UUIDv4 under the hood and serialise as plain
//! strings, so they look like ordinary `string`s on the TypeScript side.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
        #[serde(transparent)]
        #[ts(export, type = "string")]
        pub struct $name(pub Uuid);

        impl $name {
            #[allow(clippy::new_without_default)]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.as_hyphenated().fmt(f)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0.as_hyphenated())
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(s).map(Self)
            }
        }
    };
}

define_id!(
    /// Identifies a saved host.
    HostId
);
define_id!(
    /// Identifies a host group (folder).
    GroupId
);
define_id!(
    /// Identifies a private key managed by Aether.
    KeyId
);
define_id!(
    /// Identifies a command snippet.
    SnippetId
);
define_id!(
    /// Identifies a live SSH connection (one authenticated session with a server).
    ConnectionId
);
define_id!(
    /// Identifies a terminal (one shell channel, shown in one pane).
    TerminalId
);
define_id!(
    /// Identifies a file transfer job.
    TransferId
);
define_id!(
    /// Identifies a question the backend has asked the user (host key, password, ...).
    PromptId
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_roundtrip_as_plain_strings() {
        let id = HostId::new();
        let json = serde_json::to_string(&id).unwrap();
        assert!(json.starts_with('"') && json.ends_with('"'));
        let back: HostId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
        assert_eq!(id.to_string().parse::<HostId>().unwrap(), id);
    }
}
