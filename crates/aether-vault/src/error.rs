use std::io;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    /// No usable secret backend on this platform/configuration.
    #[error("secure storage is unavailable: {0}")]
    Unavailable(String),
    /// A Windows API call failed.
    #[error("{operation} failed (0x{code:08X}): {message}")]
    Os { operation: &'static str, code: u32, message: String },
    #[error("secret is too large for this backend ({size} bytes, limit {limit})")]
    TooLarge { size: usize, limit: usize },
    #[error("invalid secret identifier `{0}`")]
    InvalidId(String),
    #[error("stored secret could not be decrypted — it was probably created by another Windows user or on another PC")]
    Undecryptable,
    #[error("I/O error on {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

pub type Result<T, E = VaultError> = std::result::Result<T, E>;

impl VaultError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io { path: path.into(), source }
    }
}
