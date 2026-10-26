//! The interactive-mode state machine (pi `interactive-mode.ts`).
//!
//! [`App`] owns every piece of view state and reacts to [`AppEvent`]s (keys, paste, ticks,
//! client events, command replies, host replies). It never performs I/O: side effects are
//! queued as [`Effect`]s which the runner executes (commands on the backend, clipboard,
//! external editor, suspend, quit). This keeps the whole client testable without a
//! terminal or a runtime.

mod dialog;
mod keys;
pub(crate) mod overlay;
mod submit;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use oino_ai::{ImageContent, JsonObject, JsonValue, Model, ThinkingLevel};
use oino_client::protocol::{OinoCommand, PiCommand, RpcCommand};
use oino_client::{ClientError, RpcResponseData};
use oino_core::events::{AgentSessionEvent, ClientEvent, OinoEvent, RunState};
use oino_core::ui::UiResponse;
use oino_session::SessionEntry;
use oino_view::commands::{CommandItem, menu_items};
use oino_view::composer::Composer;
use oino_view::keys::{KeyId, Keybindings};
use oino_view::sessions::{Scope, SessionItem};
use oino_view::transcript::{BashOutcome, NoticeLevel, RowId, Transcript};
use oino_view::ui_state::{NotifyLevel, UiEffect, UiState};
use serde_json::json;

use crate::host::TuiHost;
use crate::theme::Theme;

pub(crate) use dialog::DialogUi;
pub(crate) use overlay::Overlay;
pub(crate) use submit::group;

/// Options for the interactive client.
#[derive(Clone, Debug, Default)]
pub struct TuiOptions {
    /// Version shown in the header.
    pub version: String,
    /// Prompts submitted once the session is loaded (CLI positional messages).
    pub initial_messages: Vec<String>,
    /// Images attached to the first initial message (`@file` images).
    pub initial_images: Vec<ImageContent>,
    /// Startup warnings to show in the transcript.
    pub notices: Vec<String>,
    /// Theme name override (`--theme`).
    pub theme: Option<String>,
}

/// Why a command was sent; routes its reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tag {
    /// Reply is ignored (errors still show).
    Ignore,
    /// `_oino/hello`.
    Hello,
    /// `_oino/ui_snapshot`.
    UiSnapshot,
    /// `get_state`.
    State,
    /// `get_entries` (transcript rebuild).
    Entries,
    /// `get_commands`.
    Commands,
    /// `get_available_models` for a purpose.
    Models(ModelsFor),
    /// `get_available_thinking_levels`.
    ThinkingLevels {
        /// Open the selector afterwards.
        open: bool,
    },
    /// `get_session_stats`.
    Stats {
        /// Show `/session` info.
        show: bool,
    },
    /// `_oino/get_context_usage`.
    ContextUsage,
    /// `prompt` / `steer` / `follow_up`.
    Prompt,
    /// `bash` with its request id.
    Bash(String),
    /// `compact`.
    Compact,
    /// `new_session` / `switch_session` / `clone` / `_oino/import_jsonl`.
    Replace(&'static str),
    /// `fork`.
    Fork,
    /// `export_html` / `_oino/export_jsonl`.
    Export,
    /// `get_tree`.
    Tree,
    /// `get_fork_messages`.
    ForkMessages,
    /// `_oino/navigate_tree`.
    Navigate,
    /// `_oino/set_label`.
    SetLabel {
        /// Entry.
        entry_id: String,
        /// Label.
        label: Option<String>,
    },
    /// `cycle_model`.
    CycleModel,
    /// `cycle_thinking_level`.
    CycleThinking,
    /// `set_model`.
    SetModel,
    /// `set_session_name`.
    SetName(String),
    /// `clear_queue` (restore to editor).
    Dequeue,
    /// `abort` / `abort_bash` / `abort_retry`.
    Abort,
}

/// Purpose of a model list request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelsFor {
    /// Footer provider count.
    Footer,
    /// `/model` selector with an initial query.
    Selector(Option<String>),
    /// `/model <query>` direct match.
    Direct(String),
    /// `/scoped-models`.
    Scoped,
    /// Backward model cycling.
    CycleBackward,
}

/// Purpose of an external editor run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditPurpose {
    /// The composer (`app.editor.external`).
    Composer,
    /// An `editor` dialog.
    Dialog(String),
    /// An `editExternally` dialog.
    External(String),
}

