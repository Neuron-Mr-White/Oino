//! Execution-only normalization. Never replace persisted tool calls with this copy.
//!
//! Core must invoke `normalize_call` before permission checks. Legacy limits are carried
//! separately because they are deliberately absent from the model-facing schemas.

use oino_agent::ToolError;
use oino_ai::{JsonObject, JsonValue};
use serde_json::json;

use crate::path_utils::resolve_to_cwd;

/// A normalized call and its compatibility-only output cap.
#[derive(Clone, Debug, PartialEq)]
pub struct NormalizedCall {
    /// Canonical execution name.
    pub name: String,
    /// Canonical arguments, copied from the stored call.
    pub args: JsonObject,
    /// Old ls/find result cap.
    pub result_limit: Option<u64>,
}

fn rename(args: &mut JsonObject, old: &str, new: &str) -> bool {
    if let Some(value) = args.shift_remove(old) {
        args.entry(new.to_owned()).or_insert(value);
        true
    } else {
        false
    }
}

/// Normalize a copy before validation and permission gating, preserving the source bytes.
///
/// # Errors
/// Returns an error if a legacy relative path cannot be resolved against cwd.
pub fn normalize_call(
    name: &str,
    original: &JsonObject,
    cwd: &str,
) -> Result<NormalizedCall, ToolError> {
    let mut args = original.clone();
    let mut canonical = name.to_owned();
    let mut result_limit = None;
    match name {
        "read" | "write" | "edit" => {
            if rename(&mut args, "path", "file_path")
                && let Some(path) = args.get("file_path").and_then(JsonValue::as_str)
            {
                let path =
                    resolve_to_cwd(path, cwd).map_err(|error| ToolError::new(error.to_string()))?;
                args.insert("file_path".to_owned(), json!(path));
            }
            if name == "edit" {
                args = crate::edit::prepare_edit_arguments(args);
            }
        }
        "ls" => {
            rename(&mut args, "path", "directory_path");
            result_limit = args.shift_remove("limit").and_then(|v| v.as_u64());
        }
        "find" => {
            "glob".clone_into(&mut canonical);
            if let Some(pattern) = args.shift_remove("pattern") {
                args.entry("patterns".to_owned())
                    .or_insert(json!([pattern]));
            }
            rename(&mut args, "path", "folder");
            result_limit = args.shift_remove("limit").and_then(|v| v.as_u64());
        }
        "grep" => {
            let mut legacy = args.contains_key("context") && !args.contains_key("output_mode");
            for (old, new) in [
                ("glob", "glob_pattern"),
                ("ignoreCase", "case_insensitive"),
                ("literal", "fixed_string"),
                ("limit", "head_limit"),
            ] {
                legacy |= rename(&mut args, old, new);
            }
            if legacy {
                args.entry("output_mode".to_owned())
                    .or_insert(json!("content"));
                args.entry("line_numbers".to_owned()).or_insert(json!(true));
            }
        }
        "bash" | "powershell" => {
            args.entry("summary".to_owned())
                .or_insert(json!("(legacy call)"));
            args.entry("riskLevel".to_owned()).or_insert(json!("high"));
        }
        _ => {}
    }
    Ok(NormalizedCall {
        name: canonical,
        args,
        result_limit,
    })
}

/// File path accepted by execution, including old direct callers.
pub(crate) fn file_path(args: &JsonObject) -> Result<&str, ToolError> {
    crate::args::optional_str(args, "file_path")
        .or_else(|| crate::args::optional_str(args, "path"))
        .ok_or_else(|| ToolError::new("file_path must be a string"))
}
