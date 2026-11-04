//! `http`: outbound requests through one egress choke point (D7) and a loopback listener.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use futures::stream::BoxStream;
use oino_extension_api::HostError;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

use crate::state::lock;

/// Outbound request (WIT `request`, auth already resolved into headers).
#[derive(Clone, Debug)]
pub(crate) struct Request {
    pub(crate) method: reqwest::Method,
    pub(crate) url: reqwest::Url,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Option<Vec<u8>>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) follow_redirects: bool,
    pub(crate) proxy: Option<String>,
}

/// Response head (WIT `response-head`).
#[derive(Clone, Debug)]
pub(crate) struct Head {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) final_url: String,
}

/// reqwest clients keyed by (redirect policy, proxy).
pub(crate) struct HttpService {
    default_proxy: Option<String>,
    idle_timeout: Duration,
    clients: Mutex<HashMap<(bool, Option<String>), reqwest::Client>>,
}

impl HttpService {
    pub(crate) fn new(default_proxy: Option<String>, idle_timeout: Duration) -> Self {
        Self {
            default_proxy,
            idle_timeout,
            clients: Mutex::new(HashMap::new()),
        }
    }

    fn client(&self, follow: bool, proxy: Option<String>) -> Result<reqwest::Client, HostError> {
        let proxy = proxy.or_else(|| self.default_proxy.clone());
        let key = (follow, proxy.clone());
        if let Some(client) = lock(&self.clients).get(&key) {
            return Ok(client.clone());
        }
        let mut builder = reqwest::Client::builder().redirect(if follow {
            reqwest::redirect::Policy::limited(10)
        } else {
            reqwest::redirect::Policy::none()
        });
        if let Some(proxy) = &proxy {
            builder = builder.proxy(
                reqwest::Proxy::all(proxy)
                    .map_err(|e| HostError::InvalidArgument(format!("proxy: {e}")))?,
            );
        }
        let client = builder
            .build()
            .map_err(|e| HostError::Failed(e.to_string()))?;
        lock(&self.clients).insert(key, client.clone());
        Ok(client)
    }

    /// Sends `req`, returning the head and the streaming body.
    ///
    /// The request timeout (default: the service idle timeout) works like pi's undici
    /// `headersTimeout` + `bodyTimeout`: it bounds the wait for the response headers, then the
    /// wait for every body chunk. An active stream is never cut for its total length (owner
    /// lerp graph 2026-11-03: two nodes died exactly 300 s into a live stream).
    pub(crate) async fn fetch(
        &self,
        req: Request,
    ) -> Result<(Head, BoxStream<'static, Vec<u8>>), HostError> {
        let client = self.client(req.follow_redirects, req.proxy.clone())?;
        let idle = req.timeout.unwrap_or(self.idle_timeout);
        let mut builder = client.request(req.method, req.url);
        let mut cut_marker = false;
        for (k, v) in &req.headers {
            if k.eq_ignore_ascii_case(BODY_ERRORS_HEADER) {
                cut_marker = true;
                continue;
            }
            builder = builder.header(k, v);
        }
        if let Some(body) = req.body {
            builder = builder.body(body);
        }
        let resp = match tokio::time::timeout(idle, builder.send()).await {
            Err(_) => return Err(HostError::Timeout),
            Ok(Err(e)) if e.is_timeout() => return Err(HostError::Timeout),
            Ok(Err(e)) => return Err(HostError::Failed(e.to_string())),
            Ok(Ok(resp)) => resp,
        };
        let head = Head {
            status: resp.status().as_u16(),
            headers: resp
                .headers()
                .iter()
                .map(|(k, v)| {
                    (
                        k.as_str().to_owned(),
                        String::from_utf8_lossy(v.as_bytes()).into_owned(),
                    )
                })
                .collect(),
            final_url: resp.url().to_string(),
        };
        let origin = head
            .final_url
            .split('?')
            .next()
            .unwrap_or_default()
            .to_owned();
        Ok((
            head,
            idle_body(resp.bytes_stream(), idle, cut_marker, origin),
        ))
    }
}

