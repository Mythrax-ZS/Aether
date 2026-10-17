//! Turning saved hosts into connection requests.
//!
//! This is where settings, `~/.ssh/config`, the key registry and jump-host chains are folded
//! into the fully resolved [`ConnectSpec`] the SSH engine consumes.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use aether_core::ErrorInfo;
use aether_core::error::ErrorCode;
use aether_core::model::{AuthMethodKind, Host, HostOrigin, JumpRef, KeySource, Settings};
use aether_core::ssh_config::{self, SshConfig};
use aether_core::store::{HostStore, KeyStore};
use aether_core::util::{expand_tilde, is_valid_hostname, local_username};
use aether_ssh::{AuthSpec, ConnectSpec, KeyRef, ShellParams, TransportOptions};

/// A parsed `[user@]host[:port]` target — plus whatever a pasted `ssh …` command spelled out besides.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Target {
    pub user: Option<String>,
    pub host: String,
    pub port: Option<u16>,
    /// From `-J a,b` or `-o ProxyJump=a,b`: hops to go through, each `[user@]host[:port]` or an alias of the ssh config.
    pub jump: Vec<String>,
    /// From `-i path` or `-o IdentityFile=path`.
    pub identity: Option<String>,
}

/// How deep a chain of jump aliases may go before it is taken for a loop.
const MAX_JUMP_DEPTH: usize = 8;

/// Parses what people type into a quick-connect box: `host`, `user@host`, `user@host:2222`,
/// `[::1]:22`, `ssh://user@host:2222/`, or a whole pasted command such as
/// `ssh -J bastion -i ~/.ssh/work -p 2222 deploy@10.0.0.5`.
pub fn parse_target(input: &str) -> Option<Target> {
    let s = input.trim();
    if let Some((first, rest)) = s.split_once(char::is_whitespace)
        && (first.eq_ignore_ascii_case("ssh") || first.eq_ignore_ascii_case("ssh.exe"))
    {
        return parse_ssh_command(rest);
    }
    parse_destination(s)
}

/// `[user@]host[:port]`, `[v6]:port` or an `ssh://` URL.
fn parse_destination(input: &str) -> Option<Target> {
    let s = input.trim().strip_prefix("ssh://").unwrap_or(input.trim()).trim_end_matches('/');
    if s.is_empty() || s.contains(char::is_whitespace) {
        return None;
    }
    let (user, hostport) = match s.rsplit_once('@') {
        Some((u, h)) if !u.is_empty() => (Some(u.to_string()), h),
        _ => (None, s),
    };
    let (host, port) = if let Some(rest) = hostport.strip_prefix('[') {
        let (h, tail) = rest.split_once(']')?;
        let port = tail.strip_prefix(':').map(|p| p.parse::<u16>()).transpose().ok()?;
        (h.to_string(), port)
    } else if hostport.matches(':').count() == 1 {
        let (h, p) = hostport.split_once(':')?;
        (h.to_string(), Some(p.parse::<u16>().ok()?))
    } else {
        // Bare host, or an unbracketed IPv6 literal.
        (hostport.to_string(), None)
    };
    // Whatever is typed here ends up in a known_hosts file and possibly a command line: it has to be an address.
    if !is_valid_hostname(&host) {
        return None;
    }
    Some(Target { user, host, port, ..Default::default() })
}

/// Words of a command line, with `'…'` and `"…"` keeping spaces together (backslashes are left alone: on Windows they
/// are path separators).
fn split_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for c in line.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => current.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started || !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            (None, c) => current.push(c),
        }
    }
    if started || !current.is_empty() {
        words.push(current);
    }
    words
}

