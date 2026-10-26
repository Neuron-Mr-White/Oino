//! `web-api` settings (unipi `src/settings.ts`, `src/engine/{constants,profiles}.ts`).

use serde_json::{Map, Value, json};
use unipi_kit::settings::{Field, Kind, Namespace, Section, Unit, choice, plain};

use crate::providers::PROVIDERS;

/// Extension id.
pub(crate) const EXTENSION_ID: &str = "unipi.web-api";
/// Namespace.
pub(crate) const NAMESPACE: &str = "web-api";
/// unipi `DEFAULT_BROWSER`.
pub(crate) const DEFAULT_BROWSER: &str = "chrome_145";
/// unipi `DEFAULT_OS`.
pub(crate) const DEFAULT_OS: &str = "windows";
/// unipi `DEFAULT_TIMEOUT_MS`.
pub(crate) const DEFAULT_TIMEOUT_MS: u64 = 15_000;
/// unipi `DEFAULT_BATCH_CONCURRENCY`.
pub(crate) const DEFAULT_BATCH_CONCURRENCY: usize = 8;
/// Default wigolo daemon (wigolo `daemonHost:daemonPort`).
pub(crate) const DEFAULT_WIGOLO_URL: &str = "http://127.0.0.1:3333";
/// Default Perplexity model (unipi's `llama-3.1-sonar-small-128k-online` was retired).
pub(crate) const DEFAULT_PERPLEXITY_MODEL: &str = "sonar";

/// unipi `BROWSER_PROFILES`.
pub(crate) const BROWSER_PROFILES: &[&str] = &[
    "chrome_100",
    "chrome_101",
    "chrome_104",
    "chrome_107",
    "chrome_110",
    "chrome_116",
    "chrome_119",
    "chrome_120",
    "chrome_123",
    "chrome_124",
    "chrome_131",
    "chrome_133",
    "chrome_145",
    "firefox_120",
    "firefox_133",
    "safari_15_6_1",
    "safari_16_0",
    "safari_17_0",
    "edge_101",
];

/// unipi `OS_PROFILES`.
pub(crate) const OS_PROFILES: &[&str] = &["windows", "macos", "linux", "android", "ios"];

/// unipi `resolveBrowserProfile`: exact → newest prefix match → pass-through.
pub(crate) fn resolve_browser(browser: Option<&str>) -> String {
    let Some(b) = browser.filter(|b| !b.is_empty()) else {
        return DEFAULT_BROWSER.to_owned();
    };
    if BROWSER_PROFILES.contains(&b) {
        return b.to_owned();
    }
    let prefix = b.to_lowercase();
    BROWSER_PROFILES
        .iter()
        .rev()
        .find(|p| p.starts_with(&prefix))
        .map_or_else(|| b.to_owned(), |p| (*p).to_owned())
}

/// unipi `DEFAULT_CONFIG`.
pub(crate) fn defaults() -> Value {
    let mut providers = Map::new();
    for p in PROVIDERS {
        providers.insert(p.id.to_owned(), json!({"enabled": p.default_enabled}));
    }
    providers.insert(
        "wigolo".into(),
        json!({"enabled": true, "url": DEFAULT_WIGOLO_URL}),
    );
    providers.insert(
        "perplexity".into(),
        json!({"enabled": false, "model": DEFAULT_PERPLEXITY_MODEL}),
    );
    json!({"providers": providers, "smartFetch": {}})
}

