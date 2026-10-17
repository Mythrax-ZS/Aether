//! Authentication: agent → key files → keyboard-interactive → password, like OpenSSH,
//! but with interactive retries and a friendlier failure report.

use std::collections::HashSet;
use std::sync::Arc;

use aether_core::model::AuthMethodKind;
use aether_vault::Secret;
use russh::client::{Handle, KeyboardInteractiveAuthResponse};
use russh::keys::{Algorithm, PrivateKeyWithHashAlg};
use russh::{MethodKind, MethodSet};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::agent;
use crate::delegate::{
    ConnectProgress, ConnectStage, ConnectionDelegate, HopInfo, InteractivePrompt, PromptKind, PromptRequest,
    SecretKind, SecretLookup,
};
use crate::error::{Result, SshError};
use crate::handler::ClientHandler;
use crate::keys;
use crate::spec::{ConnectSpec, KeyRef};

const MAX_PASSWORD_ATTEMPTS: u32 = 3;
const MAX_PASSES: u32 = 3;

pub(crate) struct AuthCtx<'a> {
    pub delegate: &'a Arc<dyn ConnectionDelegate>,
    pub hop: &'a HopInfo,
    pub spec: &'a ConnectSpec,
    pub cancel: &'a CancellationToken,
}

/// What one authentication method concluded.
enum Outcome {
    Success,
    /// Rejected. `partial` means the server accepted this factor and wants another.
    Failed {
        partial: bool,
    },
    /// Not attempted (not offered by the server, nothing to try, agent missing, ...).
    Skipped,
}

fn kind_of(m: AuthMethodKind) -> MethodKind {
    match m {
        AuthMethodKind::Agent | AuthMethodKind::PublicKey => MethodKind::PublicKey,
        AuthMethodKind::KeyboardInteractive => MethodKind::KeyboardInteractive,
        AuthMethodKind::Password => MethodKind::Password,
    }
}

fn method_label(m: AuthMethodKind) -> &'static str {
    match m {
        AuthMethodKind::Agent => "ssh-agent",
        AuthMethodKind::PublicKey => "public key",
        AuthMethodKind::KeyboardInteractive => "keyboard-interactive",
        AuthMethodKind::Password => "password",
    }
}

fn check_cancel(ctx: &AuthCtx<'_>) -> Result<()> {
    if ctx.cancel.is_cancelled() { Err(SshError::Cancelled) } else { Ok(()) }
}

fn announce(ctx: &AuthCtx<'_>, method: AuthMethodKind) {
    ctx.delegate.progress(ConnectProgress {
        hop: ctx.hop.clone(),
        stage: ConnectStage::Authenticating { method: method_label(method).into() },
    });
}

/// Logs in on `handle` using the methods configured in `ctx.spec.auth`.
pub(crate) async fn authenticate(handle: &mut Handle<ClientHandler>, ctx: &AuthCtx<'_>) -> Result<()> {
    let user = ctx.spec.username.clone();

    // Ask the server what it accepts. "none" also succeeds on unauthenticated servers.
    let mut remaining: MethodSet = match handle.authenticate_none(user.clone()).await? {
        russh::client::AuthResult::Success => {
            info!(host = %ctx.hop.host, "server allows access without authentication");
            return Ok(());
        }
        russh::client::AuthResult::Failure { remaining_methods, .. } => remaining_methods,
    };
    debug!(host = %ctx.hop.host, methods = ?remaining, "server authentication methods");

    let configured: Vec<AuthMethodKind> = ctx.spec.auth.methods.clone();
    let mut tried: Vec<String> = Vec::new();
    let mut exhausted: HashSet<AuthMethodKind> = HashSet::new();

    for _pass in 0..MAX_PASSES {
        let mut progressed = false;
        for &method in &configured {
            check_cancel(ctx)?;
            if exhausted.contains(&method) || !remaining.contains(&kind_of(method)) {
                continue;
            }
            announce(ctx, method);
            let outcome = match method {
                AuthMethodKind::Agent => try_agent(handle, ctx, &user, &mut remaining).await?,
                AuthMethodKind::PublicKey => try_key_files(handle, ctx, &user, &mut remaining).await?,
                AuthMethodKind::KeyboardInteractive => {
                    try_keyboard_interactive(handle, ctx, &user, &mut remaining).await?
                }
                AuthMethodKind::Password => try_password(handle, ctx, &user, &mut remaining).await?,
            };
            match outcome {
                Outcome::Success => {
                    info!(host = %ctx.hop.host, method = method_label(method), "authenticated");
                    return Ok(());
                }
                Outcome::Failed { partial } => {
                    let label = method_label(method).to_string();
                    if !tried.contains(&label) {
                        tried.push(label);
                    }
                    if partial {
                        // Multi-factor: the server took this factor; earlier methods may now work.
                        progressed = true;
                        exhausted.clear();
                    } else {
                        exhausted.insert(method);
                    }
                }
                Outcome::Skipped => {
                    exhausted.insert(method);
                }
            }
        }
        if !progressed {
            break;
        }
    }

    if tried.is_empty() {
        // Nothing was even attempted: no usable credentials for what the server offers.
        let offered: Vec<&str> = remaining.iter().map(<&str>::from).collect();
        debug!(?offered, "no usable authentication method");
        return Err(SshError::NoAuthMethods);
    }
    Err(SshError::AuthFailed { host: ctx.hop.host.clone(), user, tried })
}

