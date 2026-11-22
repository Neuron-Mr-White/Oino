//! Topic subscriptions over the `events_open` stream (v3-perf spec §4.6, P2).
//!
//! The app sends [`crate::Control::Subscribe`] / [`crate::Control::Unsubscribe`] on its
//! events stream; the host answers each subscribed topic with a
//! [`crate::HubEvent::Topic`] snapshot, then seq-numbered deltas. On reconnect the app
//! sends the last `(epoch, seq)` it saw per topic and gets the missed deltas, or a fresh
//! snapshot when the host cannot replay them (gap, restart, overflow).
//!
//! Negotiation: a host that serves topics sends [`crate::HubEvent::Hello`] with
//! [`TOPICS_CAPABILITY`] as the first event of every events stream (and lists it in
//! `host_info.capabilities`). An older host sends no hello: the app keeps polling.

use serde::{Deserialize, Serialize};

/// The capability string a topic-serving host announces.
pub const TOPICS_CAPABILITY: &str = "topics/1";

/// Longest topic name (prefix + session id / path).
pub const TOPIC_MAX: usize = 4096;

/// Topics without an argument.
pub const PLAIN_TOPICS: &[&str] = &[
    "sessions",
    "superagent",
    "needs_you",
    "extensions",
    "settings",
    "models",
    "ports",
    "cron",
];

/// Topics with a `<prefix>:<argument>` form.
pub const KEYED_TOPICS: &[&str] = &["agents", "tasks", "board", "git", "fs"];

/// A parsed topic name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Topic {
    /// One of [`PLAIN_TOPICS`].
    Plain(&'static str),
    /// `<prefix>:<arg>` (`prefix` one of [`KEYED_TOPICS`]).
    Keyed(&'static str, String),
}

impl Topic {
    /// Parses a wire topic name; `None` for an unknown or malformed one.
    #[must_use]
    pub fn parse(name: &str) -> Option<Topic> {
        if name.is_empty() || name.len() > TOPIC_MAX {
            return None;
        }
        if let Some(plain) = PLAIN_TOPICS.iter().find(|t| **t == name) {
            return Some(Topic::Plain(plain));
        }
        let (prefix, arg) = name.split_once(':')?;
        let prefix = KEYED_TOPICS.iter().find(|t| **t == prefix)?;
        if arg.is_empty() || arg.contains('\0') {
            return None;
        }
        Some(Topic::Keyed(prefix, arg.to_owned()))
    }

    /// The wire name.
    #[must_use]
    pub fn name(&self) -> String {
        match self {
            Topic::Plain(name) => (*name).to_owned(),
            Topic::Keyed(prefix, arg) => format!("{prefix}:{arg}"),
        }
    }
}

/// Where a subscriber left off: the host's numbering epoch and the last seq it applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopicCursor {
    pub epoch: u64,
    pub seq: u64,
}

/// One topic of a `subscribe` control.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopicSub {
    pub topic: String,
    /// Resume point (reconnect); `None` = send a snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<TopicCursor>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_and_keyed_topics() {
        assert_eq!(Topic::parse("sessions"), Some(Topic::Plain("sessions")));
        assert_eq!(
            Topic::parse("git:/home/me/repo"),
            Some(Topic::Keyed("git", "/home/me/repo".into()))
        );
        assert_eq!(Topic::parse("agents:s1").unwrap().name(), "agents:s1");
        for bad in ["", "nope", "git:", "sessions:x", "fs:a\0b", ":x"] {
            assert_eq!(Topic::parse(bad), None, "{bad:?}");
        }
        assert_eq!(Topic::parse(&"x".repeat(TOPIC_MAX + 1)), None);
    }

    #[test]
    fn sub_round_trips() {
        let sub: TopicSub =
            serde_json::from_str(r#"{"topic":"cron","since":{"epoch":3,"seq":9}}"#).unwrap();
        assert_eq!(sub.since, Some(TopicCursor { epoch: 3, seq: 9 }));
        let bare: TopicSub = serde_json::from_str(r#"{"topic":"cron"}"#).unwrap();
        assert_eq!(serde_json::to_string(&bare).unwrap(), r#"{"topic":"cron"}"#);
    }
}
