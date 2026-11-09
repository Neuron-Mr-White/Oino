//! Restricted Superagent chat: memory extension only, no filesystem or shell tools.
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
            no_context_files: true,
            no_skills: true,
            no_prompt_templates: true,
            ..ResourceFlags::default()
        },
        extension_flag_values: Default::default(),
        loader: Arc::new(crate::sessions::services::HostLoader::memory_only(hub.agent_dir())),
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
            "Read or mutate a project board with projectId and CLI argv. For onboarding use argv [\"project\",\"add\",\"/absolute/path\"] (requires user confirmation); project list and show are also available.",
            json!({"projectId":{"type":"string"},"argv":{"type":"array","items":{"type":"string"}}}),
            vec!["argv"],
        ),
        (
            "project_resolver",
            "List registered projects, recent sidebar workspaces and zoxide paths. Ask the user when ambiguous.",
            json!({"reason":{"type":"string","description":"Why project discovery is needed."}}),
            vec![],
        ),
        (
            "superagent_start",
            "Start or queue tickets with the project's lead. Tickets may be backlog, todo or blocked. Optional overrides: model (string, \"provider/id\"), thinking (\"off\"|\"minimal\"|\"low\"|\"medium\"|\"high\"|\"xhigh\"|\"max\"), autonomy (\"low\"|\"medium\"|\"high\"), mode (\"none\"|\"goal\"|\"ralph\"|\"swarm\"|\"graph\"), maxParallel (integer >= 1), review (\"none\"|\"light\"|\"full\"), profiles (string array), priority (boolean). Manual start works even when Auto is off; unregistered or archived projects require user confirmation. Do not move tickets to todo before calling start; start takes backlog, todo or blocked tickets itself (moving to todo first lets auto-start launch a lead with the default preset).",
            json!({"projectId":{"type":"string"},"taskIds":{"type":"array","items":{"type":"string"}},"presetId":{"type":"string"},"overrides":{"type":"object","properties":{"model":{"type":"string"},"thinking":{"type":"string","enum":["off","minimal","low","medium","high","xhigh","max"]},"autonomy":{"type":"string","enum":["low","medium","high"]},"mode":{"type":"string","enum":["none","goal","ralph","swarm","graph"]},"maxParallel":{"type":"integer","minimum":1},"review":{"type":"string","enum":["none","light","full"]},"profiles":{"type":"array","items":{"type":"string"}},"priority":{"type":"boolean"}},"additionalProperties":false}}),
            vec!["projectId", "taskIds"],
        ),
        (
            "superagent_stop",
            "Stop the worker claiming an explicit ticket.",
            json!({"projectId":{"type":"string"},"taskId":{"type":"string"}}),
            vec!["projectId", "taskId"],
        ),
        (
            "superagent_relay",
            "Relay to the project lead. Dependent work: queue; complementary work: steer; urgent work: priority true.",
            json!({"projectId":{"type":"string"},"taskIds":{"type":"array","items":{"type":"string"}},"text":{"type":"string"},"delivery":{"type":"string","enum":["queue","steer"]},"priority":{"type":"boolean"}}),
            vec!["projectId", "text", "delivery"],
        ),
        (
            "superagent_set_mode",
            "Switch the lead mode; ask the user first when practical, but the switch is not technically gated.",
            json!({"projectId":{"type":"string"},"mode":{"type":"string","enum":["none","goal","ralph","swarm","graph"]}}),
            vec!["projectId", "mode"],
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
        #[cfg(test)] dispatcher: None,
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
                m.append_custom_message_entry("oino:superagent-context", oino_ai::UserContentField::Text("Superagent session has no workspace. Manage explicit project boards and dispatch leads; ask the user if project selection is ambiguous. Manual start ignores Auto settings; registering or unarchiving requires user confirmation. Relay dependent work with queue, complementary work with steer, and urgent work with priority. Ask the user before switching a lead's mode when practical; switching is not technically gated.".into()), false, Some(json!({"author":"host"})))?;
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
fn default_ticket_autonomy(argv: &mut Vec<String>) {
    if argv.first().is_some_and(|s| s == "add")
        && !argv.iter().any(|arg| arg == "--autonomy" || arg.starts_with("--autonomy="))
    {
        argv.push("--autonomy".into());
        argv.push("high".into());
    }
}
fn project_command(argv: &[String]) -> Result<Option<&str>, ToolError> {
    if argv.first().is_none_or(|s| s != "project") { return Ok(None); }
    match (argv.get(1).map(String::as_str), argv.len()) {
        (Some("add"), 3) => {
            let path = std::path::Path::new(&argv[2]);
            if !path.is_absolute() || !path.is_dir() { return Err(ToolError::new("path must be an absolute directory")); }
            Ok(Some("add"))
        }
        (Some("list"), 2) => Ok(Some("list")),
        (Some("show"), 2) => Ok(Some("show")),
        _ => Err(ToolError::new("Only project add, list and show are allowed")),
    }
}
fn alias_comment(argv: &mut [String]) {
    if argv.first().is_some_and(|s| s == "comment") {
        argv[0] = "note".into();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ticket_default_autonomy_is_high_but_explicit_values_win() {
        for (mut argv, expected) in [
            (vec!["add", "ticket"], vec!["add", "ticket", "--autonomy", "high"]),
            (vec!["add", "ticket", "--autonomy=low"], vec!["add", "ticket", "--autonomy=low"]),
            (vec!["edit", "T-1"], vec!["edit", "T-1"]),
        ].map(|(a, b)| (a.into_iter().map(str::to_owned).collect::<Vec<_>>(), b)) {
            super::default_ticket_autonomy(&mut argv);
            assert_eq!(argv, expected);
        }
    }

    #[test]
    fn project_commands_are_narrow_and_validate_paths() {
        let args = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(super::project_command(&args(&["project", "list"])).unwrap(), Some("list"));
        assert_eq!(super::project_command(&args(&["project", "show"])).unwrap(), Some("show"));
        assert_eq!(super::project_command(&args(&["project", "add", std::env::temp_dir().to_str().unwrap()])).unwrap(), Some("add"));
        for v in [&["project", "add", "relative"][..], &["project", "archive", "x"], &["project", "list", "--home=x"]] {
            assert!(super::project_command(&args(v)).is_err());
        }
    }

    #[tokio::test]
    async fn onboarding_confirmed_then_ticket_started_and_denials_fail_closed() {
        use super::*;
        use futures_util::future::BoxFuture;
        use oino_core::ui::{UiAnswer, UiBridge, UiOutput, UiResponse, UiSink};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use unipi_kanboard::{Layout, Project, model::Task};

        #[derive(Default)]
        struct Fake(AtomicUsize);
        impl crate::superagent::Workers for Fake {
            fn live<'a>(&'a self, _: &'a str) -> BoxFuture<'a, bool> { Box::pin(async { true }) }
            fn launch<'a>(&'a self, _: &'a Project, _: &'a Task, _: &'a str) -> BoxFuture<'a, Result<(), String>> {
                Box::pin(async { self.0.fetch_add(1, Ordering::SeqCst); Ok(()) })
            }
            fn stop<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<(), String>> { Box::pin(async { Ok(()) }) }
            fn relay<'a>(&'a self, _: &'a str, _: &'a str) -> BoxFuture<'a, Result<(), String>> { Box::pin(async { Ok(()) }) }
            fn ended<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Option<String>> { Box::pin(async { None }) }
        }
        struct Sink;
        impl UiSink for Sink { fn publish(&self, _: UiOutput) {} }
        async fn call(tool: &ChatTool, value: serde_json::Value) -> Result<serde_json::Value, ToolError> {
            let result = tool.execute(ToolInvocation { tool_call_id: "test".into(), args: value.as_object().unwrap().clone(), parent_tool_call_id: None }, UpdateSink::discard(), CancellationToken::new()).await?;
            let text = match &result.content[0] { oino_ai::UserContent::Text(t) => &t.text, _ => panic!("expected text") };
            Ok(serde_json::from_str(text).unwrap())
        }
        if std::env::var_os("OIN35_ONBOARDING_CHILD").is_none() {
            let dir = tempfile::tempdir().unwrap();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "superagent_session::tests::onboarding_confirmed_then_ticket_started_and_denials_fail_closed", "--nocapture"])
                .env("OIN35_ONBOARDING_CHILD", "1")
                .env("UNIPI_KANBOARD_HOME", dir.path().join("board"))
                .status().unwrap();
            assert!(status.success());
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::with_home(std::env::var_os("UNIPI_KANBOARD_HOME").unwrap());
        let fake = Arc::new(Fake::default());
        let dispatcher = crate::superagent::Dispatcher::acquire(layout.clone(), dir.path().join("config.json"), fake.clone()).await.unwrap();
        let hub = crate::sessions::SessionHub::create(crate::sessions::HostHubOptions {
            cwd: dir.path().into(), agent_dir: dir.path().join("agent"), docs_root: None,
            session_dir: Some(dir.path().join("sessions")), mode: oino_core::extensions::api::ExtensionMode::Desktop,
            has_ui: true, approve: Some(false), flags: ResourceFlags { no_context_files: true, ..Default::default() },
            loader: Arc::new(oino_core::services::StaticLoader(Vec::new())),
            models: Some(oino_models::ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()))),
            tools: ToolSelection::Default, exclude_tools: Vec::new(), services_cap: None, host_data_dir: None,
        }).unwrap();
        let service = crate::sessions::SessionService::install(hub.clone());
        let ui = UiBridge::new(true, "oin35-onboarding", Arc::new(Sink));
        let feed = service.feed(ui.session_id());
        let subscription = feed.subscribe();
        let session = Arc::new(OnceLock::new());
        session.set(ui.clone()).unwrap();
        let tool = |name: &str, session: Arc<OnceLock<UiBridge>>| ChatTool {
            meta: ToolDefinitionMeta::new(name, name, name, json!({"type":"object"})),
            hub: hub.clone(), session, dispatcher: Some(dispatcher.clone()),
        };
        let board = tool("kanboard", session.clone());
        let start = tool("superagent_start", session.clone());
        let accepted = dir.path().join("accepted");
        let refused = dir.path().join("refused");
        let absent = dir.path().join("absent");
        for p in [&accepted, &refused, &absent] { std::fs::create_dir_all(p).unwrap(); }
        for (path, approve) in [(&refused, false), (&accepted, true)] {
            let answer_ui = ui.clone();
            let answer = async move {
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    loop {
                        if let Some(request) = answer_ui.snapshot().into_iter().find(|r| r.method == "confirm") {
                            answer_ui.respond(UiResponse { id: request.id, answer: UiAnswer::Confirmed(approve) });
                            break;
                        }
                        tokio::task::yield_now().await;
                    }
                }).await.unwrap();
            };
            let (result, ()) = tokio::join!(call(&board, json!({"argv":["project","add",path]})), answer);
            if approve { assert!(result.unwrap()["projectId"].is_string()); } else { assert!(result.unwrap_err().to_string().contains("declined")); }
        }
        let no_ui = Arc::new(OnceLock::new());
        no_ui.set(UiBridge::new(false, "no-ui", Arc::new(Sink))).unwrap();
        assert!(call(&tool("kanboard", no_ui), json!({"argv":["project","add",absent]})).await.unwrap_err().to_string().contains("confirmation"));
        let projects = layout.list_projects().unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].root, std::fs::canonicalize(&accepted).unwrap());
        let project = &projects[0].slug;
        let ticket = call(&board, json!({"projectId":project,"argv":["add","Onboarded task"]})).await.unwrap();
        let ticket = ticket["id"].as_str().expect("ticket id");
        let result = call(&start, json!({"projectId":project,"taskIds":[ticket]})).await.unwrap();
        assert!(result["sessionId"].is_string(), "{result}");
        assert_eq!(fake.0.load(Ordering::SeqCst), 1);
        drop(subscription);
    }

    #[test]
    fn comment_is_an_alias_for_note() {
        let mut argv = vec!["comment", "T-1", "looks good"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        super::alias_comment(&mut argv);
        assert_eq!(argv, ["note", "T-1", "looks good"]);
    }
}

