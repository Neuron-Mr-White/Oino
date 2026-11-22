//! Attachment detection and tokens (unipi `src/attach/detect.ts`).
//!
//! A pasted/dropped file path becomes a token: images → `[Image #N]` (sent as image content on
//! submit), documents → `[File #N]` (expanded to `[File #N: <path>]`). File access is passed in
//! as closures so the logic is host-independent (the caller decides how paths are reached).

/// Image extensions and MIME types.
pub const IMAGE_MIME: &[(&str, &str)] = &[
    (".png", "image/png"),
    (".jpg", "image/jpeg"),
    (".jpeg", "image/jpeg"),
    (".gif", "image/gif"),
    (".webp", "image/webp"),
];

const DOC_EXT: &[&str] = &[
    ".pdf", ".txt", ".md", ".csv", ".tsv", ".json", ".yaml", ".yml", ".xml", ".html", ".log",
    ".docx", ".xlsx", ".pptx", ".odt", ".ods", ".rtf", ".zip", ".svg", ".mp3", ".wav", ".mp4",
    ".mov",
];

/// Attachment kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `[Image #N]`.
    Image,
    /// `[File #N]`.
    File,
}

/// One attachment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    /// Token number.
    pub id: u32,
    /// Kind.
    pub kind: Kind,
    /// Absolute path.
    pub path: String,
    /// File name.
    pub name: String,
    /// Size.
    pub bytes: u64,
    /// MIME type (images).
    pub mime_type: Option<&'static str>,
}

/// Token text.
#[must_use]
pub fn token_for(kind: Kind, id: u32) -> String {
    match kind {
        Kind::Image => format!("[Image #{id}]"),
        Kind::File => format!("[File #{id}]"),
    }
}

fn extension(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rfind('.') {
        Some(i) if i > 0 => name[i..].to_ascii_lowercase(),
        _ => String::new(),
    }
}

/// MIME type of an image path.
#[must_use]
pub fn image_mime(path: &str) -> Option<&'static str> {
    let ext = extension(path);
    IMAGE_MIME.iter().find(|(e, _)| *e == ext).map(|(_, m)| *m)
}

/// Attachment kind by extension (`None` = not attachable).
#[must_use]
pub fn kind_of(path: &str) -> Option<Kind> {
    if image_mime(path).is_some() {
        Some(Kind::Image)
    } else if DOC_EXT.contains(&extension(path).as_str()) {
        Some(Kind::File)
    } else {
        None
    }
}

/// A path found in text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// Exact text to replace.
    pub raw: String,
    /// Normalised path.
    pub path: String,
    /// Kind.
    pub kind: Kind,
}

/// One regex candidate: `(raw, captured path)`.
fn candidates(text: &str) -> Vec<(String, String)> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if let Some((len, cap)) = match_at(&chars, i) {
            out.push((chars[i..i + len].iter().collect(), cap));
            i += len;
        } else {
            i += 1;
        }
    }
    out
}

/// unipi `CANDIDATE` alternatives at `i`: quoted `'…'`/`"…"` absolute or `~/` paths,
/// `file:///…` URIs, bare `/…` or `~…` paths with `\ ` escapes.
fn match_at(c: &[char], i: usize) -> Option<(usize, String)> {
    for q in ['\'', '"'] {
        if c[i] != q {
            continue;
        }
        // Minimum body: `/x` or `~/x`.
        let min = match (c.get(i + 1), c.get(i + 2)) {
            (Some('/'), _) => 2,
            (Some('~'), Some('/')) => 3,
            _ => continue,
        };
        if let Some(end) = c[i + 1..]
            .iter()
            .position(|ch| *ch == q || *ch == '\n')
            .map(|p| i + 1 + p)
            && c[end] == q
            && end - (i + 1) >= min
        {
            return Some((end + 1 - i, c[i + 1..end].iter().collect()));
        }
    }
    let rest: String = c[i..].iter().take(8).collect();
    if rest.starts_with("file:///") {
        let len = 8 + c[i + 8..]
            .iter()
            .take_while(|ch| !ch.is_whitespace() && **ch != '\'' && **ch != '"')
            .count();
        if len > 8 {
            return Some((len, c[i..i + len].iter().collect()));
        }
    }
    if c[i] == '~' || c[i] == '/' {
        let mut j = i + 1;
        while j < c.len() {
            let ch = c[j];
            if ch == '\\' && c.get(j + 1) == Some(&' ') {
                j += 2;
            } else if !ch.is_whitespace() && ch != '\'' && ch != '"' && ch != '\\' {
                j += 1;
            } else {
                break;
            }
        }
        if j > i + 1 {
            return Some((j - i, c[i..j].iter().collect()));
        }
    }
    None
}

