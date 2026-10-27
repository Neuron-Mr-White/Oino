//! `unipi.btw` — side questions that never reach the main session (unipi `packages/btw`,
//! unipi-port §3.12).
//!
//! | unipi | here |
//! |---|---|
//! | `/btw [question]` → panel replacing the editor | `editorReplace` panel `unipi.btw/panel` |
//! | fresh in-memory session seeded with the branch, tools `read, grep, find, ls`, main model + thinking | nested agent `seed: branchCopy`, `persistence: memory`, `extensions: none`, `ui: none` |
//! | streaming answer, tool lines (`… Reading p` → `✓ Read p`) | panel updates coalesced to ≤ 20/s |
//! | one answer at a time, history (↑/↓), `Ctrl+C` stop, `Esc` close (stream continues) | panel actions/keys; history in memory, cleared on `session_start` / `session_tree` / shutdown |
//! | print mode: answer as a notification | no UI ⇒ `notify` |
//! | `unipi.btw.api` (`ask`, `list`) | services `btw.ask`, `btw.list` |

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "tests assert by unwrapping"
    )
)]
#![allow(
    clippy::unused_async_trait_impl,
    reason = "`Extension::init` is async in the SDK; registration here is synchronous"
)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use futures::StreamExt;
use oino_extension::agents::Agent;
use oino_extension::prelude::*;
use oino_extension::sync::SessionMap;
use unipi_kit::ous::{dim, key_hints, span, text};
use unipi_kit::services::point;

/// Panel id.
pub const PANEL: &str = "unipi.btw/panel";
/// unipi `BTW_SYSTEM_PROMPT`.
pub const SYSTEM_PROMPT: &str = "You are having a one-shot aside conversation with the user, separate from their main working session. Main session messages are provided for context only — that work is being handled by another agent. Answer the question directly; there is no follow-up thread and nothing you say reaches the main session. This aside is read-only: you can read and search files but cannot run commands or modify anything. Never claim to have edited, written, or executed something. If the user wants changes or commands run, tell them to ask in the main conversation.";
const HINT_IDLE: &str = "↵ ask · ↑↓ scroll/history · esc back to chat";
const HINT_STREAMING: &str = "ctrl+c cancel answer · ↑↓ scroll/history · esc back to chat";
/// Minimum interval between panel updates while streaming (≤ 20/s).
const UPDATE_MS: i64 = 50;
const LIST_THINKING_TAIL: usize = 2000;

/// unipi `BtwPage`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Page {
    /// Id (`btw-<n>`).
    pub id: String,
    /// Question.
    pub question: String,
    /// Tool lines.
    pub tool_lines: Vec<String>,
    /// Final answer.
    pub answer: String,
    /// Streamed text.
    pub partial: String,
    /// Streamed thinking.
    pub thinking: String,
    /// Error.
    pub error: Option<String>,
    /// Aborted.
    pub aborted: bool,
    /// Finished.
    pub done: bool,
    /// Usage `{input, output, totalTokens}`.
    pub usage: Option<Value>,
}

/// unipi `pageIndexAfter`.
#[must_use]
pub fn page_index_after(current: usize, delta: i64, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    let max = i64::try_from(count - 1).unwrap_or(i64::MAX);
    let next = (i64::try_from(current).unwrap_or(0) + delta).clamp(0, max);
    usize::try_from(next).unwrap_or(0)
}

fn rel_path(p: &str, cwd: &str) -> String {
    if p == cwd {
        return ".".into();
    }
    p.strip_prefix(&format!("{cwd}/")).unwrap_or(p).to_owned()
}

