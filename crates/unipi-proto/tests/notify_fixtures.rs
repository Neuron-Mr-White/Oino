//! UNI-161 \u00a71/\u00a72 contract fixtures: same file as the vitest suite
//! (`notify_config`/`notify_register`/`notify_test` requests, and
//! `NotifyConfigData` responses).
use serde_json::Value;
use unipi_proto::{Control, NotifyConfigData};

#[derive(Debug, serde::Deserialize)]
#[allow(dead_code)]
struct Fixture {
    name: String,
    schema: String,
    value: Value,
    ok: bool,
}

#[test]
fn shared_notify_fixtures_agree() {
    let fixtures: Vec<Fixture> = serde_json::from_str(include_str!(
        "../../../apps/unipi/protocol/fixtures/notify.json"
    ))
    .unwrap();
    assert!(fixtures.len() >= 8);
    for fixture in fixtures {
        let result: Result<(), String> = match fixture.schema.as_str() {
            "Control" => serde_json::from_value::<Control>(fixture.value.clone())
                .map(|_| ())
                .map_err(|e| e.to_string()),
            "NotifyConfigData" => serde_json::from_value::<NotifyConfigData>(fixture.value.clone())
                .map(|_| ())
                .map_err(|e| e.to_string()),
            other => panic!("unknown schema {other}"),
        };
        assert_eq!(result.is_ok(), fixture.ok, "fixture `{}`", fixture.name);
    }
}
