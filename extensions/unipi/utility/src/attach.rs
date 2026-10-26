//! Image and file attachments (unipi `src/attach/index.ts`).
//!
//! After a paste or drop, any image/document path that lands in the editor is swapped for a
//! token — `[Image #1]`, `[File #2]` — and listed in a chip row above the editor (with small
//! inline previews where the frontend draws images). On submit, images are sent as image
//! content and file tokens expand to their paths. Deleting a token drops the attachment.
//! After the message is sent, a `unipi:attachments` entry records what went with it (shown
//! to the user, never to the model).
//!
//! Oino mechanism: `oino_editor` intents replace pi's `onTerminalInput` (`changed` → scan,
//! `tokenBackspace` → atomic token delete), `ui.editor-set` rewrites the text, the `input`
//! event transform expands tokens.

use std::cell::RefCell;
use std::sync::LazyLock;

use oino_extension::prelude::*;
use oino_extension::sync::SessionMap;
use regex::Regex;
use unipi_kit::attach::{self, Attachment, Kind};

use crate::hostfs::Roots;

/// Custom entry recording a message's attachments.
pub const ENTRY: &str = "unipi:attachments";
/// Chip-row widget.
pub const WIDGET: &str = "unipi-attachments";
const PREVIEW_MAX: usize = 3;

static TRAILING_TOKEN: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"\[(?:Image|File) #\d+\] ?$").ok());

#[derive(Default)]
struct Sess {
    pending: Vec<Attachment>,
    sent: Vec<Attachment>,
}

/// Attachment state per session.
#[derive(Default)]
pub struct Attachments {
    sessions: SessionMap<Sess>,
    roots: RefCell<Option<Roots>>,
}

/// Serialisable attachment (entry data, renderer input).
#[must_use]
pub fn to_json(a: &Attachment) -> Value {
    json!({
        "id": a.id,
        "kind": if a.kind == Kind::Image { "image" } else { "file" },
        "name": a.name,
        "path": a.path,
        "bytes": a.bytes,
        "mimeType": a.mime_type,
    })
}

/// Chip text of one attachment.
#[must_use]
pub fn chip(kind: &str, id: u64, name: &str, bytes: u64) -> Vec<Value> {
    let token = if kind == "image" {
        format!("[Image #{id}]")
    } else {
        format!("[File #{id}]")
    };
    let mut spans = vec![
        json!({"text": token, "tone": "accent"}),
        json!({"text": format!(" {name}")}),
    ];
    if bytes > 0 {
        spans.push(
            json!({"text": format!(" · {}", unipi_kit::fmt::format_bytes(bytes)), "tone": "muted"}),
        );
    }
    spans
}

/// Chip row + optional previews (widget and transcript entry share it).
#[must_use]
pub fn view(items: &[Value], preview: bool, hint: &str) -> Value {
    let mut spans = vec![json!({"text": "▌ ", "tone": "accent"})];
    for (i, a) in items.iter().enumerate() {
        if i > 0 {
            spans.push(json!({"text": "  ·  ", "tone": "muted"}));
        }
        spans.extend(chip(
            a.get("kind").and_then(Value::as_str).unwrap_or("file"),
            a.get("id").and_then(Value::as_u64).unwrap_or(0),
            a.get("name").and_then(Value::as_str).unwrap_or_default(),
            a.get("bytes").and_then(Value::as_u64).unwrap_or(0),
        ));
    }
    if !hint.is_empty() {
        spans.push(json!({"text": format!("   {hint}"), "tone": "muted"}));
    }
    let mut children = vec![json!({"t": "text", "value": spans, "wrap": false})];
    if preview {
        for a in items
            .iter()
            .filter(|a| a.get("kind").and_then(Value::as_str) == Some("image"))
            .take(PREVIEW_MAX)
        {
            children.push(json!({
                "t": "image",
                "path": a.get("path"),
                "mimeType": a.get("mimeType"),
                "alt": a.get("name"),
                "maxWidth": 32,
            }));
        }
    }
    if children.len() == 1 {
        return children.remove(0);
    }
    json!({"t": "stack", "direction": "vertical", "children": children})
}