/// unipi `formatToolLine`.
#[must_use]
pub fn format_tool_line(tool: &str, args: Option<&Value>, cwd: &str, state: &str) -> String {
    let verbs = match tool {
        "read" => Some(("Reading", "Read", "path")),
        "grep" => Some(("Searching", "Searched", "pattern")),
        "find" => Some(("Finding", "Found", "pattern")),
        "ls" => Some(("Listing", "Listed", "path")),
        _ => None,
    };
    let verb = if state == "running" {
        verbs.map_or("Running", |v| v.0)
    } else {
        verbs.map_or("Done", |v| v.1)
    };
    let get = |k: &str| args.and_then(|a| a.get(k)).and_then(Value::as_str);
    let path = get("path");
    let arg = verbs
        .and_then(|v| get(v.2))
        .or(path)
        .or_else(|| get("pattern"))
        .or_else(|| get("command"))
        .or_else(|| get("query"));
    let detail = arg.filter(|a| !a.is_empty()).map_or_else(String::new, |a| {
        if Some(a) == path {
            format!(" {}", rel_path(a, cwd))
        } else {
            format!(" {a}")
        }
    });
    let mark = match state {
        "running" => "…",
        "error" => "✗",
        _ => "✓",
    };
    let label = if verbs.is_some() {
        verb.to_owned()
    } else {
        format!("{verb} {tool}")
    };
    format!("{mark} {label}{detail}")
}

/// unipi `listPages` entry.
#[must_use]
pub fn list_page(p: &Page) -> Value {
    let mut v = json!({"id": p.id, "question": p.question, "answer": if p.done { &p.answer } else { &p.partial }, "done": p.done, "running": !p.done});
    if let Some(e) = &p.error {
        v["error"] = json!(e);
    }
    if !p.tool_lines.is_empty() {
        v["toolLines"] = json!(p.tool_lines);
    }
    if !p.done && !p.thinking.is_empty() {
        let n = p.thinking.chars().count();
        v["thinking"] = json!(
            p.thinking
                .chars()
                .skip(n.saturating_sub(LIST_THINKING_TAIL))
                .collect::<String>()
        );
    }
    v
}

/// Panel spec for page `index`.
#[must_use]
pub fn panel_spec(pages: &[Page], index: usize, streaming: bool) -> Value {
    let mut body: Vec<Value> = Vec::new();
    if let Some(p) = pages.get(index) {
        body.push(text(json!([span(
            &format!("❭ {}", p.question),
            Some("accent")
        )])));
        for l in &p.tool_lines {
            body.push(text(json!([dim(if l.starts_with('…') {
                l.clone()
            } else {
                format!("  {l}")
            }
            .as_str())])));
        }
        if !p.done {
            body.push(text(json!([dim("Thinking..")])));
            if !p.partial.is_empty() {
                body.push(json!({"t": "markdown", "value": p.partial}));
            }
        } else if let Some(e) = &p.error {
            body.push(text(json!([span(&format!("✗ {e}"), Some("error"))])));
        }
        if !p.answer.is_empty() {
            body.push(json!({"t": "markdown", "value": p.answer}));
        }
    }
    let title = format!(" /btw {}/{} ", (index + 1).min(pages.len()), pages.len());
    let input = if streaming {
        text(json!([dim("❭ Waiting for the answer…")]))
    } else {
        json!({"t": "input", "id": "question", "placeholder": "Ask a /btw…", "onSubmit": "ask"})
    };
    json!({"t": "stack", "direction": "vertical", "gap": 0, "children": [
        {"t": "divider", "label": title},
        {"t": "stack", "direction": "vertical", "gap": 0, "children": body},
        input,
        {"t": "divider"},
        key_hints(&if streaming {
            [("ctrl+c", "cancel answer"), ("↑↓", "scroll/history"), ("esc", "back to chat")]
        } else {
            [("↵", "ask"), ("↑↓", "scroll/history"), ("esc", "back to chat")]
        }),
        text(json!([dim(if streaming { HINT_STREAMING } else { HINT_IDLE })])),
    ]})
}

#[derive(Default)]
struct State {
    pages: Vec<Page>,
    index: usize,
    panel: Option<String>,
    active: Option<Rc<Agent>>,
    last_update: i64,
}

