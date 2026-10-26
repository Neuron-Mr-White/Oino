//! `unipi.web-api` — web search, page reading and summaries (unipi `packages/web-api`,
//! unipi-port §3.21).
//!
//! | unipi | here |
//! |---|---|
//! | `web_search {query, source?: 1..6}` (wigolo, DuckDuckGo, Jina, SerpAPI, Tavily, Perplexity) | same schema, ranks and texts ([`providers`]) |
//! | `multi_web_content_read` (smart-fetch default, sources 1..4 wigolo/Jina Reader/Firecrawl/Perplexity) | same schema; smart-fetch = host `http.fetch` with `impersonate` + `dom_smoothie` Readability → Markdown in the guest ([`extract`], [`format`]) |
//! | `web_llm_summarize {url, prompt?}` (Perplexity) | same; plus an opt-in "Session model" summariser (`models.complete`) |
//! | fallthrough when `source` is omitted, exact provider when given | same |
//! | cache 1 h in `~/.unipi/config/web-api/cache` | `/cache/web` ([`cache`]) |
//! | tools without a provider hidden via `setActiveTools` | same, on `session_start` and `before_agent_start` |
//! | wigolo SDK (AGPL, optional) | the wigolo REST daemon over loopback HTTP |
//! | hub runner `unipi:web-cache-clear`, info-screen "Web API" group | hidden command; `info-screen.groups` service |

#![allow(
    clippy::unused_async_trait_impl,
    reason = "`Extension::init` is async in the SDK; this init registers synchronously"
)]
#![cfg_attr(
    test,
    allow(
        clippy::assert_is_empty,
        reason = "emptiness asserts read better in tests"
    )
)]
#![allow(
    clippy::result_large_err,
    reason = "FetchError mirrors unipi's structured error; it is built once per failed URL"
)]

mod cache;
mod extract;
mod format;
mod providers;
mod settings;

use std::cell::Cell;

use dom_query::Document;
use futures::future::join_all;
use oino_extension::prelude::*;
use unipi_kit::settings::Store;
use unipi_kit::topics::ModuleReady;

use crate::cache::Cache;
use crate::format::{BatchItem, FetchError, FetchResult, Meta};
use crate::providers::{Cap, Provider, Req, SearchResult};
use crate::settings::NAMESPACE;

const MAX_REDIRECTS: usize = 5;
const TOOLS: [&str; 3] = ["web_search", "multi_web_content_read", "web_llm_summarize"];

/// The extension.
#[derive(Default)]
pub struct WebApi {
    settings: Store,
    cache: Cache,
    impersonated: Cell<Option<bool>>,
}

/// Raw fetch result.
struct Fetched {
    status: u16,
    body: Vec<u8>,
}

async fn send(req: &Req, timeout_ms: u32) -> std::result::Result<Fetched, String> {
    let mut b = if req.post {
        http::post(&req.url)
    } else {
        http::get(&req.url)
    }
    .timeout_ms(timeout_ms);
    for (k, v) in &req.headers {
        b = b.header(k, v);
    }
    if let Some(body) = &req.body {
        b = b.body(serde_json::to_vec(body).unwrap_or_default());
    }
    let resp = b.send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    Ok(Fetched {
        status,
        body: resp.bytes().await,
    })
}

fn ok(status: u16) -> bool {
    (200..300).contains(&status)
}

fn status_line(status: u16) -> String {
    let t = extract::status_text(status);
    if t.is_empty() {
        status.to_string()
    } else {
        format!("{status} {t}")
    }
}

fn json_of(f: &Fetched) -> Value {
    serde_json::from_slice(&f.body).unwrap_or(Value::Null)
}

/// `web_search` schema (unipi `TypeBox`).
fn search_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "query": {"type": "string", "description": "Search query string"},
            "source": {
                "type": "number", "minimum": 1, "maximum": 6,
                "description": "Provider selection (1=wigolo, 2=DuckDuckGo, 3=Jina Search, 4=SerpAPI, 5=Tavily, 6=Perplexity). Omit for auto-selection.",
            },
        },
        "required": ["query"],
    })
}

