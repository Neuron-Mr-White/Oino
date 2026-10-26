//! JSON-RPC 2.0 framing for MCP (unipi `src/bridge/client.ts` message handling) and the
//! streamable-HTTP `text/event-stream` parser.

use serde_json::{Value, json};

/// MCP protocol version sent in `initialize`.
pub(crate) const PROTOCOL_VERSION: &str = "2025-06-18";

/// One incoming message.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Incoming {
    /// Response to request `id`.
    Response {
        /// Request id.
        id: u64,
        /// `result` or `Err("MCP error <code>: <message>")`.
        outcome: Result<Value, String>,
    },
    /// Server → client request (`ping`, `roots/list`, …).
    Request {
        /// Raw id (echoed back).
        id: Value,
        /// Method.
        method: String,
    },
    /// Notification (ignored, like unipi).
    Notification {
        /// Method.
        method: String,
    },
}

/// A request line (`{"jsonrpc":"2.0","id":…,"method":…,"params":…}`).
pub(crate) fn request(id: u64, method: &str, params: &Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string()
}

/// A notification line.
pub(crate) fn notification(method: &str, params: &Value) -> String {
    json!({"jsonrpc": "2.0", "method": method, "params": params}).to_string()
}

/// `initialize` params.
pub(crate) fn initialize_params(client_version: &str) -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": {"roots": {"listChanged": false}},
        "clientInfo": {"name": "oino-unipi-mcp", "version": client_version},
    })
}

/// Our reply to a server request: `ping` → `{}`, `roots/list` → no roots, else -32601.
pub(crate) fn reply_to_server(id: &Value, method: &str) -> String {
    match method {
        "ping" => json!({"jsonrpc": "2.0", "id": id, "result": {}}).to_string(),
        "roots/list" => json!({"jsonrpc": "2.0", "id": id, "result": {"roots": []}}).to_string(),
        _ => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("Method not found: {method}")}}).to_string(),
    }
}

/// Classifies one decoded message; `None` for junk.
pub(crate) fn classify(msg: &Value) -> Option<Incoming> {
    let method = msg.get("method").and_then(Value::as_str);
    let id = msg.get("id").filter(|i| !i.is_null());
    match (method, id) {
        (Some(m), Some(id)) => Some(Incoming::Request {
            id: id.clone(),
            method: m.to_owned(),
        }),
        (Some(m), None) => Some(Incoming::Notification {
            method: m.to_owned(),
        }),
        (None, Some(id)) => {
            let id = id
                .as_u64()
                .or_else(|| id.as_str().and_then(|s| s.parse().ok()))?;
            let outcome = match msg.get("error") {
                Some(e) => Err(format!(
                    "MCP error {}: {}",
                    e.get("code").and_then(Value::as_i64).unwrap_or(0),
                    e.get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown error")
                )),
                None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
            };
            Some(Incoming::Response { id, outcome })
        }
        (None, None) => None,
    }
}

/// Newline-delimited JSON accumulator (unipi `handleStdoutData`): malformed lines skipped.
#[derive(Debug, Default)]
pub(crate) struct LineBuffer {
    buf: Vec<u8>,
}

impl LineBuffer {
    /// Feeds bytes; returns the complete messages.
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Vec<Value> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(pos) = self.buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=pos).collect();
            let text = String::from_utf8_lossy(&line);
            let t = text.trim();
            if t.is_empty() {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<Value>(t) {
                out.push(v);
            }
        }
        out
    }
}

/// `text/event-stream` accumulator: yields the JSON of each `data:` event.
#[derive(Debug, Default)]
pub(crate) struct SseBuffer {
    buf: String,
    data: Vec<String>,
}

impl SseBuffer {
    /// Feeds text; returns completed events' JSON payloads.
    pub(crate) fn push(&mut self, text: &str) -> Vec<Value> {
        self.buf.push_str(text);
        let mut out = Vec::new();
        while let Some(pos) = self.buf.find('\n') {
            let line: String = self.buf.drain(..=pos).collect();
            let line = line.trim_end_matches(['\n', '\r']);
            if line.is_empty() {
                if !self.data.is_empty() {
                    let payload = self.data.join("\n");
                    self.data.clear();
                    if let Ok(v) = serde_json::from_str::<Value>(&payload) {
                        out.push(v);
                    }
                }
            } else if let Some(d) = line.strip_prefix("data:") {
                self.data.push(d.strip_prefix(' ').unwrap_or(d).to_owned());
            }
        }
        out
    }

