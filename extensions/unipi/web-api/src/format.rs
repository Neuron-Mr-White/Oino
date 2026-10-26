//! Results and formatting (unipi `src/engine/{types,format}.ts`).

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// unipi `DEFAULT_MAX_CHARS`.
pub(crate) const DEFAULT_MAX_CHARS: usize = 50_000;
/// unipi `TRUNCATION_MARKER`.
pub(crate) const TRUNCATION_MARKER: &str = "\n\n... [truncated]";

/// unipi `FetchResult`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FetchResult {
    /// Requested URL.
    pub url: String,
    /// URL after redirects.
    pub final_url: String,
    /// Title.
    pub title: String,
    /// Author.
    pub author: String,
    /// Publication date.
    pub published: String,
    /// Site name.
    pub site: String,
    /// Language.
    pub language: String,
    /// Words in `content`.
    pub word_count: usize,
    /// Content.
    pub content: String,
    /// `markdown | html | text | json`.
    pub format: String,
    /// Response MIME type.
    pub mime_type: String,
}

/// Metadata for [`create_result`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Meta {
    /// Title.
    pub title: String,
    /// Author.
    pub author: String,
    /// Published.
    pub published: String,
    /// Site.
    pub site: String,
    /// Language.
    pub language: String,
}

/// unipi `createResult`.
pub(crate) fn create_result(
    url: &str,
    final_url: &str,
    content: String,
    meta: &Meta,
    format: &str,
    mime: &str,
) -> FetchResult {
    FetchResult {
        url: url.to_owned(),
        final_url: final_url.to_owned(),
        title: meta.title.clone(),
        author: meta.author.clone(),
        published: meta.published.clone(),
        site: meta.site.clone(),
        language: meta.language.clone(),
        word_count: content.split_whitespace().count(),
        content,
        format: format.to_owned(),
        mime_type: mime.to_owned(),
    }
}

/// unipi `FetchError` (structured failure).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FetchError {
    /// Message.
    pub error: String,
    /// Code (`invalid_url`, `http_error`, `timeout`, …).
    pub code: String,
    /// Phase (`validation`, `connecting`, `waiting`, …).
    pub phase: String,
    /// Retryable.
    pub retryable: bool,
    /// URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Final URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_url: Option<String>,
    /// HTTP status.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_code: Option<u16>,
    /// Timeout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

impl FetchError {
    /// unipi `createError`.
    pub(crate) fn new(
        code: &str,
        phase: &str,
        message: impl Into<String>,
        retryable: bool,
    ) -> Self {
        Self {
            error: message.into(),
            code: code.to_owned(),
            phase: phase.to_owned(),
            retryable,
            ..Self::default()
        }
    }

    /// With the URLs.
    #[must_use]
    pub(crate) fn at(mut self, url: &str, final_url: &str) -> Self {
        self.url = Some(url.to_owned());
        self.final_url = Some(final_url.to_owned());
        self
    }
}

/// One batch item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BatchItem {
    /// Fetched.
    Done(FetchResult),
    /// Failed.
    Error(FetchError),
}

/// unipi `truncateContent`.
pub(crate) fn truncate(content: &str, max_chars: usize) -> String {
    let len = content.chars().count();
    if content.is_empty() || len <= max_chars {
        return content.to_owned();
    }
    let target = max_chars.saturating_sub(TRUNCATION_MARKER.chars().count());
    let mut truncated: String = content.chars().take(target).collect();
    if let Some(last_space) = truncated.rfind(' ') {
        let pos_chars = truncated[..last_space].chars().count();
        #[allow(clippy::cast_precision_loss, reason = "char counts are small")]
        if pos_chars as f64 > target as f64 * 0.8 {
            truncated.truncate(last_space);
        }
    }
    format!("{}{TRUNCATION_MARKER}", truncated.trim())
}

macro_rules! re {
    ($name:ident, $pat:expr) => {
        static $name: LazyLock<Regex> =
            LazyLock::new(|| Regex::new($pat).unwrap_or_else(|_| unreachable!()));
    };
}

re!(HEADERS, r"(?m)^#{1,6}\s+");
re!(BOLD_ITALIC3, r"\*\*\*(.*?)\*\*\*");
re!(BOLD2, r"\*\*(.*?)\*\*");
re!(ITALIC1, r"\*(.*?)\*");
re!(UNDER3, r"___(.*?)___");
re!(UNDER2, r"__(.*?)__");
re!(UNDER1, r"_(.*?)_");
re!(IMAGES, r"!\[([^\]]*)\]\([^)]+\)");
re!(LINKS, r"\[([^\]]+)\]\([^)]+\)");
re!(FENCES, r"```[\s\S]*?```");
re!(INLINE_CODE, r"`([^`]+)`");
re!(RULES, r"(?m)^[-*_]{3,}$");
re!(QUOTES, r"(?m)^>\s+");
re!(BULLETS, r"(?m)^[\s]*[-*+]\s+");
re!(NUMBERED, r"(?m)^[\s]*\d+\.\s+");
re!(BLANKS, r"\n{3,}");

/// unipi `stripMarkdown` (images go before links so `![alt](src)` leaves nothing).
pub(crate) fn strip_markdown(md: &str) -> String {
    let mut t = HEADERS.replace_all(md, "").into_owned();
    for re in [
        &*BOLD_ITALIC3,
        &*BOLD2,
        &*ITALIC1,
        &*UNDER3,
        &*UNDER2,
        &*UNDER1,
    ] {
        t = re.replace_all(&t, "$1").into_owned();
    }
    t = IMAGES.replace_all(&t, "").into_owned();
    t = LINKS.replace_all(&t, "$1").into_owned();
    t = FENCES.replace_all(&t, "").into_owned();
    t = INLINE_CODE.replace_all(&t, "$1").into_owned();
    for re in [&*RULES, &*QUOTES, &*BULLETS, &*NUMBERED] {
        t = re.replace_all(&t, "").into_owned();
    }
    BLANKS.replace_all(&t, "\n\n").trim().to_owned()
}