/// `multi_web_content_read` schema (unipi `TypeBox`).
fn read_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "url": {
                "anyOf": [
                    {"type": "string", "description": "Single URL to read"},
                    {"type": "array", "items": {"type": "string"}, "description": "Array of URLs to read in batch"},
                ],
                "description": "URL or array of URLs to read",
            },
            "source": {"type": "number", "minimum": 0, "maximum": 4, "description": "Provider selection (0=smart-fetch engine, 1=wigolo, 2=Jina Reader, 3=Firecrawl, 4=Perplexity). Default is 0 (smart-fetch)."},
            "browser": {"type": "string", "description": "TLS fingerprint browser profile (e.g., chrome_145). Default: chrome_145."},
            "os": {"type": "string", "description": "OS fingerprint (windows, macos, linux). Default: windows."},
            "format": {"anyOf": [{"const": "markdown", "type": "string"}, {"const": "html", "type": "string"}, {"const": "text", "type": "string"}, {"const": "json", "type": "string"}], "description": "Output format. Default: markdown."},
            "maxChars": {"type": "number", "description": "Maximum characters in output. Default: 50000."},
            "timeoutMs": {"type": "number", "description": "Request timeout in milliseconds. Default: 15000."},
            "removeImages": {"type": "boolean", "description": "Strip image references from content. Default: false."},
            "includeReplies": {"anyOf": [{"type": "boolean"}, {"const": "extractors", "type": "string"}], "description": "Include replies/comments. Default: extractors."},
            "proxy": {"type": "string", "description": "Proxy URL for requests."},
            "batchConcurrency": {"type": "number", "description": "Concurrent requests for batch mode. Default: 8."},
            "verbose": {"type": "boolean", "description": "Include metadata header in output. Default: true."},
        },
        "required": ["url"],
    })
}

/// `web_llm_summarize` schema (unipi `TypeBox`).
fn summarize_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "url": {"type": "string", "description": "URL to summarize"},
            "prompt": {"type": "string", "description": "Custom summarization prompt. Omit for default comprehensive summary."},
        },
        "required": ["url"],
    })
}

#[derive(Deserialize)]
struct SearchArgs {
    query: String,
    #[serde(default)]
    source: Option<f64>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
struct ReadArgs {
    url: Value,
    #[serde(default)]
    source: Option<f64>,
    #[serde(default)]
    browser: Option<String>,
    /// Accepted for schema parity; the host profile carries no separate OS (deviation).
    #[serde(default, rename = "os")]
    _os: Option<String>,
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    max_chars: Option<f64>,
    #[serde(default)]
    timeout_ms: Option<f64>,
    #[serde(default)]
    remove_images: Option<bool>,
    #[serde(default)]
    include_replies: Option<Value>,
    #[serde(default)]
    proxy: Option<String>,
    #[serde(default)]
    batch_concurrency: Option<f64>,
    #[serde(default)]
    verbose: Option<bool>,
}

#[derive(Deserialize)]
struct SummarizeArgs {
    url: String,
    #[serde(default)]
    prompt: Option<String>,
}

/// Resolved smart-fetch options (unipi `resolveFetchOptions`).
#[derive(Clone, Debug)]
struct FetchOpts {
    browser: String,
    format: String,
    max_chars: usize,
    timeout_ms: u64,
    remove_images: bool,
    include_replies: Option<bool>,
    proxy: Option<String>,
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "positive tool numbers"
)]
fn positive(v: Option<f64>) -> Option<u64> {
    v.filter(|n| n.is_finite() && *n > 0.0).map(|n| n as u64)
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "ranks are small"
)]
fn rank(v: Option<f64>) -> Option<u8> {
    v.filter(|n| n.is_finite() && *n >= 0.0).map(|n| n as u8)
}

impl WebApi {
    fn values(&self) -> Value {
        settings::expand_dotted(&self.settings.get(NAMESPACE).0)
    }

    /// Key: settings value → secret (`web-api/providers.<id>.apiKey`) → env.
    fn api_key(&self, p: &Provider) -> Option<String> {
        let field = format!("providers.{}.apiKey", p.id);
        self.values()
            .pointer(&format!("/providers/{}/apiKey", p.id))
            .and_then(Value::as_str)
            .filter(|k| !k.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| secrets::get(&format!("{NAMESPACE}/{field}")).ok().flatten())
            .or_else(|| secrets::get(&field).ok().flatten())
            .or_else(|| p.env.and_then(|e| host::env(e).ok().flatten()))
            .filter(|k| !k.is_empty())
    }

    fn available(&self, p: &Provider) -> bool {
        settings::provider_enabled(&self.values(), p.id)
            && (!p.requires_key || self.api_key(p).is_some())
    }

    fn wigolo_url(&self) -> String {
        self.values()
            .pointer("/providers/wigolo/url")
            .and_then(Value::as_str)
            .filter(|u| !u.is_empty())
            .unwrap_or(settings::DEFAULT_WIGOLO_URL)
            .trim_end_matches('/')
            .to_owned()
    }

    fn perplexity_model(&self) -> String {
        self.values()
            .pointer("/providers/perplexity/model")
            .and_then(Value::as_str)
            .filter(|m| !m.is_empty())
            .unwrap_or(settings::DEFAULT_PERPLEXITY_MODEL)
            .to_owned()
    }

