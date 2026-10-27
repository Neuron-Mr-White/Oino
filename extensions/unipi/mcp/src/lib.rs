//! `unipi.mcp` — MCP client and dynamic tools (unipi `packages/mcp`, unipi-port §3.22).
//!
//! | unipi | here |
//! |---|---|
//! | stdio JSON-RPC client, 10 s timeout, initialize handshake | [`client`] over host `process` pipes; plus streamable HTTP over `http.fetch` (pi `mcp.json` `url` entries) |
//! | config merge global/project (`mcp-config.json` + `config.json` enabled states) | `/ext-global/mcp.json` + `/ext-global/config.json`; trusted `/project/.oino/mcp.json`; ACP `session/new` servers from `oino:acp:mcp-servers` ([`config`]) |
//! | parallel connect, registration barrier, sorted `<server>__<tool>`, canonical schemas, ≤ 20 servers | [`registry`], [`translate`] |
//! | 64 KiB head/tail bound + 0600 spill | `unipi_kit::bounded` |
//! | `/mcp-status`, runners `mcp-add`/`-settings`/`-sync`/`-reload` | same; runners are hidden commands; reload works live (Oino can unregister tools) |
//! | server list overlay, catalog browser with JSON editor | overlay panels + `editor` dialogs with validation |
//! | catalog seed (49) + GitHub sync | [`catalog`], cache `/ext-global/servers.json` |
//! | `MCP_SERVER_STARTED` / `_ERROR` / `MCP_TOOLS_REGISTERED`, info-screen group | same topics; `info-screen.groups` service |

#![allow(
    clippy::unused_async_trait_impl,
    reason = "`Extension::init` is async in the SDK; this init registers synchronously"
)]
#![cfg_attr(
    test,
    allow(
        clippy::assert_is_empty,
        reason = "emptiness asserts read better in tests"
    )
)]

mod catalog;
mod client;
mod config;
mod registry;
mod rpc;
mod translate;

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use futures::future::join_all;
use oino_extension::prelude::*;
use oino_extension::sync::SessionMap;
use oino_extension::wit::registry::{Exposure, ToolAnnotations};
use unipi_kit::bus::Delivery;
use unipi_kit::settings::{Field, Namespace, Section};
use unipi_kit::topics::{self, McpServer, McpTools, ModuleReady};

use crate::client::{Client, Target};
use crate::config::{ConfigFile, Resolved, ServerDef, Source, Transport};
use crate::registry::{Registry, Status};
use crate::translate::ToolDef;

/// Extension id.
const EXTENSION_ID: &str = "unipi.mcp";
const VERSION: &str = "3.0.0";
const GLOBAL_CONFIG: &str = "/ext-global/mcp.json";
const GLOBAL_META: &str = "/ext-global/config.json";
const CATALOG_CACHE: &str = "/ext-global/servers.json";
const PROJECT_CONFIG: &str = "/project/.oino/mcp.json";

#[derive(Clone, Debug)]
enum View {
    Servers,
    Server(String),
    Catalog(String),
}

/// The extension.
#[derive(Default)]
pub struct Mcp {
    registry: RefCell<Registry>,
    clients: RefCell<BTreeMap<String, Rc<Client>>>,
    acp: RefCell<Vec<(String, ServerDef)>>,
    errors: RefCell<Vec<String>>,
    active_sessions: Cell<usize>,
    started: Cell<bool>,
    panels: SessionMap<BTreeMap<String, View>>,
}

fn now_iso() -> String {
    unipi_kit::fmt::iso_from_ms(timers::now_ms())
}

fn read_doc(path: &str) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    Some(serde_json::from_str(&text).unwrap_or_else(|e| json!({"__parseError": e.to_string()})))
}

fn write_private(path: &str, value: &Value) -> Result<()> {
    let _ = unipi_kit::paths::ensure_parent(std::path::Path::new(path));
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    Ok(oino_extension::wit::fs::atomic_write(
        path,
        text.as_bytes(),
        Some(0o600),
    )?)
}

/// The `mcp` hub namespace: action rows only (unipi `settings.ts`).
fn namespace() -> Namespace {
    Namespace::new("mcp", "MCP", EXTENSION_ID).section(
        Section::new("servers", "Servers")
            .description("Server registry lives in its own overlay (catalog, jira, …)")
            .field(
                Field::action("configure", "Configure MCP servers…", "mcp-settings")
                    .description("Open the server registry: add, edit and enable servers."),
            )
            .field(
                Field::action("add", "Add server…", "mcp-add")
                    .description("Browse the catalog or paste a custom config."),
            )
            .field(
                Field::action("sync", "Sync catalog…", "mcp-sync")
                    .description("Refresh the server catalog from GitHub."),
            )
            .field(
                Field::action("reload", "Reload servers…", "mcp-reload")
                    .description("Reconnect every server and re-register its tools."),
            ),
    )
}

fn exposure_of(word: &str) -> Exposure {
    match word {
        "deferred" | "codemode" | "codemode-deferred" => Exposure::Deferred,
        "hidden" => Exposure::Hidden,
        _ => Exposure::Direct,
    }
}

fn annotations_of(a: Option<&Value>) -> Option<ToolAnnotations> {
    let a = a?;
    let b = |k: &str| a.get(k).and_then(Value::as_bool);
    Some(ToolAnnotations {
        read_only: b("readOnlyHint"),
        destructive: b("destructiveHint"),
        idempotent: b("idempotentHint"),
        open_world: b("openWorldHint"),
    })
}

