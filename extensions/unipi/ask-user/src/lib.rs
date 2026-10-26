//! `unipi.ask-user` — unipi `packages/ask-user` (unipi-port §3.5).
//!
//! | unipi | Here |
//! |---|---|
//! | tool `ask_user` (TypeBox schema, `prepareArguments`, sequential) | [`tool_schema`], [`questions::prepare_args`], `execution-mode: sequential` |
//! | `AskPanel` (`ctx.ui.custom`) + phone race | `form` dialog ([`dialog`]); first answer wins across frontends (host) |
//! | answers / clarify / cancel / unavailable texts | [`questions`], `AskUser::run` |
//! | `end_turn` / `new_session` + launcher (Compact & run, Run directly) | [`handoff`]; plus "New session" via hidden command `unipi:ask-user-new-session` |
//! | `syncAskUserTool` on `session_start` / `before_agent_start` | `AskUser::sync_tool` |
//! | `ASK_USER_PROMPT` bus event (notify listens) | `unipi_kit::bus::emit` (+ OS notification when notify is absent) |
//! | `renderResult` / `askRows` | tool renderer ([`render`]) |
//! | settings `ask-user` | [`settings`] |
//! | bundled skill `ask-user` | `skills/ask-user/SKILL.md` (manifest resources) |

#![allow(
    clippy::unused_async_trait_impl,
    reason = "`Extension::init` is async in the SDK; this init registers synchronously"
)]

pub mod dialog;
pub mod handoff;
pub mod questions;
pub mod render;
pub mod settings;

use std::time::Duration;

use oino_extension::prelude::*;
use oino_extension::wit::registry::RendererTarget;
use unipi_kit::provenance::{Harness, HarnessDelivery};
use unipi_kit::settings::Store;
use unipi_kit::topics::{AskUserPrompt, ModuleReady};

use crate::dialog::{Outcome as DialogOutcome, Settings, apply_settings, decode, form_request};
use crate::handoff::{Launch, NEW_SESSION_COMMAND, Once};
use crate::questions::{
    AskOption, AskQuestion, OptionAction, answers_text, clarify_text, prepare_args,
};
use crate::render::{AttachmentInfo, Details, Outcome, Reason};

/// Extension id.
pub const EXTENSION_ID: &str = "unipi.ask-user";
/// Tool name (unipi `ASK_USER_TOOLS.ASK`).
pub const TOOL: &str = "ask_user";

/// Refusal inside a subagent.
pub const SUBAGENT_ERROR: &str = "ask_user is not available inside a subagent. You cannot talk to the user directly — only your lead can. Do not guess an answer: state the question, the options you considered and your recommendation in your report.";
/// Turned off.
pub const DISABLED_TEXT: &str = "ask_user is turned off in settings — ask in your reply instead.";
/// No UI.
pub const NO_UI_TEXT: &str =
    "No interactive UI (non-interactive mode) — ask in your reply instead.";
/// No questions.
pub const EMPTY_ERROR: &str = "ask_user needs at least one question with a question text.";
/// Esc.
pub const CANCEL_TEXT: &str =
    "The user cancelled the questions and stopped the turn. Wait for their next message.";

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// The `ask_user` parameters schema (unipi `TypeBox` schema as JSON Schema).
#[must_use]
pub fn tool_schema(max: usize) -> Value {
    let option = json!({
        "type": "object",
        "properties": {
            "label": {"type": "string", "description": "Display text (1–5 words)"},
            "description": {"type": "string", "description": "What this option means or its trade-offs"},
            "value": {"type": "string", "description": "Returned instead of the label when set"},
            "action": {
                "anyOf": [{"const": "end_turn", "type": "string"}, {"const": "new_session", "type": "string"}],
                "description": "end_turn: stop the turn when picked · new_session: hand off to `prefill` (compact & run / run directly)",
            },
            "prefill": {"type": "string", "description": "Message or /command queued by a new_session option"},
        },
        "required": ["label"],
    });
    let question = json!({
        "type": "object",
        "properties": {
            "question": {"type": "string", "description": "The full question"},
            "header": {"type": "string", "description": format!("Short chip label, e.g. \"Database\" (≤{} chars)", questions::HEADER_MAX)},
            "options": {"type": "array", "items": option, "description": "2–4 choices. An \"Other\" free-text choice is added automatically."},
            "multi_select": {"type": "boolean", "description": "Let the user pick several (default false)"},
            "other": {"type": "boolean", "description": "Offer \"Other (type your own)\" (default true)"},
        },
        "required": ["question", "header", "options"],
    });
    json!({
        "type": "object",
        "properties": {
            "questions": {"type": "array", "items": question, "description": format!("1–{max} question{}", plural(max))},
        },
        "required": ["questions"],
    })
}

