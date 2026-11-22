//! Production runtime adapter for dispatcher workers/leads (S0 split from
//! `superagent.rs`; pure move): `apply_mode`, `HostWorkers` and the
//! long-horizon workspace helpers.

use crate::superagent::{dispose_worker, Delivery, PausedGraph, Workers, INTERRUPTED};
use crate::superagent_brief::lead_brief;
use crate::superagent_presets::{Overrides, Preset, Store};
use futures_util::future::BoxFuture;
use oino_core::runtime::{AgentSessionRuntime, SessionOptions, SessionSource};
use oino_core::session::{InputSource, PromptOptions};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use unipi_kanboard::{board::Board, dispatcher, model::Task, Layout, Project};

fn lead_model_options(
    models: &oino_models::ModelRegistry,
    preset: &Preset,
) -> Result<(Option<oino_ai::Model>, Option<oino_ai::ThinkingLevel>), String> {
    let model = preset
        .model
        .as_ref()
        .map(|name| {
            let (provider, id) = name.split_once('/').ok_or("model must be provider/id")?;
            models
                .find(provider, id)
                .ok_or_else(|| format!("Unknown model: {name}"))
        })
        .transpose()?;
    let thinking_level = preset
        .thinking
        .as_ref()
        .map(|level| {
            level
                .parse()
                .map_err(|_| format!("Invalid thinking level: {level}"))
        })
        .transpose()?;
    Ok((model, thinking_level))
}

