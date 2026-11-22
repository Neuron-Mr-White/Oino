//! Image input handling (unipi `src/image/source.ts`): a local file path, a `data:` URL or a
//! raw base64 string, normalised to `{data, mimeType}`.

use base64::Engine as _;

use crate::hostfs;

/// Media types accepted by the vision APIs.
pub const SUPPORTED_MIME_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

const EXTENSION_MIME: [(&str, &str); 6] = [
    (".png", "image/png"),
    (".jpg", "image/jpeg"),
    (".jpeg", "image/jpeg"),
    (".jfif", "image/jpeg"),
    (".gif", "image/gif"),
    (".webp", "image/webp"),
];

/// Where an image came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// A file on disk.
    File,
    /// A `data:` URL.
    DataUrl,
    /// Raw base64.
    Base64,
}

/// A resolved image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedImage {
    /// Base64 (no `data:` prefix).
    pub data: String,
    /// IANA media type.
    pub mime_type: String,
    /// Origin.
    pub source: Source,
    /// Absolute host path when loaded from disk.
    pub path: Option<String>,
}

/// What reading a host path gave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Probe {
    /// File bytes.
    Bytes(Vec<u8>),
    /// Nothing there.
    Missing,
    /// Exists but is not a file.
    NotFile,
    /// Outside every granted folder.
    Unreachable,
}

/// Media type from a file extension.
#[must_use]
pub fn mime_from_extension(path: &str) -> Option<&'static str> {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let ext = name
        .rfind('.')
        .filter(|i| *i > 0)
        .map(|i| name[i..].to_ascii_lowercase())?;
    EXTENSION_MIME
        .iter()
        .find(|(e, _)| *e == ext)
        .map(|(_, m)| *m)
}

/// Media type from magic numbers (authoritative over the extension).
#[must_use]
pub fn detect_mime(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        Some("image/png")
    } else if b.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if b.starts_with(b"GIF8") && b.len() >= 6 {
        Some("image/gif")
    } else if b.len() >= 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// The type is accepted by the vision APIs.
#[must_use]
pub fn is_supported(mime: &str) -> bool {
    SUPPORTED_MIME_TYPES.contains(&mime)
}

/// `data:<mime>;base64,<data>` → `(data, mime)`; `None` for anything else.
#[must_use]
pub fn parse_data_url(input: &str) -> Option<(String, String)> {
    let rest = input.strip_prefix("data:")?;
    let (meta, payload) = rest.split_once(',')?;
    let mut parts = meta.split(';');
    let mime = parts.next()?.trim();
    if mime.is_empty() || !parts.any(|p| p.contains("base64")) {
        return None;
    }
    Some((payload.trim().to_owned(), mime.to_ascii_lowercase()))
}

/// Plausibly raw base64 (and long enough to be an image).
#[must_use]
pub fn looks_like_base64(input: &str) -> bool {
    let compact: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.len() < 64 {
        return false;
    }
    let body = compact.trim_end_matches('=');
    compact.len() - body.len() <= 2
        && body
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/')
}

fn unsupported(mime: &str) -> String {
    format!(
        "Unsupported image type \"{mime}\". Supported types: {}.",
        SUPPORTED_MIME_TYPES.join(", ")
    )
}

fn truncate(v: &str) -> String {
    if v.chars().count() <= 60 {
        v.to_owned()
    } else {
        format!("{}…", v.chars().take(60).collect::<String>())
    }
}

