//! Hosts, groups, recents, saved passwords and `~/.ssh/config` import.

use std::collections::{HashMap, HashSet};

use aether_core::error::UserFacing;
use aether_core::model::{Group, Host, HostOrigin};
use aether_core::ssh_config::{self, SshConfig};
use aether_core::store::{GroupDeleteMode, HostsData, ImportSummary, RecentTarget};
use aether_core::util::local_username;
use aether_core::{ErrorInfo, GroupId, HostId};
use aether_vault::{Namespace, Secret, SecretKey};
use tauri::{Manager, WebviewWindow};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

use super::app::map_core;
use super::{St, blocking};
use crate::dto::{ImportCandidateDto, SshConfigPreview};
use crate::state::{Res, not_found};

#[tauri::command]
pub fn hosts_snapshot(state: St<'_>) -> HostsData {
    state.hosts.snapshot()
}

/// Asks in a native window rather than in the web view: whatever can script the page can press its buttons, but not
/// these. Returns `true` for the first button.
async fn confirm_native(window: &WebviewWindow, title: &str, message: String, yes: &str, no: &str) -> bool {
    let (tx, rx) = tokio::sync::oneshot::channel();
    window
        .app_handle()
        .dialog()
        .message(message)
        .title(title)
        .parent(window)
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(yes.into(), no.into()))
        .show(move |answer| {
            let _ = tx.send(answer);
        });
    rx.await.unwrap_or(false)
}

/// What one confirmation will show at most. It is a native message box, which does not scroll: whatever does not fit
/// is clipped, while the button that approves stays clickable. So the limits are counted in what is *printed* — control
/// characters are written out, which makes them longer than they were typed — and anything over them is not shown at
/// all, and so not approved.
const MAX_COMMAND_CHARS: usize = 800;
const MAX_REVIEW_CHARS: usize = 2000;
const MAX_DISTINCT_COMMANDS: usize = 5;
/// Host names are the person's own labels, but a config file chooses them on import.
const MAX_LABEL_CHARS: usize = 40;
const LABELS_SHOWN: usize = 5;

/// Text as the confirmation prints it: every character there, and the ones that could hide part of it — line breaks,
/// tabs, other control characters, zero-width and direction-changing characters — written out.
fn visible(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control()
                || matches!(c as u32, 0x200B..=0x200F | 0x2028..=0x202E | 0x2060..=0x2064 | 0x2066..=0x2069 | 0xFEFF) =>
            {
                out.push_str(&format!("\\u{{{:04x}}}", c as u32))
            }
            c => out.push(c),
        }
    }
    out
}

/// A host's name for the confirmation: escaped like a command (a name with a line break could otherwise write its own
/// lines into the dialog) and short.
fn short_label(label: &str) -> String {
    let shown = visible(label.trim());
    if shown.chars().count() <= MAX_LABEL_CHARS {
        shown
    } else {
        shown.chars().take(MAX_LABEL_CHARS - 1).chain(['…']).collect()
    }
}

/// A command that runs whenever a host is connected to is code execution. Shown whole — once per distinct command,
/// with the hosts that use it — or `None` when there is too much to show, in which case nothing is approved.
fn describe_commands(commands: &[(String, String)]) -> Option<String> {
    // Thirty hosts that share one command are one thing to check. Grouped by the command as it is, never by how it
    // prints: two commands that print alike but differ are two entries.
    let mut groups: Vec<(&str, Vec<&str>)> = Vec::new();
    for (label, command) in commands {
        let command = command.trim();
        match groups.iter_mut().find(|(c, _)| *c == command) {
            Some((_, labels)) => labels.push(label),
            None => groups.push((command, vec![label])),
        }
    }
    if groups.len() > MAX_DISTINCT_COMMANDS {
        return None;
    }
    let mut out = String::new();
    let mut unusual = false;
    for (command, labels) in &groups {
        let shown = visible(command);
        if shown.chars().count() > MAX_COMMAND_CHARS {
            return None;
        }
        unusual |= !shown.is_ascii();
        let names: Vec<String> = labels.iter().take(LABELS_SHOWN).map(|l| short_label(l)).collect();
        let more = labels.len().saturating_sub(LABELS_SHOWN);
        let and_more = if more > 0 { format!(" and {more} more") } else { String::new() };
        out.push_str(&format!("\n{shown}\n    for {}{and_more}\n", names.join(", ")));
    }
    if unusual {
        out.push_str("\nThis has characters beyond plain ASCII; some of them can look like ordinary letters.\n");
    }
    (out.chars().count() <= MAX_REVIEW_CHARS).then_some(out)
}

