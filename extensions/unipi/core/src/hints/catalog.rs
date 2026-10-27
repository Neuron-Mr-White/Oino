//! Hint catalog and selection rules (unipi `src/hints/index.ts` registry + `pick*`).

use std::cmp::Ordering;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use unipi_kit::fmt::compare_versions;

/// Startup hints show at most this many times by default.
pub const MAX_STARTUP: u32 = 3;
/// Event hints show at most this many times by default.
pub const MAX_EVENT: u32 = 2;
/// At most this many event hints per session.
pub const MAX_SESSION_EVENTS: u32 = 4;

/// Hint category.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    /// Slash commands.
    Command,
    /// Keys.
    Shortcut,
    /// Settings.
    Setting,
    /// Capabilities.
    Capability,
    /// Explanations of what just happened.
    Explain,
    /// Troubleshooting.
    Trouble,
    /// What's new since the last version.
    Whatsnew,
    /// Workflows.
    Workflow,
    /// Unicrab lore.
    Lore,
}

impl Category {
    /// Every category in browser order.
    pub const ALL: [Self; 9] = [
        Self::Command,
        Self::Shortcut,
        Self::Setting,
        Self::Capability,
        Self::Explain,
        Self::Trouble,
        Self::Whatsnew,
        Self::Workflow,
        Self::Lore,
    ];

    /// Wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Shortcut => "shortcut",
            Self::Setting => "setting",
            Self::Capability => "capability",
            Self::Explain => "explain",
            Self::Trouble => "trouble",
            Self::Whatsnew => "whatsnew",
            Self::Workflow => "workflow",
            Self::Lore => "lore",
        }
    }
}

/// Declarative payload predicate (replaces unipi's `match` closures).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Match {
    /// Every predicate holds.
    All {
        /// Predicates.
        all: Vec<Match>,
    },
    /// `payload[field] == eq`.
    Eq {
        /// Field.
        field: String,
        /// Value.
        eq: Value,
    },
    /// `String(payload[field])` is one of `in`.
    In {
        /// Field.
        field: String,
        /// Values.
        #[serde(rename = "in")]
        any: Vec<String>,
    },
    /// String field matches `^<prefix>\b`.
    WordPrefix {
        /// Field.
        field: String,
        /// Prefix.
        #[serde(rename = "wordPrefix")]
        prefix: String,
    },
}

impl Match {
    /// Evaluates against an event payload.
    #[must_use]
    pub fn matches(&self, payload: &Value) -> bool {
        match self {
            Self::All { all } => all.iter().all(|m| m.matches(payload)),
            Self::Eq { field, eq } => payload.get(field) == Some(eq),
            Self::In { field, any } => {
                let v = match payload.get(field) {
                    Some(Value::String(s)) => s.clone(),
                    Some(Value::Null) | None => "undefined".to_owned(),
                    Some(other) => other.to_string(),
                };
                any.contains(&v)
            }
            Self::WordPrefix { field, prefix } => payload
                .get(field)
                .and_then(Value::as_str)
                .and_then(|s| s.strip_prefix(prefix.as_str()))
                .is_some_and(|rest| !rest.starts_with(|c: char| c.is_alphanumeric() || c == '_')),
        }
    }
}

/// When a hint shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum When {
    /// `"startup"`.
    Startup(StartupTag),
    /// On an event (pi event, bus topic or derived `hints:*`).
    Event {
        /// Event name.
        event: String,
        /// Payload predicate.
        #[serde(default, rename = "match", skip_serializing_if = "Option::is_none")]
        matcher: Option<Match>,
    },
}

/// The literal `"startup"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StartupTag {
    /// Startup.
    Startup,
}

/// One hint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hint {
    /// Stable id.
    pub id: String,
    /// Category.
    pub category: Category,
    /// Text (≤ 100 chars).
    pub text: String,
    /// Trigger.
    pub when: When,
    /// Show cap override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_shows: Option<u32>,
    /// Command whose use marks the hint learned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub teaches: Option<String>,
    /// Version that introduced a whatsnew hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
}

