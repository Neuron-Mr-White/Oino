//! Frame encoding for `unipi/app/1`: `u8 kind · u32BE length · payload`.
//!
//! Kinds:
//! - 0: JSON control (UTF-8)
//! - 1: terminal output bytes
//! - 2: terminal input bytes

use thiserror::Error;

pub const FRAME_HEADER_LEN: usize = 5;
/// Maximum payload size in bytes (1 MiB).
pub const MAX_FRAME: usize = 1024 * 1024;

/// A frame's `kind` byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    Control = 0,
    Output = 1,
    Input = 2,
}

impl FrameKind {
    fn from_byte(byte: u8) -> Option<FrameKind> {
        match byte {
            0 => Some(FrameKind::Control),
            1 => Some(FrameKind::Output),
            2 => Some(FrameKind::Input),
            _ => None,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FrameError {
    #[error("unknown frame kind: {0}")]
    UnknownKind(u8),

    #[error("frame payload too large: {0} > {MAX_FRAME}")]
    Oversized(usize),
}

pub fn encode(kind: FrameKind, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(FRAME_HEADER_LEN + payload.len());
    out.push(kind as u8);
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// Encodes one frame: `[kind: u8][len: u32BE][payload]`.
pub fn try_encode(kind: FrameKind, payload: &[u8]) -> Result<Vec<u8>, FrameError> {
    if payload.len() > MAX_FRAME {
        return Err(FrameError::Oversized(payload.len()));
    }

    let mut out = Vec::with_capacity(FRAME_HEADER_LEN + payload.len());
    out.push(kind as u8);
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedFrame {
    pub kind: FrameKind,
    pub payload: Vec<u8>,
}

/// Incremental frame decoder. Push arbitrary byte chunks with
/// `push`, then drain fully-received frames with `pop`. `push`
/// returns an error as soon as an invalid header (unknown kind,
/// oversized length) is seen, even if the full frame hasn't arrived
/// yet.
#[derive(Debug, Default)]
pub struct Decoder {
    buffer: Vec<u8>,
}

impl Decoder {
    pub fn new() -> Self {
        Decoder { buffer: Vec::new() }
    }

    pub fn push(&mut self, chunk: &[u8]) -> Result<(), FrameError> {
        self.buffer.extend_from_slice(chunk);
        self.validate_header_if_present()
    }

    /// Returns the next fully-buffered frame, or `None` if none yet.
    /// Returns an error if the buffered header is invalid.
    pub fn pop(&mut self) -> Result<Option<DecodedFrame>, FrameError> {
        if self.buffer.len() < FRAME_HEADER_LEN {
            return Ok(None);
        }

        let kind_byte = self.buffer[0];
        let len = u32::from_be_bytes([
            self.buffer[1],
            self.buffer[2],
            self.buffer[3],
            self.buffer[4],
        ]) as usize;

        let kind = Self::validate_header(kind_byte, len)?;

        let total = FRAME_HEADER_LEN + len;
        if self.buffer.len() < total {
            return Ok(None);
        }

        let payload = self.buffer[FRAME_HEADER_LEN..total].to_vec();
        self.buffer.drain(0..total);
        Ok(Some(DecodedFrame { kind, payload }))
    }

    fn validate_header_if_present(&self) -> Result<(), FrameError> {
        if self.buffer.len() < FRAME_HEADER_LEN {
            return Ok(());
        }
        let kind_byte = self.buffer[0];
        let len = u32::from_be_bytes([
            self.buffer[1],
            self.buffer[2],
            self.buffer[3],
            self.buffer[4],
        ]) as usize;
        Self::validate_header(kind_byte, len).map(|_| ())
    }

    fn validate_header(kind_byte: u8, len: usize) -> Result<FrameKind, FrameError> {
        let kind = FrameKind::from_byte(kind_byte).ok_or(FrameError::UnknownKind(kind_byte))?;
        if len > MAX_FRAME {
            return Err(FrameError::Oversized(len));
        }
        Ok(kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_control_frame() {
        let payload = br#"{"type":"ok"}"#;
        let framed = encode(FrameKind::Control, payload);
        assert_eq!(framed[0], 0);
        let len = u32::from_be_bytes([framed[1], framed[2], framed[3], framed[4]]) as usize;
        assert_eq!(len, payload.len());
        assert_eq!(&framed[5..], payload);
    }

    #[test]
    fn encode_rejects_oversized_payload() {
        let oversized = vec![0u8; MAX_FRAME + 1];
        let result = try_encode(FrameKind::Output, &oversized);
        assert_eq!(result, Err(FrameError::Oversized(MAX_FRAME + 1)));
    }

    #[test]
    fn encode_accepts_exactly_max_frame() {
        let max_payload = vec![0u8; MAX_FRAME];
        assert!(try_encode(FrameKind::Output, &max_payload).is_ok());
    }

    #[test]
    fn decoder_decodes_single_frame_pushed_whole() {
        let payload = b"hello world";
        let framed = encode(FrameKind::Output, payload);

        let mut decoder = Decoder::new();
        decoder.push(&framed).expect("push");
        let frame = decoder.pop().expect("pop").expect("some frame");

        assert_eq!(frame.kind, FrameKind::Output);
        assert_eq!(frame.payload, payload);
        assert_eq!(decoder.pop().expect("pop"), None);
    }

    #[test]
    fn decoder_decodes_frame_split_byte_by_byte() {
        let payload = b"the quick brown fox jumps over the lazy dog";
        let framed = encode(FrameKind::Input, payload);

        let mut decoder = Decoder::new();
        for byte in &framed {
            assert_eq!(decoder.pop().expect("pop"), None);
            decoder.push(&[*byte]).expect("push");
        }

        let frame = decoder.pop().expect("pop").expect("some frame");
        assert_eq!(frame.kind, FrameKind::Input);
        assert_eq!(frame.payload, payload);
    }

    #[test]
    fn decoder_decodes_multiple_concatenated_frames() {
        let a = encode(FrameKind::Control, b"one");
        let b = encode(FrameKind::Input, b"two");
        let c = encode(FrameKind::Output, b"three");

        let mut merged = Vec::new();
        merged.extend_from_slice(&a);
        merged.extend_from_slice(&b);
        merged.extend_from_slice(&c);

        let mut decoder = Decoder::new();
        decoder.push(&merged).expect("push");

        assert_eq!(
            decoder.pop().expect("pop"),
            Some(DecodedFrame {
                kind: FrameKind::Control,
                payload: b"one".to_vec()
            })
        );
        assert_eq!(
            decoder.pop().expect("pop"),
            Some(DecodedFrame {
                kind: FrameKind::Input,
                payload: b"two".to_vec()
            })
        );
        assert_eq!(
            decoder.pop().expect("pop"),
            Some(DecodedFrame {
                kind: FrameKind::Output,
                payload: b"three".to_vec()
            })
        );
        assert_eq!(decoder.pop().expect("pop"), None);
    }

    #[test]
    fn decoder_errors_on_unknown_kind_as_soon_as_seen() {
        let mut decoder = Decoder::new();
        let mut bad = vec![0u8; 5];
        bad[0] = 9;
        assert_eq!(decoder.push(&bad), Err(FrameError::UnknownKind(9)));
    }

    #[test]
    fn decoder_errors_on_oversized_length_before_body_arrives() {
        let mut decoder = Decoder::new();
        let mut header = vec![0u8; 5];
        header[0] = 1;
        header[1..5].copy_from_slice(&((MAX_FRAME as u32) + 1).to_be_bytes());
        assert_eq!(
            decoder.push(&header),
            Err(FrameError::Oversized(MAX_FRAME + 1))
        );
    }

    #[test]
    fn decoder_returns_none_while_waiting_for_more_bytes() {
        let mut decoder = Decoder::new();
        decoder.push(&[0, 0, 0]).expect("push");
        assert_eq!(decoder.pop().expect("pop"), None);
    }
}