impl Mcp {
    // ── configuration ───────────────────────────────────────────────────────

    fn load(&self) -> (Vec<Resolved>, Vec<String>) {
        let mut errors = Vec::new();
        let mut file = |path: &str, project: bool| -> Option<ConfigFile> {
            let doc = read_doc(path)?;
            if let Some(e) = doc.get("__parseError").and_then(Value::as_str) {
                errors.push(format!("Failed to read {path}: {e}"));
                return None;
            }
            let f = config::parse_file(&doc, project);
            errors.extend(f.errors.iter().map(|e| format!("{path}: {e}")));
            Some(f)
        };
        let global = file(GLOBAL_CONFIG, false).unwrap_or_default();
        let project = if control::is_project_trusted() {
            file(PROJECT_CONFIG, true)
        } else {
            None
        };
        let meta = read_doc(GLOBAL_META)
            .map(|m| config::meta_enabled(&m))
            .unwrap_or_default();
        let resolved = config::resolve(&global, &meta, project.as_ref(), &meta, &self.acp.borrow());
        (resolved, errors)
    }

    fn lookup_var(name: &str) -> Option<String> {
        secrets::get(name)
            .ok()
            .flatten()
            .or_else(|| host::env(name).ok().flatten())
    }

    async fn run_value_command(cmd: &str) -> Option<String> {
        let windows = host::info().platform == "windows";
        let (prog, flag) = if windows { ("cmd", "/C") } else { ("sh", "-c") };
        let r = process::exec(prog, &[flag, cmd], None, Some(10_000))
            .await
            .ok()?;
        (r.code == Some(0)).then(|| r.stdout.trim().to_owned())
    }

    async fn expand_value(value: &str) -> Option<String> {
        match config::expand(value, &Self::lookup_var) {
            Ok(v) => Some(v),
            Err(cmd) => Self::run_value_command(&cmd).await,
        }
    }

    /// Expands variables, commands and `~/` into a connection target.
    async fn target(def: &ServerDef) -> Target {
        let home = host::env("HOME")
            .ok()
            .flatten()
            .or_else(|| host::env("USERPROFILE").ok().flatten());
        match def.transport {
            Transport::Stdio => {
                let mut env = Vec::new();
                for (k, v) in &def.env {
                    match Self::expand_value(v).await {
                        // An unresolved whole-value `${K}` is left to the inherited environment.
                        Some(x) if x == format!("${{{k}}}") => {}
                        Some(x) => env.push((k.clone(), x)),
                        None => {}
                    }
                }
                Target::Stdio {
                    command: config::expand_home(&def.command, home.as_deref()),
                    args: def
                        .args
                        .iter()
                        .map(|a| config::expand_home(a, home.as_deref()))
                        .collect(),
                    env,
                    cwd: def
                        .cwd
                        .as_deref()
                        .map(|c| config::expand_home(c, home.as_deref())),
                }
            }
            Transport::Http => {
                let mut headers = Vec::new();
                for (k, v) in &def.headers {
                    if let Some(x) = Self::expand_value(v).await {
                        headers.push((k.clone(), x));
                    }
                }
                Target::Http {
                    url: def.url.clone(),
                    headers,
                }
            }
        }
    }

    // ── lifecycle ───────────────────────────────────────────────────────────

    async fn connect(r: &Resolved) -> Result<(Client, Vec<ToolDef>), String> {
        let timeout = r.def.timeout_ms.unwrap_or(config::STARTUP_TIMEOUT_MS);
        let target = Self::target(&r.def).await;
        let client = Client::connect(&r.name, &target, timeout, VERSION).await?;
        let listed = match client.list_tools().await {
            Ok(t) => t,
            Err(e) => {
                client.disconnect().await;
                return Err(e);
            }
        };
        let mut tools: Vec<ToolDef> = listed
            .iter()
            .filter_map(|t| translate::translate(&r.name, t))
            .collect();
        tools.sort_by(|a, b| translate::compare_code_units(&a.name, &b.name));
        Ok((client, tools))
    }

    fn register_tool(&'static self, server: &Resolved, def: &ToolDef) -> Result<()> {
        let exposure = config::tool_exposure(&server.def, &def.mcp_name);
        let mut tool = Tool::<Self>::new(def.name.clone())
            .label(def.name.clone())
            .description(def.description.clone())
            .parameters(&def.parameters)
            .exposure(exposure_of(&exposure));
        if let Some(a) = annotations_of(def.annotations.as_ref()) {
            if a.read_only == Some(true) {
                tool = tool.kind(ToolKind::Read);
            }
            tool = tool.annotations(a);
        }
        let name = def.name.clone();
        tool.run(move |ext, args: Value, cx| {
            let name = name.clone();
            async move { Ok(ext.call(&name, &args, &cx.cx).await) }
        })
        .register(self)
    }

    fn unregister(names: &[String]) {
        for n in names {
            oino_extension::wit::registry::unregister_tool(n);
        }
    }

