//! `~/.ssh/config` parsing and resolution with OpenSSH semantics.
//!
//! * For every option the **first value obtained wins**, in file order, across all `Host` /
//!   `Match` blocks that apply — so specific entries placed above `Host *` override the
//!   defaults below them.
//! * Multi-valued options (`IdentityFile`, `LocalForward`, ...) accumulate.
//! * `Include` is spliced in place (globs supported); `Host`/`Match` blocks inside an
//!   included file end with that file.
//! * `%h %n %p %r %u %d %%` tokens are expanded — except in a `ProxyCommand`, which keeps its tokens so they can be
//!   filled in with the host's address and port at the moment of connecting.
//!
//! Only options Aether understands are interpreted; everything else is ignored silently
//! unless ignoring it could change behaviour, in which case a warning is recorded.

mod import;
mod parse;

use std::path::Path;

use crate::error::{CoreError, Result};
use crate::model::{ForwardKind, ForwardRule};
use crate::util::{self, pattern_list_match};

pub use import::{ImportCandidate, candidates, to_hosts, to_hosts_linked, with_jump_dependencies};
pub use parse::{FsIncludes, IncludeSource, NoIncludes};

#[derive(Debug, Clone)]
struct Opt {
    key: String,
    args: Vec<String>,
    /// The rest of the line as it was written. For the options whose value is a command line (`ProxyCommand`,
    /// `RemoteCommand`) the arguments split at spaces, lose their quotes and get joined again — which changes what
    /// such a command does.
    raw: String,
    line: usize,
}

#[derive(Debug, Clone)]
enum MatchKind {
    All,
    Host(Vec<String>),
    OriginalHost(Vec<String>),
    User(Vec<String>),
    LocalUser(Vec<String>),
    /// `canonical` / `final` — only meaningful with hostname canonicalisation, which we
    /// don't perform, so they never match.
    Never,
    /// `exec` — we deliberately never run commands from a config file.
    Exec,
}

#[derive(Debug, Clone)]
struct MatchTerm {
    negate: bool,
    kind: MatchKind,
}

#[derive(Debug, Clone)]
enum Cond {
    Global,
    Host(Vec<String>),
    Match(Vec<MatchTerm>),
}

#[derive(Debug, Clone)]
struct Section {
    cond: Cond,
    opts: Vec<Opt>,
}

/// A parsed (and `Include`-flattened) ssh_config.
#[derive(Debug, Clone, Default)]
pub struct SshConfig {
    sections: Vec<Section>,
    /// Problems found while parsing (unreadable includes, bad syntax, ...).
    pub warnings: Vec<String>,
}

/// Everything the config says about one target host, ready to turn into a connection.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResolvedHost {
    /// The name that was looked up (the `Host` alias).
    pub alias: String,
    /// `HostName`, or the alias itself.
    pub hostname: String,
    pub port: u16,
    pub user: Option<String>,
    pub identity_files: Vec<String>,
    pub identities_only: bool,
    pub identity_agent_none: bool,
    pub proxy_jump: Vec<String>,
    pub proxy_command: Option<String>,
    pub forward_agent: bool,
    pub forward_x11: bool,
    pub forwards: Vec<ForwardRule>,
    pub server_alive_interval: Option<u32>,
    pub server_alive_count_max: Option<u32>,
    pub connect_timeout: Option<u32>,
    pub compression: bool,
    pub preferred_authentications: Vec<String>,
    pub password_authentication: Option<bool>,
    pub pubkey_authentication: Option<bool>,
    pub kbd_interactive_authentication: Option<bool>,
    pub remote_command: Option<String>,
    pub set_env: Vec<(String, String)>,
    /// Config asks for algorithms OpenSSH has disabled by default (ssh-rsa, SHA-1, CBC...).
    pub legacy_algorithms: bool,
    pub strict_host_key_checking: Option<String>,
    pub user_known_hosts_file: Vec<String>,
    pub warnings: Vec<String>,
}

struct EvalCtx<'a> {
    original: &'a str,
    host: &'a str,
    user: &'a str,
    local_user: &'a str,
}

