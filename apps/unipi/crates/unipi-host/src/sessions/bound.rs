//! Bounding a session snapshot for the 1 MiB frame (v2 QA defect: sessions larger
//! than ~1 MiB failed `session_open` with "oversize frame from host" — the app's
//! `read_frame` rejects frames over [`unipi_proto::frame::MAX_FRAME`], and a big
//! session's inline base64 images and multi-megabyte tool outputs blew the snapshot
//! reply apart).
//!
//! [`bound_entries`] rewrites a snapshot's `entries` in place so any session opens:
//!
//! - **Big text** — a tool result, user or assistant text longer than the per-entry
//!   budget is cut to its head and the message carries `truncated: true`, exactly
//!   the flag the app's full-output path understands (`useFullOutput`: it refetches
//!   the whole entry with `get_entries{ids}` on expand, app §3.4 step 7). If the
//!   entries still exceed the total budget (a thousand-entry session), the
//!   per-entry budget shrinks until they fit.
//! - **Big images** — an inline base64 `data` larger than
//!   [`INLINE_IMAGE_LIMIT`] is replaced by `{type:"image", mime/mediaRef, bytes}`:
//!   the decoded bytes go into the host's blob store
//!   ([`crate::blob::store_bytes`], sha256-addressed, deduplicated, LRU-capped)
//!   and `mediaRef` is the digest. The app's image parts already model `mediaRef`
//!   (a chip shows a placeholder until fetched); the session stream answers the
//!   `media{mediaRef}` command from the same store.

use std::path::Path;

use base64::Engine as _;
use serde_json::Value;

use crate::blob;

/// Inline base64 image data longer than this becomes a blob `mediaRef`.
pub const INLINE_IMAGE_LIMIT: usize = 16 * 1024;
/// Default per-entry text budget (bytes), shared by the entry's text blocks.
pub const ENTRY_TEXT_BUDGET: usize = 8 * 1024;
/// Total budget for the whole entries array: the snapshot frame must stay well
/// under `MAX_FRAME` even after the rest of the payload.
pub const ENTRIES_BUDGET: usize = 512 * 1024;
/// Smallest per-entry text budget the shrink loop accepts.
const MIN_ENTRY_TEXT: usize = 256;
/// Marker suffix appended to truncated text, telling the user why it stops.
const TRUNCATION_NOTE: &str = "\n…[truncated by the host — expand for the full output]";

/// Bounds `entries` (the snapshot's `entries` array) in place. Returns how many
/// entries were rewritten (truncated text or referenced-out images).
///
/// Blob-store failures degrade gracefully: an image that cannot be stored keeps
/// its inline `data` (rare; the text shrink loop still bounds the rest).
#[must_use]
pub fn bound_entries(entries: &mut [Value], blobs_dir: &Path) -> usize {
    let mut rewritten = rewrite_all(entries, ENTRY_TEXT_BUDGET, blobs_dir);
    // A huge session still over the total budget: shrink the per-entry text
    // budget (the app expands the entries it cares about) until the array fits.
    let mut budget = ENTRY_TEXT_BUDGET;
    while entries.len() > 1 && budget > MIN_ENTRY_TEXT && serialized_len(entries) > ENTRIES_BUDGET {
        budget = (budget / 2).max(MIN_ENTRY_TEXT);
        rewritten += rewrite_all(entries, budget, blobs_dir);
    }
    rewritten
}

/// Serialized size of the entries array (what the frame carries).
#[must_use]
pub fn serialized_len(entries: &[Value]) -> usize {
    entries
        .iter()
        .map(|entry| serde_json::to_vec(entry).map_or(0, |bytes| bytes.len()))
        .sum()
}