    /// unipi `startServers`: parallel connect + discovery, then one sorted registration.
    async fn start_servers(
        &'static self,
        servers: Vec<Resolved>,
    ) -> std::result::Result<(), String> {
        if servers.is_empty() {
            return Ok(());
        }
        self.registry.borrow().check_batch(&servers)?;
        for s in &servers {
            self.stop_server(&s.name).await;
            self.registry.borrow_mut().begin(s, &now_iso());
        }
        let results = join_all(servers.iter().map(Self::connect)).await;
        let mut prepared: Vec<(Resolved, Client, Vec<ToolDef>)> = Vec::new();
        for (s, r) in servers.into_iter().zip(results) {
            match r {
                Ok((c, t)) => prepared.push((s, c, t)),
                Err(e) => {
                    self.registry.borrow_mut().fail(&s.name, &e);
                    unipi_kit::bus::emit_raw(
                        topics::MCP_SERVER_ERROR,
                        &McpServer {
                            name: s.name.clone(),
                            error: Some(e),
                            ..McpServer::default()
                        },
                    );
                }
            }
        }
        if prepared.is_empty() {
            return Ok(());
        }
        let plan_input: Vec<(String, Vec<ToolDef>)> = prepared
            .iter()
            .map(|(s, _, t)| (s.name.clone(), t.clone()))
            .collect();
        let plan = self.registry.borrow().plan(&plan_input);
        let plan = match plan {
            Ok(p) => p,
            Err(e) => {
                for (s, c, _) in &prepared {
                    c.disconnect().await;
                    self.registry.borrow_mut().fail(&s.name, &e);
                    unipi_kit::bus::emit_raw(
                        topics::MCP_SERVER_ERROR,
                        &McpServer {
                            name: s.name.clone(),
                            error: Some(e.clone()),
                            ..McpServer::default()
                        },
                    );
                }
                return Err(e);
            }
        };
        let mut registered: Vec<String> = Vec::new();
        for (server, def) in &plan {
            let Some((r, _, _)) = prepared.iter().find(|(s, _, _)| s.name == *server) else {
                continue;
            };
            if let Err(e) = self.register_tool(r, def) {
                let msg = e.to_string();
                Self::unregister(&registered);
                for (s, c, _) in &prepared {
                    c.disconnect().await;
                    self.registry.borrow_mut().fail(&s.name, &msg);
                    unipi_kit::bus::emit_raw(
                        topics::MCP_SERVER_ERROR,
                        &McpServer {
                            name: s.name.clone(),
                            error: Some(msg.clone()),
                            ..McpServer::default()
                        },
                    );
                }
                return Err(msg);
            }
            registered.push(def.name.clone());
        }
        prepared.sort_by(|a, b| translate::compare_code_units(&a.0.name, &b.0.name));
        for (s, c, tools) in prepared {
            let names: Vec<String> = tools.iter().map(|t| t.name.clone()).collect();
            let count = names.len();
            self.registry.borrow_mut().running(&s.name, c.pid(), tools);
            self.clients.borrow_mut().insert(s.name.clone(), Rc::new(c));
            unipi_kit::bus::emit_raw(
                topics::MCP_SERVER_STARTED,
                &McpServer {
                    name: s.name.clone(),
                    tool_count: u32::try_from(count).ok(),
                    ..McpServer::default()
                },
            );
            if count > 0 {
                unipi_kit::bus::emit(&McpTools {
                    server_name: s.name.clone(),
                    tool_names: names,
                });
            }
        }
        Ok(())
    }

    /// unipi `stopServer`: unregister tools, disconnect.
    async fn stop_server(&self, name: &str) {
        let names = self.registry.borrow_mut().stop(name);
        Self::unregister(&names);
        let client = self.clients.borrow_mut().remove(name);
        if let Some(c) = client {
            c.disconnect().await;
        }
    }

    async fn stop_all(&self) {
        let names: Vec<String> = self
            .registry
            .borrow()
            .all()
            .iter()
            .map(|e| e.resolved.name.clone())
            .collect();
        for n in names {
            self.stop_server(&n).await;
        }
    }

    /// Loads configs and starts every enabled server (unipi `session_start`).
    async fn start_all(&'static self) {
        let (resolved, errors) = self.load();
        *self.errors.borrow_mut() = errors;
        let (enabled, disabled): (Vec<Resolved>, Vec<Resolved>) =
            resolved.into_iter().partition(|r| r.enabled);
        // Errors are tracked in registry state and surfaced by status and the info screen.
        let _ = self.start_servers(enabled).await;
        for d in &disabled {
            self.registry.borrow_mut().disabled(d);
        }
        let mut tools: Vec<String> = self
            .registry
            .borrow()
            .active()
            .flat_map(|e| e.tool_names.clone())
            .collect();
        tools.sort_by(|a, b| translate::compare_code_units(a, b));
        unipi_kit::bus::emit(&ModuleReady {
            name: "@pi-unipi/mcp".into(),
            version: VERSION.into(),
            commands: vec!["mcp-status".into()],
            tools,
            load_time_ms: None,
        });
    }

    async fn reload(&'static self) {
        self.stop_all().await;
        *self.registry.borrow_mut() = Registry::default();
        self.start_all().await;
    }

