//! Talking to an SSH agent: the Windows OpenSSH agent (named pipe) or Pageant.

use aether_core::model::AgentMode;
use russh::keys::agent::client::{AgentClient, AgentStream};
use tracing::debug;

use crate::error::{Result, SshError};

/// Named pipe of the "OpenSSH Authentication Agent" Windows service.
pub const OPENSSH_AGENT_PIPE: &str = r"\\.\pipe\openssh-ssh-agent";

pub(crate) type Agent = AgentClient<Box<dyn AgentStream + Send + Unpin>>;

fn unavailable(what: &str, e: impl std::fmt::Display) -> String {
    format!("{what}: {e}")
}

#[cfg(windows)]
async fn connect_openssh() -> std::result::Result<Agent, String> {
    // Respect SSH_AUTH_SOCK when it names a pipe (Git for Windows, 1Password, etc.).
    let path = std::env::var("SSH_AUTH_SOCK")
        .ok()
        .filter(|p| p.starts_with(r"\\.\pipe\"))
        .unwrap_or_else(|| OPENSSH_AGENT_PIPE.to_string());
    AgentClient::connect_named_pipe(&path).await.map(AgentClient::dynamic).map_err(|e| unavailable("OpenSSH agent", e))
}

#[cfg(unix)]
async fn connect_openssh() -> std::result::Result<Agent, String> {
    AgentClient::connect_env().await.map(AgentClient::dynamic).map_err(|e| unavailable("ssh-agent", e))
}

#[cfg(windows)]
async fn connect_pageant() -> std::result::Result<Agent, String> {
    AgentClient::connect_pageant().await.map(AgentClient::dynamic).map_err(|e| unavailable("Pageant", e))
}

#[cfg(not(windows))]
async fn connect_pageant() -> std::result::Result<Agent, String> {
    Err("Pageant is only available on Windows".into())
}

/// Connects to an agent according to the user's preference.
pub(crate) async fn connect(mode: AgentMode) -> Result<Agent> {
    let result = match mode {
        AgentMode::Off => Err("the agent is disabled in Settings".to_string()),
        AgentMode::OpenSsh => connect_openssh().await,
        AgentMode::Pageant => connect_pageant().await,
        AgentMode::Auto => match connect_openssh().await {
            Ok(agent) => Ok(agent),
            Err(first) => {
                debug!(%first, "OpenSSH agent unavailable, trying Pageant");
                connect_pageant().await.map_err(|second| format!("{first}; {second}"))
            }
        },
    };
    result.map_err(SshError::AgentUnavailable)
}

/// A raw byte stream to the agent, for agent forwarding.
pub(crate) async fn connect_stream(mode: AgentMode) -> Result<Box<dyn AgentStream + Send + Unpin>> {
    Ok(connect(mode).await?.into_inner())
}

/// Whether any agent is reachable (used by the UI to show agent status).
pub async fn agent_available(mode: AgentMode) -> bool {
    connect(mode).await.is_ok()
}
