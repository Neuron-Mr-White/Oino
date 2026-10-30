//! UNI-215 contract fixtures: same file as the vitest suite
//! (`packages/protocol/tests/browser.test.ts`).
use serde_json::Value;
use unipi_proto::{BrowserOpenData, BrowserProfilesData, BrowserStatusData, Control};

#[derive(Debug, serde::Deserialize)]
struct Fixture {
    name: String,
    schema: String,
    value: Value,
    ok: bool,
}

#[test]
fn shared_browser_fixtures_agree() {
    let fixtures: Vec<Fixture> =
        serde_json::from_str(include_str!("../../../apps/unipi/protocol/fixtures/browser.json")).unwrap();
    assert!(fixtures.len() >= 30);
    for fixture in fixtures {
        let result: Result<(), String> = match fixture.schema.as_str() {
            "Control" => serde_json::from_value::<Control>(fixture.value.clone())
                .map_err(|e| e.to_string())
                .and_then(|control| {
                    control.validate().map_err(|e| e.to_string())?;
                    match control {
                        Control::BrowserCmd { cmd } => cmd.check(),
                        _ => Ok(()),
                    }
                }),
            "BrowserStatusData" => serde_json::from_value::<BrowserStatusData>(fixture.value.clone())
                .map(|_| ())
                .map_err(|e| e.to_string()),
            "BrowserOpenData" => serde_json::from_value::<BrowserOpenData>(fixture.value.clone())
                .map(|_| ())
                .map_err(|e| e.to_string()),
            "BrowserProfilesData" => serde_json::from_value::<BrowserProfilesData>(fixture.value.clone())
                .map(|_| ())
                .map_err(|e| e.to_string()),
            other => panic!("unknown schema {other}"),
        };
        assert_eq!(result.is_ok(), fixture.ok, "fixture `{}`: {result:?}", fixture.name);
    }
}

#[test]
fn browser_cmd_round_trips_with_defaults() {
    let cmd: unipi_proto::BrowserCmd =
        serde_json::from_str(r#"{"op":"mouse","kind":"moved","x":1,"y":2}"#).unwrap();
    let json = serde_json::to_value(&cmd).unwrap();
    assert_eq!(json["button"], "none");
    assert_eq!(json["clicks"], 0);
    let back: unipi_proto::BrowserCmd = serde_json::from_value(json).unwrap();
    assert_eq!(back, cmd);
}
