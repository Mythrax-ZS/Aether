//! OpenSSH-compatible `known_hosts` handling.
//!
//! Aether keeps its own file (plain host names so the UI can list and remove entries) and
//! can additionally *read* the user's `~/.ssh/known_hosts`, including hashed (`|1|salt|hash`)
//! entries, wildcard patterns and `@revoked` markers. It never modifies the user's file.
//!
//! This module works on raw key blobs (`algorithm` + wire-format bytes) so it stays
//! independent of any SSH library.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use hmac::{Hmac, Mac};
use parking_lot::Mutex;
use serde::Serialize;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use ts_rs::TS;

use crate::error::{CoreError, Result};
use crate::util::{glob_match, is_valid_hostname, known_hosts_name};

/// A host key as presented by a server.
#[derive(Debug, Clone, Copy)]
pub struct HostKeyRef<'a> {
    /// `ssh-ed25519`, `ecdsa-sha2-nistp256`, `ssh-rsa`, ...
    pub algorithm: &'a str,
    /// SSH wire-format public key blob (what base64 encodes in a known_hosts line).
    pub blob: &'a [u8],
}

// ── fingerprints ─────────────────────────────────────────────────────────────

/// `SHA256:<base64>` fingerprint as printed by `ssh-keygen -l`.
pub fn fingerprint_sha256(blob: &[u8]) -> String {
    format!("SHA256:{}", STANDARD_NO_PAD.encode(Sha256::digest(blob)))
}

/// Key size for display, derived from the algorithm name (and blob for RSA).
pub fn key_bits(algorithm: &str, blob: &[u8]) -> Option<u32> {
    match algorithm {
        "ssh-ed25519" => Some(256),
        "ecdsa-sha2-nistp256" => Some(256),
        "ecdsa-sha2-nistp384" => Some(384),
        "ecdsa-sha2-nistp521" => Some(521),
        "ssh-rsa" => rsa_modulus_bits(blob),
        _ => None,
    }
}

/// Reads one SSH wire-format `string` (u32 length + bytes) and advances the cursor.
fn read_ssh_string<'a>(buf: &mut &'a [u8]) -> Option<&'a [u8]> {
    let len = u32::from_be_bytes(buf.get(..4)?.try_into().ok()?) as usize;
    let field = buf.get(4..4usize.checked_add(len)?)?;
    *buf = &buf[4 + len..];
    Some(field)
}

fn rsa_modulus_bits(blob: &[u8]) -> Option<u32> {
    // blob = string "ssh-rsa" | mpint e | mpint n
    let mut cursor = blob;
    let _alg = read_ssh_string(&mut cursor)?;
    let _e = read_ssh_string(&mut cursor)?;
    let n = read_ssh_string(&mut cursor)?;
    let n: Vec<u8> = n.iter().copied().skip_while(|b| *b == 0).collect();
    let first = *n.first()?;
    Some((n.len() as u32 - 1) * 8 + (8 - first.leading_zeros()))
}

/// OpenSSH's "drunken bishop" ASCII visualisation of a key fingerprint — easy to compare
/// at a glance in a trust prompt.
pub fn randomart(algorithm: &str, blob: &[u8]) -> String {
    const W: usize = 17;
    const H: usize = 9;
    const SYMBOLS: &[u8] = b" .o+=*BOX@%&#/^SE";
    let mut field = [[0u8; H]; W];
    let (mut x, mut y) = (W / 2, H / 2);
    let start = (x, y);
    let max = (SYMBOLS.len() - 2) as u8;
    for byte in Sha256::digest(blob) {
        let mut input = byte;
        for _ in 0..4 {
            x = if input & 1 != 0 { (x + 1).min(W - 1) } else { x.saturating_sub(1) };
            y = if input & 2 != 0 { (y + 1).min(H - 1) } else { y.saturating_sub(1) };
            if field[x][y] < max {
                field[x][y] += 1;
            }
            input >>= 2;
        }
    }
    field[start.0][start.1] = (SYMBOLS.len() - 2) as u8; // 'S'
    field[x][y] = (SYMBOLS.len() - 1) as u8; // 'E'

    let label = match algorithm {
        "ssh-ed25519" => "ED25519".to_string(),
        "ssh-rsa" => "RSA".to_string(),
        a if a.starts_with("ecdsa-") => "ECDSA".to_string(),
        a => a.to_ascii_uppercase(),
    };
    let title = match key_bits(algorithm, blob) {
        Some(bits) => format!("[{label} {bits}]"),
        None => format!("[{label}]"),
    };
    let banner = |title: &str| {
        let left = W.saturating_sub(title.len()) / 2;
        let right = W.saturating_sub(title.len() + left);
        format!("+{}{}{}+", "-".repeat(left), title, "-".repeat(right))
    };
    let mut out = banner(&title);
    for row in 0..H {
        out.push_str("\n|");
        for col in 0..W {
            out.push(SYMBOLS[field[col][row] as usize] as char);
        }
        out.push('|');
    }
    out.push('\n');
    out.push_str(&banner("[SHA256]"));
    out
}

// ── parsing ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
enum Marker {
    CertAuthority,
    Revoked,
}

#[derive(Debug, Clone)]
enum HostPattern {
    Plain { pattern: String, negated: bool },
    Hashed { salt: Vec<u8>, hash: Vec<u8> },
}

#[derive(Debug, Clone)]
struct Entry {
    marker: Option<Marker>,
    hosts: Vec<HostPattern>,
    raw_hosts: String,
    algorithm: String,
    blob: Vec<u8>,
    line: usize,
}

