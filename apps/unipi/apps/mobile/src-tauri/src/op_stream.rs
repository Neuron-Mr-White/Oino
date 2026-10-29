//! UNI-230 (plan A1, docs/m8 §3.3): one logical op's two halves, whatever
//! carries them — a QUIC bi-stream to a host over iroh, or (desktop, this
//! PC's own host) one connection to its local socket `app.sock`. Both speak
//! the same `u8 kind · u32 len` frames, so callers only need AsyncRead /
//! AsyncWrite plus `finish()` (QUIC: no more data; Unix: shutdown(Write)).

use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// The sending half of an op.
pub enum OpSend {
    Iroh(iroh::endpoint::SendStream),
    #[cfg(unix)]
    Local(tokio::net::unix::OwnedWriteHalf),
    /// Tests: an in-memory pipe.
    #[cfg(test)]
    Duplex(tokio::io::WriteHalf<tokio::io::DuplexStream>),
}

/// The receiving half of an op.
pub enum OpRecv {
    Iroh(iroh::endpoint::RecvStream),
    #[cfg(unix)]
    Local(tokio::net::unix::OwnedReadHalf),
    #[cfg(test)]
    Duplex(tokio::io::ReadHalf<tokio::io::DuplexStream>),
}

/// Why `finish` failed (QUIC: the stream was already closed).
#[derive(Debug)]
pub struct FinishError(pub String);
impl std::fmt::Display for FinishError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for FinishError {}

impl OpSend {
    /// No more data from us (the host sees end-of-stream). Synchronous like
    /// QUIC's `finish`: the local socket's shutdown is a non-blocking syscall.
    pub fn finish(&mut self) -> Result<(), FinishError> {
        match self {
            OpSend::Iroh(s) => s.finish().map_err(|e| FinishError(e.to_string())),
            #[cfg(unix)]
            OpSend::Local(w) => {
                let sock: &tokio::net::UnixStream = w.as_ref();
                shutdown_write(sock).map_err(|e| FinishError(e.to_string()))
            }
            #[cfg(test)]
            OpSend::Duplex(_) => Ok(()),
        }
    }
}

#[cfg(unix)]
fn shutdown_write(sock: &tokio::net::UnixStream) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    // shutdown(2) on the shared fd: the write half is ours alone.
    let r = unsafe { libc_shutdown(sock.as_raw_fd()) };
    if r == 0 {
        Ok(())
    } else {
        let e = std::io::Error::last_os_error();
        // Already closed by the peer: nothing left to finish.
        if e.kind() == std::io::ErrorKind::NotConnected { Ok(()) } else { Err(e) }
    }
}

#[cfg(unix)]
unsafe fn libc_shutdown(fd: i32) -> i32 {
    extern "C" {
        fn shutdown(socket: i32, how: i32) -> i32;
    }
    // SHUT_WR = 1 on Linux, macOS, the BSDs and Android.
    shutdown(fd, 1)
}

impl AsyncWrite for OpSend {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            OpSend::Iroh(s) => Pin::new(s).poll_write(cx, buf).map_err(std::io::Error::other),
            #[cfg(unix)]
            OpSend::Local(w) => Pin::new(w).poll_write(cx, buf),
            #[cfg(test)]
            OpSend::Duplex(w) => Pin::new(w).poll_write(cx, buf),
        }
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            OpSend::Iroh(s) => Pin::new(s).poll_flush(cx).map_err(std::io::Error::other),
            #[cfg(unix)]
            OpSend::Local(w) => Pin::new(w).poll_flush(cx),
            #[cfg(test)]
            OpSend::Duplex(w) => Pin::new(w).poll_flush(cx),
        }
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            OpSend::Iroh(s) => Pin::new(s).poll_shutdown(cx).map_err(std::io::Error::other),
            #[cfg(unix)]
            OpSend::Local(w) => Pin::new(w).poll_shutdown(cx),
            #[cfg(test)]
            OpSend::Duplex(w) => Pin::new(w).poll_shutdown(cx),
        }
    }
}

impl AsyncRead for OpRecv {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            OpRecv::Iroh(r) => Pin::new(r).poll_read(cx, buf).map_err(std::io::Error::other),
            #[cfg(unix)]
            OpRecv::Local(r) => Pin::new(r).poll_read(cx, buf),
            #[cfg(test)]
            OpRecv::Duplex(r) => Pin::new(r).poll_read(cx, buf),
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn local_halves_write_finish_and_read() {
        let (a, b) = tokio::net::UnixStream::pair().unwrap();
        let (ar, aw) = a.into_split();
        let (mut br, mut bw) = b.into_split();
        let mut send = OpSend::Local(aw);
        let mut recv = OpRecv::Local(ar);
        send.write_all(b"hello").await.unwrap();
        send.finish().unwrap();
        let mut got = Vec::new();
        br.read_to_end(&mut got).await.unwrap();
        assert_eq!(got, b"hello", "finish = EOF for the host");
        bw.write_all(b"back").await.unwrap();
        drop(bw);
        let mut back = Vec::new();
        recv.read_to_end(&mut back).await.unwrap();
        assert_eq!(back, b"back");
    }
}