/// Side effect requested by the app.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// Send a command; `concurrent` commands may run alongside others (long-running).
    Send {
        /// Reply routing.
        tag: Tag,
        /// Command.
        command: RpcCommand,
        /// RPC request id (bash streaming).
        request_id: Option<String>,
        /// Run outside the ordered command queue.
        concurrent: bool,
    },
    /// Answer a dialog.
    Respond(UiResponse),
    /// Deliver a panel action (`extension_ui_action`).
    UiAction {
        /// Panel.
        panel_id: String,
        /// Action.
        action_id: String,
        /// Value.
        value: Option<JsonValue>,
    },
    /// Invoke an extension shortcut.
    InvokeShortcut(String),
    /// List saved sessions.
    ListSessions(Scope),
    /// Delete a session file.
    DeleteSession(String),
    /// Copy text (OSC 52).
    ClipboardWrite(String),
    /// Read a clipboard image (`None` = composer paste).
    ClipboardImage(Option<String>),
    /// Open the external editor.
    ExternalEditor {
        /// Initial text.
        text: String,
        /// File extension.
        extension: String,
        /// Where the result goes.
        purpose: EditPurpose,
    },
    /// Desktop notification.
    OsNotify {
        /// Title.
        title: String,
        /// Body.
        body: String,
    },
    /// Open a URL.
    OpenUrl(String),
    /// Set the terminal title.
    SetTitle(String),
    /// Suspend to the shell (SIGTSTP).
    Suspend,
    /// Exit.
    Quit,
}

/// Input to the app.
#[derive(Clone, Debug)]
pub enum AppEvent {
    /// Key press.
    Key(KeyId),
    /// Bracketed paste.
    Paste(String),
    /// Animation tick (~80 ms).
    Tick,
    /// Terminal resized.
    Resize,
    /// Backend event.
    Client(Box<ClientEvent>),
    /// Command reply.
    Reply(Tag, Result<RpcResponseData, ClientError>),
    /// Saved sessions.
    Sessions(Result<Vec<SessionItem>, String>),
    /// Session deletion finished.
    SessionDeleted(String, Result<(), String>),
    /// Clipboard image (or text fallback) for a paste or dialog.
    Clipboard {
        /// Dialog id (`clipboardReadImage`).
        dialog: Option<String>,
        /// Image.
        image: Option<ImageContent>,
        /// Text fallback.
        text: Option<String>,
    },
    /// External editor finished.
    Edited(EditPurpose, Option<String>),
    /// A runner-side problem to show (unsupported host operation, I/O failure).
    Notice(String),
}

/// Session state mirrored from `get_state` and events.
#[derive(Clone, Debug, Default)]
pub(crate) struct SessionView {
    pub(crate) model: Option<Model>,
    pub(crate) routed_model: Option<Model>,
    pub(crate) thinking: ThinkingLevel,
    pub(crate) streaming: bool,
    pub(crate) compacting: bool,
    pub(crate) session_id: String,
    pub(crate) session_file: Option<String>,
    pub(crate) session_name: Option<String>,
    pub(crate) auto_compaction: bool,
    pub(crate) run_state: Option<RunState>,
    pub(crate) waiting_on: Vec<String>,
}

/// Footer usage numbers.
#[derive(Clone, Debug, Default)]
pub(crate) struct Usage {
    pub(crate) input: u64,
    pub(crate) output: u64,
    pub(crate) cache_read: u64,
    pub(crate) cache_write: u64,
    pub(crate) cost: f64,
    pub(crate) context_percent: Option<f64>,
    pub(crate) context_window: u64,
}

/// Automatic retry in progress.
#[derive(Clone, Debug)]
pub(crate) struct Retry {
    pub(crate) attempt: u32,
    pub(crate) max: u32,
    pub(crate) error: String,
    pub(crate) until: Instant,
}

/// Composer autocomplete popup.
#[derive(Clone, Debug, Default)]
pub(crate) struct Completion {
    pub(crate) items: Vec<(String, String)>,
    pub(crate) selected: usize,
    /// Command (`/`) completion rather than paths.
    pub(crate) commands: bool,
}

/// Cached rendering of one transcript row.
#[derive(Clone, Debug)]
pub(crate) struct CachedRow {
    pub(crate) rev: u64,
    pub(crate) width: u16,
    pub(crate) flags: u8,
    pub(crate) lines: Vec<ratatui::text::Line<'static>>,
}

