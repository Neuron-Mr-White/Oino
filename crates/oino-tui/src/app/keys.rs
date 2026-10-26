//! Key dispatch for the main editor (pi `CustomEditor` + `setupKeyHandlers`), the
//! autocomplete popup and focused extension panels.

use std::time::{Duration, Instant};

use oino_ai::JsonValue;
use oino_client::protocol::PiCommand;
use oino_view::commands::filter_menu;
use oino_view::composer::Composer;
use oino_view::keys::{Key, KeyId};
use oino_view::ous::Interactive;
use oino_view::ui_state::PanelPlacement;

use super::overlay::line_edit;
use super::{App, Completion, EditPurpose, Effect, ModelsFor, Tag};

const DOUBLE_PRESS: Duration = Duration::from_millis(500);
const MAX_COMPLETIONS: usize = 50;

impl App {
    pub(crate) fn on_key(&mut self, key: KeyId) {
        if self.ui.active_dialog().is_some() {
            self.dialog_key(key);
            return;
        }
        if self.overlay.is_some() {
            self.overlay_key(key);
            return;
        }
        if self.ui.focused_panel().is_some() {
            self.panel_key(key);
            return;
        }
        if let Some(forward) = self.jump.take() {
            if let Some(c) = key.printable() {
                self.composer.jump(c, forward);
            }
            return;
        }
        if self.completion.is_some() && self.completion_key(&key) {
            return;
        }
        if self.app_action(&key) {
            return;
        }
        self.editor_key(&key);
        self.update_completion(false);
    }

    fn matches(&self, action: &str, key: &KeyId) -> bool {
        self.keys.matches(action, key)
    }

