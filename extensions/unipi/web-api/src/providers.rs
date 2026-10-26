//! Web providers (unipi `src/providers/*.ts`): registry with rankings, request builders and
//! response parsers. HTTP happens in the extension through `http.fetch`.

use std::fmt::Write as _;

use dom_query::Document;
use serde::Serialize;
use serde_json::{Value, json};

/// Capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cap {
    /// `web_search`.
    Search,
    /// `multi_web_content_read` providers (source ≥ 1).
    Read,
    /// `web_llm_summarize`.
    Summarize,
}

impl Cap {
    /// Name used in messages.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Search => "search",
            Self::Read => "read",
            Self::Summarize => "summarize",
        }
    }
}

/// unipi `WebProvider` metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Provider {
    /// Id (`providers.<id>` settings).
    pub id: &'static str,
    /// Display name.
    pub name: &'static str,
    /// Needs an API key.
    pub requires_key: bool,
    /// Env var with the key.
    pub env: Option<&'static str>,
    /// Ranks `(search, read, summarize)`; 0 = unsupported.
    pub rank: (u8, u8, u8),
    /// Enabled by default.
    pub default_enabled: bool,
    /// Hub blurb.
    pub blurb: &'static str,
}

impl Provider {
    /// Rank for `cap` (0 = unsupported).
    pub(crate) fn rank(&self, cap: Cap) -> u8 {
        match cap {
            Cap::Search => self.rank.0,
            Cap::Read => self.rank.1,
            Cap::Summarize => self.rank.2,
        }
    }
}

/// Every provider (unipi registration order) plus Oino's opt-in session-model summariser.
pub(crate) const PROVIDERS: &[Provider] = &[
    Provider {
        id: "wigolo",
        name: "wigolo (local)",
        requires_key: false,
        env: None,
        rank: (1, 1, 0),
        default_enabled: true,
        blurb: "Local web engine: multi-engine search and tiered fetch. Needs a running wigolo daemon (npx wigolo init), no key.",
    },
    Provider {
        id: "duckduckgo",
        name: "DuckDuckGo",
        requires_key: false,
        env: None,
        rank: (2, 0, 0),
        default_enabled: true,
        blurb: "Keyless web search.",
    },
    Provider {
        id: "jina-search",
        name: "Jina AI Search",
        requires_key: false,
        env: Some("JINA_API_KEY"),
        rank: (3, 0, 0),
        default_enabled: true,
        blurb: "Keyless web search via Jina.",
    },
    Provider {
        id: "jina-reader",
        name: "Jina AI Reader",
        requires_key: false,
        env: Some("JINA_API_KEY"),
        rank: (0, 2, 0),
        default_enabled: true,
        blurb: "Keyless URL-to-markdown reader.",
    },
    Provider {
        id: "serpapi",
        name: "SerpAPI",
        requires_key: true,
        env: Some("SERPAPI_KEY"),
        rank: (4, 0, 0),
        default_enabled: false,
        blurb: "Google results API. Paid key.",
    },
    Provider {
        id: "tavily",
        name: "Tavily",
        requires_key: true,
        env: Some("TAVILY_API_KEY"),
        rank: (5, 0, 0),
        default_enabled: false,
        blurb: "Research-oriented search API. Paid key.",
    },
    Provider {
        id: "firecrawl",
        name: "Firecrawl",
        requires_key: true,
        env: Some("FIRECRAWL_API_KEY"),
        rank: (0, 3, 0),
        default_enabled: false,
        blurb: "Scraping and crawl API. Paid key.",
    },
    Provider {
        id: "perplexity",
        name: "Perplexity",
        requires_key: true,
        env: Some("PERPLEXITY_API_KEY"),
        rank: (6, 4, 1),
        default_enabled: false,
        blurb: "Search and summarize via Perplexity. Paid key.",
    },
    Provider {
        id: "model",
        name: "Session model",
        requires_key: false,
        env: None,
        rank: (0, 0, 2),
        default_enabled: false,
        blurb: "Summarize with a configured model (smart-fetch + models.complete). Off by default.",
    },
];

/// Provider by id.
pub(crate) fn by_id(id: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|p| p.id == id)
}

/// unipi `getRankedProviders` over `available` (enabled, keyed) ids.
pub(crate) fn ranked(cap: Cap, available: impl Fn(&Provider) -> bool) -> Vec<&'static Provider> {
    let mut v: Vec<&Provider> = PROVIDERS
        .iter()
        .filter(|p| p.rank(cap) > 0 && available(p))
        .collect();
    v.sort_by_key(|p| p.rank(cap));
    v
}