fn percent_decode(s: &str) -> Option<String> {
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

/// unipi `normalize`: `file://` decoding, `\ ` unescaping, `~/` → `home`.
#[must_use]
pub fn normalize(raw: &str, home: Option<&str>) -> String {
    let mut p = raw.to_owned();
    if let Some(rest) = p.strip_prefix("file://") {
        p = percent_decode(rest).unwrap_or_else(|| rest.to_owned());
    }
    p = p.replace("\\ ", " ");
    if let (Some(rest), Some(h)) = (p.strip_prefix("~/"), home) {
        p = format!("{}/{rest}", h.trim_end_matches('/'));
    }
    p
}

/// Existing attachable files mentioned in `text` (first occurrence each).
pub fn find_paths(text: &str, home: Option<&str>, exists: &dyn Fn(&str) -> bool) -> Vec<Found> {
    let mut out: Vec<Found> = Vec::new();
    for (raw, cap) in candidates(text) {
        let path = normalize(&cap, home);
        let Some(kind) = kind_of(&path) else { continue };
        if out.iter().any(|f| f.path == path) || !exists(&path) {
            continue;
        }
        out.push(Found { raw, path, kind });
    }
    out
}

/// Replaces found paths with tokens; returns the new text and the new attachments (existing
/// attachments with the same path are reused).
pub fn tokenize(
    text: &str,
    existing: &[Attachment],
    home: Option<&str>,
    exists: &dyn Fn(&str) -> bool,
    size: &dyn Fn(&str) -> u64,
) -> (String, Vec<Attachment>) {
    let mut next = text.to_owned();
    let mut id = existing.iter().map(|a| a.id).max().unwrap_or(0);
    let mut added: Vec<Attachment> = Vec::new();
    for f in find_paths(text, home, exists) {
        let reuse = existing
            .iter()
            .chain(added.iter())
            .find(|a| a.path == f.path)
            .cloned();
        let a = if let Some(a) = reuse {
            a
        } else {
            id += 1;
            let a = Attachment {
                id,
                kind: f.kind,
                name: f.path.rsplit('/').next().unwrap_or(&f.path).to_owned(),
                bytes: size(&f.path),
                mime_type: if f.kind == Kind::Image {
                    image_mime(&f.path)
                } else {
                    None
                },
                path: f.path.clone(),
            };
            added.push(a.clone());
            a
        };
        next = next.replace(&f.raw, &token_for(a.kind, a.id));
    }
    (next, added)
}

/// Attachments whose token is still in `text`.
#[must_use]
pub fn still_referenced<'a>(text: &str, attachments: &'a [Attachment]) -> Vec<&'a Attachment> {
    attachments
        .iter()
        .filter(|a| text.contains(&token_for(a.kind, a.id)))
        .collect()
}

/// An image content part.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImagePart {
    /// Base64 data.
    pub data: String,
    /// MIME type.
    pub mime_type: String,
}

