//! Platform senders (unipi `platforms/*.ts`). Request shapes are built by pure functions so
//! they are unit-tested; sending goes through the host `http` / `ui.os-notify` imports.

use serde::Serialize;
use serde_json::{Value, json};

use crate::config::NotifyConfig;
use crate::messages::{escape_markdown_v2, map_priority, urgency};

/// One platform's outcome (unipi `NotifyResult`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct PlatformResult {
    /// Platform.
    pub platform: String,
    /// Sent (or intentionally suppressed).
    pub success: bool,
    /// Suppressed (focus or recent input).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub suppressed: bool,
    /// Effective numeric priority.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<i64>,
    /// Failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A prepared HTTP request.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Request {
    /// URL.
    pub url: String,
    /// Headers.
    pub headers: Vec<(String, String)>,
    /// JSON body.
    pub body: Value,
}

/// unipi `sendGotifyNotification` request.
pub(crate) fn gotify_request(
    server: &str,
    token: &str,
    title: &str,
    message: &str,
    priority: i64,
) -> Request {
    Request {
        url: format!("{}/message", server.trim_end_matches('/')),
        headers: vec![
            ("Content-Type".into(), "application/json".into()),
            ("X-Gotify-Key".into(), token.into()),
        ],
        body: json!({"title": title, "message": message, "priority": priority}),
    }
}

/// unipi `sendTelegramNotification` request.
pub(crate) fn telegram_request(token: &str, chat_id: &str, title: &str, message: &str) -> Request {
    Request {
        url: format!("https://api.telegram.org/bot{token}/sendMessage"),
        headers: vec![("Content-Type".into(), "application/json".into())],
        body: json!({
            "chat_id": chat_id,
            "text": format!("*{}*\n{}", escape_markdown_v2(title), escape_markdown_v2(message)),
            "parse_mode": "MarkdownV2",
        }),
    }
}

/// Telegram `getUpdates` URL (unipi `pollForChatId`).
pub(crate) fn telegram_updates_url(token: &str) -> String {
    format!("https://api.telegram.org/bot{token}/getUpdates?allowed_updates=%5B%22message%22%5D")
}

