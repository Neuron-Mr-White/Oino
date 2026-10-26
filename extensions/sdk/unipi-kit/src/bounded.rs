//! Bounded model-visible output (unipi `bounded-output.ts`).
//!
//! Output above the ceiling keeps 75 % head and 25 % tail with an omission marker, and the full
//! text is preserved in a private artifact (mode 0600 under `/tmp/tool-results/`) whose *host*
//! path is shown so the agent can `read` it with offset/limit.

use std::path::Path;

/// Default model-visible ceiling (64 KiB).
pub const DEFAULT_MODEL_OUTPUT_BYTES: usize = 64 * 1024;
/// Largest output kept as an artifact (16 MiB).
pub const MAX_RAW_ARTIFACT_BYTES: usize = 16 * 1024 * 1024;

/// Result of [`bound_with`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundedOutput {
    /// Model-visible text.
    pub text: String,
    /// The text was cut.
    pub truncated: bool,
    /// Original UTF-8 size.
    pub original_bytes: usize,
    /// Visible UTF-8 size.
    pub visible_bytes: usize,
    /// Artifact path shown to the model.
    pub artifact_path: Option<String>,
}

/// Options.
#[derive(Clone, Debug)]
pub struct Options<'a> {
    /// Ceiling (minimum 1 024).
    pub max_bytes: usize,
    /// Artifact file prefix.
    pub artifact_prefix: &'a str,
}

impl Default for Options<'_> {
    fn default() -> Self {
        Self {
            max_bytes: DEFAULT_MODEL_OUTPUT_BYTES,
            artifact_prefix: "tool-result",
        }
    }
}

/// UTF-8 byte slice decoded like Node `Buffer#toString` with trailing replacement chars cut.
fn byte_slice(text: &str, start: usize, end: Option<usize>) -> String {
    let bytes = text.as_bytes();
    let end = end.unwrap_or(bytes.len()).min(bytes.len());
    let start = start.min(end);
    let s = String::from_utf8_lossy(&bytes[start..end]);
    s.trim_end_matches('\u{FFFD}').to_owned()
}

/// unipi `safePrefix`.
#[must_use]
pub fn safe_prefix(prefix: &str) -> String {
    let mut out = String::new();
    for c in prefix.chars() {
        let c = if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            c
        } else {
            '-'
        };
        if !(c == '-' && out.ends_with('-')) {
            out.push(c);
        }
    }
    let out: String = out.chars().take(48).collect();
    if out.is_empty() {
        "tool-result".into()
    } else {
        out
    }
}

/// Bounds `text`; `write_artifact(prefix, text)` stores the full text and returns the path to
/// show, or an error message for the marker.
pub fn bound_with(
    text: &str,
    options: &Options<'_>,
    write_artifact: impl FnOnce(&str, &str) -> Result<String, String>,
) -> BoundedOutput {
    let max_bytes = options.max_bytes.max(1024);
    let original_bytes = text.len();
    if original_bytes <= max_bytes {
        return BoundedOutput {
            text: text.to_owned(),
            truncated: false,
            original_bytes,
            visible_bytes: original_bytes,
            artifact_path: None,
        };
    }
    let (artifact_path, warning) = if original_bytes <= MAX_RAW_ARTIFACT_BYTES {
        match write_artifact(&safe_prefix(options.artifact_prefix), text) {
            Ok(p) => (Some(p), None),
            Err(e) => (None, Some(format!("Full-output artifact unavailable: {e}"))),
        }
    } else {
        (
            None,
            Some(format!(
                "Full output exceeded the {MAX_RAW_ARTIFACT_BYTES}-byte local artifact safety cap and was not retained."
            )),
        )
    };
    let mut marker = vec![
        String::new(),
        "--- output bounded by UniPi ---".to_owned(),
        artifact_path.as_ref().map_or_else(
            || warning.clone().unwrap_or_default(),
            |p| format!("Full output: {p}"),
        ),
        format!("Original size: {original_bytes} bytes; model-visible ceiling: {max_bytes} bytes."),
    ];
    if artifact_path.is_some() {
        marker.push(
            "Use the read tool with offset/limit to inspect only the needed region.".to_owned(),
        );
    }
    let marker = marker.join("\n");
    let omission_reserve = 80;
    let content_budget = max_bytes
        .saturating_sub(marker.len() + omission_reserve)
        .max(1);
    let head_bytes = ceil_three_quarters(content_budget);
    let tail_bytes = content_budget - head_bytes;
    let head = byte_slice(text, 0, Some(head_bytes));
    let tail = if tail_bytes > 0 {
        byte_slice(text, original_bytes - tail_bytes, None)
    } else {
        String::new()
    };
    let omission = format!(
        "\n… {} bytes omitted …\n",
        original_bytes.saturating_sub(content_budget)
    );
    let mut bounded = format!("{head}{omission}{tail}{marker}");
    if bounded.len() > max_bytes {
        bounded = byte_slice(&bounded, 0, Some(max_bytes));
    }
    BoundedOutput {
        visible_bytes: bounded.len(),
        text: bounded,
        truncated: true,
        original_bytes,
        artifact_path,
    }
}

