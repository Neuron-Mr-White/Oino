//! Restricted Superagent chat: no workspace, extensions, filesystem or shell tools.
use futures_util::future::BoxFuture;
use oino_agent::{
    AgentTool, AgentToolResult, ToolDefinitionMeta, ToolError, ToolInvocation, UpdateSink,
};
use oino_core::{
    runtime::{AgentSessionRuntime, SessionOptions, ToolSelection},
    services::{ResourceFlags, Services, ServicesOptions},
    session::ToolEntry,
};
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{Arc, OnceLock},
};
use tokio_util::sync::CancellationToken;

/// Create a workspace-free session over a fresh isolated services set. The normal
/// v3 prompt freezing/prefix machinery is retained; the marker suppresses cwd probes.
pub async fn create(
    hub: Arc<crate::sessions::SessionHub>,
) -> Result<Arc<AgentSessionRuntime>, String> {
    create_from(hub, None).await
}

/// Resume only through the restricted tool/session factory. Never wake through generic services.
pub async fn create_from(
    hub: Arc<crate::sessions::SessionHub>,
    file: Option<PathBuf>,
) -> Result<Arc<AgentSessionRuntime>, String> {
    let base = hub.core_services().await.map_err(|e| e.to_string())?;
    let services = Services::create(ServicesOptions {
        cwd: hub.agent_dir().to_path_buf(),
        agent_dir: hub.agent_dir().to_path_buf(),
        docs_root: PathBuf::new(),
        mode: oino_core::extensions::api::ExtensionMode::Desktop,
        has_ui: true,
        flags: ResourceFlags {
            no_extensions: true,
            no_context_files: true,
            no_skills: true,
            no_prompt_templates: true,
            ..ResourceFlags::default()
        },
        extension_flag_values: Default::default(),
        loader: Arc::new(crate::sessions::services::HostLoader::build(
            hub.agent_dir(),
            hub.agent_dir(),
            true,
            &[],
        )),
        approve: Some(false),
        models: Some(base.models.clone()),
        settings: None,
    })
    .await
    .map_err(|e| e.to_string())?;
    let session = Arc::new(OnceLock::new());
    let specs = [
        (
            "kanboard",
            "Read or mutate an explicit project board. Supply projectId and CLI argv; never guess a project.",
            json!({"projectId":{"type":"string"},"argv":{"type":"array","items":{"type":"string"}}}),
            vec!["projectId", "argv"],
        ),
        (
            "project_resolver",
            "List registered projects, recent sidebar workspaces and zoxide paths. Ask the user when ambiguous.",
            json!({}),
            vec![],
        ),
        (
            "superagent.start",
            "Start one explicit todo ticket through the dispatcher.",
            json!({"projectId":{"type":"string"},"taskId":{"type":"string"}}),
            vec!["projectId", "taskId"],
        ),
        (
            "superagent.stop",
            "Stop the worker claiming an explicit ticket.",
            json!({"projectId":{"type":"string"},"taskId":{"type":"string"}}),
            vec!["projectId", "taskId"],
        ),
        (
            "superagent.relay",
            "Persist a comment and relay to the ticket's current claiming session.",
            json!({"projectId":{"type":"string"},"taskId":{"type":"string"},"text":{"type":"string"}}),
            vec!["projectId", "taskId", "text"],
        ),
        (
            "ask_user",
            "Ask the user for missing project selection or a consequential decision.",
            json!({"question":{"type":"string"}}),
            vec!["question"],
        ),
    ];

    let tools = specs.into_iter().map(|(name, description, properties, required)| ToolEntry::builtin(Arc::new(ChatTool {
        meta: ToolDefinitionMeta::new(name, name, description, json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})),
        hub: Arc::clone(&hub), session: Arc::clone(&session),
    }), Some(description.into()), Vec::new())).collect();
    let cwd = hub.agent_dir().join("superagent");
    std::fs::create_dir_all(&cwd).map_err(|e| e.to_string())?;
    let runtime = Arc::new(
        AgentSessionRuntime::create_for_hub(
            services,
            SessionOptions {
                source: file.map_or(
                    oino_core::runtime::SessionSource::New,
                    oino_core::runtime::SessionSource::Open,
                ),
                cwd: Some(cwd),
                session_dir: Some(hub.sessions_root().to_path_buf()),
                name: Some("Superagent".into()),
                tools: ToolSelection::NoBuiltin,
                custom_tools: tools,
                ..SessionOptions::default()
            },
        )
        .await
        .map_err(|e| e.to_string())?,
    );
    let agent = runtime.session();
    agent
        .with_manager(|m| {
            if !m.branch(None).iter().any(|e| matches!(&e.kind, oino_session::EntryKind::Custom { custom_type, .. } if custom_type == "oino:superagent")) {
                m.append_custom_entry("oino:superagent", Some(json!({"workspace":null})))?;
                m.append_custom_message_entry("oino:superagent-context", oino_ai::UserContentField::Text("Superagent session has no workspace. Manage explicit project boards and dispatch workers; ask the user if project selection is ambiguous.".into()), false, Some(json!({"author":"host"})))?;
            }
            m.materialize()
        })
        .map_err(|e| e.to_string())?;
    session
        .set(agent.ui())
        .map_err(|_| "Superagent already initialized")?;
    register(Arc::clone(&runtime));
    Ok(runtime)
}
struct ChatTool {
    meta: ToolDefinitionMeta,
    hub: Arc<crate::sessions::SessionHub>,
    session: Arc<OnceLock<oino_core::ui::UiBridge>>,
}
impl AgentTool for ChatTool {
    fn meta(&self) -> &ToolDefinitionMeta {
        &self.meta
    }
    fn execute(
        &self,
        call: ToolInvocation,
        _: UpdateSink,
        cancel: CancellationToken,
    ) -> BoxFuture<'static, Result<AgentToolResult, ToolError>> {
        let name = self.meta.name.clone();
        let hub = Arc::clone(&self.hub);
        let session = Arc::clone(&self.session);
        Box::pin(async move {
            let args =
                serde_json::to_value(call.args).map_err(|e| ToolError::new(e.to_string()))?;
            let field = |key: &str| {
                args[key]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .map(str::to_owned)
                    .ok_or_else(|| ToolError::new(format!("{key} is required")))
            };
            let value = match name.as_str() {
                "project_resolver" => {
                    let registered = unipi_kanboard::Layout::from_env()
                        .and_then(|l| l.list_projects())
                        .map_err(|e| ToolError::new(e.to_string()))?;
                    let recent = hub
                        .projects()
                        .await
                        .map_err(|e| ToolError::new(e.to_string()))?;
                    let zoxide = tokio::time::timeout(
                        std::time::Duration::from_secs(3),
                        tokio::process::Command::new("zoxide")
                            .args(["query", "--list"])
                            .kill_on_drop(true)
                            .output(),
                    )
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .filter(|o| o.status.success())
                    .map(|o| {
                        String::from_utf8_lossy(&o.stdout)
                            .lines()
                            .map(str::to_string)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                    json!({"registered":registered,"recent":recent,"zoxide":zoxide,"instruction":"Use an explicit registered projectId. Ask the user if ambiguous; do not guess."})
                }
                "kanboard" => {
                    let project_id = field("projectId")?;
                    let layout = unipi_kanboard::Layout::from_env()
                        .map_err(|e| ToolError::new(e.to_string()))?;
                    let project = unipi_kanboard::Project::load(&layout, &project_id)
                        .map_err(|e| ToolError::new(e.to_string()))?;
                    let argv: Vec<String> = serde_json::from_value(args["argv"].clone())
                        .map_err(|e| ToolError::new(e.to_string()))?;
                    let command = argv.first().map(String::as_str).unwrap_or("");
                    if !matches!(
                        command,
                        "add"
                            | "edit"
                            | "link"
                            | "unlink"
                            | "order"
                            | "move"
                            | "note"
                            | "list"
                            | "show"
                            | "search"
                            | "next"
                            | "chain"
                            | "status"
                    ) {
                        return Err(ToolError::new(
                            "Superagent manages boards; use superagent.start/stop for workers",
                        ));
                    }
                    if command == "move"
                        && !matches!(argv.get(2).map(String::as_str), Some("backlog" | "todo"))
                    {
                        return Err(ToolError::new(
                            "Superagent may schedule backlog/todo; worker terminal transitions belong to the worker",
                        ));
                    }
                    // Global flags cannot override the explicitly validated target/actor.
                    if argv.iter().any(|s| {
                        s.starts_with("--project")
                            || s.starts_with("--actor")
                            || s.starts_with("--home")
                            || s.starts_with("--session")
                            || s.starts_with("--gate")
                    }) {
                        return Err(ToolError::new("Global board overrides are not allowed"));
                    }
                    unipi_kanboard::service::exec(&unipi_kanboard::service::ExecRequest {
                        argv: [
                            vec![
                                "--project".into(),
                                project_id,
                                "--actor".into(),
                                "user".into(),
                            ],
                            argv,
                        ]
                        .concat(),
                        cwd: Some(project.root),
                        ..Default::default()
                    })
                }
                "ask_user" => {
                    let question = field("question")?;
                    let session = session
                        .get()
                        .ok_or_else(|| ToolError::new("Superagent not initialized"))?;
                    let params = serde_json::from_value(json!({"title":question}))
                        .map_err(|e| ToolError::new(e.to_string()))?;
                    match session
                        .dialog(Some("superagent"), "input", params, Some(cancel))
                        .await
                    {
                        oino_core::ui::UiAnswer::Value(value) => json!({"answer":value}),
                        oino_core::ui::UiAnswer::Confirmed(value) => json!({"confirmed":value}),
                        oino_core::ui::UiAnswer::Cancelled => json!({"cancelled":true}),
                    }
                }
                _ => {
                    let project = field("projectId")?;
                    let task = field("taskId")?;
                    let dispatcher =
                        crate::superagent::current().map_err(|e| ToolError::new(e.to_string()))?;
                    match name.as_str() {
                        "superagent.start" => {
                            json!({"sessionId":dispatcher.start(&project, &task).await.map_err(|e| ToolError::new(e.to_string()))?})
                        }
                        "superagent.stop" => {
                            json!({"stopped":dispatcher.stop(&project, &task).await.map_err(|e| ToolError::new(e.to_string()))?})
                        }
                        "superagent.relay" => dispatcher
                            .relay(&project, &task, &field("text")?)
                            .await
                            .map_err(|e| ToolError::new(e.to_string()))?,
                        _ => return Err(ToolError::new("Unknown Superagent tool")),
                    }
                }
            };
            Ok(AgentToolResult::text(value.to_string()))
        })
    }
}

static RUNTIMES: OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, Arc<AgentSessionRuntime>>>,
> = OnceLock::new();
pub fn register(runtime: Arc<AgentSessionRuntime>) {
    RUNTIMES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(runtime.session().session_id(), runtime);
}
pub fn runtime(id: &str) -> Option<Arc<AgentSessionRuntime>> {
    let runtimes = RUNTIMES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    runtimes.get(id).cloned().or_else(|| {
        runtimes
            .values()
            .find(|r| {
                r.session()
                    .session_file()
                    .is_some_and(|p| p == std::path::Path::new(id))
            })
            .cloned()
    })
}

/// Worker board calls carry the raw preallocated Run.session (not legacy `oino-<id>` tags).
pub fn worker_board(project: unipi_kanboard::Project, session: String) -> ToolEntry {
    ToolEntry::builtin(
        Arc::new(WorkerBoard {
            project,
            session,
            meta: ToolDefinitionMeta::new(
                "kanboard",
                "Kanboard",
                "Read/transition your assigned ticket using CLI argv (show, start, finish, move blocked). Session/project identity is supplied by the host.",
                json!({"type":"object","properties":{"argv":{"type":"array","items":{"type":"string"}}},"required":["argv"],"additionalProperties":false}),
            ),
        }),
        Some("Read and transition the assigned board ticket".into()),
        Vec::new(),
    )
}
struct WorkerBoard {
    project: unipi_kanboard::Project,
    session: String,
    meta: ToolDefinitionMeta,
}
impl AgentTool for WorkerBoard {
    fn meta(&self) -> &ToolDefinitionMeta {
        &self.meta
    }
    fn execute(
        &self,
        call: ToolInvocation,
        _: UpdateSink,
        _: CancellationToken,
    ) -> BoxFuture<'static, Result<AgentToolResult, ToolError>> {
        let project = self.project.clone();
        let session = self.session.clone();
        Box::pin(async move {
            let argv: Vec<String> =
                serde_json::from_value(call.args.get("argv").cloned().unwrap_or_default())
                    .map_err(|e| ToolError::new(e.to_string()))?;
            if argv.iter().any(|s| {
                s.starts_with("--project")
                    || s.starts_with("--actor")
                    || s.starts_with("--session")
                    || s.starts_with("--gate")
            }) {
                return Err(ToolError::new("Worker identity cannot be overridden"));
            }
            let result = unipi_kanboard::service::exec(&unipi_kanboard::service::ExecRequest {
                argv: [
                    vec![
                        "--project".into(),
                        project.slug,
                        "--session".into(),
                        session,
                        "--actor".into(),
                        "agent".into(),
                    ],
                    argv,
                ]
                .concat(),
                cwd: Some(project.root),
                ..Default::default()
            });
            Ok(AgentToolResult::text(result.to_string()))
        })
    }
}

