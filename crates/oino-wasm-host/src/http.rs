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
        let mut builder = reqwest::Client::builder()
            .redirect(if follow {
                reqwest::redirect::Policy::limited(10)
            } else {
                reqwest::redirect::Policy::none()
            })
            .read_timeout(self.idle_timeout);
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
    pub(crate) async fn fetch(
        &self,
        req: Request,
    ) -> Result<(Head, BoxStream<'static, Vec<u8>>), HostError> {
        let client = self.client(req.follow_redirects, req.proxy.clone())?;
        let mut builder = client.request(req.method, req.url);
        for (k, v) in &req.headers {
            builder = builder.header(k, v);
        }
        if let Some(body) = req.body {
            builder = builder.body(body);
        }
        if let Some(t) = req.timeout {
            builder = builder.timeout(t);
        }
        let resp = builder.send().await.map_err(|e| {
            if e.is_timeout() {
                HostError::Timeout
            } else {
                HostError::Failed(e.to_string())
            }
        })?;
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
        let body = resp
            .bytes_stream()
            .take_while(|r| futures::future::ready(r.is_ok()))
            .filter_map(|r| futures::future::ready(r.ok().map(|b| b.to_vec())));
        Ok((head, Box::pin(body)))
    }
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
}