/// Tells in a native window, which the page can't dismiss on the person's behalf.
async fn notice_native(window: &WebviewWindow, title: &str, message: String) {
    let (tx, rx) = tokio::sync::oneshot::channel();
    window
        .app_handle()
        .dialog()
        .message(message)
        .title(title)
        .parent(window)
        .kind(MessageDialogKind::Info)
        .buttons(MessageDialogButtons::Ok)
        .show(move |_| {
            let _ = tx.send(());
        });
    let _ = rx.await;
}

#[tauri::command]
pub async fn host_save(window: WebviewWindow, state: St<'_>, host: Host) -> Res<Host> {
    // A proxy command runs as you every time the host is opened, so a new or changed one gets a native yes/no first.
    let command = host.options.proxy_command.trim().to_string();
    let known = state.hosts.get(host.id).map(|h| h.options.proxy_command.trim().to_string());
    if !command.is_empty() && known.as_deref() != Some(command.as_str()) {
        // Don't ask about saving something that is about to be refused anyway.
        let mut checked = host.clone();
        checked.normalize();
        checked.validate().map_err(map_core)?;
        let name = if host.label.trim().is_empty() { host.hostname.clone() } else { host.label.clone() };
        let Some(shown) = describe_commands(&[(name.clone(), command)]) else {
            return Err(ErrorInfo::invalid(
                "proxy command",
                format!(
                    "It is too long to check by eye (more than {MAX_COMMAND_CHARS} characters as shown, where an invisible or control character counts for several). Put it in a script and run that instead."
                ),
            ));
        };
        let name = short_label(&name);
        let ok = confirm_native(
            &window,
            "Run a command when connecting?",
            format!(
                "“{name}” will run this command on your PC every time you connect to it:\n{shown}\nIt runs with your \
                 permissions. Only continue if you wrote it, or trust where it came from."
            ),
            "Save host",
            "Cancel",
        )
        .await;
        if !ok {
            return Err(ErrorInfo::cancelled());
        }
    }
    state.hosts.upsert(host).map_err(map_core)
}

#[tauri::command]
pub fn hosts_delete(state: St<'_>, ids: Vec<HostId>) -> Res<()> {
    state.hosts.delete(&ids).map_err(map_core)?;
    // Saved credentials die with the host.
    for id in ids {
        if let Ok(key) = SecretKey::new(Namespace::HostPassword, id.to_string()) {
            let _ = state.vault.delete(&key);
        }
    }
    Ok(())
}

#[tauri::command]
pub fn host_duplicate(state: St<'_>, id: HostId) -> Res<Host> {
    state.hosts.duplicate(id).map_err(map_core)
}

#[tauri::command]
pub fn host_favorite(state: St<'_>, id: HostId, favorite: bool) -> Res<()> {
    state.hosts.set_favorite(id, favorite).map_err(map_core)
}

#[tauri::command]
pub fn hosts_move(state: St<'_>, ids: Vec<HostId>, group_id: Option<GroupId>) -> Res<()> {
    state.hosts.move_hosts(&ids, group_id).map_err(map_core)
}