/// A bounded header-prefix check, not a generic session wake (which could expose shell tools).
pub fn is_superagent_file(path: &std::path::Path) -> bool {
    use std::io::{BufRead, BufReader};
    use std::sync::Mutex;
    type Cache = std::collections::HashMap<PathBuf, (std::time::SystemTime, u64, bool)>;
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let Ok(meta) = std::fs::metadata(path) else { return false };
    let Ok(modified) = meta.modified() else { return false };
    let cache = CACHE.get_or_init(Default::default);
    if let Some((_, _, result)) = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(path).filter(|(time, len, _)| *time == modified && *len == meta.len()) {
        return *result;
    }
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let result = BufReader::new(file)
        .lines()
        .take(32)
        .filter_map(Result::ok)
        .any(|line| {
            serde_json::from_str::<serde_json::Value>(&line)
                .ok()
                .is_some_and(|v| {
                    (v["type"] == "custom" && v["customType"] == "oino:superagent")
                        || (v["type"] == "session" && v["cwd"].as_str() == Some(""))
                })
        });
    cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(path.to_path_buf(), (modified, meta.len(), result));
    result
}

/// Read the worker marker near the session header, cached until the file changes.
pub fn worker_file(path: &std::path::Path) -> Option<unipi_proto::session::WorkerCard> {
    use std::io::{BufRead, BufReader};
    use std::sync::Mutex;
    type Cache = std::collections::HashMap<PathBuf, (std::time::SystemTime, u64, Option<unipi_proto::session::WorkerCard>)>;
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    let cache = CACHE.get_or_init(Default::default);
    if let Some((_, _, result)) = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(path).filter(|(time, len, _)| *time == modified && *len == meta.len()) {
        return result.clone();
    }
    let file = std::fs::File::open(path).ok()?;
    let result = BufReader::new(file).lines().take(32).map_while(Result::ok).find_map(|line| {
        let value: serde_json::Value = serde_json::from_str(&line).ok()?;
        if value["type"] != "custom" || value["customType"] != "oino:superagent-worker" { return None; }
        serde_json::from_value(value["data"].clone()).ok()
    });
    cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(path.to_path_buf(), (modified, meta.len(), result.clone()));
    result
}

pub fn unregister(id: &str) {
    RUNTIMES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(id);
}
