//! Port forwarding wire types (UNI-213, "forwarding phase 1").
//! Mirrors `packages/protocol/src/forward.ts`.
//!
//! `ports_list` stream (request/reply):
//! - client → `hello{req:{op:"ports_list"}}`
//! - host → `ok{data: PortsListData}`, or `err{E_FORWARD_DISABLED}` when the
//!   host has forwarding switched off (the default).
//!
//! `tcp_forward` stream (like `ssh -L`):
//! - client → `hello{req:{op:"tcp_forward", port}}`, then (optionally
//!   pipelined, without waiting for the reply) RAW bytes — no frames.
//! - host → `ok{}` once it has connected to `127.0.0.1:port` (or `[::1]`),
//!   then RAW bytes both ways until either side closes. Errors before that
//!   are one `err` envelope: `E_FORWARD_DISABLED` (off / this device not
//!   allowed), `E_UNAUTHORIZED` (port not in the allow-list), `E_NOT_FOUND`
//!   (nothing listening on that port).
//!
//! The host only ever connects to loopback.

use serde::{Deserialize, Serialize};

/// A TCP port, 1..=65535.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Hash)]
#[serde(transparent)]
pub struct ForwardPort(u16);

impl ForwardPort {
    pub fn value(self) -> u16 {
        self.0
    }
}

impl TryFrom<u16> for ForwardPort {
    type Error = String;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        if value == 0 {
            Err("port must be 1..=65535".to_string())
        } else {
            Ok(ForwardPort(value))
        }
    }
}

impl<'de> Deserialize<'de> for ForwardPort {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = u16::deserialize(deserializer)?;
        ForwardPort::try_from(raw).map_err(serde::de::Error::custom)
    }
}

/// One listening TCP socket reachable over loopback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListeningPort {
    pub port: u16,
    /// Bind address as text: `127.0.0.1`, `0.0.0.0`, `::1`, `::`.
    pub bind: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub pid: Option<u32>,
    /// Process name (`/proc/<pid>/comm`) when readable.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub process: Option<String>,
    /// Command line (truncated) when readable.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cmd: Option<String>,
    /// Looks like a web server (heuristic from the process/port).
    #[serde(default)]
    pub http: bool,
    /// The host's allow-list lets this device forward it.
    #[serde(default)]
    pub allowed: bool,
}

/// `ok.data` for `ports_list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortsListData {
    pub ports: Vec<ListeningPort>,
    /// `true` when any loopback port may be forwarded.
    pub any: bool,
    /// The explicit allow-list (empty when `any`).
    #[serde(default)]
    pub allowed: Vec<u16>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_bounds() {
        assert!(serde_json::from_str::<ForwardPort>("0").is_err());
        assert!(serde_json::from_str::<ForwardPort>("65536").is_err());
        assert_eq!(
            serde_json::from_str::<ForwardPort>("5173").unwrap().value(),
            5173
        );
    }
}
