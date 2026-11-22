//! Call every provider of a service name with a time box (unipi evidence contributors,
//! compaction context, skill conditions, move handlers, work-tray tabs).
//!
//! unipi semantics: each provider is time-boxed (default 2 000 ms); a failure or timeout is
//! "no contribution" and never fails the caller. Calls run concurrently.

use std::time::Duration;

use futures::future::join_all;
use serde_json::Value;

use crate::services::{EvidenceContribution, fanout};

/// unipi's default time box per provider.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_millis(2_000);

/// Extensions serving `name`, in registration order, excluding `except` (usually the caller).
#[must_use]
pub fn providers_of(
    registered: &[(String, String)],
    name: &str,
    except: Option<&str>,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (ext, service) in registered {
        if service == name && Some(ext.as_str()) != except && !out.contains(ext) {
            out.push(ext.clone());
        }
    }
    out
}

/// One provider's answer.
#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    /// Provider extension id.
    pub extension: String,
    /// Reply payload.
    pub value: Value,
}

/// Calls `name` on every provider (except `except`) with `payload`; failed/timed-out providers
/// are dropped. Replies keep provider order.
pub async fn call_all(
    name: &str,
    payload: &Value,
    timeout: Duration,
    except: Option<&str>,
) -> Vec<Reply> {
    let providers = providers_of(&oino_extension::services::list(), name, except);
    let calls = providers.into_iter().map(|ext| async move {
        let value = oino_extension::services::call(Some(&ext), name, payload, Some(timeout))
            .await
            .ok()?;
        Some(Reply {
            extension: ext,
            value,
        })
    });
    join_all(calls).await.into_iter().flatten().collect()
}

/// unipi `gatherEvidence`: merges every `evidence.contribute` reply.
pub async fn gather_evidence(timeout: Duration) -> EvidenceContribution {
    merge_evidence(call_all(fanout::EVIDENCE_CONTRIBUTE, &Value::Null, timeout, None).await)
}

/// Merges evidence replies; undecodable replies contribute nothing.
#[must_use]
pub fn merge_evidence(replies: Vec<Reply>) -> EvidenceContribution {
    let mut out = EvidenceContribution::default();
    for r in replies {
        if let Some(c) = crate::services::decode::<EvidenceContribution>(r.value) {
            out.merge(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn reg(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
            .collect()
    }

    #[test]
    fn providers_filtered_by_name_and_caller() {
        let r = reg(&[
            ("unipi.kanboard", "evidence.contribute"),
            ("unipi.long-horizon", "evidence.contribute"),
            ("unipi.kanboard", "kanboard.capture"),
            ("unipi.kanboard", "evidence.contribute"),
        ]);
        assert_eq!(
            providers_of(&r, "evidence.contribute", None),
            ["unipi.kanboard", "unipi.long-horizon"]
        );
        assert_eq!(
            providers_of(&r, "evidence.contribute", Some("unipi.long-horizon")),
            ["unipi.kanboard"]
        );
        assert_eq!(providers_of(&r, "move.handle", None), Vec::<String>::new());
    }

    #[test]
    fn evidence_aggregates_and_ignores_garbage() {
        let replies = vec![
            Reply {
                extension: "a".into(),
                value: json!({"blocking": ["claim #3 in progress"], "notes": []}),
            },
            Reply {
                extension: "b".into(),
                value: json!("nonsense"),
            },
            Reply {
                extension: "c".into(),
                value: json!({"notes": ["n"]}),
            },
            Reply {
                extension: "d".into(),
                value: Value::Null,
            },
        ];
        let e = merge_evidence(replies);
        assert_eq!(e.blocking, ["claim #3 in progress"]);
        assert_eq!(e.notes, ["n"]);
        assert_eq!(merge_evidence(Vec::new()), EvidenceContribution::default());
    }
}