/// unipi `selectProviderChain`.
pub(crate) fn chain(
    cap: Cap,
    source: Option<u8>,
    available: impl Fn(&Provider) -> bool,
) -> Result<Vec<&'static Provider>, String> {
    let avail = ranked(cap, available);
    if avail.is_empty() {
        let names: Vec<&str> = PROVIDERS
            .iter()
            .filter(|p| p.rank(cap) > 0 && p.id != "model")
            .map(|p| p.name)
            .collect();
        return Err(format!(
            "No {} provider configured.\n→ Run /unipi:settings (Web API) to enable providers and add API keys.\n→ Free options: wigolo (search + read, local), DuckDuckGo (search), Jina Reader (read).\n→ Available providers: {}",
            cap.as_str(),
            names.join(", ")
        ));
    }
    if let Some(rank) = source {
        return avail
            .iter()
            .find(|p| p.rank(cap) == rank)
            .map(|p| vec![*p])
            .ok_or_else(|| {
                let ranks: Vec<String> = avail.iter().map(|p| p.rank(cap).to_string()).collect();
                format!(
                    "No provider at rank {rank} for {}.\nAvailable ranks: {}",
                    cap.as_str(),
                    ranks.join(", ")
                )
            });
    }
    Ok(avail)
}

/// unipi `withProviderFallthrough` message for the collected failures.
pub(crate) fn fallthrough_error(failures: &[String]) -> String {
    if failures.len() == 1 {
        return failures[0].clone();
    }
    format!(
        "All {} providers failed:\n{}",
        failures.len(),
        failures
            .iter()
            .map(|f| format!("→ {f}"))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

/// unipi `SearchResult`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct SearchResult {
    /// Title.
    pub title: String,
    /// URL.
    pub url: String,
    /// Snippet.
    pub snippet: String,
}

/// A prepared request.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Req {
    /// `GET` or `POST`.
    pub post: bool,
    /// URL.
    pub url: String,
    /// Headers.
    pub headers: Vec<(String, String)>,
    /// JSON body.
    pub body: Option<Value>,
}

impl Req {
    fn get(url: String) -> Self {
        Self {
            post: false,
            url,
            headers: Vec::new(),
            body: None,
        }
    }

    fn post(url: &str, body: Value) -> Self {
        Self {
            post: true,
            url: url.to_owned(),
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: Some(body),
        }
    }

    fn header(mut self, k: &str, v: &str) -> Self {
        self.headers.push((k.to_owned(), v.to_owned()));
        self
    }

    fn bearer(self, key: Option<&str>) -> Self {
        match key.filter(|k| !k.is_empty()) {
            Some(k) => self.header("Authorization", &format!("Bearer {k}")),
            None => self,
        }
    }
}

