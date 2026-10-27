//! pi-tui `fuzzyMatch` / `fuzzyFilter` (pi 1.4.0 `fuzzy.js`), ported exactly so the enchanted
//! list matches what pi's base provider would have offered.

/// Result of [`fuzzy_match`]; lower score = better.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Match {
    /// Every query character appears in order.
    pub matches: bool,
    /// Score (lower is better).
    pub score: f64,
}

const NO_MATCH: Match = Match {
    matches: false,
    score: 0.0,
};

fn is_boundary(c: char) -> bool {
    c.is_whitespace() || matches!(c, '-' | '_' | '.' | '/' | ':')
}

#[allow(clippy::cast_precision_loss, reason = "indices are tiny")]
fn match_query(query: &[char], text: &[char]) -> Match {
    if query.is_empty() {
        return Match {
            matches: true,
            score: 0.0,
        };
    }
    if query.len() > text.len() {
        return NO_MATCH;
    }
    let mut score = 0.0_f64;
    let mut last: Option<usize> = None;
    let mut consecutive = 0_u32;
    let mut matched = 0;
    for &q in query {
        let from = last.map_or(0, |l| l + 1);
        let Some(i) = text
            .get(from..)
            .and_then(|rest| rest.iter().position(|&c| c == q))
            .map(|p| p + from)
        else {
            break;
        };
        let boundary = i == 0 || text.get(i - 1).is_some_and(|&c| is_boundary(c));
        if last.is_some_and(|l| l + 1 == i) {
            consecutive += 1;
            score -= f64::from(consecutive) * 5.0;
        } else {
            consecutive = 0;
            if let Some(l) = last {
                score += (i - l - 1) as f64 * 2.0;
            }
        }
        if boundary {
            score -= 10.0;
        }
        score += i as f64 * 0.1;
        last = Some(i);
        matched += 1;
    }
    if matched < query.len() {
        return NO_MATCH;
    }
    if query == text {
        score -= 100.0;
    }
    Match {
        matches: true,
        score,
    }
}

/// `letters+digits` ↔ `digits+letters` swap (`abc12` → `12abc`), else `None`.
fn swapped(query: &str) -> Option<String> {
    let split = query
        .find(|c: char| c.is_ascii_digit() != query.starts_with(|d: char| d.is_ascii_digit()))?;
    let (a, b) = query.split_at(split);
    let letters = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase());
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    ((letters(a) && digits(b)) || (digits(a) && letters(b))).then(|| format!("{b}{a}"))
}

/// pi `fuzzyMatch(query, text)` (case-insensitive).
#[must_use]
pub fn fuzzy_match(query: &str, text: &str) -> Match {
    let q = query.to_lowercase();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let primary = match_query(&q.chars().collect::<Vec<_>>(), &t);
    if primary.matches {
        return primary;
    }
    let Some(swap) = swapped(&q) else {
        return primary;
    };
    let alt = match_query(&swap.chars().collect::<Vec<_>>(), &t);
    if alt.matches {
        Match {
            matches: true,
            score: alt.score + 5.0,
        }
    } else {
        primary
    }
}

/// pi `fuzzyFilter(items, query, getText)`: every whitespace/slash token must match; sorted by
/// total score (stable).
#[must_use]
pub fn fuzzy_filter<'a, T>(items: &'a [T], query: &str, text: impl Fn(&T) -> &str) -> Vec<&'a T> {
    if query.trim().is_empty() {
        return items.iter().collect();
    }
    let tokens: Vec<&str> = query
        .trim()
        .split(|c: char| c.is_whitespace() || c == '/')
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.is_empty() {
        return items.iter().collect();
    }
    let mut scored: Vec<(&T, f64)> = items
        .iter()
        .filter_map(|item| {
            let mut total = 0.0;
            for token in &tokens {
                let m = fuzzy_match(token, text(item));
                if !m.matches {
                    return None;
                }
                total += m.score;
            }
            Some((item, total))
        })
        .collect();
    scored.sort_by(|a, b| a.1.total_cmp(&b.1));
    scored.into_iter().map(|(i, _)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subsequence_matching() {
        assert!(fuzzy_match("fx", "fix").matches);
        assert!(fuzzy_match("wor", "review-work").matches);
        assert!(!fuzzy_match("xyz", "brainstorm").matches);
        assert!(fuzzy_match("", "anything").matches);
        assert!(!fuzzy_match("toolong", "abc").matches);
    }

    #[test]
    fn exact_beats_prefix_beats_gap() {
        let exact = fuzzy_match("plan", "plan").score;
        let prefix = fuzzy_match("plan", "planner").score;
        let gap = fuzzy_match("plan", "p-l-a-n").score;
        assert!(exact < prefix && prefix < gap, "{exact} {prefix} {gap}");
    }

    #[test]
    fn swapped_alphanumeric_queries() {
        assert_eq!(swapped("abc12").as_deref(), Some("12abc"));
        assert_eq!(swapped("12abc").as_deref(), Some("abc12"));
        assert_eq!(swapped("a1b"), None);
        let m = fuzzy_match("v2gpt", "gpt-v2");
        assert!(!m.matches || m.score > 0.0);
        assert!(fuzzy_match("5gpt", "gpt5").matches);
    }

    #[test]
    fn filter_tokens_all_match_and_sort() {
        let items = ["compact-vcc", "compact", "plan"];
        let out = fuzzy_filter(&items, "compact", |s| s);
        assert_eq!(out, [&"compact", &"compact-vcc"]);
        let out = fuzzy_filter(&items, "pl an", |s| s);
        assert_eq!(out, [&"plan"]);
        assert_eq!(fuzzy_filter(&items, "  ", |s| s).len(), 3);
    }
}