impl SshConfig {
    /// Parses config text. `includes` supplies the contents of `Include`d files.
    pub fn parse(text: &str, includes: &dyn IncludeSource) -> Self {
        parse::parse(text, includes)
    }

    /// Loads and parses a config file (with includes resolved relative to its folder
    /// unless they are absolute or start with `~`).
    pub fn load_file(path: &Path) -> Result<Self> {
        // A config is a few kilobytes. Something enormous in its place is not one, and reading it whole would only
        // hold the app up.
        const MAX_BYTES: u64 = 4 * 1024 * 1024;
        let size = std::fs::metadata(path).map_err(|e| CoreError::io(path, e))?.len();
        if size > MAX_BYTES {
            return Err(CoreError::invalid(
                "ssh config",
                format!("{} is too large to be an ssh config", path.display()),
            ));
        }
        let text = std::fs::read_to_string(path).map_err(|e| CoreError::io(path, e))?;
        let base = path.parent().map(Path::to_path_buf).unwrap_or_default();
        Ok(parse::parse_named(&text, &FsIncludes::new(base), Some(parse::identity_of(path))))
    }

    /// Loads the user's `~/.ssh/config`. `None` when it doesn't exist.
    pub fn load_user_config() -> Option<Self> {
        let path = crate::paths::AppPaths::user_ssh_dir()?.join("config");
        if !path.is_file() {
            return None;
        }
        Self::load_file(&path).ok()
    }

    /// Concrete (non-wildcard) aliases from `Host` lines, in file order, de-duplicated.
    pub fn concrete_aliases(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for s in &self.sections {
            if let Cond::Host(patterns) = &s.cond {
                for p in patterns {
                    if util::is_literal_pattern(p) && seen.insert(p.to_ascii_lowercase()) {
                        out.push(p.clone());
                    }
                }
            }
        }
        out
    }

    /// Whether any `Host`/`Match` block (other than a bare `Host *`) mentions this name,
    /// i.e. whether the config has something specific to say about it.
    pub fn has_specific_entry(&self, name: &str) -> bool {
        self.sections.iter().any(|s| match &s.cond {
            Cond::Host(p) => !p.iter().all(|x| x == "*") && pattern_list_match(p.iter().map(String::as_str), name),
            _ => false,
        })
    }

    fn applies(&self, cond: &Cond, ctx: &EvalCtx<'_>) -> bool {
        match cond {
            Cond::Global => true,
            Cond::Host(patterns) => pattern_list_match(patterns.iter().map(String::as_str), ctx.original),
            Cond::Match(terms) => terms.iter().all(|t| {
                let hit = match &t.kind {
                    MatchKind::All => true,
                    MatchKind::Host(p) => pattern_list_match(p.iter().map(String::as_str), ctx.host),
                    MatchKind::OriginalHost(p) => pattern_list_match(p.iter().map(String::as_str), ctx.original),
                    MatchKind::User(p) => pattern_list_match(p.iter().map(String::as_str), ctx.user),
                    MatchKind::LocalUser(p) => pattern_list_match(p.iter().map(String::as_str), ctx.local_user),
                    MatchKind::Never | MatchKind::Exec => false,
                };
                hit != t.negate
            }),
        }
    }

