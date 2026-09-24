//! `oino.vcc` — deterministic session compaction + recall.
//!
//! `compact_branch`/`recall` are the same pure logic the native `vcc.rs`
//! module ran; entries arrive as JSON through `handle_event` /
//! `host.session_entries`.

mod types;
pub mod vcc;

use oino_extension_guest::{
    export_extension,
    ext::oino::extension::types::{ContentBlock, ToolDeclaration},
    host, Extension, Registration, ToolOutput,
};
use serde_json::{json, Value};

struct Vcc;

fn parse_entries(value: &Value) -> Result<Vec<types::SessionEntry>, String> {
    serde_json::from_value(value.clone()).map_err(|err| format!("entries: {err}"))
}

/// `/compact vcc` — the host sends `{branch: [SessionEntry]}` and appends the
/// returned compaction itself (`session_before_compact`).
fn compact_event(payload: &Value) -> Result<Value, String> {
    let branch = parse_entries(&payload["branch"])?;
    let Some(compaction) = vcc::compact_branch(&branch) else {
        return Err(
            "Nothing to compact yet; VCC needs at least one earlier entry before the latest user message"
                .into(),
        );
    };
    Ok(json!({
        "summary": compaction.summary,
        "first_kept": compaction.first_kept,
        "tokens_before": compaction.tokens_before,
        "compacted_entries": compaction.compacted_entries,
        "kept_entries": compaction.kept_entries,
    }))
}

fn recall_with_options(options: vcc::VccRecallOptions) -> Result<vcc::VccRecallResult, String> {
    let branch = parse_entries(&host::session_entries(false)?)?;
    let all = if options.scope_all {
        parse_entries(&host::session_entries(true)?)?
    } else {
        branch.clone()
    };
    Ok(vcc::recall(&branch, &all, options))
}

impl Extension for Vcc {
    fn init(_config: Value) -> Result<Registration, String> {
        Ok(Registration {
            commands: vec![
                ("compact".into(), "Compact older session history (VCC)".into()),
                ("recall".into(), "Search raw session history".into()),
            ],
            hooks: vec!["session_before_compact".into()],
            tools: vec![ToolDeclaration {
                name: "vcc_recall".into(),
                description: "Search raw Oino session history outside the compacted model context.".into(),
                input_schema: Some(r#"{"type":"object","additionalProperties":false,"properties":{"query":{"type":"string"},"scope":{"type":"string","enum":["active","all"]},"offset":{"type":"number"},"limit":{"type":"number"},"expand":{"type":"boolean"}}}"#.to_string()),
            }],
            ..Registration::default()
        })
    }

    fn handle_event(kind: &str, payload: Value) -> Result<Value, String> {
        match kind {
            "session_before_compact" => compact_event(&payload),
            _ => Ok(Value::Null),
        }
    }

    fn run_tool(name: &str, _call_id: &str, args: Value) -> Result<ToolOutput, String> {
        if name != "vcc_recall" {
            return Err(format!("unknown tool `{name}`"));
        }
        let options = vcc::VccRecallOptions {
            query: args["query"].as_str().map(str::to_string),
            scope_all: args["scope"].as_str() == Some("all"),
            offset: args["offset"].as_u64().unwrap_or(0) as usize,
            limit: args["limit"].as_u64().map(|v| v as usize).unwrap_or(5),
            expand: args["expand"].as_bool().unwrap_or(false),
        };
        let result = recall_with_options(options)?;
        Ok(ToolOutput {
            is_error: false,
            content: vec![ContentBlock::Text(result.output)],
            details: Some(json!({
                "total": result.total,
                "offset": result.offset,
                "limit": result.limit
            })),
            ..ToolOutput::default()
        })
    }

    fn run_command(name: &str, args: &str) -> Result<Value, String> {
        match name {
            "compact" => Err("VCC compaction runs through `/compact`; no extra args needed".into()),
            "recall" => {
                // `/recall [query]` — host decides whether to inject context;
                // returning the recall output keeps the printed result the same.
                let query = args.trim();
                let result = recall_with_options(vcc::VccRecallOptions {
                    query: (!query.is_empty()).then(|| query.to_string()),
                    ..Default::default()
                })?;
                Ok(json!({
                    "message": result.output,
                    "append_to_context": true
                }))
            }
            _ => Err(format!("unknown command `{name}`")),
        }
    }
}

export_extension!(Vcc);
