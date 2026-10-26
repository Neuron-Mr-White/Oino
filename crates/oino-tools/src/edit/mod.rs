//! `edit` (pi `tools/edit.ts` + `tools/edit-diff.ts`).

mod apply;
mod diff;

use futures::FutureExt;
use futures::future::BoxFuture;
use oino_agent::{
    AgentTool, AgentToolResult, ToolDefinitionMeta, ToolError, ToolInvocation, UpdateSink,
};
use oino_ai::{JsonObject, JsonValue};
use serde_json::json;
use tokio_util::sync::CancellationToken;

pub use apply::{Edit, EditError};

use crate::config::ToolConfig;
use crate::env::AccessMode;
use crate::info::edit_info;
use crate::path_utils::resolve_to_cwd;
use crate::{aborted, args};

const BOM: char = '\u{feff}';

/// The `edit` tool.
#[derive(Debug)]
pub struct EditTool {
    meta: ToolDefinitionMeta,
    config: ToolConfig,
}

impl EditTool {
    /// An `edit` tool.
    #[must_use]
    pub fn new(config: ToolConfig) -> Self {
        Self {
            meta: edit_info().definition_meta(),
            config,
        }
    }
}

fn is_single_edit(value: &JsonValue) -> bool {
    value.as_object().is_some_and(|edit| {
        edit.get("oldText").is_some_and(JsonValue::is_string)
            && edit.get("newText").is_some_and(JsonValue::is_string)
    })
}

/// pi `prepareEditArguments`: accepts `edits` as a JSON string or a single object, and legacy
/// top-level `oldText`/`newText` (appended as the last edit).
#[must_use]
pub fn prepare_edit_arguments(mut args: JsonObject) -> JsonObject {
    match args.get("edits") {
        Some(JsonValue::String(text)) => {
            if let Ok(parsed) = serde_json::from_str::<JsonValue>(text) {
                if parsed.is_array() {
                    args.insert("edits".to_owned(), parsed);
                } else if is_single_edit(&parsed) {
                    args.insert("edits".to_owned(), JsonValue::Array(vec![parsed]));
                }
            }
        }
        Some(single) if is_single_edit(single) => {
            let single = single.clone();
            args.insert("edits".to_owned(), JsonValue::Array(vec![single]));
        }
        _ => {}
    }
    let (Some(JsonValue::String(old_text)), Some(JsonValue::String(new_text))) =
        (args.get("oldText"), args.get("newText"))
    else {
        return args;
    };
    let legacy = json!({"oldText": old_text, "newText": new_text});
    let mut edits = match args.get("edits") {
        Some(JsonValue::Array(edits)) => edits.clone(),
        _ => Vec::new(),
    };
    edits.push(legacy);
    args.shift_remove("oldText");
    args.shift_remove("newText");
    args.insert("edits".to_owned(), JsonValue::Array(edits));
    args
}

impl AgentTool for EditTool {
    fn meta(&self) -> &ToolDefinitionMeta {
        &self.meta
    }

    fn prepare_arguments(&self, raw: JsonObject) -> JsonObject {
        prepare_edit_arguments(raw)
    }

    fn execute(
        &self,
        call: ToolInvocation,
        _updates: UpdateSink,
        cancel: CancellationToken,
    ) -> BoxFuture<'static, Result<AgentToolResult, ToolError>> {
        let config = self.config.clone();
        async move { run(&config, &call, &cancel).await }.boxed()
    }
}

fn parse_edits(args: &JsonObject) -> Result<Vec<Edit>, ToolError> {
    let invalid = || {
        ToolError::new("Edit tool input is invalid. edits must contain at least one replacement.")
    };
    let edits = args
        .get("edits")
        .and_then(JsonValue::as_array)
        .ok_or_else(invalid)?;
    if edits.is_empty() {
        return Err(invalid());
    }
    edits
        .iter()
        .map(|edit| {
            let text = |name: &str| {
                edit.get(name)
                    .and_then(JsonValue::as_str)
                    .map(str::to_owned)
                    .ok_or_else(|| ToolError::new(format!("edits[].{name} must be a string")))
            };
            Ok(Edit {
                old_text: text("oldText")?,
                new_text: text("newText")?,
            })
        })
        .collect()
}