/// The arguments of a pasted `ssh` command: the options that matter here are used, the others are understood well
/// enough not to mistake their values for the destination, and anything after the destination (the remote command) is
/// ignored.
fn parse_ssh_command(args: &str) -> Option<Target> {
    /// Options that take a value — the next word, or glued on as in `-p22`.
    const WITH_VALUE: &str = "BbcDEeFIiJLlmOoPpQRSWw";
    let mut words = split_words(args).into_iter();
    let (mut user, mut port, mut identity) = (None::<String>, None::<u16>, None::<String>);
    let mut jump: Vec<String> = Vec::new();
    let mut destination = None;

    // Like ssh, options may also follow the destination (`ssh host -p 2200`); the first plain word after it starts the
    // remote command.
    while let Some(word) = words.next() {
        if word == "--" {
            if destination.is_none() {
                destination = words.next();
            }
            break;
        }
        let Some(cluster) = word.strip_prefix('-').filter(|c| !c.is_empty()) else {
            if destination.is_none() {
                destination = Some(word);
                continue;
            }
            break;
        };
        for (i, flag) in cluster.char_indices() {
            if !WITH_VALUE.contains(flag) {
                continue; // a plain switch such as -4, -A or -v, possibly several in one word
            }
            let glued = &cluster[i + flag.len_utf8()..];
            let value = if glued.is_empty() { words.next()? } else { glued.to_string() };
            match flag {
                'p' => port = Some(value.trim().parse().ok()?),
                'l' => user = Some(value),
                'i' => identity = Some(value),
                'J' => jump.extend(value.split(',').map(str::trim).filter(|j| !j.is_empty()).map(String::from)),
                'o' => {
                    let (key, val) = value.split_once(['=', ' ']).unwrap_or((value.as_str(), ""));
                    let val = val.trim().trim_matches('"');
                    match key.trim().to_ascii_lowercase().as_str() {
                        "port" => port = Some(val.parse().ok()?),
                        "user" => user = Some(val.to_string()),
                        "identityfile" => identity = Some(val.to_string()),
                        "proxyjump" if !val.eq_ignore_ascii_case("none") => {
                            jump.extend(val.split(',').map(str::trim).filter(|j| !j.is_empty()).map(String::from))
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
            break; // the rest of the word was the value
        }
    }

    let mut target = parse_destination(&destination?)?;
    // The destination's own user and port win over -l and -p, the way `user@host` does in ssh.
    target.user = target.user.or(user.filter(|u| !u.is_empty()));
    target.port = target.port.or(port);
    target.jump = jump;
    target.identity = identity.filter(|i| !i.is_empty());
    Some(target)
}

/// Everything needed to resolve hosts.
pub struct Resolver<'a> {
    pub hosts: &'a HostStore,
    pub keys: &'a KeyStore,
    pub settings: &'a Settings,
    /// Where the user's ssh config comes from: the real `~/.ssh/config`, except in tests.
    pub ssh_config: fn() -> Option<SshConfig>,
}

impl<'a> Resolver<'a> {
    pub fn new(hosts: &'a HostStore, keys: &'a KeyStore, settings: &'a Settings) -> Self {
        Self { hosts, keys, settings, ssh_config: SshConfig::load_user_config }
    }

    /// The user's ssh config, unless they turned reading it off.
    fn user_config(&self) -> Option<SshConfig> {
        self.settings.ssh.use_ssh_config.then(|| (self.ssh_config)()).flatten()
    }

    fn key_ref(&self, source: &KeySource) -> Result<KeyRef, ErrorInfo> {
        match source {
            KeySource::File { path } => Ok(KeyRef::File(expand_tilde(path))),
            KeySource::Vault { key_id } => {
                let info = self.keys.get(*key_id).ok_or_else(|| {
                    ErrorInfo::new(
                        ErrorCode::KeyUnreadable,
                        "Key not found",
                        "The key chosen for this host was deleted from Aether.",
                    )
                    .hint("Edit the host and pick another key.")
                })?;
                Ok(KeyRef::Vault { key_id: key_id.to_string(), label: info.label })
            }
        }
    }

    fn transport(&self, host: &Host, last_hop: bool) -> TransportOptions {
        let ssh = &self.settings.ssh;
        let o = &host.options;
        let interval = if o.keepalive_interval_secs > 0 { o.keepalive_interval_secs } else { 0 };
        TransportOptions {
            connect_timeout: Duration::from_secs(u64::from(o.connect_timeout_secs.max(1))),
            keepalive_interval: (interval > 0).then(|| Duration::from_secs(u64::from(interval))),
            keepalive_max: o.keepalive_max.max(1) as usize,
            compression: o.compression,
            nodelay: ssh.tcp_nodelay,
            legacy_algorithms: o.legacy_algorithms,
            agent_forwarding: last_hop && o.agent_forwarding,
            x11: (last_hop && o.x11_forwarding)
                .then(|| aether_ssh::X11Options { display: ssh.x11_display.trim().to_string(), xauthority: None }),
            host_key_policy: ssh.host_key_policy,
        }
    }

    fn base_spec(&self, host: &Host, last_hop: bool, saved: bool) -> Result<ConnectSpec, ErrorInfo> {
        let ssh = &self.settings.ssh;
        let username = if !host.username.is_empty() {
            host.username.clone()
        } else if !ssh.default_username.is_empty() {
            ssh.default_username.clone()
        } else {
            local_username()
        };
        let key = host.auth.key.as_ref().map(|k| self.key_ref(k)).transpose()?;
        let mut methods = host.auth.methods.clone();
        if ssh.agent == aether_core::model::AgentMode::Off {
            methods.retain(|m| *m != AuthMethodKind::Agent);
        }
        Ok(ConnectSpec {
            label: host.display_name().to_string(),
            host: host.hostname.clone(),
            port: host.port,
            username,
            auth: AuthSpec {
                methods,
                key,
                default_identity_files: ssh.identity_files.iter().map(|p| expand_tilde(p)).collect::<Vec<PathBuf>>(),
                identities_only: host.auth.identities_only,
                agent: ssh.agent,
            },
            options: self.transport(host, last_hop),
            jump: Vec::new(),
            proxy_command: Some(host.options.proxy_command.clone()).filter(|c| !c.trim().is_empty()),
            // Unsaved (quick-connect) hosts have no vault entries to read or write.
            secret_scope: saved.then(|| host.id.to_string()),
        })
    }

    /// The hops for a jump written as `[user@]host[:port]` — or as the name of an entry in the user's ssh config, which
    /// stands for that entry's `HostName`, `User`, `Port`, key and own `ProxyJump`, as `ssh` itself would read it. The
    /// jump's own jumps come first.
    fn adhoc_hops(&self, spec: &str, depth: usize) -> Result<Vec<ConnectSpec>, ErrorInfo> {
        let t = parse_target(spec)
            .ok_or_else(|| ErrorInfo::invalid("jump host", format!("“{spec}” isn't a valid [user@]host[:port]")))?;
        if depth >= MAX_JUMP_DEPTH {
            return Err(ErrorInfo::invalid("jump hosts", "the jump chain is too long, or loops back on itself"));
        }
        let mut host = Host::new(t.host.clone());
        host.username = t.user.clone().unwrap_or_default();
        host.port = t.port.unwrap_or(22);
        let mut hops = Vec::new();

        if let Some(config) = self.user_config().filter(|config| config.has_specific_entry(&t.host)) {
            let r = config.resolve(&t.host, &local_username());
            host.hostname = r.hostname.clone();
            if t.port.is_none() {
                host.port = r.port;
            }
            if t.user.is_none() {
                host.username = r.user.clone().unwrap_or_default();
            }
            host.auth.key = r.identity_files.first().map(|path| KeySource::File { path: path.clone() });
            host.auth.identities_only = r.identities_only;
            host.options.proxy_command = r.proxy_command.clone().unwrap_or_default();
            for next in &r.proxy_jump {
                hops.extend(self.adhoc_hops(next, depth + 1)?);
            }
        }
        hops.push(self.base_spec(&host, false, false)?);
        Ok(hops)
    }

    /// Flattens a host's jump chain (and the chains of its jump hosts) into hop order.
    fn jump_chain(&self, host: &Host, seen: &mut HashSet<aether_core::HostId>) -> Result<Vec<ConnectSpec>, ErrorInfo> {
        let mut hops = Vec::new();
        for j in &host.jump {
            match j {
                JumpRef::Host { host_id } => {
                    if !seen.insert(*host_id) {
                        return Err(ErrorInfo::invalid("jump hosts", "the jump chain loops back on itself"));
                    }
                    let hop = self.hosts.get(*host_id).ok_or_else(|| {
                        ErrorInfo::new(
                            ErrorCode::NotFound,
                            "Jump host missing",
                            "A host used as a jump host was deleted.",
                        )
                        .hint("Edit this host's jump hosts.")
                    })?;
                    // The jump host's own jumps come first.
                    hops.extend(self.jump_chain(&hop, seen)?);
                    hops.push(self.base_spec(&hop, false, true)?);
                }
                JumpRef::Adhoc { spec } => hops.extend(self.adhoc_hops(spec, 0)?),
            }
        }
        Ok(hops)
    }

    /// Builds the connection request for `host`. `saved` says whether it's a stored host.
    pub fn spec_for(&self, host: &Host, saved: bool) -> Result<ConnectSpec, ErrorInfo> {
        let mut spec = self.base_spec(host, true, saved)?;
        let mut seen = HashSet::from([host.id]);
        spec.jump = self.jump_chain(host, &mut seen)?;
        Ok(spec)
    }

    /// Shell parameters (terminal type, environment, startup command) for a host.
    pub fn shell_params(&self, host: &Host, size: aether_ssh::PtySize) -> ShellParams {
        let mut env: Vec<(String, String)> = vec![("COLORTERM".into(), "truecolor".into())];
        for e in &host.startup.env {
            if !e.name.trim().is_empty() {
                env.push((e.name.clone(), e.value.clone()));
            }
        }
        let term = if host.options.term.trim().is_empty() {
            self.settings.ssh.default_term.clone()
        } else {
            host.options.term.clone()
        };
        ShellParams {
            term,
            size,
            env,
            agent_forwarding: host.options.agent_forwarding,
            x11: host.options.x11_forwarding,
            startup_command: Some(host.startup.command.clone()).filter(|c| !c.trim().is_empty()),
        }
    }
}

/// Builds a throwaway host for a quick-connect target, applying a matching `~/.ssh/config`
/// entry when the user has one. Whatever a pasted `ssh` command spelled out (`-J`, `-i`) wins over what is saved or
/// configured, for this connection only.
pub fn host_for_target(target: &Target, settings: &Settings, hosts: &HostStore) -> Host {
    let mut host = base_host_for_target(target, settings, hosts);
    if !target.jump.is_empty() {
        host.jump = target.jump.iter().map(|spec| JumpRef::Adhoc { spec: spec.clone() }).collect();
    }
    if let Some(path) = &target.identity {
        host.auth.key = Some(KeySource::File { path: path.clone() });
    }
    host
}

/// What to remember of a quick connection so that opening it again goes the same way: `user@host:port`, and — when it
/// was opened through jump hosts or with a particular key — the `ssh` command that says so, which quick connect reads
/// back. (Just the destination would send the next one straight to the host, around its bastion.)
pub fn recent_line(host: &Host) -> String {
    // A host that goes through a ProxyCommand from the user's own ssh config can only be reached again by naming its
    // alias. A command line can't carry the command (and must not: pasting one in would be a way to run code without the
    // native confirmation), while `user@address` would skip the config altogether and connect around the proxy.
    if !host.options.proxy_command.trim().is_empty()
        && let HostOrigin::SshConfig { alias } = &host.origin
        && !alias.is_empty()
        && !alias.contains(char::is_whitespace)
    {
        return if host.port == 22 { alias.clone() } else { format!("{alias}:{}", host.port) };
    }
    let jumps: Vec<&str> = host
        .jump
        .iter()
        .filter_map(|j| match j {
            JumpRef::Adhoc { spec } => Some(spec.as_str()),
            JumpRef::Host { .. } => None,
        })
        .collect();
    let key = match &host.auth.key {
        Some(KeySource::File { path }) => Some(path.as_str()),
        _ => None,
    };
    if jumps.is_empty() && key.is_none() {
        return host.target();
    }
    // Words with spaces are quoted the way `split_words` reads them back.
    let word = |w: &str| {
        if !w.is_empty() && !w.contains(char::is_whitespace) && !w.contains(['"', '\'']) {
            w.to_string()
        } else if !w.contains('"') {
            format!("\"{w}\"")
        } else {
            format!("'{w}'")
        }
    };
    let mut line = String::from("ssh");
    if !jumps.is_empty() {
        line.push_str(&format!(" -J {}", word(&jumps.join(","))));
    }
    if let Some(path) = key {
        line.push_str(&format!(" -i {}", word(path)));
    }
    line.push(' ');
    line.push_str(&host.target());
    line
}

fn base_host_for_target(target: &Target, settings: &Settings, hosts: &HostStore) -> Host {
    // An exact saved host wins.
    let user = target.user.clone().unwrap_or_default();
    if let Some(existing) = hosts.find_by_target(&target.host, target.port.unwrap_or(22), &user) {
        return existing;
    }

    if settings.ssh.use_ssh_config
        && target.user.is_none()
        && let Some(config) = SshConfig::load_user_config()
        && config.has_specific_entry(&target.host)
    {
        let resolved = config.resolve(&target.host, &local_username());
        let candidate = ssh_config::ImportCandidate { alias: target.host.clone(), resolved };
        if let Some(mut host) = ssh_config::to_hosts(&[candidate]).into_iter().next() {
            if let Some(port) = target.port {
                host.port = port;
            }
            // Jumps by alias stay ad-hoc: `adhoc_hops` reads them from the config when the connection is made.
            host.jump.retain(|j| matches!(j, JumpRef::Adhoc { .. }));
            return host;
        }
    }

    let mut host = Host::new(target.host.clone());
    host.username = user;
    host.port = target.port.unwrap_or(22);
    host.options.keepalive_interval_secs = settings.ssh.keepalive_interval_secs;
    host.options.keepalive_max = settings.ssh.keepalive_max;
    host.options.connect_timeout_secs = settings.ssh.connect_timeout_secs;
    host.options.auto_reconnect = settings.ssh.auto_reconnect;
    host.options.term = settings.ssh.default_term.clone();
    host.normalize();
    host
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_core::model::{KeyInfo, KeyOrigin};
    use aether_core::{HostId, KeyId};

    fn target(user: Option<&str>, host: &str, port: Option<u16>) -> Target {
        Target { user: user.map(String::from), host: host.into(), port, ..Default::default() }
    }

    /// A quick connection through a bastion is remembered *with* the bastion, in a form quick connect reads back to the
    /// same thing.
    #[test]
    fn a_recent_connection_remembers_how_it_was_reached() {
        let plain = Host { username: "me".into(), port: 2222, ..Host::new("db.example") };
        assert_eq!(recent_line(&plain), "me@db.example:2222", "nothing special: just the destination, as before");

        let dir = tempfile::tempdir().unwrap();
        let (hosts, _) = HostStore::open(dir.path().join("hosts.json"));
        let via = host_for_target(
            &parse_target(r#"ssh -J ops@bastion:2200,edge -i "C:\Users\Jo Smith\.ssh\work" -p 2222 me@db.example"#)
                .unwrap(),
            &Settings::default(),
            &hosts,
        );
        let line = recent_line(&via);
        assert_eq!(line, r#"ssh -J ops@bastion:2200,edge -i "C:\Users\Jo Smith\.ssh\work" me@db.example:2222"#);
        // Opened again from the list, it goes the same way.
        let again = parse_target(&line).unwrap();
        assert_eq!(again.jump, vec!["ops@bastion:2200", "edge"]);
        assert_eq!(again.identity.as_deref(), Some(r"C:\Users\Jo Smith\.ssh\work"));
        assert_eq!((again.user.as_deref(), again.host.as_str(), again.port), (Some("me"), "db.example", Some(2222)));
    }

    /// A host reached through a ProxyCommand from the ssh config is remembered by its alias, which brings the proxy back
    /// when it is opened again — not as a bare address, which would go around it.
    #[test]
    fn a_proxied_config_host_is_remembered_by_its_alias() {
        let mut host = Host::new("10.0.0.5");
        host.username = "deploy".into();
        host.origin = HostOrigin::SshConfig { alias: "prod-db".into() };
        host.options.proxy_command = "ssh -W %h:%p bastion".into();
        assert_eq!(recent_line(&host), "prod-db");
        host.port = 2200;
        let line = recent_line(&host);
        assert_eq!(line, "prod-db:2200");
        let again = parse_target(&line).unwrap();
        assert_eq!((again.user, again.host.as_str(), again.port), (None, "prod-db", Some(2200)));

        // Without a proxy command nothing changes: the address is as good as the alias.
        host.options.proxy_command.clear();
        assert_eq!(recent_line(&host), "deploy@10.0.0.5:2200");
    }

    #[test]
    fn parses_common_target_shapes() {
        let t = |s| parse_target(s).unwrap();
        assert_eq!(t("example.com"), target(None, "example.com", None));
        assert_eq!(t("root@10.0.0.5"), target(Some("root"), "10.0.0.5", None));
        assert_eq!(t("me@host:2222"), target(Some("me"), "host", Some(2222)));
        assert_eq!(t("[::1]:2200"), target(None, "::1", Some(2200)));
        assert_eq!(t("fe80::1"), target(None, "fe80::1", None));
        assert_eq!(t("ssh://deploy@web.internal:2022/"), target(Some("deploy"), "web.internal", Some(2022)));
        assert_eq!(t("ssh me@host -p 2200"), target(Some("me"), "host", Some(2200)));
        assert_eq!(t("  padded.example  "), target(None, "padded.example", None));
        assert_eq!(t("ssh"), target(None, "ssh", None), "a machine that happens to be called ssh");
    }

    #[test]
    fn rejects_unusable_targets() {
        for bad in [
            "",
            "   ",
            "two words",
            "host:notaport",
            "@host",
            "[::1",
            "*.example.com",
            "a,b",
            "host&calc",
            "-oProxyCommand=x",
            "user@",
            "ssh -p 22",
            "ssh -J",
            "ssh -p x host",
            "ssh -- -oddly-named-but-not-a-host",
        ] {
            assert!(parse_target(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn a_pasted_ssh_command_is_understood_option_by_option() {
        let t = |s| parse_target(s).unwrap();

        // The values of options are not mistaken for the destination (this used to pick `key.pem`).
        assert_eq!(
            t("ssh -i key.pem deploy@10.0.0.5"),
            Target { identity: Some("key.pem".into()), ..target(Some("deploy"), "10.0.0.5", None) }
        );
        assert_eq!(t("ssh -l admin -p 2200 box"), target(Some("admin"), "box", Some(2200)));
        assert_eq!(t("ssh -p2200 -ladmin box"), target(Some("admin"), "box", Some(2200)));
        assert_eq!(t("ssh -4A -v box"), target(None, "box", None), "switches can be bundled");
        assert_eq!(t("ssh -vvvp 2200 box").port, Some(2200), "an option with a value may end a bundle");

        // The destination's own user and port outrank -l / -p, like user@host does for ssh.
        assert_eq!(t("ssh -l other -p 1 me@host:2222"), target(Some("me"), "host", Some(2222)));

        // Jumps, from -J or -o, in order.
        assert_eq!(t("ssh -J bastion,ops@second:2200 db").jump, ["bastion", "ops@second:2200"]);
        assert_eq!(t("ssh -o ProxyJump=bastion db").jump, ["bastion"]);
        assert_eq!(t("ssh -oProxyJump=none db").jump, Vec::<String>::new());
        assert_eq!(t("ssh -o Port=2222 -o User=ops db"), target(Some("ops"), "db", Some(2222)));
        assert_eq!(t("ssh -o IdentityFile=~/.ssh/work db").identity.as_deref(), Some("~/.ssh/work"));

        // Quoting keeps a path with spaces together; forwards and other options are skipped over, as is the remote command.
        assert_eq!(
            t(
                r#"ssh -i "C:\Users\me\My Keys\id_ed25519" -L 8080:localhost:80 -o StrictHostKeyChecking=no me@h uptime -a"#
            ),
            Target { identity: Some(r"C:\Users\me\My Keys\id_ed25519".into()), ..target(Some("me"), "h", None) }
        );
        assert_eq!(t("ssh -- me@host"), target(Some("me"), "host", None));
        // Options may follow the destination, as in ssh; once the remote command starts, dashes belong to it.
        assert_eq!(t("ssh me@host -p 2200 -J bastion").jump, ["bastion"]);
        assert_eq!(t("ssh me@host uptime -p 99"), target(Some("me"), "host", None));
        assert_eq!(t("SSH.exe me@host"), target(Some("me"), "host", None));
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        hosts: HostStore,
        keys: KeyStore,
        settings: Settings,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let (hosts, _) = HostStore::open(dir.path().join("hosts.json"));
        let keys = KeyStore::open(dir.path().join("keys.json"));
        Fixture { _dir: dir, hosts, keys, settings: Settings::default() }
    }

    impl Fixture {
        /// A resolver that sees no ssh config at all: tests must not depend on the developer's own `~/.ssh/config`.
        fn resolver(&self) -> Resolver<'_> {
            Resolver { ssh_config: || None, ..Resolver::new(&self.hosts, &self.keys, &self.settings) }
        }

        fn resolver_with(&self, config: fn() -> Option<SshConfig>) -> Resolver<'_> {
            Resolver { ssh_config: config, ..Resolver::new(&self.hosts, &self.keys, &self.settings) }
        }
    }

    fn config_with_bastions() -> Option<SshConfig> {
        Some(SshConfig::parse(
            "Host bastion\n  HostName bastion.corp.example\n  User jump\n  Port 2200\n  IdentityFile ~/.ssh/jump_key\n  ProxyJump gateway\n\
             Host gateway\n  HostName 203.0.113.9\n  User edge\n\
             Host loop-a\n  ProxyJump loop-b\nHost loop-b\n  ProxyJump loop-a\n",
            &aether_core::ssh_config::NoIncludes,
        ))
    }

    #[test]
    fn a_plain_host_resolves_with_defaults() {
        let f = fixture();
        let mut host = Host::new("srv.example.com");
        host.username = "deploy".into();
        host.port = 2222;
        let host = f.hosts.upsert(host).unwrap();
        let spec = f.resolver().spec_for(&host, true).unwrap();
        assert_eq!((spec.host.as_str(), spec.port, spec.username.as_str()), ("srv.example.com", 2222, "deploy"));
        assert_eq!(spec.secret_scope, Some(host.id.to_string()));
        assert!(spec.jump.is_empty());
        assert_eq!(spec.auth.default_identity_files.len(), 3);
        assert_eq!(spec.options.keepalive_interval, Some(Duration::from_secs(30)));
        assert!(spec.proxy_command.is_none());
    }

    #[test]
    fn username_falls_back_to_settings_then_windows() {
        let mut f = fixture();
        let host = Host::new("h");
        assert_eq!(f.resolver().spec_for(&host, false).unwrap().username, local_username());
        f.settings.ssh.default_username = "ops".into();
        assert_eq!(f.resolver().spec_for(&host, false).unwrap().username, "ops");
        assert!(
            f.resolver().spec_for(&host, false).unwrap().secret_scope.is_none(),
            "unsaved hosts have no vault scope"
        );
    }

    #[test]
    fn jump_chains_flatten_in_connection_order() {
        let f = fixture();
        let outer = f.hosts.upsert(Host::new("outer-bastion")).unwrap();
        let mut inner = Host::new("inner-bastion");
        inner.jump = vec![JumpRef::Host { host_id: outer.id }];
        let inner = f.hosts.upsert(inner).unwrap();
        let mut target = Host::new("db");
        target.jump = vec![JumpRef::Host { host_id: inner.id }, JumpRef::Adhoc { spec: "ops@extra:2200".into() }];
        let target = f.hosts.upsert(target).unwrap();

        let spec = f.resolver().spec_for(&target, true).unwrap();
        let order: Vec<&str> = spec.jump.iter().map(|j| j.host.as_str()).collect();
        assert_eq!(order, ["outer-bastion", "inner-bastion", "extra"]);
        assert_eq!(spec.jump[2].port, 2200);
        assert_eq!(spec.jump[2].username, "ops");
        assert!(spec.jump.iter().all(|j| j.jump.is_empty()), "hops are flat");
        assert!(!spec.jump[0].options.agent_forwarding, "agent forwarding only on the final hop");
    }

    #[test]
    fn missing_jump_hosts_and_keys_explain_themselves() {
        let f = fixture();
        let mut host = Host::new("x");
        host.jump = vec![JumpRef::Host { host_id: HostId::new() }];
        let err = f.resolver().spec_for(&host, false).err().unwrap();
        assert_eq!(err.code, ErrorCode::NotFound);

        let mut host = Host::new("x");
        host.auth.key = Some(KeySource::Vault { key_id: KeyId::new() });
        let err = f.resolver().spec_for(&host, false).err().unwrap();
        assert_eq!(err.code, ErrorCode::KeyUnreadable);
    }

    #[test]
    fn vault_keys_resolve_to_their_label() {
        let f = fixture();
        let info = f
            .keys
            .add(KeyInfo {
                id: KeyId::new(),
                label: "Work".into(),
                algorithm: "ssh-ed25519".into(),
                bits: Some(256),
                fingerprint: "SHA256:x".into(),
                public_key: "ssh-ed25519 AAAA".into(),
                encrypted: false,
                origin: KeyOrigin::Generated,
                created_at: 0,
            })
            .unwrap();
        let mut host = Host::new("x");
        host.auth.key = Some(KeySource::Vault { key_id: info.id });
        let spec = f.resolver().spec_for(&host, false).unwrap();
        assert_eq!(spec.auth.key, Some(KeyRef::Vault { key_id: info.id.to_string(), label: "Work".into() }));
    }

    #[test]
    fn disabling_the_agent_removes_it_from_the_method_list() {
        let mut f = fixture();
        f.settings.ssh.agent = aether_core::model::AgentMode::Off;
        let spec = f.resolver().spec_for(&Host::new("h"), false).unwrap();
        assert!(!spec.auth.methods.contains(&AuthMethodKind::Agent));
    }

    #[test]
    fn shell_params_use_host_overrides() {
        let f = fixture();
        let mut host = Host::new("h");
        host.options.term = "screen-256color".into();
        host.startup.command = "tmux attach || tmux".into();
        host.startup.env = vec![aether_core::model::EnvVar { name: "FOO".into(), value: "bar".into() }];
        let p = f.resolver().shell_params(&host, aether_ssh::PtySize::default());
        assert_eq!(p.term, "screen-256color");
        assert_eq!(p.startup_command.as_deref(), Some("tmux attach || tmux"));
        assert!(p.env.contains(&("FOO".to_string(), "bar".to_string())));
        assert!(p.env.iter().any(|(k, _)| k == "COLORTERM"));
    }

    #[test]
    fn x11_follows_the_host_setting_and_only_applies_to_the_final_hop() {
        let mut f = fixture();
        f.settings.ssh.x11_display = "127.0.0.1:3".into();
        let bastion = f.hosts.upsert(Host::new("bastion")).unwrap();

        let mut host = Host::new("workstation");
        host.jump = vec![JumpRef::Host { host_id: bastion.id }];
        assert!(f.resolver().spec_for(&host, false).unwrap().options.x11.is_none(), "off by default");
        assert!(!f.resolver().shell_params(&host, aether_ssh::PtySize::default()).x11);

        host.options.x11_forwarding = true;
        let spec = f.resolver().spec_for(&host, false).unwrap();
        assert_eq!(spec.options.x11.as_ref().map(|x| x.display.as_str()), Some("127.0.0.1:3"));
        assert!(spec.jump[0].options.x11.is_none(), "the bastion never gets X11");
        assert!(f.resolver().shell_params(&host, aether_ssh::PtySize::default()).x11);
    }

    #[test]
    fn quick_connect_prefers_an_exact_saved_host() {
        let f = fixture();
        let mut saved = Host::new("saved.example");
        saved.username = "me".into();
        let saved = f.hosts.upsert(saved).unwrap();
        let t = target(Some("me"), "saved.example", None);
        assert_eq!(host_for_target(&t, &f.settings, &f.hosts).id, saved.id);
        let t = target(Some("someone-else"), "saved.example", None);
        assert_ne!(host_for_target(&t, &f.settings, &f.hosts).id, saved.id);
    }

    /// What a pasted command spelled out applies to this connection — also when the host is one that is saved — without
    /// touching what is saved.
    #[test]
    fn a_pasted_commands_jump_and_key_apply_to_this_connection_only() {
        let f = fixture();
        let mut saved = Host::new("saved.example");
        saved.username = "me".into();
        let saved = f.hosts.upsert(saved).unwrap();

        let t = Target {
            jump: vec!["bastion".into(), "ops@second:2200".into()],
            identity: Some("~/.ssh/work".into()),
            ..target(Some("me"), "saved.example", None)
        };
        let host = host_for_target(&t, &f.settings, &f.hosts);
        assert_eq!(host.id, saved.id, "still the saved host, so its saved password and settings apply");
        assert_eq!(
            host.jump,
            [JumpRef::Adhoc { spec: "bastion".into() }, JumpRef::Adhoc { spec: "ops@second:2200".into() }]
        );
        assert_eq!(host.auth.key, Some(KeySource::File { path: "~/.ssh/work".into() }));
        let stored = f.hosts.get(saved.id).unwrap();
        assert!(stored.jump.is_empty() && stored.auth.key.is_none(), "nothing was saved");

        // For a stranger it builds an ad-hoc host the same way.
        let t = Target { jump: vec!["bastion".into()], ..target(None, "new.example", None) };
        let host = host_for_target(&t, &f.settings, &f.hosts);
        assert_eq!(host.jump, [JumpRef::Adhoc { spec: "bastion".into() }]);

        // And the hops come out in order, ending with the target's own spec.
        let spec = f.resolver().spec_for(&host, false).unwrap();
        assert_eq!(spec.jump.iter().map(|j| j.host.as_str()).collect::<Vec<_>>(), ["bastion"]);
    }

    /// `ProxyJump bastion` in an ssh config means the `Host bastion` entry — its HostName, User, Port, key, and its own
    /// jumps — not a machine that happens to be called "bastion" in DNS.
    #[test]
    fn a_jump_named_like_an_ssh_config_entry_means_that_entry() {
        let mut f = fixture();
        f.settings.ssh.use_ssh_config = true;
        let mut host = Host::new("db.internal");
        host.jump = vec![JumpRef::Adhoc { spec: "bastion".into() }];

        let spec = f.resolver_with(config_with_bastions).spec_for(&host, false).unwrap();
        let hops: Vec<(&str, u16, &str)> =
            spec.jump.iter().map(|j| (j.host.as_str(), j.port, j.username.as_str())).collect();
        // The entry's own ProxyJump (gateway) comes first, then the entry itself.
        assert_eq!(hops, [("203.0.113.9", 22, "edge"), ("bastion.corp.example", 2200, "jump")]);
        assert!(matches!(spec.jump[1].auth.key, Some(KeyRef::File(_))), "its IdentityFile came along");
        assert!(spec.jump.iter().all(|j| j.jump.is_empty()), "hops are flat");

        // What is written out explicitly beats the entry.
        host.jump = vec![JumpRef::Adhoc { spec: "root@bastion:2299".into() }];
        let spec = f.resolver_with(config_with_bastions).spec_for(&host, false).unwrap();
        let last = spec.jump.last().unwrap();
        assert_eq!((last.host.as_str(), last.port, last.username.as_str()), ("bastion.corp.example", 2299, "root"));

        // With ssh config reading turned off it is just a name.
        f.settings.ssh.use_ssh_config = false;
        host.jump = vec![JumpRef::Adhoc { spec: "bastion".into() }];
        let spec = f.resolver_with(config_with_bastions).spec_for(&host, false).unwrap();
        assert_eq!(spec.jump.iter().map(|j| j.host.as_str()).collect::<Vec<_>>(), ["bastion"]);
    }

    #[test]
    fn jump_aliases_that_loop_are_refused_instead_of_followed_forever() {
        let mut f = fixture();
        f.settings.ssh.use_ssh_config = true;
        let mut host = Host::new("target");
        host.jump = vec![JumpRef::Adhoc { spec: "loop-a".into() }];
        let err = f.resolver_with(config_with_bastions).spec_for(&host, false).err().unwrap();
        assert_eq!(err.code, ErrorCode::InvalidInput);

        // And something that isn't an address at all says so.
        host.jump = vec![JumpRef::Adhoc { spec: "not a host".into() }];
        assert_eq!(f.resolver().spec_for(&host, false).err().unwrap().code, ErrorCode::InvalidInput);
    }
}