/// The interactive client state.
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent view toggles (expanded tools, hidden thinking, loaded, quit)"
)]
pub struct App {
    pub(crate) host: Arc<dyn TuiHost>,
    pub(crate) options: TuiOptions,
    pub(crate) session: SessionView,
    pub(crate) usage: Usage,
    pub(crate) provider_count: usize,
    pub(crate) models: Vec<Model>,
    pub(crate) thinking_levels: Vec<ThinkingLevel>,
    pub(crate) commands: Vec<CommandItem>,
    pub(crate) queue: (Vec<String>, Vec<String>),
    pub(crate) transcript: Transcript,
    pub(crate) ui: UiState,
    pub(crate) composer: Composer,
    pub(crate) keys: Keybindings,
    pub(crate) theme: Theme,
    pub(crate) settings: JsonObject,
    pub(crate) tools_expanded: bool,
    pub(crate) hide_thinking: bool,
    pub(crate) overlay: Option<Overlay>,
    pub(crate) completion: Option<Completion>,
    pub(crate) dialog_ui: Option<DialogUi>,
    pub(crate) panel_focus: usize,
    pub(crate) panel_choice: usize,
    pub(crate) panel_input: Option<Composer>,
    /// Transcript scroll offset from the bottom (lines); 0 follows new output.
    pub(crate) scroll: usize,
    pub(crate) retry: Option<Retry>,
    pub(crate) compaction_reason: Option<&'static str>,
    pub(crate) tick: u64,
    pub(crate) last_ctrl_c: Option<Instant>,
    pub(crate) last_escape: Option<Instant>,
    pub(crate) jump: Option<bool>,
    pub(crate) bash_running: Option<String>,
    pub(crate) next_request: u64,
    pub(crate) effects: Vec<Effect>,
    pub(crate) quit: bool,
    pub(crate) loaded: bool,
    pub(crate) git_branch: Option<String>,
    pub(crate) cache: HashMap<RowId, CachedRow>,
    pub(crate) transcript_height: usize,
    pub(crate) last_title: String,
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("session", &self.session.session_id)
            .field("rows", &self.transcript.rows().len())
            .finish_non_exhaustive()
    }
}

fn parse<T: serde::de::DeserializeOwned>(data: Option<&JsonValue>, key: &str) -> Option<T> {
    let value = if key.is_empty() {
        data?
    } else {
        data?.get(key)?
    };
    serde_json::from_value(value.clone()).ok()
}