/// Tool description for `max` questions.
#[must_use]
pub fn tool_description(max: usize) -> String {
    format!(
        "Ask the user 1–{max} multiple-choice question{} in one dialog and wait for the answers. Each question has a short header, 2–4 options (label + description) and optional multi-select; an \"Other\" free-text choice is always added. The user may skip questions.",
        plural(max)
    )
}

/// Prompt guidelines for `max` questions.
#[must_use]
pub fn guidelines(max: usize) -> Vec<String> {
    vec![
        "Use ask_user when a decision, preference or clarification needs the user before you continue.".into(),
        format!("Put every question you need right now in ONE call (1–{max} question{}) — never several ask_user calls at once.", plural(max)),
        "Give each question a short header (≤16 chars) and 2–4 options with a clear description; don't add an 'Other' option yourself.".into(),
        "Use multi_select when several answers can apply.".into(),
        "Skipped questions come back as skipped: respect that, don't ask them again unless you must.".into(),
        "If the user says they're not ready, ask what they want to clarify instead of re-asking.".into(),
        "Options can carry action: 'end_turn' or action: 'new_session' with a prefill for workflow handoffs.".into(),
    ]
}

/// `"[File #2]"` → `"[File #2: /path]"` so the agent can open documents.
fn expand_file_tokens(text: &str, files: &[(u32, String)]) -> String {
    files.iter().fold(text.to_owned(), |t, (id, path)| {
        t.replace(&format!("[File #{id}]"), &format!("[File #{id}: {path}]"))
    })
}

/// The extension.
#[derive(Default)]
pub struct AskUser {
    settings: Store,
}

impl AskUser {
    /// Adds/removes `ask_user` from the active tools, touching nothing else (unipi
    /// `syncAskUserTool`).
    fn sync_tool(&self, cx: &Ctx) {
        let want = self.current().enabled && !is_subagent(cx);
        let active = control::active_tools();
        if active.iter().any(|t| t == TOOL) == want {
            return;
        }
        let next: Vec<String> = if want {
            active
                .into_iter()
                .chain(std::iter::once(TOOL.to_owned()))
                .collect()
        } else {
            active.into_iter().filter(|t| t != TOOL).collect()
        };
        let _ = control::set_active_tools(&next);
    }
}

fn is_subagent(cx: &Ctx) -> bool {
    cx.is_child() && cx.child_role().is_none_or(|r| r == "subagent")
}

fn text_result(text: impl Into<String>, details: &Details) -> ToolResult {
    ToolResult::text(text).with_details(details)
}

impl AskUser {
    fn current(&self) -> Settings {
        Settings::from_json(&self.settings.get(settings::NAMESPACE).0)
    }

    fn announce(questions: &[AskQuestion]) {
        unipi_kit::bus::emit(&AskUserPrompt {
            question: questions
                .iter()
                .map(|q| q.question.as_str())
                .collect::<Vec<_>>()
                .join(" · "),
            context: None,
            option_count: u32::try_from(questions.iter().map(|q| q.options.len()).sum::<usize>())
                .ok(),
            allow_multiple: Some(questions.iter().any(AskQuestion::multi)),
            allow_freeform: Some(questions.iter().any(AskQuestion::has_other)),
        });
        // Without the notify extension nobody turns the bus event into a notification.
        let notify_loaded = control::commands()
            .unwrap_or_default()
            .iter()
            .any(|c| c.get("name").and_then(Value::as_str) == Some("unipi:notify-event"));
        if !notify_loaded && !host::frontend_focused() {
            let body = questions
                .iter()
                .map(|q| q.question.as_str())
                .collect::<Vec<_>>()
                .join(" · ");
            let _ = ui::os_notify(
                "The agent is waiting for your answer",
                &body,
                "normal",
                None,
            );
        }
    }

