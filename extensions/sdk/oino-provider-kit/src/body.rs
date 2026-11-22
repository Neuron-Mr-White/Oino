//! End-of-body cut marker of the Oino host (`oino-wasm-host` `http.rs`).
//!
//! The WIT `http.fetch` body is a plain `stream<u8>` that closes like EOF when the connection
//! fails or goes idle, so a stream cut mid-event would be parsed as a truncated last event
//! (`malformed server-sent event JSON`). A request carrying [`BODY_ERRORS_HEADER`] (the host
//! strips it) gets a fixed-size marker as the last bytes of a body that ended early; a
//! [`BodyTail`] holds back a suffix that could be the marker so it never reaches the decoder.

/// Request header asking the host for the cut marker.
pub const BODY_ERRORS_HEADER: &str = "x-oino-body-errors";
/// Leading bytes of the marker (mirrors `oino-wasm-host` `http::CUT_MAGIC`).
pub const CUT_MAGIC: &[u8; 12] = b"\0\xffOINO-CUT\xff\0";
/// Marker size: magic, kind byte, idle milliseconds (u32 LE).
pub const CUT_LEN: usize = CUT_MAGIC.len() + 1 + 4;

/// Why the host ended a body early.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyCut {
    /// No chunk arrived within the request timeout (milliseconds).
    Idle(u32),
    /// The connection failed mid-body (reset, protocol error).
    Reset,
}

impl BodyCut {
    /// Transient connection error text for the failed message.
    #[must_use]
    pub fn message(self) -> String {
        match self {
            Self::Idle(ms) if ms >= 1000 => {
                format!("Connection error: stream idle for {} s.", ms / 1000)
            }
            Self::Idle(ms) => format!("Connection error: stream idle for {ms} ms."),
            Self::Reset => "Connection error: connection reset while streaming.".to_owned(),
        }
    }
}

/// Holds back up to [`CUT_LEN`] trailing bytes that could be the marker.
#[derive(Debug, Default)]
pub struct BodyTail {
    held: Vec<u8>,
}

impl BodyTail {
    /// New, empty tail.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a chunk; returns the bytes that are certainly body data. Only a suffix that could
    /// still become a marker is held, so text bodies (no NUL byte) are never delayed.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<u8> {
        self.held.extend_from_slice(chunk);
        let len = self.held.len();
        let ready = (len.saturating_sub(CUT_LEN)..len)
            .find(|&start| {
                let candidate = &self.held[start..];
                let n = candidate.len().min(CUT_MAGIC.len());
                candidate[..n] == CUT_MAGIC[..n]
            })
            .unwrap_or(len);
        self.held.drain(..ready).collect()
    }

    /// End of body: the remaining data and the cut, if the body ended with the marker.
    #[must_use]
    pub fn end(self) -> (Vec<u8>, Option<BodyCut>) {
        let mut held = self.held;
        if held.len() == CUT_LEN && held.starts_with(CUT_MAGIC) {
            let kind = held[CUT_MAGIC.len()];
            let ms = u32::from_le_bytes([held[13], held[14], held[15], held[16]]);
            let cut = match kind {
                b'i' => Some(BodyCut::Idle(ms)),
                b'r' => Some(BodyCut::Reset),
                _ => None,
            };
            if cut.is_some() {
                held.clear();
                return (held, cut);
            }
        }
        (held, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marker(kind: u8, ms: u32) -> Vec<u8> {
        let mut m = CUT_MAGIC.to_vec();
        m.push(kind);
        m.extend_from_slice(&ms.to_le_bytes());
        m
    }

    fn run(chunks: &[&[u8]]) -> (Vec<u8>, Option<BodyCut>) {
        let mut tail = BodyTail::new();
        let mut data = Vec::new();
        for chunk in chunks {
            data.extend(tail.push(chunk));
        }
        let (rest, cut) = tail.end();
        data.extend(rest);
        (data, cut)
    }

    #[test]
    fn plain_bodies_pass_through_whole() {
        assert_eq!(
            run(&[b"data: 1\n\n", b"da", b"ta: 2"]),
            (b"data: 1\n\ndata: 2".to_vec(), None)
        );
        assert_eq!(run(&[b"x"]), (b"x".to_vec(), None));
        assert_eq!(run(&[]), (Vec::new(), None));
    }

    #[test]
    fn a_marker_split_across_chunks_is_decoded_and_withheld() {
        let mut body = b"data: {\"a\"".to_vec();
        body.extend(marker(b'i', 300_000));
        for split in 0..body.len() {
            let (head, rest) = body.split_at(split);
            assert_eq!(
                run(&[head, rest]),
                (b"data: {\"a\"".to_vec(), Some(BodyCut::Idle(300_000)))
            );
        }
        assert_eq!(run(&[&marker(b'r', 0)]), (Vec::new(), Some(BodyCut::Reset)));
    }

    #[test]
    fn text_is_released_at_once() {
        let mut tail = BodyTail::new();
        assert_eq!(tail.push(b"data: {\"a\":1}\n\n"), b"data: {\"a\":1}\n\n");
        assert_eq!(tail.push(b"x\0"), b"x");
        assert_eq!(tail.push(b"y"), b"\0y");
    }

    #[test]
    fn an_unknown_kind_is_data() {
        let m = marker(b'z', 1);
        assert_eq!(run(&[&m]), (m.clone(), None));
    }

    #[test]
    fn messages_are_transient_connection_errors() {
        assert_eq!(
            BodyCut::Idle(300_000).message(),
            "Connection error: stream idle for 300 s."
        );
        assert_eq!(
            BodyCut::Idle(200).message(),
            "Connection error: stream idle for 200 ms."
        );
        assert!(
            BodyCut::Reset
                .message()
                .starts_with("Connection error: connection reset")
        );
    }
}