/// Applies a failure reply: refresh the server's remaining-methods list.
fn absorb_failure(remaining: &mut MethodSet, new: MethodSet) {
    *remaining = new;
}

// ── ssh-agent ────────────────────────────────────────────────────────────────

async fn try_agent(
    handle: &mut Handle<ClientHandler>,
    ctx: &AuthCtx<'_>,
    user: &str,
    remaining: &mut MethodSet,
) -> Result<Outcome> {
    // IdentitiesOnly means "offer only the keys I configured, whatever the agent holds". OpenSSH reads that as: the
    // agent may sign for the public half of a configured identity file — which is how one key is picked out of an agent
    // that holds many (`IdentityFile ~/.ssh/work.pub`). With no such file there is nothing to pick.
    let only: Option<Vec<russh::keys::PublicKey>> = if ctx.spec.auth.identities_only {
        let mut chosen = Vec::new();
        for path in identity_files(ctx) {
            if let Some(key) = keys::public_half(&path).await {
                chosen.push(key);
            }
        }
        if chosen.is_empty() {
            return Ok(Outcome::Skipped);
        }
        Some(chosen)
    } else {
        None
    };
    let mut agent = match agent::connect(ctx.spec.auth.agent).await {
        Ok(a) => a,
        Err(e) => {
            debug!(error = %e, "no ssh-agent");
            return Ok(Outcome::Skipped);
        }
    };
    let identities = match agent.request_identities().await {
        Ok(i) => i,
        Err(e) => {
            warn!(error = %e, "could not list agent identities");
            return Ok(Outcome::Skipped);
        }
    };
    let identities: Vec<_> = identities
        .into_iter()
        .filter(|i| only.as_ref().is_none_or(|only| only.iter().any(|k| k.key_data() == i.public_key().key_data())))
        .collect();
    if identities.is_empty() {
        debug!("ssh-agent holds no (matching) keys");
        return Ok(Outcome::Skipped);
    }

    let mut partial_seen = false;
    for identity in identities {
        check_cancel(ctx)?;
        let public = identity.public_key().into_owned();
        let hash = if matches!(public.algorithm(), Algorithm::Rsa { .. }) {
            handle.best_supported_rsa_hash().await?.flatten()
        } else {
            None
        };
        match handle.authenticate_publickey_with(user.to_string(), public, hash, &mut agent).await {
            Ok(russh::client::AuthResult::Success) => return Ok(Outcome::Success),
            Ok(russh::client::AuthResult::Failure { remaining_methods, partial_success }) => {
                partial_seen |= partial_success;
                absorb_failure(remaining, remaining_methods);
                if partial_success {
                    break;
                }
            }
            Err(e) => {
                debug!(error = %e, comment = identity.comment(), "agent signing failed");
            }
        }
    }
    Ok(Outcome::Failed { partial: partial_seen })
}