impl Hint {
    /// Shown at startup / in the cycle pool.
    #[must_use]
    pub fn is_startup(&self) -> bool {
        matches!(self.when, When::Startup(_))
    }

    /// Event name for event hints.
    #[must_use]
    pub fn event(&self) -> Option<&str> {
        match &self.when {
            When::Event { event, .. } => Some(event),
            When::Startup(_) => None,
        }
    }

    /// Show cap.
    #[must_use]
    pub fn max_shows(&self) -> u32 {
        self.max_shows.unwrap_or(if self.is_startup() {
            MAX_STARTUP
        } else {
            MAX_EVENT
        })
    }
}

/// Show count record.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// Times shown.
    pub count: u32,
    /// Last shown (epoch ms).
    pub last: i64,
}

/// Show counts by hint id.
pub type Counts = BTreeMap<String, Record>;

/// The bundled catalog (`assets/hints.json`).
///
/// # Errors
///
/// The bundled JSON is malformed (caught by tests).
pub fn bundled() -> Result<Vec<Hint>, serde_json::Error> {
    serde_json::from_str(include_str!("../../assets/hints.json"))
}

fn rec(counts: &Counts, id: &str) -> Record {
    counts.get(id).copied().unwrap_or_default()
}

/// Least shown, then least recent, among `candidates` under their caps.
fn least_shown<'a>(
    candidates: impl Iterator<Item = &'a Hint>,
    counts: &Counts,
) -> Option<&'a Hint> {
    let mut best: Option<(&Hint, Record)> = None;
    for h in candidates {
        let r = rec(counts, &h.id);
        if r.count >= h.max_shows() {
            continue;
        }
        let better =
            best.is_none_or(|(_, b)| r.count < b.count || (r.count == b.count && r.last < b.last));
        if better {
            best = Some((h, r));
        }
    }
    best.map(|(h, _)| h)
}

/// unipi `pickStartupHint`: excludes lore, whatsnew and learned hints.
#[must_use]
pub fn pick_startup<'a>(
    hints: &'a [Hint],
    counts: &Counts,
    learned: &[String],
) -> Option<&'a Hint> {
    least_shown(
        hints.iter().filter(|h| {
            h.is_startup()
                && !matches!(h.category, Category::Lore | Category::Whatsnew)
                && !learned.contains(&h.id)
        }),
        counts,
    )
}

/// unipi `pickWhatsNewHint`: `last_seen < since <= current`.
#[must_use]
pub fn pick_whats_new<'a>(
    hints: &'a [Hint],
    last_seen: &str,
    current: &str,
    counts: &Counts,
    learned: &[String],
) -> Option<&'a Hint> {
    least_shown(
        hints.iter().filter(|h| {
            h.category == Category::Whatsnew
                && !learned.contains(&h.id)
                && h.since.as_deref().is_some_and(|s| {
                    compare_versions(s, last_seen) == Ordering::Greater
                        && compare_versions(s, current) != Ordering::Greater
                })
        }),
        counts,
    )
}

/// unipi `pickEventHint`: first matching event hint under its cap.
#[must_use]
pub fn pick_event<'a>(
    hints: &'a [Hint],
    event: &str,
    payload: &Value,
    counts: &Counts,
    learned: &[String],
) -> Option<&'a Hint> {
    hints.iter().find(|h| {
        let When::Event { event: e, matcher } = &h.when else {
            return false;
        };
        e == event
            && !learned.contains(&h.id)
            && rec(counts, &h.id).count < h.max_shows()
            && matcher.as_ref().is_none_or(|m| m.matches(payload))
    })
}

