//! UNI-260 contract fixtures: same file as the vitest suite
//! (`packages/protocol/tests/chat.test.ts`).
use serde_json::Value;
use unipi_proto::{ChatCloseData, ChatConfigData, ChatNewData, Control};

#[derive(Debug, serde::Deserialize)]
struct Fixture {
    name: String,
    schema: String,
    value: Value,
    ok: bool,
}

fn parse<T: serde::de::DeserializeOwned>(value: Value) -> Result<(), String> {
    serde_json::from_value::<T>(value).map(|_| ()).map_err(|e| e.to_string())
}

#[test]
fn shared_chat_fixtures_agree() {
    let fixtures: Vec<Fixture> = serde_json::from_str(include_str!("../../../apps/unipi/protocol/fixtures/chat.json")).unwrap();
    assert!(fixtures.len() >= 20);
    for fixture in fixtures {
        let result: Result<(), String> = match fixture.schema.as_str() {
            "Control" => serde_json::from_value::<Control>(fixture.value.clone())
                .map_err(|e| e.to_string())
                .and_then(|control| control.validate().map_err(|e| e.to_string())),
            "ChatNewData" => parse::<ChatNewData>(fixture.value.clone()),
            "ChatCloseData" => parse::<ChatCloseData>(fixture.value.clone()),
            "ChatConfigData" => {
                // Rust: launcher is a plain String; the app requires it non-empty.
                parse::<ChatConfigData>(fixture.value.clone()).and_then(|_| {
                    if fixture.value["launcher"].as_str().is_some_and(|s| s.is_empty()) {
                        Err("empty launcher".into())
                    } else {
                        Ok(())
                    }
                })
            }
            other => panic!("unknown schema {other}"),
        };
        assert_eq!(result.is_ok(), fixture.ok, "fixture `{}`: {result:?}", fixture.name);
    }
}