// ── key files / vault keys ───────────────────────────────────────────────────

/// The identity files this host's key choice comes down to: its own key, if it names one, else the defaults.
fn identity_files(ctx: &AuthCtx<'_>) -> Vec<std::path::PathBuf> {
    match &ctx.spec.auth.key {
        Some(KeyRef::File(path)) => vec![path.clone()],
        Some(KeyRef::Vault { .. }) => Vec::new(),
        None => ctx.spec.auth.default_identity_files.clone(),
    }
}

async fn try_key_files(
    handle: &mut Handle<ClientHandler>,
    ctx: &AuthCtx<'_>,
    user: &str,
    remaining: &mut MethodSet,
) -> Result<Outcome> {
    let explicit = ctx.spec.auth.key.is_some();
    let agent_configured = ctx.spec.auth.methods.contains(&AuthMethodKind::Agent);
    let candidates: Vec<KeyRef> = match &ctx.spec.auth.key {
        Some(k) => vec![k.clone()],
        None => ctx.spec.auth.default_identity_files.iter().cloned().map(KeyRef::File).collect(),
    };

    let mut attempted = false;
    for key_ref in candidates {
        check_cancel(ctx)?;
        // A public key file stands for a key that lives in an agent, which signs for it (see `try_agent`): there is
        // nothing here to load, and no reason to call it unreadable. Without the agent among the methods nobody will
        // sign for it, and saying so beats a bare "no usable method".
        if agent_configured
            && let KeyRef::File(path) = &key_ref
            && keys::names_an_agent_key(path).await
        {
            debug!(key = %key_ref.display_name(), "only the public half is a file; leaving it to the agent");
            continue;
        }
        let loaded = match keys::load_key(
            &key_ref,
            ctx.delegate.as_ref(),
            ctx.hop,
            ctx.spec.secret_scope.as_deref(),
            true,
        )
        .await
        {
            Ok(Some(k)) => k,
            Ok(None) => {
                if explicit {
                    return Err(SshError::KeyUnreadable {
                        key: key_ref.display_name(),
                        reason: "the file does not exist".into(),
                    });
                }
                continue;
            }
            // The user cancelled, or typed the wrong passphrase three times: stop here.
            Err(e @ (SshError::AuthCancelled | SshError::Cancelled)) => return Err(e),
            Err(e) if explicit => return Err(e),
            Err(e) => {
                debug!(key = %key_ref.display_name(), error = %e, "skipping unusable default key");
                continue;
            }
        };
        attempted = true;

        let hash = if matches!(loaded.key.algorithm(), Algorithm::Rsa { .. }) {
            handle.best_supported_rsa_hash().await?.flatten()
        } else {
            None
        };
        let with_hash = PrivateKeyWithHashAlg::new(loaded.key.clone(), hash);
        match handle.authenticate_publickey(user.to_string(), with_hash).await? {
            russh::client::AuthResult::Success => return Ok(Outcome::Success),
            russh::client::AuthResult::Failure { remaining_methods, partial_success } => {
                debug!(key = %loaded.name, "public key rejected");
                absorb_failure(remaining, remaining_methods);
                if partial_success {
                    return Ok(Outcome::Failed { partial: true });
                }
            }
        }
    }
    Ok(if attempted { Outcome::Failed { partial: false } } else { Outcome::Skipped })
}

// ── keyboard-interactive ─────────────────────────────────────────────────────

/// Is this question asking for the account's password — the thing a saved password is for? Hidden prompts that merely
/// mention one are not: a one-time "password" (pam_oath asks for "One-time password (OATH)") would be answered with the
/// real one and then, being refused, make Aether forget it; and the questions of a password change want the *new*
/// password, not the saved one.
fn looks_like_password_prompt(p: &russh::client::Prompt) -> bool {
    const NOT_THE_PASSWORD: [&str; 13] = [
        "one-time",
        "one time",
        "otp",
        "oath",
        "token",
        "passcode",
        "verification",
        "authenticator",
        "duo",
        "new password",
        "retype",
        "confirm",
        "again",
    ];
    if p.echo {
        return false;
    }
    let text = p.prompt.to_ascii_lowercase();
    text.contains("password") && !NOT_THE_PASSWORD.iter().any(|w| text.contains(w))
}