/// unipi `pickNextCycleHint`: startup hints (incl. lore/whatsnew), not learned, by count, last,
/// catalog order; skips the active one when possible.
#[must_use]
pub fn pick_next_cycle<'a>(
    hints: &'a [Hint],
    active: Option<&str>,
    counts: &Counts,
    learned: &[String],
) -> Option<&'a Hint> {
    let mut eligible: Vec<(usize, &Hint)> = hints
        .iter()
        .enumerate()
        .filter(|(_, h)| h.is_startup() && !learned.contains(&h.id))
        .collect();
    eligible.sort_by(|(ia, a), (ib, b)| {
        let (ra, rb) = (rec(counts, &a.id), rec(counts, &b.id));
        ra.count
            .cmp(&rb.count)
            .then(ra.last.cmp(&rb.last))
            .then(ia.cmp(ib))
    });
    if let Some(active) = active
        && let Some((_, h)) = eligible.iter().find(|(_, h)| h.id != active)
    {
        return Some(h);
    }
    eligible.first().map(|(_, h)| *h)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    pub(crate) fn startup(id: &str, max: Option<u32>) -> Hint {
        Hint {
            id: id.into(),
            category: Category::Command,
            text: format!("hint {id}"),
            when: When::Startup(StartupTag::Startup),
            max_shows: max,
            teaches: None,
            since: None,
        }
    }

    pub(crate) fn event(id: &str, ev: &str, matcher: Option<Match>, max: Option<u32>) -> Hint {
        Hint {
            id: id.into(),
            category: Category::Explain,
            text: format!("hint {id}"),
            when: When::Event {
                event: ev.into(),
                matcher,
            },
            max_shows: max,
            teaches: None,
            since: None,
        }
    }

    fn counts(pairs: &[(&str, u32, i64)]) -> Counts {
        pairs
            .iter()
            .map(|(id, c, l)| {
                (
                    (*id).to_owned(),
                    Record {
                        count: *c,
                        last: *l,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn startup_least_shown_then_least_recent() {
        let hints = [startup("a", None), startup("b", None), startup("c", None)];
        let c = counts(&[("a", 1, 100), ("b", 0, 50), ("c", 0, 10)]);
        assert_eq!(
            pick_startup(&hints, &c, &[]).map(|h| h.id.as_str()),
            Some("c")
        );
        let c = counts(&[("a", 1, 1), ("b", 1, 5), ("c", 2, 0)]);
        assert_eq!(
            pick_startup(&hints, &c, &[]).map(|h| h.id.as_str()),
            Some("a")
        );
    }

    #[test]
    fn startup_excludes_learned_lore_whatsnew_and_capped() {
        let mut lore = startup("lore", None);
        lore.category = Category::Lore;
        let mut new = startup("new", None);
        new.category = Category::Whatsnew;
        let hints = [lore, new, startup("a", None), startup("b", Some(5))];
        assert_eq!(
            pick_startup(&hints, &Counts::new(), &["a".into()]).map(|h| h.id.as_str()),
            Some("b")
        );
        let c = counts(&[("a", 3, 0), ("b", 4, 0)]);
        assert_eq!(
            pick_startup(&hints, &c, &[]).map(|h| h.id.as_str()),
            Some("b"),
            "default cap 3, explicit 5"
        );
        let c = counts(&[("a", 3, 0), ("b", 5, 0)]);
        assert_eq!(pick_startup(&hints, &c, &[]), None);
        assert_eq!(startup("x", None).max_shows(), MAX_STARTUP);
        assert_eq!(event("y", "e", None, None).max_shows(), MAX_EVENT);
    }

    #[test]
    fn whats_new_window() {
        let mk = |id: &str, since: &str| Hint {
            category: Category::Whatsnew,
            since: Some(since.into()),
            ..startup(id, None)
        };
        let hints = [
            mk("old", "3.0.0-alpha.20"),
            mk("mid", "3.0.0-alpha.22"),
            mk("future", "3.1.0"),
        ];
        let pick = |seen: &str, cur: &str| {
            pick_whats_new(&hints, seen, cur, &Counts::new(), &[]).map(|h| h.id.clone())
        };
        assert_eq!(pick("3.0.0-alpha.21", "3.0.0"), Some("mid".into()));
        assert_eq!(pick("3.0.0-alpha.22", "3.0.0"), None);
        assert_eq!(pick("3.0.0-alpha.19", "3.0.0-alpha.20"), Some("old".into()));
    }

    #[test]
    fn event_first_match_under_cap() {
        let mode_ask = Match::Eq {
            field: "mode".into(),
            eq: json!("ask"),
        };
        let hints = [
            event("ask", "unipi:permission:mode:changed", Some(mode_ask), None),
            event("any", "unipi:permission:mode:changed", None, None),
        ];
        let p = |v: Value, c: &Counts| {
            pick_event(&hints, "unipi:permission:mode:changed", &v, c, &[]).map(|h| h.id.clone())
        };
        assert_eq!(
            p(json!({"mode": "ask"}), &Counts::new()),
            Some("ask".into())
        );
        assert_eq!(
            p(json!({"mode": "full"}), &Counts::new()),
            Some("any".into())
        );
        assert_eq!(
            p(json!({"mode": "ask"}), &counts(&[("ask", 2, 0)])),
            Some("any".into()),
            "capped at 2"
        );
        assert_eq!(
            pick_event(&hints, "other", &json!({}), &Counts::new(), &[]),
            None
        );
    }

    #[test]
    fn cycle_only_startup_pool() {
        let mut lore = startup("lore", None);
        lore.category = Category::Lore;
        let hints = [
            startup("a", None),
            event("e", "x", None, None),
            lore,
            startup("b", None),
        ];
        let c = counts(&[("a", 1, 0)]);
        assert_eq!(
            pick_next_cycle(&hints, None, &c, &[]).map(|h| h.id.as_str()),
            Some("lore")
        );
        assert_eq!(
            pick_next_cycle(&hints, Some("lore"), &c, &[]).map(|h| h.id.as_str()),
            Some("b")
        );
        let only = [startup("a", None)];
        assert_eq!(
            pick_next_cycle(&only, Some("a"), &Counts::new(), &[]).map(|h| h.id.as_str()),
            Some("a")
        );
        assert_eq!(
            pick_next_cycle(&hints[1..2], None, &Counts::new(), &[]),
            None,
            "event hints never cycle"
        );
    }

    #[test]
    fn matches() {
        let lh = Match::In {
            field: "mode".into(),
            any: vec!["goal".into(), "graph".into()],
        };
        assert!(lh.matches(&json!({"mode": "goal"})));
        assert!(!lh.matches(&json!({"mode": "regular"})));
        assert!(!lh.matches(&json!({})));
        let done = Match::All {
            all: vec![
                Match::Eq {
                    field: "event".into(),
                    eq: json!("finished"),
                },
                Match::WordPrefix {
                    field: "reason".into(),
                    prefix: "complete".into(),
                },
            ],
        };
        assert!(done.matches(&json!({"event": "finished", "reason": "complete"})));
        assert!(done.matches(&json!({"event": "finished", "reason": "complete: all items"})));
        assert!(!done.matches(&json!({"event": "finished", "reason": "completed"})));
        assert!(!done.matches(&json!({"event": "suspended", "reason": "complete"})));
    }

    #[test]
    fn bundled_catalog_integrity() {
        let hints = bundled().unwrap_or_default();
        assert_eq!(hints.len(), 132);
        let mut ids: Vec<&str> = hints.iter().map(|h| h.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), hints.len(), "unique ids");
        for c in Category::ALL {
            assert!(hints.iter().any(|h| h.category == c), "{c:?} present");
        }
        assert!(
            hints.iter().all(|h| h.text.chars().count() <= 110),
            "one line (unipi targets 100; one upstream hint is 104)"
        );
        assert!(
            hints
                .iter()
                .filter(|h| h.category == Category::Whatsnew)
                .all(|h| h.since.is_some())
        );
        assert!(
            hints.iter().all(|h| !h.text.contains("/cron")),
            "dropped commands are not advertised"
        );
        let matched = hints
            .iter()
            .filter(|h| {
                matches!(
                    &h.when,
                    When::Event {
                        matcher: Some(_),
                        ..
                    }
                )
            })
            .count();
        assert_eq!(matched, 7);
    }
}