    /// Runs one MCP tool call (unipi translated `execute`).
    async fn call(&'static self, tool: &str, args: &Value, cx: &Ctx) -> ToolResult {
        let found = self
            .registry
            .borrow()
            .owner_of(tool)
            .map(|(e, t)| (e.resolved.clone(), t.mcp_name.clone()));
        let Some((server, mcp_name)) = found else {
            return ToolResult::text(translate::failure_text(
                "unknown",
                tool,
                "server is not running",
            ));
        };
        let _ = cx;
        let mut client = self.clients.borrow().get(&server.name).cloned();
        if client.as_ref().is_none_or(|c| c.is_broken()) {
            // A dropped connection reconnects on the next call (pi).
            match Self::connect(&server).await {
                Ok((c, _)) => {
                    let c = Rc::new(c);
                    self.clients
                        .borrow_mut()
                        .insert(server.name.clone(), Rc::clone(&c));
                    client = Some(c);
                }
                Err(e) => {
                    return ToolResult::text(translate::failure_text(&server.name, &mcp_name, &e))
                        .with_details(
                            json!({"error": true, "server": server.name, "tool": mcp_name}),
                        );
                }
            }
        }
        let Some(client) = client else {
            return ToolResult::text(translate::failure_text(
                &server.name,
                &mcp_name,
                "not connected",
            ));
        };
        let args = if args.is_object() {
            args.clone()
        } else {
            json!({})
        };
        match client.call_tool(&mcp_name, &args).await {
            Ok(result) => {
                let (content, details) = translate::format_result(
                    &server.name,
                    &mcp_name,
                    &result,
                    translate::bound_kit,
                );
                ToolResult {
                    content,
                    details: Some(details),
                    ..ToolResult::default()
                }
            }
            Err(e) => ToolResult::text(translate::failure_text(&server.name, &mcp_name, &e))
                .with_details(json!({"error": true, "server": server.name, "tool": mcp_name})),
        }
    }

    // ── config writes ───────────────────────────────────────────────────────

    fn save_server(name: &str, def: &Value) -> Result<()> {
        let mut doc = read_doc(GLOBAL_CONFIG)
            .filter(|d| d.get("mcpServers").is_some_and(Value::is_object))
            .unwrap_or_else(|| json!({"mcpServers": {}}));
        doc["mcpServers"][name] = def.clone();
        write_private(GLOBAL_CONFIG, &doc)?;
        let mut meta = read_doc(GLOBAL_META)
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        if meta.get("servers").is_none_or(|s| !s.is_object()) {
            meta["servers"] = json!({});
        }
        if meta["servers"].get(name).is_none() {
            meta["servers"][name] = json!({"enabled": true, "addedAt": now_iso()});
        }
        if meta.get("sync").is_none() {
            meta["sync"] =
                json!({"enabled": true, "lastSyncAt": null, "syncIntervalMs": 86_400_000});
        }
        write_private(GLOBAL_META, &meta)
    }

    fn remove_server_config(name: &str) -> Result<()> {
        if let Some(mut doc) = read_doc(GLOBAL_CONFIG)
            && let Some(m) = doc.get_mut("mcpServers").and_then(Value::as_object_mut)
        {
            m.remove(name);
            write_private(GLOBAL_CONFIG, &doc)?;
        }
        if let Some(mut meta) = read_doc(GLOBAL_META)
            && let Some(m) = meta.get_mut("servers").and_then(Value::as_object_mut)
        {
            m.remove(name);
            write_private(GLOBAL_META, &meta)?;
        }
        Ok(())
    }

    fn set_enabled(name: &str, enabled: bool) -> Result<()> {
        let mut meta = read_doc(GLOBAL_META)
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        if meta.get("servers").is_none_or(|s| !s.is_object()) {
            meta["servers"] = json!({});
        }
        let added = meta["servers"][name]
            .get("addedAt")
            .cloned()
            .unwrap_or_else(|| json!(now_iso()));
        meta["servers"][name] = json!({"enabled": enabled, "addedAt": added});
        write_private(GLOBAL_META, &meta)
    }

    /// JSON editor with validation (unipi add-overlay editor pane); returns `(name, entry)`.
    async fn edit_entry(title: &str, initial: &str) -> Option<(String, Value)> {
        let mut text = initial.to_owned();
        loop {
            let answer = ui::dialog(
                &json!({"method": "editor", "title": title, "prefill": text, "language": "json"}),
            )
            .await
            .ok()?;
            answer
                .get("value")
                .and_then(Value::as_str)?
                .clone_into(&mut text);
            let parsed: std::result::Result<Value, String> =
                serde_json::from_str(&text).map_err(|e| e.to_string());
            let problem = match &parsed {
                Err(e) => Some(format!("Invalid JSON: {e}")),
                Ok(v) => {
                    let doc = if v.get("mcpServers").is_some() {
                        v.clone()
                    } else {
                        json!({"mcpServers": v})
                    };
                    let f = config::parse_file(&doc, false);
                    if !f.errors.is_empty() {
                        Some(f.errors.join("\n"))
                    } else if f.servers.len() != 1 {
                        Some(
                            "Enter exactly one server: {\"name\": {\"command\": …, \"args\": […]}}"
                                .to_owned(),
                        )
                    } else {
                        let (name, def) = &f.servers[0];
                        return Some((name.clone(), config::entry_json(def)));
                    }
                }
            };
            if let Some(p) = problem {
                let _ = ui::notify(&format!("✗ {p}"), "error");
            }
        }
    }

    // ── panels ──────────────────────────────────────────────────────────────

