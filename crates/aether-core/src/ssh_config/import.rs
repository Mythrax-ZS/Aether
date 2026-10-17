//! Turning resolved ssh_config entries into Aether [`Host`]s.

use std::collections::{HashMap, HashSet};

use super::{ResolvedHost, SshConfig};
use crate::ids::HostId;
use crate::model::{AuthMethodKind, EnvVar, Host, HostOrigin, JumpRef, KeySource};

/// One importable `Host` entry, ready for the user to review.
#[derive(Debug, Clone)]
pub struct ImportCandidate {
    pub alias: String,
    pub resolved: ResolvedHost,
}

/// Resolves every concrete alias in the config.
pub fn candidates(config: &SshConfig, local_user: &str) -> Vec<ImportCandidate> {
    config
        .concrete_aliases()
        .into_iter()
        .map(|alias| {
            let resolved = config.resolve(&alias, local_user);
            ImportCandidate { alias, resolved }
        })
        .collect()
}

/// Converts candidates into hosts. `ProxyJump` entries that name another imported alias
/// become real host references; everything else stays an ad-hoc `user@host:port` hop.
pub fn to_hosts(candidates: &[ImportCandidate]) -> Vec<Host> {
    to_hosts_linked(candidates, &HashMap::new())
}

/// Like [`to_hosts`], for when some aliases were imported before: `existing` maps such an alias (lower-cased) to the
/// saved host, and a `ProxyJump` through it links to that host instead of to a copy that will never be saved.
pub fn to_hosts_linked(candidates: &[ImportCandidate], existing: &HashMap<String, HostId>) -> Vec<Host> {
    let ids: HashMap<String, HostId> = candidates
        .iter()
        .map(|c| {
            let alias = c.alias.to_ascii_lowercase();
            let id = existing.get(&alias).copied().unwrap_or_else(HostId::new);
            (alias, id)
        })
        .collect();
    candidates
        .iter()
        .map(|c| {
            let id = ids[&c.alias.to_ascii_lowercase()];
            to_host(c, id, &ids)
        })
        .collect()
}

/// The aliases to import for the ones the user picked: those, plus — transitively — every alias they jump through that
/// isn't saved yet. A host imported without its bastion would connect directly, around it. Lower-cased.
pub fn with_jump_dependencies(
    candidates: &[ImportCandidate],
    chosen: &[String],
    saved: &HashMap<String, HostId>,
) -> HashSet<String> {
    let by_alias: HashMap<String, &ImportCandidate> =
        candidates.iter().map(|c| (c.alias.to_ascii_lowercase(), c)).collect();
    let mut wanted: HashSet<String> = chosen.iter().map(|a| a.to_ascii_lowercase()).collect();
    let mut queue: Vec<String> = wanted.iter().cloned().collect();
    while let Some(alias) = queue.pop() {
        let Some(candidate) = by_alias.get(&alias) else { continue };
        for spec in &candidate.resolved.proxy_jump {
            let hop = spec.trim().to_ascii_lowercase();
            let is_alias = !hop.contains(['@', ':', '/']) && by_alias.contains_key(&hop);
            if is_alias && !saved.contains_key(&hop) && wanted.insert(hop.clone()) {
                queue.push(hop);
            }
        }
    }
    wanted
}

fn to_host(c: &ImportCandidate, id: HostId, ids: &HashMap<String, HostId>) -> Host {
    let r = &c.resolved;
    let mut host = Host::new(r.hostname.clone());
    host.id = id;
    host.label = c.alias.clone();
    host.port = r.port;
    host.username = r.user.clone().unwrap_or_default();
    host.origin = HostOrigin::SshConfig { alias: c.alias.clone() };

    host.auth.methods = auth_methods(r);
    host.auth.identities_only = r.identities_only;
    host.auth.key = r.identity_files.first().map(|path| KeySource::File { path: path.clone() });

    host.jump = r.proxy_jump.iter().map(|spec| jump_ref(spec, ids)).collect();

    let o = &mut host.options;
    if let Some(v) = r.server_alive_interval {
        o.keepalive_interval_secs = v;
    }
    if let Some(v) = r.server_alive_count_max {
        o.keepalive_max = v.max(1);
    }
    if let Some(v) = r.connect_timeout {
        o.connect_timeout_secs = v.max(1);
    }
    o.compression = r.compression;
    o.agent_forwarding = r.forward_agent;
    o.x11_forwarding = r.forward_x11;
    o.legacy_algorithms = r.legacy_algorithms;
    if let Some(cmd) = &r.proxy_command {
        o.proxy_command = cmd.clone();
    }

    host.forwards = r.forwards.clone();
    if let Some(cmd) = &r.remote_command {
        host.startup.command = cmd.clone();
    }
    host.startup.env =
        r.set_env.iter().map(|(name, value)| EnvVar { name: name.clone(), value: value.clone() }).collect();
    host
}

