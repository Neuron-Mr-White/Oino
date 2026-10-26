//! Incremental decoder for the AWS event stream framing (`application/vnd.amazon.eventstream`)
//! Bedrock's `ConverseStream` answers with: big-endian prelude (total length, headers length,
//! prelude CRC32), typed headers, payload, message CRC32.
//!
//! Each message becomes an [`SseEvent`]: `event` = `:event-type` for events and
//! `exception:<:exception-type>` (or `error:<:error-code>`) for modeled / unmodeled errors;
//! `data` = the UTF-8 payload.

use crate::sse::SseEvent;

const PRELUDE: usize = 12;
const MIN_MESSAGE: usize = 16;

/// Byte-chunk event stream decoder.
#[derive(Debug, Default)]
pub struct EventStreamDecoder {
    buffer: Vec<u8>,
    corrupt: bool,
}

/// A framing failure (pi surfaces the SDK's deserialization error).
pub const CORRUPT_MESSAGE: &str = "Event stream message is corrupt";

impl EventStreamDecoder {
    /// New decoder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds bytes; returns the messages completed by them. A corrupt frame yields one
    /// `event: "corrupt"` item and stops decoding.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        if self.corrupt {
            return Vec::new();
        }
        self.buffer.extend_from_slice(bytes);
        let mut out = Vec::new();
        while self.buffer.len() >= PRELUDE {
            let total = read_u32(&self.buffer, 0) as usize;
            let headers_len = read_u32(&self.buffer, 4) as usize;
            if total < MIN_MESSAGE
                || headers_len + MIN_MESSAGE > total
                || crc32(&self.buffer[..8]) != read_u32(&self.buffer, 8)
            {
                self.corrupt = true;
                out.push(corrupt());
                return out;
            }
            if self.buffer.len() < total {
                break;
            }
            let frame: Vec<u8> = self.buffer.drain(..total).collect();
            if crc32(&frame[..total - 4]) != read_u32(&frame, total - 4) {
                self.corrupt = true;
                out.push(corrupt());
                return out;
            }
            let headers = parse_headers(&frame[PRELUDE..PRELUDE + headers_len]);
            let payload = String::from_utf8_lossy(&frame[PRELUDE + headers_len..total - 4]);
            let header = |name: &str| {
                headers
                    .iter()
                    .find(|(k, _)| k == name)
                    .map(|(_, v)| v.clone())
            };
            let event = match header(":message-type").as_deref() {
                Some("exception") => format!(
                    "exception:{}",
                    header(":exception-type").unwrap_or_default()
                ),
                Some("error") => format!("error:{}", header(":error-code").unwrap_or_default()),
                _ => header(":event-type").unwrap_or_default(),
            };
            out.push(SseEvent {
                event: Some(event),
                data: payload.into_owned(),
            });
        }
        out
    }

    /// End of input (a truncated trailing frame is dropped like the SDK does).
    pub fn finish(&mut self) -> Vec<SseEvent> {
        self.buffer.clear();
        Vec::new()
    }
}

fn corrupt() -> SseEvent {
    SseEvent {
        event: Some("corrupt".into()),
        data: CORRUPT_MESSAGE.into(),
    }
}

fn read_u32(bytes: &[u8], at: usize) -> u32 {
    let mut word = [0u8; 4];
    word.copy_from_slice(&bytes[at..at + 4]);
    u32::from_be_bytes(word)
}

/// String-valued headers (other header types are skipped).
fn parse_headers(mut bytes: &[u8]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    while let Some((&name_len, rest)) = bytes.split_first() {
        let name_len = usize::from(name_len);
        if rest.len() < name_len + 1 {
            break;
        }
        let name = String::from_utf8_lossy(&rest[..name_len]).into_owned();
        let kind = rest[name_len];
        let rest = &rest[name_len + 1..];
        let (value, rest) = match kind {
            0 | 1 => (None, rest),
            2 => (None, rest.get(1..).unwrap_or_default()),
            3 => (None, rest.get(2..).unwrap_or_default()),
            4 => (None, rest.get(4..).unwrap_or_default()),
            5 | 8 => (None, rest.get(8..).unwrap_or_default()),
            9 => (None, rest.get(16..).unwrap_or_default()),
            6 | 7 => {
                if rest.len() < 2 {
                    break;
                }
                let len = usize::from(u16::from_be_bytes([rest[0], rest[1]]));
                let Some(raw) = rest.get(2..2 + len) else {
                    break;
                };
                let value = (kind == 7).then(|| String::from_utf8_lossy(raw).into_owned());
                (value, &rest[2 + len..])
            }
            _ => break,
        };
        if let Some(value) = value {
            out.push((name, value));
        }
        bytes = rest;
    }
    out
}

/// CRC-32 (IEEE, reflected) as used by the event stream checksums.
#[must_use]
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(name: &str, value: &str) -> Vec<u8> {
        let mut out = vec![u8::try_from(name.len()).unwrap()];
        out.extend_from_slice(name.as_bytes());
        out.push(7);
        out.extend_from_slice(&u16::try_from(value.len()).unwrap().to_be_bytes());
        out.extend_from_slice(value.as_bytes());
        out
    }

    fn frame(event: &str, payload: &str) -> Vec<u8> {
        let mut headers = header(":event-type", event);
        headers.extend(header(":message-type", "event"));
        let total = 12 + headers.len() + payload.len() + 4;
        let mut out = Vec::new();
        out.extend_from_slice(&u32::try_from(total).unwrap().to_be_bytes());
        out.extend_from_slice(&u32::try_from(headers.len()).unwrap().to_be_bytes());
        let crc = crc32(&out);
        out.extend_from_slice(&crc.to_be_bytes());
        out.extend(headers);
        out.extend_from_slice(payload.as_bytes());
        let crc = crc32(&out);
        out.extend_from_slice(&crc.to_be_bytes());
        out
    }

    #[test]
    fn crc_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn decodes_split_frames() {
        let mut bytes = frame("messageStart", "{\"role\":\"assistant\"}");
        bytes.extend(frame("messageStop", "{}"));
        let mut decoder = EventStreamDecoder::new();
        let mut events = Vec::new();
        for chunk in bytes.chunks(5) {
            events.extend(decoder.feed(chunk));
        }
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event.as_deref(), Some("messageStart"));
        assert_eq!(events[1].data, "{}");
    }

    #[test]
    fn flags_corruption() {
        let mut bytes = frame("messageStart", "{}");
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        let events = EventStreamDecoder::new().feed(&bytes);
        assert_eq!(events[0].event.as_deref(), Some("corrupt"));
    }
}