/// The extension.
#[derive(Default)]
pub struct Btw {
    sessions: SessionMap<State>,
    seq: Cell<u64>,
    busy: RefCell<std::collections::HashSet<String>>,
}

impl Btw {
    fn update_panel(&self, sid: &str, force: bool) {
        let now = timers::now_ms();
        let (panel, spec) = self.sessions.with(sid, |s| {
            if !force && now - s.last_update < UPDATE_MS {
                return (None, Value::Null);
            }
            s.last_update = now;
            (
                s.panel.clone(),
                panel_spec(&s.pages, s.index, s.active.is_some()),
            )
        });
        if let Some(id) = panel {
            let _ = ui::panel_update(&id, &json!({"spec": spec}));
        }
    }

    fn page_mut(&self, sid: &str, index: usize, f: impl FnOnce(&mut Page)) {
        self.sessions.with(sid, |s| {
            if let Some(p) = s.pages.get_mut(index) {
                f(p);
            }
        });
    }

    /// unipi `runQuestion`: one read-only nested session per question.
    #[allow(clippy::too_many_lines, reason = "one event loop as unipi runQuestion")]
    async fn run(&self, sid: &str, cwd: &str, question: &str) -> usize {
        let n = self.seq.get() + 1;
        self.seq.set(n);
        let index = self.sessions.with(sid, |s| {
            s.pages.push(Page {
                id: format!("btw-{n}"),
                question: question.to_owned(),
                ..Page::default()
            });
            s.index = s.pages.len() - 1;
            s.pages.len() - 1
        });
        let model = control::model().map(|m| {
            format!(
                "{}/{}",
                m.get("provider")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                m.get("id").and_then(Value::as_str).unwrap_or_default()
            )
        });
        let Some(model) = model else {
            self.page_mut(sid, index, |p| {
                p.error = Some("No active model selected.".into());
                p.done = true;
            });
            self.update_panel(sid, true);
            return index;
        };
        let spec = json!({
            "label": "btw", "model": model, "thinking": control::thinking_level(),
            "systemPrompt": {"append": SYSTEM_PROMPT},
            "tools": {"builtin": ["read", "grep", "find", "ls"], "extensionTools": [], "deny": []},
            "extensions": "none", "seed": "branchCopy", "persistence": "memory", "ui": "none", "childRole": "btw",
        });
        let agent = match Agent::spawn(&spec).await {
            Ok(a) => Rc::new(a),
            Err(e) => {
                self.page_mut(sid, index, |p| {
                    p.error = Some(e.to_string());
                    p.done = true;
                });
                self.update_panel(sid, true);
                return index;
            }
        };
        self.sessions
            .with(sid, |s| s.active = Some(Rc::clone(&agent)));
        self.update_panel(sid, true);
        let mut events = agent.events();
        let mut started: Vec<(String, Option<Value>, usize)> = Vec::new();
        let mut last: Option<Value> = None;
        let prompt = agent.prompt(question).await;
        if let Err(e) = prompt {
            self.page_mut(sid, index, |p| p.error = Some(e.to_string()));
        } else {
            while let Some(ev) = events.next().await {
                let ty = ev.get("type").and_then(Value::as_str).unwrap_or_default();
                let id = ev
                    .get("toolCallId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let tool = ev
                    .get("toolName")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                match ty {
                    "tool_execution_start" => {
                        let args = ev.get("args").cloned();
                        let line = format_tool_line(&tool, args.as_ref(), cwd, "running");
                        self.sessions.with(sid, |s| {
                            if let Some(p) = s.pages.get_mut(index) {
                                started.push((id, args, p.tool_lines.len()));
                                p.tool_lines.push(line);
                            }
                        });
                    }
                    "tool_execution_end" => {
                        let err = ev.get("isError").and_then(Value::as_bool).unwrap_or(false);
                        let slot = started.iter().find(|(i, _, _)| *i == id).cloned();
                        let line = format_tool_line(
                            &tool,
                            slot.as_ref().and_then(|s| s.1.as_ref()),
                            cwd,
                            if err { "error" } else { "done" },
                        );
                        self.page_mut(sid, index, |p| {
                            match slot.and_then(|s| p.tool_lines.get_mut(s.2)) {
                                Some(l) => *l = line,
                                None => p.tool_lines.push(line),
                            }
                        });
                    }
                    "message_update" => {
                        let se = ev.get("assistantMessageEvent").cloned().unwrap_or_default();
                        let delta = se
                            .get("delta")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        match se.get("type").and_then(Value::as_str) {
                            Some("text_delta") => {
                                self.page_mut(sid, index, |p| p.partial.push_str(&delta));
                            }
                            Some("thinking_delta") => {
                                self.page_mut(sid, index, |p| p.thinking.push_str(&delta));
                            }
                            _ => {}
                        }
                    }
                    "message_end" => {
                        if ev.pointer("/message/role").and_then(Value::as_str) == Some("assistant")
                        {
                            last = ev.get("message").cloned();
                        }
                    }
                    "agent_end" | "agent_settled" => break,
                    _ => {}
                }
                self.update_panel(sid, false);
            }
        }
        self.page_mut(sid, index, |p| finish_page(p, last.as_ref()));
        self.sessions.with(sid, |s| s.active = None);
        if let Ok(a) = Rc::try_unwrap(agent) {
            a.close(0).await;
        }
        self.update_panel(sid, true);
        index
    }

    fn abort(&self, sid: &str) {
        if let Some(a) = self.sessions.with(sid, |s| s.active.clone()) {
            a.abort();
        }
    }

    fn clear(&self, sid: &str) {
        self.abort(sid);
        self.sessions.with(sid, |s| {
            s.pages.clear();
            s.index = 0;
        });
    }

    fn start(&'static self, cx: &Ctx, question: String) {
        let sid = cx.session_id().to_owned();
        if self.sessions.with(&sid, |s| s.active.is_some())
            || !self.busy.borrow_mut().insert(sid.clone())
        {
            return;
        }
        let cwd = cx.cwd().to_string_lossy().into_owned();
        spawn(async move {
            self.run(&sid, &cwd, &question).await;
            self.busy.borrow_mut().remove(&sid);
        });
    }

    fn open(&'static self, cx: &Ctx, question: Option<String>) {
        let sid = cx.session_id().to_owned();
        let spec = self.sessions.with(&sid, |s| {
            s.index = s.pages.len().saturating_sub(1);
            panel_spec(&s.pages, s.index, s.active.is_some())
        });
        let req = json!({
            "panelId": PANEL, "placement": "editorReplace", "title": "/btw", "spec": spec, "focus": true, "closeOnEscape": true,
            "keys": [
                {"key": "ctrl+c", "actionId": "cancel", "label": "cancel answer"},
                {"key": "up", "actionId": "prev", "label": "history"},
                {"key": "down", "actionId": "next", "label": "history"},
            ],
        });
        match ui::panel_open(&req) {
            Ok(id) => self.sessions.with(&sid, |s| s.panel = Some(id)),
            Err(e) => {
                let _ = ui::notify(&e.to_string(), "error");
                return;
            }
        }
        if let Some(q) = question {
            self.start(cx, q);
        }
    }

    fn on_action(&'static self, ev: &Event, cx: &Ctx) {
        let sid = cx.session_id().to_owned();
        let panel = ev
            .get("panelId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if self
            .sessions
            .with(&sid, |s| s.panel.as_deref() != Some(panel))
            && panel != PANEL
        {
            return;
        }
        match ev
            .get("actionId")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "$close" => self.sessions.with(&sid, |s| s.panel = None),
            "cancel" => {
                if self.sessions.with(&sid, |s| s.active.is_some()) {
                    self.abort(&sid);
                } else if let Some(id) = self.sessions.with(&sid, |s| s.panel.take()) {
                    ui::panel_close(&id);
                }
            }
            "prev" | "next" => {
                let delta = if ev.get("actionId").and_then(Value::as_str) == Some("prev") {
                    -1
                } else {
                    1
                };
                self.sessions.with(&sid, |s| {
                    s.index = page_index_after(s.index, delta, s.pages.len());
                });
                self.update_panel(&sid, true);
            }
            "ask" => {
                let q = ev
                    .get("value")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
                if !q.is_empty() {
                    self.start(cx, q);
                }
            }
            _ => {}
        }
    }

    async fn command(&'static self, args: &str, cx: &CommandCtx) {
        let question = args.trim().to_owned();
        if cx.has_ui() {
            self.open(&cx.cx, (!question.is_empty()).then_some(question));
            return;
        }
        if question.is_empty() {
            let _ = ui::notify("usage: /btw <question>", "warning");
            return;
        }
        let _ = ui::notify(&format!("btw: {question}"), "info");
        let cwd = cx.cx.cwd().to_string_lossy().into_owned();
        let index = self.run(cx.session_id(), &cwd, &question).await;
        let page = self
            .sessions
            .with(cx.session_id(), |s| s.pages.get(index).cloned())
            .unwrap_or_default();
        let _ = match &page.error {
            Some(e) => ui::notify(&format!("btw failed: {e}"), "error"),
            None => ui::notify(&format!("btw: {}", page.answer), "info"),
        };
    }

    /// Service `btw.ask {question}` → `{id, answer, error?, usage?}` (one question at a time).
    async fn ask_service(&self, payload: &Value, cx: &Ctx) -> Value {
        let sid = cx.session_id().to_owned();
        let question = payload
            .get("question")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned();
        if self.sessions.with(&sid, |s| s.active.is_some()) || self.busy.borrow().contains(&sid) {
            return json!({"answer": "", "error": "btw is already answering another question"});
        }
        self.busy.borrow_mut().insert(sid.clone());
        let cwd = cx.cwd().to_string_lossy().into_owned();
        let index = self.run(&sid, &cwd, &question).await;
        self.busy.borrow_mut().remove(&sid);
        let p = self
            .sessions
            .with(&sid, |s| s.pages.get(index).cloned())
            .unwrap_or_default();
        json!({"id": p.id, "answer": p.answer, "error": p.error, "usage": p.usage})
    }
}

/// Final page state from the last assistant message (unipi `runQuestion` tail).
pub fn finish_page(p: &mut Page, last: Option<&Value>) {
    p.done = true;
    if p.error.is_some() {
        return;
    }
    let Some(m) = last else {
        p.error = Some("No response.".into());
        return;
    };
    match m.get("stopReason").and_then(Value::as_str) {
        Some("aborted") => {
            p.aborted = true;
            p.error = Some("cancelled".into());
        }
        Some("error") => {
            p.error = Some(
                m.get("errorMessage")
                    .and_then(Value::as_str)
                    .filter(|e| !e.is_empty())
                    .unwrap_or("request failed")
                    .to_owned(),
            );
        }
        _ => {
            let text: Vec<&str> = m
                .get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|c| c.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|c| c.get("text").and_then(Value::as_str))
                .collect();
            let joined = text.join("\n").trim().to_owned();
            p.answer = if joined.is_empty() {
                "(no text)".into()
            } else {
                joined
            };
            if let Some(u) = m.get("usage") {
                p.usage = Some(
                    json!({"input": u.get("input"), "output": u.get("output"), "totalTokens": u.get("totalTokens")}),
                );
            }
        }
    }
}

