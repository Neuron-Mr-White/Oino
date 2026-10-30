//! M2 contract fixtures: same file as the vitest suite.
use serde_json::Value;
use unipi_proto::{Control, PreviewData, TreeData};

#[derive(Debug, serde::Deserialize)]
#[allow(dead_code)]
struct Fixture {
    name: String,
    schema: String,
    value: Value,
    ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path: Option<Value>,
}

#[test]
fn shared_m2_fixtures_agree() {
    let fixtures: Vec<Fixture> =
        serde_json::from_str(include_str!("../../../apps/unipi/protocol/fixtures/m2.json")).unwrap();
    assert!(fixtures.len() >= 12);
    for fixture in fixtures {
        let result: Result<(), String> = match fixture.schema.as_str() {
            "Control" => serde_json::from_value::<Control>(fixture.value.clone())
                .map(|_| ())
                .map_err(|e| e.to_string()),
            "TreeData" => serde_json::from_value::<TreeData>(fixture.value.clone())
                .map(|_| ())
                .map_err(|e| e.to_string()),
            "PreviewData" => serde_json::from_value::<PreviewData>(fixture.value.clone())
                .map(|_| ())
                .map_err(|e| e.to_string()),
            other => panic!("unknown schema {other}"),
        };
        assert_eq!(result.is_ok(), fixture.ok, "fixture `{}`", fixture.name);
    }
}
