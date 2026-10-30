//! v2 slot integration in the TUI (WP-U4): `turnFooter` pull rendering, `$show`/`$hide`
//! visibility reports and resync on a `rev` gap (ui-protocol §6.2, §6.5, §6.7, §9.2).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::needless_pass_by_value,
    reason = "tests assert by unwrapping; small JSON helpers take owned values"
)]

mod common;

use common::{Harness, TestHost};
use serde_json::{Value, json};

fn patch(rev: u64, ops: Vec<Value>) -> Value {
    json!({"type": "oino_ui_patch", "scope": "session", "sessionId": "s-1", "rev": rev, "ops": ops})
}

fn footer_entry(rev: u64) -> Value {
    json!({
        "id": "probe/turnFooter:files",
        "extensionId": "probe",
        "scope": "session",
        "slot": "turnFooter",
        "order": 0,
        "seq": 1,
        "title": "Changed files",
        "view": {"kind": "renderer", "key": "files"},
        "rev": rev
    })
}

async fn with_turn(host: TestHost) -> Harness {
    let mut h = Harness::new(host).await;
    let user = json!({"role": "user", "content": "change the file", "timestamp": 0});
    h.json_event(json!({"type": "message_start", "message": user}))
        .await;
    h.json_event(json!({"type": "message_end", "message": user}))
        .await;
    let assistant = json!({"role": "assistant", "content": [{"type": "text", "text": "done"}],
        "api": "faux", "provider": "faux", "model": "faux-1", "stopReason": "stop", "timestamp": 0,
        "usage": {"input": 1, "output": 1, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 2,
                  "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}}});
    h.json_event(json!({"type": "message_start", "message": assistant}))
        .await;
    h.json_event(json!({"type": "message_end", "message": assistant}))
        .await;
    h.json_event(patch(
        1,
        vec![json!({"op": "put", "entry": footer_entry(1)})],
    ))
    .await;
    h
}

#[tokio::test]
async fn turn_footer_is_requested_and_rendered_after_the_turn() {
    let host = TestHost {
        render_reply: Some(json!({"t": "Paragraph", "text": "1 file changed"})),
        ..TestHost::default()
    };
    let mut h = with_turn(host).await;
    let frame = h.frame(60, 24);
    assert!(frame.contains("1 file changed"), "{frame}");
    let requests = h.host.renders.lock().unwrap().clone();
    let footer = requests
        .iter()
        .find(|request| request.target == "turnFooter")
        .expect("a turnFooter render request");
    assert_eq!(footer.key, "files");
    assert!(footer.turn_entry_id.is_none());
    // The footer renders after the turn's last row (the assistant text).
    let done = frame.find("done").expect("assistant row");
    let changed = frame.find("1 file changed").expect("footer row");
    assert!(changed > done, "{frame}");
}

#[tokio::test]
async fn invalidate_asks_the_renderer_again() {
    let host = TestHost {
        render_reply: Some(json!({"t": "Paragraph", "text": "1 file changed"})),
        ..TestHost::default()
    };
    let mut h = with_turn(host).await;
    let _ = h.frame(60, 24);
    let before = h.host.renders.lock().unwrap().len();
    h.json_event(patch(
        2,
        vec![json!({"op": "invalidate", "id": "probe/turnFooter:files"})],
    ))
    .await;
    let _ = h.frame(60, 24);
    let after = h.host.renders.lock().unwrap().len();
    assert!(after > before, "footer re-requested after invalidate");
}

#[tokio::test]
async fn widget_visibility_is_reported() {
    let mut h = Harness::new(TestHost::default()).await;
    h.json_event(patch(
        1,
        vec![json!({"op": "put", "entry": {
            "id": "probe/widget:todo",
            "extensionId": "probe",
            "scope": "session",
            "slot": "widget",
            "order": 0,
            "seq": 1,
            "view": {"kind": "lines", "lines": ["todo"]},
            "rev": 1
        }})],
    ))
    .await;
    let visible = h
        .commands()
        .into_iter()
        .find(|command| command["type"] == "_oino/ui_visible")
        .expect("ui_visible command");
    assert_eq!(visible["entryId"], "probe/widget:todo");
    assert_eq!(visible["visible"], true);
    assert_eq!(visible["frontend"], "tui");
}

#[tokio::test]
async fn a_rev_gap_asks_for_a_resync() {
    let mut h = Harness::new(TestHost::default()).await;
    h.json_event(patch(3, Vec::new())).await;
    let resync = h
        .commands()
        .into_iter()
        .find(|command| command["type"] == "_oino/ui_resync")
        .expect("ui_resync command");
    assert_eq!(resync["rev"], 0);
}