/// One pass of per-entry bounding at `text_budget` bytes per message (the entry's
/// text blocks share it).
fn rewrite_all(entries: &mut [Value], text_budget: usize, blobs_dir: &Path) -> usize {
    let mut rewritten = 0;
    for entry in entries {
        // Entries are `{id, parentId, timestamp, type, message?}`; non-message
        // kinds have nothing to bound.
        let Some(message) = entry.get_mut("message").and_then(Value::as_object_mut) else {
            continue;
        };
        let mut remaining = text_budget;
        let mut changed = false;
        match message.get_mut("content") {
            Some(Value::String(text)) => {
                changed |= truncate_string(text, &mut remaining);
            }
            Some(Value::Array(blocks)) => {
                for block in blocks.iter_mut() {
                    changed |= bound_block(block, &mut remaining, blobs_dir);
                }
            }
            _ => {}
        }
        if changed {
            // The app's full-output check: `message.truncated === true`
            // (`useFullOutput.snapshotTruncated`), refetched on expand.
            message.insert("truncated".into(), Value::Bool(true));
            rewritten += 1;
        }
    }
    rewritten
}

/// Bounds one content block: text heads are cut, big images become media refs.
fn bound_block(block: &mut Value, remaining: &mut usize, blobs_dir: &Path) -> bool {
    let Some(block) = block.as_object_mut() else {
        return false;
    };
    let kind = block
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let mut changed = false;
    if kind == "text" {
        if let Some(Value::String(text)) = block.get_mut("text") {
            changed |= truncate_string(text, remaining);
        }
    }
    if kind == "image" {
        if let Some(Value::String(data)) = block.get_mut("data") {
            if data.len() > INLINE_IMAGE_LIMIT {
                if let Some(reference) = reference_image(data, blobs_dir) {
                    // The app's image-part shape (`lib/chat/model.ts`): `data` for small
                    // inline images, `mediaRef` the previewer fetches with
                    // `media{mediaRef}` over the session stream (or `blob_get{sha256}`).
                    let bytes = data.len() / 4 * 3; // decoded size, before it was replaced
                    block.remove("data");
                    block.insert("mediaRef".into(), Value::String(reference));
                    block.insert("bytes".into(), Value::from(bytes));
                    changed = true;
                }
            }
        }
    }
    changed
}

/// Cuts `text` to `*remaining` budget (char-boundary safe) with the marker, when
/// it exceeds it; consumes the budget either way. Blocks after the budget is spent
/// empty out without their own marker — the entry carries `truncated: true`.
fn truncate_string(text: &mut String, remaining: &mut usize) -> bool {
    if *remaining == 0 {
        if text.is_empty() {
            return false;
        }
        text.clear();
        return true;
    }
    if text.len() <= *remaining {
        *remaining -= text.len();
        return false;
    }
    let mut keep = remaining.saturating_sub(TRUNCATION_NOTE.len());
    while keep > 0 && !text.is_char_boundary(keep) {
        keep -= 1;
    }
    text.truncate(keep);
    text.push_str(TRUNCATION_NOTE);
    *remaining = 0;
    true
}

