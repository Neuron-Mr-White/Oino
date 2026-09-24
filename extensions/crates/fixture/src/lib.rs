//! Test fixture component for `oino-wasm-host` acceptance tests.

use oino_extension_guest::{
    export_extension,
    ext::oino::extension::types::{ContentBlock, ToolDeclaration},
    host, http, Extension, Registration, ToolOutput,
};
use serde_json::{json, Value};
use std::sync::Mutex;

struct Fixture;

static KV_COMMAND_RESULT: Mutex<Vec<String>> = Mutex::new(Vec::new());

impl Extension for Fixture {
    fn init(_config: Value) -> Result<Registration, String> {
        Ok(Registration {
            tools: vec![ToolDeclaration {
                name: "echo".into(),
                description: "Echo tool args back".into(),
                input_schema: Some(r#"{"type":"object"}"#.into()),
            }],
            commands: vec![
                ("hello".into(), "Greet".into()),
                ("kv-set".into(), "Store a kv value".into()),
                ("kv-get".into(), "Read a kv value".into()),
                ("spin".into(), "Infinite loop".into()),
                ("hog".into(), "Allocate until OOM".into()),
                ("http-get".into(), "GET a url".into()),
                ("probe".into(), "Stepwise http probe".into()),
                ("probe2".into(), "get+read_chunk probe".into()),
                ("read-file".into(), "Read a file".into()),
                ("exec".into(), "Run a program".into()),
                ("secret".into(), "Read a secret".into()),
            ],
            hooks: vec!["input".into()],
            providers: Vec::new(),
        })
    }

    fn handle_event(kind: &str, payload: Value) -> Result<Value, String> {
        if kind == "input" {
            let text = payload["text"].as_str().unwrap_or_default();
            Ok(json!({ "payload": text.to_uppercase() }))
        } else {
            Ok(Value::Null)
        }
    }

    fn run_tool(name: &str, call_id: &str, args: Value) -> Result<ToolOutput, String> {
        if name != "echo" {
            return Err(format!("unknown tool `{name}`"));
        }
        host::tool_progress(call_id, "echo started", None);
        host::tool_progress(call_id, "echo done", Some(json!({"stage": 2})));
        Ok(ToolOutput {
            content: vec![ContentBlock::Text(
                serde_json::to_string(&args).unwrap_or_default(),
            )],
            ..Default::default()
        })
    }

    fn run_command(name: &str, args: &str) -> Result<Value, String> {
        match name {
            "hello" => Ok(json!({ "greeting": format!("hello {args}") })),
            "kv-set" => {
                let parts: Vec<&str> = args.splitn(2, ' ').collect();
                let (key, value) = match parts.as_slice() {
                    [key, value] => (*key, *value),
                    _ => return Err("usage: kv-set <key> <value>".into()),
                };
                host::kv_set(key, value.as_bytes())?;
                if let Ok(mut list) = KV_COMMAND_RESULT.lock() {
                    list.push(key.to_string());
                }
                Ok(json!({ "ok": true }))
            }
            "kv-get" => {
                let value = host::kv_get(args.trim())
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
                Ok(json!({ "value": value }))
            }
                        "spin" => {
                #[allow(clippy::empty_loop)]
                loop {}
            }
            "hog" => {
                let mut chunks: Vec<Vec<u8>> = Vec::new();
                loop {
                    chunks.push(vec![7u8; 8 * 1024 * 1024]);
                }
            }
            "probe" => Ok(json!({ "result": http::probe(args.trim()) })),
            "probe2" => {
                let mut response = http::get(args.trim()).map_err(|e| e.to_string())?;
                let mut chunks = 0usize;
                while response.read_chunk().is_some() {
                    chunks += 1;
                }
                Ok(json!({ "status": response.status, "chunks": chunks }))
            }
            "http-get" => {
                let mut response = http::get(args.trim())?;
                let mut body = Vec::new();
                while let Some(chunk) = response.read_chunk() {
                    body.extend_from_slice(&chunk);
                }
                Ok(json!({
                    "status": response.status,
                    "body": String::from_utf8_lossy(&body),
                }))
            }
            "read-file" => {
                let text =
                    std::fs::read_to_string(args.trim()).map_err(|err| format!("read: {err}"))?;
                Ok(json!({ "text": text }))
            }
            "exec" => {
                let parts: Vec<&str> = args.split_whitespace().collect();
                let Some(program) = parts.first() else {
                    return Err("usage: exec <prog>".into());
                };
                let output = host::exec(
                    program,
                    &parts[1..].iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                    None,
                )?;
                Ok(json!({
                    "status": output.status,
                    "stdout": String::from_utf8_lossy(&output.stdout),
                }))
            }
            "secret" => Ok(json!({ "value": host::secret(args.trim()) })),
            _ => Err(format!("unknown command `{name}`")),
        }
    }
}

export_extension!(Fixture);