    /// The `ask_user` tool.
    async fn run(&self, args: Value, cx: ToolCtx) -> Result<ToolResult> {
        self.settings.refresh();
        let s = self.current();
        let questions = apply_settings(prepare_args(&args), &s);
        if is_subagent(&cx.cx) {
            return Err(Error::msg(SUBAGENT_ERROR));
        }
        let unavailable = |text: &str, reason: Reason| {
            text_result(
                text,
                &Details {
                    questions: questions.clone(),
                    answers: None,
                    outcome: Outcome::Unavailable,
                    reason: Some(reason),
                    question: None,
                    attachments: None,
                },
            )
        };
        if !s.enabled {
            return Ok(unavailable(DISABLED_TEXT, Reason::Disabled));
        }
        if !cx.cx.has_ui() {
            return Ok(unavailable(NO_UI_TEXT, Reason::NoUi));
        }
        if questions.is_empty() {
            return Err(Error::msg(EMPTY_ERROR));
        }
        if s.notify_on_ask {
            Self::announce(&questions);
        }
        let response = ui::dialog(&form_request(&questions, &s))
            .await
            .unwrap_or(Value::Null);
        let legacy = (questions.len() == 1).then(|| questions[0].question.clone());
        let details = |answers: Option<Vec<questions::Answer>>, outcome: Outcome| Details {
            questions: questions.clone(),
            answers,
            outcome,
            reason: None,
            question: legacy.clone(),
            attachments: None,
        };
        match decode(&response, &questions) {
            DialogOutcome::Cancel => {
                Ok(text_result(CANCEL_TEXT, &details(None, Outcome::Cancelled)).terminate())
            }
            DialogOutcome::Clarify(answers) => {
                let text = clarify_text(&questions, &answers);
                Ok(text_result(text, &details(Some(answers), Outcome::Clarify)))
            }
            DialogOutcome::Action {
                option, answers, ..
            } => Ok(Self::run_action(&option, details(Some(answers), Outcome::Action)).await),
            DialogOutcome::Answered {
                mut answers,
                images,
            } => {
                let files = file_tokens(&response);
                for a in &mut answers {
                    if let Some(t) = &a.custom_text {
                        a.custom_text = Some(expand_file_tokens(t, &files));
                    }
                }
                let names: Vec<String> = (1..=images.len())
                    .map(|i| format!("[Image #{i}]"))
                    .collect();
                let mut result = ToolResult::text(answers_text(&questions, &answers, &names));
                result.content.extend(images.iter().cloned());
                let mut d = details(Some(answers), Outcome::Answered);
                let mut infos: Vec<AttachmentInfo> = (1..=images.len())
                    .map(|i| AttachmentInfo {
                        id: u32::try_from(i).unwrap_or(0),
                        kind: "image".into(),
                        path: None,
                        name: None,
                    })
                    .collect();
                infos.extend(files.iter().map(|(id, path)| AttachmentInfo {
                    id: *id,
                    kind: "file".into(),
                    path: Some(path.clone()),
                    name: None,
                }));
                d.attachments = (!infos.is_empty()).then_some(infos);
                Ok(result.with_details(&d))
            }
        }
    }