/// Decodes the inline base64 image and stores it as a blob; `Some(sha256)` is the
/// `mediaRef` the app fetches with `media{mediaRef}` (or `blob_get{sha256}`).
fn reference_image(data: &str, blobs_dir: &Path) -> Option<String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.as_bytes())
        .ok()?;
    if bytes.is_empty() {
        return None;
    }
    match blob::store_bytes(blobs_dir, &bytes) {
        Ok(digest) => Some(digest),
        Err(error) => {
            tracing::warn!(%error, "storing a snapshot image as a blob failed; keeping it inline");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(content: Value) -> Value {
        serde_json::json!({
            "id": "e1", "parentId": null, "type": "message",
            "message": {"role": "toolResult", "content": content, "timestamp": 0}
        })
    }

    #[test]
    fn small_entries_pass_through_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let mut entries = vec![message(Value::String("small".into()))];
        assert_eq!(bound_entries(&mut entries, dir.path()), 0);
        assert_eq!(entries[0]["message"]["content"], "small");
        assert!(entries[0]["message"].get("truncated").is_none());
    }

    #[test]
    fn huge_tool_text_is_cut_and_flagged() {
        let dir = tempfile::tempdir().unwrap();
        let mut entries = vec![message(Value::String("x".repeat(5 * 1024 * 1024)))];
        assert_eq!(bound_entries(&mut entries, dir.path()), 1);
        let text = entries[0]["message"]["content"].as_str().unwrap();
        assert!(text.len() <= ENTRY_TEXT_BUDGET + TRUNCATION_NOTE.len());
        assert!(text.contains("truncated"));
        assert_eq!(entries[0]["message"]["truncated"], Value::Bool(true));
    }

    #[test]
    fn an_entrys_blocks_share_the_budget() {
        let dir = tempfile::tempdir().unwrap();
        let blocks: Vec<Value> = (0..8)
            .map(|_| serde_json::json!({"type": "text", "text": "z".repeat(ENTRY_TEXT_BUDGET)}))
            .collect();
        let mut entries = vec![message(Value::Array(blocks))];
        let _ = bound_entries(&mut entries, dir.path());
        let blocks = entries[0]["message"]["content"].as_array().unwrap();
        let total: usize = blocks
            .iter()
            .map(|b| b["text"].as_str().map(str::len).unwrap_or(0))
            .sum();
        assert!(
            total <= ENTRY_TEXT_BUDGET + TRUNCATION_NOTE.len(),
            "blocks share the per-entry budget, got {total}"
        );
    }

    #[test]
    fn a_thousand_huge_entries_still_fit_the_total_budget() {
        let dir = tempfile::tempdir().unwrap();
        let mut entries: Vec<Value> = (0..1000)
            .map(|i| {
                message(Value::String(format!(
                    "{i}:{}",
                    "y".repeat(ENTRY_TEXT_BUDGET)
                )))
            })
            .collect();
        let _ = bound_entries(&mut entries, dir.path());
        assert!(
            serialized_len(&entries) <= ENTRIES_BUDGET + TRUNCATION_NOTE.len() + 1024,
            "entries still too large: {}",
            serialized_len(&entries)
        );
        assert!(entries
            .iter()
            .all(|entry| entry["message"]["truncated"] == Value::Bool(true)));
    }

    #[test]
    fn big_inline_images_become_blob_references() {
        let dir = tempfile::tempdir().unwrap();
        let blobs = dir.path().join("blobs");
        // The store hashes bytes; any non-empty payload works as an "image".
        let payload = base64::engine::general_purpose::STANDARD
            .encode([0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a].repeat(3_000));
        let mut entries = vec![message(Value::Array(vec![serde_json::json!({
            "type": "image", "data": payload, "mimeType": "image/png",
        })]))];
        assert_eq!(bound_entries(&mut entries, &blobs), 1);
        let block = &entries[0]["message"]["content"][0];
        assert!(block.get("data").is_none(), "the inline data is gone");
        let reference = block["mediaRef"].as_str().unwrap();
        assert_eq!(reference.len(), 64, "sha256 hex");
        assert!(blobs.join(reference).is_file());
        assert_eq!(block["mimeType"], "image/png");
        assert_eq!(block["bytes"], Value::from(24_000));
    }

    #[test]
    fn invalid_base64_stays_inline_and_unflagged() {
        let dir = tempfile::tempdir().unwrap();
        let mut entries = vec![message(Value::Array(vec![serde_json::json!({
            "type": "image", "data": "!!not base64!!".repeat(2_000), "mimeType": "image/png",
        })]))];
        assert_eq!(bound_entries(&mut entries, dir.path()), 0);
        assert!(
            entries[0]["message"]["content"][0]["data"]
                .as_str()
                .unwrap()
                .len()
                > INLINE_IMAGE_LIMIT
        );
    }

    #[test]
    fn truncation_cuts_on_char_boundaries() {
        let mut text = "ä".repeat(ENTRY_TEXT_BUDGET + 10);
        let mut remaining = ENTRY_TEXT_BUDGET;
        assert!(truncate_string(&mut text, &mut remaining));
        assert!(text.is_char_boundary(text.len()));
        let mut remaining = 3;
        let mut small = "abc".to_owned();
        assert!(!truncate_string(&mut small, &mut remaining));
        assert_eq!(remaining, 0);
    }
}
