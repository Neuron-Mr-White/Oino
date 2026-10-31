//! v2 (host.md §9.9) contract fixtures: same file as the vitest suite
//! (`packages/protocol/tests/settings.test.ts`).
//!
//! `host_settings_set` is validated at the proto layer for the *key* (a
//! closed list of dotted paths plus open `browser.*`/`sharing.*`/`cron.*`
//! leaves) and for the `relay` value; the rest of the value is the host's
//! schema — these fixtures pin the wire shape both sides speak.
use serde_json::Value;
use unipi_proto::{Control, HostSettingsKey, RelaySetting, HOST_SETTINGS_KEYS};

#[derive(Debug, serde::Deserialize)]
struct Fixture {
    name: String,
    schema: String,
    value: Value,
    ok: bool,
}

fn parse_host_settings(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    for (key, block) in object {
        match key.as_str() {
            "relay" => {
                if block
                    .as_str()
                    .and_then(|text| RelaySetting::try_from(text.to_string()).ok())
                    .is_none()
                {
                    return false;
                }
            }
            "relay_only" => {
                if !block.is_boolean() {
                    return false;
                }
            }
            "term" => {
                let Some(cap) = block.get("max") else {
                    continue;
                };
                if !cap.as_u64().is_some_and(|max| (1..=128).contains(&max)) {
                    return false;
                }
            }
            "forwarding" | "push" | "fs" | "browser" | "sharing" | "cron" => {
                if !block.is_object() {
                    return false;
                }
            }
            "update"
                if !matches!(
                    block.get("mode").and_then(Value::as_str),
                    None | Some("prompt") | Some("auto")
                ) =>
            {
                return false;
            }
            // Unknown top-level keys are preserved by host.md §9.9's schema
            // ("unknown keys are kept"); the app ignores them.
            _ => {}
        }
    }
    true
}

#[test]
fn shared_settings_fixtures_agree() {
    let fixtures: Vec<Fixture> =
        serde_json::from_str(include_str!("../../../protocol/fixtures/settings.json")).unwrap();
    assert!(fixtures.len() >= 11);
    for fixture in fixtures {
        let result: Result<(), String> = match fixture.schema.as_str() {
            "Control" => serde_json::from_value::<Control>(fixture.value.clone())
                .map_err(|e| e.to_string())
                .and_then(|control| control.validate().map_err(|e| e.to_string())),
            "HostSettingsData" => {
                if parse_host_settings(&fixture.value) {
                    Ok(())
                } else {
                    Err("not a valid host settings object".into())
                }
            }
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

#[test]
fn every_advertised_key_is_accepted() {
    for key in HOST_SETTINGS_KEYS {
        let parsed: HostSettingsKey = serde_json::from_value(Value::String(key.into()))
            .unwrap_or_else(|e| panic!("{key}: {e}"));
        assert_eq!(parsed.as_str(), key);
    }
    assert!(serde_json::from_value::<HostSettingsKey>(Value::String("cron".into())).is_ok());
    assert!(serde_json::from_value::<HostSettingsKey>(Value::String("cron.paused".into())).is_ok());
    assert!(serde_json::from_value::<HostSettingsKey>(Value::String("nope".into())).is_err());
}
