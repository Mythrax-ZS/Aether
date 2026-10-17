//! X11 forwarding: lets graphical programs started on the server draw on an X server running
//! on this PC (VcXsrv, X410, Xming, …).
//!
//! How it works, in the same way OpenSSH does it:
//!
//! 1. When the shell is opened we ask the server to set up forwarding and hand it a random
//!    **fake** cookie. The server stores that cookie for the remote `$DISPLAY`.
//! 2. When a remote program connects, the server opens an `x11` channel to us.
//! 3. The first packet an X client sends carries its cookie. It has to be the fake one we issued —
//!    anything else (another user on the server knocking on the forwarded display) is turned away,
//!    whatever the local X server would have said. Only then do we connect to the local X server,
//!    swapping the cookie for the **real** one from `~/.Xauthority` so that it accepts the program,
//!    and relay bytes both ways. (With no cookie file the packet goes on with the fake one, which
//!    is right for X servers that run with access control off.) Until a program has shown the
//!    cookie it costs nothing but a small task: no connection to the X server, and only so many
//!    at a time.
//!
//! The real cookie never leaves this PC.

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use russh::client::{ChannelOpenHandle, Msg};
use russh::{Channel, ChannelOpenFailure};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tracing::{debug, info};

use crate::spec::X11Options;

/// The only authentication protocol Aether speaks.
pub(crate) const PROTOCOL: &str = "MIT-MAGIC-COOKIE-1";

/// Where the local X server listens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalDisplay {
    pub host: String,
    pub display: u16,
    pub screen: u32,
}

impl LocalDisplay {
    /// Parses `host:display[.screen]`. `:0`, `localhost:0`, `127.0.0.1:10.1` and `[::1]:0` all work.
    /// Unix-socket displays can't be reached from Windows and return `None`.
    pub fn parse(spec: &str) -> Option<Self> {
        let spec = spec.trim();
        let (host, rest) = spec.rsplit_once(':')?;
        let host = match host {
            "" | "localhost" => "127.0.0.1".to_string(),
            "unix" => return None,
            other => other.trim_matches(['[', ']']).to_string(),
        };
        if host.contains('/') {
            return None;
        }
        let (display, screen) = rest.split_once('.').unwrap_or((rest, "0"));
        let display: u16 = display.parse().ok()?;
        // X servers listen on TCP port 6000 + display number.
        6000u16.checked_add(display)?;
        Some(Self { host, display, screen: screen.parse().ok()? })
    }

    pub fn address(&self) -> (String, u16) {
        (self.host.clone(), 6000 + self.display)
    }
}

// ── ~/.Xauthority ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XAuthEntry {
    pub family: u16,
    pub address: Vec<u8>,
    /// Display number as text; empty means "any display".
    pub number: String,
    pub name: String,
    pub data: Vec<u8>,
}

/// Parses an `.Xauthority` file: a sequence of big-endian length-prefixed fields.
pub fn parse_xauthority(mut bytes: &[u8]) -> Vec<XAuthEntry> {
    fn u16be(b: &mut &[u8]) -> Option<u16> {
        let (head, rest) = b.split_first_chunk::<2>()?;
        *b = rest;
        Some(u16::from_be_bytes(*head))
    }
    fn field(b: &mut &[u8]) -> Option<Vec<u8>> {
        let n = usize::from(u16be(b)?);
        if b.len() < n {
            return None;
        }
        let (value, rest) = b.split_at(n);
        *b = rest;
        Some(value.to_vec())
    }
    let mut out = Vec::new();
    while !bytes.is_empty() {
        let Some(family) = u16be(&mut bytes) else { break };
        let (Some(address), Some(number), Some(name), Some(data)) =
            (field(&mut bytes), field(&mut bytes), field(&mut bytes), field(&mut bytes))
        else {
            break;
        };
        out.push(XAuthEntry {
            family,
            address,
            number: String::from_utf8_lossy(&number).into_owned(),
            name: String::from_utf8_lossy(&name).into_owned(),
            data,
        });
    }
    out
}

/// The MIT cookie to use for `display`: an exact display number wins over a wildcard entry.
pub fn cookie_for(entries: &[XAuthEntry], display: u16) -> Option<Vec<u8>> {
    let number = display.to_string();
    let mit = || entries.iter().filter(|e| e.name == PROTOCOL);
    mit().find(|e| e.number == number).or_else(|| mit().find(|e| e.number.is_empty())).map(|e| e.data.clone())
}

