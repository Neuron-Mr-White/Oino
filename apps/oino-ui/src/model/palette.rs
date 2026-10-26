//! Command palette model (gpui-app §5.5): `>` commands, `#` sessions, `@` files; models and
//! thinking levels reuse the same list. Fuzzy subsequence filtering with simple scoring.

/// What picking an item does (resolved by the workspace).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaletteAction {
    /// App action by id (`new-session`, `toggle-sidebar`, …).
    App(String),
    /// Send a slash command as a prompt (`/compact`).
    Slash(String),
    /// Open a session by id.
    Session(String),
    /// Select a model (`provider`, `id`).
    Model(String, String),
    /// Set a thinking level.
    Thinking(String),
}

/// One palette row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteItem {
    /// Main label.
    pub label: String,
    /// Secondary text.
    pub detail: Option<String>,
    /// Key hint (`Ctrl+N`).
    pub keys: Option<String>,
    /// Group label shown as a section prefix.
    pub group: &'static str,
    /// Marked as current (selected model, active session).
    pub current: bool,
    /// What it does.
    pub action: PaletteAction,
}

/// Which list a palette shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteMode {
    /// Commands + sessions (prefix `>` / `#` narrows).
    Command,
    /// Model picker.
    Models,
    /// Thinking levels.
    Thinking,
}

/// Fuzzy score of `needle` in `hay` (lowercase subsequence); higher is better, `None` if no
/// match. Consecutive and word-start matches score more; shorter haystacks win ties.
#[must_use]
pub fn score(needle: &str, hay: &str) -> Option<i64> {
    if needle.is_empty() {
        return Some(0);
    }
    let hay_chars: Vec<char> = hay.to_lowercase().chars().collect();
    let mut score = 0i64;
    let mut pos = 0usize;
    let mut prev: Option<usize> = None;
    for n in needle.to_lowercase().chars() {
        let found = hay_chars[pos..].iter().position(|&c| c == n)? + pos;
        score += 1;
        if prev.is_some_and(|p| p + 1 == found) {
            score += 4;
        }
        if found == 0 || !hay_chars[found - 1].is_alphanumeric() {
            score += 3;
        }
        prev = Some(found);
        pos = found + 1;
    }
    let len = i64::try_from(hay_chars.len()).unwrap_or(i64::MAX);
    Some(score * 100 - len)
}

/// Filters `items` by `query` (a leading `>` keeps only app/slash commands, `#` only
/// sessions) and sorts by score, keeping input order on ties.
#[must_use]
pub fn filter(items: &[PaletteItem], query: &str) -> Vec<PaletteItem> {
    let (kind, needle) = match query.chars().next() {
        Some('>') => (Some(true), query[1..].trim()),
        Some('#') => (Some(false), query[1..].trim()),
        _ => (None, query.trim()),
    };
    let mut scored: Vec<(i64, usize, &PaletteItem)> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| match kind {
            Some(true) => matches!(item.action, PaletteAction::App(_) | PaletteAction::Slash(_)),
            Some(false) => matches!(item.action, PaletteAction::Session(_)),
            None => true,
        })
        .filter_map(|(i, item)| {
            let best = score(needle, &item.label).max(
                item.detail
                    .as_deref()
                    .and_then(|d| score(needle, d))
                    .map(|s| s - 50),
            )?;
            Some((best, i, item))
        })
        .collect();
    if !needle.is_empty() {
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    }
    scored
        .into_iter()
        .map(|(_, _, item)| item.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(label: &str, action: PaletteAction) -> PaletteItem {
        PaletteItem {
            label: label.into(),
            detail: None,
            keys: None,
            group: "x",
            current: false,
            action,
        }
    }

    #[test]
    fn scoring_prefers_word_starts_and_runs() {
        assert!(score("ns", "New session") > score("ns", "transcript names"));
        assert!(score("new", "New session") > score("new", "renew"));
        assert_eq!(score("zz", "New session"), None);
        assert_eq!(score("", "x"), Some(0));
    }

    #[test]
    fn prefixes_narrow_kinds() {
        let items = vec![
            item("New session", PaletteAction::App("new".into())),
            item("Fix parser", PaletteAction::Session("s1".into())),
            item("/compact", PaletteAction::Slash("compact".into())),
        ];
        assert_eq!(filter(&items, "").len(), 3);
        let commands: Vec<String> = filter(&items, ">").into_iter().map(|i| i.label).collect();
        assert_eq!(commands, vec!["New session", "/compact"]);
        let sessions = filter(&items, "#fix");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].action, PaletteAction::Session("s1".into()));
        assert_eq!(filter(&items, "cmp")[0].label, "/compact");
    }
}
