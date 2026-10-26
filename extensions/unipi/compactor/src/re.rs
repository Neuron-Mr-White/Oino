//! Lazily compiled regexes (unipi's JS regexes via `fancy-regex`, which supports the
//! lookaround and backreferences they use). A pattern that fails to compile matches nothing;
//! `tests::all_compile` guards every pattern.

use std::borrow::Cow;
use std::sync::LazyLock;

pub(crate) use fancy_regex::Regex;

/// A lazily compiled regex.
pub(crate) type Lazy = LazyLock<Option<Regex>>;

/// Declares lazily compiled regexes; also collects them for the compile test.
macro_rules! regexes {
    ($($name:ident = $pat:expr;)*) => {
        $(pub(crate) static $name: $crate::re::Lazy = std::sync::LazyLock::new(|| fancy_regex::Regex::new($pat).ok());)*
        #[cfg(test)]
        pub(crate) fn all_patterns() -> Vec<(&'static str, &'static $crate::re::Lazy)> {
            vec![$((stringify!($name), &$name)),*]
        }
    };
}
pub(crate) use regexes;

/// `re.test(s)`.
pub(crate) fn test(re: &Lazy, s: &str) -> bool {
    re.as_ref().is_some_and(|r| r.is_match(s).unwrap_or(false))
}

/// Capture group `i` of the first match.
pub(crate) fn cap(re: &Lazy, s: &str, i: usize) -> Option<String> {
    let caps = re.as_ref()?.captures(s).ok()??;
    caps.get(i).map(|m| m.as_str().to_owned())
}

/// All captures of every match.
pub(crate) fn caps_all(re: &Lazy, s: &str) -> Vec<Vec<Option<String>>> {
    let Some(r) = re.as_ref() else {
        return Vec::new();
    };
    r.captures_iter(s)
        .filter_map(Result::ok)
        .map(|c| {
            (0..c.len())
                .map(|i| c.get(i).map(|m| m.as_str().to_owned()))
                .collect()
        })
        .collect()
}

/// `s.replace(re /g, to)`.
pub(crate) fn replace_all<'a>(re: &Lazy, s: &'a str, to: &str) -> Cow<'a, str> {
    match re.as_ref() {
        Some(r) => r.replace_all(s, to),
        None => Cow::Borrowed(s),
    }
}

/// `s.replace(re, to)` (first match).
pub(crate) fn replace_first<'a>(re: &Lazy, s: &'a str, to: &str) -> Cow<'a, str> {
    match re.as_ref() {
        Some(r) => r.replace(s, to),
        None => Cow::Borrowed(s),
    }
}

/// Splits on matches (JS `split(re)` without captures).
pub(crate) fn split<'a>(re: &Lazy, s: &'a str) -> Vec<&'a str> {
    let Some(r) = re.as_ref() else {
        return vec![s];
    };
    let mut out = Vec::new();
    let mut last = 0;
    for m in r.find_iter(s).filter_map(Result::ok) {
        if m.end() == m.start() && m.start() == last && last != 0 {
            continue;
        }
        out.push(&s[last..m.start()]);
        last = m.end();
    }
    out.push(&s[last..]);
    out
}
