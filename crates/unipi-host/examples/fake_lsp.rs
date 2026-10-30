//! A scripted stdio language server for `tests/lsp.rs` (WP-A21): reads
//! `Content-Length`-framed JSON-RPC on stdin, answers `initialize`,
//! `textDocument/hover` and `textDocument/completion`, pushes one
//! diagnostic after `didOpen`, honours `shutdown`/`exit`, and appends
//! every received `method` (plus the document uri, when any) one per
//! line to the file named by `argv[1]` — that's how the test sees the
//! host's synthesized `didClose`s and that a cached `initialize` never
//! reaches the server twice.

use std::io::{Read, Write};

fn reply(out: &mut impl Write, id: &serde_json::Value, result: serde_json::Value) {
    let body = serde_json::to_vec(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    }))
    .unwrap();
    write!(out, "Content-Length: {}\r\n\r\n", body.len()).unwrap();
    out.write_all(&body).unwrap();
    out.flush().unwrap();
}

fn notify(out: &mut impl Write, method: &str, params: serde_json::Value) {
    let body = serde_json::to_vec(&serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    }))
    .unwrap();
    write!(out, "Content-Length: {}\r\n\r\n", body.len()).unwrap();
    out.write_all(&body).unwrap();
    out.flush().unwrap();
}

fn log_line(log: &Option<String>, message: &serde_json::Value) {
    let Some(path) = log else { return };
    let method = message
        .get("method")
        .and_then(|m| m.as_str())
        .unwrap_or("<no-method>");
    let uri = message
        .pointer("/params/textDocument/uri")
        .and_then(|u| u.as_str())
        .unwrap_or("");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{method} {uri}");
    }
}

fn main() {
    let log = std::env::args().nth(1);
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    loop {
        // Headers up to the blank line.
        let mut header = Vec::new();
        loop {
            let mut byte = [0u8; 1];
            if stdin.read(&mut byte).unwrap_or(0) == 0 {
                return;
            }
            header.push(byte[0]);
            if header.ends_with(b"\r\n\r\n") {
                break;
            }
            if header.len() > 8192 {
                return;
            }
        }
        let headers = String::from_utf8_lossy(&header);
        let mut len = 0usize;
        for line in headers.split("\r\n") {
            if let Some(value) = line.strip_prefix("Content-Length:") {
                len = value.trim().parse().unwrap_or(0);
            }
        }
        if len == 0 || len > 64 * 1024 * 1024 {
            return;
        }
        let mut body = vec![0u8; len];
        if stdin.read_exact(&mut body).is_err() {
            return;
        }
        let message: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
        log_line(&log, &message);
        match message.get("method").and_then(|m| m.as_str()) {
            Some("initialize") => reply(
                &mut stdout,
                &message["id"],
                serde_json::json!({
                    "capabilities": {
                        "hoverProvider": true,
                        "completionProvider": {"triggerCharacters": ["."]},
                        "signatureHelpProvider": {"triggerCharacters": ["("]},
                        "definitionProvider": true,
                        "referencesProvider": true,
                        "renameProvider": true,
                        "documentFormattingProvider": true,
                        "textDocumentSync": 2
                    },
                    "serverInfo": {"name": "fake_lsp", "version": "1.0"}
                }),
            ),
            Some("initialized") => {}
            Some("shutdown") => reply(&mut stdout, &message["id"], serde_json::Value::Null),
            Some("exit") => std::process::exit(0),
            Some("textDocument/hover") => reply(
                &mut stdout,
                &message["id"],
                serde_json::json!({"contents": {"kind": "markdown", "value": "**fake** hover"}}),
            ),
            Some("textDocument/completion") => reply(
                &mut stdout,
                &message["id"],
                serde_json::json!({"isIncomplete": false, "items": [
                    {"label": "fakeItem", "kind": 6, "detail": "from fake_lsp"}
                ]}),
            ),
            Some("textDocument/didOpen") => {
                let uri = message
                    .pointer("/params/textDocument/uri")
                    .and_then(|u| u.as_str())
                    .unwrap_or("")
                    .to_string();
                notify(
                    &mut stdout,
                    "textDocument/publishDiagnostics",
                    serde_json::json!({
                        "uri": uri,
                        "diagnostics": [{
                            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 4}},
                            "severity": 2,
                            "message": "fake diagnostic"
                        }]
                    }),
                );
            }
            Some(_) => {
                // Requests we don't script still get a null response so the
                // client's correlation never stalls.
                if message.get("id").is_some() {
                    reply(&mut stdout, &message["id"], serde_json::Value::Null);
                }
            }
            None => {
                if message.get("id").is_some() {
                    reply(&mut stdout, &message["id"], serde_json::Value::Null);
                }
            }
        }
    }
}