fn default_xauthority() -> Option<PathBuf> {
    std::env::var_os("XAUTHORITY").map(PathBuf::from).or_else(|| dirs::home_dir().map(|h| h.join(".Xauthority")))
}

// ── the X11 connection-setup packet ──────────────────────────────────────────

const HEADER: usize = 12;

fn pad4(n: usize) -> usize {
    (n + 3) & !3
}

fn field_at(packet: &[u8], offset: usize, big_endian: bool) -> usize {
    let raw = [packet[offset], packet[offset + 1]];
    usize::from(if big_endian { u16::from_be_bytes(raw) } else { u16::from_le_bytes(raw) })
}

/// Total size of the setup request starting with `header` (the first 12 bytes), or `None` when
/// it isn't one (byte 0 must be `B` or `l`, the byte order).
pub fn setup_len(header: &[u8]) -> Option<usize> {
    if header.len() < HEADER {
        return None;
    }
    let big = match header[0] {
        b'B' => true,
        b'l' => false,
        _ => return None,
    };
    Some(HEADER + pad4(field_at(header, 6, big)) + pad4(field_at(header, 8, big)))
}

/// Compares secrets without stopping at the first difference.
fn same_bytes(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |diff, (x, y)| diff | (x ^ y)) == 0
}

/// Why a client's connection request was turned away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupRejected {
    /// Not an X11 connection request at all.
    NotX11,
    /// It doesn't carry the cookie issued for this forwarding.
    WrongCookie,
}

/// Lets a remote client through only if it holds the cookie *we* issued for this session, and then puts the real
/// cookie (when there is one) in its place.
///
/// The check matters because the remote end of X11 forwarding is a socket every user on the server can reach. The cookie
/// is what tells the owner of this session from anyone else on that machine; the local X server is often run with access
/// control off, so it would never notice the difference.
pub fn authorize_setup(packet: &mut [u8], fake: &[u8], real: Option<&[u8]>) -> Result<(), SetupRejected> {
    let total = setup_len(packet).ok_or(SetupRejected::NotX11)?;
    if packet.len() < total {
        return Err(SetupRejected::NotX11);
    }
    let big = packet[0] == b'B';
    let name_len = field_at(packet, 6, big);
    let data_len = field_at(packet, 8, big);
    let data_at = HEADER + pad4(name_len);
    if &packet[HEADER..HEADER + name_len] != PROTOCOL.as_bytes()
        || data_len != fake.len()
        || !same_bytes(&packet[data_at..data_at + data_len], fake)
    {
        return Err(SetupRejected::WrongCookie);
    }
    if let Some(real) = real.filter(|r| r.len() == fake.len()) {
        packet[data_at..data_at + data_len].copy_from_slice(real);
    }
    Ok(())
}

// ── the forwarder ────────────────────────────────────────────────────────────

pub(crate) struct X11Forward {
    local: LocalDisplay,
    /// What the server believes the cookie is.
    fake_cookie: [u8; 16],
    /// What the local X server expects, if there is a cookie file.
    real_cookie: Option<Vec<u8>>,
    /// Room for programs that have opened a channel but not yet shown the cookie.
    waiting: Arc<Semaphore>,
}

