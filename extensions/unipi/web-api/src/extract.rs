//! Content extraction (unipi `src/engine/{extract,dom}.ts`): `linkedom` + `defuddle` become
//! `dom_query` + `dom_smoothie` (Readability) + `htmd` (turndown-style HTML → Markdown), all
//! in the guest.

use dom_query::Document;
use dom_smoothie::{Config, Readability};

use crate::format::Meta;

/// Elements dropped before converting a fallback region to markdown.
const SKIP: &[&str] = &[
    "script", "style", "meta", "head", "noscript", "template", "iframe",
];
/// Image-like elements removed with `removeImages`.
const IMAGE_SELECTOR: &str = "img, picture, svg, figure img, video, source";

/// `<meta http-equiv="refresh" content="0;url=…">` target (unipi `findMetaRefresh`).
pub(crate) fn meta_refresh(doc: &Document) -> Option<String> {
    let content = doc
        .select(r#"meta[http-equiv="refresh" i]"#)
        .first()
        .attr("content")?
        .to_string();
    let lower = content.to_ascii_lowercase();
    let pos = lower.find("url")?;
    let rest = content[pos + 3..].trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let target: String = rest
        .trim_start_matches(['\'', '"'])
        .chars()
        .take_while(|c| !matches!(c, '\'' | '"') && !c.is_whitespace())
        .collect();
    (!target.is_empty()).then_some(target)
}

fn meta_attr(doc: &Document, selector: &str) -> String {
    doc.select(selector)
        .first()
        .attr("content")
        .map(|s| s.trim().to_owned())
        .unwrap_or_default()
}

/// unipi `extractMetadata` DOM fallbacks.
pub(crate) fn dom_metadata(doc: &Document, mut meta: Meta) -> Meta {
    if meta.title.is_empty() {
        doc.select("title")
            .first()
            .text()
            .trim()
            .clone_into(&mut meta.title);
    }
    if meta.title.is_empty() {
        meta.title = meta_attr(doc, r#"meta[property="og:title"]"#);
    }
    if meta.author.is_empty() {
        meta.author = meta_attr(doc, r#"meta[name="author"]"#);
    }
    if meta.site.is_empty() {
        meta.site = meta_attr(doc, r#"meta[property="og:site_name"]"#);
    }
    if meta.language.is_empty() {
        meta.language = doc
            .select("html")
            .first()
            .attr("lang")
            .map(|s| s.to_string())
            .unwrap_or_default();
    }
    if meta.published.is_empty() {
        meta.published = meta_attr(doc, r#"meta[property="article:published_time"]"#);
    }
    meta
}

fn clean_markdown(md: &str) -> String {
    let mut out = String::with_capacity(md.len());
    let mut blank = 0;
    for line in md.lines() {
        if line.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out.trim().to_owned()
}

/// HTML fragment → Markdown (turndown rules; scripts, styles and frames skipped).
pub(crate) fn to_markdown(html: &str) -> String {
    let converter = htmd::HtmlToMarkdown::builder()
        .skip_tags(SKIP.to_vec())
        .build();
    clean_markdown(&converter.convert(html).unwrap_or_default())
}

/// unipi `fallbackExtraction`: `article, main, [role=main], .content, #content` → body → text.
pub(crate) fn fallback(doc: &Document) -> String {
    for sel in ["article, main, [role='main'], .content, #content", "body"] {
        let s = doc.select(sel).first();
        if s.exists() {
            let md = to_markdown(&s.html());
            if !md.trim().is_empty() {
                return md;
            }
        }
    }
    doc.root()
        .text()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// An extracted page.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Extracted {
    /// Markdown content.
    pub content: String,
    /// Metadata.
    pub meta: Meta,
    /// Readability succeeded (else the DOM fallback ran).
    pub readable: bool,
}

/// unipi defuddle step: main content as markdown plus metadata. `include_replies == true`
/// keeps the whole main region (comments included) instead of the Readability article.
pub(crate) fn extract(
    html: &str,
    url: &str,
    remove_images: bool,
    include_replies: Option<bool>,
) -> Extracted {
    let prepared = if remove_images {
        let doc = Document::from(html);
        doc.select(IMAGE_SELECTOR).remove();
        doc.html().to_string()
    } else {
        html.to_owned()
    };
    let doc = Document::from(prepared.as_str());
    let base = Meta::default();
    if include_replies == Some(true) {
        let meta = dom_metadata(&doc, base);
        return Extracted {
            content: fallback(&doc),
            meta,
            readable: false,
        };
    }
    let article = Readability::with_document(
        Document::from(prepared.as_str()),
        Some(url),
        Some(Config::default()),
    )
    .ok()
    .and_then(|mut r| r.parse().ok())
    .filter(|a| !a.text_content.trim().is_empty());
    // Short pages: Readability may settle on a stray block (below its 500-char threshold);
    // keep the larger of its article and the DOM fallback.
    let article = article.filter(|a| {
        a.text_content.chars().count() >= 500
            || to_markdown(&a.content).len() >= fallback(&doc).len()
    });
    match article {
        Some(a) => {
            let meta = Meta {
                title: a.title.clone(),
                author: a.byline.clone().unwrap_or_default(),
                published: a.published_time.clone().unwrap_or_default(),
                site: a.site_name.clone().unwrap_or_default(),
                language: a.lang.clone().unwrap_or_default(),
            };
            Extracted {
                content: to_markdown(&a.content),
                meta: dom_metadata(&doc, meta),
                readable: true,
            }
        }
        None => Extracted {
            content: fallback(&doc),
            meta: dom_metadata(&doc, base),
            readable: false,
        },
    }
}

/// unipi `detectContentType`: MIME type and whether it is binary.
pub(crate) fn content_type(header: Option<&str>) -> (String, bool) {
    let mime = header
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let binary = [
        "application/octet-stream",
        "application/pdf",
        "application/zip",
        "application/x-",
        "image/",
        "video/",
        "audio/",
        "font/",
    ]
    .iter()
    .any(|p| mime.starts_with(p));
    (mime, binary)
}

/// Resolves `target` against `base` (absolute, protocol-relative, root-relative, relative).
pub(crate) fn resolve_url(base: &str, target: &str) -> String {
    if target.starts_with("http://") || target.starts_with("https://") {
        return target.to_owned();
    }
    let scheme_end = base.find("://").map_or(0, |i| i + 3);
    let scheme = &base[..scheme_end.saturating_sub(3)];
    if let Some(rest) = target.strip_prefix("//") {
        return format!("{scheme}://{rest}");
    }
    let host_end = base[scheme_end..]
        .find('/')
        .map_or(base.len(), |i| scheme_end + i);
    let origin = &base[..host_end];
    if target.starts_with('/') {
        return format!("{origin}{target}");
    }
    let path = &base[host_end..];
    let path = path.split(['?', '#']).next().unwrap_or("");
    let dir = path.rfind('/').map_or("/", |i| &path[..=i]);
    format!("{origin}{dir}{target}")
}

/// unipi `validateUrl`: `Err((code, message))`.
pub(crate) fn validate_url(url: &str) -> Result<(), (&'static str, String)> {
    let Some((scheme, rest)) = url.split_once(':') else {
        return Err(("invalid_url", format!("Invalid URL format: {url}")));
    };
    if scheme.is_empty()
        || !scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        || rest.is_empty()
    {
        return Err(("invalid_url", format!("Invalid URL format: {url}")));
    }
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err((
            "unsupported_protocol",
            format!("Unsupported protocol: {scheme}:. Only http and https are supported."),
        ));
    }
    let host = rest
        .trim_start_matches('/')
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("");
    if host.is_empty() || !rest.starts_with("//") {
        return Err(("invalid_url", format!("Invalid URL format: {url}")));
    }
    Ok(())
}

/// Reason phrase for common statuses (the host reports codes only).
pub(crate) fn status_text(status: u16) -> &'static str {
    match status {
        400 => "Bad Request",
        401 => "Unauthorized",
        402 => "Payment Required",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        410 => "Gone",
        418 => "I'm a teapot",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARTICLE: &str = r#"<!doctype html><html lang="en"><head><title>Doc Title</title>
<meta name="author" content="Jane Doe"><meta property="og:site_name" content="Example Site">
</head><body><nav>Home | About | Contact</nav>
<article><h1>Big Heading</h1>
<p>This is the first paragraph of a reasonably long article about Rust and WebAssembly, written so that the readability scorer finds enough text to treat it as the main content of the page.</p>
<p>The second paragraph adds more words, with a <a href="https://example.com/x">link</a> and <strong>bold text</strong>, so the extracted markdown keeps inline formatting intact for the reader.</p>
<p>A third paragraph, because readability engines like to see several paragraphs of prose before they commit to a candidate container as the article body.</p>
<img src="https://example.com/pic.png" alt="A picture">
</article><footer>Copyright footer text</footer></body></html>"#;

    #[test]
    fn readability_to_markdown() {
        let e = extract(ARTICLE, "https://example.com/post", false, None);
        assert!(e.readable);
        assert!(e.content.contains("first paragraph"), "{}", e.content);
        assert!(
            e.content.contains("[link](https://example.com/x)"),
            "{}",
            e.content
        );
        assert!(e.content.contains("**bold text**"), "{}", e.content);
        assert!(!e.content.contains("Copyright footer"), "{}", e.content);
        assert_eq!(e.meta.author, "Jane Doe");
        assert_eq!(e.meta.site, "Example Site");
        assert_eq!(e.meta.language, "en");
        assert!(!e.meta.title.is_empty());
    }

    #[test]
    fn remove_images_option() {
        let with = extract(ARTICLE, "https://example.com/post", false, None);
        let without = extract(ARTICLE, "https://example.com/post", true, None);
        assert!(with.content.contains("pic.png"), "{}", with.content);
        assert!(!without.content.contains("pic.png"));
    }

    #[test]
    fn fallback_and_replies() {
        let tiny = "<html><head><title>T</title></head><body><main><p>Short text.</p></main><div class='comments'>c1</div></body></html>";
        let e = extract(tiny, "https://x.test/", false, None);
        assert!(e.content.contains("Short text."));
        assert_eq!(e.meta.title, "T");
        let r = extract(tiny, "https://x.test/", false, Some(true));
        assert!(!r.readable);
        assert!(r.content.contains("Short text."));
        let body_only =
            Document::from("<html><body><p>Only body</p><script>x()</script></body></html>");
        assert_eq!(fallback(&body_only), "Only body");
    }

    #[test]
    fn meta_refresh_targets() {
        let d = Document::from(
            r#"<html><head><meta http-equiv="refresh" content="0; URL='https://n.test/next'"></head></html>"#,
        );
        assert_eq!(meta_refresh(&d).as_deref(), Some("https://n.test/next"));
        let d = Document::from(
            r#"<html><head><meta http-equiv="Refresh" content="5;url=/other"></head></html>"#,
        );
        assert_eq!(meta_refresh(&d).as_deref(), Some("/other"));
        assert_eq!(meta_refresh(&Document::from("<html></html>")), None);
        assert_eq!(
            resolve_url("https://a.test/dir/page?q", "/root"),
            "https://a.test/root"
        );
        assert_eq!(
            resolve_url("https://a.test/dir/page", "rel"),
            "https://a.test/dir/rel"
        );
        assert_eq!(resolve_url("https://a.test", "rel"), "https://a.test/rel");
        assert_eq!(
            resolve_url("https://a.test/x", "//b.test/y"),
            "https://b.test/y"
        );
    }

    #[test]
    fn content_types_and_urls() {
        assert_eq!(
            content_type(Some("text/html; charset=utf-8")),
            ("text/html".into(), false)
        );
        assert_eq!(
            content_type(Some("application/pdf")),
            ("application/pdf".into(), true)
        );
        assert!(content_type(Some("image/png")).1);
        assert_eq!(content_type(None), (String::new(), false));
        assert!(validate_url("https://example.com/a").is_ok());
        assert_eq!(
            validate_url("ftp://x.test").err().map(|e| e.0),
            Some("unsupported_protocol")
        );
        assert_eq!(
            validate_url("ftp://x.test").err().map(|e| e.1),
            Some("Unsupported protocol: ftp:. Only http and https are supported.".into())
        );
        assert_eq!(
            validate_url("not a url").err().map(|e| e.0),
            Some("invalid_url")
        );
        assert_eq!(
            validate_url("https://").err().map(|e| e.0),
            Some("invalid_url")
        );
        assert_eq!(status_text(404), "Not Found");
    }
}