fn parse_line(line: &str, line_no: usize) -> Option<Entry> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let mut parts = line.split_whitespace();
    let mut first = parts.next()?;
    let marker = match first {
        "@cert-authority" => Some(Marker::CertAuthority),
        "@revoked" => Some(Marker::Revoked),
        _ => None,
    };
    if marker.is_some() {
        first = parts.next()?;
    }
    let algorithm = parts.next()?.to_string();
    let blob = STANDARD.decode(parts.next()?).ok()?;
    let hosts = first
        .split(',')
        .filter_map(|p| {
            if let Some(rest) = p.strip_prefix("|1|") {
                let (salt, hash) = rest.split_once('|')?;
                Some(HostPattern::Hashed { salt: STANDARD.decode(salt).ok()?, hash: STANDARD.decode(hash).ok()? })
            } else {
                let (negated, pattern) = match p.strip_prefix('!') {
                    Some(r) => (true, r),
                    None => (false, p),
                };
                Some(HostPattern::Plain { pattern: pattern.to_string(), negated })
            }
        })
        .collect();
    Some(Entry { marker, hosts, raw_hosts: first.to_string(), algorithm, blob, line: line_no })
}

impl Entry {
    /// Does this entry apply to `name` (already in `host` / `[host]:port` form)?
    fn matches(&self, name: &str) -> bool {
        let mut hit = false;
        for p in &self.hosts {
            match p {
                HostPattern::Plain { pattern, negated } => {
                    if glob_match(pattern, name) {
                        if *negated {
                            return false;
                        }
                        hit = true;
                    }
                }
                HostPattern::Hashed { salt, hash } => {
                    if let Ok(mut mac) = Hmac::<Sha1>::new_from_slice(salt) {
                        mac.update(name.as_bytes());
                        if mac.verify_slice(hash).is_ok() {
                            hit = true;
                        }
                    }
                }
            }
        }
        hit
    }

    fn is_hashed(&self) -> bool {
        self.hosts.iter().any(|h| matches!(h, HostPattern::Hashed { .. }))
    }

    /// Is this line about `name` and nothing else — one host, no wildcards, no negation? Only such a line can be
    /// dropped when that host's key is replaced without taking other hosts' trust with it.
    fn is_only_for(&self, name: &str) -> bool {
        match self.hosts.as_slice() {
            [HostPattern::Hashed { .. }] => self.matches(name),
            [HostPattern::Plain { pattern, negated: false }] => !pattern.contains(['*', '?']) && self.matches(name),
            _ => false,
        }
    }
}

/// Decodes a known_hosts file however an editor or a shell redirect happened to save it: UTF-8 (with or without a
/// BOM), UTF-16 with a BOM (what Windows PowerShell 5.1's `>>` writes), or anything else, where an odd byte costs only
/// that line instead of the whole file.
fn decode_text(bytes: &[u8]) -> String {
    let utf16 = |rest: &[u8], le: bool| {
        let units: Vec<u16> = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| if le { u16::from_le_bytes(pair) } else { u16::from_be_bytes(pair) })
            .collect();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        [0xFF, 0xFE, rest @ ..] => utf16(rest, true),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, false),
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// Entries of one file. A file that isn't there has none. One that is there but can't be read is an error, not an empty
/// file: the trust (and `@revoked`) lines in it aren't being applied, and carrying on as if it were empty would let a
/// changed key look like a first visit.
fn read_entries(path: &Path) -> std::io::Result<Vec<Entry>> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    Ok(decode_text(&bytes).lines().enumerate().filter_map(|(i, l)| parse_line(l, i + 1)).collect())
}

/// The line that records `key` for `host:port`.
fn record_line(host: &str, port: u16, key: &HostKeyRef<'_>) -> String {
    format!("{} {} {}\n", known_hosts_name(&host.to_ascii_lowercase(), port), key.algorithm, STANDARD.encode(key.blob))
}

/// The name becomes the first field of a line that other code reads as patterns: `*` would trust the key for every
/// host, a comma would add more, `|1|` would pass for a hashed entry. Only real host names are written down.
fn ensure_recordable(host: &str) -> Result<()> {
    if is_valid_hostname(host) {
        Ok(())
    } else {
        Err(CoreError::invalid("host", format!("“{host}” isn't a host name or address, so no key is recorded for it")))
    }
}

// ── public API ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum KnownSource {
    /// Aether's own known_hosts.
    Aether,
    /// The user's `~/.ssh/known_hosts` (read-only).
    UserSsh,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct KnownKey {
    pub algorithm: String,
    pub fingerprint: String,
    pub source: KnownSource,
    pub line: u32,
}

/// A known_hosts file that is there but couldn't be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unreadable {
    pub source: KnownSource,
    pub path: PathBuf,
    /// What the operating system said.
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The presented key matches a recorded one.
    Trusted(KnownSource),
    /// Nothing recorded for this host.
    Unknown,
    /// A file of recorded keys couldn't be read, so nothing can be said about this key: not that it is new, and not
    /// that it is the one on record. The caller must not go ahead.
    Unverifiable(Unreadable),
    /// A key of the same type is recorded but differs — possible man-in-the-middle.
    Changed { known: Vec<KnownKey> },
    /// Keys are recorded for this host, but none of the presented key's type. A server that was impersonated could
    /// offer only a type we have no record of, so this is treated like a change, never as a first visit.
    OtherKeyType { known: Vec<KnownKey> },
    /// The key is explicitly revoked.
    Revoked,
}

/// A row in the "Known hosts" manager.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct KnownHostEntry {
    pub hosts: String,
    pub algorithm: String,
    pub fingerprint: String,
    pub source: KnownSource,
    pub line: u32,
    /// Hashed entries can't be displayed or edited by name.
    pub hashed: bool,
}

