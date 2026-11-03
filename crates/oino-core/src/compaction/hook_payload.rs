//! The bounded `session_before_compact` payload (compactor-fix).
//!
//! pi hands the hook the whole branch (`branchEntries`) and the preparation. Extensions
//! receive it as JSON inside their wasm linear memory, which never shrinks: a 213 MB chat
//! grew the compactor past its memory limit and trapped it. Small payloads stay exactly as
//! pi builds them; above [`HookPayloadLimits`] they are bounded in three lossless-first steps:
//!
//! 1. **Slim** — tool-result / custom-message `details` larger than
//!    [`HookPayloadLimits::details_bytes`] are dropped and image `data` is emptied. Neither
//!    reaches a summary (pi's `serializeConversation` reads only text; the compactor counts an
//!    image as a fixed size and reads `details` only from small `ask` results).
//! 2. **Clip** — a single entry still larger than [`HookPayloadLimits::entry_bytes`] has its
//!    strings clipped to [`HookPayloadLimits::clip_bytes`].
//! 3. **Window** — branch entries are kept newest first while they fit
//!    [`HookPayloadLimits::branch_bytes`]; the live context (the newest compaction and the
//!    entries after its first kept entry) is the newest part, so it goes in before any
//!    compacted-away history. The omitted prefix is reported as `branchEntriesOmitted`.
//!
//! Sizes are measured by serializing into a byte counter, so bounding never materializes the
//! whole branch as JSON.

use std::io;

use oino_session::SessionEntry;
use serde::Serialize;
use serde_json::Value;

use super::CompactionPreparation;

/// Byte limits of the hook payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HookPayloadLimits {
    /// `branchEntries` above this (serialized) is bounded.
    pub branch_bytes: usize,
    /// The preparation above this (serialized) is slimmed.
    pub preparation_bytes: usize,
    /// `details` objects up to this size survive slimming.
    pub details_bytes: usize,
    /// A slimmed entry above this has its strings clipped.
    pub entry_bytes: usize,
    /// String length after clipping.
    pub clip_bytes: usize,
}

/// The limits core uses. Two 4 MiB halves keep the compactor under 64 MiB of linear memory
/// on the 213 MB chat; 8 MiB halves took it to 128 MiB (many small entries parse to several
/// times their JSON size, and the lossless plan copies the live messages).
pub const HOOK_PAYLOAD_LIMITS: HookPayloadLimits = HookPayloadLimits {
    branch_bytes: 4 << 20,
    preparation_bytes: 4 << 20,
    details_bytes: 16 << 10,
    entry_bytes: 512 << 10,
    clip_bytes: 64 << 10,
};

/// `branchEntries` as handed to `session_before_compact`.
#[derive(Debug, Default, PartialEq)]
pub struct BoundedBranch {
    /// Entries, oldest first (a suffix of the branch).
    pub entries: Vec<Value>,
    /// Older branch entries left out.
    pub omitted: usize,
    /// Whether any entry was slimmed or clipped.
    pub slimmed: bool,
}

/// Counts serialized bytes without storing them.
struct Counter(usize);

impl io::Write for Counter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0 += buf.len();
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Serialized JSON size of `value`.
#[must_use]
pub fn json_size<T: Serialize + ?Sized>(value: &T) -> usize {
    let mut counter = Counter(0);
    // A serialization error (non-string map keys) only under-counts; the payload builder
    // would fail the same way.
    let _ = serde_json::to_writer(&mut counter, value);
    counter.0
}

/// Drops large `details` and empties image bytes in an entry or message value.
fn slim(value: &mut Value, limits: &HookPayloadLimits) -> bool {
    let mut changed = false;
    if let Value::Object(map) = value {
        if map
            .get("details")
            .is_some_and(|d| json_size(d) > limits.details_bytes)
        {
            map.remove("details");
            changed = true;
        }
        if let Some(message) = map.get_mut("message") {
            changed |= slim(message, limits);
        }
        if let Some(content) = map.get_mut("content") {
            changed |= empty_images(content);
        }
    }
    changed
}

