//! Editor submission and built-in slash commands (pi `setupEditorSubmitHandler`).

use std::fmt::Write as _;

use oino_ai::{ImageContent, JsonValue, ThinkingLevel};
use oino_client::protocol::{OinoCommand, PiCommand};
use oino_core::session::StreamingBehavior;
use oino_view::commands::{Builtin, Submitted, classify};
use oino_view::format::format_tokens;
use oino_view::transcript::NoticeLevel;

use super::{App, Effect, ModelsFor, Tag};

impl App {
    /// Submits the composer (pi `onSubmit`).
    pub(crate) fn submit(&mut self, follow_up: bool) {
        let submission = self.composer.submission();
        if submission.text.is_empty() && submission.images.is_empty() {
            return;
        }
        self.completion = None;
        let raw = self.composer.text();
        self.composer.push_history(raw.trim());
        self.composer.clear();
        self.scroll = 0;
        self.submit_text(&submission.text, submission.images, follow_up);
    }

    /// Handles submitted text (also used for CLI initial messages).
    pub(crate) fn submit_text(&mut self, text: &str, images: Vec<ImageContent>, follow_up: bool) {
        match classify(text) {
            Submitted::Builtin(builtin, arg) => self.run_builtin(builtin, arg),
            Submitted::Bash {
                command,
                exclude_from_context,
            } => {
                if self.bash_running.is_some() {
                    self.notice(
                        NoticeLevel::Warning,
                        "A bash command is already running. Press Esc to cancel it first.",
                    );
                    self.composer.set_text(text);
                    return;
                }
                let id = self.request_id();
                self.bash_running = Some(id.clone());
                self.transcript
                    .begin_bash(Some(id.clone()), &command, exclude_from_context);
                self.effects.push(Effect::Send {
                    tag: Tag::Bash(id.clone()),
                    command: PiCommand::Bash {
                        command,
                        exclude_from_context: exclude_from_context.then_some(true),
                    }
                    .into(),
                    request_id: Some(id),
                    concurrent: true,
                });
            }
            Submitted::Prompt(message) => {
                let busy = self.session.streaming || self.session.compacting;
                let command = if busy && follow_up {
                    PiCommand::FollowUp { message, images }
                } else {
                    PiCommand::Prompt {
                        message,
                        images,
                        streaming_behavior: busy.then_some(if follow_up {
                            StreamingBehavior::FollowUp
                        } else {
                            StreamingBehavior::Steer
                        }),
                    }
                };
                self.send_concurrent(Tag::Prompt, command);
            }
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one arm per pi built-in slash command"
    )]
    pub(crate) fn run_builtin(&mut self, builtin: Builtin, arg: Option<String>) {
        match builtin {
            Builtin::Settings => self.open_settings(None),
            Builtin::Model => match arg {
                Some(query) => self.send(
                    Tag::Models(ModelsFor::Direct(query)),
                    PiCommand::GetAvailableModels,
                ),
                None => self.send(
                    Tag::Models(ModelsFor::Selector(None)),
                    PiCommand::GetAvailableModels,
                ),
            },
            Builtin::Thinking => match arg {
                Some(level) => match level.parse::<ThinkingLevel>() {
                    Ok(level) if self.thinking_levels.is_empty() || self.thinking_levels.contains(&level) => {
                        self.send(Tag::Ignore, PiCommand::SetThinkingLevel { level });
                        self.session.thinking = level;
                        self.notice(NoticeLevel::Info, format!("Thinking level: {}", level_name(level)));
                    }
                    _ => self.notice(
                        NoticeLevel::Error,
                        format!(
                            "Invalid thinking level \"{level}\". Available: {}",
                            self.thinking_levels
                                .iter()
                                .map(|l| level_name(*l))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    ),
                },
                None => self.send(
                    Tag::ThinkingLevels { open: true },
                    PiCommand::GetAvailableThinkingLevels,
                ),
            },
            Builtin::ScopedModels => {
                self.send(Tag::Models(ModelsFor::Scoped), PiCommand::GetAvailableModels);
            }
            Builtin::Login | Builtin::Logout => self.notice(
                NoticeLevel::Warning,
                "Provider sign-in is handled by provider extensions; set API keys in ~/.oino/auth.json or the provider's environment variable.",
            ),
            Builtin::New => self.send(Tag::Replace("New session"), PiCommand::NewSession { parent_session: None }),
            Builtin::Resume => self.open_sessions(),
            Builtin::Name => if let Some(name) = arg { self.send(Tag::SetName(name.clone()), PiCommand::SetSessionName { name }) } else {
                let text = self.session.session_name.as_ref().map_or_else(
                    || "No session name set".to_owned(),
                    |name| format!("Session name: {name}"),
                );
                self.notice(NoticeLevel::Info, text);
            },
            Builtin::Session => self.send(Tag::Stats { show: true }, PiCommand::GetSessionStats),
            Builtin::Tree => self.send(Tag::Tree, PiCommand::GetTree),
            Builtin::Fork => self.send(Tag::ForkMessages, PiCommand::GetForkMessages),
            Builtin::Clone => self.send(Tag::Replace("Clone"), PiCommand::Clone),
            Builtin::Compact => {
                if self.session.compacting {
                    self.notice(NoticeLevel::Warning, "Compaction already in progress");
                    return;
                }
                self.send_concurrent(
                    Tag::Compact,
                    PiCommand::Compact {
                        custom_instructions: arg,
                    },
                );
            }
            Builtin::Import => match arg {
                Some(path) => self.send(Tag::Replace("Import"), OinoCommand::ImportJsonl { path }),
                None => self.notice(NoticeLevel::Error, "Usage: /import <path>"),
            },
            Builtin::Copy => match self.transcript.last_assistant_text() {
                Some(text) => {
                    self.effects.push(Effect::ClipboardWrite(text));
                    self.notice(NoticeLevel::Info, "Copied last agent message to clipboard");
                }
                None => self.notice(NoticeLevel::Error, "No agent messages to copy yet."),
            },
            Builtin::Export => {
                let jsonl = arg.as_deref().is_some_and(|p| {
                    std::path::Path::new(p)
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("jsonl"))
                });
                if jsonl {
                    self.send(Tag::Export, OinoCommand::ExportJsonl { output_path: arg });
                } else {
                    self.send(Tag::Export, PiCommand::ExportHtml { output_path: arg });
                }
            }
            Builtin::Trust => self.notice(
                NoticeLevel::Warning,
                "Project trust decisions are not available from this frontend yet; edit ~/.oino/trust.json.",
            ),
            Builtin::Reload => {
                let overrides = self.host.keybindings();
                let (mut keys, warnings) = oino_view::keys::Keybindings::with_overrides(overrides.actions);
                keys.set_extension_overrides(overrides.extension_shortcuts);
                self.keys = keys;
                self.settings = self.host.settings();
                for warning in warnings {
                    self.notice(NoticeLevel::Warning, format!("keybindings.json: {warning}"));
                }
                self.send(Tag::Commands, PiCommand::GetCommands);
                self.notice(
                    NoticeLevel::Info,
                    "Reloaded keybindings and settings (extensions and resources reload on restart)",
                );
            }
            Builtin::Hotkeys => {
                let text = self.hotkeys_markdown();
                self.notice(NoticeLevel::Document, text);
            }
            Builtin::Changelog => match self.host.changelog() {
                Some(text) => self.notice(NoticeLevel::Document, text),
                None => self.notice(NoticeLevel::Info, "No changelog available"),
            },
            Builtin::Extensions => self.notice(
                NoticeLevel::Warning,
                "Extension management is not available from this frontend yet; use `oino list` / `oino install`.",
            ),
            Builtin::Quit => self.quit_now(),
        }
    }

