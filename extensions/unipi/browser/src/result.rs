//! `BrowserResult` JSON → `ToolResult`; the `approval_required` handshake (§10.2).

use base64::Engine as _;
use oino_extension::browser::{self, Image};
use oino_extension::prelude::*;

/// Approval choices shown to the user.
pub const CHOICES: [&str; 3] = ["Allow once", "Allow for this site (this session)", "Deny"];

/// Maps a dialog choice to the resend `approval` value (`None` = denied).
#[must_use]
pub fn approval_of(choice: Option<&str>) -> Option<&'static str> {
    match choice {
        Some(c) if c == CHOICES[0] => Some("once"),
        Some(c) if c == CHOICES[1] => Some("site"),
        _ => None,
    }
}

/// Builds the tool result: text (+ image block), `details`, `isError`.
#[must_use]
pub fn to_tool_result(r: &Value, image: Option<&Image>) -> ToolResult {
    let text = r["text"].as_str().unwrap_or_default().to_owned();
    let mut out = if r["isError"] == json!(true) {
        ToolResult::error(text)
    } else {
        ToolResult::text(text)
    };
    if let Some(img) = image {
        out.content.push(json!({
            "type": "image",
            "data": base64::engine::general_purpose::STANDARD.encode(&img.data),
            "mimeType": img.mime_type,
        }));
        if let Some(path) = &img.path {
            out.content
                .push(json!({"type": "text", "text": format!("Saved: {path}")}));
        }
    }
    out.details = r.get("details").cloned().filter(|d| !d.is_null());
    out
}

/// `permission_denied` result after the user declined (or no UI could ask).
#[must_use]
pub fn denied(message: &str, no_ui: bool) -> ToolResult {
    let why = if no_ui {
        "needs the user's approval, but no UI can ask"
    } else {
        "the user declined"
    };
    ToolResult::error(format!(
        "permission_denied: {message} — {why}; don't repeat this action (no side effect)"
    ))
    .with_details(
        json!({"error": {"code": "permission_denied", "message": message, "sideEffect": "none"}}),
    )
}

/// Runs tool `name` with `args`, asking the user when the host requires approval.
pub async fn run_tool(name: &str, args: Value, has_ui: bool) -> ToolResult {
    let op = name.strip_prefix("browser_").unwrap_or(name);
    let mut command = match args {
        Value::Object(m) => Value::Object(m),
        _ => json!({}),
    };
    command["op"] = json!(op);
    for _ in 0..2 {
        let (r, image) = match browser::execute(&command).await {
            Ok(x) => x,
            Err(e) => {
                return ToolResult::error(format!("backend_unavailable: {e} (no side effect)"));
            }
        };
        if r["details"]["error"]["code"] != json!("approval_required") {
            return to_tool_result(&r, image.as_ref());
        }
        let message = r["details"]["error"]["message"]
            .as_str()
            .unwrap_or("Agent wants to act in the browser")
            .to_owned();
        if !has_ui {
            return denied(&message, true);
        }
        let choice = ui::select(&message, &CHOICES).await.ok().flatten();
        match approval_of(choice.as_deref()) {
            Some(a) => command["approval"] = json!(a),
            None => return denied(&message, false),
        }
    }
    ToolResult::error("permission_denied: approval loop (no side effect)")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn golden_results() {
        let ok = json!({"text": "Clicked button \"Sign in\" [e7] · navigated to https://example.com/login",
            "details": {"browser": {"profile": "default", "engine": "chrome", "generation": 1, "tab": "t3",
                "openTabs": ["t3"], "url": "https://example.com/login", "sideEffect": "done"}}, "isError": false});
        let t = to_tool_result(&ok, None);
        assert_eq!(
            serde_json::to_value(&t).unwrap(),
            json!({"content": [{"type": "text", "text": "Clicked button \"Sign in\" [e7] · navigated to https://example.com/login"}],
                   "details": {"browser": {"profile": "default", "engine": "chrome", "generation": 1, "tab": "t3",
                       "openTabs": ["t3"], "url": "https://example.com/login", "sideEffect": "done"}},
                   "isError": false})
        );
        let err = json!({"text": "ref_stale: e7 is gone; take a new snapshot (no side effect)",
            "details": {"error": {"code": "ref_stale", "message": "e7 is gone; take a new snapshot", "sideEffect": "none"}}, "isError": true});
        let t = to_tool_result(&err, None);
        assert!(t.is_error);
        assert_eq!(t.details.unwrap()["error"]["code"], "ref_stale");
        let img = Image {
            mime_type: "image/png".into(),
            data: vec![1, 2, 3],
            path: Some("/tmp/s.png".into()),
        };
        let t = to_tool_result(
            &json!({"text": "Screenshot of t1", "details": {}}),
            Some(&img),
        );
        assert_eq!(
            t.content[1],
            json!({"type": "image", "data": "AQID", "mimeType": "image/png"})
        );
        assert_eq!(t.content[2]["text"], "Saved: /tmp/s.png");
    }

    #[test]
    fn approvals() {
        assert_eq!(approval_of(Some(CHOICES[0])), Some("once"));
        assert_eq!(approval_of(Some(CHOICES[1])), Some("site"));
        assert_eq!(approval_of(Some("Deny")), None);
        assert_eq!(approval_of(None), None);
        let d = denied("Agent wants to click 'Place order' on shop.example", false);
        assert!(d.is_error);
        assert_eq!(d.details.unwrap()["error"]["code"], "permission_denied");
    }
}