/// Chat id of the newest update (unipi `pollForChatId` parsing).
pub(crate) fn chat_id_from_updates(body: &Value) -> Option<String> {
    if body.get("ok").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let last = body.get("result")?.as_array()?.last()?;
    let id = last
        .pointer("/message/chat/id")
        .or_else(|| last.pointer("/callback_query/message/chat/id"))?;
    match id {
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

/// Generic text used when `appDetail` is `minimal` (unipi "Tap to open in `UniPi`").
pub(crate) const MINIMAL_MESSAGE: &str = "Tap to open in Oino";

/// unipi `buildNtfyBody` (no E2E envelope and no `unipi://` deep link, unipi-port §2.5).
pub(crate) fn ntfy_body(
    topic: Option<&str>,
    title: &str,
    message: &str,
    priority: i64,
    kind: Option<&str>,
    minimal: bool,
) -> Value {
    let mut body = json!({
        "title": title,
        "message": if minimal { MINIMAL_MESSAGE } else { message },
        "priority": priority.clamp(1, 5),
    });
    if let Some(t) = topic {
        body["topic"] = json!(t);
    }
    if let Some(k) = kind {
        body["tags"] = json!([k]);
    }
    body
}

/// unipi `sendNtfyNotification` request (JSON publish to the server root).
pub(crate) fn ntfy_request(server: &str, token: Option<&str>, body: Value) -> Request {
    let mut headers = vec![("Content-Type".into(), "application/json".into())];
    if let Some(t) = token.filter(|t| !t.is_empty()) {
        headers.push(("Authorization".into(), format!("Bearer {t}")));
    }
    Request {
        url: server.trim_end_matches('/').to_owned(),
        headers,
        body,
    }
}

/// POSTs a prepared request; non-2xx → `"<label> API error <status>: <body>"`.
pub(crate) async fn post(req: &Request, label: &str) -> Result<(), String> {
    let mut b = oino_extension::http::post(&req.url).timeout_ms(15_000);
    for (k, v) in &req.headers {
        b = b.header(k, v);
    }
    let resp = b
        .body(serde_json::to_vec(&req.body).unwrap_or_default())
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if (200..300).contains(&resp.status()) {
        return Ok(());
    }
    let status = resp.status();
    let text = resp.text().await;
    let text = if text.is_empty() {
        "<no body>".to_owned()
    } else {
        text
    };
    Err(format!("{label} API error {status}: {text}"))
}

/// Sends on one platform; `Ok(Some(priority))` for platforms with numeric priorities,
/// `Err("suppressed")`-free: suppression is reported through `PlatformResult`.
pub(crate) async fn send(
    platform: &str,
    title: &str,
    message: &str,
    cfg: &NotifyConfig,
    priority: Option<&str>,
    event: Option<&str>,
) -> PlatformResult {
    let mut r = PlatformResult {
        platform: platform.to_owned(),
        ..PlatformResult::default()
    };
    let outcome: Result<Option<i64>, String> = match platform {
        "native" => {
            if cfg.native.suppress_when_focused && oino_extension::host::frontend_focused() {
                r.success = true;
                r.suppressed = true;
                return r;
            }
            oino_extension::ui::os_notify(title, message, urgency(priority), None)
                .map(|()| None)
                .map_err(|e| format!("Native notification failed: {e}"))
        }
        "gotify" => match (&cfg.gotify.server_url, &cfg.gotify.app_token) {
            (Some(url), Some(token)) => {
                let p = priority
                    .and_then(|p| map_priority("gotify", p))
                    .unwrap_or(cfg.gotify.priority);
                post(&gotify_request(url, token, title, message, p), "Gotify")
                    .await
                    .map(|()| Some(p))
            }
            _ => Err("Gotify: serverUrl and appToken are required".to_owned()),
        },
        "telegram" => match (&cfg.telegram.bot_token, &cfg.telegram.chat_id) {
            (Some(token), Some(chat)) => {
                post(&telegram_request(token, chat, title, message), "Telegram")
                    .await
                    .map(|()| None)
            }
            _ => Err("Telegram: botToken and chatId are required".to_owned()),
        },
        "ntfy" => {
            if !cfg.ntfy.enabled {
                r.success = true;
                return r;
            }
            match (&cfg.ntfy.server_url, &cfg.ntfy.topic) {
                (Some(url), Some(topic)) => {
                    let p = priority
                        .and_then(|p| map_priority("ntfy", p))
                        .unwrap_or(cfg.ntfy.priority);
                    let minimal =
                        event.is_some() && cfg.ntfy.app_detail.as_deref() == Some("minimal");
                    let body = ntfy_body(Some(topic), title, message, p, event, minimal);
                    post(&ntfy_request(url, cfg.ntfy.token.as_deref(), body), "ntfy")
                        .await
                        .map(|()| Some(p))
                }
                _ => Err("ntfy: serverUrl and topic are required".to_owned()),
            }
        }
        other => Err(format!("unknown platform {other}")),
    };
    match outcome {
        Ok(p) => {
            r.success = true;
            r.priority = p;
        }
        Err(e) => r.error = Some(e),
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gotify_shape() {
        let r = gotify_request("https://g.example/", "tok", "T", "M", 8);
        assert_eq!(r.url, "https://g.example/message");
        assert!(r.headers.contains(&("X-Gotify-Key".into(), "tok".into())));
        assert_eq!(r.body, json!({"title": "T", "message": "M", "priority": 8}));
    }

    #[test]
    fn telegram_shape() {
        let r = telegram_request("123:abc", "42", "Oino — Done", "a.b");
        assert_eq!(r.url, "https://api.telegram.org/bot123:abc/sendMessage");
        assert_eq!(r.body["text"], "*Oino — Done*\na\\.b");
        assert_eq!(r.body["parse_mode"], "MarkdownV2");
        assert_eq!(r.body["chat_id"], "42");
        assert!(
            telegram_updates_url("t").ends_with("getUpdates?allowed_updates=%5B%22message%22%5D")
        );
    }

    #[test]
    fn telegram_chat_detection() {
        assert_eq!(
            chat_id_from_updates(
                &json!({"ok": true, "result": [{"message": {"chat": {"id": 1}}}, {"message": {"chat": {"id": -100}}}]})
            ),
            Some("-100".into())
        );
        assert_eq!(
            chat_id_from_updates(
                &json!({"ok": true, "result": [{"callback_query": {"message": {"chat": {"id": 7}}}}]})
            ),
            Some("7".into())
        );
        assert_eq!(
            chat_id_from_updates(&json!({"ok": true, "result": []})),
            None
        );
        assert_eq!(chat_id_from_updates(&json!({"ok": false})), None);
    }

    #[test]
    fn ntfy_shape() {
        let b = ntfy_body(Some("t"), "T", "M", 9, Some("input_needed"), false);
        assert_eq!(
            b,
            json!({"title": "T", "message": "M", "priority": 5, "topic": "t", "tags": ["input_needed"]})
        );
        let m = ntfy_body(None, "T", "M", 0, None, true);
        assert_eq!(
            m,
            json!({"title": "T", "message": "Tap to open in Oino", "priority": 1})
        );
        let r = ntfy_request("https://ntfy.sh/", Some("tk"), b);
        assert_eq!(r.url, "https://ntfy.sh");
        assert!(
            r.headers
                .contains(&("Authorization".into(), "Bearer tk".into()))
        );
        let anon = ntfy_request("https://ntfy.sh", Some(""), json!({}));
        assert_eq!(anon.headers.len(), 1);
    }
}
