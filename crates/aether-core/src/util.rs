//! Small shared helpers: time, path expansion and OpenSSH-style wildcard matching.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Milliseconds since the Unix epoch. Timestamps in the data model use this so they are
/// plain numbers on the TypeScript side.
pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// The current user's home directory (`C:\Users\name` on Windows).
pub fn home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}

/// Expands a leading `~` (also `~/` and `~\`) to the user's home directory.
pub fn expand_tilde(path: &str) -> PathBuf {
    let home = || home_dir().unwrap_or_default();
    if path == "~" {
        return home();
    }
    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        return home().join(rest);
    }
    PathBuf::from(path)
}

/// The name of the local OS user, used as the default SSH username.
pub fn local_username() -> String {
    std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default()
}

/// ASCII-case-insensitive glob match supporting `*` and `?`, as used by `ssh_config`
/// `Host` patterns and `known_hosts` host patterns.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p = pattern.as_bytes();
    let t = text.as_bytes();
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut mark = 0usize;

    while ti < t.len() {
        if pi < p.len() && (p[pi] == b'?' || p[pi].eq_ignore_ascii_case(&t[ti])) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }
    pi == p.len()
}

/// OpenSSH `match_pattern_list` semantics: the list matches when at least one positive
/// pattern matches and no negated (`!pattern`) entry matches.
pub fn pattern_list_match<'a, I>(patterns: I, text: &str) -> bool
where
    I: IntoIterator<Item = &'a str>,
{
    let mut matched = false;
    for pat in patterns {
        if let Some(neg) = pat.strip_prefix('!') {
            if glob_match(neg, text) {
                return false;
            }
        } else if glob_match(pat, text) {
            matched = true;
        }
    }
    matched
}

/// Whether a `Host` pattern is a concrete alias rather than a wildcard/negation.
pub fn is_literal_pattern(pattern: &str) -> bool {
    !pattern.contains(['*', '?', '!'])
}

/// `host` for port 22, `[host]:port` otherwise — the form used in `known_hosts` files.
pub fn known_hosts_name(host: &str, port: u16) -> String {
    if port == 22 { host.to_string() } else { format!("[{host}]:{port}") }
}

/// Longest host name the DNS allows.
const MAX_HOSTNAME_LEN: usize = 255;

/// Is `host` something that can only mean a machine — a DNS name or an IP literal — and nothing else?
///
/// Names travel into places where other characters have a meaning of their own: a `known_hosts` line (`*`, `?`, `!`,
/// `,`, `|` and spaces are patterns, separators and the hashed form), a command line, a window title. So what is let
/// through is what real names are made of: letters and digits (any script, for internationalised names), `.`, `-`,
/// `_`, `:` for IPv6 and `%` for its zone id — and not a leading `-`, which a program would read as an option.
pub fn is_valid_hostname(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= MAX_HOSTNAME_LEN
        && !host.starts_with('-')
        && host.chars().all(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '%'))
}

/// The stricter rule for splicing a name into a command line: plain ASCII, no `%` (which `cmd.exe` expands). Anything
/// that merely *looks* like a shell character can become one when Windows converts between code pages.
pub fn is_command_safe_hostname(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= MAX_HOSTNAME_LEN
        && !host.starts_with('-')
        && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_basics() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("web-??", "web-01"));
        assert!(!glob_match("web-??", "web-001"));
        assert!(glob_match("*.example.com", "db.prod.example.com"));
        assert!(!glob_match("*.example.com", "example.com"));
        assert!(glob_match("A*B*C", "aXXbYYc"));
        assert!(glob_match("", ""));
        assert!(!glob_match("", "x"));
    }

    #[test]
    fn pattern_lists_honour_negation() {
        assert!(pattern_list_match(["*.example.com", "!bad.example.com"], "ok.example.com"));
        assert!(!pattern_list_match(["*.example.com", "!bad.example.com"], "bad.example.com"));
        assert!(!pattern_list_match(["!bad.example.com"], "ok.example.com"));
        assert!(!pattern_list_match(["a", "b"], "c"));
    }

    #[test]
    fn known_hosts_names() {
        assert_eq!(known_hosts_name("example.com", 22), "example.com");
        assert_eq!(known_hosts_name("example.com", 2222), "[example.com]:2222");
        assert_eq!(known_hosts_name("::1", 22), "::1");
    }

    #[test]
    fn host_names_are_names_and_nothing_else() {
        for ok in ["example.com", "db-01.prod_internal", "10.0.0.5", "::1", "fe80::1%eth0", "münchen.example", "a"] {
            assert!(is_valid_hostname(ok), "{ok}");
        }
        for bad in [
            "",
            "-oProxyCommand=calc",
            "a b",
            "a\tb",
            "a\nb",
            "*",
            "*.example.com",
            "a?b",
            "!x",
            "a,b",
            "|1|salt|hash",
            "@revoked",
            "host&calc",
            "host;rm",
            "a'b",
            "a\"b",
            "a`b",
            "$(x)",
            "a/b",
            "a\\b",
            "a<b",
            "a>b",
            "a(b)",
            "a^b",
        ] {
            assert!(!is_valid_hostname(bad), "{bad:?}");
        }
        assert!(!is_valid_hostname(&"a".repeat(256)));
        assert!(is_valid_hostname(&"a".repeat(255)));
    }

    #[test]
    fn command_lines_get_only_plain_ascii_names() {
        assert!(is_command_safe_hostname("db-01.example.com"));
        assert!(is_command_safe_hostname("fe80::1"));
        assert!(!is_command_safe_hostname("fe80::1%eth0"), "cmd.exe would expand it");
        assert!(!is_command_safe_hostname("münchen.example"), "punycode (xn--) is what a command line wants");
        assert!(!is_command_safe_hostname("host\u{ff06}calc"), "a fullwidth ampersand is not a host name character");
        assert!(!is_command_safe_hostname("-h"));
        assert!(!is_command_safe_hostname("a&b"));
    }

    #[test]
    fn tilde_expansion() {
        let home = home_dir().unwrap();
        assert_eq!(expand_tilde("~"), home);
        assert_eq!(expand_tilde("~/.ssh/id_ed25519"), home.join(".ssh/id_ed25519"));
        assert_eq!(expand_tilde("C:\\keys\\id"), PathBuf::from("C:\\keys\\id"));
    }
}