    /// Application-level bindings; returns `true` when handled.
    #[allow(clippy::too_many_lines, reason = "one branch per pi app.* action")]
    fn app_action(&mut self, key: &KeyId) -> bool {
        let now = Instant::now();
        if self.matches("app.interrupt", key) {
            self.completion = None;
            if self.bash_running.is_some() {
                self.send(Tag::Abort, PiCommand::AbortBash);
            } else if self.retry.is_some() {
                self.send(Tag::Abort, PiCommand::AbortRetry);
            } else if self.busy() {
                self.send_concurrent(Tag::Abort, PiCommand::Abort);
            } else if self.composer.is_empty() {
                let double = self
                    .last_escape
                    .is_some_and(|t| now.duration_since(t) < DOUBLE_PRESS);
                if double {
                    self.last_escape = None;
                    match self
                        .settings
                        .get("doubleEscapeAction")
                        .and_then(JsonValue::as_str)
                        .unwrap_or("tree")
                    {
                        "fork" => self.send(Tag::ForkMessages, PiCommand::GetForkMessages),
                        "none" => {}
                        _ => self.send(Tag::Tree, PiCommand::GetTree),
                    }
                } else {
                    self.last_escape = Some(now);
                }
            }
            return true;
        }
        if self.matches("app.clear", key) {
            let double = self
                .last_ctrl_c
                .is_some_and(|t| now.duration_since(t) < DOUBLE_PRESS);
            if double {
                self.quit_now();
            } else {
                self.composer.clear();
                self.completion = None;
                self.last_ctrl_c = Some(now);
            }
            return true;
        }
        if self.matches("app.exit", key) && self.composer.is_empty() {
            self.quit_now();
            return true;
        }
        if self.matches("app.suspend", key) {
            self.effects.push(Effect::Suspend);
            return true;
        }
        if self.matches("app.editor.external", key) {
            self.effects.push(Effect::ExternalEditor {
                text: self.composer.text(),
                extension: "md".into(),
                purpose: EditPurpose::Composer,
            });
            return true;
        }
        if self.matches("app.clipboard.pasteImage", key) {
            self.effects.push(Effect::ClipboardImage(None));
            return true;
        }
        if self.matches("app.model.select", key) {
            self.send(
                Tag::Models(ModelsFor::Selector(None)),
                PiCommand::GetAvailableModels,
            );
            return true;
        }
        if self.matches("app.model.cycleForward", key) {
            self.send(Tag::CycleModel, PiCommand::CycleModel);
            return true;
        }
        if self.matches("app.model.cycleBackward", key) {
            self.send(
                Tag::Models(ModelsFor::CycleBackward),
                PiCommand::GetAvailableModels,
            );
            return true;
        }
        if self.matches("app.thinking.cycle", key) {
            self.send(Tag::CycleThinking, PiCommand::CycleThinkingLevel);
            return true;
        }
        if self.matches("app.thinking.toggle", key) {
            self.hide_thinking = !self.hide_thinking;
            self.cache.clear();
            return true;
        }
        if self.matches("app.tools.expand", key) {
            self.tools_expanded = !self.tools_expanded;
            self.cache.clear();
            return true;
        }
        if self.matches("app.message.copy", key) {
            self.run_builtin(oino_view::commands::Builtin::Copy, None);
            return true;
        }
        if self.matches("app.message.followUp", key) {
            self.submit(true);
            return true;
        }
        if self.matches("app.message.dequeue", key) {
            self.send(Tag::Dequeue, PiCommand::ClearQueue);
            return true;
        }
        let session_actions = [
            ("app.session.new", oino_view::commands::Builtin::New),
            ("app.session.tree", oino_view::commands::Builtin::Tree),
            ("app.session.fork", oino_view::commands::Builtin::Fork),
            ("app.session.resume", oino_view::commands::Builtin::Resume),
        ];
        for (action, builtin) in session_actions {
            if self.matches(action, key) {
                self.run_builtin(builtin, None);
                return true;
            }
        }
        let page = self.transcript_height.saturating_sub(2).max(1);
        if self.matches("tui.altScreen.pageUp", key) {
            self.scroll += page;
            return true;
        }
        if self.matches("tui.altScreen.pageDown", key) {
            self.scroll = self.scroll.saturating_sub(page);
            return true;
        }
        if self.matches("tui.altScreen.halfPageUp", key) {
            self.scroll += page / 2;
            return true;
        }
        if self.matches("tui.altScreen.halfPageDown", key) {
            self.scroll = self.scroll.saturating_sub(page / 2);
            return true;
        }
        if self.matches("tui.altScreen.lineUp", key) {
            self.scroll += 1;
            return true;
        }
        if self.matches("tui.altScreen.lineDown", key) {
            self.scroll = self.scroll.saturating_sub(1);
            return true;
        }
        if self.matches("tui.altScreen.top", key) {
            self.scroll = usize::MAX / 2;
            return true;
        }
        if self.matches("tui.altScreen.bottom", key) {
            self.scroll = 0;
            return true;
        }
        if self.matches("tui.altScreen.previousPrompt", key) {
            self.jump_prompt(true);
            return true;
        }
        if self.matches("tui.altScreen.nextPrompt", key) {
            self.jump_prompt(false);
            return true;
        }
        // Extension shortcuts: frontend-reserved keys win (ui-protocol §8).
        if !self.keys.is_reserved(key) {
            let hit = self.ui.shortcuts.iter().find_map(|s| {
                self.keys
                    .extension_keys(&s.id, &s.keys)
                    .contains(key)
                    .then(|| s.id.clone())
            });
            if let Some(id) = hit {
                self.effects.push(Effect::InvokeShortcut(id));
                return true;
            }
        }
        false
    }

    fn jump_prompt(&mut self, previous: bool) {
        // Prompt offsets are computed by the renderer; approximate by paging.
        let page = self.transcript_height.max(1);
        if previous {
            self.scroll += page / 2;
        } else {
            self.scroll = self.scroll.saturating_sub(page / 2);
        }
    }

