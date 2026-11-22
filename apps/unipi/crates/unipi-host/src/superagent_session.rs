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
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{Arc, OnceLock},
};
use tokio_util::sync::CancellationToken;

/// Create a workspace-free session over a fresh isolated services set. The normal
/// v3 prompt freezing/prefix machinery is retained; the marker suppresses cwd probes.
/// # Errors
/// Returns an error if the project session or its configured services cannot be created.
pub async fn create(
    hub: Arc<crate::sessions::SessionHub>,
) -> Result<Arc<AgentSessionRuntime>, String> {
    create_from(hub, None).await
}

/// Resume only through the restricted tool/session factory. Never wake through generic services.
/// # Errors
/// Returns an error if the project session or its configured services cannot be created.
pub async fn create_from(
    hub: Arc<crate::sessions::SessionHub>,
    file: Option<PathBuf>,
) -> Result<Arc<AgentSessionRuntime>, String> {
    let loader = Arc::new(crate::sessions::services::HostLoader::memory_only(
        hub.agent_dir(),
    ));
    create_with_loader(hub, file, loader).await
}

async fn create_with_loader(
    hub: Arc<crate::sessions::SessionHub>,
    file: Option<PathBuf>,
    loader: Arc<dyn oino_core::services::ExtensionLoader>,
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
            // The Superagent is a delegator, not a coding assistant: its own prompt replaces
            // the default/user SYSTEM.md, and APPEND_SYSTEM.md is not discovered either.
            system_prompt: Some(SUPERAGENT_PROMPT.into()),
            append_system_prompt: Some(Vec::new()),
            ..ResourceFlags::default()
        },
        extension_flag_values: std::iter::empty().collect(),
        loader,
        approve: Some(false),
        models: Some(base.models.clone()),
        settings: None,
    })
    .await
    .map_err(|e| e.to_string())?;
    let session = Arc::new(OnceLock::new());
    let specs = tool_specs();

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
                m.append_custom_message_entry("oino:superagent-context", oino_ai::UserContentField::Text("Superagent session: no workspace. Act on clear instructions without re-confirming; board edits, starts and relays are reversible. Registering or unarchiving a project asks the user itself.".into()), false, Some(json!({"author":"host"})))?;
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
/// The Superagent's own system prompt (replaces the coding prompt and the user's SYSTEM.md).
const SUPERAGENT_PROMPT: &str = include_str!("superagent_prompt.md");

/// Board commands the Superagent may run (besides `project …` and help).
const BOARD_COMMANDS: &[&str] = &[
    "add", "edit", "link", "unlink", "order", "move", "note", "list", "show", "search", "next",
    "chain", "status",
];

/// Models often prefix board commands with `task`/`tasks`/`ticket` (`task list`); drop it.
fn strip_task_prefix(argv: &mut Vec<String>) {
    if argv.len() > 1
        && matches!(argv[0].as_str(), "task" | "tasks" | "ticket" | "tickets")
        && BOARD_COMMANDS.contains(&argv[1].as_str())
    {
        argv.remove(0);
    }
}

fn default_ticket_autonomy(argv: &mut Vec<String>) {
    if argv.first().is_some_and(|s| s == "add")
        && !argv
            .iter()
            .any(|arg| arg == "--autonomy" || arg.starts_with("--autonomy="))
    {
        argv.push("--autonomy".into());
        argv.push("high".into());
    }
}
fn project_command(argv: &[String]) -> Result<Option<&str>, ToolError> {
    if argv.first().is_none_or(|s| s != "project") {
        return Ok(None);
    }
    match (argv.get(1).map(String::as_str), argv.len()) {
        (Some("add"), 3) => {
            let path = std::path::Path::new(&argv[2]);
            if !path.is_absolute() || !path.is_dir() {
                return Err(ToolError::new("path must be an absolute directory"));
            }
            Ok(Some("add"))
        }
        (Some("unarchive"), 2) => Ok(Some("unarchive")),
        (Some("list"), 2) => Ok(Some("list")),
        (Some("show"), 2) => Ok(Some("show")),
        _ => Err(ToolError::new(
            "project accepts only: [\"project\",\"list\"], [\"project\",\"show\"] (lane counts), [\"project\",\"unarchive\"] with projectId, or [\"project\",\"add\",\"/abs/path\"]. For tickets use [\"list\",\"--all\"] or [\"list\",\"--status\",\"backlog\"] with projectId.",
        )),
    }
}
fn alias_comment(argv: &mut [String]) {
    if argv.first().is_some_and(|s| s == "comment") {
        argv[0] = "note".into();
    }
}

