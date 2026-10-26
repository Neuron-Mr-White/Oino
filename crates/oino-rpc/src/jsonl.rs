//! Strict JSONL framing (pi `jsonl.ts`): records end with `\n` only; a trailing `\r` is
//! stripped; U+2028/U+2029 inside strings are payload, not separators.

use tokio::io::{AsyncRead, AsyncReadExt};

/// Serialises one record as a line (`JSON + "\n"`).
#[must_use]
pub fn serialize_line(value: &impl serde::Serialize) -> String {
    let mut line = serde_json::to_string(value).unwrap_or_else(|_| "null".to_owned());
    line.push('\n');
    line
}

/// Reads LF-terminated lines from an async byte source.
#[derive(Debug)]
pub struct LineReader<R> {
    source: R,
    buffer: Vec<u8>,
    eof: bool,
}

impl<R: AsyncRead + Unpin> LineReader<R> {
    /// Wraps `source`.
    pub const fn new(source: R) -> Self {
        Self {
            source,
            buffer: Vec::new(),
            eof: false,
        }
    }

    /// The next line without its terminator; `None` at end of input. A final unterminated
    /// line is returned before `None` (pi emits it on `end`). Invalid UTF-8 is replaced.
    ///
    /// # Errors
    ///
    /// Read failures of the source.
    pub async fn next_line(&mut self) -> std::io::Result<Option<String>> {
        loop {
            if let Some(index) = self.buffer.iter().position(|b| *b == b'\n') {
                let rest = self.buffer.split_off(index + 1);
                let mut line = std::mem::replace(&mut self.buffer, rest);
                line.pop();
                return Ok(Some(decode(line)));
            }
            if self.eof {
                if self.buffer.is_empty() {
                    return Ok(None);
                }
                return Ok(Some(decode(std::mem::take(&mut self.buffer))));
            }
            let mut chunk = [0_u8; 8192];
            let read = self.source.read(&mut chunk).await?;
            if read == 0 {
                self.eof = true;
            } else {
                self.buffer.extend_from_slice(&chunk[..read]);
            }
        }
    }
}

fn decode(mut line: Vec<u8>) -> String {
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    match String::from_utf8(line) {
        Ok(text) => text,
        Err(error) => String::from_utf8_lossy(error.as_bytes()).into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn splits_on_lf_only() {
        let input: &[u8] = "{\"a\":\"x\u{2028}y\"}\r\n\n{\"b\":1}".as_bytes();
        let mut reader = LineReader::new(input);
        assert_eq!(
            reader.next_line().await.unwrap().as_deref(),
            Some("{\"a\":\"x\u{2028}y\"}")
        );
        assert_eq!(reader.next_line().await.unwrap().as_deref(), Some(""));
        assert_eq!(
            reader.next_line().await.unwrap().as_deref(),
            Some("{\"b\":1}")
        );
        assert_eq!(reader.next_line().await.unwrap(), None);
    }

    #[test]
    fn serializes_with_lf() {
        assert_eq!(
            serialize_line(&serde_json::json!({"a": "\u{2029}"})),
            "{\"a\":\"\u{2029}\"}\n"
        );
    }
}