    /// Editor bindings (`tui.editor.*`, `tui.input.*`).
    fn editor_key(&mut self, key: &KeyId) {
        let c = &mut self.composer;
        let kb = &self.keys;
        if kb.matches("tui.editor.historyPrevious", key) {
            c.history_prev();
        } else if kb.matches("tui.editor.historyNext", key) {
            c.history_next();
        } else if kb.matches("tui.input.newLine", key) {
            c.newline();
        } else if kb.matches("tui.input.submit", key) {
            // pi: a trailing backslash turns Enter into a newline.
            let text = c.text();
            let (line, col) = c.cursor();
            let at_end = c
                .lines()
                .get(line)
                .is_some_and(|l| l.chars().count() == col);
            if at_end && text.ends_with('\\') {
                c.backspace();
                c.newline();
            } else {
                self.submit(false);
            }
        } else if kb.matches("tui.input.tab", key) {
            self.update_completion(true);
        } else if kb.matches("tui.editor.cursorUp", key) {
            if !c.up() {
                c.history_prev();
            }
        } else if kb.matches("tui.editor.cursorDown", key) {
            if !c.down() {
                c.history_next();
            }
        } else if kb.matches("tui.editor.cursorWordLeft", key) {
            c.word_left();
        } else if kb.matches("tui.editor.cursorWordRight", key) {
            c.word_right();
        } else if kb.matches("tui.editor.cursorLeft", key) {
            c.left();
        } else if kb.matches("tui.editor.cursorRight", key) {
            c.right();
        } else if kb.matches("tui.editor.cursorLineStart", key) {
            c.home();
        } else if kb.matches("tui.editor.cursorLineEnd", key) {
            c.end();
        } else if kb.matches("tui.editor.jumpForward", key) {
            self.jump = Some(true);
        } else if kb.matches("tui.editor.jumpBackward", key) {
            self.jump = Some(false);
        } else if kb.matches("tui.editor.deleteWordBackward", key) {
            c.delete_word_backward();
        } else if kb.matches("tui.editor.deleteWordForward", key) {
            c.delete_word_forward();
        } else if kb.matches("tui.editor.deleteCharBackward", key) {
            c.backspace();
        } else if kb.matches("tui.editor.deleteCharForward", key) {
            c.delete();
        } else if kb.matches("tui.editor.deleteToLineStart", key) {
            c.delete_to_line_start();
        } else if kb.matches("tui.editor.deleteToLineEnd", key) {
            c.delete_to_line_end();
        } else if kb.matches("tui.editor.yank", key) {
            c.yank();
        } else if kb.matches("tui.editor.yankPop", key) {
            c.yank_pop();
        } else if kb.matches("tui.editor.undo", key) {
            c.undo();
        } else if kb.matches("tui.editor.pageUp", key) {
            self.scroll += self.transcript_height.max(1);
        } else if kb.matches("tui.editor.pageDown", key) {
            self.scroll = self.scroll.saturating_sub(self.transcript_height.max(1));
        } else if let Some(ch) = key.printable() {
            c.insert_char(ch);
        }
    }

    pub(crate) fn on_paste(&mut self, text: &str) {
        if self.ui.active_dialog().is_some() {
            if let Some(
                super::DialogUi::Input { input, .. } | super::DialogUi::Editor { input, .. },
            ) = &mut self.dialog_ui
            {
                input.insert_str(text);
            }
            return;
        }
        if let Some(input) = &mut self.panel_input {
            input.insert_str(text);
            return;
        }
        if self.overlay.is_some() {
            for c in text.chars().filter(|c| !c.is_control()) {
                self.overlay_key(KeyId::plain(Key::Char(c)));
            }
            return;
        }
        // Dropped files arrive as pasted paths (pi `drop files to attach`).
        let trimmed = text.trim();
        let looks_like_path = !trimmed.contains('\n')
            && (trimmed.starts_with('/')
                || trimmed.starts_with("~/")
                || trimmed.starts_with("file://"))
            && is_image_path(trimmed);
        if looks_like_path {
            let path = trimmed.trim_start_matches("file://").to_owned();
            let label = path.rsplit('/').next().unwrap_or(&path).to_owned();
            self.composer
                .insert_attachment(&label, None, None, Some(path));
        } else {
            self.composer.paste(text);
        }
        self.update_completion(false);
    }