    pub(crate) fn quit_now(&mut self) {
        self.quit = true;
        self.effects.push(Effect::Quit);
    }

    fn hotkeys_markdown(&self) -> String {
        let k = |id: &str| self.keys.key_text(id);
        let rows = [
            ("app.interrupt", "Cancel / abort"),
            ("app.clear", "Clear editor (twice: exit)"),
            ("app.exit", "Exit (when editor empty)"),
            ("app.suspend", "Suspend to background"),
            ("tui.input.newLine", "Insert new line"),
            ("tui.editor.deleteToLineEnd", "Delete to end of line"),
            ("app.thinking.cycle", "Cycle thinking level"),
            ("app.model.cycleForward", "Cycle to next model"),
            ("app.model.cycleBackward", "Cycle to previous model"),
            ("app.model.select", "Open model selector"),
            ("app.tools.expand", "Collapse or expand tool output"),
            ("app.thinking.toggle", "Collapse or expand thinking blocks"),
            ("app.editor.external", "Open in external editor"),
            ("app.message.followUp", "Queue follow-up message"),
            ("app.message.dequeue", "Restore queued messages to editor"),
            (
                "app.clipboard.pasteImage",
                "Paste image or text from clipboard",
            ),
            ("app.message.copy", "Copy last assistant message"),
        ];
        let mut text = String::from("**Keyboard shortcuts**\n\n| Key | Action |\n|---|---|\n");
        for (id, description) in rows {
            let keys = k(id);
            if !keys.is_empty() {
                let _ = writeln!(text, "| `{keys}` | {description} |");
            }
        }
        text.push_str("| `/` | Commands |\n| `!` | Run bash |\n| `!!` | Run bash (no context) |\n");
        if !self.ui.shortcuts.is_empty() {
            text.push_str("\n**Extension shortcuts**\n\n| Key | Action |\n|---|---|\n");
            for shortcut in &self.ui.shortcuts {
                let keys: Vec<String> = self
                    .keys
                    .extension_keys(&shortcut.id, &shortcut.keys)
                    .iter()
                    .map(ToString::to_string)
                    .collect();
                let _ = writeln!(text, "| `{}` | {} |", keys.join("/"), shortcut.description);
            }
        }
        text
    }