fn auth_methods(r: &ResolvedHost) -> Vec<AuthMethodKind> {
    let allowed = |kind: AuthMethodKind| match kind {
        AuthMethodKind::Agent => !r.identity_agent_none,
        AuthMethodKind::PublicKey => r.pubkey_authentication != Some(false),
        AuthMethodKind::KeyboardInteractive => r.kbd_interactive_authentication != Some(false),
        AuthMethodKind::Password => r.password_authentication != Some(false),
    };

    let mut methods = Vec::new();
    if r.preferred_authentications.is_empty() {
        methods.extend(
            [
                AuthMethodKind::Agent,
                AuthMethodKind::PublicKey,
                AuthMethodKind::KeyboardInteractive,
                AuthMethodKind::Password,
            ]
            .into_iter()
            .filter(|m| allowed(*m)),
        );
    } else {
        for name in &r.preferred_authentications {
            match name.as_str() {
                "publickey" => {
                    if allowed(AuthMethodKind::Agent) && !methods.contains(&AuthMethodKind::Agent) {
                        methods.push(AuthMethodKind::Agent);
                    }
                    if allowed(AuthMethodKind::PublicKey) {
                        methods.push(AuthMethodKind::PublicKey);
                    }
                }
                "keyboard-interactive" if allowed(AuthMethodKind::KeyboardInteractive) => {
                    methods.push(AuthMethodKind::KeyboardInteractive)
                }
                "password" if allowed(AuthMethodKind::Password) => methods.push(AuthMethodKind::Password),
                _ => {}
            }
        }
    }
    if methods.is_empty() {
        methods.push(AuthMethodKind::Password);
    }
    methods
}