    fn servers_spec(&self) -> Value {
        let reg = self.registry.borrow();
        let items: Vec<Value> = reg
            .all()
            .iter()
            .map(|e| {
                let s = &e.state;
                let detail = match (&s.error, s.status) {
                    (Some(err), Status::Error) => format!("{} · {err}", e.resolved.source.as_str()),
                    _ => format!("{} · {} tools", e.resolved.source.as_str(), s.tool_count),
                };
                let tone = match s.status {
                    Status::Running => "success",
                    Status::Error => "error",
                    Status::Starting => "warning",
                    Status::Stopped => "muted",
                };
                json!({"id": e.resolved.name, "label": format!("{} {}", s.status.icon(), s.name), "detail": detail, "badge": s.status.as_str(), "tone": tone})
            })
            .collect();
        let list = if items.is_empty() {
            json!({"t": "empty", "message": "No MCP servers configured. Add one from the catalog."})
        } else {
            json!({"t": "list", "items": items, "selectable": "select"})
        };
        let mut children = vec![
            list,
            json!({"t": "stack", "direction": "horizontal", "gap": 1, "children": [
                {"t": "button", "label": "Add server…", "actionId": "add", "primary": true},
                {"t": "button", "label": "Reload all", "actionId": "reload"},
                {"t": "button", "label": "Close", "actionId": "close"},
            ]}),
        ];
        let errors = self.errors.borrow();
        if !errors.is_empty() {
            children.insert(1, json!({"t": "section", "title": "Config problems", "body": {"t": "log", "lines": *errors}}));
        }
        children.push(json!({"t": "keyHints", "hints": [{"keys": "enter", "label": "details"}, {"keys": "a", "label": "add"}, {"keys": "q", "label": "close"}]}));
        json!({"t": "stack", "direction": "vertical", "gap": 1, "children": children})
    }

    fn server_spec(&self, name: &str) -> Value {
        let reg = self.registry.borrow();
        let Some(e) = reg.get(name) else {
            return json!({"t": "empty", "message": format!("Server \"{name}\" is gone.")});
        };
        let def = &e.resolved.def;
        let target = match def.transport {
            Transport::Stdio => format!("{} {}", def.command, def.args.join(" ")),
            Transport::Http => def.url.clone(),
        };
        let mut items = vec![
            json!({"key": "Status", "value": e.state.status.as_str()}),
            json!({"key": "Source", "value": e.resolved.source.as_str()}),
            json!({"key": "Enabled", "value": if e.resolved.enabled { "yes" } else { "no" }}),
            json!({"key": "Target", "value": target}),
            json!({"key": "Tools", "value": e.state.tool_count.to_string()}),
        ];
        if let Some(pid) = e.state.pid {
            items.push(json!({"key": "PID", "value": pid.to_string()}));
        }
        if let Some(err) = &e.state.error {
            items.push(json!({"key": "Error", "value": err}));
        }
        let tools: Vec<Value> = e
            .tools
            .iter()
            .map(|t| json!({"id": t.name, "label": t.name, "detail": config::tool_exposure(def, &t.mcp_name)}))
            .collect();
        let toggle = if e.resolved.enabled {
            "Disable"
        } else {
            "Enable"
        };
        json!({"t": "stack", "direction": "vertical", "gap": 1, "children": [
            {"t": "kv", "items": items},
            {"t": "section", "title": "Tools", "body": if tools.is_empty() { json!({"t": "empty", "message": "No tools."}) } else { json!({"t": "list", "items": tools}) }},
            {"t": "stack", "direction": "horizontal", "gap": 1, "children": [
                {"t": "button", "label": toggle, "actionId": "toggle"},
                {"t": "button", "label": "Edit…", "actionId": "edit"},
                {"t": "button", "label": "Reconnect", "actionId": "reconnect"},
                {"t": "button", "label": "Remove", "actionId": "remove", "tone": "error"},
                {"t": "button", "label": "Back", "actionId": "back"},
            ]},
        ]})
    }

    fn catalog_spec(query: &str) -> Value {
        let cat = catalog::load(
            std::fs::read_to_string(CATALOG_CACHE).ok().as_deref(),
            &now_iso(),
        );
        let items: Vec<Value> = catalog::search(&cat, query)
            .into_iter()
            .map(|e| {
                let badge = format!(
                    "{}{} · {}",
                    if e.official { "⭐ " } else { "" },
                    e.scope,
                    e.language
                );
                json!({"id": e.id, "label": e.name, "detail": e.description, "badge": badge})
            })
            .collect();
        let list = if items.is_empty() {
            json!({"t": "empty", "message": "No servers match."})
        } else {
            json!({"t": "list", "items": items, "selectable": "pick"})
        };
        json!({"t": "stack", "direction": "vertical", "gap": 1, "children": [
            {"t": "input", "id": "q", "value": query, "placeholder": "Search the catalog…", "onSubmit": "search"},
            {"t": "text", "value": [{"text": format!("{} · {} servers · updated {}", cat.source, cat.total_servers, cat.last_updated), "dim": true}]},
            list,
            {"t": "stack", "direction": "horizontal", "gap": 1, "children": [
                {"t": "button", "label": "Custom server…", "actionId": "custom"},
                {"t": "button", "label": "Sync catalog", "actionId": "sync"},
                {"t": "button", "label": "Close", "actionId": "close"},
            ]},
        ]})
    }

    fn open_panel(&self, cx: &Ctx, id: &str, title: &str, spec: &Value, view: View) -> Result<()> {
        let panel = ui::panel_open(&json!({
            "panelId": format!("{EXTENSION_ID}/{id}"),
            "placement": "overlay",
            "title": title,
            "spec": spec,
            "keys": [{"key": "a", "actionId": "add", "label": "Add"}, {"key": "r", "actionId": "reload", "label": "Reload"}],
            "focus": true,
            "closeOnEscape": true,
        }))?;
        self.panels.with(cx.session_id(), |p| {
            p.insert(panel, view);
        });
        Ok(())
    }