#[tauri::command]
pub fn hosts_reorder(state: St<'_>, ids: Vec<HostId>) -> Res<()> {
    state.hosts.reorder_hosts(&ids).map_err(map_core)
}

#[tauri::command]
pub fn group_save(state: St<'_>, group: Group) -> Res<Group> {
    state.hosts.upsert_group(group).map_err(map_core)
}

#[tauri::command]
pub fn group_delete(state: St<'_>, id: GroupId, mode: GroupDeleteMode) -> Res<()> {
    state.hosts.delete_group(id, mode).map_err(map_core)
}

#[tauri::command]
pub fn group_collapse(state: St<'_>, id: GroupId, collapsed: bool) -> Res<()> {
    state.hosts.set_group_collapsed(id, collapsed).map_err(map_core)
}

#[tauri::command]
pub fn recents_list(state: St<'_>) -> Vec<RecentTarget> {
    state.recents.list()
}

#[tauri::command]
pub fn recents_clear(state: St<'_>) -> Res<()> {
    state.recents.clear().map_err(map_core)
}

// ── saved passwords ──────────────────────────────────────────────────────────

fn password_key(id: HostId) -> Res<SecretKey> {
    SecretKey::new(Namespace::HostPassword, id.to_string()).map_err(|e| aether_core::ErrorInfo::internal(e.to_string()))
}

fn vault_err(e: aether_vault::VaultError) -> aether_core::ErrorInfo {
    aether_core::ErrorInfo::new(
        aether_core::ErrorCode::VaultUnavailable,
        "Secure storage problem",
        "Aether couldn't reach the Windows credential store.",
    )
    .detail(e.to_string())
}

#[tauri::command]
pub fn host_password_exists(state: St<'_>, id: HostId) -> Res<bool> {
    state.vault.exists(&password_key(id)?).map_err(vault_err)
}

#[tauri::command]
pub fn host_password_set(state: St<'_>, id: HostId, password: String) -> Res<()> {
    if state.hosts.get(id).is_none() {
        return Err(not_found("This host"));
    }
    state.vault.put(&password_key(id)?, &Secret::from_text(&password)).map_err(vault_err)
}

#[tauri::command]
pub fn host_password_clear(state: St<'_>, id: HostId) -> Res<()> {
    state.vault.delete(&password_key(id)?).map(|_| ()).map_err(vault_err)
}

// ── ~/.ssh/config import ─────────────────────────────────────────────────────

fn load_config() -> (Option<SshConfig>, Option<String>) {
    let path = aether_core::AppPaths::user_ssh_dir().map(|d| d.join("config"));
    match path {
        Some(p) if p.is_file() => match SshConfig::load_file(&p) {
            Ok(c) => (Some(c), Some(p.display().to_string())),
            Err(e) => {
                tracing::warn!(error = %e, "could not read ssh config");
                (None, Some(p.display().to_string()))
            }
        },
        _ => (None, None),
    }
}

/// Lists the `Host` entries in `~/.ssh/config` so the user can choose what to import.
#[tauri::command]
pub async fn ssh_config_preview(state: St<'_>) -> Res<SshConfigPreview> {
    // Reading a config (and the files it includes) is disk work; it must not hold up the thread serving the UI.
    let (config, path) = blocking(|| Ok(load_config())).await?;
    let Some(config) = config else {
        return Ok(SshConfigPreview { found: path.is_some(), path, candidates: Vec::new(), warnings: Vec::new() });
    };
    let existing: HashSet<String> = state
        .hosts
        .snapshot()
        .hosts
        .iter()
        .filter_map(|h| match &h.origin {
            HostOrigin::SshConfig { alias } => Some(alias.to_lowercase()),
            _ => None,
        })
        .collect();
    let candidates = ssh_config::candidates(&config, &local_username())
        .into_iter()
        .map(|c| {
            let r = &c.resolved;
            ImportCandidateDto {
                already_imported: existing.contains(&c.alias.to_lowercase()),
                alias: c.alias.clone(),
                hostname: r.hostname.clone(),
                port: r.port,
                user: r.user.clone(),
                identity_file: r.identity_files.first().cloned(),
                via: r.proxy_jump.clone(),
                uses_proxy_command: r.proxy_command.is_some(),
                forward_count: r.forwards.len() as u32,
                warnings: r.warnings.clone(),
            }
        })
        .collect();
    Ok(SshConfigPreview { found: true, path, candidates, warnings: config.warnings.clone() })
}