    /// Recomputes the autocomplete popup. `force` = Tab (path completion, single match
    /// applied immediately).
    pub(crate) fn update_completion(&mut self, force: bool) {
        let token = self.composer.current_token().to_owned();
        let (line, _) = self.composer.cursor();
        let first_token = line == 0
            && self.composer.lines()[0]
                .trim_start()
                .starts_with(token.as_str());
        if first_token && token.starts_with('/') && !self.composer.text().contains(' ') {
            let query = &token[1..];
            let items: Vec<(String, String)> = filter_menu(&self.commands, query)
                .into_iter()
                .take(MAX_COMPLETIONS)
                .map(|i| (format!("/{}", i.name), i.description.clone()))
                .collect();
            self.completion = (!items.is_empty()).then_some(Completion {
                items,
                selected: 0,
                commands: true,
            });
            return;
        }
        let path_token = token.strip_prefix('@');
        if path_token.is_some() || force {
            let prefix = path_token.unwrap_or(&token);
            let items: Vec<(String, String)> = self
                .host
                .complete_path(prefix)
                .into_iter()
                .map(|p| {
                    let value = if path_token.is_some() {
                        format!("@{p}")
                    } else {
                        p
                    };
                    (value, String::new())
                })
                .collect();
            if force && items.len() == 1 {
                self.composer.replace_current_token(&items[0].0);
                self.completion = None;
                return;
            }
            self.completion = (!items.is_empty()).then_some(Completion {
                items,
                selected: 0,
                commands: false,
            });
            return;
        }
        self.completion = None;
    }

    /// Popup keys; returns `true` when consumed.
    fn completion_key(&mut self, key: &KeyId) -> bool {
        let Some(completion) = &mut self.completion else {
            return false;
        };
        let len = completion.items.len().min(MAX_COMPLETIONS);
        if self.keys.matches("tui.select.up", key) {
            completion.selected = (completion.selected + len - 1) % len.max(1);
            return true;
        }
        if self.keys.matches("tui.select.down", key) {
            completion.selected = (completion.selected + 1) % len.max(1);
            return true;
        }
        if key.key == Key::Escape {
            self.completion = None;
            return true;
        }
        let accept =
            self.keys.matches("tui.input.tab", key) || self.keys.matches("tui.input.submit", key);
        if accept {
            let (value, _) = completion.items[completion.selected].clone();
            let commands = completion.commands;
            let token = self.composer.current_token().to_owned();
            self.completion = None;
            if commands {
                let exact = token == value;
                if exact && self.keys.matches("tui.input.submit", key) {
                    self.submit(false);
                    return true;
                }
                self.composer.replace_current_token(&format!("{value} "));
                if self.keys.matches("tui.input.submit", key) {
                    // pi: Enter on a command without arguments runs it.
                    let name = value.trim_start_matches('/');
                    let takes_args = oino_view::commands::BUILTINS
                        .iter()
                        .find(|b| b.name == name)
                        .is_none_or(|b| !b.args.is_empty());
                    if !takes_args {
                        self.submit(false);
                    }
                }
            } else {
                let dir = value.ends_with('/');
                self.composer.replace_current_token(&value);
                if dir {
                    self.update_completion(false);
                } else {
                    self.composer.insert_char(' ');
                }
            }
            return true;
        }
        false
    }