impl X11Forward {
    pub(crate) fn new(options: &X11Options) -> Result<Self, String> {
        let local = LocalDisplay::parse(&options.display).ok_or_else(|| {
            format!("“{}” isn't an X display Aether can reach. Use an address such as localhost:0.", options.display)
        })?;
        let real_cookie = options
            .xauthority
            .clone()
            .or_else(default_xauthority)
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| cookie_for(&parse_xauthority(&bytes), local.display))
            // MIT-MAGIC-COOKIE-1 is always 128 bits; anything else is a damaged file, and a cookie of another length
            // couldn't take the fake one's place in the packet anyway.
            .filter(|cookie| cookie.len() == 16);
        Ok(Self {
            local,
            fake_cookie: uuid::Uuid::new_v4().into_bytes(),
            real_cookie,
            waiting: Arc::new(Semaphore::new(MAX_WAITING)),
        })
    }

    pub(crate) fn cookie_hex(&self) -> String {
        self.fake_cookie.iter().map(|b| format!("{b:02x}")).collect()
    }

    pub(crate) fn screen(&self) -> u32 {
        self.local.screen
    }

    pub(crate) fn display(&self) -> &LocalDisplay {
        &self.local
    }

    /// Handles one `x11` channel the server opened for a remote program.
    pub(crate) async fn serve(self: Arc<Self>, channel: Channel<Msg>, reply: ChannelOpenHandle) {
        // A program that has opened a channel and not shown the cookie costs a task and a timer. Not an unlimited number.
        let Ok(waiting) = self.waiting.clone().try_acquire_owned() else {
            info!("too many X11 programs are waiting to show the cookie; turned one away");
            reply.reject(ChannelOpenFailure::ResourceShortage).await;
            return;
        };
        reply.accept().await;
        let mut remote = channel.into_stream();

        let (host, port) = self.local.address();
        let connect = || async {
            match tokio::time::timeout(Duration::from_secs(5), TcpStream::connect((host.as_str(), port))).await {
                Ok(Ok(socket)) => {
                    let _ = socket.set_nodelay(true);
                    Ok(socket)
                }
                // The program was accepted before there was anything to say no to; closing the channel is how it hears.
                other => Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    format!("no X server to forward to at {host}:{port} ({:?})", other.map(|r| r.map(|_| ()))),
                )),
            }
        };
        if let Err(e) = self.serve_stream(&mut remote, waiting, connect).await {
            debug!(error = %e, "x11 client ended");
        }
    }

    /// One program's visit, on whatever streams: its first packet must prove it holds the cookie issued for this
    /// session, and only then is `connect` called to reach the local X server — a program that can't prove it never costs the X server
    /// a connection. `waiting` is the program's place in the queue of those yet to prove it; it is given up the moment they have.
    async fn serve_stream<R, L, F, Fut>(
        &self,
        remote: &mut R,
        waiting: OwnedSemaphorePermit,
        connect: F,
    ) -> io::Result<()>
    where
        R: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
        L: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = io::Result<L>>,
    {
        let first = self.authorize(remote).await;
        drop(waiting);
        let first = first?;
        let mut local = connect().await?;
        pump(remote, &mut local, &first).await
    }

    /// The program's first packet, once it has proved it holds the cookie issued for this session (and with the real
    /// cookie in its place). A program that doesn't send one in good time is let go.
    async fn authorize<R>(&self, remote: &mut R) -> io::Result<Vec<u8>>
    where
        R: tokio::io::AsyncRead + Unpin,
    {
        tokio::time::timeout(SETUP_TIMEOUT, self.read_setup(remote))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "the X11 client never sent its connection request"))?
    }

    /// Reads the client's connection request and lets it through only if it holds the cookie issued for this session;
    /// the real cookie goes in its place.
    async fn read_setup<R>(&self, remote: &mut R) -> io::Result<Vec<u8>>
    where
        R: tokio::io::AsyncRead + Unpin,
    {
        let invalid = |what: &str| io::Error::new(io::ErrorKind::InvalidData, what.to_string());
        let mut packet = vec![0u8; HEADER];
        remote.read_exact(&mut packet).await?;
        let total = setup_len(&packet).ok_or_else(|| invalid("not an X11 connection request"))?;
        packet.resize(total, 0);
        remote.read_exact(&mut packet[HEADER..]).await?;
        match authorize_setup(&mut packet, &self.fake_cookie, self.real_cookie.as_deref()) {
            Ok(()) => Ok(packet),
            Err(SetupRejected::NotX11) => Err(invalid("not an X11 connection request")),
            Err(SetupRejected::WrongCookie) => {
                // Someone else on the server is knocking on the forwarded display.
                info!("an X11 client without this session's cookie was turned away");
                Err(invalid("the X11 client did not present this session's cookie"))
            }
        }
    }
}

/// How long a remote program may take to send its first packet after the channel opens.
const SETUP_TIMEOUT: Duration = Duration::from_secs(10);

/// How many remote programs may be waiting, at once, to show the cookie.
const MAX_WAITING: usize = 16;

/// Hands the X server the program's (authorised) first packet, then relays everything both ways.
async fn pump<R, L>(remote: &mut R, local: &mut L, first: &[u8]) -> io::Result<()>
where
    R: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    L: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    local.write_all(first).await?;
    tokio::io::copy_bidirectional(remote, local).await.map(|_| ())
}

