//! UNI-317/UNI-318 contract fixtures: same file as the vitest suite
//! (`packages/protocol/tests/cron.test.ts`).
use serde_json::Value;
use unipi_proto::{Control, CronJob, CronSettingsData};

#[derive(Debug, serde::Deserialize)]
struct Fixture {
    name: String,
    schema: String,
    value: Value,
    ok: bool,
}

#[test]
fn shared_cron_fixtures_agree() {
    let fixtures: Vec<Fixture> = serde_json::from_str(include_str!(
        "../../../apps/unipi/protocol/fixtures/cron.json"
    ))
    .unwrap();
    assert!(fixtures.len() >= 20);
    for fixture in fixtures {
        let result: Result<(), String> = match fixture.schema.as_str() {
            "Control" => serde_json::from_value::<Control>(fixture.value.clone())
                .map_err(|e| e.to_string())
                .and_then(|control| control.validate().map_err(|e| e.to_string())),
            "CronJob" => serde_json::from_value::<CronJob>(fixture.value.clone())
                .map(|_| ())
                .map_err(|e| e.to_string()),
            "CronSettingsData" => serde_json::from_value::<CronSettingsData>(fixture.value.clone())
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
