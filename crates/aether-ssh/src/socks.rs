//! A small SOCKS4/4a/5 server handshake (CONNECT only), used by dynamic port forwards.
//!
//! The handshake is generic over any async stream so it can be unit-tested in memory.

use std::net::{Ipv4Addr, Ipv6Addr};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SocksVersion {
    V4,
    V5,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SocksError {
    #[error("client did not speak SOCKS")]
    NotSocks,
    #[error("unsupported SOCKS command {0}")]
    UnsupportedCommand(u8),
    #[error("unsupported address type {0}")]
    UnsupportedAddress(u8),
    #[error("no acceptable authentication method")]
    NoAcceptableAuth,
    #[error("malformed request")]
    Malformed,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// SOCKS5 reply codes (SOCKS4 replies are mapped from these).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Reply {
    Succeeded = 0,
    GeneralFailure = 1,
    NotAllowed = 2,
    #[allow(dead_code)]
    NetworkUnreachable = 3,
    #[allow(dead_code)]
    HostUnreachable = 4,
    ConnectionRefused = 5,
    CommandNotSupported = 7,
    AddressNotSupported = 8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Request {
    pub version: SocksVersion,
    pub host: String,
    pub port: u16,
}

/// Reads a SOCKS4/4a/5 CONNECT request, completing the method negotiation for SOCKS5.
pub(crate) async fn handshake<S>(stream: &mut S) -> Result<Request, SocksError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let version = stream.read_u8().await?;
    match version {
        5 => handshake_v5(stream).await,
        4 => handshake_v4(stream).await,
        _ => Err(SocksError::NotSocks),
    }
}

async fn handshake_v5<S: AsyncRead + AsyncWrite + Unpin>(s: &mut S) -> Result<Request, SocksError> {
    let n_methods = s.read_u8().await? as usize;
    let mut methods = vec![0u8; n_methods];
    s.read_exact(&mut methods).await?;
    if !methods.contains(&0) {
        s.write_all(&[5, 0xFF]).await?;
        return Err(SocksError::NoAcceptableAuth);
    }
    s.write_all(&[5, 0]).await?;

    let mut head = [0u8; 4];
    s.read_exact(&mut head).await?;
    let [ver, cmd, _rsv, atyp] = head;
    if ver != 5 {
        return Err(SocksError::Malformed);
    }
    let host = match atyp {
        1 => {
            let mut b = [0u8; 4];
            s.read_exact(&mut b).await?;
            Ipv4Addr::from(b).to_string()
        }
        3 => {
            let len = s.read_u8().await? as usize;
            let mut b = vec![0u8; len];
            s.read_exact(&mut b).await?;
            String::from_utf8(b).map_err(|_| SocksError::Malformed)?
        }
        4 => {
            let mut b = [0u8; 16];
            s.read_exact(&mut b).await?;
            Ipv6Addr::from(b).to_string()
        }
        other => {
            reply(s, SocksVersion::V5, Reply::AddressNotSupported).await?;
            return Err(SocksError::UnsupportedAddress(other));
        }
    };
    let port = s.read_u16().await?;
    if cmd != 1 {
        reply(s, SocksVersion::V5, Reply::CommandNotSupported).await?;
        return Err(SocksError::UnsupportedCommand(cmd));
    }
    Ok(Request { version: SocksVersion::V5, host, port })
}

async fn handshake_v4<S: AsyncRead + AsyncWrite + Unpin>(s: &mut S) -> Result<Request, SocksError> {
    let cmd = s.read_u8().await?;
    let port = s.read_u16().await?;
    let mut ip = [0u8; 4];
    s.read_exact(&mut ip).await?;
    // NUL-terminated user id (ignored).
    read_cstring(s).await?;
    // SOCKS4a: an address of 0.0.0.x (x != 0) means a host name follows.
    let host = if ip[0] == 0 && ip[1] == 0 && ip[2] == 0 && ip[3] != 0 {
        read_cstring(s).await?
    } else {
        Ipv4Addr::from(ip).to_string()
    };
    if cmd != 1 {
        reply(s, SocksVersion::V4, Reply::CommandNotSupported).await?;
        return Err(SocksError::UnsupportedCommand(cmd));
    }
    Ok(Request { version: SocksVersion::V4, host, port })
}

async fn read_cstring<S: AsyncRead + Unpin>(s: &mut S) -> Result<String, SocksError> {
    let mut out = Vec::new();
    loop {
        let b = s.read_u8().await?;
        if b == 0 {
            break;
        }
        out.push(b);
        if out.len() > 255 {
            return Err(SocksError::Malformed);
        }
    }
    String::from_utf8(out).map_err(|_| SocksError::Malformed)
}

/// Sends the final reply for a request.
pub(crate) async fn reply<S: AsyncWrite + Unpin>(s: &mut S, version: SocksVersion, code: Reply) -> std::io::Result<()> {
    match version {
        SocksVersion::V5 => s.write_all(&[5, code as u8, 0, 1, 0, 0, 0, 0, 0, 0]).await?,
        SocksVersion::V4 => {
            let granted = if code == Reply::Succeeded { 0x5A } else { 0x5B };
            s.write_all(&[0, granted, 0, 0, 0, 0, 0, 0]).await?
        }
    }
    s.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    async fn run(client_bytes: &[u8]) -> (Result<Request, SocksError>, Vec<u8>) {
        let (mut client, mut server) = duplex(1024);
        client.write_all(client_bytes).await.unwrap();
        client.shutdown().await.unwrap();
        let result = handshake(&mut server).await;
        drop(server);
        let mut replies = Vec::new();
        client.read_to_end(&mut replies).await.unwrap();
        (result, replies)
    }

    #[tokio::test]
    async fn socks5_domain_connect() {
        let mut req = vec![5, 1, 0]; // greeting: one method, no-auth
        req.extend_from_slice(&[5, 1, 0, 3, 11]);
        req.extend_from_slice(b"example.com");
        req.extend_from_slice(&443u16.to_be_bytes());
        let (r, replies) = run(&req).await;
        let r = r.unwrap();
        assert_eq!(r, Request { version: SocksVersion::V5, host: "example.com".into(), port: 443 });
        assert_eq!(replies, [5, 0], "method selection reply");
    }

    #[tokio::test]
    async fn socks5_ipv4_and_ipv6() {
        let mut req = vec![5, 1, 0, 5, 1, 0, 1, 10, 0, 0, 7];
        req.extend_from_slice(&22u16.to_be_bytes());
        assert_eq!(run(&req).await.0.unwrap().host, "10.0.0.7");

        let mut req = vec![5, 1, 0, 5, 1, 0, 4];
        req.extend_from_slice(&Ipv6Addr::LOCALHOST.octets());
        req.extend_from_slice(&80u16.to_be_bytes());
        assert_eq!(run(&req).await.0.unwrap().host, "::1");
    }

    #[tokio::test]
    async fn socks5_rejects_auth_only_clients_and_bind() {
        let (r, replies) = run(&[5, 1, 2]).await; // only username/password offered
        assert!(matches!(r, Err(SocksError::NoAcceptableAuth)));
        assert_eq!(replies, [5, 0xFF]);

        // BIND (cmd 2) is not supported.
        let mut req = vec![5, 1, 0, 5, 2, 0, 1, 1, 2, 3, 4];
        req.extend_from_slice(&80u16.to_be_bytes());
        let (r, replies) = run(&req).await;
        assert!(matches!(r, Err(SocksError::UnsupportedCommand(2))));
        assert_eq!(replies[2], 5);
        assert_eq!(replies[3], Reply::CommandNotSupported as u8);
    }

    #[tokio::test]
    async fn socks4_and_4a() {
        let mut req = vec![4, 1];
        req.extend_from_slice(&8080u16.to_be_bytes());
        req.extend_from_slice(&[192, 168, 1, 5]);
        req.extend_from_slice(b"user\0");
        let r = run(&req).await.0.unwrap();
        assert_eq!(r, Request { version: SocksVersion::V4, host: "192.168.1.5".into(), port: 8080 });

        let mut req = vec![4, 1];
        req.extend_from_slice(&80u16.to_be_bytes());
        req.extend_from_slice(&[0, 0, 0, 1]);
        req.extend_from_slice(b"\0internal.host\0");
        let r = run(&req).await.0.unwrap();
        assert_eq!(r.host, "internal.host");
    }

    #[tokio::test]
    async fn garbage_is_not_socks() {
        let (r, _) = run(b"GET / HTTP/1.1\r\n").await;
        assert!(matches!(r, Err(SocksError::NotSocks)));
    }

    #[tokio::test]
    async fn replies_have_the_right_shape() {
        let mut v5 = Vec::new();
        reply(&mut v5, SocksVersion::V5, Reply::ConnectionRefused).await.unwrap();
        assert_eq!(v5, [5, 5, 0, 1, 0, 0, 0, 0, 0, 0]);
        let mut v4 = Vec::new();
        reply(&mut v4, SocksVersion::V4, Reply::Succeeded).await.unwrap();
        assert_eq!(v4, [0, 0x5A, 0, 0, 0, 0, 0, 0]);
    }
}