/// `encodeURIComponent`.
pub(crate) fn encode_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// `URLSearchParams` value encoding (`+` for spaces).
pub(crate) fn encode_query(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b' ' => out.push('+'),
            b'*' | b'-' | b'.' | b'_' => out.push(char::from(b)),
            _ if b.is_ascii_alphanumeric() => out.push(char::from(b)),
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

/// `decodeURIComponent` (`None` on malformed sequences).
pub(crate) fn decode_component(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// `DuckDuckGo` browser UA (unipi).
pub(crate) const DDG_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

/// unipi `decodeEntities`.
pub(crate) fn decode_entities(text: &str) -> String {
    let mut s = text
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");
    while let Some(start) = s.find("&#") {
        let rest = &s[start + 2..];
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() || !rest[digits.len()..].starts_with(';') {
            break;
        }
        let ch = digits
            .parse::<u32>()
            .ok()
            .and_then(char::from_u32)
            .unwrap_or('\u{FFFD}');
        s.replace_range(start..=(start + 2 + digits.len()), &ch.to_string());
    }
    s
}

/// unipi `unwrapRedirect`.
pub(crate) fn unwrap_redirect(href: &str) -> String {
    let decoded = decode_entities(href);
    if let Some(pos) = decoded
        .find("uddg=")
        .filter(|p| *p > 0 && matches!(decoded.as_bytes()[p - 1], b'?' | b'&'))
    {
        let value = decoded[pos + 5..].split('&').next().unwrap_or("");
        if let Some(url) = decode_component(value) {
            return url;
        }
    }
    if let Some(rest) = decoded.strip_prefix("//") {
        return format!("https://{rest}");
    }
    decoded
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// unipi `parseDDGResults`: each snippet pairs with the result whose link precedes it.
pub(crate) fn parse_ddg(html: &str) -> Vec<SearchResult> {
    let doc = Document::from(html);
    let mut out: Vec<SearchResult> = Vec::new();
    let mut open = false;
    for node in doc.select("a.result__a, a.result__snippet").nodes() {
        if node.has_class("result__a") {
            let url =
                unwrap_redirect(&node.attr("href").map(|h| h.to_string()).unwrap_or_default());
            let title = squash(&node.text());
            if url.is_empty() || title.is_empty() {
                open = false;
                continue;
            }
            out.push(SearchResult {
                title,
                url,
                snippet: String::new(),
            });
            open = true;
        } else if open {
            if let Some(last) = out.last_mut()
                && last.snippet.is_empty()
            {
                last.snippet = squash(&node.text());
            }
            open = false;
        }
    }
    out
}

/// `DuckDuckGo` HTML search request.
pub(crate) fn ddg_request(query: &str) -> Req {
    Req::get(format!(
        "https://html.duckduckgo.com/html/?q={}",
        encode_component(query)
    ))
    .header("User-Agent", DDG_UA)
}

/// Jina search request.
pub(crate) fn jina_search_request(query: &str, key: Option<&str>) -> Req {
    Req::get(format!("https://s.jina.ai/{}", encode_component(query)))
        .header("Accept", "application/json")
        .bearer(key)
}

/// Jina search response.
pub(crate) fn parse_jina_search(v: &Value) -> Vec<SearchResult> {
    v.get("data")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|i| SearchResult {
                    title: str_at(i, "title"),
                    url: str_at(i, "url"),
                    snippet: {
                        let s = str_at(i, "snippet");
                        if s.is_empty() {
                            str_at(i, "description")
                        } else {
                            s
                        }
                    },
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Jina reader request (`https://r.jina.ai/<url>`).
pub(crate) fn jina_read_request(url: &str, key: Option<&str>) -> Req {
    Req::get(format!("https://r.jina.ai/{url}"))
        .header("Accept", "application/json")
        .bearer(key)
}

/// Jina reader response → `(url, content)`.
pub(crate) fn parse_jina_read(v: &Value, url: &str) -> Option<(String, String)> {
    let d = v.get("data")?;
    let content = d.get("content")?.as_str()?.to_owned();
    let u = d
        .get("url")
        .and_then(Value::as_str)
        .filter(|u| !u.is_empty())
        .unwrap_or(url);
    Some((u.to_owned(), content))
}

/// `SerpAPI` request.
pub(crate) fn serpapi_request(query: &str, key: &str) -> Req {
    Req::get(format!(
        "https://serpapi.com/search?q={}&api_key={}&engine=google",
        encode_query(query),
        encode_query(key)
    ))
}

/// `SerpAPI` response.
pub(crate) fn parse_serpapi(v: &Value) -> Vec<SearchResult> {
    v.get("organic_results")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|i| SearchResult {
                    title: str_at(i, "title"),
                    url: str_at(i, "link"),
                    snippet: str_at(i, "snippet"),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Tavily request.
pub(crate) fn tavily_request(query: &str, key: &str) -> Req {
    Req::post(
        "https://api.tavily.com/search",
        json!({"api_key": key, "query": query, "search_depth": "basic", "include_answer": false, "include_raw_content": false}),
    )
}

/// Tavily response.
pub(crate) fn parse_tavily(v: &Value) -> Vec<SearchResult> {
    v.get("results")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|i| SearchResult {
                    title: str_at(i, "title"),
                    url: str_at(i, "url"),
                    snippet: str_at(i, "content"),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Firecrawl scrape request (v1 API; unipi used the retired v0 `pageOptions`).
pub(crate) fn firecrawl_request(url: &str, key: &str) -> Req {
    Req::post(
        "https://api.firecrawl.dev/v1/scrape",
        json!({"url": url, "formats": ["markdown"], "onlyMainContent": true}),
    )
    .bearer(Some(key))
}

/// Firecrawl response → markdown.
pub(crate) fn parse_firecrawl(v: &Value) -> Result<String, String> {
    if v.get("success").and_then(Value::as_bool) != Some(true) {
        return Err("Firecrawl extraction failed".into());
    }
    v.pointer("/data/markdown")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| "Firecrawl extraction failed".into())
}

/// Perplexity chat request.
pub(crate) fn perplexity_request(model: &str, system: &str, user: &str, key: &str) -> Req {
    Req::post(
        "https://api.perplexity.ai/chat/completions",
        json!({"model": model, "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}]}),
    )
    .bearer(Some(key))
}

/// unipi perplexity search system prompt.
pub(crate) const PERPLEXITY_SEARCH_SYSTEM: &str = "You are a search assistant. Return search results as a JSON array with objects containing 'title', 'url', and 'snippet' fields.";
/// unipi default summarize prompt.
pub(crate) const DEFAULT_SUMMARY_PROMPT: &str =
    "Summarize the content of this URL concisely, highlighting key points.";
/// unipi perplexity read prompt.
pub(crate) const PERPLEXITY_READ_PROMPT: &str =
    "Extract and return the full content of this URL as markdown.";

/// Perplexity reply text.
pub(crate) fn perplexity_text(v: &Value) -> String {
    v.pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

/// unipi perplexity search parsing: a JSON array in the reply, else citations.
pub(crate) fn parse_perplexity_search(v: &Value) -> Vec<SearchResult> {
    let content = {
        let t = perplexity_text(v);
        if t.is_empty() { "[]".to_owned() } else { t }
    };
    if let (Some(start), Some(end)) = (content.find('['), content.rfind(']'))
        && start < end
        && let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&content[start..=end])
    {
        return items
            .iter()
            .map(|i| SearchResult {
                title: str_at(i, "title"),
                url: str_at(i, "url"),
                snippet: str_at(i, "snippet"),
            })
            .collect();
    }
    let snippet: String = content.chars().take(200).collect();
    let citations = v
        .pointer("/choices/0/citations")
        .or_else(|| v.get("citations"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    citations
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let url = c
                .get("url")
                .and_then(Value::as_str)
                .or_else(|| c.as_str())?;
            Some(SearchResult {
                title: format!("Result {}", i + 1),
                url: url.to_owned(),
                snippet: snippet.clone(),
            })
        })
        .collect()
}

/// wigolo search body.
pub(crate) fn wigolo_search_body(query: &str, max: u32) -> Value {
    json!({"query": query, "max_results": max})
}

/// unipi wigolo `toSearchResult` over the response.
pub(crate) fn parse_wigolo_search(v: &Value) -> Result<Vec<SearchResult>, String> {
    if let Some(e) = v
        .get("error")
        .and_then(Value::as_str)
        .filter(|e| !e.is_empty())
    {
        return Err(format!("wigolo search failed: {e}"));
    }
    Ok(v.get("results")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|r| {
                    let url = str_at(r, "url");
                    if url.is_empty() {
                        return None;
                    }
                    let snippet = ["excerpt", "snippet", "content", "description"]
                        .iter()
                        .map(|k| str_at(r, k))
                        .find(|s| !s.is_empty())
                        .unwrap_or_default();
                    let title = {
                        let t = str_at(r, "title");
                        if t.is_empty() { url.clone() } else { t }
                    };
                    Some(SearchResult {
                        title,
                        url,
                        snippet,
                    })
                })
                .collect()
        })
        .unwrap_or_default())
}

/// wigolo fetch response → `(url, markdown)`.
pub(crate) fn parse_wigolo_fetch(v: &Value, url: &str) -> Result<(String, String), String> {
    if let Some(e) = v
        .get("error")
        .and_then(Value::as_str)
        .filter(|e| !e.is_empty())
    {
        return Err(format!("wigolo fetch failed: {e}"));
    }
    let content = str_at(v, "markdown");
    if content.is_empty() {
        return Err(format!("wigolo returned no content for {url}"));
    }
    let u = str_at(v, "url");
    Ok((if u.is_empty() { url.to_owned() } else { u }, content))
}

/// wigolo daemon unreachable (unipi `NOT_RUNNING_MESSAGE`, Oino talks to the REST daemon).
pub(crate) fn wigolo_unavailable(base: &str, cause: &str) -> String {
    format!(
        "wigolo is not running at {base}.\n→ Install and start it:  npm install -g wigolo && npx wigolo init\n→ Check health:   npx wigolo doctor\n→ wigolo is a separate AGPL-licensed project and is not bundled with Oino.\n→ Disable it in /unipi:settings (Web API) to silence this.\n→ Cause: {cause}"
    )
}

/// Missing key message (unipi).
pub(crate) fn key_required(p: &Provider) -> String {
    format!(
        "{} requires an API key. Set {} environment variable or configure via /unipi:settings (Web API)",
        p.name,
        p.env.unwrap_or("its API key")
    )
}

fn str_at(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DDG_HTML: &str = r#"<div class="result">
    <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.postgresql.org%2Fdocs%2Fcurrent%2Flogical%2Dreplication.html&amp;rut=274da557">PostgreSQL: Documentation: 18: Chapter 29. Logical Replication</a>
  <a class="result__snippet" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.postgresql.org%2F&amp;rut=274da557"><b>Logical</b> <b>replication</b> is a method of replicating data objects and their changes, based upon their <b>replication</b> identity.</a>
</div><div class="result">
    <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fguide&amp;rut=abc">A Guide &amp; Reference</a>
  <a class="result__snippet">Plain snippet with no markup.</a></div>"#;

    #[test]
    fn rankings_like_unipi() {
        let all = |_: &Provider| true;
        let search: Vec<&str> = ranked(Cap::Search, all).iter().map(|p| p.id).collect();
        assert_eq!(
            search,
            [
                "wigolo",
                "duckduckgo",
                "jina-search",
                "serpapi",
                "tavily",
                "perplexity"
            ]
        );
        let read: Vec<&str> = ranked(Cap::Read, all).iter().map(|p| p.id).collect();
        assert_eq!(read, ["wigolo", "jina-reader", "firecrawl", "perplexity"]);
        let summarize: Vec<&str> = ranked(Cap::Summarize, all).iter().map(|p| p.id).collect();
        assert_eq!(summarize, ["perplexity", "model"]);
        for cap in [Cap::Search, Cap::Read] {
            let ranks: Vec<u8> = ranked(cap, all).iter().map(|p| p.rank(cap)).collect();
            let expected: Vec<u8> = (1..=u8::try_from(ranks.len()).unwrap_or(0)).collect();
            assert_eq!(
                ranks, expected,
                "contiguous ranks from 1 (0 = smart-fetch for read)"
            );
        }
    }

    #[test]
    fn chain_selection() {
        let enabled = |p: &Provider| p.default_enabled;
        let c = chain(Cap::Search, None, enabled).unwrap_or_default();
        assert_eq!(
            c.iter().map(|p| p.id).collect::<Vec<_>>(),
            ["wigolo", "duckduckgo", "jina-search"]
        );
        let one = chain(Cap::Search, Some(2), enabled).unwrap_or_default();
        assert_eq!(one.iter().map(|p| p.id).collect::<Vec<_>>(), ["duckduckgo"]);
        let missing = chain(Cap::Search, Some(5), enabled)
            .err()
            .unwrap_or_default();
        assert_eq!(
            missing,
            "No provider at rank 5 for search.\nAvailable ranks: 1, 2, 3"
        );
        let none = chain(Cap::Summarize, None, enabled)
            .err()
            .unwrap_or_default();
        assert!(
            none.starts_with("No summarize provider configured.\n→ Run /unipi:settings (Web API)")
        );
        assert!(none.ends_with("Available providers: Perplexity"));
    }

    #[test]
    fn fallthrough_messages() {
        assert_eq!(fallthrough_error(&["A: x".into()]), "A: x");
        assert_eq!(
            fallthrough_error(&["A: x".into(), "B: y".into()]),
            "All 2 providers failed:\n→ A: x\n→ B: y"
        );
    }

    #[test]
    fn duckduckgo_like_unipi() {
        assert_eq!(decode_entities("A &amp; B"), "A & B");
        assert_eq!(decode_entities("&lt;tag&gt;"), "<tag>");
        assert_eq!(decode_entities("&quot;quoted&quot;"), "\"quoted\"");
        assert_eq!(
            decode_entities("it&#x27;s it&#39;s a&nbsp;b"),
            "it's it's a b"
        );
        assert_eq!(decode_entities("caf&#233;"), "café");
        assert_eq!(decode_entities("nothing to do"), "nothing to do");
        assert_eq!(
            unwrap_redirect(
                "//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fa%3Fb%3D1&amp;rut=x"
            ),
            "https://example.com/a?b=1"
        );
        assert_eq!(
            unwrap_redirect("//example.com/page"),
            "https://example.com/page"
        );
        assert_eq!(
            unwrap_redirect("https://example.com/page"),
            "https://example.com/page"
        );
        assert_eq!(
            unwrap_redirect("//duckduckgo.com/l/?uddg=%E0%A4%A"),
            "https://duckduckgo.com/l/?uddg=%E0%A4%A"
        );
        let r = parse_ddg(DDG_HTML);
        assert_eq!(r.len(), 2);
        assert_eq!(
            r[0].url,
            "https://www.postgresql.org/docs/current/logical-replication.html"
        );
        assert!(r[0].snippet.starts_with("Logical replication is a method"));
        assert_eq!(r[1].title, "A Guide & Reference");
        assert_eq!(r[1].snippet, "Plain snippet with no markup.");
        let gap = r#"<h2><a class="result__a" href="https://a.test">First</a></h2><h2><a class="result__a" href="https://b.test">Second</a></h2><a class="result__snippet">Belongs to the second result only.</a>"#;
        let r = parse_ddg(gap);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].snippet, "");
        assert_eq!(r[1].snippet, "Belongs to the second result only.");
    }

    #[test]
    fn request_shapes() {
        assert_eq!(
            ddg_request("a b&c").url,
            "https://html.duckduckgo.com/html/?q=a%20b%26c"
        );
        let j = jina_search_request("q", Some("k"));
        assert!(
            j.headers
                .contains(&("Authorization".into(), "Bearer k".into()))
        );
        assert_eq!(
            jina_read_request("https://x", None).url,
            "https://r.jina.ai/https://x"
        );
        assert_eq!(
            serpapi_request("a b", "k&1").url,
            "https://serpapi.com/search?q=a+b&api_key=k%261&engine=google"
        );
        let t = tavily_request("q", "k");
        assert_eq!(
            t.body.as_ref().map(|b| b["search_depth"].clone()),
            Some(json!("basic"))
        );
        let f = firecrawl_request("https://x", "k");
        assert_eq!(
            f.body.as_ref().map(|b| b["onlyMainContent"].clone()),
            Some(json!(true))
        );
        assert_eq!(encode_component("a/b?c=d e"), "a%2Fb%3Fc%3Dd%20e");
        assert_eq!(decode_component("%E2%9C%93"), Some("✓".into()));
        assert_eq!(decode_component("%zz"), None);
    }

    #[test]
    fn response_parsing() {
        assert_eq!(
            parse_jina_search(&json!({"data": [{"title": "t", "url": "u", "snippet": "s"}]})).len(),
            1
        );
        assert_eq!(
            parse_jina_read(&json!({"data": {"content": "md"}}), "https://x"),
            Some(("https://x".into(), "md".into()))
        );
        assert_eq!(
            parse_serpapi(
                &json!({"organic_results": [{"title": "t", "link": "l", "snippet": "s"}]})
            )[0]
            .url,
            "l"
        );
        assert_eq!(
            parse_tavily(&json!({"results": [{"title": "t", "url": "u", "content": "c"}]}))[0]
                .snippet,
            "c"
        );
        assert_eq!(
            parse_firecrawl(&json!({"success": true, "data": {"markdown": "# x"}})),
            Ok("# x".into())
        );
        assert!(parse_firecrawl(&json!({"success": false})).is_err());
        let px = json!({"choices": [{"message": {"content": "Here: [{\"title\": \"a\", \"url\": \"https://a\", \"snippet\": \"s\"}]"}}]});
        assert_eq!(parse_perplexity_search(&px)[0].url, "https://a");
        let cited = json!({"choices": [{"message": {"content": "plain text"}, "citations": [{"url": "https://c"}]}]});
        assert_eq!(parse_perplexity_search(&cited)[0].title, "Result 1");
        let w = json!({"results": [{"url": "https://w", "excerpt": "e"}, {"title": "no url"}]});
        let ws = parse_wigolo_search(&w).unwrap_or_default();
        assert_eq!(ws.len(), 1);
        assert_eq!(ws[0].title, "https://w");
        assert_eq!(ws[0].snippet, "e");
        assert!(
            parse_wigolo_search(&json!({"error": "boom"}))
                .is_err_and(|e| e == "wigolo search failed: boom")
        );
        assert!(
            parse_wigolo_fetch(&json!({"markdown": ""}), "u")
                .is_err_and(|e| e == "wigolo returned no content for u")
        );
        assert_eq!(
            parse_wigolo_fetch(&json!({"markdown": "m", "url": "v"}), "u"),
            Ok(("v".into(), "m".into()))
        );
        let serp = by_id("serpapi").unwrap_or(&PROVIDERS[0]);
        assert_eq!(
            key_required(serp),
            "SerpAPI requires an API key. Set SERPAPI_KEY environment variable or configure via /unipi:settings (Web API)"
        );
    }
}