struct ChatTool {
    meta: ToolDefinitionMeta,
    hub: Arc<crate::sessions::SessionHub>,
    session: Arc<OnceLock<oino_core::ui::UiBridge>>,
    #[cfg(test)]
    dispatcher: Option<Arc<crate::superagent::Dispatcher>>,
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
        #[cfg(test)]
        let test_dispatcher = self.dispatcher.clone();
        Box::pin(async move {
            let dispatcher = || -> Result<Arc<crate::superagent::Dispatcher>, ToolError> {
                #[cfg(test)]
                if let Some(dispatcher) = &test_dispatcher { return Ok(Arc::clone(dispatcher)); }
                crate::superagent::current().cloned().map_err(|e| ToolError::new(e.to_string()))
            };
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
                        .map_err(|e| ToolError::new(e.to_string()))?
                        .into_iter().filter(|project| !project.archived).collect::<Vec<_>>();
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
                    let mut argv: Vec<String> = serde_json::from_value(args["argv"].clone())
                        .map_err(|e| ToolError::new(e.to_string()))?;
                    if let Some(command) = project_command(&argv)? {
                        match command {
                            "add" => {
                                let path = std::path::Path::new(&argv[2]);
                                let Some(ui) = session.get().filter(|ui| ui.has_ui()) else { return Err(ToolError::new("Project registration requires user confirmation")); };
                                if !crate::sessions::SessionService::shared().is_some_and(|service| service.feed(ui.session_id()).subscriber_count() > 0) {
                                    return Err(ToolError::new("Project registration requires user confirmation"));
                                }
                                let params = serde_json::from_value(json!({"title":"Set up Kanboard project?","message":format!("Register workspace {}?", path.display())})).map_err(|e| ToolError::new(e.to_string()))?;
                                let answer = tokio::time::timeout(std::time::Duration::from_secs(120), ui.dialog(Some("superagent"), "confirm", params, Some(cancel.clone()))).await.map_err(|_| ToolError::new("Project registration confirmation timed out"))?;
                                if answer != oino_core::ui::UiAnswer::Confirmed(true) { return Err(ToolError::new("Project registration declined")); }
                                let dispatcher = dispatcher()?;
                                let result = dispatcher.enable("register", None, Some(&argv[2])).await.map_err(|e| ToolError::new(e.to_string()))?;
                                return Ok(AgentToolResult::text(result.to_string()));
                            }
                            "list" => {
                                let layout = unipi_kanboard::Layout::from_env().map_err(|e| ToolError::new(e.to_string()))?;
                                let result = unipi_kanboard::commands::project_list(&layout).map_err(|e| ToolError::new(e.to_string()))?;
                                return Ok(AgentToolResult::text(result.to_string()));
                            }
                            "show" => {
                                let project_id = field("projectId")?;
                                let layout = unipi_kanboard::Layout::from_env().map_err(|e| ToolError::new(e.to_string()))?;
                                let project = unipi_kanboard::Project::load(&layout, &project_id).map_err(|e| ToolError::new(e.to_string()))?;
                                let result = unipi_kanboard::commands::project_show(&layout, &project).map_err(|e| ToolError::new(e.to_string()))?;
                                return Ok(AgentToolResult::text(result.to_string()));
                            }
                            _ => return Err(ToolError::new("Only project add, list and show are allowed")),
                        }
                    }
                    let project_id = field("projectId")?;
                    let layout = unipi_kanboard::Layout::from_env()
                        .map_err(|e| ToolError::new(e.to_string()))?;
                    let project = match unipi_kanboard::Project::load(&layout, &project_id) {
                        Ok(project) => project,
                        Err(_) => {
                            let dispatcher = dispatcher()?;
                            let blockers = dispatcher.blockers(&project_id).await.map_err(|e| ToolError::new(e.to_string()))?;
                            return Err(ToolError::new(blockers.first().map_or_else(|| "Project is unavailable".into(), |b| b.message.clone())));
                        }
                    };
                    if project.archived {
                        return Err(ToolError::new("Project is archived"));
                    }
                    alias_comment(&mut argv);
                    let command = argv.first().map(String::as_str).unwrap_or("");
                    let help = (argv.len() == 1
                        && matches!(command, "help" | "--help" | "-h"))
                        || (argv.len() >= 2
                            && matches!(argv[1].as_str(), "help" | "--help" | "-h"));
                    if !help
                        && !matches!(
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
                        )
                    {
                        return Err(ToolError::new(
                            "Superagent manages boards; use superagent_start or superagent_stop for workers",
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
                    if argv.iter().any(|s| s.starts_with("--project")) {
                        return Err(ToolError::new(
                            "Don't pass --project; set the projectId argument instead.",
                        ));
                    }
                    if argv.iter().any(|s| {
                        s.starts_with("--actor")
                            || s.starts_with("--home")
                            || s.starts_with("--session")
                            || s.starts_with("--gate")
                    }) {
                        return Err(ToolError::new("Global board overrides are not allowed"));
                    }
                    let schedules_todo = (command == "move" && argv.get(2).is_some_and(|s| s == "todo"))
                        || (command == "add" && argv.iter().enumerate().any(|(i, arg)| arg == "--status=todo" || (arg == "--status" && argv.get(i + 1).is_some_and(|s| s == "todo"))));
                    default_ticket_autonomy(&mut argv);
                    let mut result = unipi_kanboard::service::exec(&unipi_kanboard::service::ExecRequest {
                        argv: [
                            vec![
                                "--project".into(),
                                project_id.clone(),
                                "--actor".into(),
                                "user".into(),
                            ],
                            argv,
                        ]
                        .concat(),
                        cwd: Some(project.root),
                        ..Default::default()
                    });
                    if schedules_todo && result["ok"] != false {
                        let dispatcher = dispatcher()?;
                        let blockers = dispatcher.blockers(&project_id).await.map_err(|e| ToolError::new(e.to_string()))?;
                        if !blockers.is_empty() {
                            let reasons = blockers.iter().map(|b| b.message.as_str()).collect::<Vec<_>>().join(" ");
                            let notice = format!("Note: {project_id} will not auto-start these tickets ({reasons}). Call superagent_start to dispatch them now, or ask the user to enable Auto / Auto-start.");
                            if let Some(object) = result.as_object_mut() { object.insert("notice".into(), json!(notice)); }
                        }
                    }
                    result
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
                    let dispatcher =
                        dispatcher()?;
                    match name.as_str() {
                        "superagent_start" => {
                            let overrides: crate::superagent_presets::Overrides = args
                                .get("overrides")
                                .cloned()
                                .map(serde_json::from_value)
                                .transpose()
                                .map_err(|e| ToolError::new(format!("Invalid overrides: {e}")))?
                                .unwrap_or_default();
                            let tasks: Vec<String> = serde_json::from_value(args["taskIds"].clone()).map_err(|e| ToolError::new(e.to_string()))?;
                            let first = dispatcher.start_tasks(&project, &tasks, args["presetId"].as_str(), overrides.clone()).await;
                            let id = match first {
                                Ok(id) => id,
                                Err(error) if matches!(error.code, "E_PROJECT_NOT_REGISTERED" | "E_PROJECT_ARCHIVED") => {
                                    let path = if error.code == "E_PROJECT_NOT_REGISTERED" {
                                        let recent = hub.projects().await.unwrap_or_default();
                                        recent.into_iter().find(|workspace| {
                                            let path = std::path::Path::new(&workspace.cwd);
                                            let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
                                            unipi_kanboard::store::slug_for(&canonical) == project
                                        }).map(|workspace| workspace.cwd)
                                    } else { None };
                                    if error.code == "E_PROJECT_NOT_REGISTERED" && path.is_none() { return Err(ToolError::new(error.to_string())); }
                                    let Some(ui) = session.get().filter(|ui| ui.has_ui()) else { return Err(ToolError::new(error.to_string())); };
                                    if !crate::sessions::SessionService::shared().is_some_and(|service| service.feed(ui.session_id()).subscriber_count() > 0) { return Err(ToolError::new(error.to_string())); }
                                    let title = if path.is_some() { format!("Set up Kanboard for {project}?") } else { format!("Unarchive {project}?") };
                                    let params = serde_json::from_value(json!({"title":title,"message":format!("{} Superagent will then start {}.", error.message, tasks.join(", "))})).map_err(|e| ToolError::new(e.to_string()))?;
                                    let answer = tokio::time::timeout(std::time::Duration::from_secs(120), ui.dialog(Some("superagent"), "confirm", params, Some(cancel.clone()))).await.map_err(|_| ToolError::new(error.to_string()))?;
                                    if answer != oino_core::ui::UiAnswer::Confirmed(true) { return Err(ToolError::new(format!("User declined: {}", error.message))); }
                                    let action = if path.is_some() { "register" } else { "unarchive" };
                                    let enabled = dispatcher.enable(action, Some(&project), path.as_deref()).await.map_err(|e| ToolError::new(e.to_string()))?;
                                    let id = enabled["projectId"].as_str().unwrap_or(&project);
                                    dispatcher.start_tasks(id, &tasks, args["presetId"].as_str(), overrides).await.map_err(|e| ToolError::new(e.to_string()))?
                                }
                                Err(error) => return Err(ToolError::new(error.to_string())),
                            };
                            json!({"sessionId":id})
                        }
                        "superagent_set_mode" => dispatcher.set_mode(&project, &field("mode")?).await.map_err(|e| ToolError::new(e.to_string()))?,
                        "superagent_stop" => {
                            json!({"stopped":dispatcher.stop(&project, &field("taskId")?).await.map_err(|e| ToolError::new(e.to_string()))?})
                        }
                        "superagent_relay" => {
                            let tasks: Vec<String> = args.get("taskIds").cloned().map(serde_json::from_value).transpose().map_err(|e| ToolError::new(format!("Invalid taskIds: {e}")))?.unwrap_or_default();
                            let delivery = crate::superagent::Delivery::parse(&field("delivery")?).map_err(|e| ToolError::new(e.to_string()))?;
                            dispatcher.relay_to_lead(&project, &field("text")?, &tasks, delivery, args["priority"].as_bool().unwrap_or(false)).await.map_err(|e| ToolError::new(e.to_string()))?
                        },
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

/// History title for a Superagent chat (OIN-26): an explicit user rename wins, else the
/// first user message on one line, else `Superagent · <local date time>` from the creation
/// time. The old fixed name `Superagent` counts as unnamed, so old chats get real titles at
/// list time without rewriting their files.
#[must_use]
pub fn superagent_title(name: Option<&str>, first_message: &str, created_ms: i64) -> String {
    superagent_title_in_project(name, first_message, created_ms, None)
}

/// Include the known project for skill-paste titles, without changing explicit names.
#[must_use]
pub fn superagent_title_in_project(name: Option<&str>, first_message: &str, created_ms: i64, project: Option<&str>) -> String {
    if let Some(name) = name.map(str::trim).filter(|name| !name.is_empty() && *name != "Superagent") {
        return name.to_owned();
    }
    let mut seed = first_message.trim_start();
    let mut skill = None;
    if let Some(frontmatter) = seed.strip_prefix("---\n").or_else(|| seed.strip_prefix("---\r\n")) {
        if let Some((header, rest)) = frontmatter.split_once("\n---") {
            skill = header.lines().find_map(|line| line.trim().strip_prefix("name:").map(str::trim)).filter(|name| !name.is_empty());
            seed = rest.trim_start_matches('\r').trim_start_matches('\n').trim_start();
        }
    }
    if let Some(skill) = skill {
        let skill = skill.trim_matches(['\'', '"']);
        return match project.filter(|project| !project.trim().is_empty()) {
            Some(project) => format!("{skill} · {project}"),
            None => skill.to_owned(),
        };
    }
    while seed.starts_with("```") || seed.starts_with("~~~") {
        let fence = &seed[..3];
        let Some((_, body)) = seed.split_once('\n') else { break };
        let Some(end) = body.lines().position(|line| line.trim_start().starts_with(fence)) else { break };
        let consumed: usize = body.split_inclusive('\n').take(end + 1).map(str::len).sum();
        seed = body[consumed..].trim_start();
    }
    let message = seed.split_whitespace().collect::<Vec<_>>().join(" ");
    if !message.is_empty() && message != "(no messages)" {
        let mut chars = message.chars();
        let head: String = chars.by_ref().take(60).collect();
        return if chars.next().is_some() { format!("{head}…") } else { head };
    }
    let at = chrono::DateTime::from_timestamp_millis(created_ms)
        .map(|at| at.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "unknown".into());
    format!("Superagent · {at}")
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

#[cfg(test)]
mod title_tests {
    use super::{superagent_title, superagent_title_in_project};

    #[test]
    fn explicit_name_wins() {
        assert_eq!(superagent_title(Some("Launch checklist"), "hello", 0), "Launch checklist");
    }

    #[test]
    fn fixed_superagent_name_falls_back_to_the_first_message() {
        assert_eq!(superagent_title(Some("Superagent"), "What is running?", 0), "What is running?");
        assert_eq!(superagent_title(None, "What is running?", 0), "What is running?");
    }

    #[test]
    fn frontmatter_and_leading_fences_do_not_become_titles() {
        assert_eq!(superagent_title(None, "---\nname: untangle-codebase\ndescription: test\n---\nInstructions", 0), "untangle-codebase");
        assert_eq!(superagent_title(None, "```text\nignore this\n```\nFix the project", 0), "Fix the project");
        assert_eq!(superagent_title_in_project(None, "---\nname: untangle-codebase\n---\nInstructions", 0, Some("zmstudio-migration")), "untangle-codebase · zmstudio-migration");
        assert_eq!(superagent_title_in_project(Some("My title"), "---\nname: skill\n---\ntext", 0, Some("project")), "My title");
    }

    #[test]
    fn first_message_is_collapsed_trimmed_and_truncated() {
        assert_eq!(superagent_title(None, "  What\n\tis\t running?  ", 0), "What is running?");
        let long = "word ".repeat(40);
        let title = superagent_title(None, &long, 0);
        assert!(title.ends_with('…'), "{title}");
        assert_eq!(title.chars().count(), 61);
        // Multi-byte characters truncate on a character boundary.
        let wide = "é".repeat(100);
        let title = superagent_title(None, &wide, 0);
        assert!(title.ends_with('…'));
        assert_eq!(title.chars().count(), 61);
    }

    #[test]
    fn empty_or_no_messages_falls_back_to_the_creation_date() {
        for message in ["", "   ", "(no messages)"] {
            let title = superagent_title(None, message, 1_791_000_580_000);
            let date = &title["Superagent · ".len()..];
            assert_eq!(date.len(), "YYYY-MM-DD HH:MM".len(), "{title}");
            assert_eq!(&date[4..5], "-");
            assert_eq!(&date[7..8], "-");
            assert_eq!(&date[10..11], " ");
            assert_eq!(&date[13..14], ":");
        }
    }
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
