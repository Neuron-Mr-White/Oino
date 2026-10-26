//! Frontend-agnostic UI (ui-protocol): dialogs, fire-and-forget requests, panels, editor,
//! device integration.

use serde_json::{Value, json};

use crate::Result;
use crate::bindings::oino::extension::ui as wu;
use crate::error::parse;

/// Raises a dialog (`{method, ...}` per ui-protocol §4) and waits for the answer.
pub async fn dialog(request: &Value) -> Result<Value> {
    parse(&wu::dialog(request.to_string(), None).await?)
}

/// pi `ctx.ui.select` → chosen value (`None` when dismissed).
pub async fn select(title: &str, options: &[&str]) -> Result<Option<String>> {
    let answer = dialog(&json!({"method": "select", "title": title, "options": options})).await?;
    Ok(answer
        .get("value")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned))
}

/// pi `ctx.ui.confirm`.
pub async fn confirm(title: &str, message: &str) -> Result<bool> {
    let answer = dialog(&json!({"method": "confirm", "title": title, "message": message})).await?;
    Ok(answer
        .get("confirmed")
        .and_then(Value::as_bool)
        .unwrap_or(false))
}

/// pi `ctx.ui.input`.
pub async fn input(title: &str, placeholder: Option<&str>) -> Result<Option<String>> {
    let answer =
        dialog(&json!({"method": "input", "title": title, "placeholder": placeholder})).await?;
    Ok(answer
        .get("value")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned))
}

/// Fire-and-forget request (`notify`, `setStatus`, `setWidget`, `setTitle`, …).
pub fn fire(request: &Value) -> Result<()> {
    Ok(wu::fire(&request.to_string())?)
}

/// pi `ctx.ui.notify(message, type)`.
pub fn notify(message: &str, kind: &str) -> Result<()> {
    fire(&json!({"method": "notify", "message": message, "notifyType": kind}))
}

/// pi `ctx.ui.setStatus(key, text)`.
pub fn status(key: &str, text: Option<&str>) -> Result<()> {
    fire(&json!({"method": "setStatus", "statusKey": key, "statusText": text}))
}

/// pi `ctx.ui.setWidget(key, lines)`.
pub fn widget(key: &str, lines: Option<&[String]>) -> Result<()> {
    fire(&json!({"method": "setWidget", "widgetKey": key, "widgetLines": lines}))
}

/// Opens a panel (ui-protocol §5) → panel id.
pub fn panel_open(spec: &Value) -> Result<String> {
    Ok(wu::panel_open(&spec.to_string())?)
}
/// Replaces a panel spec.
pub fn panel_update(panel_id: &str, spec: &Value) -> Result<()> {
    Ok(wu::panel_update(panel_id, &spec.to_string())?)
}
pub use wu::{editor_insert, editor_set, editor_text, panel_close};

/// OS notification (permission `osNotify`).
pub fn os_notify(title: &str, body: &str, urgency: &str, action_url: Option<&str>) -> Result<()> {
    Ok(wu::os_notify(title, body, urgency, action_url)?)
}
/// Clipboard text (permission `clipboard.write`).
pub fn clipboard_write_text(text: &str) -> Result<()> {
    Ok(wu::clipboard_write_text(text)?)
}
/// Clipboard image (permission `clipboard.read`).
pub async fn clipboard_read_image() -> Result<Option<Value>> {
    wu::clipboard_read_image()
        .await?
        .map(|t| parse(&t))
        .transpose()
}
/// Opens a URL (permission `openUrl`).
pub fn open_url(url: &str) -> Result<()> {
    Ok(wu::open_url(url)?)
}
/// External editor (permission `editExternally`).
pub async fn edit_externally(text: &str, extension_hint: Option<&str>) -> Result<Option<String>> {
    Ok(wu::edit_externally(text.to_owned(), extension_hint.map(ToOwned::to_owned)).await?)
}