/// Request header (stripped before sending) by which a guest asks for a [`cut_marker`] at the
/// end of a body that failed. The WIT body is a plain byte stream that ends like EOF on an
/// error, so without it the guest parses the cut-off tail as data (`malformed server-sent
/// event JSON`). The provider kit (`oino-provider-kit` `body.rs`) sends it and decodes the
/// marker; other guests keep the plain stream.
pub(crate) const BODY_ERRORS_HEADER: &str = "x-oino-body-errors";
/// Leading bytes of the cut marker (mirrors `oino-provider-kit` `body::CUT_MAGIC`).
const CUT_MAGIC: &[u8; 12] = b"\0\xffOINO-CUT\xff\0";

/// Why a response body ended early.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cut {
    /// No chunk within the idle timeout.
    Idle(Duration),
    /// The connection failed (reset, protocol error).
    Reset,
}

/// Fixed-size marker: [`CUT_MAGIC`], kind (`i` idle / `r` reset), idle milliseconds (u32 LE).
fn cut_marker(cut: Cut) -> Vec<u8> {
    let (kind, ms) = match cut {
        Cut::Idle(idle) => (b'i', u32::try_from(idle.as_millis()).unwrap_or(u32::MAX)),
        Cut::Reset => (b'r', 0),
    };
    let mut marker = CUT_MAGIC.to_vec();
    marker.push(kind);
    marker.extend_from_slice(&ms.to_le_bytes());
    marker
}

/// Body chunks with a per-chunk idle timeout. A read error or an idle timeout ends the stream
/// (with a [`cut_marker`] when `marked`) and is logged with its cause.
fn idle_body<S, E>(
    inner: S,
    idle: Duration,
    marked: bool,
    origin: String,
) -> BoxStream<'static, Vec<u8>>
where
    S: futures::Stream<Item = Result<bytes::Bytes, E>> + Send + 'static,
    E: std::fmt::Display + Send + 'static,
{
    let inner = Box::pin(inner);
    Box::pin(futures::stream::unfold(Some(inner), move |state| {
        let origin = origin.clone();
        async move {
            let mut inner = state?;
            let cut = match tokio::time::timeout(idle, inner.next()).await {
                Ok(Some(Ok(chunk))) => return Some((chunk.to_vec(), Some(inner))),
                Ok(None) => return None,
                Ok(Some(Err(e))) => {
                    tracing::warn!(url = %origin, "http response body ended early: {e}");
                    Cut::Reset
                }
                Err(_) => {
                    tracing::warn!(
                        url = %origin,
                        idle_s = idle.as_secs(),
                        "http response body idle, closing it"
                    );
                    Cut::Idle(idle)
                }
            };
            marked.then(|| (cut_marker(cut), None))
        }
    }))
}

/// One accepted loopback request (WIT `incoming`).
#[derive(Clone, Debug)]
pub(crate) struct Incoming {
    pub(crate) id: u64,
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

/// Loopback HTTP/1.1 listener: one request per connection, answered by `respond`.
pub(crate) struct Listener {
    inner: TcpListener,
    port: u16,
    pending: Arc<Mutex<HashMap<u64, TcpStream>>>,
    next: Mutex<u64>,
}

const MAX_BODY: usize = 16 * 1024 * 1024;

impl Listener {
    pub(crate) fn from_std(listener: std::net::TcpListener) -> Result<Self, HostError> {
        let inner = TcpListener::from_std(listener).map_err(|e| HostError::io(&e))?;
        let port = inner.local_addr().map_err(|e| HostError::io(&e))?.port();
        Ok(Self {
            inner,
            port,
            pending: Arc::default(),
            next: Mutex::new(0),
        })
    }

    pub(crate) fn port(&self) -> u16 {
        self.port
    }

