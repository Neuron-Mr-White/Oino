//! UNI-232 contract fixtures: same file as `packages/protocol/tests/admin.test.ts`.
use serde_json::Value;
use unipi_proto::{Control, DevicesListData, HostInfoData, PairBeginData, PairStatusData};

#[derive(Debug, serde::Deserialize)]
struct Fixture {
    name: String,
    schema: String,
    value: Value,
    ok: bool,
}

#[test]
fn shared_admin_fixtures_agree() {
    let fixtures: Vec<Fixture> = serde_json::from_str(include_str!(
        "../../../apps/unipi/protocol/fixtures/admin.json"
    ))
    .unwrap();
    assert!(fixtures.len() >= 18);
    for f in fixtures {
        let v = f.value.clone();
        let result: Result<(), String> = match f.schema.as_str() {
            "Control" => serde_json::from_value::<Control>(v)
                .map_err(|e| e.to_string())
                .and_then(|c| c.validate().map_err(|e| e.to_string())),
            "HostInfoData" => serde_json::from_value::<HostInfoData>(v)
                .map(|d| d.mode)
                .map_err(|e| e.to_string())
                .and_then(|m| {
                    if ["service", "app", "manual"].contains(&m.as_str()) {
                        Ok(())
                    } else {
                        Err(format!("mode {m}"))
                    }
                }),
            "PairBeginData" => serde_json::from_value::<PairBeginData>(v)
                .map(|_| ())
                .map_err(|e| e.to_string()),
            "PairStatusData" => serde_json::from_value::<PairStatusData>(v)
                .map(|_| ())
                .map_err(|e| e.to_string()),
            "DevicesListData" => serde_json::from_value::<DevicesListData>(v)
                .map(|_| ())
                .map_err(|e| e.to_string()),
            other => panic!("unknown schema {other}"),
        };
        assert_eq!(result.is_ok(), f.ok, "fixture `{}`: {result:?}", f.name);
    }
}
