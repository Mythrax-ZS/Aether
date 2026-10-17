//! Port-forwarding rules (`ssh -L`, `-R`, `-D`).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::{CoreError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum ForwardKind {
    /// `-L`: listen locally, connect to `dest` through the SSH server.
    Local,
    /// `-R`: listen on the SSH server, connect to `dest` from this machine.
    Remote,
    /// `-D`: local SOCKS5 proxy that tunnels every connection through the server.
    Dynamic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct ForwardRule {
    /// Stable identifier (a UUID string) so the UI can track a rule across edits.
    pub id: String,
    pub kind: ForwardKind,
    /// Optional human label ("Postgres", "Grafana").
    pub label: String,
    /// Address to listen on. For local/dynamic forwards this is on this machine; for
    /// remote forwards it is on the SSH server.
    pub bind_host: String,
    pub bind_port: u16,
    /// Where connections are sent. Unused for dynamic forwards.
    pub dest_host: String,
    pub dest_port: u16,
    /// Start automatically whenever the host connects.
    pub auto_start: bool,
}

impl Default for ForwardRule {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            kind: ForwardKind::Local,
            label: String::new(),
            bind_host: "127.0.0.1".into(),
            bind_port: 0,
            dest_host: "localhost".into(),
            dest_port: 0,
            auto_start: true,
        }
    }
}

impl ForwardRule {
    pub fn validate(&self) -> Result<()> {
        let bad_host = |h: &str| h.trim().is_empty() || h.contains(char::is_whitespace);
        if self.bind_host != "*" && bad_host(&self.bind_host) {
            return Err(CoreError::invalid("bind address", "must not be empty or contain spaces"));
        }
        if self.kind != ForwardKind::Remote && self.bind_port == 0 {
            return Err(CoreError::invalid("local port", "choose a port between 1 and 65535"));
        }
        if self.kind != ForwardKind::Dynamic {
            if bad_host(&self.dest_host) {
                return Err(CoreError::invalid("destination host", "must not be empty or contain spaces"));
            }
            if self.dest_port == 0 {
                return Err(CoreError::invalid("destination port", "choose a port between 1 and 65535"));
            }
        }
        Ok(())
    }

    /// Human-readable one-liner in `ssh` flag syntax, e.g. `-L 127.0.0.1:5432:db:5432`.
    pub fn describe(&self) -> String {
        match self.kind {
            ForwardKind::Local => {
                format!("-L {}:{}:{}:{}", self.bind_host, self.bind_port, self.dest_host, self.dest_port)
            }
            ForwardKind::Remote => {
                format!("-R {}:{}:{}:{}", self.bind_host, self.bind_port, self.dest_host, self.dest_port)
            }
            ForwardKind::Dynamic => format!("-D {}:{}", self.bind_host, self.bind_port),
        }
    }

    /// Whether this rule exposes a listener beyond the loopback interface.
    pub fn is_publicly_bound(&self) -> bool {
        self.kind != ForwardKind::Remote
            && !matches!(self.bind_host.as_str(), "127.0.0.1" | "localhost" | "::1" | "[::1]")
    }

    /// Parses the argument of `ssh -L/-R` (and `LocalForward`/`RemoteForward` config
    /// lines): `[bind:]port:host:hostport`. Also accepts the two-token config form
    /// `[bind:]port host:hostport`.
    pub fn parse_forward(kind: ForwardKind, spec: &str) -> Result<Self> {
        let spec = spec.trim();
        let err = || CoreError::Parse {
            what: "port forward".into(),
            message: format!("`{spec}` is not in [bind:]port:host:hostport form"),
        };
        // Normalise the two-token form "8080 host:80" into "8080:host:80".
        let normalised = {
            let mut it = spec.split_whitespace();
            match (it.next(), it.next(), it.next()) {
                (Some(a), Some(b), None) => format!("{a}:{b}"),
                (Some(_), None, None) => spec.to_string(),
                _ => return Err(err()),
            }
        };
        let parts = split_forward_fields(&normalised);
        let (bind_host, bind_port, dest_host, dest_port) = match parts.as_slice() {
            [bp, dh, dp] => ("127.0.0.1", bp, dh, dp),
            [bh, bp, dh, dp] => (bh.as_str(), bp, dh, dp),
            _ => return Err(err()),
        };
        let bind_port: u16 = bind_port.parse().map_err(|_| err())?;
        let dest_port: u16 = dest_port.parse().map_err(|_| err())?;
        Ok(ForwardRule {
            kind,
            bind_host: bind_host.to_string(),
            bind_port,
            dest_host: dest_host.clone(),
            dest_port,
            ..Default::default()
        })
    }

