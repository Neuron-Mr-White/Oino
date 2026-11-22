//! Private worker protocol, independent of the app's 1 MiB codec.
//! Control and data MUST use different connections. A decoder is bound to one lane.
//! Payloads on the data lane are unchanged `ClientEvent` / `RpcCommand` JSON bytes.

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROTOCOL: u32 = 1;
pub const MAX_WORKER_FRAME: usize = 4 * 1024 * 1024;
pub const INITIAL_BYTES: u64 = 8 * 1024 * 1024;
pub const INITIAL_FRAMES: u64 = 512;
pub const HEADER_LEN: usize = 9;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    Control,
    Data,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Control = 0,
    Session = 1,
    Credit = 2,
    HostReply = 3,
}

impl Kind {
    fn decode(value: u8) -> Result<Self, Error> {
        match value {
            0 => Ok(Self::Control),
            1 => Ok(Self::Session),
            2 => Ok(Self::Credit),
            3 => Ok(Self::HostReply),
            _ => Err(Error::Kind(value)),
        }
    }

    fn lane(self) -> Lane {
        if self == Self::Session {
            Lane::Data
        } else {
            Lane::Control
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub kind: Kind,
    pub stream: u32,
    pub payload: Vec<u8>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum Error {
    #[error("unknown worker frame kind {0}")]
    Kind(u8),
    #[error("worker frame too large: {0}")]
    Oversize(usize),
    #[error("worker frame on wrong connection")]
    Lane,
    #[error("worker decoder poisoned after invalid header")]
    Poisoned,
    #[error("invalid worker credit")]
    Credit,
}

/// # Errors
/// Returns an error for oversized payloads or frames on the wrong lane.
pub fn encode(frame: &Frame, lane: Lane) -> Result<Vec<u8>, Error> {
    validate(frame.kind, frame.payload.len(), lane)?;
    let mut bytes = Vec::with_capacity(HEADER_LEN + frame.payload.len());
    bytes.push(frame.kind as u8);
    bytes.extend_from_slice(&frame.stream.to_be_bytes());
    bytes.extend_from_slice(
        &u32::try_from(frame.payload.len())
            .map_err(|_| Error::Oversize(frame.payload.len()))?
            .to_be_bytes(),
    );
    bytes.extend_from_slice(&frame.payload);
    Ok(bytes)
}

fn validate(kind: Kind, len: usize, lane: Lane) -> Result<(), Error> {
    if len > MAX_WORKER_FRAME {
        return Err(Error::Oversize(len));
    }
    if kind.lane() != lane {
        return Err(Error::Lane);
    }
    Ok(())
}

/// Bounded incremental decoder. Copies at most nine header bytes before checking
/// the length, even when the caller supplies a huge chunk. No unvalidated payload
/// allocation or unbounded queue of decoded frames is retained internally.
pub struct Decoder {
    lane: Lane,
    header: [u8; HEADER_LEN],
    used: usize,
    payload: Vec<u8>,
    poisoned: bool,
}

impl Decoder {
    #[must_use]
    pub fn new(lane: Lane) -> Self {
        Self {
            lane,
            header: [0; HEADER_LEN],
            used: 0,
            payload: Vec::new(),
            poisoned: false,
        }
    }

    /// Consumes up to one frame and returns the consumed byte count. The caller
    /// retains the remaining input and must deliver this frame before decoding more.
    /// # Errors
    /// Returns an error for invalid headers, wrong lanes, or a poisoned decoder.
    pub fn decode(&mut self, input: &[u8]) -> Result<(usize, Option<Frame>), Error> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        let mut consumed = 0;
        if self.used < HEADER_LEN {
            let n = (HEADER_LEN - self.used).min(input.len());
            self.header[self.used..self.used + n].copy_from_slice(&input[..n]);
            self.used += n;
            consumed += n;
            if self.used < HEADER_LEN {
                return Ok((consumed, None));
            }
        }
        let kind = Kind::decode(self.header[0]);
        let len = u32::from_be_bytes([
            self.header[5],
            self.header[6],
            self.header[7],
            self.header[8],
        ]) as usize;
        let kind = match kind.and_then(|kind| validate(kind, len, self.lane).map(|()| kind)) {
            Ok(kind) => kind,
            Err(error) => {
                self.poisoned = true;
                return Err(error);
            }
        };
        let n = (len - self.payload.len()).min(input.len() - consumed);
        self.payload
            .extend_from_slice(&input[consumed..consumed + n]);
        consumed += n;
        if self.payload.len() < len {
            return Ok((consumed, None));
        }
        let stream = u32::from_be_bytes([
            self.header[1],
            self.header[2],
            self.header[3],
            self.header[4],
        ]);
        self.used = 0;
        Ok((
            consumed,
            Some(Frame {
                kind,
                stream,
                payload: std::mem::take(&mut self.payload),
            }),
        ))
    }
}

/// Per-stream dual credit. A positive balance in BOTH dimensions authorizes one
/// frame, which may overdraw bytes. No further frame is allowed until replenished.
/// The receiver reserves `INITIAL_BYTES` + `MAX_WORKER_FRAME` and `INITIAL_FRAMES` slots.
#[derive(Clone, Debug)]
pub struct Credits {
    bytes: i64,
    frames: u64,
}

impl Default for Credits {
    fn default() -> Self {
        Self {
            bytes: INITIAL_BYTES.cast_signed(),
            frames: INITIAL_FRAMES,
        }
    }
}

impl Credits {
    #[must_use]
    pub fn can_send(&self) -> bool {
        self.bytes > 0 && self.frames > 0
    }

    /// # Errors
    /// Returns an error when the byte count exceeds [`MAX_WORKER_FRAME`].
    pub fn take(&mut self, bytes: usize) -> Result<bool, Error> {
        if bytes > MAX_WORKER_FRAME {
            return Err(Error::Oversize(bytes));
        }
        if !self.can_send() {
            return Ok(false);
        }
        self.bytes -= i64::try_from(bytes).map_err(|_| Error::Oversize(bytes))?;
        self.frames -= 1;
        Ok(true)
    }

    /// Replenish only after publish, never beyond the original reservation.
    /// # Errors
    /// Returns an error for overflowing credit or credit beyond the initial reservation.
    pub fn replenish(&mut self, bytes: u64, frames: u64) -> Result<(), Error> {
        let bytes = i64::try_from(bytes)
            .ok()
            .and_then(|n| self.bytes.checked_add(n))
            .ok_or(Error::Credit)?;
        let frames = self.frames.checked_add(frames).ok_or(Error::Credit)?;
        if bytes > INITIAL_BYTES.cast_signed() || frames > INITIAL_FRAMES {
            return Err(Error::Credit);
        }
        self.bytes = bytes;
        self.frames = frames;
        Ok(())
    }
}

/// Worker-level control messages. Engine commands and events stay opaque and do
/// not change oino-rpc or its hook ordering. Request ids are control-local ids.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Control {
    Hello {
        proto: u32,
        version: String,
        pid: u32,
        key: String,
        cwd: String,
        extensions: Vec<String>,
    },
    Ready,
    Attach {
        stream: u32,
        session_target: String,
        takeover: bool,
        ui_caps: serde_json::Value,
    },
    Attached {
        stream: u32,
        session_id: String,
    },
    AttachFailed {
        stream: u32,
        error: String,
    },
    Detach {
        stream: u32,
    },
    Create {
        stream: u32,
        cwd: String,
        model: Option<serde_json::Value>,
        id: Option<String>,
        parent: Option<String>,
    },
    Close {
        id: u64,
        session_id: String,
    },
    Reply {
        id: u64,
        result: serde_json::Value,
    },
    Ping {
        id: u64,
    },
    Pong {
        id: u64,
    },
    State {
        live_sessions: usize,
        services_sets: usize,
        busy_sessions: usize,
        keepalive_services: usize,
    },
    Credit {
        stream: u32,
        bytes: u64,
        frames: u64,
    },
    Lagged {
        stream: u32,
    },
    IndexAppend {
        session_id: String,
        data: serde_json::Value,
    },
    HostCmd {
        id: u64,
        cmd: serde_json::Value,
    },
    HostCall {
        id: u64,
        service: String,
        op: String,
        json: serde_json::Value,
        session: String,
    },
    HostReply {
        id: u64,
        result: serde_json::Value,
    },
    Route {
        id: u64,
        op: String,
        target: String,
        data: serde_json::Value,
    },
    SessionReplaced {
        old: String,
        new: String,
    },
    ReloadExtensions,
    Shutdown {
        reason: String,
    },
    Bye {
        reason: String,
    },
}

/// Explicit env override, otherwise the persisted extensionSettings.hub.workers.
/// Invalid env values fail closed, and an absent setting is always off.
#[must_use]
pub fn enabled(setting: Option<&str>, env: Option<&str>) -> bool {
    match env {
        Some("1") => true,
        Some(_) => false,
        None => setting == Some("on"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn one_frame_overdraft_and_both_limits() {
        let mut c = Credits::default();
        assert!(c.take(MAX_WORKER_FRAME - 1).unwrap());
        assert!(c.take(MAX_WORKER_FRAME).unwrap());
        assert!(c.take(MAX_WORKER_FRAME).unwrap()); // one byte positive before this frame
        assert!(!c.take(1).unwrap());
        assert_eq!(c.bytes, -(i64::try_from(MAX_WORKER_FRAME).unwrap()) + 1);
        c.replenish(MAX_WORKER_FRAME as u64, 1).unwrap();
        assert!(c.take(1).unwrap());
        let mut c = Credits::default();
        for _ in 0..INITIAL_FRAMES {
            assert!(c.take(0).unwrap());
        }
        assert!(!c.take(0).unwrap());
        assert_eq!(c.replenish(0, INITIAL_FRAMES + 1), Err(Error::Credit));
    }

    #[test]
    fn reject_before_payload_allocation() {
        let mut d = Decoder::new(Lane::Data);
        let mut header = vec![1, 0, 0, 0, 1];
        header.extend_from_slice(&u32::try_from(MAX_WORKER_FRAME + 1).unwrap().to_be_bytes());
        header.resize(MAX_WORKER_FRAME * 2, 0);
        assert_eq!(
            d.decode(&header),
            Err(Error::Oversize(MAX_WORKER_FRAME + 1))
        );
        assert_eq!(d.payload.capacity(), 0);
        assert_eq!(d.decode(&[]), Err(Error::Poisoned));
    }

    #[test]
    fn independent_lanes_and_switch() {
        let frame = Frame {
            kind: Kind::Session,
            stream: 1,
            payload: vec![42],
        };
        assert_eq!(encode(&frame, Lane::Control), Err(Error::Lane));
        let bytes = encode(&frame, Lane::Data).unwrap();
        assert_eq!(Decoder::new(Lane::Control).decode(&bytes), Err(Error::Lane));
        for (setting, env, expected) in [
            (None, None, false),
            (Some("on"), None, true),
            (Some("on"), Some("0"), false),
            (None, Some("1"), true),
            (Some("on"), Some("bad"), false),
        ] {
            assert_eq!(enabled(setting, env), expected);
        }
    }

    #[test]
    fn every_control_variant_roundtrips() {
        // Exercise the wire spelling of every discriminant, including optional fields.
        let cases = [
            r#"{"t":"hello","proto":1,"version":"1","pid":1,"key":"k","cwd":"/","extensions":[]}"#,
            r#"{"t":"ready"}"#,
            r#"{"t":"attach","stream":1,"session_target":"s","takeover":true,"ui_caps":{}}"#,
            r#"{"t":"attached","stream":1,"session_id":"s"}"#,
            r#"{"t":"attach_failed","stream":1,"error":"e"}"#,
            r#"{"t":"detach","stream":1}"#,
            r#"{"t":"create","stream":1,"cwd":"/"}"#,
            r#"{"t":"close","id":1,"session_id":"s"}"#,
            r#"{"t":"reply","id":1,"result":null}"#,
            r#"{"t":"ping","id":1}"#,
            r#"{"t":"pong","id":1}"#,
            r#"{"t":"state","live_sessions":0,"services_sets":0,"busy_sessions":0,"keepalive_services":0}"#,
            r#"{"t":"credit","stream":1,"bytes":1,"frames":1}"#,
            r#"{"t":"lagged","stream":1}"#,
            r#"{"t":"index_append","session_id":"s","data":{}}"#,
            r#"{"t":"host_cmd","id":1,"cmd":{}}"#,
            r#"{"t":"host_call","id":1,"service":"browser","op":"o","json":{},"session":"s"}"#,
            r#"{"t":"host_reply","id":1,"result":null}"#,
            r#"{"t":"route","id":1,"op":"o","target":"s","data":{}}"#,
            r#"{"t":"session_replaced","old":"a","new":"b"}"#,
            r#"{"t":"reload_extensions"}"#,
            r#"{"t":"shutdown","reason":"test"}"#,
            r#"{"t":"bye","reason":"idle"}"#,
        ];
        for json in cases {
            let msg: Control = serde_json::from_str(json).unwrap();
            assert_eq!(
                serde_json::from_slice::<Control>(&serde_json::to_vec(&msg).unwrap()).unwrap(),
                msg
            );
        }
    }

    proptest! {
        #[test]
        fn arbitrary_decoder_never_panics(input in prop::collection::vec(any::<u8>(), 0..65536), chunk in 1usize..1024) {
            let mut d = Decoder::new(Lane::Data);
            for part in input.chunks(chunk) {
                let mut offset = 0;
                while offset < part.len() {
                    match d.decode(&part[offset..]) {
                        Ok((n, _)) => { prop_assert!(n > 0); offset += n; }
                        Err(_) => return Ok(()),
                    }
                }
            }
        }

        #[test]
        fn frame_roundtrip(payload in prop::collection::vec(any::<u8>(), 0..16384), stream in any::<u32>(), chunk in 1usize..512) {
            for kind in [Kind::Control, Kind::Session, Kind::Credit, Kind::HostReply] {
                let frame = Frame { kind, stream, payload: payload.clone() };
                let bytes = encode(&frame, kind.lane()).unwrap();
                let mut d = Decoder::new(kind.lane());
                let mut output = None;
                for part in bytes.chunks(chunk) {
                    let (n, value) = d.decode(part).unwrap();
                    prop_assert_eq!(n, part.len());
                    if value.is_some() { output = value; }
                }
                prop_assert_eq!(output, Some(frame));
            }
        }
    }
}