/// Plain chip line (frontends without OUS).
#[must_use]
pub fn line(items: &[Value], hint: &str) -> String {
    let chips: Vec<String> = items
        .iter()
        .map(|a| {
            let kind = a.get("kind").and_then(Value::as_str).unwrap_or("file");
            let id = a.get("id").and_then(Value::as_u64).unwrap_or(0);
            let token = if kind == "image" {
                format!("[Image #{id}]")
            } else {
                format!("[File #{id}]")
            };
            let name = a.get("name").and_then(Value::as_str).unwrap_or_default();
            match a.get("bytes").and_then(Value::as_u64).filter(|b| *b > 0) {
                Some(b) => format!("{token} {name} · {}", unipi_kit::fmt::format_bytes(b)),
                None => format!("{token} {name}"),
            }
        })
        .collect();
    let mut l = format!("▌ {}", chips.join("  ·  "));
    if !hint.is_empty() {
        l.push_str("   ");
        l.push_str(hint);
    }
    l
}

/// Text with one trailing token removed (atomic backspace), or `None`.
#[must_use]
pub fn delete_trailing_token(text: &str) -> Option<String> {
    let m = TRAILING_TOKEN.as_ref()?.find(text)?;
    Some(text[..m.start()].to_owned())
}

impl Attachments {
    fn roots(&self) -> Roots {
        self.roots
            .borrow_mut()
            .get_or_insert_with(Roots::current)
            .clone()
    }

    fn guest(&self, host: &str) -> Option<std::path::PathBuf> {
        self.roots().to_guest(host)
    }

    fn exists(&self, host: &str) -> bool {
        self.guest(host)
            .is_some_and(|g| std::fs::metadata(g).is_ok_and(|m| m.is_file()))
    }

    fn size(&self, host: &str) -> u64 {
        self.guest(host)
            .and_then(|g| std::fs::metadata(g).ok())
            .map_or(0, |m| m.len())
    }

    fn read(&self, host: &str) -> Option<Vec<u8>> {
        std::fs::read(self.guest(host)?).ok()
    }

    /// `session_start`.
    pub fn reset(&self, sid: &str) {
        self.sessions.remove(sid);
        *self.roots.borrow_mut() = None;
        let _ = ui::widget(WIDGET, None);
    }

    fn refresh_widget(&self, sid: &str, preview: bool) {
        let items: Vec<Value> = self
            .sessions
            .with(sid, |s| s.pending.iter().map(to_json).collect());
        if items.is_empty() {
            let _ = ui::widget(WIDGET, None);
            return;
        }
        let hint = "attached to your next message";
        let _ = ui::fire(&json!({
            "method": "setWidget",
            "widgetKey": WIDGET,
            "widgetLines": [line(&items, hint)],
            "widgetPlacement": "aboveEditor",
            "widgetSpec": view(&items, preview, hint),
        }));
    }

    /// `oino_editor`: scan on change, keep the list in sync, atomic token backspace.
    pub fn on_editor(&self, sid: &str, ev: &Event, preview: bool) -> Option<Value> {
        let intent = ev.get("intent").and_then(Value::as_str).unwrap_or_default();
        let text = ev
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        match intent {
            "tokenBackspace" => {
                let next = delete_trailing_token(&text)?;
                ui::editor_set(&next);
                self.sync(sid, &next, preview);
                Some(json!({"consume": true}))
            }
            "changed" | "cleared" => {
                let existing = self.sessions.with(sid, |s| s.pending.clone());
                let home = crate::hostfs::home();
                let (next, added) = attach::tokenize(
                    &text,
                    &existing,
                    home.as_deref(),
                    &|p| self.exists(p),
                    &|p| self.size(p),
                );
                if next != text {
                    ui::editor_set(&next);
                }
                if added.is_empty() {
                    self.sync(sid, &next, preview);
                } else {
                    self.sessions.with(sid, |s| s.pending.extend(added));
                    self.refresh_widget(sid, preview);
                }
                None
            }
            _ => None,
        }
    }

