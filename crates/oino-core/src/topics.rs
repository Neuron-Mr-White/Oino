//! Frontend topic feed (`_oino/topics`, modes §4): the sticky bus topics extensions publish
//! (unipi `unipi:long-horizon:state`, `unipi:workflow:status`, `unipi:bg:tasks`,
//! `unipi:fusion:status`, `unipi:kanboard:status`, …) plus the core's own `oino:git` topic,
//! mirrored to subscribed frontends as [`crate::events::OinoEvent::Topic`].
//!
//! The extension bus itself is served by the WASM host; it mirrors every publish into
//! [`TopicStore::publish`] through [`crate::extensions::host::HostOp::BusPublish`]. The store
//! keeps the last value of sticky topics per `(cwd, agent dir)` services, so a frontend that
//! subscribes late receives the current state as a snapshot.

use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

use indexmap::IndexMap;
use oino_ai::JsonValue;
use serde::{Deserialize, Serialize};

/// Topic of the core's git summary of the session cwd (`{branch, head, dirty, changed}`).
pub const GIT_TOPIC: &str = "oino:git";

/// Prefix of the core's settings-change topics: `oino:settings:<namespace>`.
pub const SETTINGS_TOPIC_PREFIX: &str = "oino:settings:";

/// Topic announcing a write to extension settings namespace `namespace`
/// (`{namespace, keys, scope, extensionId?}`).
#[must_use]
pub fn settings_topic(namespace: &str) -> String {
    format!("{SETTINGS_TOPIC_PREFIX}{namespace}")
}

/// Publisher name of core-produced topics.
pub const CORE_PUBLISHER: &str = "oino.core";

/// One topic value as frontends see it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicValue {
    /// Topic name.
    pub topic: String,
    /// Payload.
    pub data: JsonValue,
    /// Published sticky (kept for late subscribers).
    pub sticky: bool,
    /// Publishing extension id (`oino.core` for core topics).
    pub publisher: String,
}

/// `pattern` matches `topic`: exact, `prefix*` or `*` (the bus pattern syntax).
#[must_use]
pub fn topic_matches(pattern: &str, topic: &str) -> bool {
    pattern == "*"
        || pattern == topic
        || pattern
            .strip_suffix('*')
            .is_some_and(|prefix| topic.starts_with(prefix))
}

/// Whether any of `patterns` matches `topic`.
#[must_use]
pub fn any_matches(patterns: &[String], topic: &str) -> bool {
    patterns.iter().any(|p| topic_matches(p, topic))
}

/// Last values of sticky topics.
#[derive(Debug, Default)]
pub struct TopicStore {
    sticky: Mutex<IndexMap<String, TopicValue>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl TopicStore {
    /// Records a publish; returns the value to fan out, `None` when a sticky value did not
    /// change (no event needed).
    pub fn publish(
        &self,
        publisher: &str,
        topic: &str,
        data: JsonValue,
        sticky: bool,
    ) -> Option<TopicValue> {
        let value = TopicValue {
            topic: topic.to_owned(),
            data,
            sticky,
            publisher: publisher.to_owned(),
        };
        if sticky {
            let mut map = lock(&self.sticky);
            if map.get(topic).is_some_and(|old| old.data == value.data) {
                return None;
            }
            map.insert(topic.to_owned(), value.clone());
        }
        Some(value)
    }

    /// Last sticky value of `topic`.
    #[must_use]
    pub fn get(&self, topic: &str) -> Option<TopicValue> {
        lock(&self.sticky).get(topic).cloned()
    }

    /// Sticky values matching any of `patterns`, in first-publish order.
    #[must_use]
    pub fn snapshot(&self, patterns: &[String]) -> Vec<TopicValue> {
        lock(&self.sticky)
            .values()
            .filter(|v| any_matches(patterns, &v.topic))
            .cloned()
            .collect()
    }
}

/// Git summary of `cwd` for [`GIT_TOPIC`]: `{branch, head, dirty, changed}`, `null` outside a
/// repository or without `git`.
#[must_use]
pub fn git_info(cwd: &Path) -> JsonValue {
    let output = std::process::Command::new("git")
        .args([
            "status",
            "--porcelain=v2",
            "--branch",
            "--untracked-files=normal",
        ])
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();
    match output {
        Ok(output) if output.status.success() => {
            parse_git_status(&String::from_utf8_lossy(&output.stdout))
        }
        _ => JsonValue::Null,
    }
}

/// Parses `git status --porcelain=v2 --branch`.
fn parse_git_status(text: &str) -> JsonValue {
    let mut branch = None;
    let mut head = None;
    let (mut ahead, mut behind) = (0_i64, 0_i64);
    let mut changed = 0_u64;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("# branch.head ") {
            branch = (rest != "(detached)").then(|| rest.to_owned());
        } else if let Some(rest) = line.strip_prefix("# branch.oid ") {
            head = (rest != "(initial)").then(|| rest.chars().take(7).collect::<String>());
        } else if let Some(rest) = line.strip_prefix("# branch.ab ") {
            for part in rest.split_whitespace() {
                if let Some(n) = part.strip_prefix('+') {
                    ahead = n.parse().unwrap_or(0);
                } else if let Some(n) = part.strip_prefix('-') {
                    behind = n.parse().unwrap_or(0);
                }
            }
        } else if !line.starts_with('#') && !line.is_empty() {
            changed += 1;
        }
    }
    serde_json::json!({
        "branch": branch,
        "head": head,
        "dirty": changed > 0,
        "changed": changed,
        "ahead": ahead,
        "behind": behind,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn patterns() {
        assert!(topic_matches("unipi:*", "unipi:bg:tasks"));
        assert!(topic_matches("*", "x"));
        assert!(topic_matches("a", "a"));
        assert!(!topic_matches("a", "ab"));
        assert!(any_matches(&["x".into(), "oino:*".into()], GIT_TOPIC));
    }

    #[test]
    fn sticky_values_dedupe_and_snapshot() {
        let store = TopicStore::default();
        assert!(store.publish("e", "a:x", json!(1), true).is_some());
        assert!(store.publish("e", "a:x", json!(1), true).is_none());
        assert!(store.publish("e", "a:x", json!(2), true).is_some());
        assert!(store.publish("e", "b:once", json!(1), false).is_some());
        assert!(store.publish("e", "b:once", json!(1), false).is_some());
        assert_eq!(store.snapshot(&["*".into()]).len(), 1);
        assert_eq!(store.get("a:x").unwrap().data, json!(2));
        assert!(store.get("b:once").is_none());
        assert_eq!(store.snapshot(&["b:*".into()]), Vec::new());
    }

    #[test]
    fn git_status_parse() {
        let v = parse_git_status(
            "# branch.oid 0123456789abcdef\n# branch.head main\n# branch.ab +2 -1\n1 .M N... 100644 100644 100644 a b f.rs\n? new.txt\n",
        );
        assert_eq!(
            v,
            json!({"branch": "main", "head": "0123456", "dirty": true, "changed": 2, "ahead": 2, "behind": 1})
        );
        let clean = parse_git_status("# branch.oid (initial)\n# branch.head (detached)\n");
        assert_eq!(clean["dirty"], json!(false));
        assert_eq!(clean["branch"], JsonValue::Null);
    }
}