    fn show(&self, cx: &Ctx, panel: &str, view: View) {
        let (spec, title) = match &view {
            View::Servers => (self.servers_spec(), "MCP servers".to_owned()),
            View::Server(n) => (self.server_spec(n), format!("MCP server — {n}")),
            View::Catalog(q) => (Self::catalog_spec(q), "MCP catalog".to_owned()),
        };
        let _ = ui::panel_update(panel, &json!({"spec": spec, "title": title}));
        self.panels.with(cx.session_id(), |p| {
            p.insert(panel.to_owned(), view);
        });
    }

    async fn add_from(&'static self, entry: Option<&catalog::Entry>) {
        let (name, def) = catalog::template(entry);
        let initial = serde_json::to_string_pretty(&json!({ name: def })).unwrap_or_default();
        let title = entry.map_or_else(
            || "Add MCP server".to_owned(),
            |e| format!("Add MCP server — {}", e.name),
        );
        let Some((name, entry)) = Self::edit_entry(&title, &initial).await else {
            return;
        };
        if let Err(e) = Self::save_server(&name, &entry) {
            let _ = ui::notify(&format!("MCP server not saved: {e}"), "error");
            return;
        }
        let _ = ui::notify(&format!("MCP server \"{name}\" saved. Connecting…"), "info");
        self.start_named(&name).await;
    }

    async fn start_named(&'static self, name: &str) {
        let (resolved, errors) = self.load();
        *self.errors.borrow_mut() = errors;
        if let Some(r) = resolved.into_iter().find(|r| r.name == name) {
            if r.enabled {
                let result = self.start_servers(vec![r]).await;
                let state = self.registry.borrow().get(name).map(|e| e.state.clone());
                let msg = match (result, state) {
                    (Err(e), _) => format!("✗ {name}: {e}"),
                    (Ok(()), Some(s)) if s.status == Status::Running => {
                        format!("● {name} — running ({} tools)", s.tool_count)
                    }
                    (Ok(()), Some(s)) => format!(
                        "✗ {name} — {}",
                        s.error.unwrap_or_else(|| s.status.as_str().to_owned())
                    ),
                    (Ok(()), None) => format!("{name}: not started"),
                };
                let _ = ui::notify(&msg, "info");
            } else {
                self.stop_server(name).await;
                self.registry.borrow_mut().disabled(&r);
            }
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one match over every panel view and action"
    )]
    async fn on_ui_action(&'static self, ev: &Event, cx: &Ctx) {
        let Some(panel) = ev
            .get("panelId")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
        else {
            return;
        };
        let Some(view) = self
            .panels
            .with(cx.session_id(), |p| p.get(&panel).cloned())
        else {
            return;
        };
        let action = ev.get("actionId").and_then(Value::as_str).unwrap_or("");
        let value = ev
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        match (view, action) {
            (_, "$close") => {
                self.panels.with(cx.session_id(), |p| p.remove(&panel));
            }
            (_, "close") => {
                ui::panel_close(&panel);
                self.panels.with(cx.session_id(), |p| p.remove(&panel));
            }
            (_, "reload") => {
                self.reload().await;
                self.show(cx, &panel, View::Servers);
            }
            (_, "add") => self.show(cx, &panel, View::Catalog(String::new())),
            (View::Servers, "select") => self.show(cx, &panel, View::Server(value)),
            (View::Server(_) | View::Catalog(_), "back") => self.show(cx, &panel, View::Servers),
            (View::Server(name), "toggle") => {
                let enabled = self
                    .registry
                    .borrow()
                    .get(&name)
                    .is_some_and(|e| e.resolved.enabled);
                if Self::set_enabled(&name, !enabled).is_ok() {
                    self.start_named(&name).await;
                }
                self.show(cx, &panel, View::Server(name));
            }
            (View::Server(name), "reconnect") => {
                self.start_named(&name).await;
                self.show(cx, &panel, View::Server(name));
            }
            (View::Server(name), "edit") => {
                let entry = self
                    .registry
                    .borrow()
                    .get(&name)
                    .map(|e| (e.resolved.source, config::entry_json(&e.resolved.def)));
                match entry {
                    Some((Source::Project | Source::ProjectOverride, _)) => {
                        let _ =
                            ui::notify("Project servers are edited in .oino/mcp.json.", "warning");
                    }
                    Some((_, def)) => {
                        let initial = serde_json::to_string_pretty(&json!({ name.clone(): def }))
                            .unwrap_or_default();
                        if let Some((new_name, def)) =
                            Self::edit_entry(&format!("Edit MCP server — {name}"), &initial).await
                            && Self::save_server(&new_name, &def).is_ok()
                        {
                            if new_name != name {
                                let _ = Self::remove_server_config(&name);
                                self.stop_server(&name).await;
                                let _ = self.registry.borrow_mut().remove(&name);
                            }
                            self.start_named(&new_name).await;
                            self.show(cx, &panel, View::Server(new_name));
                            return;
                        }
                    }
                    None => {}
                }
                self.show(cx, &panel, View::Server(name));
            }
            (View::Server(name), "remove") => {
                if ui::confirm(
                    &format!("Remove MCP server \"{name}\"?"),
                    "Its tools are unregistered and its config entry deleted.",
                )
                .await
                .unwrap_or(false)
                {
                    let _ = Self::remove_server_config(&name);
                    self.stop_server(&name).await;
                    let _ = self.registry.borrow_mut().remove(&name);
                    self.show(cx, &panel, View::Servers);
                }
            }
            (View::Catalog(_), "search") => self.show(cx, &panel, View::Catalog(value)),
            (View::Catalog(q), "pick") => {
                let cat = catalog::load(
                    std::fs::read_to_string(CATALOG_CACHE).ok().as_deref(),
                    &now_iso(),
                );
                let entry = cat.servers.iter().find(|e| e.id == value).cloned();
                self.add_from(entry.as_ref()).await;
                self.show(cx, &panel, View::Catalog(q));
            }
            (View::Catalog(q), "custom") => {
                self.add_from(None).await;
                self.show(cx, &panel, View::Catalog(q));
            }
            (View::Catalog(q), "sync") => {
                self.sync().await;
                self.show(cx, &panel, View::Catalog(q));
            }
            _ => {}
        }
    }

