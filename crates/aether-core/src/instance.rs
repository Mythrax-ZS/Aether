//! Keeps a profile to one running copy on systems that have no ready-made plugin for it (the BSDs).
//!
//! The first copy takes an exclusive lock on a file in the profile's run folder and keeps it for as long as it
//! lives. The system lets go of the lock when the process ends, however it ends, so a crash leaves nothing to clean
//! up. The first copy also listens on a Unix socket in the same folder: a later copy that finds the lock taken sends
//! its command line over that socket and quits, and the first copy decides what to do with it (usually: come to the
//! front).
//!
//! The run folder is private to the user (mode 0700), which is what keeps other accounts away from the socket.

use std::fs::{self, DirBuilder, File, OpenOptions, TryLockError};
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use tracing::{debug, warn};

const RUN_DIR: &str = "run";
const LOCK_FILE: &str = "instance.lock";
const SOCKET_FILE: &str = "instance.sock";
/// More than any command line is worth; keeps a stray connection from filling memory.
const MAX_MESSAGE: u64 = 256 * 1024;
/// How long one connection may take to deliver its message.
const READ_TIMEOUT: Duration = Duration::from_secs(2);
/// How long a later copy keeps trying while the first is between taking the lock and listening.
const HAND_OVER_PATIENCE: Duration = Duration::from_secs(3);

/// What [`claim`] found out.
pub enum Claim {
    /// This is the first copy. Keep the value for as long as the program runs: it holds the lock.
    First(Instance),
    /// Another copy already runs this profile; this copy's arguments were passed to it, as far as that was possible,
    /// and this one should quit.
    AlreadyRunning,
}

/// The first copy's claim on the profile.
pub struct Instance {
    _lock: File,
    listener: Option<UnixListener>,
    socket: PathBuf,
}

/// Claims the profile whose run folder lives under `dir`, or hands `args` to the copy that already did.
///
/// An error means the folder or lock file could not be used at all; the caller can carry on without a guard.
pub fn claim(dir: &Path, args: &[String]) -> io::Result<Claim> {
    let run = dir.join(RUN_DIR);
    DirBuilder::new().recursive(true).mode(0o700).create(&run)?;
    // `create` leaves a folder that was already there as it was.
    fs::set_permissions(&run, fs::Permissions::from_mode(0o700))?;

    let lock =
        OpenOptions::new().read(true).write(true).create(true).truncate(false).mode(0o600).open(run.join(LOCK_FILE))?;
    let socket = run.join(SOCKET_FILE);
    match lock.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            if let Err(e) = hand_over(&socket, args) {
                warn!(error = %e, "another copy of Aether is running, but it could not be told about this launch");
            }
            return Ok(Claim::AlreadyRunning);
        }
        Err(TryLockError::Error(e)) => return Err(e),
    }

    // Whatever socket is left over belongs to a copy that is gone: it held the lock, which we now have.
    match fs::remove_file(&socket) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let listener = match UnixListener::bind(&socket).and_then(|l| {
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
        Ok(l)
    }) {
        Ok(listener) => Some(listener),
        // Most likely a path too long for a socket. Holding the lock still keeps a second copy from starting; it just
        // can't be told to come to the front.
        Err(e) => {
            warn!(error = %e, path = %socket.display(), "cannot listen for later launches");
            None
        }
    };
    Ok(Claim::First(Instance { _lock: lock, listener, socket }))
}

impl Instance {
    /// Starts passing the command line of every later launch to `handler`, on a thread of its own.
    pub fn serve(&self, mut handler: impl FnMut(Vec<String>) + Send + 'static) -> io::Result<()> {
        let Some(listener) = &self.listener else { return Ok(()) };
        let listener = listener.try_clone()?;
        thread::Builder::new().name("aether-instance".into()).spawn(move || {
            loop {
                match listener.accept() {
                    Ok((stream, _)) => match read_message(stream) {
                        Ok(args) => handler(args),
                        Err(e) => debug!(error = %e, "ignored a malformed hand-over"),
                    },
                    Err(e) => {
                        // Out of descriptors, say. Don't spin while it lasts.
                        warn!(error = %e, "cannot accept a later launch");
                        thread::sleep(Duration::from_millis(250));
                    }
                }
            }
        })?;
        Ok(())
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        if self.listener.is_some() {
            let _ = fs::remove_file(&self.socket);
        }
    }
}

/// Sends `args` to the copy listening on `socket`, waiting a little if it is still starting up.
fn hand_over(socket: &Path, args: &[String]) -> io::Result<()> {
    let message = serde_json::to_vec(args).map_err(io::Error::other)?;
    let give_up = Instant::now() + HAND_OVER_PATIENCE;
    loop {
        match UnixStream::connect(socket) {
            Ok(mut stream) => {
                stream.write_all(&message)?;
                // The end of the stream is the end of the message.
                return stream.shutdown(Shutdown::Write);
            }
            Err(e)
                if matches!(e.kind(), io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused)
                    && Instant::now() < give_up =>
            {
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e),
        }
    }
}