/// The board moves the Superagent may run (OIN-89): backlog ↔ todo, and
/// `cancelled` only for an unclaimed backlog/todo ticket, with a reason.
fn check_move(
    layout: &unipi_kanboard::Layout,
    project: &unipi_kanboard::Project,
    argv: &[String],
) -> Result<(), ToolError> {
    if argv.first().is_none_or(|s| s != "move") {
        return Ok(());
    }
    let allowed_forms = "Superagent moves tickets only to backlog, todo or cancelled: \
                          [\"move\",\"ID\",\"todo\"], \
                          [\"move\",\"ID\",\"cancelled\",\"--comment\",\"why\"]. \
                          Later lanes belong to the lead: call superagent_start to get tickets worked.";
    let target = argv.get(2).map(String::as_str);
    match target {
        Some("backlog" | "todo") => Ok(()),
        Some("cancelled") => {
            let id = argv
                .get(1)
                .ok_or_else(|| ToolError::new("move needs a ticket id"))?;
            let why = argv
                .iter()
                .position(|arg| arg == "--comment")
                .and_then(|index| argv.get(index + 1))
                .map(String::as_str)
                .or_else(|| argv.iter().find_map(|arg| arg.strip_prefix("--comment=")));
            if why.is_none_or(|why| why.trim().is_empty()) {
                return Err(ToolError::new(
                    "Cancelling a ticket needs a reason: [\"move\",\"ID\",\"cancelled\",\"--comment\",\"why\"].",
                ));
            }
            let board = unipi_kanboard::board::Board::open(layout, project.clone())
                .map_err(|e| ToolError::new(e.to_string()))?;
            let task = board.get(id).map_err(|e| ToolError::new(e.to_string()))?;
            if task.run.is_some() {
                return Err(ToolError::new(format!(
                    "{id} is held by the project's lead; ask the lead to cancel it (superagent_relay) or stop it (superagent_stop)."
                )));
            }
            if !matches!(
                task.status,
                unipi_kanboard::model::Status::Backlog | unipi_kanboard::model::Status::Todo
            ) {
                return Err(ToolError::new(format!(
                    "{} is in {}; only backlog or todo tickets can be cancelled from here. {allowed_forms}",
                    task.id, task.status
                )));
            }
            Ok(())
        }
        _ => Err(ToolError::new(allowed_forms)),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ticket_default_autonomy_is_high_but_explicit_values_win() {
        for (mut argv, expected) in [
            (
                vec!["add", "ticket"],
                vec!["add", "ticket", "--autonomy", "high"],
            ),
            (
                vec!["add", "ticket", "--autonomy=low"],
                vec!["add", "ticket", "--autonomy=low"],
            ),
            (vec!["edit", "T-1"], vec!["edit", "T-1"]),
        ]
        .map(|(a, b)| (a.into_iter().map(str::to_owned).collect::<Vec<_>>(), b))
        {
            super::default_ticket_autonomy(&mut argv);
            assert_eq!(argv, expected);
        }
    }

    #[test]
    fn project_commands_are_narrow_and_validate_paths() {
        let strings = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            super::project_command(&strings(&["project", "list"])).unwrap(),
            Some("list")
        );
        assert_eq!(
            super::project_command(&strings(&["project", "show"])).unwrap(),
            Some("show")
        );
        assert_eq!(
            super::project_command(&strings(&["project", "unarchive"])).unwrap(),
            Some("unarchive")
        );
        assert_eq!(
            super::project_command(&strings(&[
                "project",
                "add",
                std::env::temp_dir().to_str().unwrap()
            ]))
            .unwrap(),
            Some("add")
        );
        for v in [
            &["project", "add", "relative"][..],
            &["project", "archive", "x"],
            &["project", "list", "--home=x"],
        ] {
            assert!(super::project_command(&strings(v)).is_err());
        }
    }

    #[test]
    fn superagent_cancel_guard_allows_only_unclaimed_backlog_or_todo() {
        use unipi_kanboard::commands::{self, Common, StartArgs};
        use unipi_kanboard::model::{Actor, ChainGate, Priority, Status};

        let dir = tempfile::tempdir().unwrap();
        let layout = unipi_kanboard::Layout::with_home(dir.path());
        let root = dir.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        let project =
            unipi_kanboard::Project::create(&layout, &root, Some("Guard"), Some("GRD")).unwrap();
        let common = Common::new(Actor::User, ChainGate::InReview);
        let add = |title: &str| {
            let value = commands::add(
                &layout,
                project.clone(),
                &common,
                title,
                Some("body"),
                Some(Status::Backlog),
                Priority::None,
                &[],
                &[],
                &[],
                true,
            )
            .unwrap();
            value["id"].as_str().unwrap().to_owned()
        };
        let strings = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();

        // An unclaimed backlog/todo ticket can be cancelled, with a reason.
        let backlog = add("wrong board");
        super::check_move(
            &layout,
            &project,
            &strings(&[
                "move",
                &backlog,
                "cancelled",
                "--comment",
                "filed on the wrong board",
            ]),
        )
        .expect("unclaimed backlog cancel");
        let err = super::check_move(
            &layout,
            &project,
            &strings(&["move", &backlog, "cancelled"]),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("--comment"), "{err}");

        let todo = add("obsolete");
        commands::move_task(&layout, project.clone(), &common, &todo, Status::Todo, None).unwrap();
        super::check_move(
            &layout,
            &project,
            &strings(&["move", &todo, "cancelled", "--comment=obsolete"]),
        )
        .expect("unclaimed todo cancel");

        // A lead-claimed ticket stays refused with both escape hatches named.
        commands::start(
            &layout,
            project.clone(),
            ChainGate::InReview,
            &todo,
            &StartArgs {
                session: "lead-1",
                pid: std::process::id(),
                host: &commands::hostname(),
            },
            common.now,
        )
        .unwrap();
        let err = super::check_move(
            &layout,
            &project,
            &strings(&["move", &todo, "cancelled", "--comment", "obsolete"]),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("held by the project's lead"), "{err}");
        assert!(
            err.contains("superagent_relay") && err.contains("superagent_stop"),
            "{err}"
        );

        // The refusal text lists the allowed forms exactly.
        let err = super::check_move(
            &layout,
            &project,
            &strings(&["move", &backlog, "in_progress"]),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("[\"move\",\"ID\",\"todo\"]"), "{err}");
        assert!(
            err.contains("[\"move\",\"ID\",\"cancelled\",\"--comment\",\"why\"]"),
            "{err}"
        );

        super::check_move(&layout, &project, &strings(&["move", &backlog, "todo"]))
            .expect("backlog → todo stays allowed without a comment");
    }

    #[tokio::test]
    #[allow(
        clippy::too_many_lines,
        reason = "Keep this linear integration scenario and its assertions together."
    )]
    async fn onboarding_confirmed_then_ticket_started_and_denials_fail_closed() {
        use super::*;
        use futures_util::future::BoxFuture;
        use oino_core::ui::{UiAnswer, UiBridge, UiOutput, UiResponse, UiSink};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use unipi_kanboard::{model::Task, Layout, Project};

        #[derive(Default)]
        struct Fake(AtomicUsize);
        impl crate::superagent::Workers for Fake {
            fn live<'a>(&'a self, _: &'a str) -> BoxFuture<'a, bool> {
                Box::pin(async { true })
            }
            fn launch<'a>(
                &'a self,
                _: &'a Project,
                _: &'a Task,
                _: &'a str,
            ) -> BoxFuture<'a, Result<(), String>> {
                Box::pin(async {
                    self.0.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
            }
            fn stop<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<(), String>> {
                Box::pin(async { Ok(()) })
            }
            fn relay<'a>(&'a self, _: &'a str, _: &'a str) -> BoxFuture<'a, Result<(), String>> {
                Box::pin(async { Ok(()) })
            }
            fn ended<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Option<String>> {
                Box::pin(async { None })
            }
        }
        struct Sink;
        impl UiSink for Sink {
            fn publish(&self, _: UiOutput) {}
        }
        async fn call(
            tool: &ChatTool,
            value: serde_json::Value,
        ) -> Result<serde_json::Value, ToolError> {
            let result = tool
                .execute(
                    ToolInvocation {
                        tool_call_id: "test".into(),
                        args: value.as_object().unwrap().clone(),
                        parent_tool_call_id: None,
                    },
                    UpdateSink::discard(),
                    CancellationToken::new(),
                )
                .await?;
            let text = match &result.content[0] {
                oino_ai::UserContent::Text(t) => &t.text,
                oino_ai::UserContent::Image(_) => panic!("expected text"),
            };
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
        let dispatcher = crate::superagent::Dispatcher::acquire(
            layout.clone(),
            dir.path().join("config.json"),
            fake.clone(),
        )
        .await
        .unwrap();
        let hub = crate::sessions::SessionHub::create(crate::sessions::HostHubOptions {
            cwd: dir.path().into(),
            agent_dir: dir.path().join("agent"),
            docs_root: None,
            session_dir: Some(dir.path().join("sessions")),
            mode: oino_core::extensions::api::ExtensionMode::Desktop,
            has_ui: true,
            approve: Some(false),
            flags: ResourceFlags {
                no_context_files: true,
                ..Default::default()
            },
            loader: Arc::new(oino_core::services::StaticLoader(Vec::new())),
            models: Some(oino_models::ModelRegistry::in_memory(Arc::new(
                oino_models::MapEnv::default(),
            ))),
            tools: ToolSelection::Default,
            exclude_tools: Vec::new(),
            services_cap: None,
            host_data_dir: None,
        })
        .unwrap();
        let service = crate::sessions::SessionService::install(hub.clone());
        let ui = UiBridge::new(true, "oin35-onboarding", Arc::new(Sink));
        let feed = service.feed(ui.session_id());
        let subscription = feed.subscribe();
        let session = Arc::new(OnceLock::new());
        session.set(ui.clone()).unwrap();
        let tool = |name: &str, session: Arc<OnceLock<UiBridge>>| ChatTool {
            meta: ToolDefinitionMeta::new(name, name, name, json!({"type":"object"})),
            hub: hub.clone(),
            session,
            dispatcher: Some(dispatcher.clone()),
        };
        let board = tool("kanboard", session.clone());
        let start = tool("superagent_start", session.clone());
        let accepted = dir.path().join("accepted");
        let refused = dir.path().join("refused");
        let absent = dir.path().join("absent");
        for p in [&accepted, &refused, &absent] {
            std::fs::create_dir_all(p).unwrap();
        }
        for (path, approve) in [(&refused, false), (&accepted, true)] {
            let answer_ui = ui.clone();
            let answer = async move {
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    loop {
                        if let Some(request) = answer_ui
                            .snapshot()
                            .into_iter()
                            .find(|r| r.method == "confirm")
                        {
                            answer_ui.respond(UiResponse {
                                id: request.id,
                                answer: UiAnswer::Confirmed(approve),
                            });
                            break;
                        }
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
            };
            let (result, ()) =
                tokio::join!(call(&board, json!({"argv":["project","add",path]})), answer);
            if approve {
                assert!(result.unwrap()["projectId"].is_string());
            } else {
                assert!(result.unwrap_err().to_string().contains("declined"));
            }
        }
        let no_ui = Arc::new(OnceLock::new());
        no_ui
            .set(UiBridge::new(false, "no-ui", Arc::new(Sink)))
            .unwrap();
        assert!(call(
            &tool("kanboard", no_ui),
            json!({"argv":["project","add",absent]})
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("confirmation"));
        let projects = layout.list_projects().unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].root, std::fs::canonicalize(&accepted).unwrap());
        let project = &projects[0].slug;
        unipi_kanboard::commands::project_set_archived(&layout, project, true).unwrap();
        let answer_ui = ui.clone();
        let answer = async move {
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    if let Some(request) = answer_ui
                        .snapshot()
                        .into_iter()
                        .find(|r| r.method == "confirm")
                    {
                        answer_ui.respond(UiResponse {
                            id: request.id,
                            answer: UiAnswer::Confirmed(true),
                        });
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        };
        let (result, ()) = tokio::join!(
            call(
                &board,
                json!({"projectId":project,"argv":["project","unarchive"]})
            ),
            answer
        );
        assert!(result.unwrap()["projectId"].is_string());
        assert!(!Project::load(&layout, project).unwrap().archived);
        let ticket = call(
            &board,
            json!({"projectId":project,"argv":["add","Onboarded task"]}),
        )
        .await
        .unwrap();
        let ticket = ticket["id"].as_str().expect("ticket id");
        let result = call(&start, json!({"projectId":project,"taskIds":[ticket]}))
            .await
            .unwrap();
        assert!(result["sessionId"].is_string(), "{result}");
        assert_eq!(fake.0.load(Ordering::SeqCst), 1);
        drop(subscription);
    }

    #[test]
    fn task_prefix_is_stripped_only_before_a_board_command() {
        let strings = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        for (input, expected) in [
            (
                &["task", "list", "--status", "backlog"][..],
                &["list", "--status", "backlog"][..],
            ),
            (
                &["tickets", "move", "OIN-1", "todo"],
                &["move", "OIN-1", "todo"],
            ),
            (&["task", "start", "OIN-1"], &["task", "start", "OIN-1"]),
            (&["list"], &["list"]),
            (&["task"], &["task"]),
        ] {
            let mut argv = strings(input);
            super::strip_task_prefix(&mut argv);
            assert_eq!(argv, strings(expected));
        }
    }

    #[test]
    fn superagent_prompt_is_a_delegator_prompt() {
        let prompt = super::SUPERAGENT_PROMPT;
        assert!(prompt.starts_with("# Superagent"));
        assert!(prompt.contains("Act, don't ask"));
        assert!(!prompt.contains("Karpathy"));
    }

    /// OIN-81: `superagent_status` is on the Superagent's allow list (dotted and
    /// underscored names) and its schema is read-only (no required args).
    #[test]
    fn superagent_status_is_allowed_and_read_only() {
        let specs = super::tool_specs();
        let (_, description, properties, required) = specs
            .iter()
            .find(|(name, _, _, _)| *name == "superagent_status")
            .expect("superagent_status spec");
        assert!(required.is_empty(), "no required args: {required:?}");
        assert!(description.contains("Read-only"));
        assert_eq!(properties["projectId"]["type"], "string");
        for name in ["superagent_status", "superagent.status"] {
            assert!(
                super::permission_allow_list().contains(&name),
                "{name} not allow-listed"
            );
        }
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

/// The tools the Superagent session may call (`permission.rs` allow list);
/// kept in sync with the core list the session's `check_permission` reads.
#[cfg(test)]
#[must_use]
pub fn permission_allow_list() -> &'static [&'static str] {
    &[
        "kanboard",
        "project_resolver",
        "superagent_start",
        "superagent_stop",
        "superagent_relay",
        "superagent_set_mode",
        "superagent_status",
        "superagent_lead_model",
        "superagent.start",
        "superagent.stop",
        "superagent.relay",
        "superagent.set_mode",
        "superagent.status",
        "superagent.lead_model",
        "ask_user",
        "memory_search",
        "global_memory_search",
        "memory_list",
        "global_memory_list",
        "memory_store",
    ]
}

/// The Superagent chat's tool specs (name, description, schema properties, required).
#[cfg_attr(not(test), allow(dead_code))]
fn tool_specs() -> Vec<(
    &'static str,
    &'static str,
    serde_json::Value,
    Vec<&'static str>,
)> {
    vec![
        (
            "kanboard",
            "Read and edit a project's board. argv is the board CLI command with no `task` prefix; always pass projectId (except for `project list`). Commands: list [--all | --status LANE | --ready] (bare list hides backlog); show ID; search TEXT; chain ID; next; status; add TITLE [--body TEXT] [--priority none|low|medium|high|urgent] [--after ID] [--status backlog|todo] [--label L]; edit ID [--title T] [--body B] [--priority P] [--labels A,B]; move ID backlog|todo|cancelled [--comment why] (one ticket per call; cancelled needs --comment and an unclaimed backlog/todo ticket); link ID --after DEP; unlink ID --after DEP; order ID --top|--bottom|--before ID|--after-pos ID; note ID TEXT; project list; project show (lane counts); project add /abs/path; project unarchive (with projectId; both ask the user to confirm). Examples: [\"list\",\"--status\",\"backlog\"], [\"move\",\"OIN-12\",\"todo\"], [\"move\",\"OIN-13\",\"cancelled\",\"--comment\",\"filed on the wrong board\"]. Lanes past todo belong to workers: use superagent_start to get tickets worked.",
            json!({"projectId":{"type":"string","description":"Registered project slug from project_resolver, e.g. oino-f6a9c2."},"argv":{"type":"array","items":{"type":"string"},"description":"Board CLI arguments, first element is the command, e.g. [\"list\",\"--all\"]."}}),
            vec!["argv"],
        ),
        (
            "project_resolver",
            "List registered projects (slug = projectId, name, root folder), recent workspaces and frequent folders. Use it to map the project the user names to a projectId; when exactly one project fits, use it without asking.",
            json!({"reason":{"type":"string","description":"Why project discovery is needed."}}),
            vec![],
        ),
        (
            "superagent_start",
            "Hand tickets to the project's lead so they get worked. Launches a lead when none is running, otherwise queues the tickets to the running lead. Accepts backlog, todo or blocked tickets (no need to move them to todo first) and works even when the project's Auto is off. Put every ticket for one project in ONE call. Pick the mode for the batch: none = one focused ticket; swarm = independent tickets in parallel (with maxParallel); graph = dependent or multi-step work; goal = one verified outcome; ralph = long checklist. Presets: quick-fix, bug-batch (swarm x4), feature (graph x3), urgent, research. overrides: model \"provider/id\", thinking, autonomy (default high), mode, maxParallel, review none|light|full, profiles, priority. Unregistered or archived projects ask the user to confirm. E_LEAD_RUNNING means a lead with a different model/mode runs: stop it first to switch.",
            json!({"projectId":{"type":"string"},"taskIds":{"type":"array","items":{"type":"string"},"description":"Every ticket id to hand over, e.g. [\"OIN-12\",\"OIN-13\"]."},"presetId":{"type":"string","description":"quick-fix | bug-batch | feature | urgent | research, or a user preset."},"lead":{"type":"string","enum":["auto","reuse","new"],"description":"auto (default) wakes the project's lead chat; new starts a fresh chat; reuse reopens the last stopped lead's chat."},"overrides":{"type":"object","properties":{"model":{"type":"string"},"thinking":{"type":"string","enum":["off","minimal","low","medium","high","xhigh","max"]},"autonomy":{"type":"string","enum":["low","medium","high"]},"mode":{"type":"string","enum":["none","goal","ralph","swarm","graph"]},"maxParallel":{"type":"integer","minimum":1},"review":{"type":"string","enum":["none","light","full"]},"profiles":{"type":"array","items":{"type":"string"}},"priority":{"type":"boolean"}},"additionalProperties":false}}),
            vec!["projectId", "taskIds"],
        ),
        (
            "superagent_stop",
            "Stop the lead that holds this ticket. Use when the user asks, or before restarting a lead with a different model, mode or thinking level.",
            json!({"projectId":{"type":"string"},"taskId":{"type":"string"}}),
            vec!["projectId", "taskId"],
        ),
        (
            "superagent_relay",
            "Send a message to the project's running lead. delivery queue = after its current step (follow-on or dependent work); steer = adjust what it is doing now; priority true = urgent. Fails with E_WORKER_CLAIM when the project has no lead: then use superagent_start, and read the board yourself instead of relaying read-only questions.",
            json!({"projectId":{"type":"string"},"taskIds":{"type":"array","items":{"type":"string"},"description":"Tickets the message is about (the message is also logged on them)."},"text":{"type":"string"},"delivery":{"type":"string","enum":["queue","steer"]},"priority":{"type":"boolean"}}),
            vec!["projectId", "text", "delivery"],
        ),
        (
            "superagent_set_mode",
            "Switch a running lead's long-horizon mode for its next turn: none, goal, ralph, swarm or graph. Ask the user first when practical (skip asking only when they already named the mode); the switch itself is not gated. Fails when the project has no lead.",
            json!({"projectId":{"type":"string"},"mode":{"type":"string","enum":["none","goal","ralph","swarm","graph"]}}),
            vec!["projectId", "mode"],
        ),
        (
            "superagent_status",
            "Read-only snapshot of every project lead (state, health, open owner questions) and of what waits for the owner. Use it when the user asks how work is going or what needs them. It never changes anything.",
            json!({"projectId":{"type":"string","description":"Limit to one project (optional)."}}),
            vec![],
        ),
        (
            "superagent_lead_model",
            "Set a project lead's model when the user asks. Validates the model (E_UNKNOWN_MODEL for an unknown id), switches the running lead to it, and with remember:true also saves it as the project's lead model so the next launch keeps it. Never guess a model id: use one the user named (or one from the model picker).",
            json!({"projectId":{"type":"string"},"model":{"type":"string","description":"provider/id, e.g. ors/cc/claude-opus-5-5."},"remember":{"type":"boolean","description":"Also save it as the project's lead model for future launches (default false)."}}),
            vec!["projectId", "model"],
        ),
        (
            "ask_user",
            "Ask the user one free-text question and wait for the answer. Only for a genuinely ambiguous project, an irreversible action, or when the user asked you to check first; never to re-confirm an instruction they already gave.",
            json!({"question":{"type":"string"}}),
            vec!["question"],
        ),
    ]
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
    #[allow(
        clippy::too_many_lines,
        reason = "One arm per protocol operation or state transition; keep exhaustive dispatch and resource lifetimes visible together."
    )]
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
                if let Some(dispatcher) = &test_dispatcher {
                    return Ok(Arc::clone(dispatcher));
                }
                crate::superagent::current()
                    .cloned()
                    .map_err(|e| ToolError::new(e.to_string()))
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
                        .into_iter()
                        .filter(|project| !project.archived)
                        .collect::<Vec<_>>();
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
                    let mut cli_args: Vec<String> = serde_json::from_value(args["argv"].clone())
                        .map_err(|e| ToolError::new(e.to_string()))?;
                    strip_task_prefix(&mut cli_args);
                    if let Some(command) = project_command(&cli_args)? {
                        match command {
                            "add" | "unarchive" => {
                                let project_id = if command == "unarchive" {
                                    Some(field("projectId")?)
                                } else {
                                    None
                                };
                                let message = if let Some(id) = &project_id {
                                    format!("Unarchive project {id}?")
                                } else {
                                    format!("Register workspace {}?", cli_args[2])
                                };
                                let Some(ui) = session.get().filter(|ui| ui.has_ui()) else {
                                    return Err(ToolError::new(
                                        "Project registration requires user confirmation",
                                    ));
                                };
                                if crate::sessions::SessionService::shared().is_none_or(|service| {
                                    service.feed(ui.session_id()).subscriber_count() == 0
                                }) {
                                    return Err(ToolError::new(
                                        "Project registration requires user confirmation",
                                    ));
                                }
                                let params = serde_json::from_value(
                                    json!({"title":"Set up Kanboard project?","message":message}),
                                )
                                .map_err(|e| ToolError::new(e.to_string()))?;
                                let answer = tokio::time::timeout(
                                    std::time::Duration::from_secs(120),
                                    ui.dialog(
                                        Some("superagent"),
                                        "confirm",
                                        params,
                                        Some(cancel.clone()),
                                    ),
                                )
                                .await
                                .map_err(|_| {
                                    ToolError::new("Project registration confirmation timed out")
                                })?;
                                if answer != oino_core::ui::UiAnswer::Confirmed(true) {
                                    return Err(ToolError::new("Project registration declined"));
                                }
                                let dispatcher = dispatcher()?;
                                let result = dispatcher
                                    .enable(
                                        if command == "unarchive" {
                                            "unarchive"
                                        } else {
                                            "register"
                                        },
                                        project_id.as_deref(),
                                        if command == "add" {
                                            Some(cli_args[2].as_str())
                                        } else {
                                            None
                                        },
                                    )
                                    .await
                                    .map_err(|e| ToolError::new(e.to_string()))?;
                                return Ok(AgentToolResult::text(result.to_string()));
                            }
                            "list" => {
                                let layout = unipi_kanboard::Layout::from_env()
                                    .map_err(|e| ToolError::new(e.to_string()))?;
                                let result = unipi_kanboard::commands::project_list(&layout)
                                    .map_err(|e| ToolError::new(e.to_string()))?;
                                return Ok(AgentToolResult::text(result.to_string()));
                            }
                            "show" => {
                                let project_id = field("projectId")?;
                                let layout = unipi_kanboard::Layout::from_env()
                                    .map_err(|e| ToolError::new(e.to_string()))?;
                                let project = unipi_kanboard::Project::load(&layout, &project_id)
                                    .map_err(|e| ToolError::new(e.to_string()))?;
                                let result =
                                    unipi_kanboard::commands::project_show(&layout, &project)
                                        .map_err(|e| ToolError::new(e.to_string()))?;
                                return Ok(AgentToolResult::text(result.to_string()));
                            }
                            _ => {
                                return Err(ToolError::new(
                                    "Only project add, unarchive, list and show are allowed",
                                ));
                            }
                        }
                    }
                    let project_id = field("projectId")?;
                    let layout = unipi_kanboard::Layout::from_env()
                        .map_err(|e| ToolError::new(e.to_string()))?;
                    let Ok(project) = unipi_kanboard::Project::load(&layout, &project_id) else {
                        let dispatcher = dispatcher()?;
                        let blockers = dispatcher
                            .blockers(&project_id)
                            .await
                            .map_err(|e| ToolError::new(e.to_string()))?;
                        return Err(ToolError::new(blockers.first().map_or_else(
                            || "Project is unavailable".into(),
                            |b| b.message.clone(),
                        )));
                    };
                    if project.archived {
                        return Err(ToolError::new("Project is archived"));
                    }
                    alias_comment(&mut cli_args);
                    let command = cli_args.first().cloned().unwrap_or_default();
                    let help = (cli_args.len() == 1
                        && matches!(command.as_str(), "help" | "--help" | "-h"))
                        || (cli_args.len() >= 2
                            && matches!(cli_args[1].as_str(), "help" | "--help" | "-h"));
                    if !help && !BOARD_COMMANDS.contains(&command.as_str()) {
                        return Err(ToolError::new(match command.as_str() {
                            "start" | "finish" | "release" => format!(
                                "`{command}` is a worker transition; call superagent_start to hand the tickets to the project's lead."
                            ),
                            _ => format!(
                                "Unknown board command `{command}` (cli_args has no `task` prefix). Use one of: {}. Example: [\"list\",\"--status\",\"backlog\"], [\"move\",\"OIN-12\",\"todo\"].",
                                BOARD_COMMANDS.join(", ")
                            ),
                        }));
                    }
                    if command == "move" {
                        check_move(&layout, &project, &cli_args)?;
                    }
                    // Global flags cannot override the explicitly validated target/actor.
                    if cli_args.iter().any(|s| s.starts_with("--project")) {
                        return Err(ToolError::new(
                            "Don't pass --project; set the projectId argument instead.",
                        ));
                    }
                    if cli_args.iter().any(|s| {
                        s.starts_with("--actor")
                            || s.starts_with("--home")
                            || s.starts_with("--session")
                            || s.starts_with("--gate")
                            || s.starts_with("--origin")
                    }) {
                        return Err(ToolError::new("Global board overrides are not allowed"));
                    }
                    let schedules_todo = (command == "move"
                        && cli_args.get(2).is_some_and(|s| s == "todo"))
                        || (command == "add"
                            && cli_args.iter().enumerate().any(|(i, arg)| {
                                arg == "--status=todo"
                                    || (arg == "--status"
                                        && cli_args.get(i + 1).is_some_and(|s| s == "todo"))
                            }));
                    // Computed before `cli_args` is moved into the exec call.
                    let parked_backlog = command == "add"
                        && (cli_args.windows(2).any(|w| w == ["--status", "backlog"])
                            || cli_args.iter().any(|arg| arg == "--status=backlog"));
                    default_ticket_autonomy(&mut cli_args);
                    // OIN-83 origin stamp (set once at add): the Superagent chat
                    // and its newest owner entry id, so the lead can read the
                    // owner's exact words through owner_messages.
                    let owner_sid = if command == "add" {
                        session.get().map(|ui| ui.session_id().to_owned())
                    } else {
                        None
                    };
                    let origin = owner_sid.as_ref().map(|sid| {
                        crate::superagent_owner_messages::latest_owner_entry_from(sid).map_or_else(
                            || format!("superagent:{sid}"),
                            |entry| format!("superagent:{sid}:{entry}"),
                        )
                    });
                    let mut cli_args = [
                        vec![
                            "--project".into(),
                            project_id.clone(),
                            "--actor".into(),
                            "user".into(),
                        ],
                        cli_args,
                    ]
                    .concat();
                    if let Some(origin) = origin {
                        cli_args.push("--origin".into());
                        cli_args.push(origin);
                    }
                    let mut result =
                        unipi_kanboard::service::exec(&unipi_kanboard::service::ExecRequest {
                            argv: cli_args,
                            cwd: Some(project.root.clone()),
                            ..Default::default()
                        });
                    // Attach the latest owner turn's images; attachment failures never undo a saved ticket.
                    let mut attached_owner_images = 0usize;
                    if command == "add" && result["ok"] != false {
                        if let (Some(sid), Some(id)) = (
                            owner_sid.as_deref(),
                            result["id"].as_str().or_else(|| result["taskId"].as_str()),
                        ) {
                            for (index, (mime, bytes)) in
                                crate::superagent_owner_messages::latest_owner_images_from(sid)
                                    .into_iter()
                                    .enumerate()
                            {
                                let ext = match mime.as_str() {
                                    "image/png" => "png",
                                    "image/jpeg" => "jpg",
                                    "image/gif" => "gif",
                                    "image/webp" => "webp",
                                    _ => "bin",
                                };
                                let name = format!("image-{}.{}", index + 1, ext);
                                let stage = std::env::temp_dir().join(format!(
                                    "oino-board-{}-{}",
                                    std::process::id(),
                                    uuid::Uuid::now_v7()
                                ));
                                let staged = stage.join(&name);
                                let attempt = (|| -> Result<Value, String> {
                                    std::fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
                                    std::fs::write(&staged, bytes).map_err(|e| e.to_string())?;
                                    let attach_argv = vec![
                                        "--project".into(),
                                        project_id.clone(),
                                        "--actor".into(),
                                        "user".into(),
                                        "attach".into(),
                                        id.into(),
                                        staged.to_string_lossy().into_owned(),
                                        "--name".into(),
                                        name,
                                    ];
                                    Ok(unipi_kanboard::service::exec(
                                        &unipi_kanboard::service::ExecRequest {
                                            argv: attach_argv,
                                            cwd: Some(project.root.clone()),
                                            ..Default::default()
                                        },
                                    ))
                                })();
                                let _ = std::fs::remove_dir_all(&stage);
                                match attempt {
                                    Ok(reply) if reply["ok"] != false => attached_owner_images += 1,
                                    Ok(reply) => tracing::warn!(
                                        ?reply,
                                        "failed to attach Superagent owner image"
                                    ),
                                    Err(error) => {
                                        tracing::warn!(%error, "failed to stage Superagent owner image");
                                    }
                                }
                            }
                        }
                    }
                    if attached_owner_images > 0 {
                        if let Some(object) = result.as_object_mut() {
                            let old = object
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            object.insert(
                                "message".into(),
                                json!(format!(
                                    "{old} (attached {attached_owner_images} owner image(s))"
                                )
                                .trim()),
                            );
                        }
                    }
                    // OIN-83 lossless flow: `add` hands the new ticket to the
                    // project's lead in the same step, unless the Superagent
                    // explicitly parked it in backlog. The result names the
                    // lead; a failed hand-over is a notice, never an error
                    // (the ticket exists).
                    if command == "add" && result["ok"] != false {
                        let new_id = result["id"]
                            .as_str()
                            .map(str::to_owned)
                            .or_else(|| result["taskId"].as_str().map(str::to_owned));
                        if let Some(new_id) = new_id {
                            if !parked_backlog {
                                let dispatcher = dispatcher()?;
                                match dispatcher
                                    .start_tasks(
                                        &project_id,
                                        std::slice::from_ref(&new_id),
                                        None,
                                        crate::superagent_presets::Overrides::default(),
                                    )
                                    .await
                                {
                                    Ok(session_id) => {
                                        if let Some(object) = result.as_object_mut() {
                                            object.insert(
                                                "lead".into(),
                                                json!({"sessionId": session_id}),
                                            );
                                        }
                                    }
                                    Err(error) => {
                                        if let Some(object) = result.as_object_mut() {
                                            object.insert("notice".into(), json!(format!("Ticket saved, but not handed to a lead: {error}")));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if schedules_todo && result["ok"] != false {
                        let dispatcher = dispatcher()?;
                        let blockers = dispatcher
                            .blockers(&project_id)
                            .await
                            .map_err(|e| ToolError::new(e.to_string()))?;
                        if !blockers.is_empty() {
                            let reasons = blockers
                                .iter()
                                .map(|b| b.message.as_str())
                                .collect::<Vec<_>>()
                                .join(" ");
                            let notice = format!(
                                "Note: {project_id} will not auto-start these tickets ({reasons}). Call superagent_start to dispatch them now, or ask the user to enable Auto / Auto-start."
                            );
                            if let Some(object) = result.as_object_mut() {
                                object.insert("notice".into(), json!(notice));
                            }
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
                "superagent_status" => {
                    let dispatcher = dispatcher()?;
                    let project_id = args["projectId"]
                        .as_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty());
                    crate::superagent_questions::status_payload(&dispatcher, project_id)
                        .await
                        .map_err(|e| ToolError::new(e.to_string()))?
                }
                _ => {
                    let project = field("projectId")?;
                    let dispatcher = dispatcher()?;
                    match name.as_str() {
                        "superagent_start" => {
                            let overrides: crate::superagent_presets::Overrides = args
                                .get("overrides")
                                .cloned()
                                .map(serde_json::from_value)
                                .transpose()
                                .map_err(|e| ToolError::new(format!("Invalid overrides: {e}")))?
                                .unwrap_or_default();
                            let tasks: Vec<String> =
                                serde_json::from_value(args["taskIds"].clone())
                                    .map_err(|e| ToolError::new(e.to_string()))?;
                            let lead = args["lead"]
                                .as_str()
                                .filter(|lead| !lead.is_empty() && *lead != "auto")
                                .map(str::to_owned);
                            let first = dispatcher
                                .start_tasks_with_lead(
                                    &project,
                                    &tasks,
                                    args["presetId"].as_str(),
                                    overrides.clone(),
                                    lead.as_deref(),
                                )
                                .await;
                            let id = match first {
                                Ok(id) => id,
                                Err(error)
                                    if matches!(
                                        error.code,
                                        "E_PROJECT_NOT_REGISTERED" | "E_PROJECT_ARCHIVED"
                                    ) =>
                                {
                                    let path = if error.code == "E_PROJECT_NOT_REGISTERED" {
                                        let recent = hub.projects().await.unwrap_or_default();
                                        recent
                                            .into_iter()
                                            .find(|workspace| {
                                                let path = std::path::Path::new(&workspace.cwd);
                                                let canonical = std::fs::canonicalize(path)
                                                    .unwrap_or_else(|_| path.to_path_buf());
                                                unipi_kanboard::store::slug_for(&canonical)
                                                    == project
                                            })
                                            .map(|workspace| workspace.cwd)
                                    } else {
                                        None
                                    };
                                    if error.code == "E_PROJECT_NOT_REGISTERED" && path.is_none() {
                                        return Err(ToolError::new(error.to_string()));
                                    }
                                    let Some(ui) = session.get().filter(|ui| ui.has_ui()) else {
                                        return Err(ToolError::new(error.to_string()));
                                    };
                                    if !crate::sessions::SessionService::shared().is_some_and(
                                        |service| {
                                            service.feed(ui.session_id()).subscriber_count() > 0
                                        },
                                    ) {
                                        return Err(ToolError::new(error.to_string()));
                                    }
                                    let title = if path.is_some() {
                                        format!("Set up Kanboard for {project}?")
                                    } else {
                                        format!("Unarchive {project}?")
                                    };
                                    let params = serde_json::from_value(json!({"title":title,"message":format!("{} Superagent will then start {}.", error.message, tasks.join(", "))})).map_err(|e| ToolError::new(e.to_string()))?;
                                    let answer = tokio::time::timeout(
                                        std::time::Duration::from_secs(120),
                                        ui.dialog(
                                            Some("superagent"),
                                            "confirm",
                                            params,
                                            Some(cancel.clone()),
                                        ),
                                    )
                                    .await
                                    .map_err(|_| ToolError::new(error.to_string()))?;
                                    if answer != oino_core::ui::UiAnswer::Confirmed(true) {
                                        return Err(ToolError::new(format!(
                                            "User declined: {}",
                                            error.message
                                        )));
                                    }
                                    let action = if path.is_some() {
                                        "register"
                                    } else {
                                        "unarchive"
                                    };
                                    let enabled = dispatcher
                                        .enable(action, Some(&project), path.as_deref())
                                        .await
                                        .map_err(|e| ToolError::new(e.to_string()))?;
                                    let id = enabled["projectId"].as_str().unwrap_or(&project);
                                    dispatcher
                                        .start_tasks_with_lead(
                                            id,
                                            &tasks,
                                            args["presetId"].as_str(),
                                            overrides,
                                            lead.as_deref(),
                                        )
                                        .await
                                        .map_err(|e| ToolError::new(e.to_string()))?
                                }
                                Err(error) => return Err(ToolError::new(error.to_string())),
                            };
                            json!({"sessionId":id})
                        }
                        "superagent_set_mode" => dispatcher
                            .set_mode(&project, &field("mode")?)
                            .await
                            .map_err(|e| ToolError::new(e.to_string()))?,
                        "superagent_stop" => {
                            json!({"stopped":dispatcher.stop(&project, &field("taskId")?).await.map_err(|e| ToolError::new(e.to_string()))?})
                        }
                        "superagent_relay" => {
                            let tasks: Vec<String> = args
                                .get("taskIds")
                                .cloned()
                                .map(serde_json::from_value)
                                .transpose()
                                .map_err(|e| ToolError::new(format!("Invalid taskIds: {e}")))?
                                .unwrap_or_default();
                            let delivery = crate::superagent::Delivery::parse(&field("delivery")?)
                                .map_err(|e| ToolError::new(e.to_string()))?;
                            dispatcher
                                .relay_to_lead(
                                    &project,
                                    &field("text")?,
                                    &tasks,
                                    delivery,
                                    args["priority"].as_bool().unwrap_or(false),
                                )
                                .await
                                .map_err(|e| ToolError::new(e.to_string()))?
                        }
                        "superagent_lead_model" => {
                            let remember = args["remember"].as_bool().unwrap_or(false);
                            dispatcher
                                .set_lead_model(&project, &field("model")?, remember)
                                .await
                                .map_err(|e| ToolError::new(e.to_string()))?
                        }
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
pub fn runtimes() -> Vec<Arc<AgentSessionRuntime>> {
    RUNTIMES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .values()
        .cloned()
        .collect()
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

/// The lead's tools: the board tool plus the read-only `owner_messages` tool
/// (OIN-83). Declared together at launch and on reopen.
#[must_use]
pub fn lead_tools(
    project: unipi_kanboard::Project,
    session: &str,
    sessions_root: &std::path::Path,
) -> Vec<ToolEntry> {
    let mut tools = vec![worker_board(project.clone(), session.to_owned())];
    tools.push(owner_messages_tool(project, sessions_root.to_path_buf()));
    tools
}

/// The lead's role section (OIN-83): the kanboard tool's guidelines, rendered
/// by `build_rules` into the INITIAL system prompt only (creation-time; no
/// preset values — those travel in each brief — so an old lead keeps its
/// frozen prompt and a changed preset never invalidates it).
pub const LEAD_ROLE_GUIDELINES: &[&str] = &[
    "You are this project's lead: a long-lived session that the owner's Superagent hands tickets to. New work and answers arrive as messages in this chat.",
    "Claim with [\"start\",\"ID\"]. Finish with [\"finish\",\"ID\",\"--comment\",\"summary\"], or block with [\"move\",\"ID\",\"blocked\",\"--comment\",\"what you need\"]. Never end a turn with a ticket in progress and no plan for it.",
    "Ask the owner with ask_user, and start the question with the ticket id (\"OIN-12: …\"). The answer comes back in the same turn. If nobody answers in time, the ticket becomes blocked and the answer arrives later as a message.",
    "For UI tasks, attach a screenshot of the visible result to the ticket with finish --attach <png> or attach <ID> <png> --note <what it shows>; this is optional when there is no visual result or no screenshot can be taken. Do not only paste the screenshot in chat. Keep finish comments to high-level details in simple technical English; include implementation detail only when the owner asks.",
    "When a ticket is unclear, read the owner's original words with owner_messages before you ask. Ticket images: inspect show <ID> --json and attachments <ID>; open each absolute image path with read (images are returned visually) when your model supports vision. If it does not, use image_recognize on that path. If the ticket cites an owner message in Superagent chat, read that exact message with owner_messages too; the original message may hold the only copy of an image. Never silently skip an available mockup.",
    "Each work message names its preset (mode, parallelism, review, profiles). Follow it for that batch.",
    "Add follow-up work you discover with [\"add\",\"Title\",\"--body\",\"…\"]. Follow-up tickets go to the backlog; the Superagent schedules them.",
    "After a host restart, continue where you stopped: your tickets and their claims are still yours. Check the workspace and git state before repeating any outside action (push, deploy, message).",
    "Use memory_store to keep project conventions you learn; memory is how you stay consistent across restarts.",
    "Use the kanboard tool to attach screenshots: [\"finish\",\"ID\",\"--comment\",\"summary\",\"--attach\",\"/abs/image.png\"] or [\"attach\",\"ID\",\"/abs/image.png\",\"--note\",\"what it shows\"]. For ticket images, show --json returns attachments with absolute paths; read images for vision models or use image_recognize otherwise. When a ticket cites an owner message, read it with owner_messages too; original messages may contain the only image copy. Keep reports concise and high-level in simple technical English unless requested otherwise.",
];

/// Worker board calls carry the raw preallocated Run.session (not legacy `oino-<id>` tags).
#[must_use]
pub fn worker_board(project: unipi_kanboard::Project, session: String) -> ToolEntry {
    ToolEntry::builtin(
        Arc::new(WorkerBoard {
            project,
            session,
            meta: ToolDefinitionMeta::new(
                "kanboard",
                "Kanboard",
                "Read/transition tickets using CLI cli_args: show ID --json (attachment absolute paths), attachments ID; start ID; finish ID --comment TEXT [--attach /abs/image.png]…; attach ID /abs/file --note TEXT; move ID blocked --comment TEXT. For image paths use read with a vision-capable model or image_recognize otherwise. Session/project identity is supplied by the host.",
                json!({"type":"object","properties":{"argv":{"type":"array","items":{"type":"string"}}},"required":["argv"],"additionalProperties":false}),
            ),
        }),
        Some("Read and transition the assigned board ticket".into()),
        LEAD_ROLE_GUIDELINES
            .iter()
            .map(|rule| (*rule).to_owned())
            .collect(),
    )
}
/// The board tool for a lead session woken from its file after the dispatcher retired it
/// (the user keeps talking to the lead directly). Same identity as at launch: the lead
/// marker names the project and the session id is the reserved lead id. Non-lead files,
/// and leads whose project is gone or archived, get nothing.
#[must_use]
pub fn reopened_lead_tools(path: &std::path::Path, session: &str) -> Vec<ToolEntry> {
    let Some(card) = worker_file(path) else {
        return Vec::new();
    };
    let project = unipi_kanboard::Layout::from_env()
        .and_then(|layout| unipi_kanboard::Project::load(&layout, &card.project_id));
    match project {
        Ok(project) if !project.archived => {
            // Session files live at <sessions_root>/<encoded cwd>/<file>.
            let sessions_root = path
                .parent()
                .and_then(std::path::Path::parent)
                .map(std::path::Path::to_path_buf)
                .unwrap_or_default();
            lead_tools(project, session, &sessions_root)
        }
        _ => Vec::new(),
    }
}

/// The lead's `owner_messages` tool (OIN-83): paged, read-only access to the
/// owner's verbatim messages of a Superagent chat that sent work to the lead's
/// project. Declared next to the board tool at launch and on reopen.
#[must_use]
pub fn owner_messages_tool(
    project: unipi_kanboard::Project,
    sessions_root: std::path::PathBuf,
) -> ToolEntry {
    let (name, description, properties, required) = crate::superagent_owner_messages::spec();
    ToolEntry::builtin(
        Arc::new(OwnerMessagesTool {
            project,
            sessions_root,
            meta: ToolDefinitionMeta::new(
                name,
                "Owner messages",
                description,
                json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
            ),
        }),
        Some(description.into()),
        Vec::new(),
    )
}

struct OwnerMessagesTool {
    project: unipi_kanboard::Project,
    sessions_root: std::path::PathBuf,
    meta: ToolDefinitionMeta,
}
impl AgentTool for OwnerMessagesTool {
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
        let sessions_root = self.sessions_root.clone();
        Box::pin(async move {
            let chat = call.args.get("chatId").and_then(Value::as_str);
            let query = call.args.get("query").and_then(Value::as_str);
            let before = call.args.get("before").and_then(Value::as_str);
            let limit = call.args.get("limit").and_then(Value::as_u64).unwrap_or(20);
            crate::superagent_owner_messages::page(
                &unipi_kanboard::Layout::from_env().map_err(|e| ToolError::new(e.to_string()))?,
                &project,
                chat,
                query,
                before,
                usize::try_from(limit).unwrap_or(20),
                &sessions_root,
            )
            .await
            .map(|value| AgentToolResult::text(value.to_string()))
            .map_err(ToolError::new)
        })
    }
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
            let cli_args: Vec<String> =
                serde_json::from_value(call.args.get("argv").cloned().unwrap_or_default())
                    .map_err(|e| ToolError::new(e.to_string()))?;
            if cli_args.iter().any(|s| {
                s.starts_with("--project")
                    || s.starts_with("--actor")
                    || s.starts_with("--session")
                    || s.starts_with("--gate")
                    || s.starts_with("--origin")
            }) {
                return Err(ToolError::new("Worker identity cannot be overridden"));
            }
            // OIN-83 origin stamp: a lead-created ticket names the lead session.
            let origin = if cli_args.first().is_some_and(|cmd| cmd == "add") {
                vec!["--origin".to_owned(), format!("lead:{session}")]
            } else {
                Vec::new()
            };
            let cli_args = [
                vec![
                    "--project".into(),
                    project.slug,
                    "--session".into(),
                    session,
                    "--actor".into(),
                    "agent".into(),
                ],
                origin,
                cli_args,
            ]
            .concat();
            let result = unipi_kanboard::service::exec(&unipi_kanboard::service::ExecRequest {
                argv: cli_args,
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
pub fn superagent_title_in_project(
    name: Option<&str>,
    first_message: &str,
    created_ms: i64,
    project: Option<&str>,
) -> String {
    if let Some(name) = name
        .map(str::trim)
        .filter(|name| !name.is_empty() && *name != "Superagent")
    {
        return name.to_owned();
    }
    let mut seed = first_message.trim_start();
    let mut skill = None;
    if let Some(frontmatter) = seed
        .strip_prefix("---\n")
        .or_else(|| seed.strip_prefix("---\r\n"))
    {
        if let Some((header, rest)) = frontmatter.split_once("\n---") {
            skill = header
                .lines()
                .find_map(|line| line.trim().strip_prefix("name:").map(str::trim))
                .filter(|name| !name.is_empty());
            seed = rest
                .trim_start_matches('\r')
                .trim_start_matches('\n')
                .trim_start();
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
        let Some((_, body)) = seed.split_once('\n') else {
            break;
        };
        let Some(end) = body
            .lines()
            .position(|line| line.trim_start().starts_with(fence))
        else {
            break;
        };
        let consumed: usize = body.split_inclusive('\n').take(end + 1).map(str::len).sum();
        seed = body[consumed..].trim_start();
    }
    let message = seed.split_whitespace().collect::<Vec<_>>().join(" ");
    if !message.is_empty() && message != "(no messages)" {
        let mut chars = message.chars();
        let head: String = chars.by_ref().take(60).collect();
        return if chars.next().is_some() {
            format!("{head}…")
        } else {
            head
        };
    }
    let at = chrono::DateTime::from_timestamp_millis(created_ms).map_or_else(
        || "unknown".into(),
        |at| {
            at.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        },
    );
    format!("Superagent · {at}")
}

/// A bounded header-prefix check, not a generic session wake (which could expose shell tools).
#[must_use]
pub fn is_superagent_file(path: &std::path::Path) -> bool {
    session_markers(path).0
}

// Sidebar classification and worker cards inspect the same header prefix. Share the
// read/parse and cache: cold lists used to open and parse every transcript twice.
fn session_markers(path: &std::path::Path) -> (bool, Option<unipi_proto::session::WorkerCard>) {
    use std::io::{BufRead, BufReader};
    use std::sync::Mutex;
    type Markers = (bool, Option<unipi_proto::session::WorkerCard>);
    type Cache = std::collections::HashMap<PathBuf, (std::time::SystemTime, u64, Markers)>;
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let Ok(meta) = std::fs::metadata(path) else {
        return (false, None);
    };
    let Ok(modified) = meta.modified() else {
        return (false, None);
    };
    let cache = CACHE.get_or_init(Default::default);
    if let Some((_, _, result)) = cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(path)
        .filter(|(time, len, _)| *time == modified && *len == meta.len())
    {
        return result.clone();
    }
    let Ok(file) = std::fs::File::open(path) else {
        return (false, None);
    };
    let mut result = (false, None);
    for line in BufReader::new(file).lines().take(32).map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        result.0 |= (value["type"] == "custom" && value["customType"] == "oino:superagent")
            || (value["type"] == "session" && value["cwd"].as_str() == Some(""));
        if result.1.is_none()
            && value["type"] == "custom"
            && value["customType"] == "oino:superagent-worker"
        {
            result.1 = serde_json::from_value(value["data"].clone()).ok();
        }
    }
    cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(path.to_path_buf(), (modified, meta.len(), result.clone()));
    result
}

#[cfg(test)]
mod marker_tests {
    use super::*;

    #[test]
    fn sidebar_markers_share_a_prefix_and_invalidate_after_append() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chat.jsonl");
        std::fs::write(&path, "{\"type\":\"session\",\"cwd\":\"/project\"}\n").unwrap();
        assert!(!is_superagent_file(&path));
        assert!(worker_file(&path).is_none());
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(file, "{}", serde_json::json!({"type":"custom", "customType":"oino:superagent-worker", "data":{"taskId":"P-1", "projectId":"project"}})).unwrap();
        assert_eq!(worker_file(&path).unwrap().task_id, "P-1");
        assert!(!is_superagent_file(&path));
        writeln!(
            file,
            "{}",
            serde_json::json!({"type":"custom", "customType":"oino:superagent"})
        )
        .unwrap();
        assert!(is_superagent_file(&path));
        assert_eq!(worker_file(&path).unwrap().task_id, "P-1");
    }
}

#[cfg(test)]
mod title_tests {
    use super::{superagent_title, superagent_title_in_project};

    #[test]
    fn explicit_name_wins() {
        assert_eq!(
            superagent_title(Some("Launch checklist"), "hello", 0),
            "Launch checklist"
        );
    }

    #[test]
    fn fixed_superagent_name_falls_back_to_the_first_message() {
        assert_eq!(
            superagent_title(Some("Superagent"), "What is running?", 0),
            "What is running?"
        );
        assert_eq!(
            superagent_title(None, "What is running?", 0),
            "What is running?"
        );
    }

    #[test]
    fn frontmatter_and_leading_fences_do_not_become_titles() {
        assert_eq!(
            superagent_title(
                None,
                "---\nname: untangle-codebase\ndescription: test\n---\nInstructions",
                0
            ),
            "untangle-codebase"
        );
        assert_eq!(
            superagent_title(None, "```text\nignore this\n```\nFix the project", 0),
            "Fix the project"
        );
        assert_eq!(
            superagent_title_in_project(
                None,
                "---\nname: untangle-codebase\n---\nInstructions",
                0,
                Some("demoeditor-migration")
            ),
            "untangle-codebase · demoeditor-migration"
        );
        assert_eq!(
            superagent_title_in_project(
                Some("My title"),
                "---\nname: skill\n---\ntext",
                0,
                Some("project")
            ),
            "My title"
        );
    }

    #[test]
    fn first_message_is_collapsed_trimmed_and_truncated() {
        assert_eq!(
            superagent_title(None, "  What\n\tis\t running?  ", 0),
            "What is running?"
        );
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
#[must_use]
pub fn worker_file(path: &std::path::Path) -> Option<unipi_proto::session::WorkerCard> {
    session_markers(path).1
}

pub fn unregister(id: &str) {
    RUNTIMES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(id);
}

#[cfg(test)]
mod status_tool_tests {
    use super::*;
    use crate::sessions::{HostHubOptions, SessionHub};
    use crate::superagent::{Dispatcher, Workers};
    use oino_core::extensions::api::ExtensionMode;
    use oino_core::runtime::ToolSelection;
    use oino_core::services::{ResourceFlags, StaticLoader};
    use std::collections::HashSet;
    use std::sync::Mutex;
    use unipi_kanboard::{
        commands,
        model::{Actor, ChainGate, Priority, Status, Task},
        store::{Layout, Project},
    };

    #[derive(Default)]
    struct MiniWorkers {
        live: Mutex<HashSet<String>>,
    }

    impl Workers for MiniWorkers {
        fn live<'a>(&'a self, id: &'a str) -> BoxFuture<'a, bool> {
            Box::pin(async move { self.live.lock().unwrap().contains(id) })
        }
        fn launch<'a>(
            &'a self,
            _: &'a Project,
            _: &'a Task,
            id: &'a str,
        ) -> BoxFuture<'a, std::result::Result<(), String>> {
            Box::pin(async move {
                self.live.lock().unwrap().insert(id.to_owned());
                Ok(())
            })
        }
        fn stop<'a>(&'a self, id: &'a str) -> BoxFuture<'a, std::result::Result<(), String>> {
            Box::pin(async move {
                self.live.lock().unwrap().remove(id);
                Ok(())
            })
        }
        fn relay<'a>(
            &'a self,
            _: &'a str,
            _: &'a str,
        ) -> BoxFuture<'a, std::result::Result<(), String>> {
            Box::pin(async { Ok(()) })
        }
        fn ended<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Option<String>> {
            Box::pin(async { None })
        }
    }

    #[tokio::test]
    #[allow(
        clippy::too_many_lines,
        reason = "Keep this linear integration scenario and its assertions together."
    )]
    async fn superagent_status_tool_executes_with_empty_args_and_project_filter() {
        let dir = tempfile::tempdir().unwrap();
        let root_a = tempfile::tempdir().unwrap();
        let root_b = tempfile::tempdir().unwrap();
        let layout = Layout::with_home(dir.path().join("board"));
        let proj_a = Project::create(&layout, root_a.path(), Some("alpha"), Some("ALP")).unwrap();
        let proj_b = Project::create(&layout, root_b.path(), Some("beta"), Some("BET")).unwrap();
        let common = commands::Common::new(Actor::User, ChainGate::InReview);
        let task_a = commands::add(
            &layout,
            proj_a.clone(),
            &common,
            "alpha work",
            Some("body"),
            Some(Status::Todo),
            Priority::None,
            &[],
            &[],
            &[],
            false,
        )
        .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let task_b = commands::add(
            &layout,
            proj_b.clone(),
            &common,
            "beta work",
            Some("body"),
            Some(Status::Todo),
            Priority::None,
            &[],
            &[],
            &[],
            false,
        )
        .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();

        let workers: Arc<dyn Workers> = Arc::new(MiniWorkers::default());
        let dispatcher = Dispatcher::acquire(layout, dir.path().join("config.json"), workers)
            .await
            .unwrap();
        let lead_a = dispatcher.start(&proj_a.slug, &task_a).await.unwrap();
        let lead_b = dispatcher.start(&proj_b.slug, &task_b).await.unwrap();

        let hub = SessionHub::create(HostHubOptions {
            cwd: dir.path().into(),
            agent_dir: dir.path().join("agent"),
            docs_root: None,
            session_dir: Some(dir.path().join("sessions")),
            mode: ExtensionMode::Desktop,
            has_ui: false,
            approve: Some(false),
            flags: ResourceFlags {
                no_context_files: true,
                ..Default::default()
            },
            loader: Arc::new(StaticLoader(Vec::new())),
            models: Some(oino_models::ModelRegistry::in_memory(Arc::new(
                oino_models::MapEnv::default(),
            ))),
            tools: ToolSelection::Default,
            exclude_tools: Vec::new(),
            services_cap: None,
            host_data_dir: None,
        })
        .unwrap();

        let (name, description, properties, required) = tool_specs()
            .into_iter()
            .find(|(n, ..)| *n == "superagent_status")
            .unwrap();
        assert!(required.is_empty(), "schema declares projectId optional");
        let tool = ChatTool {
            meta: ToolDefinitionMeta::new(
                name,
                "Superagent status",
                description,
                json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
            ),
            hub,
            session: Arc::new(OnceLock::new()),
            dispatcher: Some(Arc::clone(&dispatcher)),
        };

        // Call through the real `AgentTool::execute` path with `{}` (no projectId).
        let all_res = tool
            .execute(
                ToolInvocation {
                    tool_call_id: "c1".into(),
                    parent_tool_call_id: None,
                    args: serde_json::Map::new(),
                },
                UpdateSink::discard(),
                CancellationToken::new(),
            )
            .await
            .expect("empty args must return all-project status");
        let all_text = serde_json::to_value(&all_res.content[0]).unwrap()["text"]
            .as_str()
            .unwrap()
            .to_owned();
        let all_json: serde_json::Value = serde_json::from_str(&all_text).unwrap();
        let all_leads = all_json["leads"].as_array().unwrap();
        assert_eq!(all_leads.len(), 2, "{all_json}");
        let ids: HashSet<&str> = all_leads
            .iter()
            .filter_map(|r| r["sessionId"].as_str())
            .collect();
        assert!(ids.contains(lead_a.as_str()) && ids.contains(lead_b.as_str()));

        // Scoped call with `{"projectId": proj_a.slug}` returns only alpha.
        let mut scoped_args = serde_json::Map::new();
        scoped_args.insert("projectId".into(), json!(proj_a.slug));
        let scoped_res = tool
            .execute(
                ToolInvocation {
                    tool_call_id: "c2".into(),
                    parent_tool_call_id: None,
                    args: scoped_args,
                },
                UpdateSink::discard(),
                CancellationToken::new(),
            )
            .await
            .expect("projectId filter must succeed");
        let scoped_text = serde_json::to_value(&scoped_res.content[0]).unwrap()["text"]
            .as_str()
            .unwrap()
            .to_owned();
        let scoped_json: serde_json::Value = serde_json::from_str(&scoped_text).unwrap();
        let scoped_leads = scoped_json["leads"].as_array().unwrap();
        assert_eq!(scoped_leads.len(), 1);
        assert_eq!(scoped_leads[0]["sessionId"], lead_a);
    }
}

#[cfg(test)]
#[path = "superagent_note_tests.rs"]
mod note_tests;