    /// Flushes a trailing event without a blank line.
    pub(crate) fn finish(&mut self) -> Vec<Value> {
        let mut out = self.push("\n");
        if !self.data.is_empty() {
            out.extend(serde_json::from_str::<Value>(&self.data.join("\n")));
            self.data.clear();
        }
        out
    }
}

/// Messages of a JSON body (single object or batch array).
pub(crate) fn json_body(body: &[u8]) -> Vec<Value> {
    match serde_json::from_slice::<Value>(body) {
        Ok(Value::Array(items)) => items,
        Ok(v) => vec![v],
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_messages() {
        assert_eq!(
            classify(&json!({"jsonrpc": "2.0", "id": 3, "result": {"tools": []}})),
            Some(Incoming::Response {
                id: 3,
                outcome: Ok(json!({"tools": []}))
            })
        );
        assert_eq!(
            classify(
                &json!({"jsonrpc": "2.0", "id": 4, "error": {"code": -32602, "message": "bad"}})
            ),
            Some(Incoming::Response {
                id: 4,
                outcome: Err("MCP error -32602: bad".into())
            })
        );
        assert_eq!(
            classify(&json!({"jsonrpc": "2.0", "id": "x1", "method": "ping"})),
            Some(Incoming::Request {
                id: json!("x1"),
                method: "ping".into()
            })
        );
        assert_eq!(
            classify(&json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"})),
            Some(Incoming::Notification {
                method: "notifications/tools/list_changed".into()
            })
        );
        assert_eq!(classify(&json!({"jsonrpc": "2.0"})), None);
    }

    #[test]
    fn line_framing_skips_junk() {
        let mut b = LineBuffer::default();
        assert!(b.push(b"{\"id\":1,").is_empty());
        let out = b.push(b"\"result\":{}}\nnot json\n\n{\"id\":2,\"result\":1}\n{\"id\":3");
        assert_eq!(
            out,
            vec![
                json!({"id": 1, "result": {}}),
                json!({"id": 2, "result": 1})
            ]
        );
        assert_eq!(b.push(b"}\n"), vec![json!({"id": 3})]);
    }

    #[test]
    fn sse_framing() {
        let mut s = SseBuffer::default();
        let out = s.push("event: message\ndata: {\"id\":1,\n");
        assert!(out.is_empty());
        let out = s.push("data: \"result\":{}}\n\n: comment\ndata: {\"id\":2}\r\n\r\n");
        assert_eq!(out, vec![json!({"id": 1, "result": {}}), json!({"id": 2})]);
        s.push("data: {\"id\":9}");
        assert_eq!(s.finish(), vec![json!({"id": 9})]);
    }

    #[test]
    fn requests_and_replies() {
        let r: Value =
            serde_json::from_str(&request(1, "tools/list", &json!({}))).unwrap_or_default();
        assert_eq!(r["method"], "tools/list");
        assert_eq!(r["id"], 1);
        let n: Value = serde_json::from_str(&notification("notifications/initialized", &json!({})))
            .unwrap_or_default();
        assert!(n.get("id").is_none());
        let ping: Value =
            serde_json::from_str(&reply_to_server(&json!(7), "ping")).unwrap_or_default();
        assert_eq!(ping["result"], json!({}));
        let other: Value =
            serde_json::from_str(&reply_to_server(&json!(7), "sampling/createMessage"))
                .unwrap_or_default();
        assert_eq!(other["error"]["code"], -32601);
        assert_eq!(json_body(b"[{\"id\":1},{\"id\":2}]").len(), 2);
        assert_eq!(json_body(b"{\"id\":1}").len(), 1);
        assert!(json_body(b"nope").is_empty());
        assert_eq!(
            initialize_params("3.0.0")["protocolVersion"],
            PROTOCOL_VERSION
        );
    }
}
