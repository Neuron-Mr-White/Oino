//! Cross-group tier sort (unipi `src/sorting.ts`).

/// unipi commands that supersede a same-named pi built-in (`/settings` → `/unipi:settings`).
pub const PREFERRED_OVER_BUILTIN: &[&str] = &["settings"];

/// Match-quality tier of an item for `query` (lower = better):
///
/// | tier | rule |
/// |---|---|
/// | -1 | unipi command superseding the same-named built-in |
/// | 0 | full value equals the query |
/// | 1 | unipi short name (after `unipi:`) equals the query |
/// | 2 | prefix match (short or full) |
/// | 3 | fuzzy |
#[must_use]
pub fn cross_item_priority(value: &str, is_unipi: bool, query: &str) -> i32 {
    let full = value.to_lowercase();
    let short = if is_unipi {
        value.replacen("unipi:", "", 1).to_lowercase()
    } else {
        full.clone()
    };
    let q = query.to_lowercase();
    if is_unipi && short == q && PREFERRED_OVER_BUILTIN.contains(&q.as_str()) {
        return -1;
    }
    if full == q {
        return 0;
    }
    if is_unipi && short == q {
        return 1;
    }
    if short.starts_with(&q) || full.starts_with(&q) {
        return 2;
    }
    3
}

/// An item tagged with its source group.
#[derive(Clone, Debug, PartialEq)]
pub struct Tagged<T> {
    /// The item.
    pub item: T,
    /// Item value (command name without `/`).
    pub value: String,
    /// Comes from the unipi group.
    pub is_unipi: bool,
}

