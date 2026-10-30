//! Local IPC streams: a unix socket on Linux/macOS, a named pipe on
//! Windows. One [`Stream`] type so the request/reply and relay loops can
//! be written once (`chat::relay`, `mux::herdr::socket_call`).

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// ERROR_PIPE_BUSY: another process is still connecting to the pipe.
#[cfg(windows)]
const ERROR_PIPE_BUSY: i32 = 231;

/// The `\\.\pipe\` prefix a Windows named pipe path carries (`\\server\pipe\name`).
pub const PIPE_PREFIX: &str = r"\\.\pipe\";

/// Is `path` a Windows named pipe rather than a file/socket path?
pub fn is_pipe(path: &str) -> bool {
    path.starts_with(PIPE_PREFIX) && path.len() > PIPE_PREFIX.len()
}

/// `path` as a full named-pipe path: herdr 0.9.5 reports its API endpoint
/// as a plain path (e.g. `C:\Users\me\AppData\Roaming\herdr\herdr.sock`)
/// while serving it as the pipe `\\.\pipe\<that path>`.
pub fn pipe_path(path: &str) -> String {
    if is_pipe(path) {
        path.to_string()
    } else {
        format!("{PIPE_PREFIX}{path}")
    }
}

/// A connected local IPC stream.
#[derive(Debug)]
pub enum Stream {
    #[cfg(unix)]
    Unix(tokio::net::UnixStream),
    #[cfg(windows)]
    Pipe(tokio::net::windows::named_pipe::NamedPipeClient),
}

impl AsyncRead for Stream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match &mut *self {
            #[cfg(unix)]
            Stream::Unix(stream) => Pin::new(stream).poll_read(cx, buf),
            #[cfg(windows)]
            Stream::Pipe(stream) => Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Stream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match &mut *self {
            #[cfg(unix)]
            Stream::Unix(stream) => Pin::new(stream).poll_write(cx, buf),
            #[cfg(windows)]
            Stream::Pipe(stream) => Pin::new(stream).poll_write(cx, buf),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut *self {
            #[cfg(unix)]
            Stream::Unix(stream) => Pin::new(stream).poll_flush(cx),
            #[cfg(windows)]
            Stream::Pipe(stream) => Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut *self {
            #[cfg(unix)]
            Stream::Unix(stream) => Pin::new(stream).poll_shutdown(cx),
            #[cfg(windows)]
            Stream::Pipe(stream) => Pin::new(stream).poll_shutdown(cx),
        }
    }
}

/// Connects to a unix socket path.
#[cfg(unix)]
pub async fn connect_unix(path: &str) -> io::Result<Stream> {
    Ok(Stream::Unix(tokio::net::UnixStream::connect(path).await?))
}

/// Connects to a Windows named pipe (`\\.\pipe\<name>`), retrying while it
/// is `ERROR_PIPE_BUSY` (a pipe only accepts a connection once the server
/// has a free instance; herdr/pi create theirs slightly late).
#[cfg(windows)]
pub async fn connect_pipe(path: &str) -> io::Result<Stream> {
    use tokio::net::windows::named_pipe::ClientOptions;
    let mut attempt = 0u32;
    loop {
        match ClientOptions::new().open(path) {
            Ok(client) => return Ok(Stream::Pipe(client)),
            Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY) && attempt < 20 => {
                attempt += 1;
                tokio::time::sleep(std::time::Duration::from_millis(25 * u64::from(attempt))).await;
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipes_are_recognised_and_prefixed() {
        assert!(is_pipe(r"\\.\pipe\herdr"));
        assert!(!is_pipe("/home/u/.unipi/bridge/42.sock"));
        assert!(!is_pipe(r"C:\Users\u\AppData\Roaming\herdr\herdr.sock"));
        // herdr's socket_path is a plain path; the real pipe is prefixed.
        assert_eq!(
            pipe_path(r"C:\Users\oi\AppData\Roaming\herdr\herdr.sock"),
            r"\\.\pipe\C:\Users\oi\AppData\Roaming\herdr\herdr.sock"
        );
        // Already a full pipe path: unchanged (no double prefix).
        assert_eq!(
            pipe_path(r"\\.\pipe\unipi-bridge-42"),
            r"\\.\pipe\unipi-bridge-42"
        );
    }
}