/// `Math.ceil(n * 0.75)` in integers.
fn ceil_three_quarters(n: usize) -> usize {
    (n * 3).div_ceil(4)
}

/// Bounds `text`, writing the artifact to `/tmp/tool-results/<prefix>-<random>.txt` (0600) and
/// showing its host path.
#[must_use]
pub fn bound_model_output(text: &str, options: &Options<'_>) -> BoundedOutput {
    bound_with(text, options, |prefix, full| {
        let guest = crate::paths::tmp(&format!(
            "tool-results/{prefix}-{}.txt",
            crate::fmt::random_hex(16)
        ));
        crate::paths::ensure_parent(&guest).map_err(|e| e.to_string())?;
        oino_extension::wit::fs::atomic_write(
            &guest.to_string_lossy(),
            full.as_bytes(),
            Some(0o600),
        )
        .map_err(|e| oino_extension::Error::from(e).to_string())?;
        Ok(crate::paths::host_path(&guest)
            .unwrap_or_else(|| Path::new(&guest).to_path_buf())
            .to_string_lossy()
            .into_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_output_is_unchanged() {
        let out = bound_with("héllo", &Options::default(), |_, _| {
            unreachable!("no artifact")
        });
        assert!(!out.truncated);
        assert_eq!(out.text, "héllo");
        assert_eq!(out.original_bytes, 6);
        assert_eq!(out.artifact_path, None);
    }

    #[test]
    fn bounds_large_output_with_artifact() {
        let text: String = (0..5_000).fold(String::new(), |mut s, i| {
            use std::fmt::Write as _;
            let _ = writeln!(s, "line {i} é");
            s
        });
        let mut stored = String::new();
        let opts = Options {
            max_bytes: 4096,
            artifact_prefix: "bg task/1",
        };
        let out = bound_with(&text, &opts, |prefix, full| {
            assert_eq!(prefix, "bg-task-1");
            stored = full.to_owned();
            Ok("/host/tool-results/x.txt".into())
        });
        assert!(out.truncated);
        assert_eq!(stored, text);
        assert!(out.visible_bytes <= 4096);
        assert!(out.text.starts_with("line 0 é\n"));
        assert!(out.text.contains("bytes omitted …"));
        assert!(
            out.text
                .contains("--- output bounded by UniPi ---\nFull output: /host/tool-results/x.txt")
        );
        assert!(
            out.text.ends_with(
                "Use the read tool with offset/limit to inspect only the needed region."
            )
        );
        assert!(out.text.contains("line 4999 é"), "tail kept");
        assert_eq!(
            out.artifact_path.as_deref(),
            Some("/host/tool-results/x.txt")
        );
    }

    #[test]
    fn artifact_failure_is_reported_in_the_marker() {
        let text = "x".repeat(10_000);
        let out = bound_with(
            &text,
            &Options {
                max_bytes: 10,
                artifact_prefix: "",
            },
            |p, _| {
                assert_eq!(p, "tool-result");
                Err("Refusing unsafe tool-result directory: /tmp".into())
            },
        );
        assert!(out.visible_bytes <= 1024, "ceiling clamps to 1024");
        assert!(
            out.text
                .contains("Full-output artifact unavailable: Refusing unsafe")
        );
        assert!(!out.text.contains("Use the read tool"));
        assert_eq!(out.artifact_path, None);
    }

    #[test]
    fn byte_slices_never_split_chars() {
        assert_eq!(byte_slice("aé", 0, Some(2)), "a");
        assert_eq!(byte_slice("aéb", 0, None), "aéb");
        assert_eq!(ceil_three_quarters(10), 8);
        assert_eq!(safe_prefix("a  b"), "a-b");
    }
}
