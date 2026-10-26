//! Pane keyboard tables (design.md §4 "Keys", §1.7 keyboard first):
//!
//! | Where | Keys |
//! |---|---|
//! | Timeline (focus it with ⌘↑ from the composer or a click) | ↑/↓ or k/j move the row cursor, Home/End, ⌘↑/⌘↓ previous/next prompt, Enter/Space expand, `c` copy, `o` open in the side pane, Esc / `i` back to the composer |
//! | Side tabs (anywhere in the pane) | ⌘1–⌘9 pick a tab, Ctrl+Tab / Ctrl+Shift+Tab cycle, ⌘W close |
//! | Review / Diff (side pane focused) | n/p or ]/[ next/previous file |

use gpui_kit::{ClipboardItem, Context, KeyDownEvent, Window};

use crate::components::secondary;
use crate::model::tools;
use crate::model::transcript::RowKind;
use crate::views::session_pane::{PaneEvent, SessionPane, SideTab};

/// Next index from `from` by `dir`, clamped to `0..n`.
#[must_use]
pub fn step(from: Option<usize>, dir: isize, n: usize) -> Option<usize> {
    if n == 0 {
        return None;
    }
    let Some(f) = from else {
        return Some(if dir < 0 { n - 1 } else { 0 });
    };
    Some(f.saturating_add_signed(dir).min(n - 1))
}

impl SessionPane {
    fn move_cursor(&mut self, to: Option<usize>, cx: &mut Context<Self>) {
        let old = self.cursor;
        self.cursor = to;
        for ix in [old, to].into_iter().flatten() {
            if ix < self.model.transcript.len() {
                self.list.remeasure_items(ix..ix + 1);
            }
        }
        if let Some(ix) = to {
            self.list.scroll_to_reveal_item(ix);
        }
        cx.notify();
    }

    /// Moves focus from the composer to the timeline (cursor on the newest row).
    pub fn focus_timeline(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let n = self.model.transcript.len();
        let to = self.cursor.filter(|c| *c < n).or(n.checked_sub(1));
        self.move_cursor(to, cx);
    }