    /// Keys for a focused panel (ui-protocol §5).
    #[allow(
        clippy::too_many_lines,
        reason = "one branch per interactive OUS control kind"
    )]
    fn panel_key(&mut self, key: KeyId) {
        let Some(panel) = self.ui.focused_panel().cloned() else {
            return;
        };
        let controls = panel.spec.interactive();
        if let Some(input) = &mut self.panel_input {
            if key.key == Key::Escape {
                self.panel_input = None;
            } else if self.keys.matches("tui.input.submit", &key) {
                let text = input.text();
                self.panel_input = None;
                if let Some(Interactive::Input { action_id, .. }) = controls.get(self.panel_focus) {
                    self.effects.push(Effect::UiAction {
                        panel_id: panel.panel_id.clone(),
                        action_id: action_id.clone(),
                        value: Some(JsonValue::String(text)),
                    });
                }
            } else {
                line_edit(input, &key);
            }
            return;
        }
        if key.key == Key::Escape && panel.close_on_escape {
            self.ui.panels.retain(|p| p.panel_id != panel.panel_id);
            self.effects.push(Effect::UiAction {
                panel_id: panel.panel_id,
                action_id: "$close".into(),
                value: None,
            });
            self.panel_focus = 0;
            return;
        }
        if let Some(bound) = panel
            .keys
            .iter()
            .find(|k| k.key.parse::<KeyId>().is_ok_and(|id| id == key))
        {
            self.effects.push(Effect::UiAction {
                panel_id: panel.panel_id.clone(),
                action_id: bound.action_id.clone(),
                value: None,
            });
            return;
        }
        if key.key == Key::Tab && !controls.is_empty() {
            self.panel_focus = if key.mods.shift {
                (self.panel_focus + controls.len() - 1) % controls.len()
            } else {
                (self.panel_focus + 1) % controls.len()
            };
            return;
        }
        let confirm = self.keys.matches("tui.select.confirm", &key);
        match controls.get(self.panel_focus) {
            Some(Interactive::Button { action_id, .. }) if confirm => {
                self.effects.push(Effect::UiAction {
                    panel_id: panel.panel_id.clone(),
                    action_id: action_id.clone(),
                    value: None,
                });
            }
            Some(Interactive::Input { value, .. }) if confirm => {
                let mut c = Composer::new();
                c.set_text(value);
                self.panel_input = Some(c);
            }
            Some(Interactive::Toggle {
                action_id, value, ..
            }) if confirm || key.key == Key::Space => {
                self.effects.push(Effect::UiAction {
                    panel_id: panel.panel_id.clone(),
                    action_id: action_id.clone(),
                    value: Some(JsonValue::Bool(!value)),
                });
            }
            Some(Interactive::Select {
                action_id,
                options,
                value,
                ..
            }) if confirm || key.key == Key::Space => {
                let pos = options
                    .iter()
                    .position(|o| Some(&o.value) == value.as_ref())
                    .unwrap_or(0);
                if let Some(next) = options.get((pos + 1) % options.len().max(1)) {
                    self.effects.push(Effect::UiAction {
                        panel_id: panel.panel_id.clone(),
                        action_id: action_id.clone(),
                        value: Some(JsonValue::String(next.value.clone())),
                    });
                }
            }
            Some(Interactive::Choice { action_id, ids, .. }) => {
                let up = self.keys.matches("tui.select.up", &key);
                let down = self.keys.matches("tui.select.down", &key);
                if up || down {
                    let len = ids.len().max(1);
                    self.panel_choice = if up {
                        (self.panel_choice + len - 1) % len
                    } else {
                        (self.panel_choice + 1) % len
                    };
                } else if confirm && let Some(id) = ids.get(self.panel_choice) {
                    self.effects.push(Effect::UiAction {
                        panel_id: panel.panel_id.clone(),
                        action_id: action_id.clone(),
                        value: Some(JsonValue::String(id.clone())),
                    });
                }
            }
            _ => {
                if matches!(
                    panel.placement,
                    PanelPlacement::EditorReplace | PanelPlacement::BottomPane
                ) && let Some(c) = key.printable()
                {
                    // Typing while an editor-replacing panel is focused returns to the editor.
                    self.ui.panels.retain(|p| p.panel_id != panel.panel_id);
                    self.effects.push(Effect::UiAction {
                        panel_id: panel.panel_id.clone(),
                        action_id: "$close".into(),
                        value: None,
                    });
                    self.composer.insert_char(c);
                }
            }
        }
    }
}

fn is_image_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp"]
        .iter()
        .any(|ext| lower.ends_with(ext))
}