fn read_message(mut stream: UnixStream) -> io::Result<Vec<String>> {
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    let mut bytes = Vec::new();
    (&mut stream).take(MAX_MESSAGE).read_to_end(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{self, Receiver};

    use super::*;

    fn first(dir: &Path) -> Instance {
        match claim(dir, &[]).unwrap() {
            Claim::First(instance) => instance,
            Claim::AlreadyRunning => panic!("expected to be the first copy"),
        }
    }

    fn serve_into_channel(instance: &Instance) -> Receiver<Vec<String>> {
        let (tx, rx) = mpsc::channel();
        instance.serve(move |args| tx.send(args).unwrap()).unwrap();
        rx
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn a_later_copy_hands_its_arguments_to_the_first() {
        let tmp = tempfile::tempdir().unwrap();
        let instance = first(tmp.path());
        let received = serve_into_channel(&instance);

        let sent = args(&["aether", "--host", "my box", "ünïcode ✓"]);
        assert!(matches!(claim(tmp.path(), &sent).unwrap(), Claim::AlreadyRunning));

        assert_eq!(received.recv_timeout(Duration::from_secs(5)).unwrap(), sent);
    }

    #[test]
    fn every_later_copy_is_heard_in_turn() {
        let tmp = tempfile::tempdir().unwrap();
        let instance = first(tmp.path());
        let received = serve_into_channel(&instance);

        for n in 0..3 {
            let sent = args(&["aether", &n.to_string()]);
            assert!(matches!(claim(tmp.path(), &sent).unwrap(), Claim::AlreadyRunning));
            assert_eq!(received.recv_timeout(Duration::from_secs(5)).unwrap(), sent);
        }
    }

    #[test]
    fn the_claim_is_free_again_once_the_first_copy_is_gone() {
        let tmp = tempfile::tempdir().unwrap();
        drop(first(tmp.path()));
        drop(first(tmp.path()));
    }

    #[test]
    fn a_socket_left_by_a_crash_does_not_get_in_the_way() {
        let tmp = tempfile::tempdir().unwrap();
        let run = tmp.path().join(RUN_DIR);
        fs::create_dir_all(&run).unwrap();
        // A listener that went away without cleaning up leaves its file behind; nobody holds the lock.
        drop(UnixListener::bind(run.join(SOCKET_FILE)).unwrap());
        assert!(run.join(SOCKET_FILE).exists());

        let instance = first(tmp.path());
        let received = serve_into_channel(&instance);
        assert!(matches!(claim(tmp.path(), &args(&["aether"])).unwrap(), Claim::AlreadyRunning));
        assert_eq!(received.recv_timeout(Duration::from_secs(5)).unwrap(), args(&["aether"]));
    }

    #[test]
    fn nonsense_on_the_socket_is_ignored_and_the_next_launch_still_works() {
        let tmp = tempfile::tempdir().unwrap();
        let instance = first(tmp.path());
        let received = serve_into_channel(&instance);

        let mut stray = UnixStream::connect(tmp.path().join(RUN_DIR).join(SOCKET_FILE)).unwrap();
        stray.write_all(b"this is not json").unwrap();
        drop(stray);

        let sent = args(&["aether", "real"]);
        assert!(matches!(claim(tmp.path(), &sent).unwrap(), Claim::AlreadyRunning));
        assert_eq!(received.recv_timeout(Duration::from_secs(5)).unwrap(), sent);
    }

    #[test]
    fn the_run_folder_and_socket_are_private_to_the_user() {
        let tmp = tempfile::tempdir().unwrap();
        let _instance = first(tmp.path());
        let mode = |p: PathBuf| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let run = tmp.path().join(RUN_DIR);
        assert_eq!(mode(run.clone()), 0o700);
        assert_eq!(mode(run.join(LOCK_FILE)), 0o600);
        assert_eq!(mode(run.join(SOCKET_FILE)), 0o600);
    }

    #[test]
    fn a_path_too_long_for_a_socket_still_keeps_a_second_copy_out() {
        let tmp = tempfile::tempdir().unwrap();
        let long = tmp.path().join("x".repeat(60)).join("y".repeat(60)).join("z".repeat(60));
        let instance = first(&long);
        // Nothing to listen on, so nothing is served and nothing breaks.
        instance.serve(|_| unreachable!("no socket, no launches")).unwrap();
        assert!(matches!(claim(&long, &args(&["aether"])).unwrap(), Claim::AlreadyRunning));
    }

    #[test]
    fn dropping_the_first_copy_removes_its_socket() {
        let tmp = tempfile::tempdir().unwrap();
        let socket = tmp.path().join(RUN_DIR).join(SOCKET_FILE);
        let instance = first(tmp.path());
        assert!(socket.exists());
        drop(instance);
        assert!(!socket.exists());
    }
}