    pub(crate) fn show_session_info(&mut self, data: Option<&JsonValue>) {
        let Some(stats) = data else {
            return;
        };
        let s = |k: &str| {
            stats
                .get(k)
                .and_then(JsonValue::as_str)
                .unwrap_or("")
                .to_owned()
        };
        let n = |k: &str| stats.get(k).and_then(JsonValue::as_u64).unwrap_or(0);
        let t = |k: &str| {
            stats
                .get("tokens")
                .and_then(|t| t.get(k))
                .and_then(JsonValue::as_u64)
                .unwrap_or(0)
        };
        let mut text = String::from("**Session Info**\n\n");
        if let Some(name) = &self.session.session_name {
            let _ = writeln!(text, "Name: {name}");
        }
        let file = s("sessionFile");
        let _ = write!(
            text,
            "File: {}\nID: {}\n\n**Messages**\nUser: {}\nAssistant: {}\nTool Calls: {}\nTool Results: {}\nTotal: {}\n\n**Tokens**\nInput: {}\nOutput: {}\n",
            if file.is_empty() { "In-memory" } else { &file },
            s("sessionId"),
            n("userMessages"),
            n("assistantMessages"),
            n("toolCalls"),
            n("toolResults"),
            n("totalMessages"),
            group(t("input")),
            group(t("output")),
        );
        if t("cacheRead") > 0 {
            let _ = writeln!(text, "Cache Read: {}", group(t("cacheRead")));
        }
        if t("cacheWrite") > 0 {
            let _ = writeln!(text, "Cache Write: {}", group(t("cacheWrite")));
        }
        let _ = writeln!(text, "Total: {}", group(t("total")));
        let cost = stats.get("cost").and_then(JsonValue::as_f64).unwrap_or(0.0);
        if cost > 0.0 {
            let _ = write!(text, "\n**Cost**\nTotal: ${cost:.4}\n");
        }
        if let Some(usage) = stats.get("contextUsage")
            && let Some(window) = usage.get("contextWindow").and_then(JsonValue::as_u64)
        {
            let tokens = usage.get("tokens").and_then(JsonValue::as_u64);
            let _ = write!(
                text,
                "\n**Context**\n{} / {}\n",
                tokens.map_or_else(|| "?".to_owned(), format_tokens),
                format_tokens(window)
            );
        }
        self.notice(NoticeLevel::Document, text.replace('\n', "  \n"));
    }
}

/// pi thinking level display names.
pub(crate) fn level_name(level: ThinkingLevel) -> &'static str {
    match level {
        ThinkingLevel::Off => "off",
        ThinkingLevel::Minimal => "minimal",
        ThinkingLevel::Low => "low",
        ThinkingLevel::Medium => "medium",
        ThinkingLevel::High => "high",
        ThinkingLevel::Xhigh => "xhigh",
        ThinkingLevel::Max => "max",
    }
}

/// `1234567` → `1,234,567` (pi `toLocaleString`).
pub(crate) fn group(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}