    /// `end_turn` / `new_session` options (workflow handoffs).
    async fn run_action(option: &AskOption, details: Details) -> ToolResult {
        if option.action == Some(OptionAction::EndTurn) {
            return text_result(
                format!("User chose \"{}\" and ended the turn.", option.label),
                &details,
            )
            .terminate();
        }
        let prefill = option.prefill.clone().unwrap_or_default();
        let launch = match ui::dialog(&handoff::launcher_request(&prefill)).await {
            Ok(v) => handoff::decode_launch(&v),
            Err(_) => Launch::Cancel,
        };
        if launch == Launch::Cancel {
            return text_result(
                format!(
                    "User picked \"{}\" but cancelled the handoff.",
                    option.label
                ),
                &details,
            );
        }
        let Some(prefill) = handoff::normalize_prefill(Some(&prefill)) else {
            return text_result(
                format!("Failed to queue the {} handoff: ", launch.name()),
                &details,
            );
        };
        let queued = match launch {
            Launch::Direct => deliver(&prefill).await,
            Launch::Compact => {
                schedule_compact(prefill.clone());
                true
            }
            Launch::NewSession => {
                let args = prefill.clone();
                wit_spawn(async move {
                    let _ = control::invoke_command(NEW_SESSION_COMMAND, &args).await;
                });
                true
            }
            Launch::Cancel => false,
        };
        let text = if queued {
            format!("Queued {} handoff: {prefill}", launch.name())
        } else {
            format!("Failed to queue the {} handoff: {prefill}", launch.name())
        };
        // The follow-up runs once this turn ends (unipi aborted the turn; terminate is the clean form).
        text_result(text, &details).terminate()
    }

    /// Hidden command: fresh session with `prefill` in the editor.
    async fn new_session(args: String) -> Result<()> {
        let cancelled = control::new_session(None).await?;
        if !cancelled && !args.trim().is_empty() {
            ui::editor_set(args.trim());
        }
        Ok(())
    }
}

/// Files referenced in "Other" (`{answers: [{files: [{id, path}]}]}`, Oino extension).
fn file_tokens(response: &Value) -> Vec<(u32, String)> {
    response
        .pointer("/value/answers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|a| {
            a.get("files")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|f| {
            let id = u32::try_from(f.get("id")?.as_u64()?).ok()?;
            Some((id, f.get("path")?.as_str()?.to_owned()))
        })
        .collect()
}

fn wit_spawn(fut: impl Future<Output = ()> + 'static) {
    #[cfg(target_arch = "wasm32")]
    oino_extension::wit_bindgen::spawn_local(fut);
    #[cfg(not(target_arch = "wasm32"))]
    drop(fut);
}

/// Queues `prefill` as a harness user message (follow-up); falls back to the editor.
async fn deliver(prefill: &str) -> bool {
    let harness = Harness::new("Ask user", "Handoff")
        .synopsis("Prefill from the launcher")
        .delivery(HarnessDelivery::FollowUp);
    let opts = oino_extension::wit::session::UserMessageOptions {
        deliver_as: harness.send_options().deliver_as,
        expand_prompt_templates: false,
        provenance: Some(harness.provenance()),
    };
    let sent =
        oino_extension::wit::session::send_user_message(json!(prefill).to_string(), opts).await;
    if sent.is_ok() {
        return true;
    }
    ui::editor_set(prefill);
    let _ = ui::notify(
        "Automatic handoff failed. The command was placed in the editor; press Enter to run it.",
        "warning",
    );
    true
}

/// Compact, then deliver; a 1.5 s fallback delivers anyway (exactly once).
fn schedule_compact(prefill: String) {
    let once = Once::default();
    let (a, b) = (once.clone(), once);
    let p1 = prefill.clone();
    wit_spawn(async move {
        let result = control::compact(Some(&handoff::compact_instructions(&p1))).await;
        if let Err(e) = result {
            let _ = ui::notify(
                &format!("Compaction reported an error; continuing handoff anyway: {e}"),
                "warning",
            );
        }
        if a.claim() {
            deliver(&p1).await;
        }
    });
    wit_spawn(async move {
        timers::sleep(Duration::from_millis(handoff::COMPACT_HANDOFF_FALLBACK_MS)).await;
        if b.claim() {
            let _ = ui::notify(
                "Compaction is still running; queued the selected handoff command via fallback.",
                "info",
            );
            deliver(&prefill).await;
        }
    });
}