fn spawn(fut: impl Future<Output = ()> + 'static) {
    #[cfg(target_arch = "wasm32")]
    oino_extension::wit_bindgen::spawn_local(fut);
    #[cfg(not(target_arch = "wasm32"))]
    drop(fut);
}

impl Extension for Btw {
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        cx.command(
            Command::<Self>::new("btw")
                .description("Ask a side question in an inline panel — read-only, never reaches the main agent.")
                .argument_hint("[question]")
                .run(|ext, args, cx| async move {
                    ext.command(&args, &cx).await;
                    Ok(())
                })
                .complete(|_e, _p, _c| async { Vec::new() }),
        );
        for kind in [EventKind::SessionStart, EventKind::SessionTree] {
            cx.on(kind, |ext, _ev, cx| async move {
                ext.clear(cx.session_id());
                Ok(())
            });
        }
        cx.on(EventKind::SessionShutdown, |ext, _ev, cx| async move {
            ext.clear(cx.session_id());
            ext.sessions.remove(cx.session_id());
            Ok(())
        });
        cx.on(EventKind::OinoUiAction, |ext, ev, cx| async move {
            ext.on_action(&ev, &cx);
            Ok(())
        });
        cx.service(point::BTW_ASK, |ext, p, _caller, cx| async move {
            Ok(ext.ask_service(&p, &cx).await)
        });
        cx.service("btw.list", |ext, _p, _caller, cx| async move {
            Ok(Value::Array(ext.sessions.with(cx.session_id(), |s| {
                s.pages.iter().map(list_page).collect()
            })))
        });
        Ok(())
    }
}