/// The `web-api` namespace.
#[allow(clippy::too_many_lines, reason = "one declarative schema")]
pub(crate) fn namespace() -> Namespace {
    let pages: Vec<Field> = PROVIDERS
        .iter()
        .map(|p| {
            let mut fields = vec![
                Field::boolean(&format!("providers.{}.enabled", p.id), "Enabled")
                    .description("Let this provider serve requests of its kind."),
            ];
            match p.id {
                "wigolo" => fields.push(
                    Field::string("providers.wigolo.url", "Daemon URL")
                        .empty_label(DEFAULT_WIGOLO_URL)
                        .description("Where the wigolo REST daemon listens."),
                ),
                "model" => fields.push(
                    Field::model("providers.model.model", "Model")
                        .with_kind(|k| {
                            if let Kind::Model {
                                empty_option,
                                empty_label,
                                ..
                            } = k
                            {
                                *empty_option = Some("session model".into());
                                *empty_label = Some("session model".into());
                            }
                        })
                        .description("Model that writes web_llm_summarize summaries."),
                ),
                _ => fields.push(
                    Field::secret(&format!("providers.{}.apiKey", p.id), "API key")
                        .empty_label("unset (public access)")
                        .description("Key sent with every request to this provider."),
                ),
            }
            if p.id == "perplexity" {
                fields.push(
                    Field::string("providers.perplexity.model", "Model")
                        .empty_label(DEFAULT_PERPLEXITY_MODEL)
                        .description("Perplexity model used for search, read and summarize."),
                );
            }
            Field::page(
                &format!("providers.{}", p.id),
                p.id,
                vec![Section::new(p.id, p.id).fields(fields)],
            )
            .description(p.blurb)
        })
        .collect();
    let number = |key: &str, label: &str, min: f64, unit: Option<Unit>| {
        Field::number(key, label).with_kind(|k| {
            if let Kind::Number {
                min: m, unit: u, ..
            } = k
            {
                *m = Some(min);
                *u = unit;
            }
        })
    };
    Namespace::new(NAMESPACE, "Web API", EXTENSION_ID)
        .defaults(defaults())
        .section(
            Section::new("providers", "Providers")
                .description("Per-provider pages — enable + API key (tavily, serpapi, …)")
                .fields(pages),
        )
        .section(
            Section::new("smart-fetch", "Smart fetch")
                .advanced(true)
                .field(
                    Field::enumeration(
                        "smartFetch.browser",
                        "Browser profile",
                        BROWSER_PROFILES.iter().rev().map(|b| plain(b)).collect(),
                    )
                    .description(
                        "Browser identity the fetch engine presents to sites; newest first.",
                    ),
                )
                .field(
                    Field::enumeration(
                        "smartFetch.os",
                        "OS profile",
                        OS_PROFILES.iter().map(|o| plain(o)).collect(),
                    )
                    .description("Operating system the fetch engine claims to run on."),
                )
                .field(
                    Field::enumeration(
                        "smartFetch.includeReplies",
                        "Include replies",
                        vec![
                            choice("true", "yes", "fetch and include comment replies"),
                            choice("false", "no", "main content only"),
                            choice(
                                "extractors",
                                "extractors (default)",
                                "each extractor decides per site",
                            ),
                        ],
                    )
                    .description("Whether comment and reply sections are fetched and kept."),
                )
                .field(
                    number("smartFetch.maxChars", "Max chars", 1000.0, None)
                        .description("Content cap per URL before truncation."),
                )
                .field(
                    number("smartFetch.timeoutMs", "Timeout", 1000.0, Some(Unit::Ms))
                        .description("Per-request timeout."),
                )
                .field(
                    number(
                        "smartFetch.batchConcurrency",
                        "Batch concurrency",
                        1.0,
                        None,
                    )
                    .description("URLs fetched at once in batch mode."),
                )
                .field(
                    Field::boolean("smartFetch.removeImages", "Remove images")
                        .description("Strip image references from fetched content."),
                )
                .field(
                    Field::action("cache.clear", "Clear web cache…", "unipi:web-cache-clear")
                        .description("Drop every cached response (with a confirm step)."),
                ),
        )
}

/// Expands flat dotted keys over nested values (hub writes are dotted).
pub(crate) fn expand_dotted(v: &Value) -> Value {
    let Value::Object(map) = v else {
        return v.clone();
    };
    let mut out = Value::Object(Map::new());
    let mut flat = Vec::new();
    for (k, val) in map {
        if k.contains('.') {
            flat.push((k, val));
        } else if let Value::Object(o) = &mut out {
            o.insert(k.clone(), expand_dotted(val));
        }
    }
    for (k, val) in flat {
        out = unipi_kit::settings::set_field(&out, k, val.clone());
    }
    out
}

/// unipi `SmartFetchSettings` resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SmartFetch {
    /// Browser profile.
    pub browser: String,
    /// OS profile.
    pub os: String,
    /// Max chars.
    pub max_chars: usize,
    /// Timeout.
    pub timeout_ms: u64,
    /// Batch concurrency.
    pub batch_concurrency: usize,
    /// Remove images.
    pub remove_images: bool,
    /// `Some(true|false)` or `None` = extractors.
    pub include_replies: Option<bool>,
}