async fn run(
    config: &ToolConfig,
    call: &ToolInvocation,
    cancel: &CancellationToken,
) -> Result<AgentToolResult, ToolError> {
    let edits = parse_edits(&call.args)?;
    let path = args::required_str(&call.args, "path")?;
    let absolute = resolve_to_cwd(path, &config.effective_cwd())
        .map_err(|error| ToolError::new(error.to_string()))?;
    let env = config.env.as_ref();
    let check = || {
        if cancel.is_cancelled() {
            Err(aborted())
        } else {
            Ok(())
        }
    };
    config
        .mutation_queue
        .run(env, &absolute, async {
            check()?;
            if let Err(error) = env.access(&absolute, AccessMode::ReadWrite).await {
                check()?;
                return Err(ToolError::new(format!(
                    "Could not edit file: {path}. Error code: {}.",
                    error.code
                )));
            }
            check()?;
            let buffer = env
                .read_file(&absolute)
                .await
                .map_err(|error| ToolError::new(error.to_string()))?;
            let raw = String::from_utf8_lossy(&buffer);
            check()?;
            let (bom, content) = match raw.strip_prefix(BOM) {
                Some(rest) => ("\u{feff}", rest),
                None => ("", raw.as_ref()),
            };
            let ending = apply::detect_line_ending(content);
            let normalized = apply::normalize_to_lf(content);
            let applied = apply::apply_edits_to_normalized_content(&normalized, &edits, path)
                .map_err(|error| ToolError::new(error.to_string()))?;
            check()?;
            let final_content = format!(
                "{bom}{}",
                apply::restore_line_endings(&applied.new_content, ending)
            );
            env.write_file(&absolute, final_content.as_bytes())
                .await
                .map_err(|error| ToolError::new(error.to_string()))?;
            check()?;
            let display = diff::display_diff(&applied.base_content, &applied.new_content);
            let unified = diff::unified_patch(path, &applied.base_content, &applied.new_content);
            let mut details = json!({"diff": display.diff, "patch": unified});
            if let (Some(line), Some(object)) =
                (display.first_changed_line, details.as_object_mut())
            {
                object.insert("firstChangedLine".to_owned(), json!(line));
            }
            Ok(AgentToolResult {
                details: Some(details),
                ..AgentToolResult::text(format!(
                    "Successfully replaced {} block(s) in {path}.",
                    edits.len()
                ))
            })
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(value: &JsonValue) -> JsonObject {
        value.as_object().cloned().unwrap()
    }

    #[test]
    fn prepare_parses_json_string_edits() {
        let prepared = prepare_edit_arguments(object(&json!({
            "path": "a",
            "edits": "[{\"oldText\":\"x\",\"newText\":\"y\"}]"
        })));
        assert_eq!(
            JsonValue::Object(prepared),
            json!({"path": "a", "edits": [{"oldText": "x", "newText": "y"}]})
        );
    }

    #[test]
    fn prepare_appends_legacy_fields() {
        let prepared = prepare_edit_arguments(object(&json!({
            "path": "a", "oldText": "x", "newText": "y"
        })));
        assert_eq!(
            JsonValue::Object(prepared),
            json!({"path": "a", "edits": [{"oldText": "x", "newText": "y"}]})
        );
    }

    /// jsdiff / `generateDiffString` / `normalizeForFuzzyMatch` parity on 154 random edits.
    #[test]
    fn diffs_and_fuzzy_match_pi() {
        let fixture: JsonValue =
            serde_json::from_str(include_str!("../../tests/fixtures/units.json")).unwrap();
        for case in fixture["diffs"].as_array().unwrap() {
            let (old, new) = (case["old"].as_str().unwrap(), case["new"].as_str().unwrap());
            let display = diff::display_diff(old, new);
            assert_eq!(display.diff, case["display"]["diff"], "{case}");
            assert_eq!(
                display.first_changed_line.map(|line| json!(line)),
                case["display"].get("firstChangedLine").cloned(),
                "{case}"
            );
            assert_eq!(
                diff::unified_patch("f.txt", old, new),
                case["patch"],
                "{case}"
            );
        }
        for case in fixture["fuzzy"].as_array().unwrap() {
            assert_eq!(
                apply::normalize_for_fuzzy_match(case["text"].as_str().unwrap()),
                case["normalized"],
                "{case}"
            );
        }
    }

    #[test]
    fn prepare_wraps_single_object() {
        let prepared = prepare_edit_arguments(object(&json!({
            "path": "a", "edits": {"oldText": "x", "newText": "y"}
        })));
        assert_eq!(
            JsonValue::Object(prepared),
            json!({"path": "a", "edits": [{"oldText": "x", "newText": "y"}]})
        );
    }
}