/// Resolves an `image` argument (order: data URL, file path, raw base64).
///
/// # Errors
///
/// An actionable message for any unusable input.
pub fn load_image(
    input: &str,
    cwd: &str,
    home: Option<&str>,
    read: &dyn Fn(&str) -> Probe,
) -> Result<LoadedImage, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("No image provided. Pass a file path, a data: URL, or base64 data.".into());
    }
    if trimmed.starts_with("data:") {
        let (data, mime) = parse_data_url(trimmed)
            .ok_or("Malformed data: URL — expected data:<mime>;base64,<data>.")?;
        if !is_supported(&mime) {
            return Err(unsupported(&mime));
        }
        return Ok(LoadedImage {
            data,
            mime_type: mime,
            source: Source::DataUrl,
            path: None,
        });
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Err("Remote image URLs are not supported. Download the image first, then pass the local file path.".into());
    }
    let looks_like_path = trimmed.starts_with(['/', '~', '.'])
        || trimmed.contains(['/', '\\'])
        || mime_from_extension(trimmed).is_some();
    if looks_like_path {
        let resolved = hostfs::absolute(trimmed, cwd, home);
        let bytes = match read(&resolved) {
            Probe::Missing => return Err(format!("Image file not found: {resolved}")),
            Probe::NotFile => return Err(format!("Not a file: {resolved}")),
            Probe::Unreachable => {
                return Err(format!(
                    "Image file {resolved} is outside the folders the utility can read (the project, ~/Downloads, ~/Desktop, ~/Pictures, ~/Documents, /tmp). Copy it into the project first."
                ));
            }
            Probe::Bytes(b) => b,
        };
        if bytes.is_empty() {
            return Err(format!("Image file is empty: {resolved}"));
        }
        let mime = detect_mime(&bytes)
            .or_else(|| mime_from_extension(&resolved))
            .ok_or_else(|| {
                format!(
                    "Could not determine the image type of {resolved}. Supported types: {}.",
                    SUPPORTED_MIME_TYPES.join(", ")
                )
            })?;
        if !is_supported(mime) {
            return Err(unsupported(mime));
        }
        return Ok(LoadedImage {
            data: base64::engine::general_purpose::STANDARD.encode(&bytes),
            mime_type: mime.to_owned(),
            source: Source::File,
            path: Some(resolved),
        });
    }
    if looks_like_base64(trimmed) {
        let compact: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&compact)
            .unwrap_or_default();
        let mime = detect_mime(&decoded).ok_or(
            "Could not determine the image type of the supplied base64 data. Prefer a file path, or use a data: URL that declares the media type.",
        )?;
        return Ok(LoadedImage {
            data: compact,
            mime_type: mime.to_owned(),
            source: Source::Base64,
            path: None,
        });
    }
    Err(format!(
        "Could not interpret \"{}\" as an image. Pass a local file path, a data: URL, or base64-encoded image data.",
        truncate(trimmed)
    ))
}

/// Reads a host path through the preopens.
#[must_use]
pub fn probe_host(roots: &hostfs::Roots, host: &str) -> Probe {
    let Some(guest) = roots.to_guest(host) else {
        return Probe::Unreachable;
    };
    match std::fs::metadata(&guest) {
        Err(_) => Probe::Missing,
        Ok(m) if !m.is_file() => Probe::NotFile,
        Ok(_) => std::fs::read(&guest).map_or(Probe::Missing, Probe::Bytes),
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::assert_is_empty,
    clippy::format_collect,
    clippy::cloned_ref_to_slice_refs,
    clippy::duration_suboptimal_units,
    clippy::too_many_lines,
    clippy::needless_pass_by_value,
    reason = "tests assert by unwrapping"
)]
pub(crate) mod tests {
    use std::collections::HashMap;

    use super::*;

    pub(crate) const PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

    pub(crate) fn png() -> Vec<u8> {
        base64::engine::general_purpose::STANDARD
            .decode(PNG_BASE64)
            .unwrap_or_default()
    }

    fn padded(head: &[u8]) -> Vec<u8> {
        let mut v = head.to_vec();
        v.extend(std::iter::repeat_n(0x20, 128));
        v
    }

    fn fs(files: &[(&str, Vec<u8>)], dirs: &[&str]) -> impl Fn(&str) -> Probe + use<> {
        let files: HashMap<String, Vec<u8>> = files
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect();
        let dirs: Vec<String> = dirs.iter().map(|d| (*d).to_owned()).collect();
        move |p: &str| {
            if dirs.iter().any(|d| d == p) {
                Probe::NotFile
            } else if p.starts_with("/outside") {
                Probe::Unreachable
            } else {
                files.get(p).cloned().map_or(Probe::Missing, Probe::Bytes)
            }
        }
    }

    #[test]
    fn extensions() {
        for (f, m) in [
            ("a.png", "image/png"),
            ("a.jpg", "image/jpeg"),
            ("a.jpeg", "image/jpeg"),
            ("a.gif", "image/gif"),
            ("a.webp", "image/webp"),
            ("A.PNG", "image/png"),
            ("x.JFIF", "image/jpeg"),
        ] {
            assert_eq!(mime_from_extension(f), Some(m), "{f}");
        }
        assert_eq!(mime_from_extension("a.txt"), None);
        assert_eq!(mime_from_extension("noext"), None);
        assert_eq!(mime_from_extension(".png"), None);
    }