    /// unipi `mcp-sync` runner.
    async fn sync(&self) {
        let _ = ui::notify("Syncing MCP catalog from GitHub...", "info");
        let fetched = match http::get(catalog::GITHUB_RAW_URL)
            .header("user-agent", "oino-unipi-mcp")
            .timeout_ms(30_000)
            .send()
            .await
        {
            Ok(r) if (200..300).contains(&r.status()) => Ok(r.text().await),
            Ok(r) => Err(format!("Failed to fetch catalog: HTTP {}", r.status())),
            Err(e) => Err(e.to_string()),
        };
        match fetched {
            Ok(md) => {
                let cat = catalog::from_markdown(&md, &now_iso());
                let _ = write_private(
                    CATALOG_CACHE,
                    &serde_json::to_value(&cat).unwrap_or_default(),
                );
                let mut meta = read_doc(GLOBAL_META)
                    .filter(Value::is_object)
                    .unwrap_or_else(|| json!({"servers": {}}));
                meta["sync"] = json!({"enabled": true, "lastSyncAt": cat.last_updated, "syncIntervalMs": 86_400_000});
                let _ = write_private(GLOBAL_META, &meta);
                let _ = ui::notify(
                    &format!(
                        "MCP Catalog Synced\nSource: {}\nServers: {}\nUpdated: {}",
                        cat.source, cat.total_servers, cat.last_updated
                    ),
                    "info",
                );
            }
            Err(e) => {
                let _ = ui::notify(&format!("MCP sync failed: {e}"), "error");
            }
        }
    }

    fn info_group(&self) -> Value {
        let reg = self.registry.borrow();
        let servers: Vec<Value> = reg
            .all()
            .iter()
            .map(|e| json!({"name": e.state.name, "status": e.state.status, "tools": e.state.tool_count, "error": e.state.error, "startedAt": e.state.started_at, "scope": e.resolved.source.as_str()}))
            .collect();
        let failed: Vec<String> = reg.failed().map(|e| e.state.name.clone()).collect();
        let mut failed_stat = json!({"value": failed.len().to_string()});
        if !failed.is_empty() {
            failed_stat["detail"] = json!(failed.join(", "));
        }
        json!({
            "id": "mcp", "name": "MCP Servers", "short": "🔌", "priority": 15,
            "config": {"showByDefault": true, "stats": [
                {"id": "total", "label": "Total servers", "show": true},
                {"id": "active", "label": "Active", "show": true},
                {"id": "tools", "label": "Total tools", "show": true},
                {"id": "failed", "label": "Failed", "show": true},
            ]},
            "data": {
                "raw": {"value": "", "raw": {"servers": servers}},
                "total": {"value": reg.all().len().to_string()},
                "active": {"value": reg.active().count().to_string()},
                "tools": {"value": reg.total_tools().to_string()},
                "failed": failed_stat,
            },
        })
    }

    async fn on_acp(&'static self, data: &Value) {
        let servers = config::from_acp(data);
        let new: Vec<(String, ServerDef)> = {
            let mut acp = self.acp.borrow_mut();
            let fresh: Vec<(String, ServerDef)> = servers
                .into_iter()
                .filter(|(n, _)| !acp.iter().any(|(m, _)| m == n))
                .collect();
            acp.extend(fresh.iter().cloned());
            fresh
        };
        if new.is_empty() || !self.started.get() {
            return;
        }
        let (resolved, _) = self.load();
        let start: Vec<Resolved> = resolved
            .into_iter()
            .filter(|r| r.source == Source::Acp && new.iter().any(|(n, _)| *n == r.name))
            .collect();
        let _ = self.start_servers(start).await;
    }
}