async fn try_keyboard_interactive(
    handle: &mut Handle<ClientHandler>,
    ctx: &AuthCtx<'_>,
    user: &str,
    remaining: &mut MethodSet,
) -> Result<Outcome> {
    let lookup = SecretLookup { scope: ctx.spec.secret_scope.clone(), kind: SecretKind::Password };
    let can_remember = ctx.spec.secret_scope.is_some();
    let mut saved = ctx.delegate.saved_secret(&lookup);
    let mut response = handle.authenticate_keyboard_interactive_start(user.to_string(), None).await?;
    let mut rounds = 0;
    let mut attempt = 0;
    let mut error: Option<String> = None;
    // What the person asked to be remembered: stored once the server has accepted it, never before. With the questions
    // it answered, so that a refused second factor doesn't throw away a password the server did accept.
    let mut to_remember: Option<(Secret, Vec<String>)> = None;
    // The questions of the previous round, and whether the saved password was what answered them.
    let mut last_questions: Option<Vec<String>> = None;
    let mut sent_saved = false;

    loop {
        check_cancel(ctx)?;
        match response {
            KeyboardInteractiveAuthResponse::Success => {
                if let Some((secret, _)) = to_remember.take() {
                    ctx.delegate.remember_secret(&lookup, &secret);
                }
                return Ok(Outcome::Success);
            }
            KeyboardInteractiveAuthResponse::Failure { remaining_methods, partial_success } => {
                absorb_failure(remaining, remaining_methods);
                if partial_success {
                    // The server took everything it asked here and wants another method as well: what was typed was right.
                    if let Some((secret, _)) = to_remember.take() {
                        ctx.delegate.remember_secret(&lookup, &secret);
                    }
                } else if sent_saved {
                    ctx.delegate.forget_secret(&lookup);
                }
                return Ok(Outcome::Failed { partial: partial_success });
            }
            KeyboardInteractiveAuthResponse::InfoRequest { name, instructions, prompts } => {
                rounds += 1;
                if rounds > 8 {
                    return Ok(Outcome::Failed { partial: false });
                }
                // The very same questions straight after answering them mean the answer was refused. A different
                // set (a verification code after the password) is just the next step and says nothing of the kind.
                let questions: Vec<String> = prompts.iter().map(|p| p.prompt.clone()).collect();
                if !questions.is_empty() && last_questions.as_ref() == Some(&questions) {
                    // Only an answer to *these* questions is refused: a wrong code leaves the accepted password alone.
                    if to_remember.as_ref().is_some_and(|(_, asked)| *asked == questions) {
                        to_remember = None;
                    }
                    error = Some(if sent_saved {
                        ctx.delegate.forget_secret(&lookup);
                        "The saved password was rejected. Enter it again.".into()
                    } else {
                        "Permission denied, please try again.".into()
                    });
                }
                sent_saved = false;
                last_questions = Some(questions.clone());

                let single_password = prompts.len() == 1 && looks_like_password_prompt(&prompts[0]);
                let answers: Vec<String> = if prompts.is_empty() {
                    Vec::new()
                } else if single_password && saved.is_some() {
                    // Servers often implement plain passwords over keyboard-interactive (PAM).
                    let s = saved.take().expect("checked above");
                    sent_saved = true;
                    vec![s.expose_text().unwrap_or_default().to_string()]
                } else {
                    let response = ctx
                        .delegate
                        .prompt(PromptRequest {
                            hop: ctx.hop.clone(),
                            kind: PromptKind::KeyboardInteractive {
                                name,
                                instructions,
                                prompts: prompts
                                    .iter()
                                    .map(|p| InteractivePrompt { text: p.prompt.clone(), echo: p.echo })
                                    .collect(),
                            },
                            attempt,
                            error: error.take(),
                            can_remember: single_password && can_remember,
                        })
                        .await
                        .ok_or(SshError::AuthCancelled)?;
                    attempt += 1;
                    // Kept across later rounds (a code after the password) and stored only if the login completes.
                    if response.remember
                        && single_password
                        && let Some(first) = response.answers.first()
                    {
                        to_remember = Some((first.clone(), questions));
                    }
                    response.answers.iter().map(|a: &Secret| a.expose_text().unwrap_or_default().to_string()).collect()
                };
                response = handle.authenticate_keyboard_interactive_respond(answers).await?;
            }
        }
    }
}

