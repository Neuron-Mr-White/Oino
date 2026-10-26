//! Subsequence fuzzy matching (approximation of pi-tui `fuzzyFilter`).
//!
//! Every whitespace-separated query token must match the candidate as a case-insensitive
//! subsequence. Scores prefer contiguous runs, word starts and early matches; lower is better.

/// Score of `candidate` against `query` (`None` = no match). Empty queries match with 0.
#[must_use]
pub fn score(query: &str, candidate: &str) -> Option<i64> {
    let lower = candidate.to_lowercase();
    let chars: Vec<char> = lower.chars().collect();
    let mut total = 0;
    for token in query.split_whitespace() {
        total += token_score(&token.to_lowercase(), &chars)?;
    }
    Some(total)
}

fn token_score(token: &str, chars: &[char]) -> Option<i64> {
    let mut score = 0i64;
    let mut position = 0usize;
    let mut previous: Option<usize> = None;
    for needle in token.chars() {
        let found = chars[position..].iter().position(|&c| c == needle)? + position;
        let gap = i64::try_from(found - previous.map_or(0, |p| p + 1)).unwrap_or(i64::MAX / 4);
        score += gap;
        let word_start = found == 0 || !chars[found - 1].is_alphanumeric();
        if word_start {
            score -= 2;
        }
        if previous.is_some_and(|p| p + 1 == found) {
            score -= 3;
        }
        previous = Some(found);
        position = found + 1;
    }
    Some(score + i64::try_from(chars.len()).unwrap_or(0) / 16)
}

/// Indices of `items` matching `query`, best first (stable for equal scores).
#[must_use]
pub fn filter<T>(items: &[T], query: &str, text: impl Fn(&T) -> String) -> Vec<usize> {
    if query.trim().is_empty() {
        return (0..items.len()).collect();
    }
    let mut scored: Vec<(i64, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| score(query, &text(item)).map(|s| (s, i)))
        .collect();
    scored.sort_by_key(|&(s, i)| (s, i));
    scored.into_iter().map(|(_, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_subsequences_and_ranks_contiguous_first() {
        assert!(score("cls", "claude-sonnet").is_some());
        assert!(score("xyz", "claude").is_none());
        let items = ["gpt-4o-mini", "claude-opus", "opus-mini"];
        let order = filter(&items, "opus", |s| (*s).to_owned());
        assert_eq!(order, vec![2, 1]);
        assert_eq!(filter(&items, "", |s| (*s).to_owned()), vec![0, 1, 2]);
        assert_eq!(filter(&items, "mini gpt", |s| (*s).to_owned()), vec![0]);
    }
}
