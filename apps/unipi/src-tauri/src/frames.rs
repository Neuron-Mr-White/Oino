//! Reading host frames without wedging on a bad one (fix-frames P0).
//!
//! Every reader used to reject a frame over [`MAX_FRAME`] with "oversize frame from
//! host" and end its stream: a session stream that met one live event with a browser
//! screenshot went dead (banner + Retry, the chat stopped updating while the turn kept
//! "Working…"). The frame header carries the length, so an oversize frame can be
//! *skipped* — its payload drained — and the stream stays in sync: the caller logs it
//! and recovers (a session stream asks for a full resync) instead of dying.

use tokio::io::{AsyncRead, AsyncReadExt};
use unipi_proto::frame::MAX_FRAME;
use unipi_proto::{AppErrorDto, ProtoError};

/// The largest frame a reader drains to skip it. A length past this is not a frame the
/// host would send (garbage / desync): the stream really is broken then.
pub(crate) const MAX_SKIP: usize = 256 * 1024 * 1024;

/// What one read produced.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FrameRead {
    /// A frame: its kind byte and payload.
    Frame(u8, Vec<u8>),
    /// A frame over [`MAX_FRAME`], drained and dropped (the stream is still in sync).
    Skipped { kind: u8, len: usize },
    /// A clean end of stream.
    End,
}

fn transport(error: impl std::fmt::Display) -> AppErrorDto {
    ProtoError::Transport(error.to_string()).to_app_error()
}

/// Reads one frame; an oversize one is drained and reported as [`FrameRead::Skipped`].
pub(crate) async fn read_lenient<R: AsyncRead + Unpin>(
    recv: &mut R,
) -> Result<FrameRead, AppErrorDto> {
    let kind = match recv.read_u8().await {
        Ok(kind) => kind,
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Ok(FrameRead::End);
        }
        Err(error) => return Err(transport(error)),
    };
    let len = recv.read_u32().await.map_err(transport)? as usize;
    if len > MAX_FRAME {
        if len > MAX_SKIP {
            return Err(transport(format!(
                "The PC sent a broken frame ({len} bytes)"
            )));
        }
        drain(recv, len).await?;
        log::warn!(
            "skipped an oversize frame from the host: kind {kind}, {len} bytes (cap {MAX_FRAME})"
        );
        return Ok(FrameRead::Skipped { kind, len });
    }
    let mut payload = vec![0u8; len];
    recv.read_exact(&mut payload).await.map_err(transport)?;
    Ok(FrameRead::Frame(kind, payload))
}

/// Reads and discards `len` bytes.
async fn drain<R: AsyncRead + Unpin>(recv: &mut R, mut len: usize) -> Result<(), AppErrorDto> {
    let mut scratch = vec![0u8; 64 * 1024];
    while len > 0 {
        let take = len.min(scratch.len());
        recv.read_exact(&mut scratch[..take])
            .await
            .map_err(transport)?;
        len -= take;
    }
    Ok(())
}

/// The error a one-shot reader returns for a skipped oversize reply: retryable transport
/// trouble (the old non-retryable schema error left the UI with a dead banner).
pub(crate) fn oversize_error(len: usize) -> AppErrorDto {
    log::warn!("the host's reply was {len} bytes, over the {MAX_FRAME}-byte frame cap");
    ProtoError::Transport(OVERSIZE.into()).to_app_error()
}

/// The message of [`oversize_error`] (callers may recognise it to retry differently).
pub(crate) const OVERSIZE: &str = "The PC's answer was too large to receive";

#[cfg(test)]
mod tests {
    use super::*;
    use unipi_proto::frame::{encode, FrameKind};

    fn oversize(len: usize) -> Vec<u8> {
        let mut bytes = vec![0u8];
        bytes.extend_from_slice(&(len as u32).to_be_bytes());
        bytes.extend(std::iter::repeat_n(b'x', len));
        bytes
    }

    #[tokio::test]
    async fn an_oversize_frame_is_skipped_and_the_next_one_reads() {
        let mut bytes = oversize(MAX_FRAME + 10);
        bytes.extend(encode(FrameKind::Control, b"{\"type\":\"ok\"}"));
        let mut read = bytes.as_slice();
        assert_eq!(
            read_lenient(&mut read).await.unwrap(),
            FrameRead::Skipped {
                kind: 0,
                len: MAX_FRAME + 10
            }
        );
        assert_eq!(
            read_lenient(&mut read).await.unwrap(),
            FrameRead::Frame(0, b"{\"type\":\"ok\"}".to_vec())
        );
        assert_eq!(read_lenient(&mut read).await.unwrap(), FrameRead::End);
    }

    #[tokio::test]
    async fn an_absurd_length_is_a_transport_error() {
        let mut bytes = vec![0u8];
        bytes.extend_from_slice(&u32::MAX.to_be_bytes());
        let mut read = bytes.as_slice();
        let error = read_lenient(&mut read).await.unwrap_err();
        assert!(error.retryable);
    }
}