/// Drops markdown image references (`removeImages`).
pub(crate) fn remove_images(md: &str) -> String {
    IMAGES.replace_all(md, "").into_owned()
}

/// unipi `formatContent`.
pub(crate) fn format_content(result: &FetchResult, format: &str, max_chars: usize) -> String {
    let content = match format {
        "json" => serde_json::to_string_pretty(result).unwrap_or_default(),
        "text" => strip_markdown(&result.content),
        _ => result.content.clone(),
    };
    truncate(&content, max_chars)
}

/// unipi `formatSingleResult`.
pub(crate) fn format_single(r: &FetchResult, verbose: bool) -> String {
    let mut lines: Vec<String> = Vec::new();
    if verbose {
        lines.push(format!(
            "# {}",
            if r.title.is_empty() {
                "Untitled"
            } else {
                &r.title
            }
        ));
        lines.push(String::new());
        lines.push(format!("URL: {}", r.url));
        if r.final_url != r.url {
            lines.push(format!("Final URL: {}", r.final_url));
        }
        for (label, v) in [
            ("Author", &r.author),
            ("Published", &r.published),
            ("Site", &r.site),
            ("Language", &r.language),
        ] {
            if !v.is_empty() {
                lines.push(format!("{label}: {v}"));
            }
        }
        lines.push(format!("Word count: {}", r.word_count));
        lines.push(String::new());
        lines.push("---".into());
        lines.push(String::new());
    }
    lines.push(r.content.clone());
    lines.join("\n")
}

/// unipi `formatBatchResult`.
pub(crate) fn format_batch(items: &[BatchItem]) -> String {
    let total = items.len();
    let ok = items
        .iter()
        .filter(|i| matches!(i, BatchItem::Done(_)))
        .count();
    let mut lines = vec![
        "# Batch Read Results".to_owned(),
        String::new(),
        format!("Total: {total} · Succeeded: {ok} · Failed: {}", total - ok),
        String::new(),
    ];
    for (i, item) in items.iter().enumerate() {
        match item {
            BatchItem::Done(r) => {
                lines.push(format!("## [{}/{total}] ✓", i + 1));
                lines.push(format!("Title: {}", r.title));
                lines.push(format!("URL: {}", r.url));
                lines.push(format!("Words: {}", r.word_count));
                let preview: String = r.content.chars().take(500).collect();
                lines.push(String::new());
                let more = if r.content.chars().count() > 500 {
                    "..."
                } else {
                    ""
                };
                lines.push(format!("{preview}{more}"));
            }
            BatchItem::Error(e) => {
                lines.push(format!("## [{}/{total}] ✗", i + 1));
                lines.push(format!("URL: {}", e.url.as_deref().unwrap_or("unknown")));
                lines.push(format!("Error: {}", e.error));
            }
        }
        lines.push(String::new());
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_like_unipi() {
        assert_eq!(truncate("short", 100), "short");
        let words = "word ".repeat(100);
        let t = truncate(&words, 100);
        assert!(t.ends_with("... [truncated]"));
        assert!(t.chars().count() <= 100);
        assert!(!t.trim_end_matches(TRUNCATION_MARKER).ends_with(' '));
        let solid = "x".repeat(200);
        let t = truncate(&solid, 100);
        assert_eq!(t.chars().count(), 100, "no word boundary: hard cut");
    }

    #[test]
    fn strips_markdown() {
        let md = "# Title\n\n**bold** and *it* and __u__ [link](http://x) ![img](http://i)\n\n```\ncode\n```\n`inline`\n\n---\n> quote\n- item\n1. first\n\n\n\nend";
        assert_eq!(
            strip_markdown(md),
            "Title\n\nbold and it and u link \n\ninline\n\nquote\nitem\nfirst\n\nend"
        );
        assert_eq!(remove_images("a ![x](y) b"), "a  b");
    }

    #[test]
    fn single_and_batch_formats() {
        let r = create_result(
            "https://a",
            "https://b",
            "hello world".into(),
            &Meta {
                title: "T".into(),
                author: "A".into(),
                ..Meta::default()
            },
            "markdown",
            "text/html",
        );
        assert_eq!(r.word_count, 2);
        assert_eq!(
            format_single(&r, true),
            "# T\n\nURL: https://a\nFinal URL: https://b\nAuthor: A\nWord count: 2\n\n---\n\nhello world"
        );
        assert_eq!(format_single(&r, false), "hello world");
        let batch = format_batch(&[
            BatchItem::Done(r.clone()),
            BatchItem::Error(
                FetchError::new("http_error", "waiting", "HTTP error: 404 Not Found", false)
                    .at("https://c", "https://c"),
            ),
        ]);
        assert!(batch.starts_with(
            "# Batch Read Results\n\nTotal: 2 · Succeeded: 1 · Failed: 1\n\n## [1/2] ✓\nTitle: T"
        ));
        assert!(batch.contains("## [2/2] ✗\nURL: https://c\nError: HTTP error: 404 Not Found"));
        assert!(format_content(&r, "json", 50_000).contains("\"finalUrl\": \"https://b\""));
        assert_eq!(format_content(&r, "html", 50_000), "hello world");
    }
}