    /// Resolves the effective options for `name` (what you would type after `ssh`).
    pub fn resolve(&self, name: &str, local_user: &str) -> ResolvedHost {
        let mut out = ResolvedHost { alias: name.to_string(), ..Default::default() };
        let mut hostname: Option<String> = None;
        let mut port: Option<u16> = None;
        let mut user: Option<String> = None;
        let mut proxy_jump_set = false;
        let mut proxy_command_set = false;
        let mut forward_agent: Option<bool> = None;
        let mut forward_x11: Option<bool> = None;
        let mut compression: Option<bool> = None;
        let mut identities_only: Option<bool> = None;
        let mut remote_command_set = false;
        let mut agent_set = false;
        let mut preferred_set = false;
        let mut strict_set = false;

        for section in &self.sections {
            let ctx = EvalCtx {
                original: name,
                host: hostname.as_deref().unwrap_or(name),
                user: user.as_deref().unwrap_or(local_user),
                local_user,
            };
            if !self.applies(&section.cond, &ctx) {
                continue;
            }
            for opt in &section.opts {
                let first = opt.args.first().map(String::as_str).unwrap_or("");
                let joined = opt.args.join(" ");
                match opt.key.as_str() {
                    "hostname" if hostname.is_none() => hostname = Some(first.to_string()),
                    "port" if port.is_none() => match first.parse::<u16>() {
                        Ok(p) if p > 0 => port = Some(p),
                        _ => out.warnings.push(format!("line {}: invalid Port `{first}`", opt.line)),
                    },
                    "user" if user.is_none() => user = Some(first.to_string()),
                    "identityfile" => out.identity_files.push(joined),
                    "certificatefile" => {}
                    "identitiesonly" if identities_only.is_none() => identities_only = parse_bool(first),
                    "identityagent" if !agent_set => {
                        agent_set = true;
                        out.identity_agent_none = first.eq_ignore_ascii_case("none");
                    }
                    "proxyjump" if !proxy_jump_set => {
                        proxy_jump_set = true;
                        if !first.eq_ignore_ascii_case("none") {
                            out.proxy_jump =
                                joined.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                        }
                    }
                    "proxycommand" if !proxy_command_set => {
                        proxy_command_set = true;
                        // The rest of the line as written — it is a command line, so its quotes and `||` mean what they
                        // say. Its tokens (`%h`, `%p`, …) stay in it: they are filled in when the connection is made,
                        // with the host's address and port as they are then, not as they were when it was imported.
                        if !opt.raw.is_empty() && !opt.raw.eq_ignore_ascii_case("none") {
                            out.proxy_command = Some(opt.raw.clone());
                        }
                    }
                    "forwardagent" if forward_agent.is_none() => forward_agent = parse_bool(first),
                    "forwardx11" if forward_x11.is_none() => forward_x11 = parse_bool(first),
                    "compression" if compression.is_none() => compression = parse_bool(first),
                    "localforward" => push_forward(&mut out, opt, ForwardKind::Local, &joined),
                    "remoteforward" => push_forward(&mut out, opt, ForwardKind::Remote, &joined),
                    "dynamicforward" => match ForwardRule::parse_dynamic(&joined) {
                        Ok(r) => out.forwards.push(r),
                        Err(e) => out.warnings.push(format!("line {}: {e}", opt.line)),
                    },
                    "serveraliveinterval" if out.server_alive_interval.is_none() => {
                        out.server_alive_interval = first.parse().ok()
                    }
                    "serveralivecountmax" if out.server_alive_count_max.is_none() => {
                        out.server_alive_count_max = first.parse().ok()
                    }
                    "connecttimeout" if out.connect_timeout.is_none() => out.connect_timeout = first.parse().ok(),
                    "preferredauthentications" if !preferred_set => {
                        preferred_set = true;
                        out.preferred_authentications = joined
                            .split(',')
                            .map(|s| s.trim().to_ascii_lowercase())
                            .filter(|s| !s.is_empty())
                            .collect();
                    }
                    "passwordauthentication" if out.password_authentication.is_none() => {
                        out.password_authentication = parse_bool(first)
                    }
                    "pubkeyauthentication" if out.pubkey_authentication.is_none() => {
                        out.pubkey_authentication = parse_bool(first)
                    }
                    "kbdinteractiveauthentication" | "challengeresponseauthentication"
                        if out.kbd_interactive_authentication.is_none() =>
                    {
                        out.kbd_interactive_authentication = parse_bool(first)
                    }
                    "remotecommand" if !remote_command_set => {
                        remote_command_set = true;
                        out.remote_command = Some(opt.raw.clone());
                    }
                    "setenv" => {
                        for a in &opt.args {
                            if let Some((k, v)) = a.split_once('=') {
                                out.set_env.push((k.to_string(), v.to_string()));
                            }
                        }
                    }
                    "hostkeyalgorithms"
                    | "pubkeyacceptedalgorithms"
                    | "pubkeyacceptedkeytypes"
                    | "kexalgorithms"
                    | "ciphers"
                    | "macs"
                        if mentions_legacy_algorithm(&joined) =>
                    {
                        out.legacy_algorithms = true
                    }
                    "stricthostkeychecking" if !strict_set => {
                        strict_set = true;
                        out.strict_host_key_checking = Some(first.to_ascii_lowercase());
                    }
                    "userknownhostsfile" => out.user_known_hosts_file.extend(opt.args.iter().cloned()),
                    _ => {}
                }
            }
        }

        out.hostname = hostname.unwrap_or_else(|| name.to_string());
        out.port = port.unwrap_or(22);
        out.user = user;
        out.forward_agent = forward_agent.unwrap_or(false);
        out.forward_x11 = forward_x11.unwrap_or(false);
        out.compression = compression.unwrap_or(false);
        out.identities_only = identities_only.unwrap_or(false);

        // Token expansion happens last because %h/%p/%r depend on the final values.
        let tokens = Tokens {
            host: &out.hostname,
            original: name,
            port: out.port,
            remote_user: out.user.as_deref().unwrap_or(local_user),
            local_user,
        };
        for f in &mut out.identity_files {
            *f = tokens.expand(f);
        }
        for f in &mut out.user_known_hosts_file {
            *f = tokens.expand(f);
        }
        // The ProxyCommand keeps its tokens (see above); a RemoteCommand has nowhere later to have them filled in.
        if let Some(c) = &mut out.remote_command {
            *c = tokens.expand(c);
        }
        out
    }
}

