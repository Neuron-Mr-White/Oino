//! `/answer [reply|questions|web]` (unipi `src/answer/index.ts`, `reply.ts`).
//!
//! | unipi | Oino |
//! |---|---|
//! | reply panel (fixed input under the scrollable reply) | `custom` dialog: markdown reply + multiline `input` + "one by one" button |
//! | Q/A template in pi's editor (Ctrl+G → `$EDITOR`) | `editor` dialog prefilled with the template (frontends offer external editing) |
//! | `node:http` form on 127.0.0.1 | host loopback `http.listener`; waiting overlay = `openPanel` (Esc cancels) |
//! | hint widget above the editor | `setWidget` `unipi-answer-hint` (aboveEditor), cleared on the next input / turn |

pub mod extract;
pub mod web;

use std::cell::RefCell;
use std::collections::HashMap;

use futures::channel::oneshot;
use oino_extension::command::CompletionItem;
use oino_extension::http::Listener;
use oino_extension::prelude::*;
use oino_extension::sync::SessionMap;
use oino_extension::wit::session as ws;

use self::extract::{
    build_template, compose_answers, extract_questions, last_assistant_text, parse_template,
};
use self::web::{Reply, WebAnswer, choose_port, form_page, is_ssh, route, waiting_lines};
use crate::settings::AnswerMethod;

/// Command name.
pub const COMMAND: &str = "answer";
/// Hint widget key.
pub const HINT_WIDGET: &str = "unipi-answer-hint";
/// Web form waiting panel.
pub const WEB_PANEL: &str = "unipi.utility/answer-web";

const METHODS: [(&str, &str); 3] = [
    (
        "reply",
        "The reply above a fixed input box — scroll it while you type",
    ),
    (
        "questions",
        "One answer per detected question (Q/A template)",
    ),
    (
        "web",
        "Answer the questions in a browser form (works over SSH)",
    ),
];

/// Argument completions (unipi `getArgumentCompletions`).
#[must_use]
pub fn complete(prefix: &str) -> Vec<CompletionItem> {
    let needle = prefix.trim().to_lowercase();
    METHODS
        .iter()
        .filter(|(v, _)| v.starts_with(&needle))
        .map(|(v, d)| CompletionItem {
            value: (*v).to_owned(),
            label: Some((*v).to_owned()),
            description: Some((*d).to_owned()),
        })
        .collect()
}

/// The method for an argument (falls back to the configured default; `web` without
/// questions becomes `reply`).
#[must_use]
pub fn method_for(arg: &str, default: AnswerMethod, questions: usize) -> AnswerMethod {
    let m = AnswerMethod::parse(arg).unwrap_or(default);
    if m == AnswerMethod::Web && questions == 0 {
        AnswerMethod::Reply
    } else {
        m
    }
}

/// Hint line text.
#[must_use]
pub fn hint_text(n: usize) -> String {
    format!(
        "  {n} question{} in the reply — /answer questions to answer them one by one",
        if n == 1 { "" } else { "s" }
    )
}

/// The reply dialog (`custom`).
#[must_use]
pub fn reply_dialog(reply: &str, questions: usize) -> Value {
    let mut header = vec![json!({"text": " /answer · reply ", "tone": "accent"})];
    if questions > 0 {
        header.push(json!({"text": format!(" {questions} question{} · tab answers them one by one ", if questions == 1 { "" } else { "s" }), "dim": true}));
    }
    let mut children = vec![
        json!({"t": "text", "value": header}),
        json!({"t": "markdown", "key": "reply", "value": reply}),
        json!({"t": "input", "id": "answer", "placeholder": "your answer (multi-line: shift+enter)", "onSubmit": "send", "multiline": true}),
    ];
    let mut hints = vec![
        json!({"keys": "↵", "label": "send"}),
        json!({"keys": "shift+↵", "label": "newline"}),
    ];
    if questions > 0 {
        children
            .push(json!({"t": "button", "label": "Answer one by one", "actionId": "questions"}));
        hints.push(json!({"keys": "tab", "label": "questions"}));
    }
    hints.push(json!({"keys": "esc", "label": "back"}));
    children.push(json!({"t": "keyHints", "hints": hints}));
    json!({
        "method": "custom",
        "title": "/answer · reply",
        "size": "full",
        "spec": {"t": "stack", "direction": "vertical", "gap": 1, "children": children},
    })
}