pub struct KnownHosts {
    primary: PathBuf,
    user_file: Option<PathBuf>,
    user_enabled: AtomicBool,
    write_lock: Mutex<()>,
}

impl KnownHosts {
    pub fn new(primary: impl Into<PathBuf>, user_file: Option<PathBuf>) -> Self {
        Self {
            primary: primary.into(),
            user_enabled: AtomicBool::new(user_file.is_some()),
            user_file,
            write_lock: Mutex::new(()),
        }
    }

    /// Turns reading of the user's `~/.ssh/known_hosts` on or off (it is never written).
    pub fn set_read_user_file(&self, enabled: bool) {
        self.user_enabled.store(enabled, Ordering::Relaxed);
    }

    /// The files in use, Aether's first.
    fn files(&self) -> Vec<(KnownSource, &Path)> {
        let mut v = vec![(KnownSource::Aether, self.primary.as_path())];
        if let (Some(p), true) = (&self.user_file, self.user_enabled.load(Ordering::Relaxed)) {
            v.push((KnownSource::UserSsh, p.as_path()));
        }
        v
    }

    /// Everything recorded — or the first file that couldn't be read. For deciding whether a key is trusted, where a
    /// missing piece of the picture is a risk.
    fn sources(&self) -> std::result::Result<Vec<(KnownSource, Vec<Entry>)>, Unreadable> {
        self.files()
            .into_iter()
            .map(|(source, path)| match read_entries(path) {
                Ok(entries) => Ok((source, entries)),
                Err(e) => Err(Unreadable { source, path: path.to_path_buf(), reason: e.to_string() }),
            })
            .collect()
    }