/// Submit-time expansion: images become image parts (in token order), file tokens become
/// `[File #N: <path>]`; unreadable images become `[Image #N: <path> (unreadable)]`.
pub fn expand_for_submit(
    text: &str,
    attachments: &[Attachment],
    read: &dyn Fn(&str) -> Option<Vec<u8>>,
) -> (String, Vec<ImagePart>) {
    let mut used = still_referenced(text, attachments);
    used.sort_by_key(|a| text.find(&token_for(a.kind, a.id)).unwrap_or(usize::MAX));
    let mut out = text.to_owned();
    let mut images = Vec::new();
    for a in used {
        let token = token_for(a.kind, a.id);
        match a.kind {
            Kind::File => out = out.replace(&token, &format!("[File #{}: {}]", a.id, a.path)),
            Kind::Image => match read(&a.path) {
                Some(bytes) => images.push(ImagePart {
                    data: base64(&bytes),
                    mime_type: a.mime_type.unwrap_or("image/png").to_owned(),
                }),
                None => {
                    out = out.replace(
                        &token,
                        &format!("[Image #{}: {} (unreadable)]", a.id, a.path),
                    );
                }
            },
        }
    }
    (out, images)
}

/// Standard base64 with padding.
#[must_use]
pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (k, shift) in [18u32, 12, 6, 0].into_iter().enumerate() {
            if k <= chunk.len() {
                out.push(char::from(T[((n >> shift) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exists(p: &str) -> bool {
        [
            "/tmp/shot.png",
            "/home/u/My Docs/spec.pdf",
            "/home/u/a b.png",
            "/data/report.md",
        ]
        .contains(&p)
    }

    #[test]
    fn kinds() {
        assert_eq!(kind_of("/x/A.PNG"), Some(Kind::Image));
        assert_eq!(kind_of("/x/a.md"), Some(Kind::File));
        assert_eq!(kind_of("/x/a.rs"), None);
        assert_eq!(kind_of("/x/.png"), None, "dotfile has no extension");
        assert_eq!(image_mime("/a.jpeg"), Some("image/jpeg"));
    }

    #[test]
    fn finds_quoted_uri_escaped_and_tilde_paths() {
        let text = "see /tmp/shot.png and '~/My Docs/spec.pdf' plus file:///home/u/a%20b.png /data/report.md /tmp/shot.png /nope.png";
        let found = find_paths(text, Some("/home/u"), &exists);
        let paths: Vec<&str> = found.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "/tmp/shot.png",
                "/home/u/My Docs/spec.pdf",
                "/home/u/a b.png",
                "/data/report.md"
            ]
        );
        assert_eq!(found[1].raw, "'~/My Docs/spec.pdf'");
        assert_eq!(found[2].raw, "file:///home/u/a%20b.png");
        let escaped = find_paths(r"drop /home/u/a\ b.png here", None, &exists);
        assert_eq!(escaped.len(), 1);
        assert_eq!(escaped[0].raw, r"/home/u/a\ b.png");
    }

    #[test]
    fn tokenizes_and_reuses_ids() {
        let existing = vec![Attachment {
            id: 3,
            kind: Kind::File,
            path: "/data/report.md".into(),
            name: "report.md".into(),
            bytes: 9,
            mime_type: None,
        }];
        let (text, added) = tokenize(
            "a /tmp/shot.png b /data/report.md",
            &existing,
            None,
            &exists,
            &|_| 42,
        );
        assert_eq!(text, "a [Image #4] b [File #3]");
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].mime_type, Some("image/png"));
        assert_eq!(added[0].bytes, 42);
        let all: Vec<Attachment> = existing.into_iter().chain(added).collect();
        assert_eq!(still_referenced("only [File #3]", &all).len(), 1);
        let (out, images) = expand_for_submit("[Image #4] then [File #3]", &all, &|p| {
            (p == "/tmp/shot.png").then(|| b"hi".to_vec())
        });
        assert_eq!(out, "[Image #4] then [File #3: /data/report.md]");
        assert_eq!(
            images,
            [ImagePart {
                data: "aGk=".into(),
                mime_type: "image/png".into()
            }]
        );
        let (out, images) = expand_for_submit("[Image #4]", &all, &|_| None);
        assert_eq!(out, "[Image #4: /tmp/shot.png (unreadable)]");
        assert_eq!(images, Vec::<ImagePart>::new());
    }

    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