    /// Pane-wide keys (side tabs) and the timeline cursor keys while the timeline has focus.
    pub(crate) fn on_pane_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let k = &event.keystroke;
        let primary = secondary(&k.modifiers);
        // Side tabs: ⌘1–9, Ctrl+Tab, ⌘W.
        if primary && !k.modifiers.shift && !k.modifiers.alt {
            if let Some(d) = k.key.chars().next().and_then(|c| c.to_digit(10))
                && k.key.len() == 1
                && d >= 1
                && self.side.is_open()
            {
                let i = (d as usize - 1).min(self.side.tabs.len() - 1);
                self.side.active = i;
                cx.emit(PaneEvent::ShowSide);
                cx.stop_propagation();
                cx.notify();
                return;
            }
            if k.key == "w" && self.side.is_open() {
                let i = self.side.active;
                #[cfg(feature = "browser")]
                if self.side.current() == Some(&SideTab::Browser) {
                    self.browser = None;
                }
                if let Some(SideTab::Panel(id)) = self.side.tabs.get(i).cloned() {
                    self.close_panel(&id, cx);
                } else {
                    self.side.close_at(i);
                }
                cx.stop_propagation();
                cx.notify();
                return;
            }
        }
        if k.modifiers.control && k.key == "tab" && self.side.tabs.len() > 1 {
            let n = self.side.tabs.len();
            self.side.active = if k.modifiers.shift {
                (self.side.active + n - 1) % n
            } else {
                (self.side.active + 1) % n
            };
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if self.side_focus.is_focused(window) {
            self.on_side_key(event, cx);
            return;
        }
        if !self.focus.is_focused(window) {
            return;
        }
        let n = self.model.transcript.len();
        let rows = self.model.transcript.rows();
        let users: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| matches!(r.kind, RowKind::User(_)))
            .map(|(i, _)| i)
            .collect();
        let cur = self.cursor;
        let next = match k.key.as_str() {
            "up" if primary => cur.and_then(|c| users.iter().rev().find(|u| **u < c).copied()),
            "down" if primary => cur.and_then(|c| users.iter().find(|u| **u > c).copied()),
            "up" | "k" => step(cur, -1, n),
            "down" | "j" => step(cur, 1, n),
            "home" => step(None, 1, n),
            "end" => step(None, -1, n),
            "pageup" => step(cur, -10, n),
            "pagedown" => step(cur, 10, n),
            "enter" | "space" => {
                if let Some(ix) = cur {
                    self.activate_row(ix, cx);
                }
                cx.stop_propagation();
                return;
            }
            "c" => {
                if let Some(text) = cur.and_then(|ix| row_text(self, ix)) {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                    cx.emit(PaneEvent::Toast("Copied".into()));
                }
                cx.stop_propagation();
                return;
            }
            "o" => {
                if let Some(ix) = cur {
                    self.open_row(ix, cx);
                }
                cx.stop_propagation();
                return;
            }
            "escape" | "i" => {
                self.move_cursor(None, cx);
                Self::focus(&cx.entity(), window, cx);
                cx.stop_propagation();
                return;
            }
            _ => return,
        };
        if next.is_some() {
            self.move_cursor(next, cx);
        }
        cx.stop_propagation();
    }

    /// Enter on a row: expand / collapse it.
    fn activate_row(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(row) = self.model.transcript.rows().get(ix) else {
            return;
        };
        let id = row.id.clone();
        match &row.kind {
            RowKind::Tool(_) | RowKind::Summary(_) => self.toggle_row(&id, cx),
            RowKind::Assistant(a) if !a.thinking.is_empty() => {
                if !self.thinking_open.remove(&id) {
                    self.thinking_open.insert(id);
                }
                self.list.remeasure_items(ix..ix + 1);
                cx.notify();
            }
            _ => {}
        }
        if let Some((start, _, _)) =
            crate::model::timeline::explore_run(self.model.transcript.rows(), ix)
        {
            let key = self.model.transcript.rows()[start].id.clone();
            if !self.explore_open.remove(&key) {
                self.explore_open.insert(key);
            }
            self.list.remeasure();
            cx.notify();
        }
    }

    /// `o` on a row: its diff / output / agent in the side pane.
    fn open_row(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(row) = self.model.transcript.rows().get(ix) else {
            return;
        };
        let tab = match &row.kind {
            RowKind::Tool(tool) => {
                if let Some(patch) = tools::patch_of(tool.details.as_ref()) {
                    let title = tool
                        .args
                        .get("path")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or(&tool.name)
                        .to_owned();
                    SideTab::Diff { title, patch }
                } else if crate::model::timeline::agent_line(tool).is_some() {
                    SideTab::Agent {
                        call_id: tool.call_id.clone(),
                        title: tool.name.clone(),
                    }
                } else {
                    SideTab::Output {
                        title: tool.name.clone(),
                        text: tool.output.clone(),
                    }
                }
            }
            RowKind::Bash(b) => SideTab::Output {
                title: b.command.clone(),
                text: b.output.clone(),
            },
            _ => return,
        };
        self.open_side(tab, cx);
    }

    /// Review / Diff keys while the side pane has focus.
    fn on_side_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let review = matches!(
            self.side.current(),
            Some(SideTab::Review | SideTab::Diff { .. })
        );
        if !review {
            return;
        }
        let files = crate::model::timeline::file_changes(self.model.transcript.rows()).len();
        let dir: isize = match event.keystroke.key.as_str() {
            "n" | "]" | "down" | "j" => 1,
            "p" | "[" | "up" | "k" => -1,
            _ => return,
        };
        let to = step(self.diff_cursor, dir, files.max(1));
        self.diff_cursor = to;
        if let Some(i) = to {
            // Child 0 is the summary line; file cards follow.
            self.side_scroll.scroll_to_item(i + 1);
        }
        cx.stop_propagation();
        cx.notify();
    }
}

fn row_text(pane: &SessionPane, ix: usize) -> Option<String> {
    let row = pane.model.transcript.rows().get(ix)?;
    Some(match &row.kind {
        RowKind::User(u) => u.text.clone(),
        RowKind::Assistant(a) => a.text.clone(),
        RowKind::Tool(t) => t.output.clone(),
        RowKind::Bash(b) => format!("$ {}\n{}", b.command, b.output),
        RowKind::Custom(c) => format!("{c:?}"),
        RowKind::Summary(s) => format!("{s:?}"),
        RowKind::Notice(_, text) => text.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::step;

    #[test]
    fn cursor_steps_clamp() {
        assert_eq!(step(None, 1, 3), Some(0));
        assert_eq!(step(None, -1, 3), Some(2));
        assert_eq!(step(Some(2), 1, 3), Some(2));
        assert_eq!(step(Some(0), -1, 3), Some(0));
        assert_eq!(step(Some(1), 10, 3), Some(2));
        assert_eq!(step(None, 1, 0), None);
    }
}