fn empty_images(content: &mut Value) -> bool {
    let Value::Array(parts) = content else {
        return false;
    };
    let mut changed = false;
    for part in parts {
        if part.get("type").and_then(Value::as_str) == Some("image")
            && let Some(Value::String(data)) = part.get_mut("data")
            && !data.is_empty()
        {
            data.clear();
            changed = true;
        }
    }
    changed
}

/// Slims (and if still too large clips) one value; returns whether it changed.
fn bound_value(value: &mut Value, limits: &HookPayloadLimits) -> bool {
    let mut changed = slim(value, limits);
    if json_size(value) > limits.entry_bytes {
        oino_extension_api::ops::clip_strings(value, limits.clip_bytes);
        changed = true;
    }
    changed
}

/// `branchEntries` for `session_before_compact`: the branch unchanged when it fits
/// [`HookPayloadLimits::branch_bytes`], else the bounded newest suffix (module docs).
#[must_use]
pub fn bound_branch_entries(branch: &[SessionEntry], limits: &HookPayloadLimits) -> BoundedBranch {
    let total: usize = branch.iter().map(json_size).sum::<usize>() + branch.len() + 1;
    if total <= limits.branch_bytes {
        return BoundedBranch {
            entries: branch
                .iter()
                .filter_map(|e| serde_json::to_value(e).ok())
                .collect(),
            ..BoundedBranch::default()
        };
    }
    let mut out = Vec::new();
    let mut used = 2;
    let mut slimmed = false;
    for entry in branch.iter().rev() {
        let Ok(mut value) = serde_json::to_value(entry) else {
            continue;
        };
        let changed = bound_value(&mut value, limits);
        let size = json_size(&value) + 1;
        if used + size > limits.branch_bytes && !out.is_empty() {
            break;
        }
        used += size;
        slimmed |= changed;
        out.push(value);
    }
    let omitted = branch.len() - out.len();
    out.reverse();
    BoundedBranch {
        entries: out,
        omitted,
        slimmed,
    }
}

