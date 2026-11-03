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
    bound_entries_with(entries, ENTRY_TEXT_BUDGET, ENTRIES_BUDGET, blobs_dir)
}

/// [`bound_entries`] with explicit budgets: `entry_text` bytes per entry to start with,
/// halved while the array exceeds `total`.
#[must_use]
pub fn bound_entries_with(
    entries: &mut [Value],
    entry_text: usize,
    total: usize,
    blobs_dir: &Path,
) -> usize {
    let mut rewritten = rewrite_all(entries, entry_text, blobs_dir);
    // A huge session still over the total budget: shrink the per-entry text
    // budget (the app expands the entries it cares about) until the array fits.
    let mut budget = entry_text;
    while entries.len() > 1 && budget > MIN_ENTRY_TEXT && serialized_len(entries) > total {
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
            if clip_strings(entry, text_budget) {
                if let Some(object) = entry.as_object_mut() {
                    object.insert("truncated".into(), Value::Bool(true));
                }
                rewritten += 1;
            }
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
        // Whatever else is huge (thinking, tool-call arguments, `details`, summaries,
        // custom payloads) is clipped too: no single string may blow the frame.
        changed |= message.iter_mut().fold(false, |cut, (_, value)| {
            clip_strings(value, text_budget) | cut
        });
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

/// Clips every string longer than `max` anywhere in `value` (image `data` and
/// `mediaRef` excepted: cutting base64 breaks the image). Returns whether any was cut.
fn clip_strings(value: &mut Value, max: usize) -> bool {
    match value {
        Value::String(text) if text.len() > max => {
            let mut remaining = max;
            truncate_string(text, &mut remaining)
        }
        Value::Array(items) => items
            .iter_mut()
            .fold(false, |changed, item| clip_strings(item, max) | changed),
        Value::Object(map) => map
            .iter_mut()
            .filter(|(key, _)| key.as_str() != "data" && key.as_str() != "mediaRef")
            .fold(false, |changed, (_, item)| {
                clip_strings(item, max) | changed
            }),
        _ => false,
    }
}

/// One live session event's serialized budget (fix-frames P0: a browser screenshot or a
/// multi-megabyte tool output in a `message_end` / `tool_execution_*` event went out as one
/// oversize frame, the app's reader rejected it with "oversize frame from host" and the chat
/// stopped updating). [`bound_event`] keeps every published event under it.
pub const EVENT_BUDGET: usize = 256 * 1024;
/// One `events` frame's budget: a flush splits its batch into frames no larger than this
/// (a frame always carries at least one event, itself ≤ [`EVENT_BUDGET`]).
pub const BATCH_BUDGET: usize = 512 * 1024;
/// The longest string a live event keeps before the shrink loop starts halving.
const EVENT_TEXT_START: usize = 64 * 1024;

/// Exact serialized size of `value`, without building the bytes.
#[must_use]
pub fn json_len(value: &Value) -> usize {
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0 += buf.len();
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0);
    let _ = serde_json::to_writer(&mut count, value);
    count.0
}

/// Bounds one live session event (a `ClientEvent` JSON value) to `budget` serialized bytes,
/// in place. Small events (the common case) are left untouched. A bigger one:
///
/// 1. has every inline base64 image over [`INLINE_IMAGE_LIMIT`] — anywhere in the event:
///    `message.content`, a tool `result` / `partialResult`, `agent_end.messages` — moved into
///    the blob store as a `mediaRef` (when `blobs_dir` is known);
/// 2. then has its long strings clipped (64 KiB, halving) until it fits;
/// 3. and if it still does not fit, is replaced by a stub: its identity scalars (`type`,
///    `toolCallId`, `message.role`, `message.timestamp`, …) and nothing else.
///
/// A changed event carries `truncated: true` on the event, its `message` and its `result`
/// (the flag the app's full-output path reads, refetching the entry on expand). Returns
/// whether the event was changed.
pub fn bound_event(event: &mut Value, blobs_dir: Option<&Path>, budget: usize) -> bool {
    bound_event_sized(event, blobs_dir, budget).0
}

/// [`bound_event`], also returning the event's serialized size afterwards (measured once for
/// the common small event, so the feed ring can account bytes without a second pass).
pub fn bound_event_sized(
    event: &mut Value,
    blobs_dir: Option<&Path>,
    budget: usize,
) -> (bool, usize) {
    let len = json_len(event);
    if len <= budget {
        return (false, len);
    }
    let mut changed = false;
    if let Some(dir) = blobs_dir {
        changed |= reference_images(event, dir);
    }
    let mut max = EVENT_TEXT_START;
    while json_len(event) > budget && max >= MIN_ENTRY_TEXT {
        changed |= clip_strings(event, max);
        max /= 2;
    }
    if json_len(event) > budget {
        *event = stub(event, 0);
        changed = true;
    }
    if changed {
        mark_truncated(event);
    }
    (changed, json_len(event))
}

/// Moves every big inline base64 image in `value` into the blob store (`mediaRef`).
fn reference_images(value: &mut Value, blobs_dir: &Path) -> bool {
    match value {
        Value::Array(items) => items.iter_mut().fold(false, |changed, item| {
            reference_images(item, blobs_dir) | changed
        }),
        Value::Object(map) => {
            if map.get("type").and_then(Value::as_str) == Some("image") {
                let mut image = Value::Object(std::mem::take(map));
                let mut unlimited = usize::MAX;
                let changed = bound_block(&mut image, &mut unlimited, blobs_dir);
                if let Value::Object(back) = image {
                    *map = back;
                }
                return changed;
            }
            map.iter_mut().fold(false, |changed, (_, item)| {
                reference_images(item, blobs_dir) | changed
            })
        }
        _ => false,
    }
}

/// The last-resort form of an event that cannot be bounded: scalars kept (strings clipped),
/// objects two levels deep, arrays emptied.
fn stub(value: &Value, depth: usize) -> Value {
    match value {
        Value::String(text) => {
            let mut text = text.clone();
            let mut remaining = MIN_ENTRY_TEXT;
            let _ = truncate_string(&mut text, &mut remaining);
            Value::String(text)
        }
        Value::Array(_) => Value::Array(Vec::new()),
        Value::Object(map) if depth < 2 => Value::Object(
            map.iter()
                .filter(|(key, item)| key.as_str() != "data" && (depth == 0 || !item.is_object()))
                .map(|(key, item)| (key.clone(), stub(item, depth + 1)))
                .collect(),
        ),
        Value::Object(_) => Value::Object(serde_json::Map::new()),
        other => other.clone(),
    }
}

/// Flags a bounded event for the app (`truncated: true` on the event, `message`, `result`).
fn mark_truncated(event: &mut Value) {
    let Some(object) = event.as_object_mut() else {
        return;
    };
    object.insert("truncated".into(), Value::Bool(true));
    for key in ["message", "result", "partialResult"] {
        if let Some(inner) = object.get_mut(key).and_then(Value::as_object_mut) {
            inner.insert("truncated".into(), Value::Bool(true));
        }
    }
}

/// Drops the oldest entries until the array fits `budget` serialized bytes (one entry
/// always stays). Returns how many were dropped: the caller's page then starts later
/// (its cursor moves to the new first entry and older history pages in on scroll).
#[must_use]
pub fn fit_entries(entries: &mut Vec<Value>, budget: usize) -> usize {
    let sizes: Vec<usize> = entries
        .iter()
        .map(|entry| serde_json::to_vec(entry).map_or(0, |bytes| bytes.len()))
        .collect();
    let mut total: usize = sizes.iter().sum();
    let mut drop = 0;
    while drop + 1 < entries.len() && total > budget {
        total -= sizes[drop];
        drop += 1;
    }
    entries.drain(..drop);
    drop
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
    fn huge_strings_outside_the_text_blocks_are_clipped() {
        let dir = tempfile::tempdir().unwrap();
        let big = "w".repeat(3 * 1024 * 1024);
        let mut entries = vec![
            serde_json::json!({
                "id": "e1", "type": "message",
                "message": {"role": "assistant", "content": [
                    {"type": "thinking", "thinking": big},
                    {"type": "toolCall", "id": "t", "name": "write", "arguments": {"content": big}},
                ]}
            }),
            serde_json::json!({"id": "e2", "type": "custom_message", "content": big}),
        ];
        assert_eq!(bound_entries(&mut entries, dir.path()), 2);
        assert!(
            serialized_len(&entries) < 64 * 1024,
            "{}",
            serialized_len(&entries)
        );
        assert_eq!(entries[0]["message"]["truncated"], Value::Bool(true));
        assert_eq!(entries[1]["truncated"], Value::Bool(true));
    }

    #[test]
    fn fit_drops_the_oldest_until_the_page_fits() {
        let mut entries: Vec<Value> = (0..10)
            .map(|i| serde_json::json!({"id": format!("e{i}"), "pad": "p".repeat(1000)}))
            .collect();
        assert_eq!(fit_entries(&mut entries, 3500), 7);
        assert_eq!(entries[0]["id"], "e7");
        let mut one = vec![serde_json::json!({"id": "x", "pad": "p".repeat(5000)})];
        assert_eq!(fit_entries(&mut one, 10), 0, "one entry always stays");
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

    fn screenshot_b64(bytes: usize) -> String {
        let raw: Vec<u8> = (0..bytes).map(|i| (i * 31 % 251) as u8).collect();
        base64::engine::general_purpose::STANDARD.encode(raw)
    }

    #[test]
    fn small_events_pass_untouched() {
        let mut event = serde_json::json!({"type": "tool_execution_end", "toolCallId": "t", "result": {"content": [{"type": "text", "text": "ok"}]}});
        let before = event.clone();
        assert!(!bound_event(&mut event, None, EVENT_BUDGET));
        assert_eq!(event, before);
    }

    #[test]
    fn a_live_tool_end_with_five_megabytes_is_clipped_and_flagged() {
        let mut event = serde_json::json!({
            "type": "tool_execution_end", "toolCallId": "t1", "toolName": "bash", "isError": false,
            "result": {"content": [{"type": "text", "text": "x".repeat(5 * 1024 * 1024)}], "details": {}}
        });
        assert!(bound_event(&mut event, None, EVENT_BUDGET));
        assert!(json_len(&event) <= EVENT_BUDGET);
        assert_eq!(event["truncated"], Value::Bool(true));
        assert_eq!(event["result"]["truncated"], Value::Bool(true));
        assert_eq!(event["toolCallId"], "t1");
        assert!(event["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("truncated"));
    }

    #[test]
    fn a_live_screenshot_anywhere_becomes_a_media_ref() {
        let dir = tempfile::tempdir().unwrap();
        let data = screenshot_b64(3 * 1024 * 1024 / 4 * 3);
        let mut event = serde_json::json!({
            "type": "message_end",
            "message": {"role": "toolResult", "toolCallId": "b1", "content": [
                {"type": "text", "text": "navigated"},
                {"type": "image", "data": data, "mimeType": "image/png"}
            ]}
        });
        assert!(bound_event(&mut event, Some(dir.path()), EVENT_BUDGET));
        assert!(json_len(&event) <= EVENT_BUDGET);
        let image = &event["message"]["content"][1];
        assert!(image.get("data").is_none());
        assert!(dir
            .path()
            .join(image["mediaRef"].as_str().unwrap())
            .is_file());
        assert_eq!(event["message"]["content"][0]["text"], "navigated");
    }

    #[test]
    fn an_unboundable_event_becomes_a_stub() {
        // No blob store and a 3 MB image: clipping never touches `data`, so it is stubbed.
        let mut event = serde_json::json!({
            "type": "tool_execution_update", "toolCallId": "b2",
            "partialResult": {"content": [{"type": "image", "data": screenshot_b64(2_300_000)}]},
            "many": (0..20_000).map(Value::from).collect::<Vec<_>>()
        });
        assert!(bound_event(&mut event, None, EVENT_BUDGET));
        assert!(json_len(&event) <= 4096, "{}", json_len(&event));
        assert_eq!(event["type"], "tool_execution_update");
        assert_eq!(event["toolCallId"], "b2");
        assert_eq!(event["truncated"], Value::Bool(true));
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