/// unipi `sortTaggedItems`: tier, then non-unipi before unipi, then (fuzzy tier) shorter
/// values first; stable otherwise.
#[must_use]
pub fn sort_tagged<T: Clone>(items: &[Tagged<T>], query: &str) -> Vec<Tagged<T>> {
    let mut copy = items.to_vec();
    copy.sort_by(|a, b| {
        let pa = cross_item_priority(&a.value, a.is_unipi, query);
        let pb = cross_item_priority(&b.value, b.is_unipi, query);
        pa.cmp(&pb)
            .then_with(|| a.is_unipi.cmp(&b.is_unipi))
            .then_with(|| {
                if pa == 3 {
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

    fn tagged(value: &str, is_unipi: bool) -> Tagged<()> {
        Tagged {
            item: (),
            value: value.to_owned(),
            is_unipi,
        }
    }

    fn values(v: &[Tagged<()>]) -> Vec<&str> {
        v.iter().map(|t| t.value.as_str()).collect()
    }

    #[test]
    fn tiers() {
        assert_eq!(cross_item_priority("new", false, "new"), 0);
        assert_eq!(
            cross_item_priority("unipi:brainstorm", true, "unipi:brainstorm"),
            0
        );
        assert_ne!(
            cross_item_priority("unipi:brainstorm", true, "brainstorm"),
            0
        );
        assert_eq!(
            cross_item_priority("unipi:brainstorm", true, "brainstorm"),
            1
        );
        assert!(cross_item_priority("unipi:brainstorm", true, "brain") > 1);
        assert_eq!(cross_item_priority("work", false, "wor"), 2);
        assert_eq!(cross_item_priority("unipi:work", true, "wor"), 2);
        assert_eq!(
            cross_item_priority("unipi:compact-vcc", true, "unipi:co"),
            2
        );
        assert_eq!(cross_item_priority("unipi:review-work", true, "wor"), 3);
        assert_eq!(cross_item_priority("fix", false, "fx"), 3);
        assert_eq!(cross_item_priority("brainstorm", false, "xyz"), 3);
        assert_eq!(cross_item_priority("new", false, "New"), 0);
        assert_eq!(cross_item_priority("New", false, "new"), 0);
        assert_eq!(cross_item_priority("Compact", false, "co"), 2);
        assert_eq!(
            cross_item_priority("unipi:Brainstorm", true, "brainstorm"),
            1
        );
        assert_eq!(cross_item_priority("btw:new", false, "new"), 3);
        assert_eq!(
            cross_item_priority("unipi:brainstorm", true, "unipi:bra"),
            2
        );
        assert_eq!(cross_item_priority("unipi:settings", true, "settings"), -1);
    }

    #[test]
    fn merge_sort_cases() {
        let s = sort_tagged(
            &[tagged("unipi:review-work", true), tagged("new", false)],
            "new",
        );
        assert_eq!(values(&s), ["new", "unipi:review-work"]);
        let s = sort_tagged(
            &[tagged("unipi:brainstorm", true), tagged("unipi:plan", true)],
            "brainstorm",
        );
        assert_eq!(s[0].value, "unipi:brainstorm");
        let s = sort_tagged(
            &[
                tagged("unipi:brainstorm", true),
                tagged("brainstorm", false),
            ],
            "unipi:brainstorm",
        );
        assert_eq!(s[0].value, "unipi:brainstorm");
        let s = sort_tagged(&[tagged("unipi:work", true), tagged("work", false)], "wor");
        assert_eq!(values(&s), ["work", "unipi:work"]);
        let s = sort_tagged(
            &[
                tagged("unipi:review-work", true),
                tagged("fix", false),
                tagged("unipi:fix", true),
            ],
            "fx",
        );
        assert_eq!(values(&s), ["fix", "unipi:fix", "unipi:review-work"]);
        let s = sort_tagged(
            &[
                tagged("unipi:ralph", true),
                tagged("unipi:ralph-stop", true),
            ],
            "ralph",
        );
        assert_eq!(values(&s), ["unipi:ralph", "unipi:ralph-stop"]);
        let s = sort_tagged(
            &[
                tagged("skill:animate", false),
                tagged("skill:audit", false),
                tagged("unipi:auto", true),
            ],
            "a",
        );
        assert_eq!(values(&s), ["unipi:auto", "skill:audit", "skill:animate"]);
        let s = sort_tagged(
            &[
                tagged("skill:work", false),
                tagged("unipi:work", true),
                tagged("skill:workshop", false),
            ],
            "work",
        );
        assert_eq!(values(&s), ["unipi:work", "skill:work", "skill:workshop"]);
        let s = sort_tagged(
            &[
                tagged("unipi:brain", true),
                tagged("new", false),
                tagged("unipi:work", true),
                tagged("fix", false),
            ],
            "",
        );
        assert_eq!(values(&s), ["new", "fix", "unipi:brain", "unipi:work"]);
        let s = sort_tagged(
            &[
                tagged("btw:new", false),
                tagged("new", false),
                tagged("unipi:new-thing", true),
            ],
            "new",
        );
        assert_eq!(values(&s), ["new", "unipi:new-thing", "btw:new"]);
    }

    #[test]
    fn mixed_tiers_and_stability() {
        let s = sort_tagged(
            &[
                tagged("unipi:compact-vcc", true),
                tagged("compact-xyz", false),
                tagged("co", false),
                tagged("unipi:consultant", true),
                tagged("config", false),
                tagged("unipi:chore-create", true),
            ],
            "co",
        );
        assert_eq!(
            values(&s),
            [
                "co",
                "compact-xyz",
                "config",
                "unipi:compact-vcc",
                "unipi:consultant",
                "unipi:chore-create"
            ]
        );
        let s = sort_tagged(
            &[
                tagged("alpha", false),
                tagged("unipi:a-first", true),
                tagged("beta", false),
                tagged("unipi:a-second", true),
                tagged("gamma", false),
            ],
            "a",
        );
        let v = values(&s);
        let plain: Vec<&str> = v
            .iter()
            .copied()
            .filter(|x| !x.starts_with("unipi:"))
            .collect();
        let unipi: Vec<&str> = v
            .iter()
            .copied()
            .filter(|x| x.starts_with("unipi:"))
            .collect();
        assert_eq!(plain, ["alpha", "beta", "gamma"]);
        assert_eq!(unipi, ["unipi:a-first", "unipi:a-second"]);
        let s = sort_tagged(
            &[
                tagged("unipi:ab", true),
                tagged("cd", false),
                tagged("ef", false),
                tagged("unipi:gh", true),
            ],
            "z",
        );
        assert_eq!(values(&s), ["cd", "ef", "unipi:ab", "unipi:gh"]);
        assert_eq!(sort_tagged::<()>(&[], "new"), Vec::new());
        let both = sort_tagged(
            &[tagged("unipi:abc", true), tagged("unipi:abc", false)],
            "unipi:abc",
        );
        assert!(!both[0].is_unipi && both[1].is_unipi);
    }

    #[test]
    fn supersede_rule() {
        let s = sort_tagged(
            &[tagged("settings", false), tagged("unipi:settings", true)],
            "settings",
        );
        assert_eq!(values(&s), ["unipi:settings", "settings"]);
        let s = sort_tagged(
            &[tagged("model", false), tagged("unipi:model", true)],
            "model",
        );
        assert_eq!(values(&s), ["model", "unipi:model"]);
        let s = sort_tagged(
            &[tagged("unipi:Brainstorm", true), tagged("New", false)],
            "new",
        );
        assert_eq!(s[0].value, "New");
        let s = sort_tagged(
            &[tagged("unipi:Brainstorm", true), tagged("New", false)],
            "BRAINSTORM",
        );
        assert_eq!(s[0].value, "unipi:Brainstorm");
    }
}
