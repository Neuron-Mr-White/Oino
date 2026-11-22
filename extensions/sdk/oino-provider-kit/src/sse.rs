//! Incremental server-sent-events decoder with the semantics of pi-ai's Anthropic SSE reader and
//! the `OpenAI` SDK `SSEDecoder`: `\r`, `\n` and `\r\n` line breaks, `event:`/`data:` fields,
//! `:` comments, dispatch on blank lines and a trailing flush at end of input.

/// One decoded event.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SseEvent {
    /// `event:` field, if any.
    pub event: Option<String>,
    /// `data:` lines joined with `\n`.
    pub data: String,
}

/// Byte-chunk SSE decoder.
#[derive(Debug, Default)]
pub struct SseDecoder {
    pending: Vec<u8>,
    buffer: String,
    event: Option<String>,
    data: Vec<String>,
}

impl SseDecoder {
    /// New decoder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds bytes; returns the events completed by them.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        self.pending.extend_from_slice(bytes);
        let valid = match std::str::from_utf8(&self.pending) {
            Ok(text) => text.len(),
            Err(error) if error.error_len().is_none() => error.valid_up_to(),
            Err(_) => self.pending.len(),
        };
        let chunk: Vec<u8> = self.pending.drain(..valid).collect();
        self.buffer.push_str(&String::from_utf8_lossy(&chunk));
        let mut events = Vec::new();
        while let Some((line, rest)) = take_line(&self.buffer) {
            self.buffer = rest;
            if let Some(event) = self.line(&line) {
                events.push(event);
            }
        }
        events
    }

    /// Ends the input: decodes the unterminated tail and flushes a pending event.
    pub fn finish(&mut self) -> Vec<SseEvent> {
        let mut events = Vec::new();
        if !self.pending.is_empty() {
            let tail = std::mem::take(&mut self.pending);
            self.buffer.push_str(&String::from_utf8_lossy(&tail));
        }
        while let Some((line, rest)) = take_line(&self.buffer) {
            self.buffer = rest;
            events.extend(self.line(&line));
        }
        let tail = std::mem::take(&mut self.buffer);
        if !tail.is_empty() {
            events.extend(self.line(&tail));
        }
        events.extend(self.flush());
        events
    }

    fn flush(&mut self) -> Option<SseEvent> {
        if self.event.is_none() && self.data.is_empty() {
            return None;
        }
        Some(SseEvent {
            event: self.event.take(),
            data: std::mem::take(&mut self.data).join("\n"),
        })
    }

    fn line(&mut self, line: &str) -> Option<SseEvent> {
        if line.is_empty() {
            return self.flush();
        }
        if line.starts_with(':') {
            return None;
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => self.event = Some(value.to_owned()),
            "data" => self.data.push(value.to_owned()),
            _ => {}
        }
        None
    }
}

fn take_line(text: &str) -> Option<(String, String)> {
    let index = text.find(['\r', '\n'])?;
    let bytes = text.as_bytes();
    // A trailing `\r` may be the first half of `\r\n`; wait for more input.
    if bytes[index] == b'\r' && index + 1 == bytes.len() {
        return None;
    }
    let next = if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
        index + 2
    } else {
        index + 1
    };
    Some((text[..index].to_owned(), text[next..].to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_across_chunks_and_line_endings() {
        let mut decoder = SseDecoder::new();
        let mut events = decoder.feed(b"event: a\r\ndata: {\"x\"");
        events.extend(decoder.feed(b":1}\r\n\r\n: comment\ndata: 2\ndata: 3\n\nda"));
        events.extend(decoder.feed(b"ta: tail"));
        events.extend(decoder.finish());
        assert_eq!(
            events,
            vec![
                SseEvent {
                    event: Some("a".into()),
                    data: "{\"x\":1}".into()
                },
                SseEvent {
                    event: None,
                    data: "2\n3".into()
                },
                SseEvent {
                    event: None,
                    data: "tail".into()
                },
            ]
        );
    }

    #[test]
    fn keeps_split_utf8() {
        let mut decoder = SseDecoder::new();
        let bytes = "data: é\n\n".as_bytes();
        let mut events = decoder.feed(&bytes[..7]);
        events.extend(decoder.feed(&bytes[7..]));
        assert_eq!(events[0].data, "é");
    }
}