fn push_forward(out: &mut ResolvedHost, opt: &Opt, kind: ForwardKind, spec: &str) {
    match ForwardRule::parse_forward(kind, spec) {
        Ok(r) => out.forwards.push(r),
        Err(e) => out.warnings.push(format!("line {}: {e}", opt.line)),
    }
}

fn parse_bool(s: &str) -> Option<bool> {
    match s.to_ascii_lowercase().as_str() {
        "yes" | "true" => Some(true),
        "no" | "false" => Some(false),
        _ => None,
    }
}

fn mentions_legacy_algorithm(list: &str) -> bool {
    const LEGACY: [&str; 9] = [
        "ssh-rsa",
        "ssh-dss",
        "diffie-hellman-group1-sha1",
        "diffie-hellman-group14-sha1",
        "diffie-hellman-group-exchange-sha1",
        "aes128-cbc",
        "aes256-cbc",
        "3des-cbc",
        "hmac-sha1",
    ];
    list.split(',').map(|a| a.trim().trim_start_matches(['+', '-', '^'])).any(|a| LEGACY.contains(&a))
}

struct Tokens<'a> {
    host: &'a str,
    original: &'a str,
    port: u16,
    remote_user: &'a str,
    local_user: &'a str,
}

impl Tokens<'_> {
    fn expand(&self, s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c != '%' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('%') => out.push('%'),
                Some('h') => out.push_str(self.host),
                Some('n') => out.push_str(self.original),
                Some('p') => out.push_str(&self.port.to_string()),
                Some('r') => out.push_str(self.remote_user),
                Some('u') => out.push_str(self.local_user),
                Some('d') => out.push_str(&util::home_dir().unwrap_or_default().to_string_lossy()),
                Some(other) => {
                    out.push('%');
                    out.push(other);
                }
                None => out.push('%'),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parse::MemoryIncludes;

    fn cfg(text: &str) -> SshConfig {
        SshConfig::parse(text, &MemoryIncludes::default())
    }

    #[test]
    fn first_value_wins_across_blocks() {
        let c = cfg(r#"
Host web
    HostName 10.0.0.5
    User deploy
    Port 2222

Host *
    User fallback
    Port 22
    ServerAliveInterval 45
"#);
        let r = c.resolve("web", "me");
        assert_eq!(r.hostname, "10.0.0.5");
        assert_eq!(r.user.as_deref(), Some("deploy"));
        assert_eq!(r.port, 2222);
        assert_eq!(r.server_alive_interval, Some(45));

        let other = c.resolve("other", "me");
        assert_eq!(other.hostname, "other");
        assert_eq!(other.user.as_deref(), Some("fallback"));
    }

    #[test]
    fn wildcards_and_negation() {
        let c = cfg(r#"
Host *.example.com !secret.example.com
    User corp
Host secret.example.com
    User admin
"#);
        assert_eq!(c.resolve("a.example.com", "me").user.as_deref(), Some("corp"));
        assert_eq!(c.resolve("secret.example.com", "me").user.as_deref(), Some("admin"));
    }

    #[test]
    fn syntax_variants() {
        let c = cfg(
            "Host=eq\n  HostName = 1.2.3.4\n  IdentityFile \"C:\\Users\\me\\.ssh\\my key\"\n  Port 2200 # trailing\n",
        );
        let r = c.resolve("eq", "me");
        assert_eq!(r.hostname, "1.2.3.4");
        assert_eq!(r.port, 2200);
        assert_eq!(r.identity_files, vec![r"C:\Users\me\.ssh\my key"]);
    }

    #[test]
    fn identity_files_and_forwards_accumulate() {
        let c = cfg(r#"
Host db
    IdentityFile ~/.ssh/a
    IdentityFile ~/.ssh/b
    LocalForward 5432 localhost:5432
    RemoteForward 9000 127.0.0.1:3000
    DynamicForward 1080
Host *
    IdentityFile ~/.ssh/default
"#);
        let r = c.resolve("db", "me");
        assert_eq!(r.identity_files, vec!["~/.ssh/a", "~/.ssh/b", "~/.ssh/default"]);
        assert_eq!(r.forwards.len(), 3);
        assert_eq!(r.forwards[0].kind, ForwardKind::Local);
        assert_eq!(r.forwards[0].dest_port, 5432);
        assert_eq!(r.forwards[1].kind, ForwardKind::Remote);
        assert_eq!(r.forwards[2].kind, ForwardKind::Dynamic);
    }

    #[test]
    fn proxy_jump_and_command() {
        let c = cfg(r#"
Host inner
    ProxyJump jump1,me@jump2:2200
Host direct
    ProxyJump none
Host tunnel
    ProxyCommand ssh -W %h:%p bastion
Host *
    ProxyJump should-not-apply
"#);
        assert_eq!(c.resolve("inner", "me").proxy_jump, vec!["jump1", "me@jump2:2200"]);
        assert!(c.resolve("direct", "me").proxy_jump.is_empty(), "`none` disables inheritance");
        let t = c.resolve("tunnel", "me");
        assert_eq!(t.proxy_command.as_deref(), Some("ssh -W %h:%p bastion"), "tokens are left for when it runs");
    }

    /// A ProxyCommand is a command line, taken as the rest of the line: splitting it at spaces and joining it again
    /// would drop the quotes around a path with a space in it, and change what `||` hands to which shell.
    #[test]
    fn a_proxy_command_is_kept_exactly_as_written() {
        let c = cfg(r#"
Host quoted
    ProxyCommand "C:\Program Files\Git\mingw64\bin\connect.exe" -S proxy:1080 %h %p
Host shell
    ProxyCommand sh -c "nc %h %p || ncat %h %p"
Host spaced
    ProxyCommand   ssh   -W %h:%p   bastion   
Host equals
    ProxyCommand = ssh -W %h:%p bastion
Host none
    ProxyCommand none
Host empty
    ProxyCommand
Host *
    ProxyCommand ssh -W %h:%p fallback
"#);
        assert_eq!(
            c.resolve("quoted", "me").proxy_command.as_deref(),
            Some(r#""C:\Program Files\Git\mingw64\bin\connect.exe" -S proxy:1080 %h %p"#)
        );
        assert_eq!(c.resolve("shell", "me").proxy_command.as_deref(), Some(r#"sh -c "nc %h %p || ncat %h %p""#));
        assert_eq!(c.resolve("spaced", "me").proxy_command.as_deref(), Some("ssh   -W %h:%p   bastion"));
        assert_eq!(c.resolve("equals", "me").proxy_command.as_deref(), Some("ssh -W %h:%p bastion"));
        assert_eq!(c.resolve("none", "me").proxy_command, None, "`none` switches the inherited one off");
        assert_eq!(c.resolve("empty", "me").proxy_command, None);
        assert_eq!(c.resolve("other", "me").proxy_command.as_deref(), Some("ssh -W %h:%p fallback"));
    }

    #[test]
    fn a_remote_command_keeps_its_quotes_and_gets_its_tokens() {
        let c = cfg(r#"
Host app
    HostName app.internal
    RemoteCommand sh -c "cd /srv/%h && exec bash -l"
"#);
        assert_eq!(
            c.resolve("app", "me").remote_command.as_deref(),
            Some(r#"sh -c "cd /srv/app.internal && exec bash -l""#)
        );
    }

    #[test]
    fn token_expansion() {
        let c = cfg(r#"
Host app
    HostName app.internal
    User svc
    Port 2022
    IdentityFile ~/.ssh/%r@%h-%p-%n-%%
"#);
        assert_eq!(c.resolve("app", "me").identity_files, vec!["~/.ssh/svc@app.internal-2022-app-%"]);
    }

    #[test]
    fn match_blocks() {
        let c = cfg(r#"
Match host *.corp.example.com
    User corp-user
Match originalhost laptop
    HostName 192.168.1.9
Match all
    ServerAliveInterval 15
Match exec "test -f /tmp/x"
    User never
"#);
        assert_eq!(c.resolve("db.corp.example.com", "me").user.as_deref(), Some("corp-user"));
        assert_eq!(c.resolve("laptop", "me").hostname, "192.168.1.9");
        assert_eq!(c.resolve("anything", "me").server_alive_interval, Some(15));
        assert_ne!(c.resolve("anything", "me").user.as_deref(), Some("never"));
    }

    #[test]
    fn include_splices_and_scopes_blocks() {
        let mut inc = MemoryIncludes::default();
        inc.add("extra.conf", "Host from-include\n  HostName inc.example.com\n");
        let c = SshConfig::parse("Include extra.conf\nHost *\n  User everyone\n", &inc);
        let r = c.resolve("from-include", "me");
        assert_eq!(r.hostname, "inc.example.com");
        assert_eq!(r.user.as_deref(), Some("everyone"), "outer `Host *` still applies");
        assert!(c.concrete_aliases().contains(&"from-include".to_string()));
    }

    #[test]
    fn missing_include_warns_but_does_not_fail() {
        let c = cfg("Include does-not-exist.conf\nHost a\n  User x\n");
        assert_eq!(c.resolve("a", "me").user.as_deref(), Some("x"));
    }

    #[test]
    fn concrete_aliases_skip_patterns() {
        let c = cfg("Host a b *.x !c\n  User u\nHost d\n  User v\nHost a\n  User w\n");
        assert_eq!(c.concrete_aliases(), vec!["a", "b", "d"]);
    }

    #[test]
    fn legacy_algorithm_detection() {
        let c = cfg(
            "Host old\n  HostKeyAlgorithms +ssh-rsa\n  KexAlgorithms +diffie-hellman-group14-sha1\nHost new\n  Ciphers aes256-gcm@openssh.com\n",
        );
        assert!(c.resolve("old", "me").legacy_algorithms);
        assert!(!c.resolve("new", "me").legacy_algorithms);
    }

    #[test]
    fn booleans_and_auth_preferences() {
        let c = cfg(
            "Host h\n  ForwardAgent yes\n  Compression no\n  PreferredAuthentications publickey,password\n  PasswordAuthentication no\n",
        );
        let r = c.resolve("h", "me");
        assert!(r.forward_agent);
        assert!(!r.compression);
        assert_eq!(r.preferred_authentications, vec!["publickey", "password"]);
        assert_eq!(r.password_authentication, Some(false));
    }

    #[test]
    fn has_specific_entry_ignores_catch_all() {
        let c = cfg("Host prod-*\n  User p\nHost *\n  User d\n");
        assert!(c.has_specific_entry("prod-1"));
        assert!(!c.has_specific_entry("staging"));
    }
}
