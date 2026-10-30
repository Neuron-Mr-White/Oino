//! UNI-213 contract fixtures: same file as the vitest suite
//! (`packages/protocol/tests/forward.test.ts`).
use serde_json::Value;
use unipi_proto::{Control, PortsListData};

#[derive(Debug, serde::Deserialize)]
struct Fixture {
    name: String,
    schema: String,
    value: Value,
    ok: bool,
}

#[test]
fn shared_forward_fixtures_agree() {
    let fixtures: Vec<Fixture> =
        serde_json::from_str(include_str!("../../../apps/unipi/protocol/fixtures/forward.json")).unwrap();
    assert!(fixtures.len() >= 9);
    for fixture in fixtures {
        let result: Result<(), String> = match fixture.schema.as_str() {
            "Control" => serde_json::from_value::<Control>(fixture.value.clone())
                .map_err(|e| e.to_string())
                .and_then(|control| control.validate().map_err(|e| e.to_string())),
            "PortsListData" => serde_json::from_value::<PortsListData>(fixture.value.clone())
                .map(|_| ())
                .map_err(|e| e.to_string()),
            other => panic!("unknown schema {other}"),
        };
        assert_eq!(result.is_ok(), fixture.ok, "fixture `{}`: {result:?}", fixture.name);
    }
}
