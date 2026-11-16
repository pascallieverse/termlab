//! The local connection between Termlab's processes (app ↔ `termlabd`, `termlab-hook` →
//! `termlabd`): a Unix socket, or a named pipe on Windows, where tokio has no Unix sockets.
//!
//! Both are addressed by a path. On Windows [`address`] turns a file path into a pipe name,
//! so callers keep passing the path inside the data folder everywhere.

use std::io;
use std::path::{Path, PathBuf};

use tokio::io::{AsyncRead, AsyncWrite};

/// One accepted or opened connection.
pub trait Stream: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> Stream for T {}

pub type Conn = Box<dyn Stream>;

/// Where a socket path is really served: the path itself, or on Windows the named pipe that
/// stands in for it. Applying it twice changes nothing.
pub fn address(path: &Path) -> PathBuf {
    if cfg!(windows) {
        pipe_name(path)
    } else {
        path.to_path_buf()
    }
}

const PIPE_PREFIX: &str = r"\\.\pipe\";

/// `\\.\pipe\termlab-<hash of the path>`. The hash is spelled out here because it must stay
/// the same between versions: after an update a new app talks to an older daemon.
fn pipe_name(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if text.starts_with(PIPE_PREFIX) {
        return path.to_path_buf();
    }
    // FNV-1a, 64 bit. Windows paths don't distinguish case.
    let lower = text.to_lowercase();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in lower.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    // The file's name without its extension, to tell the pipes apart when listing them.
    let file = lower.rsplit(['/', '\\']).next().unwrap_or_default();
    let name = file.split('.').next().unwrap_or_default();
    PathBuf::from(format!("{PIPE_PREFIX}termlab-{hash:016x}-{name}"))
}

pub use imp::{Listener, connect};

#[cfg(unix)]
mod imp {
    use super::*;
    use tokio::net::{UnixListener, UnixStream};

    pub struct Listener(UnixListener);

    impl Listener {
        /// Replaces a socket file left by an earlier run. Only the owner may connect.
        /// Must be called inside a tokio runtime.
        pub fn bind(path: &Path) -> io::Result<Self> {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::remove_file(path);
            let listener = UnixListener::bind(path)?;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
            Ok(Self(listener))
        }

        pub async fn accept(&mut self) -> io::Result<Conn> {
            let (stream, _) = self.0.accept().await?;
            Ok(Box::new(stream))
        }
    }

    pub async fn connect(path: &Path) -> io::Result<Conn> {
        Ok(Box::new(UnixStream::connect(path).await?))
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::time::Duration;
    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions};

    /// `ERROR_PIPE_BUSY`: every instance of the pipe is taken; one frees up shortly.
    const PIPE_BUSY: i32 = 231;

    pub struct Listener {
        name: PathBuf,
        /// The instance the next client connects to.
        next: NamedPipeServer,
    }

    impl Listener {
        /// Fails when another process already serves this name. Other machines can't
        /// connect (tokio's default). Other accounts on this machine can only read: see
        /// `ipc::Server`, which says nothing until a client has spoken.
        /// Must be called inside a tokio runtime.
        pub fn bind(path: &Path) -> io::Result<Self> {
            let name = address(path);
            let next = ServerOptions::new()
                .first_pipe_instance(true)
                .create(&name)?;
            Ok(Self { name, next })
        }

        pub async fn accept(&mut self) -> io::Result<Conn> {
            self.next.connect().await?;
            let next = ServerOptions::new().create(&self.name)?;
            Ok(Box::new(std::mem::replace(&mut self.next, next)))
        }
    }

    pub async fn connect(path: &Path) -> io::Result<Conn> {
        let name = address(path);
        let mut tries = 0;
        loop {
            match ClientOptions::new().open(&name) {
                Ok(client) => return Ok(Box::new(client)),
                Err(e) if e.raw_os_error() == Some(PIPE_BUSY) && tries < 40 => {
                    tries += 1;
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                Err(e) => return Err(e),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipe_names_are_stable_and_distinct() {
        let a = pipe_name(Path::new(
            r"C:\Users\Me\AppData\Roaming\termlab\termlabd.sock",
        ));
        assert_eq!(
            a,
            pipe_name(Path::new(
                r"c:\users\me\appdata\roaming\termlab\termlabd.sock"
            )),
            "case doesn't matter"
        );
        // Pinned: an older daemon and a newer app must agree on the name.
        assert_eq!(
            a.to_string_lossy(),
            r"\\.\pipe\termlab-729baa20634f9c14-termlabd"
        );
        let hook = pipe_name(Path::new(r"C:\Users\Me\AppData\Roaming\termlab\hook.sock"));
        assert_ne!(a, hook);
        assert!(hook.to_string_lossy().ends_with("-hook"));
        assert_eq!(pipe_name(&a), a, "already a pipe name");
    }

    #[tokio::test]
    async fn a_client_reaches_the_listener() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sock");
        let mut listener = Listener::bind(&path).unwrap();
        let server = tokio::spawn(async move {
            let mut conn = listener.accept().await.unwrap();
            let mut buf = [0u8; 4];
            conn.read_exact(&mut buf).await.unwrap();
            conn.write_all(&buf).await.unwrap();
        });
        let mut client = connect(&path).await.unwrap();
        client.write_all(b"ping").await.unwrap();
        let mut back = [0u8; 4];
        client.read_exact(&mut back).await.unwrap();
        assert_eq!(&back, b"ping");
        server.await.unwrap();
    }
}
