//! Frontend topic feed (`_oino/topics`, modes §4): the sticky bus topics extensions publish
//! (an embedder's `acme:long-horizon:state`, `acme:workflow:status`, `acme:bg:tasks`,
//! `acme:fusion:status`, `acme:kanboard:status`, …) plus the core's own `oino:git` topic,
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

/// Subagent lists are owned by the root session named in their payload.
const SUBAGENTS_LIST: &str = "unipi:subagents:list";

fn owner(value: &TopicValue) -> Option<&str> {
    (value.topic == SUBAGENTS_LIST)
        .then(|| value.data.get("parentSessionId").and_then(JsonValue::as_str))
        .flatten()
        .filter(|id| !id.is_empty())
}

/// Whether a frontend may receive this topic value.
pub(crate) fn visible_to(value: &TopicValue, session_id: &str) -> bool {
    value.topic != SUBAGENTS_LIST || owner(value) == Some(session_id)
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
        let key = if topic == SUBAGENTS_LIST {
            format!("{topic}:{}", owner(&value)?)
        } else {
            topic.to_owned()
        };
        if sticky {
            let mut map = lock(&self.sticky);
            if map.get(&key).is_some_and(|old| old.data == value.data) {
                return None;
            }
            map.insert(key, value.clone());
        }
        Some(value)
    }

    /// Last sticky value of `topic`.
    #[must_use]
    pub fn get(&self, topic: &str) -> Option<TopicValue> {
        lock(&self.sticky).get(topic).cloned()
    }

    /// Last value visible to a particular session.
    pub(crate) fn get_for(&self, topic: &str, session_id: &str) -> Option<TopicValue> {
        let key = if topic == SUBAGENTS_LIST {
            format!("{topic}:{session_id}")
        } else {
            topic.to_owned()
        };
        lock(&self.sticky).get(&key).cloned()
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

    pub(crate) fn snapshot_for(&self, patterns: &[String], session_id: &str) -> Vec<TopicValue> {
        self.snapshot(patterns).into_iter().filter(|v| visible_to(v, session_id)).collect()
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
        assert!(topic_matches("acme:*", "acme:bg:tasks"));
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
    fn subagent_lists_are_scoped_for_snapshots_and_live_delivery() {
        let store = TopicStore::default();
        let a = store.publish("unipi.subagents", SUBAGENTS_LIST,
            json!({"parentSessionId": "A", "records": [{"id": 1}, {"id": 2}, {"id": 3}]}), true).unwrap();
        assert!(visible_to(&a, "A"));
        assert!(!visible_to(&a, "B"));
        let patterns = vec![SUBAGENTS_LIST.to_owned()];
        assert_eq!(store.snapshot_for(&patterns, "A")[0].data["records"].as_array().unwrap().len(), 3);
        assert!(store.snapshot_for(&patterns, "B").is_empty());
        store.publish("unipi.subagents", SUBAGENTS_LIST,
            json!({"parentSessionId": "B", "records": []}), true).unwrap();
        assert_eq!(store.snapshot_for(&patterns, "A")[0].data["records"].as_array().unwrap().len(), 3);
        assert!(store.get_for(SUBAGENTS_LIST, "B").unwrap().data["records"].as_array().unwrap().is_empty());
        assert!(store.publish("unipi.subagents", SUBAGENTS_LIST, json!({"records": [1]}), true).is_none());
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
