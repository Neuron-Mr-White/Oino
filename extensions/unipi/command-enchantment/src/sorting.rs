//! Cross-group tier sort (unipi `src/sorting.ts`, without the `unipi:` namespace tiers: feature
//! commands are bare names and core built-ins always win a tie).

/// Match-quality tier of an item for `query` (lower = better):
///
/// | tier | rule |
/// |---|---|
/// | 0 | value equals the query |
/// | 1 | prefix match |
/// | 2 | fuzzy |
#[must_use]
pub fn cross_item_priority(value: &str, query: &str) -> i32 {
    let full = value.to_lowercase();
    let q = query.to_lowercase();
    if full == q {
        0
    } else if full.starts_with(&q) {
        1
    } else {
        2
    }
}

/// An item tagged with its source group.
#[derive(Clone, Debug, PartialEq)]
pub struct Tagged<T> {
    /// The item.
    pub item: T,
    /// Item value (command name without `/`).
    pub value: String,
    /// Comes from the feature (package-tagged) group.
    pub is_feature: bool,
}

/// unipi `sortTaggedItems`: tier, then built-in/other before feature commands, then (fuzzy
/// tier) shorter values first; stable otherwise.
#[must_use]
pub fn sort_tagged<T: Clone>(items: &[Tagged<T>], query: &str) -> Vec<Tagged<T>> {
    let mut copy = items.to_vec();
    copy.sort_by(|a, b| {
        let pa = cross_item_priority(&a.value, query);
        let pb = cross_item_priority(&b.value, query);
        pa.cmp(&pb)
            .then_with(|| a.is_feature.cmp(&b.is_feature))
            .then_with(|| {
                if pa == 2 {
                    a.value
                        .encode_utf16()
                        .count()
                        .cmp(&b.value.encode_utf16().count())
                } else {
                    std::cmp::Ordering::Equal
                }
            })
    });
    copy
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tagged(value: &str, is_feature: bool) -> Tagged<()> {
        Tagged {
            item: (),
            value: value.to_owned(),
            is_feature,
        }
    }

    fn values(v: &[Tagged<()>]) -> Vec<&str> {
        v.iter().map(|t| t.value.as_str()).collect()
    }

    #[test]
    fn tiers() {
        assert_eq!(cross_item_priority("brainstorm", "brainstorm"), 0);
        assert_eq!(cross_item_priority("Brainstorm", "brainstorm"), 0);
        assert_eq!(cross_item_priority("brainstorm", "brain"), 1);
        assert_eq!(cross_item_priority("review-work", "wor"), 2);
    }

    #[test]
    fn sorting() {
        let s = sort_tagged(&[tagged("review-work", true), tagged("new", false)], "wor");
        assert_eq!(values(&s), ["new", "review-work"]);
        let s = sort_tagged(&[tagged("work", true), tagged("review-work", true)], "wor");
        assert_eq!(values(&s), ["work", "review-work"]);
        let s = sort_tagged(
            &[tagged("ralph-stop", true), tagged("ralph", true)],
            "ralph",
        );
        assert_eq!(values(&s), ["ralph", "ralph-stop"]);
        // Built-ins win a tie with a feature command.
        let s = sort_tagged(&[tagged("abc", true), tagged("abc", false)], "abc");
        assert!(!s[0].is_feature && s[1].is_feature);
        let s = sort_tagged(
            &[tagged("compact-vcc", true), tagged("compact", false)],
            "co",
        );
        assert_eq!(values(&s), ["compact", "compact-vcc"]);
        // Fuzzy tier: shorter first, stable otherwise.
        let s = sort_tagged(
            &[
                tagged("chore-create", true),
                tagged("consultant", true),
                tagged("cd", false),
            ],
            "ce",
        );
        assert_eq!(values(&s), ["cd", "consultant", "chore-create"]);
        let s = sort_tagged(&[tagged("a-first", true), tagged("a-second", true)], "a");
        assert_eq!(values(&s), ["a-first", "a-second"]);
    }
}