pub(crate) async fn apply_mode(
    session: &oino_core::session::AgentSession,
    mode: &str,
) -> std::result::Result<(), String> {
    use oino_core::extensions::{api::CallKind, host::HostOp};
    if mode == "none" && session.runner().command("lh-mode").is_none() {
        return Ok(());
    }
    let cx = session.runner().call_context().with_kind(CallKind::Command);
    let result = session
        .host_call(
            "unipi.long-horizon",
            &cx,
            HostOp::InvokeCommand {
                name: "lh-mode".into(),
                args: json!({"mode":mode}).to_string(),
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    if result["ok"] != true {
        return Err(result["reason"]
            .as_str()
            .unwrap_or("Mode switch refused")
            .into());
    }
    Ok(())
}

/// Production runtime adapter. Session files are created/attached by exact reserved id.
pub struct HostWorkers {
    hub: Arc<crate::sessions::SessionHub>,
    layout: Layout,
    runtimes: Mutex<HashMap<String, Arc<AgentSessionRuntime>>>,
}
impl HostWorkers {
    fn configure_lead_approvals(
        &self,
        session: &oino_core::session::AgentSession,
        project: &Project,
        id: &str,
    ) {
        let layout = self.layout.clone();
        let project_for_approval = project.clone();
        let worker_id = id.to_owned();
        session.set_worker_approval_transition(Arc::new(move |request, park| {
            let command = request
                .arguments
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or("");
            let question = format!(
                "Approval needed: {} (risk: {:?}, ceiling: {:?}; {}). Command: {}",
                request.tool, request.risk_level, request.ceiling, request.reason, command
            );
            let board =
                Board::open(&layout, project_for_approval.clone()).map_err(|e| e.to_string())?;
            for task in board.tasks().map_err(|e| e.to_string())? {
                if dispatcher::matches_claim(&task, &worker_id) {
                    dispatcher::approval_transition(
                        &layout,
                        project_for_approval.clone(),
                        &task.id,
                        &worker_id,
                        park.then_some(question.as_str()),
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
            Ok(())
        }));
    }
}

impl HostWorkers {
    #[must_use]
    pub fn new(hub: Arc<crate::sessions::SessionHub>, layout: Layout) -> Self {
        Self {
            hub,
            layout,
            runtimes: Mutex::new(HashMap::new()),
        }
    }
}
impl Workers for HostWorkers {
    fn agent_dir(&self) -> Option<&Path> {
        Some(self.hub.agent_dir())
    }
    fn live<'a>(&'a self, id: &'a str) -> BoxFuture<'a, bool> {
        Box::pin(async move { self.runtimes.lock().await.contains_key(id) })
    }
    fn launch<'a>(
        &'a self,
        project: &'a Project,
        task: &'a Task,
        id: &'a str,
    ) -> BoxFuture<'a, std::result::Result<(), String>> {
        Box::pin(async move {
            let preset = Store::default().resolve(None, Overrides::default())?;
            self.launch_preset(project, task, id, &preset).await
        })
    }
    fn launch_preset<'a>(
        &'a self,
        project: &'a Project,
        task: &'a Task,
        id: &'a str,
        preset: &'a Preset,
    ) -> BoxFuture<'a, std::result::Result<(), String>> {
        self.launch_lead(project, std::slice::from_ref(task), id, preset)
    }
    fn launch_lead<'a>(
        &'a self,
        project: &'a Project,
        tasks: &'a [Task],
        id: &'a str,
        preset: &'a Preset,
    ) -> BoxFuture<'a, std::result::Result<(), String>> {
        Box::pin(async move {
            // Wake/probe/reattach first (finding 3): an existing runtime is
            // reused regardless of the task list; only a genuinely fresh
            // launch (no runtime, no session file) needs a task.
            let runtimes = self.runtimes.lock().await;
            if runtimes.contains_key(id) {
                return Ok(());
            }
            drop(runtimes);
            let task = tasks.first();
            let services = self
                .hub
                .core_services_for(&project.root)
                .await
                .map_err(|e| e.to_string())?;
            let (model, thinking_level) = lead_model_options(&services.models, preset)?;
            let existing = oino_session::SessionManager::find_by_id(
                &project.root,
                id,
                self.hub.sessions_root(),
            )
            .map_err(|e| e.to_string())?;
            if existing.is_none() && task.is_none() {
                // Binding invariant: a zero-task wake/probe may only ever
                // RE-OPEN the reserved id — never mint a fresh session.
                return Err("no session file for this lead id; refusing to mint a new chat".into());
            }
            let attaching = existing.is_some();
            let source =
                existing.map_or_else(|| SessionSource::NewWithId(id.into()), SessionSource::Open);
            let runtime = Arc::new(
                AgentSessionRuntime::create_for_hub(
                    services,
                    SessionOptions {
                        source,
                        model,
                        thinking_level,
                        custom_tools: crate::superagent_session::lead_tools(
                            project.clone(),
                            id,
                            self.hub.sessions_root(),
                        ),
                        session_dir: Some(self.hub.sessions_root().to_path_buf()),
                        name: Some(format!(
                            "Lead · {}",
                            if project.name.is_empty() {
                                &project.slug
                            } else {
                                &project.name
                            }
                        )),
                        ..SessionOptions::default()
                    },
                )
                .await
                .map_err(|e| e.to_string())?,
            );
            let session = runtime.session();
            session.set_unattended();
            session.set_superagent_worker();
            // OIN-80: the lead's own retry budget (30 s, 60 s, 120 s), separate
            // from Graph auto-continue. Session-only override.
            let lead_settings = crate::superagent::current()
                .map(|dispatcher| dispatcher.lead_settings())
                .unwrap_or_default();
            session.with_settings(|s| {
                s.apply_overrides(
                    &json!({"retry":{"maxRetries":lead_settings.retries,"baseDelayMs":30_000,"maxAgentDelayMs":600_000}})
                        .as_object()
                        .cloned()
                        .unwrap_or_default(),
                );
            });
            self.configure_lead_approvals(&session, project, id);
            let ceiling =
                serde_json::from_value(json!(preset.autonomy)).map_err(|e| e.to_string())?;
            session.set_autonomy(ceiling);
            session.with_manager(|m| m.append_custom_entry("oino:superagent-worker", Some(json!({"projectId":project.slug,"projectName":project.name,"taskId":task.map_or(String::new(), |t| t.id.clone()),"author":"superagent"})))).map_err(|e| e.to_string())?;
            session
                .with_manager(oino_session::SessionManager::materialize)
                .map_err(|e| e.to_string())?;
            apply_mode(&session, &preset.mode).await?;
            tracing::info!(actor = "dispatcher", project = %project.slug, session = id, from = "unknown", to = %preset.mode, "lead mode applied");
            let prompt = lead_brief(project, tasks, preset);
            if !attaching {
                session
                    .prompt(
                        prompt,
                        PromptOptions {
                            source: InputSource::Extension,
                            ..PromptOptions::default()
                        },
                    )
                    .await
                    .map_err(|e| e.to_string())?;
            }
            crate::superagent_session::register(Arc::clone(&runtime));
            self.hub.announce_worker(Arc::clone(&runtime)).await;
            self.runtimes.lock().await.insert(id.into(), runtime);
            Ok(())
        })
    }
    fn set_mode<'a>(
        &'a self,
        id: &'a str,
        mode: &'a str,
    ) -> BoxFuture<'a, std::result::Result<(), String>> {
        Box::pin(async move {
            let runtime = self
                .runtimes
                .lock()
                .await
                .get(id)
                .cloned()
                .ok_or("Lead is not live")?;
            apply_mode(&runtime.session(), mode).await
        })
    }
    fn stop<'a>(&'a self, id: &'a str) -> BoxFuture<'a, std::result::Result<(), String>> {
        Box::pin(async move {
            let runtime = self
                .runtimes
                .lock()
                .await
                .get(id)
                .cloned()
                .ok_or("Worker is not live")?;
            runtime.session().abort().await;
            self.runtimes.lock().await.remove(id);
            crate::superagent_session::unregister(id);
            dispose_worker(&runtime).await;
            Ok(())
        })
    }
    fn relay<'a>(
        &'a self,
        id: &'a str,
        text: &'a str,
    ) -> BoxFuture<'a, std::result::Result<(), String>> {
        self.relay_delivery(id, text, Delivery::Queue)
    }
    fn relay_delivery<'a>(
        &'a self,
        id: &'a str,
        text: &'a str,
        delivery: Delivery,
    ) -> BoxFuture<'a, std::result::Result<(), String>> {
        Box::pin(async move {
            let runtime = self
                .runtimes
                .lock()
                .await
                .get(id)
                .cloned()
                .ok_or("Worker is not live")?;
            let session = runtime.session();
            session
                .with_manager(|m| {
                    m.append_custom_entry(
                        "oino:relay",
                        Some(json!({"author":"superagent","text":text})),
                    )
                })
                .map_err(|e| e.to_string())?;
            session.send_custom_message(oino_core::extensions::host::CustomMessageDraft {
            custom_type: "oino:superagent-relay".into(), content: oino_ai::UserContentField::Text(text.into()),
            display: true, details: Some(json!({"author":"superagent", "oinoProvenance":{"kind":"superagent","author":"superagent"}})),
        }, true, Some(match delivery { Delivery::Queue => oino_core::extensions::host::Delivery::FollowUp, Delivery::Steer => oino_core::extensions::host::Delivery::Steer })).await.map_err(|e| e.to_string())?;
            self.hub.announce_worker(runtime).await;
            Ok(())
        })
    }
    fn retire<'a>(&'a self, id: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let runtime = self.runtimes.lock().await.remove(id);
            crate::superagent_session::unregister(id);
            if let Some(runtime) = runtime {
                dispose_worker(&runtime).await;
            }
        })
    }
    fn approvals<'a>(
        &'a self,
        id: &'a str,
    ) -> BoxFuture<'a, Vec<oino_core::session::permission::ApprovalRequest>> {
        Box::pin(async move {
            self.runtimes
                .lock()
                .await
                .get(id)
                .map_or_else(Vec::new, |r| r.session().pending_approvals())
        })
    }
    fn approval_response<'a>(
        &'a self,
        id: &'a str,
        response: oino_core::session::permission::ApprovalResponse,
    ) -> BoxFuture<'a, std::result::Result<(), String>> {
        Box::pin(async move {
            self.runtimes
                .lock()
                .await
                .get(id)
                .ok_or("Worker is not live")?
                .session()
                .approval_response(response)
        })
    }
    fn progress<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Value> {
        Box::pin(async move {
            let runtime = self.runtimes.lock().await.get(id).cloned();
            match runtime {
                Some(runtime) => {
                    let session = runtime.session();
                    let tool = session
                        .last_assistant_message()
                        .and_then(|m| serde_json::to_value(m).ok())
                        .and_then(|m| {
                            m["content"].as_array().and_then(|content| {
                                content
                                    .iter()
                                    .rev()
                                    .find(|b| b["type"] == "toolCall")
                                    .and_then(|b| b["name"].as_str())
                                    .map(str::to_owned)
                            })
                        });
                    json!({"step":format!("{:?}",session.run_state()).to_lowercase(),"tool":tool,"cost":session.session_stats().cost,"model":session.model().map(|m| format!("{}/{}", m.provider, m.id)),"thinking":session.thinking_level().to_string(),"autonomy":session.autonomy()})
                }
                None => json!({"step":null,"tool":null,"cost":null}),
            }
        })
    }
    fn ended<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Option<String>> {
        Box::pin(async move {
            let runtime = self.runtimes.lock().await.get(id).cloned();
            let Some(runtime) = runtime else {
                return Some(INTERRUPTED.into());
            };
            let session = runtime.session();
            if session.is_streaming() || session.pending_message_count() > 0 {
                return None;
            }
            // Use the same wake/quiet split as core's waiting state and session cards.
            // Query directly rather than relying on the periodically refreshed UI state.
            if !oino_core::events::split_wait_reasons(session.runner().arbiter_wait_reasons().await)
                .0
                .is_empty()
                || has_long_horizon_owner(session.agent_dir(), &session.work_cwd(), id)
                || session.is_streaming()
                || session.pending_message_count() > 0
            {
                None
            } else {
                Some(session.last_assistant_text().unwrap_or_default())
            }
        })
    }
    // --- OIN-80 lead-machine methods ---
    /// The turn outcome: a provider error (`stop_reason == Error`) becomes
    /// `Some("provider error: …")`; a clean end is `Some(last text)`; busy None.
    fn outcome<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Option<String>> {
        Box::pin(async move {
            let runtime = self.runtimes.lock().await.get(id).cloned()?;
            let session = runtime.session();
            if session.is_streaming() || session.pending_message_count() > 0 {
                return None;
            }
            if !oino_core::events::split_wait_reasons(session.runner().arbiter_wait_reasons().await)
                .0
                .is_empty()
                || has_long_horizon_owner(session.agent_dir(), &session.work_cwd(), id)
            {
                return None;
            }
            let message = session.last_assistant_message()?;
            if message.stop_reason == oino_ai::StopReason::Error {
                return Some(format!(
                    "{}{}",
                    crate::superagent_leads::ERROR_OUTCOME_PREFIX,
                    message
                        .error_message
                        .clone()
                        .unwrap_or_else(|| "provider error".into())
                ));
            }
            Some(session.last_assistant_text().unwrap_or_default())
        })
    }
    fn set_model<'a>(
        &'a self,
        id: &'a str,
        model: &'a str,
    ) -> BoxFuture<'a, std::result::Result<(), String>> {
        Box::pin(async move {
            let runtime = self
                .runtimes
                .lock()
                .await
                .get(id)
                .cloned()
                .ok_or("Lead is not live")?;
            let (provider, model_id) = model.split_once('/').ok_or("model must be provider/id")?;
            let resolved = runtime
                .session()
                .models()
                .find(provider, model_id)
                .ok_or_else(|| format!("Unknown model: {model}"))?;
            runtime
                .session()
                .set_model(resolved, false)
                .await
                .map_err(|e| e.to_string())?;
            Ok(())
        })
    }
    fn continue_turn<'a>(
        &'a self,
        id: &'a str,
        text: &'a str,
    ) -> BoxFuture<'a, std::result::Result<(), String>> {
        Box::pin(async move {
            let runtime = self
                .runtimes
                .lock()
                .await
                .get(id)
                .cloned()
                .ok_or("Worker is not live")?;
            let session = runtime.session();
            session
                .with_manager(|m| {
                    m.append_custom_entry(
                        "oino:lead-continue",
                        Some(json!({"author":"host","text":text})),
                    )
                })
                .map_err(|e| e.to_string())?;
            session
                .send_custom_message(
                    oino_core::extensions::host::CustomMessageDraft {
                        custom_type: "oino:lead-continue".into(),
                        content: oino_ai::UserContentField::Text(text.into()),
                        display: false,
                        details: Some(json!({"author":"host"})),
                    },
                    true,
                    Some(oino_core::extensions::host::Delivery::FollowUp),
                )
                .await
                .map_err(|e| e.to_string())?;
            self.hub.announce_worker(runtime).await;
            Ok(())
        })
    }
    /// Warm-idle unload: dispose the runtime but keep the session file so the
    /// record can re-attach to the same id later.
    fn unload<'a>(&'a self, id: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let runtime = self.runtimes.lock().await.remove(id);
            crate::superagent_session::unregister(id);
            if let Some(runtime) = runtime {
                dispose_worker(&runtime).await;
            }
        })
    }
    fn session_file<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Option<std::path::PathBuf>> {
        Box::pin(async move {
            self.runtimes
                .lock()
                .await
                .get(id)
                .and_then(|runtime| runtime.session().session_file())
        })
    }
    fn core_services<'a>(
        &'a self,
        project: &'a Project,
    ) -> BoxFuture<'a, std::result::Result<std::sync::Arc<oino_core::services::Services>, String>>
    {
        Box::pin(async move {
            self.hub
                .core_services_for(&project.root)
                .await
                .map_err(|e| e.to_string())
        })
    }
    /// OIN-81: the lead's open `ask_user` dialogs — `method == "form"` raised by
    /// `unipi.ask-user` (the extension's form dialog), newest first.
    fn questions<'a>(
        &'a self,
        id: &'a str,
    ) -> BoxFuture<'a, Vec<crate::superagent_questions::LeadQuestion>> {
        Box::pin(async move {
            let runtime = self.runtimes.lock().await.get(id).cloned();
            let Some(runtime) = runtime else {
                return Vec::new();
            };
            runtime
                .session()
                .ui()
                .snapshot()
                .into_iter()
                .filter(|request| {
                    request.method == "form"
                        && request.extension_id.as_deref() == Some("unipi.ask-user")
                })
                .map(|request| {
                    let questions = request.params.get("questions").and_then(Value::as_array);
                    let texts = questions
                        .map(|questions| {
                            questions
                                .iter()
                                .filter_map(|question| {
                                    question.get("question").and_then(Value::as_str)
                                })
                                .map(str::to_owned)
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    let options = questions
                        .map(|questions| {
                            questions
                                .iter()
                                .map(|question| {
                                    question
                                        .get("options")
                                        .and_then(Value::as_array)
                                        .map(|options| {
                                            options
                                                .iter()
                                                .filter_map(|option| {
                                                    option.get("label").and_then(Value::as_str)
                                                })
                                                .map(str::to_owned)
                                                .collect::<Vec<_>>()
                                        })
                                        .unwrap_or_default()
                                })
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    let question_ids = questions
                        .map(|questions| {
                            questions
                                .iter()
                                .enumerate()
                                .map(|(idx, question)| {
                                    question
                                        .get("id")
                                        .and_then(Value::as_str)
                                        .map_or_else(|| format!("q{idx}"), str::to_owned)
                                })
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    let allow_other = questions
                        .map(|questions| {
                            questions
                                .iter()
                                .map(|question| {
                                    question
                                        .get("other")
                                        .and_then(Value::as_bool)
                                        .unwrap_or(true)
                                })
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    crate::superagent_questions::LeadQuestion {
                        request_id: request.id,
                        texts,
                        options,
                        question_ids,
                        allow_other,
                    }
                })
                .collect()
        })
    }
    /// OIN-81: deliver a work-pane answer into the waiting `ask_user` tool.
    /// First-wins; `Err` when the lead is gone or the dialog is already closed.
    fn answer<'a>(
        &'a self,
        id: &'a str,
        request_id: &'a str,
        answer: oino_core::ui::UiAnswer,
    ) -> BoxFuture<'a, std::result::Result<(), String>> {
        Box::pin(async move {
            let runtime = self
                .runtimes
                .lock()
                .await
                .get(id)
                .cloned()
                .ok_or("Worker is not live")?;
            let open = runtime
                .session()
                .ui()
                .snapshot()
                .iter()
                .any(|request| request.id == request_id);
            if !open {
                return Err("The question is no longer open".into());
            }
            runtime.session().ui().respond(oino_core::ui::UiResponse {
                id: request_id.to_owned(),
                answer,
            });
            Ok(())
        })
    }
    fn lead_notice<'a>(
        &'a self,
        id: &'a str,
        title: &'a str,
        message: &'a str,
    ) -> BoxFuture<'a, bool> {
        Box::pin(
            async move { crate::notify::lead_notice_with_hub(&self.hub, id, title, message).await },
        )
    }
    fn is_session_archived<'a>(
        &'a self,
        id: &'a str,
        session_file: Option<&'a Path>,
    ) -> BoxFuture<'a, bool> {
        Box::pin(async move { session_in_trash(&self.hub.trash_dir(), id, session_file) })
    }
}

