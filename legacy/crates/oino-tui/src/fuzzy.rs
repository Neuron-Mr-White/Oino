#![forbid(unsafe_code)]

use nucleo::{
    pattern::{CaseMatching, Normalization},
    Config, Nucleo, Utf32String,
};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FuzzyMode {
    Text,
    Path,
}

#[must_use]
pub fn fuzzy_indices<T>(
    items: &[T],
    query: &str,
    mode: FuzzyMode,
    limit: Option<usize>,
    haystack: impl Fn(&T) -> String,
) -> Vec<usize> {
    let limit = limit.unwrap_or(usize::MAX);
    if items.is_empty() || limit == 0 {
        return Vec::new();
    }

    let query = query.trim();
    if query.is_empty() {
        return (0..items.len()).take(limit).collect();
    }

    let mut nucleo = Nucleo::new(config_for_mode(mode), Arc::new(|| ()), Some(1), 1);
    let injector = nucleo.injector();
    for (index, item) in items.iter().enumerate() {
        let haystack = haystack(item);
        injector.push(index, move |_, columns| {
            columns[0] = Utf32String::from(haystack);
        });
    }
    drop(injector);

    nucleo
        .pattern
        .reparse(0, query, CaseMatching::Ignore, Normalization::Smart, false);
    while nucleo.tick(10).running {}

    nucleo
        .snapshot()
        .matched_items(..)
        .map(|item| *item.data)
        .take(limit)
        .collect()
}

pub(crate) fn ascii_subsequence_match(haystack: &str, needle: &str) -> bool {
    ascii_subsequence_match_parts([haystack], needle)
}

pub(crate) fn ascii_subsequence_match_parts<'a>(
    parts: impl IntoIterator<Item = &'a str>,
    needle: &str,
) -> bool {
    if needle.is_empty() {
        return true;
    }
    let mut needle = needle.bytes().map(|byte| byte.to_ascii_lowercase());
    let Some(mut wanted) = needle.next() else {
        return true;
    };
    for byte in parts
        .into_iter()
        .flat_map(str::bytes)
        .map(|byte| byte.to_ascii_lowercase())
    {
        if byte == wanted {
            let Some(next) = needle.next() else {
                return true;
            };
            wanted = next;
        }
    }
    false
}

#[must_use]
fn config_for_mode(mode: FuzzyMode) -> Config {
    let mut config = match mode {
        FuzzyMode::Text => Config::DEFAULT,
        FuzzyMode::Path => Config::DEFAULT.match_paths(),
    };
    config.prefer_prefix = true;
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_indices_uses_path_boundaries() {
        let files = vec![
            "README.md".to_string(),
            "crates/oino-tui/src/app.rs".to_string(),
            "crates/oino-app/src/main.rs".to_string(),
        ];

        let matches = fuzzy_indices(&files, "tui/app", FuzzyMode::Path, Some(10), Clone::clone);
        assert_eq!(matches, vec![1]);
    }

    #[test]
    fn fuzzy_indices_preserves_order_for_empty_query() {
        let items = vec!["beta".to_string(), "alpha".to_string(), "gamma".to_string()];
        let matches = fuzzy_indices(&items, "", FuzzyMode::Text, Some(2), Clone::clone);
        assert_eq!(matches, vec![0, 1]);
    }

    #[test]
    fn ascii_subsequence_match_is_case_insensitive_across_parts() {
        assert!(ascii_subsequence_match_parts(
            ["openrouter:", "Provider", " Model"],
            "PRO mod"
        ));
        assert!(ascii_subsequence_match(
            "crates/Oino-Tui/src/App.rs",
            "TUI/App"
        ));
        assert!(!ascii_subsequence_match("abc", "acb"));
    }
}