    /// Parses `-D [bind:]port` / `DynamicForward` values.
    pub fn parse_dynamic(spec: &str) -> Result<Self> {
        let spec = spec.trim();
        let fail = || CoreError::Parse {
            what: "dynamic forward".into(),
            message: format!("`{spec}` is not in [bind:]port form"),
        };
        let (bind_host, port) = match spec.rsplit_once(':') {
            Some((host, port)) => (host.trim_matches(['[', ']']).to_string(), port),
            None => ("127.0.0.1".to_string(), spec),
        };
        let bind_port: u16 = port.parse().map_err(|_| fail())?;
        Ok(ForwardRule {
            kind: ForwardKind::Dynamic,
            bind_host: if bind_host.is_empty() { "127.0.0.1".into() } else { bind_host },
            bind_port,
            dest_host: String::new(),
            dest_port: 0,
            ..Default::default()
        })
    }
}

/// Splits `a:b:c:d`, keeping bracketed IPv6 literals (`[::1]`) intact.
fn split_forward_fields(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0;
    for ch in s.chars() {
        match ch {
            '[' => {
                depth += 1;
                cur.push(ch);
            }
            ']' => {
                depth -= 1;
                cur.push(ch);
            }
            ':' if depth == 0 => out.push(std::mem::take(&mut cur)),
            _ => cur.push(ch),
        }
    }
    out.push(cur);
    out.into_iter().map(|f| f.trim_matches(['[', ']']).to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_three_and_four_field_forms() {
        let r = ForwardRule::parse_forward(ForwardKind::Local, "8080:example.com:80").unwrap();
        assert_eq!((r.bind_host.as_str(), r.bind_port), ("127.0.0.1", 8080));
        assert_eq!((r.dest_host.as_str(), r.dest_port), ("example.com", 80));

        let r = ForwardRule::parse_forward(ForwardKind::Local, "0.0.0.0:5432:db.internal:5432").unwrap();
        assert_eq!(r.bind_host, "0.0.0.0");
        assert!(r.is_publicly_bound());
    }

    #[test]
    fn parses_ssh_config_two_token_form_and_ipv6() {
        let r = ForwardRule::parse_forward(ForwardKind::Local, "8080 internal:80").unwrap();
        assert_eq!((r.bind_port, r.dest_host.as_str(), r.dest_port), (8080, "internal", 80));

        let r = ForwardRule::parse_forward(ForwardKind::Remote, "[::1]:9000:localhost:3000").unwrap();
        assert_eq!(r.bind_host, "::1");
        assert_eq!(r.dest_port, 3000);
    }

    #[test]
    fn rejects_garbage() {
        assert!(ForwardRule::parse_forward(ForwardKind::Local, "nonsense").is_err());
        assert!(ForwardRule::parse_forward(ForwardKind::Local, "1:2:3:4:5").is_err());
        assert!(ForwardRule::parse_forward(ForwardKind::Local, "99999:host:80").is_err());
    }

    #[test]
    fn dynamic_forwards() {
        let r = ForwardRule::parse_dynamic("1080").unwrap();
        assert_eq!((r.kind, r.bind_port), (ForwardKind::Dynamic, 1080));
        let r = ForwardRule::parse_dynamic("0.0.0.0:1080").unwrap();
        assert_eq!(r.bind_host, "0.0.0.0");
        assert!(r.validate().is_ok());
    }

    #[test]
    fn validation_catches_missing_ports() {
        let mut r = ForwardRule { bind_port: 8080, dest_port: 0, ..Default::default() };
        assert!(r.validate().is_err());
        r.dest_port = 80;
        assert!(r.validate().is_ok());
        r.kind = ForwardKind::Remote;
        r.bind_port = 0; // server-assigned port is fine for remote forwards
        assert!(r.validate().is_ok());
    }

    #[test]
    fn describe_matches_ssh_flags() {
        let r = ForwardRule::parse_forward(ForwardKind::Local, "8080:example.com:80").unwrap();
        assert_eq!(r.describe(), "-L 127.0.0.1:8080:example.com:80");
    }
}