/// Imports the chosen aliases. `ProxyJump` aliases become real links between hosts: to the saved host when the alias
/// was imported before, and otherwise the jump host is imported along with the host that needs it — a host must not
/// silently lose its bastion and connect around it.
#[tauri::command]
pub async fn ssh_config_import(
    window: WebviewWindow,
    state: St<'_>,
    aliases: Vec<String>,
    group_id: Option<GroupId>,
) -> Res<ImportSummary> {
    let (config, _) = blocking(|| Ok(load_config())).await?;
    let config = config.ok_or_else(|| not_found("Your SSH config"))?;
    let all = ssh_config::candidates(&config, &local_username());

    let saved: HashMap<String, HostId> = state
        .hosts
        .snapshot()
        .hosts
        .iter()
        .filter_map(|h| match &h.origin {
            HostOrigin::SshConfig { alias } => Some((alias.to_lowercase(), h.id)),
            _ => None,
        })
        .collect();

    // The chosen aliases, plus (transitively) every alias they jump through that isn't saved yet.
    let wanted = ssh_config::with_jump_dependencies(&all, &aliases, &saved);

    // Convert everything so jump references resolve, then keep the selection.
    let mut selected: Vec<Host> = ssh_config::to_hosts_linked(&all, &saved)
        .into_iter()
        .filter(|h| match &h.origin {
            HostOrigin::SshConfig { alias } => wanted.contains(&alias.to_lowercase()),
            _ => false,
        })
        .collect();

    // A ProxyCommand in a config file is code that runs on every connect. Say so, in a window the page can't click.
    let commands: Vec<(String, String)> = selected
        .iter()
        .filter(|h| !h.options.proxy_command.trim().is_empty())
        .map(|h| (h.label.clone(), h.options.proxy_command.trim().to_string()))
        .collect();
    if !commands.is_empty() {
        let ok = match describe_commands(&commands) {
            Some(shown) => {
                confirm_native(
                    &window,
                    "Import hosts that run commands?",
                    format!(
                        "{} of the hosts you are importing run a command on your PC whenever you connect to them:\n{shown}\nThey run \
                         with your permissions. Import the commands only if you trust this config; otherwise the hosts are \
                         imported without them.",
                        commands.len(),
                    ),
                    "Import with commands",
                    "Import without them",
                )
                .await
            }
            None => {
                notice_native(
                    &window,
                    "Commands left out",
                    format!(
                        "{} of the hosts you are importing run a command whenever you connect, but there is too much command text to check here, so the hosts are imported without it. Add a command to a host yourself afterwards if you want it.",
                        commands.len()
                    ),
                )
                .await;
                false
            }
        };
        if !ok {
            for host in &mut selected {
                host.options.proxy_command.clear();
            }
        }
    }
    state.hosts.import(selected, group_id).map_err(|e| e.info())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(label: &str, command: &str) -> Vec<(String, String)> {
        vec![(label.to_string(), command.to_string())]
    }

    #[test]
    fn a_command_is_shown_whole_with_hidden_characters_spelled_out() {
        assert_eq!(visible("ssh -W %h:%p bastion"), "ssh -W %h:%p bastion");
        assert_eq!(visible("a\nb\r\tc"), r"a\nb\r\tc");
        assert_eq!(visible("a\u{200b}b\u{202e}c\u{7}d"), r"a\u{200b}b\u{202e}c\u{0007}d");
        let shown = describe_commands(&one("web", "echo hi\nrm -rf x")).unwrap();
        assert!(shown.contains(r"echo hi\nrm -rf x"), "{shown}");
        assert!(!shown.contains("hi\nrm"), "a line break in the command is not a line break in the dialog");
    }

    /// The limits are on what is printed. Control characters print as eight characters each, so a command that is short
    /// as typed can still be far too long to read — and a visible tail after it must not be pushed out of the dialog.
    #[test]
    fn what_cannot_be_shown_whole_is_not_approved() {
        assert!(describe_commands(&one("web", &"x".repeat(MAX_COMMAND_CHARS + 1))).is_none());
        assert!(describe_commands(&one("web", &"x".repeat(MAX_COMMAND_CHARS))).is_some());

        let padded = format!("ssh -W %h:%p bastion{} && calc", "\u{1}".repeat(300));
        assert!(padded.chars().count() < MAX_COMMAND_CHARS, "short as typed");
        assert!(describe_commands(&one("web", &padded)).is_none(), "long as printed");

        // Several commands add up, and there are only so many distinct ones to check.
        let distinct = |n: usize, len: usize| -> Vec<(String, String)> {
            (0..n).map(|i| (format!("h{i}"), format!("{i}{}", "y".repeat(len)))).collect()
        };
        assert!(describe_commands(&distinct(MAX_DISTINCT_COMMANDS, 100)).is_some());
        assert!(describe_commands(&distinct(MAX_DISTINCT_COMMANDS + 1, 10)).is_none());
        assert!(describe_commands(&distinct(3, 700)).is_none(), "three long ones together overflow the dialog");
        assert!(describe_commands(&distinct(2, 700)).is_some());
    }

    #[test]
    fn hosts_that_share_a_command_are_one_thing_to_check() {
        let many: Vec<(String, String)> =
            (0..30).map(|i| (format!("host-{i}"), "ssh -W %h:%p bastion".to_string())).collect();
        let shown = describe_commands(&many).unwrap();
        assert_eq!(shown.matches("ssh -W %h:%p bastion").count(), 1, "{shown}");
        assert!(shown.contains("host-0, host-1, host-2, host-3, host-4 and 25 more"), "{shown}");
    }

    /// Commands that differ are never merged because they print alike.
    #[test]
    fn commands_that_print_alike_but_differ_stay_separate() {
        let both = vec![("a".to_string(), "echo x\ny".to_string()), ("b".to_string(), r"echo x\ny".to_string())];
        let shown = describe_commands(&both).unwrap();
        assert_eq!(shown.matches(r"echo x\ny").count(), 2, "{shown}");
    }

    /// A name chosen by a config file can't write lines of its own into the dialog, or crowd it.
    #[test]
    fn a_hosts_name_cannot_take_over_the_dialog() {
        let hostile = format!("fine\n\nThis command is harmless.{}", "z".repeat(5000));
        let shown = describe_commands(&one(&hostile, "ssh -W %h:%p bastion")).unwrap();
        assert!(shown.chars().count() < 400, "{}", shown.chars().count());
        assert!(shown.contains(r"fine\n\nThis"), "{shown}");
        assert_eq!(
            shown.lines().filter(|l| !l.trim().is_empty()).count(),
            2,
            "one line for the command, one for the name: {shown}"
        );
        assert!(short_label("a\u{202e}b").contains(r"\u{202e}"));
    }

    #[test]
    fn characters_that_look_like_letters_are_flagged() {
        assert!(!describe_commands(&one("web", "ssh -W %h:%p bastion")).unwrap().contains("beyond plain ASCII"));
        let shown = describe_commands(&one("web", "ssh -W %h:%p b\u{430}stion")).unwrap(); // Cyrillic a
        assert!(shown.contains("beyond plain ASCII"), "{shown}");
    }
}
