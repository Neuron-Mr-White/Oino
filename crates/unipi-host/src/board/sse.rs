//! Incremental `text/event-stream` parser (WHATWG SSE framing).
//!
//! - Lines end with CRLF, LF or CR (a CR at a chunk boundary followed by an
//!   LF in the next chunk counts once).
//! - `event:` sets the event name, `data:` lines are joined with `\n`, one
//!   leading space after the colon is dropped.
//! - A blank line dispatches the message; a message without any `data:` line
//!   is discarded (spec behaviour).
//! - Comment lines (`:` keep-alives) and `id:`/`retry:`/unknown fields are
//!   dropped.

/// One dispatched SSE message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseMessage {
    pub event: Option<String>,
    pub data: String,
}

/// Messages bigger than this are dropped (they could not fit a control
/// frame, and the kanboard stream only carries tiny revision numbers).
pub const MAX_MESSAGE: usize = 512 * 1024;

#[derive(Debug, Default)]
pub struct SseParser {
    line: Vec<u8>,
    /// The previous chunk ended in CR; swallow one leading LF.
    pending_cr: bool,
    started: bool,
    event: Option<String>,
    data: String,
    has_data: bool,
    oversized: bool,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds bytes; returns every message completed by them.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseMessage> {
        let mut out = Vec::new();
        for &byte in chunk {
            if self.pending_cr {
                self.pending_cr = false;
                if byte == b'\n' {
                    continue;
                }
            }
            match byte {
                b'\n' => self.end_line(&mut out),
                b'\r' => {
                    self.pending_cr = true;
                    self.end_line(&mut out);
                }
                _ => {
                    if self.line.len() <= MAX_MESSAGE {
                        self.line.push(byte);
                    } else {
                        self.oversized = true;
                    }
                    // A UTF-8 BOM may open the stream (possibly split).
                    if !self.started && self.line.len() == 3 {
                        self.started = true;
                        if self.line == b"\xEF\xBB\xBF" {
                            self.line.clear();
                        }
                    }
                }
            }
        }
        out
    }

    fn end_line(&mut self, out: &mut Vec<SseMessage>) {
        self.started = true;
        let line = std::mem::take(&mut self.line);
        if line.is_empty() {
            self.dispatch(out);
            return;
        }
        if line[0] == b':' {
            return;
        }
        let text = String::from_utf8_lossy(&line);
        let (field, value) = match text.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (text.as_ref(), ""),
        };
        match field {
            "event" => self.event = Some(value.to_owned()),
            "data" => {
                if self.has_data {
                    self.data.push('\n');
                }
                self.data.push_str(value);
                self.has_data = true;
                if self.data.len() > MAX_MESSAGE {
                    self.oversized = true;
                    self.data.clear();
                }
            }
            _ => {}
        }
    }

    fn dispatch(&mut self, out: &mut Vec<SseMessage>) {
        let event = self.event.take().filter(|name| !name.is_empty());
        let data = std::mem::take(&mut self.data);
        let keep = self.has_data && !self.oversized;
        self.has_data = false;
        self.oversized = false;
        if keep {
            out.push(SseMessage { event, data });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(event: Option<&str>, data: &str) -> SseMessage {
        SseMessage {
            event: event.map(str::to_owned),
            data: data.to_owned(),
        }
    }

    #[test]
    fn parses_named_event() {
        let mut parser = SseParser::new();
        assert_eq!(
            parser.push(b"event: revision\ndata: 1\n\n"),
            vec![message(Some("revision"), "1")]
        );
    }

    #[test]
    fn joins_multi_line_data_and_keeps_unnamed() {
        let mut parser = SseParser::new();
        assert_eq!(
            parser.push(b"data: one\ndata:two\ndata:  three\n\n"),
            vec![message(None, "one\ntwo\n three")]
        );
    }

    #[test]
    fn drops_comments_ids_and_dataless_messages() {
        let mut parser = SseParser::new();
        assert_eq!(parser.push(b": keep-alive\n\n"), vec![]);
        assert_eq!(parser.push(b"event: ping\n\n"), vec![]);
        assert_eq!(
            parser.push(b"id: 7\nretry: 100\nfoo: bar\ndata: x\n\n"),
            vec![message(None, "x")]
        );
    }

    #[test]
    fn event_name_resets_between_messages() {
        let mut parser = SseParser::new();
        assert_eq!(
            parser.push(b"event: revision\ndata: 1\n\ndata: 2\n\n"),
            vec![message(Some("revision"), "1"), message(None, "2")]
        );
    }

    #[test]
    fn handles_crlf_and_bare_cr() {
        let mut parser = SseParser::new();
        assert_eq!(
            parser.push(b"event: revision\r\ndata: 1\r\n\r\ndata: 2\r\r"),
            vec![message(Some("revision"), "1"), message(None, "2")]
        );
    }

    #[test]
    fn crlf_split_across_chunks_counts_once() {
        let mut parser = SseParser::new();
        assert_eq!(parser.push(b"data: a\r"), vec![]);
        assert_eq!(parser.push(b"\n"), vec![]);
        assert_eq!(parser.push(b"\r"), vec![message(None, "a")]);
        assert_eq!(parser.push(b"\ndata: b\n\n"), vec![message(None, "b")]);
    }

    #[test]
    fn byte_by_byte_feed() {
        let mut parser = SseParser::new();
        let mut out = Vec::new();
        for byte in b"\xEF\xBB\xBFevent:revision\ndata:42\n\n: hb\n\n" {
            out.extend(parser.push(&[*byte]));
        }
        assert_eq!(out, vec![message(Some("revision"), "42")]);
    }

    #[test]
    fn empty_data_line_dispatches_empty_string() {
        let mut parser = SseParser::new();
        assert_eq!(parser.push(b"data\n\n"), vec![message(None, "")]);
        assert_eq!(parser.push(b"data:\n\n"), vec![message(None, "")]);
    }

    #[test]
    fn oversized_message_is_dropped() {
        let mut parser = SseParser::new();
        let mut big = b"data: ".to_vec();
        big.extend(std::iter::repeat_n(b'x', MAX_MESSAGE + 10));
        big.extend_from_slice(b"\n\ndata: ok\n\n");
        assert_eq!(parser.push(&big), vec![message(None, "ok")]);
    }
}