/// What the reply dialog returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplyOutcome {
    /// Send this text.
    Send(String),
    /// Switch to the per-question editor.
    Questions,
    /// Closed.
    Cancel,
}

/// Decodes the reply dialog answer.
#[must_use]
pub fn reply_outcome(answer: &Value) -> ReplyOutcome {
    if answer.get("cancelled").and_then(Value::as_bool) == Some(true) {
        return ReplyOutcome::Cancel;
    }
    let v = answer.get("value").unwrap_or(&Value::Null);
    match v.get("actionId").and_then(Value::as_str) {
        Some("questions") => ReplyOutcome::Questions,
        Some("send") => {
            let text = v
                .get("value")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_owned();
            if text.is_empty() {
                ReplyOutcome::Cancel
            } else {
                ReplyOutcome::Send(text)
            }
        }
        _ => ReplyOutcome::Cancel,
    }
}

/// The per-question editor dialog.
#[must_use]
pub fn questions_dialog(questions: &[String]) -> Value {
    let n = questions.len();
    let title = if n > 0 {
        format!(
            "Answer {n} question{} — Tab next answer · external editor available",
            if n == 1 { "" } else { "s" }
        )
    } else {
        "Reply to the last message (no questions found)".to_owned()
    };
    let template = build_template(questions);
    let mut d =
        json!({"method": "editor", "title": title, "prefill": template, "language": "markdown"});
    if let Some(line) = extract::first_answer_line(&template) {
        d["cursorLine"] = json!(line);
    }
    d
}

/// The web form waiting panel.
#[must_use]
pub fn web_panel(lines: &[String]) -> Value {
    let mut children: Vec<Value> = lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            json!({"t": "text", "value": [{"text": l, "bold": i == 0, "dim": lines.len() == 4 && i == 2}]})
        })
        .collect();
    children.push(json!({"t": "text", "value": [{"text": "Waiting for your answers… esc cancels", "dim": true}]}));
    json!({
        "panelId": WEB_PANEL,
        "placement": "overlay",
        "title": "answer — web form",
        "focus": true,
        "closeOnEscape": true,
        "keys": [{"key": "escape", "actionId": "cancel", "label": "Cancel"}],
        "spec": {"t": "stack", "direction": "vertical", "children": children},
    })
}

/// `/answer` state.
#[derive(Default)]
pub struct Answerer {
    hinted: SessionMap<bool>,
    waiters: RefCell<HashMap<String, oneshot::Sender<()>>>,
}

fn env(name: &str) -> Option<String> {
    host::env(name).ok().flatten()
}

impl Answerer {
    /// `agent_end`: the questions hint.
    pub fn on_agent_end(&self, cx: &Ctx, enabled: bool) {
        if !cx.has_ui() || !enabled {
            return;
        }
        let reply = last_assistant_text(&session::branch(None).unwrap_or_default());
        let n = extract_questions(&reply).len();
        if n == 0 {
            return;
        }
        let text = hint_text(n);
        let _ = ui::fire(&json!({
            "method": "setWidget",
            "widgetKey": HINT_WIDGET,
            "widgetLines": [text],
            "widgetPlacement": "aboveEditor",
            "widgetSpec": {"t": "text", "value": [{"text": text, "dim": true}]},
        }));
        self.hinted.with(cx.session_id(), |h| *h = true);
    }

