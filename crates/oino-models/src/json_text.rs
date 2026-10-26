//! pi `utils/json.ts` + `utils/text.ts`: lenient JSON text preprocessing for config files.

use std::sync::LazyLock;

use regex::{Captures, Regex};

#[allow(
    clippy::expect_used,
    reason = "constant pattern, covered by unit tests; failure is a programming error"
)]
static LINE_COMMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""(?:\\.|[^"\\])*"|//[^\n]*"#).expect("valid regex"));

#[allow(
    clippy::expect_used,
    reason = "constant pattern, covered by unit tests; failure is a programming error"
)]
static TRAILING_COMMA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""(?:\\.|[^"\\])*"|,(\s*[}\]])"#).expect("valid regex"));

/// pi `stripBom`: drops a leading U+FEFF.
#[must_use]
pub(crate) fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

/// pi `stripJsonComments`: removes `//` line comments and trailing commas outside strings.
#[must_use]
pub(crate) fn strip_json_comments(text: &str) -> String {
    let without_comments = LINE_COMMENT.replace_all(text, |caps: &Captures<'_>| {
        let matched = &caps[0];
        if matched.starts_with('"') {
            matched.to_owned()
        } else {
            String::new()
        }
    });
    TRAILING_COMMA
        .replace_all(&without_comments, |caps: &Captures<'_>| {
            caps.get(1)
                .map_or_else(|| caps[0].to_owned(), |tail| tail.as_str().to_owned())
        })
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_comments_and_trailing_commas() {
        let input = "{ // c\n \"a\": \"x // y\", \"b\": [1,2,],}";
        assert_eq!(
            strip_json_comments(input),
            "{ \n \"a\": \"x // y\", \"b\": [1,2]}"
        );
        assert_eq!(strip_bom("\u{feff}{}"), "{}");
    }
}