fn file_has_exact_session_id(path: &Path, expected_id: &str) -> bool {
    use std::io::{BufRead, BufReader, Read};
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let mut reader = BufReader::new(file).take(1024 * 1024);
    let mut line = String::new();
    loop {
        line.clear();
        let Ok(read) = reader.read_line(&mut line) else {
            return false;
        };
        if read == 0 {
            return false;
        }
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            return matches!(
                oino_session::FileLine::parse(trimmed),
                Ok(oino_session::FileLine::Header(header)) if header.id == expected_id
            );
        }
    }
}

/// Exact archive identity check (N4): returns `true` ONLY when `<agent_dir>/trash`
/// contains a valid session `.jsonl` file whose parsed `SessionHeader.id` equals `id`.
/// Basename-only, prefix, and extended-id matches are rejected.
pub fn session_in_trash(trash: &Path, id: &str, session_file: Option<&Path>) -> bool {
    if id.is_empty() || !trash.is_dir() {
        return false;
    }
    let fast_candidate = session_file
        .and_then(Path::file_name)
        .map(|name| trash.join(name));
    if let Some(candidate) = fast_candidate.as_deref() {
        if candidate.is_file() && file_has_exact_session_id(candidate, id) {
            return true;
        }
    }
    let Ok(entries) = std::fs::read_dir(trash) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if fast_candidate.as_deref() == Some(path.as_path()) {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") || !path.is_file() {
            continue;
        }
        if file_has_exact_session_id(&path, id) {
            return true;
        }
    }
    false
}