impl Extension for AskUser {
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        let ns = settings::namespace();
        self.settings
            .declare(settings::NAMESPACE, ns.defaults.clone());
        self.settings.load(cx.settings());
        cx.settings_namespace(&ns.to_json());
        let max = self.current().max_questions;

        let mut tool = Tool::<Self>::new(TOOL)
            .label("Ask User")
            .description(tool_description(max))
            .prompt_snippet("Ask the user multiple-choice questions and wait for the answers.")
            .parameters(&tool_schema(max))
            .sequential()
            .prepare_arguments(|raw| json!({"questions": prepare_args(&raw)}))
            .run(|ext, args: Value, cx| ext.run(args, cx));
        for g in guidelines(max) {
            tool = tool.guideline(g);
        }
        if !self.current().enabled {
            tool = tool.inactive();
        }
        cx.tool(tool);
        cx.renderer(RendererTarget::Tool, TOOL, |_ext, req, _cx| {
            render::render_request(&req)
        });
        cx.command(
            Command::<Self>::new(NEW_SESSION_COMMAND)
                .description("Start a new session with a prefilled editor (ask_user handoff)")
                .hidden()
                .run(|_ext, args, _cx| AskUser::new_session(args)),
        );
        cx.on(EventKind::SessionStart, |ext, _ev, cx| async move {
            ext.settings.refresh();
            ext.sync_tool(&cx);
            unipi_kit::bus::emit(&ModuleReady {
                name: unipi_kit::suite::pkg::ASK_USER.npm.to_owned(),
                version: unipi_kit::suite::UNIPI_VERSION.to_owned(),
                commands: Vec::new(),
                tools: vec![TOOL.to_owned()],
                load_time_ms: None,
            });
            Ok(())
        });
        cx.on(EventKind::BeforeAgentStart, |ext, _ev, cx| async move {
            ext.sync_tool(&cx);
            Ok(())
        });
        cx.on(EventKind::OinoSettingsChanged, |ext, ev, cx| async move {
            if ext.settings.on_changed(&ev.payload).is_some() {
                ext.sync_tool(&cx);
            }
            Ok(())
        });
        Ok(())
    }
}

// Native builds (unit tests) skip the component exports.
#[cfg(target_arch = "wasm32")]
oino_extension::export_extension!(AskUser);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_matches_typebox_shape() {
        let s = tool_schema(4);
        assert_eq!(s["required"], json!(["questions"]));
        assert_eq!(s["properties"]["questions"]["description"], "1–4 questions");
        let q = &s["properties"]["questions"]["items"];
        assert_eq!(q["required"], json!(["question", "header", "options"]));
        assert_eq!(
            q["properties"]["header"]["description"],
            "Short chip label, e.g. \"Database\" (≤16 chars)"
        );
        assert_eq!(
            q["properties"]["options"]["items"]["properties"]["action"]["anyOf"][1]["const"],
            "new_session"
        );
        assert_eq!(
            tool_schema(1)["properties"]["questions"]["description"],
            "1–1 question"
        );
    }

    #[test]
    fn description_and_guidelines_follow_max() {
        assert!(
            tool_description(1)
                .starts_with("Ask the user 1–1 multiple-choice question in one dialog")
        );
        assert!(tool_description(3).contains("1–3 multiple-choice questions"));
        assert_eq!(
            guidelines(2)[1],
            "Put every question you need right now in ONE call (1–2 questions) — never several ask_user calls at once."
        );
        assert_eq!(guidelines(4).len(), 7);
    }

    #[test]
    fn file_tokens_expand() {
        let resp =
            json!({"value": {"answers": [{"id": "q0", "files": [{"id": 2, "path": "/a/b.md"}]}]}});
        let files = file_tokens(&resp);
        assert_eq!(files, [(2, "/a/b.md".to_owned())]);
        assert_eq!(
            expand_file_tokens("see [File #2]", &files),
            "see [File #2: /a/b.md]"
        );
    }
}
