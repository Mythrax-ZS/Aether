//! russh client configuration: algorithm preferences, windows and keepalives.

use std::borrow::Cow;
use std::sync::Arc;

use russh::keys::Algorithm;
use russh::{Preferred, SshId, cipher, client, compression, kex, mac};

use crate::spec::TransportOptions;

/// Window per channel. Bigger windows keep SFTP and `cat bigfile` fast on high-latency links.
const WINDOW_SIZE: u32 = 4 * 1024 * 1024;
const MAX_PACKET_SIZE: u32 = 32 * 1024;
/// How many messages may queue for one channel before the connection's reader waits for it. The window bounds the
/// *bytes* in flight, not the messages: a server that sends the output of a busy program in small packets can fill a
/// short queue long before the window is used, and while the reader waits, every other channel on the connection
/// (an SFTP transfer, a port forward, the next tab) waits with it. 1024 messages is a few seconds of heavy small-packet
/// output; the memory it can pin is still capped by the window.
const CHANNEL_BUFFER: usize = 1024;

/// The key type a host-key algorithm belongs to (`rsa-sha2-256` → `ssh-rsa`).
fn key_type_name(alg: &Algorithm) -> String {
    match alg {
        Algorithm::Rsa { .. } => "ssh-rsa".to_string(),
        other => other.to_string(),
    }
}

pub(crate) fn preferred(opts: &TransportOptions, known_key_types: &[String]) -> Preferred {
    // Key exchange: the library's safe defaults plus the NIST ECDH curves, which some
    // FIPS-mode servers offer exclusively.
    let mut kex_list: Vec<kex::Name> = Preferred::DEFAULT.kex.to_vec();
    let at = kex_list.iter().position(|k| *k == kex::EXTENSION_SUPPORT_AS_CLIENT).unwrap_or(kex_list.len());
    for (i, name) in [kex::ECDH_SHA2_NISTP256, kex::ECDH_SHA2_NISTP384, kex::ECDH_SHA2_NISTP521].into_iter().enumerate()
    {
        if !kex_list.contains(&name) {
            kex_list.insert(at + i, name);
        }
    }

    let mut ciphers: Vec<cipher::Name> = vec![
        cipher::CHACHA20_POLY1305,
        cipher::AES_256_GCM,
        cipher::AES_128_GCM,
        cipher::AES_256_CTR,
        cipher::AES_192_CTR,
        cipher::AES_128_CTR,
    ];
    let mut macs: Vec<mac::Name> = Preferred::DEFAULT.mac.to_vec();

    if opts.legacy_algorithms {
        kex_list.extend([kex::DH_GEX_SHA1, kex::DH_G14_SHA1, kex::DH_G1_SHA1]);
        ciphers.extend([cipher::AES_256_CBC, cipher::AES_192_CBC, cipher::AES_128_CBC]);
        macs.extend([mac::HMAC_SHA1_ETM, mac::HMAC_SHA1]);
    }

    let compression_list: Vec<compression::Name> = if opts.compression {
        vec![compression::ZLIB_LEGACY, compression::ZLIB, compression::NONE]
    } else {
        vec![compression::NONE]
    };

    // Host key algorithms: prefer types we already hold a key for, so a server that offers
    // several host keys is checked against the one we trust rather than flagged as changed.
    // The sort is stable, so the library's own order (SHA-512 RSA before SHA-256 before
    // SHA-1) is preserved within each group.
    let mut key_algs: Vec<Algorithm> = Preferred::DEFAULT.key.to_vec();
    if !opts.legacy_algorithms {
        // Plain `ssh-rsa` host-key signatures use SHA-1, which OpenSSH stopped accepting by default years ago.
        key_algs.retain(|a| !matches!(a, Algorithm::Rsa { hash: None }));
    }
    if !known_key_types.is_empty() {
        key_algs.sort_by_key(|a| !known_key_types.contains(&key_type_name(a)));
    }

    Preferred {
        kex: Cow::Owned(kex_list),
        key: Cow::Owned(key_algs),
        host_key_certificates: Cow::Borrowed(&[]),
        cipher: Cow::Owned(ciphers),
        mac: Cow::Owned(macs),
        compression: Cow::Owned(compression_list),
    }
}

pub(crate) fn client_config(opts: &TransportOptions, known_key_types: &[String]) -> Arc<client::Config> {
    Arc::new(client::Config {
        client_id: SshId::Standard(Cow::Owned(format!("SSH-2.0-Aether_{}", env!("CARGO_PKG_VERSION")))),
        window_size: WINDOW_SIZE,
        maximum_packet_size: MAX_PACKET_SIZE,
        channel_buffer_size: CHANNEL_BUFFER,
        preferred: preferred(opts, known_key_types),
        keepalive_interval: opts.keepalive_interval,
        keepalive_max: opts.keepalive_max,
        inactivity_timeout: None,
        // We set TCP_NODELAY on the socket ourselves (jump hops ride on channels).
        nodelay: false,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh::keys::HashAlg;

    #[test]
    fn modern_defaults_exclude_sha1_and_cbc() {
        let p = preferred(&TransportOptions::default(), &[]);
        assert!(!p.kex.contains(&kex::DH_G14_SHA1));
        assert!(!p.cipher.contains(&cipher::AES_128_CBC));
        assert!(!p.mac.contains(&mac::HMAC_SHA1));
        assert!(!p.key.contains(&Algorithm::Rsa { hash: None }), "no SHA-1 RSA host-key signatures");
        assert!(p.kex.contains(&kex::CURVE25519));
        assert!(p.kex.contains(&kex::ECDH_SHA2_NISTP256));
        assert_eq!(p.compression.as_ref(), &[compression::NONE]);
    }

    #[test]
    fn legacy_mode_adds_old_algorithms_last() {
        let opts = TransportOptions { legacy_algorithms: true, ..Default::default() };
        let p = preferred(&opts, &[]);
        assert!(p.kex.contains(&kex::DH_G14_SHA1));
        assert!(p.cipher.contains(&cipher::AES_128_CBC));
        assert!(p.mac.contains(&mac::HMAC_SHA1));
        assert!(p.key.contains(&Algorithm::Rsa { hash: None }));
        let ctr = p.cipher.iter().position(|c| *c == cipher::AES_128_CTR).unwrap();
        let cbc = p.cipher.iter().position(|c| *c == cipher::AES_128_CBC).unwrap();
        assert!(ctr < cbc, "modern ciphers still win when both sides support them");
    }

    #[test]
    fn known_host_key_types_are_tried_first() {
        let p = preferred(&TransportOptions::default(), &["ecdsa-sha2-nistp256".to_string()]);
        assert_eq!(p.key[0].to_string(), "ecdsa-sha2-nistp256");

        let p = preferred(&TransportOptions::default(), &["ssh-rsa".to_string()]);
        assert!(matches!(p.key[0], Algorithm::Rsa { hash: Some(HashAlg::Sha512) }));
        assert!(matches!(p.key[1], Algorithm::Rsa { hash: Some(HashAlg::Sha256) }));
    }

    #[test]
    fn compression_is_opt_in() {
        let opts = TransportOptions { compression: true, ..Default::default() };
        assert_eq!(preferred(&opts, &[]).compression[0], compression::ZLIB_LEGACY);
    }
}