pub(crate) fn long_horizon_workspace_dir(agent_dir: &Path, cwd: &Path) -> PathBuf {
    let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    let root = cwd
        .ancestors()
        .find(|dir| dir.join(".git").exists())
        .unwrap_or(&cwd);
    agent_dir
        .join("state/unipi.long-horizon/workspace")
        .join(oino_wasm_host::workspace_id(root))
}

// Long-horizon owners are workspace-scoped and may be parked without an arbiter
// wait reason. Read the extension's durable owner record, respecting chat ownership.
pub(crate) fn has_long_horizon_owner(agent_dir: &Path, cwd: &Path, id: &str) -> bool {
    let path = long_horizon_workspace_dir(agent_dir, cwd).join("state.json");
    let Ok(Some(state)) = crate::data_dir::read_json::<Value>(&path) else {
        return false;
    };
    ["active", "parked"].iter().any(|key| {
        state.get(*key).is_some_and(|owner| {
            owner.is_object()
                && owner
                    .get("session")
                    .and_then(Value::as_str)
                    .is_none_or(|session| session == id)
        })
    })
}

pub(crate) fn paused_graph_for_lead(
    agent_dir: &Path,
    cwd: &Path,
    current_session_id: &str,
) -> Option<PausedGraph> {
    let ws_dir = long_horizon_workspace_dir(agent_dir, cwd);
    let state = crate::data_dir::read_json::<Value>(&ws_dir.join("state.json"))
        .ok()
        .flatten()?;
    if state.get("active").is_some_and(|v| !v.is_null()) {
        return None;
    }
    let parked = state.get("parked").filter(|v| v.is_object())?;
    if parked.get("kind").and_then(Value::as_str) != Some("graph") {
        return None;
    }
    let graph = crate::data_dir::read_json::<Value>(&ws_dir.join("graph.json"))
        .ok()
        .flatten();
    let current_graph = graph
        .as_ref()
        .and_then(|g| g.get("current"))
        .filter(|v| v.is_object());
    let session_id = parked
        .get("session")
        .and_then(Value::as_str)
        .or_else(|| {
            current_graph
                .and_then(|g| g.get("ownerSession"))
                .and_then(Value::as_str)
        })
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != current_session_id)?
        .to_owned();
    let title = parked
        .get("label")
        .and_then(Value::as_str)
        .or_else(|| {
            current_graph
                .and_then(|g| g.get("task"))
                .and_then(Value::as_str)
        })
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    Some(PausedGraph { session_id, title })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header_line(id: &str) -> String {
        format!(
            "{{\"type\":\"session\",\"version\":3,\"id\":\"{id}\",\"timestamp\":\"2026-11-11T00:00:00.000Z\",\"cwd\":\"/tmp\"}}\n"
        )
    }

    #[tokio::test]
    async fn archive_identity_requires_exact_header_session_id_match() {
        let dir = tempfile::tempdir().unwrap();
        let agent_dir = dir.path().join("agent");
        let trash = agent_dir.join("trash");
        std::fs::create_dir_all(&trash).unwrap();

        let layout = Layout::with_home(dir.path().join("board"));
        let hub = crate::sessions::SessionHub::create(crate::sessions::HostHubOptions {
            cwd: dir.path().into(),
            agent_dir: agent_dir.clone(),
            docs_root: None,
            session_dir: Some(dir.path().join("sessions")),
            mode: oino_core::extensions::api::ExtensionMode::Desktop,
            has_ui: false,
            approve: Some(false),
            flags: oino_core::services::ResourceFlags {
                no_context_files: true,
                ..Default::default()
            },
            loader: Arc::new(oino_core::services::StaticLoader(Vec::new())),
            models: Some(oino_models::ModelRegistry::in_memory(Arc::new(
                oino_models::MapEnv::default(),
            ))),
            tools: oino_core::runtime::ToolSelection::Default,
            exclude_tools: Vec::new(),
            services_cap: None,
            host_data_dir: None,
        })
        .unwrap();
        let workers = HostWorkers::new(hub, layout);
        let expected_file = dir
            .path()
            .join("sessions/2026-11-11T00-00-00_lead-1234.jsonl");

        // 1. Same basename in trash, but DIFFERENT header session id -> NOT archived.
        let same_basename = trash.join("2026-11-11T00-00-00_lead-1234.jsonl");
        std::fs::write(&same_basename, header_line("other-session-9999")).unwrap();
        assert!(!session_in_trash(&trash, "lead-1234", Some(&expected_file)));
        assert!(
            !workers
                .is_session_archived("lead-1234", Some(&expected_file))
                .await
        );
        std::fs::remove_file(&same_basename).unwrap();

        // 2. Extended id / unique prefix in trash (`lead-1234-extended` or `lead-123456`) -> NOT archived.
        std::fs::write(
            trash.join("2026-11-11T00-00-00_lead-1234-extended.jsonl"),
            header_line("lead-1234-extended"),
        )
        .unwrap();
        std::fs::write(
            trash.join("2026-11-11T00-00-00_lead-123456.jsonl"),
            header_line("lead-123456"),
        )
        .unwrap();
        assert!(!session_in_trash(&trash, "lead-1234", Some(&expected_file)));
        assert!(
            !workers
                .is_session_archived("lead-1234", Some(&expected_file))
                .await
        );

        // 3. Exact header id match (even under a deduplicated trash filename `.1.jsonl`) -> archived.
        std::fs::write(
            trash.join("2026-11-11T00-00-00_lead-1234.1.jsonl"),
            header_line("lead-1234"),
        )
        .unwrap();
        assert!(session_in_trash(&trash, "lead-1234", Some(&expected_file)));
        assert!(
            workers
                .is_session_archived("lead-1234", Some(&expected_file))
                .await
        );
    }
}