    /// Any new input or turn clears the hint.
    pub fn clear_hint(&self, sid: &str) {
        if self.hinted.with(sid, std::mem::take) {
            let _ = ui::widget(HINT_WIDGET, None);
        }
    }

    /// `oino_ui_action` for the web panel.
    pub fn on_ui_action(&self, ev: &Event) {
        let panel = ev
            .get("panelId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let action = ev
            .get("actionId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if matches!(action, "cancel" | "$close")
            && let Some(tx) = self.waiters.borrow_mut().remove(panel)
        {
            let _ = tx.send(());
        }
    }

    /// The command.
    pub async fn run(
        &self,
        args: &str,
        cx: &CommandCtx,
        settings: crate::settings::AnswerSettings,
    ) -> Result<()> {
        if !cx.has_ui() {
            let _ = ui::notify("/answer needs an interactive frontend", "warning");
            return Ok(());
        }
        let reply = last_assistant_text(&session::branch(None).unwrap_or_default());
        if reply.is_empty() {
            ui::notify("No agent reply to answer yet.", "info")?;
            return Ok(());
        }
        let questions = extract_questions(&reply);
        let mut method = method_for(args, settings.method, questions.len());
        let mut message: Option<String> = None;
        if method == AnswerMethod::Reply {
            match reply_outcome(&ui::dialog(&reply_dialog(&reply, questions.len())).await?) {
                ReplyOutcome::Cancel => return Ok(()),
                ReplyOutcome::Send(t) => message = Some(t),
                ReplyOutcome::Questions => method = AnswerMethod::Questions,
            }
        }
        match method {
            AnswerMethod::Questions => {
                let answer = ui::dialog(&questions_dialog(&questions)).await?;
                let Some(text) = answer.get("value").and_then(Value::as_str) else {
                    return Ok(());
                };
                message = compose_answers(&questions, &parse_template(text, questions.len()), "");
            }
            AnswerMethod::Web => {
                let Some(answer) = self.web(&reply, &questions, settings.port).await? else {
                    return Ok(());
                };
                message = compose_answers(&questions, &answer.answers, &answer.note);
            }
            AnswerMethod::Reply => {}
        }
        let Some(message) = message else {
            ui::notify("Nothing answered — nothing sent.", "info")?;
            return Ok(());
        };
        let options = ws::UserMessageOptions {
            deliver_as: (!control::is_idle()).then_some(ws::Delivery::FollowUp),
            expand_prompt_templates: false,
            provenance: Some(ws::Provenance {
                source: "Answer".into(),
                title: Some("Your answers".into()),
                synopsis: Some(format!("{} question(s) answered", questions.len())),
                severity: None,
            }),
        };
        ws::send_user_message(json!(message).to_string(), options).await?;
        Ok(())
    }

    async fn web(
        &self,
        reply: &str,
        questions: &[String],
        setting: u16,
    ) -> Result<Option<WebAnswer>> {
        let ssh = is_ssh(&env);
        let listener = Listener::bind(choose_port(setting, ssh)).or_else(|_| Listener::bind(0))?;
        let port = listener.port();
        let token = unipi_kit::fmt::random_hex(24);
        let url = format!("http://127.0.0.1:{port}/a/{token}");
        let host = env("HOSTNAME").unwrap_or_else(|| "<this-host>".into());
        if !ssh {
            let _ = ui::open_url(&url);
        }
        let page = form_page(reply, questions, &token);
        let panel = ui::panel_open(&web_panel(&waiting_lines(&url, port, ssh, &host)))?;
        let (tx, mut rx) = oneshot::channel();
        self.waiters.borrow_mut().insert(panel.clone(), tx);
        let mut result = None;
        loop {
            let accept = listener.accept();
            futures::pin_mut!(accept);
            match futures::future::select(accept, &mut rx).await {
                futures::future::Either::Left((Some(req), _)) => {
                    let html = [
                        (
                            "content-type".to_owned(),
                            "text/html; charset=utf-8".to_owned(),
                        ),
                        ("cache-control".to_owned(), "no-store".to_owned()),
                    ];
                    match route(&req.method, &req.path, &req.body, &token) {
                        Reply::Page => listener.respond(req.id, 200, &html, page.as_bytes())?,
                        Reply::Submitted(a) => {
                            listener.respond(req.id, 200, &[], b"ok")?;
                            result = Some(a);
                            break;
                        }
                        Reply::Status(code) => listener.respond(req.id, code, &[], b"")?,
                    }
                }
                futures::future::Either::Left((None, _)) | futures::future::Either::Right(_) => {
                    break;
                }
            }
        }
        self.waiters.borrow_mut().remove(&panel);
        ui::panel_close(&panel);
        Ok(result)
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::assert_is_empty,
    clippy::format_collect,
    clippy::cloned_ref_to_slice_refs,
    clippy::duration_suboptimal_units,
    clippy::too_many_lines,
    clippy::needless_pass_by_value,
    reason = "tests assert by unwrapping"
)]
mod tests {
    use super::*;

    #[test]
    fn completions_and_methods() {
        assert_eq!(complete("").len(), 3);
        assert_eq!(
            complete("q").first().map(|c| c.value.as_str()),
            Some("questions")
        );
        assert!(complete("zzz").is_empty());
        assert_eq!(
            method_for("web", AnswerMethod::Reply, 0),
            AnswerMethod::Reply
        );
        assert_eq!(method_for("web", AnswerMethod::Reply, 2), AnswerMethod::Web);
        assert_eq!(
            method_for("editor", AnswerMethod::Reply, 2),
            AnswerMethod::Questions
        );
        assert_eq!(
            method_for("", AnswerMethod::Questions, 0),
            AnswerMethod::Questions
        );
        assert_eq!(
            hint_text(1),
            "  1 question in the reply — /answer questions to answer them one by one"
        );
    }

    #[test]
    fn reply_dialog_shape_and_outcomes() {
        let d = reply_dialog("hello?", 1);
        assert_eq!(d["method"], "custom");
        let kids = d["spec"]["children"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert!(
            kids.iter()
                .any(|k| k["t"] == "input" && k["onSubmit"] == "send")
        );
        assert!(kids.iter().any(|k| k["actionId"] == "questions"));
        assert!(
            !reply_dialog("x", 0)["spec"]["children"]
                .as_array()
                .is_some_and(|k| k.iter().any(|k| k["actionId"] == "questions"))
        );
        assert_eq!(
            reply_outcome(&json!({"value": {"actionId": "send", "value": " 8080\ntls "}})),
            ReplyOutcome::Send("8080\ntls".into())
        );
        assert_eq!(
            reply_outcome(&json!({"value": {"actionId": "send", "value": "  "}})),
            ReplyOutcome::Cancel
        );
        assert_eq!(
            reply_outcome(&json!({"value": {"actionId": "questions"}})),
            ReplyOutcome::Questions
        );
        assert_eq!(
            reply_outcome(&json!({"cancelled": true})),
            ReplyOutcome::Cancel
        );
    }

    #[test]
    fn questions_dialog_prefills_the_template() {
        let d = questions_dialog(&["Start?".into()]);
        assert_eq!(d["method"], "editor");
        assert_eq!(d["prefill"], "Q1. Start?\nA1: ");
        assert_eq!(d["cursorLine"], 1);
        assert!(
            questions_dialog(&[])["title"]
                .as_str()
                .is_some_and(|t| t.contains("no questions"))
        );
    }

    #[test]
    fn web_panel_shape() {
        let p = web_panel(&waiting_lines("http://x", 1, false, "h"));
        assert_eq!(p["panelId"], WEB_PANEL);
        assert_eq!(p["spec"]["children"].as_array().map(Vec::len), Some(3));
    }
}
