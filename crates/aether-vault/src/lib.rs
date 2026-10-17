//! # aether-vault
//!
//! Where Aether keeps secrets. Nothing sensitive is ever written to `hosts.json`,
//! `settings.json` or the logs; instead hosts reference secrets by id and this crate stores
//! them using the operating system's own protection:
//!
//! | Secret | Backend (installed) | Why |
//! |---|---|---|
//! | Saved passwords, key passphrases | **Windows Credential Manager** | Visible/revocable in the OS UI, bound to the Windows user |
//! | Private key material | **DPAPI**-sealed files | Keys exceed Credential Manager's 2.5 KB blob limit |
//!
//! In portable mode everything uses DPAPI files next to the executable. All secrets are
//! wrapped in [`Secret`], which is zeroed on drop and never prints its contents.

mod error;
mod memory;
mod secret;
mod store;

#[cfg(windows)]
mod credman;
#[cfg(windows)]
mod dpapi;

pub use error::{Result, VaultError};
pub use memory::MemoryStore;
pub use secret::{Namespace, Secret, SecretKey};
pub use store::{CompositeStore, SecretStore, open_default};

#[cfg(windows)]
pub use credman::CredentialManagerStore;
#[cfg(windows)]
pub use dpapi::{DpapiFileStore, dpapi_protect, dpapi_unprotect};