    /// Waits for the next well-formed request (`None` when the listener fails).
    pub(crate) async fn accept(&self) -> Option<Incoming> {
        loop {
            let (stream, _) = self.inner.accept().await.ok()?;
            if let Some((parts, stream)) = read_request(stream).await {
                let id = {
                    let mut next = lock(&self.next);
                    *next += 1;
                    *next
                };
                lock(&self.pending).insert(id, stream);
                let (method, path, headers, body) = parts;
                return Some(Incoming {
                    id,
                    method,
                    path,
                    headers,
                    body,
                });
            }
        }
    }

    pub(crate) fn respond(
        &self,
        id: u64,
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> Result<(), HostError> {
        let mut stream = lock(&self.pending)
            .remove(&id)
            .ok_or_else(|| HostError::NotFound(format!("request {id}")))?;
        tokio::spawn(async move {
            let mut head = format!(
                "HTTP/1.1 {status} {}\r\ncontent-length: {}\r\nconnection: close\r\n",
                reason(status),
                body.len()
            );
            for (k, v) in headers {
                let _ = write!(head, "{k}: {v}\r\n");
            }
            head.push_str("\r\n");
            let _ = stream.write_all(head.as_bytes()).await;
            let _ = stream.write_all(&body).await;
            let _ = stream.shutdown().await;
        });
        Ok(())
    }
}

type Parts = (String, String, Vec<(String, String)>, Vec<u8>);

async fn read_request(stream: TcpStream) -> Option<(Parts, TcpStream)> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).await.ok()?;
    let mut it = line.split_whitespace();
    let method = it.next()?.to_owned();
    let path = it.next()?.to_owned();
    let mut headers = Vec::new();
    let mut length = 0usize;
    loop {
        let mut h = String::new();
        reader.read_line(&mut h).await.ok()?;
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        let (k, v) = h.split_once(':')?;
        let (k, v) = (k.trim().to_owned(), v.trim().to_owned());
        if k.eq_ignore_ascii_case("content-length") {
            length = v.parse().ok()?;
        }
        headers.push((k, v));
    }
    if length > MAX_BODY {
        return None;
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).await.ok()?;
    Some(((method, path, headers, body), reader.into_inner()))
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        302 => "Found",
        400 => "Bad Request",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Status",
    }
}

