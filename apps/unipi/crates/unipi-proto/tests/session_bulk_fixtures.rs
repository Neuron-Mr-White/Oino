//! `protocol/fixtures/session_bulk.json` against the Rust types (`[ux-sessions]`): the
//! batch archive / restore ops, the sidebar query and their hub events.

use unipi_proto::session::HubEvent;
use unipi_proto::session_bulk::{check_batch, SessionsBatchData, SessionsQueryData};
use unipi_proto::Request;

#[derive(serde::Deserialize)]
struct Fixture {
    name: String,
    schema: String,
    ok: bool,
    value: serde_json::Value,
}

fn check(schema: &str, value: &serde_json::Value) -> Result<(), String> {
    let v = value.clone();
    match schema {
        "Request" => match serde_json::from_value::<Request>(v).map_err(|e| e.to_string())? {
            Request::SessionsArchive { sessions } | Request::SessionsRestore { sessions } => check_batch(&sessions),
            Request::SessionsQuery { .. } => Ok(()),
            _ => Err("not a bulk request".into()),
        },
        "SessionsBatchData" => serde_json::from_value::<SessionsBatchData>(v).map(|_| ()).map_err(|e| e.to_string()),
        "SessionsQueryData" => serde_json::from_value::<SessionsQueryData>(v).map(|_| ()).map_err(|e| e.to_string()),
        "HubEvent" => serde_json::from_value::<HubEvent>(v).map(|_| ()).map_err(|e| e.to_string()),
        other => Err(format!("unknown schema {other}")),
    }
}

#[test]
fn session_bulk_fixtures_match() {
    let fixtures: Vec<Fixture> =
        serde_json::from_str(include_str!("../../../protocol/fixtures/session_bulk.json")).expect("well-formed");
    assert!(fixtures.len() >= 10);
    for f in fixtures {
        let got = check(&f.schema, &f.value);
        assert_eq!(got.is_ok(), f.ok, "{}: {got:?}", f.name);
    }
}
