//! Outbound HTTP through the host (permission `network.hosts`) and a loopback listener.

use crate::Result;
use crate::bindings::oino::extension::http as wh;
pub use crate::bindings::oino::extension::http::{Auth, Incoming, Method, ResponseHead};

/// Request builder.
pub struct RequestBuilder(wh::Request);

/// `GET url`.
#[must_use]
pub fn get(url: &str) -> RequestBuilder {
    request(Method::Get, url)
}

/// `POST url`.
#[must_use]
pub fn post(url: &str) -> RequestBuilder {
    request(Method::Post, url)
}

/// Any method.
#[must_use]
pub fn request(method: Method, url: &str) -> RequestBuilder {
    RequestBuilder(wh::Request {
        method,
        url: url.to_owned(),
        headers: Vec::new(),
        body: None,
        timeout_ms: None,
        follow_redirects: true,
        auth: Auth::None,
        impersonate: None,
        proxy: None,
    })
}

/// A response with a streaming body.
pub struct Response {
    /// Head.
    pub head: ResponseHead,
    body: wit_bindgen::StreamReader<u8>,
}

impl Response {
    /// Status code.
    #[must_use]
    pub fn status(&self) -> u16 {
        self.head.status
    }
    /// Header value (case-insensitive).
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.head
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
    /// Collects the body.
    pub async fn bytes(self) -> Vec<u8> {
        self.body.collect().await
    }
    /// Body as text.
    pub async fn text(self) -> String {
        String::from_utf8_lossy(&self.bytes().await).into_owned()
    }
    /// Body as JSON.
    pub async fn json<T: serde::de::DeserializeOwned>(self) -> Result<T> {
        Ok(serde_json::from_slice(&self.bytes().await)?)
    }
    /// Streaming body.
    #[must_use]
    pub fn body(self) -> wit_bindgen::StreamReader<u8> {
        self.body
    }
}

impl RequestBuilder {
    /// Adds a header.
    #[must_use]
    pub fn header(mut self, k: &str, v: &str) -> Self {
        self.0.headers.push((k.to_owned(), v.to_owned()));
        self
    }
    /// Body bytes.
    #[must_use]
    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.0.body = Some(body.into());
        self
    }
    /// JSON body (+ content type).
    pub fn json(self, value: &impl serde::Serialize) -> Result<Self> {
        Ok(self
            .header("content-type", "application/json")
            .body(serde_json::to_vec(value)?))
    }
    /// Timeout.
    #[must_use]
    pub fn timeout_ms(mut self, ms: u32) -> Self {
        self.0.timeout_ms = Some(ms);
        self
    }
    /// Do not follow redirects.
    #[must_use]
    pub fn no_redirects(mut self) -> Self {
        self.0.follow_redirects = false;
        self
    }
    /// Inject provider credentials (permission `credentials`).
    #[must_use]
    pub fn provider_auth(mut self, provider: &str) -> Self {
        self.0.auth = Auth::Provider(provider.to_owned());
        self
    }
    /// TLS fingerprint impersonation profile (permission `network.fingerprint`).
    #[must_use]
    pub fn impersonate(mut self, profile: &str) -> Self {
        self.0.impersonate = Some(profile.to_owned());
        self
    }
    /// Proxy override.
    #[must_use]
    pub fn proxy(mut self, proxy: &str) -> Self {
        self.0.proxy = Some(proxy.to_owned());
        self
    }
    /// Sends the request.
    pub async fn send(self) -> Result<Response> {
        let (head, body) = wh::fetch(self.0).await?;
        Ok(Response { head, body })
    }
}

/// Loopback listener (permission `network.listenLoopback`).
pub struct Listener(wh::Listener);

impl Listener {
    /// Binds `127.0.0.1:port` (0 = ephemeral).
    pub fn bind(port: u16) -> Result<Self> {
        Ok(Self(wh::Listener::bind(port)?))
    }
    /// Bound port.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.0.port()
    }
    /// Next request.
    pub async fn accept(&self) -> Option<Incoming> {
        self.0.accept().await
    }
    /// Answers request `id`.
    pub fn respond(
        &self,
        id: u64,
        status: u16,
        headers: &[(String, String)],
        body: &[u8],
    ) -> Result<()> {
        Ok(self.0.respond(id, status, headers, body)?)
    }
}