/// The preparation for `session_before_compact`: unchanged when it fits
/// [`HookPayloadLimits::preparation_bytes`], else its messages slimmed (and clipped when one
/// is still too large); when that is still too large, the oldest `messagesToSummarize` are
/// left out and counted in `messagesToSummarizeOmitted`. Returns the value and whether it
/// was bounded. (Core's own summariser always uses the full preparation.)
#[must_use]
pub fn bound_preparation(
    preparation: &CompactionPreparation,
    limits: &HookPayloadLimits,
) -> (Value, bool) {
    let mut value = serde_json::to_value(preparation).unwrap_or(Value::Null);
    if json_size(&value) <= limits.preparation_bytes {
        return (value, false);
    }
    let mut changed = false;
    for key in ["messagesToSummarize", "turnPrefixMessages"] {
        if let Some(Value::Array(messages)) = value.get_mut(key) {
            for message in messages {
                changed |= bound_value(message, limits);
            }
        }
    }
    let mut over = json_size(&value).saturating_sub(limits.preparation_bytes);
    if over > 0
        && let Some(Value::Array(messages)) = value.get_mut("messagesToSummarize")
    {
        let mut drop = 0;
        while over > 0 && drop < messages.len() {
            over = over.saturating_sub(json_size(&messages[drop]) + 1);
            drop += 1;
        }
        messages.drain(..drop);
        value["messagesToSummarizeOmitted"] = Value::from(drop);
        changed = true;
    }
    (value, changed)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use serde_json::json;

    use super::*;

    const SMALL: HookPayloadLimits = HookPayloadLimits {
        branch_bytes: 64 << 10,
        preparation_bytes: 64 << 10,
        details_bytes: 1 << 10,
        entry_bytes: 16 << 10,
        clip_bytes: 4 << 10,
    };

    fn entry(v: Value) -> SessionEntry {
        serde_json::from_value(v).unwrap()
    }

    fn user(id: &str, parent: Option<&str>, text: &str) -> SessionEntry {
        entry(
            json!({"type": "message", "id": id, "parentId": parent, "timestamp": "2026-11-03T00:00:00.000Z",
            "message": {"role": "user", "content": [{"type": "text", "text": text}], "timestamp": 1}}),
        )
    }

    fn tool_result(id: &str, parent: &str, details: usize, image: usize) -> SessionEntry {
        entry(
            json!({"type": "message", "id": id, "parentId": parent, "timestamp": "2026-11-03T00:00:00.000Z",
            "message": {"role": "toolResult", "toolCallId": "c", "toolName": "edit",
                "content": [{"type": "text", "text": "Successfully edited f.rs"},
                            {"type": "image", "data": "A".repeat(image), "mimeType": "image/png"}],
                "details": {"oldContent": "x".repeat(details)}, "isError": false, "timestamp": 1}}),
        )
    }

    fn branch(n: usize, details: usize) -> Vec<SessionEntry> {
        let mut out = vec![user("u0", None, "start")];
        for i in 1..n {
            let parent = format!("e{}", i - 1);
            let parent = if i == 1 { "u0" } else { parent.as_str() };
            out.push(tool_result(&format!("e{i}"), parent, details, 8));
        }
        out
    }

    #[test]
    fn small_branch_is_the_branch_unchanged() {
        let b = branch(10, 100);
        let bounded = bound_branch_entries(&b, &SMALL);
        assert_eq!(bounded.omitted, 0);
        assert!(!bounded.slimmed);
        assert_eq!(
            Value::Array(bounded.entries),
            serde_json::to_value(&b).unwrap()
        );
    }

    #[test]
    fn large_branch_is_slimmed_before_anything_is_dropped() {
        // 40 entries × 8 KiB details: 320 KiB raw, a few KiB once the details are gone.
        let b = branch(40, 8 << 10);
        let bounded = bound_branch_entries(&b, &SMALL);
        assert_eq!(bounded.omitted, 0);
        assert!(bounded.slimmed);
        assert_eq!(bounded.entries.len(), 40);
        let last = &bounded.entries[39]["message"];
        assert!(last.get("details").is_none());
        assert_eq!(last["content"][1]["data"], "");
        assert_eq!(last["content"][0]["text"], "Successfully edited f.rs");
        assert!(json_size(&bounded.entries) <= SMALL.branch_bytes);
    }

    #[test]
    fn huge_branch_keeps_the_newest_suffix_within_the_cap() {
        let mut b = vec![user("u0", None, "start")];
        for i in 1..400 {
            let parent = if i == 1 {
                "u0".to_owned()
            } else {
                format!("e{}", i - 1)
            };
            b.push(user(&format!("e{i}"), Some(&parent), &"t".repeat(1 << 10)));
        }
        let bounded = bound_branch_entries(&b, &SMALL);
        assert!(bounded.omitted > 0);
        assert_eq!(bounded.omitted + bounded.entries.len(), 400);
        assert!(json_size(&bounded.entries) <= SMALL.branch_bytes);
        assert_eq!(bounded.entries.last().unwrap()["id"], "e399");
        // A contiguous suffix, oldest first.
        let first = 400 - bounded.entries.len();
        assert_eq!(bounded.entries[0]["id"], format!("e{first}"));
    }

    #[test]
    fn an_oversized_entry_is_clipped_not_dropped() {
        let b = vec![
            user("u0", None, "start"),
            user("u1", Some("u0"), &"t".repeat(200 << 10)),
        ];
        let bounded = bound_branch_entries(&b, &SMALL);
        assert_eq!(bounded.entries.len(), 2);
        assert!(bounded.slimmed);
        let text = bounded.entries[1]["message"]["content"][0]["text"]
            .as_str()
            .unwrap();
        assert_eq!(text.len(), SMALL.clip_bytes);
    }

    #[test]
    fn small_details_survive_slimming() {
        let mut v = serde_json::to_value(tool_result("e1", "u0", 100, 0)).unwrap();
        v["message"]["details"] = json!({"questions": ["q"], "answers": ["a"]});
        assert!(!slim(&mut v, &SMALL));
        assert_eq!(v["message"]["details"]["answers"][0], "a");
    }
}