// Native builds (unit tests) skip the component exports.
#[cfg(target_arch = "wasm32")]
oino_extension::export_extension!(Btw);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_lines_and_paging() {
        assert_eq!(
            format_tool_line(
                "read",
                Some(&json!({"path": "/w/src/a.rs"})),
                "/w",
                "running"
            ),
            "… Reading src/a.rs"
        );
        assert_eq!(
            format_tool_line("grep", Some(&json!({"pattern": "fn x"})), "/w", "done"),
            "✓ Searched fn x"
        );
        assert_eq!(
            format_tool_line("ls", Some(&json!({"path": "/w"})), "/w", "error"),
            "✗ Listed ."
        );
        assert_eq!(
            format_tool_line("web", Some(&json!({"query": "q"})), "/w", "done"),
            "✓ Done web q"
        );
        assert_eq!(page_index_after(0, -1, 3), 0);
        assert_eq!(page_index_after(1, 1, 3), 2);
        assert_eq!(page_index_after(2, 1, 3), 2);
        assert_eq!(page_index_after(5, 1, 0), 0);
    }

    #[test]
    fn pages_finish_and_list() {
        let mut p = Page {
            id: "btw-1".into(),
            question: "q".into(),
            partial: "par".into(),
            thinking: "t".into(),
            ..Page::default()
        };
        assert_eq!(list_page(&p)["answer"], "par");
        assert_eq!(list_page(&p)["thinking"], "t");
        finish_page(
            &mut p,
            Some(
                &json!({"stopReason": "stop", "content": [{"type": "text", "text": " A "}], "usage": {"input": 1, "output": 2, "totalTokens": 3}}),
            ),
        );
        assert_eq!((p.answer.as_str(), p.done), ("A", true));
        assert_eq!(list_page(&p)["running"], false);
        let mut a = Page::default();
        finish_page(&mut a, Some(&json!({"stopReason": "aborted"})));
        assert_eq!((a.error.as_deref(), a.aborted), (Some("cancelled"), true));
        let mut n = Page::default();
        finish_page(&mut n, None);
        assert_eq!(n.error.as_deref(), Some("No response."));
        let spec = panel_spec(&[p], 0, false);
        assert_eq!(spec["children"][0]["label"], " /btw 1/1 ");
        assert_eq!(spec["children"][2]["onSubmit"], "ask");
        assert_eq!(panel_spec(&[], 0, true)["children"][2]["t"], "text");
    }
}