/// Host of a URL for allowlist checks.
pub(crate) fn host_of(url: &reqwest::Url) -> Option<String> {
    url.host_str().map(str::to_ascii_lowercase)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn std_listener() -> std::net::TcpListener {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.set_nonblocking(true).unwrap();
        l
    }

    #[tokio::test]
    async fn listener_round_trip_and_fetch() {
        let listener = Arc::new(Listener::from_std(std_listener()).unwrap());
        let port = listener.port();
        let l = Arc::clone(&listener);
        let server = tokio::spawn(async move {
            let req = l.accept().await.unwrap();
            assert_eq!(
                (req.method.as_str(), req.path.as_str(), req.body.as_slice()),
                ("POST", "/x", &b"hi"[..])
            );
            l.respond(
                req.id,
                201,
                vec![("x-a".into(), "1".into())],
                b"done".to_vec(),
            )
            .unwrap();
        });
        let http = HttpService::new(None, Duration::from_secs(5));
        let (head, body) = http
            .fetch(Request {
                method: reqwest::Method::POST,
                url: format!("http://127.0.0.1:{port}/x").parse().unwrap(),
                headers: vec![],
                body: Some(b"hi".to_vec()),
                timeout: Some(Duration::from_secs(5)),
                follow_redirects: true,
                proxy: None,
            })
            .await
            .unwrap();
        assert_eq!(head.status, 201);
        assert!(head.headers.iter().any(|(k, v)| k == "x-a" && v == "1"));
        assert_eq!(body.concat().await, b"done");
        server.await.unwrap();
    }

    /// Serves one request with `respond`, returning the request head it read.
    async fn serve_once<F, Fut>(respond: F) -> (u16, tokio::task::JoinHandle<String>)
    where
        F: FnOnce(TcpStream) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(stream);
            let mut head = String::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();
                if line.trim_end().is_empty() {
                    break;
                }
                head.push_str(&line);
            }
            respond(reader.into_inner()).await;
            head
        });
        (port, task)
    }

    async fn get(port: u16, timeout: Duration, marked: bool) -> Vec<u8> {
        let http = HttpService::new(None, Duration::from_secs(30));
        let headers = if marked {
            vec![(BODY_ERRORS_HEADER.to_owned(), "1".to_owned())]
        } else {
            vec![]
        };
        let (head, body) = http
            .fetch(Request {
                method: reqwest::Method::GET,
                url: format!("http://127.0.0.1:{port}/s").parse().unwrap(),
                headers,
                body: None,
                timeout: Some(timeout),
                follow_redirects: true,
                proxy: None,
            })
            .await
            .unwrap();
        assert_eq!(head.status, 200);
        body.concat().await
    }

    const CHUNKED: &[u8] = b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n";

    async fn chunk(stream: &mut TcpStream, data: &[u8]) {
        let frame = format!("{:x}\r\n", data.len());
        stream.write_all(frame.as_bytes()).await.unwrap();
        stream.write_all(data).await.unwrap();
        stream.write_all(b"\r\n").await.unwrap();
        stream.flush().await.unwrap();
    }

    #[tokio::test]
    async fn an_active_stream_outlives_the_timeout() {
        let (port, server) = serve_once(|mut s| async move {
            s.write_all(CHUNKED).await.unwrap();
            for _ in 0..12 {
                chunk(&mut s, b"data: x\n\n").await;
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            s.write_all(b"0\r\n\r\n").await.unwrap();
        })
        .await;
        // 12 chunks over ~600 ms with a 250 ms timeout: only the gaps count.
        let body = get(port, Duration::from_millis(250), true).await;
        assert_eq!(body, b"data: x\n\n".repeat(12));
        let head = server.await.unwrap().to_ascii_lowercase();
        assert!(
            !head.contains(BODY_ERRORS_HEADER),
            "marker header leaked: {head}"
        );
    }

    #[tokio::test]
    async fn a_stalled_stream_ends_with_an_idle_marker() {
        let (port, server) = serve_once(|mut s| async move {
            s.write_all(CHUNKED).await.unwrap();
            chunk(&mut s, b"data: {\"a\"").await;
            tokio::time::sleep(Duration::from_secs(2)).await;
        })
        .await;
        let body = get(port, Duration::from_millis(200), true).await;
        let mut expected = b"data: {\"a\"".to_vec();
        expected.extend(cut_marker(Cut::Idle(Duration::from_millis(200))));
        assert_eq!(body, expected);
        server.abort();
    }

    #[tokio::test]
    async fn a_cut_body_ends_with_a_reset_marker_only_when_asked() {
        for marked in [true, false] {
            let (port, server) = serve_once(|mut s| async move {
                s.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 100\r\n\r\ndata: {\"a\"")
                    .await
                    .unwrap();
                s.shutdown().await.unwrap();
            })
            .await;
            let body = get(port, Duration::from_secs(5), marked).await;
            let mut expected = b"data: {\"a\"".to_vec();
            if marked {
                expected.extend(cut_marker(Cut::Reset));
            }
            assert_eq!(body, expected);
            server.await.unwrap();
        }
    }

    #[test]
    fn cut_marker_layout_matches_the_provider_kit() {
        let mut idle = b"\0\xffOINO-CUT\xff\0i".to_vec();
        idle.extend_from_slice(&300_000u32.to_le_bytes());
        assert_eq!(cut_marker(Cut::Idle(Duration::from_secs(300))), idle);
        assert_eq!(cut_marker(Cut::Reset).len(), 17);
    }
}
