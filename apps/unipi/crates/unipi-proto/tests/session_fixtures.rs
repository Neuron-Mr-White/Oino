//! v2 session-layer contract fixtures (host.md §7.3–7.4, WP-H4): the same file as the vitest
//! suite (`packages/protocol/tests/session.test.ts`).
//!
//! The session stream envelopes are typed on both sides (`SessionSnapshot`, `SessionReply`,
//! `HubEvent`, `SessionCard`, the `events` / `session_cmd` controls and the session
//! requests); the payloads they carry are oino's own JSON and are only checked for being
//! present (the app narrows them by `type`, ui-protocol D-U14).
use serde::Deserialize;
use serde_json::Value;
use unipi_proto::{
    Control, HubEvent, Request, SessionReply, SessionSnapshot, SessionsListData, PROTO_VERSION,
};

#[derive(Debug, Deserialize)]
struct Fixture {
    name: String,
    schema: String,
    value: Value,
    ok: bool,
}

fn fixtures() -> Vec<Fixture> {
    serde_json::from_str(include_str!("../../../protocol/fixtures/session.json"))
        .expect("session.json must be well-formed")
}

fn parses(schema: &str, value: &Value) -> Result<(), String> {
    match schema {
        "SessionControl" => match serde_json::from_value::<Control>(value.clone()) {
            Ok(control @ (Control::Events { .. } | Control::SessionCmd { .. })) => {
                control.validate().map_err(|error| error.to_string())
            }
            Ok(_) => Err("not a session-stream control".into()),
            Err(error) => Err(error.to_string()),
        },
        "SessionRequest" => match serde_json::from_value::<Request>(value.clone()) {
            Ok(
                Request::SessionsList { .. }
                | Request::SessionSearch { .. }
                | Request::EventsOpen
                | Request::SessionNew { .. }
                | Request::SessionOpen { .. }
                | Request::SessionFork { .. }
                | Request::SessionArchive { .. }
                | Request::SessionRestore { .. }
                | Request::SessionDelete { .. }
                | Request::TrashEmpty
                | Request::SessionRename { .. }
                | Request::SessionClose { .. }
                | Request::SessionMove { .. }
                | Request::SessionMetaSet { .. },
            ) => Ok(()),
            Ok(_) => Err("not a session request".into()),
            Err(error) => Err(error.to_string()),
        },
        "SessionsListData" => serde_json::from_value::<SessionsListData>(value.clone())
            .map(|_| ())
            .map_err(|error| error.to_string()),
        "SessionSnapshot" => serde_json::from_value::<SessionSnapshot>(value.clone())
            .map_err(|error| error.to_string())
            .and_then(|snapshot| snapshot.validate()),
        "TrashEmptyData" => {
            serde_json::from_value::<unipi_proto::session::TrashEmptyData>(value.clone())
                .map(|_| ())
                .map_err(|error| error.to_string())
        }
        "SessionReply" => serde_json::from_value::<SessionReply>(value.clone())
            .map(|_| ())
            .map_err(|error| error.to_string()),
        "HubEvent" => serde_json::from_value::<HubEvent>(value.clone())
            .map(|_| ())
            .map_err(|error| error.to_string()),
        other => panic!("unknown schema {other}"),
    }
}

#[test]
fn proto_is_v2_minor_friendly() {
    assert!(PROTO_VERSION.starts_with("2."));
}

#[test]
fn shared_session_fixtures_agree() {
    let fixtures = fixtures();
    assert!(fixtures.len() >= 30, "{}", fixtures.len());
    for name in [
        "SessionControl",
        "SessionRequest",
        "SessionsListData",
        "SessionSnapshot",
        "SessionReply",
        "HubEvent",
    ] {
        assert!(
            fixtures.iter().any(|fixture| fixture.schema == name),
            "no fixture for {name}"
        );
    }
    for fixture in fixtures {
        let result = parses(&fixture.schema, &fixture.value);
        assert_eq!(
            result.is_ok(),
            fixture.ok,
            "fixture `{}`: {result:?}",
            fixture.name
        );
    }
}

#[test]
fn meta_fields_round_trip_the_shared_card_fixture() {
    // WP-A4 (app spec §3.14): the meta fixture the vitest suite parses too.
    let fixtures = fixtures();
    let fixture = fixtures
        .iter()
        .find(|fixture| {
            fixture.name == "sessions_list card (pinned, unread, title locked, changes, cron)"
        })
        .expect("the meta fixture exists");
    let cards: SessionsListData = serde_json::from_value(fixture.value.clone()).expect("parses");
    let card = cards.first().expect("one card");
    assert!(card.pinned && card.unread && card.title_locked);
    assert_eq!(card.source.as_deref(), Some("cron"));
    assert_eq!(
        card.changes,
        Some(unipi_proto::session::CardChanges { add: 128, del: 14 })
    );
    assert_eq!(card.last_activity, 1_761_905_520_000);
    // …and serializes back with the same shape (false/unset omitted).
    let value = serde_json::to_value(cards).unwrap();
    assert_eq!(value[0]["pinned"], serde_json::json!(true));
    assert!(value[0].get("needs_you").is_none());
}

#[test]
fn session_cmd_is_any_rpc_command_verbatim() {
    let value = serde_json::json!({
        "type": "session_cmd",
        "id": 7,
        "cmd": {"type": "_oino/ui_action", "entryId": "x/1", "actionId": "undo"}
    });
    let Control::SessionCmd { id, cmd } = serde_json::from_value(value).expect("parse") else {
        panic!("expected session_cmd");
    };
    assert_eq!(id, Some(serde_json::json!(7)));
    assert_eq!(cmd["type"], "_oino/ui_action");
    assert_eq!(cmd["entryId"], "x/1");
    // Round-trips byte-identically apart from key order: nothing is re-interpreted.
    let round = serde_json::to_value(Control::SessionCmd { id, cmd }).expect("serialize");
    assert_eq!(round["type"], "session_cmd");
    assert_eq!(round["cmd"]["entryId"], "x/1");
}

#[test]
fn events_carry_numbered_events() {
    let value = serde_json::json!({
        "type": "events",
        "items": [{ "session": "s1", "seq": 1, "event": { "type": "agent_start" } }]
    });
    let Control::Events { items } = serde_json::from_value(value).expect("parse") else {
        panic!("expected events");
    };
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].seq, 1);
    assert_eq!(items[0].kind(), Some("agent_start"));
}