impl App {
    /// A new app over `host`.
    #[must_use]
    pub fn new(host: Arc<dyn TuiHost>, options: TuiOptions) -> Self {
        let settings = host.settings();
        let overrides = host.keybindings();
        let (mut keys, warnings) = Keybindings::with_overrides(overrides.actions);
        keys.set_extension_overrides(overrides.extension_shortcuts);
        let theme_name = options
            .theme
            .clone()
            .or_else(|| {
                settings
                    .get("theme")
                    .and_then(JsonValue::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "dark".to_owned());
        let hide_thinking = settings
            .get("hideThinkingBlock")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
        let git_branch = host.git_branch();
        let mut transcript = Transcript::new();
        for warning in warnings {
            transcript.push_notice(NoticeLevel::Warning, format!("keybindings.json: {warning}"));
        }
        Self {
            host,
            options,
            session: SessionView::default(),
            usage: Usage::default(),
            provider_count: 0,
            models: Vec::new(),
            thinking_levels: Vec::new(),
            commands: menu_items(&[]),
            queue: (Vec::new(), Vec::new()),
            transcript,
            ui: UiState::new(),
            composer: Composer::new(),
            keys,
            theme: Theme::named(&theme_name),
            settings,
            tools_expanded: false,
            hide_thinking,
            overlay: None,
            completion: None,
            dialog_ui: None,
            panel_focus: 0,
            panel_choice: 0,
            panel_input: None,
            scroll: 0,
            retry: None,
            compaction_reason: None,
            tick: 0,
            last_ctrl_c: None,
            last_escape: None,
            jump: None,
            bash_running: None,
            next_request: 0,
            effects: Vec::new(),
            quit: false,
            loaded: false,
            git_branch,
            cache: HashMap::new(),
            transcript_height: 20,
            last_title: String::new(),
        }
    }

    /// Takes the queued effects.
    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    /// Whether the app asked to exit.
    #[must_use]
    pub const fn should_quit(&self) -> bool {
        self.quit
    }

    pub(crate) fn send(&mut self, tag: Tag, command: impl Into<RpcCommand>) {
        self.effects.push(Effect::Send {
            tag,
            command: command.into(),
            request_id: None,
            concurrent: false,
        });
    }

    pub(crate) fn send_concurrent(&mut self, tag: Tag, command: impl Into<RpcCommand>) {
        self.effects.push(Effect::Send {
            tag,
            command: command.into(),
            request_id: None,
            concurrent: true,
        });
    }

    pub(crate) fn notice(&mut self, level: NoticeLevel, text: impl Into<String>) {
        self.transcript.push_notice(level, text);
        self.scroll = 0;
    }

    /// Initial requests (pi `init` + `rebindCurrentSession` + `renderInitialMessages`).
    pub fn start(&mut self) {
        for notice in self.options.notices.clone() {
            self.notice(NoticeLevel::Warning, notice);
        }
        self.send(
            Tag::Hello,
            OinoCommand::Hello {
                client: oino_client::protocol::ClientInfo {
                    name: "oino-tui".into(),
                    version: self.options.version.clone(),
                },
                ui: Some(json!({
                    "dialogs": ["select","confirm","input","editor","multiSelect","form","approval","custom","clipboardReadImage","editExternally"],
                    "ous": true,
                    "panels": ["overlay","sidePane","editorReplace","bottomPane","fullscreen"],
                    "images": false,
                    "osNotify": true,
                    "clipboard": true
                })),
                protocol: 1,
            },
        );
        self.refresh_session();
        self.send(
            Tag::Models(ModelsFor::Footer),
            PiCommand::GetAvailableModels,
        );
    }

    fn refresh_session(&mut self) {
        self.send(Tag::UiSnapshot, OinoCommand::UiSnapshot);
        self.send(Tag::State, PiCommand::GetState);
        self.send(Tag::Entries, PiCommand::GetEntries { since: None });
        self.send(Tag::Commands, PiCommand::GetCommands);
        self.send(
            Tag::ThinkingLevels { open: false },
            PiCommand::GetAvailableThinkingLevels,
        );
        self.refresh_usage();
    }

    fn refresh_usage(&mut self) {
        self.send(Tag::Stats { show: false }, PiCommand::GetSessionStats);
        self.send(Tag::ContextUsage, OinoCommand::GetContextUsage);
    }

    /// Handles one event.
    pub fn handle(&mut self, event: AppEvent) {
        match event {
            AppEvent::Key(key) => self.on_key(key),
            AppEvent::Paste(text) => self.on_paste(&text),
            AppEvent::Tick => self.tick = self.tick.wrapping_add(1),
            AppEvent::Resize => self.cache.clear(),
            AppEvent::Client(event) => self.on_client(*event),
            AppEvent::Reply(tag, result) => self.on_reply(tag, result),
            AppEvent::Sessions(result) => self.on_sessions(result),
            AppEvent::SessionDeleted(path, result) => self.on_session_deleted(&path, result),
            AppEvent::Clipboard {
                dialog,
                image,
                text,
            } => self.on_clipboard(dialog, image, text),
            AppEvent::Edited(purpose, text) => self.on_edited(purpose, text),
            AppEvent::Notice(text) => self.notice(NoticeLevel::Warning, text),
        }
        self.update_title();
    }

    fn update_title(&mut self) {
        let title = self.ui.title.clone().unwrap_or_else(|| {
            let cwd = self.host.cwd();
            let dir = cwd.file_name().map_or_else(
                || cwd.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            match &self.session.session_name {
                Some(name) => format!("oino - {name} - {dir}"),
                None => format!("oino - {dir}"),
            }
        });
        if title != self.last_title {
            self.last_title.clone_from(&title);
            self.effects.push(Effect::SetTitle(title));
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one arm per client event, mirroring interactive-mode handleEvent"
    )]
    fn on_client(&mut self, event: ClientEvent) {
        match event {
            ClientEvent::Session(event) => {
                let applied = self.transcript.apply(&event);
                if applied.reload {
                    self.send(Tag::Entries, PiCommand::GetEntries { since: None });
                }
                self.on_session_event(&event);
            }
            ClientEvent::Ui(request) => {
                if let Some(effect) = self.ui.apply(&request) {
                    self.on_ui_effect(effect);
                }
                self.sync_dialog();
            }
            ClientEvent::ExtensionError(error) => {
                let e = error.error;
                self.notice(
                    NoticeLevel::Error,
                    format!("Extension error ({}): {}", e.extension_path, e.error),
                );
            }
            ClientEvent::Oino(event) => match event {
                OinoEvent::SessionReplaced {
                    session_id,
                    session_file,
                    ..
                } => {
                    self.session.session_id = session_id;
                    self.session.session_file = session_file;
                    self.transcript.clear();
                    self.cache.clear();
                    self.ui.reset();
                    self.dialog_ui = None;
                    self.queue = (Vec::new(), Vec::new());
                    self.scroll = 0;
                    self.refresh_session();
                }
                OinoEvent::ModelChanged {
                    model,
                    routed_model,
                    ..
                } => {
                    self.session.model = Some(model);
                    self.session.routed_model = routed_model;
                    self.send(
                        Tag::ThinkingLevels { open: false },
                        PiCommand::GetAvailableThinkingLevels,
                    );
                    self.send(Tag::ContextUsage, OinoCommand::GetContextUsage);
                }
                OinoEvent::RunState { state, waiting_on } => {
                    self.session.run_state = Some(state);
                    self.session.waiting_on = waiting_on.unwrap_or_default();
                }
                OinoEvent::CommandsChanged => self.send(Tag::Commands, PiCommand::GetCommands),
                OinoEvent::ExtensionUiDismiss { id, .. } => {
                    self.ui.dismiss(&id);
                    self.sync_dialog();
                }
                OinoEvent::Lagged { dropped } => self.notice(
                    NoticeLevel::Warning,
                    format!("Display fell behind; {dropped} events were dropped"),
                ),
                OinoEvent::ToolsChanged { .. } => {}
            },
        }
    }

    fn on_session_event(&mut self, event: &AgentSessionEvent) {
        match event {
            AgentSessionEvent::AgentStart => {
                self.session.streaming = true;
                self.retry = None;
            }
            AgentSessionEvent::AgentEnd { .. } => self.session.streaming = false,
            AgentSessionEvent::AgentSettled { .. } => {
                self.session.streaming = false;
                self.refresh_usage();
            }
            AgentSessionEvent::MessageEnd { .. } => self.refresh_usage(),
            AgentSessionEvent::QueueUpdate {
                steering,
                follow_up,
            } => self.queue = (steering.clone(), follow_up.clone()),
            AgentSessionEvent::CompactionStart { reason } => {
                self.session.compacting = true;
                self.compaction_reason = Some(match reason {
                    oino_core::events::CompactionReason::Manual => "manual",
                    oino_core::events::CompactionReason::Threshold => "threshold",
                    oino_core::events::CompactionReason::Overflow => "overflow",
                });
            }
            AgentSessionEvent::CompactionEnd {
                reason,
                result,
                aborted,
                error_message,
                ..
            } => {
                self.session.compacting = false;
                self.compaction_reason = None;
                let manual = *reason == oino_core::events::CompactionReason::Manual;
                if *aborted {
                    if manual {
                        self.notice(NoticeLevel::Error, "Compaction cancelled");
                    } else {
                        self.notice(NoticeLevel::Info, "Auto-compaction cancelled");
                    }
                } else if result.is_none()
                    && let Some(error) = error_message
                {
                    self.notice(NoticeLevel::Error, error.clone());
                }
                self.refresh_usage();
            }
            AgentSessionEvent::SessionInfoChanged { name } => {
                self.session.session_name.clone_from(name);
            }
            AgentSessionEvent::ThinkingLevelChanged { level } => self.session.thinking = *level,
            AgentSessionEvent::AutoRetryStart {
                attempt,
                max_attempts,
                delay_ms,
                error_message,
            } => {
                self.retry = Some(Retry {
                    attempt: *attempt,
                    max: *max_attempts,
                    error: error_message.clone(),
                    until: Instant::now() + Duration::from_millis(*delay_ms),
                });
            }
            AgentSessionEvent::AutoRetryEnd {
                success,
                attempt,
                final_error,
            } => {
                self.retry = None;
                if !success {
                    let error = final_error
                        .clone()
                        .unwrap_or_else(|| "Unknown error".into());
                    self.notice(
                        NoticeLevel::Error,
                        format!("Retry failed after {attempt} attempts: {error}"),
                    );
                }
            }
            AgentSessionEvent::BashExecutionUpdate { id, delta } => {
                self.transcript.bash_delta(id.as_deref(), delta);
            }
            _ => {}
        }
    }

    fn on_ui_effect(&mut self, effect: UiEffect) {
        match effect {
            UiEffect::Notify { level, message } => {
                let level = match level {
                    NotifyLevel::Info => NoticeLevel::Info,
                    NotifyLevel::Warning => NoticeLevel::Warning,
                    NotifyLevel::Error => NoticeLevel::Error,
                };
                self.notice(level, message);
            }
            UiEffect::SetEditorText(text) => self.composer.set_text(&text),
            UiEffect::InsertEditorText(text) => self.composer.insert_str(&text),
            UiEffect::InsertToken(token) => {
                self.composer.insert_attachment(
                    &token.label,
                    Some(token.token_id),
                    token.data,
                    token.path,
                );
            }
            UiEffect::OsNotify { title, body } => {
                self.effects.push(Effect::OsNotify { title, body });
            }
            UiEffect::OpenUrl(url) => self.effects.push(Effect::OpenUrl(url)),
            UiEffect::ClipboardWrite(text) => self.effects.push(Effect::ClipboardWrite(text)),
            UiEffect::OpenSettings { namespace, query } => {
                self.open_settings(namespace.or(query));
            }
            UiEffect::SetTheme(name) => {
                self.theme = Theme::named(&name);
                self.cache.clear();
            }
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one arm per reply tag; each is a few lines"
    )]
    fn on_reply(&mut self, tag: Tag, result: Result<RpcResponseData, ClientError>) {
        let data = match result {
            Ok(data) => data,
            Err(error) => {
                self.on_command_error(&tag, &error);
                return;
            }
        };
        let data = data.as_ref();
        match tag {
            Tag::Ignore | Tag::Hello | Tag::Prompt | Tag::Abort | Tag::Compact => {}
            Tag::UiSnapshot => {
                let requests: Vec<oino_core::ui::UiRequest> =
                    parse(data, "requests").unwrap_or_default();
                for request in requests {
                    if let Some(effect) = self.ui.apply(&request)
                        && !matches!(effect, UiEffect::Notify { .. })
                    {
                        self.on_ui_effect(effect);
                    }
                }
                self.sync_dialog();
            }
            Tag::State => self.apply_state(data),
            Tag::Entries => {
                let entries: Vec<SessionEntry> = parse(data, "entries").unwrap_or_default();
                let leaf: Option<String> = parse(data, "leafId");
                let notices: Vec<_> = self
                    .transcript
                    .rows()
                    .iter()
                    .filter(|r| {
                        !self.loaded && matches!(r.kind, oino_view::transcript::RowKind::Notice(_))
                    })
                    .cloned()
                    .collect();
                self.transcript
                    .reset_from_entries(&entries, leaf.as_deref());
                for row in notices {
                    if let oino_view::transcript::RowKind::Notice(n) = row.kind {
                        self.transcript.push_notice(n.level, n.text);
                    }
                }
                self.cache.clear();
                if !self.loaded {
                    self.loaded = true;
                    for text in self.transcript.user_texts() {
                        self.composer.push_history(&text);
                    }
                    let compactions = entries
                        .iter()
                        .filter(|e| matches!(e.kind, oino_session::EntryKind::Compaction(_)))
                        .count();
                    if compactions > 0 {
                        let times = if compactions == 1 {
                            "1 time".to_owned()
                        } else {
                            format!("{compactions} times")
                        };
                        self.notice(NoticeLevel::Info, format!("Session compacted {times}"));
                    }
                    let mut images = std::mem::take(&mut self.options.initial_images);
                    for message in std::mem::take(&mut self.options.initial_messages) {
                        self.submit_text(&message, std::mem::take(&mut images), false);
                    }
                }
            }
            Tag::Commands => {
                let commands: Vec<JsonValue> = parse(data, "commands").unwrap_or_default();
                let list: Vec<(String, Option<String>, String)> = commands
                    .iter()
                    .filter_map(|c| {
                        Some((
                            c.get("name")?.as_str()?.to_owned(),
                            c.get("description")
                                .and_then(JsonValue::as_str)
                                .map(str::to_owned),
                            c.get("source")
                                .and_then(JsonValue::as_str)
                                .unwrap_or("extension")
                                .to_owned(),
                        ))
                    })
                    .collect();
                self.commands = menu_items(&list);
            }
            Tag::Models(purpose) => {
                let models: Vec<Model> = parse(data, "models").unwrap_or_default();
                let mut providers: Vec<&str> = models.iter().map(|m| m.provider.as_str()).collect();
                providers.sort_unstable();
                providers.dedup();
                self.provider_count = providers.len();
                self.models = models;
                self.on_models(purpose);
            }
            Tag::ThinkingLevels { open } => {
                self.thinking_levels = parse(data, "levels").unwrap_or_default();
                if open {
                    self.open_thinking_selector();
                }
            }
            Tag::Stats { show } => {
                let tokens = data.and_then(|d| d.get("tokens"));
                let n = |k: &str| {
                    tokens
                        .and_then(|t| t.get(k))
                        .and_then(JsonValue::as_u64)
                        .unwrap_or(0)
                };
                self.usage.input = n("input");
                self.usage.output = n("output");
                self.usage.cache_read = n("cacheRead");
                self.usage.cache_write = n("cacheWrite");
                self.usage.cost = data
                    .and_then(|d| d.get("cost"))
                    .and_then(JsonValue::as_f64)
                    .unwrap_or(0.0);
                if show {
                    self.show_session_info(data);
                }
            }
            Tag::ContextUsage => {
                self.usage.context_percent = data
                    .and_then(|d| d.get("percent"))
                    .and_then(JsonValue::as_f64);
                self.usage.context_window = data
                    .and_then(|d| d.get("contextWindow"))
                    .and_then(JsonValue::as_u64)
                    .unwrap_or(0);
            }
            Tag::Bash(id) => {
                self.bash_running = None;
                let outcome = BashOutcome {
                    output: parse(data, "output").unwrap_or_default(),
                    exit_code: parse(data, "exitCode"),
                    cancelled: parse(data, "cancelled").unwrap_or(false),
                    truncated: parse(data, "truncated").unwrap_or(false),
                    full_output_path: parse(data, "fullOutputPath"),
                };
                self.transcript.finish_bash(Some(&id), outcome);
            }
            Tag::Replace(what) => {
                let cancelled: bool = parse(data, "cancelled").unwrap_or(false);
                if cancelled {
                    self.notice(NoticeLevel::Info, format!("{what} cancelled"));
                } else if what == "Clone" {
                    self.notice(NoticeLevel::Info, "Cloned session");
                } else if what == "Import" {
                    self.notice(NoticeLevel::Info, "Session imported");
                }
            }
            Tag::Fork => {
                let cancelled: bool = parse(data, "cancelled").unwrap_or(false);
                if !cancelled && let Some(text) = parse::<String>(data, "text") {
                    self.composer.set_text(&text);
                }
            }
            Tag::Export => {
                if let Some(path) = parse::<String>(data, "path") {
                    self.notice(NoticeLevel::Info, format!("Session exported to: {path}"));
                }
            }
            Tag::Tree => self.open_tree(data),
            Tag::ForkMessages => {
                let messages: Vec<JsonValue> = parse(data, "messages").unwrap_or_default();
                let items = messages
                    .iter()
                    .filter_map(|m| {
                        Some((
                            m.get("entryId")?.as_str()?.to_owned(),
                            m.get("text")?.as_str()?.to_owned(),
                        ))
                    })
                    .collect();
                self.open_fork_picker(items);
            }
            Tag::Navigate => {
                let cancelled: bool = parse(data, "cancelled").unwrap_or(false);
                if let Some(text) = parse::<String>(data, "editorText")
                    && !cancelled
                {
                    self.composer.set_text(&text);
                }
                self.send(Tag::Entries, PiCommand::GetEntries { since: None });
                self.refresh_usage();
            }
            Tag::SetLabel { entry_id, label } => {
                if let Some(Overlay::Tree(tree)) = &mut self.overlay {
                    tree.model.set_label(&entry_id, label, None);
                }
            }
            Tag::CycleModel => {
                if data.is_some_and(JsonValue::is_null) {
                    self.notice(NoticeLevel::Info, "Only one model available");
                } else if let Some(model) = parse::<Model>(data, "model") {
                    let level: Option<ThinkingLevel> = parse(data, "thinkingLevel");
                    if let Some(level) = level {
                        self.session.thinking = level;
                    }
                    self.session.model = Some(model);
                }
            }
            Tag::CycleThinking => {
                if let Some(level) = parse::<ThinkingLevel>(data, "level") {
                    self.session.thinking = level;
                } else {
                    self.notice(NoticeLevel::Info, "Current model does not support thinking");
                }
            }
            Tag::SetModel => {
                if let Some(model) = parse::<Model>(data, "") {
                    self.session.model = Some(model);
                }
                self.send(
                    Tag::ThinkingLevels { open: false },
                    PiCommand::GetAvailableThinkingLevels,
                );
            }
            Tag::SetName(name) => {
                self.session.session_name = Some(name.clone());
                self.notice(NoticeLevel::Info, format!("Session name set: {name}"));
            }
            Tag::Dequeue => {
                let steering: Vec<String> = parse(data, "steering").unwrap_or_default();
                let follow: Vec<String> = parse(data, "followUp").unwrap_or_default();
                let all: Vec<String> = steering.into_iter().chain(follow).collect();
                if all.is_empty() {
                    self.notice(NoticeLevel::Info, "No queued messages");
                } else {
                    let mut text = all.join("\n\n");
                    if !self.composer.is_empty() {
                        text = format!("{text}\n\n{}", self.composer.text());
                    }
                    self.composer.set_text(&text);
                }
            }
        }
    }

    fn on_command_error(&mut self, tag: &Tag, error: &ClientError) {
        match tag {
            Tag::Bash(id) => {
                self.bash_running = None;
                self.transcript.fail_bash(Some(id), &error.to_string());
            }
            // Optional Oino commands a backend may not implement.
            Tag::Hello | Tag::UiSnapshot | Tag::ContextUsage => {}
            _ => self.notice(NoticeLevel::Error, format!("Error: {error}")),
        }
    }

    fn apply_state(&mut self, data: Option<&JsonValue>) {
        let Some(state) = data else {
            return;
        };
        self.session.model = parse(Some(state), "model");
        self.session.thinking = parse(Some(state), "thinkingLevel").unwrap_or_default();
        self.session.streaming = parse(Some(state), "isStreaming").unwrap_or(false);
        self.session.compacting = parse(Some(state), "isCompacting").unwrap_or(false);
        self.session.session_id = parse(Some(state), "sessionId").unwrap_or_default();
        self.session.session_file = parse(Some(state), "sessionFile");
        self.session.session_name = parse(Some(state), "sessionName");
        self.session.auto_compaction = parse(Some(state), "autoCompactionEnabled").unwrap_or(true);
    }

    fn on_sessions(&mut self, result: Result<Vec<SessionItem>, String>) {
        if let Some(Overlay::Sessions(picker)) = &mut self.overlay {
            match result {
                Ok(items) => picker.model.set_items(items),
                Err(error) => {
                    picker.model.set_items(Vec::new());
                    picker.status = Some(error);
                }
            }
        }
    }

    fn on_session_deleted(&mut self, path: &str, result: Result<(), String>) {
        if let Some(Overlay::Sessions(picker)) = &mut self.overlay {
            match result {
                Ok(()) => picker.model.remove(path),
                Err(error) => picker.status = Some(error),
            }
        }
    }

    fn on_clipboard(
        &mut self,
        dialog: Option<String>,
        image: Option<ImageContent>,
        text: Option<String>,
    ) {
        if let Some(id) = dialog {
            let value = image.map_or(
                JsonValue::Null,
                |i| json!({"type": "image", "data": i.data, "mimeType": i.mime_type}),
            );
            self.answer(&id, oino_core::ui::UiAnswer::Value(value));
            return;
        }
        if let Some(image) = image {
            self.composer.insert_attachment("", None, Some(image), None);
        } else if let Some(text) = text {
            self.composer.paste(&text);
        } else {
            self.notice(NoticeLevel::Warning, "Clipboard is empty");
        }
    }

    fn on_edited(&mut self, purpose: EditPurpose, text: Option<String>) {
        match purpose {
            EditPurpose::Composer => {
                if let Some(text) = text {
                    self.composer.set_text(text.trim_end_matches('\n'));
                }
            }
            EditPurpose::Dialog(id) => {
                if let Some(text) = text {
                    if let Some(DialogUi::Editor { input, .. }) = &mut self.dialog_ui {
                        input.set_text(text.trim_end_matches('\n'));
                    }
                } else {
                    let _ = id;
                }
            }
            EditPurpose::External(id) => {
                let answer = text.map_or(oino_core::ui::UiAnswer::Cancelled, |t| {
                    oino_core::ui::UiAnswer::Value(JsonValue::String(t))
                });
                self.answer(&id, answer);
            }
        }
    }

    /// Answers a dialog and removes it locally (core broadcasts the dismissal too).
    pub(crate) fn answer(&mut self, id: &str, answer: oino_core::ui::UiAnswer) {
        self.effects.push(Effect::Respond(UiResponse {
            id: id.to_owned(),
            answer,
        }));
        self.ui.dismiss(id);
        self.sync_dialog();
    }

    pub(crate) fn request_id(&mut self) -> String {
        self.next_request += 1;
        format!("tui-{}", self.next_request)
    }

    /// Current spinner frame (pi loader frames).
    pub(crate) fn spinner(&self) -> &'static str {
        const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        FRAMES[usize::try_from(self.tick % 10).unwrap_or(0)]
    }

    /// Whether something animates (spinner, countdown) and needs periodic redraws.
    #[must_use]
    pub fn animating(&self) -> bool {
        self.busy()
            || self.bash_running.is_some()
            || self
                .ui
                .dialogs
                .front()
                .is_some_and(|d| d.timeout_ms.is_some())
            || self.session.run_state == Some(RunState::Waiting)
    }

    /// Whether any agent activity is in progress.
    pub(crate) fn busy(&self) -> bool {
        self.session.streaming || self.session.compacting || self.retry.is_some()
    }
}
