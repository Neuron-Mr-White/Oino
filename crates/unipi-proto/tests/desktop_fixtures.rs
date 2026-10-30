//! UNI-216 contract fixtures: same file as the vitest suite
//! (`packages/protocol/tests/desktop.test.ts`).
use serde_json::Value;
use unipi_proto::{Control, DesktopStatusData};

#[derive(Debug, serde::Deserialize)]
struct Fixture {
    name: String,
    schema: String,
    value: Value,
    ok: bool,
}

#[test]
fn shared_desktop_fixtures_agree() {
    let fixtures: Vec<Fixture> =
        serde_json::from_str(include_str!("../../../apps/unipi/protocol/fixtures/desktop.json")).unwrap();
    assert!(fixtures.len() >= 30);
    for fixture in fixtures {
        let result: Result<(), String> = match fixture.schema.as_str() {
            "Control" => serde_json::from_value::<Control>(fixture.value.clone())
                .map_err(|e| e.to_string())
                .and_then(|control| {
                    control.validate().map_err(|e| e.to_string())?;
                    match control {
                        Control::DesktopCmd { cmd } => cmd.check(),
                        _ => Ok(()),
                    }
                }),
            "DesktopStatusData" => serde_json::from_value::<DesktopStatusData>(fixture.value.clone())
                .map(|_| ())
                .map_err(|e| e.to_string()),
            other => panic!("unknown schema {other}"),
        };
        assert_eq!(result.is_ok(), fixture.ok, "fixture `{}`: {result:?}", fixture.name);
    }
}