    #[test]
    fn magic_numbers() {
        assert_eq!(detect_mime(&png()), Some("image/png"));
        assert_eq!(
            detect_mime(&padded(&[0xff, 0xd8, 0xff, 0xe0])),
            Some("image/jpeg")
        );
        assert_eq!(detect_mime(&padded(b"GIF89a")), Some("image/gif"));
        let mut webp = b"RIFF\0\0\0\0WEBP".to_vec();
        webp.extend([0x20; 16]);
        assert_eq!(detect_mime(&webp), Some("image/webp"));
        assert_eq!(detect_mime(b"plain text data here"), None);
        assert_eq!(detect_mime(&[]), None);
        assert!(
            is_supported("image/png")
                && !is_supported("image/bmp")
                && !is_supported("image/svg+xml")
        );
    }

    #[test]
    fn data_urls_and_base64_detection() {
        assert_eq!(
            parse_data_url("data:image/PNG;base64, abc "),
            Some(("abc".into(), "image/png".into()))
        );
        assert_eq!(parse_data_url("data:image/png,notbase64"), None);
        assert_eq!(parse_data_url("image/png;base64,abc"), None);
        assert!(looks_like_base64(PNG_BASE64));
        assert!(!looks_like_base64("abc"));
        assert!(!looks_like_base64(
            "not base64 at all; has punctuation!!".repeat(3).as_str()
        ));
    }

    #[test]
    fn files() {
        let read = fs(
            &[
                ("/w/shot.png", png()),
                ("/w/mislabeled.jpg", png()),
                ("/w/empty.png", Vec::new()),
                ("/w/image.bmp", padded(b"BM")),
            ],
            &["/w"],
        );
        let l = load_image("/w/shot.png", "/", None, &read).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            (
                l.mime_type.as_str(),
                l.source,
                l.path.as_deref(),
                l.data.as_str()
            ),
            ("image/png", Source::File, Some("/w/shot.png"), PNG_BASE64)
        );
        assert_eq!(
            load_image("./shot.png", "/w", None, &read)
                .ok()
                .and_then(|l| l.path)
                .as_deref(),
            Some("/w/shot.png")
        );
        assert_eq!(
            load_image("/w/mislabeled.jpg", "/", None, &read)
                .map(|l| l.mime_type)
                .ok()
                .as_deref(),
            Some("image/png")
        );
        assert!(
            load_image("/w/nope.png", "/", None, &read)
                .is_err_and(|e| e.contains("Image file not found"))
        );
        assert!(load_image("/w", "/", None, &read).is_err_and(|e| e.contains("Not a file")));
        assert!(load_image("/w/empty.png", "/", None, &read).is_err_and(|e| e.contains("empty")));
        assert!(
            load_image("/w/image.bmp", "/", None, &read)
                .is_err_and(|e| e.contains("Could not determine") || e.contains("Unsupported"))
        );
        assert!(
            load_image("/outside/a.png", "/", None, &read)
                .is_err_and(|e| e.contains("outside the folders"))
        );
        assert!(
            load_image("./missing-screenshot.png", "/w", None, &read)
                .is_err_and(|e| e.contains("not found"))
        );
        assert_eq!(
            load_image("~/shot.png", "/", Some("/w"), &read)
                .ok()
                .and_then(|l| l.path)
                .as_deref(),
            Some("/w/shot.png")
        );
    }

    #[test]
    fn data_urls_base64_and_rejections() {
        let read = fs(&[], &[]);
        let l = load_image(
            &format!("data:image/png;base64,{PNG_BASE64}"),
            "/",
            None,
            &read,
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            (l.source, l.mime_type.as_str(), l.path),
            (Source::DataUrl, "image/png", None)
        );
        assert!(
            load_image("data:image/png,notbase64", "/", None, &read)
                .is_err_and(|e| e.contains("Malformed data"))
        );
        assert!(
            load_image("data:image/bmp;base64,Qk0AAAAAAAAAAA==", "/", None, &read)
                .is_err_and(|e| e.contains("Unsupported image type"))
        );
        let b = load_image(PNG_BASE64, "/", None, &read).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            (b.source, b.mime_type.as_str()),
            (Source::Base64, "image/png")
        );
        assert!(
            load_image("https://example.com/photo.png", "/", None, &read)
                .is_err_and(|e| e.contains("Remote image URLs are not supported"))
        );
        assert!(
            load_image("   ", "/", None, &read).is_err_and(|e| e.contains("No image provided"))
        );
        assert!(
            load_image("please look at my screenshot", "/", None, &read)
                .is_err_and(|e| e.contains("Could not interpret"))
        );
    }
}
