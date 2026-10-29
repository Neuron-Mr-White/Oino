//! Board relay contract fixtures: same file as the vitest suite
//! (`packages/protocol/tests/board.test.ts`).
use serde_json::Value;
use unipi_proto::{BoardHttpData, Control, PROTO_VERSION};

#[derive(Debug, serde::Deserialize)]
struct Fixture {
    name: String,
    schema: String,
    value: Value,
    ok: bool,
}

#[test]
fn shared_board_fixtures_agree() {
    let fixtures: Vec<Fixture> =
        serde_json::from_str(include_str!("../../../protocol/fixtures/board.json")).unwrap();
    assert!(fixtures.len() >= 30);
    for fixture in fixtures {
        let result: Result<(), String> = match fixture.schema.as_str() {
            "Control" => serde_json::from_value::<Control>(fixture.value.clone())
                .map_err(|e| e.to_string())
                .and_then(|control| control.validate().map_err(|e| e.to_string())),
            "BoardHttpData" => serde_json::from_value::<BoardHttpData>(fixture.value.clone())
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
fn board_requests_round_trip_exact_json() {
    let hello: Control = serde_json::from_value(serde_json::json!({
        "type": "hello",
        "proto": PROTO_VERSION,
        "req": {"op": "board_http", "method": "GET", "path": "/api/projects"}
    }))
    .unwrap();
    // body_len 0 and absent content_type are omitted on the wire.
    assert_eq!(
        serde_json::to_value(&hello).unwrap(),
        serde_json::json!({
            "type": "hello",
            "proto": PROTO_VERSION,
            "req": {"op": "board_http", "method": "GET", "path": "/api/projects"}
        })
    );
    let upload = serde_json::json!({
        "type": "hello",
        "proto": PROTO_VERSION,
        "req": {
            "op": "board_http",
            "method": "POST",
            "path": "/api/tasks/p/P-1/attachments?name=a.png",
            "content_type": "application/octet-stream",
            "body_len": 1234
        }
    });
    let parsed: Control = serde_json::from_value(upload.clone()).unwrap();
    assert_eq!(serde_json::to_value(&parsed).unwrap(), upload);
    let event = Control::BoardEvent {
        event: Some("revision".into()),
        data: "3".into(),
    };
    assert_eq!(
        serde_json::to_value(&event).unwrap(),
        serde_json::json!({"type": "board_event", "event": "revision", "data": "3"})
    );
    let bare = Control::BoardEvent {
        event: None,
        data: "x".into(),
    };
    assert_eq!(
        serde_json::to_value(&bare).unwrap(),
        serde_json::json!({"type": "board_event", "data": "x"})
    );
}