impl Extension for Mcp {
    #[allow(clippy::too_many_lines, reason = "one registration list")]
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        cx.settings_namespace(&namespace().to_json());
        cx.on(EventKind::SessionStart, |ext, _ev, cx| async move {
            if cx.is_child() {
                return Ok(());
            }
            ext.active_sessions.set(ext.active_sessions.get() + 1);
            if let Some(acp) = unipi_kit::bus::get_raw(topics::foreign::ACP_MCP_SERVERS) {
                let servers = config::from_acp(&acp);
                let mut cur = ext.acp.borrow_mut();
                for s in servers {
                    if !cur.iter().any(|(n, _)| *n == s.0) {
                        cur.push(s);
                    }
                }
            }
            if !ext.started.replace(true) {
                ext.start_all().await;
            }
            Ok(())
        });
        cx.on(EventKind::SessionShutdown, |ext, _ev, cx| async move {
            ext.panels.remove(cx.session_id());
            if cx.is_child() {
                return Ok(());
            }
            let n = ext.active_sessions.get().saturating_sub(1);
            ext.active_sessions.set(n);
            if n == 0 && ext.started.replace(false) {
                ext.stop_all().await;
                *ext.registry.borrow_mut() = Registry::default();
            }
            Ok(())
        });
        cx.on(EventKind::OinoBus, |ext, ev, _cx| async move {
            if let Some(d) = Delivery::from_event(&ev)
                && d.topic == topics::foreign::ACP_MCP_SERVERS
            {
                ext.on_acp(&d.data).await;
            }
            Ok(())
        });
        cx.on(EventKind::OinoUiAction, |ext, ev, cx| async move {
            ext.on_ui_action(&ev, &cx).await;
            Ok(())
        });
        cx.on_shutdown(|ext, _reason| async move {
            ext.stop_all().await;
        });
        unipi_kit::bus::subscribe(&[topics::foreign::ACP_MCP_SERVERS]);

        cx.command(
            Command::<Self>::new("mcp-status")
                .description("Show status of all configured MCP servers")
                .run(|ext, _args, _cx| async move {
                    match ext.registry.borrow().status_text() {
                        Some(text) => {
                            let _ = ui::notify(&text, "info");
                        }
                        None => {
                            let _ = ui::notify(
                                "No MCP servers configured. Add one via /settings (MCP → Add server…).",
                                "info",
                            );
                        }
                    }
                    Ok(())
                }),
        );
        cx.command(
            Command::<Self>::new("mcp-settings")
                .description("Open the MCP server registry")
                .hidden()
                .run(|ext, _args, cx| async move {
                    if !cx.has_ui() {
                        let _ = ui::notify("MCP Settings requires an interactive UI.", "warning");
                        return Ok(());
                    }
                    ext.open_panel(
                        &cx.cx,
                        "servers",
                        "MCP servers",
                        &ext.servers_spec(),
                        View::Servers,
                    )
                }),
        );
        cx.command(
            Command::<Self>::new("mcp-add")
                .description("Add an MCP server from the catalog")
                .hidden()
                .run(|ext, _args, cx| async move {
                    if !cx.has_ui() {
                        let _ = ui::notify("MCP Add requires an interactive UI.", "warning");
                        return Ok(());
                    }
                    ext.open_panel(
                        &cx.cx,
                        "catalog",
                        "MCP catalog",
                        &Self::catalog_spec(""),
                        View::Catalog(String::new()),
                    )
                }),
        );
        cx.command(
            Command::<Self>::new("mcp-sync")
                .description("Sync the MCP server catalog from GitHub")
                .hidden()
                .run(|ext, _args, _cx| async move {
                    ext.sync().await;
                    Ok(())
                }),
        );
        cx.command(
            Command::<Self>::new("mcp-reload")
                .description("Reconnect every MCP server and re-register its tools")
                .hidden()
                .run(|ext, _args, _cx| async move {
                    ext.reload().await;
                    let r = ext.registry.borrow();
                    let _ = ui::notify(
                        &format!(
                            "Reloaded MCP servers: {} active, {} failed, {} tools.",
                            r.active().count(),
                            r.failed().count(),
                            r.total_tools()
                        ),
                        "info",
                    );
                    Ok(())
                }),
        );
        cx.service(
            unipi_kit::services::fanout::INFO_SCREEN_GROUPS,
            |ext, _p, _caller, _cx| async move { Ok(ext.info_group()) },
        );
        Ok(())
    }
}

// Native builds (unit tests) skip the component exports.
#[cfg(target_arch = "wasm32")]
oino_extension::export_extension!(Mcp);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposure_words() {
        assert!(matches!(exposure_of("direct"), Exposure::Direct));
        assert!(matches!(exposure_of("codemode"), Exposure::Deferred));
        assert!(matches!(exposure_of("deferred"), Exposure::Deferred));
        assert!(matches!(exposure_of("hidden"), Exposure::Hidden));
        assert!(matches!(exposure_of("weird"), Exposure::Direct));
    }

    #[test]
    fn annotation_mapping() {
        let a = annotations_of(Some(&json!({"readOnlyHint": true, "openWorldHint": false})))
            .unwrap_or_else(|| unreachable!());
        assert_eq!(a.read_only, Some(true));
        assert_eq!(a.open_world, Some(false));
        assert_eq!(a.destructive, None);
        assert!(annotations_of(None).is_none());
    }

    #[test]
    fn hub_namespace() {
        let v = namespace().to_json();
        assert_eq!(v["namespace"], "mcp");
        let f = &v["sections"][0]["fields"];
        let cmds: Vec<&str> = f
            .as_array()
            .map(|a| a.iter().filter_map(|x| x["command"].as_str()).collect())
            .unwrap_or_default();
        assert_eq!(cmds, ["mcp-settings", "mcp-add", "mcp-sync", "mcp-reload"]);
        assert_eq!(
            v["sections"][0]["description"],
            "Server registry lives in its own overlay (catalog, jira, …)"
        );
    }
}