fn jump_ref(spec: &str, ids: &HashMap<String, HostId>) -> JumpRef {
    let bare = spec.trim();
    if !bare.contains(['@', ':', '/'])
        && let Some(id) = ids.get(&bare.to_ascii_lowercase())
    {
        return JumpRef::Host { host_id: *id };
    }
    JumpRef::Adhoc { spec: bare.trim_start_matches("ssh://").to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh_config::parse::MemoryIncludes;

    fn import(text: &str) -> Vec<Host> {
        let cfg = SshConfig::parse(text, &MemoryIncludes::default());
        to_hosts(&candidates(&cfg, "me"))
    }

    #[test]
    fn maps_core_fields() {
        let hosts = import(
            r#"
Host prod-web
    HostName 10.1.2.3
    User deploy
    Port 2200
    IdentityFile ~/.ssh/prod_ed25519
    IdentitiesOnly yes
    ForwardAgent yes
    ServerAliveInterval 20
    LocalForward 8080 localhost:80
    RemoteCommand tmux attach
    SetEnv FOO=bar
"#,
        );
        assert_eq!(hosts.len(), 1);
        let h = &hosts[0];
        assert_eq!(h.label, "prod-web");
        assert_eq!(h.hostname, "10.1.2.3");
        assert_eq!((h.port, h.username.as_str()), (2200, "deploy"));
        assert_eq!(h.auth.key, Some(KeySource::File { path: "~/.ssh/prod_ed25519".into() }));
        assert!(h.auth.identities_only && h.options.agent_forwarding);
        assert_eq!(h.options.keepalive_interval_secs, 20);
        assert_eq!(h.forwards.len(), 1);
        assert_eq!(h.startup.command, "tmux attach");
        assert_eq!(h.startup.env, vec![EnvVar { name: "FOO".into(), value: "bar".into() }]);
        assert_eq!(h.origin, HostOrigin::SshConfig { alias: "prod-web".into() });
    }

    #[test]
    fn proxy_jump_aliases_become_host_references() {
        let hosts = import(
            r#"
Host bastion
    HostName bastion.example.com
Host inner
    HostName 10.0.0.9
    ProxyJump bastion,ops@other.example.com:2222
"#,
        );
        let bastion = hosts.iter().find(|h| h.label == "bastion").unwrap();
        let inner = hosts.iter().find(|h| h.label == "inner").unwrap();
        assert_eq!(inner.jump[0], JumpRef::Host { host_id: bastion.id });
        assert_eq!(inner.jump[1], JumpRef::Adhoc { spec: "ops@other.example.com:2222".into() });
    }

    #[test]
    fn a_jump_through_an_already_imported_alias_links_to_the_saved_host() {
        let cfg = SshConfig::parse(
            "Host bastion\n  HostName bastion.example.com\nHost inner\n  HostName 10.0.0.9\n  ProxyJump bastion\n",
            &MemoryIncludes::default(),
        );
        let saved_bastion = HostId::new();
        let existing = HashMap::from([("bastion".to_string(), saved_bastion)]);
        let hosts = to_hosts_linked(&candidates(&cfg, "me"), &existing);
        let inner = hosts.iter().find(|h| h.label == "inner").unwrap();
        assert_eq!(inner.jump, [JumpRef::Host { host_id: saved_bastion }]);
        // The candidate for the saved alias carries the saved id, so selecting only `inner` never adds a copy.
        assert_eq!(hosts.iter().find(|h| h.label == "bastion").unwrap().id, saved_bastion);
    }

    fn aliases_to_import(text: &str, chosen: &[&str], saved: &[&str]) -> Vec<String> {
        let cfg = SshConfig::parse(text, &MemoryIncludes::default());
        let saved: HashMap<String, HostId> = saved.iter().map(|a| (a.to_string(), HostId::new())).collect();
        let chosen: Vec<String> = chosen.iter().map(|a| a.to_string()).collect();
        let mut out: Vec<String> =
            with_jump_dependencies(&candidates(&cfg, "me"), &chosen, &saved).into_iter().collect();
        out.sort();
        out
    }

    /// Importing `prod` alone must bring the bastion it hides behind, and the bastion's own jump, and stop at hosts
    /// that are already saved (they are linked to, not imported again) and at loops.
    #[test]
    fn importing_a_host_brings_the_jump_hosts_it_needs() {
        let config = "Host prod\n  HostName 10.0.0.9\n  ProxyJump bastion\n\
                      Host bastion\n  HostName b.example\n  ProxyJump gateway,ops@literal.example:2200\n\
                      Host gateway\n  HostName g.example\n\
                      Host unrelated\n  HostName u.example\n";
        assert_eq!(aliases_to_import(config, &["prod"], &[]), ["bastion", "gateway", "prod"]);
        assert_eq!(aliases_to_import(config, &["gateway"], &[]), ["gateway"], "nothing is pulled in without a reason");
        // The gateway was imported before: it is linked to rather than imported again.
        assert_eq!(aliases_to_import(config, &["prod"], &["gateway"]), ["bastion", "prod"]);
        // Case doesn't matter.
        assert_eq!(aliases_to_import(config, &["PROD"], &[]), ["bastion", "gateway", "prod"]);
        // A loop terminates.
        let looped = "Host a\n  ProxyJump b\nHost b\n  ProxyJump a\n";
        assert_eq!(aliases_to_import(looped, &["a"], &[]), ["a", "b"]);
    }

    #[test]
    fn auth_preferences_are_honoured() {
        let hosts = import(
            r#"
Host pwonly
    PubkeyAuthentication no
    PreferredAuthentications password
Host keyonly
    PasswordAuthentication no
    KbdInteractiveAuthentication no
    IdentityAgent none
Host default
    HostName d
"#,
        );
        let by = |l: &str| hosts.iter().find(|h| h.label == l).unwrap();
        assert_eq!(by("pwonly").auth.methods, vec![AuthMethodKind::Password]);
        assert_eq!(by("keyonly").auth.methods, vec![AuthMethodKind::PublicKey]);
        assert_eq!(by("default").auth.methods.len(), 4);
    }

    #[test]
    fn wildcard_only_configs_import_nothing() {
        assert!(import("Host *\n  User x\n").is_empty());
    }

    #[test]
    fn imported_hosts_validate() {
        for h in import("Host a\n  HostName a.example\nHost b\n  HostName b.example\n  Port 22\n") {
            assert!(h.validate().is_ok());
        }
    }
}