    fn key(&self, p: &Provider) -> std::result::Result<String, String> {
        self.api_key(p).ok_or_else(|| providers::key_required(p))
    }

    // ── providers ───────────────────────────────────────────────────────────

    async fn wigolo(&self, path: &str, body: Value) -> std::result::Result<Value, String> {
        let base = self.wigolo_url();
        let req = Req {
            post: true,
            url: format!("{base}{path}"),
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: Some(body),
        };
        let f = send(&req, 60_000)
            .await
            .map_err(|e| providers::wigolo_unavailable(&base, &e))?;
        if !ok(f.status) {
            return Err(format!(
                "wigolo {} failed: HTTP {}",
                path.trim_start_matches("/v1/"),
                status_line(f.status)
            ));
        }
        Ok(json_of(&f))
    }

    async fn perplexity(
        &self,
        system: &str,
        user: &str,
        what: &str,
    ) -> std::result::Result<Value, String> {
        let p = providers::by_id("perplexity").ok_or("perplexity")?;
        let key = self.key(p)?;
        let f = send(
            &providers::perplexity_request(&self.perplexity_model(), system, user, &key),
            120_000,
        )
        .await?;
        if !ok(f.status) {
            return Err(format!(
                "Perplexity {what} failed: {}",
                status_line(f.status)
            ));
        }
        Ok(json_of(&f))
    }