    /// What can be read, with a warning for what can't. For places where a missing piece is an inconvenience (the
    /// manager's list, the order of key types to ask for) rather than a risk.
    fn readable_sources(&self) -> Vec<(KnownSource, Vec<Entry>)> {
        self.files()
            .into_iter()
            .filter_map(|(source, path)| match read_entries(path) {
                Ok(entries) => Some((source, entries)),
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "known_hosts could not be read; its entries are not being shown");
                    None
                }
            })
            .collect()
    }

    /// Checks `key` against everything recorded for `host:port`.
    pub fn lookup(&self, host: &str, port: u16, key: &HostKeyRef<'_>) -> Verdict {
        let name = known_hosts_name(&host.to_ascii_lowercase(), port);
        let mut trusted: Option<KnownSource> = None;
        let mut changed: Vec<KnownKey> = Vec::new();
        let mut other_types: Vec<KnownKey> = Vec::new();

        let sources = match self.sources() {
            Ok(sources) => sources,
            Err(unreadable) => return Verdict::Unverifiable(unreadable),
        };
        for (source, entries) in sources {
            for e in entries.iter().filter(|e| e.matches(&name)) {
                match e.marker {
                    Some(Marker::Revoked) => {
                        if e.blob == key.blob {
                            return Verdict::Revoked;
                        }
                    }
                    Some(Marker::CertAuthority) => {}
                    None => {
                        let known = || KnownKey {
                            algorithm: e.algorithm.clone(),
                            fingerprint: fingerprint_sha256(&e.blob),
                            source,
                            line: e.line as u32,
                        };
                        if e.algorithm == key.algorithm {
                            if e.blob == key.blob {
                                trusted.get_or_insert(source);
                            } else {
                                changed.push(known());
                            }
                        } else {
                            other_types.push(known());
                        }
                    }
                }
            }
        }
        match (trusted, changed.is_empty(), other_types.is_empty()) {
            (Some(source), _, _) => Verdict::Trusted(source),
            (None, false, _) => Verdict::Changed { known: changed },
            (None, true, false) => Verdict::OtherKeyType { known: other_types },
            (None, true, true) => Verdict::Unknown,
        }
    }

    /// Key algorithms already recorded for this host. The SSH layer prefers these during
    /// negotiation so a server with several host keys doesn't look "changed" just because
    /// it offered a type we haven't recorded.
    pub fn known_algorithms(&self, host: &str, port: u16) -> Vec<String> {
        let name = known_hosts_name(&host.to_ascii_lowercase(), port);
        let mut out: Vec<String> = Vec::new();
        for (_, entries) in self.readable_sources() {
            for e in entries.iter().filter(|e| e.marker.is_none() && e.matches(&name)) {
                if !out.contains(&e.algorithm) {
                    out.push(e.algorithm.clone());
                }
            }
        }
        out
    }

    /// Appends a trusted key to Aether's known_hosts.
    pub fn add(&self, host: &str, port: u16, key: &HostKeyRef<'_>) -> Result<()> {
        ensure_recordable(host)?;
        self.update(|_| false, Some(&record_line(host, port, key))).map(|_| ())
    }

    /// Records `key` for the host in place of the old key *of the same type* (the user accepted a change). Keys of other
    /// types stay, and so does anything that also covers other hosts (a wildcard, a multi-host line): the new key is
    /// added regardless, and a matching entry always wins over a stale one. The old line goes and the new one arrives
    /// in a single write, so a failure can't leave the host with no key on record.
    pub fn replace(&self, host: &str, port: u16, key: &HostKeyRef<'_>) -> Result<()> {
        ensure_recordable(host)?;
        let name = known_hosts_name(&host.to_ascii_lowercase(), port);
        self.update(
            |e| e.marker.is_none() && e.algorithm == key.algorithm && e.is_only_for(&name),
            Some(&record_line(host, port, key)),
        )
        .map(|_| ())
    }

    /// Removes entries for `host:port` from Aether's file. Returns how many lines were removed.
    pub fn remove(&self, host: &str, port: u16) -> Result<usize> {
        let name = known_hosts_name(&host.to_ascii_lowercase(), port);
        self.update(|e| e.matches(&name), None)
    }

    /// Removes the entry on `line` (as reported by [`KnownHosts::list`]) from Aether's file — but only if it still is
    /// the key with that `fingerprint`. The list is a snapshot; lines shift when the file changes, and a stale click
    /// must not delete a different host's key.
    pub fn remove_line(&self, line: u32, fingerprint: &str) -> Result<usize> {
        self.update(|e| e.line == line as usize && fingerprint_sha256(&e.blob) == fingerprint, None)
    }

    /// Changes Aether's file in one atomic write: drops the lines `should_remove` picks, then adds `append`. Everything
    /// else stays byte for byte — comments, spacing, text in some other encoding — except that a UTF-16 file (what a
    /// PowerShell redirect writes) is carried over as UTF-8, which is the only way to keep its entries. Returns how many
    /// lines were dropped.
    fn update(&self, mut should_remove: impl FnMut(&Entry) -> bool, append: Option<&str>) -> Result<usize> {
        let _guard = self.write_lock.lock();
        // Only "there's no file yet" may start from nothing. Any other read error must not turn into overwriting the
        // file with a single line.
        let bytes = match fs::read(&self.primary) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if append.is_none() {
                    return Ok(0);
                }
                Vec::new()
            }
            Err(e) => return Err(CoreError::io(&self.primary, e)),
        };
        let bytes = if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
            decode_text(&bytes).into_bytes()
        } else {
            bytes
        };

        let mut removed = 0;
        let mut out = Vec::with_capacity(bytes.len() + 128);
        for (i, raw) in bytes.split_inclusive(|b| *b == b'\n').enumerate() {
            // Parse the way `decode_text` reads the file, but keep what is kept exactly as it was.
            let text = String::from_utf8_lossy(raw);
            let text = text.trim_end_matches(['\r', '\n']);
            let text = if i == 0 { text.trim_start_matches('\u{feff}') } else { text };
            if parse_line(text, i + 1).is_some_and(|e| should_remove(&e)) {
                removed += 1;
            } else {
                out.extend_from_slice(raw);
            }
        }
        if let Some(line) = append {
            if !out.is_empty() && !out.ends_with(b"\n") {
                out.push(b'\n');
            }
            out.extend_from_slice(line.as_bytes());
        }
        if removed > 0 || append.is_some() {
            if let Some(parent) = self.primary.parent() {
                fs::create_dir_all(parent).map_err(|e| CoreError::io(parent, e))?;
            }
            crate::store::JsonFile::new(&self.primary).write_atomic(&out)?;
        }
        Ok(removed)
    }

    /// Every entry in both files, for the manager UI.
    pub fn list(&self) -> Vec<KnownHostEntry> {
        let mut out = Vec::new();
        for (source, entries) in self.readable_sources() {
            for e in entries.iter().filter(|e| e.marker.is_none()) {
                out.push(KnownHostEntry {
                    hosts: if e.is_hashed() { "(hashed)".to_string() } else { e.raw_hosts.clone() },
                    algorithm: e.algorithm.clone(),
                    fingerprint: fingerprint_sha256(&e.blob),
                    source,
                    line: e.line as u32,
                    hashed: e.is_hashed(),
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A syntactically valid ed25519 wire blob with a recognisable payload.
    fn ed25519_blob(fill: u8) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&11u32.to_be_bytes());
        b.extend_from_slice(b"ssh-ed25519");
        b.extend_from_slice(&32u32.to_be_bytes());
        b.extend_from_slice(&[fill; 32]);
        b
    }

    fn key(blob: &[u8]) -> HostKeyRef<'_> {
        HostKeyRef { algorithm: "ssh-ed25519", blob }
    }

    fn kh(dir: &Path) -> KnownHosts {
        KnownHosts::new(dir.join("known_hosts"), Some(dir.join("user_known_hosts")))
    }

    #[test]
    fn unknown_then_trusted_after_add() {
        let dir = tempfile::tempdir().unwrap();
        let kh = kh(dir.path());
        let blob = ed25519_blob(1);
        assert_eq!(kh.lookup("Example.com", 22, &key(&blob)), Verdict::Unknown);
        kh.add("Example.com", 22, &key(&blob)).unwrap();
        assert_eq!(kh.lookup("example.com", 22, &key(&blob)), Verdict::Trusted(KnownSource::Aether));
        // A different port is a different host identity.
        assert_eq!(kh.lookup("example.com", 2222, &key(&blob)), Verdict::Unknown);
    }

    #[test]
    fn changed_key_is_detected_and_replaceable() {
        let dir = tempfile::tempdir().unwrap();
        let kh = kh(dir.path());
        let (old, new) = (ed25519_blob(1), ed25519_blob(2));
        kh.add("h", 2222, &key(&old)).unwrap();
        match kh.lookup("h", 2222, &key(&new)) {
            Verdict::Changed { known } => {
                assert_eq!(known.len(), 1);
                assert_eq!(known[0].fingerprint, fingerprint_sha256(&old));
            }
            other => panic!("expected Changed, got {other:?}"),
        }
        kh.replace("h", 2222, &key(&new)).unwrap();
        assert_eq!(kh.lookup("h", 2222, &key(&new)), Verdict::Trusted(KnownSource::Aether));
        assert_eq!(kh.list().len(), 1);
    }

    fn ecdsa_blob(fill: u8) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&19u32.to_be_bytes());
        b.extend_from_slice(b"ecdsa-sha2-nistp256");
        b.extend_from_slice(&65u32.to_be_bytes());
        b.extend_from_slice(&[fill; 65]);
        b
    }

    fn ecdsa(blob: &[u8]) -> HostKeyRef<'_> {
        HostKeyRef { algorithm: "ecdsa-sha2-nistp256", blob }
    }

    /// A server that only offers a key type we have no record of, for a host we do know, must not look like a first
    /// visit: that is exactly what an impostor would do to skip the "key changed" warning.
    #[test]
    fn a_key_of_another_type_is_not_a_first_visit() {
        let dir = tempfile::tempdir().unwrap();
        let kh = kh(dir.path());
        let (ed, ec) = (ed25519_blob(1), ecdsa_blob(9));
        kh.add("example.com", 22, &key(&ed)).unwrap();
        match kh.lookup("example.com", 22, &ecdsa(&ec)) {
            Verdict::OtherKeyType { known } => {
                assert_eq!(known.len(), 1);
                assert_eq!(known[0].algorithm, "ssh-ed25519");
                assert_eq!(known[0].fingerprint, fingerprint_sha256(&ed));
            }
            other => panic!("expected OtherKeyType, got {other:?}"),
        }
        assert_eq!(kh.known_algorithms("example.com", 22), vec!["ssh-ed25519"]);

        // A different host, or the same host on another port, has no record at all.
        assert_eq!(kh.lookup("other.example", 22, &ecdsa(&ec)), Verdict::Unknown);
        assert_eq!(kh.lookup("example.com", 2222, &ecdsa(&ec)), Verdict::Unknown);

        // Once the new type is accepted it is recorded next to the old one, and both are trusted.
        kh.replace("example.com", 22, &ecdsa(&ec)).unwrap();
        assert_eq!(kh.lookup("example.com", 22, &ecdsa(&ec)), Verdict::Trusted(KnownSource::Aether));
        assert_eq!(kh.lookup("example.com", 22, &key(&ed)), Verdict::Trusted(KnownSource::Aether), "old type kept");
    }

    /// A same-type mismatch outranks "other types are known too": it is the stronger signal.
    #[test]
    fn a_changed_key_wins_over_other_types() {
        let dir = tempfile::tempdir().unwrap();
        let kh = kh(dir.path());
        let (ed, ec, ec_now) = (ed25519_blob(1), ecdsa_blob(2), ecdsa_blob(3));
        kh.add("h", 22, &key(&ed)).unwrap();
        kh.add("h", 22, &ecdsa(&ec)).unwrap();
        assert!(matches!(kh.lookup("h", 22, &ecdsa(&ec_now)), Verdict::Changed { .. }));
        assert_eq!(kh.lookup("h", 22, &ecdsa(&ec)), Verdict::Trusted(KnownSource::Aether));
    }

    #[test]
    fn replacing_a_key_only_touches_that_hosts_key_of_that_type() {
        let dir = tempfile::tempdir().unwrap();
        let kh = kh(dir.path());
        let (old, new, other) = (ed25519_blob(1), ed25519_blob(2), ed25519_blob(3));
        // One line for the host, a line shared with another host, a wildcard line and a line for someone else.
        let line = |names: &str, blob: &[u8]| format!("{names} ssh-ed25519 {}\n", STANDARD.encode(blob));
        let mut text = String::new();
        text += &line("example.com", &old);
        text += &line("shared-a.example,shared-b.example", &other);
        text += &line("*.wild.example", &other);
        text += &line("unrelated.example", &other);
        fs::write(dir.path().join("known_hosts"), text).unwrap();

        kh.replace("example.com", 22, &key(&new)).unwrap();
        assert_eq!(kh.lookup("example.com", 22, &key(&new)), Verdict::Trusted(KnownSource::Aether));
        assert!(matches!(kh.lookup("example.com", 22, &key(&old)), Verdict::Changed { .. }), "the old key is gone");
        for host in ["shared-a.example", "shared-b.example", "x.wild.example", "unrelated.example"] {
            assert_eq!(
                kh.lookup(host, 22, &key(&other)),
                Verdict::Trusted(KnownSource::Aether),
                "{host} keeps its key"
            );
        }
    }

    #[test]
    fn replacing_a_key_keeps_the_other_algorithms() {
        let dir = tempfile::tempdir().unwrap();
        let kh = kh(dir.path());
        let (ed, ec, ed_new) = (ed25519_blob(1), ecdsa_blob(2), ed25519_blob(3));
        kh.add("h", 22, &key(&ed)).unwrap();
        kh.add("h", 22, &ecdsa(&ec)).unwrap();
        kh.replace("h", 22, &key(&ed_new)).unwrap();
        assert_eq!(kh.lookup("h", 22, &key(&ed_new)), Verdict::Trusted(KnownSource::Aether));
        assert_eq!(kh.lookup("h", 22, &ecdsa(&ec)), Verdict::Trusted(KnownSource::Aether));
        assert_eq!(kh.list().len(), 2);
    }

    #[test]
    fn reads_user_file_with_hashed_and_wildcard_entries() {
        let dir = tempfile::tempdir().unwrap();
        let blob = ed25519_blob(7);
        let b64 = STANDARD.encode(&blob);

        // Build a hashed entry for "secret.example.com" with a known salt.
        let salt = b"0123456789abcdefghij";
        let mut mac = Hmac::<Sha1>::new_from_slice(salt).unwrap();
        mac.update(b"secret.example.com");
        let hash = mac.finalize().into_bytes();
        let hashed = format!("|1|{}|{}", STANDARD.encode(salt), STANDARD.encode(hash));

        let content = format!(
            "# comment\n{hashed} ssh-ed25519 {b64}\n*.lab.example.com,!bad.lab.example.com ssh-ed25519 {b64}\n"
        );
        fs::write(dir.path().join("user_known_hosts"), content).unwrap();
        let kh = kh(dir.path());

        assert_eq!(kh.lookup("secret.example.com", 22, &key(&blob)), Verdict::Trusted(KnownSource::UserSsh));
        assert_eq!(kh.lookup("web.lab.example.com", 22, &key(&blob)), Verdict::Trusted(KnownSource::UserSsh));
        assert_eq!(kh.lookup("bad.lab.example.com", 22, &key(&blob)), Verdict::Unknown);
        assert_eq!(kh.lookup("other.example.com", 22, &key(&blob)), Verdict::Unknown);
        assert!(kh.list().iter().any(|e| e.hashed));
    }

    #[test]
    fn revoked_keys_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let blob = ed25519_blob(9);
        fs::write(dir.path().join("known_hosts"), format!("@revoked h ssh-ed25519 {}\n", STANDARD.encode(&blob)))
            .unwrap();
        assert_eq!(kh(dir.path()).lookup("h", 22, &key(&blob)), Verdict::Revoked);
    }

    #[test]
    fn remove_and_remove_line_only_touch_aether_file() {
        let dir = tempfile::tempdir().unwrap();
        let kh = kh(dir.path());
        let blob = ed25519_blob(3);
        // The user's own file also knows "a"; removing from Aether's list must leave it alone.
        fs::write(dir.path().join("user_known_hosts"), format!("a ssh-ed25519 {}\n", STANDARD.encode(&blob))).unwrap();
        kh.add("a", 22, &key(&blob)).unwrap();
        kh.add("b", 22, &key(&blob)).unwrap();
        assert_eq!(kh.remove("a", 22).unwrap(), 1);
        assert_eq!(kh.lookup("a", 22, &key(&blob)), Verdict::Trusted(KnownSource::UserSsh));
        // "b" moved up to line 1 of Aether's file.
        assert_eq!(kh.remove_line(1, &fingerprint_sha256(&blob)).unwrap(), 1);
        assert_eq!(kh.lookup("b", 22, &key(&blob)), Verdict::Unknown);
        assert!(kh.list().iter().all(|e| e.source == KnownSource::UserSsh));
    }

    #[test]
    fn removing_by_line_checks_that_it_is_still_the_same_key() {
        let dir = tempfile::tempdir().unwrap();
        let kh = kh(dir.path());
        let (a, b) = (ed25519_blob(1), ed25519_blob(2));
        kh.add("a.example", 22, &key(&a)).unwrap();
        kh.add("b.example", 22, &key(&b)).unwrap();
        let row_a = kh.list().into_iter().find(|r| r.hosts == "a.example").unwrap();

        // The file changed under the open dialog: that line now belongs to someone else's key.
        assert_eq!(kh.remove_line(row_a.line, &fingerprint_sha256(&b)).unwrap(), 0, "a stale click removes nothing");
        assert_eq!(kh.lookup("a.example", 22, &key(&a)), Verdict::Trusted(KnownSource::Aether));

        assert_eq!(kh.remove_line(row_a.line, &row_a.fingerprint).unwrap(), 1);
        assert_eq!(kh.lookup("a.example", 22, &key(&a)), Verdict::Unknown);
        assert_eq!(kh.lookup("b.example", 22, &key(&b)), Verdict::Trusted(KnownSource::Aether));
    }

    /// Windows PowerShell 5.1 redirects (`>>`) write UTF-16 and editors save "ANSI"; neither may blank the file out.
    #[test]
    fn files_in_other_encodings_still_count() {
        let dir = tempfile::tempdir().unwrap();
        let kh = kh(dir.path());
        let (trusted, revoked) = (ed25519_blob(4), ed25519_blob(5));

        // UTF-16LE with a BOM, CRLF line ends: a trusted line and a revoked one.
        let text = format!(
            "utf16.example ssh-ed25519 {}\r\n@revoked revoked.example ssh-ed25519 {}\r\n",
            STANDARD.encode(&trusted),
            STANDARD.encode(&revoked)
        );
        let mut bytes = vec![0xFF, 0xFE];
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        fs::write(dir.path().join("user_known_hosts"), &bytes).unwrap();
        assert_eq!(kh.lookup("utf16.example", 22, &key(&trusted)), Verdict::Trusted(KnownSource::UserSsh));
        assert_eq!(kh.lookup("revoked.example", 22, &key(&revoked)), Verdict::Revoked);

        // A Latin-1 comment costs that line, not the whole file.
        let mut latin = b"# caf\xe9\n".to_vec();
        latin.extend_from_slice(format!("plain.example ssh-ed25519 {}\n", STANDARD.encode(&trusted)).as_bytes());
        fs::write(dir.path().join("user_known_hosts"), &latin).unwrap();
        assert_eq!(kh.lookup("plain.example", 22, &key(&trusted)), Verdict::Trusted(KnownSource::UserSsh));

        // A BOM in front of UTF-8 is not part of the first host name.
        let mut bom = vec![0xEF, 0xBB, 0xBF];
        bom.extend_from_slice(format!("first.example ssh-ed25519 {}\n", STANDARD.encode(&trusted)).as_bytes());
        fs::write(dir.path().join("user_known_hosts"), &bom).unwrap();
        assert_eq!(kh.lookup("first.example", 22, &key(&trusted)), Verdict::Trusted(KnownSource::UserSsh));
    }

    /// Aether's own file in UTF-16 (someone appended with PowerShell): adding keeps what was there.
    #[test]
    fn adding_to_a_utf16_file_keeps_the_old_entries() {
        let dir = tempfile::tempdir().unwrap();
        let kh = kh(dir.path());
        let (old, new) = (ed25519_blob(1), ed25519_blob(2));
        let text = format!("old.example ssh-ed25519 {}\r\n", STANDARD.encode(&old));
        let mut bytes = vec![0xFF, 0xFE];
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        fs::write(dir.path().join("known_hosts"), &bytes).unwrap();
        assert_eq!(kh.lookup("old.example", 22, &key(&old)), Verdict::Trusted(KnownSource::Aether));

        kh.add("new.example", 22, &key(&new)).unwrap();
        assert_eq!(kh.lookup("old.example", 22, &key(&old)), Verdict::Trusted(KnownSource::Aether));
        assert_eq!(kh.lookup("new.example", 22, &key(&new)), Verdict::Trusted(KnownSource::Aether));

        // And so does a rewrite.
        assert_eq!(kh.remove("new.example", 22).unwrap(), 1);
        assert_eq!(kh.lookup("old.example", 22, &key(&old)), Verdict::Trusted(KnownSource::Aether));
    }

    /// A host "name" that is really a pattern would make the recorded key count for other machines.
    #[test]
    fn only_real_host_names_are_written_down() {
        let dir = tempfile::tempdir().unwrap();
        let kh = kh(dir.path());
        let blob = ed25519_blob(1);
        for bad in ["*", "*.example.com", "a,b", "a?b", "!a", "|1|salt|hash", "a b", "", "@revoked"] {
            assert!(kh.add(bad, 22, &key(&blob)).is_err(), "{bad:?}");
            assert!(kh.replace(bad, 22, &key(&blob)).is_err(), "{bad:?}");
        }
        assert!(!dir.path().join("known_hosts").exists(), "nothing was written");
        kh.add("fe80::1%eth0", 22, &key(&blob)).unwrap();
        kh.add("münchen.example", 2222, &key(&blob)).unwrap();
        assert_eq!(kh.lookup("münchen.example", 2222, &key(&blob)), Verdict::Trusted(KnownSource::Aether));
    }

    #[test]
    fn a_failed_read_never_becomes_an_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        // The "file" is a directory, so reading it fails with something other than NotFound.
        let path = dir.path().join("known_hosts");
        fs::create_dir(&path).unwrap();
        let kh = KnownHosts::new(&path, None);
        let blob = ed25519_blob(1);
        assert!(kh.add("example.com", 22, &key(&blob)).is_err());
        assert!(kh.replace("example.com", 22, &key(&blob)).is_err());
        assert!(path.is_dir(), "nothing replaced it");
    }

    /// A file that is there but unreadable is not an empty file. Answering "never seen" would turn a changed key into a
    /// first visit (and an `@revoked` line into nothing).
    #[test]
    fn an_unreadable_file_is_not_an_empty_one() {
        let dir = tempfile::tempdir().unwrap();
        let blob = ed25519_blob(1);

        // Aether's own file: a directory stands in for a file that can't be opened.
        fs::create_dir(dir.path().join("known_hosts")).unwrap();
        let kh = kh(dir.path());
        match kh.lookup("example.com", 22, &key(&blob)) {
            Verdict::Unverifiable(u) => {
                assert_eq!((u.source, u.path), (KnownSource::Aether, dir.path().join("known_hosts")))
            }
            other => panic!("expected Unverifiable, got {other:?}"),
        }
        // What only informs a choice — the manager's list, the order of key types — carries on with what it can read.
        assert!(kh.list().is_empty());
        assert!(kh.known_algorithms("example.com", 22).is_empty());

        // The user's file counts too, but only while Aether is told to read it.
        fs::remove_dir(dir.path().join("known_hosts")).unwrap();
        fs::create_dir(dir.path().join("user_known_hosts")).unwrap();
        match kh.lookup("example.com", 22, &key(&blob)) {
            Verdict::Unverifiable(u) => assert_eq!(u.source, KnownSource::UserSsh),
            other => panic!("expected Unverifiable, got {other:?}"),
        }
        kh.set_read_user_file(false);
        assert_eq!(kh.lookup("example.com", 22, &key(&blob)), Verdict::Unknown);

        // A file that simply isn't there yet is a first visit, as ever.
        let fresh = tempfile::tempdir().unwrap();
        assert_eq!(
            super::KnownHosts::new(fresh.path().join("none"), None).lookup("example.com", 22, &key(&blob)),
            Verdict::Unknown
        );
    }

    /// Dropping the old key and writing the new one is a single write, and nothing else in the file is touched: not the
    /// comments, not other hosts' lines, not the line endings, not text in some encoding Aether doesn't know.
    #[test]
    fn rewrites_keep_every_other_line_byte_for_byte() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("known_hosts");
        let (old, new, other) = (ed25519_blob(1), ed25519_blob(2), ed25519_blob(3));
        let line = |host: &str, blob: &[u8]| format!("{host} ssh-ed25519 {}", STANDARD.encode(blob));
        let mut original = Vec::new();
        original.extend_from_slice(b"# caf\xE9 (Windows-1252, not UTF-8)\r\n");
        original.extend_from_slice(format!("{}\r\n", line("other.example", &other)).as_bytes());
        original.extend_from_slice(format!("{}\r\n", line("example.com", &old)).as_bytes());
        original.extend_from_slice(b"\r\n#   odd   spacing\t\r\n");
        fs::write(&path, &original).unwrap();
        let kh = KnownHosts::new(&path, None);

        kh.replace("example.com", 22, &key(&new)).unwrap();
        let mut expected = Vec::new();
        expected.extend_from_slice(b"# caf\xE9 (Windows-1252, not UTF-8)\r\n");
        expected.extend_from_slice(format!("{}\r\n", line("other.example", &other)).as_bytes());
        expected.extend_from_slice(b"\r\n#   odd   spacing\t\r\n");
        expected.extend_from_slice(format!("{}\n", line("example.com", &new)).as_bytes());
        assert_eq!(fs::read(&path).unwrap(), expected);
        assert_eq!(kh.lookup("example.com", 22, &key(&new)), Verdict::Trusted(KnownSource::Aether));
        assert_eq!(
            kh.lookup("example.com", 22, &key(&old)),
            Verdict::Changed { known: kh_known(&kh, "example.com", &new) }
        );

        // Removing a host changes nothing but that host's lines.
        assert_eq!(kh.remove("example.com", 22).unwrap(), 1);
        let mut without = Vec::new();
        without.extend_from_slice(b"# caf\xE9 (Windows-1252, not UTF-8)\r\n");
        without.extend_from_slice(format!("{}\r\n", line("other.example", &other)).as_bytes());
        without.extend_from_slice(b"\r\n#   odd   spacing\t\r\n");
        assert_eq!(fs::read(&path).unwrap(), without);
        // Nothing to remove, nothing written.
        assert_eq!(kh.remove("example.com", 22).unwrap(), 0);
        assert_eq!(fs::read(&path).unwrap(), without);
    }

    /// The line numbers the manager lists are the ones a click removes — also for files saved with a byte-order mark,
    /// Windows line endings, or as UTF-16 (what a PowerShell redirect writes).
    #[test]
    fn listed_lines_and_removed_lines_agree_in_every_encoding() {
        let (a, b, c) = (ed25519_blob(1), ed25519_blob(2), ed25519_blob(3));
        let line = |host: &str, blob: &[u8]| format!("{host} ssh-ed25519 {}", STANDARD.encode(blob));
        let text = format!(
            "# a comment\r\n{}\r\n\r\n{}\r\n{}",
            line("one.example", &a),
            line("two.example", &b),
            line("three.example", &c) // no final line break
        );
        let utf16 = |s: &str, bom: [u8; 2], le: bool| {
            let mut out = bom.to_vec();
            for unit in s.encode_utf16() {
                out.extend_from_slice(&if le { unit.to_le_bytes() } else { unit.to_be_bytes() });
            }
            out
        };
        let mut with_bom = vec![0xEF, 0xBB, 0xBF];
        with_bom.extend_from_slice(text.as_bytes());
        let encodings: Vec<(&str, Vec<u8>)> = vec![
            ("utf-8", text.clone().into_bytes()),
            ("utf-8 with a BOM", with_bom),
            ("utf-16le", utf16(&text, [0xFF, 0xFE], true)),
            ("utf-16be", utf16(&text, [0xFE, 0xFF], false)),
        ];
        for (name, bytes) in encodings {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("known_hosts");
            fs::write(&path, &bytes).unwrap();
            let kh = KnownHosts::new(&path, None);

            let listed = kh.list();
            assert_eq!(listed.len(), 3, "{name}");
            let two = listed.iter().find(|e| e.hosts == "two.example").unwrap();
            // A click on the middle entry removes it, and only it.
            assert_eq!(kh.remove_line(two.line, &two.fingerprint).unwrap(), 1, "{name}");
            let rest: Vec<String> = kh.list().into_iter().map(|e| e.hosts).collect();
            assert_eq!(rest, ["one.example", "three.example"], "{name}");
            // A stale click (the line moved) removes nothing.
            assert_eq!(kh.remove_line(two.line, &two.fingerprint).unwrap(), 0, "{name}");
        }
    }

    fn kh_known(kh: &KnownHosts, host: &str, blob: &[u8]) -> Vec<KnownKey> {
        kh.list()
            .into_iter()
            .filter(|e| e.hosts == host && e.fingerprint == fingerprint_sha256(blob))
            .map(|e| KnownKey { algorithm: e.algorithm, fingerprint: e.fingerprint, source: e.source, line: e.line })
            .collect()
    }

    #[test]
    fn fingerprint_format_matches_ssh_keygen() {
        let fp = fingerprint_sha256(&ed25519_blob(0));
        assert!(fp.starts_with("SHA256:"));
        assert_eq!(fp.len(), "SHA256:".len() + 43, "base64 of 32 bytes without padding");
        assert!(!fp.ends_with('='));
    }

    #[test]
    fn randomart_has_the_classic_shape() {
        let art = randomart("ssh-ed25519", &ed25519_blob(5));
        let lines: Vec<&str> = art.lines().collect();
        assert_eq!(lines.len(), 11, "banner + 9 rows + banner");
        assert_eq!(lines[0], "+--[ED25519 256]--+");
        assert_eq!(lines[10], "+----[SHA256]-----+");
        for row in &lines[1..10] {
            assert_eq!(row.chars().count(), 19);
        }
        assert!(art.contains('S') && art.contains('E'));
        // Deterministic.
        assert_eq!(art, randomart("ssh-ed25519", &ed25519_blob(5)));
    }

    #[test]
    fn rsa_bits_from_blob() {
        // 2048-bit modulus: leading 0x00 pad then 256 bytes with the top bit set.
        let mut blob = Vec::new();
        blob.extend_from_slice(&7u32.to_be_bytes());
        blob.extend_from_slice(b"ssh-rsa");
        blob.extend_from_slice(&3u32.to_be_bytes());
        blob.extend_from_slice(&[1, 0, 1]);
        blob.extend_from_slice(&257u32.to_be_bytes());
        blob.push(0);
        blob.push(0x80);
        blob.extend_from_slice(&[0xAA; 255]);
        assert_eq!(key_bits("ssh-rsa", &blob), Some(2048));
    }
}