/// Checks that an X server answers at `display`. The message says what to do about it.
pub async fn probe(display: &str) -> Result<(), String> {
    let local = LocalDisplay::parse(display).ok_or_else(|| {
        format!("“{display}” isn't an X display Aether can reach. Use an address such as localhost:0.")
    })?;
    let (host, port) = local.address();
    match tokio::time::timeout(Duration::from_secs(2), TcpStream::connect((host.as_str(), port))).await {
        Ok(Ok(_)) => Ok(()),
        _ => Err(format!(
            "no X server is listening at {host}:{port}. Start VcXsrv, X410 or Xming and allow network (TCP) connections."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displays_parse_the_usual_spellings() {
        let d = LocalDisplay::parse("localhost:0").unwrap();
        assert_eq!((d.host.as_str(), d.display, d.screen), ("127.0.0.1", 0, 0));
        assert_eq!(d.address(), ("127.0.0.1".to_string(), 6000));
        let d = LocalDisplay::parse(":10.1").unwrap();
        assert_eq!((d.display, d.screen), (10, 1));
        assert_eq!(d.address().1, 6010);
        assert_eq!(LocalDisplay::parse("[::1]:2").unwrap().host, "::1");
        assert_eq!(LocalDisplay::parse("192.168.1.5:0.0").unwrap().host, "192.168.1.5");
    }

    #[test]
    fn displays_aether_cannot_reach_are_rejected() {
        assert!(LocalDisplay::parse("unix:0").is_none());
        assert!(LocalDisplay::parse("/tmp/.X11-unix/X0:0").is_none());
        assert!(LocalDisplay::parse("localhost").is_none());
        assert!(LocalDisplay::parse("localhost:abc").is_none());
        assert!(LocalDisplay::parse("localhost:65535").is_none(), "6000 + display would overflow a port");
    }

    fn xauth_record(family: u16, address: &[u8], number: &str, name: &str, data: &[u8]) -> Vec<u8> {
        let mut out = family.to_be_bytes().to_vec();
        for field in [address, number.as_bytes(), name.as_bytes(), data] {
            out.extend((field.len() as u16).to_be_bytes());
            out.extend(field);
        }
        out
    }

    #[test]
    fn xauthority_files_are_read_and_the_right_cookie_chosen() {
        let mut file = xauth_record(256, b"pc", "1", PROTOCOL, &[1; 16]);
        file.extend(xauth_record(256, b"pc", "0", PROTOCOL, &[2; 16]));
        file.extend(xauth_record(65535, b"", "", PROTOCOL, &[3; 16]));
        file.extend(xauth_record(256, b"pc", "0", "XDM-AUTHORIZATION-1", &[9; 16]));
        let entries = parse_xauthority(&file);
        assert_eq!(entries.len(), 4);
        assert_eq!(cookie_for(&entries, 0), Some(vec![2; 16]), "exact display wins");
        assert_eq!(cookie_for(&entries, 1), Some(vec![1; 16]));
        assert_eq!(cookie_for(&entries, 7), Some(vec![3; 16]), "wildcard entry as a fallback");
        assert_eq!(cookie_for(&[], 0), None);
    }

    #[test]
    fn a_truncated_xauthority_file_yields_what_it_can() {
        let mut file = xauth_record(256, b"pc", "0", PROTOCOL, &[5; 16]);
        let good = file.len();
        file.extend(&xauth_record(256, b"pc", "1", PROTOCOL, &[6; 16])[..9]);
        assert_eq!(parse_xauthority(&file).len(), 1, "stops at the damaged record (len {good})");
        assert!(parse_xauthority(&[0xFF]).is_empty());
    }

    fn setup_packet(big: bool, cookie: &[u8]) -> Vec<u8> {
        let u16 = |v: u16| if big { v.to_be_bytes() } else { v.to_le_bytes() };
        let mut p = vec![if big { b'B' } else { b'l' }, 0];
        p.extend(u16(11)); // protocol major
        p.extend(u16(0)); // minor
        p.extend(u16(PROTOCOL.len() as u16));
        p.extend(u16(cookie.len() as u16));
        p.extend([0, 0]);
        p.extend(PROTOCOL.as_bytes());
        p.resize(HEADER + pad4(PROTOCOL.len()), 0);
        p.extend(cookie);
        p.resize(HEADER + pad4(PROTOCOL.len()) + pad4(cookie.len()), 0);
        p
    }

    #[test]
    fn setup_requests_are_measured_in_either_byte_order() {
        for big in [true, false] {
            let packet = setup_packet(big, &[0xAA; 16]);
            assert_eq!(setup_len(&packet), Some(packet.len()));
        }
        assert_eq!(setup_len(b"GET / HTTP/1.1"), None);
        assert_eq!(setup_len(b"B\0\0\x0b"), None, "shorter than the fixed header");
    }

    #[test]
    fn only_the_issued_cookie_gets_through_and_it_becomes_the_real_one() {
        let (fake, real) = ([0xAA; 16], [0x55; 16]);
        for big in [true, false] {
            // With a real cookie to put in its place.
            let mut packet = setup_packet(big, &fake);
            assert_eq!(authorize_setup(&mut packet, &fake, Some(&real)), Ok(()));
            assert_eq!(packet, setup_packet(big, &real));
            // Without one (an X server with access control off), the packet goes on as it is.
            let mut packet = setup_packet(big, &fake);
            assert_eq!(authorize_setup(&mut packet, &fake, None), Ok(()));
            assert_eq!(packet, setup_packet(big, &fake));
            // Someone who doesn't know the cookie is turned away — whether or not there is a real one.
            for real in [Some(&real[..]), None] {
                let mut intruder = setup_packet(big, &[0x11; 16]);
                let before = intruder.clone();
                assert_eq!(authorize_setup(&mut intruder, &fake, real), Err(SetupRejected::WrongCookie));
                assert_eq!(intruder, before, "an intruder's packet is left untouched");
            }
        }
        // A client that sends no authentication at all, a cookie of the wrong length, or a different scheme.
        let mut none = setup_packet(true, &[]);
        assert_eq!(authorize_setup(&mut none, &fake, None), Err(SetupRejected::WrongCookie));
        let mut short = setup_packet(true, &fake[..8]);
        assert_eq!(authorize_setup(&mut short, &fake, None), Err(SetupRejected::WrongCookie));
        let mut other = setup_packet(true, &fake);
        other[HEADER] = b'X'; // "XIT-MAGIC-COOKIE-1"
        assert_eq!(authorize_setup(&mut other, &fake, None), Err(SetupRejected::WrongCookie));
        // Not X11 at all, or cut short.
        assert_eq!(authorize_setup(&mut b"GET / HTTP/1.1".to_vec(), &fake, None), Err(SetupRejected::NotX11));
        let full = setup_packet(true, &fake);
        assert_eq!(authorize_setup(&mut full[..20].to_vec(), &fake, None), Err(SetupRejected::NotX11));
    }

    fn forward(real: Option<Vec<u8>>) -> Arc<X11Forward> {
        Arc::new(X11Forward {
            local: LocalDisplay::parse("localhost:0").unwrap(),
            fake_cookie: [0xAA; 16],
            real_cookie: real,
            waiting: Arc::new(Semaphore::new(MAX_WAITING)),
        })
    }

    /// Runs one program (the remote end) against the real sequence of `serve`, with `to_x` as the local X server. The flag says whether
    /// anything ever connected to it.
    fn visit(
        fwd: Arc<X11Forward>,
        mut from_server: tokio::io::DuplexStream,
        to_x: tokio::io::DuplexStream,
    ) -> (tokio::task::JoinHandle<io::Result<()>>, Arc<std::sync::atomic::AtomicBool>) {
        let connected = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = connected.clone();
        let task = tokio::spawn(async move {
            let waiting = fwd.waiting.clone().try_acquire_owned().expect("room to wait");
            fwd.serve_stream(&mut from_server, waiting, move || async move {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(to_x)
            })
            .await
        });
        (task, connected)
    }

    #[tokio::test]
    async fn a_program_with_the_issued_cookie_reaches_the_x_server_wearing_the_real_one() {
        let fwd = forward(Some(vec![0x55; 16]));
        let (from_server, mut program) = tokio::io::duplex(4096);
        let (to_x, mut x_server) = tokio::io::duplex(4096);
        let (relay, connected) = visit(fwd, from_server, to_x);

        program.write_all(&setup_packet(true, &[0xAA; 16])).await.unwrap();
        let expected = setup_packet(true, &[0x55; 16]);
        let mut seen = vec![0u8; expected.len()];
        x_server.read_exact(&mut seen).await.unwrap();
        assert_eq!(seen, expected);
        assert!(connected.load(std::sync::atomic::Ordering::SeqCst));

        // And bytes keep flowing both ways afterwards.
        x_server.write_all(b"reply").await.unwrap();
        let mut buf = [0u8; 5];
        program.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"reply");
        program.write_all(b"request").await.unwrap();
        let mut buf = [0u8; 7];
        x_server.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"request");

        drop(program);
        drop(x_server);
        relay.await.unwrap().ok();
    }

    /// The point of checking the cookie *first*: a program without it never even makes the X server accept a connection.
    #[tokio::test]
    async fn a_program_without_the_cookie_never_reaches_the_x_server() {
        for real in [Some(vec![0x55; 16]), None] {
            let fwd = forward(real);
            let (from_server, mut intruder) = tokio::io::duplex(4096);
            let (to_x, mut x_server) = tokio::io::duplex(4096);
            let (relay, connected) = visit(fwd, from_server, to_x);

            intruder.write_all(&setup_packet(true, &[0x11; 16])).await.unwrap();
            let err = relay.await.unwrap().unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidData, "{err}");
            assert!(
                !connected.load(std::sync::atomic::Ordering::SeqCst),
                "the X server was connected to for an intruder"
            );
            let mut buf = [0u8; 64];
            assert_eq!(x_server.read(&mut buf).await.unwrap(), 0, "not a single byte got to the X server");
        }
    }

    /// A program that opens the channel and then says nothing must not be able to hold a connection to the X server — it
    /// never gets one.
    #[tokio::test(start_paused = true)]
    async fn a_program_that_never_speaks_is_cut_off() {
        let fwd = forward(None);
        let (from_server, _silent_program) = tokio::io::duplex(64);
        let (to_x, _x_server) = tokio::io::duplex(64);
        let (relay, connected) = visit(fwd, from_server, to_x);
        let err = relay.await.unwrap().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut, "{err}");
        assert!(!connected.load(std::sync::atomic::Ordering::SeqCst));
    }

    /// The place in the queue is given up as soon as the program has proved itself — or has been turned away.
    #[tokio::test]
    async fn a_program_stops_taking_a_place_in_the_queue_once_it_has_proved_itself() {
        let fwd = forward(None);
        let (from_server, mut program) = tokio::io::duplex(4096);
        let (to_x, x_server) = tokio::io::duplex(4096);
        let (relay, _) = visit(fwd.clone(), from_server, to_x);
        // Waiting for the first packet: one place taken.
        tokio::task::yield_now().await;
        assert_eq!(fwd.waiting.available_permits(), MAX_WAITING - 1);
        program.write_all(&setup_packet(true, &[0xAA; 16])).await.unwrap();
        // Proved: the place is free again although the program stays connected.
        for _ in 0..100 {
            if fwd.waiting.available_permits() == MAX_WAITING {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(fwd.waiting.available_permits(), MAX_WAITING);
        // Both ends hang up, and the relay ends with them.
        drop(program);
        drop(x_server);
        relay.await.unwrap().ok();
    }

    /// Only so many programs may stand at the door without the cookie; the rest are told there is no room.
    #[test]
    fn only_so_many_programs_may_wait_to_show_the_cookie() {
        let fwd = forward(None);
        let held: Vec<_> = (0..MAX_WAITING).map(|_| fwd.waiting.clone().try_acquire_owned().expect("room")).collect();
        assert!(fwd.waiting.clone().try_acquire_owned().is_err(), "the next one is turned away");
        drop(held);
        assert!(fwd.waiting.clone().try_acquire_owned().is_ok(), "and there is room again once they have gone");
    }

    #[test]
    fn issued_cookies_are_random_hex() {
        let opts = X11Options { display: "localhost:0".into(), xauthority: Some(PathBuf::from("nonexistent-xauth")) };
        let a = X11Forward::new(&opts).unwrap();
        let b = X11Forward::new(&opts).unwrap();
        assert_eq!(a.cookie_hex().len(), 32);
        assert_ne!(a.cookie_hex(), b.cookie_hex());
        assert!(a.cookie_hex().chars().all(|c| c.is_ascii_hexdigit()));
        assert!(X11Forward::new(&X11Options { display: "nope".into(), xauthority: None }).is_err());
    }
}