    async fn search_with(
        &self,
        p: &Provider,
        query: &str,
    ) -> std::result::Result<Vec<SearchResult>, String> {
        let get = |req: Req, label: &'static str| async move {
            let f = send(&req, 30_000).await?;
            if ok(f.status) {
                Ok(f)
            } else {
                Err(format!("{label} failed: {}", status_line(f.status)))
            }
        };
        match p.id {
            "wigolo" => providers::parse_wigolo_search(
                &self
                    .wigolo("/v1/search", providers::wigolo_search_body(query, 10))
                    .await?,
            ),
            "duckduckgo" => {
                let f = get(providers::ddg_request(query), "DuckDuckGo search").await?;
                Ok(providers::parse_ddg(&String::from_utf8_lossy(&f.body)))
            }
            "jina-search" => {
                let f = get(
                    providers::jina_search_request(query, self.api_key(p).as_deref()),
                    "Jina AI search",
                )
                .await?;
                Ok(providers::parse_jina_search(&json_of(&f)))
            }
            "serpapi" => {
                let f = get(
                    providers::serpapi_request(query, &self.key(p)?),
                    "SerpAPI search",
                )
                .await?;
                Ok(providers::parse_serpapi(&json_of(&f)))
            }
            "tavily" => {
                let f = get(
                    providers::tavily_request(query, &self.key(p)?),
                    "Tavily search",
                )
                .await?;
                Ok(providers::parse_tavily(&json_of(&f)))
            }
            "perplexity" => Ok(providers::parse_perplexity_search(
                &self
                    .perplexity(providers::PERPLEXITY_SEARCH_SYSTEM, query, "search")
                    .await?,
            )),
            other => Err(format!("Provider \"{other}\" does not support search")),
        }
    }

    async fn read_with(
        &self,
        p: &Provider,
        url: &str,
    ) -> std::result::Result<(String, String), String> {
        match p.id {
            "wigolo" => providers::parse_wigolo_fetch(
                &self.wigolo("/v1/fetch", json!({"url": url})).await?,
                url,
            ),
            "jina-reader" => {
                let f = send(
                    &providers::jina_read_request(url, self.api_key(p).as_deref()),
                    60_000,
                )
                .await?;
                if !ok(f.status) {
                    return Err(format!("Jina AI Reader failed: {}", status_line(f.status)));
                }
                providers::parse_jina_read(&json_of(&f), url)
                    .ok_or_else(|| "Jina AI Reader returned no content".into())
            }
            "firecrawl" => {
                let f = send(&providers::firecrawl_request(url, &self.key(p)?), 120_000).await?;
                if !ok(f.status) {
                    return Err(format!("Firecrawl read failed: {}", status_line(f.status)));
                }
                providers::parse_firecrawl(&json_of(&f)).map(|c| (url.to_owned(), c))
            }
            "perplexity" => {
                let v = self
                    .perplexity(
                        providers::PERPLEXITY_READ_PROMPT,
                        &format!("Summarize this URL: {url}"),
                        "summarize",
                    )
                    .await?;
                Ok((url.to_owned(), providers::perplexity_text(&v)))
            }
            other => Err(format!("Provider \"{other}\" does not support read")),
        }
    }

    async fn summarize_with(
        &self,
        p: &Provider,
        url: &str,
        prompt: Option<&str>,
    ) -> std::result::Result<String, String> {
        let system = prompt
            .filter(|p| !p.is_empty())
            .unwrap_or(providers::DEFAULT_SUMMARY_PROMPT);
        match p.id {
            "perplexity" => Ok(providers::perplexity_text(
                &self
                    .perplexity(system, &format!("Summarize this URL: {url}"), "summarize")
                    .await?,
            )),
            "model" => {
                let page = self
                    .smart_fetch(url, &self.fetch_opts(&ReadArgs::default()))
                    .await
                    .map_err(|e| e.error)?;
                let model = self
                    .values()
                    .pointer("/providers/model/model")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                let mut req = json!({
                    "context": {"systemPrompt": system, "messages": [{"role": "user", "content": format!("URL: {url}\n\n{}", page.content), "timestamp": 0}]},
                    "options": {"maxTokens": 2048},
                });
                if !model.is_empty() {
                    req["model"] = json!(model);
                }
                let reply = models::complete(&req).await.map_err(|e| e.to_string())?;
                let text: String = reply
                    .get("content")
                    .and_then(Value::as_array)
                    .map(|blocks| {
                        blocks
                            .iter()
                            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                            .filter_map(|b| b.get("text").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();
                if text.trim().is_empty() {
                    Err("the model returned no summary".into())
                } else {
                    Ok(text)
                }
            }
            other => Err(format!("Provider \"{other}\" does not support summarize")),
        }
    }

    /// unipi `withProviderFallthrough`.
    async fn search(
        &self,
        query: &str,
        source: Option<u8>,
    ) -> std::result::Result<Vec<SearchResult>, String> {
        let chain = providers::chain(Cap::Search, source, |p| self.available(p))?;
        let mut failures = Vec::new();
        for p in chain {
            match self.search_with(p, query).await {
                Ok(r) => return Ok(r),
                Err(e) => failures.push(format!("{}: {e}", p.name)),
            }
        }
        Err(providers::fallthrough_error(&failures))
    }

    async fn provider_read(
        &self,
        url: &str,
        source: Option<u8>,
    ) -> std::result::Result<(String, String), String> {
        let chain = providers::chain(Cap::Read, source, |p| self.available(p))?;
        let mut failures = Vec::new();
        for p in chain {
            match self.read_with(p, url).await {
                Ok(r) => return Ok(r),
                Err(e) => failures.push(format!("{}: {e}", p.name)),
            }
        }
        Err(providers::fallthrough_error(&failures))
    }

    async fn summarize(
        &self,
        url: &str,
        prompt: Option<&str>,
    ) -> std::result::Result<String, String> {
        let chain = providers::chain(Cap::Summarize, None, |p| self.available(p))?;
        let mut failures = Vec::new();
        for p in chain {
            match self.summarize_with(p, url, prompt).await {
                Ok(r) => return Ok(r),
                Err(e) => failures.push(format!("{}: {e}", p.name)),
            }
        }
        Err(providers::fallthrough_error(&failures))
    }

    // ── smart fetch ─────────────────────────────────────────────────────────

    fn fetch_opts(&self, a: &ReadArgs) -> FetchOpts {
        let d = settings::smart_fetch(&self.values());
        FetchOpts {
            browser: settings::resolve_browser(Some(a.browser.as_deref().unwrap_or(&d.browser))),
            format: a
                .format
                .clone()
                .filter(|f| !f.is_empty())
                .unwrap_or_else(|| "markdown".into()),
            max_chars: positive(a.max_chars)
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(d.max_chars),
            timeout_ms: positive(a.timeout_ms).unwrap_or(d.timeout_ms),
            remove_images: a.remove_images.unwrap_or(d.remove_images),
            include_replies: match &a.include_replies {
                Some(v) => settings::parse_replies(Some(v)),
                None => d.include_replies,
            },
            proxy: a.proxy.clone().filter(|p| !p.is_empty()),
        }
    }

    /// unipi `defuddleFetch`.
    async fn smart_fetch(
        &self,
        url: &str,
        o: &FetchOpts,
    ) -> std::result::Result<FetchResult, FetchError> {
        extract::validate_url(url).map_err(|(code, msg)| {
            let mut e = FetchError::new(code, "validation", msg, false);
            e.url = Some(url.to_owned());
            e
        })?;
        let mut final_url = url.to_owned();
        for _ in 0..MAX_REDIRECTS {
            let mut b = http::get(&final_url)
                .timeout_ms(u32::try_from(o.timeout_ms).unwrap_or(u32::MAX))
                .header(
                    "Accept",
                    "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
                )
                .header("Accept-Language", "en-US,en;q=0.9")
                .impersonate(&o.browser);
            if let Some(p) = &o.proxy {
                b = b.proxy(p);
            }
            let resp = b.send().await.map_err(|e| {
                let m = e.to_string();
                let lower = m.to_lowercase();
                let err = if lower.contains("timeout") || lower.contains("timed out") {
                    let mut e = FetchError::new("timeout", "waiting", m, true);
                    e.timeout_ms = Some(o.timeout_ms);
                    e
                } else if lower.contains("network")
                    || lower.contains("connect")
                    || lower.contains("refused")
                    || lower.contains("dns")
                {
                    FetchError::new("network_error", "connecting", m, true)
                } else {
                    FetchError::new("unexpected_response", "loading", m, false)
                };
                err.at(url, &final_url)
            })?;
            self.impersonated.set(Some(resp.head.impersonated));
            let status = resp.status();
            if !resp.head.final_url.is_empty() {
                final_url.clone_from(&resp.head.final_url);
            }
            if !ok(status) {
                let mut e = FetchError::new(
                    "http_error",
                    "waiting",
                    format!("HTTP error: {status} {}", extract::status_text(status))
                        .trim_end()
                        .to_owned(),
                    status >= 500 || status == 429,
                )
                .at(url, &final_url);
                e.status_code = Some(status);
                return Err(e);
            }
            let (mime, binary) = extract::content_type(resp.header("content-type"));
            let body = resp.bytes().await;
            let meta = Meta::default();
            if binary {
                return Ok(format::create_result(
                    url,
                    &final_url,
                    format!("[Binary file: {mime}]"),
                    &meta,
                    &o.format,
                    &mime,
                ));
            }
            if mime == "application/json" {
                let pretty = serde_json::from_slice::<Value>(&body)
                    .ok()
                    .and_then(|v| serde_json::to_string_pretty(&v).ok())
                    .unwrap_or_else(|| String::from_utf8_lossy(&body).into_owned());
                return Ok(format::create_result(
                    url,
                    &final_url,
                    format::truncate(&pretty, o.max_chars),
                    &meta,
                    "json",
                    &mime,
                ));
            }
            let text = String::from_utf8_lossy(&body).into_owned();
            if mime.starts_with("text/plain") {
                return Ok(format::create_result(
                    url,
                    &final_url,
                    format::truncate(&text, o.max_chars),
                    &meta,
                    "text",
                    &mime,
                ));
            }
            let doc = Document::from(text.as_str());
            if let Some(target) = extract::meta_refresh(&doc) {
                final_url = extract::resolve_url(&final_url, &target);
                continue;
            }
            let page = extract::extract(&text, &final_url, o.remove_images, o.include_replies);
            let mut content = page.content;
            if o.remove_images {
                content = format::remove_images(&content);
            }
            let content = format::truncate(&content, o.max_chars);
            let draft =
                format::create_result(url, &final_url, content, &page.meta, "markdown", &mime);
            let formatted = format::format_content(&draft, &o.format, o.max_chars);
            return Ok(format::create_result(
                url, &final_url, formatted, &page.meta, &o.format, &mime,
            ));
        }
        Err(FetchError::new(
            "too_many_redirects",
            "processing",
            format!("Too many meta refresh redirects ({MAX_REDIRECTS})"),
            false,
        )
        .at(url, &final_url))
    }

    async fn smart_read(
        &self,
        url: &str,
        a: &ReadArgs,
    ) -> std::result::Result<FetchResult, FetchError> {
        let key = cache::smart_fetch_key(
            url,
            a.browser.as_deref(),
            a.format.as_deref(),
            positive(a.max_chars),
        );
        let now = timers::now_ms();
        if let Some(hit) = self
            .cache
            .get(&key, "smart-fetch", now)
            .and_then(|v| serde_json::from_value(v).ok())
        {
            return Ok(hit);
        }
        let r = self.smart_fetch(url, &self.fetch_opts(a)).await?;
        if let Ok(v) = serde_json::to_value(&r) {
            self.cache.set(&key, "smart-fetch", &v, now);
        }
        Ok(r)
    }

    /// unipi `defuddleFetchMultiple` (bounded concurrency).
    async fn smart_batch(&self, urls: &[String], a: &ReadArgs) -> Vec<BatchItem> {
        let d = settings::smart_fetch(&self.values());
        let n = positive(a.batch_concurrency)
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(d.batch_concurrency)
            .max(1);
        let opts = self.fetch_opts(a);
        let mut out = Vec::with_capacity(urls.len());
        for chunk in urls.chunks(n) {
            let results = join_all(chunk.iter().map(|u| self.smart_fetch(u, &opts))).await;
            for (u, r) in chunk.iter().zip(results) {
                out.push(match r {
                    Ok(r) => BatchItem::Done(r),
                    Err(mut e) => {
                        e.url.get_or_insert_with(|| u.clone());
                        BatchItem::Error(e)
                    }
                });
            }
        }
        out
    }

    async fn read_tool(&self, a: ReadArgs) -> std::result::Result<ToolResult, String> {
        let source = rank(a.source).unwrap_or(0);
        let verbose = a.verbose.unwrap_or(true);
        match &a.url {
            Value::String(url) => {
                if source >= 1 {
                    let (u, content) = self.provider_read(url, Some(source)).await?;
                    return Ok(ToolResult::text(format!("Content from {u}:\n\n{content}"))
                        .with_details(json!({})));
                }
                let r = self.smart_read(url, &a).await.map_err(|e| e.error)?;
                Ok(ToolResult::text(format::format_single(&r, verbose))
                    .with_details(json!({"url": r.url, "finalUrl": r.final_url, "title": r.title, "wordCount": r.word_count})))
            }
            Value::Array(items) => {
                let urls: Vec<String> = items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect();
                if urls.is_empty() {
                    return Ok(ToolResult::text("No URLs provided.").with_details(json!({})));
                }
                if source >= 1 {
                    let results =
                        join_all(urls.iter().map(|u| self.provider_read(u, Some(source)))).await;
                    let text = urls
                        .iter()
                        .zip(&results)
                        .enumerate()
                        .map(|(i, (u, r))| match r {
                            Ok((_, content)) => format!("[{}] {u}\n{content}", i + 1),
                            Err(e) => format!("[{}] {u}\nError: {e}", i + 1),
                        })
                        .collect::<Vec<_>>()
                        .join("\n\n---\n\n");
                    return Ok(ToolResult::text(text).with_details(json!({"total": results.len()})));
                }
                let items = self.smart_batch(&urls, &a).await;
                let ok = items
                    .iter()
                    .filter(|i| matches!(i, BatchItem::Done(_)))
                    .count();
                Ok(ToolResult::text(format::format_batch(&items)).with_details(
                    json!({"total": items.len(), "succeeded": ok, "failed": items.len() - ok}),
                ))
            }
            _ => Err("Invalid url parameter.".into()),
        }
    }

    // ── tool exposure ───────────────────────────────────────────────────────

    /// unipi `webToolAvailability`.
    fn availability(&self) -> [bool; 3] {
        [
            providers::chain(Cap::Search, None, |p| self.available(p)).is_ok(),
            true,
            providers::chain(Cap::Summarize, None, |p| self.available(p)).is_ok(),
        ]
    }

    /// unipi `syncWebTools`: one `setActiveTools` at most, other tools untouched.
    fn sync_tools(&self) {
        let current = control::active_tools();
        if let Some(next) = sync_list(&current, self.availability()) {
            let _ = control::set_active_tools(&next);
        }
    }

    async fn wigolo_status(&self) -> String {
        if !settings::provider_enabled(&self.values(), "wigolo") {
            return "Disabled".into();
        }
        let req = Req {
            post: false,
            url: format!("{}/health", self.wigolo_url()),
            headers: Vec::new(),
            body: None,
        };
        match send(&req, 1500).await {
            Ok(f) if ok(f.status) => "✓ Running".into(),
            _ => "Not running (npx wigolo init)".into(),
        }
    }

    async fn info_group(&self) -> Value {
        let values = self.values();
        let enabled = providers::PROVIDERS
            .iter()
            .filter(|p| settings::provider_enabled(&values, p.id))
            .count();
        let stats = self.cache.stats(timers::now_ms());
        let smart = match self.impersonated.get() {
            Some(false) => {
                "Fallback (plain fetch): TLS fingerprinting unavailable on this host".to_owned()
            }
            _ => "✓ Ready".to_owned(),
        };
        let wigolo = self.wigolo_status().await;
        let rows: Vec<Value> = providers::PROVIDERS
            .iter()
            .map(|p| {
                let caps: Vec<&str> = [Cap::Search, Cap::Read, Cap::Summarize].iter().filter(|c| p.rank(**c) > 0).map(|c| c.as_str()).collect();
                json!({"id": p.id, "name": p.name, "caps": caps, "enabled": settings::provider_enabled(&values, p.id), "keyed": p.requires_key, "hasKey": !p.requires_key || self.api_key(p).is_some()})
            })
            .collect();
        let [s, r, m] = self.availability();
        #[allow(clippy::cast_precision_loss, reason = "cache sizes are small")]
        let kb = stats.total_size_bytes as f64 / 1024.0;
        json!({
            "id": "web-api", "name": "Web API", "short": "🌐", "priority": 50,
            "config": {"showByDefault": true, "stats": [
                {"id": "providers", "label": "Enabled Providers", "show": true},
                {"id": "wigolo", "label": "wigolo", "show": true},
                {"id": "smartFetch", "label": "Smart-Fetch", "show": true},
                {"id": "cacheEntries", "label": "Cache Entries", "show": true},
                {"id": "cacheSize", "label": "Cache Size", "show": true},
                {"id": "expired", "label": "Expired Entries", "show": true},
            ]},
            "data": {
                "raw": {"value": "", "raw": {"providers": rows, "tools": {"web_search": s, "multi_web_content_read": r, "web_llm_summarize": m}, "wigolo": wigolo, "cache": {"entries": stats.total_entries, "bytes": stats.total_size_bytes, "expired": stats.expired_entries}}},
                "providers": {"value": enabled.to_string()},
                "wigolo": {"value": wigolo},
                "smartFetch": {"value": smart},
                "cacheEntries": {"value": stats.total_entries.to_string()},
                "cacheSize": {"value": format!("{kb:.1} KB")},
                "expired": {"value": stats.expired_entries.to_string()},
            },
        })
    }
}

/// unipi `syncWebTools` list computation (`None` = no change).
fn sync_list(current: &[String], availability: [bool; 3]) -> Option<Vec<String>> {
    let wanted: Vec<&str> = TOOLS
        .iter()
        .zip(availability)
        .filter(|(_, a)| *a)
        .map(|(t, _)| *t)
        .collect();
    if TOOLS
        .iter()
        .all(|t| current.iter().any(|c| c == t) == wanted.contains(t))
    {
        return None;
    }
    let mut next: Vec<String> = current
        .iter()
        .filter(|c| !TOOLS.contains(&c.as_str()) || wanted.contains(&c.as_str()))
        .cloned()
        .collect();
    for w in wanted {
        if !current.iter().any(|c| c == w) {
            next.push(w.to_owned());
        }
    }
    Some(next)
}

impl Extension for WebApi {
    #[allow(clippy::too_many_lines, reason = "one registration list")]
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        let ns = settings::namespace();
        cx.settings_namespace(&ns.to_json());
        self.settings.declare(NAMESPACE, ns.defaults.clone());
        self.settings.load(cx.settings());

        cx.tool(
            Tool::<Self>::new("web_search")
                .label("Web Search")
                .description(
                    "Search the web for information using various providers. \
                     Lower source = simpler/cheaper providers (wigolo, DuckDuckGo, Jina Search). \
                     Higher source = more capable providers (SerpAPI, Tavily, Perplexity).",
                )
                .prompt_snippet("Search the web for information.")
                .guideline("Use web_search to find information on the web.")
                .guideline("Omit source for auto-selection (cheapest available, falling through on failure).")
                .guideline("Specify source number for specific provider (1=wigolo, 2=DuckDuckGo, 3=Jina, 4=SerpAPI, 5=Tavily, 6=Perplexity).")
                .guideline("Quick facts: source 1-3. Research: source 4-6.")
                .parameters(&search_schema())
                .kind(ToolKind::Search)
                .run(|ext, a: SearchArgs, _cx| async move {
                    let results = ext.search(&a.query, rank(a.source)).await.map_err(|e| Error::msg(format!("Search failed: {e}")))?;
                    if results.is_empty() {
                        return Ok(ToolResult::text("No results found.").with_details(json!({})));
                    }
                    let formatted = results
                        .iter()
                        .enumerate()
                        .map(|(i, r)| format!("{}. **{}**\n   {}\n   {}", i + 1, r.title, r.url, r.snippet))
                        .collect::<Vec<_>>()
                        .join("\n\n");
                    Ok(ToolResult::text(format!("Found {} results:\n\n{formatted}", results.len())).with_details(json!({})))
                }),
        );
        cx.tool(
            Tool::<Self>::new("multi_web_content_read")
                .label("Multi Web Content Read")
                .description(
                    "Read and extract content from URLs using the smart-fetch engine (default) or provider fallbacks. \
                     Supports single URL or batch URLs. \
                     Returns clean markdown with metadata (title, author, site, word count).",
                )
                .prompt_snippet("Read content from one or more URLs.")
                .guideline("Use multi_web_content_read to extract content from web pages.")
                .guideline("Pass a single URL string or an array of URLs for batch reading.")
                .guideline("Default source (0 or omitted) uses the local smart-fetch engine — free, no API key.")
                .guideline("source 1-4 uses provider fallbacks: wigolo, Jina Reader, Firecrawl, Perplexity.")
                .guideline("Batch mode: pass an array of URLs, returns results for each.")
                .parameters(&read_schema())
                .kind(ToolKind::Fetch)
                .run(|ext, a: ReadArgs, _cx| async move {
                    ext.read_tool(a).await.map_err(|e| Error::msg(format!("Read failed: {e}")))
                }),
        );
        cx.tool(
            Tool::<Self>::new("web_llm_summarize")
                .label("Web LLM Summarize")
                .description(
                    "Summarize web content using LLM (Perplexity). \
                     Fetches content from URL, then uses the LLM to summarize. \
                     Higher cost (LLM tokens + provider cost).",
                )
                .prompt_snippet("Summarize web content using LLM.")
                .guideline("Use web_llm_summarize to get a summary of web content.")
                .guideline("Specify custom prompt for targeted summaries.")
                .guideline("Higher cost due to LLM token usage.")
                .parameters(&summarize_schema())
                .kind(ToolKind::Fetch)
                .run(|ext, a: SummarizeArgs, _cx| async move {
                    let summary = ext
                        .summarize(&a.url, a.prompt.as_deref())
                        .await
                        .map_err(|e| Error::msg(format!("Summarize failed: {e}")))?;
                    Ok(
                        ToolResult::text(format!("Summary of {}:\n\n{summary}", a.url))
                            .with_details(json!({})),
                    )
                }),
        );

        cx.on(EventKind::SessionStart, |ext, _ev, _cx| async move {
            ext.sync_tools();
            ext.cache.clear_expired(timers::now_ms());
            unipi_kit::bus::emit(&ModuleReady {
                name: "@pi-unipi/web-api".into(),
                version: "3.0.0".into(),
                commands: Vec::new(),
                tools: TOOLS.iter().map(ToString::to_string).collect(),
                load_time_ms: None,
            });
            Ok(())
        });
        cx.on(EventKind::BeforeAgentStart, |ext, _ev, _cx| async move {
            ext.sync_tools();
            Ok(())
        });
        cx.on(EventKind::SessionShutdown, |ext, _ev, _cx| async move {
            ext.cache.clear_expired(timers::now_ms());
            Ok(())
        });
        cx.on(EventKind::OinoSettingsChanged, |ext, ev, _cx| async move {
            let _ = ext.settings.on_changed(&ev.payload);
            Ok(())
        });

        cx.command(
            Command::<Self>::new("unipi:web-cache-clear")
                .description("Clear the web cache")
                .hidden()
                .run(|ext, _args, _cx| async move {
                    let stats = ext.cache.stats(timers::now_ms());
                    let confirmed = ui::confirm(
                        "Clear web cache?",
                        &format!(
                            "{} bytes cached. This cannot be undone.",
                            stats.total_size_bytes
                        ),
                    )
                    .await
                    .unwrap_or(false);
                    if !confirmed {
                        return Ok(());
                    }
                    let cleared = ext.cache.clear();
                    let _ = ui::notify(
                        &format!(
                            "Cache cleared: {cleared} entries removed ({} bytes freed)",
                            stats.total_size_bytes
                        ),
                        "info",
                    );
                    Ok(())
                }),
        );
        cx.service(
            unipi_kit::services::fanout::INFO_SCREEN_GROUPS,
            |ext, _p, _caller, _cx| async move { Ok(ext.info_group().await) },
        );
        Ok(())
    }
}

// Native builds (unit tests) skip the component exports.
#[cfg(target_arch = "wasm32")]
oino_extension::export_extension!(WebApi);

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn sync_adds_only_missing_web_tools() {
        let current = names(&["read", "bash"]);
        assert_eq!(
            sync_list(&current, [true, true, false]),
            Some(names(&[
                "read",
                "bash",
                "web_search",
                "multi_web_content_read"
            ]))
        );
    }

    #[test]
    fn sync_removes_unavailable_but_keeps_others() {
        let current = names(&[
            "web_search",
            "read",
            "web_llm_summarize",
            "multi_web_content_read",
        ]);
        assert_eq!(
            sync_list(&current, [false, true, false]),
            Some(names(&["read", "multi_web_content_read"]))
        );
    }

    #[test]
    fn sync_noop_when_unchanged() {
        let current = names(&["read", "web_search", "multi_web_content_read"]);
        assert_eq!(sync_list(&current, [true, true, false]), None);
    }

    #[test]
    fn schemas_match_unipi() {
        let s = search_schema();
        assert_eq!(s["required"], json!(["query"]));
        assert_eq!(s["properties"]["source"]["maximum"], 6);
        let r = read_schema();
        assert_eq!(
            r["properties"].as_object().map(serde_json::Map::len),
            Some(12)
        );
        assert_eq!(r["properties"]["source"]["maximum"], 4);
        assert_eq!(r["properties"]["url"]["anyOf"][1]["type"], "array");
        let m = summarize_schema();
        assert_eq!(m["required"], json!(["url"]));
        assert_eq!(rank(Some(2.0)), Some(2));
        assert_eq!(positive(Some(-1.0)), None);
    }
}
