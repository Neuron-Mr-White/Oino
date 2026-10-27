//! CDP transports: `--remote-debugging-pipe` (Unix, NUL-delimited JSON on fds 3/4) and a
//! loopback WebSocket (Windows, `browser.externalCdp`, tests).

use futures::{SinkExt as _, StreamExt as _};
use tokio::sync::mpsc;

/// A connected transport, consumed by [`super::Cdp::start`].
pub(crate) enum Transport {
    /// Pipe pair (our ends).
    #[cfg(unix)]
    Pipe {
        /// Reads what the browser writes on its fd 4.
        read: std::io::PipeReader,
        /// Writes what the browser reads on its fd 3.
        write: std::io::PipeWriter,
    },
    /// Loopback WebSocket URL (`ws://127.0.0.1:<port>/devtools/browser/<id>`).
    WebSocket(
        Box<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
        >,
    ),
    /// In-memory channels (tests).
    #[cfg(test)]
    Memory {
        /// Messages to the fake browser.
        to_browser: mpsc::UnboundedSender<String>,
        /// Messages from the fake browser.
        from_browser: mpsc::UnboundedReceiver<String>,
    },
}

impl Transport {
    /// Connects to a `DevTools` WebSocket URL.
    pub(crate) async fn connect_ws(url: &str) -> Result<Self, String> {
        let (ws, _) = tokio_tungstenite::connect_async(url)
            .await
            .map_err(|e| e.to_string())?;
        Ok(Self::WebSocket(Box::new(ws)))
    }

    /// Test transport: returns it plus the fake browser's (inbox, outbox).
    #[cfg(test)]
    pub(crate) fn memory() -> (
        Self,
        mpsc::UnboundedReceiver<String>,
        mpsc::UnboundedSender<String>,
    ) {
        let (to_browser, browser_in) = mpsc::unbounded_channel();
        let (browser_out, from_browser) = mpsc::unbounded_channel();
        (
            Self::Memory {
                to_browser,
                from_browser,
            },
            browser_in,
            browser_out,
        )
    }

    /// Pumps `out` to the browser and browser messages into `incoming` until either side
    /// closes. Dropping `incoming`'s sender signals disconnect.
    pub(crate) fn spawn(
        self,
        mut out: mpsc::UnboundedReceiver<String>,
        incoming: mpsc::UnboundedSender<String>,
    ) {
        match self {
            #[cfg(unix)]
            Self::Pipe { read, write } => {
                std::thread::Builder::new()
                    .name("oino-cdp-read".into())
                    .spawn(move || read_pipe(read, &incoming))
                    .map_err(|e| tracing::warn!(%e, "cdp reader thread"))
                    .ok();
                std::thread::Builder::new()
                    .name("oino-cdp-write".into())
                    .spawn(move || write_pipe(write, &mut out))
                    .map_err(|e| tracing::warn!(%e, "cdp writer thread"))
                    .ok();
            }
            Self::WebSocket(ws) => {
                let (mut sink, mut stream) = (*ws).split();
                tokio::spawn(async move {
                    while let Some(text) = out.recv().await {
                        if sink
                            .send(tokio_tungstenite::tungstenite::Message::text(text))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    let _ = sink.close().await;
                });
                tokio::spawn(async move {
                    while let Some(Ok(msg)) = stream.next().await {
                        if let tokio_tungstenite::tungstenite::Message::Text(t) = msg
                            && incoming.send(t.to_string()).is_err()
                        {
                            break;
                        }
                    }
                });
            }
            #[cfg(test)]
            Self::Memory {
                to_browser,
                mut from_browser,
            } => {
                tokio::spawn(async move {
                    while let Some(text) = out.recv().await {
                        if to_browser.send(text).is_err() {
                            break;
                        }
                    }
                });
                tokio::spawn(async move {
                    while let Some(text) = from_browser.recv().await {
                        if incoming.send(text).is_err() {
                            break;
                        }
                    }
                });
            }
        }
    }
}

#[cfg(unix)]
fn read_pipe(read: std::io::PipeReader, incoming: &mpsc::UnboundedSender<String>) {
    use std::io::BufRead as _;
    let mut reader = std::io::BufReader::with_capacity(1 << 20, read);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(0, &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                if buf.last() == Some(&0) {
                    buf.pop();
                }
                let text = String::from_utf8_lossy(&buf).into_owned();
                if incoming.send(text).is_err() {
                    break;
                }
            }
        }
    }
}

#[cfg(unix)]
fn write_pipe(mut write: std::io::PipeWriter, out: &mut mpsc::UnboundedReceiver<String>) {
    use std::io::Write as _;
    while let Some(text) = out.blocking_recv() {
        if write.write_all(text.as_bytes()).is_err()
            || write.write_all(&[0]).is_err()
            || write.flush().is_err()
        {
            break;
        }
    }
}
