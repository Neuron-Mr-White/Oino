//! UNI-248 + UNI-253 contract fixtures: same file as the vitest suite
//! (`packages/protocol/tests/sharing.test.ts`).
use serde_json::Value;
use unipi_proto::{
    AppErrorCode, Control, FeatureLeaveData, FeatureLeaveScope, FeatureRequestsData, Request,
};

#[derive(Debug, serde::Deserialize)]
struct Fixture {
    name: String,
    schema: String,
    value: Value,
    ok: bool,
}

#[test]
fn shared_sharing_fixtures_agree() {
    let fixtures: Vec<Fixture> =
        serde_json::from_str(include_str!("../../../protocol/fixtures/sharing.json")).unwrap();
    assert!(fixtures.len() >= 24);
    for fixture in fixtures {
        let result: Result<(), String> = match fixture.schema.as_str() {
            "Control" => serde_json::from_value::<Control>(fixture.value.clone())
                .map_err(|e| e.to_string())
                .and_then(|control| control.validate().map_err(|e| e.to_string())),
            "FeatureRequestsData" => {
                serde_json::from_value::<FeatureRequestsData>(fixture.value.clone())
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
            "FeatureLeaveData" => serde_json::from_value::<FeatureLeaveData>(fixture.value.clone())
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

#[test]
fn feature_leave_round_trips() {
    let hello = r#"{"type":"hello","proto":"1.15.0","req":{"op":"feature_leave","feature":"forward","scope":"self"}}"#;
    let control: Control = serde_json::from_str(hello).expect("parse");
    control.validate().expect("valid");
    let Control::Hello { req, .. } = &control else {
        panic!("expected hello")
    };
    let Request::FeatureLeave { feature, scope } = req else {
        panic!("expected FeatureLeave")
    };
    assert_eq!(feature.as_str(), "forward");
    assert_eq!(*scope, FeatureLeaveScope::OnlySelf);
    // Serialised back out (and re-parsed) it says the same thing: `self`
    // is the default, so it isn't written at all.
    let value = serde_json::to_value(&control).unwrap();
    assert_eq!(value["req"]["op"], "feature_leave");
    assert!(value["req"].get("scope").is_none());
    let again: Control = serde_json::from_value(value).unwrap();
    assert!(again.validate().is_ok());

    // `scope` is optional and means `self`; `all` is the other value.
    let bare: Control = serde_json::from_str(
        r#"{"type":"hello","proto":"1.15.0","req":{"op":"feature_leave","feature":"push"}}"#,
    )
    .unwrap();
    let Control::Hello {
        req: Request::FeatureLeave { scope, .. },
        ..
    } = &bare
    else {
        panic!("expected FeatureLeave")
    };
    assert!(scope.is_self());
    let all: Control = serde_json::from_str(
        r#"{"type":"hello","proto":"1.15.0","req":{"op":"feature_leave","feature":"cron","scope":"all"}}"#,
    )
    .unwrap();
    let Control::Hello {
        req: Request::FeatureLeave { scope, .. },
        ..
    } = &all
    else {
        panic!("expected FeatureLeave")
    };
    assert_eq!(scope.as_str(), "all");
    // A scope nobody knows is a schema error, not a silent default.
    assert!(serde_json::from_str::<Control>(
        r#"{"type":"hello","proto":"1.15.0","req":{"op":"feature_leave","feature":"cron","scope":"me"}}"#
    )
    .is_err());
}

/// A host older than 1.15 doesn't know `feature_leave`: it answers
/// `E_SCHEMA` (unknown op) like it does for `feature_request`. That
/// rejection has to be representable on the wire both sides speak.
#[test]
fn older_host_rejection_is_representable() {
    let reply = r#"{"type":"err","error":{"code":"E_SCHEMA","message":"unknown op `feature_leave`","retryable":false}}"#;
    let control: Control = serde_json::from_str(reply).expect("parse");
    control.validate().expect("valid");
    let Control::Err { error } = control else {
        panic!("expected err")
    };
    assert_eq!(error.code, AppErrorCode::E_SCHEMA);
    assert!(error.message.as_str().contains("feature_leave"));
    assert!(!error.retryable);
    // …and the app's own copy of the same rejection parses too.
    let error: unipi_proto::AppErrorDto =
        serde_json::from_value(serde_json::to_value(&error).unwrap()).unwrap();
    assert_eq!(error.code, AppErrorCode::E_SCHEMA);
}

/// The phone's own reply for "I turned it off for myself": what it shows
/// next (the remaining devices, or "off on the PC").
#[test]
fn leave_data_mirrors_the_host_answer() {
    let json = r#"{"feature":"forward","left":true,"now_enabled":false,"devices":["Pixel 8"]}"#;
    let data: FeatureLeaveData = serde_json::from_str(json).unwrap();
    assert_eq!(data.feature, unipi_proto::FeatureKind::Forward);
    assert!(data.left);
    assert!(!data.now_enabled);
    assert_eq!(data.devices, ["Pixel 8"]);
    // Same value back out, byte for byte in the fields the app reads.
    assert_eq!(
        serde_json::to_value(&data).unwrap(),
        serde_json::from_str::<Value>(json).unwrap()
    );
}
