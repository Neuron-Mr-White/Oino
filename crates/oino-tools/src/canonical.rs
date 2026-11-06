//! Shared canonical argument view for execution, presentation and permission consumers.

use oino_agent::ToolError;
use oino_ai::{JsonObject, JsonValue};

/// Copy file-tool arguments into their canonical shape without changing stored records.
/// An existing `file_path` always wins over the legacy `path`. Missing, empty or
/// non-string destinations fail closed instead of becoming the current directory.
/// Other tools are returned unchanged. Path resolution remains the caller's job.
///
/// # Errors
/// Returns an error if a read/write/edit call has no nonempty string destination.
pub fn canonical_args(name: &str, original: &JsonObject) -> Result<JsonObject, ToolError> {
    let mut args = original.clone();
    if matches!(name, "read" | "write" | "edit") {
        let legacy = args.shift_remove("path");
        if let Some(path) = legacy {
            args.entry("file_path".to_owned()).or_insert(path);
        }
        if args
            .get("file_path")
            .and_then(JsonValue::as_str)
            .is_none_or(|path| path.trim().is_empty())
        {
            return Err(ToolError::new("file_path must be a nonempty string"));
        }
    }
    Ok(args)
}