/// unipi `loadSmartFetchSettings` (hub enum strings coerced back to booleans).
pub(crate) fn smart_fetch(values: &Value) -> SmartFetch {
    let s = values
        .get("smartFetch")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let num = |k: &str| {
        s.get(k)
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite() && *n > 0.0)
    };
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "positive settings"
    )]
    SmartFetch {
        browser: s
            .get("browser")
            .and_then(Value::as_str)
            .unwrap_or(DEFAULT_BROWSER)
            .to_owned(),
        os: s
            .get("os")
            .and_then(Value::as_str)
            .unwrap_or(DEFAULT_OS)
            .to_owned(),
        max_chars: num("maxChars").map_or(crate::format::DEFAULT_MAX_CHARS, |n| n as usize),
        timeout_ms: num("timeoutMs").map_or(DEFAULT_TIMEOUT_MS, |n| n as u64),
        batch_concurrency: num("batchConcurrency")
            .map_or(DEFAULT_BATCH_CONCURRENCY, |n| n as usize),
        remove_images: s
            .get("removeImages")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        include_replies: parse_replies(s.get("includeReplies")),
    }
}

/// `true`/`"true"` → yes, `false`/`"false"` → no, anything else → extractors.
pub(crate) fn parse_replies(v: Option<&Value>) -> Option<bool> {
    match v {
        Some(Value::Bool(b)) => Some(*b),
        Some(Value::String(s)) if s == "true" => Some(true),
        Some(Value::String(s)) if s == "false" => Some(false),
        _ => None,
    }
}

/// unipi `isProviderEnabled` (`enabled !== false`, defaults applied).
pub(crate) fn provider_enabled(values: &Value, id: &str) -> bool {
    values
        .pointer(&format!("/providers/{id}/enabled"))
        .and_then(Value::as_bool)
        .unwrap_or_else(|| crate::providers::by_id(id).is_some_and(|p| p.default_enabled))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles() {
        assert_eq!(resolve_browser(None), "chrome_145");
        assert_eq!(resolve_browser(Some("firefox_120")), "firefox_120");
        assert_eq!(
            resolve_browser(Some("chrome")),
            "chrome_145",
            "newest prefix match"
        );
        assert_eq!(resolve_browser(Some("safari")), "safari_17_0");
        assert_eq!(
            resolve_browser(Some("brave_1")),
            "brave_1",
            "unknown passes through"
        );
    }

    #[test]
    fn defaults_and_schema() {
        let d = defaults();
        assert!(provider_enabled(&d, "duckduckgo"));
        assert!(!provider_enabled(&d, "serpapi"));
        assert!(!provider_enabled(&d, "model"));
        assert_eq!(d["providers"]["wigolo"]["url"], DEFAULT_WIGOLO_URL);
        let v = namespace().to_json();
        assert_eq!(v["namespace"], "web-api");
        let pages = &v["sections"][0]["fields"];
        assert_eq!(pages.as_array().map(Vec::len), Some(PROVIDERS.len()));
        assert_eq!(pages[4]["key"], "providers.serpapi");
        assert_eq!(pages[4]["sections"][0]["fields"][1]["type"], "secret");
        let sf = &v["sections"][1];
        assert_eq!(sf["advanced"], true);
        assert_eq!(
            sf["fields"][0]["options"][0]["value"], "edge_101",
            "newest first (reversed list)"
        );
        assert_eq!(
            sf["fields"]
                .as_array()
                .and_then(|f| f.last())
                .map(|f| f["command"].clone()),
            Some(json!("unipi:web-cache-clear"))
        );
    }

    #[test]
    fn smart_fetch_settings() {
        let s = smart_fetch(&json!({}));
        assert_eq!(s.max_chars, 50_000);
        assert_eq!(s.timeout_ms, 15_000);
        assert_eq!(s.batch_concurrency, 8);
        assert_eq!(s.include_replies, None);
        let s = smart_fetch(
            &json!({"smartFetch": {"includeReplies": "false", "maxChars": 2000, "removeImages": true}}),
        );
        assert_eq!(s.include_replies, Some(false));
        assert_eq!(s.max_chars, 2000);
        assert!(s.remove_images);
        assert_eq!(parse_replies(Some(&json!(true))), Some(true));
        let flat = expand_dotted(
            &json!({"providers": {"serpapi": {"enabled": false}}, "providers.serpapi.enabled": true}),
        );
        assert!(provider_enabled(&flat, "serpapi"));
    }
}