// ── password ─────────────────────────────────────────────────────────────────

async fn try_password(
    handle: &mut Handle<ClientHandler>,
    ctx: &AuthCtx<'_>,
    user: &str,
    remaining: &mut MethodSet,
) -> Result<Outcome> {
    let lookup = SecretLookup { scope: ctx.spec.secret_scope.clone(), kind: SecretKind::Password };
    let mut error: Option<String> = None;

    // A saved password gets one silent try before we bother the user.
    if let Some(saved) = ctx.delegate.saved_secret(&lookup)
        && let Some(pw) = saved.expose_text()
    {
        match handle.authenticate_password(user.to_string(), pw.to_string()).await? {
            russh::client::AuthResult::Success => return Ok(Outcome::Success),
            russh::client::AuthResult::Failure { remaining_methods, partial_success } => {
                absorb_failure(remaining, remaining_methods);
                if partial_success {
                    return Ok(Outcome::Failed { partial: true });
                }
                ctx.delegate.forget_secret(&lookup);
                error = Some("The saved password was rejected. Enter it again.".into());
            }
        }
    }

    for attempt in 0..MAX_PASSWORD_ATTEMPTS {
        check_cancel(ctx)?;
        if !remaining.contains(&MethodKind::Password) {
            break;
        }
        let response = ctx
            .delegate
            .prompt(PromptRequest {
                hop: ctx.hop.clone(),
                kind: PromptKind::Password,
                attempt,
                error: error.take(),
                // Without a saved host there is nowhere to keep it.
                can_remember: ctx.spec.secret_scope.is_some(),
            })
            .await
            .ok_or(SshError::AuthCancelled)?;
        let Some(answer) = response.answers.first() else {
            return Err(SshError::AuthCancelled);
        };
        let password = answer.expose_text().unwrap_or_default().to_string();
        match handle.authenticate_password(user.to_string(), password).await? {
            russh::client::AuthResult::Success => {
                if response.remember {
                    ctx.delegate.remember_secret(&lookup, answer);
                }
                return Ok(Outcome::Success);
            }
            russh::client::AuthResult::Failure { remaining_methods, partial_success } => {
                absorb_failure(remaining, remaining_methods);
                if partial_success {
                    return Ok(Outcome::Failed { partial: true });
                }
                error = Some("Permission denied, please try again.".into());
            }
        }
    }
    Ok(Outcome::Failed { partial: false })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt(text: &str, echo: bool) -> russh::client::Prompt {
        russh::client::Prompt { prompt: text.into(), echo }
    }

    /// The saved password answers the account's password and nothing else: not a one-time code that happens to be
    /// called a password, and not the questions of a password change.
    #[test]
    fn only_the_accounts_own_password_counts_as_one() {
        for yes in [
            "Password:",
            "Password: ",
            "me@host's password:",
            "(me@host) Password:",
            "Enter password:",
            "Current password:",
            "(current) UNIX password:",
        ] {
            assert!(looks_like_password_prompt(&prompt(yes, false)), "{yes}");
        }
        for no in [
            "One-time password (OATH):",
            "Enter your one time password:",
            "OTP password:",
            "Duo passcode or option (1-1):",
            "Token password:",
            "Verification code:",
            "New password:",
            "Retype new password:",
            "Confirm password:",
            "Password again:",
            "Username:",
        ] {
            assert!(!looks_like_password_prompt(&prompt(no, false)), "{no}");
        }
        assert!(
            !looks_like_password_prompt(&prompt("Password:", true)),
            "a question whose answer is shown isn't a password"
        );
    }
}
