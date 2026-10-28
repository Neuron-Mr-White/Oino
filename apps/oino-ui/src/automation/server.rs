//! The Unix-socket server half of the automation layer (docs/dev/v1/gui-driver.md §2).
//!
//! A plain OS thread owns the listening socket and one client connection at a time; each
//! request is handed to the UI thread over an unbounded channel and the reply comes back on a
//! oneshot. Nothing here touches GPUI, so it is unit-testable with a fake handler.
//!
//! Unix only (a Unix-domain socket): Windows builds have no server and `install` is a no-op.
#![cfg(unix)]

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::fs::FileTypeExt as _;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use super::protocol::{Request, error};

/// One request waiting for the UI thread.
pub struct Call {
    /// The parsed request.
    pub request: Request,
    /// Where the UI thread writes the reply (`id`/`ok`/`result` already resolved).
    pub reply: oneshot::Sender<Value>,
}

/// A running server: its socket path and stop flag.
pub struct Server {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    /// The socket path clients connect to.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Unblock `accept` (the listener is non-blocking, so the loop notices the flag).
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Binds `path` and starts answering; the returned receiver feeds the UI thread.
///
/// An existing file at `path` is removed first when it is a socket (a crashed run leaves one).
///
/// # Errors
///
/// `Err` is the OS error from creating the parent directory or binding the socket.
pub fn spawn(path: &Path) -> std::io::Result<(Server, mpsc::UnboundedReceiver<Call>)> {
    if let Ok(meta) = std::fs::symlink_metadata(path)
        && meta.file_type().is_socket()
    {
        let _ = std::fs::remove_file(path);
    }
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir)?;
    }
    let listener = UnixListener::bind(path)?;
    listener.set_nonblocking(true)?;
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::unbounded_channel::<Call>();
    let stop_thread = Arc::clone(&stop);
    let thread = std::thread::Builder::new()
        .name("oino-automation".into())
        .spawn(move || accept_loop(&listener, &tx, &stop_thread))?;
    Ok((
        Server {
            path: path.to_path_buf(),
            stop,
            thread: Some(thread),
        },
        rx,
    ))
}

/// Accepts one connection at a time and serves its requests in order.
fn accept_loop(listener: &UnixListener, tx: &mpsc::UnboundedSender<Call>, stop: &AtomicBool) {
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                let _ = stream.set_nonblocking(false);
                serve(stream, tx);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

/// Serves one client until it closes the connection.
fn serve(stream: UnixStream, tx: &mpsc::UnboundedSender<Call>) {
    let Ok(read_half) = stream.try_clone() else {
        return;
    };
    let mut writer = stream;
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let reply = match Request::parse(trimmed) {
            Ok(request) => {
                let id = request.id.clone();
                let (reply_tx, reply_rx) = oneshot::channel();
                if tx
                    .send(Call {
                        request,
                        reply: reply_tx,
                    })
                    .is_err()
                {
                    error(&id, "the app is shutting down")
                } else {
                    reply_rx
                        .blocking_recv()
                        .unwrap_or_else(|_| error(&id, "the request was dropped"))
                }
            }
            Err(e) => error(&Value::Null, e.0),
        };
        if writeln!(writer, "{reply}").is_err() || writer.flush().is_err() {
            break;
        }
        if reply
            .get("result")
            .and_then(|r| r.get("quitting"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;
    use std::os::unix::net::UnixStream;

    /// A temp socket path unique to this test process.
    fn socket_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "oino-automation-test-{}-{tag}.sock",
            std::process::id()
        ))
    }

    #[test]
    fn round_trips_requests_in_order() {
        let path = socket_path("roundtrip");
        let (server, mut rx) = spawn(&path).expect("bind");
        // Fake UI thread: answer every call with its method name.
        let handler = std::thread::spawn(move || {
            while let Some(call) = rx.blocking_recv() {
                let name = call.request.method.clone();
                let id = call.request.id.clone();
                let payload = super::super::protocol::ok(&id, &serde_json::json!({"method": name}));
                if call.reply.send(payload).is_err() {
                    break;
                }
            }
        });
        let mut client = UnixStream::connect(server.path()).expect("connect");
        client
            .write_all(b"{\"id\":1,\"method\":\"tree\"}\n{\"id\":2,\"method\":\"state\"}\n")
            .expect("write");
        client.flush().expect("flush");
        let mut reader = BufReader::new(client.try_clone().expect("clone"));
        for (want_id, want_method) in [(1, "tree"), (2, "state")] {
            let mut line = String::new();
            reader.read_line(&mut line).expect("read");
            let value: Value = serde_json::from_str(&line).expect("json");
            assert_eq!(value["id"], serde_json::json!(want_id));
            assert_eq!(value["ok"], serde_json::json!(true));
            assert_eq!(value["result"]["method"], serde_json::json!(want_method));
        }
        drop(reader);
        drop(client);
        drop(server);
        drop(handler);
    }

    #[test]
    fn bad_json_gets_an_error_line_and_the_connection_survives() {
        let path = socket_path("badjson");
        let (server, mut rx) = spawn(&path).expect("bind");
        let handler = std::thread::spawn(move || {
            while let Some(call) = rx.blocking_recv() {
                let id = call.request.id.clone();
                let payload = super::super::protocol::ok(&id, &serde_json::json!({"ok": 1}));
                if call.reply.send(payload).is_err() {
                    break;
                }
            }
        });
        let mut client = UnixStream::connect(server.path()).expect("connect");
        client.write_all(b"not json\n").expect("write");
        client
            .write_all(b"{\"id\":9,\"method\":\"ping\"}\n")
            .expect("write");
        client.flush().expect("flush");
        let mut reader = BufReader::new(client.try_clone().expect("clone"));
        let mut line = String::new();
        reader.read_line(&mut line).expect("read");
        let value: Value = serde_json::from_str(&line).expect("json");
        assert_eq!(value["ok"], serde_json::json!(false));
        assert!(
            value["error"]["message"]
                .as_str()
                .is_some_and(|m| m.contains("invalid JSON")),
            "{value}"
        );
        line.clear();
        reader.read_line(&mut line).expect("read");
        let value: Value = serde_json::from_str(&line).expect("json");
        assert_eq!(value["result"]["ok"], serde_json::json!(1));
        drop((reader, client, server, handler));
    }
}
