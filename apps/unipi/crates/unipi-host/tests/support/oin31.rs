use oino_core::session::ToolInfo;
use serde_json::{json, Value};
use std::path::Path;

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn schema(value: &Value, path: &str) {
    let Some(object) = value.as_object() else {
        assert_ne!(value, &json!(false), "false schema at {path}");
        return;
    };
    if let Some(properties) = object.get("properties") {
        let properties = properties
            .as_object()
            .unwrap_or_else(|| panic!("non-object properties at {path}"));
        for (key, child) in properties {
            schema(child, &format!("{path}.{key}"));
        }
    }
    if let Some(required) = object.get("required") {
        let props = object
            .get("properties")
            .and_then(Value::as_object)
            .expect("required without properties");
        for key in required.as_array().expect("required array") {
            assert!(
                props.contains_key(key.as_str().unwrap()),
                "{path}: missing {key}"
            );
        }
    }
    if let Some(enums) = object.get("enum") {
        assert!(!enums.as_array().expect("enum array").is_empty(), "{path}");
    }
    for key in [
        "items",
        "additionalProperties",
        "not",
        "contains",
        "propertyNames",
    ] {
        if let Some(child) = object.get(key) {
            // additionalProperties: false is a constraint, not a false subschema.
            if key != "additionalProperties" || child != &json!(false) {
                schema(child, &format!("{path}.{key}"));
            }
        }
    }
    for key in ["allOf", "anyOf", "oneOf", "prefixItems"] {
        if let Some(children) = object.get(key) {
            for child in children.as_array().expect("schema array") {
                schema(child, &format!("{path}.{key}"));
            }
        }
    }
    for key in ["$defs", "definitions", "patternProperties"] {
        if let Some(children) = object.get(key) {
            for (name, child) in children.as_object().expect("schema map") {
                schema(child, &format!("{path}.{key}.{name}"));
            }
        }
    }
}

pub fn audit_and_wire(tools: &[ToolInfo]) {
    let declarations: Vec<Value> = tools
        .iter()
        .map(|tool| {
            assert!(valid_name(&tool.name), "invalid tool: {}", tool.name);
            assert_eq!(tool.parameters["type"], "object", "{}", tool.name);
            assert!(tool.parameters["properties"].is_object(), "{}", tool.name);
            schema(&tool.parameters, &tool.name);
            json!({"name":tool.name,"description":tool.description,"parameters":tool.parameters})
        })
        .collect();
    // Env-gated regen (spec, fixtures step 1): UPDATE_SUPERAGENT_FIXTURES=1
    // rewrites the snapshots before asserting.
    let mut normalized: std::collections::BTreeMap<String, Value> = serde_json::from_str(
        include_str!(
            "../../../../../../extensions/sdk/oino-provider-kit/tests/fixtures/superagent-normalized.json"
        ),
    )
    .unwrap_or_default();
    if std::env::var_os("UPDATE_SUPERAGENT_FIXTURES").is_some() {
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../../extensions/sdk/oino-provider-kit/tests/fixtures");
        let _ = std::fs::write(
            base.join("superagent-tools.json"),
            serde_json::to_string_pretty(&declarations).unwrap_or_default(),
        );
    }
    let fixture = include_str!(
        "../../../../../../extensions/sdk/oino-provider-kit/tests/fixtures/superagent-tools.json"
    );
    assert_eq!(
        serde_json::from_str::<Value>(fixture).unwrap(),
        json!(declarations)
    );
    let history = include_str!(
        "../../../../../../extensions/sdk/oino-provider-kit/tests/fixtures/superagent-history.json"
    );
    for api in [
        "anthropic-messages",
        "openai-completions",
        "openai-responses",
    ] {
        let mut context: Value = serde_json::from_str(history).unwrap();
        context["messages"][0]["toolsAdded"] = json!(declarations);
        let model = serde_json::from_value(json!({"id":"target","name":"target","api":api,"provider":"gateway","baseUrl":"https://example.com/v1","reasoning":false,"input":["text"],"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0},"contextWindow":200_000,"maxTokens":8_192})).unwrap();
        let request = oino_agent::StreamRequest {
            model,
            context: serde_json::from_value(context).unwrap(),
            options: oino_ai::StreamOptions::default(),
            api_key: Some("test".into()),
            cancel: tokio_util::sync::CancellationToken::new(),
        };
        let actual = oino_core::provider::request_json(&request);
        if std::env::var_os("UPDATE_SUPERAGENT_FIXTURES").is_some() {
            normalized.insert(api.to_owned(), actual["context"]["messages"].clone());
            let base = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../../extensions/sdk/oino-provider-kit/tests/fixtures");
            let _ = std::fs::write(
                base.join("superagent-normalized.json"),
                serde_json::to_string_pretty(&normalized).unwrap_or_default(),
            );
        }
        assert_eq!(actual["context"]["messages"], normalized[api], "{api}");
    }
}
