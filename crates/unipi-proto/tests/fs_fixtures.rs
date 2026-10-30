//! Filesystem browser/editor contract fixtures: same file as the vitest
//! suite (`packages/protocol/tests/fs.test.ts`).
use serde_json::Value;
use unipi_proto::{Control, FsListData, FsReadData, FsRootsData, FsSearchData, FsWriteData};

#[derive(Debug, serde::Deserialize)]
struct Fixture {
    name: String,
    schema: String,
    value: Value,
    ok: bool,
}

#[test]
fn shared_fs_fixtures_agree() {
    let fixtures: Vec<Fixture> = serde_json::from_str(include_str!(
        "../../../apps/unipi/protocol/fixtures/fs.json"
    ))
    .unwrap();
    assert!(fixtures.len() >= 15);
    for fixture in fixtures {
        let result: Result<(), String> = match fixture.schema.as_str() {
            "Control" => serde_json::from_value::<Control>(fixture.value.clone())
                .map_err(|e| e.to_string())
                .and_then(|control| control.validate().map_err(|e| e.to_string())),
            "FsListData" => serde_json::from_value::<FsListData>(fixture.value.clone())
                .map(|_| ())
                .map_err(|e| e.to_string()),
            "FsReadData" => serde_json::from_value::<FsReadData>(fixture.value.clone())
                .map(|_| ())
                .map_err(|e| e.to_string()),
            "FsWriteData" => serde_json::from_value::<FsWriteData>(fixture.value.clone())
                .map(|_| ())
                .map_err(|e| e.to_string()),
            "FsSearchData" => serde_json::from_value::<FsSearchData>(fixture.value.clone())
                .map(|_| ())
                .map_err(|e| e.to_string()),
            "FsRootsData" => serde_json::from_value::<FsRootsData>(fixture.value.clone())
                .map(|_| ())
                .map_err(|e| e.to_string()),
            other => panic!("unknown schema {other}"),
        };
        assert_eq!(
            result.is_ok(),
            fixture.ok,
            "fixture `{}`: {result:?}",
            fixture.name
        );
    }
}