    fn sync(&self, sid: &str, text: &str, preview: bool) {
        let changed = self.sessions.with(sid, |s| {
            let kept: Vec<Attachment> = attach::still_referenced(text, &s.pending)
                .into_iter()
                .cloned()
                .collect();
            let changed = kept.len() != s.pending.len();
            s.pending = kept;
            changed
        });
        if changed {
            self.refresh_widget(sid, preview);
        }
    }

    /// `input`: expand tokens into image content / `[File #N: path]`.
    pub fn on_input(&self, sid: &str, ev: &Event, preview: bool) -> Option<Value> {
        if ev.get("source").and_then(Value::as_str) == Some("extension") {
            return None;
        }
        let text = ev.get("text").and_then(Value::as_str).unwrap_or_default();
        let pending = self.sessions.with(sid, |s| s.pending.clone());
        if pending.is_empty() || text.trim_start().starts_with('/') {
            return None;
        }
        let used: Vec<Attachment> = attach::still_referenced(text, &pending)
            .into_iter()
            .cloned()
            .collect();
        if used.is_empty() {
            return None;
        }
        let (out, parts) = attach::expand_for_submit(text, &used, &|p| self.read(p));
        self.sessions.with(sid, |s| {
            s.sent = used;
            s.pending.clear();
        });
        self.refresh_widget(sid, preview);
        let mut images: Vec<Value> = ev
            .get("images")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        images.extend(
            parts
                .into_iter()
                .map(|p| json!({"type": "image", "data": p.data, "mimeType": p.mime_type})),
        );
        Some(json!({"action": "transform", "text": out, "images": images}))
    }

    /// `agent_start`: record what went with the message.
    pub fn on_agent_start(&self, sid: &str) {
        let sent = self.sessions.with(sid, |s| std::mem::take(&mut s.sent));
        if sent.is_empty() {
            return;
        }
        let items: Vec<Value> = sent.iter().map(to_json).collect();
        let _ = session::append_entry(ENTRY, Some(&json!({"items": items})));
    }

    /// Entry renderer.
    #[must_use]
    pub fn render_entry(data: &Value, preview: bool) -> Option<Value> {
        let items = data
            .get("items")
            .and_then(Value::as_array)
            .filter(|a| !a.is_empty())?;
        Some(view(items, preview, ""))
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
    fn trailing_token_backspace() {
        assert_eq!(
            delete_trailing_token("see [Image #1]").as_deref(),
            Some("see ")
        );
        assert_eq!(
            delete_trailing_token("see [File #12] ").as_deref(),
            Some("see ")
        );
        assert_eq!(delete_trailing_token("[Image #1] then text"), None);
    }

    #[test]
    fn chip_view_and_line() {
        let items = vec![
            json!({"id": 1, "kind": "image", "name": "a.png", "path": "/x/a.png", "bytes": 2048, "mimeType": "image/png"}),
            json!({"id": 2, "kind": "file", "name": "b.pdf", "path": "/x/b.pdf", "bytes": 0}),
        ];
        assert_eq!(
            line(&items, "hint"),
            "▌ [Image #1] a.png · 2 KB  ·  [File #2] b.pdf   hint"
        );
        let v = view(&items, true, "");
        assert_eq!(v["t"], "stack");
        assert_eq!(v["children"][1]["t"], "image");
        assert_eq!(v["children"][1]["path"], "/x/a.png");
        assert_eq!(view(&items, false, "")["t"], "text");
        assert!(Attachments::render_entry(&json!({"items": []}), true).is_none());
        assert!(Attachments::render_entry(&json!({"items": items}), false).is_some());
    }

    #[test]
    fn attachment_json() {
        let a = Attachment {
            id: 3,
            kind: Kind::Image,
            path: "/p/x.png".into(),
            name: "x.png".into(),
            bytes: 9,
            mime_type: Some("image/png"),
        };
        let j = to_json(&a);
        assert_eq!(j["kind"], "image");
        assert_eq!(j["mimeType"], "image/png");
        assert_eq!(j["id"], 3);
    }
}
